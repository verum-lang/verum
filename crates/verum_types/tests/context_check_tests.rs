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
use verum_ast::decl::{ContextDecl, Visibility};
use verum_ast::span::Span;
use verum_ast::ty::Ident;
use verum_common::Text;
use verum_types::TypeError;
use verum_types::context_check::*;

#[test]
fn test_context_requirement() {
    let req = ContextRequirement::new("Database".to_string(), Span::default());
    assert_eq!(req.full_path(), "Database");

    let sub_req = ContextRequirement::with_sub(
        "FileSystem".to_string(),
        "Read".to_string(),
        Span::default(),
    );
    assert_eq!(sub_req.full_path(), "FileSystem.Read");
}

#[test]
fn test_context_set() {
    let mut set = ContextSet::new();
    assert!(set.is_empty());

    let req1 = ContextRequirement::new("Database".to_string(), Span::default());
    set.add(req1.clone());
    assert!(!set.is_empty());
    assert!(set.contains("Database"));

    let req2 = ContextRequirement::new("Logger".to_string(), Span::default());
    set.add(req2);
    assert_eq!(set.len(), 2);
}

#[test]
fn test_context_env_scoping() {
    let env = ContextEnv::new();
    assert!(!env.has_context("Logger"));

    // Create child scope
    let child = env.child();

    // Child should also not have Logger
    assert!(!child.has_context("Logger"));

    // Note: We can't test provide() without a real ContextDecl
    // Full integration tests will cover this
}

#[test]
fn test_context_set_union() {
    let req1 = ContextRequirement::new("Database".to_string(), Span::default());
    let set1 = ContextSet::singleton(req1);

    let req2 = ContextRequirement::new("Logger".to_string(), Span::default());
    let set2 = ContextSet::singleton(req2);

    let union = set1.union(&set2);
    assert_eq!(union.len(), 2);
    assert!(union.contains("Database"));
    assert!(union.contains("Logger"));
}

// ============================================================================
// Sub-Context Validation Tests
// Context type system integration: context requirements tracked in function types, checked at call sites — Section 10.1
// ============================================================================

/// Helper function to create a test context declaration with sub-contexts
fn create_fs_context() -> ContextDecl {
    // Create Read sub-context
    let read_sub = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Read".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    // Create Write sub-context
    let write_sub = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Write".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    // Create Admin sub-context
    let admin_sub = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Admin".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    // Create main FileSystem context with sub-contexts
    let sub_contexts = vec![read_sub, write_sub, admin_sub];

    ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("FileSystem".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: sub_contexts.into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    }
}

/// Helper function to create a context without sub-contexts
fn create_simple_context(name: &str) -> ContextDecl {
    ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new(name.to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    }
}

#[test]
fn test_valid_sub_context() {
    let mut checker = ContextChecker::new();
    let fs_context = create_fs_context();

    checker.register_context("FileSystem".to_string(), fs_context);

    // Valid sub-context: FileSystem.Read
    let result = checker.check_sub_context("FileSystem", "Read", Span::default());
    assert!(result.is_ok(), "FileSystem.Read should be valid");

    // Valid sub-context: FileSystem.Write
    let result = checker.check_sub_context("FileSystem", "Write", Span::default());
    assert!(result.is_ok(), "FileSystem.Write should be valid");

    // Valid sub-context: FileSystem.Admin
    let result = checker.check_sub_context("FileSystem", "Admin", Span::default());
    assert!(result.is_ok(), "FileSystem.Admin should be valid");
}

#[test]
fn test_invalid_sub_context() {
    let mut checker = ContextChecker::new();
    let fs_context = create_fs_context();

    checker.register_context("FileSystem".to_string(), fs_context);

    // Invalid sub-context: FileSystem.Execute (doesn't exist)
    let result = checker.check_sub_context("FileSystem", "Execute", Span::default());

    match result {
        Err(TypeError::InvalidSubContext {
            context,
            sub_context,
            available,
            ..
        }) => {
            assert_eq!(context, "FileSystem");
            assert_eq!(sub_context, "Execute");
            assert_eq!(available.len(), 3);
            assert!(available.contains(&Text::from("Read")));
            assert!(available.contains(&Text::from("Write")));
            assert!(available.contains(&Text::from("Admin")));
        }
        _ => panic!("Expected InvalidSubContext error"),
    }
}

#[test]
fn test_sub_context_on_nonexistent_context() {
    let checker = ContextChecker::new();

    // Try to check sub-context on undefined context
    let result = checker.check_sub_context("NonexistentContext", "Read", Span::default());

    match result {
        Err(TypeError::UndefinedContext { name, .. }) => {
            assert_eq!(name, "NonexistentContext");
        }
        _ => panic!("Expected UndefinedContext error"),
    }
}

#[test]
fn test_sub_context_on_context_without_subs() {
    let mut checker = ContextChecker::new();
    let simple_context = create_simple_context("Logger");

    checker.register_context("Logger".to_string(), simple_context);

    // Try to use sub-context on context that has no sub-contexts
    let result = checker.check_sub_context("Logger", "Debug", Span::default());

    match result {
        Err(TypeError::InvalidSubContext {
            context,
            sub_context,
            available,
            ..
        }) => {
            assert_eq!(context, "Logger");
            assert_eq!(sub_context, "Debug");
            assert!(
                available.is_empty(),
                "Should have no available sub-contexts"
            );
        }
        _ => panic!("Expected InvalidSubContext error"),
    }
}

#[test]
fn test_case_sensitive_sub_context() {
    let mut checker = ContextChecker::new();
    let fs_context = create_fs_context();

    checker.register_context("FileSystem".to_string(), fs_context);

    // Sub-context names are case-sensitive: "read" != "Read"
    let result = checker.check_sub_context("FileSystem", "read", Span::default());

    match result {
        Err(TypeError::InvalidSubContext {
            context,
            sub_context,
            available,
            ..
        }) => {
            assert_eq!(context, "FileSystem");
            assert_eq!(sub_context, "read");
            assert!(available.contains(&Text::from("Read")));
        }
        _ => panic!("Expected InvalidSubContext error for case mismatch"),
    }
}

