//! Storing a value in a rank-2 field is checked by SKOLEMISING the binder:
//! the quantified parameter becomes a rigid constant no value can know
//! anything about, so a value that needs it to be one particular type
//! cannot typecheck. That is the mechanism, and it is the right one.
//!
//! What the author was shown was the constant:
//!
//!     expected '?skolem$TypeVar(3853)', found 'Int'
//!
//! which names nothing in their program — not the parameter they wrote,
//! not a type they can look up, and the number is a counter. Rank-2 types
//! are among the least familiar things in the language, so the first error
//! an author meets is also their first lesson about how the feature works;
//! a raw internal constant teaches nothing and reads like a compiler bug.
//!
//! Task: T1038. Sibling found by the same probe: T1037.

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

const TRANSDUCER: &str = "\
type Reducer<A, R> is fn(R, A) -> R;
type Transducer<A, B> is { transform: fn<R>(Reducer<B, R>) -> Reducer<A, R> };
";

#[test]
fn a_monomorphic_stage_is_told_what_the_field_promises() {
    let text = complaints(&format!(
        "{TRANSDUCER}
fn int_only(step: Reducer<Int, Int>) -> Reducer<Int, Int> {{ step }}
fn use_it() -> Int {{
    let t: Transducer<Int, Int> = Transducer {{ transform: int_only }};
    0
}}
"
    ))
    .join("\n");

    assert!(
        text.contains("universally quantified"),
        "the refusal must state what the field promises; got:\n{text}"
    );
    assert!(
        text.contains("Int"),
        "the refusal must still name the type the value is fixed to; got:\n{text}"
    );
}

/// The skolem constant is an implementation detail of the CHECK. It names
/// nothing the author can write, so it must never reach them.
#[test]
fn the_skolem_constant_never_reaches_the_author() {
    let text = complaints(&format!(
        "{TRANSDUCER}
fn int_only(step: Reducer<Int, Int>) -> Reducer<Int, Int> {{ step }}
fn use_it() -> Int {{
    let t: Transducer<Int, Int> = Transducer {{ transform: int_only }};
    0
}}
"
    ))
    .join("\n");

    assert!(
        !text.contains("forall-bound") && !text.contains("skolem"),
        "no internal constant may appear in a user-facing message; got:\n{text}"
    );
}

/// The differentiator. An ORDINARY field mismatch has a real expected type
/// the author wrote, and naming it is the most useful thing the message can
/// do — the rank-2 wording must not spread to it.
#[test]
fn an_ordinary_field_mismatch_still_names_both_types() {
    let text = complaints(
        "\
type Boxed is { v: Int };
fn use_it() -> Int {
    let b = Boxed { v: true };
    0
}
",
    )
    .join("\n");

    assert!(
        text.contains("expected") && text.contains("found"),
        "an ordinary mismatch keeps naming both types; got:\n{text}"
    );
    assert!(
        !text.contains("universally quantified"),
        "nothing is quantified here; got:\n{text}"
    );
}
