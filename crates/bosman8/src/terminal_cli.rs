//! Mode "terminal": options (those of the former `ang3001` program, which
//! is now this mode — also when the binary is called as `ang3001`) and the
//! run without a window (`--headless`: screenshots, text dump, injected data).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ang3001::{Ang3001, ArrowMode, Config};

pub const HELP: &str = "\
bosman8 terminal - ANG-3001 graphics terminal (Z80B, 512x256 mono)
(also: ang3001 [OPTIONS])

USAGE: bosman8 terminal [OPTIONS]

OPTIONS:
  --rom FILE          ROM image (default: ang3001_rom from /etc/bosman-8-emulator.conf,
                      else the built-in ROM 90.09.11; the older one is
                      roms/ang3001/ANG3001_ZJ_wersjaPG.BIN)
  --link SPEC         serial line endpoint (default: tcp:127.0.0.1:3001 — the
                      computer started with \"bosman8 computer\")
                      none | loopback | stdio | tcp:HOST:PORT[,raw] | tcp-listen:ADDR:PORT
                      | serial:DEV[@BAUD][,8N2][,rtscts]; the terminal drops RTS when
                      it can't keep up: over TCP via RFC 2217 (,raw = without it),
                      on a serial port with ,rtscts on the port's RTS line
  --cpu-mhz F         CPU clock in MHz (default 6)
  --pit-mhz F         8253 input clock in MHz (default 1.3)
  --char-cycles N     fixed serial character time in CPU cycles
                      (default: derived from 8253 counter 2 and the 8251 mode)
  --line-offset N     memory line (relative to the scroll register) shown at the
                      top of the screen (default 0: status band on lines 0-15,
                      text rows from line 16; 16 = text row 0 on the top line)
  --color C           green | amber | white | RRGGBB (default green)
  --scale N           window size: screen height 256*N (default 3)
  --turbo             run as fast as possible
  --arrows MODE       what the arrow keys up/down/right/left send (also the
                      ARROWS drop-down on the bar, F6 = next mode):
                      numpad (8 2 6 4, default - games such as SOKOBAN),
                      vt52 (ESC A B C D), wordstar (^E ^X ^D ^S),
                      vim (k j l h) or none / off (arrow keys send nothing)
  --inject FILE       feed FILE into the serial input after boot
  --load-ter FILE     upload a terminal binary (org 4000h) via ESC i and run it
                      with ESC 0 after boot (like RUNTERM.COM)
  --type TEXT         type TEXT on the keyboard after boot (\\r = Enter)
  --fixed-baud        keep a serial:DEV@BAUD[,FMT] link at that speed and format
                      (default: follow the speed/format the ROM programs into the
                      8251, 100000 8N2; with --fixed-baud add ,8N2 yourself)
  --config PATH       settings file of the window (default: <config dir>/bosman8/terminal.json)
  --lang LANG         interface language: en (default) or pl (also \"lang\" in the settings
                      file and the selector in the side panel)
  --trace-io          log accesses to unknown ports (and 0x0A/0x0B) to stderr
  --headless          no window; runs unpaced unless --realtime
  --realtime          pace headless mode to real time
  --after SECS        headless: stop after SECS of emulated time (default 3)
  --screenshot FILE   headless: write a PGM (P5) screenshot when stopping
  --dump-text         headless: print the decoded text screen when stopping
  -h, --help          this help

KEYS (window):
  printable keys      ASCII as typed (host keyboard layout is honoured);
                      Polish lowercase letters send their font codes
                      (ę=01 ź=02 ł=03 ń=04 ś=05 ą=06 ó=07 ż=0B ć=0C, a guess)
  Enter / Backspace / Tab / Esc / Delete  ->  0x0D / 0x08 / 0x09 / 0x1B / 0x7F
  Ctrl+letter         control code (Ctrl+A = 0x01 ... Ctrl+Z = 0x1A)
  Ctrl+[ \\ ] ^ _      0x1B 0x1C 0x1D 0x1E 0x1F
  arrows              see --arrows (default 8 / 2 / 6 / 4 for up/down/right/left)
  F6                  next arrow-key mode (shown on the ARROWS drop-down)
  F7                  LOCAL on/off   F8, Scroll Lock  STOP (paging) on/off
  F9                  turbo          F10              CAPS
  F11                 reset terminal (CPU and chips; RAM is kept)
  F12 (hold)          BREAK key
  The buttons below the screen do the same; settings (line, ROM, colour) are in
  the side panel (button with the gear).
";

/// Options of mode "terminal".
#[derive(Clone)]
pub struct Opts {
    pub rom: Option<PathBuf>,
    pub link: Option<String>,
    pub cfg: Config,
    pub color: Option<String>,
    pub line_offset: Option<u8>,
    pub scale: usize,
    pub turbo: bool,
    pub arrows: Option<ArrowMode>,
    pub inject: Vec<u8>,
    pub typed: Vec<u8>,
    pub config: Option<PathBuf>,
    /// Interface language of the window (`en`, `pl`).
    pub lang: Option<String>,
    pub headless: bool,
    pub realtime: bool,
    pub after: f64,
    pub screenshot: Option<PathBuf>,
    pub dump_text: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            rom: None,
            link: None,
            cfg: Config::default(),
            color: None,
            line_offset: None,
            scale: 3,
            turbo: false,
            arrows: None,
            inject: Vec::new(),
            typed: Vec::new(),
            config: None,
            lang: None,
            headless: false,
            realtime: false,
            after: 3.0,
            screenshot: None,
            dump_text: false,
        }
    }
}

