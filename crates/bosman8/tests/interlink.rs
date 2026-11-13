//! Two Bosman-8 machines linked SIO-B to SIO-B in one process: the server
//! runs the CCP command `DEBUG` (slave mode), the client sees the server's
//! drives as D:/E:/F:.

use b8_link::{Endpoint, LineParams, LineStatus};
use std::sync::{Arc, Mutex};

/// Endpoint wrapper recording the line parameters the machine programs
/// (what a host serial port would be switched to).
struct RecordLine {
    inner: Box<dyn Endpoint>,
    log: Arc<Mutex<Vec<LineParams>>>,
}

impl Endpoint for RecordLine {
    fn write(&mut self, b: u8) {
        self.inner.write(b)
    }
    fn read(&mut self) -> Option<u8> {
        self.inner.read()
    }
    fn set_line_params(&mut self, p: LineParams) {
        self.log.lock().unwrap().push(p);
    }
    fn status(&self) -> LineStatus {
        self.inner.status()
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
}
use bosman8::{Bosman8, Config, CPU_HZ};

fn rom() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/bosman8.bin");
    std::fs::read(path).expect("roms/bosman8.bin")
}

fn tools_disk() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../disks/narzedzia.img");
    std::fs::read(path).expect("disks/narzedzia.img")
}

struct Node {
    m: Bosman8,
    host: Box<dyn Endpoint>,
    out: Vec<u8>,
}

impl Node {
    fn type_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.host.write(b);
        }
    }
    fn drain(&mut self) {
        while let Some(b) = self.host.read() {
            self.out.push(b);
        }
        let trace = std::env::var_os("TRACE_AUX").is_some();
        for l in self.m.take_log() {
            if trace {
                eprintln!("{l}");
            }
        }
    }
    fn text(&self) -> String {
        printable(&self.out)
    }
}

/// Printable text; `ESC X col` (status line cursor positioning) is dropped.
fn printable(v: &[u8]) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i < v.len() {
        if v[i] == 0x1B && v.get(i + 1) == Some(&b'X') {
            i += 3;
            continue;
        }
        s.push(if (0x20..0x7F).contains(&v[i]) { v[i] as char } else { '.' });
        i += 1;
    }
    s
}

fn pair() -> (Node, Node, Arc<Mutex<Vec<LineParams>>>) {
    let (aux_s, aux_c) = b8_link::pair();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let aux_c: Box<dyn Endpoint> = Box::new(RecordLine { inner: aux_c, log: lines.clone() });
    let trace = std::env::var_os("TRACE_AUX").is_some();
    // TRACE_AUX=1 prints the SIO-B traffic of both machines (--trace-aux).
    let cfg = Config { fast_fdc: true, trace_aux: trace, ..Config::default() };
    let (t_s, h_s) = b8_link::pair();
    let (t_c, h_c) = b8_link::pair();
    let mut s = Bosman8::new(&rom(), cfg.clone(), t_s, aux_s);
    let fmt = bosman8::disk::format_for(cfg.dip, 0);
    s.insert_disk(0, Some(fmt.from_bytes(tools_disk())));
    let c = Bosman8::new(&rom(), cfg, t_c, aux_c);
    (Node { m: s, host: h_s, out: Vec::new() }, Node { m: c, host: h_c, out: Vec::new() }, lines)
}

/// Emulated time each machine runs before the other gets its turn. 20 ms is
/// the time slice of the `bosman8` binary in real-time mode: bytes sent
/// during one slice reach the peer together (as over TCP), which is what
/// broke the link before the SIO-B receiver hold-off (see lib.rs).
const SLICE: u64 = CPU_HZ as u64 / 50;

/// Run both machines alternately until `needle` appears in the terminal
/// output of `who` (0 = server, 1 = client) or `max_s` emulated seconds pass.
fn run_until(nodes: &mut [Node; 2], who: usize, needle: &str, max_s: f64) -> bool {
    let limit = (max_s * CPU_HZ as f64) as u64;
    let start = nodes[who].m.cycles();
    while nodes[who].m.cycles() - start < limit {
        for n in nodes.iter_mut() {
            n.m.run_cycles(SLICE);
            n.drain();
        }
        if nodes[who].text().contains(needle) {
            return true;
        }
    }
    false
}

