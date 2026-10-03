#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
use verum_vbc::interpreter::reactor::{WaitOutcome, wait_readable, wait_writable};

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    (client, server)
}

#[test]
fn an_idle_socket_is_pending_and_the_probe_does_not_wait() {
    let (_client, server) = pair();
    let start = Instant::now();
    assert_eq!(
        wait_readable(server.as_raw_fd() as i64, Duration::ZERO),
        WaitOutcome::TimedOut
    );
    assert!(start.elapsed() < Duration::from_millis(50));
}

#[test]
fn queued_data_and_write_capacity_are_ready_on_the_first_probe() {
    let (mut client, server) = pair();
    client.write_all(b"hello").expect("write");
    assert_eq!(
        wait_readable(server.as_raw_fd() as i64, Duration::from_secs(1)),
        WaitOutcome::Ready
    );
    assert_eq!(
        wait_readable(server.as_raw_fd() as i64, Duration::ZERO),
        WaitOutcome::Ready
    );
    assert_eq!(
        wait_writable(server.as_raw_fd() as i64, Duration::ZERO),
        WaitOutcome::Ready
    );
}

#[test]
fn closed_peer_is_ready_for_eof_and_invalid_descriptors_are_errors() {
    let (client, server) = pair();
    drop(client);
    assert_eq!(
        wait_readable(server.as_raw_fd() as i64, Duration::from_secs(1)),
        WaitOutcome::Ready
    );
    assert_eq!(
        wait_readable(server.as_raw_fd() as i64, Duration::ZERO),
        WaitOutcome::Ready
    );
    assert_eq!(wait_readable(-1, Duration::ZERO), WaitOutcome::Error);
    assert_eq!(wait_readable(i64::MAX, Duration::ZERO), WaitOutcome::Error);
}

#[cfg(feature = "codegen")]
#[test]
fn the_cooperative_intrinsic_samples_readiness_with_zero_timeout() {
    use std::sync::Arc;
    use verum_fast_parser::Parser;
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    use verum_vbc::interpreter::Interpreter;
    use verum_vbc::value::Value;
    let source = r#"
@intrinsic("io_wait_readable")
fn __io_wait_readable_raw(fd: Int, timeout_ms: Int) -> Int;
fn probe(fd: Int) -> Int { __io_wait_readable_raw(fd, 0) }
"#;
    let ast = Parser::new(source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("probe"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let mut interpreter = Interpreter::new(Arc::new(module));
    let (mut client, server) = pair();
    let args = [Value::from_i64(server.as_raw_fd() as i64)];
    assert_eq!(
        interpreter
            .execute_function_with_args(entry, &args)
            .expect("idle")
            .as_i64(),
        0
    );
    client.write_all(b"hello").expect("write");
    assert_eq!(
        wait_readable(server.as_raw_fd() as i64, Duration::from_secs(1)),
        WaitOutcome::Ready
    );
    assert_eq!(
        interpreter
            .execute_function_with_args(entry, &args)
            .expect("ready")
            .as_i64(),
        1
    );
}
