//! Operating modes: STOP (paging) latch, LOCAL on/off key sequence, mode tracking.

use ang3001::*;
use b8_link::Endpoint;

const SEC: u64 = 6_000_000;

fn booted_with_link() -> (Ang3001, Box<dyn Endpoint>) {
    let mut m = Ang3001::with_default_rom();
    let (a, b) = b8_link::pair();
    m.attach_link(a);
    m.run_cycles(2 * SEC);
    assert!(m.booted());
    assert_eq!(m.mode(), TermMode::Normal);
    (m, b)
}

fn received(host: &mut Box<dyn Endpoint>) -> Vec<u8> {
    std::iter::from_fn(|| host.read()).collect()
}

fn lines(range: std::ops::Range<u32>) -> Vec<u8> {
    range.flat_map(|i| format!("line {i}\r\n").into_bytes()).collect()
}

/// Run until `cond` holds (max `secs` of emulated time).
fn run_until(m: &mut Ang3001, secs: u64, cond: impl Fn(&Ang3001) -> bool) -> bool {
    for _ in 0..secs * 50 {
        if cond(m) {
            return true;
        }
        m.run_cycles(SEC / 50);
    }
    cond(m)
}

#[test]
fn stop_pages_a_screen_at_a_time() {
    let (mut m, mut host) = booted_with_link();
    m.set_stop(true);
    m.run_cycles(SEC / 10); // the ROM samples the latch at 25 Hz
    for b in lines(0..80) {
        host.write(b);
    }
    assert!(run_until(&mut m, 10, |m| m.is_stopped()), "no STOP");
    m.run_cycles(SEC / 5);
    let s = m.text_screen();
    // "STOP" at column 13, then the scroll speed choice "speed 1 2"
    assert_eq!(s[24].split_whitespace().collect::<Vec<_>>(), ["STOP", "speed", "1", "2"]);
    // the first stop comes at the first scroll: the screen was full
    // (banner + 23 lines), so the terminal still waits with data pending
    assert_eq!(s[23], "line 22", "{s:?}");
    m.run_cycles(SEC);
    assert!(m.is_stopped());
    assert!(m.rx_backlog() > 0);

    // a key shows the next page (24 lines) and is not sent to the line
    m.key_press(b' ');
    assert!(run_until(&mut m, 2, |m| !m.is_stopped()));
    assert!(run_until(&mut m, 10, |m| m.is_stopped()), "no second STOP");
    m.run_cycles(SEC / 5);
    let s = m.text_screen();
    assert_eq!(s[23], "line 46", "{s:?}");
    assert!(s[24].contains("STOP"));
    assert!(received(&mut host).is_empty(), "the page key must not be sent");

    // releasing STOP resumes without paging (smooth scroll: ~0.2 s per line)
    m.set_stop(false);
    assert!(run_until(&mut m, 30, |m| m.text_screen()[22] == "line 79"));
    m.run_cycles(SEC / 2);
    let s = m.text_screen();
    assert!(!m.is_stopped());
    assert!(!s[24].contains("STOP"), "band not restored: {:?}", s[24]);
    assert_eq!(m.mode(), TermMode::Normal);
}

#[test]
fn no_paging_without_stop() {
    let (mut m, mut host) = booted_with_link();
    for b in lines(0..80) {
        host.write(b);
    }
    assert!(run_until(&mut m, 30, |m| m.text_screen()[22] == "line 79"), "{:?}", m.text_screen());
    assert_eq!(m.mode(), TermMode::Normal);
}

#[test]
fn local_on_and_off() {
    let (mut m, mut host) = booted_with_link();
    assert!(!m.is_local());
    assert!(m.set_local(true));
    assert!(m.local_switching());
    assert!(!m.set_local(true), "second request while switching is ignored");
    assert!(run_until(&mut m, 3, |m| !m.local_switching()));
    assert!(m.is_local());
    m.run_cycles(SEC / 5);
    assert_eq!(m.text_screen()[24].trim(), "LOCAL");
    // LOCAL: keys go to the screen, nothing to the line, line input ignored
    for &k in b"hi" {
        m.key_press(k);
    }
    m.run_cycles(SEC / 2);
    assert_eq!(m.text_screen()[1], "hi");
    assert!(received(&mut host).is_empty());
    assert_eq!(m.breaks_sent(), 0, "entering LOCAL must not send a BREAK");

    // off: a single BREAK press returns to the main loop (DUPLEX)
    assert!(m.set_local(false));
    assert!(run_until(&mut m, 3, |m| !m.local_switching()));
    assert_eq!(m.mode(), TermMode::Normal);
    m.run_cycles(SEC / 5);
    assert_eq!(m.text_screen()[24].trim(), "");
    assert_eq!(m.breaks_sent(), 0, "leaving LOCAL must not send a BREAK");
    for &k in b"ok" {
        m.key_press(k);
    }
    m.run_cycles(SEC / 2);
    assert_eq!(received(&mut host), b"ok");
    assert!(!m.set_local(false), "already off");
}

#[test]
fn local_from_manual_break_prompt() {
    let (mut m, _host) = booted_with_link();
    m.set_break(true);
    m.run_cycles(SEC / 10);
    m.set_break(false);
    assert!(run_until(&mut m, 2, |m| m.mode() == TermMode::BreakPrompt));
    assert!(m.set_local(true));
    assert!(run_until(&mut m, 2, |m| m.is_local()));
    assert_eq!(m.breaks_sent(), 0);
}

#[test]
fn reset_leaves_local() {
    let (mut m, _host) = booted_with_link();
    m.set_local(true);
    assert!(run_until(&mut m, 3, |m| m.is_local()));
    m.reset();
    assert_eq!(m.mode(), TermMode::Normal);
    assert!(!m.local_switching());
    m.run_cycles(2 * SEC);
    assert!(m.booted());
    assert_eq!(m.mode(), TermMode::Normal);
}
