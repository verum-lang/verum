# `intrinsics/runtime/tier` audit

Module: `core/intrinsics/runtime/tier.vr` (~55 LOC) — execution-tier
introspection: `is_interpreted`, `get_tier`, `tier_promote`.

Tests: unit (5) + property (4) + integration (3) + regression (1) — 13/13
green on the interpreter at first run.

## 1. Defects FIXED on this branch (2026-07-03)

### TIER-TEST-CONTRACT-1 — the original suite violated the both-tiers contract

The pre-existing unit tests asserted `assert(is_interpreted())` and
`assert_eq(get_tier(), 0_u8)` — satisfiable ONLY under `--interp`, in a
suite whose contract is "the same source passes both tiers".  Rewritten
tier-agnostic: canonical-code membership (`t == 0 || t == 3`), coherence
(`is_interpreted() ⇔ get_tier() == 0`), stability across 100 reads, call
depths, methods, closures, and stdlib data flow.  Pinned in
regression_test.vr.

## 2. Contract notes

* Codes: 0=interpreter, 1=JIT baseline, 2=JIT optimized, 3=AOT.  Under
  `verum test` only {0, 3} can serve a test process — membership pinned.
* `tier_promote(func_id)` is an advisory hint; unknown ids must be ignored,
  not fault (pinned by the no-fault smoke test).  Observable promotion
  effects belong to `vcs/specs/L2-standard`.

## 3. Cross-stdlib usage

| consumer | how |
|---|---|
| diagnostics / tracing | tier tagging of samples. |
| adaptive stdlib paths | interp-vs-AOT algorithm selection (none today; the coherence law keeps the door honest). |

## 4. Crate-side hardcodes / drift surfaces

* Interpreter: `get_tier` handler returns 0; `is_interpreted` true.
* AOT: both lower to compile-time constants (3 / false) — divergence
  between the two intrinsics is impossible only while BOTH lower from the
  same tier authority; the coherence law is the tripwire.

## 5. Action items

**Landed this branch**
* TIER-TEST-CONTRACT-1 rewrite + full suite (13 tests).

**Deferred**
* JIT-tier (1/2) reporting once a JIT serves test processes.
