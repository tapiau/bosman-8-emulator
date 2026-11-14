//! Intel 8251 USART, byte-level model.
//!
//! Control port: after reset expects a mode instruction, then (sync mode) one
//! or two sync characters, then command instructions. Command bit 6 = internal
//! reset (back to expecting mode). The usual init `00 00 00 40` must work.
//! Command: bit0 TxEN, bit2 RxEN, bit3 send break, bit4 error reset.
//! Status: bit0 TxRDY, bit1 RxRDY, bit2 TxEMPTY, bit3 PE, bit4 OE, bit5 FE,
//! bit6 SYNDET, bit7 DSR (the DSR input; inactive = 0 unless [`Usart8251::set_dsr`]).
//! Timing/queues: same contract as `SioChannel` (char_cycles, take_tx, push_rx).
//!
//! Model details:
//! - Transmitter has a buffer and a shift register: TxRDY = buffer empty,
//!   TxEMPTY = buffer and shift register empty. Transmission requires TxEN
//!   (command bit 0); Rx requires RxEN (command bit 2). While disabled, bytes
//!   wait, nothing is lost. The byte reaches `take_tx()` after `char_cycles`.
//! - Parity/overrun/framing errors never occur (status bits 3-5 are 0).
//! - [`Usart8251::push_break`] (extension) reports a received break on
//!   status bit 6 (SYNDET/BRKDET) for `break_cycles` CPU cycles.

use std::collections::VecDeque;

const DEFAULT_CHAR_CYCLES: u32 = 1000;
const DEFAULT_BREAK_CYCLES: u32 = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Mode,
    Sync1,
    Sync2,
    Command,
}

#[derive(Debug, Clone)]
pub struct Usart8251 {
    expect: Expect,
    mode: u8,
    sync: [u8; 2],
    command: u8,
    char_cycles: u32,
    break_cycles: u32,
    rx_queue: VecDeque<u8>,
    rx_reg: Option<u8>,
    rx_since: u32,
    tx_buf: Option<u8>,
    tx_shift: Option<u8>,
    tx_remaining: u32,
    tx_out: VecDeque<u8>,
    break_remaining: u32,
    dsr: bool,
}

impl Default for Usart8251 {
    fn default() -> Self {
        Self::new()
    }
}

impl Usart8251 {
    pub fn new() -> Self {
        Self {
            expect: Expect::Mode,
            mode: 0,
            sync: [0; 2],
            command: 0,
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
            dsr: false,
        }
    }

    /// DSR input asserted (status bit 7). Nothing drives it on the ANG-3001
    /// side of the emulated link, so it stays inactive by default: ROM
    /// 90.09.11 treats an active DSR as an attention signal and waits in a
    /// loop until it drops (the older ROM never reads the bit).
    pub fn set_dsr(&mut self, on: bool) {
        self.dsr = on;
    }

    /// Hardware reset. The host-side Rx queue and already transmitted bytes
    /// are kept.
    pub fn reset(&mut self) {
        self.internal_reset();
        self.mode = 0;
        self.sync = [0; 2];
        self.break_remaining = 0;
    }

    fn internal_reset(&mut self) {
        self.expect = Expect::Mode;
        self.command = 0;
        self.rx_reg = None;
        self.rx_since = u32::MAX;
        self.tx_buf = None;
        self.tx_shift = None;
        self.tx_remaining = 0;
    }

    pub fn write_ctrl(&mut self, v: u8) {
        match self.expect {
            Expect::Mode => {
                self.mode = v;
                self.expect = if v & 3 == 0 { Expect::Sync1 } else { Expect::Command };
            }
            Expect::Sync1 => {
                self.sync[0] = v;
                // Mode bit 7 (SCS): 1 = single sync character.
                self.expect = if self.mode & 0x80 != 0 { Expect::Command } else { Expect::Sync2 };
            }
            Expect::Sync2 => {
                self.sync[1] = v;
                self.expect = Expect::Command;
            }
            Expect::Command => {
                if v & 0x40 != 0 {
                    self.internal_reset();
                    return;
                }
                self.command = v;
                self.start_tx();
            }
        }
    }

