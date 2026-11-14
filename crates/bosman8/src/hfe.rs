//! HxC Floppy Emulator images (HFE v1, signature `HXCPICFE`), the format
//! Gotek drives with HxC / FlashFloppy firmware read from USB sticks.
//!
//! An HFE file holds the raw MFM cell stream of every track: after a 512-byte
//! header and a track list, each track is a run of 512-byte blocks whose first
//! 256 bytes belong to side 0 and the next 256 to side 1; bits go LSB first,
//! one bit per cell (two cells per data bit: clock, data).
//!
//! [`Hfe::parse`] decodes the IBM MFM sectors (CRC-checked) and remembers
//! where each data field lies in the stream. [`Hfe::write_sector`] re-encodes
//! just that data field and its CRC in place, so gaps, sector order and
//! anything else on the track stay bit-for-bit as the Gotek (or the original
//! floppy) wrote them. [`create`] builds a fresh file with a standard WD177x
//! track layout.

use std::collections::HashMap;

use b8_chips::wd1770::crc16;

pub const SIGNATURE: &[u8; 8] = b"HXCPICFE";
const HEADER_LEN: usize = 512;
const BLOCK: usize = 512;
/// A1 with a missing clock bit (marks the start of an address mark).
const SYNC_A1: u16 = 0x4489;
/// C2 with a missing clock bit (index address mark).
const SYNC_C2: u16 = 0x5224;
/// Data bytes per DD track side (250 kbit/s, 300 rpm).
const TRACK_BYTES: usize = 6250;
/// How far (in cells) after an ID field its data mark may follow.
const MAX_ID_TO_DATA_CELLS: usize = 64 * 16;

pub fn is_hfe(data: &[u8]) -> bool {
    data.starts_with(SIGNATURE)
}

#[derive(Debug, Clone)]
struct Sector {
    /// Cell index of the first data byte (just after the FB/F8 mark).
    cell: usize,
    /// Address mark byte (FB = normal, F8 = deleted), part of the CRC.
    mark: u8,
    data: Vec<u8>,
}

/// One side of one track inside the file.
#[derive(Debug, Clone, Copy)]
struct Stream {
    /// File offset of the track's first block.
    offset: usize,
    side: usize,
    /// Number of cells.
    cells: usize,
}

impl Stream {
    fn pos(&self, cell: usize) -> (usize, u8) {
        let cell = cell % self.cells;
        let byte = cell / 8;
        (self.offset + byte / 256 * BLOCK + self.side * 256 + byte % 256, 1 << (cell % 8))
    }

    fn get(&self, file: &[u8], cell: usize) -> u8 {
        let (o, m) = self.pos(cell);
        (file[o] & m != 0) as u8
    }

    fn set(&self, file: &mut [u8], cell: usize, v: u8) {
        let (o, m) = self.pos(cell);
        if v != 0 {
            file[o] |= m;
        } else {
            file[o] &= !m;
        }
    }

    /// Data byte whose 16 cells start at `cell` (data bits are the odd cells).
    fn byte(&self, file: &[u8], cell: usize) -> u8 {
        (0..8).fold(0, |b, q| (b << 1) | self.get(file, cell + 2 * q + 1))
    }

