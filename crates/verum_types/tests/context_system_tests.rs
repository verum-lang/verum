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
//! Comprehensive tests for the Context System (Dependency Injection)
//!
//! Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Context System (Capability-based Dependency Injection)
//!
//! This test suite validates all aspects of the Context System implementation:
//! - Context declarations
//! - Context requirements
//! - Context providers
//! - Context environment (θ)
//! - Context groups
//! - Performance characteristics

use std::any::TypeId;
use std::sync::Arc;
use verum_common::{Maybe, Text};
use verum_types::di::*;
use verum_types::ty::Type;

// ============================================================================
// Context Declaration Tests
// ============================================================================

#[test]
fn test_context_decl_basic() {
    let mut logger = ContextDecl::new("Logger".into());
    logger.add_operation(ContextOperation::new(
        Text::from("log"),
        vec![
            (Text::from("level"), Type::Int),
            (Text::from("message"), Type::Text),
        ],
        Type::Unit,
        false,
    ));

    assert_eq!(logger.name, "Logger");
    assert_eq!(logger.operation_count(), 1);
    assert!(logger.has_operation("log"));
    assert!(!logger.is_async);
    assert!(logger.validate().is_ok());
}

#[test]
fn test_context_decl_async() {
    let mut db = ContextDecl::new("Database".into());
    db.add_operation(ContextOperation::new(
        Text::from("query"),
        vec![(Text::from("sql"), Type::Text)],
        Type::Text,
        true, // async operation
    ));

    assert!(db.is_async);
    assert!(db.has_operation("query"));

    if let Maybe::Some(op) = db.get_operation("query") {
        assert!(op.is_async);
        assert_eq!(op.param_count(), 1);
    } else {
        panic!("Operation 'query' not found");
    }
}

#[test]
fn test_context_decl_parameterized() {
    let state = ContextDecl::with_type_params("State".into(), vec![TypeParam::new("S".into())]);

    assert_eq!(state.qualified_name(), "State<S>");
    assert_eq!(state.type_params.len(), 1);
}

#[test]
fn test_context_decl_validation_empty() {
    let empty = ContextDecl::new("Empty".into());
    assert!(matches!(
        empty.validate(),
        Err(ContextError::EmptyContext(_))
    ));
}

#[test]
fn test_context_decl_validation_duplicate() {
    let mut ctx = ContextDecl::new("Test".into());
    ctx.add_operation(ContextOperation::new(
        Text::from("foo"),
        vec![],
        Type::Unit,
        false,
    ));
    ctx.add_operation(ContextOperation::new(
        Text::from("foo"),
        vec![],
        Type::Unit,
        false,
    ));

    assert!(matches!(
        ctx.validate(),
        Err(ContextError::DuplicateOperation { .. })
    ));
}

#[test]
fn test_context_decl_multiple_operations() {
    let mut logger = ContextDecl::new("Logger".into());

    logger.add_operation(ContextOperation::new(
        Text::from("info"),
        vec![(Text::from("message"), Type::Text)],
        Type::Unit,
        false,
    ));

    logger.add_operation(ContextOperation::new(
        Text::from("error"),
        vec![(Text::from("message"), Type::Text)],
        Type::Unit,
        false,
    ));

    logger.add_operation(ContextOperation::new(
        Text::from("debug"),
        vec![(Text::from("message"), Type::Text)],
        Type::Unit,
        false,
    ));

    assert_eq!(logger.operation_count(), 3);
    assert!(logger.has_operation("info"));
    assert!(logger.has_operation("error"));
    assert!(logger.has_operation("debug"));
    assert!(!logger.has_operation("warn"));
}

// ============================================================================
// Context Requirement Tests
// ============================================================================

#[test]
fn test_requirement_empty() {
    let req = ContextRequirement::empty();
    assert!(req.is_empty());
    assert_eq!(req.len(), 0);
}

#[test]
fn test_requirement_single() {
    let logger_ref = ContextRef::new("Logger".into(), TypeId::of::<String>());
    let req = ContextRequirement::single(logger_ref);

    assert!(!req.is_empty());
    assert_eq!(req.len(), 1);
    assert!(req.requires("Logger"));
    assert!(!req.requires("Database"));
}

