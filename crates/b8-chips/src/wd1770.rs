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
//!
//! Model details and simplifications:
//! - Up to [`MAX_DRIVES`] drives. Each drive has its own physical head
//!   position (0..=[`MAX_PHYS_TRACK`]); the track register is shared.
//!   Track 0 sensor = drive selected and head at physical track 0.
//! - Timing is configurable through [`Wd1770Timing`]. Step rates are the
//!   datasheet values (6/12/20/30 ms); spin-up is shortened (default 100 ms
//!   instead of 6 revolutions), sector search uses a fixed latency (default
//!   2 ms) instead of real rotational position, a byte takes 32 µs (MFM DD).
//! - Lost data is real: if the CPU does not serve DRQ within one byte time
//!   the next byte overwrites it (read) / 0 is written (write) and status
//!   bit 2 is set. The machine must therefore tick the chip at instruction
//!   granularity during transfers.
//! - The 1770 has no READY input. No drive selected / no disk: there are no
//!   index pulses and no ID fields, so type II/III commands end with RNF after
//!   the RNF timeout (default 5 revolutions = 1 s) and verify ends with a seek
//!   error. Spin-up still completes (a real chip would wait forever).
//! - The ID track field is the physical track, the ID side field is the
//!   selected side; the track register must equal the physical track or RNF.
//! - CRC errors never happen. The record type (deleted data mark) is always 0.
//! - Read track synthesises an IBM MFM track from the image; write track
//!   (format) parses ID fields (after 0xFE) and data fields (after 0xFB/0xF8)
//!   and writes data of sectors that fit the image geometry.
//! - Force interrupt: I3 (immediate) and I2 (each index pulse) are supported;
//!   I0/I1 are ignored (meaningless on the 1770 without READY).

/// Number of drives the controller can address.
pub const MAX_DRIVES: usize = 4;
/// Highest physical track the head can step to.
pub const MAX_PHYS_TRACK: u8 = 83;
/// Raw bytes per MFM DD track (250 kbit/s, 300 rpm).
pub const TRACK_LEN: usize = 6250;

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

impl DiskImage {
    /// Blank (0xE5-filled, i.e. empty CP/M directory) image.
    pub fn blank(tracks: u8, sides: u8, sectors_per_track: u8, sector_size: usize, first_sector: u8) -> Self {
        let mut d = Self {
            tracks,
            sides,
            sectors_per_track,
            sector_size,
            first_sector,
            data: Vec::new(),
            write_protected: false,
        };
        d.data = vec![0xE5; d.expected_len()];
        d
    }

    /// Image from raw bytes (track-major: track, side, sector). The data is
    /// padded with 0xE5 / truncated to the geometry size.
    pub fn from_bytes(
        tracks: u8,
        sides: u8,
        sectors_per_track: u8,
        sector_size: usize,
        first_sector: u8,
        mut data: Vec<u8>,
    ) -> Self {
        let mut d = Self::blank(tracks, sides, sectors_per_track, sector_size, first_sector);
        data.resize(d.expected_len(), 0xE5);
        d.data = data;
        d
    }

    /// Size in bytes implied by the geometry.
    pub fn expected_len(&self) -> usize {
        self.tracks as usize * self.sides as usize * self.sectors_per_track as usize * self.sector_size
    }

    /// Byte offset of a sector (`sector` is the ID number, from `first_sector`).
    pub fn offset(&self, track: u8, side: u8, sector: u8) -> Option<usize> {
        if track >= self.tracks || side >= self.sides || sector < self.first_sector {
            return None;
        }
        let idx = (sector - self.first_sector) as usize;
        if idx >= self.sectors_per_track as usize {
            return None;
        }
        let lin = (track as usize * self.sides as usize + side as usize) * self.sectors_per_track as usize + idx;
        let off = lin * self.sector_size;
        (off + self.sector_size <= self.data.len()).then_some(off)
    }

    /// Size code of the ID field (0 = 128, 1 = 256, 2 = 512, 3 = 1024).
    pub fn size_code(&self) -> u8 {
        match self.sector_size {
            0..=128 => 0,
            129..=256 => 1,
            257..=512 => 2,
            _ => 3,
        }
    }
}

/// Timing parameters, in microseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Wd1770Timing {
    /// Step rates selected by command bits 0-1.
    pub step_us: [u32; 4],
    /// Head settle (type I verify, type II/III with E flag).
    pub settle_us: u32,
    /// Motor spin-up when the h flag is 0 and the motor was off.
    pub spinup_us: u32,
    /// Latency to find a sector / ID field.
    pub search_us: u32,
    /// One byte on the disk.
    pub byte_us: u32,
    /// One revolution (index pulse period).
    pub revolution_us: u32,
    /// Index pulse width.
    pub index_pulse_us: u32,
    /// Revolutions without a command before the motor turns off.
    pub motor_off_revs: u32,
    /// Time before "record not found" / verify seek error.
    pub rnf_us: u32,
    /// Delay between the command write and the start of its execution
    /// (keeps BUSY observable even for commands with nothing to do).
    pub start_us: u32,
}

impl Default for Wd1770Timing {
    fn default() -> Self {
        Self {
            step_us: [6000, 12000, 20000, 30000],
            settle_us: 30000,
            spinup_us: 100_000,
            search_us: 2000,
            byte_us: 32,
            revolution_us: 200_000,
            index_pulse_us: 2000,
            motor_off_revs: 9,
            rnf_us: 1_000_000,
            start_us: 100,
        }
    }
}

