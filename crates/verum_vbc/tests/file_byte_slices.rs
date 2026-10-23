//! Actual File Read/Write bodies must pass logical bytes to the OS.
#![cfg(all(feature = "codegen", feature = "ffi", unix))]

use std::io::{Seek as _, SeekFrom, Write as _};
use std::os::fd::AsRawFd;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
};
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    Value,
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

const SUPPORT: &str = r#"
module file_bytes;
type FileDesc is (Int);
implement FileDesc { fn from_raw(fd: Int) -> FileDesc { FileDesc(fd) } }
type OSError is { code: Int };
type StreamError is { code: Int };
implement StreamError {
    fn from_raw_os_error(code: Int) -> StreamError { StreamError { code: code } }
    fn Other(message: Text) -> StreamError { StreamError { code: -1 } }
}
type IoResult<T> is Result<T, StreamError>;
type Read is protocol { fn read(&mut self, bytes: &mut [Byte]) -> IoResult<Int>; };
type Write is protocol { fn write(&mut self, bytes: &[Byte]) -> IoResult<Int>; };
fn sys_write(fd: FileDesc, bytes: &[Byte]) -> Result<Int, OSError> {
    let count = unsafe { write(fd.0 as Int32, bytes.as_ptr(), bytes.len()) };
    if count < 0 { Result.Err(OSError { code: 9 }) } else { Result.Ok(count) }
}
fn sys_read(fd: FileDesc, bytes: &mut [Byte]) -> Result<Int, OSError> {
    let count = unsafe { read(fd.0 as Int32, bytes.as_mut_ptr(), bytes.len()) };
    if count < 0 { Result.Err(OSError { code: 9 }) } else { Result.Ok(count) }
}
fn write_probe(fd: Int) -> Int {
    let mut bytes: List<Byte> = List.new();
    bytes.push(37_u8);
    let mut index = 0;
    while index < 10003 { bytes.push((index % 256) as Byte); index = index + 1; }
    bytes.push(37_u8);
    let mut file = File { fd: fd };
    let mut position = 1;
    while position < bytes.len() - 1 {
        let count = match file.write(&bytes[position..bytes.len() - 1]) {
            Result.Ok(count) => count, Result.Err(_) => return -1,
        };
        assert(count > 0 && count <= bytes.len() - 1 - position);
        position = position + count;
    }
    position - 1
}
fn read_probe(fd: Int) -> Int {
    let mut file = File { fd: fd };
    let mut destination: List<Byte> = List.new();
    let mut initialized = 0;
    while initialized < 10005 { destination.push(37_u8); initialized = initialized + 1; }
    let mut position = 1;
    while position < destination.len() - 1 {
        let count = match file.read(&mut destination[position..destination.len() - 1]) {
            Result.Ok(count) => count, Result.Err(_) => return -1,
        };
        if count == 0 { break; }
        assert(count > 0 && count <= destination.len() - 1 - position);
        position = position + count;
    }
    assert(file.read(&mut destination[1..2]) is Result.Ok(0));
    assert(destination[0] == 37);
    assert(destination[destination.len() - 1] == 37);
    let mut index = 1;
    while index < position { assert(destination[index] == ((index - 1) % 256) as Byte); index = index + 1; }
    position - 1
}
fn packed_probe(fd: Int) -> Int {
    let mut file = File { fd: fd };
    let bytes: [Byte; 5] = [37_u8, 0_u8, 255_u8, 13_u8, 37_u8];
    match file.write(&bytes[1..4]) { Result.Ok(count) => count, Result.Err(_) => -1 }
}
fn empty_probe(fd: Int) -> Int {
    let mut file = File { fd: fd };
    let mut bytes: [Byte; 1] = [37_u8];
    assert(file.write(&bytes[0..0]) is Result.Ok(0));
    assert(file.read(&mut bytes[0..0]) is Result.Ok(0));
    assert(bytes[0] == 37);
    let mut invalid = File { fd: -1 };
    assert(invalid.write(&bytes[..]) is Result.Err(_));
    assert(invalid.read(&mut bytes[..]) is Result.Err(_));
    0
}
"#;

