#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn five_field_records_use_their_declared_static_and_dynamic_methods() {
    let source = r#"
        type Outcome is Ok | Refused;
        type Coded is protocol { fn code(&self) -> Int; };
        type Envelope is { a: Int, b: Int, count: Int, d: Int, e: Int };
        implement Coded for Envelope { fn code(&self) -> Int { self.count } }
        implement Envelope { fn exit_code(&self) -> Outcome { Outcome.Refused } }
        fn invoke(packet: &dyn Coded) -> Int { packet.code() }
        fn probe() -> Int {
            let item = Envelope { a: 1, b: 2, count: 73, d: 4, e: 5 };
            if item.exit_code() is Outcome.Refused { invoke(&item) } else { 0 }
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let probe = module.find_function_by_name("probe").unwrap();
    let loaded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, loaded] {
        let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
        assert_eq!(vm.execute_function(probe).unwrap().try_as_i64(), Some(73));
    }
}
