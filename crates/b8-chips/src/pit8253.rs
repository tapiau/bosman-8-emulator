//! Intel 8253 programmable interval timer (3 counters + control word).
//!
//! Must support: control word (counter select, RL modes LSB/MSB/LSB+MSB/latch,
//! modes 0..5, BCD), counter read-back including latch command, counting in
//! time. The counters are clocked by `input_clock_hz`, the chip is ticked with
//! CPU cycles at `cpu_clock_hz`; the chip handles the fractional ratio.
//! Gate inputs are assumed high. Counter 0 of ANG-3001 may be clocked by the
//! output of counter 1 — supported through [`Pit8253::set_cascade_0_from_1`].
//!
//! Simplifications (gates are always high, there is no trigger edge):
//! - Mode 0 (interrupt on terminal count): after reaching 0 the counter
//!   **holds at 0** instead of wrapping to 0xFFFF. The ANG-3001 ROM
//!   (`WAIT_CTC_DONE`) polls the counter until it reads exactly 0x0000, and a
//!   wrapping counter could be missed between two polls.
//! - Mode 1 (hardware one-shot) is treated as if triggered when the count is
//!   loaded; it then behaves like mode 0 but keeps wrapping after terminal count.
//! - Mode 5 behaves like mode 4 (software strobe, triggered on load).
//! - The count is loaded into the counting element immediately when written
//!   (the real chip loads it on the next input clock edge).
//! - In modes 2 and 3 a new count written while running takes effect at the
//!   next reload, like the real chip.
//! - Mode 3 with an odd count: OUT is high for (N+1)/2 and low for (N-1)/2
//!   clocks; the read-back value decrements by 2.

#[derive(Debug, Clone, Default)]
struct Counter {
    /// Mode 0..=5.
    mode: u8,
    /// Read/load mode: 1 LSB, 2 MSB, 3 LSB then MSB.
    rl: u8,
    bcd: bool,
    /// Count register as written (raw, BCD-coded if `bcd`).
    cr: u16,
    /// Reload value in clocks (1..=65536 / 1..=10000), once a full count was written.
    reload: Option<u32>,
    /// Counting element: remaining clocks (may equal the modulus = reads as 0).
    ce: u32,
    running: bool,
    out: bool,
    /// LSB written, waiting for MSB (RL = 3).
    write_lsb_pending: bool,
    /// Next read returns MSB (RL = 3).
    read_msb_next: bool,
    latched: Option<u16>,
    /// Mode 4/5: strobe already generated.
    strobed: bool,
}

impl Counter {
    fn modulus(&self) -> u32 {
        if self.bcd {
            10000
        } else {
            65536
        }
    }

    fn decode_count(&self, raw: u16) -> u32 {
        let v = if self.bcd {
            let d = |s: u16| ((raw >> s) & 0xF).min(9) as u32;
            d(12) * 1000 + d(8) * 100 + d(4) * 10 + d(0)
        } else {
            raw as u32
        };
        if v == 0 {
            self.modulus()
        } else {
            v
        }
    }

    /// Current count as it would be read from the chip (BCD-coded if needed).
    fn readout(&self) -> u16 {
        let v = self.ce % self.modulus();
        if self.bcd {
            let v = v as u16;
            (v / 1000) << 12 | (v / 100 % 10) << 8 | (v / 10 % 10) << 4 | (v % 10)
        } else {
            v as u16
        }
    }

    fn control(&mut self, rl: u8, mode: u8, bcd: bool) {
        self.rl = rl;
        self.mode = mode;
        self.bcd = bcd;
        self.reload = None;
        self.running = false;
        self.out = mode != 0;
        self.write_lsb_pending = false;
        self.read_msb_next = false;
        self.latched = None;
        self.strobed = false;
    }

    fn mode3_half(&self, n: u32, high: bool) -> u32 {
        let half = if n & 1 == 0 {
            n
        } else if high {
            n + 1
        } else {
            n - 1
        };
        half.max(2)
    }

