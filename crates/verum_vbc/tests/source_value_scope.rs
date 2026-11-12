#![cfg(feature = "codegen")]
//! Each implementation is checked with its own declaration-owned binder roles.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source)
        .parse_module()
        .expect("implementation grammar");
    VbcCodegen::with_config(CodegenConfig::new("binder_scope"))
        .compile_module(&ast)
        .unwrap_or_else(|error| panic!("declaration scope: {error:?}"))
}

fn run(module: VbcModule, expected: i64) {
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let entry = module
            .find_function_by_name("binder_scope.main")
            .expect("exact entry");
        let value = verum_vbc::interpreter::Interpreter::new(module.into())
            .execute_function(entry)
            .expect("selected implementation body");
        assert_eq!(value.try_as_i64(), Some(expected));
    }
}

#[test]
fn declared_value_parameters_keep_their_role_in_implementation_signatures() {
    for domain in ["Int", "USize", "USize{N > 0}"] {
        compile(&format!(
            r#"
            module binder_scope;
            type Cell<N: meta {domain}> is {{ value: () }};
            implement<N: meta {domain}> Cell<N> {{
                fn identity(&self, value: Cell<N>) -> Cell<N> {{ value }}
            }}
        "#
        ));
    }
}

#[test]
fn value_indexed_protocol_implementations_keep_their_receiver_role() {
    for domain in ["Int", "USize", "USize{N > 0}"] {
        compile(&format!(
            r#"
            module binder_scope;
            type Action<I, O> is protocol {{
                fn apply(&self, value: I) -> O;
                fn marker(&self) -> Int {{ 73 }}
            }};
            type Cell<N: meta {domain}> is {{ value: () }};
            implement<N: meta {domain}> Action<Int, Int> for Cell<N> {{
                fn apply(&self, value: Int) -> Int {{ value }}
            }}
        "#
        ));
    }
}

#[test]
fn unrelated_value_telescope_does_not_supply_or_block_protocol_evidence() {
    for domain in ["Int", "USize", "USize{N > 0}"] {
        for required in [None, Some("Bool"), Some("Int")] {
            for first in [false, true] {
                let unrelated = format!(
                    r#"
                    type Foreign is protocol {{}};
                    type Indexed<N: meta {domain}> is {{ value: () }};
                    implement<N: meta {domain}> Foreign for Indexed<N> {{}}
                "#
                );
                let evidence = required
                    .map(|argument| format!("implement Required<{argument}> for Other {{}}"))
                    .unwrap_or_default();
                let granted = required == Some("Int");
                let operation = if granted {
                    "other.marker()"
                } else {
                    "other.value"
                };
                let declarations = format!(
                    r#"
                    type Base is protocol {{}};
                    type Required<T> is protocol {{}};
                    type Derived is protocol {{ fn marker(&self) -> Int {{ 73 }} }};
                    implement<T: Base + Required<Int>> Derived for T {{}}
                    type Other is {{ value: Int }};
                    {evidence}
                    implement Base for Other {{}}
                    fn main() -> Int {{ let other = Other {{ value: 7 }}; {operation} }}
                "#
                );
                let source = if first {
                    format!("{unrelated}{declarations}")
                } else {
                    format!("{declarations}{unrelated}")
                };
                let module = compile(&format!("module binder_scope; {source}"));
                assert_eq!(
                    module.functions.iter().any(|function| module
                        .get_string(function.name)
                        .is_some_and(|name| name.ends_with("Other.marker"))),
                    granted,
                    "only the exact Required<Int> declaration proves this body"
                );
                run(module, if granted { 73 } else { 7 });
            }
        }
    }
}

#[test]
fn a_type_parameter_cannot_supply_a_protocol_receivers_value_slot() {
    let source = r#"
        module binder_scope;
        type Foreign is protocol {};
        type Indexed<N: meta Int> is { value: () };
        implement<T> Foreign for Indexed<T> {}
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("binder_scope"))
        .compile_module(&ast)
        .expect_err("type-role witness is not a value witness");
    assert!(
        error
            .to_string()
            .contains("type parameter cannot occupy a value slot"),
        "{error:?}"
    );
}

#[test]
fn a_value_parameter_cannot_supply_a_protocol_receivers_type_slot() {
    let source = r#"
        module binder_scope;
        type Foreign is protocol {};
        type Holder<T> is { value: () };
        implement<N: meta Int> Foreign for Holder<N> {}
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("binder_scope"))
        .compile_module(&ast)
        .expect_err("value-role witness is not a type witness");
    assert!(
        error
            .to_string()
            .contains("value parameter cannot occupy a type slot"),
        "{error:?}"
    );
}

#[test]
fn unproved_value_refinements_cannot_authorize_a_blanket_body() {
    let source = r#"
        module binder_scope;
        type Base is protocol {};
        type Required is protocol {};
        type Derived is protocol { fn marker(&self) -> Int { 73 } };
        implement<T: Base + Required> Derived for T {}
        type Indexed<N: meta Int> is { value: () };
        implement<N: meta Int{N > 0}> Required for Indexed<N> {}
        implement Base for Indexed<-1> {}
    "#;
    let ast = Parser::new(source)
        .parse_module()
        .expect("refined implementation grammar");
    let error = VbcCodegen::with_config(CodegenConfig::new("binder_scope"))
        .compile_module(&ast)
        .expect_err("the matched header is not a proof of its refinement");
    assert!(
        error
            .to_string()
            .contains("implementation parameter requires additional declaration evidence"),
        "{error:?}"
    );
}
