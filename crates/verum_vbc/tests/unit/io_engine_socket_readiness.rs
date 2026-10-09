//! Bounded host fixtures for real IoEngine readiness and byte transfer (T1650).
use super::super::dispatch_table::handlers::net_runtime::{
    tcp_close, tcp_listen_v2, tcp_local_port, tcp_peer_addr,
};
use super::*;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

const FIXTURE_DEADLINE: Duration = Duration::from_secs(2);

struct TestEngine(i64);
impl TestEngine {
    fn new() -> Self {
        let handle = engine_new(8);
        assert!(handle > 0, "create IoEngine: {handle}");
        Self(handle)
    }
}
impl Drop for TestEngine {
    fn drop(&mut self) {
        engine_destroy(self.0);
    }
}

struct RuntimeSocket(i64);
impl RuntimeSocket {
    fn checked(handle: i64, operation: &str) -> Self {
        assert!(handle > 0, "{operation}: {handle}");
        Self(handle)
    }
}
impl Drop for RuntimeSocket {
    fn drop(&mut self) {
        tcp_close(self.0);
    }
}

fn connect(address: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect_timeout(&address, FIXTURE_DEADLINE)
        .expect("fixture TCP connection must complete before readiness is tested");
    stream.set_read_timeout(Some(FIXTURE_DEADLINE)).unwrap();
    stream.set_write_timeout(Some(FIXTURE_DEADLINE)).unwrap();
    stream
}

fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let peer = connect(listener.local_addr().unwrap());
    let deadline = Instant::now() + FIXTURE_DEADLINE;
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "fixture accept deadline expired");
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("fixture accept: {error}"),
        }
    };
    (peer, stream)
}

#[test]
fn poll_signals_ready_on_pending_connection() {
    let engine = TestEngine::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let fd = listener.as_raw_fd() as i64;
    assert_eq!(submit(engine.0, fd, FLAG_READ as i64), 0);
    // Keep the established peer alive through observation; a failed connect
    // is a fixture failure, not a missing readiness event.
    let _peer = connect(listener.local_addr().unwrap());
    let n = poll(engine.0, 16, 1_000_000_000);
    assert!(
        n > 0,
        "established pending connection must be readable, got {n}"
    );
    assert_eq!(is_ready(engine.0, fd, FLAG_READ as i64), 1);
    assert_eq!(take_ready(engine.0, fd, FLAG_READ as i64), 1);
    assert_eq!(is_ready(engine.0, fd, FLAG_READ as i64), 0);
    assert_eq!(remove(engine.0, fd), 0);
}

#[test]
fn async_accept_round_trip_via_io_engine() {
    let engine = TestEngine::new();
    let listener = RuntimeSocket::checked(tcp_listen_v2("127.0.0.1", 0, 8, 0), "listen");
    let port = tcp_local_port(listener.0);
    assert!((1..=65535).contains(&port), "listener port: {port}");
    let peer = connect(SocketAddr::from(([127, 0, 0, 1], port as u16)));
    let accepted = RuntimeSocket::checked(
        async_accept(engine.0, listener.0, 1_500_000_000),
        "async accept",
    );
    let peer_endpoint = peer.local_addr().unwrap();
    let (family, host, peer_port) =
        tcp_peer_addr(accepted.0).expect("accepted socket must report its connected peer");
    assert_eq!(family, 4);
    assert_eq!(host, "127.0.0.1");
    assert_eq!(peer_port, peer_endpoint.port() as i64);
    assert_eq!(tcp_local_port(accepted.0), port);
}

#[test]
fn async_read_end_to_end_via_io_engine() {
    let engine = TestEngine::new();
    let (mut peer, stream) = connected_pair();
    peer.write_all(b"hello, world!")
        .expect("fixture write deadline");
    let mut buffer = [0u8; 32];
    let n = async_read(
        engine.0,
        stream.as_raw_fd() as i64,
        buffer.as_mut_ptr() as i64,
        buffer.len() as i64,
        1_000_000_000,
    );
    assert_eq!(n, 13, "expected 13 bytes, got {n}");
    assert_eq!(&buffer[..13], b"hello, world!");
    // The std stream owns the descriptor through the engine operation.
}

#[test]
fn async_write_end_to_end_via_io_engine() {
    let engine = TestEngine::new();
    let (mut peer, stream) = connected_pair();
    let payload = b"VERUM!\n";
    let n = async_write(
        engine.0,
        stream.as_raw_fd() as i64,
        payload.as_ptr() as i64,
        payload.len() as i64,
        1_000_000_000,
    );
    assert_eq!(n, 7, "expected 7 bytes written, got {n}");
    let mut got = [0u8; 7];
    peer.read_exact(&mut got).expect("fixture read deadline");
    assert_eq!(&got, payload);
}
