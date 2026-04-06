# Verum Language Platform

## CRITICAL: Verum Grammar Specification

**AUTHORITATIVE SOURCE**: `grammar/verum.ebnf` - The ONLY source of truth for Verum syntax.

Before writing or modifying ANY `.vr` file, you MUST verify syntax against `grammar/verum.ebnf`.

### Verum is NOT Rust! Key Differences:

| Rust Syntax (WRONG) | Verum Syntax (CORRECT) | EBNF Reference |
|---------------------|------------------------|----------------|
| `struct Name { ... }` | `type Name is { ... };` | `type_def` |
| `enum Name { A, B }` | `type Name is A \| B;` | `variant_list` |
| `trait Name { ... }` | `type Name is protocol { ... };` | `protocol_def` |
| `impl Name { ... }` | `implement Name { ... }` | `impl_block` |
| `impl Trait for T` | `implement Trait for T` | `impl_type` |
| `Box::new(x)` | `Heap(x)` | semantic types |
| `Vec<T>` | `List<T>` | semantic types |
| `String` | `Text` | semantic types |
| `#[derive(...)]` | `@derive(...)` | `attribute` |
| `#[repr(C)]` | `@repr(C)` | `attribute` |
| `use foo::bar` | `mount foo.bar` | `mount_stmt` |
| `crate` | `cog` | (module system) |

### Built-in Functions and Macros (NO `!` Syntax Anywhere)

Verum does NOT use Rust-style `!` suffix anywhere:

| Rust Syntax (WRONG) | Verum Syntax (CORRECT) | Category |
|---------------------|------------------------|----------|
| `println!("...")` | `print("...")` | I/O (built-in) |
| `format!("x={}", x)` | `f"x={x}"` | Format literal |
| `panic!("error")` | `panic("error")` | Control flow (built-in) |
| `assert!(cond)` | `assert(cond)` | Testing (built-in) |
| `assert_eq!(a, b)` | `assert_eq(a, b)` | Testing (built-in) |
| `unreachable!()` | `unreachable()` | Control flow (built-in) |
| `select!{...}` | `select { ... }` | Async expression |
| `join!(a, b)` | `join(a, b)` | Async function (built-in) |
| `matches!(x, P)` | `x is P` | Pattern test (is operator) |
| `my_macro!(...)` | `@my_macro(...)` | User-defined macro |

**Rule**: All compile-time constructs use `@` prefix: `@derive(...)`, `@const`, `@cfg`, `@sql_query(...)`.

### Reserved Keywords (v5.1)
Only 3 reserved: `let`, `fn`, `is`

### Type Definition Syntax
```verum
// Record type (like struct)
type Point is { x: Float, y: Float };

// Sum type (like enum)
type Option<T> is None | Some(T);
type Tree<T> is Leaf(T) | Node { left: Heap<Tree<T>>, right: Heap<Tree<T>> };

// Protocol (like trait)
type Iterator is protocol {
    type Item;
    fn next(&mut self) -> Maybe<Self.Item>;
};

// Newtype
type UserId is (Int);

// Unit type
type Marker is ();
```

### Rank-2 Polymorphic Function Types
```verum
// Regular function type (rank-1): caller chooses T
type Processor<T> is fn(T) -> T;

// Rank-2 function type: fn<R>(...) - function works for ALL R
// The quantified type parameters scope only within the function type
type Transducer<A, B> is {
    transform: fn<R>(Reducer<B, R>) -> Reducer<A, R>,
};

// Reducer used by transducers
type Reducer<A, R> is fn(R, A) -> R;

// Example: Stateful rank-2 transducer
type StatefulTransducer<A, B, S> is {
    initial_state: S,
    transform: fn<R>(Reducer<B, R>, &mut S) -> Reducer<A, R>,
};
```
Key difference: In `fn<R>(...)`, `R` is universally quantified inside the function type - the caller cannot choose `R`, the function must work for any `R`.

## Philosophy

**Core Principles:**
- **Semantic Honesty**: Types describe meaning (`List`, `Text`, `Map`), not implementation (`Vec`, `String`, `HashMap`)
- **No Magic**: All dependencies explicit via `using [...]`, no hidden state
- **Gradual Safety**: Three-tier references allow performance/safety tradeoff
- **Zero-Cost Abstractions**: CBGR enables memory safety at ~15ns overhead

## Critical Distinctions

### Context System vs Computational Properties

| Aspect | Context System (DI) | Computational Properties |
|--------|---------------------|-------------------------|
| **Purpose** | Runtime dependency injection | Compile-time side effect tracking |
| **Keywords** | `context`, `provide`, `using` | (inferred from code) |
| **Values** | Database, Logger, FS, etc. | Pure, IO, Async, Fallible, Mutates |
| **Phase** | Runtime (~5-30ns) | Compile-time (0ns) |
| **Crate** | `verum_context` | `verum_types/computational_properties.rs` |

```rust
// Function type combines BOTH:
Function {
    contexts: List<Text>,            // DI: using [Database, Logger]
    properties: Option<PropertySet>, // Properties: {Async, IO, Fallible}
}
```

