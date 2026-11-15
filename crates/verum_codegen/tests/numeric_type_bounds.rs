//! Exact numeric extrema are owned by the resolved type declaration.
use std::sync::Arc;
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, module::VbcModule, value::Value};

fn compile(source: &str) -> Result<VbcModule, Text> {
    let ast = Parser::new(source)
        .parse_module()
        .expect("canonical source grammar");
    VbcCodegen::new()
        .compile_module(&ast)
        .map_err(|error| error.to_string().into())
}

fn evaluate(source: &str) -> Value {
    let module = compile(source).expect("numeric bounds compile");
    let function = module.find_function_by_name("probe").expect("probe");
    Interpreter::new(Arc::new(module))
        .execute_function(function)
        .expect("numeric bounds execute")
}

#[test]
fn signed_wide_extrema_are_not_shifted_in_a_narrow_host_integer() {
    for (property, expected) in [("min", i128::MIN), ("max", i128::MAX)] {
        let result = evaluate(&format!("fn probe() -> Int128 {{ Int128.{property} }}"));
        assert!(result.is_boxed_i128(), "preserve the declared width");
        assert_eq!(result.as_i128_raw() as i128, expected);
        assert!(result.boxed_i128_is_signed());
    }
}

#[test]
fn unsigned_wide_extrema_preserve_width_and_signedness() {
    for (property, expected) in [("min", 0), ("max", u128::MAX)] {
        let result = evaluate(&format!("fn probe() -> UInt128 {{ UInt128.{property} }}"));
        assert!(result.is_boxed_i128(), "even zero belongs to UInt128");
        assert_eq!(result.as_i128_raw(), expected);
        assert!(!result.boxed_i128_is_signed());
    }
}

#[test]
fn aliases_follow_the_resolved_numeric_declaration() {
    let result = evaluate("type Wide is UInt128; fn probe() -> Wide { Wide.max }");
    assert!(result.is_boxed_i128());
    assert_eq!(result.as_i128_raw(), u128::MAX);
    assert!(!result.boxed_i128_is_signed());
}

#[test]
fn nonnumeric_types_never_receive_fabricated_64_bit_bounds() {
    for source in [
        "type Token is { tag: Int }; fn probe() -> Token { Token.max }",
        "fn probe() -> Bool { Bool.max }",
    ] {
        assert!(
            compile(source).is_err(),
            "no declaration-owned numeric bounds: {source}"
        );
    }
}

#[test]
fn narrow_bounds_use_the_same_numeric_domain() {
    for (name, lower, upper) in [
        ("Int8", -128, 127),
        ("Int16", -32768, 32767),
        ("Int32", i32::MIN as i64, i32::MAX as i64),
        ("Int", i64::MIN, i64::MAX),
        ("UInt8", 0, 255),
        ("UInt16", 0, 65535),
        ("UInt32", 0, u32::MAX as i64),
        ("UInt64", 0, -1),
        ("USize", 0, -1),
        ("ISize", i64::MIN, i64::MAX),
    ] {
        for (property, expected) in [("min", lower), ("max", upper)] {
            let value = evaluate(&format!("fn probe() -> {name} {{ {name}.{property} }}"));
            assert_eq!(value.as_integer_compatible(), expected, "{name}.{property}");
            assert!(!value.is_boxed_i128(), "{name} is not 128 bits");
        }
    }
}

#[test]
fn floating_bounds_are_finite_and_follow_alias_identity() {
    for (name, lower, upper) in [
        ("Float32", f64::from(f32::MIN), f64::from(f32::MAX)),
        ("Float64", f64::MIN, f64::MAX),
    ] {
        for (property, expected) in [("min", lower), ("max", upper)] {
            let source =
                format!("type Scalar is {name}; fn probe() -> Scalar {{ Scalar.{property} }}");
            assert_eq!(evaluate(&source).as_f64().to_bits(), expected.to_bits());
        }
    }
}

