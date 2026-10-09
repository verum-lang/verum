//! Real TCP fixtures with deadlines and owned failure cleanup (T1650).
use super::*;
use std::net::Shutdown;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Instant;
use verum_common::Text;

const IO_DEADLINE: Duration = Duration::from_secs(2);
const PARK_DEADLINE: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

fn with_resource<T>(fd: i64, operation: impl FnOnce(Option<&NetResource>) -> T) -> T {
    let deadline = Instant::now() + IO_DEADLINE;
    loop {
        match REGISTRY.try_lock() {
            Ok(guard) => return operation(guard.get(&fd)),
            Err(std::sync::TryLockError::Poisoned(error)) => {
                panic!("fixture registry poisoned: {error}")
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                assert!(Instant::now() < deadline, "fixture registry lock deadline");
                thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

struct Listener {
    fd: i64,
    // On Unix this owns the raw v2 descriptor. Elsewhere it owns a clone of
    // the registered listener. In both cases accept uses the runtime API.
    socket: TcpListener,
}

impl Listener {
    fn new() -> Self {
        let fd = tcp_listen_v2("127.0.0.1", 0, 8, TCP_LISTEN_FLAG_REUSEPORT);
        assert!(fd > 0, "tcp_listen_v2: {fd}");
        #[cfg(unix)]
        let socket = {
            use std::os::fd::FromRawFd;
            // SAFETY: tcp_listen_v2 transferred this fresh raw descriptor to
            // the fixture. This wrapper is its sole owner until Drop.
            unsafe { TcpListener::from_raw_fd(fd as i32) }
        };
        #[cfg(not(unix))]
        let socket = with_resource(fd, |resource| match resource {
            Some(NetResource::Listener(listener)) => listener.try_clone().unwrap(),
            _ => panic!("v2 listener {fd} was not registered"),
        });
        let listener = Self { fd, socket };
        listener.socket.set_nonblocking(true).unwrap();
        assert_eq!(tcp_local_port(fd), listener.port());
        listener
    }

    fn port(&self) -> i64 {
        i64::from(self.socket.local_addr().unwrap().port())
    }

    fn accept(&self, timeout: Duration) -> Result<Stream, Text> {
        let deadline = Instant::now() + timeout;
        loop {
            // Refuse the pre-existing raw/synthetic namespace collision;
            // do not let a fixture consume a different registered resource.
            #[cfg(unix)]
            assert!(
                with_resource(self.fd, |resource| resource.is_none()),
                "raw listener {} aliases a synthetic socket",
                self.fd
            );
            let fd = tcp_accept(self.fd);
            if fd > 0 {
                return Ok(Stream::new(fd, IO_DEADLINE));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "tcp_accept on 127.0.0.1:{} exceeded {timeout:?} (last result {fd})",
                    self.port()
                )
                .into());
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // The Unix raw descriptor is closed by socket's Drop, even if a
        // runtime registry collision or an assertion interrupted the test.
        #[cfg(not(unix))]
        let _ = tcp_close(self.fd);
    }
}

struct Stream {
    fd: i64,
    socket: TcpStream,
}

impl Stream {
    fn new(fd: i64, timeout: Duration) -> Self {
        assert!(fd > 0, "expected connected runtime stream, got {fd}");
        let socket = with_resource(fd, |resource| match resource {
            Some(NetResource::Stream(stream)) => stream.try_clone(),
            _ => Err(std::io::Error::other(format!(
                "runtime stream {fd} was not registered"
            ))),
        });
        let socket = match socket {
            Ok(socket) => socket,
            Err(error) => {
                tcp_close(fd);
                panic!("clone runtime stream {fd}: {error}");
            }
        };
        let stream = Self { fd, socket };
        // Accepted sockets can inherit the listener's nonblocking mode.
        // Test the blocking production send/recv with kernel deadlines.
        stream.socket.set_nonblocking(false).unwrap();
        stream.socket.set_read_timeout(Some(timeout)).unwrap();
        stream.socket.set_write_timeout(Some(IO_DEADLINE)).unwrap();
        stream
    }

    fn connect(listener: &Listener) -> Self {
        let fd = tcp_connect_timeout("127.0.0.1", listener.port(), IO_DEADLINE.as_millis() as i64);
        assert!(
            fd > 0,
            "tcp_connect_timeout to 127.0.0.1:{}: {fd}",
            listener.port()
        );
        Self::new(fd, IO_DEADLINE)
    }

    fn send(&self, bytes: &[u8]) {
        assert_eq!(
            tcp_send(self.fd, bytes),
            bytes.len() as i64,
            "tcp_send on {}",
            self.fd
        );
    }

    fn receive(&self, expected: &str) {
        let actual = tcp_recv(self.fd, 64).expect("tcp_recv failed or exceeded socket deadline");
        assert_eq!(actual, expected, "tcp_recv on {}", self.fd);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // Shutdown also releases any blocking I/O on cloned descriptors.
        let _ = self.socket.shutdown(Shutdown::Both);
        let result = tcp_close(self.fd);
        if !thread::panicking() {
            assert_eq!(result, 0, "close owned runtime stream {}", self.fd);
        }
    }
}

struct ParkedReceive {
    thread: Option<JoinHandle<()>>,
    result: Receiver<Option<Text>>,
    cancel: TcpStream,
}

impl ParkedReceive {
    fn start(stream: &Stream) -> Self {
        stream.socket.set_read_timeout(Some(PARK_DEADLINE)).unwrap();
        let cancel = stream.socket.try_clone().unwrap();
        let fd = stream.fd;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (result_tx, result) = mpsc::channel();
        let thread = thread::spawn(move || {
            entered_tx.send(()).unwrap();
            let value = tcp_recv(fd, 64).map(Text::from);
            let _ = result_tx.send(value);
        });
        let worker = Self {
            thread: Some(thread),
            result,
            cancel,
        };
        entered_rx
            .recv_timeout(IO_DEADLINE)
            .expect("parked worker did not start");
        assert!(
            matches!(
                worker.result.recv_timeout(Duration::from_millis(50)),
                Err(RecvTimeoutError::Timeout)
            ),
            "silent peer must leave the real tcp_recv pending"
        );
        worker
    }

    fn finish(mut self, expected: &str) {
        let deadline = Instant::now() + IO_DEADLINE;
        let value = self
            .result
            .recv_timeout(IO_DEADLINE)
            .expect("parked tcp_recv did not finish before deadline");
        assert_eq!(value.as_deref(), Some(expected));
        assert!(self.join_until(deadline), "parked worker join deadline");
    }

    fn join_until(&mut self, deadline: Instant) -> bool {
        while self
            .thread
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(POLL_INTERVAL);
        }
        if let Some(worker) = self.thread.take() {
            // is_finished established that join cannot wait for socket I/O.
            let result = worker.join();
            if !thread::panicking() {
                assert!(result.is_ok(), "parked worker panicked");
            }
        }
        true
    }
}

impl Drop for ParkedReceive {
    fn drop(&mut self) {
        let _ = self.cancel.shutdown(Shutdown::Both);
        if !self.join_until(Instant::now() + IO_DEADLINE) {
            if thread::panicking() {
                eprintln!("parked worker did not stop after shutdown within {IO_DEADLINE:?}");
            } else {
                panic!("parked worker cleanup deadline after socket shutdown");
            }
        }
    }
}

#[test]
fn tcp_listen_accept_send_recv_round_trip() {
    let listener = Listener::new();
    let client = Stream::connect(&listener);
    let server = listener.accept(IO_DEADLINE).unwrap();
    assert_eq!(tcp_local_port(server.fd), listener.port());
    assert_eq!(
        server.socket.peer_addr().unwrap(),
        client.socket.local_addr().unwrap()
    );
    client.send(b"hello");
    server.receive("hello");
    server.send(b"world");
    client.receive("world");
}

#[test]
fn concurrent_recv_does_not_block_unrelated_send() {
    let parker_listener = Listener::new();
    let parker_client = Stream::connect(&parker_listener);
    let parker_server = parker_listener.accept(IO_DEADLINE).unwrap();
    let echo_listener = Listener::new();
    let echo_client = Stream::connect(&echo_listener);
    let echo_server = echo_listener.accept(IO_DEADLINE).unwrap();
    let parker = ParkedReceive::start(&parker_client);

    let started = Instant::now();
    echo_client.send(b"ping");
    echo_server.receive("ping");
    echo_server.send(b"pong");
    echo_client.receive("pong");
    assert!(
        started.elapsed() < IO_DEADLINE,
        "unrelated round-trip exceeded deadline; REGISTRY lock contention?"
    );
    assert!(
        matches!(parker.result.try_recv(), Err(TryRecvError::Empty)),
        "parker must still be waiting when unrelated I/O completes"
    );

    parker_server.send(b"release");
    parker.finish("release");
}

#[test]
fn silent_listener_reports_accept_deadline() {
    let listener = Listener::new();
    let started = Instant::now();
    let error = listener
        .accept(Duration::from_millis(50))
        .err()
        .expect("listener without a peer must time out");
    assert!(error.contains("tcp_accept"), "{error}");
    assert!(
        started.elapsed() < IO_DEADLINE,
        "fixture accept was not bounded"
    );
}

#[test]
fn silent_stream_reports_recv_deadline_and_cleans_registry() {
    let listener = Listener::new();
    let client = Stream::connect(&listener);
    let server = listener.accept(IO_DEADLINE).unwrap();
    client
        .socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let started = Instant::now();
    assert!(
        tcp_recv(client.fd, 64).is_none(),
        "silent recv must report its socket deadline"
    );
    assert!(
        started.elapsed() < IO_DEADLINE,
        "fixture recv was not bounded"
    );
    let handles = [client.fd, server.fd];
    drop(client);
    drop(server);
    for handle in handles {
        assert!(
            with_resource(handle, |resource| resource.is_none()),
            "runtime stream {handle} leaked"
        );
    }
}

#[test]
fn parked_worker_is_joined_and_sockets_closed_during_unwind() {
    let listener = Listener::new();
    let client = Stream::connect(&listener);
    let server = listener.accept(IO_DEADLINE).unwrap();
    let handles = [client.fd, server.fd];
    let started = Instant::now();
    let result = std::panic::catch_unwind(move || {
        let _client = client;
        let _server = server;
        let _worker = ParkedReceive::start(&_client);
        panic!("injected fixture failure after parked recv");
    });
    let panic = result.expect_err("expected injected fixture failure");
    assert_eq!(
        panic.downcast_ref::<&str>().copied(),
        Some("injected fixture failure after parked recv"),
        "fixture setup failed before the intended unwind control"
    );
    assert!(
        started.elapsed() < IO_DEADLINE,
        "failure cleanup did not finish before deadline"
    );
    for handle in handles {
        assert!(
            with_resource(handle, |resource| resource.is_none()),
            "failed fixture leaked runtime stream {handle}"
        );
    }
}
