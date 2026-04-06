# Verum Compilation Pipeline Implementation

## Overview

Complete implementation of the 14+ phase compilation pipeline as specified in `docs/detailed/06-compilation-pipeline.md`.

## Architecture

```
src/
├── phases/                      # All compilation phases
│   ├── mod.rs                   # Phase trait and common types
│   ├── entry_detection.rs       # Phase 0: Entry point detection
│   ├── lexical_parsing.rs       # Phase 1: Lexing & parsing
│   ├── meta_registry_phase.rs   # Phase 2: Meta registry
│   ├── macro_expansion.rs       # Phase 3: Macro expansion
│   ├── contract_verification.rs # Phase 3a: Contract verification
│   ├── semantic_analysis.rs     # Phase 4: Type checking
│   ├── autodiff_compilation.rs  # Phase 4a: Autodiff generation
│   ├── ffi_boundary.rs          # Phase 4b: FFI boundary
│   ├── mir_lowering.rs          # Phase 5: HIR → MIR lowering
│   ├── optimization.rs          # Phase 6: Optimizations
│   └── codegen_tiers.rs         # Phase 7: Two-tier execution (v2.1)
├── pipeline.rs                  # Main compilation pipeline
├── diagnostics_engine.rs        # Unified diagnostics
├── incremental_compiler.rs      # Incremental compilation
├── profile_system.rs            # Language profiles
└── graceful_fallback.rs         # Tier fallback system
```

## Implemented Phases

### Phase 0: Entry Point Detection
**File**: `src/phases/entry_detection.rs`

- Detects `async fn main()` vs `fn main()`
- Requires explicit context provisioning
- Discovers user-defined `@std_context` contexts

**Output**: `MainConfig` enum with provisioning strategy

### Phase 1: Lexical Analysis & Parsing
**File**: `src/phases/lexical_parsing.rs`

- Profile-aware tokenization
- Tagged literal recognition (`contract#"..."`, `sql#"..."`)
- Numeric suffix recognition (`100_km`, `5_seconds`)
- Parallel parsing of multiple files
- Error recovery with diagnostics

**Output**: AST modules

### Phase 2: Meta Registry & AST Registration
**File**: `src/phases/meta_registry_phase.rs`

- Register `@tagged_literal` handlers
- Register `@derive` macros
- Register `@differentiable` functions
- Register `@verify` annotations
- Register `@interpolation_handler`
- Circular dependency detection

**Output**: Complete `MetaRegistry`

### Phase 3: Macro Expansion & Literal Processing
**File**: `src/phases/macro_expansion.rs`

- Execute meta functions in sandbox
- Parse tagged literals
- Process interpolated strings
- Validate numeric suffixes

**Output**: Expanded AST (no meta constructs)

### Phase 3a: Contract Verification
**File**: `src/phases/contract_verification.rs`

- Translate `contract#"..."` to SMT-LIB
- Generate verification conditions
- Invoke Z3/CVC5 solver
- Verify preconditions ⇒ postconditions

**Output**: Verified AST or errors

### Phase 4: Semantic Analysis
**File**: `src/phases/semantic_analysis.rs`

- Bidirectional type checking (3x faster than Hindley-Milner)
- Refinement type subsumption
- Reference validation (exclusive `&mut T`, shared `&T`)
- Context system resolution

**Output**: Typed AST (HIR)

### Phase 4a: Autodiff Compilation
**File**: `src/phases/autodiff_compilation.rs`

- Build computational graphs
- Generate VJP (reverse-mode) functions
- Type-check generated gradient code

**Output**: HIR + autodiff functions

### Phase 4b: FFI Boundary Processing
**File**: `src/phases/ffi_boundary.rs`

- Validate FFI function signatures
- Insert boundary checks for unsafe FFI calls
- Ensure memory safety at language boundaries

**Output**: Validated HIR with FFI boundaries

### Phase 5: HIR → MIR Lowering
**File**: `src/phases/mir_lowering.rs`

- Lower to control flow graph (CFG)
- Insert safety checks (CBGR, bounds, overflow)
- Track unsafe regions (systems profile)

**Output**: MIR

### Phase 6: Optimization
**File**: `src/phases/optimization.rs`

- Escape analysis → promote `&T` to `&checked T`
- Eliminate proven-safe CBGR checks (50-90% typical)
- Eliminate proven-safe bounds checks
- Function inlining (cross-module in AOT)
- SIMD vectorization (safety-preserving only)
- Dead code elimination
- Devirtualization

**Optimization Levels**:
- `O0`: No optimization
- `O1`: Basic optimization
- `O2`: Standard optimization (default)
- `O3`: Aggressive optimization

**Output**: Optimized MIR

### Phase 7: Code Generation (Two-Tier v2.1)
**File**: `src/phases/codegen_tiers.rs`

> **Architecture Decision v2.1**: Verum uses a two-tier execution model.
> JIT infrastructure is kept internally for REPL/incremental compilation only.

#### Tier 0: VBC Interpreter
- Direct VBC bytecode dispatch
- Full safety checks (~100ns CBGR overhead)
- Rich diagnostics and debugging
- Instant startup (<100ms)
- Used during development and prototyping

