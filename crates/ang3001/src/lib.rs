//! ANG-3001 graphics terminal: machine core.
//!
//! Pure logic - no threads, sockets, windows or wall-clock time - so it can
//! be compiled to WASM later. The application drives it with
//! [`Ang3001::run_cycles`] (typically `cpu_hz / 50` cycles per 1/50 s slice),
//! renders it with [`Ang3001::render`] and feeds keys with
//! [`Ang3001::key_press`].
//!
//! Hardware model (see `doc/ang3001-hardware.md` for the evidence):
//!
//! | address / port | what                                                     |
//! |----------------|----------------------------------------------------------|
//! | 0000-3FFF      | 16 KB EPROM (reads); writes always go to the RAM below it |
//! | 4000-FFFF      | RAM (64 KB array in total, RAM under ROM included)        |
//! | 8000-BFFF      | video RAM, 512x256 1 bpp, 64 bytes/line, bit 7 = left     |
//! | OUT 0x0B / 0x0A| ROM off / ROM on (hypothesis: RAM under ROM readable)     |
//! | OUT 0x80       | scroll register: memory line shown first                 |
//! | 0x88-0x8B      | 8253: ch0 beep length (clocked by ch1 OUT), ch1 tone, ch2 UART clock |
//! | 0x8C IN        | keyboard ASCII code (clears the strobe)                   |
//! | 0x8D IN        | bit7 1 = normal boot (ROM 90.09.11: 0 = BREAK enters LOCAL), bit6 0 = CAPS, bit5 0 = BREAK, bit4 0 = STOP (paging); idle 0xF0 |
//! | 0x8E IN        | bit5 key strobe, bit2 frame clock (toggles every frame), bit0 1 = no key held |
//! | 0x90 / 0x91    | 8251 data / control+status                                |
//!
//! Operating modes of the ROM ([`TermMode`]): normal DUPLEX operation, the
//! BREAK prompt, LOCAL mode and the STOP (page full) wait. The ROM keeps no
//! RAM flag for them - each is a separate polling loop - so the core tracks
//! the mode by watching the PC enter those loops, whose addresses depend on
//! the ROM version ([`RomProfile`]). [`Ang3001::set_local`]
//! performs the BREAK / ESC key sequence that switches LOCAL on or off, the
//! way a user would.

use std::collections::VecDeque;

use b8_chips::{Pit8253, Usart8251};
use b8_link::{Endpoint, LineParams, Parity, StopBits};
use iz80::{Cpu, Machine};

/// The ROM image this emulator was written against (`roms/ang3001/ang3001.bin`).
pub const DEFAULT_ROM: &[u8] = include_bytes!("../../../roms/ang3001/ang3001.bin");

pub const SCREEN_W: usize = 512;
pub const SCREEN_H: usize = 256;
/// Video RAM start.
pub const VRAM: u16 = 0x8000;
/// Text geometry used by the ROM: 6x10 cells, 80x25, origin at pixel x=16,
/// memory line 16 (relative to the scroll register).
pub const TEXT_COLS: usize = 80;
pub const TEXT_ROWS: usize = 25;
pub const TEXT_X0: usize = 16;
pub const TEXT_LINE0: usize = 16;
/// Glyph of character `c` is at `FONT_BASE + 10*c`, 10 rows, pixels in bits 7..2.
pub const FONT_BASE: usize = 0x3600;
/// Address of the main loop of the default ROM; reaching it means the boot
/// sequence is done. Other ROMs: see [`RomProfile`].
pub const MAIN_LOOP_ADDR: u16 = PROFILE_ZJ_PG.main_loop;
/// Top of the BREAK prompt loop ("BREAK - powtórz   LOCAL - ESC"), default ROM.
pub const BREAK_PROMPT_ADDR: u16 = 0x0533;
/// Top of the LOCAL mode loop (keys go straight to the screen), default ROM.
pub const LOCAL_LOOP_ADDR: u16 = PROFILE_ZJ_PG.local_loop;
/// Top of the STOP wait loop in `LF_HANDLER` (page full, "STOP" shown), default ROM.
pub const STOP_LOOP_ADDR: u16 = PROFILE_ZJ_PG.stop_loop;

/// ROM-specific addresses of the polling loops the core watches (the ROM
/// keeps no RAM flag for its operating mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomProfile {
    pub name: &'static str,
    /// Main loop; reaching it ends the boot sequence (serial input is held until then).
    pub main_loop: u16,
    /// BREAK prompt loop, if the ROM has one.
    pub break_prompt: Option<u16>,
    pub local_loop: u16,
    pub stop_loop: u16,
    /// LOCAL is entered straight from BREAK while port 0x8D bit 7 = 0 (no
    /// prompt; with bit 7 = 1 BREAK goes out on the line).
    pub local_via_bit7: bool,
    /// Bytes at `main_loop` identifying the ROM.
    signature: [u8; 4],
}

