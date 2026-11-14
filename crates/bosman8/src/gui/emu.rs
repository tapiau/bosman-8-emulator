//! Emulator thread of the GUI: owns the [`Bosman8`], runs it in real time
//! (or turbo), takes [`Cmd`]s from the UI and publishes a [`Status`].

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use b8_chips::Wd1770Timing;
use b8_link::{Endpoint, LineStatus, NullEndpoint};
use bosman8::{disk, Bosman8, Config, CPU_HZ};

use super::config::{AppConfig, PrinterCfg, PrinterKind};

/// Commands from the UI.
pub enum Cmd {
    Run(bool),
    Turbo(bool),
    /// RESET button: CPU only, RAM and latches kept.
    WarmReset,
    /// Power cycle; applies the DIP value.
    ColdReset(u8),
    Nmi,
    Insert { drive: usize, path: PathBuf, write_protect: bool },
    Eject(usize),
    WriteProtect(usize, bool),
    /// New spec for SIO-A (`false`) or SIO-B (`true`).
    Link { aux: bool, spec: String },
    /// A ready endpoint for SIO-A (the built-in terminal's end of a `b8_link::pair`).
    TermEndpoint(Box<dyn Endpoint>),
    /// Open (Some) or close (None) the printer output.
    Printer(Option<PrinterCfg>),
    Quit,
}

#[derive(Debug, Clone, Default)]
pub struct DriveStatus {
    pub path: Option<PathBuf>,
    pub inserted: bool,
    pub image_len: usize,
    pub write_protect: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LinkStatus {
    pub describe: String,
    pub connected: bool,
    pub error: Option<String>,
}

/// State published by the emulator thread.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub running: bool,
    pub turbo: bool,
    /// Emulated seconds since the last power cycle.
    pub uptime: f64,
    pub mhz: f64,
    pub pc: u16,
    pub latch: u8,
    pub latch2: u8,
    pub bank: usize,
    pub ram_at_0: bool,
    /// DIP value in effect (the GUI's value applies on the next cold reset).
    pub dip: u8,
    pub drives: [DriveStatus; 2],
    pub term: LinkStatus,
    pub aux: LinkStatus,
    pub printer: Option<String>,
    pub printer_error: Option<String>,
    pub printed: u64,
    pub messages: Vec<String>,
    /// SIO-A output (machine → terminal), last bytes.
    pub monitor: VecDeque<u8>,
}

const MONITOR_CAP: usize = 16 * 1024;

/// Endpoint wrapper that copies the transmitted bytes for the monitor pane.
struct Tap {
    inner: Box<dyn Endpoint>,
    buf: Arc<Mutex<Vec<u8>>>,
}

