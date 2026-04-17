# Contributing to Verum

Thanks for your interest in Verum. This guide covers the practical
machinery — how to build, run tests, validate changes locally, and
what the project expects from a PR.

## Prerequisites

- Rust toolchain: **nightly**, pinned by `rust-toolchain.toml`. Running
  `cargo` in the repo auto-installs the correct channel.
- LLVM 21.x built locally under `llvm/install/` (see
  `llvm/build.sh`). `verum_llvm_sys` and `verum_codegen` link against
  it.
- Z3 (for SMT verification) — ships vendored via `z3-sys`.
- macOS / Linux. Windows is not currently tested on CI.

## Quick Build

```bash
# Workspace build. Must produce zero non-vendored Rust warnings —
# the CI `unit` job blocks on RUSTFLAGS=-D warnings.
cargo build --workspace

# Fast-path: rebuild only the CLI.
cargo build -p verum_cli --release
```

## Running the Test Suites

Verum has **two** regression-prevention layers. Both should pass
before submitting a PR.

### Rust unit tests

```bash
cargo test --workspace --lib --bins
```

Covers parser, type inference, SMT translation, VBC bytecode,
CBGR, codegen. ~850 tests in `verum_vbc` alone.

### VCS — Verum Conformance Suite

The language-level acceptance suite. Specs live in `vcs/specs/*` and
are classified by level (L0 critical, L1 core, L2 standard, L3
extended, L4 performance).

```bash
cd vcs

# Full gate (CI-equivalent — L0 100% + L1 100% + L2 >=95% + differential)
make ci-gate

# Individual levels
make test-l0            # ~2918 specs, strict pass
make test-l1            # ~499 specs, strict pass
make test-differential  # Tier 0 interpreter vs Tier 3 AOT

# Quick smoke (lexer + parser only, few seconds)
make test-quick
```

Baseline pass rates: see `vcs/baselines/l0-baseline.md`.

### Fuzzing

```bash
cd vcs && make fuzz           # 10 min, lexer + parser
cd vcs && make fuzz-cbgr      # CBGR memory-safety fuzzer
cd vcs && make fuzz-overnight # 8h run, all targets
```

## Writing a `.vr` Spec

All `.vr` files MUST follow `grammar/verum.ebnf`. Frontmatter drives
the test runner:

```verum
// @test: typecheck-pass|run|differential|...
// @tier: 0|1|2|3|all
// @level: L0|L1|L2|L3|L4
// @tags: memory-safety, async, ...
// @timeout: 5000
// @expected-stdout: hello\n
// @expected-exit: 0

fn main() {
    // ...
}
```

Details: `vcs/CLAUDE.md` has the full reference.

## Architectural Boundaries

These rules are enforced by code review, not the compiler — violating
them creates brittle implementation dependencies that surface as
runtime bugs later.

- **Do not hardcode stdlib type knowledge in the compiler.** Affine
  types, variant constructors, protocol implementations, and method
  availability are discovered from `.vr` source, not listed in
  `verum_types` / `verum_vbc`. History: hardcoded `Heap`/`File`/etc.
  affine lists and `None`/`Some`/`Ok`/`Err` variant protection have
  been repeatedly deleted. See `crates/verum_types/src/CLAUDE.md`.
- **Do not use Rust `std::vec::Vec` / `String` / `HashMap`** in new
  Verum-facing code. Use `verum_common::{List, Text, Map, Maybe}`
  (semantic types from the stdlib mirror). Compiler-internal types
  can use `Vec`/`HashMap` if they are never exposed across the
  language boundary.
- **Verum syntax is NOT Rust.** `type Name is { ... };`, not `struct
  Name { ... }`. `type Name is A | B;`, not `enum Name { A, B }`.
  `implement Name { ... }`, not `impl`. No `!` macro syntax
  anywhere — use `@macro(...)` and `f"..."` format literals. See
  `CLAUDE.md` and `grammar/verum.ebnf`.

## Commit Style

```
feat(crate): describe the new feature
fix(crate): describe the bug + the user-visible symptom
perf(crate): describe the measured win (include numbers)
docs(topic): describe the doc change
chore(scope): tooling / build hygiene only
ci(scope): CI pipeline changes
```

Commits are merged directly on `main`; no PR-squash convention, so
write commit messages that would make sense in the long term.

## Before Sending a PR

```bash
# 1. Zero warnings on the unit build (matches CI gate)
RUSTFLAGS="-D warnings" cargo build --workspace --locked

# 2. Unit tests pass
cargo test --workspace --lib --bins

# 3. VCS L0 + L1 (strict)
cd vcs && make test-l0 test-l1

# 4. Clippy + fmt (advisory, but run before landing)
cargo clippy --workspace --all-targets
cargo fmt --all
```

## Where to Ask

- `grammar/verum.ebnf` — authoritative language syntax.
- `CLAUDE.md` — project-wide design notes + architectural invariants.
- `docs/detailed/` — type system, context system, CBGR, cog
  distribution.
- `vcs/CLAUDE.md` — test runner reference.
- Interpreter feature compatibility (GPU / dynamic FFI / `vmap`/`pmap`
  fall back to AOT) is documented in
  [Runtime tiers](https://verum-lang.dev/docs/architecture/runtime-tiers).