#[test]
fn test_multiple_contexts_with_sub_contexts() {
    let mut checker = ContextChecker::new();

    // Create FileSystem context with Read, Write, Admin
    let fs_context = create_fs_context();
    checker.register_context("FileSystem".to_string(), fs_context);

    // Create Database context with Query, Execute sub-contexts
    let query_sub = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Query".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    let execute_sub = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Execute".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    let db_subs = vec![query_sub, execute_sub];

    let db_context = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Database".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: db_subs.into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    checker.register_context("Database".to_string(), db_context);

    // Validate FileSystem sub-contexts
    assert!(
        checker
            .check_sub_context("FileSystem", "Read", Span::default())
            .is_ok()
    );
    assert!(
        checker
            .check_sub_context("FileSystem", "Write", Span::default())
            .is_ok()
    );

    // Validate Database sub-contexts
    assert!(
        checker
            .check_sub_context("Database", "Query", Span::default())
            .is_ok()
    );
    assert!(
        checker
            .check_sub_context("Database", "Execute", Span::default())
            .is_ok()
    );

    // Cross-contamination test: Database.Read should fail
    let result = checker.check_sub_context("Database", "Read", Span::default());
    assert!(result.is_err(), "Database.Read should be invalid");

    // Cross-contamination test: FileSystem.Query should fail
    let result = checker.check_sub_context("FileSystem", "Query", Span::default());
    assert!(result.is_err(), "FileSystem.Query should be invalid");
}

#[test]
fn test_empty_sub_context_name() {
    let mut checker = ContextChecker::new();
    let fs_context = create_fs_context();

    checker.register_context("FileSystem".to_string(), fs_context);

    // Empty sub-context name should fail
    let result = checker.check_sub_context("FileSystem", "", Span::default());

    match result {
        Err(TypeError::InvalidSubContext { sub_context, .. }) => {
            assert_eq!(sub_context, "");
        }
        _ => panic!("Expected InvalidSubContext error for empty sub-context name"),
    }
}

#[test]
fn test_sub_context_with_special_characters() {
    let mut checker = ContextChecker::new();
    let fs_context = create_fs_context();

    checker.register_context("FileSystem".to_string(), fs_context);

    // Sub-context with special characters should fail
    let result = checker.check_sub_context("FileSystem", "Read.Write", Span::default());

    match result {
        Err(TypeError::InvalidSubContext { sub_context, .. }) => {
            assert_eq!(sub_context, "Read.Write");
        }
        _ => panic!("Expected InvalidSubContext error for special characters"),
    }
}

#[test]
fn test_nested_sub_contexts_not_supported() {
    let mut checker = ContextChecker::new();

    // Create a sub-context with its own sub-context (nested)
    let nested_sub = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Metadata".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    let read_subs = vec![nested_sub];

    let read_sub = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("Read".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: read_subs.into(), // Read has its own sub-context
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    let fs_subs = vec![read_sub];

    let fs_context = ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new("FileSystem".to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: fs_subs.into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    };

    checker.register_context("FileSystem".to_string(), fs_context);

    // FileSystem.Read should be valid
    assert!(
        checker
            .check_sub_context("FileSystem", "Read", Span::default())
            .is_ok()
    );

    // Note: Current implementation doesn't handle FileSystem.Read.Metadata
    // This would require multi-level path resolution which is not in scope for this fix
}

// ============================================================================
// Context Availability Tests
// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Section 2.5
// ============================================================================

#[test]
fn test_check_context_availability_required() {
    let mut checker = ContextChecker::new();
    let db_context = create_simple_context("Database");
    checker.register_context("Database".to_string(), db_context);

    // Set Database as required
    let mut required = ContextSet::new();
    required.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    checker.set_required(required);

    // Should be available since it's required
    let result = checker.check_context_availability("Database", Span::default());
    assert!(
        result.is_ok(),
        "Database should be available via requirement"
    );
}

#[test]
fn test_check_context_availability_provided() {
    let mut checker = ContextChecker::new();
    let logger_context = create_simple_context("Logger");
    checker.register_context("Logger".to_string(), logger_context);

    // Provide Logger in environment
    checker
        .provide_context("Logger".to_string(), Span::default())
        .unwrap();

    // Should be available since it's provided
    let result = checker.check_context_availability("Logger", Span::default());
    assert!(result.is_ok(), "Logger should be available via provide");
}

#[test]
fn test_check_context_availability_missing() {
    let checker = ContextChecker::new();

    // Database not required or provided
    let result = checker.check_context_availability("Database", Span::default());

    match result {
        Err(TypeError::MissingContext { context, .. }) => {
            assert_eq!(context, "Database");
        }
        _ => panic!("Expected MissingContext error"),
    }
}

#[test]
fn test_check_context_availability_undefined() {
    let mut checker = ContextChecker::new();

    // Set as required but not registered
    let mut required = ContextSet::new();
    required.add(ContextRequirement::new(
        "UndefinedContext".to_string(),
        Span::default(),
    ));
    checker.set_required(required);

    let result = checker.check_context_availability("UndefinedContext", Span::default());

    match result {
        Err(TypeError::UndefinedContext { name, .. }) => {
            assert_eq!(name, "UndefinedContext");
        }
        _ => panic!("Expected UndefinedContext error"),
    }
}

// ============================================================================
// Context Satisfaction Tests
// Context type system integration: context requirements tracked in function types, checked at call sites — Section 3.2
// ============================================================================

#[test]
fn test_check_context_satisfaction_fully_provided() {
    let mut checker = ContextChecker::new();
    let db_context = create_simple_context("Database");
    let logger_context = create_simple_context("Logger");
    checker.register_context("Database".to_string(), db_context);
    checker.register_context("Logger".to_string(), logger_context);

    // Required: [Database, Logger]
    let mut required = ContextSet::new();
    required.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    required.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    // Provided: [Database, Logger]
    let mut provided = ContextSet::new();
    provided.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    provided.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    let result = checker.check_context_satisfaction(&required, &provided, Span::default());
    assert!(result.is_ok(), "All requirements should be satisfied");
}

#[test]
fn test_check_context_satisfaction_partial_provide() {
    let mut checker = ContextChecker::new();
    let db_context = create_simple_context("Database");
    let logger_context = create_simple_context("Logger");
    checker.register_context("Database".to_string(), db_context);
    checker.register_context("Logger".to_string(), logger_context);

    // Provide Logger in environment
    checker
        .provide_context("Logger".to_string(), Span::default())
        .unwrap();

    // Required: [Database, Logger]
    let mut required = ContextSet::new();
    required.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    required.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    // Provided via using: [Database]
    let mut provided = ContextSet::new();
    provided.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));

    // Logger from environment, Database from provided
    let result = checker.check_context_satisfaction(&required, &provided, Span::default());
    assert!(result.is_ok(), "Should satisfy with mixed sources");
}

