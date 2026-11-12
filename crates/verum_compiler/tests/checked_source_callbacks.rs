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
