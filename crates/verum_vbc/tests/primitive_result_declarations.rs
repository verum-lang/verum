//! Scalar result declarations survive local binding and generic handoff.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{Value, codegen::VbcCodegen, interpreter::Interpreter};

fn execute(source: &str, args: &[Value]) -> Value {
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("inspect").unwrap();
    let mut vm = Interpreter::new(module.into());
    vm.run_global_ctors().unwrap();
    vm.execute_function_with_args(entry, args).unwrap()
}

#[test]
fn integer_arithmetic_retains_the_declared_result_through_local_and_generic_call() {
    let result = execute(
        "fn retain<T>(value:T)->T {value}
        fn inspect(count:Int,pos:Int)->Int {
            let remaining=count-pos;
            let rounded=(remaining+3)/4;
            retain(rounded)
        }",
        &[Value::from_i64(292), Value::from_i64(3)],
    );
    assert_eq!(result.try_as_i64(), Some(73));
}

#[test]
fn float_arithmetic_keeps_its_own_carrier() {
    let result = execute(
        "fn retain<T>(value:T)->T {value}
        fn inspect(count:Float,pos:Float)->Float {
            let remaining=count-pos;
            retain(remaining/4.0)
        }",
        &[Value::from_f64(300.0), Value::from_f64(8.0)],
    );
    assert_eq!(result.try_as_f64(), Some(73.0));
}

#[test]
fn logical_results_remain_boolean_across_callable_handoff() {
    let result = execute(
        "fn retain<T>(value:T)->T {value}
        fn inspect(value:Int)->Bool {let accepted=!(value<0) && value<100; retain(accepted)}",
        &[Value::from_i64(73)],
    );
    assert_eq!(result.try_as_bool(), Some(true));
}
