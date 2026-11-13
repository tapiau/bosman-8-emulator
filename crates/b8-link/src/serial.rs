//! Host serial port backend: `serial:DEV[@BAUD]`.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serialport::{DataBits, FlowControl, Parity, StopBits};

use crate::{Endpoint, LineParams, LineStatus};

const READ_TIMEOUT: Duration = Duration::from_millis(20);
const BREAK_LENGTH: Duration = Duration::from_millis(250);

/// How late a timed byte may be before the time base is re-anchored.
const LATE_RESYNC: Duration = Duration::from_millis(5);
/// Bytes scheduled further ahead than this are sent at once (re-anchored).
const MAX_AHEAD: Duration = Duration::from_millis(200);

enum TxMsg {
    Byte(u8),
    /// Byte with the emulated time (seconds) at which it left the emulated chip.
    Timed(u8, f64),
    Break,
    Params(LineParams),
}

fn apply_params(port: &mut Box<dyn serialport::SerialPort>, p: LineParams) -> serialport::Result<()> {
    port.set_baud_rate(p.baud)?;
    port.set_data_bits(match p.data_bits {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    })?;
    port.set_parity(match p.parity {
        crate::Parity::None => Parity::None,
        crate::Parity::Odd => Parity::Odd,
        crate::Parity::Even => Parity::Even,
    })?;
    // serialport has no 1.5 stop bits; 2 is the closest a receiver accepts.
    port.set_stop_bits(match p.stop_bits {
        crate::StopBits::One => StopBits::One,
        _ => StopBits::Two,
    })
}

pub(crate) struct SerialEndpoint {
    dev: String,
    /// Current line setting (shown by describe), updated by the writer thread.
    line: Arc<Mutex<String>>,
    tx: Sender<TxMsg>,
    rx: Receiver<u8>,
    ok: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl SerialEndpoint {
    pub(crate) fn open(dev: &str, baud: u32) -> io::Result<SerialEndpoint> {
        let port = serialport::new(dev, baud)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
            .timeout(READ_TIMEOUT)
            .open()
            .map_err(|e| io::Error::other(format!("{dev}: {e}")))?;
        let mut reader = port
            .try_clone()
            .map_err(|e| io::Error::other(format!("{dev}: {e}")))?;
        let mut writer = port;

        let ok = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, tx_rx) = mpsc::channel::<TxMsg>();
        let (rx_tx, rx) = mpsc::channel::<u8>();

        let (ok_r, stop_r) = (ok.clone(), stop.clone());
        thread::Builder::new().name("serial-reader".into()).spawn(move || {
            let mut buf = [0u8; 256];
            while !stop_r.load(Ordering::SeqCst) {
                match reader.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        if buf[..n].iter().any(|&b| rx_tx.send(b).is_err()) {
                            return;
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::TimedOut
                                | io::ErrorKind::WouldBlock
                                | io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => {
                        ok_r.store(false, Ordering::SeqCst);
                        return;
                    }
                }
            }
        })?;

        let line = Arc::new(Mutex::new(format!("{baud} 8N1")));
        let line_w = line.clone();
        let ok_w = ok.clone();
        thread::Builder::new().name("serial-writer".into()).spawn(move || {
            let mut out = Vec::with_capacity(256);
            // Mapping of emulated time to wall time for timed bytes.
            let mut anchor: Option<(Instant, f64)> = None;
            let mut pending: Option<TxMsg> = None;
            while let Some(msg) = pending.take().or_else(|| tx_rx.recv().ok()) {
                // Timed bytes: wait until the wall-clock moment corresponding to
                // their emulated time, then send them together with the
                // following bytes that are due at (almost) the same moment.
                if let TxMsg::Timed(b, t) = msg {
                    let now = Instant::now();
                    let target = match anchor {
                        Some((w, e)) if t >= e => w + Duration::from_secs_f64(t - e),
                        _ => now,
                    };
                    let target = if target + LATE_RESYNC < now || target > now + MAX_AHEAD {
                        anchor = Some((now, t));
                        now
                    } else {
                        if anchor.is_none() {
                            anchor = Some((now, t));
                        }
                        target
                    };
                    if target > now {
                        thread::sleep(target - now);
                    }
                    out.clear();
                    out.push(b);
                    for m in tx_rx.try_iter() {
                        match m {
                            TxMsg::Timed(b2, t2) if t2 - t < 0.0002 => out.push(b2),
                            TxMsg::Byte(b2) => out.push(b2),
                            other => {
                                pending = Some(other);
                                break;
                            }
                        }
                    }
                    if writer.write_all(&out).is_err() || writer.flush().is_err() {
                        ok_w.store(false, Ordering::SeqCst);
                    }
                    continue;
                }
                let mut pending_break = false;
                let mut pending_params = None;
                out.clear();
                match msg {
                    TxMsg::Byte(b) | TxMsg::Timed(b, _) => out.push(b),
                    TxMsg::Break => pending_break = true,
                    TxMsg::Params(p) => pending_params = Some(p),
                }
                if !pending_break && pending_params.is_none() {
                    for m in tx_rx.try_iter() {
                        match m {
                            TxMsg::Byte(b) => out.push(b),
                            m @ TxMsg::Timed(..) => {
                                pending = Some(m);
                                break;
                            }
                            TxMsg::Break => {
                                pending_break = true;
                                break;
                            }
                            TxMsg::Params(p) => {
                                pending_params = Some(p);
                                break;
                            }
                        }
                    }
                }
                if !out.is_empty() && (writer.write_all(&out).is_err() || writer.flush().is_err()) {
                    ok_w.store(false, Ordering::SeqCst);
                }
                if pending_break && writer.set_break().is_ok() {
                    thread::sleep(BREAK_LENGTH);
                    let _ = writer.clear_break();
                }
                if let Some(p) = pending_params {
                    // Bytes queued before the change still go out at the old speed:
                    // flush() waits until the OS has transmitted them.
                    let _ = writer.flush();
                    let text = match apply_params(&mut writer, p) {
                        // Report what the driver actually set (it may round).
                        Ok(()) => match writer.baud_rate() {
                            Ok(actual) if actual != p.baud => format!("{p} (port: {actual})"),
                            _ => p.to_string(),
                        },
                        Err(e) => format!("{p} failed: {e}"),
                    };
                    if let Ok(mut l) = line_w.lock() {
                        *l = text;
                    }
                }
            }
        })?;

        Ok(SerialEndpoint { dev: dev.to_string(), line, tx, rx, ok, stop })
    }
}

impl Drop for SerialEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Endpoint for SerialEndpoint {
    fn write(&mut self, byte: u8) {
        let _ = self.tx.send(TxMsg::Byte(byte));
    }
    fn write_at(&mut self, byte: u8, t: f64) {
        let _ = self.tx.send(TxMsg::Timed(byte, t));
    }
    fn read(&mut self) -> Option<u8> {
        self.rx.try_recv().ok()
    }
    fn send_break(&mut self) {
        let _ = self.tx.send(TxMsg::Break);
    }
    fn set_line_params(&mut self, params: LineParams) {
        let _ = self.tx.send(TxMsg::Params(params));
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: self.ok.load(Ordering::SeqCst) }
    }
    fn describe(&self) -> String {
        let state = if self.ok.load(Ordering::SeqCst) { "open" } else { "error" };
        let line = self.line.lock().map(|l| l.clone()).unwrap_or_default();
        format!("serial {} @ {line} ({state})", self.dev)
    }
}
