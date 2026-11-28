#![cfg(feature = "codegen")]
#[path = "fixtures/page_metadata.rs"]
mod page_metadata;

use verum_vbc::{
    deserialize::deserialize_module_validated, interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn page_metadata_uses_packed_bytes_on_both_sides_of_the_sqlite_decoder() {
    let source = page_metadata::module();
    let loaded = deserialize_module_validated(&serialize_module(&source).unwrap()).unwrap();
    for module in [source, loaded] {
        let entry = module
            .find_function_by_name("core.mem.allocator.main")
            .unwrap();
        let mut vm = Interpreter::new(module.into());
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
        assert!(vm.state.cbgr_allocations.is_empty());
    }
}

#[test]
fn page_metadata_grants_write_only_to_the_mutable_projection() {
    use verum_vbc::{Capabilities, Value};

    let module = page_metadata::module();
    let reserve = module
        .find_function_by_name("core.mem.allocator.reserve_metadata_page")
        .unwrap();
    let release = module
        .find_function_by_name("core.mem.allocator.release_metadata_page")
        .unwrap();
    let mut vm = Interpreter::new(module.clone().into());
    let base = vm.execute_function(reserve).unwrap().try_as_i64().unwrap();
    assert_ne!(base, 0);
    for (method, writable) in [("slot_metadata", false), ("slot_metadata_mut", true)] {
        let name = format!("core.mem.allocator.AllocPageHeader.{method}");
        let entry = module.find_function_by_name(&name).unwrap();
        let issued = vm
            .execute_function_with_args(entry, &[Value::from_ptr(base as *mut u8)])
            .unwrap()
            .try_as_fat_ref()
            .unwrap();
        assert_eq!(issued.ptr() as usize, base as usize + 64);
        assert_eq!(issued.len(), 0);
        assert!(issued.capabilities().has(Capabilities::READ));
        assert_eq!(issued.capabilities().has(Capabilities::WRITE), writable);
    }
    vm.execute_function_with_args(release, &[Value::from_i64(base)])
        .unwrap();
    assert!(vm.state.cbgr_allocations.is_empty());
}

#[test]
fn raw_slice_intrinsic_rejects_an_immutable_pointee_for_a_mutable_result() {
    let source = r#"fn forge(p: *const UInt8) -> &mut [UInt8] {
        unsafe { @intrinsic("slice_from_raw_parts_mut", p, 2_usize) }
    }"#;
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let failure = verum_vbc::codegen::VbcCodegen::new()
        .compile_module(&ast)
        .unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("writable raw slice requires a mutable pointer"),
        "{failure}"
    );
}
