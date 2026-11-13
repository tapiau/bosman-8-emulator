//! ANG-3001 terminal emulator - desktop application (minifb window or headless).
//!
//! The machine itself lives in the library (`ang3001::Ang3001`); this binary
//! only adds the window, keyboard mapping, real-time pacing and the serial
//! endpoint (`b8_link::open`). Run `ang3001 --help` for all options.
//!
//! Typical use:
//!
//! ```text
//! ang3001                                  # window, connects to tcp:127.0.0.1:3001 (Bosman-8)
//! ang3001 --link tcp-listen:0.0.0.0:3001   # wait for a computer to connect
//! ang3001 --link stdio --color amber       # host stdin/stdout as the computer
//! ang3001 --headless --link none --load-ter FIRE.TER --after 5 --screenshot fire.pgm
//! ang3001 --headless --link none --inject text.bin --dump-text
//! ```
//!
//! Window: 1024x768 picture (each emulated pixel 2x3, 4:3) plus a button
//! bar below it (`toolbar.rs`), drawn with the ROM font like a row of
//! function keys: RESET, BREAK (held while pressed), LOCAL, STOP, CAPS and
//! TURBO - toggles are lit while active - and LEDs LINK (peer connected),
//! BEEP and WAIT (the ROM waits in STOP after a page). Keys: typed
//! characters are sent as ASCII (host keyboard layout honoured), Enter =
//! 0x0D, Backspace = 0x08, Tab = 0x09, Esc = 0x1B, Delete = 0x7F,
//! Ctrl+letter = control code, arrows = VT52 `ESC A/B/C/D` (`--arrows`),
//! F12 (hold) = BREAK, F11 = reset, F10 = CAPS toggle, F9 = turbo toggle,
//! F8 / Scroll Lock = STOP (paging) toggle, F7 = LOCAL on/off.
//! The beeper is not played yet; while it sounds the window title shows
//! `BEEP <freq> Hz` and the BEEP LED is lit (headless mode prints it to
//! stderr).

mod toolbar;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ang3001::{Ang3001, Config, PHOSPHOR_AMBER, PHOSPHOR_GREEN, PHOSPHOR_WHITE, SCREEN_H, SCREEN_W};
use minifb::{InputCallback, Key, KeyRepeat, MouseButton, MouseMode, Scale, Window, WindowOptions};
use toolbar::{Action, Toolbar};

const HELP: &str = "\
ang3001 - ANG-3001 graphics terminal emulator (Z80B, 512x256 mono)

USAGE: ang3001 [OPTIONS]

OPTIONS:
  --rom FILE          ROM image (default: ang3001_rom from /etc/bosman-8-emulator.conf,
                      else built-in roms/ang3001.bin)
  --link SPEC         serial line endpoint (default: tcp:127.0.0.1:3001)
                      none | loopback | stdio | tcp:HOST:PORT | tcp-listen:ADDR:PORT
                      | serial:DEV[@BAUD]
  --cpu-mhz F         CPU clock in MHz (default 6)
  --pit-mhz F         8253 input clock in MHz (default 1.3)
  --char-cycles N     fixed serial character time in CPU cycles
                      (default: derived from 8253 counter 2 and the 8251 mode)
  --line-offset N     memory line (relative to the scroll register) shown at the
                      top of the screen (default 0: status band on lines 0-15,
                      text rows from line 16; 16 = text row 0 on the top line)
  --color C           green | amber | white | RRGGBB (default green)
  --scale N           window height multiplier 1..4 (default 3; width is 2x
                      for N>=2, i.e. 1024x768 at 3)
  --turbo             run as fast as possible
  --arrows MODE       what the arrow keys send: vt52 (ESC A/B/C/D, default),
                      wordstar (^E ^X ^D ^S) or none
  --inject FILE       feed FILE into the serial input after boot
  --load-ter FILE     upload a terminal binary (org 4000h) via ESC i and run it
                      with ESC 0 after boot (like RUNTERM.COM)
  --type TEXT         type TEXT on the keyboard after boot (\\r = Enter)
  --fixed-baud        keep a serial:DEV@BAUD link at that speed (default: follow
                      the speed/format the ROM programs into the 8251, 100000 8N2)
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
  arrows              see --arrows
  F12 (hold)          BREAK key
  F11                 reset terminal (CPU and chips; RAM is kept)
  F10                 toggle CAPS (the ROM swaps letter case while active)
  F9                  toggle turbo
  F8, Scroll Lock     toggle STOP (page mode): after a screen full of lines the
                      terminal shows STOP and waits for a key (1 / 2 = fast /
                      smooth scrolling); switching STOP off resumes
  F7                  LOCAL on/off: types BREAK + ESC (on) or BREAK (off) for
                      you; in LOCAL keys go to the screen, not to the line

