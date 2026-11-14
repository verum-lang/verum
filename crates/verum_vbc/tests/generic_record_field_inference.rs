//! A record's own field declarations bind its complete generic application.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn compile(source: &str, unit: bool) -> Result<VbcModule, CodegenError> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("record_fields").with_validation());
    if unit {
        compiler.collect_unit_declarations(&[&ast])?;
        compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        compiler.finalize_module_from_state()
    } else {
        compiler.compile_module(&ast)
    }
}

fn run(source: &str) {
    for unit in [false, true] {
        let source_module = compile(source, unit).expect("complete selected record application");
        let wire = deserialize_module(&serialize_module(&source_module).unwrap()).unwrap();
        for (route, module) in [("source", source_module), ("wire", wire)] {
            let entry = module.find_function_by_name("record_fields.probe").unwrap();
            let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
            vm.run_global_ctors().unwrap();
            assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73), "unit={unit}, route={route}");
        }
    }
}

fn refuse(source: &str) {
    for unit in [false, true] {
        let error = match compile(source, unit) {
            Err(error) => error,
            Ok(_) => panic!("accepted inconsistent owner evidence: unit={unit}, source={source}"),
        };
        assert!(error.to_string().contains("complete declared owner application"), "unit={unit}: {error}");
    }
}

fn generic_inner(field: &str, value: &str) -> verum_common::Text {
    format!(r#"
        type Inner<T> is {{ fixed: {field}, value: T }};
        type Outer<T> is {{ inner: Inner<T> }};
        fn build<T>(value: T) -> Outer<T> {{
            let inner = Inner {{ fixed: {value}, value }};
            Outer {{ inner }}
        }}
        fn probe() -> Int {{ build(73).inner.value }}
    "#).into()
}

#[test]
fn sized_fields_do_not_hide_the_owners_generic_evidence() {
    run(r#"
        type Inner<T> is { strong_count: UInt32, weak_count: UInt32, value: T };
        type Outer<T> is { inner: Inner<T> };
        implement<T> Outer<T> {
            fn new(value: T) -> Outer<T> {
                let inner = Inner { strong_count: 1, weak_count: 0, value: value };
                Outer { inner: inner }
            }
        }
        fn probe() -> Int { Outer.new(73).inner.value }
    "#);
}

#[test]
fn integer_literal_boundaries_use_the_declared_field_carrier() {
    for (field, value) in [
        ("Byte", "255"), ("Int8", "-128"), ("Int8", "127"),
        ("Int8", "-(128)"), ("Int8", "(-128)"),
        ("UInt32", "4294967295"), ("UInt64", "18446744073709551615"),
        ("Int64", "-9223372036854775808"), ("Int64", "9223372036854775807"),
        ("Float32", "1.5"), ("Float32", "1e40"),
        ("UInt32", "1u32"), ("Int8", "127i8"), ("Float32", "1.5f32"),
    ] {
        run(&generic_inner(field, value));
    }
}

#[test]
fn out_of_range_or_explicitly_different_literals_cannot_bind_the_owner() {
    for (field, value) in [
        ("Byte", "256"), ("Byte", "-1"), ("Int8", "128"), ("Int8", "-129"),
        ("UInt32", "4294967296"), ("UInt32", "-1"), ("UInt32", "1i64"),
        ("Int8", "1u8"), ("Float32", "1.5f64"), ("UInt32", "true"),
        ("UInt64", "18446744073709551616"), ("UInt64", "-1"),
        ("Int64", "9223372036854775808"), ("Int64", "-9223372036854775809"),
    ] {
        refuse(&generic_inner(field, value));
    }
}

#[test]
fn caller_parameter_order_does_not_replace_the_record_telescope() {
    run(r#"
        type Inner<Right, Left> is { fixed: UInt16, left: Left, right: Right };
        type Outer<A, B> is { inner: Inner<B, A> };
        fn build<A, B>(a: A, b: B) -> Outer<A, B> {
            let inner = Inner { right: b, fixed: 1, left: a };
            Outer { inner }
        }
        fn probe() -> Int { build(73, true).inner.left }
    "#);
}

#[test]
fn conflicting_fields_and_unconstrained_parameters_remain_refused() {
    refuse(r#"
        type Inner<T> is { fixed: UInt32, left: T, right: T };
        fn probe() -> Int { let value = Inner { fixed: 1, left: 73, right: true }; 73 }
    "#);
    refuse(r#"
        type Inner<T, Unused> is { fixed: UInt32, value: T };
        fn probe() -> Int { let value = Inner { fixed: 1, value: 73 }; 73 }
    "#);
}

#[test]
fn a_typed_place_or_foreign_namesake_is_not_a_contextual_literal() {
    refuse(r#"
        type Inner<T> is { fixed: UInt32, value: T };
        fn probe() -> Int { let fixed: Int = 1; let value = Inner { fixed, value: 73 }; 73 }
    "#);
    refuse(r#"
        module foreign { public type UInt32 is { public value: Int }; }
        type Inner<T> is { fixed: foreign.UInt32, value: T };
        fn probe() -> Int { let value = Inner { fixed: 1, value: 73 }; 73 }
    "#);
}


#[test]
fn contextual_aliases_preserve_the_declared_refinement() {
    let source = |value: i32| format!(r#"
        type Count is UInt32 {{ it > 0 }};
        type Inner<T> is {{ count: Count, value: T }};
        fn build<T>(value: T) -> Inner<T> {{
            let inner = Inner {{ count: {value}, value }};
            inner
        }}
        fn probe() -> Int {{ build(73).value }}
    "#);
    run(&source(1));
    for unit in [false, true] {
        let original = compile(&source(0), unit).expect("carrier inference keeps predicate authority");
        let wire = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
        for module in [original, wire] {
            let entry = module.find_function_by_name("record_fields.probe").unwrap();
            let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
            vm.run_global_ctors().unwrap();
            let error = vm.execute_function(entry).expect_err("zero violates Count's source predicate");
            assert!(matches!(error, verum_vbc::interpreter::InterpreterError::AssertionFailed { .. }), "{error:?}");
        }
    }
}
