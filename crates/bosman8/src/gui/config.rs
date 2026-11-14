//! GUI settings: saved as JSON in the platform config directory, one file per
//! mode (`full.json`, `computer.json`, `terminal.json` in
//! `~/.config/bosman8/`, `%APPDATA%\bosman8\`,
//! `~/Library/Application Support/bosman8/`) or in `--config PATH`, so a
//! computer and a terminal window running at the same time don't overwrite
//! each other's settings. The `gui.json` of versions before 0.5 seeds the
//! files of the computer modes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Kind of a serial line endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    None,
    TcpListen,
    TcpConnect,
    Serial,
    /// Any other `b8_link::open` spec (e.g. `loopback`, `stdio` from the CLI).
    Spec,
    /// The terminal built into the window (mode "full", SIO-A only).
    Terminal,
}

pub use crate::Mode;

/// One serial line (SIO-A terminal / SIO-B V.24): all fields are kept so
/// switching the kind back restores the previous address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkCfg {
    pub kind: LinkKind,
    pub listen: String,
    pub connect: String,
    pub serial_dev: String,
    pub baud: u32,
    /// Character format of the serial port, e.g. `8N1`, `8N2`.
    pub format: String,
    /// Hardware flow control through the port's RTS / CTS lines.
    pub rtscts: bool,
    pub spec: String,
}

impl Default for LinkCfg {
    fn default() -> Self {
        Self {
            kind: LinkKind::None,
            listen: "127.0.0.1:3001".into(),
            connect: "127.0.0.1:3002".into(),
            serial_dev: String::new(),
            baud: 9600,
            format: "8N1".into(),
            rtscts: false,
            spec: "loopback".into(),
        }
    }
}

impl LinkCfg {
    /// `b8_link::open` specification.
    pub fn spec(&self) -> String {
        match self.kind {
            LinkKind::None => "none".into(),
            LinkKind::TcpListen => format!("tcp-listen:{}", self.listen.trim()),
            LinkKind::TcpConnect => format!("tcp:{}", self.connect.trim()),
            LinkKind::Serial => {
                let mut s = format!("serial:{}@{}", self.serial_dev.trim(), self.baud);
                let f = self.format.trim();
                if !f.is_empty() && !f.eq_ignore_ascii_case("8N1") {
                    s += &format!(",{}", f.to_ascii_uppercase());
                }
                if self.rtscts {
                    s += ",rtscts";
                }
                s
            }
            LinkKind::Spec => self.spec.trim().to_string(),
            LinkKind::Terminal => "terminal".into(),
        }
    }

