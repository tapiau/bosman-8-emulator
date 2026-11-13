//! TCP backends: `tcp-listen:ADDR:PORT` and `tcp:HOST:PORT`.
//!
//! Each endpoint owns:
//! * a connection thread (accept loop or connect/retry loop) which, while a
//!   peer is connected, blocks reading from the socket and forwards received
//!   bytes through a channel;
//! * a writer thread which takes bytes queued by [`Endpoint::write`] and writes
//!   them to the current peer, or drops them when there is none.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use crate::{Endpoint, LineStatus};

const RETRY_INTERVAL: Duration = Duration::from_millis(500);
const ACCEPT_POLL: Duration = Duration::from_millis(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// State shared between the endpoint and its threads.
#[derive(Default)]
struct Shared {
    /// Write half of the current connection.
    stream: Mutex<Option<TcpStream>>,
    connected: AtomicBool,
    stop: AtomicBool,
    /// Address of the current peer (for `describe`).
    peer: Mutex<Option<SocketAddr>>,
}

impl Shared {
    fn stream(&self) -> MutexGuard<'_, Option<TcpStream>> {
        self.stream.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn peer(&self) -> Option<SocketAddr> {
        *self.peer.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Serve one connection until it drops. Returns `false` if the endpoint
    /// was dropped (the receiver is gone) and the thread should exit.
    fn serve(&self, mut stream: TcpStream, rx: &Sender<u8>) -> bool {
        let _ = stream.set_nodelay(true);
        let _ = stream.set_nonblocking(false);
        let writer = match stream.try_clone() {
            Ok(w) => w,
            Err(_) => return true,
        };
        *self.peer.lock().unwrap_or_else(|e| e.into_inner()) = stream.peer_addr().ok();
        *self.stream() = Some(writer);
        self.connected.store(true, Ordering::SeqCst);
        // Re-check after publishing the stream so Drop cannot miss it.
        let mut alive = !self.stopped();

        let mut buf = [0u8; 1024];
        while alive {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if buf[..n].iter().any(|&b| rx.send(b).is_err()) {
                        alive = false;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }

        self.connected.store(false, Ordering::SeqCst);
        if let Some(s) = self.stream().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
        *self.peer.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = stream.shutdown(Shutdown::Both);
        alive && !self.stopped()
    }
}

/// Writer thread body: forward queued bytes to the current connection.
fn writer_loop(shared: Arc<Shared>, tx: Receiver<u8>) {
    let mut out = Vec::with_capacity(256);
    while let Ok(b) = tx.recv() {
        out.clear();
        out.push(b);
        out.extend(tx.try_iter().take(4095));
        let mut guard = shared.stream();
        if let Some(s) = guard.as_mut() {
            if s.write_all(&out).is_err() {
                let _ = s.shutdown(Shutdown::Both);
                *guard = None;
            }
        }
        // else: nobody connected, drop the bytes.
    }
}

/// Common front end of both TCP endpoints.
struct Core {
    shared: Arc<Shared>,
    tx: Sender<u8>,
    rx: Receiver<u8>,
    conn_thread: Option<thread::JoinHandle<()>>,
}

impl Core {
    fn start(
        name: &str,
        conn_loop: impl FnOnce(Arc<Shared>, Sender<u8>) + Send + 'static,
    ) -> io::Result<Core> {
        let shared = Arc::new(Shared::default());
        let (tx, tx_rx) = mpsc::channel::<u8>();
        let (rx_tx, rx) = mpsc::channel::<u8>();
        let s = shared.clone();
        thread::Builder::new()
            .name(format!("{name}-writer"))
            .spawn(move || writer_loop(s, tx_rx))?;
        let s = shared.clone();
        let conn_thread = thread::Builder::new()
            .name(format!("{name}-conn"))
            .spawn(move || conn_loop(s, rx_tx))?;
        Ok(Core { shared, tx, rx, conn_thread: Some(conn_thread) })
    }

    fn connected(&self) -> bool {
        self.shared.connected.load(Ordering::SeqCst)
    }

    fn state(&self) -> String {
        match (self.connected(), self.shared.peer()) {
            (true, Some(p)) => format!("connected to {p}"),
            (true, None) => "connected".into(),
            (false, _) => "not connected".into(),
        }
    }

    /// Ask the threads to stop and drop the current connection.
    fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(s) = self.shared.stream().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `tcp-listen:ADDR:PORT` endpoint.
pub struct TcpListenEndpoint {
    core: Core,
    local: SocketAddr,
}

impl TcpListenEndpoint {
    /// Bind to `addr` (e.g. `"127.0.0.1:3001"`, port 0 for an ephemeral port)
    /// and start accepting clients in the background.
    pub fn bind(addr: &str) -> io::Result<TcpListenEndpoint> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let local = listener.local_addr()?;
        let core = Core::start("tcp-listen", move |shared, rx| loop {
            if shared.stopped() {
                return;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    if !shared.serve(stream, &rx) {
                        return;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => thread::sleep(RETRY_INTERVAL),
            }
        })?;
        Ok(TcpListenEndpoint { core, local })
    }

    /// The address actually bound (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }
}

impl Drop for TcpListenEndpoint {
    fn drop(&mut self) {
        // Wait (at most one accept poll interval) for the accept thread to
        // close the listening socket, so the port can be bound again at once.
        self.core.shutdown();
        if let Some(t) = self.core.conn_thread.take() {
            let _ = t.join();
        }
    }
}

impl Endpoint for TcpListenEndpoint {
    fn write(&mut self, byte: u8) {
        if self.core.connected() {
            let _ = self.core.tx.send(byte);
        }
    }
    fn read(&mut self) -> Option<u8> {
        self.core.rx.try_recv().ok()
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: self.core.connected() }
    }
    fn describe(&self) -> String {
        format!("tcp-listen {} ({})", self.local, self.core.state())
    }
}

/// `tcp:HOST:PORT` endpoint.
pub struct TcpConnectEndpoint {
    core: Core,
    target: String,
}

impl TcpConnectEndpoint {
    /// Start connecting to `target` (`HOST:PORT`) in the background, retrying
    /// every 500 ms and reconnecting after the connection drops.
    pub fn connect(target: &str) -> io::Result<TcpConnectEndpoint> {
        let valid_port = target
            .rsplit_once(':')
            .is_some_and(|(h, p)| !h.is_empty() && p.parse::<u16>().is_ok());
        if !valid_port {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid TCP address '{target}', expected HOST:PORT"),
            ));
        }
        let t = target.to_string();
        let core = Core::start("tcp", move |shared, rx| loop {
            if shared.stopped() {
                return;
            }
            match try_connect(&t) {
                Some(stream) => {
                    if !shared.serve(stream, &rx) {
                        return;
                    }
                }
                None => thread::sleep(RETRY_INTERVAL),
            }
        })?;
        Ok(TcpConnectEndpoint { core, target: target.to_string() })
    }
}

fn try_connect(target: &str) -> Option<TcpStream> {
    let addrs = target.to_socket_addrs().ok()?;
    addrs
        .into_iter()
        .find_map(|a| TcpStream::connect_timeout(&a, CONNECT_TIMEOUT).ok())
}

impl Endpoint for TcpConnectEndpoint {
    fn write(&mut self, byte: u8) {
        if self.core.connected() {
            let _ = self.core.tx.send(byte);
        }
    }
    fn read(&mut self) -> Option<u8> {
        self.core.rx.try_recv().ok()
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: self.core.connected() }
    }
    fn describe(&self) -> String {
        format!("tcp {} ({})", self.target, self.core.state())
    }
}