**NEVER** call Properties "Effects" - Verum has no algebraic effects.

### Three-Tier Reference Model (CBGR)

| Tier | Syntax | Overhead | Use Case |
|------|--------|----------|----------|
| 0 | `&T` | ~15ns | Default, full CBGR protection |
| 1 | `&checked T` | 0ns | Compiler-proven safe (escape analysis) |
| 2 | `&unsafe T` | 0ns | Manual safety proof required |

**Memory Layout:**
- `ThinRef<T>`: 16 bytes (ptr + generation + epoch_caps)
- `FatRef<T>`: 24 bytes (ptr + generation + epoch_caps + len)

## Semantic Types (MANDATORY)

```rust
// CORRECT
use verum_std::core::{List, Text, Map, Set, Maybe, Heap, Shared};

// FORBIDDEN - Never use Rust std types
use std::vec::Vec;        // Use List
use std::string::String;  // Use Text
use std::collections::*;  // Use Map/Set
```

## Crate Map (VBC-First Architecture)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           LAYER 4: TOOLS                                │
│  verum_cli ─────────── verum_compiler ─────────── verum_lsp            │
│      │                       │                        │                 │
│      └───────────────────────┼───────────────────────►│                 │
│                              │                        │                 │
│                    verum_interactive ◄────────────────┘                 │
│                    (Playbook TUI, REPL)                                 │
└──────┼───────────────────────┼────────────────────────┼─────────────────┘
       │                       │                        │
┌──────▼───────────────────────▼────────────────────────▼─────────────────┐
│                        LAYER 3: EXECUTION (VBC-First)                   │
│                                                                         │
│  verum_vbc ◄────────────────── verum_codegen                           │
│  (bytecode, interpreter,       (VBC→LLVM for AOT)                      │
│   codegen, intrinsics)                                                  │
│       │                              │                                  │
│       ▼                              ▼                                  │
│  verum_verification          verum_modules                              │
└───────┼──────────────────────────────┼──────────────────────────────────┘
        │                              │
┌───────▼──────────────────────────────▼──────────────────────────────────┐
│                      LAYER 2: TYPE SYSTEM                               │
│  verum_types ◄──────── verum_smt ◄──────── verum_cbgr                  │
│       │                   │ (z3)                │                        │
│       ▼                   ▼                     ▼                        │
│  verum_diagnostics    verum_error                                       │
└───────┼───────────────────┼─────────────────────────────────────────────┘
        │                   │
┌───────▼───────────────────▼─────────────────────────────────────────────┐
│                      LAYER 1: PARSING                                   │
│  verum_fast_parser ◄──── verum_lexer ◄──────── verum_ast               │
│  (main parser)           │ (logos)              │                       │
│       │                  │                      │                       │
│  verum_parser            │                      │                       │
│  (legacy, partial)       │                      │                       │
└───────┼─────────────────────┼──────────────────────┼────────────────────┘
        │                     │                      │
┌───────▼─────────────────────▼──────────────────────▼────────────────────┐
│                      LAYER 0: FOUNDATION                                │
│                         verum_common                                    │
│                         (List, Text, Map, Maybe)                        │
│                                                                         │
│                         core/ (Verum stdlib in .vr)                     │
└─────────────────────────────────────────────────────────────────────────┘
```

### Crate Responsibilities

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| **verum_common** | Semantic types, no deps | `core.rs` (List, Text, Map, Maybe) |
| **verum_cbgr** | Memory safety system | `managed.rs`, `checked.rs`, `unsafe_ref.rs` |
| **verum_ast** | AST definitions | `expr.rs`, `ty.rs`, `pattern.rs`, `decl.rs` |
| **verum_lexer** | Tokenization (logos) | `token.rs`, `lexer.rs` |
| **verum_fast_parser** | Fast recursive-descent parser | `lib.rs`, main parser used |
| **verum_parser** | Legacy parser (partial) | Kept for compatibility |
| **verum_types** | Type checking | `infer.rs`, `unify.rs`, `refinement.rs` |
| **verum_smt** | SMT verification (z3) | `z3_backend.rs`, `verify.rs`, `tactics.rs` |
| **verum_vbc** | **VBC bytecode** (core execution) | `codegen/`, `interpreter/`, `intrinsics/` |
| **verum_codegen** | VBC→LLVM (AOT path) | `llvm/`, VBC lowering to LLVM IR |
| **verum_verification** | Gradual verification | `level.rs`, `vcgen.rs`, `passes/` |
| **verum_modules** | Module resolution | `loader.rs`, `resolver.rs` |
| **verum_compiler** | Compilation pipeline | `pipeline.rs`, `session.rs`, `phases/` |
| **verum_lsp** | IDE support, script parsing | `backend.rs`, `completion.rs`, `script/` |
| **verum_interactive** | REPL and Playbook TUI | `playbook/`, re-exports from verum_lsp |
| **verum_cli** | CLI toolchain | `commands/` (build, run, test, playbook) |

### core/ Directory (Verum Standard Library)

The `core/` directory contains the Verum standard library written in `.vr` files:

| Module | Purpose |
|--------|---------|
| `core/base/` | Core types, protocols (Eq, Ord, Hash, etc.) |
| `core/collections/` | List, Map, Set, Deque |
| `core/mem/` | CBGR allocator, memory management |
| `core/async/` | Futures, Tasks, Channels |
| `core/io/` | File I/O, streams |
| `core/intrinsics/` | Compiler intrinsic declarations |

### External Dependencies

| Library | Version | Crate | Purpose |
|---------|---------|-------|---------|
| **Z3** | 0.19.5 | verum_smt | SMT solving, refinement verification |
| **LLVM** | 21.x | verum_codegen | Native code generation (AOT) |
| **logos** | 0.15.1 | verum_lexer | DFA-based lexer generation |
| **rayon** | 1.11 | verum_compiler | Parallel compilation |

## Performance Targets

```
CBGR check:        < 15ns
Type inference:    < 100ms / 10K LOC
Compilation:       > 50K LOC/sec
Runtime:           0.85-0.95x native C
Memory overhead:   < 5%
```

## Code Standards

### File Organization
- Tests in `tests/`, not inline `#[cfg(test)]`
- Benchmarks in `benches/` (criterion)
- One implementation per feature

