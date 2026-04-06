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
//! Tests for Path and related types.

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_ast::{FileId, Spanned};
use verum_common::List;
use verum_common::Text;

// Helper function to create test identifiers
fn test_ident(name: &str) -> Ident {
    Ident::new(Text::from(name), Span::dummy())
}

#[test]
fn test_path_from_ident() {
    let ident = test_ident("foo");
    let path = Path::from_ident(ident.clone());

    assert!(path.is_single());
    assert_eq!(path.as_ident().map(|i| i.name.as_str()), Some("foo"));
}

#[test]
fn test_path_from_ident_preserves_span() {
    let span = Span::new(10, 20, FileId::new(1));
    let ident = Ident::new(Text::from("test"), span);
    let path = Path::from_ident(ident);

    assert_eq!(path.span, span);
}

#[test]
fn test_path_single() {
    let ident = test_ident("bar");
    let path = Path::single(ident.clone());

    assert!(path.is_single());
    assert_eq!(path.segments.len(), 1);

    if let Some(PathSegment::Name(i)) = path.segments.first() {
        assert_eq!(i.name.as_str(), "bar");
    } else {
        panic!("Expected Name segment");
    }
}

#[test]
fn test_path_from_ident_equals_single() {
    let ident = test_ident("test");
    let path1 = Path::from_ident(ident.clone());
    let path2 = Path::single(ident);

    assert_eq!(path1, path2);
}

#[test]
fn test_path_new_with_list() {
    let mut segments_list = List::new();
    segments_list.push(PathSegment::Name(test_ident("std")));
    segments_list.push(PathSegment::Name(test_ident("collections")));
    segments_list.push(PathSegment::Name(test_ident("Vec")));

    let path = Path::new(segments_list, Span::dummy());

    assert!(!path.is_single());
    assert_eq!(path.segments.len(), 3);
}

#[test]
fn test_path_as_ident_single() {
    let ident = test_ident("x");
    let path = Path::from_ident(ident.clone());

    let result = path.as_ident();
    assert!(result.is_some());
    assert_eq!(result.unwrap().name.as_str(), "x");
}

#[test]
fn test_path_as_ident_multiple_segments() {
    let mut segments_list = List::new();
    segments_list.push(PathSegment::Name(test_ident("std")));
    segments_list.push(PathSegment::Name(test_ident("io")));

    let path = Path::new(segments_list, Span::dummy());
    assert!(path.as_ident().is_none());
}

#[test]
fn test_path_is_single_true() {
    let path = Path::from_ident(test_ident("foo"));
    assert!(path.is_single());
}

#[test]
fn test_path_is_single_false() {
    let mut segments_list = List::new();
    segments_list.push(PathSegment::Name(test_ident("a")));
    segments_list.push(PathSegment::Name(test_ident("b")));

    let path = Path::new(segments_list, Span::dummy());
    assert!(!path.is_single());
}

#[test]
fn test_path_with_self_segment() {
    let mut segments_list = List::new();
    segments_list.push(PathSegment::SelfValue);
    segments_list.push(PathSegment::Name(test_ident("method")));

    let path = Path::new(segments_list, Span::dummy());
    assert!(!path.is_single());
    assert_eq!(path.segments.len(), 2);
}

#[test]
fn test_path_with_super_segment() {
    let mut segments_list = List::new();
    segments_list.push(PathSegment::Super);
    segments_list.push(PathSegment::Name(test_ident("parent_module")));

    let path = Path::new(segments_list, Span::dummy());
    assert!(!path.is_single());
    assert_eq!(path.segments.len(), 2);
}

#[test]
fn test_path_with_crate_segment() {
    let mut segments_list = List::new();
    segments_list.push(PathSegment::Cog);
    segments_list.push(PathSegment::Name(test_ident("module")));

    let path = Path::new(segments_list, Span::dummy());
    assert!(!path.is_single());
    assert_eq!(path.segments.len(), 2);
}

#[test]
fn test_path_equality() {
    let path1 = Path::from_ident(test_ident("x"));
    let path2 = Path::from_ident(test_ident("x"));
    let path3 = Path::from_ident(test_ident("y"));

    assert_eq!(path1, path2);
    assert_ne!(path1, path3);
}

#[test]
fn test_path_spanned() {
    let span = Span::new(5, 15, FileId::new(0));
    let ident = Ident::new(Text::from("test"), span);
    let path = Path::from_ident(ident);

    assert_eq!(path.span(), span);
}

