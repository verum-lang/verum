//! Debug tagged literal tokenization

use verum_lexer::Lexer;
use verum_ast::FileId;

#[test]
fn test_tagged_literal_tokens() {
    // Using escaped quotes for JSON content
    let source = r#"let a = json#"{\"key\": \"value\"}";"#;

    println!("\nSource: {}", source);
    println!("Length: {}", source.len());
    println!("\nTokens:");

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    for (i, result) in lexer.enumerate() {
        match result {
            Ok(token) => println!("  {:2}. [{:3}..{:3}] {:?}", i, token.span.start, token.span.end, token.kind),
            Err(e) => println!("  {:2}. ERROR: {:?}", i, e),
        }
    }
}

#[test]
fn test_simple_tagged() {
    let source = r#"json#"test""#;
    
    println!("\nSimple Source: {}", source);
    println!("Length: {}", source.len());
    println!("\nTokens:");
    
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    
    for (i, result) in lexer.enumerate() {
        match result {
            Ok(token) => println!("  {:2}. [{:3}..{:3}] {:?}", i, token.span.start, token.span.end, token.kind),
            Err(e) => println!("  {:2}. ERROR: {:?}", i, e),
        }
    }
}
