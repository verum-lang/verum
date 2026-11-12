//! A source call preserves raw addresses and exact pointee loads through VBC.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, value::Value};

fn check(ty: &str, pointer: *mut u8, expected: i64) {
    let source = format!(r#"
        fn forward(p: *const {ty}) -> *const {ty} {{ p }}
        fn entry(p: *const {ty}) -> Int {{
            let address = forward(p);
            unsafe {{ *address.offset(1) as Int }}
        }}
    "#);
    let module = VbcCodegen::new().compile_module(&Parser::new(&source).parse_module().unwrap()).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    for module in [module, decoded] {
        let entry = module.find_function_by_name("entry").unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        let result = vm.execute_function_with_args(entry, &[Value::from_ptr(pointer)]).unwrap();
        assert_eq!(result.try_as_i64(), Some(expected), "{ty}");
    }
}

#[test]
fn returned_addresses_preserve_scalar_width_and_sign() {
    check("Byte", [0u8, 201].as_mut_ptr(), 201);
    check("Int16", [0i16, -130].as_mut_ptr().cast(), -130);
    check("UInt32", [0u32, 4_000_000_000].as_mut_ptr().cast(), 4_000_000_000);
}
