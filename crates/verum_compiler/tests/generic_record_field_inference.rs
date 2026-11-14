//! Imported generic records retain field-directed literal and parameter evidence.
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter};

#[test]
fn actual_compiler_preserves_an_imported_generic_record_through_archive_and_vm() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    let provider = r#"
        module core.provider;
        public type Inner<T> is { public count: UInt32, public value: T };
        public type Outer<T> is { public inner: Inner<T> };
        implement<T> Outer<T> {
            public fn new(value: T) -> Outer<T> {
                let inner = Inner { count: 1, value: value };
                Outer { inner: inner }
            }
        }
    "#;
    let consumer = r#"
        module core.consumer;
        mount core.provider.Outer as Selected;
        public fn probe() -> Int {
            let value = Selected.new(73);
            value.inner.value
        }
    "#;
    for (name, source) in [
        ("mod.vr", "module core; public module provider; public module consumer;"),
        ("provider.vr", provider),
        ("consumer.vr", consumer),
    ] {
        std::fs::write(core.join(name), source).unwrap();
    }
    let output = directory.path().join("records.vbca");
    let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core().expect("ordinary compiler pipeline");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).expect("real archive reload");
    let module = archive.load_module("core").unwrap();
    let entry = module.find_function_by_name("core.consumer.probe").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
}


#[test]
fn ordinary_unsuffixed_numeric_context_preserves_scalar_range_policy() {
    for (field, value, accepted) in [
        ("Byte", "255", true), ("Byte", "256", false), ("Byte", "-1", false),
        ("Int8", "-128", true), ("Int8", "-129", false),
        ("Int8", "127", true), ("Int8", "128", false),
        ("Int8", "-(128)", true), ("Int8", "(-128)", true),
        ("Int64", "9223372036854775807", true),
        ("Int64", "9223372036854775808", false),
        ("Int64", "-9223372036854775808", true),
        ("Int64", "-9223372036854775809", false),
        ("UInt32", "4294967295", true), ("UInt32", "4294967296", false),
        ("UInt64", "18446744073709551615", true),
        ("UInt64", "18446744073709551616", false), ("UInt64", "-1", false),
        ("Float32", "1.5", true),
        // Existing contextual Float32 admission does not impose a finite-value
        // predicate. Constructor evidence must not invent that restriction.
        ("Float32", "1e40", true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core");
        std::fs::create_dir(&core).unwrap();
        let source = format!("module core; public fn probe() -> Int {{ let fixed: {field} = {value}; 73 }}");
        std::fs::write(core.join("mod.vr"), source).unwrap();
        let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
        let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core)).compile_core();
        match result {
            Ok(result) => assert!(accepted && result.errors.is_empty(), "{field} {value}: {:?}", result.errors),
            Err(error) => assert!(!accepted && error.to_string().contains("E410"), "{field} {value}: {error}"),
        }
    }
}
