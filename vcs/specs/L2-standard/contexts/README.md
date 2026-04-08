# L2-Standard: Context System Tests

This directory contains comprehensive tests for Verum's context system, covering both
Level 1 (Static DI with @injectable) and Level 2 (Dynamic contexts with provide/using).

## Directory Structure

```
contexts/
├── level1_di/          # Static Dependency Injection (@injectable, @inject)
├── level2_dynamic/     # Dynamic Contexts (context, provide, using)
├── composition/        # Combining DI with dynamic contexts
└── errors/             # Error cases and diagnostics
```

## Error Codes (E8XX)

| Code | Error | Description |
|------|-------|-------------|
| E801 | Context not provided | Runtime: accessing a context that was never provided |
| E802 | Scope violation | Compile-time: longer-lived scope depends on shorter-lived |
| E803 | Async context mismatch | Using async context in sync function or vice versa |
| E804 | Context type mismatch | Implementation doesn't match context declaration |
| E805 | Circular dependency (DI) | DI graph contains a cycle (injectable services) |
| E806 | Missing injectable dependency | Type not marked @injectable but used as dependency |
| E807 | Context reference escapes scope | Context reference used outside provide block |
| E808 | Duplicate provide | Same context provided multiple times in same scope |
| E809 | Circular context dependency | Context initialization graph contains a cycle |
| E810 | Context type conflict | Incompatible context requirements in call chain |
| E811 | Invalid context group | Context group references undefined context |

## Test Categories

### level1_di/ - Static Dependency Injection

Tests for compile-time dependency injection using `@injectable` and `@inject` attributes.

| Test File | Type | Description |
|-----------|------|-------------|
| `injectable_singleton.vr` | run | Singleton scope - one instance per application |
| `injectable_request.vr` | run | Request scope - one instance per request |
| `injectable_transient.vr` | run | Transient scope - new instance every injection |
| `inject_constructor.vr` | run | Constructor injection with dependencies |
| `scope_compatibility.vr` | typecheck-pass | Valid scope dependency rules |
| `circular_dependency.vr` | typecheck-fail | E805: Circular dependency detection |
| `missing_dependency.vr` | typecheck-fail | E806: Missing @injectable attribute |

### level2_dynamic/ - Dynamic Contexts

Tests for runtime dependency injection using `context`, `provide`, and `using`.

| Test File | Type | Description |
|-----------|------|-------------|
| `context_declaration.vr` | typecheck-pass | Declaring context traits (sync and async) |
| `provide_statement.vr` | run | Binding implementations to contexts |
| `using_clause.vr` | typecheck-pass | Function signatures with context requirements |
| `context_groups.vr` | typecheck-pass | Grouping multiple contexts together |
| `nested_contexts.vr` | run | Lexical scoping of context provides |
| `context_override.vr` | run | Overriding contexts in inner scopes |
| `missing_context.vr` | run-panic | E801: Runtime panic for unprovided context |

### composition/ - DI with Contexts

Tests for combining static DI and dynamic contexts.

| Test File | Type | Description |
|-----------|------|-------------|
| `di_with_contexts.vr` | run | Injectable services using dynamic contexts |
| `context_in_async.vr` | run | Contexts in async functions with concurrency |
| `context_propagation.vr` | run | Automatic context propagation through call chains |
| `context_in_spawn.vr` | run | Context capture in spawned tasks |
| `context_conflict.vr` | typecheck-fail | E808: Same context provided multiple times |
| `context_resolution.vr` | run | Explicit naming to resolve context ambiguity |

### errors/ - Error Cases

Tests for error detection and diagnostics.

| Test File | Type | Description |
|-----------|------|-------------|
| `context_not_provided.vr` | run-panic | E801: Runtime panic for missing context |
| `scope_violation.vr` | typecheck-fail | E802: Singleton depending on Request |
| `async_context_mismatch.vr` | typecheck-fail | E803: Async context in sync function |
| `context_type_mismatch.vr` | typecheck-fail | E804: Provided type doesn't implement context |
| `context_scope_error.vr` | typecheck-fail | E807: Context reference escapes scope |
| `context_circular_dependency.vr` | typecheck-fail | E809: Circular context dependencies |
| `context_type_conflict.vr` | typecheck-fail | E810: Incompatible context requirements |
| `invalid_context_group.vr` | typecheck-fail | E811: Invalid or undefined context in group |

## Scope Compatibility Rules

The DI system enforces scope compatibility to prevent dangling references:

```
Scope Lifetime (longest to shortest):
1. Singleton - application lifetime
2. Request   - single request/session lifetime
3. Transient - immediate use, no storage

Dependency Rules:
- Singleton can depend on: Singleton only
- Request can depend on: Singleton, Request
- Transient can depend on: Singleton, Request, Transient
```

## Context System Overview

### Level 1: Static DI (@injectable)

```verum
@injectable(Scope.Singleton)
type DatabaseService is { ... }

implement DatabaseService {
    @inject
    fn new(config: ConfigService) -> Self { ... }
}

fn main() {
    let db = inject DatabaseService;
}
```

### Level 2: Dynamic Contexts (provide/using)

```verum
context async Database {
    async fn query(sql: Text) -> List<Row>
}

async fn fetch_users() using [Database] -> List<User> {
    Database.query("SELECT * FROM users").await
}

async fn main() {
    provide Database = PostgresDatabase.connect().await;
    let users = fetch_users().await;
}
```

### Combining DI and Contexts

```verum
@injectable(Scope.Singleton)
type UserService is { ... }

implement UserService {
    @inject
    fn new() using [Logger] -> Self {
        Logger.info("UserService initialized");
        Self { ... }
    }
}
```

## Running Tests

```bash
# Run all context tests
vtest specs/L2-standard/contexts/

# Run specific category
vtest specs/L2-standard/contexts/level1_di/
vtest specs/L2-standard/contexts/errors/

# Run single test
vtest specs/L2-standard/contexts/level1_di/injectable_singleton.vr
```

## Related Documentation

- [Context System Design](../../../../docs/detailed/16-context-system.md)
- [VCS Specification Section 10](../../../../docs/vcs-spec.md#10-система-контекстов)
- [verum_context crate](../../../../crates/verum_context/)