#[test]
fn test_check_context_satisfaction_missing() {
    let mut checker = ContextChecker::new();
    let db_context = create_simple_context("Database");
    let logger_context = create_simple_context("Logger");
    checker.register_context("Database".to_string(), db_context);
    checker.register_context("Logger".to_string(), logger_context);

    // Required: [Database, Logger]
    let mut required = ContextSet::new();
    required.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    required.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    // Provided: [Database] only
    let mut provided = ContextSet::new();
    provided.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));

    let result = checker.check_context_satisfaction(&required, &provided, Span::default());

    match result {
        Err(TypeError::MissingContext { context, .. }) => {
            assert_eq!(context, "Logger");
        }
        _ => panic!("Expected MissingContext error for Logger"),
    }
}

#[test]
fn test_check_context_satisfaction_empty_requirements() {
    let checker = ContextChecker::new();

    // Required: []
    let required = ContextSet::new();

    // Provided: [Database]
    let mut provided = ContextSet::new();
    provided.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));

    let result = checker.check_context_satisfaction(&required, &provided, Span::default());
    assert!(
        result.is_ok(),
        "Empty requirements should always be satisfied"
    );
}

#[test]
fn test_check_context_satisfaction_with_sub_context() {
    let mut checker = ContextChecker::new();
    let fs_context = create_fs_context();
    checker.register_context("FileSystem".to_string(), fs_context);

    // Required: [FileSystem.Read]
    let mut required = ContextSet::new();
    required.add(ContextRequirement::with_sub(
        "FileSystem".to_string(),
        "Read".to_string(),
        Span::default(),
    ));

    // Provided: [FileSystem]
    let mut provided = ContextSet::new();
    provided.add(ContextRequirement::new(
        "FileSystem".to_string(),
        Span::default(),
    ));

    // Should validate that Read sub-context exists
    let result = checker.check_context_satisfaction(&required, &provided, Span::default());
    assert!(result.is_ok(), "Sub-context should be validated");
}

// ============================================================================
// Context Inference Tests
// Context resolution: resolving context names to declarations, expanding groups, checking provision — .3
// ============================================================================

#[test]
fn test_infer_requirements() {
    let checker = ContextChecker::new();

    let used_contexts = vec!["Database", "Logger", "Auth"];
    let inferred = checker.infer_requirements(&used_contexts);

    assert_eq!(inferred.len(), 3);
    assert!(inferred.contains("Database"));
    assert!(inferred.contains("Logger"));
    assert!(inferred.contains("Auth"));
}

#[test]
fn test_infer_requirements_empty() {
    let checker = ContextChecker::new();

    let used_contexts: Vec<&str> = vec![];
    let inferred = checker.infer_requirements(&used_contexts);

    assert!(inferred.is_empty());
}

#[test]
fn test_infer_requirements_duplicates() {
    let checker = ContextChecker::new();

    let used_contexts = vec!["Database", "Database", "Logger"];
    let inferred = checker.infer_requirements(&used_contexts);

    // Set should deduplicate
    assert_eq!(inferred.len(), 2);
    assert!(inferred.contains("Database"));
    assert!(inferred.contains("Logger"));
}

// ============================================================================
// Transitive Requirements Tests
// Context resolution: resolving context names to declarations, expanding groups, checking provision — .2
// ============================================================================

#[test]
fn test_compute_transitive_requirements() {
    let checker = ContextChecker::new();

    // Direct requirements: [Logger]
    let mut direct = ContextSet::new();
    direct.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    // Callee 1 requirements: [Database]
    let mut callee1 = ContextSet::new();
    callee1.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));

    // Callee 2 requirements: [Auth]
    let mut callee2 = ContextSet::new();
    callee2.add(ContextRequirement::new("Auth".to_string(), Span::default()));

    let transitive = checker.compute_transitive_requirements(&direct, &[&callee1, &callee2]);

    // Should have union: [Logger, Database, Auth]
    assert_eq!(transitive.len(), 3);
    assert!(transitive.contains("Logger"));
    assert!(transitive.contains("Database"));
    assert!(transitive.contains("Auth"));
}

#[test]
fn test_compute_transitive_requirements_overlapping() {
    let checker = ContextChecker::new();

    // Direct requirements: [Logger, Database]
    let mut direct = ContextSet::new();
    direct.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));
    direct.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));

    // Callee requirements: [Database, Auth]
    let mut callee = ContextSet::new();
    callee.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    callee.add(ContextRequirement::new("Auth".to_string(), Span::default()));

    let transitive = checker.compute_transitive_requirements(&direct, &[&callee]);

    // Should deduplicate Database: [Logger, Database, Auth]
    assert_eq!(transitive.len(), 3);
    assert!(transitive.contains("Logger"));
    assert!(transitive.contains("Database"));
    assert!(transitive.contains("Auth"));
}

#[test]
fn test_compute_transitive_requirements_no_callees() {
    let checker = ContextChecker::new();

    // Direct requirements: [Logger]
    let mut direct = ContextSet::new();
    direct.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    let transitive = checker.compute_transitive_requirements(&direct, &[]);

    // Should just return direct requirements
    assert_eq!(transitive.len(), 1);
    assert!(transitive.contains("Logger"));
}

// ============================================================================
// Function Validation Tests
// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Section 2.5
// ============================================================================

#[test]
fn test_validate_function_complete() {
    let mut checker = ContextChecker::new();
    checker.register_context("Database".to_string(), create_simple_context("Database"));
    checker.register_context("Logger".to_string(), create_simple_context("Logger"));

    // Declared: [Database, Logger]
    let mut declared = ContextSet::new();
    declared.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    declared.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    // Used: [Database, Logger]
    let used = vec!["Database", "Logger"];

    // No callees
    let callees: Vec<&ContextSet> = vec![];

    let result = checker.validate_function(&declared, &used, &callees, Span::default());
    assert!(result.is_ok(), "Function should be valid");
}

