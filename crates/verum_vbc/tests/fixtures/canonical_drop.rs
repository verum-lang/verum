//! Cleanup tests import the canonical protocol from compiled source metadata.
use std::sync::OnceLock;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
};

pub fn initialize(codegen: &mut VbcCodegen) {
    codegen.initialize();
    static PROTOCOL: OnceLock<VbcModule> = OnceLock::new();
    let module = PROTOCOL.get_or_init(|| {
        let mut source = Parser::new(include_str!("../../../../core/base/protocols.vr"))
            .parse_module()
            .expect("actual standard protocol grammar");
        source.items.retain(|item| {
            matches!(&item.kind, verum_ast::ItemKind::Module(decl) if decl.items.is_none())
                || item
                    .kind
                    .type_or_protocol_name()
                    .is_some_and(|name| name.as_str() == "Drop")
        });
        assert_eq!(
            source.items.len(),
            2,
            "source header and actual Drop declaration"
        );
        let module = VbcCodegen::with_config(CodegenConfig::new("core.base.protocols"))
            .compile_module(&source)
            .expect("compile canonical Drop declaration");
        deserialize_module(&serialize_module(&module).expect("serialize Drop declaration"))
            .expect("decode Drop declaration")
    });
    codegen.import_archive_module_types(module);
}
