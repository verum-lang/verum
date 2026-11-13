//! Both source spellings preserve context protocol calls through archive and VM.
use std::{fs, sync::Arc};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{
    archive::read_archive_from_file, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module, validate::validate_module,
};

#[test]
fn imported_context_protocol_uses_the_provided_instance() {
    for declaration in [
        "context protocol Signal { fn read(&self) -> Int; }",
        "context type Signal is protocol { fn read(&self) -> Int; };",
    ] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("core");
        fs::create_dir(&source).unwrap();
        let entry = source.join("mod.vr");
        fs::write(
            source.join("provider.vr"),
            format!(
                "module core.provider; public {declaration}
             public type Device is {{ public value: Int }};
             implement Signal for Device {{ fn read(&self) -> Int {{ self.value }} }}"
            ),
        )
        .unwrap();
        fs::write(
            &entry,
            "module core; mount core.provider.{Signal as Gate, Device};
             fn read() -> Int using [Gate] { Gate.read() }
             public fn probe() -> Int {
                 provide Gate = Device { value: 42 };
                 let first = read();
                 let mut second = 0;
                 provide Gate = Device { value: 73 } in { second = read(); }
                 first * 100 + second
             }",
        )
        .unwrap();
        let output = root.path().join("context.vbca");
        let mut session = Session::new(CompilerOptions {
            input: entry,
            ..Default::default()
        });
        let result = CompilationPipeline::new_core(
            &mut session,
            CoreConfig::new(&source).with_output(&output),
        )
        .compile_core()
        .unwrap();
        assert!(
            result.errors.is_empty(),
            "{declaration}: {:?}",
            result.errors
        );
        let module = read_archive_from_file(&output)
            .unwrap()
            .load_module("core")
            .unwrap();
        let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        for module in [module, wire] {
            validate_module(&module).unwrap();
            let probe = module.find_function_by_name("core.probe").unwrap();
            assert!(!verum_vbc::stub_ranges::is_stub_id(probe.0));
            let mut vm = Interpreter::new(Arc::new(module));
            vm.run_global_ctors().unwrap();
            assert_eq!(vm.execute_function(probe).unwrap().try_as_i64(), Some(4273));
        }
    }
}


#[test]
fn independent_context_owners_survive_aliases_and_archive_linking() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("core");
    fs::create_dir(&source).unwrap();
    for (file, body) in [
        ("provider.vr", "module core.provider;
            public context Gate { fn read() -> Int; }
            public type Device is { public value: Int };
            implement Device { public fn read(&self) -> Int { self.value } }
            public fn snapshot() -> Int using [Gate] { Gate.read() }"),
        ("other.vr", "module core.other;
            public context Gate { fn read() -> Int; }
            public fn measure() -> Int using [Gate] { Gate.read() }"),
        ("facade.vr", "module core.facade;
            public mount core.provider.{Gate as Primary, Device, snapshot};"),
        ("mod.vr", "module core;
            mount core.facade.{Primary, Device, snapshot};
            mount core.other.{Gate as Secondary, measure};
            public fn probe() -> Int {
                provide Primary = Device { value: 37 };
                provide Secondary = Device { value: 61 };
                let before = snapshot();
                let mut nested = 0;
                provide Primary = Device { value: 73 } in { nested = snapshot(); }
                before * 1000000 + nested * 10000 + snapshot() * 100 + measure()
            }"),
    ] {
        fs::write(source.join(file), body).unwrap();
    }
    let output = root.path().join("context.vbca");
    let mut session = Session::new(CompilerOptions {
        input: source.join("mod.vr"),
        ..Default::default()
    });
    let result = CompilationPipeline::new_core(
        &mut session, CoreConfig::new(&source).with_output(&output),
    ).compile_core().unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core").unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        validate_module(&module).unwrap();
        for (function, owner) in [
            ("core.provider.snapshot", "core.provider.Gate"),
            ("core.other.measure", "core.other.Gate"),
        ] {
            let id = module.find_function_by_name(function).unwrap();
            let descriptor = module.get_function(id).unwrap();
            let contexts: Vec<_> = descriptor.contexts.iter().map(|reference|
                module.strings.get(module.context_names[reference.0 as usize]).unwrap()
            ).collect();
            assert_eq!(contexts, [owner], "{function}");
        }
        let probe = module.find_function_by_name("core.probe").unwrap();
        assert!(!verum_vbc::stub_ranges::is_stub_id(probe.0));
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(probe).unwrap().try_as_i64(), Some(37733761));
    }
}
