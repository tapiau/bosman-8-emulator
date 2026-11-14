//! Keyboard helpers shared by the front ends: what the arrow keys send
//! ([`ArrowMode`]) and how a typed character maps to a terminal key code.

use crate::POLISH_GLYPHS;

/// What the arrow keys (up, down, right, left) send.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArrowMode {
    /// Digits 8 / 2 / 6 / 4, like a numeric keypad (games: SOKOBAN, ...).
    Numpad,
    /// `ESC A` / `ESC B` / `ESC C` / `ESC D`.
    Vt52,
    /// WordStar: ^E / ^X / ^D / ^S.
    WordStar,
    /// vi / vim: k / j / l / h.
    Vim,
    /// Nothing.
    None,
}

impl ArrowMode {
    pub const ALL: [ArrowMode; 5] =
        [ArrowMode::Numpad, ArrowMode::Vt52, ArrowMode::WordStar, ArrowMode::Vim, ArrowMode::None];

    /// Bytes sent for up, down, right, left.
    pub fn keys(self) -> [&'static [u8]; 4] {
        match self {
            ArrowMode::Numpad => [b"8", b"2", b"6", b"4"],
            ArrowMode::Vt52 => [b"\x1bA", b"\x1bB", b"\x1bC", b"\x1bD"],
            ArrowMode::WordStar => [b"\x05", b"\x18", b"\x04", b"\x13"],
            ArrowMode::Vim => [b"k", b"j", b"l", b"h"],
            ArrowMode::None => [b"", b"", b"", b""],
        }
    }

    /// Short name shown on the bar (at most 4 characters).
    pub fn label(self) -> &'static [u8] {
        match self {
            ArrowMode::Numpad => b"2468",
            ArrowMode::Vt52 => b"VT52",
            ArrowMode::WordStar => b"WS",
            ArrowMode::Vim => b"VIM",
            ArrowMode::None => b"NONE",
        }
    }

    /// Entry in the drop-down list.
    pub fn describe(self) -> &'static [u8] {
        match self {
            ArrowMode::Numpad => b"2468      8 2 4 6",
            ArrowMode::Vt52 => b"VT52      ESC A B D C",
            ArrowMode::WordStar => b"WORDSTAR  ^E ^X ^S ^D",
            ArrowMode::Vim => b"VIM       k j h l",
            ArrowMode::None => b"NONE",
        }
    }

    /// Next mode (F6 cycles through them).
    pub fn next(self) -> ArrowMode {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Command-line name.
    pub fn parse(s: &str) -> Option<ArrowMode> {
        match s.to_ascii_lowercase().as_str() {
            "numpad" | "2468" => Some(ArrowMode::Numpad),
            "vt52" => Some(ArrowMode::Vt52),
            "wordstar" | "ws" => Some(ArrowMode::WordStar),
            "vim" | "vi" => Some(ArrowMode::Vim),
            "none" | "off" => Some(ArrowMode::None),
            _ => None,
        }
    }
}

impl Default for ArrowMode {
    fn default() -> Self {
        ArrowMode::Numpad
    }
}

/// Phosphor colour by name (`green`, `amber`, `white`) or `RRGGBB` (optionally `#RRGGBB`).
pub fn parse_color(s: &str) -> Result<u32, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "green" => Ok(crate::PHOSPHOR_GREEN),
        "amber" => Ok(crate::PHOSPHOR_AMBER),
        "white" => Ok(crate::PHOSPHOR_WHITE),
        h => {
            let h = h.trim_start_matches('#');
            u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6).ok_or_else(|| format!("bad colour '{s}'"))
        }
    }
}

/// Key code the terminal keyboard sends for a typed character: printable
/// ASCII as is, Polish lowercase letters as their font codes (a guess, see
/// [`POLISH_GLYPHS`]); `None` for anything else.
pub fn char_code(c: char) -> Option<u8> {
    if (' '..='~').contains(&c) {
        Some(c as u8)
    } else {
        POLISH_GLYPHS.iter().find(|&&(_, ch)| ch == c).map(|&(code, _)| code)
    }
}

/// Control code of Ctrl+`c` (letters and `[ \ ] ^ _ @ space 2 6 -`).
pub fn ctrl_code(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        l @ 'A'..='Z' => Some(l as u8 & 0x1F),
        '[' => Some(0x1B),
        '\\' => Some(0x1C),
        ']' => Some(0x1D),
        '^' | '6' => Some(0x1E),
        '_' | '-' => Some(0x1F),
        '@' | ' ' | '2' => Some(0x00),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_modes() {
        assert_eq!(ArrowMode::default(), ArrowMode::Numpad);
        assert_eq!(ArrowMode::Numpad.keys(), [&b"8"[..], b"2", b"6", b"4"]);
        assert_eq!(ArrowMode::Vim.keys(), [&b"k"[..], b"j", b"l", b"h"]);
        assert_eq!(ArrowMode::None.next(), ArrowMode::Numpad);
        assert_eq!(ArrowMode::parse("off"), Some(ArrowMode::None));
        assert!(ArrowMode::ALL.iter().all(|m| m.label().len() <= 4));
    }

    #[test]
    fn chars_and_ctrl() {
        assert_eq!(char_code('A'), Some(b'A'));
        assert_eq!(char_code('ł'), Some(0x03));
        assert_eq!(char_code('é'), None);
        assert_eq!(ctrl_code('c'), Some(0x03));
        assert_eq!(ctrl_code('['), Some(0x1B));
        assert_eq!(ctrl_code(' '), Some(0x00));
        assert_eq!(ctrl_code('1'), None);
        assert_eq!(parse_color("amber"), Ok(crate::PHOSPHOR_AMBER));
        assert_eq!(parse_color("#102030"), Ok(0x102030));
        assert!(parse_color("blue").is_err());
    }
}
