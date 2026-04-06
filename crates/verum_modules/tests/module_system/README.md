# Module System Integration Tests

Comprehensive end-to-end tests for the Verum module system, demonstrating multi-file project compilation and module resolution.

## Overview

These tests validate the complete Verum module system: hierarchical namespaces, visibility control (private by default), and deterministic name resolution (local > explicit imports > glob imports > prelude). Each test file focuses on a specific aspect of the module system, using temporary file structures to simulate real-world projects.

## Test Structure

All tests follow a consistent pattern:

1. **Setup**: Create temporary directory with test file structure
2. **Execute**: Load modules using `ModuleLoader`
3. **Verify**: Assert expected behavior (compilation success/failure, correct imports, etc.)

## Test Files

### `simple.rs` - Basic Two-File Imports (7 tests)

Tests fundamental module loading and import resolution:

- ✅ Basic two-file import (File 1 defines type, File 2 imports it)
- ✅ Single item import
- ✅ Multiple items import with `{}`
- ✅ Module not found error handling
- ✅ Absolute paths with `crate.` prefix
- ✅ Cross-module imports (3+ files)

**Coverage**: Module structure, file-system mapping, basic imports (mount/import syntax)

### `visibility.rs` - Visibility Enforcement (8 tests)

Tests visibility modifiers across module boundaries:

- ✅ `public` visibility (accessible everywhere)
- ✅ Private visibility (default, same module only)
- ✅ `public(crate)` visibility (crate-local)
- ✅ Visibility checker for public items
- ✅ Visibility checker for private items
- ✅ Struct field visibility (mixed public/private/crate)
- ✅ Visibility hierarchy (parent/child modules)
- ✅ Mixed visibility items in single module

**Coverage**: Visibility modifiers (public, private, public(crate), public(parent)), field visibility, import resolution algorithm visibility checks

### `directory.rs` - Directory Modules (10 tests)

Tests directory-based module organization:

- ✅ Directory module with `mod.vr`
- ✅ Nested directory modules (multi-level)
- ✅ Module file vs directory precedence
- ✅ Sibling modules in same directory
- ✅ Deep directory hierarchy (4+ levels)
- ✅ Relative imports (`super`, `self`) in directory modules
- ✅ ModulePath resolution utilities
- ✅ Module hierarchy descendant checking

**Coverage**: Module tree organization (single-file, file-based, directory-based with mod.vr, mixed hierarchy)

### `reexport.rs` - Re-exports (8 tests)

Tests re-exporting types through module boundaries:

- ✅ Basic re-export (`public import`)
- ✅ Re-export with rename (`as NewName`)
- ✅ Transitive re-exports (3+ levels)
- ✅ Flatten module hierarchy (expose internal modules)
- ✅ Selective re-export (hide some items)
- ✅ Re-export from subdirectories
- ✅ Re-export multiple items in one statement
- ✅ Glob re-export (`import mod.*`)

**Coverage**: Re-exports (`public import`), rename re-exports, transitive re-exports, glob re-exports

### `imports.rs` - Import Patterns (13 tests)

Tests various import syntaxes and resolution:

- ✅ Glob imports (`import mod.*`)
- ✅ Nested imports simple (`{A, B, C}`)
- ✅ Deeply nested imports (`std.{io.{Read, Write}, ...}`)
- ✅ Import with alias (`as Name`)
- ✅ Self import (`import io.{self, Read}`)
- ✅ Relative import with `super`
- ✅ Relative import with `self`
- ✅ Module path resolution (`super.super`, etc.)
- ✅ Import shadowing (local bindings)
- ✅ Complex nested imports
- ✅ Import trailing commas
- ✅ Absolute vs relative imports
- ✅ Import resolution priority

**Coverage**: Import syntax (glob, nested, alias, self, relative super/self), path resolution priority (6-step algorithm), name shadowing

### `circular.rs` - Circular Dependencies (13 tests)

Tests cycle detection and handling:

- ✅ Type dependency cycles (allowed)
- ✅ Function dependency cycles (allowed)
- ✅ Dependency graph cycle detection
- ✅ Dependency graph without cycles
- ✅ Self-dependency detection
- ✅ Diamond dependency pattern
- ✅ Complex cycle detection (mid-chain)
- ✅ Topological sort order verification
- ✅ Multiple independent modules
- ✅ Long dependency chains (10+ modules)
- ✅ Mutual type dependencies
- ✅ Three-way cycles

**Coverage**: Circular dependencies (type cycles allowed, value cycles prevented, topological sort, diamond pattern)

## Total Test Coverage

- **Test Files**: 6
- **Test Cases**: 59
- **Lines of Test Code**: ~2,500

## Running Tests

### Run all integration tests:
```bash
cargo test --test integration
```

### Run specific test file:
```bash
cargo test --test integration simple::
cargo test --test integration visibility::
cargo test --test integration directory::
cargo test --test integration reexport::
cargo test --test integration imports::
cargo test --test integration circular::
```

### Run specific test case:
```bash
cargo test --test integration test_basic_two_file_import
```

### Run with output:
```bash
cargo test --test integration -- --nocapture
```

## Test Utilities

All tests use a common `TestProject` helper:

```rust
struct TestProject {
    temp_dir: TempDir,
}

impl TestProject {
    fn new() -> Self { /* ... */ }
    fn create_file(&self, path: &str, content: &str) { /* ... */ }
    fn root_path(&self) -> &std::path::Path { /* ... */ }
}
```

This creates isolated temporary directories for each test, ensuring no interference between tests.

## Specification Compliance

Every test includes a comment referencing the relevant specification section:

```rust
#[test]
fn test_basic_two_file_import() {
    // File system mapping: each .vr file = one module, directory with mod.vr = module
    // ...
}
```

This ensures traceability between tests and the module system design.

## Edge Cases Tested

- Module not found errors
- File vs directory module precedence
- Deep nesting (4+ levels)
- Long dependency chains (10+ modules)
- Diamond dependencies
- Three-way cycles
- Import shadowing
- Mixed visibility levels
- Glob imports
- Deeply nested imports
- Transitive re-exports

## Future Enhancements

Potential areas for additional tests:

- [ ] External crate imports (cross-crate)
- [ ] Conditional compilation (`@cfg`)
- [ ] Profile-aware module loading
- [ ] Prelude system integration
- [ ] Path-restricted visibility (`public(in path)`)
- [ ] Protocol coherence across modules
- [ ] Incremental compilation with module changes
- [ ] Module caching performance
- [ ] Parallel module loading
- [ ] Error recovery in module resolution

## Contributing

When adding new integration tests:

1. Create test in appropriate file (or new file if needed)
2. Use `TestProject` helper for file setup
3. Reference specification section in comment
4. Test both success and failure cases
5. Add clear assertions with helpful messages
6. Update this README with test count

## Debugging Tips

If a test fails:

1. Check the test output for specific error messages
2. Use `-- --nocapture` to see full output
3. Examine the temporary file structure being created
4. Verify the specification reference is correct
5. Check if related tests also fail (indicates systemic issue)

## Performance

Tests are designed to be fast:

- Use in-memory temporary directories
- Minimal file I/O
- No network calls
- Parallel test execution enabled

Expected runtime: < 2 seconds for all 59 tests.
