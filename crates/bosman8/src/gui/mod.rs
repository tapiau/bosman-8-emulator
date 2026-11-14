//! Desktop GUI of the Bosman-8 emulator (eframe/egui), one window for all
//! modes ([`config::Mode`]): "full" (computer + terminal, the default),
//! "computer" (computer only) and "terminal" (ANG-3001 only).
//!
//! * terminal screen in the centre, its buttons and lamps in the bottom bar
//!   ([`terminal`]: emulator thread, keyboard, ARROWS drop-down);
//! * machine panel: Start/Stop, RESET (warm: CPU only), power cycle (cold),
//!   NMI, turbo, speed / uptime / latch status;
//! * floppy drives B: and C:: insert / eject / new blank image, write
//!   protect, activity LED, format check against the DIP jumpers;
//! * DIP jumpers of port 0x98 (applied on power cycle);
//! * serial lines SIO-A (terminal) and SIO-B (V.24): none / TCP listen /
//!   TCP connect / host serial port, re-connected at run time;
//! * Centronics output: file / `b8_link` endpoint / host printer device;
//! * monitor of the raw SIO-A output.
//!
//! The machine runs in its own thread ([`emu`]); settings are kept in a JSON
//! file ([`config`]).

mod app;
mod config;
mod emu;
pub mod i18n;
mod terminal;

use std::path::PathBuf;
use std::process::ExitCode;

pub use config::Mode;
use config::{AppConfig, PrinterKind};

/// Command line options relevant to the GUI. They override the saved settings.
struct GuiArgs {
    config: Option<PathBuf>,
    rom: Option<PathBuf>,
    term: Option<String>,
    aux: Option<String>,
    dip: Option<u8>,
    disk: [Option<PathBuf>; 2],
    printer: Option<PathBuf>,
    turbo: bool,
    terminal_rom: Option<PathBuf>,
    arrows: Option<ang3001::ArrowMode>,
    lang: Option<i18n::Lang>,
    typed: Vec<u8>,
}

fn parse_args(cli: &[String]) -> Result<GuiArgs, String> {
    let mut a = GuiArgs {
        config: None,
        rom: None,
        term: None,
        aux: None,
        dip: None,
        disk: [None, None],
        printer: None,
        turbo: false,
        terminal_rom: None,
        arrows: None,
        lang: None,
        typed: Vec::new(),
    };
    let mut it = cli.iter().cloned();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut val = || -> Result<String, String> {
            inline.clone().or_else(|| it.next()).ok_or_else(|| format!("{name} needs a value"))
        };
        match name.as_str() {
            "--config" => a.config = Some(val()?.into()),
            "--rom" => a.rom = Some(val()?.into()),
            "--term" => a.term = Some(val()?),
            "--aux" => a.aux = Some(val()?),
            "--dip" => {
                let s = val()?;
                let t = s.trim_start_matches("0x").trim_end_matches(['h', 'H']);
                a.dip = Some(u8::from_str_radix(t, 16).map_err(|_| format!("invalid --dip '{s}'"))?);
            }
            "--disk-b" => a.disk[0] = Some(val()?.into()),
            "--disk-c" => a.disk[1] = Some(val()?.into()),
            "--printer" => a.printer = Some(val()?.into()),
            "--turbo" => a.turbo = true,
            "--terminal-rom" => a.terminal_rom = Some(val()?.into()),
            "--type" => a.typed.extend(val()?.replace("\\r", "\r").bytes()),
            "--lang" => {
                let v = val()?;
                a.lang = Some(i18n::Lang::parse(&v).ok_or_else(|| format!("--lang: unknown language '{v}' (en, pl)"))?);
            }
            "--arrows" => {
                let v = val()?;
                a.arrows = Some(ang3001::ArrowMode::parse(&v).ok_or_else(|| format!("--arrows: unknown mode '{v}'"))?);
            }
            "--no-gui" => {}
            "--trace-io" | "--trace-aux" => eprintln!("bosman8: {name} is ignored in GUI mode (use --no-gui)"),
            "--trace-cpu-from" | "--max-seconds" => {
                val()?;
                eprintln!("bosman8: {name} is ignored in GUI mode (use --no-gui)");
            }
            _ => return Err(format!("unknown option '{arg}' (see --help)")),
        }
    }
    Ok(a)
}

