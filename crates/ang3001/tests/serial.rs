//! Serial line: text and escape sequences in, keyboard out, code upload.

use ang3001::*;
use b8_link::Endpoint;

const SEC: u64 = 6_000_000;

fn booted_with_link() -> (Ang3001, Box<dyn Endpoint>) {
    let mut m = Ang3001::new(ROM_ZJ_PG, Config::default());
    let (a, b) = b8_link::pair();
    m.attach_link(a);
    m.run_cycles(2 * SEC);
    assert!(m.booted());
    (m, b)
}

fn send(m: &mut Ang3001, host: &mut Box<dyn Endpoint>, bytes: &[u8]) {
    for &c in bytes {
        host.write(c);
    }
    // run until everything was consumed, plus some time to draw
    for _ in 0..500 {
        m.run_cycles(SEC / 50);
        if m.rx_backlog() == 0 {
            break;
        }
    }
    m.run_cycles(SEC / 5);
}

fn received(host: &mut Box<dyn Endpoint>) -> Vec<u8> {
    std::iter::from_fn(|| host.read()).collect()
}

#[test]
fn plain_text_and_crlf() {
    let (mut m, mut host) = booted_with_link();
    send(&mut m, &mut host, b"Hello, world!\r\nSecond line");
    let s = m.text_screen();
    assert_eq!(s[1], "Hello, world!");
    assert_eq!(s[2], "Second line");
}

#[test]
fn cursor_addressing_row_then_column() {
    // ESC = <row+32> <col+32> (and the alias ESC Y): the first parameter is
    // the ROW (the ROM keeps it in 0x7F07 -> IXL), the second the column.
    let (mut m, mut host) = booted_with_link();
    send(&mut m, &mut host, b"\x1b=\x25\x2aA\x1bY\x2a\x48B\x1b=\x30\x21C");
    let s = m.text_screen();
    assert_eq!(s[5], format!("{:10}A", ""));
    assert_eq!(s[10], format!("{:40}B", ""));
    assert_eq!(s[16], format!(" C"));
}

#[test]
fn clear_screen_home_and_erase() {
    let (mut m, mut host) = booted_with_link();
    send(&mut m, &mut host, b"line1\r\nline2\r\nline3");
    // ESC H home, ESC J erase to end of screen
    send(&mut m, &mut host, b"\x1bH\x1bJtop");
    let s = m.text_screen();
    assert_eq!(s[0], "top");
    assert!(s[1..].iter().all(|l| l.is_empty()), "{s:?}");
    // ESC K erase to end of line
    send(&mut m, &mut host, b"\x1b=\x22\x20abcdef\x1b=\x22\x23\x1bK");
    assert_eq!(m.text_screen()[2], "abc");
}

#[test]
fn polish_letters_from_font() {
    let (mut m, mut host) = booted_with_link();
    send(&mut m, &mut host, b"\x1bH\x1bJ\x06\x01\x03\x04\x05\x02\x0b\x0c");
    assert_eq!(m.text_screen()[0], "ąęłńśźżć");
}

#[test]
fn scrolling_uses_scroll_register() {
    let (mut m, mut host) = booted_with_link();
    let mut text = Vec::new();
    for i in 0..30 {
        text.extend(format!("line {i}\r\n").bytes());
    }
    send(&mut m, &mut host, &text);
    // working rows are 0..23; banner + 30 lines + cursor on an empty line
    // need 32 rows -> scrolled by 8 rows of 10 lines
    assert_eq!(m.scroll(), 80);
    let s = m.text_screen();
    assert_eq!(s[0], "line 7");
    assert_eq!(s[22], "line 29");
    assert_eq!(s[23], "");
}

#[test]
fn keyboard_goes_to_the_line() {
    let (mut m, mut host) = booted_with_link();
    for &k in b"dir\r" {
        m.key_press(k);
    }
    m.run_cycles(SEC / 2);
    assert_eq!(received(&mut host), b"dir\r");
    // DUPLEX: nothing is echoed locally
    assert!(m.text_screen()[1].is_empty());
}

