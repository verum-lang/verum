# Verum Language Platform

## CRITICAL: No libc in interpreter or AOT

**Architectural invariant**: Verum's VBC interpreter (Tier 0) and
AOT-compiled binaries (Tier 1) MUST NOT call into libc.  All
runtime functionality goes through:

* **Linux**: direct syscalls via `syscall` / `svc #0` instructions.
* **macOS**: libSystem.B.dylib only (Apple-required boundary, NOT
  libc in the glibc/musl sense).
* **Windows**: kernel32.dll + ntdll.dll only (no MSVC CRT, no UCRT).
* **FreeBSD**: direct syscalls.
* **Embedded**: bare-metal, no OS dependencies.

See `docs/architecture/no-libc-architecture.md` for the full
ruleset, verification procedure (`ldd` / `otool` / `dumpbin`), and
the remaining migration punch-list.

When emitting LLVM IR, every per-platform decision (syscall
numbers, sockaddr layout, errno-fn name, socket-option constants,
…) reads `module.get_triple()` — the **target** triple — never
host `#[cfg(target_os = "...")]` directives.  HOST gates miscompile
cross builds.  Helpers in
`crates/verum_codegen/src/llvm/target_triple.rs`
(`target_is_linux` / `target_is_darwin` / `target_is_windows` /
`target_is_aarch64` / `target_is_x86_64`) are the canonical
inspection API.

## CRITICAL: Multi-session coordination (task pool)

Several sessions (Claude agents and humans) work on this repo in
parallel. All cross-session work items live in ONE shared task pool;
per-session task numbering is FORBIDDEN (the duplicate `#N` labels in
old commit history came from it).

* Pool: `<main-checkout>/.taskpool/` — reachable from any worktree as
  `$(git rev-parse --git-common-dir)/../.taskpool` (machine-local,
  gitignored). CLI: `scripts/taskpool/tp`. Full protocol:
  `docs/architecture/multi-session-taskpool.md`.
* IDs are global and monotone (`T0123`), allocated by `tp new` — never
  invented by hand. Every commit that works a task references it in
  the subject: `fix(vbc): ... (T0123)`; finish with
  `tp done T0123 -c <sha>`.
* Claim before you work (`tp claim T0123` or `tp claim --next`) —
  claims are atomic, exactly one winner. Blocked → `tp release` with a
  note. Found an unrelated defect → file it with `tp new` instead of
  stretching your claim.
* Session bootstrap: `scripts/taskpool/tp status && scripts/taskpool/tp list open`.
* Working-tree hygiene (each rule exists because its violation burned
  a real session):
  - Stage only your own paths; NEVER `git add -A`; never commit,
    stash, or revert changes you did not make.
  - Build/test with a session-private `CARGO_TARGET_DIR` (e.g. under
    your scratchpad); never trust binaries in the shared `target/`.
  - Never run two AOT test suites concurrently on this machine.

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
| **Crate** | (context runtime lives in verum_compiler/verum_vbc) | `verum_types/computational_properties.rs` |

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
- `FatRef<T>`: 32 bytes (ThinRef + metadata:8 + offset:4 + reserved:4)

## Semantic Types (MANDATORY)

```rust
// CORRECT
use verum_common::{List, Text, Map, Set, Maybe, Shared};

// FORBIDDEN - Never use Rust std types
use std::vec::Vec;        // Use List
use std::string::String;  // Use Text
use std::collections::*;  // Use Map/Set
```

## Crate Responsibilities (VBC-First Architecture)

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| **verum_common** | Semantic types, no deps | `semantic_types.rs` (List, Text, Map), `types.rs`, `shared.rs`; `Maybe<T>` is a type alias in `lib.rs` |
| **verum_cbgr** | Memory safety system | `escape_analysis.rs`, `ownership_analysis.rs`, `tier_analysis.rs` |
| **verum_ast** | AST definitions | `expr.rs`, `ty.rs`, `pattern.rs`, `decl.rs` |
| **verum_lexer** | Tokenization (logos) | `token.rs`, `lexer.rs` |
| **verum_fast_parser** | Fast recursive-descent parser | `lib.rs`, main parser used |
| **verum_parser** | IDE parser: lossless + incremental | `syntax_bridge` (LosslessParser), `IncrementalDocument`; used by verum_lsp, NOT the compile path |
| **verum_types** | Type checking | `infer/` (dir), `unify.rs`, `refinement.rs` |
| **verum_smt** | SMT verification (z3) | `z3_backend.rs`, `verify.rs`, `tactics.rs` |
| **verum_vbc** | **VBC bytecode** (core execution) | `codegen/`, `interpreter/`, `intrinsics/` |
| **verum_codegen** | VBC→LLVM (AOT path) | `llvm/`, VBC lowering to LLVM IR |
| **verum_verification** | Gradual verification | `level.rs`, `vcgen.rs`, `passes/` |
| **verum_modules** | Module resolution | `loader.rs`, `resolver.rs` |
| **verum_compiler** | Compilation pipeline | `pipeline.rs`, `session.rs`, `phases/` |
| **verum_lsp** | IDE support, script parsing | `backend.rs`, `completion.rs`, `script/` |
| **verum_interactive** | REPL and Playbook TUI | `playbook/`, re-exports from verum_lsp |
| **verum_cli** | CLI toolchain | `commands/` (build, run, test, playbook) |
| **verum_kernel** | Proof kernel (trusted core) | `proof_tree.rs` (KernelRule) |
| **verum_syntax** | Lossless red-green tree parser infra | grammar-facing surface |
| **verum_dap** | Debug Adapter Protocol | debugger integration |
| **verum_protocol_types** | Shared protocol type defs | LSP/DAP wire types |
| **verum_stdlib_precompiler** | Bakes core/ into the embedded .vbca archive | build-time tool |
| **verum_core** | Core support crate | shared runtime pieces |
| **verum_integration_tests** | Cross-crate integration tests | test-only crate |

