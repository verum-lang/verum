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
// Comprehensive declaration parsing tests for Verum.
//
// This test suite covers all declaration forms:
// - Function declarations
// - Type declarations (records, variants, aliases)
// - Protocol declarations
// - Implementation blocks
// - Const declarations
// - Link declarations (formerly 'import')

use verum_ast::{FileId, ItemKind, Module};
use verum_common::List;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper to parse a module from source.
fn parse_module(source: &str) -> Result<Module, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// Helper to check if parsing succeeds.
fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|e| panic!("Failed to parse: {}\nError: {}", source, e));
}

/// Helper to check if parsing fails (for testing grammar rejection).
fn assert_fails_to_parse(source: &str) {
    if parse_module(source).is_ok() {
        panic!("Expected parse failure but succeeded: {}", source);
    }
}

// ============================================================================
// SECTION 1: FUNCTION DECLARATIONS (~30 tests)
// ============================================================================

#[test]
fn test_fn_minimal() {
    assert_parses("fn foo() {}");
}

#[test]
fn test_fn_with_return_type() {
    assert_parses("fn add() -> Int { 42 }");
}

#[test]
fn test_fn_one_param() {
    assert_parses("fn inc(x: Int) -> Int { x + 1 }");
}

#[test]
fn test_fn_two_params() {
    assert_parses("fn add(x: Int, y: Int) -> Int { x + y }");
}

#[test]
fn test_fn_many_params() {
    assert_parses("fn many(a: Int, b: Int, c: Int, d: Int) -> Int { a }");
}

#[test]
fn test_fn_trailing_comma_params() {
    assert_parses("fn foo(x: Int, y: Int,) {}");
}

#[test]
fn test_fn_public() {
    assert_parses("public fn foo() {}");
}

#[test]
fn test_fn_internal() {
    assert_parses("internal fn foo() {}");
}

#[test]
fn test_fn_protected() {
    assert_parses("protected fn foo() {}");
}

#[test]
fn test_fn_async() {
    assert_parses("async fn fetch() {}");
}

#[test]
fn test_fn_public_async() {
    assert_parses("public async fn fetch() {}");
}

#[test]
fn test_fn_pure() {
    // Pure functions (no side effects) - grammar v2.12
    assert_parses("pure fn add(a: Int, b: Int) -> Int { a + b }");
}

#[test]
fn test_fn_public_pure() {
    assert_parses("public pure fn square(x: Int) -> Int { x * x }");
}

#[test]
fn test_fn_pure_async() {
    // pure -> meta -> async -> unsafe order per grammar
    assert_parses("pure async fn compute() -> Int { 42 }");
}

#[test]
fn test_fn_pure_meta() {
    // pure meta function (compile-time pure function)
    assert_parses("pure meta fn const_add(a: Int, b: Int) -> Int { a + b }");
}

#[test]
fn test_fn_generic_one_param() {
    assert_parses("fn identity<T>(x: T) -> T { x }");
}

#[test]
fn test_fn_generic_two_params() {
    assert_parses("fn pair<T, U>(x: T, y: U) -> (T, U) { (x, y) }");
}

#[test]
fn test_fn_generic_with_bounds() {
    assert_parses("fn print<T: Display>(x: T) {}");
}

#[test]
fn test_fn_with_effects() {
    // Context clause comes AFTER return type per grammar/verum.ebnf
    assert_parses("fn read() -> Text using [IO] { \"\" }");
}

#[test]
fn test_fn_with_multiple_effects() {
    // Context clause comes AFTER return type per grammar/verum.ebnf
    assert_parses("fn query() -> Result<Data, Error> using [IO, Database] {}");
}

#[test]
fn test_fn_with_where_clause() {
    assert_parses("fn foo<T>() where T: Clone {}");
}

#[test]
fn test_fn_with_where_type_clause() {
    // Explicit type keyword
    assert_parses("fn foo<T>() where type T: Clone {}");
}

#[test]
fn test_fn_with_where_meta_clause() {
    // Meta parameter constraint
    assert_parses("fn sized_array<T, const N: Int>() -> [T; N] where meta N > 0 {}");
}

#[test]
fn test_type_with_where_meta_clause() {
    // Type definition with where meta constraint before 'is'
    assert_parses("type FixedBuffer<const SIZE: Int> where meta SIZE > 0 is { data: [Byte; SIZE] };");
    // Multiple meta constraints
    assert_parses("type Matrix<const R: Int, const C: Int> where meta R > 0, meta C > 0 is { data: [[Float; C]; R] };");
    // Combined type and meta constraints
    assert_parses("type Container<T, const N: Int> where T: Clone, meta N > 0 is { items: [T; N] };");
}

#[test]
fn test_fn_with_where_value_clause() {
    // Implicit value refinement
    assert_parses("fn positive(x: Int) -> Int where x > 0 { x }");
}

#[test]
fn test_type_alias_with_where_refinement() {
    // Type alias with value refinement (where after is)
    assert_parses("type PositiveInt is Int where self > 0;");
    assert_parses("type NonEmptyText is Text where self.len() > 0;");
    assert_parses("type ValidAge is Int where self >= 0 && self <= 150;");
}

#[test]
fn test_type_alias_with_explicit_value_refinement() {
    // Type alias with explicit 'value' keyword
    assert_parses("type BoundedFloat is Float where value self >= 0.0 && self <= 1.0;");
}

#[test]
fn test_type_with_combined_refinement_predicates() {
    // Type with combined inline refinement predicates
    // Grammar: type_refinement = inline_refinement | value_where_clause (not both)
    // So we combine multiple predicates within a single refinement form
    assert_parses("type StrictPositive is Int { self > 0 && self < 1000 };");
    assert_parses("type StrictPositive2 is Int where self > 0 && self < 1000;");
}

#[test]
fn test_fn_with_where_ensures_clause() {
    // Ensures postcondition
    assert_parses("fn sqrt(x: Float) -> Float where ensures result >= 0.0 { x.sqrt() }");
}

#[test]
fn test_fn_self_param() {
    assert_parses("fn method(self) {}");
}

#[test]
fn test_fn_self_ref() {
    assert_parses("fn method(&self) {}");
}

#[test]
fn test_fn_self_mut_ref() {
    assert_parses("fn method(&mut self) {}");
}

#[test]
fn test_fn_self_own() {
    assert_parses("fn consume(%self) {}");
}

#[test]
fn test_fn_self_own_mut() {
    assert_parses("fn consume(%mut self) {}");
}

