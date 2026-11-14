//! Button bar under the emulated screen, drawn into the minifb buffer with
//! the terminal ROM's own 6x10 font: function-key style buttons (RESET,
//! BREAK, LOCAL, STOP, CAPS, TURBO), the ARROWS drop-down (what the arrow
//! keys send, see [`ArrowMode`]) and status LEDs (LINK, BEEP, WAIT).
//!
//! Mouse handling: a button is "pressed" on the press edge of the left
//! button over it and acts on the release edge if the pointer is still over
//! it (like a desktop button); BREAK is momentary and held while pressed.
//! The drop-down opens a list above the bar ([`Toolbar::draw_popup`]); a click
//! on an entry selects it, a click anywhere else closes the list.

/// What the arrow keys (up, down, right, left) send.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArrowMode {
    /// Digits 8 / 2 / 6 / 4, like a numeric keypad (games: SOKOBAN, ...).
    Numpad,
    /// `ESC A` / `ESC B` / `ESC C` / `ESC D`.
    Vt52,
    /// WordStar: ^E / ^X / ^D / ^S.
    WordStar,
    /// vi / vim: k / j / l / h.
    Vim,
    /// Nothing.
    None,
}

impl ArrowMode {
    pub const ALL: [ArrowMode; 5] =
        [ArrowMode::Numpad, ArrowMode::Vt52, ArrowMode::WordStar, ArrowMode::Vim, ArrowMode::None];

    /// Bytes sent for up, down, right, left.
    pub fn keys(self) -> [&'static [u8]; 4] {
        match self {
            ArrowMode::Numpad => [b"8", b"2", b"6", b"4"],
            ArrowMode::Vt52 => [b"\x1bA", b"\x1bB", b"\x1bC", b"\x1bD"],
            ArrowMode::WordStar => [b"\x05", b"\x18", b"\x04", b"\x13"],
            ArrowMode::Vim => [b"k", b"j", b"l", b"h"],
            ArrowMode::None => [b"", b"", b"", b""],
        }
    }

    /// Short name shown on the bar (at most 4 characters).
    pub fn label(self) -> &'static [u8] {
        match self {
            ArrowMode::Numpad => b"2468",
            ArrowMode::Vt52 => b"VT52",
            ArrowMode::WordStar => b"WS",
            ArrowMode::Vim => b"VIM",
            ArrowMode::None => b"NONE",
        }
    }

    /// Entry in the drop-down list.
    pub fn describe(self) -> &'static [u8] {
        match self {
            ArrowMode::Numpad => b"2468      8 2 4 6",
            ArrowMode::Vt52 => b"VT52      ESC A B D C",
            ArrowMode::WordStar => b"WORDSTAR  ^E ^X ^S ^D",
            ArrowMode::Vim => b"VIM       k j h l",
            ArrowMode::None => b"NONE",
        }
    }

    /// Next mode (F6 cycles through them).
    pub fn next(self) -> ArrowMode {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Command-line name.
    pub fn parse(s: &str) -> Option<ArrowMode> {
        match s.to_ascii_lowercase().as_str() {
            "numpad" | "2468" => Some(ArrowMode::Numpad),
            "vt52" => Some(ArrowMode::Vt52),
            "wordstar" | "ws" => Some(ArrowMode::WordStar),
            "vim" | "vi" => Some(ArrowMode::Vim),
            "none" | "off" => Some(ArrowMode::None),
            _ => None,
        }
    }
}

/// What a button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Reset,
    Break,
    Local,
    Stop,
    Caps,
    Turbo,
    /// Arrow-key mode chosen from the drop-down.
    Arrows(ArrowMode),
    /// F6: next arrow-key mode.
    ArrowsNext,
    /// The drop-down button itself (opens / closes the list; internal).
    ArrowsMenu,
}

/// What the bar shows, collected from the machine every frame.
#[derive(Clone, Copy, Default)]
pub struct State {
    pub break_held: bool,
    pub local: bool,
    /// The LOCAL on/off key sequence is still running.
    pub local_pending: bool,
    pub stop: bool,
    pub caps: bool,
    pub turbo: bool,
    /// The ROM waits in the STOP loop (page full).
    pub waiting: bool,
    pub beep: bool,
    pub link: bool,
    pub arrows: ArrowMode,
}

impl Default for ArrowMode {
    fn default() -> Self {
        ArrowMode::Numpad
    }
}

struct Button {
    action: Action,
    label: &'static [u8],
    hint: &'static [u8],
    x: usize,
    w: usize,
}

