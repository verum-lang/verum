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
//! Comprehensive coverage tests for verum_fast_parser.
//!
//! Tests grammar productions that may be undertested:
//! - Empty input and minimal programs
//! - Unicode identifiers
//! - All literal types (hex, octal, binary, float suffixes)
//! - Rank-2 polymorphic function types
//! - Select expressions
//! - Nursery (structured concurrency)
//! - Provide statements
//! - Context definitions
//! - Advanced pattern matching
//! - Three-tier references
//! - Where clauses with multiple predicates
//! - Operator precedence edge cases
//! - Deeply nested generics
//! - Try/errdefer

use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module_ok(input: &str) {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok(), "Failed to parse module:\n{}\nError: {:?}", input, result.err());
}

fn parse_expr_ok(input: &str) {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(input, file_id);
    assert!(result.is_ok(), "Failed to parse expression:\n{}\nError: {:?}", input, result.err());
}

fn parse_module_err(input: &str) {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Should either fail outright or produce diagnostics
    // Some malformed input may still parse with error recovery
    let _ = result;
}

// ============================================================================
// 1. EMPTY AND MINIMAL INPUT
// ============================================================================

#[test]
fn test_empty_input() {
    let file_id = FileId::new(0);
    let lexer = Lexer::new("", file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    // Empty input should parse as an empty module
    assert!(result.is_ok());
}

#[test]
fn test_whitespace_only() {
    let file_id = FileId::new(0);
    let lexer = Lexer::new("   \n\t\n   ", file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_comment_only() {
    parse_module_ok("// just a comment\n");
}

#[test]
fn test_block_comment_only() {
    parse_module_ok("/* block comment */");
}

#[test]
fn test_minimal_function() {
    parse_module_ok("fn f() {}");
}

#[test]
fn test_minimal_type() {
    parse_module_ok("type T is ();");
}

// ============================================================================
// 2. UNICODE IDENTIFIERS
// ============================================================================

#[test]
fn test_unicode_identifier_latin() {
    parse_module_ok("fn caf\u{00E9}() {}");
}

#[test]
fn test_unicode_identifier_greek() {
    parse_expr_ok("\u{03B1} + \u{03B2}");
}

#[test]
fn test_underscore_identifier() {
    parse_module_ok("fn _private() { let _unused = 1; }");
}

#[test]
fn test_identifier_with_numbers() {
    parse_module_ok("fn test123() { let x1 = 1; let y_2 = 2; }");
}

// ============================================================================
// 3. LITERAL TYPES
// ============================================================================

#[test]
fn test_hexadecimal_literal() {
    parse_expr_ok("0xFF");
}

#[test]
fn test_hex_with_underscores() {
    parse_expr_ok("0xFF_FF_FF");
}

#[test]
fn test_octal_literal() {
    parse_expr_ok("0o777");
}

#[test]
fn test_binary_literal() {
    parse_expr_ok("0b1010_1010");
}

#[test]
fn test_float_with_exponent() {
    parse_expr_ok("1.0e10");
}

#[test]
fn test_float_negative_exponent() {
    parse_expr_ok("3.14e-2");
}

#[test]
fn test_integer_with_suffix_i8() {
    parse_expr_ok("42i8");
}

#[test]
fn test_integer_with_suffix_u64() {
    parse_expr_ok("100u64");
}

#[test]
fn test_float_with_suffix_f32() {
    parse_expr_ok("1.5f32");
}

#[test]
fn test_large_integer() {
    parse_expr_ok("9999999999999999999");
}

#[test]
fn test_string_with_escapes() {
    parse_expr_ok(r#""hello\nworld\t\"escaped\"""#);
}

#[test]
fn test_byte_string() {
    parse_expr_ok(r#"b"hello""#);
}

#[test]
fn test_format_string_simple() {
    parse_expr_ok(r#"f"x = {x}""#);
}

#[test]
fn test_format_string_expression() {
    parse_expr_ok(r#"f"result = {a + b}""#);
}

// ============================================================================
// 4. TYPE DEFINITIONS
// ============================================================================

#[test]
fn test_record_type() {
    parse_module_ok("type Point is { x: Float, y: Float };");
}

#[test]
fn test_sum_type() {
    parse_module_ok("type Option<T> is None | Some(T);");
}

#[test]
fn test_newtype() {
    parse_module_ok("type UserId is (Int);");
}

#[test]
fn test_unit_type_definition() {
    parse_module_ok("type Marker is ();");
}

#[test]
fn test_protocol_type() {
    parse_module_ok("type Iterator is protocol { type Item; fn next(&mut self) -> Maybe<Self.Item>; };");
}

#[test]
fn test_generic_type_multiple_params() {
    parse_module_ok("type Map<K, V> is { entries: List<(K, V)> };");
}

#[test]
fn test_function_with_where_clause() {
    parse_module_ok("fn sort<T>(items: List<T>) -> List<T> where T: Ord {}");
}

#[test]
fn test_function_with_multiple_where_predicates() {
    parse_module_ok("fn display_sorted<T>(items: List<T>) where T: Ord + Display {}");
}

#[test]
fn test_complex_sum_type() {
    parse_module_ok("type Tree<T> is Leaf(T) | Node { left: Heap<Tree<T>>, right: Heap<Tree<T>> };");
}

#[test]
fn test_type_alias() {
    parse_module_ok("type Name is Text;");
}

// ============================================================================
// 5. RANK-2 POLYMORPHIC FUNCTION TYPES
// ============================================================================

#[test]
fn test_rank2_function_type_in_record() {
    parse_module_ok("type Transducer<A, B> is { transform: fn<R>(fn(R, B) -> R) -> fn(R, A) -> R };");
}

#[test]
fn test_rank2_function_parameter() {
    parse_module_ok("fn apply_forall(f: fn<T>(T) -> T) -> Int { f(42) }");
}

// ============================================================================
// 6. THREE-TIER REFERENCES
// ============================================================================

#[test]
fn test_tier0_reference() {
    parse_module_ok("fn f(x: &Int) -> Int { 0 }");
}

#[test]
fn test_tier0_mut_reference() {
    parse_module_ok("fn f(x: &mut Int) { }");
}

#[test]
fn test_tier1_checked_reference() {
    parse_module_ok("fn f(x: &checked Int) -> Int { 0 }");
}

#[test]
fn test_tier2_unsafe_reference() {
    parse_module_ok("fn f(x: &unsafe Int) -> Int { 0 }");
}

// ============================================================================
// 7. IMPLEMENT BLOCKS
// ============================================================================

#[test]
fn test_simple_implement() {
    parse_module_ok("implement Point { fn origin() -> Self { Point { x: 0.0, y: 0.0 } } }");
}

#[test]
fn test_implement_protocol() {
    parse_module_ok("implement Display for Point { fn fmt(&self) -> Text { f\"({self.x}, {self.y})\" } }");
}

#[test]
fn test_implement_generic() {
    parse_module_ok("implement<T> Container<T> { fn new() -> Self { Container { items: [] } } }");
}

// ============================================================================
// 8. CONTEXT SYSTEM
// ============================================================================

#[test]
fn test_context_definition() {
    parse_module_ok("context Database { fn query(&self, sql: Text) -> List<Map<Text, Text>>; }");
}

#[test]
fn test_using_clause() {
    parse_module_ok("fn get_users() -> List<Text> using [Database] { Database.query(\"SELECT name\") }");
}

#[test]
fn test_provide_statement() {
    parse_module_ok("fn main() { provide Database = SqliteDb.new(\"test.db\"); }");
}

#[test]
fn test_provide_in_block() {
    parse_module_ok("fn main() { provide Logger = StdLogger.new() in { Logger.info(\"hello\"); } }");
}

// ============================================================================
// 9. MOUNT STATEMENTS
// ============================================================================

#[test]
fn test_simple_mount() {
    parse_module_ok("mount std.io;");
}

#[test]
fn test_mount_selective() {
    parse_module_ok("mount std.collections.{List, Map, Set};");
}

#[test]
fn test_mount_alias() {
    parse_module_ok("mount std.io.File as IoFile;");
}

#[test]
fn test_mount_super() {
    parse_module_ok("mount super.utils;");
}

// ============================================================================
// 10. CONTROL FLOW
// ============================================================================

#[test]
fn test_if_else() {
    parse_module_ok("fn f(x: Int) -> Int { if x > 0 { x } else { 0 } }");
}

#[test]
fn test_if_else_if_chain() {
    parse_module_ok("fn f(x: Int) -> Int { if x > 0 { 1 } else if x < 0 { -1 } else { 0 } }");
}

#[test]
fn test_match_basic() {
    parse_module_ok("fn f(x: Int) -> Text { match x { 0 => \"zero\", 1 => \"one\", _ => \"other\" } }");
}

#[test]
fn test_match_with_guard() {
    parse_module_ok("fn f(x: Int) -> Text { match x { n if n > 0 => \"positive\", _ => \"non-positive\" } }");
}

#[test]
fn test_while_loop() {
    parse_module_ok("fn f() { let mut x = 0; while x < 10 { x = x + 1; } }");
}

#[test]
fn test_for_loop() {
    parse_module_ok("fn f() { for i in 0..10 { print(i); } }");
}

#[test]
fn test_loop_break_continue() {
    parse_module_ok("fn f() -> Int { let mut x = 0; loop { x = x + 1; if x > 10 { break x; } continue; } }");
}

// ============================================================================
// 11. ASYNC/AWAIT
// ============================================================================

#[test]
fn test_async_function() {
    parse_module_ok("async fn fetch(url: Text) -> Text { \"response\" }");
}

#[test]
fn test_await_expression() {
    parse_module_ok("async fn main() { let data = fetch(\"http://example.com\").await; }");
}

#[test]
fn test_spawn_expression() {
    parse_module_ok("async fn main() { let handle = spawn fetch(\"url\"); }");
}

// ============================================================================
// 12. SELECT EXPRESSIONS
// ============================================================================

#[test]
fn test_select_basic() {
    parse_module_ok(r#"async fn race() {
    select {
        val = ch1.recv().await => { print(val); }
        val = ch2.recv().await => { print(val); }
    }
}"#);
}

// ============================================================================
// 13. TRY AND ERROR HANDLING
// ============================================================================

#[test]
fn test_try_expression() {
    parse_module_ok("fn f() -> Result<Int, Text> { try { let x = risky()?; Ok(x) } }");
}

#[test]
fn test_question_mark_operator() {
    parse_module_ok("fn f() -> Result<Int, Text> { let x = might_fail()?; Ok(x) }");
}

#[test]
fn test_defer_statement() {
    parse_module_ok("fn f() { let f = open(\"file.txt\"); defer close(f); }");
}

// ============================================================================
// 14. PATTERN MATCHING ADVANCED
// ============================================================================

#[test]
fn test_tuple_pattern() {
    parse_module_ok("fn f(pair: (Int, Int)) { let (a, b) = pair; }");
}

#[test]
fn test_nested_pattern() {
    parse_module_ok("fn f(x: Option<Option<Int>>) { match x { Some(Some(v)) => v, _ => 0 } }");
}

#[test]
fn test_or_pattern() {
    parse_module_ok("fn f(x: Int) -> Bool { match x { 1 | 2 | 3 => true, _ => false } }");
}

#[test]
fn test_rest_pattern() {
    parse_module_ok("fn f(list: List<Int>) { let [first, ..rest] = list; }");
}

#[test]
fn test_is_pattern_test() {
    parse_module_ok("fn f(x: Option<Int>) -> Bool { x is Some(_) }");
}

// ============================================================================
// 15. ATTRIBUTES
// ============================================================================

#[test]
fn test_derive_attribute() {
    parse_module_ok("@derive(Eq, Hash, Debug)\ntype Point is { x: Float, y: Float };");
}

#[test]
fn test_cfg_attribute() {
    parse_module_ok("@cfg(target_os = \"linux\")\nfn linux_only() {}");
}

#[test]
fn test_repr_attribute() {
    parse_module_ok("@repr(C)\ntype CStruct is { a: Int, b: Float };");
}

// ============================================================================
// 16. CLOSURES AND LAMBDAS
// ============================================================================

#[test]
fn test_closure_simple() {
    parse_expr_ok("fn(x: Int) -> Int { x + 1 }");
}

#[test]
fn test_closure_short() {
    parse_expr_ok("|x| x + 1");
}

#[test]
fn test_closure_no_params() {
    parse_expr_ok("|| 42");
}

#[test]
fn test_closure_multi_params() {
    parse_expr_ok("|a, b, c| a + b + c");
}

// ============================================================================
// 17. OPERATOR PRECEDENCE EDGE CASES
// ============================================================================

#[test]
fn test_mixed_arithmetic_precedence() {
    // a + b * c should be a + (b * c)
    parse_expr_ok("a + b * c");
}

#[test]
fn test_comparison_chain() {
    parse_expr_ok("a < b && b < c");
}

#[test]
fn test_bitwise_vs_comparison() {
    parse_expr_ok("a & b == c");
}

#[test]
fn test_unary_minus_precedence() {
    parse_expr_ok("-a * b");
}

#[test]
fn test_not_and_or_precedence() {
    parse_expr_ok("!a && b || c");
}

#[test]
fn test_method_chain() {
    parse_expr_ok("x.foo().bar().baz()");
}

#[test]
fn test_index_chain() {
    parse_expr_ok("x[0][1][2]");
}

#[test]
fn test_method_and_field_mixed() {
    parse_expr_ok("x.field.method().another_field");
}

// ============================================================================
// 18. GENERICS EDGE CASES
// ============================================================================

#[test]
fn test_nested_generics() {
    parse_module_ok("type Nested is List<Map<Text, List<Int>>>;");
}

#[test]
fn test_deeply_nested_generics() {
    parse_module_ok("type Deep is Option<Result<List<Map<Text, Set<Int>>>, Text>>;");
}

#[test]
fn test_generic_function_call() {
    parse_expr_ok("parse::<Int>(\"42\")");
}

// ============================================================================
// 19. REFINEMENT TYPES
// ============================================================================

#[test]
fn test_refinement_type_basic() {
    parse_module_ok("type Positive is Int where it > 0;");
}

#[test]
fn test_refinement_type_range() {
    parse_module_ok("type Percentage is Int where it >= 0 && it <= 100;");
}

// ============================================================================
// 20. MODULE DECLARATION
// ============================================================================

#[test]
fn test_module_declaration() {
    parse_module_ok("module my_module;");
}

#[test]
fn test_pub_function() {
    parse_module_ok("pub fn api() -> Int { 42 }");
}

#[test]
fn test_visibility_modifiers() {
    parse_module_ok("pub type PublicType is Int;\npub fn public_fn() {}\nfn private_fn() {}");
}

// ============================================================================
// 21. FFI
// ============================================================================

#[test]
fn test_ffi_function() {
    parse_module_ok("@ffi(\"C\")\nfn malloc(size: Int) -> &unsafe Byte;");
}

// ============================================================================
// 22. RANGE EXPRESSIONS
// ============================================================================

#[test]
fn test_range_exclusive() {
    parse_expr_ok("0..10");
}

#[test]
fn test_range_inclusive() {
    parse_expr_ok("0..=10");
}

// ============================================================================
// 23. PIPELINE OPERATOR
// ============================================================================

#[test]
fn test_pipeline() {
    parse_expr_ok("data |> filter(|x| x > 0) |> map(|x| x * 2)");
}

// ============================================================================
// 24. LIST AND MAP LITERALS
// ============================================================================

#[test]
fn test_empty_list() {
    parse_expr_ok("[]");
}

#[test]
fn test_list_literal() {
    parse_expr_ok("[1, 2, 3]");
}

#[test]
fn test_map_literal() {
    parse_expr_ok("{\"a\": 1, \"b\": 2}");
}

// ============================================================================
// 25. DEEPLY NESTED EXPRESSIONS
// ============================================================================

#[test]
fn test_deeply_nested_parens() {
    parse_expr_ok("((((((((((1))))))))))");
}

#[test]
fn test_deeply_nested_if() {
    parse_module_ok("fn f(x: Int) -> Int { if x > 5 { if x > 10 { if x > 15 { 3 } else { 2 } } else { 1 } } else { 0 } }");
}

// ============================================================================
// 26. MULTIPLE DECLARATIONS
// ============================================================================

#[test]
fn test_multiple_functions() {
    parse_module_ok(r#"
fn add(a: Int, b: Int) -> Int { a + b }
fn sub(a: Int, b: Int) -> Int { a - b }
fn mul(a: Int, b: Int) -> Int { a * b }
fn div(a: Int, b: Int) -> Int { a / b }
"#);
}

#[test]
fn test_mixed_declarations() {
    parse_module_ok(r#"
type Color is Red | Green | Blue;
fn color_name(c: Color) -> Text {
    match c {
        Red => "red",
        Green => "green",
        Blue => "blue",
    }
}
implement Display for Color {
    fn fmt(&self) -> Text { color_name(self) }
}
"#);
}

// ============================================================================
// 27. AS CAST EXPRESSIONS
// ============================================================================

#[test]
fn test_as_cast() {
    parse_expr_ok("x as Float");
}

#[test]
fn test_chained_cast() {
    parse_expr_ok("x as Float as Int");
}

// ============================================================================
// 28. CONST AND STATIC
// ============================================================================

#[test]
fn test_const_declaration() {
    parse_module_ok("const MAX_SIZE: Int = 1024;");
}

#[test]
fn test_static_declaration() {
    parse_module_ok("static COUNTER: Int = 0;");
}

// ============================================================================
// 29. RETURN AND BREAK WITH VALUES
// ============================================================================

#[test]
fn test_return_value() {
    parse_module_ok("fn f() -> Int { return 42; }");
}

#[test]
fn test_return_unit() {
    parse_module_ok("fn f() { return; }");
}

// ============================================================================
// 30. COMPLEX REAL-WORLD PATTERNS
// ============================================================================

#[test]
fn test_method_with_generic_return() {
    parse_module_ok(r#"
implement<T> List<T> {
    pub fn first(&self) -> Maybe<&T> {
        if self.len() > 0 {
            Some(&self[0])
        } else {
            None
        }
    }
}
"#);
}

#[test]
fn test_nested_match_with_bindings() {
    parse_module_ok(r#"
fn process(input: Result<Option<Int>, Text>) -> Int {
    match input {
        Ok(Some(value)) if value > 0 => value,
        Ok(Some(value)) => 0,
        Ok(None) => -1,
        Err(msg) => -2,
    }
}
"#);
}
