//! Host serial port backend: `serial:DEV[@BAUD][,FORMAT][,rtscts]`.
//!
//! With `rtscts` a third thread polls the port's CTS input every
//! [`LINES_POLL`] and applies RTS changes requested through
//! [`Endpoint::set_rts`] right away (not queued behind timed bytes); the
//! writer holds back data while CTS is off. The driver's own hardware flow
//! control stays off: on Linux it would tie RTS to the OS receive buffer,
//! while here RTS must follow the emulated receiver.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serialport::{DataBits, FlowControl, Parity, StopBits};

use crate::{Endpoint, LineParams, LineStatus};

const READ_TIMEOUT: Duration = Duration::from_millis(20);
/// How often the RTS/CTS thread samples CTS and applies RTS.
const LINES_POLL: Duration = Duration::from_millis(1);
const BREAK_LENGTH: Duration = Duration::from_millis(250);
// USB serial tcdrain may finish when a bulk transfer reaches the adapter,
// before its UART has shifted the final bytes. Keep a guard before changing
// the line coding; otherwise the 60/00 command can be corrupted in flight.
const BAUD_CHANGE_GUARD: Duration = Duration::from_millis(20);

/// How late a timed byte may be before the time base is re-anchored.
const LATE_RESYNC: Duration = Duration::from_millis(5);
/// Bytes scheduled further ahead than this are sent at once (re-anchored).
const MAX_AHEAD: Duration = Duration::from_millis(200);

#[derive(Default)]
struct TxClock {
    anchor: Option<(Instant, f64)>,
}

impl TxClock {
    fn target(&mut self, now: Instant, t: f64) -> Instant {
        let target = match self.anchor {
            Some((w, e)) if t >= e => w + Duration::from_secs_f64(t - e),
            _ => now,
        };
        if self.anchor.is_none() || target + LATE_RESYNC < now || target > now + MAX_AHEAD {
            self.anchor = Some((now, t));
            now
        } else {
            target
        }
    }

    fn params_applied(&mut self, now: Instant, t: Option<f64>) {
        self.anchor = t.map(|t| (now, t));
    }
}

enum TxMsg {
    Byte(u8),
    /// Byte with the emulated time (seconds) at which it left the emulated chip.
    Timed(u8, f64),
    Break,
    Params(LineParams, Option<f64>),
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
    /// Hardware flow control through the RTS / CTS lines.
    rtscts: bool,
    /// RTS requested by the emulator.
    rts: Arc<AtomicBool>,
    /// Last sampled CTS.
    cts: Arc<AtomicBool>,
}