### Documentation
```rust
// SAFETY: [reason] - required for unsafe blocks
// Spec: XX-name.md#section - for spec-tied code
```

### Commits
```
feat(crate): Add feature
fix(crate): Fix issue
perf(crate): Optimize by X%
```

## Build Order

```
1. verum_common
2. verum_cbgr, verum_std
3. verum_ast, verum_lexer, verum_parser, verum_diagnostics, verum_error
4. verum_types, verum_smt, verum_modules
5. verum_runtime, verum_codegen, verum_context, verum_resolve, verum_verification
6. verum_compiler (includes derives module), verum_lsp, verum_cli
```

## Reference Documentation

| Topic | Location |
|-------|----------|
| **Verum Grammar** | `grammar/verum.ebnf` |
| Type System | `docs/detailed/03-type-system.md` |
| Syntax | `docs/detailed/05-syntax-grammar.md` |
| Context System | `docs/detailed/16-context-system.md` |
| CBGR | `docs/detailed/26-cbgr-implementation.md` |
| Cog Distribution | `docs/detailed/15-cog-distribution-architecture.md` |
| Cog Management | `docs/detailed/15-cog-management.md` |
| Roadmap | `docs/detailed/28-implementation-roadmap.md` |
| Z3 examples | `experiments/z3.rs/` |
| Inkwell examples | `experiments/inkwell/` |

## VCS: Verum Conformance Suite

The `vcs/` directory contains the comprehensive test and verification infrastructure.

### Directory Structure

```
vcs/
├── specs/                    # Language specification tests (1000+ files)
│   ├── L0-critical/          # Lexer, parser, ownership, memory-safety
│   ├── L1-core/              # Types, refinement, verification
│   ├── L2-standard/          # Async, contexts, modules
│   ├── L3-extended/          # Dependent types, FFI, GPU, meta
│   └── L4-performance/       # Performance benchmarks
├── differential/             # Differential testing (interpreter vs AOT)
│   ├── tier-oracle/          # Reference implementations
│   └── cross-impl/           # Cross-implementation compatibility
├── benchmarks/               # Performance measurements
│   ├── micro/                # Micro-benchmarks (30 tests)
│   └── macro/                # Real-world scenarios (10 tests)
├── fuzz/                     # Fuzzing infrastructure
│   └── seeds/                # Fuzz test seeds
├── runner/                   # Test runners
│   ├── vtest/                # Test execution framework
│   └── vbench/               # Benchmark runner
└── scripts/                  # Automation scripts
```

### Spec Levels

| Level | Purpose | Examples |
|-------|---------|----------|
| **L0-critical** | Must never fail | Lexer tokens, parser AST, memory safety |
| **L1-core** | Type system correctness | Type inference, refinements, generics |
| **L2-standard** | Language features | Async/await, context system, modules |
| **L3-extended** | Advanced features | Dependent types, FFI, GPU compute |
| **L4-performance** | Performance targets | CBGR latency, compilation speed |

### Test File Format (.vr)

```verum
// @test: unit|integration|property|differential
// @tier: 0|1|2|3 (execution tier)
// @level: L0|L1|L2|L3|L4
// @tags: comma, separated, tags
// @timeout: milliseconds
// @expect: pass|fail|error(ErrorType)

fn main() {
    // Test implementation using correct Verum syntax
}
```

### Running VCS

```bash
cd vcs
make test              # Run all tests
make test-l0           # Run L0-critical only
make bench             # Run benchmarks
make fuzz              # Run fuzzer
make differential      # Run differential tests
```

### IMPORTANT: .vr File Syntax

All `.vr` files MUST use correct Verum syntax as defined in `grammar/verum.ebnf`.
**DO NOT** use Rust syntax (struct, enum, impl, trait, Box::new, Vec, String, etc.).
