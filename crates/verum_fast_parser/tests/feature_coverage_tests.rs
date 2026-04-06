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
//! Feature coverage tests for under-tested Verum language features.
//!
//! This file covers:
//! - Context system: using clauses, provide statements, scoped provide
//! - CBGR three-tier references: &T, &checked T, &unsafe T
//! - Async/await: async fn, .await, select, spawn
//! - Pattern matching: nested, guards, or-patterns, binding
//! - Module system: mount, module declarations

use verum_ast::{FileId, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Helper to parse a complete module.
fn parse_module(source: &str) -> Result<Module, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Helper to check if parsing succeeds.
fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|e| panic!("Failed to parse:\n{}\nErrors:\n{}", source, e));
}

// ============================================================================
// SECTION 1: CONTEXT SYSTEM - using clauses with function types
// ============================================================================

#[test]
fn test_context_fn_using_database_logger() {
    assert_parses(
        r#"
        fn fetch_data() using [Database, Logger] {
            let rows = Database.query("SELECT * FROM users");
            Logger.info("fetched data");
        }
    "#,
    );
}

#[test]
fn test_context_provide_scoped_block() {
    assert_parses(
        r#"
        fn main() {
            provide Database = PostgresDb.new("localhost:5432") in {
                fetch_data();
            };
        }
    "#,
    );
}

#[test]
fn test_context_provide_semicolon() {
    assert_parses(
        r#"
        fn main() {
            provide Database = PostgresDb.new("localhost:5432");
            provide Logger = ConsoleLogger.new();
            fetch_data();
        }
    "#,
    );
}

#[test]
fn test_context_fn_type_with_using() {
    // Function type parameter with using clause
    assert_parses(
        r#"
        fn run_with_db<R>(f: fn() -> R using [Database]) -> R {
            f()
        }
    "#,
    );
}

#[test]
fn test_context_provide_in_nested_scopes() {
    assert_parses(
        r#"
        fn setup() {
            provide Database = ProdDb.new() in {
                provide Logger = FileLogger.new() in {
                    run_app();
                };
            };
        }
    "#,
    );
}

#[test]
fn test_context_using_single_no_brackets() {
    assert_parses(
        r#"
        fn get_user() using Database {
            Database.query("SELECT 1");
        }
    "#,
    );
}

#[test]
fn test_context_using_with_generics() {
    assert_parses(
        r#"
        fn process<T>(items: List<T>) using [Database, Logger] {
            for item in items {
                Logger.info("processing");
            }
        }
    "#,
    );
}

#[test]
fn test_context_row_polymorphism_with_multiple_params() {
    assert_parses(
        r#"
        fn compose_contexts<C1, C2>(
            f: fn() -> Int using [C1],
            g: fn() -> Int using [C2]
        ) -> Int using [C1, C2] {
            f() + g()
        }
    "#,
    );
}

// ============================================================================
// SECTION 2: CBGR THREE-TIER REFERENCES
// ============================================================================

#[test]
fn test_cbgr_tier0_default_reference() {
    // Tier 0: &T - default, full CBGR protection (~15ns overhead)
    assert_parses(
        r#"
        fn read_value(x: &Int) -> Int {
            *x
        }
    "#,
    );
}

#[test]
fn test_cbgr_tier1_checked_reference() {
    // Tier 1: &checked T - compiler-proven safe (0ns overhead)
    assert_parses(
        r#"
        fn read_checked(x: &checked Int) -> Int {
            *x
        }
    "#,
    );
}

#[test]
fn test_cbgr_tier2_unsafe_reference() {
    // Tier 2: &unsafe T - manual safety proof required (0ns overhead)
    assert_parses(
        r#"
        fn read_unsafe(x: &unsafe Int) -> Int {
            *x
        }
    "#,
    );
}

#[test]
fn test_cbgr_all_three_tiers_in_one_function() {
    assert_parses(
        r#"
        fn mixed_refs(
            a: &Int,
            b: &checked Int,
            c: &unsafe Int
        ) -> Int {
            *a + *b + *c
        }
    "#,
    );
}

#[test]
fn test_cbgr_mut_references_all_tiers() {
    assert_parses(
        r#"
        fn mutate_refs(
            a: &mut Int,
            b: &checked mut Int,
            c: &unsafe mut Int
        ) {
            *a = 1;
            *b = 2;
            *c = 3;
        }
    "#,
    );
}

#[test]
fn test_cbgr_reference_in_return_type() {
    assert_parses(
        r#"
        fn borrow(data: &List<Int>) -> &Int {
            data.first()
        }

        fn borrow_checked(data: &checked List<Int>) -> &checked Int {
            data.first()
        }
    "#,
    );
}

#[test]
fn test_cbgr_reference_in_type_def() {
    assert_parses(
        r#"
        type Borrowed is {
            data: &Int,
            checked_data: &checked Int,
            raw_data: &unsafe Int,
        };
    "#,
    );
}

#[test]
fn test_cbgr_generic_with_references() {
    assert_parses(
        r#"
        fn apply_ref<T>(f: fn(&T) -> Int, value: &T) -> Int {
            f(value)
        }
    "#,
    );
}

