//! Intel 8253 programmable interval timer (3 counters + control word).
//!
//! Must support: control word (counter select, RL modes LSB/MSB/LSB+MSB/latch,
//! modes 0..5, BCD), counter read-back including latch command, counting in
//! time. The counters are clocked by `input_clock_hz`, the chip is ticked with
//! CPU cycles at `cpu_clock_hz`; the chip handles the fractional ratio.
//! Gate inputs are assumed high. Counter 0 of ANG-3001 may be clocked by the
//! output of counter 1 — supported through [`Pit8253::set_cascade_0_from_1`].

#[derive(Debug, Clone)]
pub struct Pit8253 { /* private state */ _p: () }

impl Pit8253 {
    pub fn new(input_clock_hz: u32, cpu_clock_hz: u32) -> Self { let _ = (input_clock_hz, cpu_clock_hz); todo!() }
    pub fn reset(&mut self) { todo!() }
    /// `reg`: 0..=2 counters, 3 control word.
    pub fn write(&mut self, reg: u8, value: u8) { let _ = (reg, value); todo!() }
    pub fn read(&mut self, reg: u8) -> u8 { let _ = reg; todo!() }
    pub fn tick(&mut self, cpu_cycles: u32) { let _ = cpu_cycles; todo!() }
    /// Programmed reload value of a counter (0 means 65536 / 10000 BCD), if loaded.
    pub fn reload(&self, counter: usize) -> Option<u32> { let _ = counter; todo!() }
    /// Output frequency of a counter in Hz (input clock / reload), if running.
    pub fn output_hz(&self, counter: usize) -> Option<f64> { let _ = counter; todo!() }
    /// Current OUT pin level of a counter.
    pub fn out(&self, counter: usize) -> bool { let _ = counter; todo!() }
    pub fn set_cascade_0_from_1(&mut self, on: bool) { let _ = on; todo!() }
}