#[test]
fn test_requirement_multiple() {
    let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let database = ContextRef::new("Database".into(), TypeId::of::<String>());
    let auth = ContextRef::new("Auth".into(), TypeId::of::<i32>());

    let req = ContextRequirement::from_contexts(vec![logger, database, auth]);

    assert_eq!(req.len(), 3);
    assert!(req.requires("Logger"));
    assert!(req.requires("Database"));
    assert!(req.requires("Auth"));
    assert!(!req.requires("Metrics"));
}

#[test]
fn test_requirement_merge() {
    let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let database = ContextRef::new("Database".into(), TypeId::of::<String>());
    let auth = ContextRef::new("Auth".into(), TypeId::of::<i32>());

    let req1 = ContextRequirement::from_contexts(vec![logger, database]);
    let req2 = ContextRequirement::single(auth);

    let merged = req1.merge(&req2);

    assert_eq!(merged.len(), 3);
    assert!(merged.requires("Logger"));
    assert!(merged.requires("Database"));
    assert!(merged.requires("Auth"));
}

#[test]
fn test_requirement_subset() {
    let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let database = ContextRef::new("Database".into(), TypeId::of::<String>());

    let small = ContextRequirement::single(logger.clone());
    let large = ContextRequirement::from_contexts(vec![logger, database]);

    assert!(small.is_subset_of(&large));
    assert!(!large.is_subset_of(&small));
}

#[test]
fn test_requirement_add_remove() {
    let mut req = ContextRequirement::empty();

    let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
    req.add_context(logger);

    assert_eq!(req.len(), 1);
    assert!(req.requires("Logger"));

    assert!(req.remove_context("Logger"));
    assert!(req.is_empty());
    assert!(!req.remove_context("Logger")); // Already removed
}

#[test]
fn test_requirement_async() {
    let db_ref = ContextRef::new("Database".into(), TypeId::of::<()>()).as_async();
    let req = ContextRequirement::single(db_ref);

    assert!(req.has_async_contexts());
}

#[test]
fn test_context_ref_qualified_name() {
    let simple = ContextRef::new("Logger".into(), TypeId::of::<()>());
    assert_eq!(simple.qualified_name(), "Logger");

    let parameterized =
        ContextRef::with_type_args("State".into(), TypeId::of::<()>(), vec!["Int".into()]);
    assert_eq!(parameterized.qualified_name(), "State<Int>");

    let complex = ContextRef::with_type_args(
        "Cache".into(),
        TypeId::of::<()>(),
        vec!["Text".into(), "User".into()],
    );
    assert_eq!(complex.qualified_name(), "Cache<Text, User>");
}

// ============================================================================
// Context Provider Tests
// ============================================================================

#[test]
fn test_provider_local() {
    let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let provider = ContextProvider::new(ctx_ref, "console_logger()".into(), TypeId::of::<String>());

    assert!(provider.is_local());
    assert!(!provider.is_module());
    assert!(!provider.is_global());
    assert_eq!(provider.context_name(), "Logger");
}

#[test]
fn test_provider_module_scope() {
    let ctx_ref = ContextRef::new("Database".into(), TypeId::of::<()>());
    let provider = ContextProvider::with_scope(
        ctx_ref,
        "postgres_connection()".into(),
        TypeId::of::<String>(),
        ProviderScope::Module,
    );

    assert!(!provider.is_local());
    assert!(provider.is_module());
    assert!(!provider.is_global());
}

#[test]
fn test_provider_global_scope() {
    let ctx_ref = ContextRef::new("Runtime".into(), TypeId::of::<()>());
    let provider = ContextProvider::with_scope(
        ctx_ref,
        "VerumNativeRuntime.new()".into(),
        TypeId::of::<String>(),
        ProviderScope::Global,
    );

    assert!(provider.is_global());
}