#[test]
fn caps_swaps_letter_case() {
    let (mut m, mut host) = booted_with_link();
    m.set_caps(true);
    for &k in b"aB1" {
        m.key_press(k);
    }
    m.run_cycles(SEC / 2);
    assert_eq!(received(&mut host), b"Ab1");
}

#[test]
fn break_menu_and_local_mode() {
    let (mut m, mut host) = booted_with_link();
    m.set_break(true);
    m.run_cycles(SEC / 10);
    m.set_break(false);
    m.run_cycles(SEC / 2);
    // status band = memory lines scroll+0..9 = text row 24 in ROM terms
    assert_eq!(m.text_screen()[24].trim(), "BREAK - powtórz   LOCAL - ESC");
    // ESC selects LOCAL mode: keys are echoed on the screen, not sent
    m.key_press(0x1B);
    m.run_cycles(SEC / 2);
    assert_eq!(m.text_screen()[24].trim(), "LOCAL");
    for &k in b"hi" {
        m.key_press(k);
    }
    m.run_cycles(SEC / 2);
    assert_eq!(m.text_screen()[1], "hi");
    assert!(received(&mut host).is_empty());
}

#[test]
fn break_twice_sends_break_condition() {
    let (mut m, _host) = booted_with_link();
    for _ in 0..2 {
        m.set_break(true);
        m.run_cycles(SEC / 10);
        m.set_break(false);
        m.run_cycles(SEC / 2);
    }
    m.run_cycles(SEC);
    assert_eq!(m.breaks_sent(), 1);
}

/// Terminal-side program (org 0x4000) uploaded with ESC i and run with ESC 0.
#[test]
fn upload_and_run_code() {
    let (mut m, mut host) = booted_with_link();
    #[rustfmt::skip]
    let code = [
        0x21, 0x00, 0x80,       // ld hl,8000h
        0x11, 0x01, 0x80,       // ld de,8001h
        0x01, 0x3F, 0x00,       // ld bc,63
        0x36, 0xAA,             // ld (hl),0AAh
        0xED, 0xB0,             // ldir
        0x3E, 0x5A,             // ld a,5Ah
        0x32, 0x00, 0x50,       // ld (5000h),a
        0x18, 0xFE,             // jr $
    ];
    send(&mut m, &mut host, &ter_upload_stream(&code));
    assert_eq!(m.ram()[0x5000], 0x5A);
    assert!(m.ram()[0x8000..0x8040].iter().all(|&b| b == 0xAA));
    assert!((0x4000..0x4020).contains(&m.pc()));
    // memory line 0 is the top screen line with the default offset
    assert_eq!(m.line_offset(), 0);
    assert!((0..512).all(|x| m.pixel(x, 0) == (x % 2 == 0)));
}

#[test]
fn uploaded_code_can_beep() {
    // like BEEP.TER: call BEEP (0353h) in a loop
    let (mut m, mut host) = booted_with_link();
    let code = [0xCD, 0x53, 0x03, 0x18, 0xFB];
    send(&mut m, &mut host, &ter_upload_stream(&code));
    let mut active = 0;
    for _ in 0..50 {
        m.run_cycles(SEC / 50);
        active += m.beep().active as u32;
    }
    assert!(active > 40, "beep active in {active}/50 frames");
    let f = m.beep().freq_hz;
    assert!((f - 1_300_000.0 / 3823.0).abs() < 1.0, "{f}");
}

