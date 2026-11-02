//! Release-gate checks against the chosen serialized stdlib and its bytecode.
use std::{path::PathBuf, sync::Arc};
use verum_compiler::archive_ctx_loader::ArchiveCtxCache;
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::read_archive_from_file,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
#[ignore = "requires the candidate VERUM_TEST_FROZEN_STDLIB archive"]
fn sqlite_snapshot_checks_execute_from_the_published_archive() {
    let archive =
        PathBuf::from(std::env::var_os("VERUM_TEST_FROZEN_STDLIB").expect("candidate .vbca"));
    let archive = read_archive_from_file(archive).unwrap();
    let source = r#"
        module app;
        mount core.database.sqlite.native.l1_pager.snapshot.{SnapshotError, snapshot_new, ensure_fresh};
        fn main() -> Int {
            let snapshot = snapshot_new(11, 0, 0);
            let fresh = match ensure_fresh(&snapshot, 11) {
                Result.Ok(_) => 5,
                Result.Err(_) => 0,
            };
            let stale = match ensure_fresh(&snapshot, 19) {
                Result.Err(error) => match error {
                    SnapshotError.Stale(now, old) => now - old,
                    _ => 0,
                },
                Result.Ok(_) => 0,
            };
            fresh + stale
        }
    "#;
    let app = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new().apply_lazy_with_types(&archive, &mut codegen, &app, &[]);
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let main = module.find_function_by_name("app.main").unwrap();
    let mut vm = Interpreter::new(Arc::new(module));
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(13));
}