/// Type a command on the client and wait for the next prompt.
fn command(n: &mut [Node; 2], cmd: &str, max_s: f64) -> String {
    n[1].out.clear();
    n[1].type_str(cmd);
    n[1].type_str("\r");
    // Echo first, then the prompt after it.
    assert!(run_until(n, 1, cmd, 2.0), "{cmd}: no echo: {}", n[1].text());
    let echo = n[1].text().find(cmd).unwrap() + cmd.len();
    let limit = (max_s * CPU_HZ as f64) as u64;
    let start = n[1].m.cycles();
    while n[1].m.cycles() - start < limit {
        if n[1].text()[echo..].contains("A>") {
            return n[1].text()[echo..].to_string();
        }
        run_until(n, 1, "\u{0}", 0.1);
    }
    panic!("{cmd}: no prompt: {}", n[1].text());
}

#[test]
fn remote_drives_over_sio_b() {
    let (s, c, lines) = pair();
    let mut n = [s, c];
    let bauds = || lines.lock().unwrap().iter().map(|p| p.baud).collect::<Vec<_>>();
    assert!(run_until(&mut n, 0, "A>", 10.0), "server: {}", n[0].text());
    assert!(run_until(&mut n, 1, "A>", 10.0), "client: {}", n[1].text());
    n[0].out.clear();
    n[0].type_str("DEBUG\r");
    assert!(run_until(&mut n, 0, "DEBUG", 1.0), "server: {}", n[0].text());
    run_until(&mut n, 0, "\u{0}", 0.2);

    // First access to D/E/F asks whether to connect.
    n[1].out.clear();
    n[1].type_str("DIR E:\r");
    assert!(run_until(&mut n, 1, "(R)", 5.0), "client: {}", n[1].text());
    n[1].type_str("D");
    let ok = run_until(&mut n, 1, "TRPLIK", 30.0);
    let text = n[1].text();
    assert!(ok, "DIR E: = server's B: (tools disk): {text}");
    for f in ["PIP", "STAT", "SUBMIT", "UNERASE"] {
        assert!(text.contains(f), "{f} missing: {text}");
    }
    assert!(!text.contains("Brak komunikacji"), "{text}");
    // The client's SIO-B started at 9615 bit/s (8253 counter 2 = 13) and
    // switched to 20833 bit/s (counter 2 = 6) for the transfer: a host serial
    // port behind the endpoint would have been reprogrammed accordingly.
    assert_eq!(bauds().first(), Some(&9615), "{:?}", bauds());
    assert!(bauds().contains(&20833), "{:?}", bauds());

    // D: = server's A: (empty RAM disk), F: = server's C: (not present).
    assert!(command(&mut n, "DIR D:", 10.0).contains("Brak pliku"));
    n[1].out.clear();
    n[1].type_str("DIR F:\r");
    assert!(run_until(&mut n, 1, "brak nap", 10.0), "DIR F: {}", n[1].text());
    // The BDOS error message waits for a key, then warm boots.
    n[1].out.clear();
    n[1].type_str(" ");
    assert!(run_until(&mut n, 1, "A>", 10.0), "{}", n[1].text());

    // Read a file from the server and run it locally.
    // PIP itself is loaded from E: as well.
    let t = command(&mut n, "E:PIP A:=E:STAT.COM", 60.0);
    assert!(!t.contains("ERROR") && !t.contains("Brak"), "PIP read: {t}");
    let t = command(&mut n, "STAT", 20.0);
    assert!(t.contains("A: R/W"), "STAT on the client: {t}");

    // Write a file to the server's floppy.
    let t = command(&mut n, "E:PIP E:COPY.COM=A:STAT.COM", 60.0);
    assert!(!t.contains("ERROR"), "PIP write: {t}");
    let t = command(&mut n, "DIR E:COPY.COM", 20.0);
    assert!(t.contains("COPY"), "{t}");
    let disk = n[0].m.disk(0).unwrap();
    let dir = &disk.data[..2048];
    assert!(dir.windows(11).any(|w| w == b"COPY    COM"), "not on the server's disk");

    // Selecting a drive >= G: releases D/E/F: the client sends F0h and the
    // server leaves the slave mode (warm boot to its CCP).
    n[0].out.clear();
    n[1].out.clear();
    n[1].type_str("DIR G:\r");
    assert!(run_until(&mut n, 0, "A>", 10.0), "server did not return to CCP: {}", n[0].text());
    // After the release (F0h) the client's line is back at 9600.
    run_until(&mut n, 1, "\u{0}", 0.5);
    assert_eq!(bauds().last(), Some(&9615), "{:?}", bauds());
}
