//! TCP backends: `tcp-listen:ADDR:PORT` and `tcp:HOST:PORT`.
//!
//! Each endpoint owns:
//! * a connection thread (accept loop or connect/retry loop) which, while a
//!   peer is connected, blocks reading from the socket and forwards received
//!   bytes through a channel;
//! * a writer thread which takes bytes queued by [`Endpoint::write`] and writes
//!   them to the current peer, or drops them when there is none.
//!
//! Flow control (RTS/CTS) is carried with a subset of RFC 2217, see the crate
//! documentation and [`Telnet`].

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use crate::{Endpoint, LineStatus};

// Telnet (RFC 854) and COM Port Control (RFC 2217) codes.
const IAC: u8 = 0xFF;
const DONT: u8 = 0xFE;
const DO: u8 = 0xFD;
const WONT: u8 = 0xFC;
const WILL: u8 = 0xFB;
const SB: u8 = 0xFA;
const SE: u8 = 0xF0;
const COM_PORT: u8 = 44;
/// Client → server: SET-CONTROL (value 11 = RTS ON, 12 = RTS OFF); the
/// server answers with the same value under code 105.
const SET_CONTROL: u8 = 5;
const SET_CONTROL_ACK: u8 = 105;
const RTS_ON: u8 = 11;
const RTS_OFF: u8 = 12;
/// Server → client: NOTIFY-MODEMSTATE; bit 4 = CTS.
const NOTIFY_MODEMSTATE: u8 = 107;
const MODEM_CTS: u8 = 0x10;
/// What a client sends first on every connection.
const OFFER: [u8; 3] = [IAC, WILL, COM_PORT];

/// Something the Telnet parser wants done.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// A data byte for the emulator.
    Data(u8),
    /// Bytes to send to the peer as they are (Telnet replies).
    Send(Vec<u8>),
    /// The peer's RTS (our CTS).
    PeerRts(bool),
    /// Telnet framing is now active: start escaping 0xFF, announce our RTS.
    Activate,
    /// The peer refused the option: back to a raw byte stream.
    Deactivate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// Server, start of a connection: is it [`OFFER`]?
    Detect(Vec<u8>),
    Raw,
    /// `confirmed`: the server agreed (client side) / always true on a server.
    Telnet { confirmed: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    Data,
    Iac,
    Opt(u8),
    Sb,
    SbIac,
}

/// Incoming side of the RFC 2217 subset, one per connection. Pure state
/// machine: bytes in, [`Action`]s out.
#[derive(Debug, Clone)]
struct Telnet {
    server: bool,
    mode: Mode,
    st: St,
    sb: Vec<u8>,
}

impl Telnet {
    /// Server side: Telnet only if the connection starts with [`OFFER`].
    fn server() -> Self {
        Telnet { server: true, mode: Mode::Detect(Vec::new()), st: St::Data, sb: Vec::new() }
    }

    /// Client side, after sending [`OFFER`] (`offer = false`: raw stream).
    fn client(offer: bool) -> Self {
        let mode = if offer { Mode::Telnet { confirmed: false } } else { Mode::Raw };
        Telnet { server: false, mode, st: St::Data, sb: Vec::new() }
    }

    fn feed(&mut self, b: u8, out: &mut Vec<Action>) {
        match &mut self.mode {
            Mode::Raw => out.push(Action::Data(b)),
            Mode::Detect(buf) => {
                buf.push(b);
                if buf[..] == OFFER[..] {
                    self.mode = Mode::Telnet { confirmed: true };
                    out.push(Action::Send(vec![IAC, DO, COM_PORT]));
                    out.push(Action::Activate);
                } else if !OFFER.starts_with(buf) {
                    out.extend(buf.iter().map(|&x| Action::Data(x)));
                    self.mode = Mode::Raw;
                }
            }
            Mode::Telnet { confirmed } => {
                let confirmed = *confirmed;
                self.telnet_byte(b, confirmed, out);
            }
        }
    }