    /// Set from a spec string (CLI `--term` / `--aux`).
    pub fn set_spec(&mut self, spec: &str) {
        let spec = spec.trim();
        let (kind, arg) = spec.split_once(':').unwrap_or((spec, ""));
        match kind.to_ascii_lowercase().as_str() {
            "none" | "null" => self.kind = LinkKind::None,
            "terminal" | "builtin" => self.kind = LinkKind::Terminal,
            "tcp-listen" => {
                self.kind = LinkKind::TcpListen;
                self.listen = arg.into();
            }
            "tcp" => {
                self.kind = LinkKind::TcpConnect;
                self.connect = arg.into();
            }
            "serial" => {
                self.kind = LinkKind::Serial;
                // DEV[@BAUD][,FORMAT][,rtscts]
                let mut parts = arg.split(',');
                let first = parts.next().unwrap_or_default();
                self.format = "8N1".into();
                self.rtscts = false;
                for opt in parts {
                    if opt.eq_ignore_ascii_case("rtscts") || opt.eq_ignore_ascii_case("crtscts") {
                        self.rtscts = true;
                    } else {
                        self.format = opt.to_ascii_uppercase();
                    }
                }
                match first.rsplit_once('@') {
                    Some((d, b)) if b.parse::<u32>().is_ok() => {
                        self.serial_dev = d.into();
                        self.baud = b.parse().unwrap_or(9600);
                    }
                    _ => self.serial_dev = first.into(),
                }
            }
            _ => {
                self.kind = LinkKind::Spec;
                self.spec = spec.into();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrinterKind {
    None,
    File,
    Link,
    Device,
}

/// Centronics output (port 0x98 OUT).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrinterCfg {
    pub kind: PrinterKind,
    pub file: Option<PathBuf>,
    pub append: bool,
    /// `b8_link` spec, e.g. `tcp:host:9100` or `serial:/dev/ttyUSB0@9600`.
    pub link: String,
    /// Host printer device, e.g. `/dev/usb/lp0`.
    pub device: String,
    /// Open the output at start-up.
    pub open: bool,
}

impl Default for PrinterCfg {
    fn default() -> Self {
        Self {
            kind: PrinterKind::None,
            file: None,
            append: true,
            link: "tcp:127.0.0.1:9100".into(),
            device: if cfg!(windows) { "LPT1".into() } else { "/dev/usb/lp0".into() },
            open: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DriveCfg {
    pub path: Option<PathBuf>,
    pub write_protect: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub rom: Option<PathBuf>,
    pub dip: u8,
    pub turbo: bool,
    /// B: and C:.
    pub drives: [DriveCfg; 2],
    pub term: LinkCfg,
    pub aux: LinkCfg,
    pub printer: PrinterCfg,
    pub show_monitor: bool,
    pub terminal: TermCfg,
    /// Side panel with the settings shown.
    pub panel_open: bool,
    /// Interface language: `en` (default) or `pl`.
    pub lang: String,
}

/// The ANG-3001 terminal (modes "full" and "terminal").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TermCfg {
    /// ROM image; `None` = `ang3001_rom` from /etc/bosman-8-emulator.conf, else the built-in ROM 90.09.11.
    pub rom: Option<PathBuf>,
    /// Line of the terminal in mode "terminal" (in "full" it is the built-in connection).
    pub link: LinkCfg,
    /// green | amber | white | RRGGBB.
    pub color: String,
    pub line_offset: u8,
    /// numpad | vt52 | wordstar | vim | none.
    pub arrows: String,
}

impl Default for TermCfg {
    fn default() -> Self {
        Self {
            rom: None,
            link: LinkCfg { kind: LinkKind::TcpConnect, connect: "127.0.0.1:3001".into(), ..LinkCfg::default() },
            color: "green".into(),
            line_offset: ang3001::DEFAULT_LINE_OFFSET,
            arrows: "numpad".into(),
        }
    }
}

impl TermCfg {
    pub fn arrows(&self) -> ang3001::ArrowMode {
        ang3001::ArrowMode::parse(&self.arrows).unwrap_or_default()
    }

    pub fn set_arrows(&mut self, a: ang3001::ArrowMode) {
        self.arrows = arrow_name(a).into();
    }

    /// Foreground colour 0x00RRGGBB.
    pub fn fg(&self) -> u32 {
        parse_color(&self.color).unwrap_or(ang3001::PHOSPHOR_GREEN)
    }
}

pub fn arrow_name(a: ang3001::ArrowMode) -> &'static str {
    match a {
        ang3001::ArrowMode::Numpad => "numpad",
        ang3001::ArrowMode::Vt52 => "vt52",
        ang3001::ArrowMode::WordStar => "wordstar",
        ang3001::ArrowMode::Vim => "vim",
        ang3001::ArrowMode::None => "none",
    }
}

pub use ang3001::parse_color;

impl Default for AppConfig {
    fn default() -> Self {
        let mut term = LinkCfg::default();
        term.kind = LinkKind::TcpListen;
        Self {
            rom: None,
            dip: 0x40,
            turbo: false,
            drives: Default::default(),
            term,
            aux: LinkCfg { listen: "127.0.0.1:3003".into(), ..LinkCfg::default() },
            printer: PrinterCfg::default(),
            show_monitor: false,
            terminal: TermCfg::default(),
            panel_open: true,
            lang: "en".into(),
        }
    }
}

impl AppConfig {
    /// Defaults for a mode: in "full" SIO-A is the built-in terminal.
    pub fn for_mode(mode: Mode) -> AppConfig {
        let mut c = AppConfig::default();
        if mode == Mode::Full {
            c.term.kind = LinkKind::Terminal;
        }
        c
    }

    /// Load the settings of `mode` from `path`. A missing file is seeded
    /// from `legacy` (the `gui.json` of older versions) if given, else the
    /// mode's defaults.
    pub fn load_for(mode: Mode, path: &Path, legacy: Option<&Path>) -> Result<AppConfig, String> {
        if path.exists() {
            return Self::load(path);
        }
        let mut c = match legacy.filter(|l| l.exists() && mode.has_computer()) {
            Some(l) => Self::load(l)?,
            None => return Ok(Self::for_mode(mode)),
        };
        // Old computer settings; in "full" SIO-A goes to the built-in terminal,
        // in "computer" the built-in terminal does not exist.
        match mode {
            Mode::Full => c.term.kind = LinkKind::Terminal,
            _ if c.term.kind == LinkKind::Terminal => c.term = AppConfig::default().term,
            _ => {}
        }
        Ok(c)
    }
}

/// Directory of the settings files.
pub fn config_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let dir = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    dir.map(|d| d.join("bosman8"))
}

/// Settings file of versions before 0.5 (single file for the computer GUI).
pub fn legacy_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("gui.json"))
}

/// Default settings file of `mode`.
pub fn path_for(mode: Mode) -> Option<PathBuf> {
    config_dir().map(|d| d.join(mode.file_name()))
}

/// Default settings file (old single-file name; kept for reference).
#[allow(dead_code)]
pub fn default_path() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let dir = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    dir.map(|d| d.join("bosman8").join("gui.json"))
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_roundtrip() {
        let mut l = LinkCfg::default();
        for s in [
            "none",
            "tcp-listen:127.0.0.1:3001",
            "tcp:host:23",
            "tcp:host:23,raw",
            "serial:/dev/ttyUSB0@19200",
            "serial:/dev/ttyUSB0@100000,8N2",
            "serial:/dev/ttyUSB0@100000,8N2,rtscts",
            "serial:COM4@9600,rtscts",
            "loopback",
        ] {
            l.set_spec(s);
            assert_eq!(l.spec(), s);
        }
        l.set_spec("serial:COM3");
        assert_eq!(l.spec(), "serial:COM3@9600", "options reset, baud kept");
        l.set_spec("serial:/dev/ttyUSB0@100000,8n2");
        assert_eq!((l.serial_dev.as_str(), l.baud, l.format.as_str()), ("/dev/ttyUSB0", 100000, "8N2"));
    }

