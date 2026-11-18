//! T1536: explicit Panic uses the configured common runtime policy.
use verum_codegen::llvm::context::FunctionContext;
use verum_codegen::llvm::instruction::lower_instruction;
use verum_codegen::llvm::vbc_lowering::{LoweringConfig, PanicStrategy, VbcToLlvmLowering};
use verum_llvm::attributes::{Attribute, AttributeLoc};
use verum_llvm::context::Context;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_llvm::values::AnyValue;
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::module::{FunctionDescriptor, VbcModule};

fn assert_try_begin_returns_twice(triple: &str, callee_name: &str, predeclare: bool) {
    let context = Context::create();
    let mut vbc = VbcModule::new("try_begin_contract".to_owned());
    let mut function = FunctionDescriptor::new(vbc.intern_string("main"));
    function.register_count = 2;
    function.return_type = verum_vbc::types::TypeRef::concrete(verum_vbc::types::TypeId::INT);
    function.instructions = Some(vec![
        Instruction::TryBegin { handler_offset: 4 },
        Instruction::LoadI {
            dst: Reg(0),
            value: 37,
        },
        Instruction::TryEnd,
        Instruction::Ret { value: Reg(0) },
        Instruction::GetException { dst: Reg(1) },
        Instruction::Ret { value: Reg(1) },
    ]);
    vbc.add_function(function);
    let config = LoweringConfig::new("try_begin_contract").with_target(triple);
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    lowering.module().add_function(
        "unmarked_internal",
        context.i32_type().fn_type(&[], false),
        None,
    );
    if predeclare {
        lowering.module().add_function(
            callee_name,
            context
                .i32_type()
                .fn_type(&[context.ptr_type(Default::default()).into()], false),
            None,
        );
    }
    lowering.lower_module(&vbc).expect("lower real TryBegin");
    let module = lowering.module();
    let actual_calls = module.get_functions().any(|function| {
        function
            .print_to_string()
            .to_string()
            .contains(&format!("call i32 @{callee_name}("))
    });
    assert!(
        actual_calls,
        "TryBegin must call {callee_name} for {triple}"
    );
    let callee = module
        .get_function(callee_name)
        .expect("actual TryBegin callee");
    assert_eq!(
        callee.count_basic_blocks(),
        0,
        "{callee_name} must not become a zero-return stub"
    );
    assert_eq!(callee.get_linkage(), verum_llvm::module::Linkage::External);
    let kind = Attribute::get_named_enum_kind_id("returns_twice");
    assert!(
        callee
            .get_enum_attribute(AttributeLoc::Function, kind)
            .is_some(),
        "{triple}: actual callee lacks LLVM returns_twice: {}",
        callee.print_to_string(),
    );
    if callee_name == "_setjmp" {
        let longjmp = module.get_function("longjmp").expect("actual throw callee");
        assert_eq!(
            longjmp.count_basic_blocks(),
            0,
            "longjmp must remain linker-resolved"
        );
        assert_eq!(longjmp.get_linkage(), verum_llvm::module::Linkage::External);
        assert!(
            longjmp
                .get_enum_attribute(
                    AttributeLoc::Function,
                    Attribute::get_named_enum_kind_id("noreturn")
                )
                .is_some()
        );
    } else {
        assert!(module.get_function("_setjmp").is_none());
        assert!(module.get_function("longjmp").is_none());
    }
    assert!(
        module
            .get_function("unmarked_internal")
            .expect("negative control")
            .count_basic_blocks()
            > 0
    );
}

#[test]
fn darwin_try_begin_marks_the_actual_setjmp_callee_returns_twice() {
    for triple in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        assert_try_begin_returns_twice(triple, "_setjmp", false);
        assert_try_begin_returns_twice(triple, "_setjmp", true);
    }
}

#[test]
fn sjlj_try_begin_preserves_the_same_double_return_contract() {
    for triple in ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"] {
        assert_try_begin_returns_twice(triple, "llvm.eh.sjlj.setjmp", false);
    }
}

#[test]
fn explicit_panic_calls_canonical_runtime_abi() {
    let context = Context::create();
    let module = context.create_module("panic_policy");
    let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
    let mut vbc = VbcModule::new("panic_policy".to_owned());
    let message_id = vbc.intern_string("exact panic message");
    let mut ctx = FunctionContext::with_vbc_module(&context, &module, &vbc, function, "probe");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    lower_instruction(
        &mut ctx,
        &Instruction::Panic {
            message_id: message_id.0,
        },
    )
    .expect("lower panic");
    module.verify().expect("valid IR");
    let ir = function.print_to_string().to_string();
    assert!(ir.contains("call void @verum_panic(ptr"), "{ir}");
    assert!(ir.contains("i64 19, ptr null, i32 0"), "{ir}");
    assert!(!ir.contains("exit") && !ir.contains("puts"), "{ir}");
}