#[test]
fn frame_clock_toggles_at_frame_rate() {
    // count changes of port 0x8E bit 2 during one emulated second
    let (mut m, mut host) = booted_with_link();
    #[rustfmt::skip]
    let code = [
        0x21, 0x00, 0x00,       // ld hl,0       ; toggles
        0xDB, 0x8E,             // in a,(8Eh)
        0xE6, 0x04,             // and 4
        0x47,                   // ld b,a
        // loop:
        0xDB, 0x8E,             // in a,(8Eh)
        0xE6, 0x04,             // and 4
        0xB8,                   // cp b
        0x28, 0xF9,             // jr z,loop
        0x47,                   // ld b,a
        0x23,                   // inc hl
        0x22, 0x00, 0x50,       // ld (5000h),hl
        0x18, 0xF2,             // jr loop
    ];
    send(&mut m, &mut host, &ter_upload_stream(&code));
    let start = u16::from_le_bytes([m.ram()[0x5000], m.ram()[0x5001]]);
    m.run_cycles(SEC);
    let end = u16::from_le_bytes([m.ram()[0x5000], m.ram()[0x5001]]);
    assert!((49..=51).contains(&(end - start)), "{}", end - start);
}

/// ESC ( <r1> <c1> <r2> <c2> opens a window (saving what is under it into
/// the RAM below the ROM), ESC ) closes it and restores the background. The
/// restore reads that RAM through OUT (0Bh) = ROM off ... OUT (0Ah) = ROM on.
#[test]
fn window_push_pop_uses_ram_under_rom() {
    let (mut m, mut host) = booted_with_link();
    let mut s = b"\x1bH\x1bJ".to_vec();
    for i in 0..12 {
        s.extend(format!("background line {i:02} xxxxxxxxxxxxxxxxxxxx\r\n").bytes());
    }
    send(&mut m, &mut host, &s);
    let before = m.text_screen();
    assert!(m.ram()[..0x4000].iter().all(|&b| b == 0));
    send(&mut m, &mut host, b"\x1b(\x25\x25\x2a\x40\x1bH\x1bJWINDOW");
    let win = m.text_screen();
    assert_eq!(win[5], format!("backgWINDOW{:22}xxxxxx", ""));
    assert_eq!(win[6], format!("backg{:28}xxxxxx", ""));
    assert!(m.ram()[..0x4000].iter().any(|&b| b != 0), "nothing saved under the ROM");
    assert_eq!(m.rom_switch_count(), 0);
    send(&mut m, &mut host, b"\x1b)");
    assert!(m.rom_switch_count() > 0);
    assert!(m.rom_enabled());
    assert_eq!(m.text_screen(), before);
}

/// Smoke test with a real demo from the ROM analysis repository (skipped
/// when that repository is not checked out next to this one).
#[test]
fn demo_fire_animates_if_available() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../bosman-8-ang-3000-rom/demo/fire/FIRE.TER");
    let Ok(code) = std::fs::read(path) else {
        eprintln!("skipped: {path} not found");
        return;
    };
    let (mut m, mut host) = booted_with_link();
    send(&mut m, &mut host, &ter_upload_stream(&code));
    m.run_cycles(SEC);
    let a = m.ram()[0x8000..0xC000].to_vec();
    m.run_cycles(SEC / 10);
    let b = m.ram()[0x8000..0xC000].to_vec();
    assert!((0x4000..0x4100).contains(&m.pc()), "pc={:04X}", m.pc());
    assert_ne!(a, b, "fire does not animate");
    assert!(b.iter().filter(|&&x| x != 0).count() > 50);
}

/// More received data than the terminal has taken in: RTS goes off (the
/// computer's CTS) until the backlog is drawn, then back on.
#[test]
fn rts_off_while_backlog() {
    let (mut m, mut host) = booted_with_link();
    assert!(host.cts());
    let text: Vec<u8> = (0..2000).map(|i| if i % 70 == 69 { b'\n' } else { b'A' + (i % 26) as u8 }).collect();
    for &b in &text {
        host.write(b);
    }
    let mut seen_off = false;
    for _ in 0..400 {
        m.run_cycles(SEC / 100);
        seen_off |= !host.cts();
        if seen_off && host.cts() && m.rx_backlog() == 0 {
            break;
        }
    }
    assert!(seen_off, "RTS never went off");
    assert!(host.cts(), "RTS did not come back, backlog {}", m.rx_backlog());
}
