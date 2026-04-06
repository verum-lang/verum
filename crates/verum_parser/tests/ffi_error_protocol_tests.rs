#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Comprehensive FFI Error Protocol Parsing Tests
//
// Tests all 7 error protocol variants according to specification:
// - None
// - Errno
// - Exception
// - ReturnCode(pattern)
// - ReturnValue(expr)
// - ReturnValue(expr) with Errno
//
// Tests for Verum v6 syntax compliance#2.7.3 - FFI error protocol
// Tests for FFI boundary declarations: C ABI bindings, contracts, memory effects#3.4 - Error handling protocols

use verum_ast::ffi::ErrorProtocol;
use verum_ast::{FileId, ItemKind};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper to extract error protocol from parsed FFI boundary
fn extract_error_protocol(source: &str) -> ErrorProtocol {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI boundary: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1, "Expected 1 FFI boundary item");

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert!(
                !boundary.functions.is_empty(),
                "Expected at least 1 function"
            );
            boundary.functions[0].error_protocol.clone()
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

// ============================================================================
// 1. ErrorProtocol::None - Function cannot fail
// ============================================================================

#[test]
fn test_error_protocol_none() {
    let source = r#"
ffi LibMath {
    @extern("C")
    fn sqrt(x: f64) -> f64;
    requires x >= 0.0;
    memory_effects = Pure;
    errors_via = None;
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::None),
        "Expected ErrorProtocol.None, got {:?}",
        protocol
    );
}

// ============================================================================
// 2. ErrorProtocol::Errno - POSIX errno for error reporting
// ============================================================================

#[test]
fn test_error_protocol_errno() {
    let source = r#"
ffi Libc {
    @extern("C")
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    memory_effects = Writes(buf);
    errors_via = Errno;
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::Errno),
        "Expected ErrorProtocol.Errno, got {:?}",
        protocol
    );
}

// ============================================================================
// 3. ErrorProtocol::Exception - C++ exceptions
// ============================================================================

#[test]
fn test_error_protocol_exception() {
    let source = r#"
ffi CppLib {
    @extern("C++")
    fn process_data(data: *const u8) -> i32;
    memory_effects = Reads(data);
    errors_via = Exception;
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::Exception),
        "Expected ErrorProtocol.Exception, got {:?}",
        protocol
    );
}

// ============================================================================
// 4. ErrorProtocol::ReturnCode - Success/error based on return value pattern
// ============================================================================

#[test]
fn test_error_protocol_return_code_success_value() {
    // Success when result == SQLITE_OK
    let source = r#"
ffi Sqlite {
    @extern("C")
    fn sqlite3_open(filename: *const char, db: *mut *void) -> i32;
    memory_effects = Allocates;
    errors_via = ReturnCode(SQLITE_OK);
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnCode(_)),
        "Expected ErrorProtocol.ReturnCode, got {:?}",
        protocol
    );
}

#[test]
fn test_error_protocol_return_code_error_pattern() {
    // Error when result != Z_OK
    let source = r#"
ffi Zlib {
    @extern("C")
    fn compress(dest: *mut u8, destLen: *mut usize, src: *const u8, srcLen: usize) -> i32;
    memory_effects = Reads(src) + Writes(dest);
    errors_via = ReturnCode(Z_OK);
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnCode(_)),
        "Expected ErrorProtocol.ReturnCode, got {:?}",
        protocol
    );
}

#[test]
fn test_error_protocol_return_code_null_sentinel() {
    let source = r#"
ffi WinAPI {
    @extern("C")
    fn CreateFileA(
        filename: *const char,
        access: u32,
        share: u32,
        security: *void,
        disposition: u32,
        flags: u32,
        template: *void
    ) -> *void;
    memory_effects = Allocates;
    errors_via = ReturnCode(null);
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnCode(_)),
        "Expected ErrorProtocol.ReturnCode, got {:?}",
        protocol
    );
}

#[test]
fn test_error_protocol_return_code_negative_sentinel() {
    let source = r#"
ffi StdIO {
    @extern("C")
    fn fgetc(stream: *FILE) -> i32;
    memory_effects = Reads(stream);
    errors_via = ReturnCode(EOF);
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnCode(_)),
        "Expected ErrorProtocol.ReturnCode, got {:?}",
        protocol
    );
}

// ============================================================================
// 5. ErrorProtocol::ReturnValue - Sentinel value on error
// ============================================================================

#[test]
fn test_error_protocol_return_value_null() {
    let source = r#"
ffi Allocator {
    @extern("C")
    fn malloc(size: usize) -> *mut u8;
    memory_effects = Allocates;
    errors_via = ReturnValue(null);
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnValue(_)),
        "Expected ErrorProtocol.ReturnValue, got {:?}",
        protocol
    );
}

