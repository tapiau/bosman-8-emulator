use super::*;
use std::time::{Duration, Instant};

fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn read_n(ep: &mut dyn Endpoint, n: usize) -> Vec<u8> {
    let mut got = Vec::new();
    wait_until(|| {
        while let Some(b) = ep.read() {
            got.push(b);
        }
        got.len() >= n
    });
    got
}

#[test]
fn none_endpoint() {
    let mut ep = open("none").unwrap();
    ep.write(1);
    assert_eq!(ep.read(), None);
    assert!(!ep.status().connected);
    assert_eq!(ep.describe(), "none");
}

#[test]
fn loopback_echoes_in_order() {
    let mut ep = open("loopback").unwrap();
    assert_eq!(ep.read(), None);
    for b in [1u8, 2, 3, 0xff] {
        ep.write(b);
    }
    assert_eq!(read_n(ep.as_mut(), 4), vec![1, 2, 3, 0xff]);
    assert_eq!(ep.read(), None);
}

#[test]
fn pair_is_crossed() {
    let (mut a, mut b) = pair();
    a.write(b'x');
    a.write(b'y');
    b.write(b'z');
    assert_eq!(b.read(), Some(b'x'));
    assert_eq!(b.read(), Some(b'y'));
    assert_eq!(b.read(), None);
    assert_eq!(a.read(), Some(b'z'));
    assert_eq!(a.read(), None);
    assert!(a.status().connected);
    drop(b);
    assert!(!a.status().connected);
}

#[test]
fn bad_specs_rejected() {
    let bad = [
        "",
        "bogus",
        "tcp",
        "tcp:nohost",
        "tcp:host:notaport",
        "tcp-listen",
        "serial:",
        "serial:/dev/x@fast",
        "loopback:1",
    ];
    for spec in bad {
        assert!(open(spec).is_err(), "spec {spec:?} should be rejected");
    }
}

#[test]
fn tcp_listen_connect_round_trip() {
    let server = TcpListenEndpoint::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().port();
    assert_ne!(port, 0);
    assert!(server.describe().contains(&format!("127.0.0.1:{port}")));
    let mut server: Box<dyn Endpoint> = Box::new(server);
    assert!(!server.status().connected);
    // Dropped: nobody connected yet.
    server.write(0xEE);

    let mut client = open(&format!("tcp:127.0.0.1:{port}")).unwrap();
    assert!(wait_until(|| server.status().connected && client.status().connected));

    for b in b"hello" {
        client.write(*b);
    }
    assert_eq!(read_n(server.as_mut(), 5), b"hello");
    for b in 0u8..=255 {
        server.write(b);
    }
    assert_eq!(read_n(client.as_mut(), 256), (0u8..=255).collect::<Vec<_>>());

    // Client goes away; server notices and accepts a new one.
    drop(client);
    assert!(wait_until(|| !server.status().connected));
    let mut client2 = open(&format!("tcp:127.0.0.1:{port}")).unwrap();
    assert!(wait_until(|| server.status().connected && client2.status().connected));
    server.write(b'!');
    assert_eq!(read_n(client2.as_mut(), 1), b"!");
}

#[test]
fn tcp_client_retries_until_server_up() {
    // Find a free port, release it, start the client first.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut client = open(&format!("tcp:127.0.0.1:{port}")).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(!client.status().connected);
    let mut server = open(&format!("tcp-listen:127.0.0.1:{port}")).unwrap();
    assert!(wait_until(|| client.status().connected && server.status().connected));
    client.write(b'a');
    assert_eq!(read_n(server.as_mut(), 1), b"a");

    // Server restart: client reconnects.
    drop(server);
    assert!(wait_until(|| !client.status().connected));
    let mut server = open(&format!("tcp-listen:127.0.0.1:{port}")).unwrap();
    assert!(wait_until(|| client.status().connected && server.status().connected));
    server.write(b'b');
    assert_eq!(read_n(client.as_mut(), 1), b"b");
}

#[test]
fn pair_carries_rts_cts() {
    let (mut a, b) = pair();
    assert!(a.cts() && b.cts());
    a.set_rts(false);
    assert!(!b.cts());
    assert!(a.cts());
    a.set_rts(true);
    assert!(b.cts());
}

