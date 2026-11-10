//! AOT must not invent successful bodies for unsupported intrinsic declarations.
//! Both producer VBC and independently decoded wire use the ordinary lowerer.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{context::Context, values::AnyValue};
use verum_vbc::{codegen::VbcCodegen, module::VbcModule};

fn routes(source: &str) -> [VbcModule; 2] {
    let source = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().expect("source grammar"))
        .expect("source VBC");
    source_and_wire(source)
}

fn source_and_wire(source: VbcModule) -> [VbcModule; 2] {
    let bytes = verum_vbc::serialize::serialize_module(&source).expect("source serialization");
    let mut wire = verum_vbc::deserialize::deserialize_module(&bytes).expect("wire decode");
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end])
            .expect("independent body decode");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    [source, wire]
}

fn assert_refused(source: &str, declaration: &str, intrinsic: &str) {
    for module in routes(source) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("intrinsic-refusal").with_debug_info(false),
        );
        let result = lower.lower_module(&module);
        let error =
            result.expect_err("an unavailable intrinsic must not compile to a successful default");
        // Match the exact public diagnostic as well as the selected declaration,
        // so a parser failure or unrelated lowering defect cannot pass this test.
        assert_eq!(
            error.to_string(),
            format!(
                "Intrinsic `{intrinsic}` declared by `{declaration}` has no native implementation"
            )
        );
    }
}

#[test]
fn unknown_declaration_refuses_direct_call() {
    assert_refused(
        r#"
        @intrinsic("missing_native_operation") fn unavailable() -> Int;
        fn main() -> Int { unavailable() }
    "#,
        "unavailable",
        "missing_native_operation",
    );
}

#[test]
fn unknown_declaration_refuses_first_class_call() {
    assert_refused(
        r#"
        @intrinsic("missing_native_operation") fn unavailable() -> Int;
        fn main() -> Int { let operation = unavailable; operation() }
    "#,
        "unavailable",
        "missing_native_operation",
    );
}

#[test]
fn unsupported_public_library_declaration_cannot_be_exported_as_zero() {
    assert_refused(
        r#"
        @intrinsic("missing_library_operation") public fn exported() -> Int;
    "#,
        "exported",
        "missing_library_operation",
    );
}

#[test]
fn unreachable_unimplemented_intrinsic_remains_a_declaration() {
    for module in routes(
        r#"
        @intrinsic("missing_dead_operation") fn unavailable() -> Int;
        fn main() -> Int { 17 }
    "#,
    ) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("dead-intrinsic").with_debug_info(false),
        );
        lower
            .lower_module(&module)
            .expect("unreachable declaration");
        let descriptor = module
            .functions
            .iter()
            .find(|f| module.strings.get(f.name) == Some("unavailable"))
            .unwrap();
        let name = format!("unavailable.__intrinsic_decl_{}", descriptor.id.0);
        let declaration = lower
            .module()
            .get_function(&name)
            .expect("intrinsic declaration retained");
        assert_eq!(
            declaration.count_basic_blocks(),
            0,
            "dead intrinsic must not acquire invented executable semantics"
        );
        lower.module().verify().expect("native module");
    }
}

#[test]
fn ordinary_matching_name_keeps_its_source_implementation() {
    for module in routes(
        r#"
        fn exec_with_recovery() -> Int { 37 }
        fn main() -> Int { let operation = exec_with_recovery; operation() }
    "#,
    ) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("ordinary-function").with_debug_info(false),
        );
        lower
            .lower_module(&module)
            .expect("ordinary source implementation");
        let function = lower
            .module()
            .get_function("exec_with_recovery")
            .expect("source function");
        let ir = function.print_to_string().to_string();
        assert!(ir.contains("i64 37"), "source value was replaced: {ir}");
        lower.module().verify().expect("native module");
    }
}

