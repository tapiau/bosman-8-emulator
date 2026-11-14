//! The ANG-3001 terminal inside the GUI: an emulator thread running the
//! `ang3001` core ([`TermEmu`]), the screen drawn as an egui texture, the
//! keyboard mapped from egui events ([`key_bytes`], [`fkey_action`]) and the
//! terminal's button bar ([`bar`]).

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ang3001::{Ang3001, ArrowMode, Config, TermMode, SCREEN_H, SCREEN_W};
use b8_link::Endpoint;
use eframe::egui::{self, Color32, Key, Modifiers, RichText, Ui};

/// Commands from the UI to the terminal thread.
pub enum TCmd {
    Key(u8),
    /// Type text once the ROM has booted (one key at a time, like `--type`).
    Type(Vec<u8>),
    /// Bytes as if received over the line, after boot (`--inject`, `--load-ter`).
    Inject(Vec<u8>),
    Break(bool),
    Reset,
    Caps(bool),
    Stop(bool),
    /// Switch LOCAL on / off with the key sequence a user would type.
    Local(bool),
    Turbo(bool),
    Colors(u32, u32),
    LineOffset(u8),
    Link(Box<dyn Endpoint>),
    /// Reply with the decoded text screen (tests, diagnostics).
    #[cfg_attr(not(test), allow(dead_code))]
    Text(Sender<Vec<String>>),
    Quit,
}

/// What the terminal thread publishes after every frame.
#[derive(Clone)]
pub struct TermFrame {
    /// `SCREEN_W` × `SCREEN_H` pixels, 0x00RRGGBB.
    pub pixels: Vec<u32>,
    /// Increments with every new frame.
    pub seq: u64,
    pub mode: TermMode,
    pub caps: bool,
    pub stop: bool,
    pub local_pending: bool,
    pub beep: Option<f64>,
    pub booted: bool,
    pub link: String,
    pub link_connected: bool,
}

impl Default for TermFrame {
    fn default() -> Self {
        TermFrame {
            pixels: vec![0; SCREEN_W * SCREEN_H],
            seq: 0,
            mode: TermMode::Normal,
            caps: false,
            stop: false,
            local_pending: false,
            beep: None,
            booted: false,
            link: String::new(),
            link_connected: false,
        }
    }
}

/// Handle of a running terminal thread.
pub struct TermEmu {
    tx: Sender<TCmd>,
    frame: Arc<Mutex<TermFrame>>,
    handle: Option<JoinHandle<()>>,
}

impl TermEmu {
    pub fn start(rom: Vec<u8>, cfg: Config, link: Box<dyn Endpoint>, turbo: bool) -> TermEmu {
        let (tx, rx) = mpsc::channel();
        let frame = Arc::new(Mutex::new(TermFrame::default()));
        let f = frame.clone();
        let handle = std::thread::Builder::new()
            .name("ang3001-emu".into())
            .spawn(move || run(Ang3001::new(&rom, cfg), link, turbo, rx, f))
            .expect("spawn terminal thread");
        TermEmu { tx, frame, handle: Some(handle) }
    }

    pub fn send(&self, c: TCmd) {
        let _ = self.tx.send(c);
    }

