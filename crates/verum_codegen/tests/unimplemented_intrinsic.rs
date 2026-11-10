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

#[test]
fn signal_opcode_refuses_an_unavailable_target() {
    for module in routes(
        r#"@intrinsic("linux_signal_take") fn poll(t: Int) -> Int;
        fn main() -> Int { let read = poll; read(10) }"#,
    ) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("signal").with_target("aarch64-apple-darwin"),
        );
        let err = lower.lower_module(&module).unwrap_err().to_string();
        assert!(
            err.contains("requires Linux x86-64 or AArch64 LP64"),
            "{err}"
        );
    }
}

#[cfg(unix)]
#[path = "support/linux_elf.rs"]
mod linux_elf;

#[cfg(unix)]
#[test]
fn linux_signal_program_links_freestanding_for_both_lanes() {
    for (arch, getpid, kill) in [("aarch64", 172, 129), ("x86_64", 39, 62)] {
        // SIGWINCH is ignored by default. Its post-restore delivery must be
        // harmless. The VM test independently inspects a non-default action.
        let source = format!(
            r#"
            @intrinsic("linux_signal_install") fn acquire(s: Int) -> Int;
            @intrinsic("linux_signal_take") fn drain(t: Int) -> Int;
            @intrinsic("linux_signal_restore") fn release(t: Int) -> Int;
            fn deliver() -> Int {{
                let pid = @intrinsic("syscall0", {getpid});
                @intrinsic("syscall2", {kill}, pid, 28)
            }}
            fn main() -> Int {{
                if acquire(9) != -22 {{ return 1; }}
                let ticket = acquire(28);
                if ticket < 1 {{ return 2; }}
                if acquire(28) != -16 {{ return 3; }}
                if deliver() != 0 || deliver() != 0 {{ return 4; }}
                let take = drain;
                if take(ticket) != 2 || take(ticket) != 0 {{ return 5; }}
                if release(ticket) != 0 {{ return 6; }}
                let next = acquire(28);
                if next <= ticket || release(ticket) != -9 {{ return 7; }}
                if deliver() != 0 || take(next) != 1 {{ return 8; }}
                if release(next) != 0 || deliver() != 0 {{ return 9; }}
                73
            }}
        "#
        );
        let [_, wire] = routes(&source);
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("linux_signals")
                .with_debug_info(false)
                .with_target(format!("{arch}-unknown-linux-gnu")),
        );
        lower
            .lower_module(&wire)
            .expect("wire -> production native lowering");
        linux_elf::check(lower.module(), arch, "verum_main", "watch", 73, b"");

        // Ticket 74 belongs to signal 10. Allow only signal 28: neither a
        // first-class call nor a stale ticket may evade the process policy.
        let [_, denied] = routes(
            r#"@intrinsic("linux_signal_take") fn drain(t: Int) -> Int;
            fn main() -> Int { let op = drain; op(74) }"#,
        );
        let mut policy = verum_codegen::llvm::permissions::AotPermissionPolicy::default();
        policy.specific.insert((
            verum_vbc::interpreter::permission::PermissionScope::Process.to_wire_tag(),
            28,
        ));
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("signal_deny")
                .with_debug_info(false)
                .with_target(format!("{arch}-unknown-linux-gnu"))
                .with_permission_policy(Some(policy)),
        );
        lower.lower_module(&denied).unwrap();
        linux_elf::check(
            lower.module(),
            arch,
            "verum_main",
            "denied",
            143,
            b"permission denied: script lacks `process` grant for this operation\n",
        );
    }
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
