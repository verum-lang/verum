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
// Regression tests for generic-variant pattern matching (#66) —
// verify that `Either<A, B>::Right(s)` binds `s` to the SECOND
// type parameter (`B`), not the first (`A`).
//
// Pre-fix bug (closed as not-reproducing on the current pipeline,
// kept as guard against regression):
//   type Either<A, B> is | Left(A) | Right(B);
//   match e { Either.Right(s) => ... }
// type-checked `s` as the LEFT type (`A`) — causing E400
// "expected 'Int', found 'Text'" on the Either<Int, Text> shape.
// Triple-test covers both Right and Left bindings plus a swapped
// argument order to lock in the substitution invariant.

use verum_parser::Parser;
use verum_types::Type;
use verum_types::infer::TypeChecker;

fn check(code: &str, msg: &str) {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("Parsing should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
            checker
                .register_type_declaration(type_decl)
                .expect("type registration should succeed");
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }
    let mut errs: Vec<String> = Vec::new();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            errs.push(format!("{:?}", e));
        }
    }
    assert!(errs.is_empty(), "{}: {:?}", msg, errs);
}

#[test]
fn test_either_right_binds_to_second_type_param() {
    // Right(s) on `Either<Int, Text>` must bind s: Text — directly
    // exercising the substitution path that previously mapped tv_B
    // to args[0]=Int instead of args[1]=Text.
    check(
        r#"
type Either<A, B> is
    | Left(A)
    | Right(B);

fn use_right(e: Either<Int, Text>) -> Text {
    match e {
        Either.Left(_) => "default",
        Either.Right(s) => s,
    }
}
"#,
        "Either<Int, Text>::Right(s) must bind s: Text",
    );
}

#[test]
fn test_either_left_binds_to_first_type_param() {
    check(
        r#"
type Either<A, B> is
    | Left(A)
    | Right(B);

fn use_left(e: Either<Int, Text>) -> Int {
    match e {
        Either.Left(n) => n,
        Either.Right(_) => 0,
    }
}
"#,
        "Either<Int, Text>::Left(n) must bind n: Int",
    );
}

#[test]
fn test_either_swapped_args_binds_correctly() {
    // Swapping the type-arg order forces tv_A→Text, tv_B→Int.
    // The pre-fix bug would invert the binding (Right(n): Text)
    // and the type checker would reject `n + 1`.
    check(
        r#"
type Either<A, B> is
    | Left(A)
    | Right(B);

fn use_swap(e: Either<Text, Int>) -> Int {
    match e {
        Either.Left(_) => 0,
        Either.Right(n) => n,
    }
}
"#,
        "Either<Text, Int>::Right(n) must bind n: Int",
    );
}

#[test]
fn test_either_constructor_application_with_annotation() {
    // The original repro shape: a let-binding annotated as
    // `Either<Int, Text>` whose RHS is `Either.Right("hello")` —
    // exercises constructor inference that previously bound the
    // payload type to A instead of B.
    check(
        r#"
type Either<A, B> is
    | Left(A)
    | Right(B);

fn shape() -> Text {
    let right: Either<Int, Text> = Either.Right("hello");
    match right {
        Either.Left(_) => "default",
        Either.Right(s) => s,
    }
}
"#,
        "let right: Either<Int, Text> = Either.Right(\"hello\") must type-check",
    );
}
