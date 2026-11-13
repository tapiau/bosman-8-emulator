//! Virtual serial line: a non-blocking byte-stream endpoint that an emulated
//! serial chip channel is attached to.
//!
//! The emulator core never blocks on I/O: it calls [`Endpoint::write`] for
//! each transmitted byte and polls [`Endpoint::read`] for received bytes.
//! Backends doing real I/O (TCP, host serial port, stdio) run a background
//! thread and talk to the emulator through channels.
//!
//! Endpoint specification strings (used on the command line), see [`open`]:
//!
//! | spec                    | meaning                                              |
//! |-------------------------|------------------------------------------------------|
//! | `none`                  | not connected; writes dropped, nothing received      |
//! | `tcp-listen:ADDR:PORT`  | accept connections (one at a time, re-accepts after disconnect) |
//! | `tcp:HOST:PORT`         | connect as client, retry until the server is up, reconnect on drop |
//! | `serial:DEV[@BAUD]`     | host serial port, e.g. `serial:/dev/ttyUSB0@9600`, `serial:COM3@115200` |
//! | `stdio`                 | raw stdin/stdout (stdin in raw mode where possible)  |
//! | `loopback`              | everything written comes back                        |
//!
//! Details:
//!
//! * `tcp-listen` binds immediately (so errors such as "address in use" are
//!   reported by [`open`]). Only one client is served at a time; further
//!   clients wait in the listen backlog until the current one disconnects.
//!   Bytes written while no client is connected are dropped. Port `0` picks
//!   an ephemeral port; the actual address is shown by [`Endpoint::describe`]
//!   and by [`TcpListenEndpoint::local_addr`]. `TCP_NODELAY` is set.
//! * `tcp` connects in the background, retrying every 500 ms, and reconnects
//!   after the connection drops. Bytes written while disconnected are dropped.
//! * `serial` defaults to 9600 baud, 8 data bits, no parity, 1 stop bit, no
//!   flow control. [`Endpoint::send_break`] asserts BREAK for about 250 ms.
//! * `stdio` puts the terminal into raw mode on Unix when stdin is a tty (no
//!   echo, no line editing, no signal keys - Ctrl-C is delivered as byte 0x03)
//!   and restores it when the endpoint is dropped. No byte translation is done
//!   in either direction.

use std::io;

pub mod sysconf;
mod mem;
mod serial;
mod stdio;
mod tcp;

pub use mem::{Loopback, NullEndpoint};
pub use tcp::{TcpConnectEndpoint, TcpListenEndpoint};

/// Line status reported by an endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineStatus {
    /// Remote side is connected (TCP peer present, serial port open, ...).
    pub connected: bool,
}

/// Parity of an asynchronous serial line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
}

/// Stop bits of an asynchronous serial line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBits {
    One,
    OnePointFive,
    Two,
}

/// Character format and speed of an asynchronous serial line, as programmed
/// by the emulated machine (see [`Endpoint::set_line_params`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineParams {
    /// Bits per second (rounded; may be non-standard, e.g. 20833).
    pub baud: u32,
    /// Data bits, 5..=8.
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: StopBits,
}

impl std::fmt::Display for LineParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let p = match self.parity {
            Parity::None => 'N',
            Parity::Odd => 'O',
            Parity::Even => 'E',
        };
        let s = match self.stop_bits {
            StopBits::One => "1",
            StopBits::OnePointFive => "1.5",
            StopBits::Two => "2",
        };
        write!(f, "{} {}{}{}", self.baud, self.data_bits, p, s)
    }
}

/// A byte-stream endpoint of a virtual serial line.
pub trait Endpoint: Send {
    /// Queue one byte for transmission. Never blocks.
    fn write(&mut self, byte: u8);
    /// Take the next received byte, if any. Never blocks.
    fn read(&mut self) -> Option<u8>;
    /// Send a BREAK condition (best effort; may be a no-op).
    fn send_break(&mut self) {}
    /// The emulated chip changed the line speed / character format. Endpoints
    /// backed by a real serial port reprogram it (after bytes already queued
    /// have been sent); all others ignore it. Never blocks.
    fn set_line_params(&mut self, _params: LineParams) {}
    /// Current line status.
    fn status(&self) -> LineStatus {
        LineStatus { connected: true }
    }
    /// Human readable description, e.g. "tcp-listen 127.0.0.1:3001 (connected)".
    fn describe(&self) -> String;
}

/// Open an endpoint from a specification string (see crate docs).
pub fn open(spec: &str) -> io::Result<Box<dyn Endpoint>> {
    let spec = spec.trim();
    let (kind, arg) = match spec.split_once(':') {
        Some((k, a)) => (k, Some(a)),
        None => (spec, None),
    };
    let need_arg = |what: &str| -> io::Result<&str> {
        match arg {
            Some(a) if !a.is_empty() => Ok(a),
            _ => Err(bad_spec(spec, what)),
        }
    };
    let no_arg = || -> io::Result<()> {
        match arg {
            None => Ok(()),
            Some(_) => Err(bad_spec(spec, "takes no argument")),
        }
    };
    match kind.to_ascii_lowercase().as_str() {
        "none" | "null" => {
            no_arg()?;
            Ok(Box::new(NullEndpoint))
        }
        "loopback" => {
            no_arg()?;
            Ok(Box::new(Loopback::default()))
        }
        "stdio" => {
            no_arg()?;
            Ok(Box::new(stdio::StdioEndpoint::open()?))
        }
        "tcp-listen" => {
            let addr = need_arg("expected tcp-listen:ADDR:PORT")?;
            Ok(Box::new(TcpListenEndpoint::bind(addr)?))
        }
        "tcp" => {
            let addr = need_arg("expected tcp:HOST:PORT")?;
            Ok(Box::new(TcpConnectEndpoint::connect(addr)?))
        }
        "serial" => {
            let a = need_arg("expected serial:DEV[@BAUD]")?;
            let (dev, baud) = match a.rsplit_once('@') {
                Some((d, b)) => {
                    let baud = b
                        .parse::<u32>()
                        .ok()
                        .filter(|&b| b > 0)
                        .ok_or_else(|| bad_spec(spec, "invalid baud rate"))?;
                    (d, baud)
                }
                None => (a, 9600),
            };
            if dev.is_empty() {
                return Err(bad_spec(spec, "missing device"));
            }
            Ok(Box::new(serial::SerialEndpoint::open(dev, baud)?))
        }
        _ => Err(bad_spec(
            spec,
            "unknown endpoint type (expected none, loopback, stdio, \
             tcp-listen:ADDR:PORT, tcp:HOST:PORT or serial:DEV[@BAUD])",
        )),
    }
}

fn bad_spec(spec: &str, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("invalid link spec '{spec}': {why}"),
    )
}

/// Create two endpoints connected to each other in memory (for tests and for
/// running both emulators in one process, e.g. the future web build).
pub fn pair() -> (Box<dyn Endpoint>, Box<dyn Endpoint>) {
    let (a, b) = mem::PairEndpoint::new_pair();
    (Box::new(a), Box::new(b))
}

#[cfg(test)]
mod tests;