    fn load(&mut self) {
        let n = self.decode_count(self.cr);
        self.reload = Some(n);
        match self.mode {
            0 | 1 => {
                self.ce = n;
                self.out = false;
                self.running = true;
            }
            2 | 3 => {
                if !self.running {
                    self.running = true;
                    self.out = true;
                    self.ce = if self.mode == 3 { self.mode3_half(n, true) } else { n };
                }
            }
            _ => {
                self.ce = n;
                self.out = true;
                self.strobed = false;
                self.running = true;
            }
        }
    }

    fn write(&mut self, v: u8) {
        match self.rl {
            1 => {
                self.cr = v as u16;
                self.load();
            }
            2 => {
                self.cr = (v as u16) << 8;
                self.load();
            }
            _ => {
                if !self.write_lsb_pending {
                    self.cr = (self.cr & 0xFF00) | v as u16;
                    self.write_lsb_pending = true;
                    if self.mode == 0 {
                        // Writing the first byte stops counting in mode 0.
                        self.running = false;
                        self.out = false;
                    }
                } else {
                    self.cr = (self.cr & 0x00FF) | (v as u16) << 8;
                    self.write_lsb_pending = false;
                    self.load();
                }
            }
        }
    }

    fn read(&mut self) -> u8 {
        let value = self.latched.unwrap_or_else(|| self.readout());
        match self.rl {
            1 => {
                self.latched = None;
                value as u8
            }
            2 => {
                self.latched = None;
                (value >> 8) as u8
            }
            _ => {
                if self.read_msb_next {
                    self.read_msb_next = false;
                    self.latched = None;
                    (value >> 8) as u8
                } else {
                    self.read_msb_next = true;
                    value as u8
                }
            }
        }
    }

    fn latch(&mut self) {
        if self.latched.is_none() {
            self.latched = Some(self.readout());
        }
    }

    /// Advance by `n` input clocks; returns the number of falling OUT edges.
    fn advance(&mut self, mut n: u64) -> u64 {
        if !self.running || n == 0 {
            return 0;
        }
        let Some(reload) = self.reload else { return 0 };
        let modulus = self.modulus();
        let mut edges = 0;
        match self.mode {
            0 => {
                let k = n.min(self.ce as u64) as u32;
                self.ce -= k;
                if self.ce == 0 {
                    self.out = true;
                    self.running = false; // hold at 0 (see module docs)
                }
            }
            1 | 4 | 5 => {
                while n > 0 {
                    if self.ce == 0 {
                        // Wrap: 0 -> modulus-1.
                        self.ce = modulus - 1;
                        n -= 1;
                        if self.mode != 1 && !self.out {
                            self.out = true;
                        }
                        continue;
                    }
                    let k = n.min(self.ce as u64) as u32;
                    self.ce -= k;
                    n -= k as u64;
                    if self.ce == 0 {
                        if self.mode == 1 {
                            self.out = true;
                        } else if !self.strobed {
                            self.strobed = true;
                            self.out = false;
                            edges += 1;
                        }
                    }
                }
            }
            2 => {
                while n > 0 {
                    if self.ce > 1 {
                        let k = n.min((self.ce - 1) as u64) as u32;
                        self.ce -= k;
                        n -= k as u64;
                        if self.ce == 1 {
                            self.out = false;
                            edges += 1;
                        }
                    } else {
                        // Reload on the clock after reaching 1.
                        n -= 1;
                        self.ce = reload;
                        if reload == 1 {
                            edges += 1;
                            self.out = false;
                        } else {
                            self.out = true;
                        }
                    }
                }
            }
            _ => {
                // Mode 3: count down by 2, toggle OUT at the end of each half.
                while n > 0 {
                    let clocks = (self.ce / 2).max(1) as u64;
                    if n < clocks {
                        self.ce -= 2 * n as u32;
                        n = 0;
                    } else {
                        n -= clocks;
                        self.out = !self.out;
                        if !self.out {
                            edges += 1;
                        }
                        self.ce = self.mode3_half(reload, self.out);
                    }
                }
            }
        }
        edges
    }
}

#[derive(Debug, Clone)]
pub struct Pit8253 {
    counters: [Counter; 3],
    input_clock_hz: u32,
    cpu_clock_hz: u32,
    /// Accumulator for the fractional input/CPU clock ratio.
    acc: u64,
    cascade_0_from_1: bool,
}