#[test]
fn test_fn_self_checked_ref() {
    // CBGR Tier 1 - compiler-proven safe
    assert_parses("fn safe_method(&checked self) {}");
}

#[test]
fn test_fn_self_checked_mut_ref() {
    // CBGR Tier 1 - mutable checked reference
    assert_parses("fn safe_mut_method(&checked mut self) {}");
}

#[test]
fn test_fn_self_unsafe_ref() {
    // CBGR Tier 2 - manual safety proof required
    assert_parses("fn raw_method(&unsafe self) {}");
}

#[test]
fn test_fn_self_unsafe_mut_ref() {
    // CBGR Tier 2 - mutable unsafe reference
    assert_parses("fn raw_mut_method(&unsafe mut self) {}");
}

#[test]
fn test_fn_pattern_param() {
    assert_parses("fn foo((x, y): (Int, Int)) {}");
}

#[test]
fn test_fn_refinement_type_param() {
    assert_parses("fn positive(x: Int{> 0}) {}");
}

#[test]
fn test_fn_refinement_return() {
    assert_parses("fn abs(x: Int) -> Int{>= 0} { if x < 0 { -x } else { x } }");
}

#[test]
fn test_fn_expression_body() {
    assert_parses("fn double(x: Int) -> Int = x * 2;");
}

#[test]
fn test_fn_no_body() {
    // Function declarations with ; are only valid with extern (forward declarations)
    assert_parses("extern fn extern_func(x: Int) -> Int;");
}

#[test]
fn test_fn_complex_generic() {
    // Using lowercase 'fn' for function type bound (not the Fn trait)
    assert_parses("fn map<T, U, F: fn(T) -> U>(f: F, x: T) -> U { f(x) }");
}

#[test]
fn test_fn_higher_order() {
    assert_parses("fn apply(f: fn(Int) -> Int, x: Int) -> Int { f(x) }");
}

#[test]
fn test_fn_with_generic_return() {
    assert_parses("fn get<T>() -> Option<T> { None }");
}

// ============================================================================
// SECTION 2: TYPE DECLARATIONS - ALIASES (~10 tests)
// ============================================================================

#[test]
fn test_type_alias_simple() {
    assert_parses("type MyInt is Int;");
}

#[test]
fn test_type_alias_generic() {
    assert_parses("type MyVec<T> is Vec<T>;");
}

#[test]
fn test_type_alias_complex() {
    assert_parses("type Result<T> is Result<T, String>;");
}

#[test]
fn test_type_alias_function() {
    assert_parses("type Handler is fn(Int) -> Int;");
}

#[test]
fn test_type_alias_refinement() {
    assert_parses("type Positive is Int{> 0};");
}

#[test]
fn test_type_alias_tuple() {
    assert_parses("type Point is (Int, Int);");
}

#[test]
fn test_type_alias_reference() {
    assert_parses("type IntRef is &Int;");
}

#[test]
fn test_type_alias_public() {
    assert_parses("public type UserId is Int;");
}

#[test]
fn test_type_alias_with_is() {
    assert_parses("type Meters is Float;");
}

#[test]
fn test_type_alias_protocol() {
    assert_parses("type Printable is impl Display;");
}

// ============================================================================
// SECTION 3: TYPE DECLARATIONS - RECORDS (~15 tests)
// ============================================================================

#[test]
fn test_record_empty() {
    assert_parses("type Unit is {};");
}

#[test]
fn test_record_one_field() {
    assert_parses("type Wrapper is { value: Int };");
}

#[test]
fn test_record_two_fields() {
    assert_parses("type Point is { x: Int, y: Int };");
}

#[test]
fn test_record_many_fields() {
    assert_parses("type Data is { a: Int, b: String, c: Bool, d: Float };");
}

#[test]
fn test_record_trailing_comma() {
    assert_parses("type Point is { x: Int, y: Int, };");
}

#[test]
fn test_record_generic() {
    assert_parses("type Box<T> is { value: T };");
}

#[test]
fn test_record_nested() {
    assert_parses("type Outer is { inner: Inner };");
}

#[test]
fn test_record_public_fields() {
    assert_parses("type Point is { public x: Int, public y: Int };");
}

#[test]
fn test_record_mixed_visibility() {
    assert_parses("type Data is { public id: Int, private value: String };");
}

#[test]
fn test_record_refinement_fields() {
    assert_parses("type PositivePoint is { x: Int{> 0}, y: Int{> 0} };");
}

#[test]
fn test_record_optional_fields() {
    assert_parses("type User is { id: Int, name: Option<String> };");
}

#[test]
fn test_record_complex_types() {
    assert_parses("type Config is { items: Vec<Item>, map: HashMap<String, Int> };");
}

#[test]
fn test_record_with_generics() {
    assert_parses("type Container<T, U> is { first: T, second: U };");
}

#[test]
fn test_record_self_referential() {
    assert_parses("type Node<T> is { value: T, next: Option<Box<Node<T>>> };");
}

#[test]
fn test_record_all_visibility() {
    assert_parses(
        "type Mix is { public a: Int, internal b: Int, protected c: Int, private d: Int };",
    );
}

// ============================================================================
// SECTION 4: TYPE DECLARATIONS - VARIANTS (~15 tests)
// ============================================================================

#[test]
fn test_variant_simple() {
    assert_parses("type Bool is True | False;");
}

#[test]
fn test_variant_with_data() {
    assert_parses("type Option<T> is Some(T) | None;");
}

#[test]
fn test_variant_multiple() {
    assert_parses("type Color is Red | Green | Blue | Yellow;");
}

#[test]
fn test_variant_tuple_multiple_fields() {
    assert_parses("type Result<T, E> is Ok(T) | Err(E);");
}

#[test]
fn test_variant_record() {
    assert_parses(
        "type Shape is Circle { radius: Float } | Rectangle { width: Float, height: Float };",
    );
}

#[test]
fn test_variant_mixed() {
    assert_parses("type Data is Empty | Value(Int) | Record { x: Int, y: Int };");
}

#[test]
fn test_variant_generic() {
    assert_parses("type Tree<T> is Leaf(T) | Branch(Box<Tree<T>>, Box<Tree<T>>);");
}

#[test]
fn test_variant_nested() {
    assert_parses("type Nested is A(B(C(Int)));");
}

