//! Install B:BSRV.COM (user 0) via an October 1987 ZJ ROM's DEBUG monitor.
//! No existing file is overwritten. Installer reopens and verifies the file.
use std::io::{Read, Write};
use std::time::Duration;
use serialport::SerialPort;

fn address(p: &mut dyn SerialPort, a: u16) -> std::io::Result<()> {
    p.write_all(&[0x56, a as u8, 0x58, (a >> 8) as u8])?;
    p.flush()?;
    std::thread::sleep(Duration::from_millis(10));
    Ok(())
}
fn read(p: &mut dyn SerialPort, a: u16, n: usize) -> std::io::Result<Vec<u8>> {
    address(p, a)?;
    let mut result = Vec::with_capacity(n);
    for _ in 0..n {
        p.write_all(&[0x5b])?;
        p.flush()?;
        let mut b = [0];
        p.read_exact(&mut b)?;
        result.push(b[0]);
    }
    Ok(result)
}
fn upload(p: &mut dyn SerialPort, a: u16, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    for (i, chunk) in data.chunks(128).enumerate() {
        let a = a + (i * 128) as u16;
        address(p, a)?;
        for &b in chunk {
            p.write_all(&[0x5a, b])?;
        }
        p.flush()?;
        std::thread::sleep(Duration::from_millis(20));
        if read(p, a, chunk.len())? != chunk {
            return Err(format!("RAM verification failed at {a:04X}; installer not started").into());
        }
        println!("RAM {a:04X}: {} bytes verified", chunk.len());
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dev = std::env::args().nth(1).ok_or("usage: install_bsrv DEVICE")?;
    let mut port = serialport::new(dev, 9615)
        .stop_bits(serialport::StopBits::Two)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_secs(2)).open()?;
    port.clear(serialport::ClearBuffer::Input)?;
    port.write_all(&[0x5d])?;
    port.flush()?;
    let mut hello = [0; 2];
    port.read_exact(&mut hello)?;
    if hello != [0x80, 0x88] { return Err(format!("unexpected DEBUG greeting: {hello:02X?}").into()); }
    let rom = include_bytes!("../../../roms/bosman8/bosman8-zj-19871020.bin");
    if read(port.as_mut(), 0xf4a4, 32)? != rom[0x13a4..0x13c4] {
        return Err("BIOS is not the supported ZJ 1987-10-20; no memory written".into());
    }
    println!("ZJ 1987-10-20 confirmed; installing B:BSRV.COM in user 0");
    let com = include_bytes!("../../../tools/bsrv/BSRV.COM");
    if com.len() > 640 { return Err("COM exceeds installer's five records".into()); }
    let mut padded = com.to_vec();
    padded.resize(640, 0);
    upload(port.as_mut(), 0xd800, include_bytes!("../../../tools/bsrv/install.bin"))?;
    upload(port.as_mut(), 0xe000, &padded)?;
    // October monitor's saved JP operand; command 59 restores registers and runs it.
    upload(port.as_mut(), 0xfbf2, &0xd800u16.to_le_bytes())?;
    println!("Starting BDOS installer; waiting for disk write and readback verification");
    port.write_all(&[0x59])?;
    port.flush()?;
    port.set_timeout(Duration::from_secs(30))?;
    let mut reply = [0; 7];
    port.read_exact(&mut reply)?;
    println!("installer reply: {}", String::from_utf8_lossy(&reply));
    if reply != *b"B8I:K\r\n" {
        return Err(format!("installer did not confirm success: {reply:02X?} (E=exists, C=create/close, W=write, O=open, R=read, V=compare)").into());
    }
    println!("B:BSRV.COM installed, closed, reopened and verified byte for byte (640 bytes).");
    println!("Bosman returns to CCP. Start with B:BSRV.");
    Ok(())
}
