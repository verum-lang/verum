//! Exercise the default allocator's mounted public declarations on both backends.
#[path = "support/source_image.rs"]
mod source_image;
#[cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/page_metadata.rs"]
mod page_metadata;

#[test]
#[cfg(target_os = "macos")]
fn page_metadata_uses_packed_bytes_in_the_native_sqlite_decoder() {
    let bytes = verum_vbc::serialize::serialize_module(&page_metadata::module()).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    source_image::run_module(wire, |_| {});
}

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
    allocator_client(concat!(
        include_str!("../../../core-tests/mem/allocator/fresh_storage_test.vr"),
        "\nfn main() -> Int { allocation_control() }",
    ))
}

fn allocator_client(source: &str) -> verum_vbc::VbcModule {
    use verum_vbc::{
        codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
        deserialize::deserialize_module,
        serialize::serialize_module,
    };
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

const FAILURE_CLIENT: &str = r#"
    module allocation_errors;
    mount core.base.result.Result;
    mount core.mem.allocator.{AllocError, cbgr_alloc, cbgr_alloc_zeroed};

    type StorageIssue is Other | OutOfMemory { requested: Int } | InvalidSize { size: Int };
    @intrinsic("cbgr_realloc")
    fn resize(ptr: &unsafe Byte, old: Int, size: Int, alignment: Int)
        -> Result<(&unsafe Byte, UInt32, UInt16), StorageIssue>;

    fn main() -> Int {
        let invalid = match cbgr_alloc(-13, 8) {
            Result.Err(AllocError.InvalidSize { size }) => size, _ => 0,
        };
        let exhausted = match cbgr_alloc_zeroed(19, 8) {
            Result.Err(AllocError.OutOfMemory { requested }) => requested, _ => 0,
        };
        let refused = match resize(0 as &unsafe Byte, 0, -7, 8) {
            Result.Err(StorageIssue.InvalidSize { size }) => size, _ => 0,
        };
        let exhausted_resize = match resize(0 as &unsafe Byte, 0, 34, 8) {
            Result.Err(StorageIssue.OutOfMemory { requested }) => requested, _ => 0,
        };
        if invalid == -13 && exhausted == 19 && refused == -7 && exhausted_resize == 34 { 73 } else { 0 }
    }
"#;

#[test]
fn allocation_failures_use_each_declared_error_in_vm() {
    let module = allocator_client(FAILURE_CLIENT);
    let entry = module
        .find_function_by_name("allocation_errors.main")
        .unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(module.into());
    vm.state.cbgr_generation_counter = u64::from(verum_common::cbgr::GEN_MAX);
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    assert!(vm.state.cbgr_allocations.is_empty());
}

#[test]
#[cfg(target_os = "macos")]
fn allocation_failures_use_each_declared_error_in_native_image() {
    source_image::run_module(allocator_client(FAILURE_CLIENT), |module| {
        let counter = module.get_global("verum_ir_generation_counter").unwrap();
        let word = counter
            .get_initializer()
            .unwrap()
            .into_int_value()
            .get_type();
        counter.set_initializer(&word.const_int(u64::from(verum_common::cbgr::GEN_MAX), false));
    });
}
