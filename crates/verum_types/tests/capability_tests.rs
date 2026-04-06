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
//! Tests for capability tracking in the type system
//!
//! Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0

use verum_common::Text;
use verum_types::capability::*;

#[test]
fn test_type_capability_conversion() {
    use verum_ast::expr::Capability as AstCap;

    // Test conversion from AST capabilities
    let ast_cap = AstCap::ReadOnly;
    let type_cap = TypeCapability::from_ast(&ast_cap);
    assert_eq!(type_cap, TypeCapability::ReadOnly);
    assert_eq!(type_cap.name(), "ReadOnly");

    let ast_custom = AstCap::Custom("CustomOp".to_string().into());
    let type_custom = TypeCapability::from_ast(&ast_custom);
    match type_custom {
        TypeCapability::Custom(name) => assert_eq!(name.as_str(), "CustomOp"),
        _ => panic!("Expected Custom capability"),
    }
}

#[test]
fn test_type_capability_set() {
    // Create empty set
    let empty = TypeCapabilitySet::empty();
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);

    // Create set with capabilities
    let mut set = TypeCapabilitySet::empty();
    set.insert(TypeCapability::ReadOnly);
    set.insert(TypeCapability::Query);

    assert!(!set.is_empty());
    assert_eq!(set.len(), 2);
    assert!(set.contains(&TypeCapability::ReadOnly));
    assert!(set.contains(&TypeCapability::Query));
    assert!(!set.contains(&TypeCapability::Execute));
}

#[test]
fn test_capability_set_operations() {
    let mut set1 = TypeCapabilitySet::empty();
    set1.insert(TypeCapability::ReadOnly);
    set1.insert(TypeCapability::Query);

    let mut set2 = TypeCapabilitySet::empty();
    set2.insert(TypeCapability::ReadOnly);
    set2.insert(TypeCapability::Execute);

    // Test subset
    let mut subset = TypeCapabilitySet::empty();
    subset.insert(TypeCapability::ReadOnly);
    assert!(subset.is_subset_of(&set1));
    assert!(!set1.is_subset_of(&subset));

    // Test intersection
    let intersection = set1.intersect(&set2);
    assert_eq!(intersection.len(), 1);
    assert!(intersection.contains(&TypeCapability::ReadOnly));
    assert!(!intersection.contains(&TypeCapability::Query));
    assert!(!intersection.contains(&TypeCapability::Execute));

    // Test union
    let union = set1.union(&set2);
    assert_eq!(union.len(), 3);
    assert!(union.contains(&TypeCapability::ReadOnly));
    assert!(union.contains(&TypeCapability::Query));
    assert!(union.contains(&TypeCapability::Execute));

    // Test difference
    let diff = set1.difference(&set2);
    assert_eq!(diff.len(), 1);
    assert!(diff.iter().any(|c| c == &TypeCapability::Query));
}

#[test]
fn test_context_capabilities() {
    // Create full capabilities context
    let full = ContextCapabilities::full(Text::from("Database"));
    assert!(!full.is_attenuated);
    assert!(full.has_capability(&TypeCapability::ReadOnly));
    assert!(full.has_capability(&TypeCapability::Execute));

    // Create context with specific capabilities
    let mut caps = TypeCapabilitySet::empty();
    caps.insert(TypeCapability::ReadOnly);
    caps.insert(TypeCapability::Query);

    let limited = ContextCapabilities::with_capabilities(Text::from("Database"), caps);
    assert!(!limited.is_attenuated);
    assert!(limited.has_capability(&TypeCapability::ReadOnly));
    assert!(limited.has_capability(&TypeCapability::Query));
    assert!(!limited.has_capability(&TypeCapability::Execute));
}

#[test]
fn test_context_attenuation() {
    // Start with full capabilities
    let full = ContextCapabilities::full(Text::from("Database"));

    // Attenuate to read-only
    let mut read_only_caps = TypeCapabilitySet::empty();
    read_only_caps.insert(TypeCapability::ReadOnly);
    read_only_caps.insert(TypeCapability::Query);

    let attenuated = full.attenuate(read_only_caps);

    assert!(attenuated.is_attenuated);
    assert!(attenuated.has_capability(&TypeCapability::ReadOnly));
    assert!(attenuated.has_capability(&TypeCapability::Query));
    assert!(!attenuated.has_capability(&TypeCapability::Execute));

    // Original capabilities should be preserved
    match &attenuated.original_capabilities {
        Option::Some(original) => {
            assert!(original.contains(&TypeCapability::Execute));
        }
        _ => panic!("Expected original capabilities"),
    }
}