    pub fn frame(&self) -> MutexGuard<'_, TermFrame> {
        self.frame.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for TermEmu {
    fn drop(&mut self) {
        let _ = self.tx.send(TCmd::Quit);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

const FRAME: Duration = Duration::from_millis(20);

fn run(mut m: Ang3001, link: Box<dyn Endpoint>, mut turbo: bool, rx: Receiver<TCmd>, out: Arc<Mutex<TermFrame>>) {
    m.attach_link(link);
    let slice = m.config().cpu_hz as u64 / 50;
    let mut typed: VecDeque<u8> = VecDeque::new();
    let mut pixels = vec![0u32; SCREEN_W * SCREEN_H];
    let mut seq = 0u64;
    let mut next = Instant::now();
    loop {
        loop {
            match rx.try_recv() {
                Ok(TCmd::Key(k)) => m.key_press(k),
                Ok(TCmd::Type(t)) => typed.extend(t),
                Ok(TCmd::Inject(b)) => m.inject_rx(&b),
                Ok(TCmd::Break(b)) => m.set_break(b),
                Ok(TCmd::Reset) => m.reset(),
                Ok(TCmd::Caps(c)) => m.set_caps(c),
                Ok(TCmd::Stop(s)) => m.set_stop(s),
                Ok(TCmd::Local(l)) => {
                    m.set_local(l);
                }
                Ok(TCmd::Turbo(t)) => turbo = t,
                Ok(TCmd::Colors(fg, bg)) => m.set_colors(fg, bg),
                Ok(TCmd::LineOffset(o)) => m.set_line_offset(o),
                Ok(TCmd::Link(l)) => m.attach_link(l),
                Ok(TCmd::Text(reply)) => {
                    let _ = reply.send(m.text_screen());
                }
                Ok(TCmd::Quit) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }
        if m.booted() && m.keys_pending() == 0 {
            if let Some(k) = typed.pop_front() {
                m.key_press(k);
            }
        }
        if turbo {
            let until = Instant::now() + FRAME;
            while Instant::now() < until {
                m.run_cycles(slice);
            }
        } else {
            m.run_cycles(slice);
        }
        m.take_io_log();
        m.render(&mut pixels);
        seq += 1;
        {
            let b = m.beep();
            let link = m.link();
            let mut f = out.lock().unwrap_or_else(|e| e.into_inner());
            f.pixels.copy_from_slice(&pixels);
            f.seq = seq;
            f.mode = m.mode();
            f.caps = m.caps();
            f.stop = m.stop();
            f.local_pending = m.local_switching();
            f.beep = b.active.then_some(b.freq_hz);
            f.booted = m.booted();
            f.link = link.map(|l| l.describe()).unwrap_or_default();
            f.link_connected = link.is_some_and(|l| l.status().connected);
        }
        next += FRAME;
        let now = Instant::now();
        if turbo || next < now {
            next = now;
        } else {
            std::thread::sleep(next - now);
        }
    }
}

// ---- keyboard ---------------------------------------------------------------

/// Terminal functions on F6-F12 (as in the former `ang3001` window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FAction {
    ArrowsNext,
    Local,
    Stop,
    Turbo,
    Caps,
    Reset,
}

/// Function key → terminal action (F12 = BREAK is a held key, see [`TermView`]).
pub fn fkey_action(key: Key) -> Option<FAction> {
    match key {
        Key::F6 => Some(FAction::ArrowsNext),
        Key::F7 => Some(FAction::Local),
        Key::F8 => Some(FAction::Stop),
        Key::F9 => Some(FAction::Turbo),
        Key::F10 => Some(FAction::Caps),
        Key::F11 => Some(FAction::Reset),
        _ => None,
    }
}

/// Bytes a key press sends (keys that don't come as text events): Enter,
/// Backspace, Tab, Esc, Delete, arrows (per `arrows`), Ctrl combinations.
pub fn key_bytes(key: Key, mods: Modifiers, arrows: ArrowMode) -> Vec<u8> {
    let arrow = |i: usize| arrows.keys()[i].to_vec();
    match key {
        Key::Enter => vec![b'\r'],
        Key::Backspace => vec![0x08],
        Key::Tab => vec![b'\t'],
        Key::Escape => vec![0x1B],
        Key::Delete => vec![0x7F],
        Key::ArrowUp => arrow(0),
        Key::ArrowDown => arrow(1),
        Key::ArrowRight => arrow(2),
        Key::ArrowLeft => arrow(3),
        _ if mods.ctrl && !mods.alt => {
            let name = key.name();
            let c = match key {
                Key::OpenBracket => Some('['),
                Key::Backslash => Some('\\'),
                Key::CloseBracket => Some(']'),
                Key::Minus => Some('-'),
                Key::Space => Some(' '),
                Key::Num2 => Some('2'),
                Key::Num6 => Some('6'),
                _ if name.len() == 1 => name.chars().next(),
                _ => None,
            };
            c.and_then(ang3001::ctrl_code).map(|b| vec![b]).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Text typed (egui `Event::Text`) → key codes.
pub fn text_bytes(text: &str) -> Vec<u8> {
    text.chars().filter_map(ang3001::char_code).collect()
}

// ---- view -------------------------------------------------------------------

/// The terminal screen and its keyboard: keeps the texture and UI-side state
/// (BREAK held, arrow mode).
pub struct TermView {
    texture: Option<egui::TextureHandle>,
    seq: u64,
    pub arrows: ArrowMode,
    break_mouse: bool,
    break_key: bool,
}

impl TermView {
    pub fn new(arrows: ArrowMode) -> TermView {
        TermView { texture: None, seq: 0, arrows, break_mouse: false, break_key: false }
    }

    /// Draw the screen into the available space (4:3, centred).
    pub fn screen(&mut self, ui: &mut Ui, term: &TermEmu) {
        let f = term.frame();
        if self.texture.is_none() || f.seq != self.seq {
            let rgb: Vec<u8> = f.pixels.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8]).collect();
            let img = egui::ColorImage::from_rgb([SCREEN_W, SCREEN_H], &rgb);
            match &mut self.texture {
                Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                None => self.texture = Some(ui.ctx().load_texture("ang3001-screen", img, egui::TextureOptions::NEAREST)),
            }
            self.seq = f.seq;
        }
        drop(f);
        let avail = ui.available_size();
        // 512×256 pixels shown at 4:3 (pixels 2:3, as on the original monitor).
        let w = avail.x.min(avail.y * 4.0 / 3.0).max(64.0);
        let size = egui::vec2(w, w * 3.0 / 4.0);
        let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        let r = egui::Rect::from_center_size(rect.center(), size);
        if let Some(t) = &self.texture {
            ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
    }

    /// Route keyboard input to the terminal (unless a text field has focus).
    /// Returns the F-key actions for the caller to apply.
    pub fn keyboard(&mut self, ctx: &egui::Context, term: &TermEmu) -> Vec<FAction> {
        let mut actions = Vec::new();
        if ctx.wants_keyboard_input() {
            return actions;
        }
        let (events, f12) = ctx.input(|i| (i.events.clone(), i.key_down(Key::F12)));
        for e in events {
            match e {
                egui::Event::Text(t) => {
                    for b in text_bytes(&t) {
                        term.send(TCmd::Key(b));
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, repeat, .. } => {
                    if let Some(a) = fkey_action(key) {
                        if !repeat {
                            actions.push(a);
                        }
                        continue;
                    }
                    for b in key_bytes(key, modifiers, self.arrows) {
                        term.send(TCmd::Key(b));
                    }
                }
                _ => {}
            }
        }
        if f12 != self.break_key {
            self.break_key = f12;
            term.send(TCmd::Break(self.break_key || self.break_mouse));
        }
        actions
    }

    /// Apply an F-key / button action.
    pub fn apply(&mut self, a: FAction, term: &TermEmu) {
        let f = term.frame().clone_state();
        match a {
            FAction::ArrowsNext => self.arrows = self.arrows.next(),
            FAction::Local => term.send(TCmd::Local(f.mode != TermMode::Local)),
            FAction::Stop => term.send(TCmd::Stop(!f.stop)),
            FAction::Turbo => {} // shared with the computer: handled by the application
            FAction::Caps => term.send(TCmd::Caps(!f.caps)),
            FAction::Reset => term.send(TCmd::Reset),
        }
    }

    /// The terminal's buttons and lamps (bottom bar). Returns actions to apply.
    pub fn bar(&mut self, ui: &mut Ui, term: &TermEmu, fg: Color32) -> Vec<FAction> {
        let f = term.frame().clone_state();
        let mut actions = Vec::new();
        let key = |ui: &mut Ui, label: &str, hint: &str, lit: bool| {
            ui.add(egui::Button::new(RichText::new(label).strong()).shortcut_text(hint).selected(lit))
        };
        if key(ui, "RESET", "F11", false).on_hover_text("Reset terminala (procesor i układy; RAM zostaje)").clicked() {
            actions.push(FAction::Reset);
        }
        let brk = key(ui, "BREAK", "F12", self.break_mouse || self.break_key)
            .on_hover_text("Klawisz BREAK — trzymany, dopóki wciśnięty");
        let down = brk.is_pointer_button_down_on();
        if down != self.break_mouse {
            self.break_mouse = down;
            term.send(TCmd::Break(self.break_key || self.break_mouse));
        }
        let local = f.mode == TermMode::Local;
        if key(ui, "LOCAL", "F7", local || f.local_pending)
            .on_hover_text("LOCAL: klawiatura pisze na ekran, nic nie idzie do komputera")
            .clicked()
        {
            actions.push(FAction::Local);
        }
        if key(ui, "STOP", "F8", f.stop).on_hover_text("Stronicowanie: po ekranie tekstu STOP i czekanie na klawisz").clicked() {
            actions.push(FAction::Stop);
        }
        if key(ui, "CAPS", "F10", f.caps).on_hover_text("CAPS: ROM odwraca wielkość liter").clicked() {
            actions.push(FAction::Caps);
        }
        ui.separator();
        ui.label(RichText::new("ARROWS").small());
        egui::ComboBox::from_id_salt("arrows")
            .width(64.0)
            .selected_text(String::from_utf8_lossy(self.arrows.label()))
            .show_ui(ui, |ui| {
                for m in ArrowMode::ALL {
                    ui.selectable_value(&mut self.arrows, m, String::from_utf8_lossy(m.describe()));
                }
            })
            .response
            .on_hover_text("Co wysyłają strzałki (F6 = następny tryb)");
        ui.separator();
        lamp(ui, f.link_connected, fg, "LINK").on_hover_text(&f.link);
        lamp(ui, f.beep.is_some(), fg, "BEEP")
            .on_hover_text(f.beep.map(|h| format!("{h:.0} Hz")).unwrap_or_else(|| "brzęczyk".into()));
        lamp(ui, f.mode == TermMode::Stopped, fg, "WAIT").on_hover_text("Terminal czeka w STOP — naciśnij klawisz");
        actions
    }
}

/// Small copy of the frame state (without the pixels).
pub struct FrameState {
    pub mode: TermMode,
    pub caps: bool,
    pub stop: bool,
    pub local_pending: bool,
    pub beep: Option<f64>,
    pub link: String,
    pub link_connected: bool,
}

impl TermFrame {
    pub fn clone_state(&self) -> FrameState {
        FrameState {
            mode: self.mode,
            caps: self.caps,
            stop: self.stop,
            local_pending: self.local_pending,
            beep: self.beep,
            link: self.link.clone(),
            link_connected: self.link_connected,
        }
    }
}

/// A lamp with a label.
pub fn lamp(ui: &mut Ui, on: bool, color: Color32, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 5.0, if on { color } else { Color32::from_gray(70) });
        ui.label(RichText::new(label).small());
    })
    .response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_like_the_old_window() {
        let none = Modifiers::NONE;
        assert_eq!(key_bytes(Key::Enter, none, ArrowMode::Numpad), b"\r");
        assert_eq!(key_bytes(Key::Backspace, none, ArrowMode::Numpad), [0x08]);
        assert_eq!(key_bytes(Key::Delete, none, ArrowMode::Numpad), [0x7F]);
        assert_eq!(key_bytes(Key::ArrowUp, none, ArrowMode::Numpad), b"8");
        assert_eq!(key_bytes(Key::ArrowLeft, none, ArrowMode::Vim), b"h");
        assert_eq!(key_bytes(Key::ArrowRight, none, ArrowMode::Vt52), b"\x1bC");
        assert!(key_bytes(Key::ArrowDown, none, ArrowMode::None).is_empty());
        assert_eq!(key_bytes(Key::C, Modifiers::CTRL, ArrowMode::Numpad), [0x03]);
        assert_eq!(key_bytes(Key::OpenBracket, Modifiers::CTRL, ArrowMode::Numpad), [0x1B]);
        assert_eq!(key_bytes(Key::Space, Modifiers::CTRL, ArrowMode::Numpad), [0x00]);
        assert!(key_bytes(Key::A, none, ArrowMode::Numpad).is_empty(), "letters come as text events");
        assert_eq!(text_bytes("Ab łę"), vec![b'A', b'b', b' ', 0x03, 0x01]);
        assert_eq!(fkey_action(Key::F7), Some(FAction::Local));
        assert_eq!(fkey_action(Key::F12), None);
    }
}
