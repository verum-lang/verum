//! Default arguments preserve the selected parameter's value representation.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenError, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::{Constant, VbcModule},
    value::Value,
};
fn codegen(owner: &str) -> VbcCodegen {
    VbcCodegen::with_config(
        CodegenConfig::new(owner)
            .with_validation()
            .with_strict_codegen(),
    )
}
fn compile(source: &str) -> VbcModule {
    codegen("fixture")
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}
fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::validate::validate_module(module).unwrap();
    let result = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&result).unwrap();
    result
}
fn check(module: VbcModule, entry: &str, expected: impl Fn(Value)) {
    for module in [module.clone(), wire(&module)] {
        let id = module.find_function_by_name(entry).unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().unwrap();
        expected(vm.execute_function(id).unwrap());
    }
}
fn run(source: &str, expected: impl Fn(Value)) {
    check(compile(source), "fixture.probe", expected);
}
#[test]
fn bool_defaults_preserve_both_tags() {
    for (literal, expected) in [("true", true), ("false", false)] {
        run(
            &format!(
                "fn selected(value:Bool={literal})->Bool{{value}} fn probe()->Bool{{selected()}}"
            ),
            |value| assert!(value.is_bool() && value.as_bool() == expected, "{value:?}"),
        );
    }
}
#[test]
fn explicit_bool_argument_control() {
    run(
        "fn selected(value:Bool=false)->Bool{value} fn probe()->Bool{selected(true)}",
        |v| assert!(v.is_bool() && v.as_bool(), "{v:?}"),
    );
}
#[test]
fn wide_defaults_preserve_payload_signedness_and_small_wide_carriers() {
    for (ty, literal, raw, signed) in [
        ("UInt128", "18446744073709551616u128", 1u128 << 64, false),
        ("UInt128", "5", 5, false),
        ("Int128", "5", 5, true),
        (
            "Int128",
            "-170141183460469231731687303715884105728i128",
            i128::MIN as u128,
            true,
        ),
    ] {
        run(
            &format!(
                "fn selected(value:{ty}={literal})->{ty}{{value}} fn probe()->{ty}{{selected()}}"
            ),
            |v| {
                assert!(
                    v.is_boxed_i128()
                        && v.boxed_i128_is_signed() == signed
                        && v.as_i128_raw() == raw,
                    "{ty}/{literal}: {v:?}"
                )
            },
        );
    }
}
#[test]
fn explicit_wide_argument_control() {
    run(
        "fn selected(value:UInt128)->UInt128{value} fn probe()->UInt128{selected(18446744073709551616u128)}",
        |v| {
            assert!(
                v.is_boxed_i128() && !v.boxed_i128_is_signed() && v.as_i128_raw() == (1u128 << 64),
                "{v:?}"
            )
        },
    );
}
#[test]
fn narrow_integer_defaults_preserve_bounds_and_unsigned_bits() {
    for (ty, literal, bits) in [
        ("Int", "-9223372036854775808", i64::MIN),
        ("Int8", "-128", -128),
        ("UInt8", "255", 255),
        ("UInt64", "18446744073709551615u64", -1),
    ] {
        run(
            &format!(
                "fn selected(value:{ty}={literal})->{ty}{{value}} fn probe()->{ty}{{selected()}}"
            ),
            |v| {
                assert!(
                    v.is_int() && !v.is_boxed_i128() && v.as_i64() == bits,
                    "{ty}: {v:?}"
                )
            },
        );
    }
}
#[test]
fn float_default_control() {
    run(
        "fn selected(value:Float=1.25)->Float{value} fn probe()->Float{selected()}",
        |v| assert!(v.is_float() && v.as_f64() == 1.25, "{v:?}"),
    );
}
#[test]
fn text_default_control() {
    run(
        "fn selected(value:Text=\"carrier\")->Bool{value==\"carrier\"} fn probe()->Bool{selected()}",
        |v| assert!(v.is_bool() && v.as_bool(), "{v:?}"),
    );
}
#[test]
fn invalid_native_literal_defaults_are_refused_before_erasure() {
    for source in [
        "fn selected(value:Bool=2)->Bool{value}",
        "fn selected(value:Bool=-(-true))->Bool{value}",
        "fn selected(value:Text=-(-\"wrong\"))->Text{value}",
        "fn selected(value:Int8=128)->Int8{value}",
        "fn selected(value:UInt8=-1)->UInt8{value}",
        "fn selected(value:Int=true)->Int{value}",
        "fn selected(value:Float=\"wrong\")->Float{value}",
    ] {
        let parsed = Parser::new(source).parse_module().unwrap();
        let error = codegen("fixture")
            .compile_module(&parsed)
            .expect_err(source);
        assert!(error.to_string().contains("constant default"), "{error}");
    }
}
fn imported(
    default_type: &str,
    default: &str,
    mutate: Option<Constant>,
) -> Result<VbcModule, CodegenError> {
    let source = format!(
        "module fixture.provider; public fn selected(value:{default_type}={default})->{default_type}{{value}}"
    );
    imported_modified(default_type, &source, |library| {
        if let Some(value) = mutate {
            let selected = library
                .functions
                .iter()
                .find(|f| library.strings.get(f.name) == Some("fixture.provider.selected"))
                .unwrap();
            library.constants[selected.params[0].default.unwrap().0 as usize] = value;
        }
    })
}
fn imported_modified(
    default_type: &str,
    provider: &str,
    mutate: impl FnOnce(&mut VbcModule),
) -> Result<VbcModule, CodegenError> {
    let provider = Parser::new(provider).parse_module().unwrap();
    let mut producer = codegen("fixture.provider");
    let mut library = producer.compile_module(&provider)?;
    mutate(&mut library);
    let library = wire(&library);
    let caller=Parser::new(&format!("module fixture.consumer; mount fixture.provider.selected; public fn probe()->{default_type}{{selected()}}"))
        .parse_module().unwrap();
    let mut consumer = codegen("fixture.consumer");
    consumer.initialize();
    consumer.import_functions(&producer.export_functions());
    consumer.import_bootstrap_nominal_dependencies(&[&caller], &[&library])?;
    let map = library
        .functions
        .iter()
        .map(|f| {
            let name = library.strings.get(f.name).unwrap();
            (f.id.0, consumer.ctx_mut().lookup_function(name).unwrap().id)
        })
        .collect();
    assert!(consumer.merge_archive_function_bodies(&library, &map) > 0);
    consumer.collect_owned_unit_declarations(&[("fixture.consumer", &caller)])?;
    consumer.resolve_pending_imports();
    consumer
        .compile_owned_unit_items(&[("fixture.consumer", &caller)], ItemFailurePolicy::Strict)?;
    consumer.finalize_module()
}
#[test]
fn bool_default_survives_wire_and_body_import() {
    check(
        imported("Bool", "true", None).unwrap(),
        "fixture.consumer.probe",
        |v| assert!(v.is_bool() && v.as_bool(), "{v:?}"),
    );
}
#[test]
fn wide_default_survives_wire_and_body_import() {
    check(
        imported("UInt128", "18446744073709551616u128", None).unwrap(),
        "fixture.consumer.probe",
        |v| {
            assert!(
                v.is_boxed_i128() && !v.boxed_i128_is_signed() && v.as_i128_raw() == 1u128 << 64,
                "{v:?}"
            )
        },
    );
}
#[test]
fn malformed_bool_default_payload_is_refused_on_import() {
    for payload in [Constant::Int(2), Constant::Float(1.0)] {
        let error =
            imported("Bool", "true", Some(payload)).expect_err("malformed carrier admitted");
        assert!(error.to_string().contains("constant default"), "{error}");
    }
}

