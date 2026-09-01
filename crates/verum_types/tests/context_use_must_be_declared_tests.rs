//! A function that reaches for a context it does not name in `using`
//! compiles, runs, and returns a value from an unrelated type.
//!
//!     fn declared() -> Int using [Clock] { Clock.now() }   -> 42
//!     fn sneaky()   -> Int              { Clock.now() }    -> MaxRetriesExceeded
//!
//! Both `verum check` and `verum run` exit 0. `MaxRetriesExceeded` is a
//! variant of an UNRELATED type: the call is reading whatever occupies a
//! slot, not a Clock.
//!
//! The signature is the whole argument for capabilities over globals — if
//! a function can acquire a capability without saying so, the signature is
//! optional and the argument is gone. Worse, `pure fn p() -> Int {
//! Clock.now() }` is accepted too, so the property the rest of the system
//! reasons from (purity) is wrong about a function that reads a clock.
//!
//! Task: T1027.

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

const CLOCK: &str = "\
context protocol Clock { fn now(&self) -> Int; }
type FixedClock is { at: Int };
implement Clock for FixedClock {
    fn now(&self) -> Int { self.at }
}
";

#[test]
fn a_function_that_uses_an_undeclared_context_is_refused() {
    let text = complaints(&format!(
        "{CLOCK}
fn sneaky() -> Int {{ Clock.now() }}
"
    ))
    .join("\n");

    assert!(
        !text.is_empty(),
        "reaching for `Clock` without `using [Clock]` must be refused — \
         otherwise the signature is optional and capabilities are globals \
         with extra syntax; got no diagnostic at all"
    );
}

/// The control that must stay green: the honest form still works.
#[test]
fn a_function_that_declares_the_context_is_accepted() {
    let text = complaints(&format!(
        "{CLOCK}
fn declared() -> Int using [Clock] {{ Clock.now() }}
"
    ))
    .join("\n");

    assert!(
        text.is_empty(),
        "declaring the context is the correct form and must not be \
         refused; got:\n{text}"
    );
}

/// A `pure fn` reaching a capability is the worse half: purity is the
/// premise the verifier, caching and reordering all reason from, so a
/// wrong answer here is wrong about code that never touches a context.
#[test]
fn a_pure_function_may_not_reach_a_context_either() {
    let text = complaints(&format!(
        "{CLOCK}
pure fn p() -> Int {{ Clock.now() }}
"
    ))
    .join("\n");

    assert!(
        !text.is_empty(),
        "a `pure fn` reading an injected capability must be refused; \
         got no diagnostic at all"
    );
}
