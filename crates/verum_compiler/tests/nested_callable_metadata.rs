//! Runtime linkage of a local callable is not a module-level source export.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{archive::ArchiveBuilder, codegen::{CodegenConfig, VbcCodegen}, interpreter::Interpreter};
fn check(source: &str) {
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("fixture.nested").with_validation())
        .compile_module(&ast).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    for module in [module, decoded] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.functions.iter().find(|function| function.declaration_identity.as_ref()
            .is_some_and(|identity| identity.lexical_parent.is_none()
                && module.strings.get(identity.source_name) == Some("outer"))).unwrap().id;
        assert_eq!(Interpreter::new(Arc::new(module.clone())).execute_function(entry).unwrap().try_as_i64(), Some(42));
        let locals: Vec<_> = module.functions.iter().filter(|function| function.declaration_identity.as_ref()
            .is_some_and(|identity| identity.lexical_parent.is_some())).collect();
        assert!(!locals.is_empty(), "local functions must remain executable archive members");
        let mut builder = ArchiveBuilder::new();
        builder.add_module(&module.name, &module, &[]).unwrap();
        let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(&builder.finish()).unwrap();
        assert!(metadata.functions.values().any(|function| function.source_signature.as_ref()
            .is_some_and(|signature| signature.name.name == "outer")));
        assert!(metadata.functions.values().all(|function| function.source_signature.as_ref()
            .is_none_or(|signature| signature.name.name != "inner")));
        let encoded = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
        let decoded = verum_types::core_metadata::decode_metadata(&encoded).unwrap();
        assert!(decoded.functions.values().all(|function| function.source_signature.as_ref()
            .is_none_or(|signature| signature.name.name != "inner")));
    }
}
#[test]
fn ordinary_nested_function_is_reachable_without_a_publication_alias() {
    check("module fixture.nested; public fn outer()->Int{fn inner(value:Int)->Int{value+1} inner(41)}");
}
#[test]
fn generic_local_signature_is_admitted_without_renaming_source() {
    check("module fixture.nested; public fn outer()->Int{fn inner<N:meta Int>(value:Int)->Int{value} 42}");
}
#[test]
fn method_local_callable_preserves_nominal_parent_boundary() {
    check("module fixture.nested; type Carrier is {marker:()}; implement Carrier{fn compute()->Int{fn inner(value:Int)->Int{value+1} inner(41)}} public fn outer()->Int{Carrier.compute()}");
}
