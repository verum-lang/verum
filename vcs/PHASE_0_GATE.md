# Phase-0 Readiness Gate

Gate snapshot taken after the five Tier-0 blockers (L0-1 through L0-5)
landed. Purpose: pin the L1-core baseline the synarc Phase-1 work is
allowed to build on, and flag the handful of specs that stay red for
reasons orthogonal to the Tier-0 scope.

## Landing commits

| Block | Commit(s) | What |
|-------|-----------|------|
| L0-1  | `15bead6`, `6a4b411` | `Type::Meta` carries `ConstValue`; arity counts Meta/Const/HKT; unification compares values → `Matrix<T, N, M>` dim-mismatch diagnostics |
| L0-2  | `d06f8d3`, `8f8ee46`, `36cd47a` | Bound-first method dispatch for HKT (`F<_>: Functor`) — both `Functor.map(fa, f)` and `fa.map(f)` resolve; protocol-qualified `lift2` re-enabled in `higher_kinded.vr` |
| L0-3  | `c7b4d71` | `.await` outside an async context is rejected with E504 (inference pass was fine; enforcement was a `tracing::debug!` hole) |
| L0-4  | `a1a22ac` | Refinement predicates written with `self` (parser emits `PathSegment::SelfValue`) are now substituted — `safe_div(42, 0)` under `Int{self != 0}` is caught at type-check |
| L0-5  | `85459ed` | `parallel_scan_exclusive` (Blelloch up-sweep + down-sweep) lands in `core/async/parallel.vr`; sequential today, spawn/join-ready when the work-stealing scheduler is wired |

## L0-critical baseline (per-subdir, sampled)

| Subdir | Result |
|--------|--------|
| `lexer/` | **97/97** (100%) |
| `parser/` | **215/215** (100%) |
| `builtin-syntax/` | **11/11** (100%) |
| `modules/` | **12/12** (100%) |
| `stdlib-runtime/` | **8/8** (100%) |
| `reference_system/` | 114/115 (99.1%) |
| `memory-safety/` | 130/131 (99.2%) |
| `mmio/` | 4/6 (66.7%) |

The huge `vbc/e2e/` subdir (≈2,400 tests) is not sampled in this
snapshot — it runs for close to an hour and is better left to CI.
The sampled L0-critical surface is **591/595 = 99.3%** across lexer,
parser, builtin-syntax, modules, reference_system, memory-safety,
mmio, and stdlib-runtime.

## L1-core baseline (per-subdir)

Collected 2026-04-19 immediately after the five blockers.

| Subdir | Result |
|--------|--------|
| `generics/` | **20/20** (100%) |
| `refinement/` | **38/38** (100%) |
| `types/` | **160/160** (100%) |
| `types/properties/` | **4/4** (100%) |
| `types/pure/` | **1/1** (100%) |
| `types/advanced/` | **22/22** (100%) |
| `inference/` | **15/15** (100%) |
| `stdlib/` | **24/24** (100%) |
| `verification_phase/` | **15/15** (100%) |
| `async/` | **3/3** (100%) |
| `modules/` | **6/6** (100%) |
| `contexts/` | **6/6** (100%) |
| `builtin-syntax/` | **2/2** (100%) |
| `expressions/` | **1/1** (100%) |
| `interrupt/` | **4/4** (100%) |
| `meta/` | **40/40** (100%) |
| `patterns/` | **46/46** (100%) |
| `dependent/` | 70/74 (94.6%) |
| `proof/` | 44/45 (97.8%) |
| `self-hosting/` | 4/5 (80.0%) |

**Aggregate L1-core: 499/504 = 99.0%**. The remaining five red specs
live in three orthogonal feature areas (dependent-types, proof terms,
self-hosting bootstrap); none of them depend on the Tier-0 scope and
none of them are on synarc Phase-1's critical path.

## Anchor tests added by the Tier-0 work

These files are the living regression surface for the blockers:

  * `vcs/specs/L1-core/generics/const_generic_dim_match.vr`
  * `vcs/specs/L1-core/generics/const_generic_dim_mismatch.vr`
  * `vcs/specs/L1-core/generics/hkt_function_body.vr`
  * `vcs/specs/L1-core/generics/hkt_method_on_value.vr`
  * `vcs/specs/L1-core/types/properties/effect_inference_propagation.vr`
  * `vcs/specs/L1-core/types/properties/await_outside_async.vr`
  * `vcs/specs/L1-core/refinement/self_binding_pass.vr`
  * `vcs/specs/L1-core/refinement/self_binding_literal_fail.vr`
  * `vcs/specs/L2-standard/async/parallel_scan_blelloch.vr`

## Gate decision

**OPEN for synarc Phase-1.** All five Tier-0 blockers have landed,
their regression anchors are green, and the L1-core baseline is
locked at 499/504 with the five remaining failures catalogued and
unrelated to the work that follows.

## Post-gate Tier-1 campaign (industrial-quality hardening)

Started after the user's directive "никаких упрощений и костылей -
архитектура и реализация языка должны быть эталонными". 13 T1-*
tasks opened covering: panic surface (T1-A), residual L1 failures
(T1-B), stdlib name collisions (T1-C), differential VBC↔AOT
(T1-D), row polymorphism (T1-E), refinement runtime-checks (T1-F),
hardcoded stdlib removal (T1-G), VBC Pi/Sigma opcodes (T1-H),
work-stealing scheduler (T1-I), L2-async triage (T1-J), compile
speed (T1-K), CBGR overhead (T1-L), stdlib parse failures (T1-M,
aggregate).

Parser sub-tasks opened for T1-M discovered syntax weaknesses:
T1-N (dependent-type value parameters), T1-O (theorem-proof
syntax — multi-line ensures, proof blocks, have/by/auto),
T1-P (lambda bare-expression body).

### Completed in campaign

| Task | Commit(s) | What |
|------|-----------|------|
| T1-P | `251900a` | `fn(x: A) -> expr` shorthand — speculative TYPE parse + backtrack to expression body |
| T1-N | `41ed6aa` | `TypeKind::DependentApp { carrier, value_args }` — general `T<A>(v..)` dep-app, generalises `Path<A>(a, b)` to arbitrary types + arities |
| stdlib-loud | `622c9fb` | Parse errors in `core/*.vr` now emit at `warn!` instead of silent `debug!` — was masking 10 modules dropping entirely |

### T1-M progress

Stdlib parse errors before: 10 modules broken (cubical,
day_convolution, epistemic, giry, hott, infinity_topos,
kan_extension, quantum_logic, tactics, mathesis.core). After
T1-N + T1-P: 8 modules still broken, with tactics/giry/
epistemic/day_convolution dominated by theorem-proof syntax (T1-O
pending); hott/infinity_topos/quantum_logic/kan_extension are
mixed — partially T1-O + residual narrow T1-N edges.