#[test]
fn test_progressive_attenuation() {
    // Start with full capabilities
    let full = ContextCapabilities::full(Text::from("Database"));

    // First attenuation: remove admin
    let mut no_admin = TypeCapabilitySet::empty();
    no_admin.insert(TypeCapability::ReadOnly);
    no_admin.insert(TypeCapability::Query);
    no_admin.insert(TypeCapability::Execute);

    let first = full.attenuate(no_admin);
    assert!(first.is_attenuated);
    assert!(first.has_capability(&TypeCapability::Execute));

    // Second attenuation: read-only
    let mut read_only = TypeCapabilitySet::empty();
    read_only.insert(TypeCapability::ReadOnly);
    read_only.insert(TypeCapability::Query);

    let second = first.attenuate(read_only);
    assert!(second.is_attenuated);
    assert!(second.has_capability(&TypeCapability::Query));
    assert!(!second.has_capability(&TypeCapability::Execute));

    // Original capabilities should still be from the first full context
    match &second.original_capabilities {
        Option::Some(original) => {
            // Should be the full capabilities
            assert!(original.contains(&TypeCapability::Admin));
        }
        _ => panic!("Expected original capabilities"),
    }
}

#[test]
fn test_capability_requirement() {
    let mut required = TypeCapabilitySet::empty();
    required.insert(TypeCapability::Query);

    let requirement = CapabilityRequirement::new(
        Text::from("Database"),
        required,
        Text::from("query operation"),
    );

    // Create context with sufficient capabilities
    let mut caps = TypeCapabilitySet::empty();
    caps.insert(TypeCapability::ReadOnly);
    caps.insert(TypeCapability::Query);

    let context = ContextCapabilities::with_capabilities(Text::from("Database"), caps);

    // Should be satisfied
    assert!(requirement.is_satisfied_by(&context));

    // Create context with insufficient capabilities
    let mut insufficient = TypeCapabilitySet::empty();
    insufficient.insert(TypeCapability::ReadOnly);

    let context_insufficient =
        ContextCapabilities::with_capabilities(Text::from("Database"), insufficient);

    // Should not be satisfied
    assert!(!requirement.is_satisfied_by(&context_insufficient));

    let missing = requirement.missing_capabilities(&context_insufficient);
    assert_eq!(missing.len(), 1);
    assert!(missing.iter().any(|c| c == &TypeCapability::Query));
}

#[test]
fn test_capability_checker() {
    let mut checker = CapabilityChecker::new();

    // Register a context with specific capabilities
    let mut caps = TypeCapabilitySet::empty();
    caps.insert(TypeCapability::ReadOnly);
    caps.insert(TypeCapability::Query);

    let context = ContextCapabilities::with_capabilities(Text::from("Database"), caps);
    checker.register_context(context);

    // Check successful requirement
    let mut required = TypeCapabilitySet::empty();
    required.insert(TypeCapability::Query);

    let req = CapabilityRequirement::new(Text::from("Database"), required, Text::from("query"));

    assert!(checker.check_requirement(&req).is_ok());

    // Check failed requirement
    let mut required_write = TypeCapabilitySet::empty();
    required_write.insert(TypeCapability::Execute);

    let req_write =
        CapabilityRequirement::new(Text::from("Database"), required_write, Text::from("delete"));

    let result = checker.check_requirement(&req_write);
    assert!(result.is_err());

    match result {
        Err(CapabilityError::InsufficientCapabilities {
            missing,
            context_name,
            ..
        }) => {
            assert_eq!(context_name.as_str(), "Database");
            assert_eq!(missing.len(), 1);
            assert!(missing.iter().any(|c| c == &TypeCapability::Execute));
        }
        _ => panic!("Expected InsufficientCapabilities error"),
    }
}

