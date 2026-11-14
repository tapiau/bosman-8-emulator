//! `bosman8` — front end of the Bosman-8 emulator: GUI window (feature `gui`, default) or headless CLI (`--no-gui`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bosman8::{disk, Bosman8, Config, CPU_HZ};

#[cfg(feature = "gui")]
mod gui;

const USAGE: &str = "\
usage: bosman8 [options]
  --no-gui               headless (command line) mode; without it a GUI window
                         opens and the options below pre-set its settings
  --config PATH          GUI settings file (default: <config dir>/bosman8/gui.json)
  --rom PATH             ROM image (default: bosman8_rom from /etc/bosman-8-emulator.conf,
                         else roms/bosman8/bosman8.bin here or next to the executable,
                         else the built-in 32 KB CPM-R ROM)
  --term SPEC            terminal line, SIO-A (default tcp-listen:127.0.0.1:3001)
  --aux SPEC             V.24 line, SIO-B (default none)
                         SPEC: none | loopback | stdio | tcp-listen:ADDR:PORT |
                               tcp:HOST:PORT[,raw] | serial:DEV[@BAUD][,8N2]
                         (TCP between the emulators carries RTS/CTS: the
                         terminal paces SIO-A; ,raw = plain byte stream)
  --dip HEX              jumper bits of port 0x98 (bits 1,3,5,6 used; default 40,
                         00 when --disk-c is given). Bits 3/5 select the floppy
                         format: 00 = B:200K C:200K, 08 = 400K/400K,
                         20 = 800K/800K, 28 = B:800K C:200K
  --disk-b IMG           floppy image for drive B: raw sectors or Gotek .hfe
                         (created blank if missing, written back as it runs)
  --disk-c IMG           same for drive C:
  --printer FILE         append Centronics output to FILE
  --turbo                don't synchronise with the wall clock
  --max-seconds N        stop after N emulated seconds
  --trace-io             log I/O port accesses (first 16 per port) to stderr
  --trace-cpu-from ADDR  trace instructions from PC=ADDR (hex) to stderr
  --fixed-baud           keep host serial ports at the speed/format given in
                         serial:DEV@BAUD[,FMT] (SIO-A terminal line: ,8N2)
                         (default: follow the speed/format the guest programs
                         into the SIO, e.g. 9600 -> 20833 bit/s of the D/E/F link)
  --trace-aux            log bytes sent (->) and received (<-) on SIO-B to stderr,
                         with the emulated time in seconds
  -h, --help             this help";

struct Args {
    rom: Option<PathBuf>,
    term: String,
    aux: String,
    dip: u8,
    dip_explicit: bool,
    turbo: bool,
    trace_io: bool,
    trace_aux: bool,
    fixed_baud: bool,
    trace_cpu_from: Option<u16>,
    disk_b: Option<PathBuf>,
    disk_c: Option<PathBuf>,
    printer: Option<PathBuf>,
    max_seconds: Option<f64>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        rom: None,
        term: "tcp-listen:127.0.0.1:3001".into(),
        aux: "none".into(),
        dip: 0x40,
        dip_explicit: false,
        turbo: false,
        trace_io: false,
        trace_aux: false,
        fixed_baud: false,
        trace_cpu_from: None,
        disk_b: None,
        disk_c: None,
        printer: None,
        max_seconds: None,
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
        let hex = |s: String| -> Result<u32, String> {
            let t = s.trim_start_matches("0x").trim_end_matches(['h', 'H']);
            u32::from_str_radix(t, 16).map_err(|_| format!("invalid hex value '{s}'"))
        };
        match name.as_str() {
            "--rom" => a.rom = Some(val()?.into()),
            "--term" => a.term = val()?,
            "--aux" => a.aux = val()?,
            "--dip" => {
                a.dip = hex(val()?)?.try_into().map_err(|_| "--dip must be 00..FF")?;
                a.dip_explicit = true;
            }
            "--turbo" => a.turbo = true,
            "--trace-io" => a.trace_io = true,
            "--trace-aux" => a.trace_aux = true,
            "--fixed-baud" => a.fixed_baud = true,
            "--trace-cpu-from" => {
                a.trace_cpu_from = Some(hex(val()?)?.try_into().map_err(|_| "address must be 0000..FFFF")?)
            }
            "--disk-b" => a.disk_b = Some(val()?.into()),
            "--disk-c" => a.disk_c = Some(val()?.into()),
            "--printer" => a.printer = Some(val()?.into()),
            "--max-seconds" => {
                let v = val()?;
                a.max_seconds = Some(v.parse().map_err(|_| format!("invalid number '{v}'"))?)
            }
            "--no-gui" => {}
            "--config" => drop(val()?), // GUI only
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }
    Ok(a)
}

fn find_rom(explicit: Option<&Path>) -> Result<(PathBuf, Vec<u8>), String> {
    let mut candidates = Vec::new();
    match explicit {
        Some(p) => candidates.push(p.to_path_buf()),
        None => {
            // System configuration (Debian package): /etc/bosman-8-emulator.conf, key bosman8_rom.
            if let Some(p) = b8_link::sysconf::get_path("bosman8_rom") {
                candidates.push(p);
            }
            // roms/<machine>/ since 0.4.1; the flat roms/bosman8.bin of older layouts is still accepted.
            candidates.push(PathBuf::from("roms/bosman8/bosman8.bin"));
            candidates.push(PathBuf::from("roms/bosman8.bin"));
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    candidates.push(dir.join("bosman8.bin"));
                    candidates.push(dir.join("roms/bosman8/bosman8.bin"));
                    candidates.push(dir.join("roms/bosman8.bin"));
                    // Installed layout (Debian package, `make install` style): <prefix>/bin + <prefix>/share
                    candidates.push(dir.join("../share/bosman8-emulator/roms/bosman8/bosman8.bin"));
                    candidates.push(dir.join("../share/bosman8-emulator/roms/bosman8.bin"));
                }
            }
        }
    }
    for c in &candidates {
        if let Ok(data) = std::fs::read(c) {
            return Ok((c.clone(), data));
        }
    }
    if explicit.is_none() {
        // No file around (e.g. started from another directory): the built-in 32 KB ROM.
        return Ok((PathBuf::from("(built-in roms/bosman8/bosman8.bin)"), bosman8::DEFAULT_ROM.to_vec()));
    }
    Err(format!(
        "cannot read ROM (tried {})",
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    ))
}