#[test]
fn test_validate_function_missing_declaration() {
    let mut checker = ContextChecker::new();
    checker.register_context("Database".to_string(), create_simple_context("Database"));
    checker.register_context("Logger".to_string(), create_simple_context("Logger"));

    // Declared: [Database] only
    let mut declared = ContextSet::new();
    declared.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));

    // Used: [Database, Logger]
    let used = vec!["Database", "Logger"];

    // No callees
    let callees: Vec<&ContextSet> = vec![];

    let result = checker.validate_function(&declared, &used, &callees, Span::default());

    match result {
        Err(TypeError::MissingContext { context, .. }) => {
            assert_eq!(context, "Logger");
        }
        _ => panic!("Expected MissingContext error for undeclared Logger"),
    }
}

#[test]
fn test_validate_function_with_callees() {
    let mut checker = ContextChecker::new();
    checker.register_context("Database".to_string(), create_simple_context("Database"));
    checker.register_context("Logger".to_string(), create_simple_context("Logger"));
    checker.register_context("Auth".to_string(), create_simple_context("Auth"));

    // Declared: [Database, Logger, Auth]
    let mut declared = ContextSet::new();
    declared.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    declared.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));
    declared.add(ContextRequirement::new("Auth".to_string(), Span::default()));

    // Used directly: [Database]
    let used = vec!["Database"];

    // Callee requirements: [Logger, Auth]
    let mut callee_reqs = ContextSet::new();
    callee_reqs.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));
    callee_reqs.add(ContextRequirement::new("Auth".to_string(), Span::default()));

    let callees = vec![&callee_reqs];

    let result = checker.validate_function(&declared, &used, &callees, Span::default());
    assert!(
        result.is_ok(),
        "Function should satisfy transitive requirements"
    );
}

#[test]
fn test_validate_function_missing_transitive() {
    let mut checker = ContextChecker::new();
    checker.register_context("Database".to_string(), create_simple_context("Database"));
    checker.register_context("Logger".to_string(), create_simple_context("Logger"));

    // Declared: [Database] only
    let mut declared = ContextSet::new();
    declared.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));

    // Used directly: [Database]
    let used = vec!["Database"];

    // Callee requirements: [Logger]
    let mut callee_reqs = ContextSet::new();
    callee_reqs.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));

    let callees = vec![&callee_reqs];

    let result = checker.validate_function(&declared, &used, &callees, Span::default());

    match result {
        Err(TypeError::MissingContext { context, .. }) => {
            assert_eq!(context, "Logger");
        }
        _ => panic!("Expected MissingContext error for transitive Logger requirement"),
    }
}

// ============================================================================
// Integration Test: Complete Context Checking Flow
// ============================================================================

#[test]
fn test_complete_context_checking_workflow() {
    let mut checker = ContextChecker::new();

    // Register contexts
    checker.register_context("Database".to_string(), create_simple_context("Database"));
    checker.register_context("Logger".to_string(), create_simple_context("Logger"));
    checker.register_context("Auth".to_string(), create_simple_context("Auth"));

    // Simulate function: fn process() using [Database, Logger]
    let mut func_contexts = ContextSet::new();
    func_contexts.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    func_contexts.add(ContextRequirement::new(
        "Logger".to_string(),
        Span::default(),
    ));
    checker.set_required(func_contexts.clone());

    // Check that Database is available
    assert!(
        checker
            .check_context_availability("Database", Span::default())
            .is_ok()
    );

    // Check that Logger is available
    assert!(
        checker
            .check_context_availability("Logger", Span::default())
            .is_ok()
    );

    // Check that Auth is not available (not in requirements)
    assert!(
        checker
            .check_context_availability("Auth", Span::default())
            .is_err()
    );

    // Simulate calling a function requiring [Database, Auth]
    let mut callee_contexts = ContextSet::new();
    callee_contexts.add(ContextRequirement::new(
        "Database".to_string(),
        Span::default(),
    ));
    callee_contexts.add(ContextRequirement::new("Auth".to_string(), Span::default()));

    // Should fail because Auth is missing
    let result =
        checker.check_context_satisfaction(&callee_contexts, &func_contexts, Span::default());
    assert!(result.is_err(), "Should fail due to missing Auth");

    // Now provide Auth locally
    checker
        .provide_context("Auth".to_string(), Span::default())
        .unwrap();

    // Should now succeed
    let result =
        checker.check_context_satisfaction(&callee_contexts, &func_contexts, Span::default());
    assert!(result.is_ok(), "Should succeed with Auth provided");
}

// ============================================================================
// Advanced context patterns (negative contexts, call graph verification, module aliases): Negative Context Tests
// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
// ============================================================================

#[test]
fn test_negative_context_requirement() {
    // Test creating a negative context requirement
    let neg_req = ContextRequirement::negative("Database".to_string(), Span::default());
    assert!(neg_req.is_negative);
    assert!(neg_req.is_excluded());
    assert_eq!(neg_req.full_path(), "Database");
}

#[test]
fn test_context_set_negative_tracking() {
    let mut set = ContextSet::new();

    // Add positive context
    set.add(ContextRequirement::new("Logger".to_string(), Span::default()));

    // Add negative context
    set.add(ContextRequirement::negative("Database".to_string(), Span::default()));

    assert!(set.contains("Logger"), "Should contain positive Logger");
    assert!(!set.contains("Database"), "contains() should return false for negative contexts");
    assert!(set.is_excluded("Database"), "Database should be excluded");
    assert!(!set.is_excluded("Logger"), "Logger should not be excluded");
}

#[test]
fn test_context_set_validate_usage() {
    let mut set = ContextSet::new();
    set.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    set.add(ContextRequirement::new("Logger".to_string(), Span::default()));

    // Validation should fail for excluded context
    let result = set.validate_usage("Database");
    assert!(result.is_err());

    // Validation should pass for positive context
    let result = set.validate_usage("Logger");
    assert!(result.is_ok());

    // Validation should pass for unrelated context
    let result = set.validate_usage("Cache");
    assert!(result.is_ok());
}

