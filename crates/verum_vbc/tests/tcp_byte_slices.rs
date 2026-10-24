//! Execute the actual synchronous TcpStream bodies against stream sockets.
#![cfg(all(feature = "codegen", feature = "ffi", unix))]

use std::io::{Read as _, Write as _};
use std::os::{fd::AsRawFd, unix::net::UnixStream};
use std::sync::Arc;
use verum_ast::{
    decl::{ImplItemKind, ImplKind},
    ItemKind,
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
    Value,
};

const SOURCE: &str = r#"
module tcp_bytes;
type FileDesc is (Int);
type TcpStream is { fd: FileDesc };
type OSError is { code: Int };
type IoError is { code: Int };
implement IoError {
    fn from_os(error: OSError) -> IoError { IoError { code: error.code } }
    fn Other(message: Text) -> IoError { IoError { code: -1 } }
}
type Read is protocol { fn read(&mut self, buf: &mut [Byte]) -> Result<Int, IoError>; };
type Write is protocol { fn write(&mut self, buf: &[Byte]) -> Result<Int, IoError>; };
const MSG_NOSIGNAL: Int = 0;
fn sys_send(fd: FileDesc, buf: &[Byte], flags: Int) -> Result<Int, OSError> {
    let n = unsafe { socket_send(fd.0 as Int32, buf.as_ptr(), buf.len(), flags as Int32) };
    if n < 0 { Result.Err(OSError { code: 9 }) } else { Result.Ok(n) }
}
fn sys_recv(fd: FileDesc, buf: &mut [Byte], flags: Int) -> Result<Int, OSError> {
    let n = unsafe { socket_recv(fd.0 as Int32, buf.as_mut_ptr(), buf.len(), flags as Int32) };
    if n < 0 { Result.Err(OSError { code: 9 }) } else { Result.Ok(n) }
}
fn send(fd: FileDesc, buf: &[Byte], flags: Int) -> Result<Int, OSError> { sys_send(fd, buf, flags) }
fn recv(fd: FileDesc, buf: &mut [Byte], flags: Int) -> Result<Int, OSError> { sys_recv(fd, buf, flags) }
fn write_probe(fd: Int) -> Int {
    let mut stream = TcpStream { fd: FileDesc(fd) };
    let mut bytes: List<Byte> = List.new();
    bytes.push(37_u8);
    let mut i = 0;
    while i < 10003 { bytes.push((i % 256) as Byte); i = i + 1; }
    bytes.push(37_u8);
    let mut offset = 1;
    while offset < bytes.len() - 1 {
        let n = stream.write(&bytes[offset..bytes.len() - 1]).unwrap();
        assert(n > 0 && n <= bytes.len() - 1 - offset);
        offset = offset + n;
    }
    offset - 1
}
fn read_probe(fd: Int) -> Int {
    let mut stream = TcpStream { fd: FileDesc(fd) };
    let mut bytes: List<Byte> = List.new();
    let mut i = 0;
    while i < 10005 { bytes.push(37_u8); i = i + 1; }
    let mut offset = 1;
    while offset < bytes.len() - 1 {
        let n = stream.read(&mut bytes[offset..bytes.len() - 1]).unwrap();
        if n == 0 { break; }
        assert(n > 0 && n <= bytes.len() - 1 - offset);
        offset = offset + n;
    }
    assert(bytes[0] == 37 && bytes[bytes.len() - 1] == 37);
    i = 1;
    while i < offset { assert(bytes[i] == ((i - 1) % 256) as Byte); i = i + 1; }
    offset - 1
}
fn packed_probe(fd: Int) -> Int {
    let mut stream = TcpStream { fd: FileDesc(fd) };
    let mut bytes: [Byte; 5] = [37_u8, 0_u8, 255_u8, 13_u8, 37_u8];
    assert(stream.write(&bytes[0..0]) is Result.Ok(0));
    assert(stream.read(&mut bytes[0..0]) is Result.Ok(0));
    assert(stream.write(&bytes[1..4]) is Result.Ok(3));
    bytes[1] = 37; bytes[2] = 37; bytes[3] = 37;
    let mut offset = 1;
    while offset < 4 {
        let n = stream.read(&mut bytes[offset..4]).unwrap();
        assert(n > 0 && n <= 4 - offset);
        offset = offset + n;
    }
    assert(bytes[0] == 37 && bytes[4] == 37);
    assert(bytes[1] == 0 && bytes[2] == 255 && bytes[3] == 13);
    let mut invalid = TcpStream { fd: FileDesc(-1) };
    assert(invalid.read(&mut bytes[..]) is Result.Err(_));
    assert(invalid.write(&bytes[..]) is Result.Err(_));
    offset - 1
}
"#;

