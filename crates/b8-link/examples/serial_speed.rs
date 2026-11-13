//! Check which line speeds a host serial port accepts:
//! `cargo run -p b8-link --example serial_speed -- /dev/ttyUSB0 9615 20833 100000`
use b8_link::{LineParams, Parity, StopBits};
use std::{thread, time::Duration};

fn main() {
    let mut args = std::env::args().skip(1);
    let dev = args.next().expect("usage: serial_speed DEV BAUD...");
    let mut ep = b8_link::open(&format!("serial:{dev}@9600")).expect("open");
    println!("{}", ep.describe());
    for baud in args {
        let baud: u32 = baud.parse().expect("baud");
        ep.set_line_params(LineParams { baud, data_bits: 8, parity: Parity::None, stop_bits: StopBits::Two });
        thread::sleep(Duration::from_millis(200));
        println!("{}", ep.describe());
    }
}
