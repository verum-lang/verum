//! Unix raw and registered resources must share OS descriptor authority (T1708).
use super::*;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::thread;
use std::time::Instant;
use verum_common::List;

const DEADLINE: Duration = Duration::from_secs(2);

fn resource<T>(fd: i64, inspect: impl FnOnce(Option<&NetResource>) -> T) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match REGISTRY.try_lock() {
            Ok(registry) => return inspect(registry.get(&fd)),
            Err(std::sync::TryLockError::Poisoned(error)) => panic!("registry poisoned: {error}"),
            Err(std::sync::TryLockError::WouldBlock) => {
                assert!(Instant::now() < deadline, "registry lookup deadline");
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

struct RegisteredSocket(Option<i64>);

impl RegisteredSocket {
    fn udp() -> Self {
        let fd = udp_bind(0);
        assert!(fd > 0, "udp_bind: {fd}");
        Self(Some(fd))
    }

    fn listener() -> Self {
        let fd = tcp_listen(0);
        assert!(fd > 0, "tcp_listen: {fd}");
        Self(Some(fd))
    }

    fn fd(&self) -> i64 {
        self.0.unwrap()
    }

    fn actual_fd(&self) -> i64 {
        resource(self.fd(), |entry| match entry {
            Some(NetResource::Listener(listener)) => i64::from(listener.as_raw_fd()),
            Some(NetResource::Stream(stream)) => i64::from(stream.as_raw_fd()),
            Some(NetResource::Udp(socket)) => i64::from(socket.as_raw_fd()),
            None => panic!("owned registration {} disappeared", self.fd()),
        })
    }

    fn close(mut self) {
        assert_eq!(tcp_close(self.0.take().unwrap()), 0);
    }
}

impl Drop for RegisteredSocket {
    fn drop(&mut self) {
        if let Some(fd) = self.0.take() {
            // Remove only the owned registry entry. In the failing old model,
            // raw close may already have consumed it; never fall through to
            // closing the unrelated raw descriptor with the same integer.
            let deadline = Instant::now() + DEADLINE;
            loop {
                match REGISTRY.try_lock() {
                    Ok(mut registry) => {
                        registry.remove(&fd);
                        return;
                    }
                    Err(std::sync::TryLockError::Poisoned(error)) => {
                        error.into_inner().remove(&fd);
                        return;
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {
                        if Instant::now() >= deadline {
                            if thread::panicking() {
                                eprintln!("registry cleanup deadline for owned fd {fd}");
                                return;
                            }
                            panic!("registry cleanup deadline for owned fd {fd}");
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        }
    }
}

struct RawListener(Option<TcpListener>);

impl RawListener {
    fn new() -> Self {
        let fd = tcp_listen_v2("127.0.0.1", 0, 8, 0);
        assert!(fd > 0, "tcp_listen_v2: {fd}");
        // SAFETY: v2 transferred sole ownership of this new raw descriptor.
        Self(Some(unsafe { TcpListener::from_raw_fd(fd as i32) }))
    }

    fn fd(&self) -> i64 {
        i64::from(self.0.as_ref().unwrap().as_raw_fd())
    }

    fn port(&self) -> i64 {
        i64::from(self.0.as_ref().unwrap().local_addr().unwrap().port())
    }

    fn register_live_udp_peers(&self) -> List<RegisteredSocket> {
        // In an isolated process the old counter starts at one. Registering
        // through this live raw descriptor number deterministically exposes
        // its shadowing. This bound is a fixture precondition, not a new ABI.
        let count = usize::try_from(self.fd()).unwrap();
        assert!(
            count <= 128,
            "fixture descriptor exceeds bounded setup: {count}"
        );
        let mut peers = List::new();
        for _ in 0..count {
            let peer = RegisteredSocket::udp();
            assert_eq!(
                resource(peer.fd(), |entry| matches!(
                    entry,
                    Some(NetResource::Udp(_))
                )),
                true
            );
            peers.push(peer);
        }
        peers
    }
}

#[test]
fn raw_listener_endpoint_is_not_shadowed_by_registered_udp() {
    let listener = RawListener::new();
    let peers = listener.register_live_udp_peers();
    let actual = listener.port();
    let selected = tcp_local_port(listener.fd());
    // Do not infer identity from ephemeral port uniqueness: protocol namespaces
    // may choose the same port. The actual registered resource kind/descriptor
    // is a separate oracle even if endpoint numbers happen to coincide.
    assert!(resource(listener.fd(), |entry| entry.is_none()), "live raw listener {} selected a registered resource; raw port {actual}, runtime port {selected}", listener.fd());
    assert_eq!(selected, actual);
    assert!(peers.iter().all(|peer| peer.actual_fd() != listener.fd()));
}

#[test]
fn raw_listener_close_preserves_registered_udp_ownership() {
    let mut listener = RawListener::new();
    let peers = listener.register_live_udp_peers();
    let raw_fd = listener.fd();
    let colliding = resource(raw_fd, |entry| entry.is_some());
    // Before the repair, identify the exact owned registration which close
    // will wrongly consume. Refuse any unrelated concurrently created entry.
    if colliding {
        assert!(
            peers.iter().any(|peer| peer.fd() == raw_fd),
            "collision belongs to another fixture"
        );
    }
    let fd = listener.0.take().unwrap().into_raw_fd();
    assert_eq!(tcp_close(i64::from(fd)), 0);
    if colliding {
        // SAFETY: the retained old registry-first path removed an owned UDP
        // entry and did not close this raw listener. Reclaim it before the
        // failure assertion; its lifetime never depended on a reused fd probe.
        listener.0 = Some(unsafe { TcpListener::from_raw_fd(fd) });
    }
    for peer in &peers {
        assert!(
            resource(peer.fd(), |entry| matches!(
                entry,
                Some(NetResource::Udp(_))
            )),
            "closing raw listener {raw_fd} removed live UDP registration {}",
            peer.fd()
        );
    }
}

#[test]
fn registered_udp_uses_its_owned_os_descriptor() {
    let socket = RegisteredSocket::udp();
    assert_eq!(
        socket.fd(),
        socket.actual_fd(),
        "registered UDP identity must be its owned OS descriptor"
    );
    socket.close();
}

#[test]
fn registered_listener_uses_its_owned_os_descriptor() {
    let listener = RegisteredSocket::listener();
    assert_eq!(
        listener.fd(),
        listener.actual_fd(),
        "registered listener identity must be its owned OS descriptor"
    );
    assert!(tcp_local_port(listener.fd()) > 0);
    listener.close();
}

#[test]
fn registered_stream_uses_its_owned_os_descriptor() {
    // No peer or loopback traffic is needed to check registry ownership.
    // SAFETY: socket creates a fresh TCP descriptor owned by this fixture.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    assert!(fd >= 0, "create owned TCP socket");
    // SAFETY: fd is the live TCP socket just created, with no other owner.
    let stream = unsafe { TcpStream::from_raw_fd(fd) };
    let registered = RegisteredSocket(Some(register_accepted_stream(stream)));
    assert_eq!(registered.fd(), i64::from(fd));
    assert_eq!(registered.fd(), registered.actual_fd());
    registered.close();
}

fn positive_handle_with_closed_stdin(kind: &str, test_name: &str) {
    const CHILD_ROLE: &str = "VERUM_SOCKET_FD_ZERO_CHILD";
    if std::env::var(CHILD_ROLE).as_deref() == Ok(kind) {
        // The test harness is fully initialized in this isolated child before
        // fd 0 is released. Parent standard descriptors are never touched.
        // SAFETY: the child owns stdin, supplied as /dev/null by its parent.
        assert_eq!(unsafe { libc::close(0) }, 0);
        let value = match kind {
            "udp" => NetResource::Udp(UdpSocket::bind("127.0.0.1:0").unwrap()),
            "listener" => NetResource::Listener(TcpListener::bind("127.0.0.1:0").unwrap()),
            "stream" => {
                // SAFETY: socket creates a fresh owned TCP descriptor.
                let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
                assert!(fd >= 0);
                // SAFETY: fd is this child's live TCP socket and has no other owner.
                NetResource::Stream(unsafe { TcpStream::from_raw_fd(fd) })
            }
            _ => unreachable!(),
        };
        let initial_fd = match &value {
            NetResource::Listener(socket) => socket.as_raw_fd(),
            NetResource::Stream(socket) => socket.as_raw_fd(),
            NetResource::Udp(socket) => socket.as_raw_fd(),
        };
        assert_eq!(
            initial_fd, 0,
            "isolated setup must exercise descriptor zero"
        );
        let socket = RegisteredSocket(Some(register(value)));
        assert!(
            socket.fd() > 0,
            "{kind}: public success handle must stay positive, got {}",
            socket.fd()
        );
        assert_eq!(
            socket.fd(),
            socket.actual_fd(),
            "positive handle must still be an actual owned descriptor"
        );
        socket.close();
        return;
    }

    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([test_name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ROLE, kind)
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start isolated descriptor-zero control");
    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().expect("inspect isolated child status") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let cleanup = Instant::now() + DEADLINE;
            while child.try_wait().expect("reap killed child").is_none() && Instant::now() < cleanup
            {
                thread::sleep(Duration::from_millis(1));
            }
            panic!("isolated {kind} descriptor-zero control exceeded deadline");
        }
        thread::sleep(Duration::from_millis(1));
    };
    assert!(
        status.success(),
        "isolated {kind} descriptor-zero control failed: {status}"
    );
}

#[test]
fn registered_udp_keeps_positive_handle_with_closed_stdin() {
    positive_handle_with_closed_stdin(
        "udp",
        concat!(
            module_path!(),
            "::registered_udp_keeps_positive_handle_with_closed_stdin"
        ),
    );
}

#[test]
fn registered_listener_keeps_positive_handle_with_closed_stdin() {
    positive_handle_with_closed_stdin(
        "listener",
        concat!(
            module_path!(),
            "::registered_listener_keeps_positive_handle_with_closed_stdin"
        ),
    );
}

#[test]
fn registered_stream_keeps_positive_handle_with_closed_stdin() {
    positive_handle_with_closed_stdin(
        "stream",
        concat!(
            module_path!(),
            "::registered_stream_keeps_positive_handle_with_closed_stdin"
        ),
    );
}