#[test]
fn test_capability_checker_attenuation() {
    let mut checker = CapabilityChecker::new();

    // Register full capability context
    let full = ContextCapabilities::full(Text::from("Database"));
    checker.register_context(full);

    // Attenuate to read-only
    let mut read_only = TypeCapabilitySet::empty();
    read_only.insert(TypeCapability::ReadOnly);
    read_only.insert(TypeCapability::Query);

    let result = checker.attenuate_context("Database", read_only);
    assert!(result.is_ok());

    let attenuated = result.unwrap();
    assert!(attenuated.is_attenuated);
    assert!(attenuated.has_capability(&TypeCapability::Query));
    assert!(!attenuated.has_capability(&TypeCapability::Execute));

    // Now check that write operations fail
    let mut write_req = TypeCapabilitySet::empty();
    write_req.insert(TypeCapability::Execute);

    let req = CapabilityRequirement::new(Text::from("Database"), write_req, Text::from("delete"));

    assert!(checker.check_requirement(&req).is_err());
}

#[test]
fn test_context_not_found_error() {
    let checker = CapabilityChecker::new();

    let mut required = TypeCapabilitySet::empty();
    required.insert(TypeCapability::Query);

    let req = CapabilityRequirement::new(
        Text::from("UnknownContext"),
        required,
        Text::from("operation"),
    );

    let result = checker.check_requirement(&req);
    assert!(result.is_err());

    match result {
        Err(CapabilityError::ContextNotFound {
            context_name,
            operation,
        }) => {
            assert_eq!(context_name.as_str(), "UnknownContext");
            assert_eq!(operation.as_str(), "operation");
        }
        _ => panic!("Expected ContextNotFound error"),
    }
}

#[test]
fn test_capability_error_messages() {
    // Test error message generation
    let error = CapabilityError::ContextNotFound {
        context_name: Text::from("Database"),
        operation: Text::from("query"),
    };

    let message = error.message();
    assert!(message.as_str().contains("Database"));
    assert!(message.as_str().contains("query"));

    let mut missing = verum_common::List::new();
    missing.push(TypeCapability::Execute);
    missing.push(TypeCapability::Admin);

    let error = CapabilityError::InsufficientCapabilities {
        context_name: Text::from("Database"),
        operation: Text::from("delete_all"),
        required: TypeCapabilitySet::empty(),
        available: TypeCapabilitySet::empty(),
        missing,
    };

    let message = error.message();
    assert!(message.as_str().contains("Database"));
    assert!(message.as_str().contains("delete_all"));
    assert!(message.as_str().contains("Execute"));
    assert!(message.as_str().contains("Admin"));
}

#[test]
fn test_multiple_contexts() {
    let mut checker = CapabilityChecker::new();

    // Register multiple contexts
    let mut db_caps = TypeCapabilitySet::empty();
    db_caps.insert(TypeCapability::Query);
    db_caps.insert(TypeCapability::Execute);

    let db_context = ContextCapabilities::with_capabilities(Text::from("Database"), db_caps);
    checker.register_context(db_context);

    let mut fs_caps = TypeCapabilitySet::empty();
    fs_caps.insert(TypeCapability::FileSystem);
    fs_caps.insert(TypeCapability::ReadOnly);

    let fs_context = ContextCapabilities::with_capabilities(Text::from("FileSystem"), fs_caps);
    checker.register_context(fs_context);

    // Check database requirement
    let mut db_req = TypeCapabilitySet::empty();
    db_req.insert(TypeCapability::Query);

    let req = CapabilityRequirement::new(Text::from("Database"), db_req, Text::from("query"));
    assert!(checker.check_requirement(&req).is_ok());

    // Check filesystem requirement
    let mut fs_req = TypeCapabilitySet::empty();
    fs_req.insert(TypeCapability::FileSystem);

    let req = CapabilityRequirement::new(Text::from("FileSystem"), fs_req, Text::from("read_file"));
    assert!(checker.check_requirement(&req).is_ok());

    // Cross-context should fail
    let mut cross_req = TypeCapabilitySet::empty();
    cross_req.insert(TypeCapability::Query);
    let req = CapabilityRequirement::new(Text::from("FileSystem"), cross_req, Text::from("query"));
    assert!(checker.check_requirement(&req).is_err());
}
