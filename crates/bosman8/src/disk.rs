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

#[cfg(test)]
mod tests {
    use super::*;

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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/bosman8.bin");
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
