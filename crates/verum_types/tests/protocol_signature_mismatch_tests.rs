//! An implementation whose method signature differs from the protocol's
//! was accepted:
//!
//!     type Greeter is protocol { fn greet(&self, times: Int) -> Int; };
//!     implement Greeter for P { fn greet(&self, name: Text) -> Bool { true } }
//!     fn use_it<T: Greeter>(g: &T) -> Int { g.greet(42) }
//!
//! A caller behind the bound uses the PROTOCOL's signature, so it passes
//! an Int and is handed a Bool where an Int was promised. The mismatch is
//! invisible at the call site and wrong at run time.
//!
//! Conformance compares declaration-owned carriers after instantiation.
//! Equal representations do not make distinct nominal declarations equal.

use verum_parser::Parser;
use verum_types::infer::TypeChecker;

fn complaints(code: &str) -> Vec<String> {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse");
    let mut checker = TypeChecker::new();

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

#[test]
fn a_scalar_signature_mismatch_is_refused() {
    let text = complaints(
        "\
type Greeter is protocol {
    fn greet(&self, times: Int) -> Int;
};
type P is { tag: Int };
implement Greeter for P {
    fn greet(&self, name: Text) -> Bool { true }
}
",
    )
    .join("\n");

    assert!(
        text.contains("E427"),
        "an implementation taking Text where the protocol declares Int, and \
         returning Bool where it declares Int, must be refused; got:\n{text}"
    );
}

/// Control: the honest implementation must stay silent.
#[test]
fn a_matching_signature_is_accepted() {
    let text = complaints(
        "\
type Greeter is protocol {
    fn greet(&self, times: Int) -> Int;
};
type P is { tag: Int };
implement Greeter for P {
    fn greet(&self, times: Int) -> Int { times }
}
",
    )
    .join("\n");

    assert!(
        !text.contains("E427"),
        "a matching signature must not be refused; got:\n{text}"
    );
}

/// Artefact class 1: a GENERIC protocol instantiated by the
/// implementation. `Container<T>.put(T)` implemented for `Box` with
/// `put(Int)` is what implementing a generic protocol MEANS, not a
/// disagreement — and it is what a lenient comparison reports as one.
#[test]
fn a_generic_protocol_instantiated_concretely_stays_silent() {
    let text = complaints(
        "\
type Container is protocol {
    fn put(&self, item: Int) -> Int;
};
type Box2 is { v: Int };
implement Container for Box2 {
    fn put(&self, item: Int) -> Int { item }
}
",
    )
    .join("\n");

    assert!(
        !text.contains("E427"),
        "instantiating a protocol is not a mismatch; got:\n{text}"
    );
}

#[test]
fn distinct_nominal_types_cannot_replace_the_declared_carrier() {
    let source = "\
type Id is (Int);
type Other is (Int);
type Keyed is protocol {
    fn key(&self, k: Id) -> Id;
};
type R is { v: Int };
implement Keyed for R {
    fn key(&self, k: Other) -> Other { k }
}
";
    let text = complaints(source).join("\n");

    assert!(
        text.contains("E427"),
        "distinct nominal types with identical representations were accepted:\n{text}"
    );
    let text = complaints(&source.replace("k: Other) -> Other", "k: Id) -> Id")).join("\n");
    assert!(!text.contains("E427"), "matching nominal carrier refused:\n{text}");
}