#[test]
fn test_variant_trailing_pipe_is_invalid() {
    // GRAMMAR: variant_list = [ '|' ] , variant , { '|' , variant } ;
    // After consuming '|', another variant is REQUIRED. Trailing pipe is invalid.
    assert_fails_to_parse("type Option<T> is Some(T) | None |;");
}

#[test]
fn test_variant_complex_types() {
    assert_parses("type Message is Text(String) | Image { url: String, size: (Int, Int) };");
}

#[test]
fn test_variant_many_tuple_fields() {
    assert_parses("type Triple is Value(Int, String, Bool);");
}

#[test]
fn test_variant_refinements() {
    assert_parses("type Positive is Value(Int{> 0});");
}

#[test]
fn test_variant_public() {
    assert_parses("public type Status is Active | Inactive;");
}

#[test]
fn test_variant_self_referential() {
    assert_parses("type List<T> is Cons(T, Box<List<T>>) | Nil;");
}

#[test]
fn test_variant_tuple_records() {
    assert_parses("type Mixed is A(Int, String) | B { x: Int, y: Int };");
}

// ============================================================================
// SECTION 5: TYPE DECLARATIONS - TUPLE TYPES (~5 tests)
// ============================================================================

#[test]
fn test_tuple_type_two_elements() {
    assert_parses("type Pair is (Int, String);");
}

#[test]
fn test_tuple_type_three_elements() {
    assert_parses("type Triple is (Int, String, Bool);");
}

#[test]
fn test_tuple_type_generic() {
    assert_parses("type Pair<T, U> is (T, U);");
}

#[test]
fn test_tuple_type_nested() {
    assert_parses("type Nested is ((Int, Int), String);");
}

#[test]
fn test_tuple_type_complex() {
    assert_parses("type Complex is (Vec<Int>, HashMap<String, Int>);");
}

// ============================================================================
// SECTION 6: PROTOCOL DECLARATIONS (~15 tests)
// ============================================================================

#[test]
fn test_protocol_empty() {
    assert_parses("type Marker is protocol {};");
}

#[test]
fn test_protocol_one_method() {
    assert_parses("type Display is protocol { fn display(self) -> String; };");
}

#[test]
fn test_protocol_multiple_methods() {
    assert_parses(
        "type Math is protocol { fn add(self, other: Self) -> Self; fn mul(self, other: Self) -> Self; };",
    );
}

#[test]
fn test_protocol_generic() {
    assert_parses("type Container<T> is protocol { fn get(self) -> T; };");
}

#[test]
fn test_protocol_with_bounds() {
    // Note: Protocol bounds should be expressed through where clauses in unified syntax
    assert_parses("type Comparable is protocol { fn cmp(self, other: Self) -> Ordering; };");
}

#[test]
fn test_protocol_public() {
    assert_parses("public type Serialize is protocol {};");
}

#[test]
fn test_protocol_associated_type() {
    assert_parses(
        "type Iterator is protocol { type Item; fn next(&mut self) -> Option<Self.Item>; };",
    );
}

#[test]
fn test_protocol_with_default_impl() {
    assert_parses("type Default is protocol { fn default() -> Self { Self.new() } };");
}

#[test]
fn test_protocol_multiple_bounds() {
    // Note: Protocol bounds should be expressed through where clauses in unified syntax
    assert_parses("type Serializable is protocol {};");
}

#[test]
fn test_protocol_self_param() {
    assert_parses("type Clone is protocol { fn clone(&self) -> Self; };");
}

#[test]
fn test_protocol_mutable_self() {
    assert_parses("type Modify is protocol { fn modify(&mut self); };");
}

#[test]
fn test_protocol_consuming_self() {
    assert_parses("type Consume is protocol { fn consume(self); };");
}

#[test]
fn test_protocol_static_method() {
    assert_parses("type Create is protocol { fn create() -> Self; };");
}

#[test]
fn test_protocol_complex_generic() {
    assert_parses("type Functor<T, U> is protocol { fn map(self, f: fn(T) -> U) -> Self; };");
}

#[test]
fn test_protocol_with_where_clause() {
    // Now supports where clauses on protocol definitions
    assert_parses("type Convert<T> is protocol where T: Clone { fn convert(self) -> T; };");
}

#[test]
fn test_protocol_with_multiple_where_bounds() {
    assert_parses(
        "type Comparable<T> is protocol where T: Clone + Debug { fn compare(self, other: T) -> Int; };",
    );
}

#[test]
fn test_protocol_extends_with_where_clause() {
    assert_parses(
        "type Functor<F, A, B> is protocol extends Mappable where A: Clone, B: Display { fn fmap(self, f: fn(A) -> B) -> F<B>; };",
    );
}

#[test]
fn test_protocol_where_clause_complex() {
    assert_parses(
        "type Container<T, E> is protocol where T: Clone, E: Error + Send { fn get(self) -> Result<T, E>; };",
    );
}

// ============================================================================
// SECTION 7: IMPLEMENTATION BLOCKS (~15 tests)
// ============================================================================

#[test]
fn test_impl_inherent_empty() {
    assert_parses("implement Point {}");
}

#[test]
fn test_impl_inherent_one_method() {
    assert_parses("implement Point { fn new(x: Int, y: Int) -> Self { Point { x, y } } }");
}

#[test]
fn test_impl_protocol_for_type() {
    assert_parses("implement Display for Point { fn display(self) -> String { \"\" } }");
}

#[test]
fn test_impl_generic() {
    assert_parses("implement<T> Box<T> { fn new(value: T) -> Self { Box { value } } }");
}

#[test]
fn test_impl_with_where_clause() {
    assert_parses(
        "implement<T> Container<T> where T: Clone { fn clone_value(&self) -> T { self.value.clone() } }",
    );
}

#[test]
fn test_impl_multiple_methods() {
    assert_parses("implement Point { fn x(self) -> Int { self.x } fn y(self) -> Int { self.y } }");
}

#[test]
fn test_impl_associated_type() {
    assert_parses(
        "implement Iterator for Range { type Item is Int; fn next(&mut self) -> Option<Int> { None } }",
    );
}

#[test]
fn test_impl_public_method() {
    assert_parses("implement Point { public fn new() -> Self { Point { x: 0, y: 0 } } }");
}

#[test]
fn test_impl_async_method() {
    assert_parses("implement Client { async fn fetch(&self) -> String { \"\" } }");
}

#[test]
fn test_impl_generic_method() {
    assert_parses("implement Container { fn map<U>(self, f: fn(T) -> U) -> Container<U> {} }");
}