/// `ANG3001_ZJ_wersjaPG.BIN` (the default `roms/ang3001/ang3001.bin`).
pub const PROFILE_ZJ_PG: RomProfile = RomProfile {
    name: "ZJ wersja PG",
    main_loop: 0x04BD,
    break_prompt: Some(0x0533),
    local_loop: 0x0578,
    stop_loop: 0x0A23,
    local_via_bit7: false,
    signature: [0xDB, 0x91, 0xE6, 0x02], // IN A,(91h); AND 02h
};

/// `ANG-3001_90.09.11.BIN`.
pub const PROFILE_90_09_11: RomProfile = RomProfile {
    name: "90.09.11",
    main_loop: 0x05CF,
    break_prompt: None,
    local_loop: 0x080F,
    stop_loop: 0x08FB,
    local_via_bit7: true,
    signature: [0xDB, 0x91, 0xE6, 0x82], // IN A,(91h); AND 82h (RxRDY | DSR)
};

impl RomProfile {
    /// Known profile matching the ROM image.
    pub fn detect(rom: &[u8]) -> Option<RomProfile> {
        [PROFILE_ZJ_PG, PROFILE_90_09_11].into_iter().find(|p| {
            let a = p.main_loop as usize;
            rom.get(a..a + 4) == Some(&p.signature[..])
        })
    }
}

/// Unknown ROM: serial input is let through after this much emulated time.
const UNKNOWN_ROM_BOOT_MS: u64 = 1500;

/// What the ROM is doing, derived from which of its loops the CPU is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TermMode {
    /// Booting, or the main loop (DUPLEX: keys to the line, line to the screen).
    #[default]
    Normal,
    /// BREAK was pressed: "BREAK - powtórz   LOCAL - ESC" in the message band.
    BreakPrompt,
    /// LOCAL mode: keys go to the screen, the line is ignored.
    Local,
    /// Paging is on and a screen full of lines was shown: "STOP" in the
    /// message band, waiting for a key (or STOP released, or BREAK).
    Stopped,
}

/// Emulated time the automatic BREAK press of [`Ang3001::set_local`] lasts
/// (the ROM samples the key at 25 Hz).
const AUTO_BREAK_MS: u64 = 100;
/// Give up an automatic key sequence after this long.
const AUTO_TIMEOUT_MS: u64 = 2000;

/// Steps of the automatic LOCAL on/off key sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoSeq {
    Idle,
    /// BREAK held until `until`; then wait for `next`.
    BreakDown { until: u64, enter: bool },
    /// Waiting for the BREAK prompt to appear, then ESC is typed.
    WaitPrompt { deadline: u64 },
    /// Waiting for the mode to become `want`.
    WaitMode { want: TermMode, deadline: u64 },
}

/// Default display line offset: the picture starts at the memory line held
/// in the scroll register (simplest hardware). The ROM treats the 16 lines
/// at `scroll+0` as a band for the status/message row (it clears and prints
/// messages there with base 0x8002) and puts text row 0 at `scroll+16`
/// (base 0x8402), so the screen shows: status band (lines 0-9), 6 blank
/// separator lines, text rows 0-23 on lines 16-255. The alternative, 16,
/// puts text row 0 on the top line and the status band at the bottom.
/// See doc/ang3001-hardware.md.
pub const DEFAULT_LINE_OFFSET: u8 = 0;

#[derive(Debug, Clone)]
pub struct Config {
    /// CPU clock (Z80B; real value unknown).
    pub cpu_hz: u32,
    /// 8253 input clock (estimate: divider 13 -> ~100 kbit/s).
    pub pit_hz: u32,
    /// Frame rate of the video circuit (bit 2 of port 0x8E toggles once per frame).
    pub frame_hz: u32,
    /// Memory line (relative to the scroll register) shown at the top.
    pub line_offset: u8,
    /// Foreground / background colour, 0x00RRGGBB.
    pub fg: u32,
    pub bg: u32,
    /// Fixed serial character time in CPU cycles; `None` = derive it from the
    /// 8253 counter 2 frequency and the 8251 mode.
    pub char_cycles: Option<u32>,
    /// Record accesses to unknown / interesting ports (see [`Ang3001::take_io_log`]).
    pub trace_io: bool,
    /// Pass the line speed / format programmed into the 8251 and 8253 to the
    /// link endpoint (a host serial port then follows, e.g. 100000 8N2).
    pub follow_line_params: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cpu_hz: 6_000_000,
            pit_hz: 1_300_000,
            frame_hz: 50,
            line_offset: DEFAULT_LINE_OFFSET,
            fg: PHOSPHOR_GREEN,
            bg: 0x000000,
            char_cycles: None,
            trace_io: false,
            follow_line_params: true,
        }
    }
}

pub const PHOSPHOR_GREEN: u32 = 0x33FF66;
pub const PHOSPHOR_AMBER: u32 = 0xFFB000;
pub const PHOSPHOR_WHITE: u32 = 0xE8E8E8;

