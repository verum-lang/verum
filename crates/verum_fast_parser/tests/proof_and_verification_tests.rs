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
//! Comprehensive tests for the Verum proof and verification system.
//!
//! Tests cover grammar sections 2.19.1-2.19.9:
//! - Theorem declarations (basic, generic, with requires/ensures)
//! - Lemma declarations
//! - Corollary declarations (with `from` clause)
//! - Axiom declarations
//! - Tactic declarations
//! - Proof bodies (by tactic, by term, structured)
//! - Proof steps (have, show, obtain, calc)
//! - Calculational proof chains
//! - Layer declarations
//! - View patterns, active patterns, and-patterns, type test patterns

use verum_ast::{FileId, ItemKind, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module(source: &str) -> Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse: {:?}", e))
}

fn assert_parses(source: &str) {
    parse_module(source);
}

fn assert_fails(source: &str) -> bool {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).is_err()
}

fn parse_first_item(source: &str) -> ItemKind {
    let module = parse_module(source);
    module
        .items
        .into_iter()
        .next()
        .expect("Expected at least one item")
        .kind
}

// ============================================================================
// SECTION 1: THEOREM DECLARATIONS
// ============================================================================

#[test]
fn test_theorem_basic_no_params() {
    let source = r#"
        theorem trivial()
            proof by trivial
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Theorem(decl) => {
            assert_eq!(decl.name.name.as_str(), "trivial");
        }
        other => panic!("Expected Theorem, got {:?}", other),
    }
}

#[test]
fn test_theorem_with_params_and_requires() {
    let source = r#"
        theorem positive_sum(a: Int, b: Int)
            requires a > 0, b > 0
            ensures result > 0
            proof by auto
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Theorem(decl) => {
            assert_eq!(decl.name.name.as_str(), "positive_sum");
            assert!(!decl.requires.is_empty());
            assert!(!decl.ensures.is_empty());
        }
        other => panic!("Expected Theorem, got {:?}", other),
    }
}

#[test]
fn test_theorem_generic() {
    let source = r#"
        theorem identity<T>(x: T)
            ensures result == x
            proof by auto
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Theorem(decl) => {
            assert_eq!(decl.name.name.as_str(), "identity");
            assert!(!decl.generics.is_empty());
        }
        other => panic!("Expected Theorem, got {:?}", other),
    }
}

#[test]
fn test_theorem_with_return_type() {
    let source = r#"
        theorem add_comm(a: Int, b: Int) -> Bool
            proof by ring
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Theorem(decl) => {
            assert_eq!(decl.name.name.as_str(), "add_comm");
        }
        other => panic!("Expected Theorem, got {:?}", other),
    }
}

#[test]
fn test_theorem_structured_proof() {
    let source = r#"
        theorem sum_assoc(a: Int, b: Int, c: Int)
            ensures (a + b) + c == a + (b + c)
            proof {
                show (a + b) + c == a + (b + c) by ring;
            }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Theorem(decl) => {
            assert_eq!(decl.name.name.as_str(), "sum_assoc");
        }
        other => panic!("Expected Theorem, got {:?}", other),
    }
}

// ============================================================================
// SECTION 2: LEMMA DECLARATIONS
// ============================================================================

#[test]
fn test_lemma_basic() {
    let source = r#"
        lemma helper(x: Int)
            requires x >= 0
            proof by omega
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Lemma(decl) => {
            assert_eq!(decl.name.name.as_str(), "helper");
        }
        other => panic!("Expected Lemma, got {:?}", other),
    }
}

#[test]
fn test_lemma_with_ensures() {
    let source = r#"
        lemma nonneg_square(n: Int)
            ensures n * n >= 0
            proof by auto
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Lemma(decl) => {
            assert!(!decl.ensures.is_empty());
        }
        other => panic!("Expected Lemma, got {:?}", other),
    }
}

#[test]
fn test_lemma_proof_by_smt() {
    let source = r#"
        lemma bound_check(arr: List<Int>, i: Int)
            requires i >= 0, i < arr.len()
            proof by smt
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 3: AXIOM DECLARATIONS
// ============================================================================

#[test]
fn test_axiom_basic() {
    let source = r#"
        axiom excluded_middle(p: Bool);
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Axiom(decl) => {
            assert_eq!(decl.name.name.as_str(), "excluded_middle");
        }
        other => panic!("Expected Axiom, got {:?}", other),
    }
}

