//! Button bar under the emulated screen, drawn into the minifb buffer with
//! the terminal ROM's own 6x10 font: function-key style buttons (RESET,
//! BREAK, LOCAL, STOP, CAPS, TURBO) and status LEDs (LINK, BEEP, WAIT).
//!
//! Mouse handling: a button is "pressed" on the press edge of the left
//! button over it and acts on the release edge if the pointer is still over
//! it (like a desktop button); BREAK is momentary and held while pressed.

/// What a button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Reset,
    Break,
    Local,
    Stop,
    Caps,
    Turbo,
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
            leds,
            pressed: None,
            mouse_was_down: false,
        }
    }

    fn key_rect(&self, b: &Button) -> (usize, usize, usize, usize) {
        let m = 3 * self.fs;
        (b.x, self.y0 + m, b.w, self.h - 2 * m)
    }

    fn hit(&self, pos: Option<(f32, f32)>) -> Option<Action> {
        let (mx, my) = pos?;
        if mx < 0.0 || my < 0.0 {
            return None;
        }
        let (mx, my) = (mx as usize, my as usize);
        self.buttons.iter().find_map(|b| {
            let (x, y, w, h) = self.key_rect(b);
            (mx >= x && mx < x + w && my >= y && my < y + h).then_some(b.action)
        })
    }

    /// Feed the mouse state (window coordinates, left button). Returns the
    /// action of a completed click.
    pub fn mouse(&mut self, pos: Option<(f32, f32)>, down: bool) -> Option<Action> {
        let mut click = None;
        if down && !self.mouse_was_down {
            self.pressed = self.hit(pos);
        } else if !down && self.mouse_was_down {
            if let Some(p) = self.pressed.take() {
                if self.hit(pos) == Some(p) {
                    click = Some(p);
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
                Action::Reset => false,
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
            assert!(last.x + last.w <= width);
            assert_eq!(t.leds.len(), 3, "LEDs fit at width {width}");
        }
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
