//! System-wide configuration file shared by the emulator programs:
//! `/etc/bosman-8-emulator.conf` (installed by the Debian package).
//!
//! Format: `key = value` lines, `#` starts a comment, blank lines ignored.
//! Keys used now:
//!
//! | key           | meaning                              |
//! |---------------|--------------------------------------|
//! | `bosman8_rom` | Bosman-8 EPROM image (32/64 KB)      |
//! | `ang3001_rom` | ANG-3001 EPROM image (16 KB)         |
//!
//! A missing file or key is not an error — callers fall back to their defaults.

use std::path::PathBuf;

/// Location of the system configuration file.
pub const CONF_PATH: &str = "/etc/bosman-8-emulator.conf";

/// Parse configuration text; returns the value of `key`, if present.
pub fn lookup(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Value of `key` from [`CONF_PATH`], if the file exists and defines it.
pub fn get(key: &str) -> Option<String> {
    lookup(&std::fs::read_to_string(CONF_PATH).ok()?, key)
}

/// Path-valued `key` from [`CONF_PATH`].
pub fn get_path(key: &str) -> Option<PathBuf> {
    get(key).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::lookup;

    #[test]
    fn parses_keys_comments_and_blanks() {
        let t = "# comment\n\n bosman8_rom = /a/b.bin  # trailing\nang3001_rom=\nother=x=y\n";
        assert_eq!(lookup(t, "bosman8_rom").as_deref(), Some("/a/b.bin"));
        assert_eq!(lookup(t, "ang3001_rom"), None);
        assert_eq!(lookup(t, "other").as_deref(), Some("x=y"));
        assert_eq!(lookup(t, "missing"), None);
    }
}