impl Wd1770Timing {
    /// Very short delays (useful for tests / turbo mode). Byte time stays
    /// realistic so that polled transfers keep working.
    pub fn fast() -> Self {
        Self {
            step_us: [100, 100, 100, 100],
            settle_us: 100,
            spinup_us: 100,
            search_us: 100,
            byte_us: 32,
            revolution_us: 200_000,
            index_pulse_us: 2000,
            motor_off_revs: 9,
            rnf_us: 1000,
            start_us: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    Idle,
    Start,
    SpinUp,
    TypeIStep,
    TypeIVerify,
    VerifyFail,
    Settle,
    Search,
    Rnf,
    ReadByte,
    WriteFirst,
    WriteByte,
}

#[derive(Debug, Clone)]
pub struct Wd1770 {
    cpu_clock_hz: u32,
    timing: Wd1770Timing,
    drives: Vec<Option<DiskImage>>,
    phys: [u8; MAX_DRIVES],
    sel: Option<usize>,
    side: u8,
    // Registers
    cmd: u8,
    cmd_type: u8,
    tr: u8,
    sr: u8,
    dr: u8,
    // Status flags
    busy: bool,
    drq: bool,
    intrq: bool,
    lost: bool,
    crc_err: bool,
    rnf: bool,
    seek_err: bool,
    wp_err: bool,
    spun_up: bool,
    motor_on: bool,
    // Engine
    st: St,
    timer: i64,
    direction_in: bool,
    steps: u32,
    buf: Vec<u8>,
    pos: usize,
    rot: u64,
    idle: u64,
    int_on_index: bool,
    id_counter: u8,
}

impl Wd1770 {
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self {
            cpu_clock_hz: cpu_clock_hz.max(1),
            timing: Wd1770Timing::default(),
            drives: vec![None; MAX_DRIVES],
            phys: [0; MAX_DRIVES],
            sel: None,
            side: 0,
            cmd: 0,
            cmd_type: 1,
            tr: 0,
            sr: 1,
            dr: 0,
            busy: false,
            drq: false,
            intrq: false,
            lost: false,
            crc_err: false,
            rnf: false,
            seek_err: false,
            wp_err: false,
            spun_up: false,
            motor_on: false,
            st: St::Idle,
            timer: 0,
            direction_in: true,
            steps: 0,
            buf: Vec::new(),
            pos: 0,
            rot: 0,
            idle: 0,
            int_on_index: false,
            id_counter: 0,
        }
    }

    pub fn set_timing(&mut self, timing: Wd1770Timing) {
        self.timing = timing;
    }

    pub fn timing(&self) -> &Wd1770Timing {
        &self.timing
    }

    /// Master reset (disks and head positions are kept).
    pub fn reset(&mut self) {
        self.cmd = 0;
        self.cmd_type = 1;
        self.tr = 0;
        self.sr = 1;
        self.dr = 0;
        self.busy = false;
        self.drq = false;
        self.intrq = false;
        self.lost = false;
        self.crc_err = false;
        self.rnf = false;
        self.seek_err = false;
        self.wp_err = false;
        self.spun_up = false;
        self.motor_on = false;
        self.st = St::Idle;
        self.timer = 0;
        self.int_on_index = false;
        self.buf.clear();
    }

    pub fn write(&mut self, reg: u8, v: u8) {
        match reg & 3 {
            0 => self.command(v),
            1 => self.tr = v,
            2 => self.sr = v,
            _ => {
                self.dr = v;
                self.drq = false;
            }
        }
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 3 {
            0 => {
                self.intrq = false;
                self.status()
            }
            1 => self.tr,
            2 => self.sr,
            _ => {
                self.drq = false;
                self.dr
            }
        }
    }

    /// Status register value without side effects (INTRQ is not cleared).
    pub fn status(&self) -> u8 {
        let mut s = 0u8;
        if self.busy {
            s |= 0x01;
        }
        if self.crc_err {
            s |= 0x08;
        }
        if self.motor_on {
            s |= 0x80;
        }
        if self.cmd_type == 1 {
            if self.index() {
                s |= 0x02;
            }
            if self.track0() {
                s |= 0x04;
            }
            if self.seek_err {
                s |= 0x10;
            }
            if self.spun_up {
                s |= 0x20;
            }
            if self.disk_cur().is_some_and(|d| d.write_protected) {
                s |= 0x40;
            }
        } else {
            if self.drq {
                s |= 0x02;
            }
            if self.lost {
                s |= 0x04;
            }
            if self.rnf {
                s |= 0x10;
            }
            if self.wp_err {
                s |= 0x40;
            }
        }
        s
    }

    pub fn tick(&mut self, cpu_cycles: u32) {
        let cycles = cpu_cycles as u64;
        // Rotation / index pulses.
        if self.motor_on {
            let rev = self.us(self.timing.revolution_us).max(1) as u64;
            let new = self.rot + cycles;
            if new >= rev && self.int_on_index && self.disk_cur().is_some() {
                self.intrq = true;
            }
            self.rot = new % rev;
            if !self.busy {
                self.idle += cycles;
                if self.idle >= rev * self.timing.motor_off_revs as u64 {
                    self.motor_on = false;
                    self.spun_up = false;
                }
            }
        }
        if self.st == St::Idle {
            return;
        }
        self.timer -= cycles as i64;
        while self.st != St::Idle && self.timer <= 0 {
            self.event();
        }
    }

    pub fn intrq(&self) -> bool {
        self.intrq
    }

    pub fn drq(&self) -> bool {
        self.drq
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Select drive (None = no drive selected) and side.
    pub fn select(&mut self, drive: Option<usize>, side: u8) {
        self.sel = drive.filter(|&d| d < MAX_DRIVES);
        self.side = side & 1;
    }

    pub fn insert(&mut self, drive: usize, disk: Option<DiskImage>) {
        if drive < MAX_DRIVES {
            self.drives[drive] = disk;
        }
    }

    pub fn disk(&self, drive: usize) -> Option<&DiskImage> {
        self.drives.get(drive)?.as_ref()
    }

    pub fn disk_mut(&mut self, drive: usize) -> Option<&mut DiskImage> {
        self.drives.get_mut(drive)?.as_mut()
    }

    /// Physical head position of a drive.
    pub fn head_track(&self, drive: usize) -> u8 {
        self.phys.get(drive).copied().unwrap_or(0)
    }

    pub fn motor_on(&self) -> bool {
        self.motor_on
    }

    // ---- internals ----

    fn us(&self, us: u32) -> i64 {
        (us as u64 * self.cpu_clock_hz as u64 / 1_000_000) as i64
    }

    fn disk_cur(&self) -> Option<&DiskImage> {
        self.disk(self.sel?)
    }

    fn cur_phys(&self) -> u8 {
        self.sel.map_or(0, |d| self.phys[d])
    }

    fn index(&self) -> bool {
        self.motor_on
            && self.disk_cur().is_some()
            && (self.rot as i64) < self.us(self.timing.index_pulse_us)
    }

    fn track0(&self) -> bool {
        self.sel.is_some_and(|d| self.phys[d] == 0)
    }

    /// The current physical track/side is readable (disk present, inside geometry).
    fn track_readable(&self) -> bool {
        self.disk_cur()
            .is_some_and(|d| self.cur_phys() < d.tracks && self.side < d.sides)
    }

    fn sector_offset(&self) -> Option<usize> {
        if self.tr != self.cur_phys() {
            return None;
        }
        self.disk_cur()?.offset(self.cur_phys(), self.side, self.sr)
    }

    fn set_timer(&mut self, st: St, cycles: i64) {
        self.st = st;
        self.timer += cycles;
    }

    fn finish(&mut self) {
        self.busy = false;
        self.intrq = true;
        self.st = St::Idle;
        self.timer = 0;
        self.idle = 0;
    }

    fn command(&mut self, v: u8) {
        if v & 0xF0 == 0xD0 {
            self.force_interrupt(v);
            return;
        }
        if self.busy {
            return;
        }
        self.cmd = v;
        self.intrq = false;
        self.busy = true;
        self.timer = 0;
        self.idle = 0;
        if v & 0x80 == 0 {
            self.cmd_type = 1;
            self.crc_err = false;
            self.seek_err = false;
            self.drq = false;
        } else {
            self.cmd_type = if v & 0x40 == 0 { 2 } else { 3 };
            self.drq = false;
            self.lost = false;
            self.crc_err = false;
            self.rnf = false;
            self.wp_err = false;
        }
        let was_on = self.motor_on;
        self.motor_on = true;
        if v & 0x08 == 0 && !was_on {
            self.spun_up = false;
            let t = self.us(self.timing.spinup_us);
            self.set_timer(St::SpinUp, t);
        } else {
            if v & 0x08 != 0 {
                self.spun_up = true;
            }
            let t = self.us(self.timing.start_us);
            self.set_timer(St::Start, t);
        }
    }

    fn force_interrupt(&mut self, v: u8) {
        if self.busy {
            self.busy = false;
            self.st = St::Idle;
            self.timer = 0;
            self.idle = 0;
        } else {
            self.cmd_type = 1;
        }
        self.drq = false;
        self.int_on_index = v & 0x04 != 0;
        self.intrq = v & 0x08 != 0;
    }

    /// Start the command proper (after spin-up).
    fn begin(&mut self) {
        let v = self.cmd;
        if self.cmd_type == 1 {
            self.steps = 0;
            match v >> 5 {
                0 => {
                    if v & 0x10 == 0 {
                        // Restore
                        self.tr = 0xFF;
                        self.dr = 0;
                    }
                }
                1 => {} // step: keep direction
                2 => self.direction_in = true,
                _ => self.direction_in = false,
            }
            self.set_timer(St::TypeIStep, 0);
        } else if v & 0x04 != 0 {
            let t = self.us(self.timing.settle_us);
            self.set_timer(St::Settle, t);
        } else {
            self.start_search();
        }
    }

    fn step_head(&mut self) {
        if let Some(d) = self.sel {
            let p = &mut self.phys[d];
            if self.direction_in {
                *p = (*p + 1).min(MAX_PHYS_TRACK);
            } else {
                *p = p.saturating_sub(1);
            }
        }
    }

    fn type1_step(&mut self) {
        let v = self.cmd;
        let rate = self.us(self.timing.step_us[(v & 3) as usize]);
        match v >> 5 {
            0 if v & 0x10 == 0 => {
                // Restore: step out until the track 0 sensor, at most 255 steps.
                if self.track0() {
                    self.tr = 0;
                    return self.type1_verify();
                }
                if self.steps >= 255 {
                    self.seek_err = true;
                    return self.finish();
                }
                self.direction_in = false;
                self.step_head();
                self.steps += 1;
                self.set_timer(St::TypeIStep, rate);
            }
            0 => {
                // Seek to the track in the data register.
                if self.tr == self.dr {
                    return self.type1_verify();
                }
                self.direction_in = self.dr > self.tr;
                self.tr = if self.direction_in { self.tr.wrapping_add(1) } else { self.tr.wrapping_sub(1) };
                self.step_head();
                self.set_timer(St::TypeIStep, rate);
            }
            _ => {
                if self.steps == 0 {
                    if v & 0x10 != 0 {
                        self.tr = if self.direction_in { self.tr.wrapping_add(1) } else { self.tr.wrapping_sub(1) };
                    }
                    self.step_head();
                    self.steps = 1;
                    self.set_timer(St::TypeIStep, rate);
                } else {
                    self.type1_verify();
                }
            }
        }
    }

    fn type1_verify(&mut self) {
        if self.cmd & 0x04 != 0 {
            let t = self.us(self.timing.settle_us);
            self.set_timer(St::TypeIVerify, t);
        } else {
            self.finish();
        }
    }

    fn start_search(&mut self) {
        let write = matches!(self.cmd >> 4, 0xA | 0xB | 0xF);
        if write && self.disk_cur().is_some_and(|d| d.write_protected) {
            self.wp_err = true;
            return self.finish();
        }
        let t = self.us(self.timing.search_us);
        self.set_timer(St::Search, t);
    }

    fn rnf_wait(&mut self) {
        let t = (self.us(self.timing.rnf_us) - self.us(self.timing.search_us)).max(1);
        self.set_timer(St::Rnf, t);
    }

    fn search(&mut self) {
        match self.cmd >> 4 {
            0x8 | 0x9 => match self.sector_offset() {
                Some(off) => {
                    let d = self.disk_cur().unwrap();
                    self.buf = d.data[off..off + d.sector_size].to_vec();
                    self.pos = 0;
                    self.set_timer(St::ReadByte, 0);
                }
                None => self.rnf_wait(),
            },
            0xA | 0xB => match self.sector_offset() {
                Some(_) => {
                    let n = self.disk_cur().unwrap().sector_size;
                    self.buf = vec![0; n];
                    self.pos = 0;
                    self.drq = true;
                    let t = self.us(self.timing.byte_us) * 11;
                    self.set_timer(St::WriteFirst, t);
                }
                None => self.rnf_wait(),
            },
            0xC => {
                if self.track_readable() {
                    let d = self.disk_cur().unwrap();
                    let spt = d.sectors_per_track.max(1);
                    let sector = d.first_sector.wrapping_add(self.id_counter % spt);
                    let size_code = d.size_code();
                    self.id_counter = self.id_counter.wrapping_add(1) % spt;
                    let id = [self.cur_phys(), self.side, sector, size_code];
                    self.buf = id_with_crc(id).to_vec();
                    self.pos = 0;
                    self.set_timer(St::ReadByte, 0);
                } else {
                    self.rnf_wait();
                }
            }
            0xE => {
                if self.track_readable() {
                    self.buf = self.build_track();
                    self.pos = 0;
                    self.set_timer(St::ReadByte, 0);
                } else {
                    // No index pulses: nothing to read. Give up after the timeout.
                    self.rnf_wait();
                }
            }
            _ => {
                // Write track
                if self.disk_cur().is_some() {
                    self.buf = vec![0; TRACK_LEN];
                    self.pos = 0;
                    self.drq = true;
                    let t = self.us(self.timing.byte_us) * 3;
                    self.set_timer(St::WriteFirst, t);
                } else {
                    self.rnf_wait();
                }
            }
        }
    }

    fn event(&mut self) {
        let byte = self.us(self.timing.byte_us).max(1);
        match self.st {
            St::Idle => {}
            St::Start => self.begin(),
            St::SpinUp => {
                self.spun_up = true;
                self.begin();
            }
            St::TypeIStep => self.type1_step(),
            St::TypeIVerify => {
                if self.track_readable() && self.tr == self.cur_phys() {
                    self.finish();
                } else {
                    let t = self.us(self.timing.rnf_us).max(1);
                    self.set_timer(St::VerifyFail, t);
                }
            }
            St::VerifyFail => {
                self.seek_err = true;
                self.finish();
            }
            St::Settle => self.start_search(),
            St::Search => self.search(),
            St::Rnf => {
                // Write track / read track without disk: no RNF bit on the
                // real chip, but nothing else is sensible either.
                self.rnf = true;
                self.finish();
            }
            St::ReadByte => {
                if self.pos < self.buf.len() {
                    if self.drq {
                        self.lost = true;
                    }
                    self.dr = self.buf[self.pos];
                    self.pos += 1;
                    self.drq = true;
                    // After the last byte: CRC (2 bytes) before completion.
                    let t = if self.pos == self.buf.len() { byte * 2 } else { byte };
                    self.set_timer(St::ReadByte, t);
                } else {
                    match self.cmd >> 4 {
                        0x9 => {
                            self.sr = self.sr.wrapping_add(1);
                            let t = self.us(self.timing.search_us);
                            self.set_timer(St::Search, t);
                        }
                        0xC => {
                            self.sr = self.buf[0];
                            self.finish();
                        }
                        _ => self.finish(),
                    }
                }
            }
            St::WriteFirst => {
                if self.drq {
                    self.lost = true;
                    self.drq = false;
                    return self.finish();
                }
                self.buf[0] = self.dr;
                self.pos = 1;
                if self.pos < self.buf.len() {
                    self.drq = true;
                }
                self.set_timer(St::WriteByte, byte);
            }
            St::WriteByte => {
                if self.pos < self.buf.len() {
                    let b = if self.drq {
                        self.lost = true;
                        self.drq = false;
                        0
                    } else {
                        self.dr
                    };
                    self.buf[self.pos] = b;
                    self.pos += 1;
                    if self.pos < self.buf.len() {
                        self.drq = true;
                        self.set_timer(St::WriteByte, byte);
                    } else {
                        self.set_timer(St::WriteByte, byte * 2);
                    }
                } else {
                    self.commit_write();
                }
            }
        }
    }

    fn commit_write(&mut self) {
        let side = self.side;
        let phys = self.cur_phys();
        if self.cmd >> 4 == 0xF {
            let buf = std::mem::take(&mut self.buf);
            if let Some(d) = self.sel.and_then(|s| self.drives[s].as_mut()) {
                format_track(d, phys, side, &buf);
            }
            return self.finish();
        }
        if let Some(off) = self.sector_offset() {
            let buf = std::mem::take(&mut self.buf);
            if let Some(d) = self.sel.and_then(|s| self.drives[s].as_mut()) {
                d.data[off..off + buf.len()].copy_from_slice(&buf);
            }
        }
        if self.cmd & 0x10 != 0 {
            self.sr = self.sr.wrapping_add(1);
            let t = self.us(self.timing.search_us);
            self.set_timer(St::Search, t);
        } else {
            self.finish();
        }
    }

    /// Synthesise the raw MFM track as returned by "read track".
    fn build_track(&self) -> Vec<u8> {
        let d = self.disk_cur().unwrap();
        let phys = self.cur_phys();
        let mut t = Vec::with_capacity(TRACK_LEN);
        t.extend(std::iter::repeat_n(0x4E, 80));
        t.extend(std::iter::repeat_n(0x00, 12));
        t.extend([0xC2, 0xC2, 0xC2, 0xFC]);
        t.extend(std::iter::repeat_n(0x4E, 50));
        for i in 0..d.sectors_per_track {
            let sec = d.first_sector.wrapping_add(i);
            t.extend(std::iter::repeat_n(0x00, 12));
            t.extend([0xA1, 0xA1, 0xA1, 0xFE]);
            t.extend(id_with_crc([phys, self.side, sec, d.size_code()]));
            t.extend(std::iter::repeat_n(0x4E, 22));
            t.extend(std::iter::repeat_n(0x00, 12));
            t.extend([0xA1, 0xA1, 0xA1, 0xFB]);
            let off = d.offset(phys, self.side, sec).unwrap_or(0);
            let data = &d.data[off..off + d.sector_size];
            t.extend_from_slice(data);
            let mut crc = crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, 0xFB]);
            crc = crc16(crc, data);
            t.extend([(crc >> 8) as u8, crc as u8]);
            t.extend(std::iter::repeat_n(0x4E, 54));
        }
        t.resize(TRACK_LEN.max(t.len()), 0x4E);
        t
    }
}