#[test]
fn test_error_protocol_return_value_negative_one() {
    let source = r#"
ffi Socket {
    @extern("C")
    fn socket(domain: i32, type_: i32, protocol: i32) -> i32;
    memory_effects = Allocates;
    errors_via = ReturnValue(-1);
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnValue(_)),
        "Expected ErrorProtocol.ReturnValue, got {:?}",
        protocol
    );
}

// ============================================================================
// 6. ErrorProtocol::ReturnValueWithErrno - Sentinel value + errno
// ============================================================================

#[test]
fn test_error_protocol_return_value_with_errno_null() {
    let source = r#"
ffi Libc {
    @extern("C")
    fn fopen(pathname: *const char, mode: *const char) -> *FILE;
    memory_effects = Allocates;
    errors_via = ReturnValue(null) with Errno;
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnValueWithErrno(_)),
        "Expected ErrorProtocol.ReturnValueWithErrno, got {:?}",
        protocol
    );
}

#[test]
fn test_error_protocol_return_value_with_errno_negative() {
    let source = r#"
ffi Posix {
    @extern("C")
    fn open(pathname: *const char, flags: i32, mode: u32) -> i32;
    memory_effects = Allocates;
    errors_via = ReturnValue(-1) with Errno;
}
"#;

    let protocol = extract_error_protocol(source);
    assert!(
        matches!(protocol, ErrorProtocol::ReturnValueWithErrno(_)),
        "Expected ErrorProtocol.ReturnValueWithErrno, got {:?}",
        protocol
    );
}

// ============================================================================
// Integration Tests - Complete FFI Boundaries
// ============================================================================

#[test]
fn test_complete_ffi_boundary_with_all_error_protocols() {
    let source = r#"
ffi TestLib {
    @extern("C")
    fn sqrt(x: f64) -> f64;
    errors_via = None;

    @extern("C")
    fn read_file(fd: i32, buf: *mut u8) -> isize;
    errors_via = Errno;

    @extern("C")
    fn sqlite_open(path: *const char) -> i32;
    errors_via = ReturnCode(SQLITE_OK);

    @extern("C")
    fn malloc(size: usize) -> *mut u8;
    errors_via = ReturnValue(null);

    @extern("C")
    fn fopen(path: *const char) -> *FILE;
    errors_via = ReturnValue(null) with Errno;

    @extern("C++")
    fn cpp_process() -> i32;
    errors_via = Exception;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse multi-protocol FFI boundary: {:?}",
        result.err()
    );

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.functions.len(), 6, "Expected 6 FFI functions");

            // Verify each error protocol
            assert!(matches!(
                boundary.functions[0].error_protocol,
                ErrorProtocol::None
            ));
            assert!(matches!(
                boundary.functions[1].error_protocol,
                ErrorProtocol::Errno
            ));
            assert!(matches!(
                boundary.functions[2].error_protocol,
                ErrorProtocol::ReturnCode(_)
            ));
            assert!(matches!(
                boundary.functions[3].error_protocol,
                ErrorProtocol::ReturnValue(_)
            ));
            assert!(matches!(
                boundary.functions[4].error_protocol,
                ErrorProtocol::ReturnValueWithErrno(_)
            ));
            assert!(matches!(
                boundary.functions[5].error_protocol,
                ErrorProtocol::Exception
            ));
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

// ============================================================================
// Memory Effects Tests
// ============================================================================

#[test]
fn test_memory_effects_pure() {
    let source = r#"
ffi Math {
    @extern("C")
    fn abs(x: i32) -> i32;
    memory_effects = Pure;
    errors_via = None;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse memory_effects = Pure: {:?}",
        result.err()
    );
}

#[test]
fn test_memory_effects_reads() {
    let source = r#"
ffi Libc {
    @extern("C")
    fn strlen(s: *const char) -> usize;
    memory_effects = Reads(s);
    errors_via = None;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse memory_effects = Reads(s): {:?}",
        result.err()
    );
}

#[test]
fn test_memory_effects_writes() {
    let source = r#"
ffi Libc {
    @extern("C")
    fn strcpy(dest: *mut char, src: *const char) -> *mut char;
    memory_effects = Writes(dest);
    errors_via = None;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse memory_effects = Writes(dest): {:?}",
        result.err()
    );
}

#[test]
fn test_memory_effects_allocates() {
    let source = r#"
ffi Allocator {
    @extern("C")
    fn malloc(size: usize) -> *mut u8;
    memory_effects = Allocates;
    errors_via = ReturnValue(null);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse memory_effects = Allocates: {:?}",
        result.err()
    );
}

#[test]
fn test_memory_effects_deallocates() {
    let source = r#"
ffi Allocator {
    @extern("C")
    fn free(ptr: *mut u8);
    memory_effects = Deallocates(ptr);
    errors_via = None;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse memory_effects = Deallocates(ptr): {:?}",
        result.err()
    );
}