#### Tier 1: AOT Compiler (Production)
- VBC → LLVM IR lowering
- Proven-safe checks eliminated (0ns)
- 50-90% check elimination (typical)
- 85-95% native C performance
- Multi-target GPU via MLIR (when @device(GPU) annotated)
- Used for production builds

**Output**: Native executable or interpreted result

**Fallback**: AOT failure → Interpreter (graceful degradation)

## Supporting Systems

### Compilation Pipeline
**File**: `src/pipeline.rs`

- Orchestrates all compilation phases
- Multi-pass compilation (Registration, Expansion, Analysis)
- Performance metrics collection
- Error recovery and diagnostics
- Test execution support

**Usage**:
```rust
let mut pipeline = CompilationPipeline::new(&mut session);
let result = pipeline.compile_files(source_files)?;
println!("Compiled {} modules", result.modules.len());
```

### Diagnostics Engine
**File**: `src/diagnostics_engine.rs`

- Unified error/warning collection
- Colorized output
- Source code snippets
- Suggestions and fixes
- Error counting and summarization

### Incremental Compiler
**File**: `src/incremental_compiler.rs`

- Module-level caching
- Dependency tracking
- Automatic invalidation
- File modification detection

**Features**:
- Cache parsed ASTs
- Cache type checking results
- Cache meta registry
- Cache optimization results

### Profile System
**File**: `src/profile_system.rs`

#### Three Profiles

**Application Profile** (Default):
- Safe, productive development
- Full safety checks
- Refinement types enabled
- Context system enabled
- CBGR enabled
- No unsafe code

**Systems Profile**:
- Performance-critical code
- Optional unsafe code
- Inline assembly
- Raw pointers
- All Application features

**Research Profile**:
- Experimental features
- Dependent types
- Formal proofs
- Linear types
- Effect system
- All Application features

### Graceful Fallback
**File**: `src/graceful_fallback.rs`

Ensures compilation always succeeds:
- SMT timeout → runtime checks
- LLVM unavailable → interpreter fallback

**Fallback Chain (v2.1)**:
```
Tier 1 (AOT/LLVM) → Tier 0 (Interpreter)
```

## Performance Targets

| Metric | Target | Status |
|--------|--------|--------|
| Compilation Speed | > 50K LOC/sec | ⏳ Pending |
| Type Checking | < 100ms/10K LOC | ⏳ Pending |
| CBGR Overhead | < 15ns per check | ✅ Achieved |
| Check Elimination | 50-90% typical | ⏳ Pending |
| Runtime Performance | 0.85-0.95x Rust | ⏳ Pending |
| Memory Overhead | < 5% vs unsafe | ⏳ Pending |

## Testing

**Test File**: `tests/pipeline_phases_tests.rs`

- Unit tests for each phase
- Integration tests for complete pipeline
- Performance regression tests
- Error recovery tests
- Incremental compilation tests
- Profile system tests
- Graceful fallback tests

## Usage Examples

### Basic Compilation
```rust
use verum_compiler::*;

let mut session = Session::new(CompilerOptions::default());
let mut pipeline = CompilationPipeline::new(&mut session);

let result = pipeline.compile_files(vec!["main.vr".to_string()])?;
println!("Compiled {} modules", result.modules.len());
```

### With Incremental Compilation
```rust
let mut incremental = IncrementalCompiler::new();

if incremental.needs_recompile(&path) {
    // Recompile only what changed
    pipeline.compile_files(vec![path.display().to_string()])?;
}

let stats = incremental.stats();
println!("Cached modules: {}", stats.cached_modules);
```

### With Profile Selection
```rust
let mut profile_mgr = ProfileManager::new(Profile::Systems);

if profile_mgr.is_feature_enabled(Feature::UnsafeCode) {
    // Enable unsafe code paths
}
```

### With Graceful Fallback
```rust
let mut fallback = GracefulFallback::new(ExecutionTier::Aot);

if !fallback.llvm_available() {
    fallback.fallback("LLVM not available");
}

match fallback.active_tier() {
    ExecutionTier::Interpreter => { /* Tier 0: Interpreter */ }
    ExecutionTier::Aot => { /* Tier 1: AOT via LLVM */ }
}
```

## Future Enhancements

### Phase 8+: Additional Phases (Future)
- **Phase 8**: Link-time optimization (LTO)
- **Phase 9**: Profile-guided optimization (PGO)
- **Phase 10**: Binary optimization
- **Phase 11**: Debug info generation

### Advanced Features (Future)
- Distributed compilation
- Persistent incremental cache
- Cloud-based SMT solving
- Machine learning-guided optimization

## Specification Compliance

All phases implement the specification in:
- `docs/detailed/06-compilation-pipeline.md`

Key requirements met:
✅ Multi-pass architecture (Phase 2 before Phase 3)
✅ Order-independent meta definitions
✅ Sandboxed meta execution
✅ Two-tier execution model (v2.1)
✅ Graceful fallback guarantee
✅ Profile system integration
✅ Performance metrics collection

## Contributing

When adding new phases:
1. Implement `CompilationPhase` trait
2. Add phase to `src/phases/mod.rs`
3. Integrate in `CompilationPipeline`
4. Add tests in `tests/pipeline_phases_tests.rs`
5. Update this document

## License

Same as parent project (see root LICENSE file).
