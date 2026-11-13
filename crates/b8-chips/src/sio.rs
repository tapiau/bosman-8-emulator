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

#[derive(Debug, Clone)]
pub struct SioChannel { /* private */ _p: () }

impl SioChannel {
    pub fn write_ctrl(&mut self, v: u8) { let _ = v; todo!() }
    pub fn read_ctrl(&mut self) -> u8 { todo!() }
    pub fn write_data(&mut self, v: u8) { let _ = v; todo!() }
    pub fn read_data(&mut self) -> u8 { todo!() }
    /// Byte transmitted by the CPU, ready to be sent to the endpoint.
    pub fn take_tx(&mut self) -> Option<u8> { todo!() }
    /// Byte received from the endpoint.
    pub fn push_rx(&mut self, b: u8) { let _ = b; todo!() }
    /// Mark that a break condition was received.
    pub fn push_break(&mut self) { todo!() }
    pub fn set_char_cycles(&mut self, cycles: u32) { let _ = cycles; todo!() }
    /// Clock divider selected in WR4 bits 6-7 (1, 16, 32, 64); 1 in sync modes.
    pub fn clock_divider(&self) -> u32 { todo!() }
    /// Number of data bits per character for Tx (from WR5), used for timing.
    pub fn bits_per_char(&self) -> u32 { todo!() }
}

#[derive(Debug, Clone)]
pub struct Sio { pub a: SioChannel, pub b: SioChannel }

impl Sio {
    pub fn new() -> Self { todo!() }
    pub fn reset(&mut self) { todo!() }
    pub fn tick(&mut self, cpu_cycles: u32) { let _ = cpu_cycles; todo!() }
}
