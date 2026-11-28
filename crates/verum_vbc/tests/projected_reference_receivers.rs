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

#[test]
fn positional_method_calls_execute_the_selected_payload_implementation() {
    let source = r#"
        module app;
        type Sink is { state: Int };
        type Hash is protocol { fn hash(&self, sink: &mut Sink); };
        type Token is { value: Int };
        implement Hash for Text {
            fn hash(&self, sink: &mut Sink) {
                if *self == "stop" { panic("visited text"); }
                sink.state = if *self == "first" { 71 } else { 13 };
            }
        }
        implement Hash for Token { fn hash(&self, sink: &mut Sink) { sink.state = self.value; } }
        type Key is (Text);
        implement Hash for Key { fn hash(&self, sink: &mut Sink) { self.0.hash(sink); } }
        type Wrap<T> is (T);
        type Row is (Text, Token);
        fn main() -> Int {
            let mut sink = Sink { state: 0 };
            let a = Key("first"); a.hash(&mut sink);
            let first = sink.state;
            let b = Row("second", Token { value: 29 }); b.0.hash(&mut sink);
            let second = sink.state; b.1.hash(&mut sink);
            let third = sink.state; let last: Wrap<Token> = Wrap(Token { value: 37 }); last.0.hash(&mut sink);
            first * 1000000 + second * 10000 + third * 100 + sink.state
        }
        fn fail() {
            let mut sink = Sink { state: 0 };
            let a = Key("stop"); a.hash(&mut sink);
        }
    "#;
    for unit_entry in [false, true] {
        let module = compile(source, unit_entry).unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
        verum_vbc::validate::validate_module(&module).unwrap();
        let main = module.find_function_by_name("app.main").unwrap();
        let fail = module.find_function_by_name("app.fail").unwrap();
        let mut vm = Interpreter::new(module.into());
        assert_eq!(
            vm.execute_function(main).unwrap().try_as_i64(),
            Some(71132937)
        );
        assert!(
            matches!(vm.execute_function(fail), Err(verum_vbc::interpreter::InterpreterError::Panic { message }) if message == "visited text")
        );
    }
}
