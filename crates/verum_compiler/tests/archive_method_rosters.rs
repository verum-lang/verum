//! Imported carriers must not replace declaring types or lose their methods.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::{FastParser, Parser};
use verum_types::TypeChecker;
use verum_vbc::{
    archive::ArchiveBuilder,
    codegen::{CodegenConfig, VbcCodegen},
};

#[test]
fn primitive_impl_carrier_does_not_own_text_methods() {
    let compile = |source, bundle| {
        VbcCodegen::with_config(CodegenConfig::new(bundle))
            .compile_module(&Parser::new(source).parse_module().unwrap())
            .unwrap()
    };
    let carrier = compile(
        "module core.sync; public type Marker is protocol {}; implement Marker for Text {}",
        "core",
    );
    let declaration = compile(
        r#"
        module core.text.text;
        public type Text is { count: Int };
        implement Text { public fn byte_count(&self) -> Int { self.count } }
    "#,
        "core.text",
    );
    let mut archive = ArchiveBuilder::stdlib();
    archive.add_module("core", &carrier, &[]).unwrap();
    archive.add_module("core.text", &declaration, &[]).unwrap();
    let metadata = Arc::new(archive_to_core_metadata(&archive.finish()));
    for key in ["Text", "core.text.Text", "core.text.text.Text"] {
        let ty = metadata.types.get(&key.into()).unwrap();
        assert_eq!(
            ty.origin_module_path.as_deref(),
            Some("core.text.text"),
            "{key}"
        );
        assert!(
            ty.methods.iter().any(|m| m == "byte_count"),
            "{key}: {:?}",
            ty.methods
        );
    }
    let ast = FastParser::new()
        .parse_module_str(
            "fn probe(value: &Text) -> Int { value.byte_count() }",
            FileId::new(0),
        )
        .unwrap();
    let mut checker = TypeChecker::new_with_core(metadata);
    checker.set_current_cog("app");
    checker.set_current_module_path("app.main");
    checker.register_stdlib_types_for_module(&ast);
    for item in &ast.items {
        if let ItemKind::Function(f) = &item.kind {
            checker.register_function_signature(f).unwrap();
        }
    }
    for item in &ast.items {
        checker.check_item(item).unwrap();
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostic_sources().is_empty());
}

#[test]
fn same_spelled_types_keep_independent_method_rosters() {
    let mut archive = ArchiveBuilder::stdlib();
    for (owner, method) in [("core.alpha", "first"), ("core.beta", "second")] {
        let source = format!(
            "module {owner}; public type Envelope is {{ value: Int }}; implement Envelope {{ public fn {method}(&self) -> Int {{ self.value }} }}"
        );
        let module = VbcCodegen::with_config(CodegenConfig::new(owner))
            .compile_module(&Parser::new(&source).parse_module().unwrap())
            .unwrap();
        archive.add_module(owner, &module, &[]).unwrap();
    }
    let metadata = archive_to_core_metadata(&archive.finish());
    for (key, expected) in [
        ("Envelope", "first"),
        ("core.alpha.Envelope", "first"),
        ("core.beta.Envelope", "second"),
    ] {
        let methods = &metadata.types.get(&key.into()).unwrap().methods;
        assert_eq!(
            methods.iter().map(|name| name.as_str()).collect::<Vec<_>>(),
            [expected],
            "{key}"
        );
    }
}