#[test]
fn tcp_rfc2217_flow_control_and_ff_bytes() {
    let mut srv = TcpListenEndpoint::bind("127.0.0.1:0").unwrap();
    let addr = srv.local_addr().to_string();
    let mut cli = open(&format!("tcp:{addr}")).unwrap();
    assert!(wait_until(|| srv.status().connected && cli.describe().contains("RTS/CTS")), "{}", cli.describe());
    assert!(wait_until(|| srv.describe().contains("RTS/CTS")));
    // 0xFF survives in both directions (doubled on the wire)
    for b in [0xFF, b'A', 0xFF, 0xFF, 0x00] {
        cli.write(b);
        srv.write(b);
    }
    assert_eq!(read_n(&mut srv, 5), vec![0xFF, b'A', 0xFF, 0xFF, 0x00]);
    assert_eq!(read_n(cli.as_mut(), 5), vec![0xFF, b'A', 0xFF, 0xFF, 0x00]);
    // client RTS -> server CTS
    assert!(srv.cts());
    cli.set_rts(false);
    assert!(wait_until(|| !srv.cts()));
    cli.set_rts(true);
    assert!(wait_until(|| srv.cts()));
    // server RTS -> client CTS
    srv.set_rts(false);
    assert!(wait_until(|| !cli.cts()));
    srv.set_rts(true);
    assert!(wait_until(|| cli.cts()));
    // no stray bytes from the negotiation
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(srv.read(), None);
    assert_eq!(cli.read(), None);
}

/// Read exactly `n` bytes from a plain socket (TCP may deliver them in any
/// number of pieces), then check that nothing else follows.
fn read_exact_then_quiet(s: &mut std::net::TcpStream, n: usize) -> Vec<u8> {
    use std::io::Read;
    s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut got = vec![0u8; n];
    s.read_exact(&mut got).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let mut extra = [0u8; 16];
    match s.read(&mut extra) {
        Ok(0) => {}
        Ok(k) => panic!("unexpected extra bytes {:?} after {:?}", &extra[..k], got),
        Err(e) => assert!(
            matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut),
            "{e}"
        ),
    }
    got
}

#[test]
fn tcp_server_stays_raw_for_plain_clients() {
    use std::io::Write;
    let mut srv = TcpListenEndpoint::bind("127.0.0.1:0").unwrap();
    let mut s = std::net::TcpStream::connect(srv.local_addr()).unwrap();
    s.write_all(&[b'h', 0xFF, b'i']).unwrap();
    assert_eq!(read_n(&mut srv, 3), vec![b'h', 0xFF, b'i']);
    srv.write(0xFF);
    srv.write(b'!');
    srv.set_rts(false); // nothing to send without Telnet
    assert_eq!(read_exact_then_quiet(&mut s, 2), vec![0xFF, b'!']);
    assert!(srv.cts());
    assert!(!srv.describe().contains("RTS/CTS"));
}

#[test]
fn tcp_raw_client_sends_no_offer() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let mut cli = open(&format!("tcp:{addr},raw")).unwrap();
    let (mut s, _) = l.accept().unwrap();
    assert!(wait_until(|| cli.status().connected));
    cli.write(b'x');
    assert_eq!(read_exact_then_quiet(&mut s, 1), b"x", "no Telnet offer before the data");
}

#[test]
fn serial_format_parsing() {
    assert_eq!(parse_format("8N2"), Some((8, Parity::None, StopBits::Two)));
    assert_eq!(parse_format("7e1"), Some((7, Parity::Even, StopBits::One)));
    assert_eq!(parse_format("8O1"), Some((8, Parity::Odd, StopBits::One)));
    for bad in ["", "8N", "9N1", "8X1", "8N3", "8N1x"] {
        assert_eq!(parse_format(bad), None, "{bad}");
    }
    // A bad format is reported before the device is touched.
    let e = open("serial:/nonexistent@100000,8Q2").err().unwrap();
    assert!(e.to_string().contains("invalid format"), "{e}");
}