    fn telnet_byte(&mut self, b: u8, confirmed: bool, out: &mut Vec<Action>) {
        self.st = match self.st {
            St::Data if b == IAC => St::Iac,
            St::Data => {
                out.push(Action::Data(b));
                St::Data
            }
            St::Iac => match b {
                IAC => {
                    out.push(Action::Data(IAC));
                    St::Data
                }
                SB => {
                    self.sb.clear();
                    St::Sb
                }
                WILL | WONT | DO | DONT => St::Opt(b),
                // Before the server answered, a 0xFF that does not start a
                // command is data from a raw server.
                _ if !confirmed && b < SE => {
                    out.push(Action::Data(IAC));
                    out.push(Action::Data(b));
                    St::Data
                }
                _ => St::Data, // NOP, GA and other commands: ignored
            },
            St::Opt(cmd) => {
                self.option(cmd, b, out);
                St::Data
            }
            St::Sb if b == IAC => St::SbIac,
            St::Sb => {
                if self.sb.len() < 64 {
                    self.sb.push(b);
                }
                St::Sb
            }
            St::SbIac => match b {
                SE => {
                    self.subnegotiation(out);
                    St::Data
                }
                IAC => {
                    self.sb.push(IAC);
                    St::Sb
                }
                _ => St::Data,
            },
        };
    }

    fn option(&mut self, cmd: u8, opt: u8, out: &mut Vec<Action>) {
        match (cmd, opt) {
            (DO, COM_PORT) if !self.server => {
                if self.mode == (Mode::Telnet { confirmed: false }) {
                    self.mode = Mode::Telnet { confirmed: true };
                    out.push(Action::Activate);
                }
            }
            (WILL, COM_PORT) if self.server => out.push(Action::Send(vec![IAC, DO, COM_PORT])),
            (DONT | WONT, COM_PORT) => {
                self.mode = Mode::Raw;
                out.push(Action::Deactivate);
            }
            (DO, o) => out.push(Action::Send(vec![IAC, WONT, o])),
            (WILL, o) => out.push(Action::Send(vec![IAC, DONT, o])),
            _ => {}
        }
    }

    fn subnegotiation(&mut self, out: &mut Vec<Action>) {
        let [COM_PORT, cmd, v, ..] = self.sb[..] else { return };
        match cmd {
            SET_CONTROL if self.server => {
                match v {
                    RTS_ON => out.push(Action::PeerRts(true)),
                    RTS_OFF => out.push(Action::PeerRts(false)),
                    _ => {}
                }
                out.push(Action::Send(sb(SET_CONTROL_ACK, v)));
            }
            NOTIFY_MODEMSTATE if !self.server => out.push(Action::PeerRts(v & MODEM_CTS != 0)),
            _ => {}
        }
    }
}

/// `IAC SB COM-PORT-OPTION cmd value IAC SE` (value 0xFF doubled).
fn sb(cmd: u8, v: u8) -> Vec<u8> {
    let mut m = vec![IAC, SB, COM_PORT, cmd, v];
    if v == IAC {
        m.push(IAC);
    }
    m.extend([IAC, SE]);
    m
}

/// Our RTS as sent by a client (SET-CONTROL) or a server (NOTIFY-MODEMSTATE).
fn rts_message(server: bool, ready: bool) -> Vec<u8> {
    if server {
        sb(NOTIFY_MODEMSTATE, if ready { MODEM_CTS } else { 0 })
    } else {
        sb(SET_CONTROL, if ready { RTS_ON } else { RTS_OFF })
    }
}

/// Item queued for the writer thread.
enum Out {
    Data(u8),
    /// Telnet command bytes, sent as they are.
    Raw(Vec<u8>),
    /// Send our current RTS (if Telnet framing is active).
    Rts,
}