impl Pit8253 {
    pub fn new(input_clock_hz: u32, cpu_clock_hz: u32) -> Self {
        Self {
            counters: Default::default(),
            input_clock_hz,
            cpu_clock_hz: cpu_clock_hz.max(1),
            acc: 0,
            cascade_0_from_1: false,
        }
    }

    pub fn reset(&mut self) {
        self.counters = Default::default();
        self.acc = 0;
    }

    /// `reg`: 0..=2 counters, 3 control word.
    pub fn write(&mut self, reg: u8, value: u8) {
        match reg & 3 {
            3 => {
                let sel = (value >> 6) as usize;
                if sel == 3 {
                    return; // read-back command does not exist on the 8253
                }
                let rl = (value >> 4) & 3;
                let c = &mut self.counters[sel];
                if rl == 0 {
                    c.latch();
                } else {
                    let mut mode = (value >> 1) & 7;
                    if mode >= 6 {
                        mode -= 4;
                    }
                    c.control(rl, mode, value & 1 != 0);
                }
            }
            n => self.counters[n as usize].write(value),
        }
    }

    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 3 {
            3 => 0xFF,
            n => self.counters[n as usize].read(),
        }
    }

    pub fn tick(&mut self, cpu_cycles: u32) {
        self.acc += cpu_cycles as u64 * self.input_clock_hz as u64;
        let clocks = self.acc / self.cpu_clock_hz as u64;
        self.acc %= self.cpu_clock_hz as u64;
        if clocks == 0 {
            return;
        }
        let edges1 = self.counters[1].advance(clocks);
        let c0_clocks = if self.cascade_0_from_1 { edges1 } else { clocks };
        self.counters[0].advance(c0_clocks);
        self.counters[2].advance(clocks);
    }

    /// Programmed reload value of a counter (0 means 65536 / 10000 BCD), if loaded.
    ///
    /// The value is returned in clocks, i.e. already BCD-decoded, with 0
    /// translated to 65536 (binary) or 10000 (BCD).
    pub fn reload(&self, counter: usize) -> Option<u32> {
        self.counters.get(counter)?.reload
    }

    /// Output frequency of a counter in Hz (input clock / reload), if running.
    ///
    /// Only periodic modes (2 and 3) have an output frequency. For counter 0
    /// in cascade mode the input is counter 1's output frequency.
    pub fn output_hz(&self, counter: usize) -> Option<f64> {
        let c = self.counters.get(counter)?;
        if !c.running || !(c.mode == 2 || c.mode == 3) {
            return None;
        }
        let input = if counter == 0 && self.cascade_0_from_1 {
            self.output_hz(1)?
        } else {
            self.input_clock_hz as f64
        };
        Some(input / c.reload? as f64)
    }

    /// Current OUT pin level of a counter.
    pub fn out(&self, counter: usize) -> bool {
        self.counters.get(counter).is_some_and(|c| c.out)
    }

    pub fn set_cascade_0_from_1(&mut self, on: bool) {
        self.cascade_0_from_1 = on;
    }

    /// Current mode (0..=5) of a counter, for debugging.
    pub fn mode(&self, counter: usize) -> u8 {
        self.counters.get(counter).map_or(0, |c| c.mode)
    }

    /// Current count (as the CPU would read it, BCD-coded in BCD mode),
    /// without affecting the read/latch state. For debugging.
    pub fn peek_count(&self, counter: usize) -> u16 {
        self.counters.get(counter).map_or(0, |c| c.readout())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read16(p: &mut Pit8253, c: u8) -> u16 {
        let lo = p.read(c) as u16;
        let hi = p.read(c) as u16;
        hi << 8 | lo
    }

    #[test]
    fn bosman_init() {
        // 2 MHz input, 4 MHz CPU.
        let mut p = Pit8253::new(2_000_000, 4_000_000);
        p.write(3, 0x35);
        p.write(0, 0x99);
        p.write(0, 0x99);
        p.write(3, 0x76);
        p.write(1, 20);
        p.write(1, 0);
        p.write(3, 0xB6);
        p.write(2, 13);
        p.write(2, 0);
        assert_eq!(p.mode(0), 2);
        assert_eq!(p.mode(1), 3);
        assert_eq!(p.mode(2), 3);
        assert_eq!(p.reload(0), Some(9999));
        assert_eq!(p.reload(1), Some(20));
        assert_eq!(p.reload(2), Some(13));
        assert_eq!(p.output_hz(1), Some(100_000.0));
        assert!((p.output_hz(2).unwrap() - 153_846.15).abs() < 0.1);
        // BCD counting: after 2 CPU cycles = 1 clock, 9999 -> 9998.
        assert_eq!(p.peek_count(0), 0x9999);
        p.tick(2);
        assert_eq!(p.peek_count(0), 0x9998);
        // 9 more clocks -> 9989 (BCD borrow).
        p.tick(18);
        assert_eq!(p.peek_count(0), 0x9989);
        // Latch and read.
        p.write(3, 0x00);
        p.tick(100);
        assert_eq!(read16(&mut p, 0), 0x9989);
        assert_ne!(read16(&mut p, 0), 0x9989);
    }

    #[test]
    fn fractional_ratio() {
        let mut p = Pit8253::new(1_000_000, 3_000_000);
        p.write(3, 0x30);
        p.write(0, 0x00);
        p.write(0, 0x10); // 4096
        for _ in 0..30 {
            p.tick(1);
        }
        assert_eq!(p.peek_count(0), 4096 - 10);
    }

    #[test]
    fn mode0_counts_to_zero_and_holds() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x30);
        p.write(0, 0x0F);
        assert!(!p.out(0));
        p.write(0, 0x00);
        assert!(!p.out(0));
        p.tick(10);
        assert_eq!(read16(&mut p, 0), 5);
        p.tick(5);
        assert_eq!(read16(&mut p, 0), 0);
        assert!(p.out(0));
        p.tick(100_000);
        assert_eq!(read16(&mut p, 0), 0);
        assert!(p.out(0));
    }

    #[test]
    fn mode0_lsb_write_stops_counting() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x30);
        p.write(0, 100);
        p.write(0, 0);
        p.tick(10);
        p.write(0, 50); // LSB only
        p.tick(10);
        assert_eq!(p.peek_count(0), 90);
        p.write(0, 0);
        assert_eq!(p.peek_count(0), 50);
    }

    /// ANG-3001 BEEP: counter 1 = tone (mode 3), counter 0 = duration in tone
    /// periods (mode 0, cascaded), polled until it reads 0x0000.
    #[test]
    fn ang_beep_wait_ctc_done() {
        let mut p = Pit8253::new(1_300_000, 6_000_000);
        p.set_cascade_0_from_1(true);
        p.write(3, 0x76);
        p.write(1, 0x00);
        p.write(1, 0x02); // 512 -> ~2.5 kHz
        p.write(3, 0x30);
        p.write(0, 0x0F);
        p.write(0, 0x00);
        assert!(p.output_hz(0).is_none());
        assert!((p.output_hz(1).unwrap() - 1_300_000.0 / 512.0).abs() < 1e-6);
        // Poll like WAIT_CTC_DONE: read LSB, MSB until both zero.
        let mut cycles = 0u64;
        let mut last = 0x0F;
        loop {
            let lo = p.read(0);
            let hi = p.read(0);
            let v = (hi as u16) << 8 | lo as u16;
            assert!(v <= last, "counter must not wrap ({v:#x})");
            last = v;
            if lo == 0 && hi == 0 {
                break;
            }
            p.tick(40);
            cycles += 40;
            assert!(cycles < 10_000_000, "BEEP never finished");
        }
        // 15 tone periods of 512 clocks at 1.3 MHz ~ 5.9 ms ~ 35k cycles.
        let expected = 15.0 * 512.0 / 1_300_000.0 * 6_000_000.0;
        assert!((cycles as f64 - expected).abs() < expected * 0.1, "{cycles}");
    }

    #[test]
    fn cascade_without_counter1_does_not_count() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.set_cascade_0_from_1(true);
        p.write(3, 0x30);
        p.write(0, 5);
        p.write(0, 0);
        p.tick(1000);
        assert_eq!(p.peek_count(0), 5);
    }

    #[test]
    fn mode2_rate_generator() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x14); // counter 0, LSB only, mode 2
        p.write(0, 4);
        assert!(p.out(0));
        p.tick(2);
        assert_eq!(p.peek_count(0), 2);
        p.tick(1);
        assert_eq!(p.peek_count(0), 1);
        assert!(!p.out(0));
        p.tick(1);
        assert_eq!(p.peek_count(0), 4);
        assert!(p.out(0));
        // Cascade edge count: counter 1 mode 2 with N=4 -> 1 edge per 4 clocks.
        p.write(3, 0x54);
        p.write(1, 4);
        p.set_cascade_0_from_1(true);
        p.write(3, 0x10); // counter 0, LSB, mode 0
        p.write(0, 10);
        p.tick(40);
        assert_eq!(p.peek_count(0), 0);
        p.write(0, 10);
        p.tick(20);
        assert_eq!(p.peek_count(0), 5);
    }

    #[test]
    fn mode3_square_wave() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x16); // counter 0, LSB, mode 3
        p.write(0, 6);
        assert!(p.out(0));
        p.tick(2);
        assert!(p.out(0));
        assert_eq!(p.peek_count(0), 2);
        p.tick(1);
        assert!(!p.out(0));
        p.tick(3);
        assert!(p.out(0));
        // Odd count 5: high 3 clocks, low 2 clocks.
        p.write(3, 0x16);
        p.write(0, 5);
        p.tick(2);
        assert!(p.out(0));
        p.tick(1);
        assert!(!p.out(0));
        p.tick(1);
        assert!(!p.out(0));
        p.tick(1);
        assert!(p.out(0));
    }

    #[test]
    fn mode3_new_count_at_next_half() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x16);
        p.write(0, 10);
        p.tick(2);
        p.write(0, 4);
        assert_eq!(p.reload(0), Some(4));
        p.tick(3); // finish 5-clock half
        assert!(!p.out(0));
        p.tick(2);
        assert!(p.out(0));
    }

    #[test]
    fn mode4_strobe_once() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x18); // counter 0, LSB, mode 4
        p.write(0, 3);
        assert!(p.out(0));
        p.tick(3);
        assert!(!p.out(0));
        assert_eq!(p.peek_count(0), 0);
        p.tick(1);
        assert!(p.out(0));
        assert_eq!(p.peek_count(0), 0xFFFF);
        p.tick(0xFFFF);
        assert!(p.out(0));
    }

    #[test]
    fn rl_msb_only_and_zero_count() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x24); // counter 0, MSB only, mode 2
        p.write(0, 0x01);
        assert_eq!(p.reload(0), Some(256));
        assert_eq!(p.read(0), 0x01);
        p.write(3, 0x34);
        p.write(0, 0);
        p.write(0, 0);
        assert_eq!(p.reload(0), Some(65536));
        p.write(3, 0x35);
        p.write(0, 0);
        p.write(0, 0);
        assert_eq!(p.reload(0), Some(10000));
        assert_eq!(read16(&mut p, 0), 0x0000);
        p.tick(1);
        assert_eq!(read16(&mut p, 0), 0x9999);
    }

    #[test]
    fn latch_holds_until_read() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        p.write(3, 0x30);
        p.write(0, 0x34);
        p.write(0, 0x12);
        p.write(3, 0x00);
        p.tick(0x10);
        p.write(3, 0x00); // second latch ignored while latched
        assert_eq!(p.read(0), 0x34);
        p.tick(0x10);
        assert_eq!(p.read(0), 0x12);
        assert_eq!(read16(&mut p, 0), 0x1234 - 0x20);
    }

    #[test]
    fn unloaded_counter_and_reset() {
        let mut p = Pit8253::new(1_000_000, 1_000_000);
        assert_eq!(p.reload(0), None);
        assert_eq!(p.output_hz(2), None);
        p.write(3, 0xB6);
        p.write(2, 13);
        p.write(2, 0);
        p.reset();
        assert_eq!(p.reload(2), None);
        assert_eq!(p.reload(5), None);
    }
}
