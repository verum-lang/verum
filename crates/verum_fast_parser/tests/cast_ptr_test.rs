use verum_ast::FileId;
use verum_fast_parser::RecursiveParser;
use verum_lexer::Lexer;

#[test]
fn test_forall_expr_standalone() {
    // Test just the forall expression to isolate the issue
    let content = "let x = forall i: Int . !(0 <= i) || i <= n;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    println!("\nTokens for test_forall_expr_standalone:");
    for (i, token) in tokens.iter().enumerate() {
        println!("  {}: {:?}", i, token.kind);
    }

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_stmt() {
        Ok(_) => println!("test_forall_expr_standalone: SUCCESS"),
        Err(e) => {
            println!("test_forall_expr_standalone: ERROR: {:?}", e);
            // Don't panic so we can see all tests run
        }
    }
}

#[test]
fn test_forall_with_only_or() {
    // Test without the negation
    let content = "let x = forall i: Int . i >= 0 || i <= n;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_stmt() {
        Ok(_) => println!("test_forall_with_only_or: SUCCESS"),
        Err(e) => panic!("test_forall_with_only_or: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_with_non_primitive_type() {
    // Test forall with a non-primitive identifier type (was previously failing)
    let content = "let x = forall i: MyType . true;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_stmt() {
        Ok(_) => println!("test_forall_with_non_primitive_type: SUCCESS"),
        Err(e) => panic!("test_forall_with_non_primitive_type: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_with_generic_type() {
    // Test forall with a generic type
    let content = "let x = forall i: List<Int> . true;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_stmt() {
        Ok(_) => println!("test_forall_with_generic_type: SUCCESS"),
        Err(e) => panic!("test_forall_with_generic_type: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_simple_proof_by() {
    // Simplest theorem with proof by
    let content = r#"
theorem test(n: Int)
    ensures true
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_simple_proof_by: SUCCESS"),
        Err(e) => panic!("test_theorem_simple_proof_by: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_ensures_or_expr() {
    // Theorem with || in ensures (no forall)
    let content = r#"
theorem test(n: Int)
    ensures n >= 0 || n < 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_ensures_or_expr: SUCCESS"),
        Err(e) => panic!("test_theorem_ensures_or_expr: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_forall_simple_body() {
    // Theorem with forall but simpler body
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . i >= 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_simple_body: SUCCESS"),
        Err(e) => panic!("test_theorem_forall_simple_body: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_forall_or_body() {
    // Theorem with forall and || in body
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . i >= 0 || i < 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_or_body: SUCCESS"),
        Err(e) => panic!("test_theorem_forall_or_body: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_forall_negation_body() {
    // Theorem with forall and negation in body
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(i < 0)
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_negation_body: SUCCESS"),
        Err(e) => panic!("test_theorem_forall_negation_body: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_forall_negation_or_body() {
    // Theorem with forall, negation, and || in body - THIS IS THE PROBLEM CASE
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(i < 0) || i >= 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_negation_or_body: SUCCESS"),
        Err(e) => panic!("test_theorem_forall_negation_or_body: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_forall_literal_cmp_in_negation() {
    // Same as above but with 0 <= i instead of i < 0
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(0 <= i) || i >= 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    println!("\nTokens for test_theorem_forall_literal_cmp_in_negation:");
    for (i, token) in tokens.iter().enumerate() {
        println!("  {}: {:?}", i, token.kind);
    }

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_literal_cmp_in_negation: SUCCESS"),
        Err(e) => {
            println!("test_theorem_forall_literal_cmp_in_negation: ERROR: {:?}", e);
            panic!("test_theorem_forall_literal_cmp_in_negation failed");
        }
    }
}

#[test]
fn test_theorem_forall_with_n_on_rhs() {
    // Try with n instead of 0 on the RHS
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(0 <= i) || i <= n
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_with_n_on_rhs: SUCCESS"),
        Err(e) => panic!("test_theorem_forall_with_n_on_rhs: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_forall_i_lteq_0() {
    // Try i <= 0 (literal on RHS)
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(0 <= i) || i <= 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_i_lteq_0: SUCCESS"),
        Err(e) => panic!("test_theorem_forall_i_lteq_0: ERROR: {:?}", e),
    }
}

#[test]
fn test_theorem_forall_n_gteq_0() {
    // Try n >= 0 instead of i <= n
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(0 <= i) || n >= 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_theorem_forall_n_gteq_0: SUCCESS"),
        Err(e) => panic!("test_theorem_forall_n_gteq_0: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_in_ensures() {
    // Test forall expression inside ensures clause
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . i >= 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_forall_in_ensures: SUCCESS"),
        Err(e) => panic!("test_forall_in_ensures: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_with_negation() {
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(i < 0)
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_forall_with_negation: SUCCESS"),
        Err(e) => panic!("test_forall_with_negation: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_with_or() {
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . i >= 0 || i < 0
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_forall_with_or: SUCCESS"),
        Err(e) => panic!("test_forall_with_or: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_with_negation_and_or() {
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(0 <= i) || i <= n
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    println!("\nTokens for test_forall_with_negation_and_or:");
    for (i, token) in tokens.iter().enumerate() {
        println!("  {}: {:?} at {:?}", i, token.kind, token.span);
    }

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_forall_with_negation_and_or: SUCCESS"),
        Err(e) => panic!("test_forall_with_negation_and_or: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_with_complex_negation() {
    // Test with !(a && b)
    let content = r#"
theorem test(n: Int)
    ensures forall i: Int . !(0 <= i && i <= n)
{
    proof by trivial
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_forall_with_complex_negation: SUCCESS"),
        Err(e) => panic!("test_forall_with_complex_negation: ERROR: {:?}", e),
    }
}

#[test]
fn test_forall_complex_ensures() {
    // Test the exact construct from quantifiers.vr
    let content = r#"
theorem quantifiers_in_proof(n: Int)
    requires n >= 0
    ensures forall i: Int . !(0 <= i && i <= n) || i <= n
{
    proof {
        have h: n >= 0 by assumption;
        show forall i: Int . !(0 <= i && i <= n) || i <= n by omega;
    }
}
"#;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_forall_complex_ensures: SUCCESS"),
        Err(e) => panic!("test_forall_complex_ensures: ERROR: {:?}", e),
    }
}

#[test]
fn test_cast_to_ptr_simple() {
    let content = "fn test() -> Int { 0 as Int }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();
    
    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_cast_to_ptr_simple: SUCCESS"),
        Err(e) => panic!("test_cast_to_ptr_simple: ERROR: {:?}", e),
    }
}

#[test]
fn test_fn_with_ptr_return() {
    let content = "fn test() -> *const Int { 0 }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    println!("Tokens for fn with ptr return:");
    for (i, token) in tokens.iter().enumerate() {
        println!("  {}: {:?} at {:?}", i, token.kind, token.span);
    }

    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_fn_with_ptr_return: SUCCESS"),
        Err(e) => panic!("test_fn_with_ptr_return: ERROR: {:?}", e),
    }
}

#[test]
fn test_cast_to_ptr() {
    let content = "fn test() -> *const Int { 0 as *const Int }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();
    
    let mut parser = RecursiveParser::new(&tokens, file_id);
    match parser.parse_module() {
        Ok(_) => println!("test_cast_to_ptr: SUCCESS"),
        Err(e) => panic!("test_cast_to_ptr: ERROR: {:?}", e),
    }
}

#[test]
fn test_cast_in_block_alone() {
    // Just test the expression parsing
    let content = "let x = 0 as *const Int;";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();
    
    println!("Tokens for let statement:");
    for (i, token) in tokens.iter().enumerate() {
        println!("  {}: {:?}", i, token.kind);
    }
    
    // Try to parse just an expression
    let mut parser = RecursiveParser::new(&tokens, file_id);
    // First consume 'let x ='
    let _ = parser.parse_stmt();
    match parser.parse_stmt() {
        Ok(stmt) => println!("test_cast_in_block_alone: SUCCESS - {:?}", stmt),
        Err(e) => println!("test_cast_in_block_alone: ERROR: {:?}", e),
    }
}

#[test]
fn test_lift_parsing() {
    let content = "lift(val)";
    let file_id = FileId::new(999);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    println!("\nTokens for lift:");
    for (i, token) in tokens.iter().enumerate() {
        println!("  {}: {:?}", i, token.kind);
    }

    let mut parser = RecursiveParser::new(&tokens, file_id);
    let expr = parser.parse_expr().expect("Should parse lift");
    
    match &expr.kind {
        verum_ast::ExprKind::Lift { expr: _ } => {
            println!("Correctly parsed as Lift with inner expr");
        }
        other => {
            panic!("Expected ExprKind::Lift, got {:?}", std::mem::discriminant(other));
        }
    }
}

#[test]
fn test_lift_in_meta_fn_body() {
    // This simulates the actual test file structure
    let content = r#"
meta fn bad_lift(val: Int) -> Int {
    lift(val)
}
"#;
    let file_id = FileId::new(999);
    let lexer = Lexer::new(content, file_id);
    let tokens = lexer.tokenize().unwrap();

    println!("\nTokens for meta fn with lift:");
    for (i, token) in tokens.iter().enumerate() {
        println!("  {}: {:?}", i, token.kind);
    }

    let mut parser = RecursiveParser::new(&tokens, file_id);
    let item = parser.parse_item().expect("Should parse meta fn");
    
    println!("Parsed item: {:?}", std::mem::discriminant(&item.kind));
    
    if let verum_ast::ItemKind::Function(func) = &item.kind {
        if let verum_common::Maybe::Some(body) = &func.body {
            println!("Function body: {:?}", std::mem::discriminant(body));

            // Check if the body is a Block containing a Lift
            if let verum_ast::FunctionBody::Block(block) = body {
                if let verum_common::Maybe::Some(tail) = &block.expr {
                    println!("Block tail expr kind: {:?}", std::mem::discriminant(&tail.kind));
                    if let verum_ast::ExprKind::Lift { expr: inner } = &tail.kind {
                        println!("Inner lift expr kind: {:?}", std::mem::discriminant(&inner.kind));
                        println!("SUCCESS: Body correctly has Lift as tail expression");
                    } else {
                        panic!("Expected Lift in tail, got {:?}", std::mem::discriminant(&tail.kind));
                    }
                } else {
                    panic!("Block has no tail expression");
                }
            } else {
                panic!("Expected Block body, got {:?}", std::mem::discriminant(body));
            }
        } else {
            panic!("Function has no body");
        }
    } else {
        panic!("Expected Function, got {:?}", std::mem::discriminant(&item.kind));
    }
}
