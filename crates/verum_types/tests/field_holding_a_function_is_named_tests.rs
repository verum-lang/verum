//! A record field can hold a function, and calling it needs `(x.f)(a)` —
//! bare `x.f(a)` is a method call. That disambiguation is defensible: a
//! field and a method may share a name.
//!
//! What was not defensible is what the refusal SAID. `Holder` has a field
//! `f` of a function type the checker had already resolved, and the
//! diagnostic answered with three help lines that all point elsewhere:
//! check the spelling, implement a protocol, read the protocol docs. The
//! author is told to doubt the one thing that was right.
//!
//! This is on the path of the language's most distinctive feature, not a
//! corner: rank-2 function types (`fn<R>(Reducer<B, R>) -> Reducer<A, R>`)
//! live in record FIELDS by construction — that is what makes them rank-2
//! rather than generic functions. Every transducer-shaped program meets
//! this diagnostic on its first call.
//!
//! Task: T1037.

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

    // Read the error the way a user sees it — through its Diagnostic, which
    // is the channel the help lines live on.
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

const HOLDER: &str = "\
type Holder is { f: fn(Int) -> Int };
pure fn twice(x: Int) -> Int { x * 2 }
";

#[test]
fn a_bare_call_on_a_function_typed_field_names_the_field() {
    let text = complaints(&format!(
        "{HOLDER}
fn use_it() -> Int {{
    let h = Holder {{ f: twice }};
    h.f(21)
}}
"
    ))
    .join("\n");

    assert!(
        text.contains("field `f`"),
        "the refusal must say the receiver HAS `f`, as a field; got:\n{text}"
    );
    assert!(
        text.contains("(receiver.f)"),
        "the refusal must show the call form that works; got:\n{text}"
    );
}

/// The differentiator. Without it, a change that appended the field help to
/// EVERY missing method would satisfy the test above while making the
/// common case worse.
#[test]
fn a_genuinely_missing_name_keeps_the_old_wording() {
    let text = complaints(&format!(
        "{HOLDER}
fn use_it() -> Int {{
    let h = Holder {{ f: twice }};
    h.absent(21)
}}
"
    ))
    .join("\n");

    assert!(
        !text.contains("has a field `absent`"),
        "`absent` is neither a method nor a field — no field help is owed; got:\n{text}"
    );
    assert!(
        text.contains("absent"),
        "the missing name must still be reported; got:\n{text}"
    );
}

/// A field that holds a NON-function must not attract the help either: the
/// advice `(x.f)(…)` would be wrong there, not merely unhelpful.
#[test]
fn a_field_that_is_not_a_function_gets_no_call_advice() {
    let text = complaints(
        "\
type Boxed is { v: Int };
fn use_it() -> Int {
    let b = Boxed { v: 1 };
    b.v(2)
}
",
    )
    .join("\n");

    assert!(
        !text.contains("(receiver.v)"),
        "`v` holds an Int; suggesting a call form would be actively wrong; got:\n{text}"
    );
}

/// Separates the two candidate causes for the rank-2 case below: is the
/// field missed because the record is GENERIC, or because the field type is
/// RANK-2? This one is generic with an ordinary rank-1 function field.
#[test]
fn a_generic_record_with_a_rank1_function_field_is_still_named() {
    let text = complaints(
        "\
type Cell<T> is { step: fn(T) -> T };
pure fn bump(x: Int) -> Int { x + 1 }
fn use_it() -> Int {
    let c = Cell { step: bump };
    c.step(1)
}
",
    )
    .join("\n");

    assert!(
        text.contains("field `step`"),
        "a generic record's function field must be named too; got:\n{text}"
    );
}

/// The motivating case: the rank-2 field a transducer is made of.
#[test]
fn a_rank2_field_gets_the_same_help() {
    let text = complaints(
        "\
type Reducer<A, R> is fn(R, A) -> R;
type Transducer<A, B> is { transform: fn<R>(Reducer<B, R>) -> Reducer<A, R> };
pure fn add(acc: Int, value: Int) -> Int { acc + value }
fn stage<R>(step: Reducer<Int, R>) -> Reducer<Int, R> { step }
fn use_it() -> Int {
    let t = Transducer { transform: stage };
    let r = t.transform(add);
    0
}
",
    )
    .join("\n");

    assert!(
        text.contains("field `transform`"),
        "a rank-2 transducer field must be named, not answered with a spelling \
         hint — internally it is `Forall {{ body: Function }}`, and matching only \
         `Type::Function` missed precisely the motivating case; got:\n{text}"
    );
}