fn load_disk(path: &Path, format: disk::Format) -> Result<(b8_chips::DiskImage, disk::Container), String> {
    match std::fs::read(path) {
        Ok(data) => {
            let o = disk::open_bytes(data, format).map_err(|e| format!("{}: {e}", path.display()))?;
            if let Some(w) = &o.warning {
                eprintln!("bosman8: warning: {}: {w}", path.display());
            }
            Ok((o.image, o.container))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("bosman8: {} not found, creating a blank {} KB disk", path.display(), format.kbytes());
            let file = disk::blank_file(path, format);
            std::fs::write(path, &file).map_err(|e| format!("{}: {e}", path.display()))?;
            let o = disk::open_bytes(file, format)?;
            Ok((o.image, o.container))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn run() -> Result<(), String> {
    let mut args = parse_args()?;
    if args.disk_c.is_some() && args.dip & 0x40 != 0 {
        if args.dip_explicit {
            eprintln!("bosman8: warning: DIP bit 6 is set, the ROM won't initialise drive C:");
        } else {
            args.dip &= !0x40; // bit 6 = 0: the ROM initialises drive C: too
        }
    }
    let (rom_path, rom) = find_rom(args.rom.as_deref())?;
    let term = b8_link::open(&args.term).map_err(|e| format!("--term: {e}"))?;
    let aux = b8_link::open(&args.aux).map_err(|e| format!("--aux: {e}"))?;
    let config = Config {
        dip: args.dip,
        trace_io: args.trace_io,
        trace_aux: args.trace_aux,
        follow_line_params: !args.fixed_baud,
        trace_cpu_from: args.trace_cpu_from,
        fast_fdc: args.turbo,
        ..Config::default()
    };
    let mut m = Bosman8::new(&rom, config, term, aux);
    let disks = [(0usize, &args.disk_b), (1, &args.disk_c)];
    let mut containers: Vec<Option<disk::Container>> = vec![None, None];
    for (i, (drive, path)) in disks.into_iter().enumerate() {
        if let Some(p) = path {
            let format = disk::format_for(args.dip, drive);
            let (img, container) = load_disk(p, format)?;
            m.insert_disk(drive, Some(img));
            containers[i] = Some(container);
        }
    }
    let mut printer = match &args.printer {
        Some(p) => Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| format!("--printer {}: {e}", p.display()))?,
        ),
        None => None,
    };
    eprintln!(
        "bosman8: ROM {} | term (SIO-A): {} | aux (SIO-B): {} | DIP {:02X}{}",
        rom_path.display(),
        m.term_describe(),
        m.aux_describe(),
        args.dip,
        if args.turbo { " | turbo" } else { "" }
    );

    let slice = (CPU_HZ / 50) as u64;
    let max_cycles = args.max_seconds.map(|s| (s * CPU_HZ as f64) as u64);
    let start = Instant::now();
    let mut emulated = Duration::ZERO;
    let mut stderr = std::io::stderr();
    // Floppy contents as last written to the image files.
    let mut saved: Vec<Option<Vec<u8>>> = disks.iter().map(|&(d, _)| m.disk(d).map(|x| x.data.clone())).collect();
    let mut save_disks = |m: &Bosman8| -> Result<(), String> {
        for (i, &(drive, path)) in disks.iter().enumerate() {
            if let (Some(p), Some(d), Some(c)) = (path, m.disk(drive), containers[i].as_mut()) {
                if saved[i].as_deref() != Some(&d.data[..]) {
                    std::fs::write(p, c.to_bytes(d)).map_err(|e| format!("{}: {e}", p.display()))?;
                    saved[i] = Some(d.data.clone());
                }
            }
        }
        Ok(())
    };
    let mut slices = 0u64;
    loop {
        m.run_cycles(slice);
        slices += 1;
        if slices % 50 == 0 {
            save_disks(&m)?; // about once per emulated second
        }
        for line in m.take_log() {
            let _ = writeln!(stderr, "{line}");
        }
        let out = m.take_printer();
        if !out.is_empty() {
            if let Some(f) = printer.as_mut() {
                f.write_all(&out).map_err(|e| format!("printer: {e}"))?;
                let _ = f.flush();
            }
        }
        if max_cycles.is_some_and(|mc| m.cycles() >= mc) {
            break;
        }
        if !args.turbo {
            emulated += Duration::from_millis(20);
            let now = start.elapsed();
            if emulated > now {
                std::thread::sleep(emulated - now);
            } else if now - emulated > Duration::from_millis(500) {
                emulated = now; // fell behind: don't try to catch up
            }
        }
    }
    save_disks(&m)?;
    // Give background endpoint threads a moment to flush.
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

fn main() -> ExitCode {
    let gui = !std::env::args().skip(1).any(|a| matches!(a.as_str(), "--no-gui" | "-h" | "--help"));
    #[cfg(feature = "gui")]
    if gui {
        return gui::run();
    }
    #[cfg(not(feature = "gui"))]
    if gui {
        eprintln!("bosman8: built without the GUI (feature \"gui\"), running headless");
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bosman8: {e}");
            ExitCode::FAILURE
        }
    }
}