/// Parse the options of mode "terminal" (`Ok(None)` = help printed).
pub fn parse(args: &[String]) -> Result<Option<Opts>, String> {
    let mut o = Opts::default();
    let mut it = args.iter().cloned();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        fn num<T: std::str::FromStr>(name: &str, v: String) -> Result<T, String> {
            v.parse().map_err(|_| format!("{name}: bad number '{v}'"))
        }
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "--rom" | "--terminal-rom" => o.rom = Some(val(&a)?.into()),
            "--link" => o.link = Some(val(&a)?),
            "--cpu-mhz" => o.cfg.cpu_hz = (num::<f64>(&a, val(&a)?)? * 1e6) as u32,
            "--pit-mhz" => o.cfg.pit_hz = (num::<f64>(&a, val(&a)?)? * 1e6) as u32,
            "--char-cycles" => o.cfg.char_cycles = Some(num(&a, val(&a)?)?),
            "--line-offset" => {
                let v = num(&a, val(&a)?)?;
                o.cfg.line_offset = v;
                o.line_offset = Some(v);
            }
            "--color" => {
                let c = val(&a)?;
                o.cfg.fg = ang3001::parse_color(&c)?;
                o.color = Some(c);
            }
            "--scale" => o.scale = num::<usize>(&a, val(&a)?)?.clamp(1, 4),
            "--turbo" => o.turbo = true,
            "--arrows" => {
                let v = val(&a)?;
                o.arrows = Some(
                    ArrowMode::parse(&v)
                        .ok_or_else(|| format!("--arrows: unknown mode '{v}' (numpad, vt52, wordstar, vim, none/off)"))?,
                );
            }
            "--inject" => {
                let f = val(&a)?;
                o.inject.extend(std::fs::read(&f).map_err(|e| format!("{f}: {e}"))?);
            }
            "--load-ter" => {
                let f = val(&a)?;
                let code = std::fs::read(&f).map_err(|e| format!("{f}: {e}"))?;
                o.inject.extend(ang3001::ter_upload_stream(&code));
            }
            "--type" => o.typed.extend(val(&a)?.replace("\\r", "\r").bytes()),
            "--config" => o.config = Some(val(&a)?.into()),
            "--lang" => {
                let v = val(&a)?;
                if !matches!(v.to_ascii_lowercase().as_str(), "en" | "pl" | "english" | "polski" | "polish") {
                    return Err(format!("--lang: unknown language '{v}' (en, pl)"));
                }
                o.lang = Some(v);
            }
            "--trace-io" => o.cfg.trace_io = true,
            "--fixed-baud" => o.cfg.follow_line_params = false,
            "--headless" => o.headless = true,
            "--realtime" => o.realtime = true,
            "--after" => o.after = num(&a, val(&a)?)?,
            "--screenshot" => o.screenshot = Some(val(&a)?.into()),
            "--dump-text" => o.dump_text = true,
            _ => return Err(format!("unknown option '{a}' (see bosman8 terminal --help)")),
        }
    }
    if o.cfg.cpu_hz < 100_000 || o.cfg.pit_hz < 1000 {
        return Err("clock too low".into());
    }
    Ok(Some(o))
}

