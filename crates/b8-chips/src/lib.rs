//! Peripheral chips shared by the Bosman-8 and ANG-3001 emulators.
//!
//! All chips are pure logic: no threads, no I/O, no wall clock. Time advances
//! only through `tick(cycles)` calls with CPU cycles (each chip is told the CPU
//! clock or its own input clock at construction). Serial chips don't own an
//! endpoint: the machine moves bytes between a chip channel's
//! `take_tx()` / `push_rx()` and a `b8_link::Endpoint`. This keeps the crate
//! WASM-friendly and testable.

pub mod latch;
pub mod pit8253;
pub mod sio;
pub mod usart8251;
pub mod wd1770;

pub use latch::Latch259;
pub use pit8253::Pit8253;
pub use sio::{Sio, SioChannel};
pub use usart8251::Usart8251;
pub use wd1770::{DiskImage, Wd1770, Wd1770Timing};
