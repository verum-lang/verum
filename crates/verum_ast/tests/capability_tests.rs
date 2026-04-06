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
//! Tests for capability attenuation AST nodes
//!
//! Tests for context-based capability system.

use verum_ast::expr::{Capability, CapabilitySet, Expr, ExprKind};
use verum_ast::span::Span;
use verum_common::{Heap, List, Maybe, Text};

#[test]
fn test_capability_enum() {
    // Test all standard capabilities
    assert_eq!(Capability::ReadOnly.as_str(), "ReadOnly");
    assert_eq!(Capability::WriteOnly.as_str(), "WriteOnly");
    assert_eq!(Capability::ReadWrite.as_str(), "ReadWrite");
    assert_eq!(Capability::Admin.as_str(), "Admin");
    assert_eq!(Capability::Transaction.as_str(), "Transaction");
    assert_eq!(Capability::Network.as_str(), "Network");
    assert_eq!(Capability::FileSystem.as_str(), "FileSystem");
    assert_eq!(Capability::Query.as_str(), "Query");
    assert_eq!(Capability::Execute.as_str(), "Execute");
    assert_eq!(Capability::Logging.as_str(), "Logging");
    assert_eq!(Capability::Metrics.as_str(), "Metrics");
    assert_eq!(Capability::Config.as_str(), "Config");
    assert_eq!(Capability::Cache.as_str(), "Cache");
    assert_eq!(Capability::Auth.as_str(), "Auth");

    // Test custom capability
    let custom = Capability::Custom(Text::from("MyCustomOp"));
    assert_eq!(custom.as_str(), "MyCustomOp");
    assert!(!custom.is_standard());
}

#[test]
fn test_capability_from_str() {
    assert_eq!(
        Capability::from_str("ReadOnly"),
        Maybe::Some(Capability::ReadOnly)
    );
    assert_eq!(
        Capability::from_str("Execute"),
        Maybe::Some(Capability::Execute)
    );

    // Custom capabilities
    let custom = Capability::from_str("CustomOp");
    match custom {
        Maybe::Some(Capability::Custom(name)) => assert_eq!(name.as_str(), "CustomOp"),
        _ => panic!("Expected Custom capability"),
    }
}

#[test]
fn test_capability_set_creation() {
    let span = Span::default();

    // Empty set
    let empty = CapabilitySet::empty(span);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);

    // Single capability
    let single = CapabilitySet::single(Capability::ReadOnly, span);
    assert!(!single.is_empty());
    assert_eq!(single.len(), 1);
    assert!(single.contains(&Capability::ReadOnly));

    // Multiple capabilities
    let caps = List::from(vec![Capability::ReadOnly, Capability::Query]);
    let multi = CapabilitySet::new(caps, span);
    assert_eq!(multi.len(), 2);
    assert!(multi.contains(&Capability::ReadOnly));
    assert!(multi.contains(&Capability::Query));
    assert!(!multi.contains(&Capability::Execute));
}

#[test]
fn test_capability_set_operations() {
    let span = Span::default();

    let caps1 = List::from(vec![Capability::ReadOnly, Capability::Query]);
    let set1 = CapabilitySet::new(caps1, span);

    let caps2 = List::from(vec![
        Capability::ReadOnly,
        Capability::Execute,
        Capability::Transaction,
    ]);
    let set2 = CapabilitySet::new(caps2, span);

    // Test contains
    assert!(set1.contains(&Capability::ReadOnly));
    assert!(!set1.contains(&Capability::Execute));

    // Test subset
    let caps_sub = List::from(vec![Capability::ReadOnly]);
    let subset = CapabilitySet::new(caps_sub, span);
    assert!(subset.is_subset_of(&set1));
    assert!(!set1.is_subset_of(&subset));

    // Test merge (union)
    let merged = set1.merge(&set2);
    assert_eq!(merged.len(), 4); // ReadOnly, Query, Execute, Transaction
    assert!(merged.contains(&Capability::ReadOnly));
    assert!(merged.contains(&Capability::Query));
    assert!(merged.contains(&Capability::Execute));
    assert!(merged.contains(&Capability::Transaction));

    // Test intersect
    let intersected = set1.intersect(&set2);
    assert_eq!(intersected.len(), 1); // Only ReadOnly is common
    assert!(intersected.contains(&Capability::ReadOnly));
    assert!(!intersected.contains(&Capability::Query));
    assert!(!intersected.contains(&Capability::Execute));
}

