//! Script callbacks retain their host environment and never borrow a foreign heap.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, Value,
    bytecode::encode_instruction,
    codegen::{CodegenConfig, VbcCodegen},
    instruction::{LayoutProperty, Reg},
    interpreter::{Interpreter, script_engine::{ScriptEngine, install_compiler_hook}},
    module::{FunctionDescriptor, FunctionId, VbcModule},
    types::{TypeId, TypeParamDescriptor, TypeParamId, TypeRef},
};

fn compile(source: &str) -> Result<VbcModule, String> {
    let ast = Parser::new(source).parse_module().map_err(|e| format!("{e:?}"))?;
    VbcCodegen::with_config(CodegenConfig::new("script_callable"))
        .compile_module(&ast).map_err(|e| format!("{e:?}"))
}

fn run_source(source: &str) -> Value {
    install_compiler_hook(Arc::new(compile));
    let module = Arc::new(compile(source).unwrap());
    let entry = module.find_function_by_unique_bare_suffix("main").unwrap();
    Interpreter::new(module).execute_function(entry).unwrap_or_else(|error| panic!("{error:?}\n{source}"))
}

fn script() -> Arc<VbcModule> {
    Arc::new(compile("fn main() -> Int { @intrinsic(\"script_host_call_int\", \"callback\", 34) }").unwrap())
}

fn creator(source: &str) -> (Interpreter, Value) {
    let module = Arc::new(compile(source).unwrap());
    let entry = module.find_function_by_unique_bare_suffix("make").unwrap();
    let mut host = Interpreter::new(module);
    let value = host.execute_function(entry).unwrap();
    (host, value)
}

const CAPTURED: &str = "fn make() -> fn(Int) -> Int { let offset = 8; |x: Int| x + offset }";

#[test]
fn standard_library_engine_preserves_opaque_ids_and_callbacks() {
    install_compiler_hook(Arc::new(compile));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("core/script/engine.vr");
    let source = std::fs::read_to_string(&path).unwrap() + r#"
        fn probe() -> Int {
            let engine = Engine.new();
            let offset = 8;
            engine.register("callback", |x: Int| x + offset);
            match engine.eval("fn main() -> Int { @intrinsic(\"script_host_call_int\", \"callback\", 34) }") {
                ScriptResult.Ok(ScriptValue.Int(value)) => value,
                _ => -1,
            }
        }
    "#;
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module_with_mounts(
        &ast, path.to_str().unwrap(), root.join("core").to_str().unwrap(),
    ).unwrap();
    let entry = module.find_function_by_unique_bare_suffix("probe").unwrap();
    assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().as_i64(), 42);
}

