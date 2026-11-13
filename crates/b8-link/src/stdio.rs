//! Host stdin/stdout backend: `stdio`.

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::{Endpoint, LineStatus};

pub(crate) struct StdioEndpoint {
    tx: Sender<u8>,
    rx: Receiver<u8>,
    #[cfg(unix)]
    raw: Option<raw::RawMode>,
}

impl StdioEndpoint {
    pub(crate) fn open() -> io::Result<StdioEndpoint> {
        #[cfg(unix)]
        let raw = raw::RawMode::enable();

        let (rx_tx, rx) = mpsc::channel::<u8>();
        thread::Builder::new().name("stdio-reader".into()).spawn(move || {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let mut buf = [0u8; 256];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => return, // EOF
                    Ok(n) => {
                        if buf[..n].iter().any(|&b| rx_tx.send(b).is_err()) {
                            return;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => return,
                }
            }
        })?;

        let (tx, tx_rx) = mpsc::channel::<u8>();
        thread::Builder::new().name("stdio-writer".into()).spawn(move || {
            let stdout = io::stdout();
            let mut out = Vec::with_capacity(256);
            while let Ok(b) = tx_rx.recv() {
                out.clear();
                out.push(b);
                out.extend(tx_rx.try_iter().take(4095));
                let mut lock = stdout.lock();
                let _ = lock.write_all(&out);
                let _ = lock.flush();
            }
        })?;

        Ok(StdioEndpoint {
            tx,
            rx,
            #[cfg(unix)]
            raw,
        })
    }
}

impl Endpoint for StdioEndpoint {
    fn write(&mut self, byte: u8) {
        let _ = self.tx.send(byte);
    }
    fn read(&mut self) -> Option<u8> {
        self.rx.try_recv().ok()
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: true }
    }
    fn describe(&self) -> String {
        #[cfg(unix)]
        if self.raw.is_some() {
            return "stdio (raw tty)".into();
        }
        "stdio".into()
    }
}

#[cfg(unix)]
mod raw {
    /// Saved terminal attributes; restored on drop.
    pub(super) struct RawMode {
        saved: libc::termios,
    }

    impl RawMode {
        /// Switch stdin to raw mode if it is a terminal.
        pub(super) fn enable() -> Option<RawMode> {
            // SAFETY: plain libc calls on fd 0 with a properly sized termios.
            unsafe {
                if libc::isatty(libc::STDIN_FILENO) != 1 {
                    return None;
                }
                let mut saved: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) != 0 {
                    return None;
                }
                let mut raw = saved;
                libc::cfmakeraw(&mut raw);
                raw.c_cc[libc::VMIN] = 1;
                raw.c_cc[libc::VTIME] = 0;
                if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                    return None;
                }
                Some(RawMode { saved })
            }
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            // SAFETY: restoring attributes previously read from the same fd.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved);
            }
        }
    }
}