#[test]
fn test_context_set_negative_iteration() {
    let mut set = ContextSet::new();
    set.add(ContextRequirement::new("Logger".to_string(), Span::default()));
    set.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    set.add(ContextRequirement::negative("Network".to_string(), Span::default()));

    // Should have 1 positive context
    let positive: Vec<_> = set.positive_contexts().collect();
    assert_eq!(positive.len(), 1);
    assert_eq!(positive[0].name, "Logger");

    // Should have 2 negative contexts
    let negative: Vec<_> = set.negative_contexts().collect();
    assert_eq!(negative.len(), 2);

    // Check excluded names
    let excluded = set.excluded_names();
    assert!(excluded.iter().any(|n| n.as_str() == "Database"));
    assert!(excluded.iter().any(|n| n.as_str() == "Network"));
}

#[test]
fn test_check_context_not_excluded() {
    let mut checker = ContextChecker::new();

    // Set up requirements with negative context
    let mut required = ContextSet::new();
    required.add(ContextRequirement::new("Logger".to_string(), Span::default()));
    required.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    checker.set_required(required);

    // Accessing Logger should be OK
    let result = checker.check_context_not_excluded("Logger", Span::default());
    assert!(result.is_ok());

    // Accessing Database should fail
    let result = checker.check_context_not_excluded("Database", Span::default());
    assert!(result.is_err());
}

#[test]
fn test_transitive_negative_context_violation() {
    use verum_common::List;

    let mut checker = ContextChecker::new();

    // Register the callee function that requires Database
    let mut callee_contexts = ContextSet::new();
    callee_contexts.add(ContextRequirement::new("Database".to_string(), Span::default()));

    let callee_info = FunctionContextInfo {
        name: "db_query".into(),
        required_contexts: callee_contexts,
        excluded_contexts: List::new(),
        callees: List::new(),
        call_sites: verum_common::Map::new(),
        span: Span::default(),
    };
    checker.register_function(callee_info);

    // Set up caller with negative Database constraint
    let mut caller_contexts = ContextSet::new();
    caller_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    checker.set_required(caller_contexts);

    // Calling db_query should fail because it uses excluded Database
    let result = checker.check_call_negative_constraints("db_query", Span::default());

    match result {
        Err(TypeError::TransitiveNegativeContextViolation { excluded_context, callee, .. }) => {
            assert_eq!(excluded_context, "Database");
            assert_eq!(callee, "db_query");
        }
        _ => panic!("Expected TransitiveNegativeContextViolation"),
    }
}

#[test]
fn test_transitive_verification_deep_call_chain() {
    use verum_common::List;

    let mut checker = ContextChecker::new();

    // Set up call chain: pure -> logger -> db_helper (uses Database)

    // db_helper uses Database
    let mut db_helper_contexts = ContextSet::new();
    db_helper_contexts.add(ContextRequirement::new("Database".to_string(), Span::default()));
    checker.register_function(FunctionContextInfo {
        name: "db_helper".into(),
        required_contexts: db_helper_contexts,
        excluded_contexts: List::new(),
        callees: List::new(),
        call_sites: verum_common::Map::new(),
        span: Span::default(),
    });

    // logger calls db_helper (transitively uses Database)
    let mut logger_contexts = ContextSet::new();
    logger_contexts.add(ContextRequirement::new("Logger".to_string(), Span::default()));
    let mut logger_callees = List::new();
    logger_callees.push("db_helper".into());
    checker.register_function(FunctionContextInfo {
        name: "logger".into(),
        required_contexts: logger_contexts,
        excluded_contexts: List::new(),
        callees: logger_callees,
        call_sites: verum_common::Map::new(),
        span: Span::default(),
    });

    // pure excludes Database and calls logger
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    let mut pure_excluded = List::new();
    pure_excluded.push("Database".into());
    let mut pure_callees = List::new();
    pure_callees.push("logger".into());
    checker.register_function(FunctionContextInfo {
        name: "pure".into(),
        required_contexts: pure_contexts,
        excluded_contexts: pure_excluded,
        callees: pure_callees,
        call_sites: verum_common::Map::new(),
        span: Span::default(),
    });

    // Verify should fail due to transitive Database usage
    let result = checker.verify_transitive_negative_contexts("pure");

    match result {
        Err(TypeError::TransitiveNegativeContextViolation { excluded_context, callee, .. }) => {
            assert_eq!(excluded_context, "Database");
            assert!(callee.contains("db_helper"), "Error should mention db_helper");
        }
        _ => panic!("Expected TransitiveNegativeContextViolation for deep call chain"),
    }
}

#[test]
fn test_transitive_verification_no_violation() {
    use verum_common::List;

    let mut checker = ContextChecker::new();

    // pure_logger only uses Logger, no Database
    let mut logger_contexts = ContextSet::new();
    logger_contexts.add(ContextRequirement::new("Logger".to_string(), Span::default()));
    checker.register_function(FunctionContextInfo {
        name: "pure_logger".into(),
        required_contexts: logger_contexts,
        excluded_contexts: List::new(),
        callees: List::new(),
        call_sites: verum_common::Map::new(),
        span: Span::default(),
    });

    // pure excludes Database but only calls pure_logger (no Database usage)
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    let mut pure_excluded = List::new();
    pure_excluded.push("Database".into());
    let mut pure_callees = List::new();
    pure_callees.push("pure_logger".into());
    checker.register_function(FunctionContextInfo {
        name: "pure".into(),
        required_contexts: pure_contexts,
        excluded_contexts: pure_excluded,
        callees: pure_callees,
        call_sites: verum_common::Map::new(),
        span: Span::default(),
    });

    // Verification should pass
    let result = checker.verify_transitive_negative_contexts("pure");
    assert!(result.is_ok(), "Should pass when callees don't use excluded contexts");
}

// ============================================================================
// Advanced context patterns (negative contexts, call graph verification, module aliases): Module-Level Alias Validation Tests
// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
// ============================================================================

#[test]
fn test_module_alias_registry_basic() {
    let mut registry = ModuleAliasRegistry::new();

    // Register alias in function1
    registry.register_alias("function1", "db", "Database", Span::default());

    // Register same alias for same context in function2 - should be OK
    registry.register_alias("function2", "db", "Database", Span::default());

    // Validate - should pass
    let result = registry.validate_module();
    assert!(result.is_ok(), "Same alias for same context is OK");
}

