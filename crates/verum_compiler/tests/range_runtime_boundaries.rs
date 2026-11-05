//! T2359: source range iteration at zero and full-width endpoints.
//! These are independent small source fixtures, not a pruned standard library.
use std::{fs, sync::Arc};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{VbcModule, archive::read_archive_from_file, interpreter::Interpreter};

fn produce(source: &str) -> Result<VbcModule, String> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), source).unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core();
    match result {
        Ok(result) => {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            Ok(read_archive_from_file(&output).unwrap().load_module("core").unwrap())
        }
        Err(error) => {
            assert!(!output.exists(), "rejected source published an archive");
            Err(format!("{error:#}"))
        }
    }
}

fn execute(source: &str, expected: i64) {
    let module = produce(source).unwrap_or_else(|error| panic!("source:\n{source}\n{error}"));
    verum_vbc::validate::validate_module(&module).expect("strict producer validation");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize actual module"),
    ).expect("deserialize actual module");
    for (route, module) in [("produced", module), ("wire", wire)] {
        verum_vbc::validate::validate_module(&module).expect("strict execution validation");
        let entry = module.find_function_by_name("core.probe").expect("actual probe declaration");
        assert!(!verum_vbc::stub_ranges::is_stub_id(entry.0), "probe is a forward stub");
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().expect("actual global initialization");
        let result = vm.execute_function(entry).unwrap_or_else(|error| panic!("{route}: {error:?}"));
        assert_eq!(result.try_as_i64(), Some(expected), "{route}: receiver/body dispatch changed");
    }
}



#[test]
fn crosses_zero_without_restarting() {
    execute("module core; public fn probe() -> Int { let mut count = 0; let mut sum = 0; for value in -2..=2 { count += 1; if count > 5 { return -99; } sum += value; } count * 10 + sum }", 50);
}
#[test]
fn inclusive_maximum_yields_once_and_finishes() {
    execute("module core; public fn probe() -> Int { let mut count = 0; for value in 9223372036854775806..=9223372036854775807 { count += 1; if count > 2 { return -99; } } count }", 2);
}
#[test]
fn singleton_maximum_is_not_empty() {
    execute("module core; public fn probe() -> Int { let mut count = 0; for value in 9223372036854775807..=9223372036854775807 { count += 1; if count > 1 { return -99; } } count }", 1);
}
#[test]
fn exclusive_maximum_finishes_without_advancing_past_it() {
    execute("module core; public fn probe() -> Int { let mut count = 0; for value in 9223372036854775806..9223372036854775807 { count += 1; if count > 1 { return -99; } } count }", 1);
}
#[test]
fn backwards_range_is_empty() {
    execute("module core; public fn probe() -> Int { let mut count = 0; for value in 2..=-2 { count += 1; if count > 1 { return -99; } } count }", 0);
}
#[test]
fn minimum_range_preserves_negative_elements() {
    execute("module core; public fn probe() -> Int { let mut count = 0; for value in -9223372036854775808..-9223372036854775806 { count += 1; if count > 2 { return -99; } } count }", 2);
}
