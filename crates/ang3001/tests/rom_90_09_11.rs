//! ROM ANG-3001 90.09.11 (`roms/ang3001/ANG-3001_90.09.11.BIN`): boot, text,
//! LOCAL / BREAK handling, which differ from the default ROM.

use ang3001::*;

const ROM: &[u8] = include_bytes!("../../../roms/ang3001/ANG-3001_90.09.11.BIN");
const SEC: u64 = 6_000_000; // default CPU clock

fn booted() -> Ang3001 {
    let mut m = Ang3001::new(ROM, Config::default());
    assert_eq!(m.profile(), Some(PROFILE_90_09_11));
    m.run_cycles(2 * SEC);
    // With DSR reported active the ROM would sit in its DSR wait loop (0x0711).
    assert!(m.booted(), "ROM did not reach its main loop, pc={:04X}", m.pc());
    m
}

#[test]
fn profiles_are_detected() {
    assert_eq!(RomProfile::detect(DEFAULT_ROM), Some(PROFILE_ZJ_PG));
    assert_eq!(RomProfile::detect(ROM), Some(PROFILE_90_09_11));
    assert_eq!(RomProfile::detect(&[0u8; 0x4000]), None);
}

#[test]
fn banner_and_received_text() {
    let mut m = booted();
    assert!(m.text_screen()[0].contains("ANG-3000   V.01"));
    m.inject_rx(b"Linia 1\r\nLinia 2\r\n");
    m.run_cycles(SEC / 2);
    let s = m.text_screen();
    assert_eq!(&s[1..3], ["Linia 1", "Linia 2"], "{s:#?}");
}

#[test]
fn local_on_and_off() {
    let mut m = booted();
    assert_eq!(m.mode(), TermMode::Normal);
    assert!(m.set_local(true));
    m.run_cycles(SEC);
    assert!(m.is_local(), "mode {:?}, pc={:04X}", m.mode(), m.pc());
    assert_eq!(m.breaks_sent(), 0, "entering LOCAL must not send BREAK");
    assert!(m.set_local(false));
    m.run_cycles(SEC);
    assert_eq!(m.mode(), TermMode::Normal);
    // Back in DUPLEX, port 0x8D bit 7 = 1 again: BREAK goes out on the line.
    m.set_break(true);
    m.run_cycles(SEC / 10);
    m.set_break(false);
    m.run_cycles(SEC);
    assert_eq!(m.breaks_sent(), 1);
    assert_eq!(m.mode(), TermMode::Normal);
}
