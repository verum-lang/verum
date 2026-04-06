//! Basic test of quote module functionality
//!
//! This example demonstrates that the quote module is fully functional.

use verum_ast::{FileId, Span};
use verum_compiler::quote::{TokenStream, ident, literal_int, literal_string};

fn main() {
    let file_id = FileId::new(999);
    let span = Span::new(0, 0, file_id);

    println!("Testing verum_compiler quote module...\n");

    // Test 1: Create identifier
    let id_stream = ident("test_var", span);
    println!("✓ ident() creates {} token(s)", id_stream.len());
    assert_eq!(id_stream.len(), 1);

    // Test 2: Create integer literal
    let int_stream = literal_int(42, span);
    println!("✓ literal_int() creates {} token(s)", int_stream.len());
    assert_eq!(int_stream.len(), 1);

    // Test 3: Create string literal
    let str_stream = literal_string("hello", span);
    println!("✓ literal_string() creates {} token(s)", str_stream.len());
    assert_eq!(str_stream.len(), 1);

    // Test 4: TokenStream from string
    let ts = TokenStream::from_str("1 + 2", file_id).unwrap();
    println!("✓ TokenStream::from_str() creates {} token(s)", ts.len());
    assert!(ts.len() >= 3);

    // Test 5: Parse as expression
    let expr = ts.parse_as_expr();
    match expr {
        Ok(_) => println!("✓ parse_as_expr() works"),
        Err(e) => {
            println!("✗ parse_as_expr() failed: {:?}", e);
            panic!("parse_as_expr should work");
        }
    }

    // Test 6: Parse as type
    let type_ts = TokenStream::from_str("List<Int>", file_id).unwrap();
    let ty = type_ts.parse_as_type();
    match ty {
        Ok(_) => println!("✓ parse_as_type() works"),
        Err(e) => {
            println!("✗ parse_as_type() failed: {:?}", e);
            panic!("parse_as_type should work");
        }
    }

    // Test 7: Parse as item
    let item_ts = TokenStream::from_str("fn test() -> Int { 42 }", file_id).unwrap();
    let item = item_ts.parse_as_item();
    match item {
        Ok(_) => println!("✓ parse_as_item() works"),
        Err(e) => {
            println!("✗ parse_as_item() failed: {:?}", e);
            panic!("parse_as_item should work");
        }
    }

    println!("\n✅ All core quote module functions are working!");
}