/// Font codes of the Polish lowercase letters (glyphs in the ROM font at
/// `FONT_BASE + 10*code`). Codes 0x01-0x06, 0x0B and 0x0C are drawn when
/// received over the line (the C0 table maps them to `DRAW_CHAR`); 0x07 is
/// BEL there, so `ó` only appears in ROM messages ("BREAK - powtórz").
pub const POLISH_GLYPHS: [(u8, char); 9] = [
    (0x01, 'ę'),
    (0x02, 'ź'),
    (0x03, 'ł'),
    (0x04, 'ń'),
    (0x05, 'ś'),
    (0x06, 'ą'),
    (0x07, 'ó'),
    (0x0B, 'ż'),
    (0x0C, 'ć'),
];

/// One logged port access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoEvent {
    /// CPU cycle (emulator time) of the access.
    pub cycle: u64,
    pub pc: u16,
    pub port: u8,
    /// `Some(value)` for OUT, `None` for IN.
    pub out: Option<u8>,
}

/// Sound state: the beeper is driven by 8253 counter 1 (tone, mode 3) gated
/// by counter 0 (mode 0, counts periods of the tone; sound while counting).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beep {
    pub active: bool,
    pub freq_hz: f64,
}

struct Bus {
    rom: Box<[u8; 0x4000]>,
    ram: Box<[u8; 0x10000]>,
    rom_enabled: bool,
    scroll: u8,
    pit: Pit8253,
    usart: Usart8251,
    // keyboard
    keys: VecDeque<u8>,
    key_data: u8,
    key_strobe: bool,
    key_held: bool,
    break_held: bool,
    break_auto: bool,
    /// Port 0x8D bit 7 = 0 (ROM 90.09.11: BREAK enters LOCAL instead of
    /// sending BREAK; at reset it would start the monitor).
    local_switch: bool,
    caps: bool,
    stop: bool,
    // time
    now: u64,
    frame_cycles: u64,
    char_cycles_fixed: Option<u32>,
    cpu_hz: u32,
    timing_dirty: bool,
    /// Asynchronous line parameters programmed into the 8251 / 8253 (None = sync or no clock).
    line: Option<LineParams>,
    // debug
    pc: u16,
    trace_io: bool,
    io_log: Vec<IoEvent>,
    rom_switches: u64,
}

impl Bus {
    fn log(&mut self, port: u8, out: Option<u8>) {
        if self.trace_io && self.io_log.len() < 100_000 {
            self.io_log.push(IoEvent { cycle: self.now, pc: self.pc, port, out });
        }
    }

    fn present_next_key(&mut self) {
        if !self.key_strobe {
            if let Some(k) = self.keys.pop_front() {
                self.key_data = k;
                self.key_strobe = true;
            }
        }
    }

    fn frame_bit(&self) -> bool {
        (self.now / self.frame_cycles) & 1 != 0
    }

    /// Serial character time from the 8253 counter 2 (baud clock) and the
    /// 8251 mode instruction.
    fn update_char_timing(&mut self) {
        self.timing_dirty = false;
        self.line = self.pit.output_hz(2).and_then(|clk| {
            let mode = self.usart.mode();
            if mode & 3 == 0 || !self.usart.initialized() {
                return None; // synchronous mode / not programmed yet
            }
            Some(LineParams {
                baud: ((clk / self.usart.clock_divider() as f64).round() as u32).max(1),
                data_bits: self.usart.bits_per_char() as u8,
                parity: match (mode >> 4) & 3 {
                    1 => Parity::Odd,
                    3 => Parity::Even,
                    _ => Parity::None,
                },
                stop_bits: match mode >> 6 {
                    2 => StopBits::OnePointFive,
                    3 => StopBits::Two,
                    _ => StopBits::One,
                },
            })
        });
        let cycles = match self.char_cycles_fixed {
            Some(c) => c,
            None => {
                let Some(clk) = self.pit.output_hz(2) else { return };
                let mode = self.usart.mode();
                let data = self.usart.bits_per_char();
                let bits = if mode & 3 == 0 {
                    data as f64 // synchronous: no start/stop bits
                } else {
                    let parity = if mode & 0x10 != 0 { 1.0 } else { 0.0 };
                    let stop = match mode >> 6 {
                        2 => 1.5,
                        3 => 2.0,
                        _ => 1.0,
                    };
                    1.0 + data as f64 + parity + stop
                };
                let baud = clk / self.usart.clock_divider() as f64;
                ((self.cpu_hz as f64 * bits / baud).round() as u32).max(1)
            }
        };
        self.usart.set_char_cycles(cycles);
    }
}

impl Machine for Bus {
    fn peek(&mut self, address: u16) -> u8 {
        if address < 0x4000 && self.rom_enabled {
            self.rom[address as usize]
        } else {
            self.ram[address as usize]
        }
    }

    fn poke(&mut self, address: u16, value: u8) {
        self.ram[address as usize] = value;
    }