BUTTONS (window, below the screen; click with the mouse):
  RESET BREAK LOCAL STOP CAPS TURBO   same as the keys above; BREAK is held
                      while the button is pressed, toggles are lit while on
  LEDs                LINK = peer connected, BEEP = beeper sounding,
                      WAIT = terminal waits in STOP (press a key for more)
";

struct Opts {
    rom: Option<PathBuf>,
    link: String,
    cfg: Config,
    scale: usize,
    turbo: bool,
    arrows: Arrows,
    inject: Vec<u8>,
    typed: Vec<u8>,
    headless: bool,
    realtime: bool,
    after: f64,
    screenshot: Option<PathBuf>,
    dump_text: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Arrows {
    Vt52,
    WordStar,
    None,
}

fn parse_color(s: &str) -> Result<u32, String> {
    match s.to_ascii_lowercase().as_str() {
        "green" => Ok(PHOSPHOR_GREEN),
        "amber" => Ok(PHOSPHOR_AMBER),
        "white" => Ok(PHOSPHOR_WHITE),
        h => u32::from_str_radix(h.trim_start_matches('#'), 16)
            .ok()
            .filter(|_| h.trim_start_matches('#').len() == 6)
            .ok_or_else(|| format!("bad colour '{s}'")),
    }
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts {
        rom: None,
        link: "tcp:127.0.0.1:3001".into(),
        cfg: Config::default(),
        scale: 3,
        turbo: false,
        arrows: Arrows::Vt52,
        inject: Vec::new(),
        typed: Vec::new(),
        headless: false,
        realtime: false,
        after: 3.0,
        screenshot: None,
        dump_text: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        fn num<T: std::str::FromStr>(name: &str, v: String) -> Result<T, String> {
            v.parse().map_err(|_| format!("{name}: bad number '{v}'"))
        }
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--rom" => o.rom = Some(val(&a)?.into()),
            "--link" => o.link = val(&a)?,
            "--cpu-mhz" => o.cfg.cpu_hz = (num::<f64>(&a, val(&a)?)? * 1e6) as u32,
            "--pit-mhz" => o.cfg.pit_hz = (num::<f64>(&a, val(&a)?)? * 1e6) as u32,
            "--char-cycles" => o.cfg.char_cycles = Some(num(&a, val(&a)?)?),
            "--line-offset" => o.cfg.line_offset = num(&a, val(&a)?)?,
            "--color" => o.cfg.fg = parse_color(&val(&a)?)?,
            "--scale" => o.scale = num::<usize>(&a, val(&a)?)?.clamp(1, 4),
            "--turbo" => o.turbo = true,
            "--arrows" => {
                o.arrows = match val(&a)?.as_str() {
                    "vt52" => Arrows::Vt52,
                    "wordstar" => Arrows::WordStar,
                    "none" => Arrows::None,
                    v => return Err(format!("--arrows: unknown mode '{v}'")),
                }
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
            "--trace-io" => o.cfg.trace_io = true,
            "--fixed-baud" => o.cfg.follow_line_params = false,
            "--headless" => o.headless = true,
            "--realtime" => o.realtime = true,
            "--after" => o.after = num(&a, val(&a)?)?,
            "--screenshot" => o.screenshot = Some(val(&a)?.into()),
            "--dump-text" => o.dump_text = true,
            _ => return Err(format!("unknown option '{a}' (see --help)")),
        }
    }
    if o.cfg.cpu_hz < 100_000 || o.cfg.pit_hz < 1000 {
        return Err("clock too low".into());
    }
    Ok(o)
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ang3001: {e}");
            return ExitCode::from(2);
        }
    };
    let rom = match &o.rom {
        Some(p) => match std::fs::read(p) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ang3001: {}: {e}", p.display());
                return ExitCode::FAILURE;
            }
        },
        // /etc/bosman-8-emulator.conf (ang3001_rom), else the built-in image.
        None => match b8_link::sysconf::get_path("ang3001_rom") {
            Some(p) => match std::fs::read(&p) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!(
                        "ang3001: {}: {e} (from {}), using the built-in ROM",
                        p.display(),
                        b8_link::sysconf::CONF_PATH
                    );
                    ang3001::DEFAULT_ROM.to_vec()
                }
            },
            None => ang3001::DEFAULT_ROM.to_vec(),
        },
    };
    let link = match b8_link::open(&o.link) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ang3001: --link {}: {e}", o.link);
            return ExitCode::FAILURE;
        }
    };
    let mut m = Ang3001::new(&rom, o.cfg.clone());
    eprintln!("ang3001: link {}", link.describe());
    m.attach_link(link);
    m.inject_rx(&o.inject);

    let r = if o.headless { headless(&mut m, &o) } else { gui(&mut m, &o) };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ang3001: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Types the `--type` text once the ROM has booted (one key per frame).
