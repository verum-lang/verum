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
// FFI Boundary Declaration Parsing Tests
//
// Tests for parsing FFI boundary declarations according to Verum specification.
// Tests for FFI boundary declarations: C ABI bindings, contracts, memory effects

use verum_ast::{FileId, ItemKind};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

#[test]
fn test_simple_ffi_boundary_parses() {
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
}
"#;

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

    // Check that the item is indeed an FFI boundary
    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.name.name, "LibMath");
            assert_eq!(boundary.functions.len(), 1);

            let func = &boundary.functions[0];
            assert_eq!(func.name.name, "sqrt");
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 1);
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_with_multiple_functions() {
    let source = r#"
ffi LibC {
    @extern("C")
    fn malloc(size: Int) -> Int;
    requires size > 0;
    ensures result != 0;
    memory_effects = Allocates;
    thread_safe = true;
    errors_via = None;
    @ownership(transfer_from);

    @extern("C")
    fn free(ptr: Int);
    requires ptr != 0;
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

    assert!(
        result.is_ok(),
        "Failed to parse multi-function FFI boundary: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.name.name, "LibC");
            assert_eq!(boundary.functions.len(), 2, "Expected 2 FFI functions");
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_with_no_params() {
    let source = r#"
ffi System {
    @extern("C")
    fn random() -> Int;
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

    assert!(
        result.is_ok(),
        "Failed to parse FFI boundary with no params: {:?}",
        result.err()
    );

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            let func = &boundary.functions[0];
            assert_eq!(func.signature.params.len(), 0);
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}

#[test]
fn test_ffi_extends_clause() {
    let source = r#"
ffi BaseLib {
    @extern("C")
    fn base_func() -> Int;
    requires true;
    ensures true;
    memory_effects = Pure;
    thread_safe = true;
    errors_via = None;
    @ownership(borrow);
}

ffi MyLib extends BaseLib {
    @extern("C")
    fn my_func() -> Int;
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

    assert!(
        result.is_ok(),
        "Failed to parse FFI with extends clause: {:?}",
        result.err()
    );

    let module = result.unwrap();
    assert_eq!(module.items.len(), 2, "Expected 2 FFI boundary items");

    // Check base library has no extends
    match &module.items[0].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.name.name, "BaseLib");
            assert!(
                boundary.extends.is_none(),
                "BaseLib should not extend anything"
            );
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }

    // Check derived library extends BaseLib
    match &module.items[1].kind {
        ItemKind::FFIBoundary(boundary) => {
            assert_eq!(boundary.name.name, "MyLib");
            assert!(boundary.extends.is_some(), "MyLib should extend BaseLib");
            if let verum_common::Maybe::Some(parent) = &boundary.extends {
                assert_eq!(parent.name, "BaseLib", "MyLib should extend BaseLib");
            }
        }
        other => panic!("Expected FFIBoundary, got {:?}", other),
    }
}
