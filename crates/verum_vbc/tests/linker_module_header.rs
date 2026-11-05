//! Linked source modules are valid before the serializer rebuilds their headers.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::{CodegenConfig, VbcCodegen}, interpreter::Interpreter, module::VbcModule};
fn compile(name: &str, source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new(name).with_validation()).compile_module(&ast).unwrap()
}
#[test]
fn finalized_header_matches_linked_tables_before_and_after_wire() {
    let mut linker = verum_vbc::linker::VbcLinker::new("aarch64-apple-darwin");
    linker.add_user_module(compile("fixture.padding", "module fixture.padding; fn padding()->Int{7}")).unwrap();
    linker.add_user_module(compile("fixture.header", "module fixture.header; type Marker is {value:Int}; public fn outer()->Int{42} public fn message()->Text{\"actual\"}")).unwrap();
    let module = linker.finalize();
    assert!(!module.types.is_empty() && !module.functions.is_empty() && !module.constants.is_empty());
    let decoded = verum_vbc::deserialize::deserialize_module(&verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    for module in [module, decoded] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.functions.iter().find(|function| module.strings.get(function.name).unwrap().ends_with("outer")).unwrap().id;
        assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().try_as_i64(), Some(42));
    }
}