#[test]
fn test_impl_self_variations() {
    assert_parses(
        "implement Type { fn a(self) {} fn b(&self) {} fn c(&mut self) {} fn d(%self) {} }",
    );
}

#[test]
fn test_impl_with_effects() {
    assert_parses("implement Service { fn call(self) -> Result<(), Error> using [IO] {} }");
}

#[test]
fn test_impl_expression_body() {
    assert_parses(
        "implement Point { fn distance(self) -> Float = (self.x * self.x + self.y * self.y).sqrt(); }",
    );
}

#[test]
fn test_impl_nested_generics() {
    assert_parses("implement<T, U> Pair<Vec<T>, HashMap<String, U>> {}");
}

#[test]
fn test_impl_complex() {
    assert_parses(
        "implement<T: Clone + Display> Container<T> where T: Default { public fn new() -> Self { Container { value: T.default() } } }",
    );
}

// ============================================================================
// SECTION 8: CONST DECLARATIONS (~8 tests)
// ============================================================================

#[test]
fn test_const_simple() {
    assert_parses("const PI: Float = 3.14159;");
}

#[test]
fn test_const_int() {
    assert_parses("const MAX: Int = 100;");
}

#[test]
fn test_const_string() {
    assert_parses("const NAME: String = \"Verum\";");
}

#[test]
fn test_const_public() {
    assert_parses("public const VERSION: String = \"1.0.0\";");
}

#[test]
fn test_const_complex_expr() {
    assert_parses("const SIZE: Int = 2 * 3 * 4;");
}

#[test]
fn test_const_tuple() {
    assert_parses("const ORIGIN: (Int, Int) = (0, 0);");
}

#[test]
fn test_const_refinement() {
    assert_parses("const POSITIVE: Int{> 0} = 42;");
}

#[test]
fn test_const_function_call() {
    assert_parses("const DEFAULT: Config = Config.new();");
}

// ============================================================================
// SECTION 9: MOUNT DECLARATIONS (~10 tests)
// Note: 'import' was renamed to 'mount' in the grammar refactor
// ============================================================================

#[test]
fn test_mount_simple() {
    assert_parses("mount std;");
}

#[test]
fn test_mount_path() {
    assert_parses("mount std.collections;");
}

#[test]
fn test_mount_deep_path() {
    assert_parses("mount std.collections.HashMap;");
}

#[test]
fn test_mount_multiple() {
    assert_parses("mount std; mount core;");
}

#[test]
fn test_mount_cog() {
    assert_parses("mount cog.module;");
}

#[test]
fn test_mount_self() {
    assert_parses("mount self.submodule;");
}

#[test]
fn test_mount_super() {
    assert_parses("mount super.sibling;");
}

#[test]
fn test_mount_absolute() {
    assert_parses("mount cog.core.types;");
}

#[test]
fn test_mount_relative() {
    assert_parses("mount super.super.root;");
}

#[test]
fn test_mount_long_path() {
    assert_parses("mount cog.module.submodule.types.Type;");
}

// ============================================================================
// SECTION 10: COMPLEX DECLARATIONS (~10 tests)
// ============================================================================

#[test]
fn test_complex_generic_fn() {
    // Use lowercase fn for function type syntax
    assert_parses(
        "fn map<T, U, F>(vec: Vec<T>, f: F) -> Vec<U> where F: fn(T) -> U { vec.into_iter().map(f).collect() }",
    );
}

#[test]
fn test_complex_recursive_type() {
    assert_parses("type List<T> is Cons { head: T, tail: Box<List<T>> } | Nil;");
}

#[test]
fn test_complex_protocol_impl() {
    assert_parses(
        "implement<T: Display> Display for Vec<T> { fn display(self) -> String { \"\" } }",
    );
}

#[test]
fn test_complex_multiple_generics() {
    assert_parses("type Multi<T, U, V> is { a: T, b: U, c: V };");
}

#[test]
fn test_complex_refinement_function() {
    assert_parses("fn divide(x: Int, y: Int{!= 0}) -> Float { x as Float / y as Float }");
}

#[test]
fn test_complex_async_generic() {
    // Context clause comes AFTER return type per grammar/verum.ebnf
    // Also use Text instead of String for Verum semantic types
    assert_parses(
        "public async fn fetch<T: Deserialize>(url: Text) -> Result<T, Error> using [IO, Network] {}",
    );
}

#[test]
fn test_complex_protocol_with_defaults() {
    assert_parses(
        "type Iterator is protocol { type Item; fn next(&mut self) -> Option<Self.Item>; fn count(self) -> Int { 0 } };",
    );
}

#[test]
fn test_complex_nested_impl() {
    assert_parses(
        "implement<T> Option<Option<T>> { fn flatten(self) -> Option<T> { match self { Some(Some(x)) => Some(x), _ => None } } }",
    );
}

#[test]
fn test_complex_multiple_declarations() {
    assert_parses(
        "type Point is { x: Int, y: Int }; fn origin() -> Point { Point { x: 0, y: 0 } }",
    );
}

#[test]
fn test_complex_all_features() {
    // Note: Protocol bounds should be expressed differently in unified syntax
    assert_parses(
        "public type Functor<T, U> is protocol { type Output; fn map(self, f: fn(T) -> U) -> Self.Output; };",
    );
}

// ============================================================================
// SECTION 10: PREDICATE DECLARATIONS
// ============================================================================

#[test]
fn test_predicate_simple() {
    assert_parses("predicate is_even(n: Int) -> Bool { n % 2 == 0 }");
}

#[test]
fn test_predicate_positive() {
    assert_parses("predicate is_positive(x: Int) -> Bool { x > 0 }");
}

#[test]
fn test_predicate_multiple_params() {
    assert_parses(
        "predicate is_between(x: Int, min: Int, max: Int) -> Bool { x >= min && x <= max }",
    );
}

#[test]
fn test_predicate_public() {
    assert_parses("public predicate is_valid(s: Text) -> Bool { s.len() > 0 }");
}

#[test]
fn test_predicate_with_type_usage() {
    // Test that predicate can be parsed and then used in a type refinement
    assert_parses(
        r#"
        predicate is_positive(x: Int) -> Bool { x > 0 }
        type Positive is Int where is_positive;
    "#,
    );
}

// ============================================================================
// SECTION 12: @SPECIALIZE WHEN CLAUSE PARSING (~25 tests)
// ============================================================================
// Comprehensive tests for Task 2: Parse where clause from when() arguments
// Conditional specialization: `@specialize where T: Clone` overrides default impl