struct Typist {
    pending: VecDeque<u8>,
}

impl Typist {
    fn step(&mut self, m: &mut Ang3001) {
        if m.booted() && m.keys_pending() == 0 {
            if let Some(k) = self.pending.pop_front() {
                m.key_press(k);
            }
        }
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

fn headless(m: &mut Ang3001, o: &Opts) -> Result<(), String> {
    let slice = (o.cfg.cpu_hz / 50) as u64;
    let total = (o.after * o.cfg.cpu_hz as f64) as u64;
    let mut typist = Typist { pending: o.typed.iter().copied().collect() };
    let start = Instant::now();
    let mut beeping = false;
    while m.cycles() < total {
        typist.step(m);
        m.run_cycles(slice.min(total - m.cycles()));
        print_io_log(m);
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

/// Collects typed characters from minifb (honours the host keyboard layout).
struct CharInput {
    queue: Rc<RefCell<VecDeque<u8>>>,
    ctrl: Rc<RefCell<bool>>,
}

impl InputCallback for CharInput {
    fn add_char(&mut self, c: u32) {
        if *self.ctrl.borrow() {
            return; // Ctrl combinations are handled from key events
        }
        if (0x20..0x7F).contains(&c) {
            self.queue.borrow_mut().push_back(c as u8);
        } else if let Some(&(code, _)) =
            ang3001::POLISH_GLYPHS.iter().find(|&&(_, ch)| ch as u32 == c)
        {
            // guess: the keyboard sends the font code of the Polish letter
            self.queue.borrow_mut().push_back(code);
        }
    }

    fn set_key_state(&mut self, key: Key, state: bool) {
        if matches!(key, Key::LeftCtrl | Key::RightCtrl) {
            *self.ctrl.borrow_mut() = state;
        }
    }
}

fn letter_code(k: Key) -> Option<u8> {
    use Key::*;
    let letters = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    letters.iter().position(|&l| l == k).map(|i| b'A' + i as u8)
}

fn gui(m: &mut Ang3001, o: &Opts) -> Result<(), String> {
    let (sx, sy) = match o.scale {
        1 => (1, 1),
        n => (2, n),
    };
    let fs = sx; // button label font scale
    let screen_h = SCREEN_H * sy;
    let (w, h) = (SCREEN_W * sx, screen_h + Toolbar::height(fs));
    let font = (0..=255u8).map(|c| m.glyph(c)).collect();
    let mut bar = Toolbar::new(font, w, screen_h, fs);
    let mut win = Window::new(
        "ANG-3001",
        w,
        h,
        WindowOptions { scale: Scale::X1, ..WindowOptions::default() },
    )
    .map_err(|e| format!("cannot open window: {e}"))?;
    win.set_target_fps(0);
    let chars = Rc::new(RefCell::new(VecDeque::new()));
    let ctrl = Rc::new(RefCell::new(false));
    win.set_input_callback(Box::new(CharInput { queue: chars.clone(), ctrl: ctrl.clone() }));

    let mut screen = vec![0u32; SCREEN_W * SCREEN_H];
    let mut buf = vec![0u32; w * h];
    let slice = (o.cfg.cpu_hz / 50) as u64;
    let frame = Duration::from_millis(20);
    let mut turbo = o.turbo;
    let mut typist = Typist { pending: o.typed.iter().copied().collect() };
    let mut title = String::new();
    let mut next = Instant::now();

    while win.is_open() {
        let ctrl_down = win.is_key_down(Key::LeftCtrl) || win.is_key_down(Key::RightCtrl);

        // special keys
        for k in win.get_keys_pressed(KeyRepeat::Yes) {
            let mut send: &[u8] = &[];
            let code;
            match k {
                Key::Enter | Key::NumPadEnter => send = b"\r",
                Key::Backspace => send = b"\x08",
                Key::Tab => send = b"\t",
                Key::Escape => send = b"\x1b",
                Key::Delete => send = b"\x7f",
                Key::Up | Key::Down | Key::Right | Key::Left => {
                    let i = [Key::Up, Key::Down, Key::Right, Key::Left].iter().position(|&a| a == k).unwrap();
                    send = match o.arrows {
                        Arrows::Vt52 => [b"\x1bA", b"\x1bB", b"\x1bC", b"\x1bD"][i],
                        Arrows::WordStar => [b"\x05", b"\x18", b"\x04", b"\x13"][i],
                        Arrows::None => b"",
                    };
                }
                _ if ctrl_down => {
                    code = match k {
                        Key::LeftBracket => Some(0x1B),
                        Key::Backslash => Some(0x1C),
                        Key::RightBracket => Some(0x1D),
                        Key::Key6 => Some(0x1E),
                        Key::Minus => Some(0x1F),
                        Key::Space | Key::Key2 => Some(0x00),
                        _ => letter_code(k).map(|c| c & 0x1F),
                    };
                    if let Some(c) = code {
                        m.key_press(c);
                    }
                }
                _ => {}
            }
            for &b in send {
                m.key_press(b);
            }
        }
        for c in chars.borrow_mut().drain(..) {
            m.key_press(c);
        }
        // function keys (no auto-repeat) and the button bar
        let mut actions: Vec<Action> = win
            .get_keys_pressed(KeyRepeat::No)
            .into_iter()
            .filter_map(|k| match k {
                Key::F11 => Some(Action::Reset),
                Key::F10 => Some(Action::Caps),
                Key::F9 => Some(Action::Turbo),
                Key::F8 | Key::ScrollLock => Some(Action::Stop),
                Key::F7 => Some(Action::Local),
                _ => None,
            })
            .collect();
        let mouse = win.get_mouse_pos(MouseMode::Discard);
        actions.extend(bar.mouse(mouse, win.get_mouse_down(MouseButton::Left)));
        for a in actions {
            match a {
                Action::Reset => m.reset(),
                Action::Caps => m.set_caps(!m.caps()),
                Action::Turbo => turbo = !turbo,
                Action::Stop => m.set_stop(!m.stop()),
                Action::Local => {
                    m.set_local(!m.is_local());
                }
                Action::Break => {} // momentary, see below
            }
        }
        let break_held = win.is_key_down(Key::F12) || bar.break_held();
        m.set_break(break_held);
        typist.step(m);

        // emulate
        if turbo {
            let until = Instant::now() + frame;
            while Instant::now() < until {
                m.run_cycles(slice);
            }
        } else {
            m.run_cycles(slice);
        }
        print_io_log(m);

        // draw
        m.render(&mut screen);
        let b = m.beep();
        let link_up = m.link().is_some_and(|l| l.status().connected);
        let st = toolbar::State {
            break_held,
            local: m.is_local(),
            local_pending: m.local_switching(),
            stop: m.stop(),
            caps: m.caps(),
            turbo,
            waiting: m.is_stopped(),
            beep: b.active,
            link: link_up,
        };
        bar.draw(&mut buf, &st, o.cfg.fg);
        for y in 0..SCREEN_H {
            let src = &screen[y * SCREEN_W..(y + 1) * SCREEN_W];
            let dst0 = y * sy * w;
            for (x, &p) in src.iter().enumerate() {
                for i in 0..sx {
                    buf[dst0 + x * sx + i] = p;
                }
            }
            for r in 1..sy {
                buf.copy_within(dst0..dst0 + w, dst0 + r * w);
            }
        }
        win.update_with_buffer(&buf, w, h).map_err(|e| e.to_string())?;

        let link = m.link().map(|l| l.describe()).unwrap_or_default();
        let t = format!(
            "ANG-3001 - {link}{}{}{}",
            if m.caps() { " - CAPS" } else { "" },
            if turbo { " - TURBO" } else { "" },
            if b.active { format!(" - BEEP {:.0} Hz", b.freq_hz) } else { String::new() },
        );
        if t != title {
            win.set_title(&t);
            title = t;
        }

        // pace
        next += frame;
        let now = Instant::now();
        if turbo || next < now {
            next = now;
        } else {
            std::thread::sleep(next - now);
        }
    }
    Ok(())
}
