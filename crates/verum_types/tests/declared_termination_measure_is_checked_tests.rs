//! `decreases n` is an ASSERTION BY THE AUTHOR, not a hint: it says the
//! recursion terminates and asks the compiler to hold them to it.
//!
//! It held nobody to anything. A function whose declared measure provably
//! INCREASES verified clean under `verum check`, `verum verify`,
//! `@verify(thorough)` and `@total` alike — four instruments, four clean
//! verdicts, and unguarded recursion with no measure at all was equally
//! clean. Two roots, and the second is the one that produced the silence:
//!
//!   1. the parser dropped the measure into `let _expr`, so `FunctionDecl`
//!      never carried it and no check ever received one;
//!   2. `check_block_termination` returned Ok as soon as the body had ANY
//!      non-recursive branch, which made every per-call check below
//!      unreachable. A base case is NECESSARY for termination and nowhere
//!      near SUFFICIENT: `forever` has one and never reaches it.
//!
//! Fixing (1) alone would have looked inert, because (2) returns first.
//!
//! The leniency in (2) is not a mistake in general — it suppresses false
//! positives on code that terminates for reasons a syntactic walk cannot
//! see. What changed is who it applies to: an author who declares a
//! measure this walk can DECIDE has asked to be checked.
//!
//! Task: T1026.

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

fn complaints(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse");
    let mut checker = TypeChecker::new();

    for item in &module.items {
        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
            let _ = checker.register_type_declaration(type_decl);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(func) = &item.kind {
            let _ = checker.register_function_signature(func);
        }
    }

    let mut out: Vec<String> = Vec::new();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            let d = e.to_diagnostic();
            out.push(format!("{:?} {}", d.code(), d.message()));
        }
    }
    for d in checker.diagnostics().iter() {
        out.push(format!("{:?}", d));
    }
    out
}

/// The defect. The measure is declared and the recursion grows it.
#[test]
fn a_declared_measure_that_grows_is_refused() {
    let text = complaints(
        "\
fn forever(n: Int) -> Int
    requires n >= 0
    decreases n
{
    if n <= 0 { 0 } else { forever(n + 1) }
}
",
    )
    .join("\n");

    assert!(
        !text.is_empty(),
        "a declared measure that INCREASES must be refused — it is the \
         author's own assertion, and accepting it reports success for a \
         claim that is false"
    );
}

/// Control that must stay green: the same shape, measure honoured.
#[test]
fn a_declared_measure_that_shrinks_still_passes() {
    let text = complaints(
        "\
fn count_down(n: Int) -> Int
    requires n >= 0
    decreases n
{
    if n <= 0 { 0 } else { count_down(n - 1) }
}
",
    )
    .join("\n");

    assert!(
        text.is_empty(),
        "a correct measure must not be refused; got:\n{text}"
    );
}

/// Lexicographic `decreases m, n`: a later component may shrink only while
/// every earlier one is passed through unchanged. Ackermann's inner call
/// keeps `m` and shrinks `n`; its outer calls shrink `m`.
#[test]
fn a_lexicographic_measure_accepts_a_later_component_shrinking() {
    let text = complaints(
        "\
fn ack(m: Int, n: Int) -> Int
    requires m >= 0
    requires n >= 0
    decreases m, n
{
    if m <= 0 {
        n + 1
    } else {
        if n <= 0 { ack(m - 1, 1) } else { ack(m - 1, ack(m, n - 1)) }
    }
}
",
    )
    .join("\n");

    assert!(
        text.is_empty(),
        "lexicographic ordering must accept `ack(m, n - 1)`; got:\n{text}"
    );
}

/// The leniency stays for code that declares NOTHING. Removing it wholesale
/// would refuse working `core/` code that terminates for reasons a syntactic
/// walk cannot see — the guard's own comment names `gamma(1.0 - x)`.
#[test]
fn recursion_without_a_declared_measure_keeps_the_old_leniency() {
    let text = complaints(
        "\
fn gamma_like(x: Float) -> Float {
    if x < 0.5 { gamma_like(1.0 - x) } else { x }
}
",
    )
    .join("\n");

    assert!(
        text.is_empty(),
        "an undeclared recursion must keep passing — this fix does not \
         change that policy; got:\n{text}"
    );
}

/// A measure this walk cannot decide (`a + b` is arithmetic, not a bare
/// parameter) keeps the leniency too. The conformance suite declares three
/// such measures — `decreases a + b`, `decreases t.size()`,
/// `decreases len(list)` — and none of them must turn red for a measure
/// nobody can yet check.
#[test]
fn a_measure_this_walk_cannot_decide_keeps_the_leniency() {
    let text = complaints(
        "\
fn gcd(a: Int, b: Int) -> Int
    requires a >= 0
    requires b >= 0
    decreases a + b
{
    if b <= 0 { a } else { gcd(b, a % b) }
}
",
    )
    .join("\n");

    assert!(
        text.is_empty(),
        "a compound measure is undecidable here and must not be refused; \
         got:\n{text}"
    );
}
