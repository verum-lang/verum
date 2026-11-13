#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn run(source: &str) {
    let ast = Parser::new(source).parse_module().unwrap();
    for unit_entry in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("app"));
        let module = if unit_entry {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).unwrap()
        };
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("app.main").unwrap();
        let result = Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("unit_entry={unit_entry}: {error:?}"));
        assert_eq!(result.try_as_i64(), Some(73), "unit_entry={unit_entry}");
    }
}

#[test]
fn a_refined_text_observes_its_declared_base_borrow() {
    run(r#"
        module app;
        type Observe is protocol { fn observe(&self) -> Int; };
        type Label is Text { true };
        implement Observe for Text {
            fn observe(&self) -> Int { 72 }
        }
        implement Observe for Label {
            fn observe(&self) -> Int { 13 }
        }
        fn widen(value: &Label) -> Int {
            let carrier: &Text = value;
            carrier.observe() + 1
        }
        fn main() -> Int {
            let value: Label = "id";
            widen(&value)
        }
    "#);
}

#[test]
fn a_refined_integer_observes_its_declared_base_borrow() {
    run(r#"
        module app;
        type Observe is protocol { fn observe(&self) -> Int; };
        type Positive is Int { self > 0 };
        implement Observe for Int {
            fn observe(&self) -> Int { *self }
        }
        implement Observe for Positive {
            fn observe(&self) -> Int { 13 }
        }
        fn widen(value: &Positive) -> Int {
            let carrier: &Int = value;
            carrier.observe() + 1
        }
        fn main() -> Int {
            let value: Positive = 72;
            widen(&value)
        }
    "#);
}
