//! Archive-shaped missing imports retain refusals without copying a whole surface.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Span, Visibility};
use verum_fast_parser::FastParser;
use verum_modules::exports::{ExportKind, ExportedItem};
use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry};
use verum_types::core_metadata::CoreMetadata;
use verum_types::{TypeChecker, TypeError};

#[test]
fn failed_archive_imports_share_the_exact_public_table_surface() {
    let file = FileId::new(0);
    let empty = FastParser::new().parse_module_str("", file).unwrap();
    let id = ModuleId::new(1);
    let path = ModulePath::from_str("core.snapshot_surface");
    let mut info = ModuleInfo::new(id, path.clone(), empty, file, "".into());
    for index in 0..128 {
        info.exports
            .add_export(ExportedItem::new(
                format!("Missing{index}"),
                ExportKind::Type,
                Visibility::Public,
                id,
                Span::dummy(),
            ))
            .unwrap();
    }
    info.exports
        .add_export(ExportedItem::new(
            "Hidden",
            ExportKind::Type,
            Visibility::Private,
            id,
            Span::dummy(),
        ))
        .unwrap();
    let serialized = serde_json::to_vec(&info.exports).unwrap();
    let decoded = serde_json::from_slice(&serialized).unwrap();
    assert_eq!(
        info.exports, decoded,
        "snapshot cache must not change serialized semantics"
    );
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    let mut checker = TypeChecker::new_with_core(Arc::new(CoreMetadata::default()));
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    for index in 0..128 {
        let source = format!("mount core.snapshot_surface.{{Missing{index}}};");
        let ast = FastParser::new()
            .parse_module_str(&source, FileId::new(1))
            .unwrap();
        let ItemKind::Mount(import) = &ast.items[0].kind else {
            panic!("mount expected")
        };
        let error = checker
            .process_import(import, "consumer", &registry)
            .expect_err("an export without declaration must remain refused");
        let TypeError::ImportItemNotFound {
            item_name,
            module_path,
            available_items,
            ..
        } = error
        else {
            panic!("unexpected error: {error:?}")
        };
        // Start cold: the failed import must initialize the registry-owned
        // snapshot itself, not depend on a caller warming it beforehand.
        let registered = registry.get_by_path("core.snapshot_surface").unwrap();
        let expected = registered.exports.public_export_names();
        assert_eq!(item_name.as_str(), format!("Missing{index}"));
        assert_eq!(module_path.as_str(), "core.snapshot_surface");
        assert_eq!(available_items.len(), 128);
        assert!(!available_items.iter().any(|name| name.as_str() == "Hidden"));
        assert!(
            std::ptr::eq(&*available_items, &*expected),
            "each missing leaf must share the table snapshot"
        );
    }
    let registered = registry.get_by_path("core.snapshot_surface").unwrap();
    assert_eq!(serialized, serde_json::to_vec(&registered.exports).unwrap());
}
