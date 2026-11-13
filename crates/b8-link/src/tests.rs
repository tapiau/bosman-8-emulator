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