// ============================================================================
// SECTION 3: ASYNC/AWAIT
// ============================================================================

#[test]
fn test_async_fn_basic_declaration() {
    assert_parses(
        r#"
        async fn fetch() -> Text {
            "hello"
        }
    "#,
    );
}

#[test]
fn test_async_fn_await_expression() {
    assert_parses(
        r#"
        async fn get_data() -> Text {
            let response = fetch().await;
            response
        }
    "#,
    );
}

#[test]
fn test_async_fn_with_using_clause() {
    assert_parses(
        r#"
        async fn fetch_user(id: Int) -> User using [Database, Logger] {
            Logger.info("fetching user");
            Database.query("SELECT * FROM users WHERE id = ?")
        }
    "#,
    );
}

#[test]
fn test_async_fn_chained_await() {
    assert_parses(
        r#"
        async fn pipeline() -> Data {
            let raw = fetch_raw().await;
            let parsed = parse(raw).await;
            let validated = validate(parsed).await;
            validated
        }
    "#,
    );
}

#[test]
fn test_async_select_expression() {
    assert_parses(
        r#"
        async fn race() -> Int {
            select {
                result = task_a().await => result,
                result = task_b().await => result + 1,
            }
        }
    "#,
    );
}

#[test]
fn test_async_spawn_expression() {
    assert_parses(
        r#"
        async fn concurrent() {
            let handle = spawn compute_heavy();
            let result = handle.await;
        }
    "#,
    );
}

#[test]
fn test_async_block() {
    assert_parses(
        r#"
        fn start() {
            let future = async {
                let x = fetch().await;
                x + 1
            };
        }
    "#,
    );
}

#[test]
fn test_async_fn_with_error_handling() {
    assert_parses(
        r#"
        async fn safe_fetch() -> Result<Text, Error> {
            let data = fetch().await?;
            Ok(data)
        }
    "#,
    );
}

// ============================================================================
// SECTION 4: PATTERN MATCHING - Advanced variants
// ============================================================================