#[test]
fn actual_registration_preserves_returned_capture_and_nested_generic_evidence() {
    for (factory, expected) in [
        ("fn make() -> fn(Int) -> Int { let offset = 8; |x: Int| x + offset }", 42),
        ("fn build<T>() -> fn(Int) -> Int { |x: Int| { let inner = || x + 8; inner() } } fn make() -> fn(Int) -> Int { build<Int>() }", 42),
        ("fn build<T>() -> fn(T) -> T { |x: T| { let inner = || x; inner() } } fn make() -> fn(Int) -> Int { build<Int>() }", 34),
    ] {
        let source = format!(r#"{factory}
            fn main() -> Int {{
                let engine: Int = @intrinsic("script_engine_new");
                let callback = make();
                @intrinsic("script_engine_register", engine, "callback", callback);
                let result = @intrinsic("script_engine_eval", engine,
                    "fn main() -> Int {{ @intrinsic(\"script_host_call_int\", \"callback\", 34) }}");
                if !@intrinsic("script_outcome_is_ok", result) {{
                    panic(@intrinsic("script_outcome_error", result));
                }}
                let value = @intrinsic("script_outcome_as_int", result);
                @intrinsic("script_outcome_free", result);
                @intrinsic("script_engine_free", engine);
                value
            }}"#);
        assert_eq!(run_source(&source).as_i64(), expected, "{factory}");
    }
}

#[test]
fn host_movement_is_safe_but_other_heap_expired_host_and_reset_refuse() {
    let (host, value) = creator(CAPTURED);
    let module = host.state.module.clone();
    let mut engine = ScriptEngine::new();
    engine.register_callable("callback", value, &host.state).unwrap();
    // Moving the interpreter changes its address, not its callback authority.
    let mut moved = Box::new(host);
    let success = engine.run_with_host(script(), "main", &[], Some(&mut moved.state));
    assert!(success.is_ok(), "{:?}", success.error);
    assert_eq!(success.as_int(), 42);
    assert_eq!(moved.state.call_stack.depth(), 0);

    let mut foreign = Interpreter::new(module.clone());
    let refused = engine.run_with_host(script(), "main", &[], Some(&mut foreign.state));
    assert!(refused.error_message().unwrap().contains("different or expired host"));
    assert_eq!(foreign.state.call_stack.depth(), 0);
    moved.state.reset();
    let refused = engine.run_with_host(script(), "main", &[], Some(&mut moved.state));
    assert!(refused.error_message().unwrap().contains("different or expired host"));
    drop(moved);
    let refused = engine.run_with_host(script(), "main", &[], Some(&mut foreign.state));
    assert!(refused.error_message().unwrap().contains("different or expired host"));
    assert!(engine.run_to_outcome(script(), "main", &[]).error_message().unwrap().contains("unavailable"));
}

#[test]
fn registration_rejects_noncallables_foreign_allocations_and_missing_witnesses() {
    let (mut host, closure) = creator(CAPTURED);
    let foreign = Interpreter::new(host.state.module.clone());
    let mut engine = ScriptEngine::new();
    for invalid in [Value::unit(), Value::from_i64(7), Value::from_ptr(0x1000usize as *mut u8)] {
        assert!(engine.register_callable("callback", invalid, &host.state).is_err());
    }
    assert!(engine.register_callable("callback", closure, &foreign.state).is_err());
    for (id, bytes) in [(TypeId::INT, 8), (TypeId(0xC000), 0)] {
        let object = host.state.heap.alloc(id, bytes).unwrap();
        assert!(engine.register_callable("callback", Value::from_ptr(object.as_ptr()), &host.state).is_err());
    }
    let (generic, value) = witnessed_host(false);
    assert!(engine.register_callable("callback", value, &generic.state).unwrap_err()
        .to_string().contains("lexical witness mismatch"));
}

#[test]
fn callback_errors_propagate_and_host_remains_usable() {
    let (mut host, callback) = creator("fn make() -> fn(Int) -> Int { |x: Int| { assert(x == 0); x } }");
    let mut engine = ScriptEngine::new();
    engine.register_callable("callback", callback, &host.state).unwrap();
    let failed = engine.run_with_host(script(), "main", &[], Some(&mut host.state));
    assert!(!failed.is_ok(), "a trapped callback must not become successful zero");
    assert_eq!(failed.error.unwrap().kind_code(), 2);
    assert_eq!(host.state.call_stack.depth(), 0);
    let healthy = Arc::new(compile("fn main() -> Int { @intrinsic(\"script_host_call_int\", \"callback\", 0) }").unwrap());
    let success = engine.run_with_host(healthy, "main", &[], Some(&mut host.state));
    assert!(success.is_ok(), "{:?}", success.error);
    assert_eq!(success.as_int(), 0);
}

#[test]
fn invalid_intrinsic_registration_and_unknown_callback_are_errors() {
    let source = "fn main() { let e = @intrinsic(\"script_engine_new\"); @intrinsic(\"script_engine_register\", e, \"bad\", 7); }";
    let module = Arc::new(compile(source).unwrap());
    let entry = module.find_function_by_unique_bare_suffix("main").unwrap();
    assert!(Interpreter::new(module).execute_function(entry).is_err());
    let (mut host, _) = creator(CAPTURED);
    let mut engine = ScriptEngine::new();
    assert!(engine.run_with_host(script(), "main", &[], Some(&mut host.state))
        .error_message().unwrap().contains("unavailable"));
}

fn add(module: &mut VbcModule, name: &str, generic: bool, body: &[Instruction]) {
    let mut function = FunctionDescriptor::new(module.intern_string(name));
    function.id = FunctionId(module.functions.len() as u32);
    function.register_count = 8;
    if generic {
        function.type_params.push(TypeParamDescriptor { id: TypeParamId(42), ..Default::default() });
    }
    function.bytecode_offset = module.bytecode.len() as u32;
    for instruction in body { encode_instruction(instruction, &mut module.bytecode); }
    function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
    module.functions.push(function);
}

fn witnessed_host(evidence: bool) -> (Interpreter, Value) {
    let mut module = VbcModule::new("host_lexical".into());
    add(&mut module, "make", false, &[
        Instruction::NewClosure { dst: Reg(0), func_id: 1, captures: vec![], capture_types: Some(vec![]),
            lexical_types: if evidence { vec![TypeRef::Concrete(TypeId::INT)] } else { vec![] } },
        Instruction::Ret { value: Reg(0) },
    ]);
    add(&mut module, "callback", true, &[
        Instruction::TypeLayout { dst: Reg(0), type_ref: TypeRef::Generic(TypeParamId(42)), property: LayoutProperty::Size },
        Instruction::Ret { value: Reg(0) },
    ]);
    let mut host = Interpreter::new(Arc::new(module));
    let callable = host.execute_function(FunctionId(0)).unwrap();
    (host, callable)
}

#[test]
fn sparse_lexical_witnesses_and_rust_function_registration_use_shared_dispatch() {
    let (mut host, value) = witnessed_host(true);
    let mut engine = ScriptEngine::new();
    engine.register_callable("callback", value, &host.state).unwrap();
    let result = engine.run_with_host(script(), "main", &[], Some(&mut host.state));
    assert!(result.is_ok(), "{:?}", result.error);
    assert_eq!(result.as_int(), 8);
    // A raw FunctionId has no lexical witness; it cannot borrow one from the caller.
    engine.register("callback", FunctionId(1));
    assert!(engine.run_with_host(script(), "main", &[], Some(&mut host.state))
        .error_message().unwrap().contains("lexical witness mismatch"));
    let module = Arc::new(compile("fn double(x: Int) -> Int { x * 2 }").unwrap());
    let function = module.find_function_by_unique_bare_suffix("double").unwrap();
    let mut host = Interpreter::new(module);
    engine.register("callback", function);
    let result = engine.run_with_host(script(), "main", &[], Some(&mut host.state));
    assert!(result.is_ok(), "{:?}", result.error);
    assert_eq!(result.as_int(), 68);
}

#[test]
fn active_engine_free_register_and_eval_refuse_without_destroying_the_engine() {
    for action in [
        "@intrinsic(\"script_engine_free\", engine);",
        "@intrinsic(\"script_engine_register\", engine, \"other\", identity);",
        "let nested = @intrinsic(\"script_engine_eval\", engine, \"fn main() -> Int { 99 }\");",
    ] {
        let source = format!(r#"
            fn identity(x: Int) -> Int {{ x }}
            fn main() -> Int {{
                let engine: Int = @intrinsic("script_engine_new");
                let callback = |x: Int| {{ {action} x }};
                @intrinsic("script_engine_register", engine, "callback", callback);
                let failed = @intrinsic("script_engine_eval", engine,
                    "fn main() -> Int {{ @intrinsic(\"script_host_call_int\", \"callback\", 34) }}");
                let accepted = @intrinsic("script_outcome_is_ok", failed);
                @intrinsic("script_outcome_free", failed);
                let failure_kind = @intrinsic("script_engine_last_error_kind", engine);
                @intrinsic("script_engine_register", engine, "callback", identity);
                let recovered = @intrinsic("script_engine_eval", engine,
                    "fn main() -> Int {{ @intrinsic(\"script_host_call_int\", \"callback\", 42) }}");
                let result = @intrinsic("script_outcome_as_int", recovered);
                @intrinsic("script_outcome_free", recovered);
                @intrinsic("script_engine_free", engine);
                if accepted {{ -1 }} else {{ if failure_kind == 2 {{ result }} else {{ -2 }} }}
            }}"#);
        assert_eq!(run_source(&source).as_i64(), 42, "{action}");
    }
}

#[test]
fn engine_ids_refuse_foreign_stale_reset_and_wrong_kind_uses() {
    let module = Arc::new(compile(r#"
        fn create() -> Int { @intrinsic("script_engine_new") }
        fn release(engine: Int) { @intrinsic("script_engine_free", engine); }
        fn inspect(engine: Int) -> Int { @intrinsic("script_engine_last_error_kind", engine) }
        fn wrong_kind(engine: Int) -> Int { @intrinsic("script_outcome_kind", engine) }
    "#).unwrap());
    let entry = |name| module.find_function_by_unique_bare_suffix(name).unwrap();
    let mut first = Interpreter::new(module.clone());
    let mut second = Interpreter::new(module.clone());
    let id = first.execute_function(entry("create")).unwrap();
    assert!(id.is_inline_int());
    assert!(second.execute_function_with_args(entry("inspect"), &[id]).is_err());
    assert!(first.execute_function_with_args(entry("wrong_kind"), &[id]).is_err());
    first.execute_function_with_args(entry("release"), &[id]).unwrap();
    assert!(first.execute_function_with_args(entry("release"), &[id]).is_err());
    assert!(first.execute_function_with_args(entry("inspect"), &[id]).is_err());
    let next = first.execute_function(entry("create")).unwrap();
    assert!(next.as_i64() > id.as_i64());
    first.state.reset();
    assert!(first.execute_function_with_args(entry("inspect"), &[next]).is_err());
    let after_reset = first.execute_function(entry("create")).unwrap();
    assert!(after_reset.as_i64() > next.as_i64());
    first.execute_function_with_args(entry("release"), &[after_reset]).unwrap();
}