/// CRC-16-CCITT (poly 0x1021) as used by the WD177x.
pub fn crc16(mut crc: u16, data: &[u8]) -> u16 {
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// ID field bytes as returned by "read address": track, side, sector, size, CRC.
fn id_with_crc(id: [u8; 4]) -> [u8; 6] {
    let crc = crc16(crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, 0xFE]), &id);
    [id[0], id[1], id[2], id[3], (crc >> 8) as u8, crc as u8]
}

/// Interpret a "write track" byte stream: store data fields of IDs that fit
/// the image geometry (sector size must match). Other content is ignored.
fn format_track(d: &mut DiskImage, phys: u8, side: u8, raw: &[u8]) {
    let mut i = 0;
    let mut id: Option<[u8; 4]> = None;
    while i < raw.len() {
        match raw[i] {
            0xFE if i + 4 < raw.len() => {
                id = Some([raw[i + 1], raw[i + 2], raw[i + 3], raw[i + 4]]);
                i += 5;
            }
            0xFB | 0xF8 if id.is_some() => {
                let [_, _, sec, code] = id.take().unwrap();
                let size = 128usize << (code & 3);
                let end = (i + 1 + size).min(raw.len());
                let data = &raw[i + 1..end];
                if size == d.sector_size && data.len() == size {
                    if let Some(off) = d.offset(phys, side, sec) {
                        d.data[off..off + size].copy_from_slice(data);
                    }
                }
                i = end;
            }
            _ => i += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU: u32 = 4_000_000;

    fn pattern_disk() -> DiskImage {
        // 40 tracks, 2 sides, 5 x 1024 B sectors numbered from 1.
        let mut d = DiskImage::blank(40, 2, 5, 1024, 1);
        for (i, b) in d.data.iter_mut().enumerate() {
            *b = (i / 1024) as u8 ^ (i as u8);
        }
        d
    }

    fn sector_bytes(d: &DiskImage, t: u8, s: u8, r: u8) -> Vec<u8> {
        let o = d.offset(t, s, r).unwrap();
        d.data[o..o + d.sector_size].to_vec()
    }

    fn fdc(timing: Wd1770Timing) -> Wd1770 {
        let mut f = Wd1770::new(CPU);
        f.set_timing(timing);
        f.insert(0, Some(pattern_disk()));
        f.select(Some(0), 0);
        f
    }

    /// Tick until not busy; returns cycles spent.
    fn wait_idle(f: &mut Wd1770) -> u64 {
        let mut c = 0u64;
        while f.status() & 1 != 0 {
            f.tick(20);
            c += 20;
            assert!(c < 100_000_000, "command never finished");
        }
        c
    }

    /// Polled read loop like a BIOS: returns data and final status.
    fn polled_read(f: &mut Wd1770, cmd: u8) -> (Vec<u8>, u8) {
        f.write(0, cmd);
        let mut out = Vec::new();
        let mut n = 0u64;
        loop {
            let s = f.read(0);
            if s & 0x02 != 0 {
                out.push(f.read(3));
            } else if s & 0x01 == 0 {
                break;
            }
            f.tick(24);
            n += 1;
            assert!(n < 10_000_000);
        }
        (out, f.read(0))
    }

    fn polled_write(f: &mut Wd1770, cmd: u8, data: &[u8]) -> u8 {
        f.write(0, cmd);
        let mut i = 0;
        let mut n = 0u64;
        loop {
            let s = f.read(0);
            if s & 0x02 != 0 {
                f.write(3, data.get(i).copied().unwrap_or(0));
                i += 1;
            } else if s & 0x01 == 0 {
                break;
            }
            f.tick(24);
            n += 1;
            assert!(n < 10_000_000);
        }
        f.read(0)
    }

    fn seek(f: &mut Wd1770, track: u8) -> u8 {
        f.write(3, track);
        f.write(0, 0x14); // seek, verify, 6 ms
        wait_idle(f);
        f.read(0)
    }

    #[test]
    fn restore_with_verify() {
        let mut f = fdc(Wd1770Timing::default());
        f.phys[0] = 5;
        f.write(1, 5);
        f.write(0, 0x04); // restore, spin-up, verify, 6 ms
        assert_eq!(f.status() & 0x81, 0x81);
        assert!(!f.intrq());
        let c = wait_idle(&mut f);
        // spin-up 100 ms + 5 steps x 6 ms + settle 30 ms = 160 ms
        assert!((600_000..700_000).contains(&c), "{c}");
        assert!(f.intrq());
        assert_eq!(f.read(1), 0);
        assert_eq!(f.head_track(0), 0);
        let s = f.read(0);
        assert!(!f.intrq());
        assert_eq!(s & 0x01, 0);
        assert_eq!(s & 0x04, 0x04, "track 0");
        assert_eq!(s & 0x10, 0, "no seek error");
        assert_eq!(s & 0x20, 0x20, "spun up");
        assert_eq!(s & 0x80, 0x80, "motor on");
    }

    #[test]
    fn busy_observable_even_when_nothing_to_do() {
        let mut f = fdc(Wd1770Timing::default());
        f.write(0, 0x08); // restore at track 0, no spin-up, no verify
        let mut busy = 0;
        while f.read(0) & 1 != 0 {
            f.tick(10);
            busy += 10;
        }
        assert!(busy >= 300, "{busy}");
    }

    #[test]
    fn restore_without_drive_seek_error() {
        let mut f = fdc(Wd1770Timing::fast());
        f.select(None, 0);
        f.write(0, 0x00);
        wait_idle(&mut f);
        let s = f.read(0);
        assert_eq!(s & 0x10, 0x10);
        assert_eq!(s & 0x04, 0);
        assert_eq!(s & 0x02, 0, "no index without disk");
    }

    #[test]
    fn seek_and_verify() {
        let mut f = fdc(Wd1770Timing::fast());
        assert_eq!(seek(&mut f, 10) & 0x10, 0);
        assert_eq!(f.read(1), 10);
        assert_eq!(f.head_track(0), 10);
        assert_eq!(seek(&mut f, 3) & 0x14, 0);
        assert_eq!(f.head_track(0), 3);
        // Beyond the disk: head moves but verify fails.
        assert_eq!(seek(&mut f, 45) & 0x10, 0x10);
        assert_eq!(f.head_track(0), 45);
        // Mismatched track register: verify fails.
        let mut f2 = fdc(Wd1770Timing::fast());
        f2.write(1, 2);
        f2.write(3, 2);
        f2.write(0, 0x14);
        wait_idle(&mut f2);
        assert_eq!(f2.read(0) & 0x10, 0x10);
    }

    #[test]
    fn seek_step_timing() {
        let mut f = fdc(Wd1770Timing::default());
        f.write(3, 4);
        f.write(0, 0x1B); // seek, no spin-up, no verify, 30 ms
        let c = wait_idle(&mut f);
        let expected = 4 * 30 * 4000 + 400;
        assert!((c as i64 - expected as i64).abs() < 100, "{c}");
    }

    #[test]
    fn step_commands() {
        let mut f = fdc(Wd1770Timing::fast());
        f.write(0, 0x58); // step in, update, no spin-up
        wait_idle(&mut f);
        assert_eq!((f.read(1), f.head_track(0)), (1, 1));
        f.write(0, 0x48); // step in, no update
        wait_idle(&mut f);
        assert_eq!((f.read(1), f.head_track(0)), (1, 2));
        f.write(0, 0x38); // step (same direction), update
        wait_idle(&mut f);
        assert_eq!((f.read(1), f.head_track(0)), (2, 3));
        f.write(0, 0x78); // step out, update
        wait_idle(&mut f);
        assert_eq!((f.read(1), f.head_track(0)), (1, 2));
        f.write(0, 0x3C); // step out again with verify: tr 0, phys 1 -> seek error
        wait_idle(&mut f);
        assert_eq!((f.read(1), f.head_track(0)), (0, 1));
        assert_eq!(f.read(0) & 0x10, 0x10);
    }

    #[test]
    fn read_sector_polled() {
        let mut f = fdc(Wd1770Timing::default());
        seek(&mut f, 7);
        f.write(2, 3);
        let (data, s) = polled_read(&mut f, 0x88);
        assert_eq!(s & 0x5F, 0, "status {s:#x}");
        assert_eq!(data, sector_bytes(f.disk(0).unwrap(), 7, 0, 3));
        assert!(f.intrq() || s & 1 == 0);
        // side 1
        f.select(Some(0), 1);
        f.write(2, 5);
        let (data, s) = polled_read(&mut f, 0x88);
        assert_eq!(s & 0x5F, 0);
        assert_eq!(data, sector_bytes(f.disk(0).unwrap(), 7, 1, 5));
    }

    #[test]
    fn read_sector_with_settle_takes_longer() {
        let mut f = fdc(Wd1770Timing::default());
        f.write(2, 1);
        f.write(0, 0x8C); // h=1, E=1
        let mut c = 0;
        while f.status() & 2 == 0 {
            f.tick(10);
            c += 10;
        }
        // start 0.1 + settle 30 + search 2 ms
        assert!((128_000..129_000).contains(&c), "{c}");
    }

    #[test]
    fn read_sector_lost_data() {
        let mut f = fdc(Wd1770Timing::fast());
        f.write(2, 1);
        f.write(0, 0x88);
        wait_idle(&mut f);
        let s = f.read(0);
        assert_eq!(s & 0x04, 0x04);
        assert_eq!(s & 0x02, 0x02, "last byte still pending");
        f.read(3);
        assert_eq!(f.read(0) & 0x02, 0);
    }

    #[test]
    fn read_multiple_until_rnf() {
        let mut f = fdc(Wd1770Timing::fast());
        seek(&mut f, 2);
        f.write(2, 2);
        let (data, s) = polled_read(&mut f, 0x98);
        assert_eq!(s & 0x10, 0x10, "ends with RNF");
        assert_eq!(s & 0x04, 0);
        assert_eq!(data.len(), 4 * 1024);
        let d = f.disk(0).unwrap();
        let o = d.offset(2, 0, 2).unwrap();
        assert_eq!(data, d.data[o..o + 4096]);
        assert_eq!(f.read(2), 6);
    }

    #[test]
    fn rnf_cases() {
        let mut f = fdc(Wd1770Timing::default());
        f.write(2, 9);
        f.write(0, 0x88);
        let c = wait_idle(&mut f);
        assert!(c >= 4_000_000, "RNF only after timeout ({c})");
        assert_eq!(f.read(0) & 0x10, 0x10);
        // track register mismatch
        let mut f = fdc(Wd1770Timing::fast());
        f.write(1, 3);
        f.write(2, 1);
        let (d, s) = polled_read(&mut f, 0x88);
        assert!(d.is_empty());
        assert_eq!(s & 0x10, 0x10);
        // no disk
        let mut f = fdc(Wd1770Timing::fast());
        f.insert(0, None);
        f.write(2, 1);
        let (_, s) = polled_read(&mut f, 0x80);
        assert_eq!(s & 0x10, 0x10);
        // sector 0 (below first_sector)
        let mut f = fdc(Wd1770Timing::fast());
        f.write(2, 0);
        let (_, s) = polled_read(&mut f, 0x88);
        assert_eq!(s & 0x10, 0x10);
    }

    #[test]
    fn write_sector_polled() {
        let mut f = fdc(Wd1770Timing::fast());
        seek(&mut f, 12);
        f.select(Some(0), 1);
        f.write(2, 4);
        let data: Vec<u8> = (0..1024).map(|i| (i * 7) as u8).collect();
        let s = polled_write(&mut f, 0xA8, &data);
        assert_eq!(s & 0x5F, 0, "{s:#x}");
        assert_eq!(sector_bytes(f.disk(0).unwrap(), 12, 1, 4), data);
        assert_eq!(sector_bytes(f.disk(0).unwrap(), 12, 1, 3), sector_bytes(&pattern_disk(), 12, 1, 3));
        let (back, _) = polled_read(&mut f, 0x88);
        assert_eq!(back, data);
    }

    #[test]
    fn write_multiple() {
        let mut f = fdc(Wd1770Timing::fast());
        f.write(2, 4);
        let data = vec![0x5A; 2048];
        let s = polled_write(&mut f, 0xB8, &data);
        assert_eq!(s & 0x10, 0x10);
        assert_eq!(sector_bytes(f.disk(0).unwrap(), 0, 0, 4), vec![0x5A; 1024]);
        assert_eq!(sector_bytes(f.disk(0).unwrap(), 0, 0, 5), vec![0x5A; 1024]);
    }

    #[test]
    fn write_protect_and_lost_data() {
        let mut f = fdc(Wd1770Timing::fast());
        f.disk_mut(0).unwrap().write_protected = true;
        assert_eq!(f.read(0) & 0x40, 0x40, "type I shows WP");
        f.write(2, 1);
        let s = polled_write(&mut f, 0xA8, &[1; 1024]);
        assert_eq!(s & 0x40, 0x40);
        assert_eq!(f.disk(0).unwrap().data, pattern_disk().data);
        // lost data on first byte: command terminates, nothing written
        f.disk_mut(0).unwrap().write_protected = false;
        f.write(0, 0xA8);
        wait_idle(&mut f);
        let s = f.read(0);
        assert_eq!(s & 0x44, 0x04);
        assert_eq!(f.disk(0).unwrap().data, pattern_disk().data);
    }

    #[test]
    fn read_address() {
        let mut f = fdc(Wd1770Timing::fast());
        seek(&mut f, 9);
        f.select(Some(0), 1);
        let (id, s) = polled_read(&mut f, 0xC8);
        assert_eq!(s & 0x5F, 0);
        assert_eq!(id.len(), 6);
        assert_eq!(&id[..4], &[9, 1, 1, 3]);
        let crc = crc16(0xFFFF, &[0xA1, 0xA1, 0xA1, 0xFE, 9, 1, 1, 3]);
        assert_eq!(crc16(crc, &id[4..]), 0, "CRC checks");
        assert_eq!(f.read(2), 9, "sector register := track");
        let (id, _) = polled_read(&mut f, 0xC8);
        assert_eq!(id[2], 2, "next sector passes the head");
        f.insert(0, None);
        let (_, s) = polled_read(&mut f, 0xC8);
        assert_eq!(s & 0x10, 0x10);
    }

    #[test]
    fn force_interrupt() {
        let mut f = fdc(Wd1770Timing::fast());
        f.write(2, 1);
        f.write(0, 0x88);
        f.tick(2000);
        assert!(f.drq());
        f.write(0, 0xD0);
        assert_eq!(f.status() & 0x03, 0);
        assert!(!f.intrq());
        f.write(0, 0xD0); // when idle: status switches to type I
        assert_eq!(f.status() & 0x04, 0x04, "track 0 bit of type I status");
        f.tick(100_000);
        assert!(!f.drq());
        // Immediate interrupt
        f.write(0, 0xD8);
        assert!(f.intrq());
        f.read(0);
        assert!(!f.intrq());
        f.write(0, 0xD0);
        // Commands are ignored while busy, except force interrupt.
        f.write(3, 5);
        f.write(0, 0x18);
        f.write(0, 0x88);
        wait_idle(&mut f);
        assert_eq!(f.head_track(0), 5);
        // Index interrupt
        f.write(0, 0xD4);
        assert!(!f.intrq());
        f.tick(800_000);
        assert!(f.intrq());
        f.write(0, 0xD0);
        f.read(0);
        f.tick(800_000);
        assert!(!f.intrq());
    }

    #[test]
    fn index_pulses_and_motor_off() {
        let mut f = fdc(Wd1770Timing::default());
        f.write(0, 0x08);
        wait_idle(&mut f);
        let mut seen_on = 0;
        let mut seen_off = 0;
        for _ in 0..1000 {
            f.tick(800); // 200 µs
            if f.status() & 2 != 0 {
                seen_on += 1;
            } else {
                seen_off += 1;
            }
        }
        // 200 ms total = 1 revolution: ~10 samples in the 2 ms pulse
        assert!((9..=11).contains(&seen_on), "{seen_on}");
        assert!(seen_off > 900);
        assert!(f.motor_on());
        f.tick(8 * 800_000);
        assert!(!f.motor_on());
        assert_eq!(f.status() & 0xA2, 0);
        // with h=0 the next command spins up again
        f.write(0, 0x00);
        assert!(f.status() & 0x80 != 0);
        assert_eq!(f.status() & 0x20, 0);
        wait_idle(&mut f);
        assert_eq!(f.status() & 0x20, 0x20);
    }

    #[test]
    fn write_track_format_and_read_track() {
        let mut f = fdc(Wd1770Timing::fast());
        seek(&mut f, 1);
        // Build a format stream: per sector gap, sync, ID, CRC, gap, data.
        let mut raw = vec![0x4E; 60];
        for r in 1..=5u8 {
            raw.extend([0x00; 12]);
            raw.extend([0xF5, 0xF5, 0xF5, 0xFE, 1, 0, r, 3, 0xF7]);
            raw.extend([0x4E; 22]);
            raw.extend([0x00; 12]);
            raw.extend([0xF5, 0xF5, 0xF5, 0xFB]);
            raw.extend(std::iter::repeat_n(0x10 + r, 1024));
            raw.push(0xF7);
            raw.extend([0x4E; 40]);
        }
        let s = polled_write(&mut f, 0xF8, &raw);
        assert_eq!(s & 0x5F, 0, "{s:#x}");
        for r in 1..=5u8 {
            assert_eq!(sector_bytes(f.disk(0).unwrap(), 1, 0, r), vec![0x10 + r; 1024]);
        }
        assert_eq!(sector_bytes(f.disk(0).unwrap(), 1, 1, 1), sector_bytes(&pattern_disk(), 1, 1, 1));
        let (track, s) = polled_read(&mut f, 0xE8);
        assert_eq!(s & 0x04, 0);
        assert!(track.len() >= TRACK_LEN);
        let pos = track.windows(4).position(|w| w == [0xA1, 0xA1, 0xA1, 0xFB]).unwrap();
        assert_eq!(&track[pos + 4..pos + 4 + 1024], &vec![0x11; 1024][..]);
    }

    #[test]
    fn blank_and_from_bytes() {
        let d = DiskImage::blank(40, 1, 10, 512, 1);
        assert_eq!(d.data.len(), 40 * 10 * 512);
        assert_eq!(d.size_code(), 2);
        let d = DiskImage::from_bytes(2, 1, 2, 128, 0, vec![1, 2, 3]);
        assert_eq!(d.data.len(), 512);
        assert_eq!(&d.data[..4], &[1, 2, 3, 0xE5]);
        assert_eq!(d.offset(1, 0, 1), Some(384));
        assert_eq!(d.offset(1, 0, 2), None);
        assert_eq!(d.offset(2, 0, 0), None);
        assert_eq!(d.offset(0, 1, 0), None);
    }

    #[test]
    fn reset_clears_status() {
        let mut f = fdc(Wd1770Timing::fast());
        f.write(2, 1);
        f.write(0, 0x88);
        f.tick(2000);
        f.reset();
        assert_eq!(f.status() & 0x03, 0);
        assert!(!f.drq() && !f.intrq());
        assert_eq!(f.read(2), 1);
    }
}
