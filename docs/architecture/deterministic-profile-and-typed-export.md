# Deterministic profile, total arithmetic, bounded loops, and the typed-IR export

**Status:** design accepted 2026-07-30; staged in the task pool as T0671–T0677.

Verum is a general systems language. Some consumers of compiled code — consensus
systems, safety-critical controllers, auditable pipelines, reproducible-build
environments — need a *subset* of it with hard guarantees: determinism, totality,
proof-carrying arithmetic, and a machine-readable typed artefact they can lower
themselves. This document defines those capabilities as **general language and
toolchain features**. Verum knows nothing about any particular downstream IR or
platform; external code generators consume the typed export (§4) out of tree.

## 1. Verification-strategy wiring (T0671)

`@verify(thorough)` is documented as *formal + mandatory invariant / frame /
termination obligations* (`grammar/verum.ebnf:478-479`). The ladder dispatcher
already routes Thorough through kernel re-check → SMT portfolio → tautology
decider; what completes the contract is **mandatoriness**:

- under Thorough, every loop yields a Termination obligation — a loop with **no**
  `decreases` measure is a *failing* obligation, not silence;
- a `DispatchPending` verdict is a build **failure** under Thorough/Certified,
  never a skip;
- the gate is proven by a two-direction test: an unproven measure fails the
  build, the same function with a valid `decreases` passes. A strategy that only
  ever accepts is indistinguishable from one that is not wired.

`@verify(certified)` = Thorough with the kernel route mandatory (the proof
backend must close it; SMT alone is insufficient).

## 2. The deterministic profile (T0672)

A **profile** is a named restriction set applied to a compilation unit
(`--profile deterministic`) or an item/module (`@profile(deterministic)` — plain
attribute syntax, no grammar change). Restrictions are expressed in the
language's own vocabulary — contexts, computational properties, types,
constructs — and each violation is a compile error **naming the construct and
the profile rule**, with one test per construct. A restriction list that
silently admits one construct is worse than none.

`deterministic` forbids, at compile time:

| Construct | Why | Detected via |
|---|---|---|
| floating-point types/literals/ops | cross-machine FP divergence (modes, contractions) | type system |
| ambient time | reads a clock the result then depends on | contexts + intrinsics |
| randomness | ditto | contexts + intrinsics |
| FFI / `extern` | unbounded, unauditable | decl walk |
| I/O | result depends on the world | IO context/property |
| `spawn` / `async` / threads | scheduling nondeterminism | Async property + constructs |
| unbounded loops / recursion without a measure | totality (§1, §3) | Termination obligations |

The profile composes with `@verify`: `--profile deterministic` implies the
Termination-mandatory behaviour of Thorough for the profiled unit.

## 3. Total arithmetic (T0673) and bounded loops (T0674)

Under the deterministic profile, partial arithmetic is a proof obligation, not a
runtime surprise:

- `a / b` and `a % b` require the divisor provably nonzero — a nonzero literal,
  a refinement-typed operand (`Int{ it != 0 }`), or a dominating check the
  verifier accepts. Otherwise: compile error suggesting the refined type.
- `+`, `-`, `*` over fixed-width integers require a no-overflow proof or an
  explicit `checked_*` / `wrapping_*` call — the author states intent; silence
  is an error.

Both reuse the existing refinement/SMT obligation machinery — no parallel
checker.

**Bounded loops:** a loop whose proved `decreases` measure has a compile-time
constant bound `k` is *boundedly total*; the toolchain exposes `k` through a
stable API (per-loop: `ConstantBound(k)` | `FiniteNotConstant` | `Unproven`).
`FiniteNotConstant` carries the plain-language diagnostic ("loop bound is not a
compile-time constant") so downstream consumers that require constant bounds
can reject with the author-actionable message.

## 4. The typed-IR export (T0675) — the external-backend seam

`verum export --typed-ir <file> -o <artefact>` emits a **canonical, versioned,
self-describing** serialization of the *checked* program — after type checking,
before any bytecode/native lowering — carrying what downstream code generators
need and the compiler already knows:

- items with full types, **refinements**, **capability sets**, declared
  **contexts** (`using`), inferred **computational properties**;
- attributes verbatim (including `@effect(<kind>)` and custom attributes —
  the extension point external toolchains key on);
- loop metadata: the `decreases` measure and its §3 bound classification;
- structured bodies (statements/expressions) for profiled items;
- no timestamps, no absolute paths, canonical field and map ordering.

The export schema is an **explicit conversion layer**, not a derive over
internal AST types — the schema is the stability boundary, versioned
independently of compiler internals. External backends live out of tree and
consume the artefact; Verum's contract to them is exactly: schema stability,
canonical bytes, and the guarantees of §§1–3 recorded in the artefact.

Prior art followed: `proof_export.rs` (typed AST → external formats) for shape,
the VBC archive writer for canonical-bytes discipline.

## 5. Reproducibility (T0677)

Two runs of `verum export --typed-ir` over the same source produce
**byte-identical** artefacts — asserted in CI (double-compile + hash compare).
Any nondeterminism found (map iteration order, parallel ordering, environment
leakage) is named and removed, not tolerated. This is a general compiler
property: content-addressed caching, audit trails, and reproducible builds all
rest on it.

## 6. Capability propagation (T0676)

The capability system (`crates/verum_types/src/capability.rs` — attenuation,
intersection, propagation through calls) must demonstrably survive the
archive/bake round-trip, so that a library module's capability-typed signatures
are readable by consumers of §4 exports. Verified by test, not assumed; any gap
found is fixed at source or filed with a measured repro.
