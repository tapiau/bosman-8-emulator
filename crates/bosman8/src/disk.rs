//! Floppy formats of the Bosman-8 WD1770 drives (B: = drive 0, C: = drive 1).
//!
//! Derived from the CPM-R disk code (0x0A62, 0x0CBA): every format uses
//! 5 physical sectors of 1024 bytes per track (= 40 logical CP/M sectors),
//! the same sector numbers on both sides. The format is selected by the
//! jumpers on port 0x98 bits 3 and 5 (which DPB table is copied to F2BF).
//! See `doc/bosman8-hardware.md` for details.

use b8_chips::DiskImage;

pub const SECTORS: u8 = 5;
pub const SECTOR_SIZE: usize = 1024;

/// Physical format of one drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub cylinders: u8,
    pub sides: u8,
    /// ID of the first sector (both sides).
    pub first_sector: u8,
}

impl Format {
    /// Image size in bytes.
    pub fn image_len(&self) -> usize {
        self.cylinders as usize * self.sides as usize * SECTORS as usize * SECTOR_SIZE
    }

    pub fn kbytes(&self) -> usize {
        self.image_len() / 1024
    }

    /// Raw sector image → disk. The image is track-major (cylinder, side,
    /// sector), which for double sided formats equals CP/M logical track order
    /// (logical track t = cylinder t/2, side t&1). Short images are padded with 0xE5.
    pub fn from_bytes(&self, data: Vec<u8>) -> DiskImage {
        DiskImage::from_bytes(self.cylinders, self.sides, SECTORS, SECTOR_SIZE, self.first_sector, data)
    }

    /// Blank, formatted (0xE5 = empty directory) disk.
    pub fn blank(&self) -> DiskImage {
        DiskImage::blank(self.cylinders, self.sides, SECTORS, SECTOR_SIZE, self.first_sector)
    }
}

/// 200 KB: 40 cylinders, 1 side, sectors 0x11–0x15.
pub const F200: Format = Format { cylinders: 40, sides: 1, first_sector: 0x11 };
/// 400 KB: 40 cylinders, 2 sides, sectors 0x21–0x25.
pub const F400: Format = Format { cylinders: 40, sides: 2, first_sector: 0x21 };
/// 800 KB: 80 cylinders, 2 sides, sectors 0x31–0x35.
pub const F800: Format = Format { cylinders: 80, sides: 2, first_sector: 0x31 };

/// Format used by the ROM for `drive` (0 = B:, 1 = C:) with the given port 0x98 jumpers.
pub fn format_for(dip: u8, drive: usize) -> Format {
    match dip & 0x28 {
        0x00 => F200,            // DPB built into the RAM image (ROM 0x2DBF)
        0x08 => F400,            // table at 0x0079
        0x20 => F800,            // table at 0x00A1
        _ if drive == 0 => F800, // table at 0x00C9
        _ => F200,
    }
}

/// Format whose first sector ID is `id` (as found on an HFE disk).
pub fn format_by_first_sector(id: u8) -> Option<Format> {
    [F200, F400, F800].into_iter().find(|f| f.first_sector == id)
}

/// How a disk image is stored in its file.
#[derive(Debug, Clone)]
pub enum Container {
    /// Raw sectors, track-major (the emulator's own `.img`).
    Raw,
    /// Gotek HFE: written back by patching sector data fields in place.
    Hfe(Box<crate::hfe::Hfe>),
}

/// A disk file opened for a drive.
#[derive(Debug)]
pub struct Opened {
    pub image: DiskImage,
    pub container: Container,
    /// Mismatch with the drive format. For HFE the image is then already
    /// write-protected (sectors missing from the file can't be written back);
    /// a raw image of the wrong size is left to the caller.
    pub warning: Option<String>,
}

/// File contents → disk for a drive using `format`.
pub fn open_bytes(data: Vec<u8>, format: Format) -> Result<Opened, String> {
    if !crate::hfe::is_hfe(&data) && !data.starts_with(b"HXCHFEV3") {
        let warning = (data.len() != format.image_len()).then(|| {
            format!("{} bytes, expected {} ({} KB) for this format", data.len(), format.image_len(), format.kbytes())
        });
        return Ok(Opened { image: format.from_bytes(data), container: Container::Raw, warning });
    }
    let hfe = crate::hfe::Hfe::parse(data)?;
    let mut image = format.blank();
    let mut missing = 0;
    for c in 0..format.cylinders {
        for s in 0..format.sides {
            for id in format.first_sector..format.first_sector + SECTORS {
                let off = image.offset(c, s, id).expect("geometry");
                match hfe.sector(c, s, id) {
                    Some(d) if d.len() == SECTOR_SIZE => image.data[off..off + SECTOR_SIZE].copy_from_slice(d),
                    _ => missing += 1,
                }
            }
        }
    }
    let warning = (missing > 0).then(|| {
        let total = format.cylinders as usize * format.sides as usize * SECTORS as usize;
        let looks = match hfe.first_sector_id().and_then(format_by_first_sector) {
            Some(f) if f != format => format!("; the disk looks like the {} KB format", f.kbytes()),
            _ => String::new(),
        };
        image.write_protected = true;
        format!(
            "HFE: {missing} of {total} sectors of the {} KB format not found{looks} — write-protected",
            format.kbytes()
        )
    });
    Ok(Opened { image, container: Container::Hfe(Box::new(hfe)), warning })
}

