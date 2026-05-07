# Verum Kernel Audit — 2026-05-08

> **Status:** load-bearing. This document records every kernel-level
> defect surfaced by the May 2026 cross-prover audit, the fundamental
> fix that landed for each, and the regression test that locks it
> down. The goal is not "patches"; the goal is that none of these
> defect classes can recur silently in any future revision of the
> kernel without the audit gate noticing.

## 1. Methodology

Three external proof assistants — **Lean 4 (lake build)**,
**Coq / Rocq 9.x (`coqc`)**, and **Isabelle/HOL** — replay the
kernel-soundness corpus on every release via
`verum audit --external-prover-replay`. The shape-only export
(written for ~12 months) had hidden a number of real defects in the
kernel and the emitter; this audit forced every defect into the open
by demanding that the foreign prover *actually accept* the export.

Two parallel readings of `crates/verum_kernel/src/proof_checker.rs`
(the 826-LOC trusted base) were performed in addition to the foreign
replay: a **bidirectional algorithm trace** (one rule at a time,
checking soundness invariants) and a **adversarial-input survey**
(how does the kernel behave on inputs that are intentionally hostile
to its meta-theoretic assumptions?).

Findings group into two classes:

* **Soundness gaps** (DEFECT-2, DEFECT-4) — the kernel could
  silently accept ill-formed inputs.
* **Robustness / completeness gaps** (DEFECT-1, DEFECT-3) — the
  kernel could either reject valid inputs or hang on hostile ones.

A fifth class (DEFECT-0) covers the *meta-theoretic* gap that
existed in the soundness export itself.

## 2. The defect ledger

### DEFECT-0 — Vacuous external-prover export

**Severity:** meta-completeness. The export was shape-only.

**Symptom.** `KernelSoundness.lean` and `kernel_soundness.v` defined
the kernel-soundness theorem against an opaque axiom
`well_typed : CoreTerm → CoreTerm → Prop` plus four wider axioms
`ctx_lookup_sound : ∀ t T, well_typed t T` (and three siblings).
**Every per-rule lemma was vacuously true** — the placeholder
axioms unconditionally entail the conclusion.

**Why it slipped.** The opaque-axiom layer was a deliberate design
choice for the first phase (allow the soundness statements to be
rendered uniformly without committing to the elaborator's
`Context` shape) but the deliberate-ness concealed the fact that
the *content* of soundness was zero.

**Fundamental fix.** The Lean export now defines a real

```lean
inductive Typing : Ctx → CoreTerm → CoreTerm → Prop where
  | var   : ∀ {Γ x T}, (x, T) ∈ Γ → Typing Γ (Var x) T
  | univ  : ∀ Γ i, Typing Γ (Universe i) (Universe (i + 1))
  | pi    : ∀ {Γ x A B i}, Typing Γ A (Universe i) →
                            Typing ((x, A) :: Γ) B (Universe i) →
                            Typing Γ (Pi x A B) (Universe i)
  -- ... 6 more structural constructors
```

mirroring the structural-fragment of `proof_checker.rs`. The nine
structural per-rule lemmas (`K_Var_sound` through `K_Snd_Elim_sound`)
are now discharged by **direct constructor application** —
`K_Var_sound := @Typing.var`, etc. Real meta-theoretic content,
not vacuous.

The 27 IOU rules (Cubical / Refinement / Quotient / Inductive /
SmtAxiom / Diakrisis) remain admitted; their sorrys are *honest*
and each carries a per-rule reason. The corpus-wide
`kernel_structural_soundness` theorem bundles the nine structural
lemmas into a single fully-proved statement.

**Lock-down.**
`crates/verum_kernel/src/soundness/lean.rs::structural_signature`
emits the rule-specific signature + proof for each structural rule.
Removing any of them (or breaking constructor agreement with the
corpus) fails `verum audit --external-prover-replay`.

### DEFECT-1 — η-rule incompleteness on β-reducible argument

**Severity:** completeness. Kernel rejected valid proof terms.

**Symptom.** `def_eq` η-rule (`λx. (f x) ≡ f`) required the
argument to be syntactically `Var(0)`. A redex like
`λx. (f ((λy. y) x))` β-reduces argument-side to `Var(0)`, but the
syntactic match failed — the η-rule did not fire — and the kernel
rejected the equation.

**Fundamental fix.** `eta_match` now whnf-reduces the argument
before the structural `Var(0)` test. The reduction is bounded by
`whnf_fuel` (DEFECT-3 fix) so this introduces no new termination
risk. See `crates/verum_kernel/src/proof_checker.rs:297-336`.

**Lock-down.** Test
`proof_checker::tests::defect_1_eta_under_beta_reduces_argument`
constructs `λx. (f ((λy. y) x))` and asserts `def_eq` succeeds
against the bare `f`. Pre-fix this returned false; post-fix it
returns true.

### DEFECT-2 — Universe-level overflow

**Severity:** soundness in release builds.

**Symptom.** T-Univ inferred `Universe(n) : Universe(n + 1)` with
naive `u32` addition. In release mode, `n = u32::MAX` wraps to
zero, yielding `Universe(u32::MAX) : Universe(0)` — *unsound*.
Debug mode panics, which is also unacceptable as a kernel-side
behaviour.

**Fundamental fix.** Use `n.checked_add(1)` and reject overflow
with the new structured error
`CheckError::UniverseOverflow { level: u32 }`. See
`crates/verum_kernel/src/proof_checker.rs:455-462`.

