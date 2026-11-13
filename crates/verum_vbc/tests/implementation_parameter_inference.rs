#![cfg(feature = "codegen")]
//! Implementation binders follow declared structural evidence, never a guessed type.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, VbcCodegen},
};

fn compile(bound: &str, argument: &str, result: &str, required: &str) -> CodegenResult<VbcModule> {
    let source = format!(
        r#"
        module inference;
        type Source is {{ value: Int }};
        type Trigger is protocol {{}};
        implement Trigger for Source {{}}
        type Produced<F> is protocol {{ type Output; }};
        type Granted is protocol {{ fn stamp(&self) -> Int; }};
        implement<F{bound}, T> Produced<F> for Source {{ type Output = T; }}
        implement<X: Trigger + Produced<fn({argument}) -> {result}, Output = {required}>> Granted for X {{
            fn stamp(&self) -> Int {{ 73 }}
        }}
        fn main() -> Int {{
            let source = Source {{ value: 7 }};
            source.stamp()
        }}
    "#
    );
    let module = Parser::new(&source)
        .parse_module()
        .expect("implementation grammar");
    VbcCodegen::with_config(CodegenConfig::new("inference")).compile_module(&module)
}

#[test]
fn callable_output_determines_an_implementation_parameter() {
    let module = compile(": fn(Int) -> T", "Int", "Bool", "Bool")
        .expect("a complete callable witness determines the result binder");
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let main = module.find_function_by_name("inference.main").unwrap();
        let result = verum_vbc::interpreter::Interpreter::new(module.into())
            .execute_function(main)
            .expect("execute selected implementation");
        assert_eq!(result.try_as_i64(), Some(73));
    }
}

#[test]
fn unconstrained_implementation_parameter_is_not_invented() {
    let error = compile("", "Int", "Bool", "Bool")
        .expect_err("an associated result declaration alone is not its witness");
    assert!(
        error.to_string().contains("undetermined parameter 'T'"),
        "{error}"
    );
}

#[test]
fn callable_argument_and_associated_result_contracts_are_preserved() {
    for (argument, required) in [("Bool", "Bool"), ("Int", "Int")] {
        compile(": fn(Int) -> T", argument, "Bool", required)
            .expect_err("a mismatching callable or associated result cannot prove membership");
    }
}

#[test]
fn generic_receiver_retains_the_callable_hypothesis_in_its_implementation_scope() {
    let source = r#"
        module inference;
        type Subject is protocol { type Item; };
        type Transformer<Fut, F> is { input: Fut, callback: F };
        type Produced is protocol { type Output; };
        type Granted is protocol { fn stamp(&self) -> Int; };
        implement<Fut: Subject, F: fn(Fut.Item) -> T, T> Produced for Transformer<Fut, F> {
            type Output = T;
        }
        implement<X: Produced> Granted for X { fn stamp(&self) -> Int { 73 } }
    "#;
    let module = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("inference"))
        .compile_module(&module)
        .expect("the implementation's declared callable bound is a lexical premise");
    let receiver = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Transformer"))
        .expect("the generic receiver is declared");
    let method = module
        .functions
        .iter()
        .find(|method| {
            method.parent_type == Some(receiver.id)
                && module.get_string(method.name) == Some("Transformer.stamp")
        })
        .expect("the proved blanket body belongs to the declared receiver");
    assert!(
        !method
            .instructions
            .as_deref()
            .unwrap_or_default()
            .is_empty()
            || method.bytecode_length > 0
    );
}