The `core/` directory is the Verum standard library written in `.vr` files
(one subdirectory per module; see `core/`). External dependency versions:
the workspace `Cargo.toml` is authoritative.

## Performance Targets

```
CBGR check:        < 15ns       (measured ~0.93ns — production_targets bench)
Type inference:    < 100ms / 10K LOC
Compilation:       > 50K LOC/sec (measured ~1.4M LOC/sec parse — gated by tests/compilation_speed_contract.rs)
Runtime:           1x native C — parity is the bar, not the ceiling (>1x sought via whole-program opt)
Memory overhead:   < 5%
```

## Code Standards

### File Organization
- Tests in `tests/`, not inline `#[cfg(test)]`
  - **`tests/` DOES gate — the note that said otherwise was stale.**
    `.github/workflows/ci.yml` runs `--tests` for the pure tier plus
    `verum_types`, `verum_verification`, `verum_smt`, `verum_vbc`,
    `verum_lsp` and `verum_codegen` (jobs `integration` and
    `integration-vbc`), on top of the `--lib --bins` unit job. That
    campaign is T0709; check the job list before assuming a suite is
    inert, and say in the commit which job runs your gate.
  - Still NOT gated on a PR: the AOT-heavy crates (`verum_cli`,
    `verum_compiler`, `verum_codegen`'s AOT suites,
    `verum_integration_tests`), which run in `nightly-aot.yml` as a
    non-blocking measurement lane.
- Benchmarks in `benches/` (criterion)
- One implementation per feature

### Documentation
```rust
// SAFETY: [reason] - required for unsafe blocks
// Spec: <spec-name> §section - for spec-tied code (logical name, never a path)
```

**Public-file hygiene (STANDING)**: tracked files must NEVER reference the
`internal/` directory — state the requirement in place, or cite a public doc
(`docs/architecture/*.md`, `grammar/verum.ebnf`, `website:docs/...`).
Gate: `make check-internal-refs`.

### Commits
```
feat(crate): Add feature
fix(crate): Fix issue
perf(crate): Optimize by X%
```

## Reference Documentation

| Topic | Location |
|-------|----------|
| **Verum Grammar** | `grammar/verum.ebnf` |
| **Live status / debt** | `docs/architecture/tech-debt-register.md` (LIVING) + `core-tests/INVENTORY.md` (per-module conformance truth) |
| **Multi-session task pool** | `docs/architecture/multi-session-taskpool.md` + `scripts/taskpool/tp` |
| Type System | `docs/architecture/unified-type-theory.md`; the implementation is `crates/verum_types/src/infer/` |
| Syntax | `grammar/verum.ebnf` (the ONLY source of truth) + `docs/by-example/` |
| Context System | specified in place in this file (§ Context System vs Computational Properties) + `docs/by-example/16-context-system/` |
| CBGR | specified in place in this file (§ Three-Tier Reference Model) + `docs/by-example/14-cbgr-references/` |
| **Intrinsic Dispatch Contract** | `docs/architecture/intrinsic-dispatch-contract.md` — body `@intrinsic` vs table authority, LLVM-canonical alias requirements, `static mut` cell-backed address-of, CBGR-ref bound-check, three-tier reference dispatch. Pinned rules with regression-test references. |
| **Value-Copy Contract** | `docs/architecture/value-copy-contract.md` — what copying a value MEANS, and the seven syntactic positions that copy: places copy, temporaries do not; `Shared<T>` bumps its refcount; containers copy their spine; references copy as references. One carrier (`value_copy`), pinned by unit tests + spec 628. |
| **FFI Byte-Buffer Contract** | `docs/architecture/ffi-byte-buffer-contract.md` — how byte buffers cross the Verum↔C ABI: packed `[Byte;N]` (`TypeId::U8`) vs NaN-boxed `List`; reserved-stride `FatRef`; `.as_mut_ptr()` on a subslice param (never a raw array, never `transmute`); per-platform `sockaddr` layout. Root of the B1 net-stack cascade; the `&arr`-vs-`&arr[..]` coercion footgun (task #24). |
| Cog system | `crates/verum_modules/` (loader/resolver) + `docs/by-example/10-mount-system/` |
| Roadmap / status | there is NO trusted roadmap document — implementation status truth is `docs/architecture/tech-debt-register.md` + `core-tests/INVENTORY.md` |
| SMT backend examples | `experiments/smt.rs/` |
| verum_llvm fork  | `crates/llvm/verum_llvm/` (in-tree LLVM bindings; do NOT use `inkwell`) |

## VCS: Verum Conformance Suite

`vcs/` holds the spec-test and verification infrastructure. Everything —
test-type/level tables, `@test:`/`@tier:` directive format, runner and
Makefile invocations — is documented in `vcs/CLAUDE.md`, which loads
automatically when working under `vcs/`. All `.vr` test files follow the
grammar rules above (`grammar/verum.ebnf`), never Rust syntax.