const RETRY_INTERVAL: Duration = Duration::from_millis(500);
const ACCEPT_POLL: Duration = Duration::from_millis(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// State shared between the endpoint and its threads.
struct Shared {
    /// Write half of the current connection.
    stream: Mutex<Option<TcpStream>>,
    connected: AtomicBool,
    stop: AtomicBool,
    /// Address of the current peer (for `describe`).
    peer: Mutex<Option<SocketAddr>>,
    /// Server side of the Telnet negotiation (else client).
    server: bool,
    /// Client: open each connection with [`OFFER`].
    offer: bool,
    /// Telnet framing active on the current connection.
    telnet: AtomicBool,
    /// Our RTS (what the emulator asked for).
    rts: AtomicBool,
    /// The peer's RTS = our CTS (true when unknown).
    peer_rts: AtomicBool,
}

impl Shared {
    fn new(server: bool, offer: bool) -> Shared {
        Shared {
            stream: Mutex::new(None),
            connected: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            peer: Mutex::new(None),
            server,
            offer,
            telnet: AtomicBool::new(false),
            rts: AtomicBool::new(true),
            peer_rts: AtomicBool::new(true),
        }
    }

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
    fn serve(&self, mut stream: TcpStream, rx: &Sender<u8>, out: &Sender<Out>) -> bool {
        let _ = stream.set_nodelay(true);
        let _ = stream.set_nonblocking(false);
        let mut writer = match stream.try_clone() {
            Ok(w) => w,
            Err(_) => return true,
        };
        self.telnet.store(false, Ordering::SeqCst);
        self.peer_rts.store(true, Ordering::SeqCst);
        let mut telnet = if self.server { Telnet::server() } else { Telnet::client(self.offer) };
        // The offer must be the very first bytes of the connection.
        if self.offer && writer.write_all(&OFFER).is_err() {
            return true;
        }
        *self.peer.lock().unwrap_or_else(|e| e.into_inner()) = stream.peer_addr().ok();
        *self.stream() = Some(writer);
        self.connected.store(true, Ordering::SeqCst);
        // Re-check after publishing the stream so Drop cannot miss it.
        let mut alive = !self.stopped();

        let mut buf = [0u8; 1024];
        let mut actions = Vec::new();
        while alive {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    actions.clear();
                    for &b in &buf[..n] {
                        telnet.feed(b, &mut actions);
                    }
                    for a in actions.drain(..) {
                        let ok = match a {
                            Action::Data(b) => rx.send(b).is_ok(),
                            Action::Send(v) => out.send(Out::Raw(v)).is_ok(),
                            Action::PeerRts(r) => {
                                self.peer_rts.store(r, Ordering::SeqCst);
                                true
                            }
                            Action::Activate => {
                                self.telnet.store(true, Ordering::SeqCst);
                                out.send(Out::Rts).is_ok()
                            }
                            Action::Deactivate => {
                                self.telnet.store(false, Ordering::SeqCst);
                                self.peer_rts.store(true, Ordering::SeqCst);
                                true
                            }
                        };
                        if !ok {
                            alive = false;
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }

        self.connected.store(false, Ordering::SeqCst);
        self.telnet.store(false, Ordering::SeqCst);
        self.peer_rts.store(true, Ordering::SeqCst);
        if let Some(s) = self.stream().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
        *self.peer.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = stream.shutdown(Shutdown::Both);
        alive && !self.stopped()
    }
}

/// Writer thread body: forward queued bytes to the current connection.
fn writer_loop(shared: Arc<Shared>, tx: Receiver<Out>) {
    let mut out = Vec::with_capacity(256);
    while let Ok(first) = tx.recv() {
        out.clear();
        let telnet = shared.telnet.load(Ordering::SeqCst);
        for item in std::iter::once(first).chain(tx.try_iter().take(4095)) {
            match item {
                Out::Data(IAC) if telnet => out.extend([IAC, IAC]),
                Out::Data(b) => out.push(b),
                Out::Raw(v) => out.extend(v),
                Out::Rts if telnet => out.extend(rts_message(shared.server, shared.rts.load(Ordering::SeqCst))),
                Out::Rts => {}
            }
        }
        if out.is_empty() {
            continue;
        }
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
    tx: Sender<Out>,
    rx: Receiver<u8>,
    conn_thread: Option<thread::JoinHandle<()>>,
}

impl Core {
    fn start(
        name: &str,
        shared: Shared,
        conn_loop: impl FnOnce(Arc<Shared>, Sender<u8>, Sender<Out>) + Send + 'static,
    ) -> io::Result<Core> {
        let shared = Arc::new(shared);
        let (tx, tx_rx) = mpsc::channel::<Out>();
        let (rx_tx, rx) = mpsc::channel::<u8>();
        let s = shared.clone();
        thread::Builder::new()
            .name(format!("{name}-writer"))
            .spawn(move || writer_loop(s, tx_rx))?;
        let s = shared.clone();
        let out = tx.clone();
        let conn_thread = thread::Builder::new()
            .name(format!("{name}-conn"))
            .spawn(move || conn_loop(s, rx_tx, out))?;
        Ok(Core { shared, tx, rx, conn_thread: Some(conn_thread) })
    }

    fn write(&self, byte: u8) {
        if self.connected() {
            let _ = self.tx.send(Out::Data(byte));
        }
    }

    fn set_rts(&self, ready: bool) {
        if self.shared.rts.swap(ready, Ordering::SeqCst) != ready && self.connected() {
            let _ = self.tx.send(Out::Rts);
        }
    }

    fn cts(&self) -> bool {
        self.shared.peer_rts.load(Ordering::SeqCst)
    }

    fn connected(&self) -> bool {
        self.shared.connected.load(Ordering::SeqCst)
    }

    fn state(&self) -> String {
        let fc = if self.shared.telnet.load(Ordering::SeqCst) { ", RTS/CTS" } else { "" };
        match (self.connected(), self.shared.peer()) {
            (true, Some(p)) => format!("connected to {p}{fc}"),
            (true, None) => format!("connected{fc}"),
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
        let core = Core::start("tcp-listen", Shared::new(true, false), move |shared, rx, out| loop {
            if shared.stopped() {
                return;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    if !shared.serve(stream, &rx, &out) {
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
        self.core.write(byte);
    }
    fn set_rts(&mut self, ready: bool) {
        self.core.set_rts(ready);
    }
    fn cts(&self) -> bool {
        self.core.cts()
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
    /// every 500 ms and reconnecting after the connection drops. Each
    /// connection offers RFC 2217 flow control (see the crate docs).
    pub fn connect(target: &str) -> io::Result<TcpConnectEndpoint> {
        Self::start(target, true)
    }

    /// Like [`connect`](Self::connect), but a plain byte stream (no Telnet).
    pub fn connect_raw(target: &str) -> io::Result<TcpConnectEndpoint> {
        Self::start(target, false)
    }

    fn start(target: &str, offer: bool) -> io::Result<TcpConnectEndpoint> {
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
        let core = Core::start("tcp", Shared::new(false, offer), move |shared, rx, out| loop {
            if shared.stopped() {
                return;
            }
            match try_connect(&t) {
                Some(stream) => {
                    if !shared.serve(stream, &rx, &out) {
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
        self.core.write(byte);
    }
    fn set_rts(&mut self, ready: bool) {
        self.core.set_rts(ready);
    }
    fn cts(&self) -> bool {
        self.core.cts()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(t: &mut Telnet, bytes: &[u8]) -> Vec<Action> {
        let mut out = Vec::new();
        for &b in bytes {
            t.feed(b, &mut out);
        }
        out
    }

    fn data(a: &[Action]) -> Vec<u8> {
        a.iter().filter_map(|x| if let Action::Data(b) = x { Some(*b) } else { None }).collect()
    }

    #[test]
    fn server_detects_offer_only_at_start() {
        let mut t = Telnet::server();
        let a = feed(&mut t, &[IAC, WILL, COM_PORT, b'A', IAC, IAC, b'B']);
        assert_eq!(a[0], Action::Send(vec![IAC, DO, COM_PORT]));
        assert_eq!(a[1], Action::Activate);
        assert_eq!(data(&a), vec![b'A', 0xFF, b'B']);
        // A raw client: everything is data, 0xFF included.
        let mut t = Telnet::server();
        assert_eq!(data(&feed(&mut t, &[IAC, b'x', IAC, WILL, COM_PORT])), vec![IAC, b'x', IAC, WILL, COM_PORT]);
        let mut t = Telnet::server();
        assert_eq!(data(&feed(&mut t, b"DIR\r")), b"DIR\r");
    }

    #[test]
    fn rts_messages_both_ways() {
        // client RTS -> server
        let mut srv = Telnet::server();
        feed(&mut srv, &OFFER);
        let a = feed(&mut srv, &rts_message(false, false));
        assert_eq!(a, vec![Action::PeerRts(false), Action::Send(sb(SET_CONTROL_ACK, RTS_OFF))]);
        assert_eq!(feed(&mut srv, &rts_message(false, true))[0], Action::PeerRts(true));
        // server RTS -> client (after DO)
        let mut cli = Telnet::client(true);
        assert_eq!(feed(&mut cli, &[IAC, DO, COM_PORT]), vec![Action::Activate]);
        assert_eq!(feed(&mut cli, &rts_message(true, false)), vec![Action::PeerRts(false)]);
        assert_eq!(feed(&mut cli, &rts_message(true, true)), vec![Action::PeerRts(true)]);
        // the ack of our SET-CONTROL is ignored
        assert!(feed(&mut cli, &sb(SET_CONTROL_ACK, RTS_ON)).is_empty());
    }

    #[test]
    fn client_tolerates_raw_server_and_refuses_other_options() {
        let mut cli = Telnet::client(true);
        // unconfirmed: 0xFF followed by an ordinary byte is data
        assert_eq!(data(&feed(&mut cli, &[b'a', IAC, b'b'])), vec![b'a', IAC, b'b']);
        assert_eq!(feed(&mut cli, &[IAC, DO, 1]), vec![Action::Send(vec![IAC, WONT, 1])]);
        assert_eq!(feed(&mut cli, &[IAC, DONT, COM_PORT]), vec![Action::Deactivate]);
        assert_eq!(data(&feed(&mut cli, &[IAC, IAC])), vec![IAC, IAC], "raw after refusal");
        let mut raw = Telnet::client(false);
        assert_eq!(data(&feed(&mut raw, &[IAC, DO, COM_PORT])), vec![IAC, DO, COM_PORT]);
    }
}