#[test]
fn test_attenuate_expr_creation() {
    let span = Span::default();

    // Create a context expression
    let context_expr = Expr::new(
        ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            "Database", span,
        ))),
        span,
    );

    // Create capability set
    let caps = CapabilitySet::single(Capability::ReadOnly, span);

    // Create attenuate expression
    let attenuate_expr = Expr::new(
        ExprKind::Attenuate {
            context: Heap::new(context_expr),
            capabilities: caps.clone(),
        },
        span,
    );

    // Verify structure
    match &attenuate_expr.kind {
        ExprKind::Attenuate {
            context,
            capabilities: caps_check,
        } => {
            match &context.kind {
                ExprKind::Path(_) => {} // Expected
                _ => panic!("Expected Path expression"),
            }
            assert_eq!(caps_check.len(), 1);
            assert!(caps_check.contains(&Capability::ReadOnly));
        }
        _ => panic!("Expected Attenuate expression"),
    }
}

#[test]
fn test_capability_equality() {
    assert_eq!(Capability::ReadOnly, Capability::ReadOnly);
    assert_ne!(Capability::ReadOnly, Capability::WriteOnly);

    let custom1 = Capability::Custom(Text::from("Custom1"));
    let custom2 = Capability::Custom(Text::from("Custom1"));
    let custom3 = Capability::Custom(Text::from("Custom2"));

    assert_eq!(custom1, custom2);
    assert_ne!(custom1, custom3);
}

#[test]
fn test_complex_capability_set() {
    let span = Span::default();

    // Database with read-only capabilities
    let db_read = List::from(vec![
        Capability::ReadOnly,
        Capability::Query,
        Capability::Transaction,
    ]);
    let db_read_set = CapabilitySet::new(db_read, span);

    // Database with write capabilities
    let db_write = List::from(vec![
        Capability::ReadWrite,
        Capability::Execute,
        Capability::Transaction,
    ]);
    let db_write_set = CapabilitySet::new(db_write, span);

    // File system capabilities
    let fs = List::from(vec![Capability::FileSystem, Capability::ReadOnly]);
    let fs_set = CapabilitySet::new(fs, span);

    // Test various operations
    assert!(db_read_set.contains(&Capability::Query));
    assert!(!db_read_set.contains(&Capability::Execute));

    // Intersection of read and write should give Transaction only
    let intersection = db_read_set.intersect(&db_write_set);
    assert!(intersection.contains(&Capability::Transaction));
    assert!(!intersection.contains(&Capability::Query));
    assert!(!intersection.contains(&Capability::Execute));

    // No intersection between database and filesystem (except ReadOnly)
    let db_fs_intersection = db_read_set.intersect(&fs_set);
    assert!(db_fs_intersection.contains(&Capability::ReadOnly));
    assert!(!db_fs_intersection.contains(&Capability::FileSystem));
}

#[test]
fn test_progressive_attenuation() {
    let span = Span::default();

    // Start with full capabilities
    let full = List::from(vec![
        Capability::ReadWrite,
        Capability::Query,
        Capability::Execute,
        Capability::Admin,
    ]);
    let full_set = CapabilitySet::new(full, span);

    // First attenuation: remove admin
    let no_admin = List::from(vec![
        Capability::ReadWrite,
        Capability::Query,
        Capability::Execute,
    ]);
    let no_admin_set = CapabilitySet::new(no_admin, span);
    let first_attenuation = full_set.intersect(&no_admin_set);

    assert!(first_attenuation.contains(&Capability::Query));
    assert!(first_attenuation.contains(&Capability::Execute));
    assert!(!first_attenuation.contains(&Capability::Admin));

    // Second attenuation: read-only
    let read_only = List::from(vec![Capability::ReadOnly, Capability::Query]);
    let read_only_set = CapabilitySet::new(read_only, span);
    let second_attenuation = first_attenuation.intersect(&read_only_set);

    assert!(second_attenuation.contains(&Capability::Query));
    assert!(!second_attenuation.contains(&Capability::Execute));
    assert!(!second_attenuation.contains(&Capability::Admin));
}

#[test]
fn test_serialization_roundtrip() {
    use serde_json;

    let cap = Capability::ReadOnly;
    let json = serde_json::to_string(&cap).unwrap();
    let deserialized: Capability = serde_json::from_str(&json).unwrap();
    assert_eq!(cap, deserialized);

    let custom = Capability::Custom(Text::from("CustomOp"));
    let json = serde_json::to_string(&custom).unwrap();
    let deserialized: Capability = serde_json::from_str(&json).unwrap();
    assert_eq!(custom, deserialized);
}

#[test]
fn test_capability_set_serialization() {
    use serde_json;

    let span = Span::default();
    let caps = List::from(vec![Capability::ReadOnly, Capability::Query]);
    let set = CapabilitySet::new(caps, span);

    let json = serde_json::to_string(&set).unwrap();
    let deserialized: CapabilitySet = serde_json::from_str(&json).unwrap();

    assert_eq!(set.len(), deserialized.len());
    assert!(deserialized.contains(&Capability::ReadOnly));
    assert!(deserialized.contains(&Capability::Query));
}