#[test]
fn test_memory_effects_combined() {
    let source = r#"
ffi Sqlite {
    @extern("C")
    fn sqlite3_exec(
        db: *void,
        sql: *const char,
        callback: *void,
        arg: *void,
        errmsg: **char
    ) -> i32;
    memory_effects = Reads(sql) + Writes(errmsg);
    errors_via = ReturnCode(SQLITE_OK);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse combined memory effects: {:?}",
        result.err()
    );
}

// ============================================================================
// Ownership Tests
// ============================================================================

#[test]
fn test_ownership_borrow() {
    let source = r#"
ffi Libc {
    @extern("C")
    fn strlen(s: *const char) -> usize;
    memory_effects = Reads(s);
    errors_via = None;
    @ownership(borrow);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse @ownership(borrow): {:?}",
        result.err()
    );
}

#[test]
fn test_ownership_shared() {
    let source = r#"
ffi RefCount {
    @extern("C")
    fn ref_increment(ptr: *void);
    memory_effects = Writes(ptr);
    errors_via = None;
    @ownership(shared);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse @ownership(shared): {:?}",
        result.err()
    );
}

#[test]
fn test_ownership_transfer_to() {
    let source = r#"
ffi Allocator {
    @extern("C")
    fn free(ptr: *mut u8);
    memory_effects = Deallocates(ptr);
    errors_via = None;
    @ownership(transfer_to = "C");
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse @ownership(transfer_to = \"C\"): {:?}",
        result.err()
    );
}

#[test]
fn test_ownership_transfer_from() {
    let source = r#"
ffi Allocator {
    @extern("C")
    fn malloc(size: usize) -> *mut u8;
    memory_effects = Allocates;
    errors_via = ReturnValue(null);
    @ownership(transfer_from = "C");
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse @ownership(transfer_from = \"C\"): {:?}",
        result.err()
    );
}

#[test]
fn test_minimal_ffi_boundary() {
    let source = "ffi LibMath { }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse minimal FFI boundary: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.name.as_str(), "LibMath");
            assert_eq!(boundary.functions.len(), 0);
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_function_no_contracts() {
    let source = r#"
ffi LibMath {
    fn sqrt(x: f64) -> f64;
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI function without contracts: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.functions.len(), 1);
            assert_eq!(boundary.functions[0].name.as_str(), "sqrt");
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_function_with_extern_attr() {
    let source = r#"
ffi LibMath {
    @extern("C")
    fn sqrt(x: f64) -> f64;
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI function with @extern: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.functions.len(), 1);
            assert_eq!(boundary.functions[0].name.as_str(), "sqrt");
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_function_with_requires() {
    let source = r#"
ffi LibMath {
    @extern("C")
    fn sqrt(x: f64) -> f64;
    requires x >= 0.0;
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI function with requires: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.functions.len(), 1);
            assert_eq!(boundary.functions[0].name.as_str(), "sqrt");
            assert_eq!(boundary.functions[0].requires.len(), 1);
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_function_with_requires_no_newlines() {
    let source = "ffi LibMath { @extern(\"C\") fn sqrt(x: f64) -> f64; requires x >= 0.0; }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI function with requires (no newlines): {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.functions.len(), 1);
            assert_eq!(boundary.functions[0].name.as_str(), "sqrt");
            assert_eq!(boundary.functions[0].requires.len(), 1);
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_with_all_contracts() {
    let source = r#"
ffi LibMath {
    @extern("C")
    fn sqrt(x: f64) -> f64;
    requires x >= 0.0;
    memory_effects = Pure;
    errors_via = None;
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI with all contracts: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.functions.len(), 1);
            assert_eq!(boundary.functions[0].name.as_str(), "sqrt");
            assert_eq!(boundary.functions[0].requires.len(), 1);
            assert!(matches!(
                boundary.functions[0].error_protocol,
                ErrorProtocol::None
            ));
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_with_memory_effects() {
    let source = r#"
ffi LibMath {
    @extern("C")
    fn sqrt(x: f64) -> f64;
    requires x >= 0.0;
    memory_effects = Pure;
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI with memory_effects: {:?}",
        result.err()
    );
}

#[test]
fn test_ffi_with_errors_via() {
    let source = r#"
ffi LibMath {
    @extern("C")
    fn sqrt(x: f64) -> f64;
    requires x >= 0.0;
    errors_via = None;
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI with errors_via: {:?}",
        result.err()
    );
}

#[test]
fn test_ffi_with_return_code_simple() {
    let source = r#"
ffi Sqlite {
    fn open(filename: Text) -> i32;
    errors_via = ReturnCode(SQLITE_OK);
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(
        result.is_ok(),
        "Failed to parse FFI with ReturnCode: {:?}",
        result.err()
    );
}
