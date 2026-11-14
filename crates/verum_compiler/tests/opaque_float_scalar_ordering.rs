//! Original scalar controls pass source admission before archive/wire execution.
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{
    VbcModule, archive::read_archive_from_file, bytecode::decode_instructions,
    deserialize::deserialize_module, interpreter::Interpreter, serialize::serialize_module,
};

const VALID: &str = include_str!("fixtures/opaque_float_scalar.vr");
const HISTORICAL_MIXED: &str = include_str!("fixtures/opaque_float_mixed_historical.vr");

fn produce(source: &str) -> Result<VbcModule, Text> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), source).unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        num_threads: 1,
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core();
    match result {
        Ok(result) => {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(session.error_count(), 0, "{}", session.format_diagnostics());
            Ok(read_archive_from_file(&output)
                .unwrap()
                .load_module("core")
                .unwrap())
        }
        Err(error) => {
            assert!(!output.exists(), "rejected source published an archive");
            Err(format!("{error:#}\n{}", session.format_diagnostics()).into())
        }
    }
}

fn run_original(name: &str) {
    let source = produce(VALID).unwrap_or_else(|error| panic!("{error}"));
    let wire = serialize_module(&source).expect("serialize admitted source");
    let module = deserialize_module(&wire).expect("decode wire");
    let qualified = format!("core.{name}");
    let entry = module
        .find_function_by_name(&qualified)
        .expect("exact original control");
    let function = module.get_function(entry).unwrap();
    let start = function.bytecode_offset as usize;
    let instructions =
        decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize])
            .expect("decode original control");
    eprintln!("{qualified} selected instructions: {instructions:?}");
    let mut vm = Interpreter::try_new_validated(module.into()).expect("validated VM admission");
    vm.run_global_ctors().expect("global initialization");
    let result = vm.execute_function(entry).unwrap_or_else(|error| {
        panic!("{qualified}: {error:?}\nselected instructions: {instructions:?}")
    });
    assert!(
        result.is_unit(),
        "{qualified} did not return Unit: {result:?}"
    );
}

macro_rules! original_controls {
    ($($name:ident),* $(,)?) => {
        $(#[test]
        fn $name() { run_original(stringify!($name)); })*
    };
}

original_controls!(
    regression_opaque_float_negation_pinned,
    regression_opaque_float_binary_arithmetic_pinned,
    regression_opaque_float_ordering_pinned,
    regression_opaque_float_loop_bound_pinned,
    regression_opaque_float_nan_is_unordered_pinned,
);

#[test]
fn historical_mixed_operands_require_explicit_conversion() {
    let error = produce(HISTORICAL_MIXED).expect_err("implicit Int/Float widening was admitted");
    eprintln!("historical mixed control: {error}");
    assert!(error.contains("E400"), "{error}");
    assert!(error.contains("expected 'Float', found 'Int'"), "{error}");
    assert!(error.contains("expected 'Int', found 'Float'"), "{error}");
}