#[test]
fn test_provider_async() {
    let ctx_ref = ContextRef::new("Database".into(), TypeId::of::<()>()).as_async();
    let provider = ContextProvider::new(
        ctx_ref,
        "Database.connect(url)".into(),
        TypeId::of::<String>(),
    )
    .as_async();

    assert!(provider.is_async);
    assert!(provider.validate().is_ok());
}

#[test]
fn test_provider_validation_empty() {
    let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let provider = ContextProvider::new(ctx_ref, "".into(), TypeId::of::<String>());

    assert!(matches!(
        provider.validate(),
        Err(ProviderError::EmptyExpression(_))
    ));
}

#[test]
fn test_provider_validation_async_mismatch() {
    let ctx_ref = ContextRef::new("Database".into(), TypeId::of::<()>()).as_async();
    let provider = ContextProvider::new(ctx_ref, "sync_provider()".into(), TypeId::of::<String>());

    assert!(matches!(
        provider.validate(),
        Err(ProviderError::AsyncMismatch { .. })
    ));
}

#[test]
fn test_provider_set_scope() {
    let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let mut provider =
        ContextProvider::new(ctx_ref, "console_logger()".into(), TypeId::of::<String>());

    assert!(provider.is_local());

    provider.set_scope(ProviderScope::Module);
    assert!(provider.is_module());

    provider.set_scope(ProviderScope::Global);
    assert!(provider.is_global());
}

// ============================================================================
// Context Environment Tests
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
struct TestLogger {
    name: String,
}

#[derive(Debug, Clone, PartialEq)]
struct TestDatabase {
    url: String,
}

#[derive(Debug, Clone, PartialEq)]
struct TestAuth {
    token: String,
}

#[test]
fn test_env_new() {
    let env = ContextEnv::new();
    assert!(env.is_empty());
    assert_eq!(env.len(), 0);
    assert_eq!(env.depth(), 0);
}

#[test]
fn test_env_insert_get() {
    let mut env = ContextEnv::new();
    let logger = TestLogger {
        name: "console".to_string(),
    };

    env.insert(logger.clone());

    if let Maybe::Some(retrieved) = env.get::<TestLogger>() {
        assert_eq!(retrieved.name, "console");
    } else {
        panic!("Logger not found");
    }
}

#[test]
fn test_env_multiple_contexts() {
    let mut env = ContextEnv::new();

    env.insert(TestLogger {
        name: "logger".to_string(),
    });
    env.insert(TestDatabase {
        url: "postgres://localhost".to_string(),
    });
    env.insert(TestAuth {
        token: "abc123".to_string(),
    });

    assert_eq!(env.len(), 3);

    assert!(matches!(env.get::<TestLogger>(), Maybe::Some(_)));
    assert!(matches!(env.get::<TestDatabase>(), Maybe::Some(_)));
    assert!(matches!(env.get::<TestAuth>(), Maybe::Some(_)));
}

#[test]
fn test_env_get_mut() {
    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "original".to_string(),
    });

    if let Maybe::Some(logger) = env.get_mut::<TestLogger>() {
        logger.name = "modified".to_string();
    }

    if let Maybe::Some(logger) = env.get::<TestLogger>() {
        assert_eq!(logger.name, "modified");
    }
}

#[test]
fn test_env_parent_chain() {
    let mut parent = ContextEnv::new();
    parent.insert(TestLogger {
        name: "parent_logger".to_string(),
    });

    let mut child = ContextEnv::with_parent(Arc::new(parent));
    child.insert(TestDatabase {
        url: "postgres://child".to_string(),
    });

    assert_eq!(child.len(), 1); // Local only
    assert_eq!(child.total_len(), 2); // Including parent
    assert_eq!(child.depth(), 1);

    // Can access parent's context

    assert!(matches!(
        child.get_or_parent::<TestLogger>(),
        Maybe::Some(_)
    ));
    // And own context
    assert!(matches!(child.get::<TestDatabase>(), Maybe::Some(_)));
}

#[test]
fn test_env_parent_override() {
    let mut parent = ContextEnv::new();
    parent.insert(TestLogger {
        name: "parent_logger".to_string(),
    });

    let mut child = ContextEnv::with_parent(Arc::new(parent));
    child.insert(TestLogger {
        name: "child_logger".to_string(),
    });

    // Child's logger shadows parent's
    if let Maybe::Some(logger) = child.get_or_parent::<TestLogger>() {
        assert_eq!(logger.name, "child_logger");
    }
}

