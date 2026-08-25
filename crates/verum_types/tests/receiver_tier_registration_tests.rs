//! T0868 — a method's REFERENCE TIER must not decide whether it is a method.
//!
//! Verum's three-tier reference model gives `&self`, `&checked self` and
//! `&unsafe self` the same meaning as receivers; the tiers differ only in
//! how much the runtime has to check. The registration path disagreed:
//! two copies of an `is_static` predicate in `infer/decls.rs` listed six
//! of the ten receiver kinds, so a method declared with a tiered receiver
//! was filed as an ASSOCIATED FUNCTION under the `$static$` key.
//!
//! The observable symptom was asymmetric and therefore misleading:
//!
//! ```verum
//! implement Counter {
//!     public fn checked_read(&checked self) -> Int { self.value }
//! }
//! Counter.checked_read(&c)   // worked — it had been filed as static
//! c.checked_read()           // E400: no method named `checked_read`
//! ```
//!
//! Both sites now ask `FunctionParam::is_self`, which is the one place
//! that enumerates receiver kinds.

use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

/// Register a source module and report which key `method_name` landed
/// under: the instance key, the `$static$` key, or neither.
///
/// The distinction is the whole subject here, so the two are read
/// SEPARATELY. An earlier version of this file asked
/// `inherent_method_pattern_allows`, which answers a different
/// question — whether the receiver's type ARGUMENTS match an impl
/// pattern — and is permissive when there are none. It returned true
/// for a static `new()` as readily as for a method, so every assertion
/// passed without measuring anything. The negative pole below is what
/// exposed that, and it is why it exists.
struct Registration {
    instance: bool,
    static_fn: bool,
}

fn register(code: &str, type_name: &str, method_name: &str) -> Registration {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Impl(impl_block) = &item.kind {
            checker
                .register_impl_block(impl_block)
                .expect("impl registration should succeed");
        }
    }
    Registration {
        instance: checker
            .lookup_instance_method_for_testing(type_name, method_name)
            .is_some(),
        static_fn: checker
            .lookup_static_method_for_testing(type_name, method_name)
            .is_some(),
    }
}

fn registers_instance_method(code: &str, type_name: &str, method_name: &str) -> bool {
    let r = register(code, type_name, method_name);
    assert!(
        r.instance || r.static_fn,
        "`{method_name}` was not registered on `{type_name}` at all — \
         the harness is measuring nothing"
    );
    r.instance
}

fn counter_with_receiver(receiver: &str, method: &str) -> String {
    format!(
        r#"
type Counter is {{ value: Int }};

implement Counter {{
    public fn {method}({receiver}) -> Int {{
        self.value
    }}
}}
"#
    )
}

#[test]
fn plain_reference_receiver_is_an_instance_method() {
    // The control: this one always worked, and it is what pins the
    // predicate below as measuring registration rather than harness noise.
    let code = counter_with_receiver("&self", "read");
    assert!(
        registers_instance_method(&code, "Counter", "read"),
        "`&self` must register as an instance method"
    );
}

#[test]
fn checked_reference_receiver_is_an_instance_method() {
    let code = counter_with_receiver("&checked self", "checked_read");
    assert!(
        registers_instance_method(&code, "Counter", "checked_read"),
        "`&checked self` is a receiver — the tier must not demote the \
         method to an associated function"
    );
}

#[test]
fn unsafe_reference_receiver_is_an_instance_method() {
    let code = counter_with_receiver("&unsafe self", "unsafe_read");
    assert!(
        registers_instance_method(&code, "Counter", "unsafe_read"),
        "`&unsafe self` is a receiver — the tier must not demote the \
         method to an associated function"
    );
}

#[test]
fn mutable_tiered_receivers_are_instance_methods() {
    for receiver in ["&checked mut self", "&unsafe mut self", "&mut self"] {
        let code = counter_with_receiver(receiver, "bump");
        assert!(
            registers_instance_method(&code, "Counter", "bump"),
            "`{receiver}` must register as an instance method"
        );
    }
}

#[test]
fn owning_receiver_is_an_instance_method() {
    let code = counter_with_receiver("%self", "consume");
    assert!(
        registers_instance_method(&code, "Counter", "consume"),
        "`%self` is a receiver — an owning method is still a method"
    );
}

/// The negative pole. Without this, every assertion above would pass
/// against a predicate that simply answered `true` for everything.
#[test]
fn a_function_without_a_receiver_is_not_an_instance_method() {
    let code = r#"
type Counter is { value: Int };

implement Counter {
    public fn new() -> Counter {
        Counter { value: 0 }
    }
}
"#;
    let r = register(code, "Counter", "new");
    assert!(
        r.static_fn,
        "`new()` takes no receiver — it must be registered as an \
         associated function"
    );
    assert!(
        !r.instance,
        "`new()` must NOT also appear as an instance method"
    );
}

/// Every receiver spelling the grammar accepts, checked in one place —
/// so a newly added receiver kind fails here rather than silently
/// becoming a static method.
#[test]
fn every_receiver_spelling_registers_as_a_method() {
    let receivers = [
        "self",
        "mut self",
        "&self",
        "&mut self",
        "&checked self",
        "&checked mut self",
        "&unsafe self",
        "&unsafe mut self",
        "%self",
        "%mut self",
    ];
    for receiver in receivers {
        let code = counter_with_receiver(receiver, "peek");
        assert!(
            registers_instance_method(&code, "Counter", "peek"),
            "receiver `{receiver}` must register `peek` as an instance method"
        );
    }
}
