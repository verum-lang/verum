//! Universe declarations retain lexical AST binders across the binary boundary.
use verum_ast::{
    ItemKind,
    decl::FunctionDecl,
    ty::{GenericParamKind, TypeKind, UniverseLevelExpr},
};
use verum_fast_parser::Parser;
use verum_vbc::{
    deserialize::deserialize_module,
    module::{FunctionDescriptor, ParamDescriptor, VbcModule},
    serialize::serialize_module,
    types::{StringId, TypeId, TypeRef},
};

fn declaration(source: &str) -> FunctionDecl {
    let mut ast = Parser::new(source).parse_module().unwrap();
    let ItemKind::Function(function) = ast.items.remove(0).kind else {
        panic!("function")
    };
    function
}

fn module(signature: FunctionDecl) -> VbcModule {
    let mut module = VbcModule::new("universe_fixture".into());
    let name = module.strings.intern("retain");
    let mut descriptor = FunctionDescriptor::new(name);
    for _ in &signature.params {
        descriptor.params.push(ParamDescriptor {
            name: StringId::EMPTY,
            type_ref: TypeRef::Concrete(TypeId::UNIT),
            is_mut: false,
            default: None,
            type_name: StringId::EMPTY,
        });
    }
    descriptor.source_signature = Some(signature);
    module.functions.push(descriptor);
    module
}

#[test]
fn typed_carrier_roundtrips_without_codegen_feature() {
    for source in [
        "fn retain<universe u>(value: Type(u)) -> Type(u) { value }",
        "fn retain<u: Level>(value: Type(u)) -> Type(u) { value }",
        "fn retain(value: Type(1)) -> Type(1) { value }",
    ] {
        let signature = verum_ast::source_signature::source_signature(&declaration(source), false)
            .unwrap()
            .unwrap();
        assert!(signature.body.is_none());
        let module = module(signature.clone());
        let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        assert_eq!(decoded.functions[0].source_signature, Some(signature));
        assert_eq!(
            decoded.functions[0].params[0].type_ref,
            TypeRef::Concrete(TypeId::UNIT)
        );
    }
}

#[test]
fn writer_refuses_body_and_wrong_kind_level_references() {
    let original = declaration("fn retain<universe u>(value: Type(u)) -> Type(u) { value }");
    assert!(serialize_module(&module(original.clone())).is_err());
    let mut signature = verum_ast::source_signature::source_signature(&original, false)
        .unwrap()
        .unwrap();
    let GenericParamKind::Level { name } = signature.generics[0].kind.clone() else {
        panic!("level")
    };
    signature.generics[0].kind = GenericParamKind::Type {
        name,
        bounds: Default::default(),
        default: None,
    };
    assert!(serialize_module(&module(signature)).is_err());
}

#[test]
fn reader_revalidates_tampered_lexical_binding() {
    let source = declaration("fn retain<universe u>(value: Type(u)) -> Type(u) { value }");
    let signature = verum_ast::source_signature::source_signature(&source, false)
        .unwrap()
        .unwrap();
    let mut corrupted = signature.clone();
    let Some(return_type) = &mut corrupted.return_type else {
        panic!("return")
    };
    let TypeKind::Universe {
        level: Some(UniverseLevelExpr::Variable(name)),
    } = &mut return_type.kind
    else {
        panic!("universe")
    };
    name.name = "v".into();
    let payload = serde_json::to_vec(&signature).unwrap();
    let corrupt_payload = serde_json::to_vec(&corrupted).unwrap();
    assert_eq!(payload.len(), corrupt_payload.len());
    let mut bytes = serialize_module(&module(signature)).unwrap();
    let start = bytes
        .windows(payload.len())
        .position(|part| part == payload)
        .expect("structured payload");
    bytes[start..start + payload.len()].copy_from_slice(&corrupt_payload);
    assert!(deserialize_module(&bytes).is_err());
}

#[test]
fn carrier_arity_and_size_are_checked() {
    let source = declaration("fn retain(value: Type(1)) -> Type(1) { value }");
    let signature = verum_ast::source_signature::source_signature(&source, false)
        .unwrap()
        .unwrap();
    let mut malformed = module(signature.clone());
    malformed.functions[0].params.clear();
    assert!(serialize_module(&malformed).is_err());
    let mut huge = signature;
    huge.name.name = "x".repeat(1024 * 1024 + 1).into();
    assert!(serialize_module(&module(huge)).is_err());
}

#[test]
#[cfg(feature = "codegen")]
fn source_producer_keeps_free_functions_and_refuses_unsupported_declarations() {
    use verum_vbc::codegen::{SkipClass, VbcCodegen};
    let ast = Parser::new("fn retain<universe u>(value: Type(u)) -> Type(u) { value } fn ordinary(value: Int) -> Int { value }").parse_module().unwrap();
    let compiled = VbcCodegen::new().compile_module(&ast).unwrap();
    let retained = compiled
        .functions
        .iter()
        .find(|f| compiled.get_string(f.name) == Some("retain"))
        .unwrap();
    assert!(retained.source_signature.is_some());
    let ordinary = compiled
        .functions
        .iter()
        .find(|f| compiled.get_string(f.name) == Some("ordinary"))
        .unwrap();
    assert!(ordinary.source_signature.is_none());
    for source in [
        "fn bad(value: Type(missing)) { }",
        "fn bad<universe u, A: Type(u)>(value: A) -> A { value }",
        "fn bad<A: Type(1)>(value: A) -> A { value }",
        "fn bad(value: fn<universe u>(Type(u)) -> Type(u)) { }",
        "implement Int { fn bad<universe u>(value: Type(u)) { } }",
        "type Holder<universe u> is { value: Type(u) };",
        "public const VALUE: Type(1) = 1;",
        "public static VALUE: Type(1) = 1;",
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let error = VbcCodegen::new().compile_module(&ast).expect_err(source);
        assert_eq!(error.skip_class(), SkipClass::Fatal, "{error}");
    }
}
