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
//! FFI Attribute Extraction Tests
//!
//! Tests that @extern and @ownership attributes are properly parsed and extracted.

use verum_ast::ffi::{CallingConvention, Ownership};
use verum_ast::{FileId, ItemKind};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

#[test]
fn test_extern_c_calling_convention() {
    let source = r#"
ffi LibC {
    @extern("C")
    fn malloc(size: Int) -> Int;
    requires size > 0;
    ensures result != 0;
    memory_effects = Allocates;
    thread_safe = true;
    errors_via = None;
    @ownership(borrow);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            let func = &boundary.functions[0];
            assert_eq!(
                func.signature.calling_convention,
                CallingConvention::C,
                "Expected C calling convention"
            );
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_extern_stdcall_calling_convention() {
    let source = r#"
ffi WinApi {
    @extern("C", calling_convention = "stdcall")
    fn GetLastError() -> Int;
    requires true;
    ensures result >= 0;
    memory_effects = Pure;
    thread_safe = true;
    errors_via = None;
    @ownership(borrow);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            let func = &boundary.functions[0];
            assert_eq!(
                func.signature.calling_convention,
                CallingConvention::StdCall,
                "Expected StdCall calling convention"
            );
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ownership_borrow() {
    let source = r#"
ffi Test {
    @extern("C")
    fn read_only(ptr: Int) -> Int;
    requires true;
    ensures true;
    memory_effects = Pure;
    thread_safe = true;
    errors_via = None;
    @ownership(borrow);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            let func = &boundary.functions[0];
            assert_eq!(
                func.ownership,
                Ownership::Borrow,
                "Expected Borrow ownership"
            );
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ownership_shared() {
    let source = r#"
ffi Test {
    @extern("C")
    fn shared_access(ptr: Int) -> Int;
    requires true;
    ensures true;
    memory_effects = Reads;
    thread_safe = true;
    errors_via = None;
    @ownership(shared);
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            let func = &boundary.functions[0];
            assert_eq!(
                func.ownership,
                Ownership::Shared,
                "Expected Shared ownership"
            );
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ownership_transfer_to() {
    let source = r#"
ffi Test {
    @extern("C")
    fn give_to_c(ptr: Int);
    requires true;
    ensures true;
    memory_effects = Pure;
    thread_safe = true;
    errors_via = None;
    @ownership(transfer_to = "ptr");
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            let func = &boundary.functions[0];
            match &func.ownership {
                Ownership::TransferTo(param) => {
                    assert_eq!(param.as_str(), "ptr", "Expected transfer_to(ptr)");
                }
                other => panic!("Expected TransferTo, got {:?}", other),
            }
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ownership_transfer_from() {
    let source = r#"
ffi Test {
    @extern("C")
    fn take_from_c() -> Int;
    requires true;
    ensures true;
    memory_effects = Allocates;
    thread_safe = true;
    errors_via = None;
    @ownership(transfer_from = "result");
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            let func = &boundary.functions[0];
            match &func.ownership {
                Ownership::TransferFrom(param) => {
                    assert_eq!(param.as_str(), "result", "Expected transfer_from(result)");
                }
                other => panic!("Expected TransferFrom, got {:?}", other),
            }
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_complete_ffi_boundary_with_all_features() {
    let source = r#"
ffi LibMath {
    @extern("C")
    fn sqrt(x: Float) -> Float;
    requires x >= 0.0;
    ensures result >= 0.0;
    memory_effects = Pure;
    thread_safe = true;
    errors_via = None;
    @ownership(borrow);

    @extern("C", calling_convention = "fastcall")
    fn fast_multiply(a: Int, b: Int) -> Int;
    requires true;
    ensures true;
    memory_effects = Pure;
    thread_safe = true;
    errors_via = None;
    @ownership(borrow);

    @extern("C")
    fn allocate_buffer(size: Int) -> Int;
    requires size > 0;
    ensures result != 0;
    memory_effects = Allocates;
    thread_safe = true;
    errors_via = ReturnValue(0);
    @ownership(transfer_from = "result");
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    let module = result.unwrap();

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.name.name, "LibMath");
            assert_eq!(boundary.functions.len(), 3, "Expected 3 FFI functions");

            // First function: sqrt with C calling convention and borrow
            let sqrt = &boundary.functions[0];
            assert_eq!(sqrt.name.name, "sqrt");
            assert_eq!(sqrt.signature.calling_convention, CallingConvention::C);
            assert_eq!(sqrt.ownership, Ownership::Borrow);

            // Second function: fast_multiply with fastcall
            let fast_mul = &boundary.functions[1];
            assert_eq!(fast_mul.name.name, "fast_multiply");
            assert_eq!(
                fast_mul.signature.calling_convention,
                CallingConvention::FastCall
            );
            assert_eq!(fast_mul.ownership, Ownership::Borrow);

            // Third function: allocate_buffer with transfer_from
            let alloc = &boundary.functions[2];
            assert_eq!(alloc.name.name, "allocate_buffer");
            assert_eq!(alloc.signature.calling_convention, CallingConvention::C);
            match &alloc.ownership {
                Ownership::TransferFrom(param) => {
                    assert_eq!(param.as_str(), "result");
                }
                other => panic!("Expected TransferFrom for allocate_buffer, got {:?}", other),
            }
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}
