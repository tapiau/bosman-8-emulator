//! Front-panel style control of a running machine: reset buttons, DIP
//! switches, hot-swapping floppies and serial endpoints, status queries.
//! Used by the GUI; everything here is additive to the core in `lib.rs`.

use b8_chips::{DiskImage, Pit8253, Sio};
use b8_link::{Endpoint, LineStatus};
use iz80::Cpu;

use crate::{Bosman8, PIT_HZ, CPU_HZ};

impl Bosman8 {
    /// Warm reset = the RESET button of the real machine (🟡 hypothesis, see
    /// `doc/bosman8-hardware.md`, "Reset"): only the CPU (and the WD1770
    /// master reset) is reset; RAM and both 74LS259 latches keep their state.
    /// With RAM mapped at 0 the CPU executes `JP F203` (CP/M warm boot) and
    /// the CCP prints just the prompt, without the banner.
    pub fn warm_reset(&mut self) {
        self.cpu = Cpu::new_z80();
        self.bus.fdc.reset();
        self.nmi_prev = false;
    }

    /// Cold reset = power cycle: CPU, latches, SIO, 8253 and WD1770 reset,
    /// RAM cleared, the pending DIP value applied. Floppies and serial
    /// endpoints stay attached. The ROM then boots with the banner.
    pub fn cold_reset(&mut self) {
        self.cpu = Cpu::new_z80();
        let b = &mut self.bus;
        b.banks.iter_mut().for_each(|x| *x = 0);
        b.high.iter_mut().for_each(|x| *x = 0);
        b.latch.reset();
        b.latch2.reset();
        b.sio = Sio::new();
        b.pit = Pit8253::new(PIT_HZ, CPU_HZ);
        b.fdc.reset();
        b.update_fdc_select();
        b.io_counts.clear();
        b.timing_dirty = true;
        self.nmi_prev = false;
        self.tx_break = [false; 2];
        self.tracing_cpu = false;
    }

    /// Jumper bits of port 0x98 (see [`crate::p98::DIP_MASK`]). The ROM reads
    /// them while booting, so a change normally needs [`Self::cold_reset`].
    pub fn set_dip(&mut self, dip: u8) {
        self.bus.dip = dip;
    }

    pub fn dip(&self) -> u8 {
        self.bus.dip
    }

    /// Remove the floppy from `drive` and return it (e.g. to save it).
    pub fn eject_disk(&mut self, drive: usize) -> Option<DiskImage> {
        let d = self.bus.fdc.disk(drive).cloned();
        self.bus.fdc.insert(drive, None);
        d
    }

    /// Set the write-protect tab of the floppy in `drive` (if any).
    pub fn set_write_protect(&mut self, drive: usize, on: bool) {
        if let Some(d) = self.bus.fdc.disk_mut(drive) {
            d.write_protected = on;
        }
    }

    /// True while the WD1770 works with `drive` (selected and motor on or
    /// command running) — the drive LED.
    pub fn drive_active(&self, drive: usize) -> bool {
        self.bus.selected_drive() == Some(drive) && (self.bus.fdc.motor_on() || self.bus.fdc.busy())
    }

    /// Replace the terminal (SIO-A) endpoint; the old one is returned (drop
    /// it to close it — before opening a new listener on the same port).
    pub fn set_term_endpoint(&mut self, ep: Box<dyn Endpoint>) -> Box<dyn Endpoint> {
        std::mem::replace(&mut self.term, ep)
    }

    /// Replace the V.24 (SIO-B) endpoint; the old one is returned.
    pub fn set_aux_endpoint(&mut self, ep: Box<dyn Endpoint>) -> Box<dyn Endpoint> {
        std::mem::replace(&mut self.aux, ep)
    }

    /// Pulse the NMI line (as an INTRQ of the WD1770 would).
    pub fn nmi(&mut self) {
        self.cpu.signal_nmi();
    }

    pub fn term_status(&self) -> LineStatus {
        self.term.status()
    }

    pub fn aux_status(&self) -> LineStatus {
        self.aux.status()
    }

    /// Current RAM bank (0–7) and whether RAM (not ROM) is mapped at 0000.
    pub fn bank(&self) -> (usize, bool) {
        (self.bus.bank(), self.bus.latch.bit(7))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn rom() -> Option<Vec<u8>> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/bosman8.bin")).ok()
    }

    fn machine(dip: u8) -> Option<(Bosman8, Box<dyn Endpoint>)> {
        let (term, host) = b8_link::pair();
        let cfg = Config { fast_fdc: true, dip, ..Config::default() };
        Some((Bosman8::new(&rom()?, cfg, term, Box::new(b8_link::NullEndpoint)), host))
    }

    /// Run until `needle` is seen in the terminal output; returns the output.
    fn run_until(m: &mut Bosman8, host: &mut Box<dyn Endpoint>, needle: &str, max_s: f64) -> Option<String> {
        let mut out = Vec::new();
        let start = m.cycles();
        while m.cycles() - start < (max_s * CPU_HZ as f64) as u64 {
            m.run_cycles(CPU_HZ as u64 / 100);
            while let Some(b) = host.read() {
                out.push(b);
            }
            let s: String = out.iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' }).collect();
            if s.contains(needle) {
                return Some(s);
            }
        }
        None
    }

