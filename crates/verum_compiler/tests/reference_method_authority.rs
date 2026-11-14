//! Ordinary source, archive and execution agree on declared reference methods.
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter};

#[test]
fn declared_inherent_and_dynamic_methods_survive_archive_reload() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    let provider = r#"
        module core.provider;
        public type Report is { public value: Int };
        public type Inspect is protocol { fn capabilities(&self) -> Report; };
        public type Item is { public value: Int };
        implement Item { public fn epoch(&self) -> Report { Report { value: self.value } } }
        implement Inspect for Item {
            fn capabilities(&self) -> Report { Report { value: self.value } }
        }
    "#;
    let consumer = r#"
        module core.consumer;
        mount core.provider.{Item as Selected, Inspect};
        fn dynamic(value: &dyn Inspect) -> Int { value.capabilities().value }
        fn inherent(value: &Selected) -> Int { value.epoch().value }
        public fn probe() -> Int {
            let left = Selected { value: 36 };
            let right = Selected { value: 37 };
            inherent(&left) + dynamic(&right)
        }
    "#;
    for (name, source) in [
        ("mod.vr", "module core; public module provider; public module consumer;"),
        ("provider.vr", provider), ("consumer.vr", consumer),
    ] { std::fs::write(core.join(name), source).unwrap(); }
    let output = directory.path().join("methods.vbca");
    let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core().expect("ordinary compiler pipeline");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).expect("archive reload");
    let module = archive.load_module("core").unwrap();
    let entry = module.find_function_by_name("core.consumer.probe").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
}