#[test]
fn actual_stdlib_intrinsic_function_value_binds_its_native_emitter() {
    use verum_ast::ItemKind;
    let mut parsed = Parser::new(include_str!("../../../core/intrinsics/runtime/time.vr"))
        .parse_module()
        .expect("actual stdlib source");
    let mut declarations = 0;
    parsed.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Function(function) if function.name.name.as_str() == "num_cpus" => {
            declarations += 1;
            assert!(function.body.is_none());
            true
        }
        _ => false,
    });
    assert_eq!(declarations, 1);
    for entry in [
        "fn main() -> Int { num_cpus() }",
        "fn main() -> Int { let operation = num_cpus; operation() }",
    ] {
        let mut unit = parsed.clone();
        unit.items
            .extend(Parser::new(entry).parse_module().unwrap().items);
        let module = VbcCodegen::new()
            .compile_module(&unit)
            .expect("actual stdlib VBC");
        let main = module
            .functions
            .iter()
            .find(|f| {
                module
                    .strings
                    .get(f.name)
                    .is_some_and(|n| n.ends_with(".main"))
            })
            .unwrap();
        assert!(
            !main
                .instructions
                .as_ref()
                .unwrap()
                .iter()
                .any(|i| matches!(i, verum_vbc::instruction::Instruction::LoadI { .. })),
            "CPU count must be queried at execution, not folded from the compiler host"
        );
        for module in source_and_wire(module) {
            let descriptor = module
                .functions
                .iter()
                .find(|f| {
                    module
                        .strings
                        .get(f.name)
                        .is_some_and(|name| name.ends_with(".num_cpus"))
                })
                .unwrap();
            assert_eq!(
                descriptor
                    .intrinsic_name
                    .and_then(|id| module.strings.get(id)),
                Some("num_cpus")
            );
            let context = Context::create();
            let mut lower = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("actual-runtime-intrinsic").with_debug_info(false),
            );
            lower
                .lower_module(&module)
                .expect("actual intrinsic binding");
            let wrapper_name = format!(
                "{}.__intrinsic_decl_{}",
                module.strings.get(descriptor.name).unwrap(),
                descriptor.id.0
            );
            let wrapper = lower
                .module()
                .get_function(&wrapper_name)
                .expect("selected declaration wrapper");
            let ir = wrapper.print_to_string().to_string();
            assert!(ir.contains("call i64 @verum_num_cpus()"), "{ir}");
            let main_ir = lower
                .module()
                .get_function("verum_main")
                .unwrap()
                .print_to_string()
                .to_string();
            assert!(
                main_ir.contains(&wrapper_name),
                "entry must use selected declaration: {main_ir}"
            );
            assert!(
                lower
                    .module()
                    .get_function("verum_num_cpus")
                    .unwrap()
                    .count_basic_blocks()
                    > 0
            );
            lower
                .module()
                .verify()
                .expect("actual native runtime module");
        }
    }
}

#[test]
fn unrelated_intrinsic_cannot_borrow_runtime_name_authority() {
    assert_refused(
        r#"
        @intrinsic("missing_native_operation") fn verum_num_cpus() -> Int;
        fn main() -> Int { verum_num_cpus() }
    "#,
        "verum_num_cpus",
        "missing_native_operation",
    );
}

#[test]
fn intrinsic_source_alias_selects_registered_native_identity() {
    for module in routes(
        r#"
        @intrinsic("num_cpus") fn processor_count() -> Int;
        fn main() -> Int { let operation = processor_count; operation() }
    "#,
    ) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("native-intrinsic-alias").with_debug_info(false),
        );
        lower
            .lower_module(&module)
            .expect("actual intrinsic identity owns alias");
        lower.module().verify().expect("native alias module");
    }
}

#[test]
fn ordinary_source_body_cannot_authorize_a_native_intrinsic() {
    assert_refused(
        r#"
        fn user_function() -> Int { 37 }
        @intrinsic("user_function") fn impersonator() -> Int;
        fn main() -> Int { user_function() + impersonator() }
    "#,
        "impersonator",
        "user_function",
    );
}

#[test]
fn wrong_declared_abi_cannot_reuse_native_implementation() {
    for module in routes(
        r#"
        @intrinsic("num_cpus") fn processor_count() -> Float;
        fn main() -> Int { let operation = processor_count; let value = operation(); 17 }
    "#,
    ) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("native-intrinsic-abi").with_debug_info(false),
        );
        let error = lower
            .lower_module(&module)
            .expect_err("native ABI mismatch must refuse");
        assert!(
            matches!(
                error,
                verum_codegen::llvm::LlvmLoweringError::TypeLowering(_)
            ),
            "{error}"
        );
        assert!(error.to_string().contains("ABI"), "{error}");
    }
}

#[test]
fn another_registered_intrinsic_cannot_borrow_native_symbol_identity() {
    assert_refused(
        r#"
        @intrinsic("monotonic_nanos") fn verum_num_cpus() -> Int;
        fn main() -> Int { let operation = verum_num_cpus; operation() }
    "#,
        "verum_num_cpus",
        "monotonic_nanos",
    );
}