/// Settings file of `mode` (`--config` wins), seeded from the old `gui.json`.
fn load_config(mode: Mode, explicit: Option<PathBuf>) -> (AppConfig, Option<PathBuf>) {
    let path = explicit.clone().or_else(|| config::path_for(mode));
    let legacy = if explicit.is_none() { config::legacy_path() } else { None };
    let cfg = match &path {
        Some(p) => AppConfig::load_for(mode, p, legacy.as_deref()).unwrap_or_else(|e| {
            eprintln!("bosman8: {e} (using defaults)");
            AppConfig::for_mode(mode)
        }),
        None => AppConfig::for_mode(mode),
    };
    (cfg, path)
}

/// Window icon: ship's wheel with an "8" (`packaging/icons/`, see render.sh).
fn icon() -> eframe::egui::IconData {
    const RGBA: &[u8] = include_bytes!("../../../../packaging/icons/bosman8-128.rgba");
    eframe::egui::IconData { rgba: RGBA.to_vec(), width: 128, height: 128 }
}

/// Interface language: `--lang`, else the settings file, else English.
fn apply_lang(cfg: &mut AppConfig, cli: Option<i18n::Lang>) {
    let l = cli.or_else(|| i18n::Lang::parse(&cfg.lang)).unwrap_or(i18n::Lang::En);
    i18n::set(l);
    cfg.lang = l.code().into();
}

/// Open the window (all modes).
fn open_window(app: app::App, title: &str, size: [f32; 2]) -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(title)
            .with_icon(icon())
            .with_app_id("bosman8")
            .with_inner_size(size)
            .with_min_inner_size([480.0, 360.0]),
        ..Default::default()
    };
    match eframe::run_native(title, options, Box::new(move |_cc| Ok(Box::new(app)))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bosman8: cannot open the window: {e} (try --no-gui / --headless)");
            ExitCode::FAILURE
        }
    }
}

/// Mode "terminal" with a window.
pub fn run_terminal(o: crate::terminal_cli::Opts) -> ExitCode {
    let (mut cfg, config_path) = load_config(Mode::Terminal, o.config.clone());
    if let Some(r) = &o.rom {
        cfg.terminal.rom = Some(r.clone());
    }
    if let Some(l) = &o.link {
        cfg.terminal.link.set_spec(l);
    }
    if let Some(c) = &o.color {
        cfg.terminal.color = c.clone();
    }
    if let Some(off) = o.line_offset {
        cfg.terminal.line_offset = off;
    }
    if let Some(a) = o.arrows {
        cfg.terminal.set_arrows(a);
    }
    if o.turbo {
        cfg.turbo = true;
    }
    apply_lang(&mut cfg, o.lang.as_deref().and_then(i18n::Lang::parse));
    let opts = app::TermOpts { base: o.cfg.clone(), typed: o.typed.clone(), inject: o.inject.clone() };
    let screen_h = 256.0 * o.scale as f32;
    let panel = if cfg.panel_open { 400.0 } else { 0.0 };
    let size = [screen_h * 4.0 / 3.0 + panel, screen_h + 40.0];
    open_window(app::App::new(Mode::Terminal, cfg, config_path, opts), "ANG-3001 (terminal)", size)
}