    fn port_in(&mut self, address: u16) -> u8 {
        let port = address as u8;
        match port {
            0x88..=0x8A => self.pit.read(port - 0x88),
            0x8C => {
                let k = self.key_data;
                self.key_strobe = false;
                self.present_next_key();
                k
            }
            0x8D => {
                let mut v = if self.local_switch { 0 } else { 0x80 };
                if !self.caps {
                    v |= 0x40;
                }
                if !(self.break_held || self.break_auto) {
                    v |= 0x20;
                }
                if !self.stop {
                    v |= 0x10;
                }
                v
            }
            0x8E => {
                let mut v = 0;
                if self.key_strobe {
                    v |= 0x20;
                }
                if self.frame_bit() {
                    v |= 0x04;
                }
                if !self.key_held {
                    v |= 0x01;
                }
                v
            }
            0x90 => self.usart.read_data(),
            0x91 => self.usart.read_status(),
            _ => {
                self.log(port, None);
                0xFF
            }
        }
    }

    fn port_out(&mut self, address: u16, value: u8) {
        let port = address as u8;
        match port {
            0x0A => {
                self.log(port, Some(value));
                self.rom_enabled = true;
                self.rom_switches += 1;
            }
            0x0B => {
                self.log(port, Some(value));
                self.rom_enabled = false;
                self.rom_switches += 1;
            }
            0x80 => self.scroll = value,
            0x88..=0x8B => {
                self.pit.write(port - 0x88, value);
                if port >= 0x8A {
                    self.timing_dirty = true;
                }
            }
            0x90 => self.usart.write_data(value),
            0x91 => {
                self.usart.write_ctrl(value);
                self.timing_dirty = true;
            }
            _ => self.log(port, Some(value)),
        }
    }
}

/// The ANG-3001 machine.
pub struct Ang3001 {
    cpu: Cpu,
    bus: Bus,
    cfg: Config,
    link: Option<Box<dyn Endpoint>>,
    /// Bytes injected by the host, delivered to the 8251 after boot.
    inject: VecDeque<u8>,
    booted: bool,
    profile: Option<RomProfile>,
    mode: TermMode,
    auto: AutoSeq,
    break_sent: bool,
    /// Line parameters last passed to the link endpoint.
    line_applied: Option<LineParams>,
    break_count: u64,
    since_pump: u32,
}

impl Ang3001 {
    /// Create a machine with the given ROM (16 KB; shorter images are padded
    /// with 0xFF, longer ones truncated).
    pub fn new(rom: &[u8], cfg: Config) -> Self {
        let mut r = Box::new([0xFFu8; 0x4000]);
        let n = rom.len().min(0x4000);
        r[..n].copy_from_slice(&rom[..n]);
        let mut pit = Pit8253::new(cfg.pit_hz, cfg.cpu_hz);
        pit.set_cascade_0_from_1(true);
        let bus = Bus {
            rom: r,
            ram: Box::new([0u8; 0x10000]),
            rom_enabled: true,
            scroll: 0,
            pit,
            usart: Usart8251::new(),
            keys: VecDeque::new(),
            key_data: 0,
            key_strobe: false,
            key_held: false,
            break_held: false,
            break_auto: false,
            local_switch: false,
            caps: false,
            stop: false,
            now: 0,
            frame_cycles: (cfg.cpu_hz / cfg.frame_hz.max(1)).max(1) as u64,
            char_cycles_fixed: cfg.char_cycles,
            cpu_hz: cfg.cpu_hz,
            timing_dirty: true,
            line: None,
            pc: 0,
            trace_io: cfg.trace_io,
            io_log: Vec::new(),
            rom_switches: 0,
        };
        Self {
            cpu: Cpu::new_z80(),
            bus,
            cfg,
            link: None,
            inject: VecDeque::new(),
            booted: false,
            profile: RomProfile::detect(rom),
            mode: TermMode::Normal,
            auto: AutoSeq::Idle,
            break_sent: false,
            line_applied: None,
            break_count: 0,
            since_pump: 0,
        }
    }

