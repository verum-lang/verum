#![cfg(feature = "codegen")]
#[path = "fixtures/maybe_replace_owner.rs"]
mod fixture;

#[test]
fn actual_maybe_replace_and_take_transfer_each_owner_once() {
    let source = fixture::module();
    let bytes = verum_vbc::serialize::serialize_module(&source).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for module in [source, wire] {
        let entry = module
            .find_function_by_name("core.base.maybe.main")
            .unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}
