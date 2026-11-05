//! Declaration identity survives runtime linkage, relocation and wire round trips.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen}, interpreter::Interpreter,
    module::{FunctionDescriptor, FunctionId, VbcModule},
};
fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new("fixture.nested").with_validation())
        .compile_module(&ast).unwrap()
}
fn leaf<'a>(module: &'a VbcModule, function: &FunctionDescriptor) -> Option<&'a str> {
    function.declaration_identity.as_ref().and_then(|identity| module.strings.get(identity.source_name))
}
fn outer(module: &VbcModule) -> FunctionId {
    module.functions.iter().find(|function| leaf(module, function) == Some("outer")
        && function.declaration_identity.as_ref().unwrap().lexical_parent.is_none()).unwrap().id
}
fn child_index(module: &VbcModule) -> usize {
    module.functions.iter().position(|function| function.declaration_identity.as_ref()
        .is_some_and(|identity| identity.lexical_parent.is_some())).unwrap()
}
fn assert_source_and_wire(module: VbcModule) {
    let decoded = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    for module in [module, decoded] {
        verum_vbc::validate::validate_module(&module).unwrap();
        verum_vbc::callable_identity::validate_module(&module).unwrap();
        let entry = outer(&module);
        assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().try_as_i64(), Some(42));
    }
}
const SIMPLE: &str = "module fixture.nested; public fn outer()->Int{fn inner(value:Int)->Int{value+1} inner(41)}";
const GENERIC: &str = "module fixture.nested; public fn outer()->Int{fn inner<N:meta Int>(value:Int)->Int{value} 42}";
#[test]
fn ordinary_nested_callable_executes_source_and_wire() { assert_source_and_wire(compile(SIMPLE)); }
#[test]
fn generic_nested_declaration_keeps_source_name_and_parent() {
    let module = compile(GENERIC);
    let child = &module.functions[child_index(&module)];
    assert_eq!(leaf(&module, child), Some("inner"));
    assert_eq!(child.source_signature.as_ref().unwrap().name.name.as_str(), "inner");
    assert_eq!(child.declaration_identity.as_ref().unwrap().lexical_parent, Some(outer(&module)));
    assert_ne!(module.strings.get(child.name), Some("inner"));
    assert_source_and_wire(module);
}
#[test]
fn same_source_name_in_distinct_parents_does_not_alias() {
    assert_source_and_wire(compile("module fixture.nested; fn left()->Int{fn item()->Int{20} item()} fn right()->Int{fn item()->Int{22} item()} public fn outer()->Int{left()+right()}"));
}
#[test]
fn method_local_identity_retains_nominal_and_lexical_boundaries() {
    let module = compile("module fixture.nested; type Left is {marker:()}; type Right is {marker:()}; implement Left {fn first()->Int{fn item()->Int{20} item()}} implement Right {fn second()->Int{fn item()->Int{22} item()}} public fn outer()->Int{Left.first()+Right.second()}");
    for function in &module.functions {
        if leaf(&module, function) == Some("item") {
            assert!(function.parent_type.is_none());
            let parent = function.declaration_identity.as_ref().unwrap().lexical_parent.unwrap();
            let parent = module.functions.iter().find(|candidate| candidate.id == parent).unwrap();
            assert!(parent.parent_type.is_some());
        }
    }
    assert_source_and_wire(module);
}
#[test]
fn source_location_is_not_lexical_ownership_authority() {
    let mut module = compile(GENERIC);
    let index = child_index(&module);
    module.functions[index].source_signature.as_mut().unwrap().span.file_id = verum_ast::FileId::new(991);
    assert_source_and_wire(module);
}
fn reject(mutator: impl FnOnce(&mut VbcModule, usize)) {
    let mut module = compile(GENERIC);
    let index = child_index(&module);
    mutator(&mut module, index);
    assert!(verum_vbc::callable_identity::validate_module(&module).is_err());
    assert!(verum_vbc::serialize::serialize_module(&module).is_err());
    assert!(verum_vbc::validate::validate_module(&module).is_err());
}
#[test]
fn dangling_parent_is_refused() {
    reject(|module, index| module.functions[index].declaration_identity.as_mut().unwrap().lexical_parent = Some(FunctionId(999_999)));
}
#[test]
fn cyclic_parent_is_refused() {
    reject(|module, index| {
        let child = module.functions[index].id;
        let parent = outer(module);
        module.functions.iter_mut().find(|function| function.id == parent).unwrap()
            .declaration_identity.as_mut().unwrap().lexical_parent = Some(child);
    });
}
#[test]
fn foreign_module_parent_is_refused() {
    reject(|module, index| {
        let foreign = module.intern_string("fixture.foreign");
        module.functions[index].origin_module = Some(foreign);
    });
}
#[test]
fn changed_source_name_is_refused() {
    reject(|module, index| {
        let forged = module.intern_string("another_declaration");
        module.functions[index].declaration_identity.as_mut().unwrap().source_name = forged;
    });
}
#[test]
fn erased_parent_declaration_is_refused() {
    reject(|module, _| {
        let parent = outer(module);
        module.functions.iter_mut().find(|function| function.id == parent).unwrap().declaration_identity = None;
    });
}
#[test]
fn linker_relocates_typed_parent_and_source_string() {
    let mut linker = verum_vbc::linker::VbcLinker::new("aarch64-apple-darwin");
    linker.add_user_module(compile("module fixture.padding; public fn padding()->Int{7}")).unwrap();
    linker.add_user_module(compile(SIMPLE)).unwrap();
    assert_source_and_wire(linker.finalize());
}
#[test]
fn mono_compaction_relocates_absolute_parent_independent_of_bytecode_base() {
    let mut module = compile(GENERIC);
    for function in &mut module.functions {
        function.id.0 += 100;
        function.func_id_base = 700; // instruction namespace only; this body makes no calls.
        if let Some(parent) = &mut function.declaration_identity.as_mut().unwrap().lexical_parent { parent.0 += 100; }
    }
    let (module, _) = verum_vbc::mono::ModuleMerger::new(module, None, vec![],
        verum_vbc::mono::MonomorphizationResolver::new()).merge().unwrap();
    assert_source_and_wire(module);
}
#[test]
fn wire_reader_refuses_corrupted_typed_parent() {
    let module = compile(GENERIC);
    let identity = module.functions[child_index(&module)].declaration_identity.as_ref().unwrap();
    let mut pattern = identity.source_name.0.to_le_bytes().to_vec();
    pattern.push(1);
    pattern.extend_from_slice(&identity.lexical_parent.unwrap().0.to_le_bytes());
    let mut bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let positions: Vec<_> = bytes.windows(pattern.len()).enumerate()
        .filter_map(|(offset, found)| (found == pattern).then_some(offset)).collect();
    assert_eq!(positions.len(), 1, "the selected typed wire field must be unambiguous");
    let offset = positions[0] + 5;
    bytes[offset..offset + 4].copy_from_slice(&999_999_u32.to_le_bytes());
    assert!(verum_vbc::deserialize::deserialize_module(&bytes).unwrap_err().to_string().contains("lexical parent"));
}
