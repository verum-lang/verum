//! Source/interpreter/native default Float formatting contract (T1582).
use std::{ffi::CStr, sync::Arc};
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
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, value::Value};
const SOURCE: &str = r#"fn convert(value: Float) -> Text { f"{value}" }"#;
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
    let mut pending = List::new();
    pending.push(Text::from(root));
    if root == "convert" {
        pending.push(Text::from("verum_text_get_ptr"));
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
fn source_float_conversion_matches_interpreter_contract() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let ast = Parser::new(SOURCE).parse_module().unwrap();
    let vbc = VbcCodegen::new().compile_module(&ast).unwrap();
    let id = vbc.find_function_by_name("convert").unwrap();
    let mut interpreter = Interpreter::new(Arc::new(vbc.clone()));
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("float_contract").with_debug_info(false),
    );
    lowering.lower_module(&vbc).unwrap();
    let body = lowering
        .module()
        .get_function("convert")
        .unwrap()
        .print_to_string();
    assert!(
        body.to_str().unwrap().contains("@verum_float_to_text("),
        "{body}"
    );
    let ir = reachable_ir(lowering.module(), "convert");
    let module = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "float_contract",
        ))
        .unwrap();
    module.verify().unwrap();
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    let mut mismatches = 0;
    let mut roundtrip_failures = 0;
    // SAFETY: `convert` has the source-declared f64 → Text pointer ABI.
    // `verum_float_to_text` constructs the native {data, byte_len, capacity}
    // Text object; both its length field and NUL-terminated data are checked.
    unsafe {
        let native = engine
            .get_function::<unsafe extern "C" fn(f64) -> u64>("convert")
            .unwrap();
        let bytes = engine
            .get_function::<unsafe extern "C" fn(u64) -> *const i8>("verum_text_get_ptr")
            .unwrap();
        for value in [
            0.0,
            -0.0,
            1.25,
            1.0e-7,
            -1.0e-7,
            1.0e20,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ] {
            let interpreted = interpreter
                .execute_function_with_args(id, &[Value::from_f64(value)])
                .unwrap();
            let expected = interpreter
                .read_text(interpreted)
                .expect("interpreter Text");
            let text = native.call(value);
            assert_eq!(
                (text as *const u64).add(1).read() as usize,
                expected.len(),
                "native Text byte length"
            );
            let actual = CStr::from_ptr(bytes.call(text)).to_str().unwrap();
            let same = expected == actual;
            mismatches += usize::from(!same);
            let roundtrip = if value.is_nan() {
                actual.parse::<f64>().is_ok_and(f64::is_nan)
            } else {
                actual
                    .parse::<f64>()
                    .is_ok_and(|back| back.to_bits() == value.to_bits())
            };
            roundtrip_failures += usize::from(!roundtrip);
            eprintln!(
                "bits={:016x} input={value:?} interp={expected:?} native={actual:?} exact={same} roundtrip={roundtrip}",
                value.to_bits()
            );
        }
    }
    eprintln!("lexical_mismatches={mismatches}, roundtrip_failures={roundtrip_failures}");
    assert_eq!((mismatches, roundtrip_failures), (0, 0));
}

