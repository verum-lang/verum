# Verum Compiler Driver

The main compiler binary for the Verum programming language.

## Overview

This crate orchestrates all compilation phases:

1. **Lexing**: Tokenization with `verum_lexer`
2. **Parsing**: AST construction with `verum_parser`
3. **Type Checking**: Bidirectional type inference with `verum_types`
4. **Refinement Verification**: SMT-based verification with `verum_smt`
5. **Code Generation**: LLVM IR generation with `verum_codegen` (future)

## CLI Commands

### Build

Compile source file to executable:

```bash
verum build main.vr
verum build main.vr -o output --verify-mode proof
verum build main.vr -O3 --show-costs
```

### Check

Type check without compilation:

```bash
verum check main.vr
verum check main.vr --continue-on-error
```

### Verify (P0 Feature!)

Run refinement type verification with cost reporting:

```bash
verum verify main.vr --show-costs
verum verify main.vr --mode proof --timeout 60
verum verify main.vr --function binary_search
```

**Example Output:**

```
Verification Report:
  ✓ algorithm(): Proved in 1.2s (Z3)
  ⚠ complex_fn(): Timeout after 30s, falling back to runtime
  ✗ invalid_fn(): Counterexample found: n = 0

Suggestions:
  - Use @verify(runtime) for complex_fn (30s → 0s)
  - Add precondition n > 0 to invalid_fn
```

### Profile (P0 Feature!)

Profile CBGR memory overhead:

```bash
verum profile --memory main.vr
verum profile --memory main.vr --hot-threshold 10.0 --suggest
verum profile --memory main.vr -o report.json
```

**Example Output:**

```
CBGR Performance Report:
  🔥 hot_loop(): 15.0% CBGR overhead
      5 CBGR refs, 0 ownership refs, 50 checks
      ⚠ Hot path detected! Consider optimization

  safe_parse(): 0.1% CBGR overhead
      2 CBGR refs, 0 ownership refs, 20 checks

Total: 10 functions, 1 hot paths

Optimization Suggestions:
  • hot_loop(): 15.0% overhead
    → Convert CBGR refs to ownership:
      fn hot_loop(data: %T) instead of &T
      Benefit: 15.0% → 0% overhead
```

### Run

Execute program (interpreter mode):

```bash
verum run main.vr
verum run main.vr -- arg1 arg2
verum run main.vr --skip-verify  # unsafe, for testing
```

### REPL

Interactive read-eval-print loop:

```bash
verum repl
verum repl --preload std.vr
verum repl --skip-verify  # faster experimentation
```

**REPL Commands:**

- `:help, :h` - Show help
- `:quit, :q` - Exit REPL
- `:clear, :c` - Clear multiline buffer
- `:reset, :r` - Reset type checker state

### Info

Display compiler information:

```bash
verum info
verum info --features
verum info --llvm
verum info --all
```

## Global Flags

- `-v, --verbose` - Verbose output (can be repeated)
- `-q, --quiet` - Suppress warnings
- `--color <WHEN>` - Color output (auto/always/never)
- `--emit-json` - JSON diagnostics for IDE integration

## Architecture

### Module Structure

```
verum_compiler/
├── src/
│   ├── main.rs           # CLI entry point (~350 lines)
│   ├── lib.rs            # Public API
│   ├── options.rs        # Compiler configuration (~200 lines)
│   ├── session.rs        # Compilation session (~250 lines)
│   ├── pipeline.rs       # Compilation pipeline (~250 lines)
│   ├── verify_cmd.rs     # Verification command (P0) (~250 lines)
│   ├── profile_cmd.rs    # CBGR profiling (P0) (~250 lines)
│   └── repl.rs           # Interactive REPL (~200 lines)
└── Cargo.toml
```

### Compilation Pipeline

```rust
use verum_compiler::{Session, CompilerOptions, CompilationPipeline};

let options = CompilerOptions::new("main.vr".into(), "main".into())
    .with_verify_mode(VerifyMode::Proof)
    .with_optimization(3);

let mut session = Session::new(options);
let mut pipeline = CompilationPipeline::new(&mut session);

pipeline.run_full_compilation()?;
```

### Session Management

The `Session` tracks:

- Source files by `FileId`
- Parsed AST modules (cached)
- Diagnostics for error reporting
- Compiler options

### Error Handling

- Excellent error messages with colors and source snippets
- JSON output for IDE integration
- Multiple errors reported in one pass
- Proper exit codes

## P0 Features for v1.0

### 1. CBGR Profiling

Identifies hot paths with high CBGR overhead and suggests converting to ownership references (`%T`).

**Key Metrics:**
- CBGR reference count
- Number of generation checks
- Time spent in CBGR validation
- Overhead percentage

**Recommendations:**
- Convert `&T` → `%T` for hot paths (>5% overhead)
- Keep `&T` for cold paths (<1% overhead)

### 2. Verification Cost Reporting

Tracks SMT solver performance and suggests optimizations.

**Key Metrics:**
- Verification time per function
- Timeout detection
- Counterexample generation

**Recommendations:**
- Use `@verify(runtime)` for expensive proofs
- Simplify refinements that timeout
- Add preconditions to fix failed verifications

## Performance

- **Fast startup**: <100ms cold start
- **Incremental compilation**: Planned for future versions
- **Parallel type checking**: Uses all CPU cores
- **Efficient caching**: ASTs and type information cached

## Testing

```bash
# Run all tests
cargo test -p verum_compiler

# Run with verbose output
cargo test -p verum_compiler -- --nocapture

# Run specific test
cargo test -p verum_compiler test_pipeline_load_source
```

## Future Work

- [ ] Complete code generation (LLVM backend)
- [ ] Incremental compilation
- [ ] LSP server for IDE integration
- [ ] Package manager integration
- [ ] Cross-compilation support
- [ ] Debugger integration

## License

Apache-2.0