#[test]
fn full_width_bounds_survive_archive_roundtrip() {
    use verum_vbc::{
        deserialize::deserialize_module, serialize::serialize_module, validate::validate_module,
    };
    for (name, property, expected, signed) in [
        ("Int128", "min", i128::MIN as u128, true),
        ("Int128", "max", i128::MAX as u128, true),
        ("UInt128", "min", 0, false),
        ("UInt128", "max", u128::MAX, false),
    ] {
        let source = format!("fn probe() -> {name} {{ {name}.{property} }}");
        let module = compile(&source).unwrap();
        let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        validate_module(&wire).unwrap();
        let id = wire.find_function_by_name("probe").unwrap();
        let result = Interpreter::new(Arc::new(wire))
            .execute_function(id)
            .unwrap();
        assert!(result.is_boxed_i128());
        assert_eq!(result.as_i128_raw(), expected);
        assert_eq!(result.boxed_i128_is_signed(), signed);
    }
}

fn roundtrip(module: &VbcModule) -> VbcModule {
    let wire = verum_vbc::serialize::serialize_module(module).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    verum_vbc::validate::validate_module(&restored).unwrap();
    restored
}

#[test]
fn generic_bounds_follow_the_actual_witness_in_source_and_wire() {
    for (name, property, expected) in [
        ("Int128", "min", i128::MIN as u128),
        ("UInt128", "max", u128::MAX),
    ] {
        let source = format!(
            "type Scalar is {name}; type Alias is Scalar;\n\
            fn bound<T>() -> T {{ T.{property} }}\n\
            fn probe() -> Alias {{ bound<Alias>() }}"
        );
        let module = compile(&source).unwrap();
        for module in [roundtrip(&module), module] {
            let id = module.find_function_by_name("probe").unwrap();
            let value = Interpreter::new(Arc::new(module))
                .execute_function(id)
                .unwrap();
            assert!(value.is_boxed_i128());
            assert_eq!(value.as_i128_raw(), expected);
        }
    }
}

#[test]
fn missing_or_nonnumeric_generic_witness_refuses_execution() {
    for source in [
        "fn probe<T>() -> T { T.min }",
        "type Token is { tag: Int }; fn bound<T>() -> T { T.max }\n\
            fn probe() -> Token { bound<Token>() }",
    ] {
        let module = compile(source).unwrap();
        for module in [roundtrip(&module), module] {
            let id = module.find_function_by_name("probe").unwrap();
            assert!(
                Interpreter::new(Arc::new(module))
                    .execute_function(id)
                    .is_err()
            );
        }
    }
}

fn native_result(source: &str, argument: Option<u128>) -> u128 {
    native_result_with_width(source, argument, 128)
}

