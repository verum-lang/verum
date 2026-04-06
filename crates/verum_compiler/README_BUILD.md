# Verum Compiler - No-LLVM Build

## Overview

This is a complete working Verum compiler that runs **without LLVM dependencies**. It uses a tree-walk interpreter for execution instead of generating machine code.

## Architecture

The compiler implements a standard multi-phase pipeline:

```
Source Code
    ↓
1. Lexing (verum_lexer)
    ↓
2. Parsing (verum_parser)
    ↓
3. Type Checking (verum_types)
    ↓
4. Refinement Verification (verum_smt, optional)
    ↓
5. CBGR Analysis (verum_cbgr)
    ↓
6. Interpretation (verum_interpreter)
    ↓
Output
```

## Key Changes from Previous Build

- **Removed:** verum_codegen (LLVM-based code generation)
- **Added:** verum_interpreter integration for direct execution
- **Result:** Faster builds, no LLVM API compatibility issues

## Building

```bash
# Build the compiler
cargo build --package verum_compiler --release

# The binary will be at target/release/verum
```

## Usage

### Run a program directly (with interpretation)
```bash
./target/release/verum run examples/hello_world.vr
```

### Type check only (no execution)
```bash
./target/release/verum check examples/hello_world.vr
```

### View compiler information
```bash
./target/release/verum info --all
```

### REPL (interactive mode)
```bash
./target/release/verum repl
```

## Example Programs

The `examples/` directory contains:

- **hello_world.vr** - Basic output
- **fibonacci.vr** - Recursive function
- **factorial.vr** - Factorial with type safety
- **quicksort.vr** - List sorting algorithm
- **binary_tree.vr** - Tree data structure

All examples can be run with:
```bash
./target/release/verum run examples/[name].vr
```

## Running Tests

```bash
# Run all compiler tests
cargo test --package verum_compiler

# Run integration tests only
cargo test --package verum_compiler --test '*'

# Run with output
cargo test --package verum_compiler -- --nocapture
```

## Performance

- **Compilation speed:** No LLVM overhead, typically <100ms for small programs
- **Memory overhead:** Minimal interpreter state
- **Execution speed:** Tree-walk interpretation (not optimized for speed)

## Limitations

- No machine code generation (use interpreter instead)
- No ahead-of-time compilation to native binaries
- Performance limited by interpreter overhead

## Future Improvements

- [ ] Bytecode compilation for faster execution
- [ ] LLVM integration when API stabilizes
- [ ] JIT compilation for hot functions
- [ ] Comprehensive standard library

## Related Documentation

- Specification: `docs/detailed/28-implementation-roadmap.md`
- Type System: `docs/detailed/03-type-system.md`
- CBGR System: `docs/detailed/24-cbgr-implementation.md`

## Development

### Adding a new example

1. Create `examples/my_program.vr`
2. Add a test in `tests/compile_examples.rs`
3. Run `cargo test` to verify

### Compiler pipeline extensions

To add new compilation phases:

1. Add a new `CompilerPass` variant
2. Implement a `fn phase_*` method in `CompilationPipeline`
3. Call it from `run_full_compilation` or `run_interpreter`

## Troubleshooting

### "Failed to load source file"
- Ensure the .vr file exists and is readable
- Use absolute paths if relative paths don't work

### "Parse error"
- Check Verum syntax matches the language specification
- Use `verum parse file.vr` to see detailed AST

### "Type checking failed"
- Ensure all types are properly annotated
- Check variable names match declarations