    /// Machine with the built-in ROM and default configuration.
    pub fn with_default_rom() -> Self {
        Self::new(DEFAULT_ROM, Config::default())
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Attach the serial line (8251) to an endpoint.
    pub fn attach_link(&mut self, ep: Box<dyn Endpoint>) {
        self.line_applied = None; // the new endpoint gets the current line parameters
        self.link = Some(ep);
    }

    /// Asynchronous line parameters the ROM programmed (None before init / in sync mode).
    pub fn line_params(&self) -> Option<LineParams> {
        self.bus.line
    }

    pub fn link(&self) -> Option<&dyn Endpoint> {
        self.link.as_deref()
    }

    /// Hardware reset (F11 / RESET button in the GUI): CPU, 8253, 8251,
    /// ROM mapping, scroll register and the key queue. RAM keeps its
    /// contents (the ROM tests and clears what it uses anyway); latching
    /// keys (CAPS, STOP) keep their state.
    pub fn reset(&mut self) {
        self.cpu.signal_reset();
        self.bus.rom_enabled = true;
        self.bus.scroll = 0;
        self.bus.pit.reset();
        self.bus.usart.reset();
        self.bus.key_strobe = false;
        self.bus.keys.clear();
        self.bus.timing_dirty = true;
        self.booted = false;
        self.mode = TermMode::Normal;
        self.auto = AutoSeq::Idle;
        self.bus.break_auto = false;
        self.bus.local_switch = false;
        self.break_sent = false;
    }

    /// Run for (at least) `cycles` CPU cycles.
    pub fn run_cycles(&mut self, cycles: u64) {
        let end = self.bus.now + cycles;
        self.pump_link();
        while self.bus.now < end {
            let pc = self.cpu.immutable_registers().pc();
            self.bus.pc = pc;
            if let Some(p) = self.profile {
                if pc == p.main_loop {
                    self.booted = true;
                    self.mode = TermMode::Normal;
                } else if Some(pc) == p.break_prompt {
                    self.mode = TermMode::BreakPrompt;
                } else if pc == p.local_loop {
                    self.mode = TermMode::Local;
                } else if pc == p.stop_loop {
                    self.mode = TermMode::Stopped;
                }
            } else if !self.booted && self.bus.now >= self.ms(UNKNOWN_ROM_BOOT_MS) {
                self.booted = true;
            }
            let before = self.cpu.cycle_count();
            self.cpu.execute_instruction(&mut self.bus);
            let mut delta = self.cpu.cycle_count().wrapping_sub(before) as u32;
            if delta == 0 {
                delta = 4; // halted: time still passes
            }
            self.bus.now += delta as u64;
            self.bus.pit.tick(delta);
            if self.bus.timing_dirty {
                self.bus.update_char_timing();
            }
            self.bus.usart.tick(delta);
            self.since_pump += delta;
            if self.since_pump >= 2000 {
                self.pump_link();
            }
        }
        self.pump_link();
    }

    fn pump_link(&mut self) {
        self.since_pump = 0;
        if self.auto != AutoSeq::Idle {
            self.step_auto();
        }
        let now = self.bus.now as f64 / self.bus.cpu_hz as f64;
        let usart = &mut self.bus.usart;
        while let Some(b) = usart.take_tx() {
            if let Some(l) = self.link.as_mut() {
                l.write_at(b, now);
            }
        }
        if self.cfg.follow_line_params && self.bus.line.is_some() && self.bus.line != self.line_applied {
            if let (Some(l), Some(p)) = (self.link.as_mut(), self.bus.line) {
                l.set_line_params_at(p, now);
                self.line_applied = Some(p);
            }
        }
        let brk = usart.break_active();
        if brk && !self.break_sent {
            self.break_count += 1;
            if let Some(l) = self.link.as_mut() {
                l.send_break();
            }
        }
        self.break_sent = brk;
        if !self.booted {
            return; // the ROM is not listening yet; keep input waiting
        }
        // Keep the chip-side queue short so that late input does not pile up.
        while usart.rx_pending() < 64 {
            if let Some(b) = self.inject.pop_front() {
                usart.push_rx(b);
            } else if let Some(b) = self.link.as_mut().and_then(|l| l.read()) {
                usart.push_rx(b);
            } else {
                break;
            }
        }
    }

    /// Queue bytes as if received over the serial line (delivered after boot,
    /// before anything from the attached link).
    pub fn inject_rx(&mut self, bytes: &[u8]) {
        self.inject.extend(bytes);
    }

    /// Bytes still waiting to be delivered to the 8251 (injected + chip queue).
    pub fn rx_backlog(&self) -> usize {
        self.inject.len() + self.bus.usart.rx_pending()
    }

    /// True once the ROM reached its main loop after (re)set.
    pub fn booted(&self) -> bool {
        self.booted
    }

    /// Emulated time in CPU cycles since power-on.
    pub fn cycles(&self) -> u64 {
        self.bus.now
    }

    // ---- keyboard --------------------------------------------------------

    /// Queue a key (ASCII code as produced by the keyboard). One key is
    /// presented per strobe; the next one after the ROM read port 0x8C.
    pub fn key_press(&mut self, ascii: u8) {
        self.bus.keys.push_back(ascii);
        self.bus.present_next_key();
    }

    /// Keys waiting in the queue (including the one currently strobed).
    pub fn keys_pending(&self) -> usize {
        self.bus.keys.len() + self.bus.key_strobe as usize
    }

    /// BREAK key held (port 0x8D bit 5 = 0).
    pub fn set_break(&mut self, held: bool) {
        self.bus.break_held = held;
    }

    /// STOP (page mode) latch, port 0x8D bit 4 = 0 when on. With it on the
    /// ROM counts lines scrolled off the bottom; after a screen full (24
    /// lines; the first time right at the next scroll) it shows "STOP" (and
    /// the scroll speed choice "speed 1 2") in the message band and waits:
    /// any key shows the next page (the key is swallowed; `1` / `2` select
    /// fast / smooth scrolling), switching STOP off or pressing BREAK
    /// resumes without paging. Received data waits meanwhile.
    pub fn set_stop(&mut self, on: bool) {
        self.bus.stop = on;
    }

    pub fn stop(&self) -> bool {
        self.bus.stop
    }

    /// Addresses of the loaded ROM's loops, if it is a known one (else the
    /// mode is not tracked, LOCAL can't be switched automatically and serial
    /// input is let through after a fixed boot time).
    pub fn profile(&self) -> Option<RomProfile> {
        self.profile
    }

    /// Current ROM operating mode (see [`TermMode`]).
    pub fn mode(&self) -> TermMode {
        self.mode
    }

    /// True while the ROM is in LOCAL mode.
    pub fn is_local(&self) -> bool {
        self.mode == TermMode::Local
    }

    /// True while the ROM waits in the STOP (page full) loop.
    pub fn is_stopped(&self) -> bool {
        self.mode == TermMode::Stopped
    }

    /// Switch LOCAL mode on or off the way a user would on the real
    /// keyboard, over the next few hundred ms of emulated time:
    /// on = BREAK (press + release) then ESC in the BREAK prompt;
    /// off = BREAK (press + release) in LOCAL mode, which returns to the
    /// main loop (DUPLEX). Does nothing if the mode already matches or a
    /// sequence is still running. Returns whether a sequence was started.
    pub fn set_local(&mut self, on: bool) -> bool {
        if self.auto != AutoSeq::Idle || !self.booted || self.is_local() == on || self.profile.is_none() {
            return false;
        }
        if on && self.profile.is_some_and(|p| p.local_via_bit7) {
            // ROM 90.09.11: no prompt, BREAK with port 0x8D bit 7 = 0 enters LOCAL
            self.bus.local_switch = true;
        }
        if on && self.mode == TermMode::BreakPrompt {
            // the prompt is already up (BREAK pressed by hand): just answer it
            self.key_press(0x1B);
            let deadline = self.bus.now + self.ms(AUTO_TIMEOUT_MS);
            self.auto = AutoSeq::WaitMode { want: TermMode::Local, deadline };
            return true;
        }
        let until = self.bus.now + self.ms(AUTO_BREAK_MS);
        self.auto = AutoSeq::BreakDown { until, enter: on };
        self.bus.break_auto = true;
        true
    }

    /// True while a [`set_local`](Self::set_local) key sequence is running.
    pub fn local_switching(&self) -> bool {
        self.auto != AutoSeq::Idle
    }

    fn ms(&self, ms: u64) -> u64 {
        self.cfg.cpu_hz as u64 * ms / 1000
    }

    fn step_auto(&mut self) {
        let now = self.bus.now;
        self.auto = match self.auto {
            AutoSeq::Idle => AutoSeq::Idle,
            AutoSeq::BreakDown { until, enter } if now >= until => {
                self.bus.break_auto = false;
                let deadline = now + self.ms(AUTO_TIMEOUT_MS);
                if enter && self.bus.local_switch {
                    AutoSeq::WaitMode { want: TermMode::Local, deadline }
                } else if enter {
                    AutoSeq::WaitPrompt { deadline }
                } else {
                    AutoSeq::WaitMode { want: TermMode::Normal, deadline }
                }
            }
            s @ AutoSeq::BreakDown { .. } => s,
            AutoSeq::WaitPrompt { deadline } => {
                if self.mode == TermMode::BreakPrompt {
                    self.key_press(0x1B);
                    AutoSeq::WaitMode { want: TermMode::Local, deadline: now + self.ms(AUTO_TIMEOUT_MS) }
                } else if now >= deadline {
                    AutoSeq::Idle
                } else {
                    self.auto
                }
            }
            AutoSeq::WaitMode { want, deadline } => {
                if self.mode == want || now >= deadline {
                    self.bus.local_switch = false;
                    AutoSeq::Idle
                } else {
                    self.auto
                }
            }
        };
    }

    /// CAPS lock state (port 0x8D bit 6 = 0 when active; the ROM then swaps
    /// the case of letters).
    pub fn set_caps(&mut self, on: bool) {
        self.bus.caps = on;
    }

    pub fn caps(&self) -> bool {
        self.bus.caps
    }

    /// "A key is held" (port 0x8E bit 0 = 0). The ROM then auto-repeats the
    /// last key by itself, so a host which does its own auto-repeat should
    /// leave this false (the default).
    pub fn set_key_held(&mut self, held: bool) {
        self.bus.key_held = held;
    }

    // ---- sound -----------------------------------------------------------

    pub fn beep(&self) -> Beep {
        let p = &self.bus.pit;
        let active = p.mode(0) == 0 && p.reload(0).is_some() && !p.out(0);
        let freq_hz = p.output_hz(1).unwrap_or(0.0);
        Beep { active: active && freq_hz > 0.0, freq_hz }
    }

    // ---- video -----------------------------------------------------------

    /// Current value of the scroll register (port 0x80).
    pub fn scroll(&self) -> u8 {
        self.bus.scroll
    }

    pub fn line_offset(&self) -> u8 {
        self.cfg.line_offset
    }

    pub fn set_line_offset(&mut self, off: u8) {
        self.cfg.line_offset = off;
    }

    pub fn set_colors(&mut self, fg: u32, bg: u32) {
        self.cfg.fg = fg;
        self.cfg.bg = bg;
    }

    /// The 64 bytes of video memory shown on screen line `y` (0..256).
    pub fn screen_line(&self, y: usize) -> &[u8] {
        let line = (self.bus.scroll as usize + self.cfg.line_offset as usize + y) & 0xFF;
        let a = VRAM as usize + line * 64;
        &self.bus.ram[a..a + 64]
    }

    /// Pixel on screen (after scroll/offset).
    pub fn pixel(&self, x: usize, y: usize) -> bool {
        self.screen_line(y)[x / 8] & (0x80 >> (x % 8)) != 0
    }

    /// Render the 512x256 screen as 0x00RRGGBB pixels (`out.len() >= 512*256`).
    pub fn render(&self, out: &mut [u32]) {
        let (fg, bg) = (self.cfg.fg, self.cfg.bg);
        for y in 0..SCREEN_H {
            let row = &mut out[y * SCREEN_W..(y + 1) * SCREEN_W];
            for (i, &b) in self.screen_line(y).iter().enumerate() {
                for bit in 0..8 {
                    row[i * 8 + bit] = if b & (0x80 >> bit) != 0 { fg } else { bg };
                }
            }
        }
    }

    /// Render as 8-bit grey levels (255 = lit), e.g. for PGM screenshots.
    pub fn render_gray(&self) -> Vec<u8> {
        let mut v = vec![0u8; SCREEN_W * SCREEN_H];
        for y in 0..SCREEN_H {
            for (i, &b) in self.screen_line(y).iter().enumerate() {
                for bit in 0..8 {
                    if b & (0x80 >> bit) != 0 {
                        v[y * SCREEN_W + i * 8 + bit] = 255;
                    }
                }
            }
        }
        v
    }

    /// Binary PGM (P5) image of the screen.
    pub fn screenshot_pgm(&self) -> Vec<u8> {
        let mut v = format!("P5\n{} {}\n255\n", SCREEN_W, SCREEN_H).into_bytes();
        v.extend(self.render_gray());
        v
    }

    /// 6x10 bitmap of the text cell (row, col) as the ROM addresses it:
    /// memory line `16 + 10*row + scroll`, pixel x `16 + 6*col`. Each entry
    /// holds the 6 pixels in bits 7..2 (same layout as the ROM font).
    pub fn text_cell(&self, row: usize, col: usize) -> [u8; 10] {
        let mut cell = [0u8; 10];
        let x0 = TEXT_X0 + 6 * col;
        for (dy, c) in cell.iter_mut().enumerate() {
            let line = (self.bus.scroll as usize + TEXT_LINE0 + 10 * row + dy) & 0xFF;
            let a = VRAM as usize + line * 64;
            let mut bits = 0u8;
            for dx in 0..6 {
                let x = x0 + dx;
                if self.bus.ram[a + x / 8] & (0x80 >> (x % 8)) != 0 {
                    bits |= 0x80 >> dx;
                }
            }
            *c = bits;
        }
        cell
    }

    /// Glyph of character code `c` from the ROM font (10 rows, bits 7..2).
    pub fn glyph(&self, c: u8) -> [u8; 10] {
        let mut g = [0u8; 10];
        let a = FONT_BASE + 10 * c as usize;
        for (i, b) in g.iter_mut().enumerate() {
            *b = self.bus.rom.get(a + i).copied().unwrap_or(0) & 0xFC;
        }
        g
    }

    /// Decode the text screen by matching every cell against the ROM font
    /// (normal or inverse video). Unknown cells become `?`, cells matching
    /// a glyph in inverse video are returned as-is (no marking). ASCII
    /// 0x20..0x7E plus the Polish lowercase letters at codes 0x01..0x0C
    /// (see [`POLISH_GLYPHS`]) are recognised.
    pub fn text_screen(&self) -> Vec<String> {
        let mut glyphs: Vec<(char, [u8; 10])> =
            (0x20u8..0x7F).map(|c| (c as char, self.glyph(c))).collect();
        glyphs.extend(POLISH_GLYPHS.iter().map(|&(code, ch)| (ch, self.glyph(code))));
        // prefer space for an empty cell
        glyphs.sort_by_key(|&(c, _)| c != ' ');
        (0..TEXT_ROWS)
            .map(|r| {
                let mut s: String = (0..TEXT_COLS)
                    .map(|c| {
                        let cell = self.text_cell(r, c);
                        let inv = cell.map(|b| !b & 0xFC);
                        glyphs
                            .iter()
                            .find(|(_, g)| *g == cell || *g == inv)
                            .map_or('?', |&(ch, _)| ch)
                    })
                    .collect();
                s.truncate(s.trim_end().len());
                s
            })
            .collect()
    }

    // ---- debugging -------------------------------------------------------

    pub fn peek(&self, addr: u16) -> u8 {
        if addr < 0x4000 && self.bus.rom_enabled {
            self.bus.rom[addr as usize]
        } else {
            self.bus.ram[addr as usize]
        }
    }

    pub fn ram(&self) -> &[u8] {
        &self.bus.ram[..]
    }

    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.bus.ram[..]
    }