#[test]
fn test_env_deep_parent_chain() {
    let mut level1 = ContextEnv::new();
    level1.insert(TestLogger {
        name: "level1".to_string(),
    });

    let mut level2 = ContextEnv::with_parent(Arc::new(level1));
    level2.insert(TestDatabase {
        url: "level2".to_string(),
    });

    let mut level3 = ContextEnv::with_parent(Arc::new(level2));
    level3.insert(TestAuth {
        token: "level3".to_string(),
    });

    assert_eq!(level3.depth(), 2);
    assert_eq!(level3.total_len(), 3);

    // Can access all levels

    assert!(matches!(
        level3.get_or_parent::<TestLogger>(),
        Maybe::Some(_)
    ));
    assert!(matches!(
        level3.get_or_parent::<TestDatabase>(),
        Maybe::Some(_)
    ));
    assert!(matches!(level3.get_or_parent::<TestAuth>(), Maybe::Some(_)));
}

#[test]
fn test_env_remove() {
    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "test".to_string(),
    });

    assert!(matches!(env.get::<TestLogger>(), Maybe::Some(_)));

    let removed = env.remove::<TestLogger>();
    assert!(matches!(removed, Maybe::Some(_)));
    assert!(matches!(env.get::<TestLogger>(), Maybe::None));
}

#[test]
fn test_env_clear() {
    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "test".to_string(),
    });
    env.insert(TestDatabase {
        url: "test".to_string(),
    });

    assert_eq!(env.len(), 2);

    env.clear();

    assert!(env.is_empty());
    assert_eq!(env.len(), 0);
}

#[test]
fn test_env_has_context() {
    let mut parent = ContextEnv::new();
    parent.insert(TestLogger {
        name: "parent".to_string(),
    });

    let child = ContextEnv::with_parent(Arc::new(parent));

    let logger_id = TypeId::of::<TestLogger>();
    let db_id = TypeId::of::<TestDatabase>();

    assert!(child.has_context(logger_id));
    assert!(!child.has_context(db_id));
}

#[test]
fn test_env_create_child() {
    let mut parent = ContextEnv::new();
    parent.insert(TestLogger {
        name: "parent".to_string(),
    });

    let child = parent.create_child();

    assert_eq!(child.depth(), 1);

    assert!(matches!(
        child.get_or_parent::<TestLogger>(),
        Maybe::Some(_)
    ));
}

#[test]
fn test_env_requirement_satisfies() {
    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "logger".to_string(),
    });
    env.insert(TestDatabase {
        url: "db".to_string(),
    });

    let logger = ContextRef::new("Logger".into(), TypeId::of::<TestLogger>());
    let database = ContextRef::new("Database".into(), TypeId::of::<TestDatabase>());
    let auth = ContextRef::new("Auth".into(), TypeId::of::<TestAuth>());

    let req1 = ContextRequirement::from_contexts(vec![logger.clone(), database.clone()]);
    assert!(req1.satisfies(&env));

    let req2 = ContextRequirement::from_contexts(vec![logger, database, auth]);
    assert!(!req2.satisfies(&env)); // Auth missing
}

#[test]
fn test_env_missing_contexts() {
    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "logger".to_string(),
    });

    let logger = ContextRef::new("Logger".into(), TypeId::of::<TestLogger>());
    let database = ContextRef::new("Database".into(), TypeId::of::<TestDatabase>());

    let req = ContextRequirement::from_contexts(vec![logger, database]);

    let missing = req.missing_contexts(&env);
    assert_eq!(missing.len(), 1);
    assert!(missing.iter().any(|n| n.as_str() == "Database"));
}

// ============================================================================
// Context Group Tests
// ============================================================================

fn make_context_ref(name: &str, type_id: TypeId) -> ContextRef {
    ContextRef::new(name.into(), type_id)
}