#[test]
fn test_module_alias_registry_conflict() {
    let mut registry = ModuleAliasRegistry::new();

    // Register alias 'primary' for Database in function1
    registry.register_alias("migrate", "primary", "Database", Span::default());

    // Register alias 'primary' for Cache in function2 - conflict!
    registry.register_alias("verify", "primary", "Cache", Span::default());

    // Validate - should fail
    let result = registry.validate_module();

    match result {
        Err(conflicts) => {
            assert_eq!(conflicts.len(), 1, "Should have one conflict");
            let conflict = &conflicts[0];
            assert_eq!(conflict.alias, "primary");
            assert_eq!(conflict.first_usage.context_path, "Database");
            assert_eq!(conflict.first_usage.function_name, "migrate");
            assert_eq!(conflict.second_usage.context_path, "Cache");
            assert_eq!(conflict.second_usage.function_name, "verify");
        }
        Ok(_) => panic!("Expected conflict error"),
    }
}

#[test]
fn test_module_alias_registry_multiple_conflicts() {
    let mut registry = ModuleAliasRegistry::new();

    // First conflict: 'primary' alias
    registry.register_alias("func1", "primary", "Database", Span::default());
    registry.register_alias("func2", "primary", "Cache", Span::default());

    // Second conflict: 'logger' alias
    registry.register_alias("func3", "logger", "FileLogger", Span::default());
    registry.register_alias("func4", "logger", "ConsoleLogger", Span::default());

    // Validate
    let result = registry.validate_module();

    match result {
        Err(conflicts) => {
            assert_eq!(conflicts.len(), 2, "Should have two conflicts");
        }
        Ok(_) => panic!("Expected conflicts"),
    }
}

#[test]
fn test_module_alias_registry_no_conflicts() {
    let mut registry = ModuleAliasRegistry::new();

    // Different aliases for different contexts - OK
    registry.register_alias("func1", "db", "Database", Span::default());
    registry.register_alias("func2", "cache", "Cache", Span::default());
    registry.register_alias("func3", "logger", "Logger", Span::default());

    // Same context with same alias in multiple functions - OK
    registry.register_alias("func4", "db", "Database", Span::default());

    let result = registry.validate_module();
    assert!(result.is_ok(), "No conflicts should be detected");
}

#[test]
fn test_alias_conflict_to_type_error() {
    let conflicts = vec![AliasConflict {
        alias: "primary".into(),
        first_usage: AliasUsage {
            function_name: "migrate".into(),
            context_path: "Database".into(),
            span: Span::default(),
        },
        second_usage: AliasUsage {
            function_name: "verify".into(),
            context_path: "Cache".into(),
            span: Span::default(),
        },
    }];

    let errors = ModuleAliasRegistry::conflicts_to_type_errors(&conflicts);

    assert_eq!(errors.len(), 1);
    match &errors[0] {
        TypeError::ContextAliasConflict {
            alias,
            first_context,
            first_function,
            second_context,
            second_function,
            ..
        } => {
            assert_eq!(alias, "primary");
            assert_eq!(first_context, "Database");
            assert_eq!(first_function, "migrate");
            assert_eq!(second_context, "Cache");
            assert_eq!(second_function, "verify");
        }
        _ => panic!("Expected ContextAliasConflict"),
    }
}

// ============================================================================
// Build Negative Context Map Tests
// ============================================================================

#[test]
fn test_build_negative_context_map() {
    let mut set = ContextSet::new();
    set.add(ContextRequirement::new("Logger".to_string(), Span::default()));
    set.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    set.add(ContextRequirement::negative("Network".to_string(), Span::default()));

    let map = build_negative_context_map(&set);

    assert_eq!(map.len(), 2, "Should have 2 negative contexts");
    assert!(map.contains_key(&Text::from("Database")));
    assert!(map.contains_key(&Text::from("Network")));
    assert!(!map.contains_key(&Text::from("Logger")));
}

// ============================================================================
// CallGraph and Enhanced Transitive Verification Tests (Advanced context patterns (negative contexts, call graph verification, module aliases) Section 7.3)
// ============================================================================

use verum_types::context_check::{
    CallGraph, CallSiteInfo, CallChainStep, TransitiveViolationInfo, ContextPath,
};
use verum_common::Map;

#[test]
fn test_call_graph_basic() {
    let mut graph = CallGraph::new();

    // Add functions
    let mut db_contexts = ContextSet::new();
    db_contexts.add(ContextRequirement::new("Database".to_string(), Span::default()));
    graph.add_function("db_query", db_contexts, Span::default());

    let mut logger_contexts = ContextSet::new();
    logger_contexts.add(ContextRequirement::new("Logger".to_string(), Span::default()));
    graph.add_function("log_helper", logger_contexts, Span::default());

    graph.add_function("pure_function", ContextSet::new(), Span::default());

    assert!(graph.contains("db_query"));
    assert!(graph.contains("log_helper"));
    assert!(graph.contains("pure_function"));
    assert!(!graph.contains("nonexistent"));
}

#[test]
fn test_call_graph_edges() {
    let mut graph = CallGraph::new();

    // Add functions
    let mut db_contexts = ContextSet::new();
    db_contexts.add(ContextRequirement::new("Database".to_string(), Span::default()));
    graph.add_function("db_helper", db_contexts, Span::default());

    graph.add_function("middle_function", ContextSet::new(), Span::default());
    graph.add_function("top_function", ContextSet::new(), Span::default());

    // Add call edges: top -> middle -> db_helper
    graph.add_call("middle_function", CallSiteInfo::new("db_helper", 42, 5, Span::default()));
    graph.add_call("top_function", CallSiteInfo::new("middle_function", 15, 3, Span::default()));

    // Check callees
    let top_callees = graph.get_callees("top_function");
    assert_eq!(top_callees.len(), 1);
    assert_eq!(top_callees[0].callee_name, "middle_function");
    assert_eq!(top_callees[0].line, 15);

    let middle_callees = graph.get_callees("middle_function");
    assert_eq!(middle_callees.len(), 1);
    assert_eq!(middle_callees[0].callee_name, "db_helper");
    assert_eq!(middle_callees[0].line, 42);
}