impl Endpoint for Tap {
    fn write(&mut self, byte: u8) {
        if let Ok(mut b) = self.buf.lock() {
            if b.len() < MONITOR_CAP {
                b.push(byte);
            }
        }
        self.inner.write(byte);
    }
    fn write_at(&mut self, byte: u8, t: f64) {
        if let Ok(mut b) = self.buf.lock() {
            if b.len() < MONITOR_CAP {
                b.push(byte);
            }
        }
        self.inner.write_at(byte, t);
    }
    fn read(&mut self) -> Option<u8> {
        self.inner.read()
    }
    fn send_break(&mut self) {
        self.inner.send_break()
    }
    fn set_line_params(&mut self, p: b8_link::LineParams) {
        self.inner.set_line_params(p)
    }
    fn set_line_params_at(&mut self, p: b8_link::LineParams, t: f64) {
        self.inner.set_line_params_at(p, t)
    }
    fn set_rts(&mut self, ready: bool) {
        self.inner.set_rts(ready)
    }
    fn cts(&self) -> bool {
        self.inner.cts()
    }
    fn status(&self) -> LineStatus {
        self.inner.status()
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
}

/// An endpoint used as a printer sink.
struct LinkWriter(Box<dyn Endpoint>);

impl Write for LinkWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        buf.iter().for_each(|&b| self.0.write(b));
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Handle of the emulator thread; dropping it stops the thread (disks are
/// written back first).
pub struct Emu {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    handle: Option<JoinHandle<()>>,
}

impl Emu {
    pub fn start(rom: Vec<u8>, cfg: &AppConfig) -> Emu {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let cfg = cfg.clone();
        let st = status.clone();
        let handle = std::thread::Builder::new()
            .name("bosman8-emu".into())
            .spawn(move || Worker::new(&rom, &cfg, st).run(rx))
            .expect("spawn emulator thread");
        Emu { tx, status, handle: Some(handle) }
    }

    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    pub fn status(&self) -> MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Emu {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct Drive {
    path: PathBuf,
    container: disk::Container,
    /// Contents as last written to the file.
    saved: Vec<u8>,
}

struct Worker {
    m: Bosman8,
    drives: [Option<Drive>; 2],
    printer: Option<Box<dyn Write + Send>>,
    printer_desc: Option<String>,
    printed: u64,
    running: bool,
    turbo: bool,
    reset_cycles: u64,
    tap: Arc<Mutex<Vec<u8>>>,
    monitor: VecDeque<u8>,
    messages: Vec<String>,
    term_err: Option<String>,
    aux_err: Option<String>,
    printer_err: Option<String>,
    status: Arc<Mutex<Status>>,
}

impl Worker {
    fn new(rom: &[u8], cfg: &AppConfig, status: Arc<Mutex<Status>>) -> Worker {
        let config = Config { dip: cfg.dip, ..Config::default() };
        let m = Bosman8::new(rom, config, Box::new(NullEndpoint), Box::new(NullEndpoint));
        let mut w = Worker {
            m,
            drives: [None, None],
            printer: None,
            printer_desc: None,
            printed: 0,
            running: true,
            turbo: false,
            reset_cycles: 0,
            tap: Arc::new(Mutex::new(Vec::new())),
            monitor: VecDeque::new(),
            messages: Vec::new(),
            term_err: None,
            aux_err: None,
            printer_err: None,
            status,
        };
        w.set_turbo(cfg.turbo);
        w.set_link(false, &cfg.term.spec());
        w.set_link(true, &cfg.aux.spec());
        for (d, dc) in cfg.drives.iter().enumerate() {
            if let Some(p) = &dc.path {
                w.insert(d, p.clone(), dc.write_protect);
            }
        }
        if cfg.printer.open && cfg.printer.kind != PrinterKind::None {
            w.open_printer(Some(cfg.printer.clone()));
        }
        w
    }

    fn msg(&mut self, s: String) {
        if self.messages.len() >= 100 {
            self.messages.remove(0);
        }
        self.messages.push(s);
    }

    fn set_turbo(&mut self, on: bool) {
        self.turbo = on;
        self.m.bus.fdc.set_timing(if on { Wd1770Timing::fast() } else { Wd1770Timing::default() });
    }

    fn set_link(&mut self, aux: bool, spec: &str) {
        // Close the old endpoint first so a listener can re-bind the same port.
        let old = if aux {
            self.m.set_aux_endpoint(Box::new(NullEndpoint))
        } else {
            self.m.set_term_endpoint(Box::new(NullEndpoint))
        };
        drop(old);
        if !aux && spec == "terminal" {
            // The built-in terminal: the GUI sends its endpoint (Cmd::TermEndpoint).
            self.term_err = None;
            return;
        }
        let err = match b8_link::open(spec) {
            Ok(ep) => {
                if aux {
                    self.m.set_aux_endpoint(ep);
                } else {
                    self.m.set_term_endpoint(Box::new(Tap { inner: ep, buf: self.tap.clone() }));
                }
                None
            }
            Err(e) => Some(e.to_string()),
        };
        let name = if aux { "SIO-B" } else { "SIO-A" };
        if let Some(e) = &err {
            self.msg(format!("{name}: {e}"));
        }
        if aux {
            self.aux_err = err;
        } else {
            self.term_err = err;
        }
    }

    fn flush(&mut self, d: usize) {
        let Some(drive) = self.drives[d].as_mut() else { return };
        let Some(img) = self.m.disk(d) else { return };
        if img.data != drive.saved {
            match std::fs::write(&drive.path, drive.container.to_bytes(img)) {
                Ok(()) => drive.saved = img.data.clone(),
                Err(e) => {
                    let s = format!("{}: {e}", drive.path.display());
                    // Don't retry every second with the same error.
                    drive.saved = img.data.clone();
                    self.msg(s);
                }
            }
        }
    }

    fn eject(&mut self, d: usize) {
        self.flush(d);
        self.m.eject_disk(d);
        self.drives[d] = None;
    }

    fn insert(&mut self, d: usize, path: PathBuf, write_protect: bool) {
        self.eject(d);
        let format = disk::format_for(self.m.dip(), d);
        let data = match std::fs::read(&path) {
            Ok(x) => x,
            Err(e) => {
                self.msg(format!("{}: {e}", path.display()));
                return;
            }
        };
        let o = match disk::open_bytes(data, format) {
            Ok(o) => o,
            Err(e) => {
                self.msg(format!("{}: {e}", path.display()));
                return;
            }
        };
        let mut img = o.image;
        if let Some(w) = &o.warning {
            if matches!(o.container, disk::Container::Raw) {
                // Writing back would pad/truncate the file: protect it.
                img.write_protected = true;
                self.msg(format!("{}: {w} (current DIP) — inserted write-protected", path.display()));
            } else {
                self.msg(format!("{}: {w}", path.display()));
            }
        }
        img.write_protected |= write_protect;
        let saved = img.data.clone();
        self.m.insert_disk(d, Some(img));
        self.drives[d] = Some(Drive { path, container: o.container, saved });
    }

    fn open_printer(&mut self, cfg: Option<PrinterCfg>) {
        if let Some(mut p) = self.printer.take() {
            let _ = p.flush();
        }
        self.printer_desc = None;
        self.printer_err = None;
        let Some(cfg) = cfg else { return };
        let r: Result<(Box<dyn Write + Send>, String), String> = match cfg.kind {
            PrinterKind::None => return,
            PrinterKind::File => match &cfg.file {
                None => Err("no file chosen".into()),
                Some(f) => std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(cfg.append)
                    .truncate(!cfg.append)
                    .open(f)
                    .map(|h| (Box::new(h) as Box<dyn Write + Send>, format!("file {}", f.display())))
                    .map_err(|e| format!("{}: {e}", f.display())),
            },
            PrinterKind::Device => std::fs::OpenOptions::new()
                .write(true)
                .open(&cfg.device)
                .map(|h| (Box::new(h) as Box<dyn Write + Send>, format!("device {}", cfg.device)))
                .map_err(|e| format!("{}: {e}", cfg.device)),
            PrinterKind::Link => b8_link::open(&cfg.link)
                .map(|ep| {
                    let d = ep.describe();
                    (Box::new(LinkWriter(ep)) as Box<dyn Write + Send>, d)
                })
                .map_err(|e| e.to_string()),
        };
        match r {
            Ok((w, d)) => {
                self.printer = Some(w);
                self.printer_desc = Some(d);
            }
            Err(e) => {
                self.msg(format!("printer: {e}"));
                self.printer_err = Some(e);
            }
        }
    }

    /// Returns false on Quit.
    fn handle(&mut self, c: Cmd) -> bool {
        match c {
            Cmd::Run(r) => self.running = r,
            Cmd::Turbo(t) => self.set_turbo(t),
            Cmd::WarmReset => self.m.warm_reset(),
            Cmd::ColdReset(dip) => {
                self.m.set_dip(dip);
                self.m.cold_reset();
                self.reset_cycles = self.m.cycles();
                self.running = true;
            }
            Cmd::Nmi => self.m.nmi(),
            Cmd::Insert { drive, path, write_protect } => self.insert(drive, path, write_protect),
            Cmd::Eject(d) => self.eject(d),
            Cmd::WriteProtect(d, on) => self.m.set_write_protect(d, on),
            Cmd::Link { aux, spec } => self.set_link(aux, &spec),
            Cmd::TermEndpoint(ep) => {
                drop(self.m.set_term_endpoint(Box::new(NullEndpoint)));
                self.m.set_term_endpoint(Box::new(Tap { inner: ep, buf: self.tap.clone() }));
                self.term_err = None;
            }
            Cmd::Printer(p) => self.open_printer(p),
            Cmd::Quit => return false,
        }
        true
    }

    fn drain(&mut self) {
        let _ = self.m.take_log();
        let out = self.m.take_printer();
        if !out.is_empty() {
            self.printed += out.len() as u64;
            if let Some(p) = self.printer.as_mut() {
                if let Err(e) = p.write_all(&out).and_then(|_| p.flush()) {
                    self.printer = None;
                    self.printer_desc = None;
                    self.printer_err = Some(e.to_string());
                    self.msg(format!("printer: {e} (closed)"));
                }
            }
        }
        let bytes = std::mem::take(&mut *self.tap.lock().unwrap_or_else(|e| e.into_inner()));
        self.monitor.extend(bytes);
        while self.monitor.len() > MONITOR_CAP {
            self.monitor.pop_front();
        }
    }

    fn publish(&mut self, mhz: f64) {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        let m = &self.m;
        s.running = self.running;
        s.turbo = self.turbo;
        s.uptime = (m.cycles() - self.reset_cycles) as f64 / CPU_HZ as f64;
        s.mhz = mhz;
        s.pc = m.pc();
        s.latch = m.bus.latch.value;
        s.latch2 = m.bus.latch2.value;
        (s.bank, s.ram_at_0) = m.bank();
        s.dip = m.dip();
        for d in 0..2 {
            let img = m.disk(d);
            s.drives[d] = DriveStatus {
                path: self.drives[d].as_ref().map(|x| x.path.clone()),
                inserted: img.is_some(),
                image_len: img.map_or(0, |i| i.data.len()),
                write_protect: img.is_some_and(|i| i.write_protected),
                active: m.drive_active(d),
            };
        }
        s.term = LinkStatus { describe: m.term_describe(), connected: m.term_status().connected, error: self.term_err.clone() };
        s.aux = LinkStatus { describe: m.aux_describe(), connected: m.aux_status().connected, error: self.aux_err.clone() };
        s.printer = self.printer_desc.clone();
        s.printer_error = self.printer_err.clone();
        s.printed = self.printed;
        s.messages.append(&mut self.messages);
        if s.messages.len() > 100 {
            let n = s.messages.len() - 100;
            s.messages.drain(..n);
        }
        s.monitor.clone_from(&self.monitor);
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        const SLICE: Duration = Duration::from_millis(10);
        let slice_cycles = CPU_HZ as u64 / 100;
        let mut start = Instant::now();
        let mut emulated = Duration::ZERO;
        let mut last_pub = Instant::now();
        let mut last_flush = Instant::now();
        let mut pub_cycles = self.m.cycles();
        let mut drive_was_active = [false; 2];
        self.publish(0.0);
        'outer: loop {
            // Commands (block while stopped).
            if !self.running {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(c) => {
                        if !self.handle(c) {
                            break 'outer;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break 'outer,
                }
                start = Instant::now();
                emulated = Duration::ZERO;
            }
            loop {
                match rx.try_recv() {
                    Ok(c) => {
                        if !self.handle(c) {
                            break 'outer;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break 'outer,
                }
            }
            if self.running {
                self.m.run_cycles(slice_cycles);
                emulated += SLICE;
            }
            self.drain();
            // Publish often while a drive LED changes, else 10×/s.
            let leds = [self.m.drive_active(0), self.m.drive_active(1)];
            if last_pub.elapsed() >= Duration::from_millis(100) || leds != drive_was_active {
                let dt = last_pub.elapsed().as_secs_f64().max(1e-6);
                let mhz = (self.m.cycles() - pub_cycles) as f64 / dt / 1e6;
                pub_cycles = self.m.cycles();
                last_pub = Instant::now();
                drive_was_active = leds;
                self.publish(mhz);
            }
            if last_flush.elapsed() >= Duration::from_secs(1) {
                last_flush = Instant::now();
                self.flush(0);
                self.flush(1);
            }
            if self.running && !self.turbo {
                let now = start.elapsed();
                if emulated > now {
                    std::thread::sleep(emulated - now);
                } else if now - emulated > Duration::from_millis(500) {
                    start = Instant::now();
                    emulated = Duration::ZERO;
                }
            } else if self.running {
                start = Instant::now();
                emulated = Duration::ZERO;
            }
        }
        self.flush(0);
        self.flush(1);
        if let Some(mut p) = self.printer.take() {
            let _ = p.flush();
        }
    }
}
