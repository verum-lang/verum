//! File IDs identify source locations, never the namespace of a declaration.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::{Interpreter, ObjectHeader},
    serialize::serialize_module,
    types::TypeKind,
    validate::validate_module,
};

fn check_sum_owner(sources: &[&str], root: &str, owner: &str) {
    let ast: verum_common::List<_> = sources.iter()
        .map(|source| Parser::new(source).parse_module().unwrap()).collect();
    let roots: verum_common::List<_> = ast.iter().collect();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new(root));
    compiler.collect_unit_declarations(&roots).unwrap();
    compiler.compile_unit_items(&roots, ItemFailurePolicy::Strict).unwrap();
    let compiled = compiler.finalize_module_from_state().unwrap();
    let module = deserialize_module(&serialize_module(&compiled).unwrap()).unwrap();
    validate_module(&module).unwrap();
    let declaration = module.types.iter().find(|ty| ty.kind == TypeKind::Sum
        && module.get_string(ty.name).is_some_and(|name| name == "Packet" || name.ends_with(".Packet")))
        .unwrap();
    let declared_owner = declaration.origin_module.and_then(|id| module.get_string(id))
        .unwrap_or(&module.name);
    assert_eq!(declared_owner, owner);
    let expected = declaration.id;
    let entry = module.functions.iter().find(|function|
        function.source_signature.as_ref().is_some_and(|source| source.name.name.as_str() == "probe"))
        .unwrap();
    assert_eq!(entry.semantic_return.as_ref().and_then(|ty| ty.base_type_id()), Some(expected));
    let entry = entry.id;
    let mut vm = Interpreter::new(Arc::new(module));
    let result = vm.execute_function(entry).unwrap();
    assert!(result.is_regular_ptr());
    let pointer = result.as_ptr::<ObjectHeader>();
    assert!(vm.state.heap.contains(pointer));
    // SAFETY: exact membership establishes a live header for this VM result.
    assert_eq!(unsafe { (*pointer).type_id }, expected,
        "a declared sum cannot be replaced by a synthetic tag-only allocation");
}

#[test]
fn headerless_and_named_units_keep_the_same_nominal_owner() {
    let body = "type Packet<T> is Empty | Held(T); fn probe() -> Packet<Int> { Packet.Held(73) }";
    check_sum_owner(&[body], "main", "main");
    check_sum_owner(&[body], "workspace", "workspace");
    check_sum_owner(&[&format!("module app; {body}")], "main", "app");
}

#[test]
fn headerless_files_share_the_declared_unit_not_their_parser_file_id() {
    let declaration = "type Packet<T> is Empty | Held(T);";
    let consumer = "fn probe() -> Packet<Int> { Packet.Held(73) }";
    check_sum_owner(&[declaration, consumer], "main", "main");
    check_sum_owner(&[consumer, declaration], "main", "main");
}

#[test]
fn protocol_signatures_resolve_headerless_local_record_declarations() {
    let ast = Parser::new("type Context is { token: Int }; type Visitor is protocol { fn visit(&self, cx: &Context); };")
        .parse_module().unwrap();
    VbcCodegen::new().compile_module(&ast).expect("a local record is visible to its protocol signature");
}
