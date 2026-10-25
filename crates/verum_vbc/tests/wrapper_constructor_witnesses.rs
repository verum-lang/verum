//! Wrapper constructors retain generic witnesses without bypassing CallM.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn wrapper_static_initializer_and_generic_forwarder_receive_witnesses() {
    for wrapper in ["Shared", "Heap"] {
        let source = format!(
            r#"
module wrapper_witness;
type Payload is {{ value: Int }};
type {wrapper}<T> is {{ value: T }};
implement<T> {wrapper}<T> {{
    fn new(value: T) -> {wrapper}<T> {{ {wrapper} {{ value: value }} }}
}}
static ROOT: {wrapper}<Payload> = {wrapper}.new(Payload {{ value: 42 }});
fn forward<T>(value: T) -> {wrapper}<T> {{ {wrapper}.new(value) }}
fn probe() -> Int {{
    let local = forward<Payload>(Payload {{ value: 17 }});
    42
}}
"#
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        for bootstrap in [false, true] {
            let mut codegen = VbcCodegen::with_config(CodegenConfig::new("wrapper_witness"));
            let module = if bootstrap {
                codegen.collect_unit_declarations(&[&ast]).unwrap();
                codegen
                    .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                    .unwrap();
                codegen.finalize_module_from_state().unwrap()
            } else {
                codegen.compile_module(&ast).unwrap()
            };
            let module = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap();
            let probe = module
                .find_function_by_name("wrapper_witness.probe")
                .unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            interpreter.run_global_ctors().unwrap();
            assert_eq!(
                interpreter.execute_function(probe).unwrap().as_i64(),
                42,
                "{wrapper}, bootstrap={bootstrap}"
            );
        }
    }
}
