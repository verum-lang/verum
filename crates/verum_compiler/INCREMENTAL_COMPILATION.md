# Incremental Compilation System

## Overview

The Verum incremental compilation system provides fast rebuilds by tracking file changes, building a dependency graph, and only recompiling changed modules and their dependents.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                  IncrementalCompiler                          │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌─────────────────┐  ┌──────────────────┐                  │
│  │  Content Hashes │  │ Dependency Graph │                  │
│  │   (SHA-256)     │  │  (forward/back)  │                  │
│  └─────────────────┘  └──────────────────┘                  │
│                                                               │
│  ┌─────────────────┐  ┌──────────────────┐                  │
│  │  Module Cache   │  │ Type Check Cache │                  │
│  │   (AST nodes)   │  │   (results)      │                  │
│  └─────────────────┘  └──────────────────┘                  │
│                                                               │
│  ┌──────────────────────────────────────┐                   │
│  │      Disk Persistence                │                   │
│  │  (target/incremental/metadata.json)  │                   │
│  └──────────────────────────────────────┘                   │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

## Key Features

### 1. Content Hash Tracking

Each file is tracked using:
- **SHA-256 hash** of content for precise change detection
- **Modification timestamp** for quick rejection of unchanged files
- **Persistent storage** in `target/incremental/metadata.json`

### 2. Dependency Graph

Two-way dependency tracking:
- **Forward edges**: Module → Dependencies
- **Reverse edges**: Module → Dependents (who depends on me?)

This enables:
- Minimal recompilation set calculation
- Topological sorting for correct build order
- Circular dependency detection

### 3. Change Propagation

When a file changes:
1. Compute new content hash
2. Compare with cached hash
3. If different, mark file for recompilation
4. Recursively mark all dependents
5. Topologically sort the recompilation set

### 4. Cache Persistence

Cache state persists across compilation sessions in:
```
target/incremental/
├── metadata.json          # Content hashes, dependencies, timestamps
└── modules/               # (future) Serialized ASTs, type info
```

## Usage

### Basic Usage

```rust
use verum_compiler::IncrementalCompiler;
use std::path::PathBuf;

// Create incremental compiler
let mut compiler = IncrementalCompiler::with_defaults(
    PathBuf::from("/path/to/project")
);

// Load previous compilation cache
compiler.load_cache()?;

// Check if a file needs recompilation
let file = PathBuf::from("src/main.vr");
if compiler.needs_recompile(&file) {
    // Compile the file
    let module = compile_file(&file)?;

    // Extract dependencies from imports
    let deps = compiler.extract_dependencies(&module);

    // Cache the compiled module
    compiler.cache_module(file, module, deps);
}

// Save cache for next session
compiler.save_cache()?;
```

### Integration with CompilationPipeline

```rust
use verum_compiler::{CompilationPipeline, IncrementalCompiler};

let mut pipeline = CompilationPipeline::new(&mut session);
let mut incremental = IncrementalCompiler::with_defaults(project_root);

incremental.load_cache()?;

// Discover all .vr files
let all_files = discover_project_files()?;

// Determine which files changed
let changed_files: Vec<PathBuf> = all_files
    .into_iter()
    .filter(|f| incremental.needs_recompile(f))
    .collect();

if changed_files.is_empty() {
    println!("No files changed, skipping compilation");
    return Ok(());
}

// Get minimal recompilation set (topologically sorted)
let to_compile = incremental.get_recompilation_set(&changed_files);

println!("Recompiling {} of {} modules", to_compile.len(), all_files.len());

// Compile only what's needed
for file in to_compile {
    let module = pipeline.compile_file(&file)?;
    let deps = incremental.extract_dependencies(&module);
    incremental.cache_module(file, module, deps);
}

incremental.save_cache()?;
```

## Change Detection Algorithm

```
needs_recompile(file):
  1. Check if file exists → if not, return false
  2. Compute current content hash (SHA-256)
  3. Check cache for previous hash:
     a. If not in cache → return true (new file)
     b. If hash differs → return true (content changed)
     c. If mod time > cached time → return true (likely changed)
  4. Check dependencies (recursive):
     for dep in dependencies(file):
       if needs_recompile(dep) → return true
  5. Return false (no recompilation needed)
```

