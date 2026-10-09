//! Parsed declarations retain body presence through real archive registration.
use super::{CodegenContext, populate_ctx_from_archive, register_module_filtered};
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::ArchiveBuilder,
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
};

#[test]
fn full_and_filtered_archive_imports_preserve_source_and_legacy_body_presence() {
    let ast = Parser::new(
        "public fn empty() -> Unit {} public fn forward() -> Unit; \
         implement USize { public fn empty(self) -> Unit {} \
         public fn forward(self) -> List<Byte>; }",
    )
    .parse_module()
    .expect("body declaration syntax");
    let source = VbcCodegen::with_config(CodegenConfig::new("body_archive"))
        .compile_module(&ast)
        .expect("body declaration compilation");
    for legacy in [false, true] {
        let mut bytes = serialize_module(&source).expect("encode source bodies");
        if legacy {
            // The old descriptor format has no declaration-owned flag, even
            // if a reserved bit happens to be set in its existing flag byte.
            bytes[6..8].copy_from_slice(&23_u16.to_le_bytes());
        }
        let module = deserialize_module(&bytes).expect("decode source or legacy descriptors");
        let mut builder = ArchiveBuilder::new();
        builder.add_module("body_archive", &module, &[]).unwrap();
        let archive = builder.finish();
        let loaded = archive.load_module("body_archive").expect("decode archived module");
        let mut full = CodegenContext::new();
        populate_ctx_from_archive(&archive, &mut full, &mut 0).unwrap();
        let mut filtered = CodegenContext::new();
        let wanted = ["empty", "forward", "USize", "USize.empty", "USize.forward"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        register_module_filtered(&loaded, "body_archive", &mut filtered, &wanted, &mut 0);
        for (name, has_body) in [
            ("body_archive.empty", true),
            ("body_archive.forward", false),
            ("body_archive.USize.empty", true),
            ("body_archive.USize.forward", false),
        ] {
            for context in [&full, &filtered] {
                let info = context.lookup_qualified_function(name).expect(name);
                assert_eq!(info.has_source_body, !legacy && has_body, "{name}, legacy={legacy}");
            }
        }
    }
}