#[test]
fn an_actual_empty_source_body_is_not_an_intrinsic_declaration_stub() {
    for module in routes(
        r#"
        @intrinsic("not_a_runtime_implementation") fn source_unit() {}
        fn main() { let operation = source_unit; operation(); }
    "#,
    ) {
        let source = module
            .functions
            .iter()
            .find(|f| module.strings.get(f.name) == Some("source_unit"))
            .unwrap();
        assert!(source.has_source_body);
        assert!(!source.requires_intrinsic_implementation());
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("source-unit-authority").with_debug_info(false),
        );
        lower
            .lower_module(&module)
            .expect("actual empty source body owns Unit");
        lower.module().verify().expect("source Unit module");
    }
}

#[test]
fn private_native_symbol_spelling_is_not_an_intrinsic_identity() {
    assert_refused(
        r#"
        @intrinsic("verum_num_cpus") fn private_symbol() -> Int;
        fn main() -> Int { let operation = private_symbol; operation() }
    "#,
        "private_symbol",
        "verum_num_cpus",
    );
}

#[test]
fn linux_signal_intrinsic_has_a_native_body_for_both_supported_targets() {
    let source = r#"
        @intrinsic("linux_signal_watch") fn observe(number: Int, operation: Int) -> Int;
        fn main() -> Int { let watch = observe; watch(10, 1) }
    "#;
    for module in routes(source) {
        for triple in ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"] {
            let context = Context::create();
            let config = LoweringConfig::debug("signal_watch").with_target(triple);
            let mut lower = VbcToLlvmLowering::new(&context, config);
            lower.lower_module(&module).expect("declared first-class signal operation");
            lower.module().verify().expect("valid target IR");
            let body = lower.module().get_function("verum_linux_signal_watch").expect("kernel operation");
            assert_ne!(body.count_basic_blocks(), 0);
        }
    }
}

#[test]
fn signal_observation_requires_process_permission_in_the_vm() {
    use std::sync::Arc;
    use verum_vbc::interpreter::{Interpreter, InterpreterError, permission::{PermissionDecision, PermissionScope}};
    for module in routes(r#"
        @intrinsic("linux_signal_watch") fn observe(number: Int, operation: Int) -> Int;
        fn main() -> Int { observe(10, 1) }
    "#) {
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        vm.state.set_permission_policy(|scope, operation| {
            assert_eq!((scope, operation), (PermissionScope::Process, 1));
            PermissionDecision::Deny
        });
        assert!(matches!(vm.execute_function(entry), Err(InterpreterError::Panic { message })
            if message == "permission denied: Process(1)"));
    }
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
#[test]
fn vm_observes_two_real_linux_signal_deliveries_on_different_threads() {
    use std::sync::Arc;
    use verum_vbc::interpreter::Interpreter;
    unsafe extern "C" { fn raise(signal: i32) -> i32; }
    for module in routes(r#"
        @intrinsic("linux_signal_watch") fn observe(number: Int, operation: Int) -> Int;
        fn install() -> Int { observe(10, 0) }
        fn take() -> Int { observe(10, 1) }
        fn restore() -> Int { observe(10, 2) }
    "#) {
        let module = Arc::new(module);
        let mut vm = Interpreter::new(module.clone());
        let mut call = |name| vm.execute_function(module.find_function_by_name(name).unwrap())
            .unwrap().try_as_i64().unwrap();
        assert_eq!(call("install"), 0);
        assert_eq!(call("install"), -16);
        assert_eq!(std::thread::spawn(|| unsafe { raise(10) }).join().unwrap(), 0);
        assert_eq!(unsafe { raise(10) }, 0);
        assert_eq!(call("take"), 2);
        assert_eq!(call("take"), 0);
        assert_eq!(call("restore"), 0);
        assert_eq!(call("take"), -9);
    }
}

#[test]
fn unavailable_target_cpu_query_cannot_be_implemented_as_one() {
    for module in routes(
        r#"
        @intrinsic("num_cpus") fn processor_count() -> Int;
        fn main() -> Int { let operation = processor_count; operation() }
    "#,
    ) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("unsupported-cpu-query")
                .with_debug_info(false)
                .with_target("x86_64-pc-windows-msvc"),
        );
        let error = lower
            .lower_module(&module)
            .expect_err("unsupported target must not invent a CPU count");
        assert_eq!(
            error.to_string(),
            "Intrinsic `num_cpus` declared by `processor_count` has no native implementation"
        );
    }
}

#[test]
fn opaque_future_poll_has_no_native_pending_stub() {
    for entry in [
        "fn main() -> Int { let pending = operation(7); 17 }",
        "fn main() -> Int { let poll = operation; let pending = poll(7); 17 }",
    ] {
        assert_refused(
            &format!(
                r#"@intrinsic("future_poll_sync") fn operation(future: Int) -> Bool; {entry}"#
            ),
            "operation",
            "future_poll_sync",
        );
    }
}