fn native_result_with_width(source: &str, argument: Option<u128>, width: u32) -> u128 {
    use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering, target_settings::TargetSettings};
    use verum_llvm::{
        OptimizationLevel, context::Context, module::Linkage, passes::PassBuilderOptions,
    };
    use verum_vbc::mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal};

    // Consume the public wire representation and ordinary monomorphizer.
    // Hydration decodes instructions; it supplies no type or generic facts.
    let mut module = roundtrip(&compile(source).unwrap());
    let mut graph = InstantiationGraph::new();
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions =
            verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end]).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    for function in &module.functions {
        discover_call_instantiations(
            &module,
            function.instructions.as_ref().unwrap(),
            function.func_id_base,
            &mut graph,
        )
        .unwrap();
    }
    if !graph.is_empty() {
        module = monomorphize_minimal(module, &graph).unwrap().module;
    }
    let settings = TargetSettings::resolve(None, None, None).unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("bounds")
            .with_debug_info(false)
            .with_target(settings.triple.clone()),
    );
    lowering.lower_module(&module).unwrap();
    let native = lowering.into_module();
    native.verify().unwrap();
    let probe = native.get_function("probe").unwrap();
    assert_eq!(
        probe.get_type().get_return_type(),
        Some(context.custom_width_int_type(width).into())
    );
    assert_eq!(probe.count_params(), u32::from(argument.is_some()));
    if argument.is_some() {
        assert_eq!(
            probe.get_nth_param(0).unwrap().get_type(),
            context.i128_type().into()
        );
    }
    // Eliminate only unreachable runtime bodies, preserving the actual call graph.
    for function in native.get_functions() {
        if function.count_basic_blocks() > 0 {
            function.set_linkage(if function == probe {
                Linkage::External
            } else {
                Linkage::Internal
            });
        }
    }
    native
        .run_passes(
            "globaldce",
            &settings.machine().unwrap(),
            PassBuilderOptions::create(),
        )
        .unwrap();
    native.verify().unwrap();
    assert!(
        native
            .get_function("probe")
            .is_some_and(|function| function.count_basic_blocks() > 0)
    );
    let engine = native
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: verified LLVM signature above exactly matches each selected ABI;
    // the engine and reachable generated bodies remain alive throughout the call.
    assert!(width == 64 || width == 128);
    unsafe {
        if width == 64 {
            assert!(argument.is_none());
            engine
                .get_function::<unsafe extern "C" fn() -> u64>("probe")
                .unwrap()
                .call() as u128
        } else if let Some(value) = argument {
            engine
                .get_function::<unsafe extern "C" fn(u128) -> u128>("probe")
                .unwrap()
                .call(value)
        } else {
            engine
                .get_function::<unsafe extern "C" fn() -> u128>("probe")
                .unwrap()
                .call()
        }
    }
}

#[test]
fn native_wide_bounds_return_the_exact_128_bits() {
    for (name, property, expected) in [
        ("Int128", "min", i128::MIN as u128),
        ("Int128", "max", i128::MAX as u128),
        ("UInt128", "min", 0),
        ("UInt128", "max", u128::MAX),
    ] {
        assert_eq!(
            native_result(
                &format!("fn probe() -> {name} {{ {name}.{property} }}"),
                None
            ),
            expected
        );
    }
}

#[test]
fn native_alias_parameters_and_call_results_keep_the_high_bits() {
    for name in ["Int128", "UInt128"] {
        let source = format!(
            "type Scalar is {name}; type Alias is Scalar;\n\
            fn preserve(value: Alias) -> Alias {{ value }}\n\
            fn probe(value: Alias) -> Alias {{ preserve(value) }}"
        );
        for value in [0, u128::MAX, (1_u128 << 100) | 73] {
            assert_eq!(native_result(&source, Some(value)), value, "{name}");
        }
    }
}

#[test]
fn native_generic_bounds_use_real_specialization() {
    for (name, property, expected) in [
        ("Int128", "min", i128::MIN as u128),
        ("UInt128", "max", u128::MAX),
    ] {
        let source = format!(
            "fn bound<T>() -> T {{ T.{property} }}\n\
            fn probe() -> {name} {{ bound<{name}>() }}"
        );
        assert_eq!(native_result(&source, None), expected);
    }
}

#[test]
fn narrow_parameter_ingress_preserves_declared_signedness_through_aliases() {
    for (name, lower, upper) in [
        ("Int8", -128_i64, 127_i64),
        ("Int16", -32768, 32767),
        ("Int32", i32::MIN as i64, i32::MAX as i64),
        ("UInt8", 0, 255),
        ("UInt16", 0, 65535),
        ("UInt32", 0, u32::MAX as i64),
    ] {
        for alias in [false, true] {
            let (declaration, ty): (Text, &str) = if alias {
                (format!("type Scalar is {name};").into(), "Scalar")
            } else {
                (Text::new(), name)
            };
            for (property, expected) in [("min", lower), ("max", upper)] {
                let source = format!(
                    "{declaration}\nfn preserve(value: {ty}) -> {ty} {{ value }}\n\
                    fn probe() -> Int {{ preserve({ty}.{property}) as Int }}"
                );
                assert_eq!(
                    native_result_with_width(&source, None, 64),
                    expected as u64 as u128,
                    "{name}.{property}, alias={alias}"
                );
            }
        }
    }
}