/// Terminal ROM: `--rom`, else /etc/bosman-8-emulator.conf, else built in.
pub fn load_rom(rom: Option<&PathBuf>) -> Result<Vec<u8>, String> {
    match rom {
        Some(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())),
        None => match b8_link::sysconf::get_path("ang3001_rom") {
            Some(p) => Ok(std::fs::read(&p).unwrap_or_else(|e| {
                eprintln!("bosman8: {}: {e} (from {}), using the built-in ROM", p.display(), b8_link::sysconf::CONF_PATH);
                ang3001::DEFAULT_ROM.to_vec()
            })),
            None => Ok(ang3001::DEFAULT_ROM.to_vec()),
        },
    }
}

fn print_io_log(m: &mut Ang3001) {
    for e in m.take_io_log() {
        match e.out {
            Some(v) => eprintln!("io: cycle {} pc {:04X} OUT ({:02X}h),{:02X}h", e.cycle, e.pc, e.port, v),
            None => eprintln!("io: cycle {} pc {:04X} IN ({:02X}h)", e.cycle, e.pc, e.port),
        }
    }
}

/// Run the terminal without a window (`--headless`).
pub fn headless(o: &Opts) -> Result<(), String> {
    let rom = load_rom(o.rom.as_ref())?;
    let spec = o.link.clone().unwrap_or_else(|| "tcp:127.0.0.1:3001".into());
    let link = b8_link::open(&spec).map_err(|e| format!("--link {spec}: {e}"))?;
    let mut m = Ang3001::new(&rom, o.cfg.clone());
    eprintln!("bosman8 terminal: link {}", link.describe());
    m.attach_link(link);
    m.inject_rx(&o.inject);
    let slice = (o.cfg.cpu_hz / 50) as u64;
    let total = (o.after * o.cfg.cpu_hz as f64) as u64;
    let mut typed: VecDeque<u8> = o.typed.iter().copied().collect();
    let start = Instant::now();
    let mut beeping = false;
    while m.cycles() < total {
        if m.booted() && m.keys_pending() == 0 {
            if let Some(k) = typed.pop_front() {
                m.key_press(k);
            }
        }
        m.run_cycles(slice.min(total - m.cycles()));
        print_io_log(&mut m);
        let b = m.beep();
        if b.active != beeping {
            beeping = b.active;
            if beeping {
                eprintln!("beep: {:.0} Hz", b.freq_hz);
            }
        }
        if o.realtime {
            let due = start + Duration::from_secs_f64(m.cycles() as f64 / o.cfg.cpu_hz as f64);
            if let Some(d) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    }
    if let Some(p) = &o.screenshot {
        std::fs::write(p, m.screenshot_pgm()).map_err(|e| format!("{}: {e}", p.display()))?;
        eprintln!("screenshot written to {}", p.display());
    }
    if o.dump_text {
        for l in m.text_screen() {
            println!("{l}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn options_of_the_former_ang3001() {
        let o = parse(&args("--link serial:/dev/ttyUSB0@100000,8N2 --arrows vim --color amber --headless --after 2 --fixed-baud"))
            .unwrap()
            .unwrap();
        assert_eq!(o.link.as_deref(), Some("serial:/dev/ttyUSB0@100000,8N2"));
        assert_eq!((o.arrows, o.cfg.fg, o.headless, o.after), (Some(ArrowMode::Vim), ang3001::PHOSPHOR_AMBER, true, 2.0));
        assert!(!o.cfg.follow_line_params);
        assert!(parse(&args("--bogus")).is_err());
        assert!(parse(&args("--arrows sideways")).is_err());
    }
}