#[test]
fn raw_and_decoded_exception_dispositions_use_the_same_native_operations() {
    use verum_vbc::instruction::ExtendedSubOpcode as Ext;

    for (op, operands, canonical, routine) in [
        (
            Ext::DiscardException,
            vec![1],
            Instruction::DiscardException { dst: Reg(1) },
            "verum_exception_discard",
        ),
        (
            Ext::RethrowException,
            vec![],
            Instruction::RethrowException,
            "verum_exception_rethrow",
        ),
    ] {
        for instr in [
            canonical,
            Instruction::Extended {
                sub_op: op.to_byte(),
                operands: operands.clone(),
            },
        ] {
            let context = Context::create();
            let module = context.create_module("exception_disposition");
            let function =
                module.add_function("probe", context.void_type().fn_type(&[], false), None);
            let vbc = VbcModule::new("exception_disposition".into());
            let mut ctx =
                FunctionContext::with_vbc_module(&context, &module, &vbc, function, "probe");
            let entry = context.append_basic_block(function, "entry");
            ctx.builder().position_at_end(entry);
            lower_instruction(&mut ctx, &instr).unwrap();
            if op == Ext::DiscardException {
                lower_instruction(&mut ctx, &Instruction::LoadUnit { dst: Reg(2) }).unwrap();
                assert_eq!(
                    ctx.get_register(1).unwrap().get_type(),
                    ctx.get_register(2).unwrap().get_type(),
                );
                ctx.builder().build_return(None).unwrap();
            }
            module.verify().unwrap();
            let ir = function.print_to_string().to_string();
            assert!(ir.contains(&format!("call void @{routine}()")), "{ir}");
            assert_eq!(
                ir.contains("unreachable"),
                op == Ext::RethrowException,
                "{ir}"
            );
        }
    }
    let context = Context::create();
    let module = context.create_module("malformed_disposition");
    let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
    let vbc = VbcModule::new("malformed_disposition".into());
    let mut ctx = FunctionContext::with_vbc_module(&context, &module, &vbc, function, "probe");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    for (op, operands) in [
        (Ext::DiscardException, vec![0x80]),
        (Ext::RethrowException, vec![1]),
    ] {
        assert!(
            lower_instruction(
                &mut ctx,
                &Instruction::Extended {
                    sub_op: op.to_byte(),
                    operands
                }
            )
            .is_err()
        );
    }
}

