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
fn callable_declarations_distinguish_lexical_witnesses_from_local_quantifiers() {
    use verum_vbc::types::{TypeParamId, TypeRef};
    let mut module = compile("fn echo<T>(value:T)->T { value }", false);
    let id = module.find_function_by_name("callable_parameter.echo").unwrap();
    let integer = TypeRef::Concrete(TypeId::INT);
    let witness = std::slice::from_ref(&integer);
    let actual = module.resolve_callable_producer(id, 0, witness).unwrap();
    assert_eq!(actual.function.id, id);
    assert!(matches!(actual.signature, TypeRef::Function { params, return_type, .. }
        if params.as_slice() == witness && *return_type == integer));
    assert!(module.resolve_callable_producer(id, 0, &[]).is_err());
    assert!(module.resolve_callable_producer(id, 1, witness).is_err());

    let locally_bound = TypeRef::Generic(TypeParamId(0));
    let family = TypeRef::Rank2Function {
        is_async: false, type_param_count: 1, params: vec![locally_bound.clone()],
        return_type: Box::new(locally_bound), contexts: Default::default(),
    };
    module.functions[id.0 as usize].semantic_return = Some(family.clone());
    let actual = module.resolve_callable_producer(id, 0, witness).unwrap();
    assert!(matches!(actual.signature, TypeRef::Function { return_type, .. } if *return_type == family));

    for missing in [
        TypeRef::Generic(TypeParamId(9)),
        TypeRef::AssociatedProjection {
            base: Box::new(integer.clone()), qualifier: None, assoc: "Absent".into(), args: vec![],
        },
    ] {
        module.functions[id.0 as usize].semantic_return = Some(missing);
        assert!(matches!(module.resolve_callable_producer(id, 0, witness),
            Err(reason) if reason.contains("unresolved semantic signature")));
    }
    module.functions[id.0 as usize].id = FunctionId(id.0 + 1);
    assert!(matches!(module.resolve_callable_producer(id, 0, witness),
        Err(reason) if reason.contains("function identity is not declared")));
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
    check_capture_lifecycle(COPIES, "duplicate", "");
}

// An immutable capture has an owned environment. A writable capture cell
// still borrows storage, and its carrier cannot cross an owning boundary.
fn check_capture_lifecycle(declarations: &str, consumer: &str, tail: &str) {
    for (capture, closure, owned) in [
        ("let value: Int = 73;", "|| value", true),
        ("let mut value: Int = 72;", "|| { value = value + 1; value }", false),
    ] {
        let source = format!("{declarations} fn probe()->Int {{ {capture} {consumer}({closure}){tail} }}");
        for unit in [false, true] {
            let module = compile(&source, unit);
            let entry = module.find_function_by_name("callable_parameter.probe").unwrap();
            let result = Interpreter::new(module.into()).execute_function(entry);
            if owned {
                assert_eq!(result.unwrap().try_as_i64(), Some(73));
            } else {
                let error = result.expect_err("accepted a borrowed mutable environment");
                assert!(error.to_string().contains("captured environment requires an owned lifecycle"), "{error}");
            }
        }
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
    check_capture_lifecycle("fn consume<F:fn()->Int>(selected:F) {}", "consume", "; 73");
}
