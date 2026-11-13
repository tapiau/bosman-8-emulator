//! 74LS259 8-bit addressable latch as wired in Bosman-8 (and probably ANG-3001):
//! an OUT to port `p` (within the decoded block) sets bit `(p >> 1) & 7`
//! to `p & 1`. The data bus value is ignored. Reset clears all bits.

#[derive(Debug, Clone, Default)]
pub struct Latch259 {
    pub value: u8,
}

impl Latch259 {
    pub fn new() -> Self { todo!() }
    pub fn reset(&mut self) { todo!() }
    /// Handle OUT to `port` (only the low 4 bits are used).
    pub fn write(&mut self, port: u8) { let _ = port; todo!() }
    pub fn bit(&self, n: u8) -> bool { let _ = n; todo!() }
}