    fn bytes(&self, file: &[u8], cell: usize, n: usize) -> Vec<u8> {
        (0..n).map(|k| self.byte(file, cell + 16 * k)).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Hfe {
    file: Vec<u8>,
    tracks: u8,
    sides: u8,
    /// Per track, per side.
    streams: Vec<[Option<Stream>; 2]>,
    /// (track index in the file, side, sector ID) → sector.
    sectors: HashMap<(u8, u8, u8), Sector>,
}

impl Hfe {
    pub fn parse(file: Vec<u8>) -> Result<Self, String> {
        if file.starts_with(b"HXCHFEV3") {
            return Err("HFE v3 is not supported (save the image as HFE v1)".into());
        }
        if !is_hfe(&file) || file.len() < HEADER_LEN {
            return Err("not an HFE file".into());
        }
        let tracks = file[9];
        let sides = file[10];
        let list = u16::from_le_bytes([file[18], file[19]]) as usize * BLOCK;
        if tracks == 0 || !(1..=2).contains(&sides) || list + 4 * tracks as usize > file.len() {
            return Err(format!("bad HFE header ({tracks} tracks, {sides} sides)"));
        }
        let mut h = Hfe { file, tracks, sides, streams: Vec::new(), sectors: HashMap::new() };
        for t in 0..tracks as usize {
            let e = list + 4 * t;
            let offset = u16::from_le_bytes([h.file[e], h.file[e + 1]]) as usize * BLOCK;
            let len = u16::from_le_bytes([h.file[e + 2], h.file[e + 3]]) as usize;
            let mut pair = [None, None];
            // Each side gets len/2 bytes, interleaved in 256-byte halves of blocks.
            for (side, slot) in pair.iter_mut().enumerate().take(sides as usize) {
                let s = Stream { offset, side, cells: len / 2 * 8 };
                if s.cells >= 16 && s.pos(s.cells - 1).0 < h.file.len() {
                    *slot = Some(s);
                }
            }
            for (side, s) in pair.iter().enumerate() {
                if let Some(s) = s {
                    h.scan(t as u8, side as u8, *s);
                }
            }
            h.streams.push(pair);
        }
        Ok(h)
    }

    /// Finds the CRC-correct sectors of one track side.
    fn scan(&mut self, track: u8, side: u8, s: Stream) {
        let f = &self.file;
        // Prime the shift register with the end of the track so a mark that
        // straddles the index is found too.
        let mut w: u16 = 0;
        for c in s.cells - 16..s.cells {
            w = (w << 1) | s.get(f, c) as u16;
        }
        let mut id: Option<(usize, [u8; 4])> = None;
        for c in 0..s.cells {
            w = (w << 1) | s.get(f, c) as u16;
            if w != SYNC_A1 {
                continue;
            }
            let p = c + 1; // first cell of the byte after the sync
            let mark = s.byte(f, p);
            match mark {
                0xFE => {
                    let b = s.bytes(f, p + 16, 6);
                    let crc = crc16(crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, 0xFE]), &b[..4]);
                    id = (crc == u16::from_be_bytes([b[4], b[5]])).then(|| (p, [b[0], b[1], b[2], b[3]]));
                }
                0xFB | 0xF8 => {
                    let Some((at, [_, _, r, n])) = id.take() else { continue };
                    if p - at > MAX_ID_TO_DATA_CELLS {
                        continue;
                    }
                    let size = 128usize << (n & 3);
                    let b = s.bytes(f, p + 16, size + 2);
                    let crc = crc16(crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, mark]), &b[..size]);
                    if crc == u16::from_be_bytes([b[size], b[size + 1]]) {
                        self.sectors.entry((track, side, r)).or_insert(Sector {
                            cell: (p + 16) % s.cells,
                            mark,
                            data: b[..size].to_vec(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    pub fn tracks(&self) -> u8 {
        self.tracks
    }

    pub fn sides(&self) -> u8 {
        self.sides
    }

    /// Data of a sector, `track` being the track index in the file.
    pub fn sector(&self, track: u8, side: u8, id: u8) -> Option<&[u8]> {
        self.sectors.get(&(track, side, id)).map(|s| &s.data[..])
    }

    /// Lowest sector ID on track 0 side 0 (identifies the Bosman format).
    pub fn first_sector_id(&self) -> Option<u8> {
        self.sectors.keys().filter(|k| k.0 == 0 && k.1 == 0).map(|k| k.2).min()
    }

    /// Rewrites a sector's data field and CRC in place. Fails if the sector
    /// is not in the file or `data` has a different size.
    pub fn write_sector(&mut self, track: u8, side: u8, id: u8, data: &[u8]) -> bool {
        let Some(sec) = self.sectors.get_mut(&(track, side, id)) else { return false };
        if sec.data.len() != data.len() {
            return false;
        }
        if sec.data == data {
            return true;
        }
        let Some(s) = self.streams[track as usize][side as usize] else { return false };
        let crc = crc16(crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, sec.mark]), data);
        let mut prev = s.get(&self.file, sec.cell + s.cells - 1); // last data bit of the mark
        let mut c = sec.cell;
        for &b in data.iter().chain(&crc.to_be_bytes()) {
            for q in (0..8).rev() {
                let d = (b >> q) & 1;
                s.set(&mut self.file, c, (prev == 0 && d == 0) as u8);
                s.set(&mut self.file, c + 1, d);
                prev = d;
                c += 2;
            }
        }
        // The clock of the first gap bit depends on the last CRC bit.
        let next = s.get(&self.file, c + 1);
        s.set(&mut self.file, c, (prev == 0 && next == 0) as u8);
        sec.data.copy_from_slice(data);
        true
    }

    pub fn bytes(&self) -> &[u8] {
        &self.file
    }
}

/// MFM cell writer.
struct Mfm {
    cells: Vec<u8>,
    prev: u8,
}

impl Mfm {
    fn byte(&mut self, b: u8) {
        for q in (0..8).rev() {
            let d = (b >> q) & 1;
            self.cells.push((self.prev == 0 && d == 0) as u8);
            self.cells.push(d);
            self.prev = d;
        }
    }

    fn fill(&mut self, b: u8, n: usize) {
        (0..n).for_each(|_| self.byte(b));
    }

    fn sync(&mut self, pattern: u16, data: u8) {
        for _ in 0..3 {
            self.cells.extend((0..16).rev().map(|q| ((pattern >> q) & 1) as u8));
        }
        self.prev = data & 1;
    }
}

/// New HFE v1 file holding a sector image (track-major: track, side, sector),
/// laid out the way the WD177x formats a DD track.
pub fn create(tracks: u8, sides: u8, sectors: u8, size: usize, first_sector: u8, data: &[u8]) -> Vec<u8> {
    let n = sectors as usize;
    let size_code = (size / 128).trailing_zeros() as u8;
    let per_sector = 12 + 4 + 6 + 22 + 12 + 4 + size + 2;
    let gap3 = (TRACK_BYTES.saturating_sub(146 + n * per_sector) / (n + 1)).clamp(1, 84);
    let side_bytes = TRACK_BYTES * 2; // one bit per cell, two cells per data bit
    let track_len = side_bytes * 2;
    let track_blocks = track_len.div_ceil(BLOCK);
    let list_blocks = (4 * tracks as usize).div_ceil(BLOCK);
    let first_track_block = 1 + list_blocks;
    let mut file = vec![0xFF; (first_track_block + tracks as usize * track_blocks) * BLOCK];
    let header = &mut file[..20];
    header[..8].copy_from_slice(SIGNATURE);
    header[8] = 0; // format revision
    header[9] = tracks;
    header[10] = sides;
    header[11] = 0; // ISO/IBM MFM
    header[12..14].copy_from_slice(&250u16.to_le_bytes()); // kbit/s
    header[14..16].copy_from_slice(&300u16.to_le_bytes()); // rpm
    header[16] = 7; // generic Shugart DD
    header[17] = 1;
    header[18..20].copy_from_slice(&1u16.to_le_bytes()); // track list at block 1
    for t in 0..tracks as usize {
        let block = first_track_block + t * track_blocks;
        let e = BLOCK + 4 * t;
        file[e..e + 2].copy_from_slice(&(block as u16).to_le_bytes());
        file[e + 2..e + 4].copy_from_slice(&(track_len as u16).to_le_bytes());
        for side in 0..2usize {
            let mut m = Mfm { cells: Vec::with_capacity(side_bytes * 8), prev: 0 };
            m.fill(0x4E, 80);
            m.fill(0x00, 12);
            m.sync(SYNC_C2, 0xC2);
            m.byte(0xFC);
            m.fill(0x4E, 50);
            if side < sides as usize {
                for i in 0..n {
                    let id = [t as u8, side as u8, first_sector + i as u8, size_code];
                    m.fill(0x00, 12);
                    m.sync(SYNC_A1, 0xA1);
                    m.byte(0xFE);
                    let crc = crc16(crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, 0xFE]), &id);
                    id.iter().chain(&crc.to_be_bytes()).for_each(|&b| m.byte(b));
                    m.fill(0x4E, 22);
                    m.fill(0x00, 12);
                    m.sync(SYNC_A1, 0xA1);
                    m.byte(0xFB);
                    let off = ((t * sides as usize + side) * n + i) * size;
                    let blank = vec![0xE5; size];
                    let sec = data.get(off..off + size).unwrap_or(&blank);
                    let crc = crc16(crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, 0xFB]), sec);
                    sec.iter().chain(&crc.to_be_bytes()).for_each(|&b| m.byte(b));
                    m.fill(0x4E, gap3);
                }
            }
            while m.cells.len() < side_bytes * 8 {
                m.byte(0x4E);
            }
            m.cells.truncate(side_bytes * 8);
            let s = Stream { offset: block * BLOCK, side, cells: side_bytes * 8 };
            for (c, &v) in m.cells.iter().enumerate() {
                s.set(&mut file, c, v);
            }
        }
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(tracks: u8, sides: u8) -> Vec<u8> {
        (0..tracks as usize * sides as usize * 5 * 1024).map(|i| (i * 7 + i / 1024) as u8).collect()
    }

    #[test]
    fn create_parse_roundtrip() {
        let data = image(40, 2);
        let h = Hfe::parse(create(40, 2, 5, 1024, 0x21, &data)).unwrap();
        assert_eq!((h.tracks(), h.sides()), (40, 2));
        assert_eq!(h.first_sector_id(), Some(0x21));
        for t in 0..40u8 {
            for side in 0..2u8 {
                for i in 0..5u8 {
                    let off = ((t as usize * 2 + side as usize) * 5 + i as usize) * 1024;
                    assert_eq!(h.sector(t, side, 0x21 + i), Some(&data[off..off + 1024]), "{t}/{side}/{i}");
                }
            }
        }
    }

    #[test]
    fn write_sector_in_place() {
        let data = image(40, 1);
        let mut h = Hfe::parse(create(40, 1, 5, 1024, 0x11, &data)).unwrap();
        let before = h.bytes().to_vec();
        let new: Vec<u8> = (0..1024).map(|i| (255 - i % 256) as u8).collect();
        assert!(h.write_sector(7, 0, 0x13, &new));
        assert!(!h.write_sector(7, 0, 0x16, &new), "no such sector");
        assert!(!h.write_sector(7, 0, 0x13, &new[..512]), "wrong size");
        let after = h.bytes().to_vec();
        assert_eq!(before.len(), after.len());
        // Only track 7 changed, and re-reading the file finds the new data
        // with a correct CRC while the neighbours are intact.
        let h2 = Hfe::parse(after).unwrap();
        assert_eq!(h2.sector(7, 0, 0x13), Some(&new[..]));
        let off = (7 * 5 + 1) * 1024;
        assert_eq!(h2.sector(7, 0, 0x12), Some(&data[off..off + 1024]));
        let off = (7 * 5 + 3) * 1024;
        assert_eq!(h2.sector(7, 0, 0x14), Some(&data[off..off + 1024]));
        assert_eq!(h2.sectors.len(), 200);
    }

    #[test]
    fn rejects_other_files() {
        assert!(Hfe::parse(vec![0xE5; 204_800]).is_err());
        assert!(Hfe::parse(b"HXCHFEV3".to_vec()).is_err());
    }
}
