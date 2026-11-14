//! Receiver adjustments retain semantic types that are not display strings.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::CodegenConfig,
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

#[path = "fixtures/canonical_deref.rs"]
mod canonical_deref;

fn execute(body: &str) {
    let source = Parser::new(&format!(r#"
        module receiver_case;
        mount core.base.protocols.Deref;
        type Reader is protocol {{ fn read(&self) -> Int; }};
        type Witness<T> is {{ value: Int }};
        type Holder<T> is {{ value: T }};
        implement<T> Holder<T> {{ fn keep(&self, mut value: T) -> T {{ value }} }}
        type Wrapper<T> is {{ value: T }};
        implement<T> Deref for Wrapper<T> {{
            type Target = T;
            fn deref(&self) -> &T {{ &self.value }}
        }}
        type Verdict<T, E> is Got(T) | Failed(E);
        implement<T, E> Verdict<T, E> {{
            fn unwrap_or_else<F: fn(E) -> T>(self, fallback: F) -> T {{
                match self {{ Verdict.Got(value) => value, Verdict.Failed(error) => fallback(error) }}
            }}
        }}
        type Outcome<T> is Verdict<T, Int>;
        fn succeed<T>(value: T) -> Outcome<T> {{ Verdict.Got(value) }}
        fn probe() -> Int {{
            let token: Witness<dyn Reader> = Witness {{ value: 73 }};
            {body}
        }}
    "#)).parse_module().expect("receiver source grammar");
    let mut module = canonical_deref::compile(CodegenConfig::new("receiver_case"), &source)
        .expect("all bodies");
    assert!(module.resolve_external_bands().is_empty());
    module.resolve_protocol_dispatch();
    let module = deserialize_module(&serialize_module(&module).expect("encode")).expect("decode");
    verum_vbc::validate::validate_module(&module).expect("wire invariants");
    let entry = module.find_function_by_name("receiver_case.probe").expect("entry");
    let value = Interpreter::new(module.into()).execute_function(entry).expect("execute");
    assert_eq!(value.try_as_i64(), Some(73));
}

#[test]
fn direct_receiver_preserves_a_nested_dynamic_argument() {
    execute(r#"
        let holder: Holder<Witness<dyn Reader>> = Holder { value: token };
        holder.keep(token).value
    "#);
}

#[test]
fn adjusted_receiver_preserves_a_nested_dynamic_argument() {
    execute(r#"
        let holder: Wrapper<Holder<Witness<dyn Reader>>> = Wrapper {
            value: Holder { value: token },
        };
        holder.keep(token).value
    "#);
}

#[test]
fn inferred_method_result_preserves_a_nested_dynamic_argument() {
    execute(r#"
        let holder: Wrapper<Holder<Witness<dyn Reader>>> = Wrapper {
            value: Holder { value: token },
        };
        let returned = holder.keep(token);
        let next: Holder<Witness<dyn Reader>> = Holder { value: token };
        next.keep(returned).value
    "#);
}

#[test]
fn alias_receiver_and_diverging_callback_preserve_the_result_witness() {
    execute(r#"
        let returned = succeed(token).unwrap_or_else(|_| panic("unreachable failure"));
        let next: Holder<Witness<dyn Reader>> = Holder { value: token };
        next.keep(returned).value
    "#);
}

#[test]
fn alias_receiver_with_a_returning_callback_preserves_the_result_witness() {
    execute(r#"
        let returned = succeed(token).unwrap_or_else(|_| token);
        let next: Holder<Witness<dyn Reader>> = Holder { value: token };
        next.keep(returned).value
    "#);
}

#[test]
fn mismatched_nested_deref_target_is_refused_for_read_and_write() {
    for (method, expected) in [("read", "declared Target"), ("write", "mutable dereference")] {
        let source = Parser::new(&format!(r#"
            module target_mismatch;
            mount core.base.protocols.{{Deref, DerefMut}};
            type Cell<T> is {{ value: T }};
            implement<T> Cell<T> {{
                fn read(&self) -> Int {{ 73 }}
                fn write(&mut self) -> Int {{ 73 }}
            }}
            type Wrapper is {{ value: Cell<Bool> }};
            implement Deref for Wrapper {{
                type Target = Cell<Int>;
                fn deref(&self) -> &Cell<Bool> {{ &self.value }}
            }}
            implement DerefMut for Wrapper {{
                fn deref_mut(&mut self) -> &mut Cell<Bool> {{ &mut self.value }}
            }}
            fn probe(value: &mut Wrapper) -> Int {{ value.{method}() }}
        "#)).parse_module().expect("mismatched declaration grammar");
        let error = canonical_deref::compile(CodegenConfig::new("target_mismatch"), &source)
            .expect_err("equal nominal carriers cannot hide different Target arguments");
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn unrelated_deref_homonym_is_not_projection_authority() {
    let source = Parser::new(r#"
        module unrelated_projection;
        type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
        type Cell is { value: Int };
        implement Cell { fn read(&self) -> Int { self.value } }
        type Wrapper is { value: Cell };
        implement Deref for Wrapper {
            type Target = Cell;
            fn deref(&self) -> &Cell { &self.value }
        }
        fn probe(value: Wrapper) -> Int { value.read() }
    "#).parse_module().expect("unrelated protocol grammar");
    let error = canonical_deref::compile(CodegenConfig::new("unrelated_projection"), &source)
        .expect_err("same spelling does not establish the language protocol");
    assert!(error.to_string().contains("declared Target"), "{error}");
}

#[test]
fn equal_layout_does_not_erase_a_refined_deref_target() {
    let source = Parser::new(r#"
        module refined_projection;
        mount core.base.protocols.Deref;
        type Positive is Int { it > 0 };
        type Negative is Int { it < 0 };
        implement Positive { fn read(&self) -> Int { 73 } }
        type Wrapper is { value: Negative };
        implement Deref for Wrapper {
            type Target = Positive;
            fn deref(&self) -> &Negative { &self.value }
        }
        fn probe(value: Wrapper) -> Int { value.read() }
    "#).parse_module().expect("refined target grammar");
    let error = canonical_deref::compile(CodegenConfig::new("refined_projection"), &source)
        .expect_err("equal representation does not establish the refined Target");
    assert!(error.to_string().contains("declared Target"), "{error}");
}
