//! Callable parameter cleanup requires its actual producer and semantic type.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{
    Value,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::{FunctionId, VbcModule},
    types::TypeId,
};

fn compile(source: &str, unit: bool) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("callable_parameter"));
    let module = if unit {
        compiler.collect_unit_declarations(&[&ast]).unwrap();
        compiler
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap();
        compiler.finalize_module_from_state().unwrap()
    } else {
        compiler.compile_module(&ast).unwrap()
    };
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap()
}

const DECLARATIONS: &str = "
fn known() -> Int { 1 }
fn wrong(value: Int) -> Int { value }
fn dispose(selected: fn() -> Int) -> Int { 73 }
";

const COPIES: &str = "
fn known() -> Int { 73 }
fn wrong(value: Int) -> Int { value }
fn duplicate(selected: fn() -> Int) -> Int { let copied = selected; copied() }
";

#[test]
fn empty_callable_copies_preserve_the_selected_function() {
    for unit in [false, true] {
        for formation in ["known", "|| 73"] {
            let source = format!("{COPIES} fn probe()->Int {{duplicate({formation})}}");
            let module = compile(&source, unit);
            let entry = module.find_function_by_name("callable_parameter.probe").unwrap();
            assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(73));
        }
    }
}

#[test]
fn copying_a_callable_requires_the_actual_producer_and_signature() {
    for unit in [false, true] {
        let module = compile(COPIES, unit);
        let entry = module.find_function_by_name("callable_parameter.duplicate").unwrap();
        let wrong = module.find_function_by_name("callable_parameter.wrong").unwrap();
        for (value, required) in [
            (Value::from_function(wrong), "parameter signature"),
            (Value::from_function(FunctionId(u32::MAX)), "function identity is not declared"),
            (Value::from_ptr(std::ptr::NonNull::<u8>::dangling().as_ptr()), "producer-issued environment receipt"),
        ] {
            let mut vm = Interpreter::new(module.clone().into());
            let error = vm.execute_function_with_args(entry, &[value]).unwrap_err();
            assert!(error.to_string().contains(required), "{error}");
        }
    }
}

#[test]
fn copying_a_captured_environment_requires_its_owned_lifecycle() {
    for unit in [false, true] {
        let module = compile(&format!("{COPIES} fn probe()->Int {{let value=73;duplicate(|| value)}}"), unit);
        let entry = module.find_function_by_name("callable_parameter.probe").unwrap();
        let error = Interpreter::new(module.into()).execute_function(entry).unwrap_err();
        assert!(error.to_string().contains("captured environment requires an owned lifecycle"), "{error}");
    }
}

#[test]
fn a_known_function_reference_has_no_environment_obligation() {
    for unit in [false, true] {
        let module = compile(DECLARATIONS, unit);
        let known = module
            .find_function_by_name("callable_parameter.known")
            .unwrap();
        let dispose = module
            .find_function_by_name("callable_parameter.dispose")
            .unwrap();
        let mut vm = Interpreter::new(module.into());
        assert_eq!(
            vm.execute_function_with_args(dispose, &[Value::from_function(known)])
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn wrong_or_missing_function_identity_cannot_authorize_cleanup() {
    for unit in [false, true] {
        let module = compile(DECLARATIONS, unit);
        let wrong = module
            .find_function_by_name("callable_parameter.wrong")
            .unwrap();
        let dispose = module
            .find_function_by_name("callable_parameter.dispose")
            .unwrap();
        for (function, required) in [
            (wrong, "parameter signature"),
            (FunctionId(u32::MAX), "function identity is not declared"),
        ] {
            let mut vm = Interpreter::new(module.clone().into());
            let error = vm
                .execute_function_with_args(dispose, &[Value::from_function(function)])
                .unwrap_err();
            assert!(error.to_string().contains(required), "{error}");
        }
    }
}

#[test]
fn an_unissued_object_or_pointer_is_not_a_callable_receipt() {
    for unit in [false, true] {
        let module = compile(DECLARATIONS, unit);
        let dispose = module
            .find_function_by_name("callable_parameter.dispose")
            .unwrap();
        for allocated in [false, true] {
            let mut vm = Interpreter::new(module.clone().into());
            let pointer = if allocated {
                vm.state
                    .heap
                    .alloc(TypeId::TUPLE, 16)
                    .unwrap()
                    .as_ptr()
                    .cast::<u8>()
            } else {
                std::ptr::NonNull::<u8>::dangling().as_ptr()
            };
            let error = vm
                .execute_function_with_args(dispose, &[Value::from_ptr(pointer)])
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("producer-issued environment receipt"),
                "{error}"
            );
        }
    }
}

#[test]
fn a_captured_callable_refuses_without_an_owned_environment_lifecycle() {
    let source = "
        fn consume<F: fn() -> Int>(selected: F) {}
        fn probe() -> Int { let captured = 11; consume(|| captured); 73 }
    ";
    for unit in [false, true] {
        let module = compile(source, unit);
        let entry = module
            .find_function_by_name("callable_parameter.probe")
            .unwrap();
        let error = Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("captured environment requires an owned lifecycle"),
            "{error}"
        );
    }
}
