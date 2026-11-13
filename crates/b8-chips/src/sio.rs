//! Zilog Z80-SIO (two channels), byte-level model.
//!
//! Register access: `write_ctrl/read_ctrl` (WR0..WR7 via pointer in WR0,
//! RR0..RR2), `write_data/read_data`. Implement at least:
//! - WR0: register pointer (bits 0-2), commands in bits 3-5 (reset ext/status
//!   int = 2, channel reset = 3, error reset = 6, ...); pointer resets to 0
//!   after each access to another register.
//! - RR0: bit0 Rx char available, bit2 Tx buffer empty, bit3 DCD, bit5 CTS
//!   (report 1 = asserted), bit7 break/abort (set while a break was received,
//!   cleared by reset ext/status).
//! - RR1: bit0 all sent.
//! - WR3 bit0 Rx enable, WR5 bit3 Tx enable, WR4 clock mode / sync mode.
//! Timing: one character takes `char_cycles` CPU cycles (set by the machine
//! from 8253 frequency and WR4 clock divider — use `set_char_cycles`). After a
//! data write the Tx buffer is busy that long, then the byte becomes available
//! through `take_tx()`. Received bytes are queued with `push_rx()` (unbounded
//! host-side queue) and moved into the Rx register one at a time, when the
//! register is empty and at least `char_cycles` have passed since the previous
//! one. Interrupts are not needed by the ROMs — not implemented.
//!
//! Model details:
//! - The transmitter has a buffer and a shift register like the real chip. A
//!   data write goes to the buffer and, if the shift register is idle (and Tx
//!   is enabled), moves on immediately — so RR0 bit 2 (Tx buffer empty) is set
//!   again at once while RR1 bit 0 (all sent) stays 0 for `char_cycles`.
//!   After `char_cycles` the byte is handed to `take_tx()`.
//! - Tx needs WR5 bit 3 (Tx enable); Rx needs WR3 bit 0 (Rx enable). While
//!   disabled, bytes wait (in the Tx buffer / host Rx queue) — nothing is lost.
//! - Sync modes are treated as a plain byte stream (no hunt / sync characters).
//! - Break: [`SioChannel::push_break`] starts a received break lasting
//!   `break_cycles` (default 200 000 CPU cycles, see
//!   [`SioChannel::set_break_cycles`]). RR0 bit 7 = break in progress OR the
//!   latched break bit; "reset ext/status" (WR0 command 2) clears the latch,
//!   but not a break still in progress — so the Bosman sequence
//!   `OUT (82h),10h ; IN A,(82h) ; BIT 7,A` sees a break while it lasts.
//! - Transmit side break (WR5 bit 4) is reported by [`SioChannel::break_active`]
//!   for the machine to forward to its endpoint.

use std::collections::VecDeque;

const DEFAULT_CHAR_CYCLES: u32 = 1000;
const DEFAULT_BREAK_CYCLES: u32 = 200_000;

#[derive(Debug, Clone)]
pub struct SioChannel {
    ptr: u8,
    wr: [u8; 8],
    char_cycles: u32,
    break_cycles: u32,
    // Receiver
    rx_queue: VecDeque<u8>,
    rx_reg: Option<u8>,
    rx_since: u32,
    // Transmitter
    tx_buf: Option<u8>,
    tx_shift: Option<u8>,
    tx_remaining: u32,
    tx_out: VecDeque<u8>,
    // Break
    break_remaining: u32,
    break_latched: bool,
}

impl Default for SioChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl SioChannel {
    pub fn new() -> Self {
        Self {
            ptr: 0,
            wr: [0; 8],
            char_cycles: DEFAULT_CHAR_CYCLES,
            break_cycles: DEFAULT_BREAK_CYCLES,
            rx_queue: VecDeque::new(),
            rx_reg: None,
            rx_since: u32::MAX,
            tx_buf: None,
            tx_shift: None,
            tx_remaining: 0,
            tx_out: VecDeque::new(),
            break_remaining: 0,
            break_latched: false,
        }
    }

    /// Channel reset (WR0 command 3 or hardware reset). The host-side Rx
    /// queue and already transmitted bytes (`take_tx`) are kept.
    pub fn reset(&mut self) {
        self.ptr = 0;
        self.wr = [0; 8];
        self.rx_reg = None;
        self.rx_since = u32::MAX;
        self.tx_buf = None;
        self.tx_shift = None;
        self.tx_remaining = 0;
        self.break_latched = false;
    }

