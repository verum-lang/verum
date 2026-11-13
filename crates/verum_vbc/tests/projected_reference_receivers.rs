#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(
    source: &str,
    unit_entry: bool,
) -> Result<verum_vbc::VbcModule, verum_vbc::codegen::CodegenError> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("app"));
    if unit_entry {
        compiler.collect_unit_declarations(&[&ast])?;
        compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        compiler.finalize_module_from_state()
    } else {
        compiler.compile_module(&ast)
    }
}

const COUNTER: &str = r#"
    module app;
    type Counter is { value: Int };
    implement Counter {
        fn bump(&mut self) { self.value = self.value + 1; }
    }
"#;

fn run(body: &str) {
    let source = format!("{COUNTER}\n{body}");
    for unit_entry in [false, true] {
        let module = compile(&source, unit_entry).unwrap();
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("app.main").unwrap();
        let result = Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("unit_entry={unit_entry}: {error:?}"));
        assert_eq!(result.try_as_i64(), Some(73), "unit_entry={unit_entry}");
    }
}

#[test]
fn a_mutable_reference_field_passes_its_referent_to_the_method() {
    run(r#"
        type Holder is { target: &mut Counter };
        implement Holder { fn bump(&mut self) { self.target.bump(); } }
        fn main() -> Int {
            let mut value = Counter { value: 72 };
            { let mut holder = Holder { target: &mut value }; holder.bump(); };
            value.value
        }
    "#);
}

#[test]
fn an_owned_field_still_borrows_its_containing_storage() {
    run(r#"
        type Holder is { target: Counter };
        implement Holder { fn bump(&mut self) { self.target.bump(); } }
        fn main() -> Int {
            let mut holder = Holder { target: Counter { value: 72 } };
            holder.bump();
            holder.target.value
        }
    "#);
}

#[test]
fn a_nested_generic_holder_keeps_the_declared_reference() {
    run(r#"
        type Holder<T> is { target: &mut T };
        type Outer is { holder: Holder<Counter> };
        fn main() -> Int {
            let mut value = Counter { value: 71 };
            {
                let mut outer = Outer { holder: Holder { target: &mut value } };
                outer.holder.target.bump();
                (outer.holder.target).bump();
            };
            value.value
        }
    "#);
}

#[test]
fn a_stored_reference_preserves_its_access_capability() {
    for body in [
        "type Holder is { target: &Counter }; fn probe(holder: &mut Holder) { holder.target.bump(); }",
        "type Holder is { target: &mut Counter }; fn probe(holder: &Holder) { holder.target.bump(); }",
    ] {
        let source = format!("{COUNTER}\n{body}");
        for unit_entry in [false, true] {
            let error = compile(&source, unit_entry)
                .err()
                .expect("immutable receiver must be refused")
                .to_string();
            assert!(
                error.contains("mutable"),
                "unit_entry={unit_entry}: {error}"
            );
        }
    }
}