#[test]
fn test_pattern_nested_option() {
    // Nested: Some(Some(x))
    assert_parses(
        r#"
        fn unwrap_nested(opt: Option<Option<Int>>) -> Int {
            match opt {
                Some(Some(x)) => x,
                Some(None) => 0,
                None => -1,
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_guard_condition() {
    // Guards: x if x > 0
    assert_parses(
        r#"
        fn classify(n: Int) -> Text {
            match n {
                x if x > 0 => "positive",
                x if x < 0 => "negative",
                _ => "zero",
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_or_alternatives() {
    // Or-patterns: A | B
    assert_parses(
        r#"
        fn is_weekend(day: Day) -> Bool {
            match day {
                Saturday | Sunday => true,
                _ => false,
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_at_binding() {
    // Binding: y @ Some(_)
    assert_parses(
        r#"
        fn extract(opt: Option<Int>) -> Int {
            match opt {
                y @ Some(_) => {
                    let val = y;
                    0
                },
                None => -1,
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_nested_record() {
    assert_parses(
        r#"
        fn extract_name(person: Person) -> Text {
            match person {
                Person { name: Name { first, last }, age } => first,
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_guard_with_or() {
    assert_parses(
        r#"
        fn classify(x: Int) -> Text {
            match x {
                1 | 2 | 3 if x > 1 => "small but not one",
                n if n > 100 => "big",
                _ => "other",
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_nested_tuple() {
    assert_parses(
        r#"
        fn process(data: (Int, (Text, Bool))) -> Int {
            match data {
                (x, ("hello", true)) => x,
                (x, (_, false)) => x + 1,
                _ => 0,
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_deeply_nested_variant() {
    assert_parses(
        r#"
        fn depth(tree: Tree) -> Int {
            match tree {
                Leaf(v) => 1,
                Node { left: Leaf(_), right: Leaf(_) } => 2,
                Node { left: Node { .. }, right: _ } => 3,
                _ => 0,
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_at_binding_with_guard() {
    assert_parses(
        r#"
        fn check(opt: Option<Int>) -> Int {
            match opt {
                val @ Some(x) if x > 10 => x,
                Some(x) => x,
                None => 0,
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_range_in_match() {
    assert_parses(
        r#"
        fn grade(score: Int) -> Text {
            match score {
                90..=100 => "A",
                80..=89 => "B",
                70..=79 => "C",
                _ => "F",
            }
        }
    "#,
    );
}

#[test]
fn test_pattern_let_destructuring() {
    assert_parses(
        r#"
        fn unpack() {
            let (a, b, c) = get_triple();
            let Point { x, y } = get_point();
            let Some(value) = try_get() else { return; };
        }
    "#,
    );
}

// ============================================================================
// SECTION 5: MODULE SYSTEM
// ============================================================================

#[test]
fn test_mount_wildcard() {
    // mount core.base.protocols.*
    assert_parses(
        r#"
        mount core.base.protocols.*;
    "#,
    );
}

#[test]
fn test_mount_specific_items() {
    assert_parses(
        r#"
        mount core.collections.{List, Map, Set};
    "#,
    );
}

#[test]
fn test_mount_with_alias() {
    assert_parses(
        r#"
        mount core.io.FileReader as Reader;
    "#,
    );
}

#[test]
fn test_module_declaration_with_function() {
    assert_parses(
        r#"
        module inner {
            fn helper() -> Int { 42 }
        }
    "#,
    );
}

#[test]
fn test_module_nested() {
    assert_parses(
        r#"
        module outer {
            module inner {
                fn deep() -> Int { 0 }
            }
            fn shallow() -> Int { 1 }
        }
    "#,
    );
}

#[test]
fn test_module_with_types_and_functions() {
    assert_parses(
        r#"
        module math {
            type Vector2 is { x: Float, y: Float };

            fn dot(a: Vector2, b: Vector2) -> Float {
                a.x * b.x + a.y * b.y
            }

            fn add(a: Vector2, b: Vector2) -> Vector2 {
                Vector2 { x: a.x + b.x, y: a.y + b.y }
            }
        }
    "#,
    );
}

#[test]
fn test_mount_and_module_combined() {
    assert_parses(
        r#"
        mount core.base.protocols.*;
        mount core.collections.List;

        module app {
            fn process(items: List<Int>) -> Int {
                items.len()
            }
        }
    "#,
    );
}

#[test]
fn test_module_public_visibility() {
    assert_parses(
        r#"
        pub module api {
            pub fn handler(req: Request) -> Response {
                Response.ok()
            }
        }
    "#,
    );
}

// ============================================================================
// SECTION 6: CROSS-FEATURE INTERACTIONS
// ============================================================================

#[test]
fn test_async_with_context() {
    assert_parses(
        r#"
        async fn fetch_user(id: Int) -> Result<User, Error> using [Database, Logger] {
            Logger.info("fetching");
            let user = Database.query("SELECT").await;
            Ok(user)
        }
    "#,
    );
}

#[test]
fn test_pattern_match_with_cbgr_ref() {
    assert_parses(
        r#"
        fn check_ref(data: &Option<Int>) -> Int {
            match data {
                Some(x) => *x,
                None => 0,
            }
        }
    "#,
    );
}

#[test]
fn test_async_with_pattern_match() {
    assert_parses(
        r#"
        async fn handle_result() -> Int {
            let result = fetch().await;
            match result {
                Ok(data) => data.len(),
                Err(e) => 0,
            }
        }
    "#,
    );
}

#[test]
fn test_module_with_contexts_and_async() {
    assert_parses(
        r#"
        module service {
            context Storage {
                async fn get(key: Text) -> Option<Text>;
                async fn set(key: Text, value: Text);
            }

            async fn cached_get(key: Text) -> Text using [Storage] {
                match Storage.get(key).await {
                    Some(v) => v,
                    None => "default",
                }
            }
        }
    "#,
    );
}

#[test]
fn test_provide_with_async() {
    assert_parses(
        r#"
        async fn main() {
            provide Database = PostgresDb.connect("localhost").await;
            let user = fetch_user(1).await;
        }
    "#,
    );
}

#[test]
fn test_generic_fn_with_context_and_pattern() {
    assert_parses(
        r#"
        fn transform<T, U>(input: Option<T>, f: fn(T) -> U) -> Option<U> using [Logger] {
            match input {
                Some(val) => {
                    Logger.info("transforming");
                    Some(f(val))
                },
                None => None,
            }
        }
    "#,
    );
}

// ============================================================================
// SECTION 7: TYPE DEFINITIONS WITH VARIANTS
// ============================================================================

#[test]
fn test_sum_type_definition() {
    assert_parses(
        r#"
        type Shape is
            Circle(Float)
            | Rectangle { width: Float, height: Float }
            | Triangle { a: Float, b: Float, c: Float };
    "#,
    );
}

#[test]
fn test_sum_type_with_pattern_match() {
    assert_parses(
        r#"
        type Expr is
            Literal(Int)
            | Add(Heap<Expr>, Heap<Expr>)
            | Mul(Heap<Expr>, Heap<Expr>);

        fn eval(e: Expr) -> Int {
            match e {
                Literal(n) => n,
                Add(l, r) => eval(*l) + eval(*r),
                Mul(l, r) => eval(*l) * eval(*r),
            }
        }
    "#,
    );
}

#[test]
fn test_protocol_definition() {
    assert_parses(
        r#"
        type Printable is protocol {
            fn to_string(&self) -> Text;
        };
    "#,
    );
}

#[test]
fn test_implement_block() {
    assert_parses(
        r#"
        type Counter is { value: Int };

        implement Counter {
            fn new() -> Counter {
                Counter { value: 0 }
            }

            fn increment(&mut self) {
                self.value = self.value + 1;
            }
        }
    "#,
    );
}

#[test]
fn test_implement_protocol_for_type() {
    assert_parses(
        r#"
        type Printable is protocol {
            fn to_string(&self) -> Text;
        };

        type Point is { x: Int, y: Int };

        implement Printable for Point {
            fn to_string(&self) -> Text {
                f"({self.x}, {self.y})"
            }
        }
    "#,
    );
}
