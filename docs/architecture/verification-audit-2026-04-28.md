# Verification-Implementation Audit (2026-04-28)

Comprehensive audit of `core/` theorems / lemmas / axioms vs.
industrial proof-assistant standards (Lean 4 / Coq / Agda /
Isabelle/HOL).

## Executive Summary

| Metric | core/ | Industrial standard | Gap |
|--------|-------|---------------------|-----|
| Theorems | 77 | should be ≥ 80% of total | — |
| Axioms | 299 | should be ≤ 20% of total | **80% are axioms** |
| Tautological axioms (`ensures true`) | 70 | 0 | **70 hard-coded admits** |
| Theorems with empty proof body | 6 | 0 | 6 untested |
| Theorems with multi-step proof | 44/77 (57%) | ≥ 95% | 33 single-line |
| Apply-chain proofs | 8 | the default pattern | minimal usage |
| `apply <named-lemma>` discharge | minimal | every non-axiom step | structural gap |

## Industrial-Standard Comparison

### Lean 4 (mathlib4)

Every `theorem` carries a constructive term-mode definition or an
explicit tactic block; every `axiom` is justified by a comment
citing the ZFC/HoTT result it admits. `Mathlib.SetTheory.Cardinal`
contains ~50 axioms in 23K lines (≈0.2%). Verum's ratio is **400x worse**.

### Coq (mathcomp / Coq.Init)

The kernel has 4 axioms (Prop_extensionality, propositional_extensionality,
ProofIrrelevance, classic via Hilbert ε). Every other lemma is
proven with `Qed.`. Use of `Admitted.` is gated on PR review.

### Agda (cubical-stdlib)

`--safe` flag forbids postulates; postulates that DO appear are
listed in `Postulates.agda` with explicit `{-# WARNING_ON_USAGE #-}`
to surface the trusted boundary.

### Isabelle/HOL (HOL.thy)

Foundational axioms: 7 (Hilbert ε / extensionality / well-foundedness /
infinity / Choice / Arity / Type-Definition). Every other result is
`by(...)` discharged.

## Verum core/ Theorem-Body Honesty Distribution

| Body shape | Count | Industrial equivalent | Honesty |
|------------|-------|----------------------|---------|
| `proof { true }` (single-line) | ~24 (post recent V2 work) | Coq `reflexivity.` | **conditionally honest** — works iff ensures β-reduces to true |
| `proof { let ...; <ensures-expr> }` (multi-step) | 4 (just shipped in pure.vr) | Coq tactic chain | **honest** — exhibits reduction |
| `proof { apply <lemma>(args); }` | 8 | Coq `apply X.` | **honest** — delegates to named lemma |
| Empty proof body | 6 | (none) | **stub — needs strengthening** |
| `ensures true; (axiom)` | 70 | Lean `axiom : True` | **dishonest framework hook** |

## Critical Gaps

### Gap 1 — 70 tautological framework-citation axioms

**Files**: math/, theory_interop/, action/ (largely closed in this session)

These declare a framework dependency via `@framework(name, "citation")`
but their `-> Bool ensures true` shape asserts NOTHING about the
result. An adversary inheriting these axioms can derive any
proposition by chaining citations. **In Lean 4 this would require
`postulate` syntax with explicit module-level review.**

