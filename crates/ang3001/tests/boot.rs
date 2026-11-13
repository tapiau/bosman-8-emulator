//! Boot sequence: banner, sound, timing.

use ang3001::*;

const SEC: u64 = 6_000_000; // default CPU clock

fn booted() -> Ang3001 {
    let mut m = Ang3001::with_default_rom();
    m.run_cycles(2 * SEC);
    assert!(m.booted(), "ROM did not reach MAIN_LOOP, pc={:04X}", m.pc());
    m
}

#[test]
fn banner_text() {
    let m = booted();
    let screen = m.text_screen();
    // ESC S " ANG-3000 " ESC T "  V." + hex 01 at column 30, then at
    // column 50 the port 0x8D value (F0) and ROM bytes 0003..0008.
    assert_eq!(screen[0], format!("{:30} ANG-3000   V.01    F0CD0D00871127", ""));
    assert!(screen[1..].iter().all(|l| l.is_empty()));
    assert_eq!(m.scroll(), 0);
}

#[test]
fn banner_is_inverse_video_and_matches_font() {
    let m = booted();
    // "ANG-3000" occupies columns 31..38 in inverse video (ESC S ... ESC T).
    for (i, ch) in "ANG-3000".bytes().enumerate() {
        let cell = m.text_cell(0, 31 + i);
        let inv = m.glyph(ch).map(|b| !b & 0xFC);
        assert_eq!(cell, inv, "column {}", 31 + i);
    }
    // "V.01" is normal video at columns 42..45.
    for (i, ch) in "V.01".bytes().enumerate() {
        assert_eq!(m.text_cell(0, 42 + i), m.glyph(ch));
    }
}

#[test]
fn render_matches_font_pixels() {
    let mut m = booted();
    m.set_colors(0x00FF00, 0x000000);
    let mut img = vec![0u32; SCREEN_W * SCREEN_H];
    m.render(&mut img);
    // 'V' of "V.01": text row 0, column 42 -> x = 16 + 6*42, memory line 16
    // (+ scroll 0), shown at screen line 16 - line_offset.
    let g = m.glyph(b'V');
    let x0 = TEXT_X0 + 6 * 42;
    let y0 = TEXT_LINE0 - m.line_offset() as usize;
    for (dy, row) in g.iter().enumerate() {
        for dx in 0..6 {
            let lit = row & (0x80 >> dx) != 0;
            let px = img[(y0 + dy) * SCREEN_W + x0 + dx];
            assert_eq!(px, if lit { 0x00FF00 } else { 0 }, "dx={dx} dy={dy}");
        }
    }
    // PGM screenshot has the same content.
    let pgm = m.screenshot_pgm();
    let hdr = b"P5\n512 256\n255\n";
    assert_eq!(&pgm[..hdr.len()], hdr);
    let gray = &pgm[hdr.len()..];
    assert_eq!(gray.len(), SCREEN_W * SCREEN_H);
    for (i, &p) in img.iter().enumerate() {
        assert_eq!(gray[i] == 255, p == 0x00FF00);
    }
}

#[test]
fn line_offset_moves_picture() {
    let mut m = booted();
    m.set_line_offset(16);
    // text row 0 now starts on the first screen line
    let g = m.glyph(b'V');
    let x0 = TEXT_X0 + 6 * 42;
    for (dy, row) in g.iter().enumerate() {
        for dx in 0..6 {
            assert_eq!(m.pixel(x0 + dx, dy), row & (0x80 >> dx) != 0);
        }
    }
}

#[test]
fn boot_beeps_and_beep_ends() {
    let mut m = Ang3001::with_default_rom();
    let mut seen = false;
    for _ in 0..100 {
        m.run_cycles(SEC / 50);
        let b = m.beep();
        if b.active {
            seen = true;
            // default pitch: 1.3 MHz / 0x0EEF
            assert!(b.freq_hz > 100.0 && b.freq_hz < 2000.0, "{}", b.freq_hz);
        }
    }
    assert!(seen, "no beep during boot");
    m.run_cycles(2 * SEC);
    assert!(!m.beep().active, "beep must stop (counter 0 reaches 0)");
}

#[test]
fn reset_reboots() {
    let mut m = booted();
    m.ram_mut()[0x8000..0xC000].fill(0x55);
    m.reset();
    assert!(!m.booted());
    m.run_cycles(2 * SEC);
    assert!(m.booted());
    assert!(m.text_screen()[0].contains("ANG-3000"));
}

#[test]
fn serial_timing_follows_pit() {
    let m = booted();
    // 8253 counter 2 = 13 -> 100 kHz; mode 0xCD = async x1, 8N2 -> 11 bits
    assert_eq!(m.pit().reload(2), Some(13));
    assert_eq!(m.usart().mode(), 0xCD);
}
