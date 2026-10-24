//! T1807: generic aggregate handoffs preserve declared resource ownership.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};
const DECLARATIONS: &str = r#"
type Count is { value: Int };
type affine Handle is { counter: &mut Count, digit: Int };
implement Drop for Handle { fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; } }
type Envelope<T> is { value: T };
implement<T> Drop for Envelope<T> { fn drop(&mut self) { self.value.drop(); } }
type Named<T> is Held { value: T } | Empty;
implement<T> Drop for Named<T> { fn drop(&mut self) { match &mut self { Named.Held { value } => value.drop(), Named.Empty => {} } } }
type Choice<T> is Held(T) | Empty;
implement<T> Drop for Choice<T> { fn drop(&mut self) { match &mut self { Choice.Held(value) => value.drop(), Choice.Empty => {} } } }
"#;
fn compile(body: &str, bootstrap: bool) -> VbcModule {
    let ast = Parser::new(&format!("{DECLARATIONS}\n{body}"))
        .parse_module()
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("generic_transfer"));
    let mut module = if bootstrap {
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap();
        codegen.finalize_module_from_state().unwrap()
    } else {
        codegen.compile_module(&ast).unwrap()
    };
    for f in &mut module.functions {
        f.value_uses = None;
    }
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap()
}
fn run(body: &str, bootstrap: bool) -> i64 {
    let module = compile(body, bootstrap);
    let id = module
        .find_function_by_name("generic_transfer.probe")
        .unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .unwrap()
        .as_i64()
}
#[test]
fn generic_local_and_aggregate_transfers_keep_the_resource_alive() {
    for (ty, constructor) in [
        ("Envelope<T>", "Envelope { value: local }"),
        ("Envelope<T>", "Envelope(local)"),
        ("Choice<T>", "Choice.Held(local)"),
        ("Named<T>", "Named.Held { value: local }"),
    ] {
        for mutable in ["", "mut "] {
            for ret in ["", "return "] {
                let source = format!(
                    r#"
fn wrap<T>({mutable}item: T) -> {ty} {{
    let local: T = item;
    {ret}{constructor}
}}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{ let owner = wrap<Handle>(Handle {{ counter: &mut counter, digit: 7 }}); if counter.value != 0 {{ return 99; }} }}
    counter.value
}}
"#
                );
                for bootstrap in [false, true] {
                    assert_eq!(
                        run(&source, bootstrap),
                        7,
                        "{ty}, {mutable}, {ret}, bootstrap={bootstrap}"
                    );
                }
            }
        }
    }
}
#[test]
fn ordinary_generic_local_still_copies_a_mutable_value() {
    let source = r#"
type Plain is { value: Int };
fn copied<T>(item: T) -> T { let local: T = item; local }
fn probe() -> Int {
    let original = Plain { value: 4 };
    let mut changed = copied<Plain>(original);
    changed.value = 9;
    original.value * 10 + changed.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 49);
    }
}
#[test]
fn borrowed_generic_local_keeps_the_original_owner() {
    let source = r#"
fn copied<T>(item: T) -> T { let local: T = item; local }
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let handle = Handle { counter: &mut counter, digit: 6 };
      { let borrowed = copied<&Handle>(&handle); }
      if counter.value != 0 { return 99; }
    }
    counter.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 6);
    }
}

#[test]
fn missing_generic_witness_refuses_copy_and_transfer() {
    for bootstrap in [false, true] {
        let module = compile(
            "fn unresolved<T>(item: T) -> T { let local: T = item; local }",
            bootstrap,
        );
        let id = module
            .find_function_by_name("generic_transfer.unresolved")
            .unwrap();
        let error = Interpreter::new(Arc::new(module))
            .execute_function_with_args(id, &[verum_vbc::Value::from_i64(42)])
            .expect_err("unbound generic declaration must not authorize a handoff");
        assert!(
            matches!(
                error,
                verum_vbc::interpreter::InterpreterError::AssertionFailed { .. }
            ),
            "{error:?}"
        );
    }
}

#[test]
fn generic_transfer_clears_only_the_executed_branch() {
    for flag in ["true", "false"] {
        let source = format!(
            r#"
fn consume<T>(item: T) {{
    let local: T = item;
    if {flag} {{ let owner = Envelope {{ value: local }}; }}
}}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    consume<Handle>(Handle {{ counter: &mut counter, digit: 8 }});
    counter.value
}}
"#
        );
        for bootstrap in [false, true] {
            assert_eq!(run(&source, bootstrap), 8, "branch={flag}");
        }
    }
}
