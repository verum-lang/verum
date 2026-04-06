#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for exports module
// Migrated from src/exports.rs per CLAUDE.md standards
//
// Tests refined type tracking across module boundaries: export, re-export,
// predicate visibility (public/crate-local/private), and accessibility validation.

use verum_common::{Maybe, Text};
use verum_modules::exports::*;
use verum_modules::refinement_info::RefinementInfo;
use verum_modules::{ModuleId, ModulePath, Visibility};

use verum_ast::{FileId, Span};

// Helper function to create a simple refinement info for testing
fn create_test_refinement(span: Span) -> RefinementInfo {
    // Create a simple Int{> 0} refinement for testing
    let base_type = verum_ast::ty::Type::int(span);
    let predicate = verum_ast::Expr::literal(verum_ast::literal::Literal::bool(true, span));
    RefinementInfo::new(base_type, predicate, None, span)
}

#[test]
fn test_export_table_basic() {
    let mut table = ExportTable::new();
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);

    let item = ExportedItem::new(
        "test_fn",
        ExportKind::Function,
        Visibility::Public,
        module_id,
        span,
    );

    table.add_export(item).unwrap();
    assert!(table.contains("test_fn"));
    assert_eq!(table.len(), 1);
}

#[test]
fn test_export_reexport() {
    let mut table = ExportTable::new();
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);

    let item = ExportedItem::with_rename(
        "renamed_fn",
        "original_fn",
        ExportKind::Function,
        Visibility::Public,
        module_id,
        span,
    );

    table.add_export(item).unwrap();
    let exported = table.get(&Text::from("renamed_fn")).unwrap();
    assert!(exported.is_reexport());
    assert_eq!(
        exported.original_name.as_ref().unwrap().as_str(),
        "original_fn"
    );
}

/// Test exporting refined types (e.g., public type PositiveInt is Int{> 0}).
/// Refinement becomes part of the public API contract.
#[test]
fn test_export_refined_type() {
    let mut table = ExportTable::new();
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);

    let refinement = create_test_refinement(span);

    let item = ExportedItem::new(
        "PositiveInt",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement, Visibility::Public);

    table.add_export(item).unwrap();

    let exported = table.get(&Text::from("PositiveInt")).unwrap();
    assert!(exported.has_refinement());
    assert!(matches!(exported.get_refinement(), Maybe::Some(_)));
}

/// Test re-exporting refined types preserves refinement
/// Re-exported types preserve their refinements across module boundaries.
#[test]
fn test_reexport_refined_type() {
    let mut source_table = ExportTable::new();
    let mut target_table = ExportTable::new();
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);

    let refinement = create_test_refinement(span);

    // Create refined type in source module
    let item = ExportedItem::new(
        "PositiveInt",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement, Visibility::Public);

    source_table.add_export(item).unwrap();

    // Re-export to target module
    target_table
        .merge(&source_table, Visibility::Public)
        .unwrap();

    // Verify refinement is preserved
    let reexported = target_table.get(&Text::from("PositiveInt")).unwrap();
    assert!(reexported.has_refinement());
    assert!(matches!(reexported.get_refinement(), Maybe::Some(_)));
}

/// Test predicate visibility checking
/// Predicate visibility: public (any module), crate-local, or private.
#[test]
fn test_predicate_visibility_public() {
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);
    let refinement = create_test_refinement(span);

    let item = ExportedItem::new(
        "PositiveInt",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement, Visibility::Public);

    // Public predicate should be accessible from any module
    let from_module = ModulePath::from_str("other.module");
    let item_module = ModulePath::from_str("my.module");

    assert!(item.is_predicate_accessible(&from_module, &item_module));
}

/// Test predicate visibility - crate-local
/// Predicate visibility: public (any module), crate-local, or private.
#[test]
fn test_predicate_visibility_crate_local() {
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);
    let refinement = create_test_refinement(span);

    let item = ExportedItem::new(
        "InternalType",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement, Visibility::PublicCrate);

    // Same crate - should be accessible
    let from_module = ModulePath::from_str("mycrate.other");
    let item_module = ModulePath::from_str("mycrate.internal");
    assert!(item.is_predicate_accessible(&from_module, &item_module));

    // Different crate - should NOT be accessible
    let from_module_external = ModulePath::from_str("othercrate.module");
    assert!(!item.is_predicate_accessible(&from_module_external, &item_module));
}

/// Test predicate visibility - private
/// Predicate visibility: public (any module), crate-local, or private.
#[test]
fn test_predicate_visibility_private() {
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);
    let refinement = create_test_refinement(span);

    let item = ExportedItem::new(
        "PrivateRefined",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement, Visibility::Private);

    // Same module - should be accessible
    let module_path = ModulePath::from_str("my.module");
    assert!(item.is_predicate_accessible(&module_path, &module_path));

    // Different module - should NOT be accessible
    let from_module = ModulePath::from_str("other.module");
    assert!(!item.is_predicate_accessible(&from_module, &module_path));
}

/// Test refinement accessibility validation
/// Validates refinement predicate accessibility across module boundaries.
#[test]
fn test_validate_refinement_accessibility() {
    let mut table = ExportTable::new();
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);
    let module_path = ModulePath::from_str("my.module");

    table.set_module_path(module_path.clone());

    let refinement = create_test_refinement(span);

    // Public predicate - should validate successfully
    let item = ExportedItem::new(
        "PublicRefined",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement.clone(), Visibility::Public);

    table.add_export(item).unwrap();

    let from_module = ModulePath::from_str("other.module");
    assert!(
        table
            .validate_refinement_accessibility(&from_module)
            .is_ok()
    );
}

/// Test that inaccessible predicates are caught during validation
/// Validates refinement predicate accessibility across module boundaries.
#[test]
fn test_validate_inaccessible_predicate() {
    let mut table = ExportTable::new();
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);
    let module_path = ModulePath::from_str("my.module");

    table.set_module_path(module_path.clone());

    let refinement = create_test_refinement(span);

    // Private predicate on public type
    let item = ExportedItem::new(
        "RestrictedRefined",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement, Visibility::Private);

    table.add_export(item).unwrap();

    // Validation from different module should fail
    let from_module = ModulePath::from_str("other.module");
    assert!(
        table
            .validate_refinement_accessibility(&from_module)
            .is_err()
    );
}

/// Test re-exporting with different visibility preserves refinement
/// Re-exported types preserve their refinements across module boundaries.
#[test]
fn test_reexport_with_different_visibility() {
    let mut source_table = ExportTable::new();
    let mut target_table = ExportTable::new();
    let span = Span::new(0, 10, FileId::new(0));
    let module_id = ModuleId::new(1);

    let refinement = create_test_refinement(span);

    // Public type with public predicate in source
    let item = ExportedItem::new(
        "RefinedType",
        ExportKind::Type,
        Visibility::Public,
        module_id,
        span,
    )
    .with_refinement(refinement, Visibility::Public);

    source_table.add_export(item).unwrap();

    // Re-export with crate-local visibility
    target_table
        .merge(&source_table, Visibility::PublicCrate)
        .unwrap();

    // Verify refinement is preserved but type visibility changed
    let reexported = target_table.get(&Text::from("RefinedType")).unwrap();
    assert!(reexported.has_refinement());
    assert_eq!(reexported.visibility, Visibility::PublicCrate);
    // Predicate visibility should be unchanged
    assert_eq!(reexported.predicate_visibility, Visibility::Public);
}
