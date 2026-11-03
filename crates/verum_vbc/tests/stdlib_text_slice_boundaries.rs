#![cfg(feature = "codegen")]
//! Execute the emitted stdlib body directly: dynamic Text dispatch can use a
//! separate implementation and must not conceal errors in the source method.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    Value,
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

#[test]
fn source_text_slice_accepts_terminal_boundaries_and_rejects_invalid_ranges() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    let path = root.join("text/text.vr");
    let source = std::fs::read_to_string(&path).unwrap();
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("core.text.text").with_validation());
    let module = compiler.compile_module_with_mounts(
        &ast, path.to_str().unwrap(), root.to_str().unwrap(),
    ).unwrap();
    let module = Arc::new(deserialize_module(&serialize_module(&module).unwrap()).unwrap());
    let entry = module.find_function_by_name("core.text.text.Text.slice")
        .or_else(|| module.find_function_by_name("Text.slice"))
        .expect("emitted stdlib Text.slice");
    for text in ["", "ascii", "ω"] {
        let mut vm = Interpreter::new(module.clone());
        let value = vm.alloc_string(text).unwrap();
        let end = Value::from_i64(text.len() as i64);
        let result = vm.execute_function_with_args(entry, &[value, end, end])
            .unwrap_or_else(|error| panic!("{text:?} terminal slice: {error}"));
        assert_eq!(vm.read_text(result).as_deref(), Some(""));
    }
    for (text, start, end, message) in [
        ("ω", 1, 1, "UTF-8 boundary"),
        ("ascii", -1, 0, "start out of bounds"),
        ("ascii", 6, 6, "start out of bounds"),
        ("ascii", 2, 1, "end out of bounds"),
    ] {
        let mut vm = Interpreter::new(module.clone());
        let value = vm.alloc_string(text).unwrap();
        let error = vm.execute_function_with_args(entry, &[
            value, Value::from_i64(start), Value::from_i64(end),
        ]).expect_err("invalid range must be rejected");
        assert!(error.to_string().contains(message), "{error}");
    }
}