#[test]
fn emitted_integer_kernel_roundtrips_corpus_and_respects_fixed_buffer() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let ast = Parser::new(SOURCE).parse_module().unwrap();
    let vbc = VbcCodegen::new().compile_module(&ast).unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("float_corpus").with_debug_info(false),
    );
    lowering.lower_module(&vbc).unwrap();
    let ir = reachable_ir(lowering.module(), "verum_internal_f64_to_decimal");
    assert!(!ir.contains("fptosi") && !ir.contains("fptoui"));
    for libc in [
        "printf", "snprintf", "sprintf", "malloc", "memcpy", "memset",
    ] {
        assert!(
            !ir.contains(&format!("@{libc}(")),
            "numeric kernel references {libc}"
        );
    }
    let module = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "float_corpus",
        ))
        .unwrap();
    module.verify().unwrap();
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .unwrap();
    // SAFETY: this is the emitted helper's declared ptr/i64/i1 → i64 ABI.
    // Every invocation provides328 writable bytes plus surrounding canaries.
    let format = unsafe {
        engine
            .get_function::<unsafe extern "C" fn(*mut u8, u64, bool) -> u64>(
                "verum_internal_f64_to_decimal",
            )
            .unwrap()
    };
    let mut checks = 0_u64;
    let mut maximum = 0;
    let mut check = |bits: u64| {
        let value = f64::from_bits(bits);
        for debug in [false, true] {
            let mut bytes = [0xa5_u8; 16 + 328 + 16];
            // SAFETY: the live JIT helper receives its exact integer-bit ABI
            // and a 328-byte writable destination inside the canary allocation.
            let len = unsafe { format.call(bytes.as_mut_ptr().add(16), bits, debug) } as usize;
            assert!(len < 328, "bits={bits:016x} length={len}");
            assert!(bytes[..16].iter().all(|byte| *byte == 0xa5));
            assert!(
                bytes[16 + len..].iter().all(|byte| *byte == 0xa5),
                "write beyond returned extent for {bits:016x}"
            );
            let actual = std::str::from_utf8(&bytes[16..16 + len]).unwrap();
            let mut expected: Text = format!("{value}").into();
            if debug && value.is_finite() && !expected.contains(".") {
                expected.push_str(".0");
            }
            assert_eq!(actual, expected.as_str(), "bits={bits:016x}, debug={debug}");
            let parsed = actual.parse::<f64>().unwrap();
            if value.is_nan() {
                assert!(parsed.is_nan());
            } else {
                assert_eq!(parsed.to_bits(), bits, "roundtrip bits={bits:016x}");
            }
            maximum = maximum.max(len);
            checks += 1;
        }
    };
    let mask = (1_u64 << 52) - 1;
    for exponent in 0..=2047 {
        for mantissa in [0, 1, 2, mask / 2, mask / 2 + 1, mask - 1, mask] {
            for sign in [0, 1_u64 << 63] {
                check(sign | exponent << 52 | mantissa);
            }
        }
    }
    for exponent in -323..=308 {
        let power = format!("1e{exponent}").parse::<f64>().unwrap().to_bits();
        for bits in [power - 1, power, power + 1] {
            check(bits);
            check(bits | 1 << 63);
        }
    }
    let mut state = 0xdce6_e599_16e3_5417_u64;
    for _ in 0..100_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        check(state);
    }
    assert_eq!(checks, 264_928);
    assert_eq!(maximum, 327);
    eprintln!(
        "actual emitted formatter: checks={checks}, max_bytes={maximum}, lexical/roundtrip/canary failures=0"
    );
}

#[test]
fn numeric_objects_do_not_import_formatting_memory_or_wide_integer_helpers() {
    use verum_llvm::targets::{CodeModel, FileType, RelocMode, TargetTriple};
    Target::initialize_all(&InitializationConfig::default());
    let ast = Parser::new(SOURCE).parse_module().unwrap();
    let vbc = VbcCodegen::new().compile_module(&ast).unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("float_objects").with_debug_info(false),
    );
    lowering.lower_module(&vbc).unwrap();
    let ir = reachable_ir(lowering.module(), "verum_internal_f64_to_decimal");
    for triple in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
    ] {
        let target_triple = TargetTriple::create(triple);
        let machine = Target::from_triple(&target_triple)
            .unwrap()
            .create_target_machine(
                &target_triple,
                "generic",
                "",
                OptimizationLevel::Aggressive,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .unwrap();
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "float_object",
            ))
            .unwrap();
        module.set_triple(&target_triple);
        module.set_data_layout(&machine.get_target_data().get_data_layout());
        module
            .get_function("verum_internal_f64_to_decimal")
            .unwrap()
            .set_linkage(verum_llvm::module::Linkage::External);
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
        if let Some(dir) = std::env::var_os("VERUM_FLOAT_OBJECT_EVIDENCE") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("float-{triple}.o")),
                bytes.as_slice(),
            )
            .unwrap();
        }
        let object = bytes.create_object_file().unwrap();
        let names: List<Text> = object
            .get_symbols()
            .filter_map(|symbol| {
                symbol
                    .get_name()
                    .map(|name| Text::from(name.to_string_lossy().as_ref()))
            })
            .collect();
        for name in &names {
            let name = name.trim_start_matches('_');
            assert!(
                ![
                    "printf", "fprintf", "sprintf", "snprintf", "malloc", "free", "memcpy",
                    "memmove", "memset", "bzero", "multi3", "udivti3", "divti3", "umodti3",
                    "modti3", "ashlti3", "lshrti3"
                ]
                .contains(&name.as_str()),
                "unexpected generated numeric dependency {name} on {triple}"
            );
        }
        eprintln!("numeric object {triple}: {names:?}");
    }
}