    pub fn write_ctrl(&mut self, v: u8) {
        let p = self.ptr;
        self.ptr = 0;
        if p != 0 {
            self.wr[p as usize] = v;
            if p == 5 {
                self.start_tx();
            }
            return;
        }
        self.wr[0] = v;
        self.ptr = v & 7;
        match (v >> 3) & 7 {
            2 => {
                // Reset ext/status interrupts: re-latch current state.
                self.break_latched = false;
            }
            3 => {
                self.reset();
            }
            _ => {} // null, abort, int commands, error reset: nothing to do
        }
    }

    pub fn read_ctrl(&mut self) -> u8 {
        let p = self.ptr;
        self.ptr = 0;
        match p {
            0 => self.rr0(),
            1 => self.rr1(),
            2 => self.wr[2],
            _ => 0,
        }
    }

    fn rr0(&self) -> u8 {
        let mut v = 0x08 | 0x20; // DCD, CTS asserted
        if self.rx_reg.is_some() {
            v |= 0x01;
        }
        if self.tx_buf.is_none() {
            v |= 0x04;
        }
        if self.break_remaining > 0 || self.break_latched {
            v |= 0x80;
        }
        v
    }

    fn rr1(&self) -> u8 {
        self.all_sent() as u8
    }

    pub fn write_data(&mut self, v: u8) {
        // Writing to a full buffer overwrites it (as on the real chip).
        self.tx_buf = Some(v);
        self.start_tx();
    }

    pub fn read_data(&mut self) -> u8 {
        self.rx_reg.take().unwrap_or(0xFF)
    }

    /// Byte transmitted by the CPU, ready to be sent to the endpoint.
    pub fn take_tx(&mut self) -> Option<u8> {
        self.tx_out.pop_front()
    }

    /// Byte received from the endpoint.
    pub fn push_rx(&mut self, b: u8) {
        self.rx_queue.push_back(b);
    }

    /// Mark that a break condition was received.
    pub fn push_break(&mut self) {
        self.break_remaining = self.break_cycles.max(1);
        self.break_latched = true;
    }

    pub fn set_char_cycles(&mut self, cycles: u32) {
        self.char_cycles = cycles;
    }

    /// Duration of a received break (see module docs).
    pub fn set_break_cycles(&mut self, cycles: u32) {
        self.break_cycles = cycles;
    }

    /// Clock divider selected in WR4 bits 6-7 (1, 16, 32, 64); 1 in sync modes.
    pub fn clock_divider(&self) -> u32 {
        if self.sync_mode() {
            return 1;
        }
        match self.wr[4] >> 6 {
            0 => 1,
            1 => 16,
            2 => 32,
            _ => 64,
        }
    }

    /// Number of data bits per character for Tx (from WR5), used for timing.
    pub fn bits_per_char(&self) -> u32 {
        match (self.wr[5] >> 5) & 3 {
            0 => 5,
            1 => 7,
            2 => 6,
            _ => 8,
        }
    }

    /// True if WR4 selects a synchronous mode (stop bits field = 00).
    pub fn sync_mode(&self) -> bool {
        self.wr[4] & 0x0C == 0
    }

    /// Value last written to write register `n` (0..=7), for debugging.
    pub fn wr(&self, n: usize) -> u8 {
        self.wr[n & 7]
    }

    pub fn rx_enabled(&self) -> bool {
        self.wr[3] & 0x01 != 0
    }

    pub fn tx_enabled(&self) -> bool {
        self.wr[5] & 0x08 != 0
    }

    /// True while the CPU asks to send a break (WR5 bit 4).
    pub fn break_active(&self) -> bool {
        self.wr[5] & 0x10 != 0
    }

    /// True when the transmitter is completely idle (RR1 bit 0).
    pub fn all_sent(&self) -> bool {
        self.tx_buf.is_none() && self.tx_shift.is_none()
    }

    /// Number of bytes waiting in the host-side Rx queue (not yet in the Rx register).
    pub fn rx_pending(&self) -> usize {
        self.rx_queue.len()
    }

    fn start_tx(&mut self) {
        if self.tx_shift.is_none() && self.tx_enabled() {
            if let Some(b) = self.tx_buf.take() {
                self.tx_shift = Some(b);
                self.tx_remaining = self.char_cycles;
            }
        }
    }

    pub fn tick(&mut self, cpu_cycles: u32) {
        // Transmitter.
        let mut cycles = cpu_cycles;
        while let Some(b) = self.tx_shift {
            if self.tx_remaining > cycles {
                self.tx_remaining -= cycles;
                break;
            }
            cycles -= self.tx_remaining;
            self.tx_out.push_back(b);
            self.tx_shift = None;
            self.start_tx();
        }
        // Receiver.
        self.rx_since = self.rx_since.saturating_add(cpu_cycles);
        if self.rx_enabled()
            && self.rx_reg.is_none()
            && self.rx_since >= self.char_cycles
        {
            if let Some(b) = self.rx_queue.pop_front() {
                self.rx_reg = Some(b);
                self.rx_since = 0;
            }
        }
        // Break.
        self.break_remaining = self.break_remaining.saturating_sub(cpu_cycles);
    }
}

