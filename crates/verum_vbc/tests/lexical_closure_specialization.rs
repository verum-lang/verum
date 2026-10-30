//! Lifted bodies retain declaration IDs and specialize along creation edges.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction,
    bytecode::decode_instructions,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    module::{FunctionDescriptor, VbcModule},
    mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
    serialize::serialize_module,
    types::{TypeId, TypeParamId, TypeRef},
};

fn compile(source: &str, bootstrap: bool) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("lexical"));
    let module = if bootstrap {
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
        codegen.finalize_module_from_state().unwrap()
    } else { codegen.compile_module(&ast).unwrap() };
    deserialize_module(&serialize_module(&module).unwrap()).unwrap()
}

fn body(module: &VbcModule, function: &FunctionDescriptor) -> Vec<Instruction> {
    decode_instructions(&module.bytecode[function.bytecode_offset as usize
        ..(function.bytecode_offset + function.bytecode_length) as usize]).unwrap()
}

fn specialize(module: VbcModule) -> VbcModule {
    let mut graph = InstantiationGraph::new();
    for function in &module.functions {
        discover_call_instantiations(&module, &body(&module, function),
                                     function.func_id_base, &mut graph).unwrap();
    }
    assert!(!graph.is_empty());
    let module = monomorphize_minimal(module, &graph).unwrap().module;
    deserialize_module(&serialize_module(&module).unwrap()).unwrap()
}

fn execute(module: VbcModule) -> i64 {
    let probe = module.find_function_by_name("lexical.probe").unwrap();
    Interpreter::new(Arc::new(module)).execute_function(probe).unwrap().as_i64()
}

#[test]
fn nested_creations_specialize_each_lexical_instantiation() {
    let source = "fn make<T>(value: T) -> fn() -> T { || { let inner = || value; inner() } } fn probe() -> Int { let first = make(17); let second = make(true); if second() { first() } else { 0 } }";
    for bootstrap in [false, true] {
        let module = specialize(compile(source, bootstrap));
        let mut creations = 0;
        for function in &module.functions {
            if !module.get_string(function.name).unwrap().contains("$mono$") { continue; }
            for instruction in body(&module, function) {
                if let Instruction::NewClosure { func_id, lexical_types, .. } = instruction {
                    creations += 1;
                    assert!(lexical_types.is_empty(), "specialized creation retained dynamic args");
                    let target = module.get_function(verum_vbc::module::FunctionId(func_id)).unwrap();
                    assert!(target.type_params.is_empty());
                    assert!(module.get_string(target.name).unwrap().contains("$mono$"));
                }
            }
        }
        assert_eq!(creations, 4, "both creator and nested body require two instantiations");
        assert_eq!(execute(module), 17);
    }
}

#[test]
fn shadowed_receiver_ids_and_noncapturing_bodies_keep_lexical_rosters() {
    for bootstrap in [false, true] {
        let module = compile("type Factory<T> is { value: T }; implement<T> Factory<T> { fn make<T>(self, value: T) -> fn() -> T { || { let inner = || value; inner() } } } fn probe() -> Int { let factory: Factory<Bool> = Factory { value: true }; let f = factory.make<Int>(17); f() }", bootstrap);
        let closures: Vec<_> = module.functions.iter().filter(|f|
            module.get_string(f.name).unwrap().contains("$closure$")).collect();
        assert_eq!(closures.len(), 2);
        for closure in closures {
            assert_eq!(closure.type_params.iter().map(|p| p.id).collect::<Vec<_>>(),
                       vec![TypeParamId(0), TypeParamId(0x8000)]);
        }
        assert_eq!(execute(specialize(module)), 17);

        let module = compile("fn make<T>() -> fn(T) -> T { |value: T| -> T { value } } fn probe() -> Int { let f = make<Int>(); f(17) }", bootstrap);
        let creator = module.functions.iter().find(|f|
            module.get_string(f.name).unwrap().ends_with(".make")).unwrap();
        assert!(body(&module, creator).iter().any(|i| matches!(i,
            Instruction::NewClosure { captures, lexical_types, .. }
            if captures.is_empty() && lexical_types == &[TypeRef::Generic(TypeParamId(0))])));
        assert_eq!(execute(specialize(module)), 17);
    }
}

#[test]
fn creation_discovery_requires_exact_complete_lexical_arguments() {
    let module = compile("fn make<T>(value: T) -> fn() -> T { || value } fn probe() -> Int { 0 }", false);
    let creator = module.functions.iter().find(|f|
        module.get_string(f.name).unwrap().ends_with(".make")).unwrap();
    let original = body(&module, creator).into_iter().find(|i|
        matches!(i, Instruction::NewClosure { .. })).unwrap();
    for args in [vec![], vec![TypeRef::Generic(TypeParamId(0))],
                 vec![TypeRef::Concrete(TypeId::INT); 2], vec![TypeRef::Concrete(TypeId::INT)]] {
        let mut instruction = original.clone();
        if let Instruction::NewClosure { lexical_types, .. } = &mut instruction { *lexical_types = args.clone(); }
        let mut graph = InstantiationGraph::new();
        discover_call_instantiations(&module, &[instruction], creator.func_id_base, &mut graph).unwrap();
        assert_eq!(graph.len(), usize::from(args == [TypeRef::Concrete(TypeId::INT)]));
    }
}

#[test]
fn colliding_legacy_const_slots_cannot_become_lexical_authority() {
    let ast = Parser::new("fn make<const Count: Int, T>(value: T) -> fn() -> T { || value }")
        .parse_module().unwrap();
    for bootstrap in [false, true] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("collision"));
        let error = if bootstrap {
            codegen.collect_unit_declarations(&[&ast]).unwrap();
            codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap_err()
        } else { codegen.compile_module(&ast).unwrap_err() };
        assert!(error.to_string().contains("closure lexical witness for Count collides"), "{error}");
    }
    let module = compile("fn make<T, const Count: Int>(value: T) -> fn() -> T { || value }", false);
    let closure = module.functions.iter().find(|f|
        module.get_string(f.name).unwrap().contains("$closure$")).unwrap();
    assert_eq!(closure.type_params.iter().map(|p| p.id).collect::<Vec<_>>(),
               vec![TypeParamId(0), TypeParamId(1)]);
}

#[test]
fn named_generic_callable_uses_exact_expected_signature() {
    for bootstrap in [false, true] {
        let module = compile("fn identity<T>(value: T) -> T { value } fn probe() -> Int { let f: fn(Int) -> Int = identity; f(17) }", bootstrap);
        let probe = module.functions.iter().find(|f|
            module.get_string(f.name).unwrap().ends_with(".probe")).unwrap();
        assert!(body(&module, probe).iter().any(|i| matches!(i,
            Instruction::NewClosure { lexical_types, .. }
            if lexical_types == &[TypeRef::Concrete(TypeId::INT)])));
        assert_eq!(execute(module.clone()), 17);
        assert_eq!(execute(specialize(module)), 17);
    }
}
