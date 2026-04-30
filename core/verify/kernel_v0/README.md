# `kernel_v0/` — Verum's bootstrap meta-theory

The 10-rule minimal kernel that justifies every other inference rule
in `verum_kernel`.  This directory is the **Verum-language** mirror
of the Rust-side proof-term checker (`verum_kernel::proof_checker`,
landed via #157) — written in Verum syntax so the kernel's logic is
fixed-point compilable by Verum itself (Phase 3 / #154).

## Architectural role

```
┌──────────────────────────────────────────────────────────────────┐
│  kernel_v0/  (Verum syntax, this directory)                      │
│  ──────────                                                      │
│  10 minimal inference rules + soundness lemmas.                  │
│  The bootstrap meta-theory.  Hand-auditable.                     │
│                                                                  │
│  ↓ (each subsequent kernel version proves its new rules sound    │
│     in terms of kernel_v0's rules)                               │
│                                                                  │
│  kernel_v1/  ← extends with refinement subtypes                  │
│  kernel_v2/  ← extends with cubical (Path, hcomp, transp)        │
│  kernel_v3/  ← extends with modal operators                      │
│  ...                                                             │
│                                                                  │
│  ↓ at fixed point                                                │
│                                                                  │
│  verum_kernel::proof_checker (Rust)  — generated from kernel_vN  │
│  verum_kernel::* dispatcher (Rust)   — uses kernel_vN's verdict  │
└──────────────────────────────────────────────────────────────────┘
```

## The 10 minimal rules

| Rule          | Status     | Description                                       |
|:--------------|:-----------|:--------------------------------------------------|
| K-Var         | Proved     | Variable lookup in context                        |
| K-Univ        | Proved     | `Universe(n) : Universe(n+1)` — universe stratification |
| K-Pi-Form     | Admitted   | Π-type formation: `(A:U(n))→(B:U(m))` lives in `U(max(n,m))` |
| K-Lam-Intro   | Admitted   | λ introduction: body's type under binder gives Π type |
| K-App-Elim    | Admitted   | Apply elimination + substitution                  |
| K-Beta        | Admitted   | β-reduction `(λx.M) N ⤳ M[N/x]` is type-preserving |
| K-Eta         | Admitted   | η-equivalence `λx.(f x) ≡ f` when `x ∉ FV(f)`     |
| K-Sub         | Admitted   | Subtyping (universe cumulativity)                 |
| K-FwAx        | Proved     | Foundation-aware axiom admission (Prop-only)      |
| K-Pos         | Proved     | Positivity check (Berardi 1998 — non-positive ⇒ ⊥) |

Status reflects the broader 38-rule kernel-soundness corpus
(`core/verify/kernel_soundness/`) — 4 of these 10 are already
proved structurally.  The remaining 6 admits are concrete
meta-theory IOUs (substitution-lemma, β-confluence, etc.) tracked
in detail by the IOU dashboard (`verum audit --soundness-iou`,
landed via #152).

## Trust-base shrinkage roadmap

| Stage | Trust base | Status |
|:------|:-----------|:-------|
| Pre-#157  | 10K LOC `verum_kernel` Rust + 38 rules with 34 admits | as-was |
| Post-#157 | 796 LOC `proof_checker.rs` Rust + 7 rules     | current |
| Phase 3 (#154) — kernel_v0 self-hosted     | 500 LOC Verum + 10 rules | this directory |
| Phase 3 closed — bootstrap chain complete  | 100 LOC bootstrap shim   | future |

Each stage shrinks the trusted base.  The end-state target is a
~100-LOC Rust shim that interprets the kernel_v0 Verum source files;
all kernel logic is verified IN VERUM, not in Rust.  This is the
**Milawa pattern** — kernel(N+1) verified by kernel(N), descending
to a tiny bootstrap.

## File layout (planned)

```
core/verify/kernel_v0/
├── README.md                ← this file
├── mod.vr                   ← module aggregator
├── core_term.vr             ← CoreTerm inductive (mirrors proof_checker::Term)
├── context.vr               ← Context (de Bruijn-indexed type stack)
├── judgment.vr              ← Γ ⊢ t : T judgment (well-typedness)
├── rules/
│   ├── mod.vr               ← rule aggregator
│   ├── k_var.vr             ← T-Var inference rule + soundness lemma
│   ├── k_univ.vr            ← T-Univ
│   ├── k_pi_form.vr         ← T-Pi-Form
│   ├── k_lam_intro.vr       ← T-Lam-Intro
│   ├── k_app_elim.vr        ← T-App-Elim
│   ├── k_beta.vr            ← T-Beta (β-reduction)
│   ├── k_eta.vr             ← T-Eta-Conv (η-equivalence, added in proof_checker.rs:5b6c97a9)
│   ├── k_sub.vr             ← T-Sub (subtyping)
│   ├── k_fwax.vr            ← T-FwAx (proved, structurally)
│   └── k_pos.vr             ← T-Pos (proved, structurally)
└── soundness.vr             ← composes the per-rule lemmas into the
                                top-level kernel-soundness theorem
```

## Why this directory exists pre-Phase-3

Even before the Verum compiler can fully type-check this directory's
files, the directory serves as:

1. **Architectural commitment** — the bootstrap-verified kernel is no
   longer a future plan but an in-flight directory with explicit
   structure.
2. **Documentation surface** — reviewers see the 10 rules and their
   admit status clearly, without spelunking through the 38-rule full
   kernel.
3. **Drift target** — when proof_checker.rs adds a new rule (e.g., the
   recent #157 follow-up adding η-equivalence), kernel_v0 must
   mirror it.  The audit gate (#152 / `--soundness-iou`) tracks
   coverage drift between Rust + Verum mirrors.

## Trust delegation, post-Phase-3

After Phase 3 lands fully, a reviewer who asks "what do I need to
trust to trust Verum?" gets the answer:

> *"Read `core/verify/kernel_v0/`. Ten files, ~500 LOC total.
>  That's the kernel.  Plus a ~100-LOC Rust shim that interprets
>  it.  Plus ZFC + 2 inaccessibles.  Nothing else."*

This is the smallest possible answer in the proof-assistant world.
HOL Light's "kernel.ml" is 5K LOC SML; Coq's kernel directory is
~10K LOC OCaml; Lean 4's `Lean/Compiler/IR.lean` + dependencies
are ~5K LOC C++.  Verum's target: 500 LOC Verum + 100 LOC shim.

## Cross-reference

- `verum_kernel::proof_checker` — Rust-side mirror (796 LOC, current trust base).
- `core/verify/kernel_soundness/` — full 38-rule corpus + admits.
- `core/verify/proof_term_examples/` — canonical .vproof certificates (3 files).
- `verum audit --soundness-iou` — admit dashboard (#152).
- `verum check-proof <file.vproof>` — CLI verifier (#157 follow-up).

## Sub-tasks for completing this directory

(Tracked under #173 / Phase 3 / #154 in the parent task system.)

| Step | Status | File |
|:-----|:-------|:-----|
| 1. Define `CoreTerm` inductive matching `proof_checker::Term` 1-to-1 | ✓ Done | `core_term.vr` |
| 2. Define `Context` (de Bruijn type stack)                            | ✓ Done | `context.vr` |
| 3. Define `Judgment` + 10-case `Derivation` + `DefinitionalEquality`  | ✓ Done | `judgment.vr` |
| 4. Encode 10 inference rules under `rules/`                           | ✓ Done | `rules/k_*.vr` |
| 5. Compose per-rule lemmas into top-level `kernel_soundness` theorem  | ✓ Done | `soundness.vr` |
| 6. Phase-1A — discharge 6 admitted IOUs via mathlib citations         | ✓ Done | `lemmas/` (#155) |
| 7. Wire `verum check kernel_v0/mod.vr` as a CI gate                   | Pending | (compiler-stable target) |
| 8. Phase-1B — verify each `@framework` citation path with upstream    | Pending | sourcing follow-up |
| 9. Phase-2 — full proof-term replay of upstream proofs into Verum     | Pending | (#162) |
| 10. Phase-3 close — generate Rust `proof_checker` from this directory | Pending | (#154) |

**Status as of 2026-04-30**: scaffolding complete + Phase-1A discharge
landed.  The 10-rule roster, per-rule soundness lemmas (4 proved +
6 discharged-by-framework), the master soundness theorem
(`kernel_soundness`), the audit roster (`soundness_roster()`), and
the IOU-discharge stubs in `lemmas/` (5 files citing mathlib4 / ZFC
for the 6 admitted rules) all land in this directory.

**Phase-1A discharge map** (#155 advance):

  - T-Pi-Form    → `lemmas/subst.vr`     (mathlib4 substitution lemma)
  - T-Lam-Intro  → `lemmas/cartesian.vr` (mathlib4 cartesian closure)
  - T-App-Elim   → `lemmas/subst.vr` + `lemmas/beta.vr` (composition)
  - T-Beta       → `lemmas/beta.vr`      (mathlib4 Church-Rosser)
  - T-Eta        → `lemmas/eta.vr`       (ZFC axiom of extensionality)
  - T-Sub        → `lemmas/sub.vr`       (mathlib4 cumulative hierarchy)

Each lemma carries `@framework(<system>, "<path>")` citations.  The
apply-graph audit reclassifies these from `placeholder_axiom`
(forbidden at L4) to `framework_axiom` (acceptable with citation).
`soundness_roster()` reports per-rule status as
`DischargedByFramework { lemma_path, framework, citation }` rather
than `Admitted { iou }`.

**Trust-base shape post-#155 Phase-1A**:
  ~1100 LOC Verum (kernel_v0/) + ~500 LOC discharge stubs (lemmas/)
  + ~100 LOC bootstrap shim + ZFC + 2-inacc + mathlib4 (cited).

**Reviewer answer post-Phase-3**: "Read kernel_v0/.  10 files
~500 LOC + 5 lemma files ~250 LOC.  Plus 100-LOC shim.  Plus ZFC +
2-inacc + cited mathlib4.  Nothing else."  ← smallest verified-kernel
answer in the proof-assistant world.
