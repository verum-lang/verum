//! T1581: source integer output shares the unbuffered Text/Bool writer.
use std::process::Command;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::codegen::VbcCodegen;

const SOURCE: &str = r#"
    fn narrow(value: UInt8) { print(value); }
    fn output() {
        print("print_begin");
        print(9001);
        print(true);
        print(-9002);
        print(false);
        print(9223372036854775807);
        print(-9223372036854775807 - 1);
        print(0);
        narrow(255);
        let unsigned: UInt64 = 18446744073709551615_u64;
        print(f"{unsigned}");
        let mut index = 0;
        while index < 3 { print(index); index += 1; }
        print("print_end");
    }
"#;

fn with_ir(check: impl FnOnce(&Context, &verum_llvm::module::Module)) {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let ast = Parser::new(SOURCE).parse_module().expect("source parses");
    let vbc = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("integer_print").with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("source LLVM");
    check(&context, lower.module());
}

#[test]
fn integer_source_print_uses_internal_format_and_writer() {
    with_ir(|_, module| {
        let output = module
            .get_function("output")
            .expect("output")
            .print_to_string();
        assert!(!output.to_str().unwrap().contains("@printf("), "{output}");
        assert!(
            module.get_function("printf").is_none(),
            "integer program must not import printf"
        );
        assert!(
            module
                .get_function("verum_internal_i64_to_decimal")
                .is_some()
        );
        assert!(module.get_function("verum_internal_write").is_some());
        // Loop iterations reuse an entry-block buffer rather than growing the stack.
        let function = module.get_function("output").unwrap();
        for block in function.get_basic_blocks() {
            for instruction in block.get_instructions() {
                let text = instruction.print_to_string();
                if text.to_str().unwrap().contains("print_integer_buffer")
                    && text.to_str().unwrap().contains("alloca")
                {
                    assert_eq!(block, function.get_first_basic_block().unwrap());
                }
            }
        }
    });
}

// Retain the real source functions and their actual emitted dependencies, so
// unrelated runtime declarations cannot affect this focused JIT execution.
fn reachable_ir(module: &verum_llvm::module::Module, root: &str) -> Text {
    let mut ir = Text::new();
    let full = module.print_to_string();
    for line in full.to_str().unwrap().lines() {
        if line.starts_with("target ")
            || line.starts_with("attributes #")
            || (line.starts_with('%') && line.contains(" = type "))
        {
            ir.push_str(line);
            ir.push('\n');
        }
    }
    for global in module.get_globals() {
        ir.push_str(global.print_to_string().to_str().unwrap());
        ir.push('\n');
    }
    let mut pending = List::new();
    pending.push(Text::from(root));
    let mut seen = Set::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(function) = module.get_function(&name) else {
            continue;
        };
        let text = function.print_to_string();
        let text = text.to_str().unwrap();
        ir.push_str(text);
        ir.push('\n');
        for tail in text.split('@').skip(1) {
            let name = tail
                .split(|c: char| !c.is_ascii_alphanumeric() && !"_.$".contains(c))
                .next()
                .unwrap();
            if module.get_function(name).is_some() {
                pending.push(Text::from(name));
            }
        }
    }
    ir
}

#[test]
fn redirected_integer_output_preserves_source_order_and_boundaries() {
    let result = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "integer_print_child", "--nocapture"])
        .env("VERUM_INTEGER_PRINT_CHILD", "1")
        .output()
        .expect("run isolated native output");
    let stdout = Text::from_utf8(result.stdout).expect("UTF-8 output");
    let stderr = Text::from_utf8(result.stderr).expect("UTF-8 errors");
    assert!(result.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("print_begin\n9001\ntrue\n-9002\nfalse\n9223372036854775807\n-9223372036854775808\n0\n255\n18446744073709551615\n0\n1\n2\nprint_end\n"), "{stdout}\n{stderr}");
}

#[test]
fn integer_print_child() {
    if std::env::var_os("VERUM_INTEGER_PRINT_CHILD").is_none() {
        return;
    }
    with_ir(|context, module| {
        let ir = reachable_ir(module, "output");
        let executable = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "integer_print",
            ))
            .unwrap_or_else(|error| panic!("{error}\n{ir}"));
        executable.verify().expect("valid focused IR");
        let engine = executable
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        // SAFETY: this is the zero-argument source function's actual native ABI.
        unsafe {
            engine
                .get_function::<unsafe extern "C" fn()>("output")
                .unwrap()
                .call();
        }
    });
}
