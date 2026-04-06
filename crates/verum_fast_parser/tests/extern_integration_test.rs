#![allow(unused_imports)]
use verum_ast::{FileId, ItemKind, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

#[test]
fn test_extern_functions_integration() {
    let source = r#"
extern fn builtin_unix_timestamp_secs() -> Int;
extern "C" fn printf(format: *const c_char) -> Int;
public extern "system" fn win32_api(handle: Int) -> Int;
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    let module = parser
        .parse_module(lexer, file_id)
        .expect("Failed to parse");

    assert_eq!(module.items.len(), 3);

    // Check first function: extern fn (default ABI)
    if let ItemKind::Function(func) = &module.items[0].kind {
        assert_eq!(func.name.name.as_str(), "builtin_unix_timestamp_secs");
        assert!(func.extern_abi.is_some());
        assert_eq!(func.extern_abi.as_ref().unwrap().as_str(), "");
        assert!(func.body.is_none());
    } else {
        panic!("Expected function");
    }

    // Check second function: extern "C" fn
    if let ItemKind::Function(func) = &module.items[1].kind {
        assert_eq!(func.name.name.as_str(), "printf");
        assert!(func.extern_abi.is_some());
        assert_eq!(func.extern_abi.as_ref().unwrap().as_str(), "C");
        assert!(func.body.is_none());
    } else {
        panic!("Expected function");
    }

    // Check third function: public extern "system" fn
    if let ItemKind::Function(func) = &module.items[2].kind {
        assert_eq!(func.name.name.as_str(), "win32_api");
        assert!(func.extern_abi.is_some());
        assert_eq!(func.extern_abi.as_ref().unwrap().as_str(), "system");
        assert!(func.body.is_none());
    } else {
        panic!("Expected function");
    }
}
/// Test parsing of extern functions from an example file.
/// This test uses an inline example rather than requiring an external file.
#[test]
fn test_extern_example_file() {
    use verum_ast::{FileId, ItemKind};
    use verum_lexer::Lexer;
    use verum_fast_parser::VerumParser;

    // Inline extern function example (rather than requiring /tmp/extern_example.vr)
    let source = r#"
// Extern function declarations for FFI
extern fn c_malloc(size: Int) -> *mut ();
extern "C" fn c_free(ptr: *mut ());
extern "system" fn win_api_call(handle: Int) -> Int;
public extern fn libc_exit(code: Int);
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    let module = parser
        .parse_module(lexer, file_id)
        .expect("Failed to parse extern example");

    // Count extern functions
    let extern_count = module
        .items
        .iter()
        .filter(|item| {
            if let ItemKind::Function(func) = &item.kind {
                func.extern_abi.is_some()
            } else {
                false
            }
        })
        .count();

    println!("Successfully parsed {} extern functions", extern_count);
    assert_eq!(
        extern_count, 4,
        "Should have parsed exactly 4 extern functions"
    );

    // Verify no function has a body
    for item in &module.items {
        if let ItemKind::Function(func) = &item.kind
            && func.extern_abi.is_some() {
                assert!(
                    func.body.is_none(),
                    "Extern function {} should not have a body",
                    func.name.name
                );
            }
    }
}