#[test]
fn signed_wide_boundaries_and_nested_negation_are_checked() {
    for literal in [
        "170141183460469231731687303715884105728i128",
        "170141183460469231731687303715884105728",
        "-(-170141183460469231731687303715884105728i128)",
        "-(-(-170141183460469231731687303715884105728i128))",
    ] {
        let source = format!("fn selected(value:Int128={literal})->Int128{{value}}");
        let parsed = Parser::new(&source).parse_module().unwrap();
        let error = codegen("fixture")
            .compile_module(&parsed)
            .expect_err(&source);
        assert!(error.to_string().contains("constant default"), "{error}");
    }
    for (literal, expected) in [
        ("170141183460469231731687303715884105727i128", i128::MAX),
        ("-(170141183460469231731687303715884105728i128)", i128::MIN),
        ("-(-42)", 42),
    ] {
        run(
            &format!(
                "fn selected(value:Int128={literal})->Int128{{value}} fn probe()->Int128{{selected()}}"
            ),
            |value| {
                assert!(
                    value.is_boxed_i128()
                        && value.boxed_i128_is_signed()
                        && value.as_i128_raw() == expected as u128,
                    "{literal}: {value:?}"
                )
            },
        );
    }
}
#[test]
fn erased_pointer_carrier_is_not_integer_authority() {
    // These source domains all erase to PTR. Until the descriptor preserves
    // their semantic distinction, serializing a scalar default must refuse.
    for ty in ["*const Int", "USize", "ISize"] {
        let source = format!("fn selected(value:{ty}=0)->{ty}{{value}}");
        let parsed = Parser::new(&source).parse_module().unwrap();
        let error = codegen("fixture")
            .compile_module(&parsed)
            .expect_err(&source);
        assert!(
            error.to_string().contains("constant default"),
            "{ty}: {error}"
        );
    }
}

#[test]
fn forged_type_name_cannot_authorize_erased_pointer_default() {
    for spelling in ["USize", "Int", "UInt64"] {
        let error = imported_modified(
            "USize",
            "module fixture.provider; public fn selected(value:USize)->USize{value}",
            |library| {
                let type_name = library.strings.intern(spelling);
                let constant = verum_vbc::module::ConstId(library.constants.len() as u32);
                library.constants.push(Constant::Int(0));
                library.header.constant_pool_count = library.constants.len() as u32;
                let selected = library
                    .functions
                    .iter_mut()
                    .find(|f| f.params.len() == 1)
                    .unwrap();
                selected.params[0].default = Some(constant);
                selected.params[0].type_name = type_name;
            },
        )
        .expect_err("textual label authorized an erased carrier");
        assert!(
            error.to_string().contains("constant default"),
            "{spelling}: {error}"
        );
    }
}
