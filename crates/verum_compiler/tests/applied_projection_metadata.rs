//! Applied family arguments must not be reconstructed from erased text metadata.
use verum_common::List;
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_vbc::{archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen}, module::VbcModule,
    types::{ProtocolApplication, ProtocolId, TypeId, TypeRef}};

fn source() -> VbcModule {
    let parsed = Parser::new("module core.projections; public type View is protocol { type Rows<R>; }; public fn retain<T:View,R>(value:T.Rows<R>)->T.Rows<R>{value}")
        .parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.projections")).compile_module(&parsed).unwrap()
}
fn publish(module: &VbcModule) -> Result<verum_types::core_metadata::CoreMetadata, String> {
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.projections", module, &[]).unwrap();
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    archive_to_core_metadata(&archive).map_err(|error| error.to_string())
}

#[test]
fn applied_source_signature_is_the_metadata_authority() {
    let metadata = publish(&source()).unwrap();
    let function = metadata.functions.values().find(|function| function.source_signature.as_ref().is_some_and(|signature| signature.name.name.as_str() == "retain")).unwrap();
    let signature = function.source_signature.as_ref().expect("full declaration-owned signature");
    assert_eq!(signature.generics.len(), 2);
    assert!(matches!(&signature.return_type.as_ref().unwrap().kind,
        verum_ast::TypeKind::Generic { args, .. } if args.len() == 1));
}

#[test]
fn removing_the_source_telescope_cannot_publish_a_textual_substitute() {
    let mut module = source();
    for function in &mut module.functions { function.source_signature = None; }
    let error = publish(&module).unwrap_err();
    assert!(error.contains("applied associated projection lacks its declaration-owned source signature"), "{error}");
}

#[test]
fn qualified_projection_is_refused_until_metadata_carries_its_exact_application() {
    let mut module = source();
    let function = module.functions.iter_mut().find(|function| function.source_signature.is_some()).unwrap();
    function.return_type = TypeRef::AssociatedProjection {
        base: Box::new(TypeRef::Concrete(TypeId::INT)),
        qualifier: Some(ProtocolApplication { protocol: ProtocolId(3000), args: vec![TypeRef::ConstValue(1)] }),
        assoc: "Rows".into(), args: vec![TypeRef::Concrete(TypeId::TEXT)],
    };
    let error = publish(&module).unwrap_err();
    assert!(error.contains("qualified associated projection requires a typed metadata carrier"), "{error}");
}

#[test]
fn source_carrier_cannot_hide_a_qualifier_behind_an_unqualified_wire_type() {
    let mut module = source();
    let function = module.functions.iter_mut().find(|function| function.source_signature.is_some()).unwrap();
    let signature = function.source_signature.as_mut().unwrap();
    let return_type = signature.return_type.as_mut().unwrap();
    let verum_ast::TypeKind::Generic { base, .. } = &mut return_type.kind else { panic!("family application") };
    let verum_ast::TypeKind::Qualified { trait_ref, .. } = &mut base.kind else { panic!("associated projection") };
    *trait_ref = verum_ast::Path::single(verum_ast::Ident::new("View", base.span));
    let error = verum_vbc::serialize::serialize_module(&module).unwrap_err();
    assert!(error.to_string().contains("qualified associated projection requires a typed protocol application carrier"), "{error}");
}
