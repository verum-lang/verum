//! Refinement method conformance through checked source, archive and wire execution.
use std::{fs, sync::Arc};
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{VbcModule, archive::read_archive_from_file, interpreter::Interpreter};

fn produce(source: &str) -> Result<VbcModule, Text> {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), source).unwrap();
    let output = directory.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    match CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core()
    {
        Ok(result) => {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            Ok(read_archive_from_file(&output)
                .unwrap()
                .load_module("core")
                .unwrap())
        }
        Err(error) => {
            assert!(
                !output.exists(),
                "invalid method conformance published an archive"
            );
            Err(format!("{error:#}").into())
        }
    }
}
fn source(actual: &str) -> Text {
    format!(
        "module core;
        type Contract is protocol {{fn pass(value:Int{{>=0}})->Int{{>=0}};}};
        type Worker is ();
        implement Contract for Worker {{fn pass(value:{actual})->Int{{>=0}} {{value}}}}
        public fn probe()->Int {{Worker.pass(73)}}"
    )
    .into()
}
#[test]
fn independently_declared_refinements_survive_archive_and_wire() {
    let module = produce(&source("Int{>=0}")).expect("checked method contract");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for (route, module) in [("archive", module), ("wire", wire)] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("core.probe").unwrap();
        assert!(!verum_vbc::stub_ranges::is_stub_id(entry.0));
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().unwrap();
        let result = vm
            .execute_function(entry)
            .unwrap_or_else(|e| panic!("{route}: {e:?}"));
        assert_eq!(result.try_as_i64(), Some(73), "{route}");
    }
}
#[test]
fn stronger_or_different_domains_do_not_publish_a_method_archive() {
    for actual in ["Int{>0}", "Int{>=1}", "UInt8{>=0}"] {
        let error = produce(&source(actual)).expect_err("changed declared refinement domain");
        assert!(error.contains("E427"), "{actual}: {error}");
    }
}