#[test]
fn test_call_site_info() {
    let call_site = CallSiteInfo::new("target_function", 100, 25, Span::default());
    assert_eq!(call_site.callee_name, "target_function");
    assert_eq!(call_site.line, 100);
    assert_eq!(call_site.column, 25);
}

#[test]
fn test_context_path_simple() {
    let path = ContextPath::simple("Database");
    assert_eq!(path.as_string(), "Database");
    assert!(path.matches("Database"));
    assert!(!path.matches("Logger"));
}

#[test]
fn test_context_path_multi_segment() {
    let path = ContextPath::from_segments(vec!["FileSystem", "Read"]);
    assert_eq!(path.as_string(), "FileSystem.Read");
    assert!(path.matches("FileSystem.Read"));
    assert!(!path.matches("FileSystem"));
}

#[test]
fn test_transitive_violation_info_format() {
    let mut call_chain = verum_common::List::new();
    call_chain.push(CallChainStep {
        function_name: "helper_function".into(),
        line: 15,
        uses_context: false,
    });
    call_chain.push(CallChainStep {
        function_name: "database_helper".into(),
        line: 42,
        uses_context: true,
    });

    let violation = TransitiveViolationInfo {
        origin_function: "pure_function".into(),
        excluded_context: "Database".into(),
        call_chain,
        declaration_span: Span::default(),
    };

    let error_msg = violation.format_error();

    // Check that the error message matches the expected format
    assert!(error_msg.contains("Function 'pure_function' excludes 'Database'"));
    assert!(error_msg.contains("helper_function() at line 15"));
    assert!(error_msg.contains("database_helper() at line 42"));
    assert!(error_msg.contains("uses Database"));
}

#[test]
fn test_transitive_violation_to_type_error() {
    let mut call_chain = verum_common::List::new();
    call_chain.push(CallChainStep {
        function_name: "middle".into(),
        line: 10,
        uses_context: false,
    });
    call_chain.push(CallChainStep {
        function_name: "db_user".into(),
        line: 20,
        uses_context: true,
    });

    let violation = TransitiveViolationInfo {
        origin_function: "pure".into(),
        excluded_context: "Database".into(),
        call_chain,
        declaration_span: Span::default(),
    };

    let type_error = violation.to_type_error();

    match type_error {
        TypeError::TransitiveNegativeContextViolation { excluded_context, callee, .. } => {
            assert_eq!(excluded_context, "Database");
            assert!(callee.contains("middle() at line 10"));
            assert!(callee.contains("db_user() at line 20"));
        }
        _ => panic!("Expected TransitiveNegativeContextViolation"),
    }
}

#[test]
fn test_verify_transitive_with_call_graph() {
    let mut checker = ContextChecker::new();

    // Set up a call graph with a violation chain
    let mut graph = CallGraph::new();

    // db_helper uses Database
    let mut db_contexts = ContextSet::new();
    db_contexts.add(ContextRequirement::new("Database".to_string(), Span::default()));
    graph.add_function("db_helper", db_contexts, Span::default());

    // middle_function calls db_helper
    graph.add_function("middle_function", ContextSet::new(), Span::default());
    graph.add_call("middle_function", CallSiteInfo::new("db_helper", 42, 5, Span::default()));

    // pure_function excludes Database and calls middle_function
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    graph.add_function("pure_function", pure_contexts.clone(), Span::default());
    graph.add_call("pure_function", CallSiteInfo::new("middle_function", 15, 3, Span::default()));

    // Register the pure function with the checker
    let mut call_sites = Map::new();
    call_sites.insert("middle_function".into(), CallSiteInfo::new("middle_function", 15, 3, Span::default()));

    checker.register_function(FunctionContextInfo {
        name: "pure_function".into(),
        required_contexts: pure_contexts,
        excluded_contexts: {
            let mut list = verum_common::List::new();
            list.push("Database".into());
            list
        },
        callees: {
            let mut list = verum_common::List::new();
            list.push("middle_function".into());
            list
        },
        call_sites,
        span: Span::default(),
    });

    // Verify using call graph - should detect the transitive violation
    let excluded = vec![ContextPath::simple("Database")];
    let result = checker.verify_transitive_negative_contexts_with_graph(
        "pure_function",
        &excluded,
        &graph,
    );

    match result {
        Err(violation) => {
            assert_eq!(violation.origin_function, "pure_function");
            assert_eq!(violation.excluded_context, "Database");
            assert_eq!(violation.call_chain.len(), 2);
            assert_eq!(violation.call_chain[0].function_name, "middle_function");
            assert_eq!(violation.call_chain[1].function_name, "db_helper");
            assert!(violation.call_chain[1].uses_context);
        }
        Ok(_) => panic!("Expected transitive violation to be detected"),
    }
}

#[test]
fn test_verify_transitive_no_violation() {
    let checker = ContextChecker::new();

    // Set up a call graph without violations
    let mut graph = CallGraph::new();

    // log_helper uses Logger (not Database)
    let mut logger_contexts = ContextSet::new();
    logger_contexts.add(ContextRequirement::new("Logger".to_string(), Span::default()));
    graph.add_function("log_helper", logger_contexts, Span::default());

    // pure_function excludes Database, but only calls log_helper (which doesn't use Database)
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    graph.add_function("pure_function", pure_contexts, Span::default());
    graph.add_call("pure_function", CallSiteInfo::new("log_helper", 10, 3, Span::default()));

    // Verify using call graph - should pass
    let excluded = vec![ContextPath::simple("Database")];
    let result = checker.verify_transitive_negative_contexts_with_graph(
        "pure_function",
        &excluded,
        &graph,
    );

    assert!(result.is_ok(), "Should pass when no violations");
}

