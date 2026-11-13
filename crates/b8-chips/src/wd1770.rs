//! Western Digital WD1770 floppy disk controller.
//!
//! Registers: 0 status/command, 1 track, 2 sector, 3 data.
//! Type I (restore/seek/step), type II (read/write sector), type III (read
//! address; read/write track may be unimplemented), type IV (force interrupt).
//! Data transfer is polled through DRQ (status bit 1) by the CPU; `intrq()`
//! and `drq()` expose the pins so the machine can wire them (e.g. to NMI).
//! Commands take time (`tick`) — at least long enough that ROM polling loops
//! observe BUSY.
//!
//! Disks are sector images (`DiskImage`): geometry tracks × sides × sectors
//! × sector size, sectors numbered from `first_sector`. Missing disk → status
//! "not ready"/ record not found as appropriate.

#[derive(Debug, Clone)]
pub struct DiskImage {
    pub tracks: u8,
    pub sides: u8,
    pub sectors_per_track: u8,
    pub sector_size: usize,
    pub first_sector: u8,
    pub data: Vec<u8>,
    pub write_protected: bool,
}

#[derive(Debug, Clone)]
pub struct Wd1770 { /* private */ _p: () }

impl Wd1770 {
    pub fn new(cpu_clock_hz: u32) -> Self { let _ = cpu_clock_hz; todo!() }
    pub fn reset(&mut self) { todo!() }
    pub fn write(&mut self, reg: u8, v: u8) { let _ = (reg, v); todo!() }
    pub fn read(&mut self, reg: u8) -> u8 { let _ = reg; todo!() }
    pub fn tick(&mut self, cpu_cycles: u32) { let _ = cpu_cycles; todo!() }
    pub fn intrq(&self) -> bool { todo!() }
    pub fn drq(&self) -> bool { todo!() }
    /// Select drive (None = no drive selected) and side.
    pub fn select(&mut self, drive: Option<usize>, side: u8) { let _ = (drive, side); todo!() }
    pub fn insert(&mut self, drive: usize, disk: Option<DiskImage>) { let _ = (drive, disk); todo!() }
    pub fn disk(&self, drive: usize) -> Option<&DiskImage> { let _ = drive; todo!() }
}