#[derive(Debug, Clone)]
pub struct Sio {
    pub a: SioChannel,
    pub b: SioChannel,
}

impl Default for Sio {
    fn default() -> Self {
        Self::new()
    }
}

impl Sio {
    pub fn new() -> Self {
        Self { a: SioChannel::new(), b: SioChannel::new() }
    }

    pub fn reset(&mut self) {
        self.a.reset();
        self.b.reset();
    }

    pub fn tick(&mut self, cpu_cycles: u32) {
        self.a.tick(cpu_cycles);
        self.b.tick(cpu_cycles);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SIOB_INIT-like sequence: pointer, value pairs for WR3, WR4, WR5.
    fn init(ch: &mut SioChannel, wr3: u8, wr4: u8, wr5: u8) {
        ch.write_ctrl(0x18); // channel reset
        ch.write_ctrl(3);
        ch.write_ctrl(wr3);
        ch.write_ctrl(4);
        ch.write_ctrl(wr4);
        ch.write_ctrl(5);
        ch.write_ctrl(wr5);
    }

    /// Bosman BIOS CONOUT: wait for RR1.0 then write data.
    fn conout(ch: &mut SioChannel, c: u8, max_polls: u32) -> u32 {
        let mut polls = 0;
        loop {
            ch.write_ctrl(1);
            let a = ch.read_ctrl();
            if a & 1 != 0 {
                break;
            }
            ch.tick(20);
            polls += 1;
            assert!(polls < max_polls, "CONOUT hangs");
        }
        ch.write_data(c);
        polls
    }

    #[test]
    fn siob_init_registers() {
        let mut s = Sio::new();
        // async x16, 1 stop bit, 8 bits Rx/Tx, enables
        init(&mut s.b, 0xC1, 0x44, 0x68);
        assert_eq!(s.b.wr(3), 0xC1);
        assert_eq!(s.b.wr(4), 0x44);
        assert_eq!(s.b.wr(5), 0x68);
        assert_eq!(s.b.clock_divider(), 16);
        assert_eq!(s.b.bits_per_char(), 8);
        assert!(!s.b.sync_mode());
        assert!(s.b.rx_enabled() && s.b.tx_enabled());
        // pointer is back at 0 -> reads RR0
        assert_eq!(s.b.read_ctrl() & 0x2C, 0x2C);
    }

    #[test]
    fn sync_mode_divider_is_one() {
        let mut ch = SioChannel::new();
        init(&mut ch, 0xC1, 0xC0, 0x68); // x64 bits but sync (bits 2-3 = 0)
        assert!(ch.sync_mode());
        assert_eq!(ch.clock_divider(), 1);
        ch.write_ctrl(4);
        ch.write_ctrl(0xC4);
        assert_eq!(ch.clock_divider(), 64);
        ch.write_ctrl(4);
        ch.write_ctrl(0x84);
        assert_eq!(ch.clock_divider(), 32);
        ch.write_ctrl(4);
        ch.write_ctrl(0x04);
        assert_eq!(ch.clock_divider(), 1);
    }

    #[test]
    fn bits_per_char_values() {
        let mut ch = SioChannel::new();
        for (wr5, bits) in [(0x00, 5), (0x20, 7), (0x40, 6), (0x60, 8)] {
            ch.write_ctrl(5);
            ch.write_ctrl(wr5);
            assert_eq!(ch.bits_per_char(), bits);
        }
    }

    #[test]
    fn bosman_conout_sequence() {
        let mut ch = SioChannel::new();
        init(&mut ch, 0xC1, 0x10, 0x68);
        ch.set_char_cycles(400);
        assert_eq!(conout(&mut ch, b'A', 10), 0);
        assert_eq!(ch.take_tx(), None);
        // RR0: Tx buffer already empty (byte in shift register), not all sent.
        assert_eq!(ch.read_ctrl() & 0x04, 0x04);
        ch.write_ctrl(1);
        assert_eq!(ch.read_ctrl() & 1, 0);
        let polls = conout(&mut ch, b'B', 1000);
        assert!((19..=21).contains(&polls), "{polls}");
        assert_eq!(ch.take_tx(), Some(b'A'));
        assert_eq!(ch.take_tx(), None);
        ch.tick(400);
        assert_eq!(ch.take_tx(), Some(b'B'));
        ch.write_ctrl(1);
        assert_eq!(ch.read_ctrl() & 1, 1);
    }

    #[test]
    fn tx_double_buffer() {
        let mut ch = SioChannel::new();
        init(&mut ch, 0xC1, 0x44, 0x68);
        ch.set_char_cycles(100);
        ch.write_data(1);
        assert_eq!(ch.read_ctrl() & 4, 4);
        ch.write_data(2);
        assert_eq!(ch.read_ctrl() & 4, 0);
        ch.tick(250); // both done within one tick
        assert_eq!(ch.take_tx(), Some(1));
        assert_eq!(ch.take_tx(), Some(2));
        assert!(ch.all_sent());
    }

    #[test]
    fn tx_disabled_waits() {
        let mut ch = SioChannel::new();
        ch.set_char_cycles(10);
        ch.write_data(0x55);
        ch.tick(1000);
        assert_eq!(ch.take_tx(), None);
        assert_eq!(ch.read_ctrl() & 4, 0);
        ch.write_ctrl(5);
        ch.write_ctrl(0x68);
        ch.tick(10);
        assert_eq!(ch.take_tx(), Some(0x55));
    }

    #[test]
    fn rx_pacing_and_const() {
        let mut ch = SioChannel::new();
        init(&mut ch, 0xC1, 0x44, 0x68);
        ch.set_char_cycles(100);
        ch.push_rx(b'x');
        ch.push_rx(b'y');
        // CONST: IN A,(82h) AND 1
        assert_eq!(ch.read_ctrl() & 1, 0);
        ch.tick(1);
        assert_eq!(ch.read_ctrl() & 1, 1);
        assert_eq!(ch.read_data(), b'x');
        assert_eq!(ch.read_ctrl() & 1, 0);
        ch.tick(50);
        assert_eq!(ch.read_ctrl() & 1, 0);
        ch.tick(50);
        assert_eq!(ch.read_ctrl() & 1, 1);
        // Not overwritten while unread.
        ch.push_rx(b'z');
        ch.tick(1000);
        assert_eq!(ch.read_data(), b'y');
        ch.tick(1);
        assert_eq!(ch.read_data(), b'z');
        assert_eq!(ch.rx_pending(), 0);
    }

    #[test]
    fn rx_disabled_keeps_queue() {
        let mut ch = SioChannel::new();
        ch.push_rx(7);
        ch.tick(10_000);
        assert_eq!(ch.read_ctrl() & 1, 0);
        ch.write_ctrl(3);
        ch.write_ctrl(0xC1);
        ch.tick(1);
        assert_eq!(ch.read_data(), 7);
    }

    #[test]
    fn break_check_sequence() {
        let mut ch = SioChannel::new();
        init(&mut ch, 0xC1, 0x10, 0x68);
        ch.set_break_cycles(1000);
        let check = |ch: &mut SioChannel| {
            ch.write_ctrl(0x10);
            ch.read_ctrl() & 0x80 != 0
        };
        assert!(!check(&mut ch));
        ch.push_break();
        assert!(check(&mut ch));
        ch.tick(500);
        assert!(check(&mut ch));
        ch.tick(600);
        assert!(!check(&mut ch));
        assert_eq!(ch.read_ctrl() & 0x80, 0);
        // A short break that ended unseen stays latched until reset ext/status.
        ch.push_break();
        ch.tick(2000);
        assert_eq!(ch.read_ctrl() & 0x80, 0x80);
        assert!(!check(&mut ch));
    }

    #[test]
    fn pointer_resets_and_rr2() {
        let mut ch = SioChannel::new();
        ch.write_ctrl(2);
        ch.write_ctrl(0x40);
        ch.write_ctrl(2);
        assert_eq!(ch.read_ctrl(), 0x40);
        // next read is RR0 again
        assert_eq!(ch.read_ctrl() & 0x28, 0x28);
        // command + pointer in one WR0 write
        ch.write_ctrl(0x11); // reset ext/status, pointer 1
        assert_eq!(ch.read_ctrl(), 1);
    }

    #[test]
    fn channel_reset_clears_state() {
        let mut ch = SioChannel::new();
        init(&mut ch, 0xC1, 0x44, 0x78);
        assert!(ch.break_active());
        ch.write_data(1);
        ch.push_rx(2);
        ch.tick(1);
        ch.write_ctrl(0x18);
        assert_eq!(ch.wr(5), 0);
        assert!(!ch.break_active());
        assert!(ch.all_sent());
        assert_eq!(ch.read_ctrl() & 1, 0);
    }

    #[test]
    fn channels_independent() {
        let mut s = Sio::new();
        init(&mut s.a, 0xC1, 0x10, 0x68);
        s.a.set_char_cycles(10);
        s.a.write_data(b'a');
        s.tick(10);
        assert_eq!(s.a.take_tx(), Some(b'a'));
        assert_eq!(s.b.take_tx(), None);
        s.reset();
        assert_eq!(s.a.wr(3), 0);
    }
}
