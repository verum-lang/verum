//! Bounded, owned peer setup for actual reactor readiness checks (T1650).
use super::{wait_writable, WaitOutcome};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

const FIXTURE_DEADLINE: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind reactor fixture listener");
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let client = TcpStream::connect_timeout(&address, FIXTURE_DEADLINE)
        .expect("reactor fixture connect deadline; writable readiness was not tested");
    let deadline = Instant::now() + FIXTURE_DEADLINE;
    let (server, peer) = loop {
        match listener.accept() {
            Ok(accepted) => break accepted,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "reactor fixture accept deadline; writable readiness was not tested"
                );
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(error) => panic!("reactor fixture accept failed before readiness: {error}"),
        }
    };
    assert_eq!(client.peer_addr().unwrap(), address);
    assert_eq!(peer, client.local_addr().unwrap());
    assert_eq!(server.local_addr().unwrap(), address);
    assert_eq!(server.peer_addr().unwrap(), client.local_addr().unwrap());
    (client, server)
}

#[test]
fn writable_signalled_for_fresh_socket() {
    // Both endpoints stay owned through the assertion. The fixture has no
    // peer worker to detach on setup errors or readiness assertion failures.
    let (stream, peer) = connected_pair();
    stream.set_nonblocking(true).unwrap();
    let result = wait_writable(i64::from(stream.as_raw_fd()), FIXTURE_DEADLINE);
    assert_eq!(result, WaitOutcome::Ready);
    drop(peer);
}