    #[test]
    fn modes_and_migration() {
        assert_eq!(Mode::parse("full"), Some(Mode::Full));
        assert_eq!(Mode::parse("komplet"), None);
        assert_eq!(Mode::parse("Computer"), Some(Mode::Computer));
        assert_eq!(Mode::parse("terminal"), Some(Mode::Terminal));
        assert_eq!(Mode::parse("x"), None);
        assert_eq!(AppConfig::for_mode(Mode::Full).term.kind, LinkKind::Terminal);
        assert_eq!(AppConfig::for_mode(Mode::Computer).term.spec(), "tcp-listen:127.0.0.1:3001");
        let mut l = LinkCfg::default();
        l.set_spec("terminal");
        assert_eq!(l.spec(), "terminal");

        let dir = std::env::temp_dir().join(format!("b8cfg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let legacy = dir.join("gui.json");
        let mut old = AppConfig::default();
        old.dip = 0x08;
        old.term.set_spec("serial:/dev/ttyUSB0@100000,8N2");
        old.save(&legacy).unwrap();
        // "computer" keeps the old SIO-A, "full" switches it to the built-in terminal
        let c = AppConfig::load_for(Mode::Computer, &dir.join("computer.json"), Some(&legacy)).unwrap();
        assert_eq!((c.dip, c.term.spec().as_str()), (0x08, "serial:/dev/ttyUSB0@100000,8N2"));
        let c = AppConfig::load_for(Mode::Full, &dir.join("full.json"), Some(&legacy)).unwrap();
        assert_eq!((c.dip, c.term.kind), (0x08, LinkKind::Terminal));
        // the terminal mode does not inherit computer settings
        let c = AppConfig::load_for(Mode::Terminal, &dir.join("terminal.json"), Some(&legacy)).unwrap();
        assert_eq!(c.terminal.link.spec(), "tcp:127.0.0.1:3001");
        // an existing file wins; a file without the new fields gets their defaults
        std::fs::write(dir.join("full.json"), r#"{"dip": 64}"#).unwrap();
        let c = AppConfig::load_for(Mode::Full, &dir.join("full.json"), Some(&legacy)).unwrap();
        assert_eq!((c.dip, c.terminal.arrows(), c.panel_open), (64, ang3001::ArrowMode::Numpad, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_roundtrip() {
        let mut c = AppConfig::default();
        c.drives[1].path = Some("c.img".into());
        c.printer.kind = PrinterKind::Device;
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<AppConfig>(&s).unwrap(), c);
        // Missing fields take defaults.
        let p: AppConfig = serde_json::from_str(r#"{"dip": 0}"#).unwrap();
        assert_eq!(p.dip, 0);
        assert_eq!(p.term.kind, LinkKind::TcpListen);
    }
}
