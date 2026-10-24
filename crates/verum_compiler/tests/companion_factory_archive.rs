//! Companion factories retain their declaring identity through the real archive writer.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::List;
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::{FastParser, Parser};
use verum_types::TypeChecker;
use verum_vbc::archive::ArchiveBuilder;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

#[test]
fn archived_companion_factories_keep_their_owner_and_visibility() {
    let ast = Parser::new(
        r#"
module core.factory;
public type Factory is { value: Int };
public module Factory {
    public fn make(value: Int) -> Int { value + 1 }
    fn secret() -> Int { 9 }
}
"#,
    )
    .parse_module()
    .expect("producer grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("core"))
        .compile_module(&ast)
        .expect("producer");
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let mut builder = ArchiveBuilder::stdlib();
    builder.add_module("core", &module, &[]).unwrap();
    let metadata = Arc::new(archive_to_core_metadata(&builder.finish()));
    let factory = metadata
        .functions
        .get(&"core.factory.Factory.make".into())
        .expect("exact key");
    assert_eq!(
        factory.origin_module_path.as_deref(),
        Some("core.factory.Factory")
    );
    assert!(factory.parent_type.is_none());
    assert!(
        !metadata
            .functions
            .get(&"core.factory.Factory.secret".into())
            .unwrap()
            .is_public
    );
    for (call, valid) in [
        ("Factory.make(7)", true),
        ("Factory.make(true)", false),
        ("Factory.secret()", false),
    ] {
        let source = format!("mount core.factory.Factory; fn probe() -> Int {{ {call} }}");
        let ast = FastParser::new()
            .parse_module_str(&source, FileId::new(0))
            .unwrap();
        let mut checker = TypeChecker::new_with_core(metadata.clone());
        checker.set_current_cog("app");
        checker.set_current_module_path("app.main");
        checker.register_stdlib_types_for_module(&ast);
        for item in &ast.items {
            if let ItemKind::Function(function) = &item.kind {
                checker.register_function_signature(function).unwrap();
            }
        }
        let mut errors: List<_> = ast
            .items
            .iter()
            .filter_map(|item| checker.check_item(item).err())
            .collect();
        errors.extend(checker.take_deferred_errors());
        assert_eq!(
            errors.is_empty() && checker.diagnostic_sources().is_empty(),
            valid,
            "{call}: {errors:?}"
        );
    }
}
