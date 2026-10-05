//! Ordinary source Float print uses the common formatter and writer (T1581).
use std::process::Command;
use verum_codegen::llvm::{
    LoweringConfig, VbcToLlvmLowering, context::FunctionContext, instruction::lower_instruction,
};
use verum_common::{List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    module::Module,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple},
    values::AnyValue,
};
use verum_vbc::{
    codegen::VbcCodegen,
    instruction::{Instruction, Reg},
};

const SOURCE: &str = r#"
    fn source_f32(value: Float32) { print(value); }
    fn output(value: Float) {
        print("mixed_begin");
        print(9001);
        print(true);
        print(value);
        print(false);
        print(-9002);
        let mut index = 0;
        while index < 2 { print(1.25); index += 1; }
        print("mixed_end");
    }
"#;

fn values() -> [f64; 14] {
    [
        0.0,
        -0.0,
        1.25,
        1.0e-7,
        -1.0e-7,
        1.0e20,
        f64::MAX,
        -f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ]
}

fn spelling(value: f64) -> Text {
    let mut text: Text = format!("{value}").into();
    if value.is_finite() && !text.contains(".") {
        text.push_str(".0");
    }
    text
}

fn with_ir(target: Option<&str>, check: impl FnOnce(&Context, &Module)) {
    let ast = Parser::new(SOURCE).parse_module().expect("source parses");
    let vbc = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let context = Context::create();
    let mut config = LoweringConfig::debug("float_print").with_debug_info(false);
    if let Some(target) = target {
        config = config.with_target(target);
    }
    let mut lower = VbcToLlvmLowering::new(&context, config);
    lower.lower_module(&vbc).expect("source LLVM");
    // The general lowerer uses register allocas. Exercise the other documented
    // DebugPrint entry with an actual SSA FloatValue and no register type mark.
    for (name, ty) in [
        ("ssa_print", context.f64_type()),
        ("ssa_f32_print", context.f32_type()),
    ] {
        let function = lower.module().add_function(
            name,
            context.void_type().fn_type(&[ty.into()], false),
            None,
        );
        let mut ctx = FunctionContext::new(&context, lower.module(), function, name);
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        ctx.set_register(0, function.get_first_param().unwrap());
        assert!(!ctx.is_float_register(0));
        lower_instruction(&mut ctx, &Instruction::DebugPrint { value: Reg(0) }).unwrap();
        ctx.builder().build_return(None).unwrap();
    }
    check(&context, lower.module());
}

fn reachable_ir(module: &Module, roots: &[&str]) -> Text {
    let mut ir = Text::new();
    let full = module.print_to_string();
    for line in full.to_str().unwrap().lines() {
        if line.starts_with("$_fltused =")
            || line.starts_with("target ")
            || line.starts_with("attributes #")
            || (line.starts_with('%') && line.contains(" = type "))
        {
            ir.push_str(line);
            ir.push('\n');
        }
    }
    let mut pending: List<Text> = roots.iter().map(|name| Text::from(*name)).collect();
    // The backend adds this ABI reference after IR optimization; it is not
    // a textual call edge from the retained source functions.
    if module.get_global("_fltused").is_some() {
        pending.push(Text::from("_fltused"));
    }
    let mut seen = Set::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let text = if let Some(function) = module.get_function(&name) {
            function.print_to_string()
        } else if let Some(global) = module.get_global(&name) {
            global.print_to_string()
        } else {
            continue;
        };
        let text = text.to_str().unwrap();
        ir.push_str(text);
        ir.push('\n');
        for tail in text.split('@').skip(1) {
            let name = tail
                .split(|c: char| !c.is_ascii_alphanumeric() && !"_.$".contains(c))
                .next()
                .unwrap();
            if module.get_function(name).is_some() || module.get_global(name).is_some() {
                pending.push(Text::from(name));
            }
        }
    }
    ir
}

#[test]
fn source_and_ssa_float_print_use_common_formatter_and_entry_buffers() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    with_ir(None, |_, module| {
        assert!(
            module.get_function("printf").is_none(),
            "ordinary Float must not declare printf"
        );
        for name in ["output", "source_f32", "ssa_print", "ssa_f32_print"] {
            let function = module.get_function(name).unwrap();
            let body = function.print_to_string();
            assert!(
                body.to_str()
                    .unwrap()
                    .contains("@verum_internal_f64_to_decimal("),
                "{body}"
            );
            assert!(
                body.to_str().unwrap().contains("@verum_internal_puts("),
                "{body}"
            );
            for block in function.get_basic_blocks() {
                for instruction in block.get_instructions() {
                    let text = instruction.print_to_string();
                    if text.to_str().unwrap().contains("print_float_buffer")
                        && text.to_str().unwrap().contains("alloca")
                    {
                        assert_eq!(
                            block,
                            function.get_first_basic_block().unwrap(),
                            "loop must reuse buffer"
                        );
                        assert!(text.to_str().unwrap().contains("[328 x i8]"), "{text}");
                    }
                }
            }
        }
        let f32 = module
            .get_function("ssa_f32_print")
            .unwrap()
            .print_to_string();
        assert!(
            f32.to_str().unwrap().contains("fpext float"),
            "declared f32 value must widen before the f64 kernel: {f32}"
        );
    });
}

