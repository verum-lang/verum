//! Exercise the default allocator's mounted public declarations on both backends.
#[path = "support/source_image.rs"]
mod source_image;

// Build the executable link unit from the operation declarations used by this
// consumer. These operations are bodyless and their implementation is the
// intrinsic registry, interpreter, and native lowering.
fn operations(source: &str, file: u32, names: &[&str]) -> verum_ast::Module {
    use verum_ast::ItemKind;
    let mut module = verum_fast_parser::FastParser::new()
        .parse_module_str(source, verum_ast::FileId::new(file))
        .expect("stdlib declaration grammar");
    module.items.retain(|item| match &item.kind {
        ItemKind::Module(header) => header.items.is_none(),
        ItemKind::Type(declared) => names.contains(&declared.name.name.as_str()),
        ItemKind::Function(declared) => names.contains(&declared.name.name.as_str()),
        _ => false,
    });
    assert_eq!(
        module.items.len(),
        1 + names.len(),
        "module header and every requested declaration"
    );
    module
}

fn allocated_blocks() -> verum_vbc::VbcModule {
    use verum_vbc::{
        codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
        deserialize::deserialize_module,
        serialize::serialize_module,
    };
    let source = concat!(
        include_str!("../../../core-tests/mem/allocator/fresh_storage_test.vr"),
        "\nfn main() -> Int { allocation_control() }",
    );
    let source = verum_fast_parser::FastParser::new()
        .parse_module_str(source, verum_ast::FileId::new(0))
        .expect("source grammar");
    let allocator = operations(
        include_str!("../../../core/mem/allocator.vr"),
        1,
        &["cbgr_alloc", "cbgr_alloc_zeroed", "AllocError"],
    );
    let cbgr = operations(
        include_str!("../../../core/intrinsics/runtime/cbgr.vr"),
        2,
        &["cbgr_deallocate"],
    );
    let raw = operations(
        include_str!("../../../core/intrinsics/runtime/mem_raw.vr"),
        3,
        &["load_byte"],
    );
    let result = operations(include_str!("../../../core/base/result.vr"), 4, &["Result"]);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("allocator_writable"));
    let files = [&result, &allocator, &cbgr, &raw, &source];
    codegen
        .collect_unit_declarations(&files)
        .expect("declared allocator imports");
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&files, ItemFailurePolicy::Strict)
        .expect("public allocator calls");
    let wire = codegen
        .finalize_module_from_state()
        .expect("linked allocator consumer");
    let wire = deserialize_module(&serialize_module(&wire).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).expect("validated allocator consumer");
    wire
}

#[test]
fn default_allocator_supplies_distinct_writable_blocks_in_vm() {
    let wire = allocated_blocks();
    let entry = wire
        .find_function_by_name("allocator_writable.main")
        .unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(wire.into());
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    assert!(vm.state.cbgr_allocations.is_empty());
}

#[test]
#[cfg(target_os = "macos")]
fn default_allocator_supplies_distinct_writable_blocks_in_native_image() {
    source_image::run_module(allocated_blocks(), |_| {});
}
