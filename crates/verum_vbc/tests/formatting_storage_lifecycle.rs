//! Formatting observes named places and retires operation-owned temporaries.
//! Execute both source producers and the serialized artifact on the real heap.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn exercise(source: &str) {
    let ast = Parser::new(source).parse_module().expect("fixture grammar");
    for unit in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("format_lifetime"));
        let module = if unit {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).unwrap()
        };
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        ).unwrap();
        for (route, module) in [("source", module), ("wire", wire)] {
            verum_vbc::validate::validate_module(&module).unwrap();
            let probe = module.find_function_by_name("format_lifetime.probe").unwrap();
            let mut interpreter = Interpreter::new(module.into());
            assert_eq!(interpreter.execute_function(probe).unwrap().try_as_i64(), Some(73));
            // Pooled constants belong to the module. Once loaded, repeated
            // execution must leave exactly the same live allocations.
            let retained = interpreter.state.heap.object_count();
            for _ in 0..3 {
                let value = interpreter.execute_function(probe)
                    .unwrap_or_else(|error| panic!("{route}, unit={unit}: {error}"));
                assert_eq!(value.try_as_i64(), Some(73), "{route}, unit={unit}");
                assert_eq!(interpreter.state.heap.object_count(), retained,
                    "formatting leaked heap objects: {route}, unit={unit}");
            }
        }
    }
}

#[test]
fn repeated_formatting_retires_conversions_and_keeps_named_fields() {
    exercise(r#"
        type Packet is { text: Text };
        fn probe() -> Int {
            let original = f"{123456789}";
            let packet = Packet { text: original };
            let mut step = 0;
            let mut total = 0;
            while step < 128 {
                let line = f"{packet.text}:{123456789}";
                total = total + line.len();
                step = step + 1;
            }
            if total == 2432 && original.len() == 9 && packet.text.len() == 9 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn generic_and_nested_text_conversions_do_not_consume_the_input() {
    exercise(r#"
        fn render<T>(value: T) -> Text { f"{value}" }
        fn probe() -> Int {
            let original = f"{123456789}";
            let converted = render(original);
            let nested = f"prefix {f"{original}"} suffix";
            if original.len() == 9 && converted.len() == 9 && nested.len() == 23 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn printing_a_formatted_temporary_preserves_named_text() {
    exercise(r#"
        fn probe() -> Int {
            let original = f"{123456789}";
            print(f"{original}");
            if original.len() == 9 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn an_inline_interpolation_supplies_its_text_type_to_a_generic_method() {
    exercise(r#"
        type Holder<T> is { sample: T };
        implement<T> Holder<T> {
            fn accept(&self, value: T) -> T { value }
        }
        fn probe() -> Int {
            let holder: Holder<Text> = Holder { sample: "original" };
            let received = holder.accept(f"answer {73}");
            if received == "answer 73" { 73 } else { 0 }
        }
    "#);

    let source = r#"
        type Holder<T> is { sample: T };
        implement<T> Holder<T> { fn accept(&self, value: T) -> T { value } }
        fn reject(holder: Holder<Int>) -> Int { holder.accept(f"answer {73}") }
    "#;
    let ast = Parser::new(source).parse_module().expect("fixture grammar");
    let error = VbcCodegen::with_config(CodegenConfig::new("format_lifetime"))
        .compile_module(&ast).expect_err("formatted Text cannot satisfy the receiver's Int slot");
    assert!(error.to_string().contains("conflicting source argument evidence"), "{error}");
}

#[test]
fn binary_concat_retires_temporary_operands_and_observes_shared_places() {
    exercise(r#"
        type Packet is { text: Text };
        fn probe() -> Int {
            let original = f"{123456789}";
            let packet = Packet { text: original };
            let doubled = original + original;
            let fields = packet.text + packet.text;
            let temporaries = f"{123456789}" + f"{987654321}";
            if original.len() == 9 && packet.text.len() == 9
                && doubled.len() == 18 && fields.len() == 18 && temporaries.len() == 18 {
                73
            } else { 0 }
        }
    "#);
}