impl Container {
    /// File contents for `img` (opened with this container).
    pub fn to_bytes(&mut self, img: &DiskImage) -> Vec<u8> {
        match self {
            Container::Raw => img.data.clone(),
            Container::Hfe(h) => {
                for c in 0..img.tracks {
                    for s in 0..img.sides {
                        for id in img.first_sector..img.first_sector + img.sectors_per_track {
                            if let Some(off) = img.offset(c, s, id) {
                                h.write_sector(c, s, id, &img.data[off..off + img.sector_size]);
                            }
                        }
                    }
                }
                h.bytes().to_vec()
            }
        }
    }
}

/// True if `path` names an HFE file (by extension).
pub fn is_hfe_path(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("hfe"))
}

/// Contents of a new blank disk file: HFE if the name ends in `.hfe`, raw otherwise.
pub fn blank_file(path: &std::path::Path, format: Format) -> Vec<u8> {
    let d = format.blank();
    if is_hfe_path(path) {
        crate::hfe::create(format.cylinders, format.sides, SECTORS, SECTOR_SIZE, format.first_sector, &d.data)
    } else {
        d.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hfe_open_and_write_back() {
        let raw: Vec<u8> = (0..F400.image_len()).map(|i| (i % 251) as u8).collect();
        let file = crate::hfe::create(40, 2, SECTORS, SECTOR_SIZE, 0x21, &raw);
        let mut o = open_bytes(file.clone(), F400).unwrap();
        assert!(o.warning.is_none() && !o.image.write_protected);
        assert_eq!(o.image.data, raw);
        let off = o.image.offset(39, 1, 0x25).unwrap();
        o.image.data[off..off + 3].copy_from_slice(b"B8!");
        let saved = o.container.to_bytes(&o.image);
        assert_eq!(saved.len(), file.len());
        assert_eq!(open_bytes(saved, F400).unwrap().image.data, o.image.data);
        // Wrong format: protected, with a hint.
        let w = open_bytes(file, F200).unwrap();
        assert!(w.image.write_protected);
        assert!(w.warning.unwrap().contains("400 KB"));
        // Blank .hfe file.
        let b = open_bytes(blank_file(std::path::Path::new("x.HFE"), F800), F800).unwrap();
        assert!(b.warning.is_none() && b.image.data.iter().all(|&x| x == 0xE5));
    }

    #[test]
    fn formats() {
        assert_eq!(format_for(0x40, 0).image_len(), 204_800);
        assert_eq!(format_for(0x40, 1), F200);
        assert_eq!(format_for(0x08, 1).kbytes(), 400);
        assert_eq!(format_for(0x20, 0).kbytes(), 800);
        assert_eq!(format_for(0x28, 0), F800);
        assert_eq!(format_for(0x28, 1), F200);
        let d = F800.blank();
        assert_eq!(d.offset(1, 1, 0x35), Some((3 * 5 + 4) * 1024));
    }

    /// The tables in the ROM agree with the constants above.
    #[test]
    fn rom_tables() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/bosman8/bosman8.bin");
        let Ok(rom) = std::fs::read(path) else { return };
        // (table address, drive, format): per drive 20 bytes =
        // 15-byte DPB + PSH, PHM, first sector side 0, side 1, flags.
        let cases = [
            (0x2DBF, 0, F200),
            (0x2DD3, 1, F200),
            (0x0079, 0, F400),
            (0x0079 + 20, 1, F400),
            (0x00A1, 0, F800),
            (0x00A1 + 20, 1, F800),
            (0x00C9, 0, F800),
            (0x00C9 + 20, 1, F200),
        ];
        for (addr, _drive, f) in cases {
            let e = &rom[addr..addr + 20];
            let spt = u16::from_le_bytes([e[0], e[1]]) as usize;
            let bsh = e[2];
            let dsm = u16::from_le_bytes([e[5], e[6]]) as usize;
            assert_eq!(spt, 40, "{addr:#x}");
            assert_eq!(e[15], 3, "PSH {addr:#x}");
            assert_eq!(spt >> e[15], SECTORS as usize);
            assert_eq!((dsm + 1) * (128 << bsh), f.image_len(), "{addr:#x}");
            assert_eq!(e[17], f.first_sector, "{addr:#x}");
            assert_eq!(e[18], f.first_sector, "{addr:#x}");
            // Flags bit 6: alternate sides per logical track; else side split
            // at 40 (or 80 with bit 5) logical tracks, i.e. single sided here.
            let alternate = e[19] & 0x50 == 0x40;
            assert_eq!(alternate, f.sides == 2, "{addr:#x}");
        }
    }
}