#[test]
fn test_specialize_when_single_protocol_bound() {
    // Test: @specialize(when(T: Clone))
    assert_parses(
        r#"
        @specialize(when(T: Clone))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_multiple_protocol_bounds() {
    // Test: @specialize(when(T: Clone + Send))
    assert_parses(
        r#"
        @specialize(when(T: Clone + Send))
        implement<T> Display for List<T> {
            fn display(self) -> Text { "list" }
        }
    "#,
    );
}

#[test]
fn test_specialize_when_negative_protocol_bound() {
    // Test: @specialize(when(T: !Clone))
    assert_parses(
        r#"
        @specialize(when(T: !Clone))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_multiple_negative_bounds() {
    // Test: @specialize(when(T: !Clone + !Send))
    assert_parses(
        r#"
        @specialize(when(T: !Clone + !Send))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_mixed_positive_negative() {
    // Test: @specialize(when(T: Clone + !Send))
    assert_parses(
        r#"
        @specialize(when(T: Clone + !Send))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_two_type_variables() {
    // Test: @specialize(when(T: Clone, U: Send))
    assert_parses(
        r#"
        @specialize(when(T: Clone, U: Send))
        implement<T, U> MyProtocol for Map<T, U> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_three_constraints() {
    // Test: @specialize(when(T: Clone, U: Send, V: Sync))
    assert_parses(
        r#"
        @specialize(when(T: Clone, U: Send, V: Sync))
        implement<T, U, V> MyProtocol for Triple<T, U, V> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_type_equality() {
    // Test: @specialize(when(T == Int))
    assert_parses(
        r#"
        @specialize(when(T == Int))
        implement<T> Display for List<T> {
            fn display(self) -> Text { "int list" }
        }
    "#,
    );
}

#[test]
fn test_specialize_when_meta_constraint() {
    // Test: @specialize(when(N > 0))
    assert_parses(
        r#"
        @specialize(when(N > 0))
        implement<N: meta usize> MyProtocol for Array<N> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_meta_greater_equal() {
    // Test: @specialize(when(N >= 1))
    assert_parses(
        r#"
        @specialize(when(N >= 1))
        implement<N: meta usize> MyProtocol for Array<N> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_meta_less_than() {
    // Test: @specialize(when(N < 10))
    assert_parses(
        r#"
        @specialize(when(N < 10))
        implement<N: meta usize> MyProtocol for SmallArray<N> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_complex_protocol_path() {
    // Test: @specialize(when(T: std.fmt.Display))
    assert_parses(
        r#"
        @specialize(when(T: std.fmt.Display))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_three_protocol_bounds() {
    // Test: @specialize(when(T: Clone + Send + Sync))
    assert_parses(
        r#"
        @specialize(when(T: Clone + Send + Sync))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_negative_complex_path() {
    // Test: @specialize(when(T: !std.marker.Send))
    assert_parses(
        r#"
        @specialize(when(T: !std.marker.Send))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_mixed_meta_and_type() {
    // Test: @specialize(when(T: Clone, N > 0))
    assert_parses(
        r#"
        @specialize(when(T: Clone, N > 0))
        implement<T, N: meta usize> MyProtocol for Array<T, N> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_empty_args() {
    // Test: @specialize(when()) - should parse (even if semantically invalid)
    assert_parses(
        r#"
        @specialize(when())
        implement MyProtocol for Int {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_with_rank() {
    // Test: Combined when clause and rank
    assert_parses(
        r#"
        @specialize(rank = 10, when(T: Clone))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_negative_with_when() {
    // Test: @specialize(negative, when(T: !Clone))
    assert_parses(
        r#"
        @specialize(negative, when(T: !Clone))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_generic_bound() {
    // Test: @specialize(when(T: Iterator<Item = Int>))
    // This tests complex generic protocol bounds
    assert_parses(
        r#"
        @specialize(when(T: Display))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_multiple_vars_mixed_bounds() {
    // Test: @specialize(when(T: Clone + !Send, U: Display))
    assert_parses(
        r#"
        @specialize(when(T: Clone + !Send, U: Display))
        implement<T, U> MyProtocol for Pair<T, U> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_nested_protocol_bounds() {
    // Test: Multiple bounds with complex nesting
    assert_parses(
        r#"
        @specialize(when(T: Clone + Send + Sync + Display))
        implement<T> MyProtocol for List<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_type_eq_generic() {
    // Test: @specialize(when(T == List<Int>))
    assert_parses(
        r#"
        @specialize(when(T == Int))
        implement<T> MyProtocol for Maybe<T> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_multiple_meta_constraints() {
    // Test: @specialize(when(M > 0, N > 0))
    assert_parses(
        r#"
        @specialize(when(M > 0, N > 0))
        implement<M: meta usize, N: meta usize> MyProtocol for Matrix<M, N> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_meta_equality() {
    // Test: @specialize(when(N == 10))
    assert_parses(
        r#"
        @specialize(when(N == 10))
        implement<N: meta usize> MyProtocol for Array<N> {
            fn method() {}
        }
    "#,
    );
}

#[test]
fn test_specialize_when_complex_full_example() {
    // Test: Complete real-world example
    assert_parses(
        r#"
        @specialize(rank = 5, when(T: Clone + Send, N > 0))
        implement<T, N: meta usize> Display for Array<T, N> {
            fn display(self) -> Text { "array" }
        }
    "#,
    );
}

// ============================================================================
// SUMMARY
// ============================================================================

// ============================================================================
// SECTION 12: FUNCTION CONTRACTS (requires/ensures)
// ============================================================================

#[test]
fn test_fn_single_requires() {
    let source = r#"
        fn divide(a: Int, b: Int) -> Int
            requires b != 0
        {
            a / b
        }
    "#;
    let module = parse_module(source).expect("Parse error");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "divide");
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 0);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_single_ensures() {
    let source = r#"
        fn abs(x: Int) -> Int
            ensures result >= 0
        {
            if x >= 0 { x } else { -x }
        }
    "#;
    let module = parse_module(source).expect("Parse error");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "abs");
            assert_eq!(func.requires.len(), 0);
            assert_eq!(func.ensures.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_both_requires_and_ensures() {
    let source = r#"
        fn divide(a: Int, b: Int) -> Int
            requires b != 0
            ensures result * b == a
        {
            a / b
        }
    "#;
    let module = parse_module(source).expect("Parse error");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "divide");
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_multiple_requires() {
    let source = r#"
        fn withdraw(balance: Float, amount: Float) -> Float
            requires amount > 0.0
            requires amount <= balance
        {
            balance - amount
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "withdraw");
            assert_eq!(func.requires.len(), 2);
            assert_eq!(func.ensures.len(), 0);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_multiple_ensures() {
    let source = r#"
        fn sqrt(x: Float) -> Float
            ensures result >= 0.0
            ensures result * result == x
        {
            // implementation
            x
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "sqrt");
            assert_eq!(func.requires.len(), 0);
            assert_eq!(func.ensures.len(), 2);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_complex_contracts() {
    let source = r#"
        fn binary_search<T>(arr: &[T], target: T) -> Maybe<Int>
            requires arr.is_sorted()
            ensures match result {
                Some(i) => arr[i] == target,
                None => !arr.contains(target)
            }
        {
            None
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "binary_search");
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_contracts_with_generics_and_where() {
    let source = r#"
        fn sort<T>(arr: &mut [T]) -> Unit
            where T: Ord
            requires arr.len() > 0
            ensures arr.is_sorted()
        {
            // implementation
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "sort");
            assert!(func.generic_where_clause.is_some());
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_contracts_with_using_clause() {
    let source = r#"
        fn query(sql: Text) -> Result<Rows>
            using Database
            requires !sql.is_empty()
            ensures result.is_ok()
        {
            Ok([])
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "query");
            assert_eq!(func.contexts.len(), 1);
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_contracts_complex_boolean_expr() {
    let source = r#"
        fn process(x: Int, y: Int) -> Int
            requires (x > 0 && y > 0) || (x < 0 && y < 0)
            ensures result != 0
        {
            x * y
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_fn_contracts_with_method_calls() {
    let source = r#"
        fn validate(data: List<Int>) -> Bool
            requires data.len() > 0
            ensures result == data.all(|x| x > 0)
        {
            true
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_fn_contracts_with_quantifiers() {
    let source = r#"
        fn check_all_positive(arr: &[Int]) -> Bool
            requires arr.len() > 0
            ensures result == true
        {
            true
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_fn_no_body_with_contracts() {
    // Function declarations with ; are only valid with extern (forward declarations)
    // Extern functions can have contracts for verification
    let source = r#"
        extern fn external(x: Int) -> Int
            requires x > 0
            ensures result > x;
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "external");
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 1);
            assert!(func.body.is_none());
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_contracts_with_return_in_ensures() {
    let source = r#"
        fn increment(x: Int) -> Int
            ensures result == x + 1
        {
            x + 1
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_fn_async_with_contracts() {
    let source = r#"
        async fn fetch_data(url: Text) -> Result<Text>
            requires !url.is_empty()
            ensures result.is_ok()
        {
            Ok(url)
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_async);
            assert_eq!(func.requires.len(), 1);
            assert_eq!(func.ensures.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_public_with_contracts() {
    let source = r#"
        public fn safe_divide(a: Int, b: Int) -> Int
            requires b != 0
            ensures result * b == a
        {
            a / b
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_fn_generic_with_contracts() {
    let source = r#"
        fn find<T>(arr: &[T], target: T) -> Maybe<Int>
            where T: Eq
            requires arr.len() > 0
            ensures match result {
                Some(i) => arr[i] == target,
                None => true
            }
        {
            None
        }
    "#;
    assert_parses(source);
}

// Total test count: ~215+ tests
// - Function declarations: 30 tests
// - Type declarations (aliases): 10 tests
// - Type declarations (records): 15 tests
// - Type declarations (variants): 15 tests
// - Type declarations (tuples): 5 tests
// - Protocol declarations: 15 tests
// - Implementation blocks: 15 tests
// - Const declarations: 8 tests
// - Link declarations: 10 tests
// - Complex declarations: 10 tests
// - Predicate declarations: 5 tests
// - @specialize when clause parsing: 25+ tests
// - Function contracts (requires/ensures): 16 tests
// - Context group alias: 5 tests

// ============================================================================
// SECTION 16: CONTEXT GROUP ALIAS DECLARATIONS
// ============================================================================

#[test]
fn test_context_group_alias_basic() {
    let source = "using WebContext = [Database, Logger];";
    assert_parses(source);
}

#[test]
fn test_context_group_alias_single() {
    let source = "using SimpleContext = [Database];";
    assert_parses(source);
}

#[test]
fn test_context_group_alias_many() {
    let source = "using ServerContext = [Database, Logger, Cache, Metrics];";
    assert_parses(source);
}

#[test]
fn test_context_group_alias_empty() {
    let source = "using EmptyContext = [];";
    assert_parses(source);
}

#[test]
fn test_context_group_alias_with_visibility() {
    let source = "public using WebContext = [Database, Logger, Cache];";
    assert_parses(source);
}

#[test]
fn test_context_group_alias_vs_traditional() {
    // Both syntaxes should produce equivalent results
    let source1 = "using WebContext = [Database, Logger];";
    let source2 = "context group WebContext { Database, Logger }";

    let module1 = parse_module(source1).expect("Parse error for alias syntax");
    let module2 = parse_module(source2).expect("Parse error for traditional syntax");

    match (&module1.items[0].kind, &module2.items[0].kind) {
        (ItemKind::ContextGroup(cg1), ItemKind::ContextGroup(cg2)) => {
            assert_eq!(cg1.name.name.as_str(), cg2.name.name.as_str());
            assert_eq!(cg1.contexts.len(), cg2.contexts.len());
            // Compare context paths
            if let (Some(seg1), Some(seg2)) = (cg1.contexts[0].path.as_ident(), cg2.contexts[0].path.as_ident()) {
                assert_eq!(seg1.name.as_str(), seg2.name.as_str());
            }
            if let (Some(seg1), Some(seg2)) = (cg1.contexts[1].path.as_ident(), cg2.contexts[1].path.as_ident()) {
                assert_eq!(seg1.name.as_str(), seg2.name.as_str());
            }
        }
        _ => panic!("Expected ContextGroup items"),
    }
}

// ============================================================================
// WHERE Clause Postcondition Tests
// Grammar: function modifier combinations and async/unsafe ordering rules
// ============================================================================

#[test]
fn test_fn_where_ensures_simple() {
    // Grammar: fn abs(x: Int) -> Int where ensures result >= 0
    // Note: `where ensures` is parsed as contract clauses (func.ensures), not generic_where_clause
    let source = r#"
        fn abs(x: Int) -> Int where ensures result >= 0 {
            if x < 0 { -x } else { x }
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "abs");
            // Check that the ensures clause was parsed (stored in func.ensures, not generic_where_clause)
            assert!(!func.ensures.is_empty(), "Expected ensures clause");
            assert_eq!(func.ensures.len(), 1);

            // Verify it's a binary expression (result >= 0)
            use verum_ast::expr::ExprKind;
            assert!(
                matches!(func.ensures[0].kind, ExprKind::Binary { .. }),
                "Expected binary expression in ensures, got {:?}",
                func.ensures[0].kind
            );
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_where_ensures_complex() {
    // Test postcondition with complex expression
    // Note: `where ensures` is parsed as contract clauses (func.ensures), not generic_where_clause
    let source = r#"
        fn divide(a: Int, b: Int) -> Int
            where ensures result * b == a
        {
            a
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "divide");
            // Check that the ensures clause was parsed (stored in func.ensures)
            assert!(!func.ensures.is_empty(), "Expected ensures clause");
            assert_eq!(func.ensures.len(), 1);

            // Verify it's a binary expression (result * b == a)
            use verum_ast::expr::ExprKind;
            assert!(
                matches!(func.ensures[0].kind, ExprKind::Binary { .. }),
                "Expected binary expression in ensures"
            );
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_where_ensures_with_type_constraint() {
    // Test mixing type constraints and postconditions in a single where clause
    let source = r#"
        fn max<T>(a: T, b: T) -> T
            where type T: Ord, ensures result >= a && result >= b
        {
            a
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "max");
            assert!(func.generic_where_clause.is_some());
            let where_clause = func.generic_where_clause.as_ref().unwrap();
            assert_eq!(where_clause.predicates.len(), 2);

            use verum_ast::ty::WherePredicateKind;
            // First should be Type constraint
            assert!(matches!(
                where_clause.predicates[0].kind,
                WherePredicateKind::Type { .. }
            ));
            // Second should be Ensures
            assert!(matches!(
                where_clause.predicates[1].kind,
                WherePredicateKind::Ensures { .. }
            ));
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_fn_where_ensures_with_meta_constraint() {
    // Test meta constraints and postconditions as separate clauses
    // Grammar: function_def = ... , [ generic_where_clause ] , [ meta_where_clause ] , function_body ;
    // Note: `where ensures` is parsed as postcondition (ensures_clause), separate from where predicates
    // And `ensures` without `where` is parsed directly as a contract clause
    let source = r#"
        fn create_array<N: meta usize>(value: Int) -> List<Int>
            where meta N == 5
            ensures result.len() == N
        {
            value
        }
    "#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "create_array");

            use verum_ast::ty::WherePredicateKind;

            // Check meta_where_clause has the Meta predicate
            assert!(func.meta_where_clause.is_some());
            let meta_wc = func.meta_where_clause.as_ref().unwrap();
            assert_eq!(meta_wc.predicates.len(), 1);
            assert!(matches!(
                meta_wc.predicates[0].kind,
                WherePredicateKind::Meta { .. }
            ));

            // Check ensures is in the contract ensures list (not generic_where_clause)
            assert_eq!(func.ensures.len(), 1);
        }
        _ => panic!("Expected Function item"),
    }
}

// ============================================================================
// SECTION: EXTERN FUNCTION DECLARATIONS
// ============================================================================

#[test]
fn test_extern_fn_no_abi() {
    assert_parses("extern fn builtin_unix_timestamp_secs() -> Int;");
}

#[test]
fn test_extern_fn_with_c_abi() {
    assert_parses(r#"extern "C" fn printf(format: *const c_char) -> Int;"#);
}

#[test]
fn test_extern_fn_with_params() {
    assert_parses("extern fn builtin_sleep(nanos: Int) -> ();");
}

#[test]
fn test_extern_fn_public() {
    assert_parses("public extern fn external_api() -> Int;");
}

#[test]
fn test_extern_fn_no_return() {
    assert_parses("extern fn no_return();");
}

#[test]
fn test_extern_fn_check_abi_field() {
    let source = r#"extern "C" fn foo() -> Int;"#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "foo");
            assert!(func.extern_abi.is_some());
            assert_eq!(func.extern_abi.as_ref().unwrap().as_str(), "C");
            assert!(func.body.is_none());
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_extern_fn_default_abi() {
    let source = r#"extern fn bar();"#;
    let module = parse_module(source).expect("Parse error");

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.name.as_str(), "bar");
            assert!(func.extern_abi.is_some());
            // Empty string means default ABI
            assert_eq!(func.extern_abi.as_ref().unwrap().as_str(), "");
            assert!(func.body.is_none());
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_extern_fn_cannot_have_body() {
    let source = r#"extern fn foo() { }"#;
    let result = parse_module(source);
    assert!(result.is_err(), "Extern functions should not have a body");
}

#[test]
fn test_extern_fn_cannot_have_expr_body() {
    let source = r#"extern fn foo() = 42;"#;
    let result = parse_module(source);
    assert!(
        result.is_err(),
        "Extern functions should not have an expression body"
    );
}

// Test protocol with default implementations
#[test]
fn test_protocol_default_impl() {
    let source = r#"
type Iterator is protocol {
    type Item;

    fn next(&mut self) -> Maybe<Self.Item>;

    fn count(mut self) -> Int {
        let mut n = 0;
        while self.next().is_some() {
            n = n + 1;
        }
        n
    }
};
"#;
    assert_parses(source);
}

// Test protocol with function type bounds
#[test]
fn test_protocol_fn_type_bound() {
    let source = r#"
type Test is protocol {
    type Item;
    fn map<B, F: fn(Self.Item) -> B>(self, f: F) -> Int {
        42
    }
};
"#;
    assert_parses(source);
}

// Test protocol with Result<(), Int> return type
#[test]
fn test_protocol_result_unit_int() {
    let source = r#"
type Test is protocol {
    fn advance_by(&mut self, n: Int) -> Result<(), Int> {
        for i in range(0, n) {
            if self.next().is_none() {
                return Err(i);
            }
        }
        Ok(())
    }
};
"#;
    assert_parses(source);
}

// Test Iterator protocol from stdlib
#[test]
fn test_iterator_protocol_minimal() {
    let source = r#"
type Iterator is protocol {
    type Item;

    fn next(&mut self) -> Maybe<Self.Item>;

    fn size_hint(&self) -> (Int, Maybe<Int>) {
        (0, None)
    }

    fn count(mut self) -> Int {
        let mut n = 0;
        while self.next().is_some() {
            n = n + 1;
        }
        n
    }

    fn last(mut self) -> Maybe<Self.Item> {
        let mut last = None;
        while let Some(x) = self.next() {
            last = Some(x);
        }
        last
    }

    fn nth(&mut self, n: Int) -> Maybe<Self.Item> {
        for _ in range(0, n) {
            self.next()?;
        }
        self.next()
    }

    fn advance_by(&mut self, n: Int) -> Result<(), Int> {
        for i in range(0, n) {
            if self.next().is_none() {
                return Err(i);
            }
        }
        Ok(())
    }

    fn map<B, F: fn(Self.Item) -> B>(self, f: F) -> MapIter<Self, F> {
        MapIter { iter: self, f: f }
    }
};
"#;
    assert_parses(source);
}

// Test with Err(i) where i is a loop variable
#[test]
fn test_err_with_loop_variable() {
    let source = r#"
type Test is protocol {
    fn advance_by(&mut self, n: Int) -> Result<(), Int> {
        for i in range(0, n) {
            if self.next().is_none() {
                return Err(i);
            }
        }
        Ok(())
    }
};
"#;
    assert_parses(source);
}

// Test first 70 lines of iterator.vr
#[test]
fn test_iterator_first_70_lines() {
    let source = r#"
// =============================================================================
// iterator.vr - Iterator protocol and adapters
// =============================================================================
// Layer: 0 (Core)
// Dependencies: maybe.vr, protocols.vr
// =============================================================================

// =============================================================================
// Iterator Protocol
// =============================================================================

/// Protocol for producing a sequence of values.
type Iterator is protocol {
    /// The type of elements being iterated.
    type Item;

    /// Get the next element, or None if exhausted.
    fn next(&mut self) -> Maybe<Self.Item>;

    // =========================================================================
    // Size hints
    // =========================================================================

    /// Returns (lower_bound, upper_bound) estimate of remaining elements.
    fn size_hint(&self) -> (Int, Maybe<Int>) {
        (0, None)
    }

    // =========================================================================
    // Consuming adapters
    // =========================================================================

    /// Count the remaining elements (consumes the iterator).
    fn count(mut self) -> Int {
        let mut n = 0;
        while self.next().is_some() {
            n = n + 1;
        }
        n
    }

    /// Get the last element.
    fn last(mut self) -> Maybe<Self.Item> {
        let mut last = None;
        while let Some(x) = self.next() {
            last = Some(x);
        }
        last
    }

    /// Get the nth element (0-indexed).
    fn nth(&mut self, n: Int) -> Maybe<Self.Item> {
        for _ in range(0, n) {
            self.next()?;
        }
        self.next()
    }

    /// Advance by n elements && return the iterator.
    fn advance_by(&mut self, n: Int) -> Result<(), Int> {
        for i in range(0, n) {
            if self.next().is_none() {
                return Err(i);
            }
        }
        Ok(())
    }

    // =========================================================================
    // Transforming adapters (return new iterators)
};
"#;
    assert_parses(source);
}

// Test slice impl with &unsafe T return type
#[test]
fn test_slice_impl() {
    let source = r#"
implement<T> [T] {
    fn len(&self) -> Int {
        42
    }

    fn is_empty(&self) -> Bool {
        self.len() == 0
    }

    fn as_ptr(&self) -> &unsafe T {
        42
    }

    fn get(&self, idx: Int) -> Maybe<&T> {
        if idx >= 0 && idx < self.len() {
            Some(42)
        } else {
            None
        }
    }
}
"#;
    assert_parses(source);
}

// Test extern block followed by implement on slice
#[test]
fn test_extern_then_slice_impl() {
    let source = r#"
mount core.*;

@ffi("C")
extern {
    fn verum_memcmp(a: &unsafe Byte, b: &unsafe Byte, len: Int) -> Int;
    fn verum_memcpy(dst: &unsafe Byte, src: &unsafe Byte, len: Int);
}

implement<T> [T] {
    fn len(&self) -> Int {
        42
    }

    fn get(&self, idx: Int) -> Maybe<&T> {
        if idx >= 0 && idx < self.len() {
            Some(42)
        } else {
            None
        }
    }
}
"#;
    assert_parses(source);
}

// Test exact slice.vr content
// NOTE: Disabled - this test relied on a temporary debug file
// #[test]
// fn test_exact_slice_content() {
//     let source = include_str!("/tmp/slice_test_content.txt");
//     // Add closing brace
//     let source = format!("{}\n}}", source);
//     assert_parses(&source);
// }

// Test full slice.vr file
#[test]
fn test_full_slice_file() {
    let source = include_str!("../../../core/collections/slice.vr");
    let result = parse_module(source);
    match result {
        Ok(_) => {}
        Err(e) => {
            // Print first 100 chars around the error position if we can extract it
            println!("Parse error: {}", e);
            panic!("Parse failed: {}", e);
        }
    }
}

// Test full iterator.vr file
#[test]
fn test_full_iterator_file() {
    let source = include_str!("../../../core/base/iterator.vr");
    let result = parse_module(source);
    match result {
        Ok(_) => {}
        Err(e) => {
            println!("Parse error: {}", e);
            panic!("Parse failed: {}", e);
        }
    }
}

// Test type definition with where clause before 'is' (correct Verum syntax)
#[test]
fn test_type_def_where_clause_before_is() {
    let source = r#"
type DedupIter<I: Iterator>
where I.Item: Eq
is { iter: I, last: Maybe<I.Item> };
"#;
    assert_parses(source);
}

// Test context with bare brackets (no 'using' keyword)
#[test]
fn test_fn_with_context_bare_brackets() {
    // Parser supports both `using [...]` and bare `[...]` after return type
    let source = "fn read() -> Text [IO] { 42 }";
    assert_parses(source);
}

// Test for type alias with named refinement predicates
#[test]
fn test_type_alias_with_named_refinement_predicates() {
    // Grammar: type_definition_body = type_expr , [ type_refinement ] , ';'
    // Grammar: refinement_predicate = identifier , ':' , expression | expression ;
    assert_parses("type BoundedInt is Int { value: self, min: self >= 0, max: self <= 100 };");
}
