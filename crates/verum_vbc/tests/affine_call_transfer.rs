//! Call boundaries must hand the same resource obligation to the callee.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};
const TYPES: &str = r#"
type Count is { value: Int };
type affine Socket is { counter: &mut Count, digit: Int };
implement Drop for Socket { fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; } }
"#;
fn run(source: &str, bootstrap: bool) -> i64 {
    let ast = Parser::new(&format!("{TYPES}\n{source}"))
        .parse_module()
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("call_ownership"));
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
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let id = module
        .find_function_by_name("call_ownership.probe")
        .unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .unwrap()
        .as_i64()
}
#[test]
fn named_argument_transfers_instead_of_leaving_a_second_caller_owner() {
    for (declaration, call) in [
        (
            "fn finish(socket: Socket) { drop(socket); }",
            "finish(socket)",
        ),
        (
            "fn finish<T>(socket: T) { let owned: T = socket; drop(owned); }",
            "finish<Socket>(socket)",
        ),
    ] {
        let source = format!(
            r#"
{declaration}
fn probe() -> Int {{
    let mut counter=Count {{ value: 0 }};
    {{ let socket=Socket {{ counter: &mut counter, digit: 7 }};
       {call};
       if counter.value != 7 {{ return 99; }}
    }}
    counter.value
}}
"#
        );
        for bootstrap in [false, true] {
            assert_eq!(run(&source, bootstrap), 7);
        }
    }
}
#[test]
fn returned_parameter_transfers_its_obligation_to_the_caller() {
    for result in ["socket", "return socket;"] {
        let source = format!(
            r#"
fn pass<T>(socket: T) -> T {{ {result} }}
fn probe() -> Int {{
    let mut counter=Count {{ value: 0 }};
    {{ let socket=Socket {{ counter: &mut counter, digit: 4 }};
       let received=pass<Socket>(socket);
       if counter.value != 0 {{ return 99; }}
    }}
    counter.value
}}
"#
        );
        for bootstrap in [false, true] {
            assert_eq!(run(&source, bootstrap), 4);
        }
    }
}
#[test]
fn borrowed_calls_do_not_consume_or_close_the_referent() {
    let source = r#"
fn inspect(socket: &Socket) -> Int { socket.digit }
fn inspect_generic<T>(socket: &T) {}
fn probe() -> Int {
    let mut counter=Count { value: 0 };
    { let socket=Socket { counter: &mut counter, digit: 2 };
      assert_eq(inspect(&socket),2);
      inspect_generic<Socket>(&socket);
      if counter.value != 0 { return 99; }
    }
    counter.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 2);
    }
}

#[test]
fn forwarded_generic_arguments_use_the_callers_own_witness() {
    let source = r#"
fn finish<T>(socket: T) { let local: T = socket; drop(local); }
fn forward<T>(socket: T) { let local: T = socket; finish<T>(local); }
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let socket = Socket { counter: &mut counter, digit: 6 };
      forward<Socket>(socket);
      if counter.value != 6 { return 99; }
    }
    counter.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 6);
    }
}

#[test]
fn ordinary_by_value_arguments_retain_the_existing_copy_semantics() {
    let source = r#"
type Plain is { value: Int };
fn change<T>(mut item: T) -> T { item }
fn probe() -> Int {
    let original = Plain { value: 4 };
    let mut changed = change<Plain>(original);
    changed.value = 9;
    original.value * 10 + changed.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 49);
    }
}
