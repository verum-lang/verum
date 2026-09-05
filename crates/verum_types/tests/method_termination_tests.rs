#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Regression tests: termination checking did not run on a METHOD.
//!
//! Measured 2026-09-05 with the free function as the control, the same body
//! on both sides:
//!
//! ```text
//!                              free            impl
//!   no base case, no measure   E321            CLEAN
//!   `decreases n`, n GROWS     E321            CLEAN
//!   `decreases n`, n shrinks   clean           clean
//! ```
//!
//! Two separate causes, both found by reading the walker rather than guessing:
//!
//! 1. `find_recursive_calls_impl`'s `MethodCall` arm recorded NOTHING — it
//!    only descended into the receiver and the arguments. So
//!    `find_recursive_calls` came back empty for every method and each
//!    per-call check was unreachable.
//! 2. The `is_recursive` gate asked a SECOND, narrower walk
//!    (`has_self_recursive_method_call`) that covered six expression kinds
//!    against the real walker's seventy-three; a self-call inside a `while`,
//!    a `for` or a `try` set no flag at all. That walk is deleted and the
//!    gate now asks the walker the checks use.
//!
//! Under both, the receiver is `self` as its OWN path segment —
//! `PathSegment::SelfValue`, never `Name("self")` — which is why a test
//! written against `as_ident()` matched nothing.
//!
//! This is the method half of T1026, whose own comment in the free-function
//! path says why it matters: a declared measure has to REACH the checker for
//! anything to check it, and until it does, every instrument is green by
//! construction.
//!
//! WHAT THESE TESTS PIN: that a method and a free function get the SAME
//! verdict on the same recursion. Not the wording of the diagnostic, and not
//! today's leniency about what counts as decreasing.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Type-check a module and return only the TERMINATION diagnostics.
///
/// Filtered to E321 on purpose: a bare `TypeChecker` has no stdlib, so an
/// unfiltered comparison measures `TypeNotFound` on both sides and agrees for
/// reasons having nothing to do with termination.
fn termination_errors(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            let _ = checker.register_function_signature(f);
        }
    }
    let mut errs: Vec<String> = module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect();
    errs.extend(checker.diagnostics().iter().map(|d| format!("{:?}", d)));
    errs.retain(|e| e.contains("E321") || e.contains("Termination") || e.contains("decreas"));
    errs
}

/// The same recursion, written free and as a method, must get the same answer.
fn assert_same_verdict(shape: &str, free: &str, method: &str) {
    let f = termination_errors(free);
    let m = termination_errors(method);
    assert_eq!(
        f.is_empty(),
        m.is_empty(),
        "`{shape}`: the free function gave {} termination error(s) and the \
         method gave {}. A method is not exempt from termination checking.\n  \
         free:   {f:?}\n  method: {m:?}",
        f.len(),
        m.len(),
    );
}

/// Recursion with no base case and no measure. This is the shape that showed
/// the check was absent rather than lenient.
#[test]
fn unbounded_recursion_is_refused_in_a_method_as_in_a_function() {
    assert_same_verdict(
        "unbounded",
        "fn m(n: Int) -> Int { m(n + 1) }\n",
        "type R is { };\n\nimplement R {\n    fn m(&self, n: Int) -> Int { self.m(n + 1) }\n}\n",
    );
}

/// A declared measure that provably INCREASES. The parser now carries
/// `decreases` into a method's AST; this pins that something reads it.
#[test]
fn a_growing_decreases_measure_is_refused_in_a_method_as_in_a_function() {
    assert_same_verdict(
        "decreases grows",
        "fn m(n: Int) -> Int\n    decreases n\n{ if n > 100 { 0 } else { m(n + 1) } }\n",
        "type R is { };\n\nimplement R {\n    fn m(&self, n: Int) -> Int\n        \
         decreases n\n    { if n > 100 { 0 } else { self.m(n + 1) } }\n}\n",
    );
}

/// NEGATIVE CONTROL. Honest recursion must still be ACCEPTED on both sides —
/// otherwise "make the method agree" is satisfiable by refusing everything.
#[test]
fn a_shrinking_measure_is_accepted_in_a_method_as_in_a_function() {
    let free = "fn m(n: Int) -> Int\n    decreases n\n{ if n <= 0 { 0 } else { m(n - 1) } }\n";
    let method = "type R is { };\n\nimplement R {\n    fn m(&self, n: Int) -> Int\n        \
                  decreases n\n    { if n <= 0 { 0 } else { self.m(n - 1) } }\n}\n";
    assert_same_verdict("decreases shrinks", free, method);
    assert!(
        termination_errors(free).is_empty(),
        "a measure that shrinks must be accepted in a free function"
    );
    assert!(
        termination_errors(method).is_empty(),
        "a measure that shrinks must be accepted in a method"
    );
}

/// A self-call inside a `while` — an expression kind the deleted narrow walk
/// did not cover, so the recursion flag was never set no matter what the body
/// did. `check_block_termination` hand-walked only `Expr` and `Let`
/// statements for the same question, so even a flagged call in any other
/// statement was never CHECKED; both now use the block walker.
#[test]
fn a_self_call_inside_a_loop_still_counts_as_recursion() {
    assert_same_verdict(
        "self-call in a while",
        "fn m(n: Int) -> Int { while n > 0 { let _ = m(n + 1); } 0 }\n",
        "type R is { };\n\nimplement R {\n    fn m(&self, n: Int) -> Int \
         { while n > 0 { let _ = self.m(n + 1); } 0 }\n}\n",
    );
}

/// `return self.m(n + 1);` — a statement kind `check_block_termination` did
/// not walk, so the call was found by the gate and then never checked.
#[test]
fn a_self_call_in_a_return_statement_is_checked_like_a_free_one() {
    assert_same_verdict(
        "return self-call",
        "fn m(n: Int) -> Int { return m(n + 1); }\n",
        "type R is { };\n\nimplement R {\n    fn m(&self, n: Int) -> Int \
         { return self.m(n + 1); }\n}\n",
    );
}
