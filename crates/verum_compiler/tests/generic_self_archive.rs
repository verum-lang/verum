//! Generic Self method conformance through checked source, archive and wire execution.
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
                "invalid Self conformance published an archive"
            );
            Err(format!("{error:#}").into())
        }
    }
}

fn execute(source: &str) {
    let module = produce(source).expect("checked generic Self conformance");
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
            .unwrap_or_else(|error| panic!("{route}: {error:?}"));
        assert_eq!(result.try_as_i64(), Some(73), "{route}");
    }
}
#[test]
fn a_generic_self_return_preserves_the_actual_value_through_archive_and_wire() {
    execute(
        "module core; type Holder<T> is {value:T}; type Echo is protocol {fn echo(self)->Self;}; implement<T> Echo for Holder<T> {fn echo(self)->Holder<T>{self}} public fn probe()->Int {let item=Holder{value:73}; item.echo().value}",
    );
}
#[test]
fn inverse_generic_self_parameters_preserve_the_declared_receiver() {
    execute(
        "module core; type Holder<T> is {value:T}; type Observe<A> is protocol {fn observe(&self, other:&Holder<A>)->Int;}; implement<T> Observe<T> for Holder<T> {fn observe(&self,other:&Self)->Int{73}} public fn probe()->Int {let first=Holder{value:11};let second=Holder{value:37};first.observe(&second)}",
    );
}
#[test]
fn incompatible_generic_self_carriers_do_not_publish_an_archive() {
    for actual in ["Holder<Bool>", "foreign.Holder<T>"] {
        let source = format!(
            "module core; type Holder<T> is {{value:T}}; module foreign {{public type Holder<T> is {{value:T}};}} type Observe is protocol {{fn observe(&self,other:&Self)->Unit;}}; implement<T> Observe for Holder<T> {{fn observe(&self,other:&{actual})->Unit{{}}}}"
        );
        let error = produce(&source).expect_err("distinct declaration or argument");
        assert!(error.contains("E427"), "{actual}: {error}");
    }
}
