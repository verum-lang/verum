//! Bounded, owned peer setup for actual reactor readiness checks (T1650).
use super::{wait_readable, wait_writable, WaitOutcome};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use verum_common::Text;

const FIXTURE_DEADLINE: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(1);
const ARRIVAL_DELAY: Duration = Duration::from_millis(50);

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

struct ConnectingPeer {
    // A completed worker's return value keeps its successful peer socket alive
    // inside JoinHandle until the main thread takes ownership after the wait.
    worker: Option<JoinHandle<std::io::Result<TcpStream>>>,
}

impl ConnectingPeer {
    fn start(address: SocketAddr) -> Self {
        Self {
            worker: Some(thread::spawn(move || {
                // Preserve the arrival-style fixture's scheduling. This delay
                // does not establish registration-before-arrival ordering.
                thread::sleep(ARRIVAL_DELAY);
                TcpStream::connect_timeout(&address, FIXTURE_DEADLINE)
            })),
        }
    }

    fn join_until(&mut self, deadline: Instant) -> Result<std::io::Result<TcpStream>, Text> {
        while !self.worker.as_ref().unwrap().is_finished() {
            if Instant::now() >= deadline {
                return Err("reactor fixture connector cleanup deadline".into());
            }
            thread::sleep(POLL_INTERVAL);
        }
        // Only a finished worker is joined, so join cannot wait for connect.
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| "reactor fixture connector panicked before setup completed".into())
    }

    fn finish(mut self) -> std::io::Result<TcpStream> {
        self.join_until(Instant::now() + FIXTURE_DEADLINE + ARRIVAL_DELAY)
            .expect("bounded reactor connector must finish before its result is asserted")
    }
}

impl Drop for ConnectingPeer {
    fn drop(&mut self) {
        if self.worker.is_none() {
            return;
        }
        // Cover early wait failures as well as the normal result path. A peer
        // returned during unwinding is dropped here after its worker is joined.
        if let Err(error) = self.join_until(Instant::now() + FIXTURE_DEADLINE + ARRIVAL_DELAY) {
            if thread::panicking() {
                eprintln!("{error}");
            } else {
                panic!("{error}");
            }
        }
    }
}

#[test]
fn ready_signalled_when_connection_arrives() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind reactor fixture listener");
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let connector = ConnectingPeer::start(address);
    let result = wait_readable(i64::from(listener.as_raw_fd()), FIXTURE_DEADLINE);
    // Complete and join the bounded setup before classifying a reactor result.
    // The peer stays owned through this assertion even if the worker finished
    // earlier; a failed connect never becomes a missing-readiness verdict.
    let peer = connector
        .finish()
        .expect("reactor fixture connect failed; readable readiness is inconclusive");
    assert_eq!(peer.peer_addr().unwrap(), address);
    assert_eq!(result, WaitOutcome::Ready);
    drop(peer);
}
