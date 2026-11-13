//! Intel 8251 USART, byte-level model.
//!
//! Control port: after reset expects a mode instruction, then (sync mode) one
//! or two sync characters, then command instructions. Command bit 6 = internal
//! reset (back to expecting mode). The usual init `00 00 00 40` must work.
//! Command: bit0 TxEN, bit2 RxEN, bit3 send break, bit4 error reset.
//! Status: bit0 TxRDY, bit1 RxRDY, bit2 TxEMPTY, bit3 PE, bit4 OE, bit5 FE,
//! bit6 SYNDET, bit7 DSR (report 1).
//! Timing/queues: same contract as `SioChannel` (char_cycles, take_tx, push_rx).

#[derive(Debug, Clone)]
pub struct Usart8251 { /* private */ _p: () }

impl Usart8251 {
    pub fn new() -> Self { todo!() }
    pub fn reset(&mut self) { todo!() }
    pub fn write_ctrl(&mut self, v: u8) { let _ = v; todo!() }
    pub fn read_status(&mut self) -> u8 { todo!() }
    pub fn write_data(&mut self, v: u8) { let _ = v; todo!() }
    pub fn read_data(&mut self) -> u8 { todo!() }
    pub fn tick(&mut self, cpu_cycles: u32) { let _ = cpu_cycles; todo!() }
    pub fn take_tx(&mut self) -> Option<u8> { todo!() }
    pub fn push_rx(&mut self, b: u8) { let _ = b; todo!() }
    /// True while the CPU has the "send break" command bit set.
    pub fn break_active(&self) -> bool { todo!() }
    pub fn set_char_cycles(&mut self, cycles: u32) { let _ = cycles; todo!() }
    /// Baud rate factor from the mode instruction (1, 16, 64; 1 for sync).
    pub fn clock_divider(&self) -> u32 { todo!() }
}