#[test]
fn test_ident_new() {
    let span = Span::new(0, 5, FileId::new(0));
    let ident = Ident::new("hello", span);

    assert_eq!(ident.name.as_str(), "hello");
    assert_eq!(ident.span, span);
}

#[test]
fn test_ident_as_str() {
    let ident = test_ident("world");
    assert_eq!(ident.as_str(), "world");
}

#[test]
fn test_ident_display() {
    let ident = test_ident("test_name");
    assert_eq!(format!("{}", ident), "test_name");
}

#[test]
fn test_ident_equality() {
    let ident1 = test_ident("foo");
    let ident2 = test_ident("foo");
    let ident3 = test_ident("bar");

    assert_eq!(ident1, ident2);
    assert_ne!(ident1, ident3);
}

#[test]
fn test_ident_spanned() {
    let span = Span::new(10, 20, FileId::new(1));
    let ident = Ident::new("test", span);

    assert_eq!(ident.span(), span);
}

#[test]
fn test_path_segment_name() {
    let segment = PathSegment::Name(test_ident("segment"));

    if let PathSegment::Name(ident) = segment {
        assert_eq!(ident.name.as_str(), "segment");
    } else {
        panic!("Expected Name segment");
    }
}

#[test]
fn test_path_segment_self_value() {
    let segment = PathSegment::SelfValue;
    assert!(matches!(segment, PathSegment::SelfValue));
}

#[test]
fn test_path_segment_super() {
    let segment = PathSegment::Super;
    assert!(matches!(segment, PathSegment::Super));
}

#[test]
fn test_path_segment_crate() {
    let segment = PathSegment::Cog;
    assert!(matches!(segment, PathSegment::Cog));
}

#[test]
fn test_path_segment_equality() {
    let seg1 = PathSegment::Name(test_ident("a"));
    let seg2 = PathSegment::Name(test_ident("a"));
    let seg3 = PathSegment::Name(test_ident("b"));
    let seg4 = PathSegment::SelfValue;

    assert_eq!(seg1, seg2);
    assert_ne!(seg1, seg3);
    assert_ne!(seg1, seg4);
}

#[test]
fn test_complex_path() {
    let mut segments_list = List::new();
    segments_list.push(PathSegment::Cog);
    segments_list.push(PathSegment::Name(test_ident("std")));
    segments_list.push(PathSegment::Name(test_ident("collections")));
    segments_list.push(PathSegment::Name(test_ident("HashMap")));

    let path = Path::new(segments_list, Span::dummy());

    assert_eq!(path.segments.len(), 4);
    assert!(!path.is_single());
    assert!(path.as_ident().is_none());
}

#[test]
fn test_path_from_ident_chain() {
    // Test creating multiple paths from different identifiers
    let idents = vec!["a", "b", "c", "d", "e"];
    let paths: Vec<_> = idents
        .into_iter()
        .map(|name| Path::from_ident(test_ident(name)))
        .collect();

    assert_eq!(paths.len(), 5);
    for path in paths.iter() {
        assert!(path.is_single());
    }
}

#[test]
fn test_empty_path_segments() {
    // Create an empty path (edge case)
    let empty_list = List::new();
    let path = Path::new(empty_list, Span::dummy());

    assert_eq!(path.segments.len(), 0);
    assert!(!path.is_single()); // Not single because it's empty
    assert!(path.as_ident().is_none());
}

#[test]
fn test_path_with_unicode_ident() {
    let span = Span::dummy();
    let ident = Ident::new("函数", span);
    let path = Path::from_ident(ident);

    assert!(path.is_single());
    assert_eq!(path.as_ident().unwrap().name.as_str(), "函数");
}

#[test]
fn test_path_hash_and_eq() {
    use std::collections::HashSet;

    let path1 = Path::from_ident(test_ident("foo"));
    let path2 = Path::from_ident(test_ident("foo"));
    let path3 = Path::from_ident(test_ident("bar"));

    let mut set = HashSet::new();
    set.insert(path1.clone());

    assert!(set.contains(&path2));
    assert!(!set.contains(&path3));
}

#[test]
fn test_path_clone() {
    let original = Path::from_ident(test_ident("original"));
    let cloned = original.clone();

    assert_eq!(original, cloned);
    assert_eq!(original.segments.len(), cloned.segments.len());
}

#[test]
fn test_path_debug_format() {
    let path = Path::from_ident(test_ident("test"));
    let debug_str = format!("{:?}", path);

    assert!(debug_str.contains("Path"));
}
