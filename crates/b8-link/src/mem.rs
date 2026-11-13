//! In-memory endpoints: `none`, `loopback` and [`crate::pair`].

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::{Endpoint, LineStatus};

/// Not connected: writes are dropped, nothing is ever received.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullEndpoint;

impl Endpoint for NullEndpoint {
    fn write(&mut self, _byte: u8) {}
    fn read(&mut self) -> Option<u8> {
        None
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: false }
    }
    fn describe(&self) -> String {
        "none".into()
    }
}

/// Everything written comes back on read.
#[derive(Debug, Default)]
pub struct Loopback {
    buf: VecDeque<u8>,
}

impl Endpoint for Loopback {
    fn write(&mut self, byte: u8) {
        self.buf.push_back(byte);
    }
    fn read(&mut self) -> Option<u8> {
        self.buf.pop_front()
    }
    fn describe(&self) -> String {
        "loopback".into()
    }
}

type Queue = Arc<Mutex<VecDeque<u8>>>;

/// One side of an in-memory pair.
pub(crate) struct PairEndpoint {
    rx: Queue,
    tx: Queue,
    name: &'static str,
}

impl PairEndpoint {
    pub(crate) fn new_pair() -> (PairEndpoint, PairEndpoint) {
        let ab: Queue = Arc::default();
        let ba: Queue = Arc::default();
        (
            PairEndpoint { rx: ba.clone(), tx: ab.clone(), name: "pair (A)" },
            PairEndpoint { rx: ab, tx: ba, name: "pair (B)" },
        )
    }
}

fn lock(q: &Queue) -> std::sync::MutexGuard<'_, VecDeque<u8>> {
    q.lock().unwrap_or_else(|e| e.into_inner())
}

impl Endpoint for PairEndpoint {
    fn write(&mut self, byte: u8) {
        // Only queue if the peer still exists.
        if Arc::strong_count(&self.tx) > 1 {
            lock(&self.tx).push_back(byte);
        }
    }
    fn read(&mut self) -> Option<u8> {
        lock(&self.rx).pop_front()
    }
    fn status(&self) -> LineStatus {
        LineStatus { connected: Arc::strong_count(&self.tx) > 1 }
    }
    fn describe(&self) -> String {
        self.name.into()
    }
}