#[test]
fn test_group_create() {
    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let database = make_context_ref("Database", TypeId::of::<String>());
    let auth = make_context_ref("Auth", TypeId::of::<i32>());

    let group = ContextGroup::new("WebContext".into(), vec![logger, database, auth]);

    assert_eq!(group.name, "WebContext");
    assert_eq!(group.len(), 3);
    assert!(group.contains("Logger"));
    assert!(group.contains("Database"));
    assert!(group.contains("Auth"));
    assert!(!group.contains("Metrics"));
}

#[test]
fn test_group_empty() {
    let group = ContextGroup::empty("EmptyGroup".into());
    assert!(group.is_empty());
    assert_eq!(group.len(), 0);
}

#[test]
fn test_group_add_context() {
    let mut group = ContextGroup::empty("TestGroup".into());

    group.add_context(make_context_ref("Logger", TypeId::of::<()>()));
    assert_eq!(group.len(), 1);

    group.add_context(make_context_ref("Database", TypeId::of::<String>()));
    assert_eq!(group.len(), 2);
}

#[test]
fn test_group_expand() {
    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let database = make_context_ref("Database", TypeId::of::<String>());
    let auth = make_context_ref("Auth", TypeId::of::<i32>());

    let group = ContextGroup::new("WebContext".into(), vec![logger, database, auth]);
    let requirement = group.expand();

    assert_eq!(requirement.len(), 3);
    assert!(requirement.requires("Logger"));
    assert!(requirement.requires("Database"));
    assert!(requirement.requires("Auth"));
}

#[test]
fn test_group_validate_empty() {
    let group = ContextGroup::empty("Empty".into());
    assert!(matches!(group.validate(), Err(GroupError::EmptyGroup(_))));
}

#[test]
fn test_group_validate_duplicates() {
    let logger1 = make_context_ref("Logger", TypeId::of::<()>());
    let logger2 = make_context_ref("Logger", TypeId::of::<()>());

    let group = ContextGroup::new("Duplicate".into(), vec![logger1, logger2]);

    assert!(matches!(
        group.validate(),
        Err(GroupError::DuplicateContext { .. })
    ));
}

#[test]
fn test_group_validate_success() {
    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let database = make_context_ref("Database", TypeId::of::<String>());

    let group = ContextGroup::new("Valid".into(), vec![logger, database]);
    assert!(group.validate().is_ok());
}

#[test]
fn test_group_merge() {
    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let database = make_context_ref("Database", TypeId::of::<String>());
    let auth = make_context_ref("Auth", TypeId::of::<i32>());
    let metrics = make_context_ref("Metrics", TypeId::of::<u64>());

    let group1 = ContextGroup::new("Group1".into(), vec![logger, database]);
    let group2 = ContextGroup::new("Group2".into(), vec![auth, metrics]);

    let merged = group1.merge(&group2, "Merged".into());

    assert_eq!(merged.name, "Merged");
    assert_eq!(merged.len(), 4);
    assert!(merged.contains("Logger"));
    assert!(merged.contains("Database"));
    assert!(merged.contains("Auth"));
    assert!(merged.contains("Metrics"));
}

#[test]
fn test_group_merge_removes_duplicates() {
    let logger1 = make_context_ref("Logger", TypeId::of::<()>());
    let logger2 = make_context_ref("Logger", TypeId::of::<()>());
    let database = make_context_ref("Database", TypeId::of::<String>());

    let group1 = ContextGroup::new("Group1".into(), vec![logger1, database]);
    let group2 = ContextGroup::new("Group2".into(), vec![logger2]);

    let merged = group1.merge(&group2, "Merged".into());

    // Should only have 2 contexts (Logger and Database), not 3
    assert_eq!(merged.len(), 2);
    assert!(merged.contains("Logger"));
    assert!(merged.contains("Database"));
}

// ============================================================================
// Context Group Registry Tests
// ============================================================================

