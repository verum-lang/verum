# `meta/mod` audit

Module: `core/meta/mod.vr` (~535 LOC) — module root: 9 submodule
declarations + 21 `public mount` re-exports + MetaError 13-variant
+ MetaResult alias + MetaOutput protocol + VERSION constants.

Tests: 20 unit tests over MetaError 13-variant + variant
disjointness + default constants + VERSION_INFO tuple.

## 1. Cross-stdlib usage

Every consumer of `core.meta.*` accesses the top-level types via
this file's re-exports. Direct submodule mounts (`mount
core.meta.span.MetaSpan`) work too, but the canonical entry is
`mount core.meta.{TokenStream, Token, MetaError, ...}`.

## 2. Crate-side hardcodes

* `verum_compiler::meta_errors::MetaError` mirrors the 13-variant
  error enum + the `.message()` formatting table.
* `verum_compiler::limits::DEFAULTS` mirrors the 4 default constants
  (recursion=256, iteration=1M, memory=64MiB, timeout=30s). These
  ARE configurable per `verum.toml [meta]`, but the defaults must
  match — a drift-pinning Rust unit test is recommended.
* `verum_compiler::version::META_VERSION` mirrors `VERSION_INFO`
  (`(0, 1, 0)` for the v0.1.0 series).

## 3. Language-implementation gaps

### §3.1 `MetaError.message()` uses `f"..."` format strings

The `.message()` body interpolates field values into format
strings; `f"..."` desugars to a cross-module Text builder. The
cross-module fn-return record-layout defect (see
[meta/span audit §3.1](../span/audit.md)) blocks property tests of
`.message()` output until the defect closes.

The 13-variant test in this folder uses `is`-checks, not
`.message()` field access, so it works around the defect.

### §3.2 `MetaResult<T>` is a type alias, not a newtype

`public type MetaResult<T> is Result<T, MetaError>` — pure type
alias. No methods of its own; `MetaResult<T>` and
`Result<T, MetaError>` are interchangeable. No surface to test
beyond confirming that a `MetaResult<T>` value can be constructed
and pattern-matched as `Ok(v)` / `Err(e)`. Covered implicitly via
the parse-tests in other meta submodules.

### §3.3 `MetaOutput` protocol has zero implementers at the stdlib layer

```verum
public type MetaOutput is protocol {
    meta fn emit(&self) using AstAccess;
};
```

Built-in impls (`TokenStream`, `()`, `Result<T, E> where T: MetaOutput`)
are claimed in the docstring but not declared in `mod.vr`. The
impls live in the compiler as `@compiler_provided` shims; no
runtime test surface.

### §3.4 Submodule declaration ordering

The 9 `public module X;` declarations are alphabetised neither
nor topologically-ordered. The current order is:

```
contexts → span → token → quote → reflection → attribute →
diakrisis_attrs → framework_hygiene → oracle → tactic
```

Module-load order is independent of declaration order (the
loader uses topo-sort via `augment_dependencies_from_mounts`), so
this is cosmetic. Suggested topological order for readability:

```
span → token → attribute → reflection → quote → contexts →
diakrisis_attrs → framework_hygiene → oracle → tactic
```

## Action items landed in this branch

* `core-tests/meta/mod/unit_test.vr` — 20 unit tests over:
  - MetaError 13-variant ctors (AssetNotFound, AssetReadError,
    SyntaxError, ParseFailed, TypeError, RecursionLimit,
    IterationLimit, MemoryLimit, Timeout, InvalidOperation,
    CacheError, MethodNotFound, Other)
  - Pairwise disjointness across resource + io families
  - Default constants: DEFAULT_RECURSION_LIMIT (256) /
    DEFAULT_ITERATION_LIMIT (1M) / DEFAULT_MEMORY_LIMIT (64MiB) /
    DEFAULT_TIMEOUT_MS (30K)
  - Relative ordering of the 4 defaults
  - VERSION_INFO tuple (0, 1, 0)
* `core-tests/meta/mod/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test: `MetaError.message()` is non-empty for every variant (§3.1) | this folder | 30 min after cross-module fix |
| Drift-pinning Rust unit test for DEFAULT_* constants (§3.2 in crate-side) | crates/verum_compiler/src/limits.rs | 30 min |
| Topological reordering of submodule declarations (§3.4) — cosmetic | core/meta/mod.vr | 5 min |
| Integration test: `MetaResult<TokenStream>` round-trip — `Ok(ts)` survives through a `try_*` chain | this folder | 1 h post-cross-module-fix |
