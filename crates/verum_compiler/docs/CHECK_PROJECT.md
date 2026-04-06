# Multi-File Type Checking with `check_project()`

## Overview

The `check_project()` method provides multi-file type checking without code generation. It's designed for:

- **IDE Integration**: Fast feedback during development
- **CI/CD Pipelines**: Validation without compilation overhead
- **Development Workflows**: Quick checks before committing

## Usage

```rust
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};
use std::path::PathBuf;

// Create compiler session
let options = CompilerOptions {
    input: PathBuf::from("src/main.vr"),
    output: PathBuf::new(),
    verbose: 1,
    ..Default::default()
};

let mut session = Session::new(options);
let mut pipeline = CompilationPipeline::new_check(&mut session);

// Run type checking
let result = pipeline.check_project()?;

println!("Checked {} files with {} types in {:.2}s",
    result.files_checked,
    result.types_inferred,
    result.elapsed.as_secs_f64()
);

if result.is_ok() {
    println!("Type checking succeeded!");
} else {
    println!("Found {} errors and {} warnings",
        result.errors,
        result.warnings
    );
}
```

## How It Works

The `check_project()` method performs the following steps:

### 1. File Discovery

Discovers all `.vr` files in the project directory:

```rust
let project_files = self.session.discover_project_files()?;
```

### 2. Source Loading

Loads all source files into memory:

```rust
for file_path in &project_files {
    let source_text = std::fs::read_to_string(&file_path)?;
    sources.insert(module_path, source_text);
}
```

### 3. Three-Pass Compilation

Runs the multi-pass compilation pipeline:

#### Pass 1: Registration
- Parses all files
- Registers meta functions and macros
- Builds cross-file dependency graph

#### Pass 2: Expansion
- Executes meta functions
- Expands macros
- Generates compile-time code

#### Pass 3: Type Checking
- Resolves imports between files
- Infers types across all modules
- Validates protocol implementations
- Checks function signatures

### 4. Result Collection

Returns a `CheckResult` with:

```rust
pub struct CheckResult {
    pub files_checked: usize,
    pub types_inferred: usize,
    pub warnings: usize,
    pub errors: usize,
    pub elapsed: Duration,
}
```

## Differences from `compile_project()`

| Feature | `check_project()` | `compile_project()` |
|---------|-------------------|---------------------|
| File Discovery | ✓ | ✓ |
| Parsing | ✓ | ✓ |
| Type Checking | ✓ | ✓ |
| Import Resolution | ✓ | ✓ |
| Code Generation | ✗ | ✓ |
| LLVM IR | ✗ | ✓ |
| Executable Output | ✗ | ✓ |
| Performance | Fast (~100ms/10K LOC) | Slower (full compilation) |

## Performance

Type checking performance targets:

- **Speed**: < 100ms per 10,000 lines of code
- **Throughput**: > 50,000 LOC/second
- **Memory**: < 5% overhead compared to single-file check

## Integration Examples

### CI/CD Pipeline

```yaml
# .github/workflows/ci.yml
- name: Type check Verum code
  run: |
    verum check src/
```

### Pre-commit Hook

```bash
#!/bin/bash
# .git/hooks/pre-commit
verum check . --quiet || {
    echo "Type checking failed. Aborting commit."
    exit 1
}
```

### IDE Integration (LSP)

```rust
// In your LSP server
fn on_did_save(&mut self, params: DidSaveTextDocumentParams) {
    let result = self.pipeline.check_project()?;
    self.publish_diagnostics(result);
}
```

### Build Script

```rust
// build.rs
use verum_compiler::{CompilationPipeline, Session};

fn main() {
    let mut session = Session::new(/* ... */);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    if let Err(e) = pipeline.check_project() {
        panic!("Type checking failed: {}", e);
    }
}
```

## Error Handling

The method returns:

- `Ok(CheckResult)`: Type checking completed (may have errors/warnings in result)
- `Err(anyhow::Error)`: Fatal error during checking (file I/O, parsing failure, etc.)

```rust
match pipeline.check_project() {
    Ok(result) => {
        if result.is_ok() {
            println!("Success!");
        } else {
            eprintln!("Found {} errors", result.errors);
            // Display diagnostics
            session.display_diagnostics()?;
        }
    }
    Err(e) => {
        eprintln!("Fatal error: {}", e);
    }
}
```

## Limitations

1. **No Code Generation**: Does not produce executable output
2. **No Optimization**: Skips optimization passes
3. **No Linking**: Does not resolve external dependencies
4. **Stdlib Optional**: Can run without compiled stdlib (limited built-ins)

## Best Practices

1. **Use for Development**: Fast iteration during coding
2. **CI Integration**: Validate PRs before expensive compilation
3. **Incremental Checks**: Only check changed files (future enhancement)
4. **Watch Mode**: Re-check on file changes (future enhancement)

## Future Enhancements

- Incremental type checking (only check changed files)
- Parallel file processing
- Fine-grained diagnostics export (JSON/LSP format)
- Watch mode for continuous checking
- Cache type information between runs

## See Also

- [Compilation Pipeline](../../../docs/detailed/06-compilation-pipeline.md)
- [Module System](../../../docs/detailed/14-module-system.md)
- [Type System](../../../docs/detailed/03-type-system.md)
