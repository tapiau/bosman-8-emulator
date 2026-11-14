//! Exercise the exact DEBUG upload/GO and BDOS installer used on hardware.
use b8_link::Endpoint;
use bosman8::{Bosman8, Config, CPU_HZ};

fn send(m: &mut Bosman8, link: &mut dyn Endpoint, bytes: &[u8]) {
    for &b in bytes { link.write(b); }
    m.run_cycles((bytes.len() as u64 + 10) * CPU_HZ as u64 / 500);
}
fn upload(m: &mut Bosman8, link: &mut dyn Endpoint, addr: u16, data: &[u8]) {
    send(m, link, &[0x56, addr as u8, 0x58, (addr >> 8) as u8]);
    for &b in data { send(m, link, &[0x5a, b]); }
    for (i, &b) in data.iter().enumerate() {
        assert_eq!(m.peek(addr + i as u16), b);
    }
}

#[test]
fn debug_installer_creates_verifies_and_refuses_overwrite() {
    let (term, mut console) = b8_link::pair();
    let (aux, mut host) = b8_link::pair();
    let mut m = Bosman8::new(include_bytes!("../../../roms/bosman8/bosman8-zj-19871020.bin"),
        Config { fast_fdc: true, ..Config::default() }, term, aux);
    let fmt = bosman8::disk::F200;
    let disk = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../disks/narzedzia.img")).unwrap();
    m.insert_disk(0, Some(fmt.from_bytes(disk)));
    m.run_cycles(CPU_HZ as u64 * 6);
    while console.read().is_some() {}
    for &b in b"DEBUG\r" { console.write(b); }
    m.run_cycles(CPU_HZ as u64);
    while host.read().is_some() {}
    let com = include_bytes!("../../../tools/bsrv/BSRV.COM");
    let mut padded = com.to_vec();
    padded.resize(640, 0);
    assert!(com.len() <= padded.len());
    for expected in [b'K', b'E'] {
        upload(&mut m, host.as_mut(), 0xd800, include_bytes!("../../../tools/bsrv/install.bin"));
        upload(&mut m, host.as_mut(), 0xe000, &padded);
        upload(&mut m, host.as_mut(), 0xfbf2, &0xd800u16.to_le_bytes());
        send(&mut m, host.as_mut(), &[0x59]);
        m.run_cycles(CPU_HZ as u64 * 5);
        let mut reply = Vec::new();
        while let Some(b) = host.read() { reply.push(b); }
        assert!(reply.windows(5).any(|w| w == [b'B',b'8',b'I',b':',expected]),
            "installer status {expected}: {reply:02X?}, PC={:04X}", m.pc());
        let mut disk = m.disk(0).unwrap().data.clone();
        let fs = bosman8::cpmfs::CpmFs::new(&mut disk, bosman8::cpmfs::Params::for_format(fmt)).unwrap();
        assert_eq!(fs.get(0, "BSRV.COM").unwrap(), padded);
        for &b in b"DEBUG\r" { console.write(b); }
        m.run_cycles(CPU_HZ as u64);
        while host.read().is_some() {}
    }
}
