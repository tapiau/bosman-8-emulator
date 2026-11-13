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

use std::io;

/// Line status reported by an endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineStatus {
    /// Remote side is connected (TCP peer present, serial port open, ...).
    pub connected: bool,
}

/// A byte-stream endpoint of a virtual serial line.
pub trait Endpoint: Send {
    /// Queue one byte for transmission. Never blocks.
    fn write(&mut self, byte: u8);
    /// Take the next received byte, if any. Never blocks.
    fn read(&mut self) -> Option<u8>;
    /// Send a BREAK condition (best effort; may be a no-op).
    fn send_break(&mut self) {}
    /// Current line status.
    fn status(&self) -> LineStatus {
        LineStatus { connected: true }
    }
    /// Human readable description, e.g. "tcp-listen 127.0.0.1:3001 (connected)".
    fn describe(&self) -> String;
}

/// Open an endpoint from a specification string (see crate docs).
pub fn open(spec: &str) -> io::Result<Box<dyn Endpoint>> {
    let _ = spec;
    todo!("implemented in T1.2")
}

/// Create two endpoints connected to each other in memory (for tests and for
/// running both emulators in one process, e.g. the future web build).
pub fn pair() -> (Box<dyn Endpoint>, Box<dyn Endpoint>) {
    todo!("implemented in T1.2")
}