/// Modes "full" and "computer" with a window.
pub fn run(mode: Mode, cli: &[String]) -> ExitCode {
    let args = match parse_args(cli) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bosman8: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (mut cfg, config_path) = load_config(mode, args.config.clone());
    // Command line overrides.
    if args.rom.is_some() {
        cfg.rom = args.rom.clone();
    }
    if let Some(t) = &args.term {
        cfg.term.set_spec(t);
        if mode == Mode::Computer && cfg.term.kind == config::LinkKind::Terminal {
            eprintln!("bosman8: --term terminal works in mode full only");
            cfg.term = AppConfig::for_mode(Mode::Computer).term;
        }
    }
    if let Some(r) = &args.terminal_rom {
        cfg.terminal.rom = Some(r.clone());
    }
    if let Some(a) = args.arrows {
        cfg.terminal.set_arrows(a);
    }
    apply_lang(&mut cfg, args.lang);
    if let Some(t) = &args.aux {
        cfg.aux.set_spec(t);
    }
    if let Some(d) = args.dip {
        cfg.dip = d;
    }
    for (d, p) in args.disk.iter().enumerate() {
        if let Some(p) = p {
            if !p.exists() {
                let f = bosman8::disk::format_for(cfg.dip, d);
                eprintln!("bosman8: {} not found, creating a blank {} KB disk", p.display(), f.kbytes());
                if let Err(e) = std::fs::write(p, bosman8::disk::blank_file(p, f)) {
                    eprintln!("bosman8: {}: {e}", p.display());
                }
            }
            cfg.drives[d].path = Some(p.clone());
        }
    }
    if args.disk[1].is_some() && args.dip.is_none() {
        cfg.dip &= !0x40; // bit 6 = 0: the ROM initialises drive C: too
    }
    if let Some(p) = &args.printer {
        cfg.printer.kind = PrinterKind::File;
        cfg.printer.file = Some(p.clone());
        cfg.printer.append = true;
        cfg.printer.open = true;
    }
    if args.turbo {
        cfg.turbo = true;
    }

    let (title, size) = match mode {
        Mode::Full => ("Bosman-8", [1024.0 + if cfg.panel_open { 400.0 } else { 0.0 }, 820.0]),
        _ => ("Bosman-8 (computer)", [1100.0, 760.0]),
    };
    let opts = app::TermOpts { typed: args.typed, ..app::TermOpts::default() };
    open_window(app::App::new(mode, cfg, config_path, opts), title, size)
}

#[cfg(test)]
mod tests {
    use super::emu::{Cmd, Emu};
    use super::terminal::{TCmd, TermEmu};
    use super::*;
    use std::time::{Duration, Instant};

    fn screen(t: &TermEmu) -> String {
        let (tx, rx) = std::sync::mpsc::channel();
        t.send(TCmd::Text(tx));
        rx.recv_timeout(Duration::from_secs(5)).map(|l| l.join("\n")).unwrap_or_default()
    }

    fn wait_for(t: &TermEmu, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let s = screen(t);
            if s.contains(needle) || Instant::now() > deadline {
                return s;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Mode "full" without a window: the computer thread and the terminal
    /// thread joined by an in-memory pair, as the window does it.
    #[test]
    fn full_mode_computer_and_terminal_talk() {
        let mut cfg = AppConfig::for_mode(Mode::Full);
        cfg.turbo = true;
        let emu = Emu::start(bosman8::DEFAULT_ROM.to_vec(), &cfg);
        let (a, b) = b8_link::pair();
        emu.sender().send(Cmd::TermEndpoint(a)).unwrap();
        let term = TermEmu::start(ang3001::DEFAULT_ROM.to_vec(), ang3001::Config::default(), b, true);
        let s = wait_for(&term, "A>");
        assert!(s.contains("A>"), "no prompt on the terminal:\n{s}");
        assert!(s.contains("Mikrokomputer"), "no banner:\n{s}");
        term.send(TCmd::Type(b"DIR\r".to_vec()));
        let s = wait_for(&term, "Brak pliku");
        assert!(s.contains("A>DIR"), "typed command not echoed:\n{s}");
        assert!(s.contains("Brak pliku"), "no answer from the CCP:\n{s}");
    }
}