struct Led {
    label: &'static [u8],
    x: usize,
}

const BUTTONS: [(Action, &[u8], &[u8]); 6] = [
    (Action::Reset, b"RESET", b"F11"),
    (Action::Break, b"BREAK", b"F12"),
    (Action::Local, b"LOCAL", b"F7"),
    (Action::Stop, b"STOP", b"F8"),
    (Action::Caps, b"CAPS", b"F10"),
    (Action::Turbo, b"TURBO", b"F9"),
];
const LEDS: [&[u8]; 3] = [b"LINK", b"BEEP", b"WAIT"];

const BAR_BG: u32 = 0x1C1C1C;
const KEY_FACE: u32 = 0x3C3C3C;
const KEY_LIGHT: u32 = 0x666666;
const KEY_SHADOW: u32 = 0x0C0C0C;
const KEY_TEXT: u32 = 0xD8D8D8;
const KEY_HINT: u32 = 0x9A9A9A;
const LIT_TEXT: u32 = 0x101010;
const LED_OFF: u32 = 0x303030;

pub struct Toolbar {
    font: Vec<[u8; 10]>,
    /// Label font scale and F-key hint scale.
    fs: usize,
    hs: usize,
    y0: usize,
    w: usize,
    h: usize,
    buttons: Vec<Button>,
    /// ARROWS drop-down button: x, width.
    dropdown: (usize, usize),
    menu_open: bool,
    leds: Vec<Led>,
    pressed: Option<Action>,
    mouse_was_down: bool,
}

impl Toolbar {
    /// Bar height for label font scale `fs`.
    pub fn height(fs: usize) -> usize {
        24 * fs
    }

    /// `font[c]` = 6x10 glyph of code `c` (pixels in bits 7..2), `width` =
    /// buffer width, `y0` = first buffer line of the bar.
    pub fn new(font: Vec<[u8; 10]>, width: usize, y0: usize, fs: usize) -> Self {
        let hs = fs.saturating_sub(1).max(1);
        let (pad, gap, margin) = (6 * fs, 3 * fs, 4 * fs);
        let mut x = margin;
        let mut buttons = Vec::new();
        for (action, label, hint) in BUTTONS {
            let w = pad + 6 * fs * label.len() + gap + 6 * hs * hint.len() + pad;
            buttons.push(Button { action, label, hint, x, w });
            x += w + gap;
        }
        x += gap;
        let dd_w = pad + 6 * fs * 4 + gap + 6 * hs * 2 + gap + 4 * fs + pad;
        let dropdown = (x, dd_w);
        x += dd_w + gap;
        // LEDs right-aligned, only if they fit
        let led_w = |l: &[u8]| 8 * fs + gap + 6 * fs * l.len();
        let total: usize = LEDS.iter().map(|l| led_w(l) + 4 * fs).sum();
        let mut leds = Vec::new();
        if x + total + margin <= width {
            let mut lx = width - margin - total + 4 * fs;
            for label in LEDS {
                leds.push(Led { label, x: lx });
                lx += led_w(label) + 4 * fs;
            }
        }
        Self {
            font,
            fs,
            hs,
            y0,
            w: width,
            h: Self::height(fs),
            buttons,
            dropdown,
            menu_open: false,
            leds,
            pressed: None,
            mouse_was_down: false,
        }
    }

    fn key_rect(&self, b: &Button) -> (usize, usize, usize, usize) {
        let m = 3 * self.fs;
        (b.x, self.y0 + m, b.w, self.h - 2 * m)
    }

    fn dropdown_rect(&self) -> (usize, usize, usize, usize) {
        let m = 3 * self.fs;
        (self.dropdown.0, self.y0 + m, self.dropdown.1, self.h - 2 * m)
    }

    /// Drop-down list: panel rectangle and the height of one entry.
    fn menu_rect(&self) -> (usize, usize, usize, usize, usize) {
        let fs = self.fs;
        let item_h = 14 * fs;
        let longest = ArrowMode::ALL.iter().map(|m| m.describe().len()).max().unwrap_or(0);
        let pw = 6 * fs * longest + 16 * fs;
        let ph = item_h * (ArrowMode::ALL.len() + 1) + 4 * fs; // + title row
        let px = self.dropdown.0.min(self.w.saturating_sub(pw));
        let py = self.y0.saturating_sub(ph);
        (px, py, pw, ph, item_h)
    }

