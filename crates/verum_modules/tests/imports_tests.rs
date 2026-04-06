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
// Tests for imports module
// Migrated from src/imports.rs per CLAUDE.md standards

use verum_modules::ModuleId;
use verum_modules::exports::ExportKind;
use verum_modules::imports::*;

#[test]
fn test_imported_item() {
    use verum_ast::{FileId, Span};
    let span = Span::new(0, 10, FileId::new(0));
    let item = ImportedItem::direct("test", ModuleId::new(1), ExportKind::Function, span);
    assert!(!item.is_renamed());
    assert_eq!(item.name.as_str(), "test");
}

#[test]
fn test_imported_item_renamed() {
    use verum_ast::{FileId, Span};
    let span = Span::new(0, 10, FileId::new(0));
    let item = ImportedItem::new(
        "renamed",
        "original",
        ModuleId::new(1),
        ExportKind::Function,
        span,
    );
    assert!(item.is_renamed());
    assert_eq!(item.name.as_str(), "renamed");
    assert_eq!(item.original_name.as_str(), "original");
}

#[test]
fn test_path_to_module_path_simple() {
    use verum_ast::{FileId, Ident, Path, PathSegment, Span};
    use verum_common::Text;

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(
        vec![
            PathSegment::Name(Ident::new(Text::from("std"), span)),
            PathSegment::Name(Ident::new(Text::from("io"), span)),
            PathSegment::Name(Ident::new(Text::from("File"), span)),
        ].into(),
        span,
    );

    let module_path = path_to_module_path(&path);
    assert_eq!(module_path.to_string(), "std.io.File");
}

#[test]
fn test_path_to_module_path_with_self() {
    use verum_ast::{FileId, Ident, Path, PathSegment, Span};
    use verum_common::Text;

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(
        vec![
            PathSegment::SelfValue,
            PathSegment::Name(Ident::new(Text::from("utils"), span)),
        ].into(),
        span,
    );

    let module_path = path_to_module_path(&path);
    assert_eq!(module_path.to_string(), "self.utils");
}

#[test]
fn test_path_to_module_path_with_super() {
    use verum_ast::{FileId, Ident, Path, PathSegment, Span};
    use verum_common::Text;

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(
        vec![
            PathSegment::Super,
            PathSegment::Name(Ident::new(Text::from("parent_module"), span)),
        ].into(),
        span,
    );

    let module_path = path_to_module_path(&path);
    assert_eq!(module_path.to_string(), "super.parent_module");
}

#[test]
fn test_path_to_module_path_with_crate() {
    use verum_ast::{FileId, Ident, Path, PathSegment, Span};
    use verum_common::Text;

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(
        vec![
            PathSegment::Cog,
            PathSegment::Name(Ident::new(Text::from("core"), span)),
            PathSegment::Name(Ident::new(Text::from("types"), span)),
        ].into(),
        span,
    );

    let module_path = path_to_module_path(&path);
    assert_eq!(module_path.to_string(), "cog.core.types");
}

#[test]
fn test_path_last_segment_name() {
    use verum_ast::{FileId, Ident, Path, PathSegment, Span};
    use verum_common::Text;

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(
        vec![
            PathSegment::Name(Ident::new(Text::from("std"), span)),
            PathSegment::Name(Ident::new(Text::from("collections"), span)),
            PathSegment::Name(Ident::new(Text::from("HashMap"), span)),
        ].into(),
        span,
    );

    let last_name = path_last_segment_name(&path);
    assert_eq!(last_name.unwrap().as_str(), "HashMap");
}

#[test]
fn test_path_last_segment_name_keyword() {
    use verum_ast::{FileId, Path, PathSegment, Span};

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(vec![PathSegment::SelfValue].into(), span);

    // SelfValue keyword returns None
    let last_name = path_last_segment_name(&path);
    assert!(last_name.is_none());
}

#[test]
fn test_path_parent() {
    use verum_ast::{FileId, Ident, Path, PathSegment, Span};
    use verum_common::Text;

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(
        vec![
            PathSegment::Name(Ident::new(Text::from("std"), span)),
            PathSegment::Name(Ident::new(Text::from("io"), span)),
            PathSegment::Name(Ident::new(Text::from("File"), span)),
        ].into(),
        span,
    );

    let parent = path_parent(&path);
    assert!(parent.is_some());
    assert_eq!(parent.unwrap().to_string(), "std.io");
}

#[test]
fn test_path_parent_single_segment() {
    use verum_ast::{FileId, Ident, Path, PathSegment, Span};
    use verum_common::Text;

    let span = Span::new(0, 10, FileId::new(0));
    let path = Path::new(
        vec![PathSegment::Name(Ident::new(Text::from("std"), span))].into(),
        span,
    );

    // Single segment returns None
    let parent = path_parent(&path);
    assert!(parent.is_none());
}