    fn type_line(host: &mut Box<dyn Endpoint>, s: &str) {
        for b in s.bytes() {
            host.write(b);
        }
    }

    /// Reset while a program runs from the TPA (RAM bank 0 mapped at 0000):
    /// the CPU starts at RAM 0000 = `JP F203` → warm boot → prompt, no banner.
    #[test]
    fn warm_reset_in_program_gives_prompt_without_banner() {
        let Some((mut m, mut host)) = machine(0x40) else { return };
        let fmt = crate::disk::format_for(0x40, 0);
        let mut disk = fmt.blank();
        crate::cpmfs::CpmFs::new(&mut disk.data, crate::cpmfs::Params::for_format(fmt))
            .unwrap()
            .put(0, "LOOP.COM", &[0x18, 0xFE]) // JR $
            .unwrap();
        m.insert_disk(0, Some(disk));
        let boot = run_until(&mut m, &mut host, ".A>", 10.0).expect("first boot");
        assert!(boot.contains("Mikrokomputer"), "{boot}");
        type_line(&mut host, "B:LOOP\r");
        run_until(&mut m, &mut host, "never", 2.0);
        assert_eq!(m.bank(), (0, true), "LOOP should run from RAM");
        m.warm_reset();
        let s = run_until(&mut m, &mut host, ".A>", 5.0).expect("prompt after warm reset");
        assert!(!s.contains("Mikrokomputer"), "banner after warm reset: {s}");
        type_line(&mut host, "DIR B:\r");
        let s = run_until(&mut m, &mut host, "COM", 5.0);
        assert!(s.is_some(), "CCP dead after warm reset");
    }

    /// Reset at the CCP prompt: the CCP of CPM-R runs from ROM (latch Q7 = 0),
    /// so under the "CPU only" model the ROM cold start runs (with banner).
    #[test]
    fn warm_reset_at_prompt_keeps_ram_and_latch() {
        let Some((mut m, mut host)) = machine(0x40) else { return };
        run_until(&mut m, &mut host, ".A>", 10.0).expect("first boot");
        let latch = m.bus.latch.value;
        let marker = m.peek(0x8800);
        m.warm_reset();
        assert_eq!(m.bus.latch.value, latch);
        assert_eq!(m.peek(0x8800), marker);
        assert_eq!(m.pc(), 0);
        assert!(run_until(&mut m, &mut host, ".A>", 10.0).is_some(), "no prompt after warm reset");
    }

    #[test]
    fn cold_reset_gives_banner() {
        let Some((mut m, mut host)) = machine(0x40) else { return };
        run_until(&mut m, &mut host, ".A>", 10.0).expect("first boot");
        m.cold_reset();
        let s = run_until(&mut m, &mut host, ".A>", 10.0).expect("prompt after cold reset");
        assert!(s.contains("Mikrokomputer"), "no banner after cold reset: {s}");
    }

    #[test]
    fn disk_hot_swap() {
        let Some((mut m, mut host)) = machine(0x40) else { return };
        let fmt = crate::disk::format_for(0x40, 0);
        run_until(&mut m, &mut host, ".A>", 10.0).expect("boot");
        // Insert a blank disk while running, write a file to it.
        m.insert_disk(0, Some(fmt.blank()));
        for b in b"SAVE 1 B:X.COM\r" {
            host.write(*b);
        }
        run_until(&mut m, &mut host, ".A>", 10.0).expect("SAVE");
        let written = m.eject_disk(0).expect("disk was inserted");
        assert!(m.disk(0).is_none());
        assert_eq!(&written.data[1..12], b"X       COM", "nothing written");
        // Swap in another blank disk: CP/M must see an empty directory
        // (after ^C, which re-logs the disks).
        m.insert_disk(0, Some(fmt.blank()));
        host.write(0x03);
        run_until(&mut m, &mut host, ".A>", 5.0).expect("^C");
        for b in b"DIR B:\r" {
            host.write(*b);
        }
        let s = run_until(&mut m, &mut host, "Brak pliku", 10.0);
        assert!(s.is_some(), "swapped disk not seen as empty");
        // And the written one again.
        m.insert_disk(0, Some(written));
        host.write(0x03);
        run_until(&mut m, &mut host, ".A>", 5.0).expect("^C");
        for b in b"DIR B:\r" {
            host.write(*b);
        }
        let s = run_until(&mut m, &mut host, ".A>", 10.0).unwrap_or_default();
        assert!(s.contains("X ") && s.contains("COM"), "re-inserted disk lost its file: {s}");
    }

    #[test]
    fn write_protect_and_activity() {
        let Some((mut m, _host)) = machine(0x40) else { return };
        m.insert_disk(0, Some(crate::disk::F200.blank()));
        m.set_write_protect(0, true);
        assert!(m.disk(0).unwrap().write_protected);
        assert!(!m.drive_active(1));
        m.set_dip(0x00);
        assert_eq!(m.dip(), 0);
    }
}