Status (as of 2026-04-28):
- Closed this session: 47 axioms in `core/action/{monads,effects,ludics,ludics_lazy}` (commits b506a9b5 + ddec8638 + f41fe593 + 3fc5255f).
- Remaining: ~23 in `core/theory_interop/` (#39 pending) + math/ frameworks.

### Gap 2 — single-line `proof { true }` bodies don't EXHIBIT reduction

**Files**: action/monads (largely shipped V2 promotion in this session)

Even when the kernel β-discharges the obligation, the proof body
should NAME the intermediate reducts via `let` bindings so the
proof tree is auditable. Pre-this-session 100% of monad theorems
shipped without intermediate `let`s; commit `3fc5255f` (this session)
demonstrated the pattern for `core/action/monads/pure.vr` (4 theorems
gained 15 let-bindings + step comments).

**Standard**: every proof body should have ≥ 1 `let` binding per
β / ι / δ reduction step the kernel performs to discharge the
obligation. Today's `core/math/s_definable/lemma_3_4.vr` has the
correct shape (3-step chain into `syn_mod_lemma_3_4_steps_2_3`).

### Gap 3 — Empty proof bodies (6 theorems)

These are theorems whose `proof { }` block contains zero
statements. The kernel must discharge them via β alone — but with
no `let` bindings to anchor the reduction, the trust boundary is
opaque. **These should be either filled with reduction chains or
demoted to `axiom` with a framework citation.**

### Gap 4 — Audit-trail integration with K-rule V2/V3

The kernel V2/V3 promotions (this session: K-Round-Trip, K-Eps-Mu,
K-Refine, K-Universe-Ascent) carry `BridgeAudit` trails. But the
corpus-side @theorem proofs have no mechanism to surface which
bridge admits their reduction relies on. **Industrial parallel**:
Lean 4's `#check @sorry` raises a warning per usage; Verum needs
`verum audit --bridge-admits` (shipped 2026-04-28) to walk the
corpus and report.

### Gap 5 — No CI gate on theorem-vs-axiom ratio

Every PR that adds a new theorem-citation should be required to
either ship a real proof body or justify the axiom in PR description.
Today there's no CI check enforcing this — `verum audit
--proof-honesty` exists but isn't blocking.

## Prioritised Remediation Roadmap

### Tier 0 — IMMEDIATE (this session continuation)

1. ✓ V2 promotion of action/ tautologies → witness-parameterised
   theorems (47 axioms closed in 4 commits).
2. ✓ Multi-step proof bodies for action/monads/pure.vr (4 theorems
   gained 15 let-bindings).
3. (next) Multi-step proof bodies for action/monads/{reader, writer,
   state, probability, quantum} — same pattern as pure.vr.
4. (next) Multi-step proof bodies for action/effects.vr (8 short bodies
   need expansion).

### Tier 1 — NEXT WEEK

1. Promote remaining `theory_interop/` tautological axioms (#39 — 37 axioms).
2. CI gate: `verum audit --proof-honesty` blocking PRs with `axiom +
   ensures true` shape.
3. CI gate: `verum audit --bridge-admits` reporting trusted-boundary
   delta per PR.
4. Document each preprint-blocked admit in
   `docs/architecture/diakrisis-bridge-roster.md` with V3 promotion path.

### Tier 2 — NEXT MONTH

1. Constructive promotion: every @theorem proof body that currently
   reads `proof { true }` augmented with the reduction chain via
   explicit `let` bindings (matching `pure.vr` pattern). Target: 95%
   of theorems have ≥ 1 let-binding per β-step.
2. Audit `core/math/frameworks/` for `ensures true` axioms — these
   are real framework citations; classify into:
   - **Postulate** (Lean's `axiom` — admits are documented assumptions)
   - **Theorem** (proof should exist — demand a body)
   - **Bridge admit** (preprint-blocked — surface via `BridgeAudit`)

### Tier 3 — NEXT QUARTER

1. Industrial-grade tactic infrastructure:
   - `auto` tactic that searches theorem database for matching
     ensures clauses (Coq's `auto` analogue).
   - `induct` tactic for inductive types (Coq's `induction`).
   - `decide` tactic for Bool-valued propositions (Lean's `decide`).
2. Dependent-type elaboration: every `requires`/`ensures` clause
   should support universe-polymorphic types (Lean's universe levels).
3. Bridge-admit removal track: each Diakrisis bridge admit
   (16.7 / 16.10 / 14.3 / A-3 / 131.L4) gets a V3 promotion task
   when the corresponding preprint result lands.

### Tier 4 — MULTI-MONTH

1. Full corpus-wide proof-honesty audit: every theorem ships with
   a multi-step proof body OR an explicit axiom-with-citation
   demotion + PR review.
2. Cross-format CI: every promoted theorem must round-trip through
   `verum_codegen::proof_export` to Lean 4 / Coq / Agda / Dedukti /
   Metamath, and the downstream verifier must accept.

## Honest Self-Assessment

The work shipped this session moved **47 tautological axioms** to
witness-parameterised theorems (all of `core/action/` closed). The
multi-step proof-body pattern was demonstrated on 4 theorems
(pure.vr). **The remaining 33 single-line `proof { true }` theorems
shipped today require the same multi-step rewrite to meet
industrial standard.** They are:
- Honest in SIGNATURE (ensures clause states the law)
- Trivial in PROOF BODY (kernel β-discharges via `true` return)
- Closer to Coq's `reflexivity.` than to a real tactic chain

The path to "industrial proof-assistant grade" is multi-month — this
audit's prioritised roadmap traces it. The first concrete next steps
are Tier 0 items 3+4: extend the pure.vr multi-step pattern across
the remaining 5 monad files + effects.vr.