    pub fn read_status(&mut self) -> u8 {
        let mut s = if self.dsr { 0x80 } else { 0 };
        if self.tx_buf.is_none() {
            s |= 0x01;
        }
        if self.rx_reg.is_some() {
            s |= 0x02;
        }
        if self.tx_buf.is_none() && self.tx_shift.is_none() {
            s |= 0x04;
        }
        if self.break_remaining > 0 {
            s |= 0x40;
        }
        s
    }

    pub fn write_data(&mut self, v: u8) {
        self.tx_buf = Some(v);
        self.start_tx();
    }

    pub fn read_data(&mut self) -> u8 {
        self.rx_reg.take().unwrap_or(0xFF)
    }

    pub fn tick(&mut self, cpu_cycles: u32) {
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
        self.rx_since = self.rx_since.saturating_add(cpu_cycles);
        if self.rx_enabled() && self.rx_reg.is_none() && self.rx_since >= self.char_cycles {
            if let Some(b) = self.rx_queue.pop_front() {
                self.rx_reg = Some(b);
                self.rx_since = 0;
            }
        }
        self.break_remaining = self.break_remaining.saturating_sub(cpu_cycles);
    }

    pub fn take_tx(&mut self) -> Option<u8> {
        self.tx_out.pop_front()
    }

    pub fn push_rx(&mut self, b: u8) {
        self.rx_queue.push_back(b);
    }

    /// Mark that a break condition was received (status bit 6 for a while).
    pub fn push_break(&mut self) {
        self.break_remaining = self.break_cycles.max(1);
    }

    /// Duration of a received break in CPU cycles.
    pub fn set_break_cycles(&mut self, cycles: u32) {
        self.break_cycles = cycles;
    }

    /// True while the CPU has the "send break" command bit set.
    pub fn break_active(&self) -> bool {
        self.expect == Expect::Command && self.command & 0x08 != 0
    }

    pub fn set_char_cycles(&mut self, cycles: u32) {
        self.char_cycles = cycles;
    }

    /// Baud rate factor from the mode instruction (1, 16, 64; 1 for sync).
    pub fn clock_divider(&self) -> u32 {
        match self.mode & 3 {
            2 => 16,
            3 => 64,
            _ => 1,
        }
    }

    /// Data bits per character from the mode instruction (5..=8).
    pub fn bits_per_char(&self) -> u32 {
        5 + ((self.mode >> 2) & 3) as u32
    }

    /// Last mode instruction, for debugging.
    pub fn mode(&self) -> u8 {
        self.mode
    }

    /// Last command instruction, for debugging.
    pub fn command(&self) -> u8 {
        self.command
    }

    /// True when the chip expects a command (mode and sync chars written).
    pub fn initialized(&self) -> bool {
        self.expect == Expect::Command
    }

    pub fn tx_enabled(&self) -> bool {
        self.command & 0x01 != 0
    }

    pub fn rx_enabled(&self) -> bool {
        self.command & 0x04 != 0
    }