    pub fn pc(&self) -> u16 {
        self.cpu.immutable_registers().pc()
    }

    pub fn rom_enabled(&self) -> bool {
        self.bus.rom_enabled
    }

    /// Number of BREAK conditions the 8251 transmitted (command bit 3).
    pub fn breaks_sent(&self) -> u64 {
        self.break_count
    }

    /// Number of writes to ports 0x0A/0x0B (ROM on/off) so far.
    pub fn rom_switch_count(&self) -> u64 {
        self.bus.rom_switches
    }

    /// Drain the port access log (filled only with `Config::trace_io`):
    /// accesses to ports the model does not implement, plus 0x0A/0x0B.
    pub fn take_io_log(&mut self) -> Vec<IoEvent> {
        std::mem::take(&mut self.bus.io_log)
    }

    pub fn usart(&self) -> &Usart8251 {
        &self.bus.usart
    }

    pub fn pit(&self) -> &Pit8253 {
        &self.bus.pit
    }
}

/// Byte stream that uploads a terminal-side binary (`.TER`, `org 0x4000`)
/// through `ESC i <page>` and starts it with `ESC 0`, exactly like
/// `RUNTERM.COM` in the ROM analysis repository: first a `JP 0x4000`
/// trampoline into the `ESC 0` slot (page 0x60), then the code from page
/// 0x40, then `ESC 0`.
pub fn ter_upload_stream(code: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(code.len() + 600);
    let mut tramp = [0xC9u8; 256];
    tramp[..3].copy_from_slice(&[0xC3, 0x00, 0x40]);
    v.extend_from_slice(&[0x1B, b'i', 0x60]);
    v.extend_from_slice(&tramp);
    for (i, chunk) in code.chunks(256).enumerate() {
        v.extend_from_slice(&[0x1B, b'i', 0x40 + i as u8]);
        v.extend_from_slice(chunk);
        v.extend(std::iter::repeat(0).take(256 - chunk.len()));
    }
    v.extend_from_slice(&[0x1B, b'0']);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_stream_layout() {
        let s = ter_upload_stream(&[1, 2, 3]);
        assert_eq!(s.len(), 3 + 256 + 3 + 256 + 2);
        assert_eq!(&s[..6], &[0x1B, b'i', 0x60, 0xC3, 0x00, 0x40]);
        assert_eq!(s[6], 0xC9);
        assert_eq!(&s[259..265], &[0x1B, b'i', 0x40, 1, 2, 3]);
        assert_eq!(&s[s.len() - 2..], b"\x1b0");
    }

    #[test]
    fn rom_switch_ports() {
        let mut b = Ang3001::with_default_rom();
        b.bus.poke(0x0000, 0x77);
        assert_eq!(b.bus.peek(0x0000), 0xC3); // ROM visible, write went to RAM
        b.bus.port_out(0x0B, 0);
        assert_eq!(b.bus.peek(0x0000), 0x77);
        b.bus.port_out(0x0A, 0);
        assert_eq!(b.bus.peek(0x0000), 0xC3);
    }

    #[test]
    fn keyboard_ports() {
        let mut m = Ang3001::with_default_rom();
        assert_eq!(m.bus.port_in(0x8D), 0xF0);
        assert_eq!(m.bus.port_in(0x8E) & 0x21, 0x01);
        m.key_press(b'a');
        m.key_press(b'b');
        assert_eq!(m.bus.port_in(0x8E) & 0x20, 0x20);
        assert_eq!(m.bus.port_in(0x8C), b'a');
        assert_eq!(m.bus.port_in(0x8E) & 0x20, 0x20); // next key strobed
        assert_eq!(m.bus.port_in(0x8C), b'b');
        assert_eq!(m.bus.port_in(0x8E) & 0x20, 0);
        m.set_caps(true);
        m.set_break(true);
        assert_eq!(m.bus.port_in(0x8D), 0x90);
        m.set_stop(true);
        assert_eq!(m.bus.port_in(0x8D), 0x80);
        m.set_break(false);
        m.bus.break_auto = true; // automatic BREAK of set_local ORs in
        assert_eq!(m.bus.port_in(0x8D), 0x80);
    }
}