    /// Entry of the open list under the pointer.
    fn menu_hit(&self, mx: usize, my: usize) -> Option<ArrowMode> {
        let (px, py, pw, _, ih) = self.menu_rect();
        let top = py + 2 * self.fs + ih; // below the title
        if mx < px || mx >= px + pw || my < top {
            return None;
        }
        ArrowMode::ALL.get((my - top) / ih).copied()
    }

    fn hit(&self, pos: Option<(f32, f32)>) -> Option<Action> {
        let (mx, my) = pos?;
        if mx < 0.0 || my < 0.0 {
            return None;
        }
        let (mx, my) = (mx as usize, my as usize);
        let inside = |(x, y, w, h): (usize, usize, usize, usize)| mx >= x && mx < x + w && my >= y && my < y + h;
        if inside(self.dropdown_rect()) {
            return Some(Action::ArrowsMenu);
        }
        self.buttons.iter().find_map(|b| inside(self.key_rect(b)).then_some(b.action))
    }

    /// Feed the mouse state (window coordinates, left button). Returns the
    /// action of a completed click.
    pub fn mouse(&mut self, pos: Option<(f32, f32)>, down: bool) -> Option<Action> {
        let mut click = None;
        if self.menu_open && down && !self.mouse_was_down {
            // Any press while the list is open closes it; on an entry it selects.
            self.menu_open = false;
            self.mouse_was_down = true;
            let p = pos.filter(|&(x, y)| x >= 0.0 && y >= 0.0).map(|(x, y)| (x as usize, y as usize));
            if let Some(mode) = p.and_then(|(x, y)| self.menu_hit(x, y)) {
                return Some(Action::Arrows(mode));
            }
            return None;
        }
        if down && !self.mouse_was_down {
            self.pressed = self.hit(pos);
        } else if !down && self.mouse_was_down {
            if let Some(p) = self.pressed.take() {
                if self.hit(pos) == Some(p) {
                    if p == Action::ArrowsMenu {
                        self.menu_open = true;
                    } else {
                        click = Some(p);
                    }
                }
            }
        }
        self.mouse_was_down = down;
        click
    }

    /// BREAK button held down with the mouse.
    pub fn break_held(&self) -> bool {
        self.pressed == Some(Action::Break)
    }

    /// Draw the bar into `buf` (`self.w` pixels per line).
    pub fn draw(&self, buf: &mut [u32], st: &State, fg: u32) {
        let w = self.w;
        fill(buf, w, 0, self.y0, w, self.h, BAR_BG);
        fill(buf, w, 0, self.y0, w, 1, KEY_SHADOW);
        for b in &self.buttons {
            let (x, y, bw, bh) = self.key_rect(b);
            let lit = match b.action {
                Action::Break => st.break_held,
                Action::Local => st.local,
                Action::Stop => st.stop,
                Action::Caps => st.caps,
                Action::Turbo => st.turbo,
                _ => false,
            };
            let pending = b.action == Action::Local && st.local_pending;
            let down = self.pressed == Some(b.action) || (b.action == Action::Break && st.break_held);
            let face = if lit {
                fg
            } else if pending {
                blend(fg, KEY_FACE, 0.35)
            } else {
                KEY_FACE
            };
            let (tl, br) = if down { (KEY_SHADOW, KEY_LIGHT) } else { (KEY_LIGHT, KEY_SHADOW) };
            let e = self.fs; // bevel width
            fill(buf, w, x, y, bw, bh, br);
            fill(buf, w, x, y, bw - e, bh - e, tl);
            fill(buf, w, x + e, y + e, bw - 2 * e, bh - 2 * e, face);
            let (text, hint) = if lit { (LIT_TEXT, LIT_TEXT) } else { (KEY_TEXT, KEY_HINT) };
            let shift = if down { self.fs } else { 0 };
            let ty = y + (bh - 10 * self.fs) / 2 + shift;
            let tx = x + 6 * self.fs + shift;
            self.text(buf, tx, ty, b.label, self.fs, text);
            let hx = tx + 6 * self.fs * b.label.len() + 3 * self.fs;
            let hy = ty + 10 * self.fs - 10 * self.hs;
            self.text(buf, hx, hy, b.hint, self.hs, hint);
        }
        // ARROWS drop-down: current mode, F6 hint, a small down triangle.
        {
            let (x, y, bw, bh) = self.dropdown_rect();
            let fs = self.fs;
            let down = self.pressed == Some(Action::ArrowsMenu) || self.menu_open;
            let (tl, br) = if down { (KEY_SHADOW, KEY_LIGHT) } else { (KEY_LIGHT, KEY_SHADOW) };
            fill(buf, w, x, y, bw, bh, br);
            fill(buf, w, x, y, bw - fs, bh - fs, tl);
            fill(buf, w, x + fs, y + fs, bw - 2 * fs, bh - 2 * fs, KEY_FACE);
            let ty = y + (bh - 10 * fs) / 2;
            let tx = x + 6 * fs;
            self.text(buf, tx, ty, st.arrows.label(), fs, KEY_TEXT);
            let hx = tx + 6 * fs * 4 + 3 * fs;
            self.text(buf, hx, ty + 10 * fs - 10 * self.hs, b"F6", self.hs, KEY_HINT);
            let ax = x + bw - 6 * fs - 4 * fs;
            for r in 0..3 {
                fill(buf, w, ax + r * fs, ty + (4 + r) * fs, (4 - 2 * r) * fs, fs, KEY_TEXT);
            }
        }
        for l in &self.leds {
            let on = match l.label {
                b"LINK" => st.link,
                b"BEEP" => st.beep,
                _ => st.waiting,
            };
            let fs = self.fs;
            let ly = self.y0 + (self.h - 10 * fs) / 2;
            fill(buf, w, l.x, ly + 3 * fs, 8 * fs, 4 * fs, if on { fg } else { LED_OFF });
            self.text(buf, l.x + 8 * fs + 3 * fs, ly, l.label, fs, if on { KEY_TEXT } else { KEY_HINT });
        }
    }

