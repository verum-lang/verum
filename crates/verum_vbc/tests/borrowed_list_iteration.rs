//! The declared cursor borrows live List storage through independent positions.
#![cfg(feature = "codegen")]
#[path = "fixtures/borrowed_list_iteration.rs"]
mod fixture;

#[test]
fn borrowed_list_positions_survive_the_next_cursor_call() {
    let original = fixture::module();
    let bytes = verum_vbc::serialize::serialize_module(&original).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for module in [original, wire] {
        let entry = module
            .find_function_by_name("core.collections.list.main")
            .unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}
