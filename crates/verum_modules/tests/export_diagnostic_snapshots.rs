//! Diagnostic surfaces share exact export-table snapshots across failures.
use verum_ast::{Span, Visibility};
use verum_modules::ModuleId;
use verum_modules::exports::{ExportKind, ExportTable, ExportedItem};

fn item(name: &str, kind: ExportKind, visibility: Visibility) -> ExportedItem {
    ExportedItem::new(name, kind, visibility, ModuleId::new(1), Span::dummy())
}

#[test]
fn public_snapshot_preserves_local_names_and_visibility() {
    let mut table = ExportTable::new();
    table
        .add_export(item("Visible", ExportKind::Type, Visibility::Public))
        .unwrap();
    table
        .add_export(item("Hidden", ExportKind::Type, Visibility::Private))
        .unwrap();
    table
        .add_export(ExportedItem::with_rename(
            "Alias",
            "Original",
            ExportKind::Type,
            Visibility::Public,
            ModuleId::new(2),
            Span::dummy(),
        ))
        .unwrap();
    let names = table.public_export_names();
    assert_eq!(
        names.iter().map(|name| name.as_str()).collect::<Vec<_>>(),
        ["Alias", "Visible"]
    );
    assert!(
        !names
            .iter()
            .any(|name| matches!(name.as_str(), "Hidden" | "Original" | "Absent"))
    );
}

#[test]
fn repeated_queries_share_one_surface_allocation() {
    let mut table = ExportTable::new();
    for index in 0..4096 {
        table
            .add_export(item(
                &format!("Export{index}"),
                ExportKind::Type,
                Visibility::Public,
            ))
            .unwrap();
    }
    let names = table.public_export_names();
    for _ in 0..4096 {
        let next = table.public_export_names();
        assert!(std::ptr::eq(&*names, &*next));
    }
    assert_eq!(names.len(), 4096);
}

#[test]
fn mutations_preserve_old_snapshots_and_clone_isolation() {
    let mut table = ExportTable::new();
    table
        .add_export(item("First", ExportKind::Type, Visibility::Public))
        .unwrap();
    let unobserved = table.clone();
    let old = table.public_export_names();
    assert_eq!(
        table, unobserved,
        "cache initialization is not semantic state"
    );
    let mut changed = table.clone();
    assert!(std::ptr::eq(&*old, &*changed.public_export_names()));
    changed
        .add_export(item("Second", ExportKind::Type, Visibility::Public))
        .unwrap();
    let new = changed.public_export_names();
    assert!(!std::ptr::eq(&*old, &*new));
    assert_eq!(old.len(), 1);
    assert_eq!(table.public_export_names().len(), 1);
    assert_eq!(new.len(), 2);
    changed.clear();
    assert!(changed.public_export_names().is_empty());
    assert_eq!(new.len(), 2);
}

#[test]
fn replacement_invalidates_visibility_but_failed_changes_keep_the_snapshot() {
    let mut table = ExportTable::new();
    table
        .add_export(item("Entry", ExportKind::Module, Visibility::Public))
        .unwrap();
    let visible = table.public_export_names();
    table
        .add_export(item("Entry", ExportKind::Type, Visibility::Private))
        .unwrap();
    assert!(table.public_export_names().is_empty());
    assert_eq!(visible[0].as_str(), "Entry");
    let hidden = table.public_export_names();
    table
        .add_export(item("Entry", ExportKind::Type, Visibility::Private))
        .unwrap();
    assert!(std::ptr::eq(&*hidden, &*table.public_export_names()));
    let mut conflicting = item("Entry", ExportKind::Type, Visibility::Public);
    conflicting.source_module = ModuleId::new(9);
    assert!(table.add_export(conflicting).is_err());
    assert!(std::ptr::eq(&*hidden, &*table.public_export_names()));
}
