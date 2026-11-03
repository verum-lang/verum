//! The bytecode source carrier must not erase associated equalities or binders.
#![cfg(feature = "codegen")]
use verum_ast::{
    TypeDeclBody,
    decl::ProtocolItemKind,
    ty::{GenericArg, TypeBoundKind},
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
};

#[test]
fn family_bound_roundtrips_with_its_source_telescope() {
    let ast = Parser::new("module families; type Stream is protocol { type Item; }; type Connection<A> is protocol { type Rows<R>: Stream<Item = R>; };").parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("families"))
        .compile_module(&ast)
        .unwrap();
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let original = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Connection"))
        .unwrap()
        .source_declaration
        .as_ref()
        .expect("family needs source carrier");
    let source = restored
        .types
        .iter()
        .find(|ty| restored.get_string(ty.name) == Some("Connection"))
        .unwrap()
        .source_declaration
        .as_ref()
        .unwrap();
    assert_eq!(original, source);
    let TypeDeclBody::Protocol(body) = &source.body else {
        panic!("protocol")
    };
    let ProtocolItemKind::Type {
        type_params,
        bounds,
        ..
    } = &body.items[0].kind
    else {
        panic!("family")
    };
    assert_eq!(type_params.len(), 1);
    let TypeBoundKind::GenericProtocol(ty) = &bounds[0].kind else {
        panic!("full generic bound")
    };
    let verum_ast::TypeKind::Generic { args, .. } = &ty.kind else {
        panic!("arguments")
    };
    assert!(
        matches!(&args[0], GenericArg::Binding(binding) if binding.name.as_str() == "Item" && matches!(&binding.ty.kind, verum_ast::TypeKind::Path(path) if path.as_ident().unwrap().as_str() == "R"))
    );
}