#[test]
fn runtime_unwind_throws_an_owned_message_packet_while_abort_terminates() {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    for policy in [PanicStrategy::Unwind, PanicStrategy::Abort] {
        let context = Context::create();
        let mut config = LoweringConfig::new("runtime_panic_policy");
        config.panic_strategy = policy;
        let mut lowering = VbcToLlvmLowering::new(&context, config);
        lowering
            .lower_module(&VbcModule::new("runtime_panic_policy".to_owned()))
            .expect("ordinary runtime setup");
        let module = lowering.module();
        let panic = module.get_function("verum_panic").expect("canonical panic");
        assert_eq!(panic.count_params(), 4);
        let ir = panic.print_to_string().to_string();
        match policy {
            PanicStrategy::Unwind => {
                assert!(ir.contains("@verum_text_from_stride"), "{ir}");
                assert!(
                    ir.contains("@verum_exception_raise(i64 %panic_packet, i8 2)"),
                    "{ir}"
                );
                assert!(!ir.contains("@_exit("), "{ir}");
                assert!(!ir.contains("@verum_exception_throw(i64 0)"), "{ir}");
                let discard = module.get_function("verum_exception_discard").unwrap().print_to_string().to_string();
                assert!(discard.contains("@verum_text_free") && discard.contains("@verum_os_free"), "{discard}");
                let rethrow = module.get_function("verum_exception_rethrow").unwrap().print_to_string().to_string();
                assert!(rethrow.contains("@verum_exception_raise(i64 %value, i8 %origin)"), "{rethrow}");
            }
            PanicStrategy::Abort => {
                assert!(ir.contains("@_exit(i32 1)"), "{ir}");
                assert!(!ir.contains("@verum_exception_throw"), "{ir}");
                assert!(!ir.contains("@verum_exception_raise"), "{ir}");
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn native_process(
    source: &str,
    strategy: PanicStrategy,
    trace_releases: bool,
) -> std::process::Output {
    use std::process::Command;
    use verum_fast_parser::Parser;
    use verum_llvm::OptimizationLevel;
    use verum_llvm::targets::{CodeModel, FileType, RelocMode, TargetMachine};
    use verum_vbc::{
        codegen::VbcCodegen, deserialize::deserialize_module_validated, serialize::serialize_module,
    };

    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let module = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap();
    let mut wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions =
            verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end]).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    let context = Context::create();
    let mut config = LoweringConfig::debug("panic_value").with_debug_info(false);
    config.panic_strategy = strategy;
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    lowering.lower_module(&wire).expect("native lowering");
    let module = lowering.module();
    if trace_releases {
        // Instrument the actual return after both native deallocators. The same
        // source runs separately without instrumentation.
        let cleanup = module.get_function("verum_exception_discard").unwrap();
        let release = cleanup
            .get_basic_blocks()
            .into_iter()
            .find(|block| block.get_name().to_bytes() == b"release_packet")
            .unwrap();
        let builder = context.create_builder();
        builder.position_before(&release.get_terminator().unwrap());
        let marker = "retired panic packet\n";
        let bytes = builder
            .build_global_string_ptr(marker, "release_marker")
            .unwrap();
        let int = context.i64_type();
        builder
            .build_call(
                module.get_function("write").unwrap(),
                &[
                    int.const_int(2, false).into(),
                    bytes.as_pointer_value().into(),
                    int.const_int(marker.len() as u64, false).into(),
                ],
                "",
            )
            .unwrap();
    }
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple)
        .unwrap()
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::None,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .unwrap();
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    module.verify().expect("verified native IR");
    let temp = tempfile::tempdir().unwrap();
    let object = temp.path().join("panic.o");
    let binary = temp.path().join("panic");
    machine
        .write_to_file(module, FileType::Object, &object)
        .unwrap();
    let linked = Command::new("cc")
        .arg(&object)
        .args(["-Wl,-dead_strip", "-nostdlib", "-lSystem", "-o"])
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    Command::new(binary).output().unwrap()
}

#[cfg(target_os = "macos")]
#[test]
fn caught_and_uncaught_runtime_messages_reach_the_native_process() {
    for caught in [true, false] {
        let body = if caught {
            r#"if check("first") == 1 && check("второе сообщение") == 1 { 86 } else { 42 }"#
        } else {
            r#"fail("uncaught второй"); 0"#
        };
        let source = format!(r#"
            module core.intrinsics.control;
            type Location is {{ line: Int }};
            type IntrinsicPanicInfo is {{ message: Text, location: Maybe<Location> }};
            @intrinsic("catch_unwind")
            fn fence<T>(f:fn()->T)->Result<T,IntrinsicPanicInfo> {{ @intrinsic("catch_unwind", f) }}
            fn fail(message:Text)->Never {{ @intrinsic("panic", message) }}
            fn check(message:Text)->Int {{
                let result = fence(|| {{ fail(message); 0 }});
                match result {{ Result.Err(info) => if info.message == message {{ 1 }} else {{ 2 }}, _ => 3 }}
            }}
            fn main()->Int {{ {body} }}
        "#);
        let actual = native_process(&source, if caught { PanicStrategy::Unwind } else { PanicStrategy::Abort }, false);
        if caught {
            assert_eq!(actual.status.code(), Some(86), "{}", String::from_utf8_lossy(&actual.stderr));
        } else {
            assert_eq!(actual.status.code(), Some(1));
            assert!(String::from_utf8_lossy(&actual.stderr).contains("uncaught второй"));
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_process_discards_and_forwards_with_the_original_issuer() {
    let source = r#"
        fn main()->Int {
            let ignored = try { panic("discard this owned panic"); 0 } recover |unused| { 20 };
            let propagated = try {
                try { panic("forward this owned panic"); 0 } recover |inner| { throw inner }
            } recover |_| { 36 };
            let raw = try { try { throw 17 } recover |inner| { throw inner } }
                recover |value| { value };
            ignored + propagated + raw
        }
    "#;
    for trace in [false, true] {
        let output = native_process(source, PanicStrategy::Unwind, trace);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(73), "{stderr}");
        assert!(
            stderr.contains("discard this owned panic")
                && stderr.contains("forward this owned panic"),
            "{stderr}"
        );
        assert_eq!(
            stderr.matches("retired panic packet\n").count(),
            if trace { 2 } else { 0 },
            "{stderr}"
        );
    }
}
