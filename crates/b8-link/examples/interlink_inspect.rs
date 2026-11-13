//! Read-only inspection of a real Bosman waiting in DEBUG at 9615 8N2.
//! Optional second argument probes a disk-server header at the given baud.
//! Does not write remote memory or disk sectors.
use std::io::{Read, Write};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dev = std::env::args().nth(1).ok_or("usage: interlink_inspect DEVICE [DISK_BAUD] [--active]")?;
    let active = std::env::args().any(|a| a == "--active");
    let mut port = serialport::new(dev, 9615)
        .stop_bits(serialport::StopBits::Two)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(500))
        .open()?;
    port.clear(serialport::ClearBuffer::Input)?;
    if !active {
        port.write_all(&[0x5d])?;
        port.flush()?;
        let mut hello = [0; 2];
        port.read_exact(&mut hello)?;
        println!("hello: {:02X?}", hello);
        if hello[1] != 0x88 {
            return Err("unexpected DEBUG greeting; stopping".into());
        }
        // 56/58 set the monitor address, 5B reads and increments it.
        // Read V.24 configuration, drive tables and identification in common RAM.
        // Monitor memory reads below 8000 expose RAM underneath ROM.
        for (address, count) in [(0xf360u16, 16), (0xf2ab, 60), (0xf4a4, 32)] {
            for b in [0x56, address as u8, 0x58, (address >> 8) as u8] {
                port.write_all(&[b])?;
                port.flush()?;
                std::thread::sleep(Duration::from_millis(3));
            }
            let mut bytes = Vec::new();
            for _ in 0..count {
                port.write_all(&[0x5b])?;
                port.flush()?;
                let mut b = [0];
                port.read_exact(&mut b)?;
                bytes.push(b[0]);
            }
            if address == 0xf4a4 {
                let version = match bytes.as_slice() {
                    [0xc0, 0xd3, 0x0e, 0xc9, 0x3a, 0x6b, 0xf2, 0xcb,
                     0x57, 0xc8, 0xcd, 0x8f, 0xf3, 0xc5, 0xcd, 0x12,
                     0x0c, 0xc1, 0xc3, 0x7e, 0xf3, 0xcd, 0x8f, 0xf3,
                     0xcd, 0xdd, 0x0b, 0xc3, 0x7e, 0xf3, 0xcd, 0x8f] =>
                        "UNIMOR ZJCPMR 2311, 1987-10-20 (no DEBUG command 60)",
                    [0xc0, 0xd3, 0x0e, 0xc9, 0x3a, 0x6b, 0xf2, 0xcb,
                     0x57, 0xc8, 0xcd, 0x8f, 0xf3, 0xc5, 0xcd, 0x88,
                     0x0b, 0xc1, 0xc3, 0x7e, 0xf3, 0xdb, 0x83, 0xe6,
                     0x01, 0x18, 0xa9, 0xdb, 0x83, 0xe6, 0x01, 0x28] =>
                        "UNIMOR ZJ CPMR, 1987-03-24 (no DEBUG command 60)",
                    _ => "unknown BIOS signature",
                };
                println!("BIOS: {version}");
            }
            for (i, row) in bytes.chunks(16).enumerate() {
                print!("{:04X}:", address + (i * 16) as u16);
                for b in row {
                    print!(" {b:02X}");
                }
                println!();
            }
        }
    }
    if let Some(baud) = std::env::args().nth(2) {
        let baud: u32 = baud.parse()?;
        println!("disk server probe at {baud} bit/s (read-only header)");
        if !active {
            for b in [0x60, 0x00] {
                port.write_all(&[b])?;
                port.flush()?;
                std::thread::sleep(Duration::from_millis(3));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if std::env::args().any(|a| a == "--finish-enter") {
            port.write_all(&[0x00])?;
            port.flush()?;
            std::thread::sleep(Duration::from_millis(30));
        }
        port.set_baud_rate(baud)?;
        std::thread::sleep(Duration::from_millis(50));
        for b in [0xf1, 0xff, 0x00, 0x00, 0x00, 0x10] {
            port.write_all(&[b])?;
            port.flush()?;
            std::thread::sleep(Duration::from_millis(3));
        }
        let mut reply = [0];
        let result = port.read_exact(&mut reply);
        println!("header response: {result:?}, {:02X?}", reply);
        if result.is_err() {
            return Err("no header response; no release command sent".into());
        }
        if !matches!(reply[0], 0x00 | 0x5a) {
            return Err("unexpected header response; no release command sent".into());
        }
        if std::env::args().any(|a| a == "--keep") {
            return Ok(());
        }
        // Release the server without issuing READ or WRITE.
        port.write_all(&[0xf0])?;
        port.flush()?;
        std::thread::sleep(Duration::from_millis(20));
        port.set_baud_rate(9615)?;
        port.write_all(&[0x00])?;
        port.flush()?;
    }
    Ok(())
}