impl SerialEndpoint {
    /// Open `dev` with the initial line setting `p` (the emulated chip may
    /// change it later via [`Endpoint::set_line_params`]).
    pub(crate) fn open(dev: &str, p: LineParams, rtscts: bool) -> io::Result<SerialEndpoint> {
        let baud = p.baud;
        let port = serialport::new(dev, baud)
            .data_bits(match p.data_bits {
                5 => DataBits::Five,
                6 => DataBits::Six,
                7 => DataBits::Seven,
                _ => DataBits::Eight,
            })
            .parity(match p.parity {
                crate::Parity::None => Parity::None,
                crate::Parity::Odd => Parity::Odd,
                crate::Parity::Even => Parity::Even,
            })
            .stop_bits(match p.stop_bits {
                crate::StopBits::One => StopBits::One,
                _ => StopBits::Two,
            })
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
        let rts = Arc::new(AtomicBool::new(true));
        let cts = Arc::new(AtomicBool::new(true));
        if rtscts {
            let mut lines = writer.try_clone().map_err(|e| io::Error::other(format!("{dev}: {e}")))?;
            let (rts_l, cts_l, stop_l) = (rts.clone(), cts.clone(), stop.clone());
            thread::Builder::new().name("serial-lines".into()).spawn(move || {
                let mut applied: Option<bool> = None;
                while !stop_l.load(Ordering::SeqCst) {
                    let want = rts_l.load(Ordering::SeqCst);
                    if applied != Some(want) && lines.write_request_to_send(want).is_ok() {
                        applied = Some(want);
                    }
                    // A driver that can't report CTS must not stall the emulator.
                    let c = lines.read_clear_to_send().unwrap_or(true);
                    cts_l.store(c, Ordering::SeqCst);
                    thread::sleep(LINES_POLL);
                }
            })?;
        }
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

        let line = Arc::new(Mutex::new(p.to_string()));
        let line_w = line.clone();
        let ok_w = ok.clone();
        let (cts_w, stop_w) = (cts.clone(), stop.clone());
        // Hold data while the peer's RTS (our CTS) is off.
        let wait_cts = move || {
            while rtscts && !cts_w.load(Ordering::SeqCst) && !stop_w.load(Ordering::SeqCst) {
                thread::sleep(LINES_POLL);
            }
        };
        thread::Builder::new().name("serial-writer".into()).spawn(move || {
            let mut out = Vec::with_capacity(256);
            // Mapping of emulated time to wall time for timed bytes.
            let mut clock = TxClock::default();
            let mut pending: Option<TxMsg> = None;
            let mut last_write: Option<Instant> = None;
            while let Some(msg) = pending.take().or_else(|| tx_rx.recv().ok()) {
                // Timed bytes: wait until the wall-clock moment corresponding to
                // their emulated time, then send them together with the
                // following bytes that are due at (almost) the same moment.
                if let TxMsg::Timed(b, t) = msg {
                    let now = Instant::now();
                    let target = clock.target(now, t);
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
                    wait_cts();
                    if writer.write_all(&out).is_err() || writer.flush().is_err() {
                        ok_w.store(false, Ordering::SeqCst);
                    }
                    last_write = Some(Instant::now());
                    continue;
                }
                let mut pending_break = false;
                let mut pending_params = None;
                out.clear();
                match msg {
                    TxMsg::Byte(b) | TxMsg::Timed(b, _) => out.push(b),
                    TxMsg::Break => pending_break = true,
                    TxMsg::Params(p, t) => pending_params = Some((p, t)),
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
                            TxMsg::Params(p, t) => {
                                pending_params = Some((p, t));
                                break;
                            }
                        }
                    }
                }
                if !out.is_empty() {
                    wait_cts();
                    if writer.write_all(&out).is_err() || writer.flush().is_err() {
                        ok_w.store(false, Ordering::SeqCst);
                    }
                    last_write = Some(Instant::now());
                }
                if pending_break && writer.set_break().is_ok() {
                    thread::sleep(BREAK_LENGTH);
                    let _ = writer.clear_break();
                }
                if let Some((p, t)) = pending_params {
                    // Bytes queued before the change still go out at the old speed:
                    // Drain the OS queue, then allow the USB adapter's UART
                    // to finish too before changing its line coding.
                    let _ = writer.flush();
                    if let Some(last) = last_write.take() {
                        let remaining = (last + BAUD_CHANGE_GUARD).saturating_duration_since(Instant::now());
                        if !remaining.is_zero() {
                            thread::sleep(remaining);
                        }
                    }
                    let text = match apply_params(&mut writer, p) {
                        // Report what the driver actually set (it may round).
                        Ok(()) => match writer.baud_rate() {
                            Ok(actual) if actual != p.baud => format!("{p} (port: {actual})"),
                            _ => p.to_string(),
                        },
                        Err(e) => format!("{p} failed: {e}"),
                    };
                    // Port configuration can take longer than the guest's
                    // settling delay (especially through USB). Start a new
                    // wall-clock epoch *after* it completes, so the next byte
                    // still observes that delay rather than catching up.
                    clock.params_applied(Instant::now(), t);
                    if let Ok(mut l) = line_w.lock() {
                        *l = text;
                    }
                }
            }
        })?;

        Ok(SerialEndpoint { dev: dev.to_string(), line, tx, rx, ok, stop, rtscts, rts, cts })
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
        let _ = self.tx.send(TxMsg::Params(params, None));
    }
    fn set_line_params_at(&mut self, params: LineParams, t: f64) {
        let _ = self.tx.send(TxMsg::Params(params, Some(t)));
    }
    fn set_rts(&mut self, ready: bool) {
        if self.rtscts {
            self.rts.store(ready, Ordering::SeqCst);
        }
    }
    fn cts(&self) -> bool {
        !self.rtscts || self.cts.load(Ordering::SeqCst)
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: self.ok.load(Ordering::SeqCst) }
    }
    fn describe(&self) -> String {
        let state = if self.ok.load(Ordering::SeqCst) { "open" } else { "error" };
        let line = self.line.lock().map(|l| l.clone()).unwrap_or_default();
        let fc = match (self.rtscts, self.cts.load(Ordering::SeqCst)) {
            (false, _) => "",
            (true, true) => ", RTS/CTS",
            (true, false) => ", RTS/CTS: CTS off",
        };
        format!("serial {} @ {line}{fc} ({state})", self.dev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_baud_change_preserves_guest_settling_delay() {
        let start = Instant::now();
        let mut clock = TxClock::default();
        assert_eq!(clock.target(start, 8.182), start); // command 60
        // The USB driver finished draining 00 and changing baud 20 ms later.
        let configured = start + Duration::from_millis(20);
        clock.params_applied(configured, Some(8.183));
        let first_header = clock.target(configured, 8.1855);
        let gap = first_header.duration_since(configured).as_secs_f64();
        assert!((gap - 0.0025).abs() < 0.000001, "settling gap: {gap}");
        // Subsequent bytes retain their spacing within the new epoch.
        let next = clock.target(configured, 8.186);
        assert!((next.duration_since(first_header).as_secs_f64() - 0.0005).abs() < 0.000001);
    }

    #[test]
    fn untimed_change_discards_previous_epoch() {
        let start = Instant::now();
        let mut clock = TxClock::default();
        clock.target(start, 10.0);
        clock.params_applied(start, None);
        assert_eq!(clock.target(start, 10.1), start);
    }
}