#[test]
fn test_registry_new() {
    let registry = ContextGroupRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

#[test]
fn test_registry_register() {
    let mut registry = ContextGroupRegistry::new();

    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let group = ContextGroup::new("WebContext".into(), vec![logger]);

    assert!(registry.register(group).is_ok());
    assert_eq!(registry.len(), 1);
    assert!(registry.has_group("WebContext"));
}

#[test]
fn test_registry_duplicate() {
    let mut registry = ContextGroupRegistry::new();

    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let group1 = ContextGroup::new("WebContext".into(), vec![logger.clone()]);
    let group2 = ContextGroup::new("WebContext".into(), vec![logger]);

    assert!(registry.register(group1).is_ok());
    assert!(matches!(
        registry.register(group2),
        Err(GroupError::AlreadyDefined(_))
    ));
}

#[test]
fn test_registry_get() {
    let mut registry = ContextGroupRegistry::new();

    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let group = ContextGroup::new("WebContext".into(), vec![logger]);

    registry.register(group).unwrap();

    assert!(matches!(registry.get("WebContext"), Maybe::Some(_)));
    assert!(matches!(registry.get("MissingContext"), Maybe::None));
}

#[test]
fn test_registry_expand() {
    let mut registry = ContextGroupRegistry::new();

    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let database = make_context_ref("Database", TypeId::of::<String>());
    let group = ContextGroup::new("WebContext".into(), vec![logger, database]);

    registry.register(group).unwrap();

    let requirement = registry.expand("WebContext").unwrap();
    assert_eq!(requirement.len(), 2);
    assert!(requirement.requires("Logger"));
    assert!(requirement.requires("Database"));
}

#[test]
fn test_registry_expand_not_found() {
    let registry = ContextGroupRegistry::new();
    let result = registry.expand("NonExistent");
    assert!(matches!(result, Err(GroupError::NotFound(_))));
}

#[test]
fn test_registry_multiple_groups() {
    let mut registry = ContextGroupRegistry::new();

    let logger = make_context_ref("Logger", TypeId::of::<()>());
    let database = make_context_ref("Database", TypeId::of::<String>());
    let auth = make_context_ref("Auth", TypeId::of::<i32>());
    let metrics = make_context_ref("Metrics", TypeId::of::<u64>());

    let web = ContextGroup::new("WebContext".into(), vec![logger, database]);
    let observability = ContextGroup::new("Observability".into(), vec![auth, metrics]);

    registry.register(web).unwrap();
    registry.register(observability).unwrap();

    assert_eq!(registry.len(), 2);
    assert!(registry.has_group("WebContext"));
    assert!(registry.has_group("Observability"));

    let names = registry.group_names();
    assert_eq!(names.len(), 2);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_integration_complete_workflow() {
    // 1. Declare contexts
    let mut logger_ctx = ContextDecl::new("Logger".into());
    logger_ctx.add_operation(ContextOperation::new(
        Text::from("log"),
        vec![
            (Text::from("level"), Type::Int),
            (Text::from("message"), Type::Text),
        ],
        Type::Unit,
        false,
    ));

    let mut db_ctx = ContextDecl::new("Database".into());
    db_ctx.add_operation(ContextOperation::new(
        Text::from("query"),
        vec![(Text::from("sql"), Type::Text)],
        Type::Text,
        true, // async
    ));

    assert!(logger_ctx.validate().is_ok());
    assert!(db_ctx.validate().is_ok());

    // 2. Create context requirements
    let logger_ref = ContextRef::new("Logger".into(), TypeId::of::<TestLogger>());
    let db_ref = ContextRef::new("Database".into(), TypeId::of::<TestDatabase>()).as_async();

    let requirement = ContextRequirement::from_contexts(vec![logger_ref.clone(), db_ref.clone()]);

    assert_eq!(requirement.len(), 2);
    assert!(requirement.has_async_contexts());

    // 3. Create providers
    let logger_provider = ContextProvider::new(
        logger_ref,
        "console_logger()".into(),
        TypeId::of::<TestLogger>(),
    );

    let db_provider = ContextProvider::new(
        db_ref,
        "postgres_connection()".into(),
        TypeId::of::<TestDatabase>(),
    )
    .as_async();

    assert!(logger_provider.validate().is_ok());
    assert!(db_provider.validate().is_ok());

    // 4. Set up runtime environment
    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "ConsoleLogger".to_string(),
    });
    env.insert(TestDatabase {
        url: "postgres://localhost".to_string(),
    });

    // 5. Verify requirement is satisfied
    assert!(requirement.satisfies(&env));
    assert_eq!(requirement.missing_contexts(&env).len(), 0);
}