fn modules() -> [verum_vbc::module::VbcModule; 2] {
    let library = if cfg!(target_os = "macos") {
        "libSystem.B.dylib"
    } else {
        "libc.so.6"
    };
    let source = format!(
        r#"{SUPPORT}
        @ffi("{library}") extern {{
            fn write(fd: Int32, data: &unsafe Byte, count: Int) -> Int;
            fn read(fd: Int32, data: &unsafe Byte, count: Int) -> Int;
        }}"#
    );
    let mut ast = Parser::new(&source)
        .parse_module()
        .expect("fixture grammar");
    let mut file = Parser::new(include_str!("../../../core/io/file.vr"))
        .parse_module()
        .expect("actual File grammar");
    file.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(decl) => decl.name.name == "File",
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Protocol { protocol, .. }
            if protocol.as_ident().is_some_and(|name| name.name == "Read" || name.name == "Write")) => {
            decl.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(function)
                if function.name.name == "read" || function.name.name == "write"));
            true
        }
        _ => false,
    });
    assert_eq!(
        file.items.len(),
        3,
        "File declaration and both actual protocol implementations"
    );
    ast.items.extend(file.items);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("file_bytes"));
    codegen.register_builtin_variants();
    let module = codegen
        .compile_module(&ast)
        .expect("strict actual File bodies");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("deserialize");
    [module, wire]
}

struct ScratchFile {
    path: std::path::PathBuf,
    file: std::fs::File,
}
impl ScratchFile {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "verum-byte-io-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .expect("test-owned file");
        Self { path, file }
    }
}
impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn slot_backed_byte_slices_write_exact_bytes_across_short_writes() {
    for module in modules() {
        let scratch = ScratchFile::new();
        let expected: List<u8> = (0..10003).map(|i| (i % 256) as u8).collect();
        let entry = module
            .find_function_by_name("file_bytes.write_probe")
            .expect("entry");
        let mut interpreter = Interpreter::new(Arc::new(module));
        assert_eq!(
            interpreter
                .call(
                    entry,
                    &[Value::from_i64(i64::from(scratch.file.as_raw_fd()))]
                )
                .expect("actual File.write")
                .as_i64(),
            expected.len() as i64
        );
        let actual: List<u8> = std::fs::read(&scratch.path).unwrap().into();
        assert!(
            actual == expected,
            "File.write changed logical bytes: first difference {:?}",
            actual.iter().zip(expected.iter()).position(|(a, b)| a != b)
        );
    }
}

#[test]
fn reads_update_only_the_selected_logical_bytes() {
    for module in modules() {
        let mut scratch = ScratchFile::new();
        let expected: List<u8> = (0..10003).map(|i| (i % 256) as u8).collect();
        scratch.file.write_all(&expected).unwrap();
        scratch.file.seek(SeekFrom::Start(0)).unwrap();
        let entry = module
            .find_function_by_name("file_bytes.read_probe")
            .expect("entry");
        let mut interpreter = Interpreter::new(Arc::new(module));
        assert_eq!(
            interpreter
                .call(
                    entry,
                    &[Value::from_i64(i64::from(scratch.file.as_raw_fd()))]
                )
                .expect("actual File.read")
                .as_i64(),
            expected.len() as i64
        );
    }
}

#[test]
fn packed_empty_and_invalid_descriptor_controls() {
    for module in modules() {
        let scratch = ScratchFile::new();
        let packed = module
            .find_function_by_name("file_bytes.packed_probe")
            .expect("packed entry");
        let empty = module
            .find_function_by_name("file_bytes.empty_probe")
            .expect("empty entry");
        let mut interpreter = Interpreter::new(Arc::new(module));
        let fd = [Value::from_i64(i64::from(scratch.file.as_raw_fd()))];
        assert_eq!(
            interpreter
                .call(packed, &fd)
                .expect("packed write")
                .as_i64(),
            3
        );
        assert_eq!(
            interpreter
                .call(empty, &fd)
                .expect("empty and error controls")
                .as_i64(),
            0
        );
        let bytes: List<u8> = std::fs::read(&scratch.path).unwrap().into();
        assert_eq!(bytes.as_slice(), &[0, 255, 13]);
    }
}
