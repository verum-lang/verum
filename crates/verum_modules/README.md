# verum_modules

> Comprehensive module system for the Verum programming language

## Overview

The `verum_modules` crate implements the complete module system for Verum. It provides module loading, name resolution, dependency management, and import/export handling. The module system has three core responsibilities: namespace management (hierarchical modules), visibility control (private by default, fine-grained modifiers), and dependency resolution (deterministic, unambiguous name resolution with priority: local scope > explicit imports > glob imports > prelude).

## Features

- **Module Loading**: Load .vr files from filesystem with caching
- **Name Resolution**: Resolve identifiers across module boundaries
- **Dependency Management**: Track dependencies and compute compilation order
- **Import/Export**: Handle complex import patterns and re-exports
- **Visibility Control**: Enforce fine-grained visibility rules
- **Caching**: Thread-safe module cache for performance
- **Error Reporting**: Detailed error messages for all module-related issues

## Architecture

```
┌─────────────────┐
│   ModuleLoader  │  Loads .vr files from filesystem
└────────┬────────┘
         │
         v
┌─────────────────┐
│ DependencyGraph │  Builds module dependency graph
└────────┬────────┘
         │
         v
┌─────────────────┐
│  NameResolver   │  Resolves names with scope rules
└────────┬────────┘
         │
         v
┌─────────────────┐
│VisibilityChecker│  Checks access permissions
└─────────────────┘
```

## Module Components

### Core Types (`path.rs`)

- `ModuleId`: Unique identifier for modules
- `ModulePath`: Hierarchical module path (e.g., `std.collections.List`)
- `ModuleInfo`: Complete module information with AST and metadata

### Loading (`loader.rs`)

The module loader implements the file system mapping rules:

- `lib.vr` or `main.vr` → crate root
- `foo.vr` → module `foo`
- `foo/bar.vr` → module `foo.bar`
- `foo/mod.vr` → module `foo` with child modules

Example:
```rust
use verum_modules::ModuleLoader;
use std::path::Path;

let mut loader = ModuleLoader::new(Path::new("src"));
let module = loader.load_and_parse(&ModulePath::from_str("main"), ModuleId::ROOT)?;
```

### Resolution (`resolver.rs`)

The name resolver implements the resolution priority algorithm:

1. Check local scope
2. Check explicit imports
3. Check glob imports
4. Check prelude
5. Error if ambiguous

Example:
```rust
use verum_modules::NameResolver;

let mut resolver = NameResolver::new();
let scope = resolver.create_scope(module_id);
let resolved = resolver.resolve_name("List", module_id)?;
```

### Dependencies (`dependency.rs`)

The dependency graph tracks module dependencies and computes topological order:

```rust
use verum_modules::DependencyGraph;

let mut graph = DependencyGraph::new();
graph.add_module(mod1, path1);
graph.add_module(mod2, path2);
graph.add_dependency(mod1, mod2)?; // mod1 depends on mod2

// Get compilation order
let order = graph.topological_order()?;
```

### Imports (`imports.rs`)

Handles all import patterns:

- Simple: `import std.io.File`
- Glob: `import std.io.*`
- Nested: `import std.io.{File, Read, Write}`
- Renaming: `import std.io.File as MyFile`

### Exports (`exports.rs`)

Manages module exports and re-exports:

```rust
use verum_modules::exports::{ExportTable, ExportedItem};

let mut exports = ExportTable::new();
exports.add_export(ExportedItem::new(
    "my_function",
    ExportKind::Function,
    Visibility::Public,
    module_id,
    span,
))?;
```

### Visibility (`visibility.rs`)

Enforces visibility rules:

- `public`: Visible everywhere
- `internal`: Visible within crate
- `protected`: Visible within module tree
- `private`: Visible only in module (default)

### Caching (`cache.rs`)

Thread-safe module cache with validity checking:

```rust
use verum_modules::ModuleCache;

let cache = ModuleCache::new();
cache.insert(file_path, entry);

if let Some(module) = cache.get_by_path(&file_path) {
    // Use cached module
}
```

## Usage Example

```rust
use verum_modules::*;
use std::path::Path;

// Create module registry
let mut registry = ModuleRegistry::new();

// Load modules
let mut loader = ModuleLoader::new(Path::new("src"));
let root_id = registry.allocate_id();
let root_module = loader.load_and_parse(&ModulePath::root(), root_id)?;
registry.register(root_module);

// Build dependency graph
let mut graph = DependencyGraph::new();
// ... add modules and dependencies ...

// Get compilation order
let order = graph.topological_order()?;

// Resolve names
let mut resolver = NameResolver::new();
for &module_id in &order {
    let module = registry.get(module_id).unwrap();
    // ... resolve imports and names ...
}
```

## Specification Compliance

This implementation follows the Verum module system specification:

- **Section 1**: Module structure and file system mapping
- **Section 3**: Import system with all patterns
- **Section 4**: Protocol coherence (orphan rules)
- **Section 5**: Import resolution algorithm
- **Section 6**: Path resolution with priorities
- **Section 7**: Circular dependency detection

Key principles: explicit is better than implicit (no magical globals), file system mirrors
module hierarchy, visibility defaults to private, name resolution is deterministic.

## v6.0-BALANCED Compliance

This crate uses Verum semantic types throughout:

- `List` instead of `Vec`
- `Map` instead of `HashMap`
- `Set` instead of `HashSet`
- `Text` instead of `String`
- `Maybe` instead of `Option`
- `Shared` instead of `Arc`

## Testing

Comprehensive tests are provided in `tests/module_resolution_tests.rs`:

```bash
cargo test
```

## Performance

The module system is designed for performance:

- Thread-safe caching with `DashMap`
- Lazy evaluation of glob imports
- Efficient dependency graph with `petgraph`
- Incremental compilation support

Typical performance targets:
- Module loading: < 1ms per file
- Name resolution: < 100μs per name
- Dependency graph: < 10ms for 1000 modules

## Error Handling

All errors are detailed and actionable:

```rust
match result {
    Err(ModuleError::ModuleNotFound { path, searched_paths, .. }) => {
        eprintln!("Module '{}' not found. Searched:", path);
        for p in searched_paths {
            eprintln!("  - {}", p.display());
        }
    }
    Err(ModuleError::CircularDependency { cycle, .. }) => {
        eprintln!("Circular dependency detected:");
        for id in cycle {
            eprintln!("  → {:?}", id);
        }
    }
    // ... other error cases ...
}
```

## Integration

The module system integrates with other Verum crates:

- `verum_ast`: Provides AST types
- `verum_parser`: Parses module files
- `verum_types`: Type checking across modules
- `verum_diagnostics`: Error reporting
- `verum_compiler`: Compilation pipeline

## Future Enhancements

Planned features:

1. **Parallel Loading**: Load multiple modules concurrently
2. **Incremental Updates**: Watch for file changes
3. **Module Interfaces**: Generate interface files for faster compilation
4. **Profile-Aware Resolution**: Handle language profile restrictions
5. **Package Integration**: Support for external packages

## License

MIT OR Apache-2.0