#[test]
fn redirected_mixed_source_output_preserves_order_and_float_values() {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "float_print_child", "--nocapture"])
        .env("VERUM_FLOAT_PRINT_CHILD", "1")
        .output()
        .unwrap();
    let stdout = Text::from_utf8(result.stdout).unwrap();
    let stderr = Text::from_utf8(result.stderr).unwrap();
    assert!(result.status.success(), "{stdout}\n{stderr}");
    let mut expected = Text::new();
    for value in values() {
        expected.push_str(&format!(
            "mixed_begin\n9001\ntrue\n{}\nfalse\n-9002\n1.25\n1.25\nmixed_end\n",
            spelling(value)
        ));
        expected.push_str(&format!("{}\n", spelling(value)));
    }
    // f32 SSA carrier normalization is exact widening, not a shortest-f32 claim.
    for value in [
        1.25_f32,
        -0.0,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
        f32::MAX,
    ] {
        expected.push_str(&format!("{0}\n{0}\n", spelling(f64::from(value))));
    }
    assert!(
        stdout.contains(expected.as_str()),
        "expected contiguous source order:\n{expected}\nactual:\n{stdout}\nstderr:{stderr}"
    );
}

#[test]
fn float_print_child() {
    if std::env::var_os("VERUM_FLOAT_PRINT_CHILD").is_none() {
        return;
    }
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    with_ir(None, |context, module| {
        let ir = reachable_ir(
            module,
            &["output", "source_f32", "ssa_print", "ssa_f32_print"],
        );
        let executable = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "float_print",
            ))
            .unwrap();
        executable.verify().unwrap();
        let engine = executable
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        // SAFETY: actual source function and explicit SSA probes have exactly
        // these declared ABIs, and the engine remains live for all invocations.
        unsafe {
            let source = engine
                .get_function::<unsafe extern "C" fn(f64)>("output")
                .unwrap();
            let ssa = engine
                .get_function::<unsafe extern "C" fn(f64)>("ssa_print")
                .unwrap();
            for value in values() {
                source.call(value);
                ssa.call(value);
            }
            let source_f32 = engine
                .get_function::<unsafe extern "C" fn(f32)>("source_f32")
                .unwrap();
            let f32_print = engine
                .get_function::<unsafe extern "C" fn(f32)>("ssa_f32_print")
                .unwrap();
            for value in [
                1.25_f32,
                -0.0,
                f32::MIN_POSITIVE,
                f32::from_bits(1),
                f32::MAX,
            ] {
                source_f32.call(value);
                f32_print.call(value);
            }
        }
    });
}

#[test]
fn source_print_objects_have_no_libc_formatting_dependency_on_six_targets() {
    Target::initialize_all(&InitializationConfig::default());
    for triple in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
    ] {
        with_ir(Some(triple), |context, module| {
            let ir = reachable_ir(
                module,
                &["output", "source_f32", "ssa_print", "ssa_f32_print"],
            );
            let module = context
                .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                    ir.as_bytes(),
                    "float_print_object",
                ))
                .unwrap();
            let triple_value = TargetTriple::create(triple);
            let machine = Target::from_triple(&triple_value)
                .unwrap()
                .create_target_machine(
                    &triple_value,
                    "generic",
                    "",
                    OptimizationLevel::Aggressive,
                    RelocMode::PIC,
                    CodeModel::Default,
                )
                .unwrap();
            module.set_triple(&triple_value);
            module.set_data_layout(&machine.get_target_data().get_data_layout());
            // Keep actual source entry points alive under GlobalDCE; merely
            // retaining manual SSA roots would not certify the source path.
            for root in ["output", "source_f32", "ssa_print", "ssa_f32_print"] {
                module
                    .get_function(root)
                    .unwrap()
                    .set_linkage(verum_llvm::module::Linkage::External);
            }
            module
                .run_passes(
                    "default<O2>",
                    &machine,
                    verum_llvm::passes::PassBuilderOptions::create(),
                )
                .unwrap();
            module.verify().unwrap();
            let bytes = machine
                .write_to_memory_buffer(&module, FileType::Object)
                .unwrap();
            if let Some(dir) = std::env::var_os("VERUM_FLOAT_PRINT_OBJECT_EVIDENCE") {
                std::fs::write(
                    std::path::Path::new(&dir).join(format!("print-{triple}.o")),
                    bytes.as_slice(),
                )
                .unwrap();
            }
            let object = bytes.create_object_file().unwrap();
            let names: List<Text> = object
                .get_symbols()
                .filter_map(|s| {
                    s.get_name()
                        .map(|n| Text::from(n.to_string_lossy().as_ref()))
                })
                .collect();
            for name in &names {
                let plain = name.trim_start_matches('_');
                assert_ne!(
                    plain.as_str(),
                    "strlen",
                    "optimized scan must remain internal on {triple}"
                );
                if !triple.contains("apple-darwin") {
                    assert_ne!(plain.as_str(), "write", "no POSIX libc write on {triple}");
                }
                assert!(
                    !["printf", "fprintf", "sprintf", "snprintf", "fflush"]
                        .contains(&name.trim_start_matches('_').as_str()),
                    "{triple}: {name}"
                );
            }
            eprintln!("source print object {triple}: {names:?}");
        });
    }
}
