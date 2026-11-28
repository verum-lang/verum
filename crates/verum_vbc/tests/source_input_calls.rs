//! Input functions with source bodies use ordinary call dispatch.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn source_input_names_use_their_declared_bodies() {
    let ast = Parser::new(
        "module research.io.stdio;
         fn read_line() -> Int { 1 }
         fn read_int() -> Int { 4 }
         fn read_float() -> Int { 16 }
         fn read_to_end() -> Int { 64 }
         fn probe() -> Int { read_line() + read_int() + read_float() + read_to_end() }",
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let entry = module
            .find_function_by_name("research.io.stdio.probe")
            .unwrap();
        let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 85);
    }
}
