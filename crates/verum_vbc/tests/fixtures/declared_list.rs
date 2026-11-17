use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
};

pub fn compile_consumer(source: &str) -> VbcModule {
    let mut list = Parser::new(include_str!("../../../../core/collections/list.vr"))
        .parse_module()
        .expect("parse the standard List declaration");
    list.items.retain(|item| match &item.kind {
        ItemKind::Module(header) => header.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.name == "List",
        _ => false,
    });
    assert_eq!(list.items.len(), 2, "standard module header and List type");
    let consumer = Parser::new(source).parse_module().expect("consumer grammar");
    let units = [&list, &consumer];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.initialize();
    codegen.collect_unit_declarations(&units).expect("source declarations");
    codegen.compile_unit_items(&units, ItemFailurePolicy::Strict).expect("compile source");
    codegen.finalize_module().expect("completed source module")
}

pub fn source_and_wire(source: VbcModule) -> [VbcModule; 2] {
    let bytes = serialize_module(&source).expect("encode");
    let wire = deserialize_module(&bytes).expect("decode");
    [source, wire]
}
