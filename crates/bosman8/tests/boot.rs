//! Boot the real ROM in turbo mode and talk to the CCP over SIO-A.

use b8_link::Endpoint;
use bosman8::{Bosman8, Config, CPU_HZ};

fn rom() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/bosman8.bin");
    std::fs::read(path).expect("roms/bosman8.bin")
}

fn machine() -> (Bosman8, Box<dyn Endpoint>) {
    let (term, host) = b8_link::pair();
    let aux = b8_link::open("none").unwrap();
    let cfg = Config { fast_fdc: true, ..Config::default() };
    (Bosman8::new(&rom(), cfg, term, aux), host)
}

/// Run until `needle` appears in the terminal output (or `max_s` emulated seconds pass).
fn run_until(m: &mut Bosman8, host: &mut Box<dyn Endpoint>, out: &mut Vec<u8>, needle: &[u8], max_s: f64) -> bool {
    let start = m.cycles();
    let limit = (max_s * CPU_HZ as f64) as u64;
    while m.cycles() - start < limit {
        m.run_cycles(CPU_HZ as u64 / 100);
        while let Some(b) = host.read() {
            out.push(b);
        }
        if out.windows(needle.len()).any(|w| w == needle) {
            return true;
        }
    }
    false
}

fn printable(v: &[u8]) -> String {
    v.iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' }).collect()
}

#[test]
fn boots_to_prompt_and_runs_dir() {
    let (mut m, mut host) = machine();
    let mut out = Vec::new();
    assert!(
        run_until(&mut m, &mut host, &mut out, b"\rA>", 10.0),
        "no A> prompt; output: {}",
        printable(&out)
    );
    let text = printable(&out);
    assert!(text.contains("Mikrokomputer"), "banner missing: {text}");
    assert!(text.contains("RAMDYSK  408 KB"), "banner missing: {text}");
    assert!(!text.contains("uszkodzony"), "error reported: {text}");

    out.clear();
    for &b in b"DIR\r" {
        host.write(b);
    }
    // The CCP echoes the command and answers (empty RAM disk) with a new prompt.
    assert!(
        run_until(&mut m, &mut host, &mut out, b"\rA>", 5.0),
        "no response to DIR; output: {}",
        printable(&out)
    );
    let text = printable(&out);
    assert!(text.contains("DIR"), "no echo: {text}");
    eprintln!("DIR output: {text}");
}

fn type_line(host: &mut Box<dyn Endpoint>, s: &str) {
    for b in s.bytes() {
        host.write(b);
    }
}

#[test]
fn floppy_b_save_and_dir() {
    let (term, mut host) = b8_link::pair();
    let aux = b8_link::open("none").unwrap();
    let cfg = Config { fast_fdc: true, ..Config::default() };
    let mut m = Bosman8::new(&rom(), cfg.clone(), term, aux);
    let fmt = bosman8::disk::format_for(cfg.dip, 0);
    m.insert_disk(0, Some(fmt.blank()));
    let mut out = Vec::new();
    assert!(run_until(&mut m, &mut host, &mut out, b"\rA>", 10.0), "{}", printable(&out));

    for (cmd, expect) in [
        ("B:\r", "B>"),
        ("DIR\r", "B>"),
        ("SAVE 1 HELLO.COM\r", "B>"),
        ("DIR\r", "B>"),
    ] {
        out.clear();
        type_line(&mut host, cmd);
        let ok = run_until(&mut m, &mut host, &mut out, format!("\r{expect}").as_bytes(), 10.0);
        eprintln!("{cmd:?} -> {}", printable(&out));
        assert!(ok, "{cmd:?}: {}", printable(&out));
    }
    let text = printable(&out);
    assert!(text.contains("HELLO") && text.contains("COM"), "file not listed: {text}");
    // Directory entry in the first sector of track 0.
    let d = m.disk(0).unwrap();
    assert_eq!(&d.data[1..12], b"HELLO   COM");
}

#[test]
fn floppy_c_and_printer() {
    let (term, mut host) = b8_link::pair();
    let aux = b8_link::open("none").unwrap();
    // DIP bit 6 = 0: the ROM initialises drive C: as well.
    let cfg = Config { fast_fdc: true, dip: 0x00, ..Config::default() };
    let mut m = Bosman8::new(&rom(), cfg.clone(), term, aux);
    m.insert_disk(1, Some(bosman8::disk::format_for(cfg.dip, 1).blank()));
    let mut out = Vec::new();
    assert!(run_until(&mut m, &mut host, &mut out, b"\rA>", 10.0), "{}", printable(&out));
    for (cmd, expect) in [("C:\r", "C>"), ("SAVE 2 TEST.COM\r", "C>"), ("\x10DIR\r", "C>")] {
        out.clear();
        type_line(&mut host, cmd);
        let ok = run_until(&mut m, &mut host, &mut out, format!("\r{expect}").as_bytes(), 10.0);
        assert!(ok, "{cmd:?}: {}", printable(&out));
    }
    assert_eq!(&m.disk(1).unwrap().data[1..12], b"TEST    COM");
    // ^P echoes the console to the printer.
    let printed = printable(&m.take_printer());
    assert!(printed.contains("TEST"), "printer: {printed}");
}
