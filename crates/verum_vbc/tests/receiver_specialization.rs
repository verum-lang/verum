//! A nominal runtime layout can carry several distinct method implementations.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module, validate::validate_module,
};

#[test]
fn specialized_self_selects_its_own_body_in_either_declaration_order() {
    let integer = "implement Measure for Cell<Int> { fn width(&self) -> Int { 41 } }";
    let boolean = "implement Measure for Cell<Bool> { fn width(&self) -> Int { 73 } }";
    for (first, second) in [(integer, boolean), (boolean, integer)] {
        let source = format!(
            r#"
            protocol Measure {{ fn width(&self) -> Int; }}
            type Cell<T> is {{ value: T }};
            {first}
            {second}
            fn width_int(c: &Cell<Int>) -> Int {{ c.width() }}
            fn width_bool(c: Cell<Bool>) -> Int {{ c.width() }}
            fn probe() -> Int {{
                let x: Cell<Int> = Cell {{ value: 0 }};
                let y: Cell<Bool> = Cell {{ value: true }};
                width_int(&x) * 100 + width_bool(y)
            }}
        "#
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        let mut module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        assert!(module.resolve_external_bands().is_empty());
        validate_module(&module).unwrap();
        let entry = module.find_function_by_name("probe").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.as_i64(), 4173, "{first}");
    }
}

#[test]
fn a_nonmatching_instantiation_cannot_reuse_the_first_body() {
    let ast = Parser::new(
        r#"
        type Cell<T> is { value: T };
        implement Cell<Int> { fn width(&self) -> Int { 1 } }
        implement Cell<Bool> { fn width(&self) -> Int { 2 } }
        fn invalid(c: Cell<Text>) -> Int { c.width() }
    "#,
    )
    .parse_module()
    .unwrap();
    let err = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(
        err.to_string()
            .contains("does not select a unique implementation"),
        "{err}"
    );
}

#[test]
fn a_protocol_bound_uses_the_callers_concrete_receiver_on_each_invocation() {
    let ast = Parser::new(
        r#"
        protocol Measure { fn width(&self) -> Int; }
        type Cell<T> is { value: T };
        implement Measure for Cell<Int> { fn width(&self) -> Int { 13 } }
        implement Measure for Cell<Bool> { fn width(&self) -> Int { 29 } }
        fn measure<T: Measure>(value: T) -> Int { value.width() }
        fn probe() -> Int {
            let a: Cell<Int> = Cell { value: 0 };
            let b: Cell<Bool> = Cell { value: true };
            measure<Cell<Int>>(a) * 100 + measure<Cell<Bool>>(b)
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let mut module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert!(module.resolve_external_bands().is_empty());
    module.resolve_protocol_dispatch();
    validate_module(&module).unwrap();
    let cell = module
        .get_function(module.find_function_by_name("Cell.width").unwrap())
        .unwrap()
        .parent_type
        .unwrap();
    let width = module.strings.lookup("width").unwrap();
    use verum_vbc::types::{TypeId, TypeRef};
    let target = |item| {
        let declared = TypeRef::instantiated(cell, [TypeRef::Concrete(item)]);
        module.try_bound_instance_method(&declared, width, 0)
    };
    let (integer, _) = target(TypeId::INT).unwrap().unwrap();
    let (boolean, _) = target(TypeId::BOOL).unwrap().unwrap();
    assert_ne!(integer, boolean);
    assert!(target(TypeId::TEXT).is_err());
    let entry = module.find_function_by_name("probe").unwrap();
    let value = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert_eq!(value.as_i64(), 1329);
}

#[test]
fn monomorphized_bodies_do_not_become_competing_source_implementations() {
    use verum_vbc::{
        bytecode::decode_instructions,
        mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
        types::{TypeId, TypeRef},
    };

    let ast = Parser::new(include_str!("fixtures/bound_receiver.vr"))
        .parse_module().unwrap();
    let mut module = VbcCodegen::new().compile_module(&ast).unwrap();
    module.resolve_protocol_dispatch();
    let mut graph = InstantiationGraph::new();
    for function in &mut module.functions {
        function.is_generic = !function.type_params.is_empty();
    }
    for function in &module.functions {
        let start = function.bytecode_offset as usize;
        let body = decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize])
            .unwrap();
        discover_call_instantiations(&module, &body, function.func_id_base, &mut graph).unwrap();
    }
    let specialized = monomorphize_minimal(module, &graph).unwrap().module;
    let mut module = deserialize_module(&serialize_module(&specialized).unwrap()).unwrap();
    module.resolve_protocol_dispatch();
    validate_module(&module).unwrap();
    let declaration = module.find_function_by_name("Cell.width").unwrap();
    assert!(module.get_function(declaration).unwrap().source_definition.is_some());
    for specialization in &module.specializations {
        let id = module.specialized_function_id(specialization.generic_fn, &specialization.type_args).unwrap();
        let function = module.get_function(id).unwrap();
        assert!(function.source_definition.is_none(),
            "a substituted body is not the original source definition");
    }
    let parent = module.get_function(declaration).unwrap().parent_type.unwrap();
    let method = module.strings.lookup("width").unwrap();
    for carrier in [TypeId::U8, TypeId::INT] {
        let arguments = vec![TypeRef::Concrete(carrier)];
        let receiver = TypeRef::instantiated(parent, arguments.clone());
        let (selected, selected_arguments) = module.try_bound_instance_method(&receiver, method, 0)
            .expect("one source implementation despite multiple compiled bodies")
            .expect("declared method");
        assert_eq!(selected, declaration);
        assert_eq!(selected_arguments, arguments);
        assert_ne!(module.specialized_function_id(selected, &arguments).unwrap(), selected);
    }
    let entry = module.find_function_by_name("probe").unwrap();
    assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().as_i64(), 108);
}
