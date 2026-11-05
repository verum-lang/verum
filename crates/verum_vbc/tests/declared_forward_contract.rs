//! Forward callable ABI is declaration evidence, not a name/arity hint.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_ast::Module;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen, ItemFailurePolicy, CodegenError},
    module::{VbcModule, FunctionDescriptor},
    types::{TypeRef, TypeId, Mutability},
    interpreter::Interpreter,
};

fn parse(source: &str) -> Module { Parser::new(source).parse_module().unwrap() }
fn interface(source: &Module) -> (VbcModule, VbcCodegen) {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture"));
    let module = codegen.collect_owned_declaration_interface(&[("fixture", source)]).unwrap();
    (module, codegen)
}
fn consumer(source: &Module, interface: &VbcModule, declarations: &VbcCodegen, typed: bool) -> Result<VbcModule, CodegenError> {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation());
    codegen.initialize();
    codegen.import_functions(&declarations.export_functions());
    if typed { codegen.import_bootstrap_nominal_dependencies(&[source], &[interface])?; }
    codegen.collect_owned_unit_declarations(&[("fixture", source)])?;
    codegen.resolve_pending_imports();
    codegen.compile_owned_unit_items(&[("fixture", source)], ItemFailurePolicy::Strict)?;
    codegen.finalize_module()
}
fn function<'a>(module: &'a VbcModule, owner: &str) -> &'a FunctionDescriptor {
    module.functions.iter().find(|function| {
        let name = module.strings.get(function.name).unwrap();
        let declared = function.origin_module.and_then(|id| module.strings.get(id)).unwrap_or(&module.name);
        verum_vbc::module::qualify_module_name(declared, name) == owner
    }).unwrap_or_else(|| panic!("missing exact callable {owner}"))
}
fn through_wire(module: &VbcModule) -> VbcModule {
    verum_vbc::validate::validate_module(module).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    verum_vbc::validate::validate_module(&decoded).unwrap();
    decoded
}

const DECLARATION: &str = "module fixture.provider; @intrinsic(\"declared_forward_fixture\") public fn transfer<T>(slot: &mut T, bytes: &[Byte]) -> T;";
const CALLER: &str = "module fixture.consumer; mount fixture.provider.transfer; public fn probe() -> Int { let mut slot = 42; transfer(&mut slot, \"bytes\".as_bytes()) }";

#[test]
fn referenced_forward_retains_source_generic_reference_and_slice_carriers() {
    let provider = parse(DECLARATION);
    let (declaration, registered) = interface(&provider);
    for supplied in [&declaration, &through_wire(&declaration)] {
        let result = consumer(&parse(CALLER), supplied, &registered, true).unwrap();
        for module in [&result, &through_wire(&result)] {
            let descriptor = function(module, "fixture.provider.transfer");
            assert_eq!(descriptor.params.len(), 2);
            assert_eq!(descriptor.return_type, TypeRef::Generic(verum_vbc::types::TypeParamId(0)));
            assert!(matches!(&descriptor.params[0].type_ref, TypeRef::Reference { inner, mutability: Mutability::Mutable, .. } if matches!(**inner, TypeRef::Generic(_))));
            assert!(matches!(&descriptor.params[1].type_ref, TypeRef::Reference { inner, .. } if matches!(**inner, TypeRef::Slice(_))));
            assert!(descriptor.semantic_params.as_ref().unwrap().iter().all(Option::is_some));
            assert!(descriptor.semantic_return.is_some());
            assert_eq!(descriptor.bytecode_length, 0, "declaration is not an executable RetV body");
            assert_eq!(module.strings.get(descriptor.intrinsic_name.unwrap()), Some("declared_forward_fixture"));
        }
    }
}

#[test]
fn function_info_without_declaration_descriptor_cannot_fabricate_an_abi() {
    let (declaration, registered) = interface(&parse(DECLARATION));
    let error = consumer(&parse(CALLER), &declaration, &registered, false).unwrap_err();
    assert!(error.to_string().contains("no declaration-owned forward contract"), "{error}");
}

#[test]
fn source_body_must_be_loaded_not_replaced_with_a_return_stub() {
    let source = parse("module fixture.provider; public fn actual(value: Int) -> Int { value + 1 }");
    let (declaration, registered) = interface(&source);
    let caller = parse("module fixture.consumer; mount fixture.provider.actual; public fn probe()->Int{actual(41)}");
    let error = consumer(&caller, &declaration, &registered, true).unwrap_err();
    assert!(error.to_string().contains("no loaded implementation"), "{error}");
}

#[test]
fn real_source_body_executes_after_owned_declaration_collection_in_either_order() {
    let provider = parse("module fixture.provider; public fn actual(value: Int)->Int{value + 1}");
    let caller = parse("module fixture.consumer; mount fixture.provider.actual; public fn probe()->Int{actual(41)}");
    for reverse in [false, true] {
        let mut files = [("fixture", &provider), ("fixture", &caller)];
        if reverse { files.reverse(); }
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation());
        codegen.initialize();
        codegen.collect_owned_unit_declarations(&files).unwrap();
        codegen.resolve_pending_imports();
        codegen.compile_owned_unit_items(&files, ItemFailurePolicy::Strict).unwrap();
        let result = codegen.finalize_module().unwrap();
        for module in [result.clone(), through_wire(&result)] {
            let id = function(&module, "fixture.consumer.probe").id;
            assert_eq!(Interpreter::new(Arc::new(module)).execute_function(id).unwrap().try_as_i64(), Some(42));
        }
    }
}