**Lock-down.** Two tests: `defect_2_universe_overflow_is_rejected`
(asserts `UniverseOverflow` for `Universe(u32::MAX)`) and
`defect_2_one_below_max_universe_still_typechecks` (asserts
`Universe(u32::MAX - 1) : Universe(u32::MAX)` succeeds — the
ceiling sits at `u32::MAX - 1`).

### DEFECT-3 — `whnf` non-termination on ill-typed input

**Severity:** robustness / DoS.

**Symptom.** `whnf(App(Ω, Ω))` where `Ω = λ. App(Var 0, Var 0)`
loops forever. CoC strong-normalisation guarantees termination on
*type-correct* inputs, but the kernel must not depend on its
caller's discipline; an ill-typed certificate could otherwise hang
the verifier indefinitely.

**Fundamental fix.** `whnf` now delegates to `whnf_fuel` with a
`WHNF_FUEL_CEILING = 1 << 20` head-reductions. CoC-typed terms
never reach this bound; ill-typed inputs that exhaust fuel return
the partially-reduced term, and downstream `def_eq` rejects them
structurally. See `crates/verum_kernel/src/proof_checker.rs:236-282`.

**Lock-down.** Test `defect_3_whnf_terminates_on_omega_omega`
constructs the CoC encoding of Ω(Ω) and asserts `whnf` returns at
all. Pre-fix this test would hang; post-fix it returns within the
fuel ceiling.

### DEFECT-4 — `claimed_type` not validated as a type

**Severity:** soundness in adversarial certificates.

**Symptom.** `Certificate::verify` ran `check(ctx, term, claimed_type)`
without independently verifying that `claimed_type` is itself a
type (i.e. its own type is some `Universe(_)`). An adversary
could ship a certificate whose `claimed_type` is a *value*
(e.g. a closed lambda) and have the verifier "accept" the
obligation if the term inferred a coincidentally-matching shape.

**Fundamental fix.** `Certificate::verify` now type-checks
`claimed_type` first and rejects with the new
`CheckError::ClaimedTypeNotAType { claimed_type, actual }` if its
inferred kind is not a `Universe`. See
`crates/verum_kernel/src/proof_checker.rs:557-587`.

**Lock-down.** Tests
`defect_4_claimed_type_must_be_a_type` (free variable as
claimed_type — rejected) and
`defect_4_claimed_type_well_formed_term_but_not_type`
(closed identity λ as claimed_type — rejected with
`ClaimedTypeNotAType`).

## 3. Aggregate test surface

| Layer | Tests | Pre-audit | Post-audit |
|-------|------:|----------:|-----------:|
| `proof_checker.rs` lib tests | 28 | 23 | 28 |
| External-prover replay (Lean) | 1 file | shape-only | real `Typing`, real proofs for 9/38 rules |
| External-prover replay (Coq) | 1 file | shape-only | (refactor pending; tracked under FV-2 Coq mirror) |
| External-prover replay (Isabelle) | — | not implemented | tracked under FV-2 Isabelle backend |

The five new defect-pinning tests (DEFECT-1, DEFECT-2 ×2,
DEFECT-3, DEFECT-4 ×2) all pass on the patched kernel.

## 4. What is now mechanically guaranteed

The combination of `--kernel-soundness` (drift gate) +
`--external-prover-replay` (foreign re-elaboration) now mechanically
guarantees:

1. The Rust-side `KernelRule` enum agrees with the `.vr` corpus
   row count.
2. The 38 per-rule lemma signatures parse and type-check in Lean 4.
3. The 9 structural-fragment lemmas have **real proofs** stated
   against an inductive `Typing` relation; they cannot be reduced
   to placeholder axioms.
4. The 27 IOU lemmas carry their meta-theory dependency in plain
   text (`-- reason: …` comments / `Admitted. (* reason: … *)`)
   and the count is pinned — any drift in the count surfaces
   immediately.
5. Each kernel defect has a regression test in `proof_checker.rs`
   that fails if the fix is reverted.

## 5. What is not yet guaranteed

Items below are tracked for future audits:

* **Coq / Rocq mirror** of the real-`Typing` refactor. The
  `coq.rs` backend still emits the vacuous-axiom shape; updating it
  is mechanical (mirror of `lean.rs`) and tracked under FV-2.
* **Isabelle/HOL backend.** The skeleton exists in
  `corpus_export.rs` but no `IsabelleBackend` exists at the
  per-rule layer. Tracked under FV-2.
* **Lean reference checker** (`Kernel.check : Ctx → CoreTerm →
  Maybe CoreTerm`) for differential testing against the Rust
  kernel. Tracked under FV-3.
* **Random-term + mutation harness.** Tracked under FV-4.
* **Discharge of the 27 IOUs.** Each names its meta-theory; the
  full discharge plan is in
  `docs/architecture/external-prover-verification.md` §4.

## 6. Cross-references

* `crates/verum_kernel/src/proof_checker.rs` — the trusted base.
* `crates/verum_kernel/src/soundness/lean.rs` — the Lean emitter
  (now real-`Typing`-aware).
* `crates/verum_kernel/src/soundness/coq.rs` — the Coq emitter
  (vacuous-axiom shape; refactor pending).
* `verification/external/lean/VerumExternalReplay/KernelSoundness.lean`
  — the regenerated Lean export the audit gate replays on every
  release.
* `docs/architecture/external-prover-verification.md` — the
  external-prover replay gate's user-facing documentation.
* `docs/architecture/trusted-kernel.md` — the kernel architecture
  this audit critiques.