#[test]
fn test_verify_transitive_with_cycle() {
    let checker = ContextChecker::new();

    // Set up a call graph with a cycle: A -> B -> C -> A
    let mut graph = CallGraph::new();

    graph.add_function("func_a", ContextSet::new(), Span::default());
    graph.add_function("func_b", ContextSet::new(), Span::default());
    graph.add_function("func_c", ContextSet::new(), Span::default());

    graph.add_call("func_a", CallSiteInfo::new("func_b", 10, 1, Span::default()));
    graph.add_call("func_b", CallSiteInfo::new("func_c", 20, 1, Span::default()));
    graph.add_call("func_c", CallSiteInfo::new("func_a", 30, 1, Span::default())); // Cycle!

    // pure_function excludes Database and calls func_a
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    graph.add_function("pure_function", pure_contexts, Span::default());
    graph.add_call("pure_function", CallSiteInfo::new("func_a", 5, 1, Span::default()));

    // Verify - should handle cycle without infinite loop and pass (no Database usage)
    let excluded = vec![ContextPath::simple("Database")];
    let result = checker.verify_transitive_negative_contexts_with_graph(
        "pure_function",
        &excluded,
        &graph,
    );

    assert!(result.is_ok(), "Should handle cycles gracefully");
}

#[test]
fn test_verify_transitive_cycle_with_violation() {
    let checker = ContextChecker::new();

    // Set up a call graph with a cycle where one node uses Database
    let mut graph = CallGraph::new();

    let mut db_contexts = ContextSet::new();
    db_contexts.add(ContextRequirement::new("Database".to_string(), Span::default()));

    graph.add_function("func_a", ContextSet::new(), Span::default());
    graph.add_function("func_b", db_contexts, Span::default()); // Uses Database!
    graph.add_function("func_c", ContextSet::new(), Span::default());

    graph.add_call("func_a", CallSiteInfo::new("func_b", 10, 1, Span::default()));
    graph.add_call("func_b", CallSiteInfo::new("func_c", 20, 1, Span::default()));
    graph.add_call("func_c", CallSiteInfo::new("func_a", 30, 1, Span::default())); // Cycle back

    // pure_function excludes Database and calls func_a
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    graph.add_function("pure_function", pure_contexts, Span::default());
    graph.add_call("pure_function", CallSiteInfo::new("func_a", 5, 1, Span::default()));

    // Verify - should detect violation in cycle
    let excluded = vec![ContextPath::simple("Database")];
    let result = checker.verify_transitive_negative_contexts_with_graph(
        "pure_function",
        &excluded,
        &graph,
    );

    match result {
        Err(violation) => {
            assert_eq!(violation.excluded_context, "Database");
            // Should find func_b which uses Database
            assert!(violation.call_chain.iter().any(|step| step.function_name == "func_b"));
        }
        Ok(_) => panic!("Expected violation to be detected in cycle"),
    }
}

#[test]
fn test_build_call_graph_from_registry() {
    use verum_common::List;

    let mut checker = ContextChecker::new();

    // Register functions with callees
    let mut db_contexts = ContextSet::new();
    db_contexts.add(ContextRequirement::new("Database".to_string(), Span::default()));

    checker.register_function(FunctionContextInfo {
        name: "db_helper".into(),
        required_contexts: db_contexts.clone(),
        excluded_contexts: List::new(),
        callees: List::new(),
        call_sites: Map::new(),
        span: Span::default(),
    });

    let mut middle_callees = List::new();
    middle_callees.push("db_helper".into());
    let mut middle_call_sites = Map::new();
    middle_call_sites.insert("db_helper".into(), CallSiteInfo::new("db_helper", 42, 1, Span::default()));

    checker.register_function(FunctionContextInfo {
        name: "middle".into(),
        required_contexts: ContextSet::new(),
        excluded_contexts: List::new(),
        callees: middle_callees,
        call_sites: middle_call_sites,
        span: Span::default(),
    });

    // Build call graph from registry
    let graph = checker.build_call_graph_from_registry();

    assert!(graph.contains("db_helper"));
    assert!(graph.contains("middle"));

    let callees = graph.get_callees("middle");
    assert_eq!(callees.len(), 1);
    assert_eq!(callees[0].callee_name, "db_helper");
    assert_eq!(callees[0].line, 42);
}

#[test]
fn test_verify_all_negative_contexts() {
    use verum_common::List;

    let mut checker = ContextChecker::new();

    // Register a function with no violations
    let mut logger_contexts = ContextSet::new();
    logger_contexts.add(ContextRequirement::new("Logger".to_string(), Span::default()));

    checker.register_function(FunctionContextInfo {
        name: "log_helper".into(),
        required_contexts: logger_contexts,
        excluded_contexts: List::new(),
        callees: List::new(),
        call_sites: Map::new(),
        span: Span::default(),
    });

    // Register a pure function that excludes Database
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));

    let mut pure_callees = List::new();
    pure_callees.push("log_helper".into());

    checker.register_function(FunctionContextInfo {
        name: "pure".into(),
        required_contexts: pure_contexts,
        excluded_contexts: {
            let mut list = List::new();
            list.push("Database".into());
            list
        },
        callees: pure_callees,
        call_sites: Map::new(),
        span: Span::default(),
    });

    // Verify all - should pass (log_helper doesn't use Database)
    let result = checker.verify_all_negative_contexts();
    assert!(result.is_ok());
}

#[test]
fn test_multiple_excluded_contexts() {
    let checker = ContextChecker::new();

    // Set up call graph where a function uses one of multiple excluded contexts
    let mut graph = CallGraph::new();

    // network_helper uses Network
    let mut network_contexts = ContextSet::new();
    network_contexts.add(ContextRequirement::new("Network".to_string(), Span::default()));
    graph.add_function("network_helper", network_contexts, Span::default());

    // pure_function excludes both Database and Network
    let mut pure_contexts = ContextSet::new();
    pure_contexts.add(ContextRequirement::negative("Database".to_string(), Span::default()));
    pure_contexts.add(ContextRequirement::negative("Network".to_string(), Span::default()));
    graph.add_function("pure_function", pure_contexts, Span::default());
    graph.add_call("pure_function", CallSiteInfo::new("network_helper", 15, 1, Span::default()));

    // Verify - should detect Network violation
    let excluded = vec![
        ContextPath::simple("Database"),
        ContextPath::simple("Network"),
    ];
    let result = checker.verify_transitive_negative_contexts_with_graph(
        "pure_function",
        &excluded,
        &graph,
    );

    match result {
        Err(violation) => {
            assert_eq!(violation.excluded_context, "Network");
        }
        Ok(_) => panic!("Expected Network violation"),
    }
}
