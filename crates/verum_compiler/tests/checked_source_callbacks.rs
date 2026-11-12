//! Checked method contracts must still invoke the source implementation.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn checked_callback_executes_its_method_before_and_after_serialization() {
    let ast = Parser::new(r#"
module callbacks;
type Event is Value(Int);
type Device is ();
fn apply(f: fn(Int) -> Event) -> Event { f(36) }
implement Device { fn package(value: Int) -> Event { Event.Value(value * 2 + 1) } }
fn probe() -> Int {
    match apply(|value| Device.package(value)) { Event.Value(value) => value }
}
"#).parse_module().expect("callback source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    assert!(checker.prepare_module_declarations(&ast).is_empty());
    for item in &ast.items { checker.check_item(item).expect("source body contract"); }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(!checker.diagnostics().iter().any(|diagnostic| diagnostic.is_error()));

    let module = VbcCodegen::with_config(CodegenConfig::new("callbacks"))
        .compile_module(&ast).expect("lower checked source");
    let restored = deserialize_module(&serialize_module(&module).expect("serialize"))
        .expect("deserialize");
    for module in [module, restored] {
        let entry = module.find_function_by_name("callbacks.probe").expect("source entry");
        let result = Interpreter::new(Arc::new(module)).execute_function(entry)
            .expect("execute checked callback");
        assert_eq!(result.try_as_i64(), Some(73));
    }
}

#[test]
fn imported_source_factories_execute_through_a_static_callback() {
    use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    for (path, source) in [
        ("mod.vr", "module core; public module model; public module device; public module bridge;"),
        ("model.vr", "module core.model;
            public type Failure is { public code: Int };
            implement Failure { public fn create(code: Int) -> Failure { Failure { code: code } } }
            public type Outcome is Value(Int) | Failed(Failure);"),
        ("device.vr", "module core.device;
            mount core.model.{Failure, Outcome};
            public fn error_code(value: Int) -> Int { value }
            public fn inspect(value: Int) -> Outcome {
                if value < 0 { Outcome.Failed(Failure.create(core.device.error_code(value))) }
                else { Outcome.Value(value) }
            }"),
        ("bridge.vr", "module core.bridge;
            mount core.model.Outcome; mount core.device.inspect;
            static HANDLER: fn(Int) -> Outcome = handler;
            fn handler(value: Int) -> Outcome { inspect(value) }
            public fn probe() -> Int {
                let error = match HANDLER(-9) { Outcome.Failed(error) => error.code, _ => 0 };
                let value = match HANDLER(82) { Outcome.Value(value) => value, _ => 0 };
                error + value
            }"),
    ] { std::fs::write(core.join(path), source).unwrap(); }
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core().unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core").unwrap();
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("core.bridge.probe").unwrap();
        let mut interpreter = Interpreter::new(Arc::new(module));
        interpreter.run_global_ctors().unwrap();
        let result = interpreter.execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
}

#[test]
fn a_forward_static_method_keeps_the_declaring_sum_owner() {
    use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module transport;").unwrap();
    std::fs::write(core.join("transport.vr"), r#"
        module core.transport;
        type Outcome<T, E> is Ok(T) | Err(E);
        type Completion is { result: Int32 };
        implement Completion {
            fn to_result(&self) -> Outcome<UInt32, TransportError> {
                if self.result >= 0 { Outcome.Ok(self.result as UInt32) }
                else { Outcome.Err(TransportError.from_code(-self.result)) }
            }
        }
        type TransportError is Blocked | Missing { path: Text } | Other(Text);
        implement TransportError {
            pure fn from_code(code: Int32) -> TransportError {
                match code { 2 => Missing { path: "" }, 11 => Blocked, _ => Other("unknown") }
            }
        }
        public fn probe() -> Int {
            let failed = match (Completion { result: -11 }).to_result() {
                Outcome.Err(TransportError.Blocked) => 73, _ => 0,
            };
            match (Completion { result: 1 }).to_result() {
                Outcome.Ok(1) => failed, _ => 0,
            }
        }
    "#).unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core().unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let module = verum_vbc::archive::read_archive_from_file(&output).unwrap().load_module("core").unwrap();
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("core.transport.probe").unwrap();
        let result = Interpreter::new(Arc::new(module)).execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
}
