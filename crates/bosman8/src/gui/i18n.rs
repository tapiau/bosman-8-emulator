//! Interface language of the window: English (default) or Polish, chosen
//! with `--lang` or `"lang"` in the settings file, switchable at run time in
//! the side panel. Every text is written in place as an English / Polish
//! pair ([`t`], [`tf!`](crate::tf)) — with two languages that keeps each
//! translation next to where it is used.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Pl,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Pl];

    /// `en` / `pl` (also `english`, `polski`, `pl_PL…`), case-insensitive.
    pub fn parse(s: &str) -> Option<Lang> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "en" | "english" => Some(Lang::En),
            "pl" | "polski" | "polish" => Some(Lang::Pl),
            _ if s.starts_with("en_") || s.starts_with("en-") => Some(Lang::En),
            _ if s.starts_with("pl_") || s.starts_with("pl-") => Some(Lang::Pl),
            _ => None,
        }
    }

    /// Code stored in the settings file.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Pl => "pl",
        }
    }

    /// Name shown in the language selector (in that language).
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Pl => "Polski",
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::Pl as u8 {
        Lang::Pl
    } else {
        Lang::En
    }
}

pub fn is_pl() -> bool {
    get() == Lang::Pl
}

/// The English or the Polish text, per the current language.
pub fn t(en: &'static str, pl: &'static str) -> &'static str {
    if is_pl() {
        pl
    } else {
        en
    }
}

/// `format!` with an English and a Polish format string.
#[macro_export]
macro_rules! tf {
    ($en:literal, $pl:literal $(, $arg:expr)* $(,)?) => {
        if $crate::gui::i18n::is_pl() { format!($pl $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_switch() {
        assert_eq!(Lang::parse("PL"), Some(Lang::Pl));
        assert_eq!(Lang::parse("pl_PL.UTF-8"), Some(Lang::Pl));
        assert_eq!(Lang::parse("en"), Some(Lang::En));
        assert_eq!(Lang::parse("de"), None);
        set(Lang::Pl);
        let n = 3;
        assert_eq!(t("Insert", "Włóż"), "Włóż");
        assert_eq!(crate::tf!("{n} files", "{n} pliki"), "3 pliki");
        set(Lang::En);
        assert_eq!(t("Insert", "Włóż"), "Insert");
        assert_eq!(crate::tf!("{n} files", "{n} pliki"), "3 files");
    }
}