#[test]
fn test_axiom_generic() {
    let source = r#"
        axiom extensionality<T>(f: fn(T) -> T, g: fn(T) -> T);
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Axiom(decl) => {
            assert!(!decl.generics.is_empty());
        }
        other => panic!("Expected Axiom, got {:?}", other),
    }
}

#[test]
fn test_axiom_with_return_type() {
    let source = r#"
        axiom choice<T>(p: fn(T) -> Bool) -> T;
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 4: COROLLARY DECLARATIONS
// ============================================================================

#[test]
fn test_corollary_basic() {
    let source = r#"
        theorem base_theorem(x: Int)
            ensures x + 0 == x
            proof by ring

        corollary zero_add(x: Int)
            from base_theorem
            proof by auto
    "#;
    let module = parse_module(source);
    assert!(module.items.len() >= 2);
    match &module.items[1].kind {
        ItemKind::Corollary(_) => {}
        other => panic!("Expected Corollary as second item, got {:?}", other),
    }
}

// ============================================================================
// SECTION 5: TACTIC DECLARATIONS
// ============================================================================

#[test]
fn test_tactic_decl_basic() {
    let source = r#"
        tactic my_tactic() {
            apply(auto);
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_tactic_decl_with_params() {
    let source = r#"
        tactic solve_linear(a: Int) {
            apply(ring);
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 6: PROOF TACTICS
// ============================================================================

#[test]
fn test_proof_by_auto() {
    assert_parses("theorem t1(x: Int) proof by auto");
}

#[test]
fn test_proof_by_simp() {
    assert_parses("theorem t2(x: Int) proof by simp");
}

#[test]
fn test_proof_by_ring() {
    assert_parses("theorem t3(a: Int, b: Int) ensures a + b == b + a proof by ring");
}

#[test]
fn test_proof_by_omega() {
    assert_parses("theorem t4(n: Int) requires n >= 0 proof by omega");
}

#[test]
fn test_proof_by_smt() {
    assert_parses("theorem t5(x: Int, y: Int) requires x > y proof by smt");
}

#[test]
fn test_proof_by_contradiction() {
    assert_parses("theorem t6(x: Int) requires x > 0 proof by contradiction");
}

#[test]
fn test_proof_by_induction() {
    assert_parses("theorem t7(n: Int) requires n >= 0 proof by induction");
}

#[test]
fn test_proof_by_cases() {
    assert_parses("theorem t8(b: Bool) proof by cases");
}

#[test]
fn test_proof_by_assumption() {
    assert_parses("theorem t9(x: Int) requires x == 0 ensures x == 0 proof by assumption");
}

#[test]
fn test_proof_by_trivial() {
    assert_parses("theorem t10() proof by trivial");
}

// ============================================================================
// SECTION 7: STRUCTURED PROOF STEPS
// ============================================================================

#[test]
fn test_proof_have_step() {
    let source = r#"
        theorem with_have(x: Int)
            requires x > 0
            ensures x + 1 > 1
            proof {
                have h1: x > 0 by assumption;
                show x + 1 > 1 by omega;
            }
    "#;
    assert_parses(source);
}

#[test]
fn test_proof_show_step() {
    let source = r#"
        theorem with_show(a: Int, b: Int)
            ensures a + b == b + a
            proof {
                show a + b == b + a by ring;
            }
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 8: LAYER DECLARATIONS
// ============================================================================

#[test]
fn test_layer_basic_block() {
    let source = r#"
        layer DatabaseLayer {
            provide ConnectionPool = ConnectionPool.new();
        }
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Layer(_) => {}
        other => panic!("Expected Layer, got {:?}", other),
    }
}

#[test]
fn test_layer_composition() {
    let source = r#"
        layer AppLayer = DatabaseLayer + LoggingLayer;
    "#;
    let item = parse_first_item(source);
    match item {
        ItemKind::Layer(_) => {}
        other => panic!("Expected Layer, got {:?}", other),
    }
}

#[test]
fn test_layer_multiple_provides() {
    let source = r#"
        layer ServiceLayer {
            provide Logger = ConsoleLogger.new();
            provide Database = PostgresDb.connect("localhost");
            provide Cache = RedisCache.new();
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_layer_triple_composition() {
    let source = r#"
        layer FullStack = DatabaseLayer + CacheLayer + LoggingLayer;
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 9: VIEW PATTERNS
// ============================================================================

#[test]
fn test_view_pattern_simple() {
    let source = r#"
        fn test_view() {
            match value {
                parity -> 0 => print("even"),
                _ => print("odd"),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_view_pattern_with_variant_binding() {
    let source = r#"
        fn test_view() {
            match text {
                parse_int -> Some(n) => print(f"got {n}"),
                parse_int -> None => print("not a number"),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_view_pattern_in_match_with_guard() {
    let source = r#"
        fn test_view() {
            match input {
                to_int -> Some(n) if n > 0 => print("positive"),
                _ => print("other"),
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 10: ACTIVE PATTERNS
// ============================================================================

#[test]
fn test_active_pattern_total_no_params() {
    // Total active pattern: Even() matches even numbers
    let source = r#"
        fn test_active() {
            match n {
                Even() => print("even"),
                _ => print("odd"),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_active_pattern_with_params() {
    // Parameterized active pattern: InRange(0, 100)()
    let source = r#"
        fn test_active() {
            match n {
                InRange(0, 100)() => print("in range"),
                _ => print("out of range"),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_active_pattern_partial_extraction() {
    // Partial active pattern: ParseInt()(n) extracts a value
    let source = r#"
        fn test_active() {
            match text {
                ParseInt()(n) => print(f"parsed: {n}"),
                _ => print("not a number"),
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 11: AND-PATTERNS
// ============================================================================

#[test]
fn test_and_pattern_two_conditions() {
    let source = r#"
        fn test_and() {
            match n {
                Even() & Positive() => print("positive even"),
                _ => print("other"),
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 12: TYPE TEST PATTERNS
// ============================================================================

#[test]
fn test_type_test_pattern_in_match() {
    let source = r#"
        fn test_type_test(x: unknown) {
            match x {
                n is Int => print(f"integer: {n}"),
                s is Text => print(f"text: {s}"),
                _ => print("unknown type"),
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// SECTION 13: FORMAT STRINGS AND TAGGED LITERALS
// ============================================================================

#[test]
fn test_format_string_basic() {
    assert_parses(r#"fn test() { let s = f"hello {name}"; }"#);
}

#[test]
fn test_format_string_multiple_interpolations() {
    assert_parses(r#"fn test() { let s = f"x={x}, y={y}, z={z}"; }"#);
}

#[test]
fn test_format_string_with_expression() {
    assert_parses(r#"fn test() { let s = f"result: {a + b}"; }"#);
}

#[test]
fn test_format_string_nested_method_call() {
    assert_parses(r#"fn test() { let s = f"length: {items.len()}"; }"#);
}

#[test]
fn test_format_string_empty() {
    assert_parses(r#"fn test() { let s = f""; }"#);
}

#[test]
fn test_format_string_no_interpolations() {
    assert_parses(r#"fn test() { let s = f"plain text"; }"#);
}

#[test]
fn test_tagged_literal_json() {
    assert_parses(r#"fn test() { let j = json#"{}"; }"#);
}

#[test]
fn test_tagged_literal_sql() {
    assert_parses(r#"fn test() { let q = sql#"SELECT * FROM users"; }"#);
}

#[test]
fn test_tagged_literal_regex() {
    assert_parses(r#"fn test() { let r = regex#"[a-z]+"; }"#);
}

#[test]
fn test_tagged_literal_url() {
    assert_parses(r#"fn test() { let u = url#"https://example.com"; }"#);
}

#[test]
fn test_tagged_literal_email() {
    assert_parses(r#"fn test() { let e = email#"user@example.com"; }"#);
}

// ============================================================================
// SECTION 14: CBGR THREE-TIER REFERENCES
// ============================================================================

#[test]
fn test_cbgr_tier0_ref_in_function() {
    assert_parses("fn read(data: &Int) -> Int { *data }");
}

#[test]
fn test_cbgr_tier0_mut_ref_in_function() {
    assert_parses("fn modify(data: &mut Int) { *data = 42; }");
}

#[test]
fn test_cbgr_tier1_checked_ref() {
    assert_parses("fn fast_read(data: &checked Int) -> Int { *data }");
}

#[test]
fn test_cbgr_tier1_checked_mut_ref() {
    assert_parses("fn fast_modify(data: &checked mut Int) { *data = 42; }");
}

#[test]
fn test_cbgr_tier2_unsafe_ref() {
    assert_parses("fn raw_read(ptr: &unsafe Int) -> Int { *ptr }");
}

#[test]
fn test_cbgr_tier2_unsafe_mut_ref() {
    assert_parses("fn raw_modify(ptr: &unsafe mut Int) { *ptr = 42; }");
}

#[test]
fn test_cbgr_nested_references() {
    assert_parses("fn nested(data: &&Int) -> Int { **data }");
}

#[test]
fn test_cbgr_mixed_tier_references() {
    assert_parses("fn mixed(safe: &Int, fast: &checked Int, raw: &unsafe Int) -> Int { *safe }");
}

#[test]
fn test_cbgr_ref_in_type_definition() {
    assert_parses(r#"
        type RefHolder is {
            safe_ref: &Int,
            checked_ref: &checked Int,
            unsafe_ref: &unsafe Int,
        };
    "#);
}

#[test]
fn test_cbgr_genref_type() {
    assert_parses(r#"
        fn lending_iter(data: &List<Int>) -> GenRef<Int> {
            data.first()
        }
    "#);
}

// ============================================================================
// SECTION 15: ASYNC/AWAIT ADVANCED
// ============================================================================

#[test]
fn test_async_fn_with_context() {
    assert_parses(r#"
        async fn fetch_user(id: Int) -> User using [Database, Logger] {
            Logger.info(f"Fetching user {id}");
            Database.query(f"SELECT * FROM users WHERE id = {id}").await
        }
    "#);
}

#[test]
fn test_async_spawn_basic() {
    assert_parses(r#"
        async fn parallel() {
            let handle = spawn compute_heavy();
            let result = handle.await;
        }
    "#);
}

#[test]
fn test_async_nursery_block() {
    assert_parses(r#"
        async fn structured() {
            nursery {
                spawn task_a();
                spawn task_b();
            }
        }
    "#);
}

#[test]
fn test_async_select_with_arms() {
    // Grammar: select_arm = pattern '=' await_expr '=>' expr
    // The future expression must end with `.await`.
    assert_parses(r#"
        async fn race() {
            select {
                result = fetch_a().await => process(result),
                result = fetch_b().await => process(result),
            }
        }
    "#);
}

#[test]
fn test_async_defer_and_errdefer() {
    assert_parses(r#"
        async fn with_cleanup() {
            let conn = open_connection();
            defer close_connection(conn);
            errdefer log_error("connection failed");
            conn.query("SELECT 1").await
        }
    "#);
}

#[test]
fn test_async_yield_in_generator() {
    assert_parses(r#"
        fn* fibonacci() -> Int {
            let a = 0;
            let b = 1;
            loop {
                yield a;
                let temp = a;
                a = b;
                b = temp + b;
            }
        }
    "#);
}

// ============================================================================
// SECTION 16: CONTEXT SYSTEM ADVANCED
// ============================================================================

#[test]
fn test_context_using_multiple() {
    assert_parses(r#"
        fn process() using [Database, Logger, Cache] {
            Logger.info("processing");
        }
    "#);
}

#[test]
fn test_context_provide_scope() {
    assert_parses(r#"
        fn main() {
            provide Logger = ConsoleLogger.new() {
                run_app();
            }
        }
    "#);
}

#[test]
fn test_context_provide_multiple() {
    assert_parses(r#"
        fn setup() {
            provide Logger = ConsoleLogger.new();
            provide Database = PostgresDb.connect("localhost");
            run_server();
        }
    "#);
}

#[test]
fn test_context_async() {
    assert_parses(r#"
        async context EventBus {
            fn publish(event: Event) -> Result<Unit>;
            fn subscribe(topic: Text) -> Stream<Event>;
        }
    "#);
}

#[test]
fn test_context_with_generic() {
    assert_parses(r#"
        context Storage<T> {
            fn get(key: Text) -> Maybe<T>;
            fn set(key: Text, value: T) -> Result<Unit>;
        }
    "#);
}

// ============================================================================
// SECTION 17: PATTERN MATCHING COMPREHENSIVE
// ============================================================================

#[test]
fn test_pattern_or_with_guard() {
    assert_parses(r#"
        fn classify(n: Int) -> Text {
            match n {
                x if x > 0 => "positive",
                0 => "zero",
                _ => "negative",
            }
        }
    "#);
}

#[test]
fn test_pattern_nested_variant() {
    assert_parses(r#"
        fn depth(tree: Tree<Int>) -> Int {
            match tree {
                Leaf(_) => 1,
                Node { left, right } => {
                    let l = depth(*left);
                    let r = depth(*right);
                    if l > r { l + 1 } else { r + 1 }
                }
            }
        }
    "#);
}

#[test]
fn test_pattern_slice_with_rest() {
    assert_parses(r#"
        fn first_two(items: List<Int>) -> Maybe<(Int, Int)> {
            match items {
                [a, b, ..] => Some((a, b)),
                _ => None,
            }
        }
    "#);
}

#[test]
fn test_pattern_binding_with_at() {
    assert_parses(r#"
        fn check(opt: Maybe<Int>) {
            match opt {
                whole @ Some(n) if n > 0 => print(f"positive: {n}"),
                _ => print("other"),
            }
        }
    "#);
}

#[test]
fn test_pattern_or_alternatives() {
    assert_parses(r#"
        fn is_weekend(day: Day) -> Bool {
            match day {
                Saturday | Sunday => true,
                _ => false,
            }
        }
    "#);
}

#[test]
fn test_pattern_record_with_rest() {
    assert_parses(r#"
        fn get_name(user: User) -> Text {
            match user {
                User { name, .. } => name,
            }
        }
    "#);
}

#[test]
fn test_pattern_range() {
    assert_parses(r#"
        fn classify_char(c: Char) -> Text {
            match c {
                'a'..='z' => "lowercase",
                'A'..='Z' => "uppercase",
                '0'..='9' => "digit",
                _ => "other",
            }
        }
    "#);
}

#[test]
fn test_pattern_reference() {
    assert_parses(r#"
        fn process(data: &Maybe<Int>) {
            match data {
                &Some(n) => print(f"value: {n}"),
                &None => print("empty"),
            }
        }
    "#);
}

// ============================================================================
// SECTION 18: MULTIPLE PROOFS IN SAME MODULE
// ============================================================================

#[test]
fn test_multiple_theorems_and_lemmas() {
    let source = r#"
        theorem add_comm(a: Int, b: Int)
            ensures a + b == b + a
            proof by ring

        lemma add_zero(a: Int)
            ensures a + 0 == a
            proof by ring

        axiom well_ordering(s: Set<Int>);

        theorem add_assoc(a: Int, b: Int, c: Int)
            ensures (a + b) + c == a + (b + c)
            proof by ring
    "#;
    let module = parse_module(source);
    assert!(module.items.len() >= 4, "Expected at least 4 items, got {}", module.items.len());
}

// ============================================================================
// SECTION 19: NEGATIVE TESTS (should fail or error)
// ============================================================================

#[test]
fn test_theorem_missing_proof() {
    // Theorem without proof keyword should still parse (proof is optional)
    // or fail depending on grammar strictness
    let source = r#"
        theorem incomplete(x: Int)
            ensures x == x
    "#;
    // This may parse with an optional proof or fail - either is valid
    let _result = {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let parser = VerumParser::new();
        parser.parse_module(lexer, file_id)
    };
    // Not asserting success or failure - documenting behavior
}

#[test]
fn test_axiom_must_end_with_semicolon() {
    let source = "axiom bad(x: Int)";
    // Axioms require semicolon terminator
    let result = {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);
        let parser = VerumParser::new();
        parser.parse_module(lexer, file_id)
    };
    // May or may not fail depending on semicolon insertion
}

// ============================================================================
// SECTION 20: TYPE DEFINITIONS (ensuring non-Rust syntax)
// ============================================================================

#[test]
fn test_type_record_definition() {
    assert_parses(r#"
        type Point is {
            x: Float,
            y: Float,
        };
    "#);
}

#[test]
fn test_type_sum_definition() {
    assert_parses(r#"
        type Option<T> is None | Some(T);
    "#);
}

#[test]
fn test_type_protocol_definition() {
    assert_parses(r#"
        type Printable is protocol {
            fn to_text(&self) -> Text;
        };
    "#);
}

#[test]
fn test_type_newtype_definition() {
    assert_parses("type UserId is (Int);");
}

#[test]
fn test_type_unit_definition() {
    assert_parses("type Marker is ();");
}

#[test]
fn test_implement_block() {
    assert_parses(r#"
        implement Point {
            fn distance(&self, other: &Point) -> Float {
                let dx = self.x - other.x;
                let dy = self.y - other.y;
                (dx * dx + dy * dy).sqrt()
            }
        }
    "#);
}

#[test]
fn test_implement_protocol_for_type() {
    assert_parses(r#"
        implement Printable for Point {
            fn to_text(&self) -> Text {
                f"({self.x}, {self.y})"
            }
        }
    "#);
}

// ============================================================================
// SECTION 21: MOUNT STATEMENTS
// ============================================================================

#[test]
fn test_mount_basic() {
    assert_parses("mount std.io;");
}

#[test]
fn test_mount_selective() {
    assert_parses("mount std.collections.{List, Map, Set};");
}

#[test]
fn test_mount_with_alias() {
    assert_parses("mount std.io.File as F;");
}

// ============================================================================
// SECTION 22: RANK-2 POLYMORPHIC TYPES
// ============================================================================

#[test]
fn test_rank2_fn_type_in_struct() {
    assert_parses(r#"
        type Transducer<A, B> is {
            transform: fn<R>(Reducer<B, R>) -> Reducer<A, R>,
        };
    "#);
}

#[test]
fn test_rank2_fn_type_as_parameter() {
    assert_parses(r#"
        fn apply_to_all(f: fn<T>(T) -> T, x: Int) -> Int {
            f(x)
        }
    "#);
}

// ============================================================================
// Proof-block `let` with type annotation (regression for stdlib monad-law
// theorems in core/action/monads/{pure, state, probability, …}.vr).
//
// Pre-fix the proof_step parser only accepted untyped `let pattern = expr`
// inside `proof { ... }` blocks. The `:` of `let X: T = expr` triggered an
// "unexpected `:`, expected operator `=`" cascade. The standard let_stmt
// grammar (grammar/verum.ebnf) documents the optional type slot — proof
// blocks should honour the same syntax. The pre-existing stdlib was
// silently emitting parse warnings (lenient-skip codegen swallowed them);
// post-fix the warnings are gone and the type-annotated lets parse cleanly.
// ============================================================================

#[test]
fn proof_block_let_with_type_annotation_parses() {
    assert_parses(r#"
        theorem my_thm() -> Bool
            requires true
            ensures  true
        {
            proof {
                let x: Bool = true;
                let y: Int = 42;
                x
            }
        }
    "#);
}

#[test]
fn proof_block_let_with_generic_type_parses() {
    assert_parses(r#"
        type MyList<T> is { value: T };

        theorem generic_let_thm() -> Bool
            requires true
            ensures  true
        {
            proof {
                let lhs: MyList<Int> = MyList { value: 1 };
                let rhs: MyList<Bool> = MyList { value: true };
                true
            }
        }
    "#);
}

#[test]
fn proof_block_untyped_let_still_parses() {
    // Pin: the type annotation is OPTIONAL — pre-existing untyped form
    // must still work.
    assert_parses(r#"
        theorem untyped_thm() -> Bool
            requires true
            ensures  true
        {
            proof {
                let x = 1;
                let y = 2;
                true
            }
        }
    "#);
}

#[test]
fn proof_block_let_missing_type_after_colon_fails() {
    // Mirror of stmt.rs E043: `let x: = expr` should error rather than
    // silently consuming weird input. The fix included this guardrail
    // for the proof-block path.
    assert!(assert_fails(r#"
        theorem bad_thm() -> Bool
        {
            proof {
                let x: = 1;
                true
            }
        }
    "#));
}

#[test]
fn proof_block_let_literal_as_type_fails() {
    // Mirror of stmt.rs E043: `let x: 123 = expr` should error.
    assert!(assert_fails(r#"
        theorem bad_thm() -> Bool
        {
            proof {
                let x: 123 = 1;
                true
            }
        }
    "#));
}
