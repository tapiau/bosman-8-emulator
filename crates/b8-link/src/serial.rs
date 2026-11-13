//! Host serial port backend: `serial:DEV[@BAUD]`.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serialport::{DataBits, FlowControl, Parity, StopBits};

use crate::{Endpoint, LineStatus};

const READ_TIMEOUT: Duration = Duration::from_millis(20);
const BREAK_LENGTH: Duration = Duration::from_millis(250);

enum TxMsg {
    Byte(u8),
    Break,
}

pub(crate) struct SerialEndpoint {
    dev: String,
    baud: u32,
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

        let ok_w = ok.clone();
        thread::Builder::new().name("serial-writer".into()).spawn(move || {
            let mut out = Vec::with_capacity(256);
            while let Ok(msg) = tx_rx.recv() {
                let mut pending_break = false;
                out.clear();
                match msg {
                    TxMsg::Byte(b) => out.push(b),
                    TxMsg::Break => pending_break = true,
                }
                if !pending_break {
                    for m in tx_rx.try_iter() {
                        match m {
                            TxMsg::Byte(b) => out.push(b),
                            TxMsg::Break => {
                                pending_break = true;
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
            }
        })?;

        Ok(SerialEndpoint { dev: dev.to_string(), baud, tx, rx, ok, stop })
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
    fn read(&mut self) -> Option<u8> {
        self.rx.try_recv().ok()
    }
    fn send_break(&mut self) {
        let _ = self.tx.send(TxMsg::Break);
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: self.ok.load(Ordering::SeqCst) }
    }
    fn describe(&self) -> String {
        let state = if self.ok.load(Ordering::SeqCst) { "open" } else { "error" };
        format!("serial {} @ {} 8N1 ({state})", self.dev, self.baud)
    }
}
