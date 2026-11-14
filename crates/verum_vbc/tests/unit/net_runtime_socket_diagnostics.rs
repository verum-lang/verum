//! Failure-only observations for the existing socket controls (T1725).
//! No observer retries I/O, consumes queued connections, clears SO_ERROR,
//! changes socket options, or extends the operation's original deadline.
use std::fmt::Arguments;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};
use verum_common::{Maybe, Text};

fn error_detail(error: &io::Error) -> Text {
    format!(
        "kind={:?} raw_os_error={:?} message={error}",
        error.kind(),
        error.raw_os_error()
    )
    .into()
}

pub(crate) fn io_failure(
    operation: &str,
    details: Arguments<'_>,
    error: &io::Error,
    elapsed: Maybe<Duration>,
) {
    eprintln!(
        "[socket-diagnostic] operation={operation} {details} elapsed={elapsed:?} {}",
        error_detail(error)
    );
}

/// Observe the original direct-std connect result without retrying or replacing it.
pub(crate) fn connect_timeout(
    operation: &str,
    address: &SocketAddr,
    timeout: Duration,
) -> io::Result<TcpStream> {
    let started = Instant::now();
    let result = TcpStream::connect_timeout(address, timeout);
    if let Err(error) = &result {
        io_failure(
            operation,
            format_args!("destination={address} budget={timeout:?}"),
            error,
            Some(started.elapsed()),
        );
    }
    result
}

/// Snapshot only the still-owned listener after the actual connect failed.
/// Poll has a zero timeout and does not accept a connection. In particular,
/// querying SO_ERROR would clear a pending error, so it is never used here.
pub(crate) fn listener_snapshot(listener: &TcpListener) -> Text {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        format!(
            "local={:?} {}",
            listener.local_addr(),
            socket_snapshot(listener.as_fd())
        )
        .into()
    }
    #[cfg(not(unix))]
    {
        format!(
            "local={:?} native_descriptor_snapshot=unsupported",
            listener.local_addr()
        )
        .into()
    }
}

/// The borrow comes from the existing fixture owner; this creates no new owner.
#[cfg(unix)]
pub(crate) fn socket_snapshot(socket: std::os::fd::BorrowedFd<'_>) -> Text {
    use std::os::fd::AsRawFd;
    let fd = socket.as_raw_fd();
    format!(
        "fd={fd} descriptor_flags={:?} file_flags={:?} accepting={:?} reuse_port={:?} read_poll={:?}",
        descriptor_flags(fd, libc::F_GETFD),
        descriptor_flags(fd, libc::F_GETFL),
        socket_option(fd, libc::SO_ACCEPTCONN),
        socket_option(fd, libc::SO_REUSEPORT),
        read_poll(fd)
    )
    .into()
}

#[cfg(unix)]
fn descriptor_flags(fd: libc::c_int, command: libc::c_int) -> io::Result<libc::c_int> {
    // SAFETY: callers supply only F_GETFD/F_GETFL, read-only flag queries.
    let flags = unsafe { libc::fcntl(fd, command) };
    if flags < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(flags)
    }
}

#[cfg(unix)]
fn socket_option(fd: libc::c_int, option: libc::c_int) -> io::Result<libc::c_int> {
    let mut value: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&value) as libc::socklen_t;
    // SAFETY: both output pointers name live storage of the supplied size.
    // Callers query only SO_ACCEPTCONN/SO_REUSEPORT; neither clears errors.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&mut value as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(value)
    }
}

#[cfg(unix)]
fn read_poll(fd: libc::c_int) -> io::Result<(libc::c_int, libc::c_short)> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one live pollfd; timeout zero observes readiness without waiting.
    let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((result, descriptor.revents))
    }
}