## Minimal Recompilation Set

Given a set of changed files, compute the minimal set to recompile:

```
get_recompilation_set(changed_files):
  1. Initialize to_recompile = ∅
  2. For each file in changed_files:
     mark_for_recompilation(file, to_recompile)
  3. Return topological_sort(to_recompile)

mark_for_recompilation(file, set):
  1. If file ∈ set → return (already marked)
  2. Add file to set
  3. For each dependent in dependents[file]:
     mark_for_recompilation(dependent, set)
```

## Topological Sort

Ensures dependencies are compiled before dependents:

```
topological_sort(modules):
  1. Initialize result = [], visited = ∅, visiting = ∅
  2. For each module in modules:
     if module ∉ visited:
       visit(module, modules, visited, visiting, result)
  3. Return result

visit(module, modules, visited, visiting, result):
  1. If module ∈ visited → return
  2. If module ∈ visiting → warn("circular dependency")
  3. Add module to visiting
  4. For each dep in dependencies[module]:
     if dep ∈ modules:
       visit(dep, modules, visited, visiting, result)
  5. Remove module from visiting
  6. Add module to visited
  7. Append module to result
```

## Performance Characteristics

| Operation | Time Complexity | Notes |
|-----------|----------------|-------|
| Content hash | O(n) | n = file size, ~100MB/s |
| Needs recompile | O(d) | d = dependency depth |
| Mark dependents | O(n + e) | n = modules, e = edges |
| Topological sort | O(n + e) | DFS-based |
| Cache load/save | O(n) | n = tracked files |

## Example: Incremental Build Flow

```
Initial state (empty cache):
  ├── main.vr        [changed: yes] → compile
  ├── lib.vr         [changed: yes] → compile
  └── utils.vr       [changed: yes] → compile

After change to utils.vr:
  ├── main.vr        [deps: lib] → recompile (dependent)
  ├── lib.vr         [deps: utils] → recompile (dependent)
  └── utils.vr       [changed: yes] → recompile

Recompilation order (topological):
  1. utils.vr       (no deps)
  2. lib.vr         (depends on utils)
  3. main.vr        (depends on lib)
```

## Cache Format

The `target/incremental/metadata.json` file contains:

```json
{
  "content_hashes": {
    "/path/to/main.vr": {
      "hash": "a3f2b8c9...",
      "timestamp": "2024-12-18T20:30:15Z"
    },
    "/path/to/lib.vr": {
      "hash": "e8d4f1a2...",
      "timestamp": "2024-12-18T19:15:42Z"
    }
  },
  "dependencies": {
    "/path/to/main.vr": ["/path/to/lib.vr"],
    "/path/to/lib.vr": ["/path/to/utils.vr"]
  },
  "last_compile_time": "2024-12-18T20:30:20Z"
}
```

## Statistics

Get compilation statistics:

```rust
let stats = compiler.stats();
println!("{}", stats.report());
```

Output:
```
Incremental Cache Stats:
- Cached modules: 42
- Tracked files: 45
- Dependency edges: 87
- Meta registry valid: true
```

## Future Enhancements

1. **AST Serialization**: Cache parsed ASTs to disk
2. **Type Info Cache**: Persist type checking results
3. **IR Cache**: Cache LLVM IR for AOT compilation
4. **Distributed Cache**: Share cache across machines
5. **Parallel Compilation**: Compile independent modules in parallel
6. **Query-based**: Use Salsa-style query system for finer granularity
7. **Incremental Type Checking**: Only re-check changed signatures

## Testing

Comprehensive test coverage in `tests/incremental_compilation_tests.rs`:

- ✅ Basic cache operations
- ✅ Content hash change detection
- ✅ Dependency tracking
- ✅ Invalidation
- ✅ Cache persistence
- ✅ Type check caching
- ✅ Topological sorting
- ✅ Circular dependency detection
- ✅ Clear cache
- ✅ Statistics reporting

Run tests:
```bash
cargo test -p verum_compiler --test incremental_compilation_tests
```

## References

- Spec: `docs/detailed/06-compilation-pipeline.md#28-incremental-compilation-strategy`
- Implementation: `crates/verum_compiler/src/incremental_compiler.rs`
- Tests: `crates/verum_compiler/tests/incremental_compilation_tests.rs`