#[test]
fn wrong_arity_remains_a_strict_validation_failure() {
    let (declaration, mut registered) = interface(&parse(DECLARATION));
    // An argument-count hint cannot override the source-owned two-slot contract.
    let mut info = registered.export_functions();
    for value in info.values_mut() { if value.param_count == 2 { value.param_count = 1; } }
    registered.import_functions(&info);
    let malformed = parse("module fixture.consumer; mount fixture.provider.transfer; fn probe()->Int{ let mut slot=42; transfer(&mut slot) }");
    let result = consumer(&malformed, &declaration, &registered, true);
    if let Ok(module) = &result {
        for f in &module.functions {
            eprintln!("ARITY {:?} params={} body={:?}", module.strings.get(f.name), f.params.len(),
                verum_vbc::bytecode::decode_instructions(&module.bytecode[f.bytecode_offset as usize..(f.bytecode_offset+f.bytecode_length) as usize]));
        }
    }
    assert!(result.is_err());
}

#[test]
fn actual_runtime_intrinsic_declaration_executes_from_source_and_wire() {
    // Complete actual module, no copied declaration or filtered source AST.
    let root = std::env::var("VERUM_FORWARD_SOURCE_ROOT").unwrap_or_else(|_| format!("{}/../../core", env!("CARGO_MANIFEST_DIR")));
    let source = std::fs::read_to_string(format!("{root}/intrinsics/runtime/time.vr")).unwrap();
    let (declaration, registered) = interface(&parse(&source));
    let caller = parse("module fixture.consumer; mount core.intrinsics.runtime.time.num_cpus; public fn probe()->Int{num_cpus()}");
    let result = consumer(&caller, &declaration, &registered, true).unwrap();
    for module in [result.clone(), through_wire(&result)] {
        let operation = function(&module, "core.intrinsics.runtime.time.num_cpus");
        assert_eq!(operation.return_type, TypeRef::Concrete(TypeId::INT));
        assert!(operation.params.is_empty());
        let entry = function(&module, "fixture.consumer.probe").id;
        assert!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().try_as_i64().unwrap() > 0);
    }
}

#[test]
fn text_conversion_without_real_dependency_does_not_get_a_name_based_forward() {
    let source = parse("public fn probe()->Text{Text.from(\"declaration authority\")}");
    assert!(VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation()).compile_module(&source).is_err());
}

#[test]
fn too_many_arguments_are_rejected_without_changing_the_declaration() {
    let (declaration, registered) = interface(&parse(DECLARATION));
    let caller = parse("module fixture.consumer; mount fixture.provider.transfer; fn probe()->Int{let mut slot=42; transfer(&mut slot, \"x\".as_bytes(), 99)}");
    assert!(consumer(&caller, &declaration, &registered, true).is_err());
}

#[test]
fn declared_default_constant_survives_import_and_wire() {
    let source = parse("module fixture.provider; @intrinsic(\"declared_forward_fixture\") public fn selected(value: Int = 41)->Int;");
    let (declaration, registered) = interface(&source);
    let caller = parse("module fixture.consumer; mount fixture.provider.selected; fn probe()->Int{selected()}");
    for supplied in [&declaration, &through_wire(&declaration)] {
        let result = consumer(&caller, supplied, &registered, true).unwrap();
        for module in [&result, &through_wire(&result)] {
            let target = function(module, "fixture.provider.selected");
            let default = target.params[0].default.expect("declared default");
            assert!(matches!(module.constants[default.0 as usize], verum_vbc::module::Constant::Int(41)));
            let entry = function(module, "fixture.consumer.probe");
            let instructions = verum_vbc::bytecode::decode_instructions(&module.bytecode[entry.bytecode_offset as usize..(entry.bytecode_offset+entry.bytecode_length) as usize]).unwrap();
            assert!(instructions.iter().any(|instruction| matches!(instruction, verum_vbc::Instruction::LoadK { const_id, .. } if matches!(module.constants[*const_id as usize], verum_vbc::module::Constant::Int(41)))));
        }
    }
}

#[test]
fn same_leaf_defaults_remain_with_the_selected_source_owner() {
    let provider = parse("module fixture.provider; public fn selected(value: Int = 41)->Int{value + 1}");
    let sibling = parse("module fixture.sibling; public fn selected(value: Int = 98)->Int{value + 1}");
    let caller = parse("module fixture.consumer; mount fixture.provider.selected; public fn probe()->Int{selected()}");
    for reverse in [false, true] {
        let mut files = [("fixture", &provider), ("fixture", &sibling), ("fixture", &caller)];
        if reverse { files.reverse(); }
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation());
        codegen.initialize(); codegen.collect_owned_unit_declarations(&files).unwrap();
        codegen.resolve_pending_imports(); codegen.compile_owned_unit_items(&files, ItemFailurePolicy::Strict).unwrap();
        let result = codegen.finalize_module().unwrap();
        for module in [result.clone(), through_wire(&result)] {
            let entry = function(&module, "fixture.consumer.probe").id;
            assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().try_as_i64(), Some(42));
        }
    }
}
