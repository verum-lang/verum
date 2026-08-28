//! A context requirement travels with the function type, so a CALL can
//! be refused when the caller does not have the context.
//!
//! The check existed and was dead. `infer_expr_call` reads the callee's
//! `contexts` and hands them to `ContextChecker::check_context_satisfaction`
//! — correct code, never reached, because the field it reads was always
//! `None`.
//!
//! WHY IT WAS ALWAYS NONE, read off an instrumented binary rather than
//! inferred. One function is registered THREE times:
//!
//!     [fnins] work skip=false ctx=Some(1)   <- signature pass
//!     [fnins] work skip=false ctx=Some(1)   <- initial scheme
//!     [fnins] work skip=false ctx=None      <- final scheme, LAST writer
//!
//! The last writer built its type with `Type::function_with_properties`,
//! which carries the inferred properties and drops the contexts. The
//! comment directly above that line already described the same class for
//! PROTOCOL BOUNDS — "this is the LAST writer, so the env ends up holding
//! an unbounded scheme no matter what the earlier passes did" — and the
//! contexts were going out the same hole, three lines up.
//!
//! What it cost: calling a function that requires a context from one that
//! declares nothing compiled with zero errors and panicked at runtime with
//! "Context Log not provided". "No Magic — all dependencies explicit via
//! `using [...]`" is one of the four core principles, and a requirement
//! that vanishes when you wrap the call in a function is hidden state.
//!
//! Task: T0935.

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

fn complaints(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse");
    let mut checker = TypeChecker::new();
    // Contexts are registered by `check_item` itself (modules.rs handles
    // `ItemKind::Context`), and the items below are in source order with
    // the context first, so no separate registration pass is needed.
    for item in &module.items {
        if let verum_ast::ItemKind::Type(t) = &item.kind {
            let _ = checker.register_type_declaration(t);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            let _ = checker.register_function_signature(f);
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
        out.push(format!("{:?} {}", d.code(), d.message()));
    }
    out
}

const PRELUDE: &str = "\
context Log { fn note(m: Text); }
type C is { n: Int };
implement Log for C { fn note(m: Text) { } }
fn work() using [Log] { Log.note(\"x\"); }
";

#[test]
fn a_caller_that_declares_the_context_is_accepted() {
    // The POSITIVE control, and it is not optional: without it a fix that
    // refuses every call would look like a fix.
    let cs = complaints(&format!("{PRELUDE}fn outer() using [Log] {{ work(); }}"));
    assert!(
        cs.is_empty(),
        "declaring the context the callee needs must be enough: {cs:?}"
    );
}

#[test]
fn the_callee_itself_still_type_checks() {
    // `work` uses `Log.note` and declares `[Log]`; nothing here should
    // complain. If this fails the harness is wrong, not the rule.
    let cs = complaints(PRELUDE);
    assert!(cs.is_empty(), "the declaration itself must be clean: {cs:?}");
}

#[test]
fn a_caller_that_declares_nothing_is_refused() {
    let cs = complaints(&format!("{PRELUDE}fn outer() {{ work(); }}"));
    assert!(
        cs.iter().any(|c| c.contains("Log")),
        "calling a function that requires [Log] from a function that \
         declares nothing must be refused; got {cs:?}"
    );
}

#[test]
fn a_caller_declaring_a_different_context_is_refused() {
    // Declaring SOME context is not declaring THIS one — the check must
    // compare names, not merely notice that a using clause exists.
    let cs = complaints(&format!(
        "context Other {{ fn ping(); }}\n{PRELUDE}fn outer() using [Other] {{ work(); }}"
    ));
    assert!(
        cs.iter().any(|c| c.contains("Log")),
        "an unrelated context must not satisfy the requirement: {cs:?}"
    );
}