    /// Draw the open drop-down list (over the screen, above the bar). Call
    /// after the screen has been copied into `buf`.
    pub fn draw_popup(&self, buf: &mut [u32], st: &State, fg: u32) {
        if !self.menu_open {
            return;
        }
        let (w, fs) = (self.w, self.fs);
        let (px, py, pw, ph, ih) = self.menu_rect();
        fill(buf, w, px, py, pw, ph, KEY_LIGHT);
        fill(buf, w, px + fs, py + fs, pw - 2 * fs, ph - 2 * fs, BAR_BG);
        let mut y = py + 2 * fs;
        self.text(buf, px + 8 * fs, y + 2 * fs, b"ARROWS", fs, KEY_HINT);
        y += ih;
        for m in ArrowMode::ALL {
            let cur = m == st.arrows;
            if cur {
                fill(buf, w, px + 2 * fs, y, pw - 4 * fs, ih, fg);
            }
            self.text(buf, px + 8 * fs, y + 2 * fs, m.describe(), fs, if cur { LIT_TEXT } else { KEY_TEXT });
            y += ih;
        }
    }

    #[cfg(test)]
    fn menu_open(&self) -> bool {
        self.menu_open
    }

    fn text(&self, buf: &mut [u32], x: usize, y: usize, s: &[u8], scale: usize, color: u32) {
        let w = self.w;
        for (i, &c) in s.iter().enumerate() {
            let g = &self.font[c as usize];
            let cx = x + 6 * scale * i;
            for (dy, &row) in g.iter().enumerate() {
                for dx in 0..6 {
                    if row & (0x80 >> dx) != 0 {
                        fill(buf, w, cx + dx * scale, y + dy * scale, scale, scale, color);
                    }
                }
            }
        }
    }
}

fn fill(buf: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, c: u32) {
    for yy in y..y + h {
        let row = yy * stride;
        if row + stride > buf.len() {
            break;
        }
        let x1 = (x + w).min(stride);
        if x < x1 {
            buf[row + x..row + x1].fill(c);
        }
    }
}

fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (ca, cb) = (((a >> s) & 0xFF) as f32, ((b >> s) & 0xFF) as f32);
        ((ca * t + cb * (1.0 - t)).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(width: usize, fs: usize) -> Toolbar {
        Toolbar::new(vec![[0xFC; 10]; 256], width, 100, fs)
    }

    #[test]
    fn layout_fits_window() {
        for (width, fs) in [(1024, 2), (512, 1)] {
            let t = bar(width, fs);
            let last = t.buttons.last().unwrap();
            assert!(last.x + last.w <= t.dropdown.0);
            assert!(t.dropdown.0 + t.dropdown.1 <= width);
            if let Some(l) = t.leds.first() {
                assert!(t.dropdown.0 + t.dropdown.1 <= l.x, "drop-down before the LEDs at width {width}");
            }
            let (px, py, pw, _, _) = t.menu_rect();
            assert!(px + pw <= width && py < 100);
        }
        // the LEDs fit at the default size (--scale 3: 1024 px); at 512 px
        // they are left out (the link state is in the window title as well)
        assert_eq!(bar(1024, 2).leds.len(), 3);
    }

    #[test]
    fn arrows_dropdown_opens_selects_and_closes() {
        let mut t = bar(1024, 2);
        let (x, y, w, h) = t.dropdown_rect();
        let on_button = Some(((x + w / 2) as f32, (y + h / 2) as f32));
        assert_eq!(t.mouse(on_button, true), None);
        assert_eq!(t.mouse(on_button, false), None);
        assert!(t.menu_open());
        // click on the third entry (WORDSTAR)
        let (px, py, pw, _, ih) = t.menu_rect();
        let entry = |i: usize| Some(((px + pw / 2) as f32, (py + 2 * 2 + ih * (i + 1) + ih / 2) as f32));
        assert_eq!(t.mouse(entry(2), true), Some(Action::Arrows(ArrowMode::WordStar)));
        assert!(!t.menu_open());
        assert_eq!(t.mouse(entry(2), false), None, "release after selecting does nothing");
        // open again, click outside: closes without a choice
        t.mouse(on_button, true);
        t.mouse(on_button, false);
        assert!(t.menu_open());
        assert_eq!(t.mouse(Some((1.0, 1.0)), true), None);
        assert!(!t.menu_open());
        t.mouse(Some((1.0, 1.0)), false);
        // popup drawing stays in the buffer and marks the current mode
        t.mouse(on_button, true);
        t.mouse(on_button, false);
        let mut buf = vec![0u32; 1024 * (100 + Toolbar::height(2))];
        t.draw_popup(&mut buf, &State { arrows: ArrowMode::Vt52, ..State::default() }, 0x12AB34);
        assert!(buf.iter().any(|&p| p == 0x12AB34));
    }

    #[test]
    fn arrow_modes() {
        assert_eq!(ArrowMode::default(), ArrowMode::Numpad);
        assert_eq!(ArrowMode::Numpad.keys(), [&b"8"[..], b"2", b"6", b"4"]);
        assert_eq!(ArrowMode::Vt52.keys()[3], b"\x1bD");
        assert_eq!(ArrowMode::None.next(), ArrowMode::Numpad);
        assert_eq!(ArrowMode::parse("WordStar"), Some(ArrowMode::WordStar));
        assert_eq!(ArrowMode::parse("off"), Some(ArrowMode::None));
        assert_eq!(ArrowMode::parse("vim"), Some(ArrowMode::Vim));
        assert_eq!(ArrowMode::Vim.keys(), [&b"k"[..], b"j", b"l", b"h"]);
        assert_eq!(ArrowMode::WordStar.next(), ArrowMode::Vim);
        assert_eq!(ArrowMode::parse("x"), None);
        assert!(ArrowMode::ALL.iter().all(|m| m.label().len() <= 4));
    }

    #[test]
    fn click_is_press_and_release_on_the_same_button() {
        let mut t = bar(1024, 2);
        let stop = t.buttons.iter().find(|b| b.action == Action::Stop).unwrap();
        let (x, y, w, h) = t.key_rect(stop);
        let at = Some(((x + w / 2) as f32, (y + h / 2) as f32));
        assert_eq!(t.mouse(at, true), None);
        assert_eq!(t.mouse(at, true), None);
        assert_eq!(t.mouse(at, false), Some(Action::Stop));
        // press on STOP, release elsewhere: nothing
        t.mouse(at, true);
        assert_eq!(t.mouse(Some((1.0, 1.0)), false), None);
        // BREAK is held while pressed
        let brk = t.buttons.iter().find(|b| b.action == Action::Break).unwrap();
        let (x, y, ..) = t.key_rect(brk);
        let at = Some(((x + 2) as f32, (y + 2) as f32));
        t.mouse(at, true);
        assert!(t.break_held());
        t.mouse(at, false);
        assert!(!t.break_held());
    }

    #[test]
    fn draw_stays_in_buffer() {
        let t = bar(512, 1);
        let mut buf = vec![0u32; 512 * (100 + Toolbar::height(1))];
        let st = State { stop: true, link: true, ..State::default() };
        t.draw(&mut buf, &st, 0x33FF66);
        assert!(buf[512 * 100..].iter().any(|&p| p == 0x33FF66));
    }
}