    /// Number of bytes waiting in the host-side Rx queue.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ang_init(u: &mut Usart8251, mode: u8) {
        for v in [0x00, 0x00, 0x00, 0x40, mode, 0x05] {
            u.write_ctrl(v);
        }
    }

    #[test]
    fn ang_init_cd() {
        let mut u = Usart8251::new();
        ang_init(&mut u, 0xCD);
        assert!(u.initialized());
        assert_eq!(u.mode(), 0xCD);
        assert_eq!(u.command(), 0x05);
        assert_eq!(u.clock_divider(), 1);
        assert_eq!(u.bits_per_char(), 8);
        assert!(u.tx_enabled() && u.rx_enabled());
        assert_eq!(u.read_status(), 0x05);
        u.set_dsr(true);
        assert_eq!(u.read_status(), 0x85);
    }

    #[test]
    fn ang_init_fa() {
        let mut u = Usart8251::new();
        ang_init(&mut u, 0xFA);
        assert_eq!(u.clock_divider(), 16);
        assert_eq!(u.bits_per_char(), 7);
        assert_eq!(u.command(), 0x05);
    }

    #[test]
    fn init_from_any_state() {
        // Chip already in command mode: 00 00 00 are commands, 40 resets.
        let mut u = Usart8251::new();
        ang_init(&mut u, 0xCD);
        ang_init(&mut u, 0xFA);
        assert_eq!(u.mode(), 0xFA);
        assert_eq!(u.command(), 0x05);
        // Chip that has a sync mode with a single sync char.
        let mut u = Usart8251::new();
        u.write_ctrl(0x80); // sync, single sync char
        u.write_ctrl(0x16);
        assert!(u.initialized());
        assert_eq!(u.clock_divider(), 1);
    }

    #[test]
    fn transmit_timing() {
        let mut u = Usart8251::new();
        ang_init(&mut u, 0xCD);
        u.set_char_cycles(300);
        u.write_data(b'H');
        let s = u.read_status();
        assert_eq!(s & 0x01, 0x01); // moved to shift register
        assert_eq!(s & 0x04, 0x00);
        u.write_data(b'i');
        assert_eq!(u.read_status() & 0x05, 0x00);
        u.tick(299);
        assert_eq!(u.take_tx(), None);
        u.tick(1);
        assert_eq!(u.take_tx(), Some(b'H'));
        assert_eq!(u.read_status() & 0x05, 0x01);
        u.tick(300);
        assert_eq!(u.take_tx(), Some(b'i'));
        assert_eq!(u.read_status() & 0x05, 0x05);
    }

    #[test]
    fn tx_needs_txen() {
        let mut u = Usart8251::new();
        u.write_ctrl(0xCD);
        u.write_ctrl(0x04); // RxEN only
        u.set_char_cycles(10);
        u.write_data(1);
        u.tick(100);
        assert_eq!(u.take_tx(), None);
        assert_eq!(u.read_status() & 1, 0);
        u.write_ctrl(0x05);
        u.tick(10);
        assert_eq!(u.take_tx(), Some(1));
    }

    #[test]
    fn receive_pacing() {
        let mut u = Usart8251::new();
        u.set_char_cycles(100);
        u.push_rx(b'a');
        u.push_rx(b'b');
        u.tick(10);
        assert_eq!(u.read_status() & 2, 0); // not initialised -> Rx disabled
        ang_init(&mut u, 0xCD);
        u.tick(1);
        assert_eq!(u.read_status() & 2, 2);
        assert_eq!(u.read_data(), b'a');
        u.tick(99);
        assert_eq!(u.read_status() & 2, 0);
        u.tick(1);
        assert_eq!(u.read_data(), b'b');
        assert_eq!(u.read_status() & 2, 0);
    }

    #[test]
    fn send_break_command() {
        let mut u = Usart8251::new();
        ang_init(&mut u, 0xCD);
        assert!(!u.break_active());
        u.write_ctrl(0x0D);
        assert!(u.break_active());
        u.write_ctrl(0x05);
        assert!(!u.break_active());
        u.write_ctrl(0x0D);
        u.write_ctrl(0x40);
        assert!(!u.break_active());
    }

    #[test]
    fn received_break() {
        let mut u = Usart8251::new();
        u.set_break_cycles(50);
        u.push_break();
        assert_eq!(u.read_status() & 0x40, 0x40);
        u.tick(50);
        assert_eq!(u.read_status() & 0x40, 0);
    }

    #[test]
    fn reset_returns_to_mode() {
        let mut u = Usart8251::new();
        ang_init(&mut u, 0xCD);
        u.reset();
        assert!(!u.initialized());
        u.write_ctrl(0x4E);
        u.write_ctrl(0x37);
        assert_eq!(u.clock_divider(), 16);
        assert_eq!(u.command(), 0x37);
    }
}
