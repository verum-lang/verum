//! Callable values and constructors are selected by declarations, never case.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn execute(source: &str, expected: i64) {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("callables"))
        .compile_module(&ast).expect("declared callable");
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("callables.probe").unwrap();
        let mut interpreter = Interpreter::new(Arc::new(module));
        interpreter.run_global_ctors().unwrap();
        assert_eq!(interpreter.execute_function(entry).unwrap().try_as_i64(), Some(expected));
    }
}

#[test]
fn constants_and_static_cells_invoke_the_stored_callable() {
    for name in ["HANDLER", "Handler", "handler"] {
        for storage in ["const", "static", "static mut", "@thread_local static"] {
            execute(&format!(
                "fn apply(value: Int) -> Int {{ value * 2 + 1 }}
                 {storage} {name}: fn(Int) -> Int = apply;
                 fn probe() -> Int {{ {name}(36) }}"
            ), 73);
        }
        execute(&format!(
            "fn apply(value: Int) -> Int {{ value * 2 + 1 }}
             fn probe() -> Int {{ static {name}: fn(Int) -> Int = apply; {name}(36) }}"
        ), 73);
    }
}

#[test]
fn a_local_callable_shadows_a_global_callable() {
    execute("fn first(value: Int) -> Int { 11 }
        static HANDLER: fn(Int) -> Int = first;
        fn probe() -> Int { let HANDLER = |value| value + 1; HANDLER(72) }", 73);
}

#[test]
fn undeclared_callees_cannot_fabricate_a_variant() {
    for callee in ["MISSING", "Missing", "missing", "Absent.Variant"] {
        let source = format!("fn probe() {{ {callee}(7); }}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let result = VbcCodegen::with_config(CodegenConfig::new("callables"))
            .compile_module(&ast);
        assert!(result.is_err(), "undeclared callee {callee} produced bytecode");
    }
}

#[test]
fn declared_tuple_and_sum_constructors_keep_their_payloads() {
    execute("type Number is (Int); type Outcome is Value(Int) | Empty;
        fn probe() -> Int { match Outcome.Value(Number(73).0) {
            Outcome.Value(value) => value, Outcome.Empty => 0,
        } }", 73);
}

#[test]
fn declared_generic_receivers_keep_deferred_method_dispatch() {
    let ast = Parser::new("type Factory is protocol { fn make(value: Self) -> Self; };
        fn select<T: Factory>(value: T) -> T { T.make(value) }")
        .parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new("callables"))
        .compile_module(&ast).expect("declared generic receiver");
}
