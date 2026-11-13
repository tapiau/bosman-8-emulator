//! 74LS259 8-bit addressable latch as wired in Bosman-8 (and probably ANG-3001):
//! an OUT to port `p` (within the decoded block) sets bit `(p >> 1) & 7`
//! to `p & 1`. The data bus value is ignored. Reset clears all bits.

#[derive(Debug, Clone, Default)]
pub struct Latch259 {
    pub value: u8,
}

impl Latch259 {
    pub fn new() -> Self {
        Self { value: 0 }
    }

    pub fn reset(&mut self) {
        self.value = 0;
    }

    /// Handle OUT to `port` (only the low 4 bits are used).
    pub fn write(&mut self, port: u8) {
        let bit = (port >> 1) & 7;
        if port & 1 != 0 {
            self.value |= 1 << bit;
        } else {
            self.value &= !(1 << bit);
        }
    }

    pub fn bit(&self, n: u8) -> bool {
        n < 8 && self.value & (1 << n) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears() {
        let mut l = Latch259::new();
        l.write(0x0F);
        assert!(l.bit(7));
        l.reset();
        assert_eq!(l.value, 0);
    }

    #[test]
    fn addressing() {
        let mut l = Latch259::new();
        for b in 0..8u8 {
            l.write(b << 1 | 1);
            assert!(l.bit(b));
        }
        assert_eq!(l.value, 0xFF);
        l.write(0x06); // Q3 := 0
        assert_eq!(l.value, 0xF7);
        l.write(0x0E); // Q7 := 0
        assert!(!l.bit(7));
        assert_eq!(l.value, 0x77);
        assert!(!l.bit(8));
    }

    #[test]
    fn high_port_bits_ignored() {
        let mut l = Latch259::new();
        l.write(0xF5); // low nibble 5 -> Q2 := 1
        assert_eq!(l.value, 0x04);
        l.write(0xF4);
        assert_eq!(l.value, 0x00);
    }

    #[test]
    fn bosman_bank_select_sequence() {
        // F30F: bank bits 0,1,2 -> Q6, Q4, Q5. Bank 5 = 0b101.
        let mut l = Latch259::new();
        l.write(0x0D); // Q6 = 1 (bit0)
        l.write(0x08); // Q4 = 0 (bit1)
        l.write(0x0B); // Q5 = 1 (bit2)
        l.write(0x0F); // Q7 = 1 RAM
        assert!(l.bit(6) && !l.bit(4) && l.bit(5) && l.bit(7));
    }
}
