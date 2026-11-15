//! Closure signatures and frame conventions belong to their producer, not names.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{
    TypeId, TypeRef, VbcModule,
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn compile(source: &str) -> VbcModule {
    VbcCodegen::new().compile_module(&Parser::new(source).parse_module().unwrap()).unwrap()
}

#[test]
fn source_and_wire_keep_user_parameters_separate_from_the_environment() {
    let original = compile("fn main()->Int {let callback:fn(Int)->Int=|value:Int| value+1;callback(72)}");
    let wire = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    for module in [original, wire] {
        let declaration = module.functions.iter().find(|f| f.closure_capture_count.is_some()).unwrap();
        assert_eq!(declaration.closure_capture_count, Some(0));
        assert_eq!(declaration.callable_parameter_count(), Some(1));
        assert_eq!(declaration.params.len(), 2);
        assert_eq!(declaration.semantic_params.as_ref().unwrap()[0], None);
        assert_eq!(declaration.semantic_callable_signature(), Some(TypeRef::Function {
            is_async: false,
            params: vec![TypeRef::Concrete(TypeId::INT)],
            return_type: Box::new(TypeRef::Concrete(TypeId::INT)),
            contexts: Default::default(),
        }));
        let entry = module.find_function_by_name("main").unwrap();
        assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn a_missing_closure_parameter_is_refused_before_abi_lowering() {
    let ast = Parser::new("fn main()->Int {let callback=|value| value;73}").parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(error.to_string().contains("owned closure parameter requires a complete declaration"), "{error}");
}

#[test]
fn abi_parameter_types_cannot_replace_missing_semantic_evidence() {
    let mut module = compile("fn known()->Int {73} fn main()->Int {let selected:fn()->Int=known;let copied=selected;copied()}");
    let known = module.find_function_by_name("known").unwrap();
    module.functions.iter_mut().find(|f| f.id == known).unwrap().semantic_params = None;
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let entry = wire.find_function_by_name("main").unwrap();
    let error = Interpreter::new(wire.into()).execute_function(entry).unwrap_err();
    assert!(error.to_string().contains("function lacks its semantic signature"), "{error}");
}

#[test]
fn malformed_environment_layout_and_old_wire_markers_are_refused() {
    let module = compile("fn main()->Int {let callback:fn()->Int=|| 73;callback()}");
    let index = module.functions.iter().position(|f| f.closure_capture_count.is_some()).unwrap();
    for corrupt_parameter in [false, true] {
        let mut changed = module.clone();
        if corrupt_parameter {
            changed.functions[index].params[0].type_ref = TypeRef::Concrete(TypeId::INT);
        } else {
            changed.functions[index].closure_capture_count = Some(u16::MAX);
        }
        let error = serialize_module(&changed).unwrap_err();
        assert!(error.to_string().contains("requires its parameter and register layout"), "{error}");
    }
    let mut bytes = serialize_module(&module).unwrap();
    bytes[6..8].copy_from_slice(&60u16.to_le_bytes());
    let error = deserialize_module(&bytes).unwrap_err();
    assert!(error.to_string().contains("closure environment convention requires VBC minor 61"), "{error}");
}

#[test]
fn changing_the_environment_invalidates_the_body_receipt() {
    let module = compile("fn probe(value:Int)->Int {let copied=value;copied}");
    let mut declaration = module.functions.iter().find(|f| f.value_uses.is_some()).unwrap().clone();
    let plan = declaration.value_uses.as_ref().unwrap().clone();
    assert!(plan.matches_signature(&declaration));
    declaration.closure_capture_count = Some(0);
    assert!(!plan.matches_signature(&declaration));
}

#[test]
fn formation_cannot_substitute_a_different_environment_layout() {
    let mut module = compile("fn main()->Int {let value=73;let callback:fn()->Int=|| value;callback()}");
    let declaration = module.functions.iter_mut().find(|f| f.closure_capture_count.is_some()).unwrap();
    assert_eq!(declaration.closure_capture_count, Some(1));
    declaration.closure_capture_count = Some(0);
    let entry = module.find_function_by_name("main").unwrap();
    let error = Interpreter::new(module.into()).execute_function(entry).unwrap_err();
    assert!(error.to_string().contains("capture count differs from its declared environment"), "{error}");
}
