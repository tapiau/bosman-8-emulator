//! Desktop GUI of the Bosman-8 emulator (eframe/egui, default front end of
//! the `bosman8` binary; `--no-gui` runs the headless CLI instead).
//!
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

use std::path::PathBuf;
use std::process::ExitCode;

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
}

fn parse_args() -> Result<GuiArgs, String> {
    let mut a = GuiArgs {
        config: None,
        rom: None,
        term: None,
        aux: None,
        dip: None,
        disk: [None, None],
        printer: None,
        turbo: false,
    };
    let mut it = std::env::args().skip(1);
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

/// Run the GUI. Called from `main` unless `--no-gui` is given.
pub fn run() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bosman8: {e}");
            return ExitCode::FAILURE;
        }
    };
    let config_path = args.config.clone().or_else(config::default_path);
    let mut cfg = match &config_path {
        Some(p) => AppConfig::load(p).unwrap_or_else(|e| {
            eprintln!("bosman8: {e} (using defaults)");
            AppConfig::default()
        }),
        None => AppConfig::default(),
    };
    // Command line overrides.
    if args.rom.is_some() {
        cfg.rom = args.rom.clone();
    }
    if let Some(t) = &args.term {
        cfg.term.set_spec(t);
    }
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

    let app = app::App::new(cfg, config_path);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Bosman-8")
            .with_inner_size([780.0, 760.0])
            .with_min_inner_size([480.0, 400.0]),
        ..Default::default()
    };
    match eframe::run_native("Bosman-8", options, Box::new(move |_cc| Ok(Box::new(app)))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bosman8: cannot open the GUI window: {e} (try --no-gui)");
            ExitCode::FAILURE
        }
    }
}