fn module() -> verum_vbc::module::VbcModule {
    let library = if cfg!(target_os = "macos") {
        "libSystem.B.dylib"
    } else {
        "libc.so.6"
    };
    let source = format!(
        r#"{SOURCE}
        @ffi("{library}") extern {{
            @link_name("send") fn socket_send(fd: Int32, buf: &unsafe Byte, count: Int, flags: Int32) -> Int;
            @link_name("recv") fn socket_recv(fd: Int32, buf: &unsafe Byte, count: Int, flags: Int32) -> Int;
        }}"#
    );
    let mut ast = Parser::new(&source).parse_module().unwrap();
    let mut tcp = Parser::new(include_str!("../../../core/net/tcp.vr"))
        .parse_module()
        .unwrap();
    tcp.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Protocol { protocol, .. }
            if protocol.as_ident().is_some_and(|name| name.name == "Read" || name.name == "Write")) => {
            decl.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(function)
                if function.name.name == "read" || function.name.name == "write"));
            true
        }
        _ => false,
    });
    assert_eq!(
        tcp.items.len(),
        2,
        "both actual synchronous protocol implementations"
    );
    ast.items.extend(tcp.items);
    VbcCodegen::with_config(CodegenConfig::new("tcp_bytes"))
        .compile_module(&ast)
        .unwrap()
}

fn run(module: verum_vbc::module::VbcModule, name: &str, socket: &UnixStream) -> i64 {
    let entry = module.find_function_by_name(name).unwrap();
    Interpreter::new(Arc::new(module))
        .call(entry, &[Value::from_i64(socket.as_raw_fd() as i64)])
        .unwrap()
        .as_i64()
}

#[test]
fn synchronous_socket_writes_preserve_logical_bytes_and_subslice_offsets() {
    let module = module();
    for module in [
        module.clone(),
        verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap(),
    ] {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        socket
            .set_write_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let reader = std::thread::spawn(move || {
            peer.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut actual = Vec::new();
            peer.read_to_end(&mut actual).unwrap();
            actual
        });
        assert_eq!(run(module, "tcp_bytes.write_probe", &socket), 10003);
        socket.shutdown(std::net::Shutdown::Write).unwrap();
        let actual = reader.join().unwrap();
        let expected: Vec<_> = (0..10003).map(|i| (i % 256) as u8).collect();
        assert_eq!(actual.len(), expected.len());
        assert_eq!(
            actual.iter().zip(&expected).position(|(a, b)| a != b),
            None,
            "socket bytes differ from the logical slice"
        );
    }
}

#[test]
fn synchronous_socket_reads_update_only_the_requested_logical_slice() {
    let (socket, mut peer) = UnixStream::pair().unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let writer = std::thread::spawn(move || {
        peer.set_write_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        peer.write_all(&(0..10003).map(|i| (i % 256) as u8).collect::<Vec<_>>())
            .unwrap();
        peer.shutdown(std::net::Shutdown::Write).unwrap();
    });
    assert_eq!(run(module(), "tcp_bytes.read_probe", &socket), 10003);
    writer.join().unwrap();
}

#[test]
fn packed_empty_and_invalid_socket_controls() {
    let (socket, mut peer) = UnixStream::pair().unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let echo = std::thread::spawn(move || {
        peer.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut bytes = [0; 3];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, [0, 255, 13]);
        peer.write_all(&bytes).unwrap();
    });
    assert_eq!(run(module(), "tcp_bytes.packed_probe", &socket), 3);
    echo.join().unwrap();
}