#[test]
fn test_integration_context_groups() {
    // Create a registry
    let mut registry = ContextGroupRegistry::new();

    // Define WebContext group
    let logger = ContextRef::new("Logger".into(), TypeId::of::<TestLogger>());
    let database = ContextRef::new("Database".into(), TypeId::of::<TestDatabase>());
    let auth = ContextRef::new("Auth".into(), TypeId::of::<TestAuth>());

    let web_context = ContextGroup::new("WebContext".into(), vec![logger, database, auth]);
    registry.register(web_context).unwrap();

    // Use the group
    let requirement = registry.expand("WebContext").unwrap();
    assert_eq!(requirement.len(), 3);

    // Set up environment
    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "logger".to_string(),
    });
    env.insert(TestDatabase {
        url: "db".to_string(),
    });
    env.insert(TestAuth {
        token: "token".to_string(),
    });

    // Verify all contexts are available
    assert!(requirement.satisfies(&env));
}

#[test]
fn test_integration_lexical_scoping() {
    // Global scope
    let mut global = ContextEnv::new();
    global.insert(TestLogger {
        name: "GlobalLogger".to_string(),
    });

    // Module scope
    let mut module = ContextEnv::with_parent(Arc::new(global));
    module.insert(TestDatabase {
        url: "ModuleDB".to_string(),
    });

    // Function scope
    let mut function = ContextEnv::with_parent(Arc::new(module));
    function.insert(TestAuth {
        token: "FunctionAuth".to_string(),
    });

    // Verify all scopes are accessible
    assert_eq!(function.depth(), 2);
    assert_eq!(function.total_len(), 3);

    assert!(matches!(
        function.get_or_parent::<TestLogger>(),
        Maybe::Some(_)
    ));
    assert!(matches!(
        function.get_or_parent::<TestDatabase>(),
        Maybe::Some(_)
    ));
    assert!(matches!(
        function.get_or_parent::<TestAuth>(),
        Maybe::Some(_)
    ));
}

// ============================================================================
// Performance Benchmarks
// ============================================================================

#[test]
fn test_performance_context_lookup() {
    use std::time::Instant;

    let mut env = ContextEnv::new();
    env.insert(TestLogger {
        name: "logger".to_string(),
    });

    // Warmup
    for _ in 0..1000 {
        let _ = env.get::<TestLogger>();
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();

    for _ in 0..iterations {
        let _ = env.get::<TestLogger>();
    }

    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    // Spec requirement: < 50ns lookup in release mode
    // Debug mode is typically 3-5x slower, so we allow up to 500ns
    println!("Average context lookup time: {}ns", avg_ns);
    assert!(
        avg_ns < 500,
        "Context lookup too slow: {}ns (target: <500ns in debug mode)",
        avg_ns
    );
}

#[test]
fn test_performance_parent_chain_lookup() {
    use std::time::Instant;

    // Create a 3-level parent chain
    let mut level1 = ContextEnv::new();
    level1.insert(TestLogger {
        name: "logger".to_string(),
    });

    let level2 = ContextEnv::with_parent(Arc::new(level1));
    let level3 = ContextEnv::with_parent(Arc::new(level2));

    // Warmup
    for _ in 0..1000 {
        let _ = level3.get_or_parent::<TestLogger>();
    }

    // Benchmark
    let iterations = 100_000;
    let start = Instant::now();

    for _ in 0..iterations {
        let _ = level3.get_or_parent::<TestLogger>();
    }

    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() / iterations;

    println!("Average parent chain lookup time: {}ns", avg_ns);
    // With 2 parent levels, should still be < 100ns in release mode
    // Debug mode allows up to 750ns
    assert!(
        avg_ns < 750,
        "Parent chain lookup too slow: {}ns (target: <750ns in debug mode)",
        avg_ns
    );
}
