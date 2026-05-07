# External-Prover Verification

> Status: load-bearing CI gate. The kernel-soundness corpus is
> regenerated and re-checked through Lean 4 (`lake build`) and Coq /
> Rocq (`coqc`) on every release. Hard-error verdicts block merges.
> Honest IOUs (`sorry` / `Admitted`) are accountability surface, not
> failures — their count is pinned and surfaces immediately if it
> drifts.

## 1. Why this gate exists

Verum's kernel-soundness pipeline (`verum audit --kernel-soundness`,
task #80 / VERUM-TRUST-1) historically did three things:

1. Drift-checked the Rust-side rule list against the `.vr` corpus.
2. Enumerated per-rule status (`Proved` / `Admitted` / `DischargedByFramework`).
3. **Emitted** parallel Lean 4 + Coq theory files into
   `core/target/audit-reports/kernel-soundness/` for "independent
   re-checking".

Step 3 was load-bearing **on paper** but never invoked in CI. A
foreign auditor was supposed to run `lake build` / `coqc` themselves.
This is exactly the kind of trust-trap the pipeline was meant to
*prevent*: the output existed, it looked plausible, and nothing
mechanical confirmed Lean / Coq would actually accept it.

When this gate landed (2026-05-07) it caught **four type errors** in
the supposedly-proved lemmas — broken proof tactics that had been in
the emitter for the entire history of the corpus:

| Rule | Error | Root cause |
|------|-------|------------|
| `K_Var_sound`   | `ctx_lookup_sound Hrule Hwf` — applied to `Prop` where `CoreTerm` expected | proof emitter used the wrong variable names from `intros` |
| `K_Univ_sound`  | `exact universe_form_sound` — residual ∀-binders in the goal | same shape: `intros d Hrule` only consumes 2 binders, leaves `t T : CoreTerm` to be supplied as args |
| `K_FwAx_sound`  | `axiom_body_typed_in_prop body_prop` — `body_prop` undefined | the emitter `rcases`d a `Prop` (`Hside`) as a tuple — Lean refuses |
| `K_Pos_sound`   | `strict_positivity_sound strict_pos` — `strict_pos` was a `ByteArray` | same `rcases` mistake; identifier elaborated as a default literal |

`verum audit --kernel-soundness` reported the gate green for all four
because it never asked Lean. `verum audit --external-prover-replay`
fails immediately.

## 2. What gets verified

### 2.1 Verum-side source of truth

- `core/verify/kernel_soundness/` — the corpus (rule list, lemma
  names, admit reasons, framework citations).
- `crates/verum_kernel/src/proof_tree.rs::KernelRule` — the Rust-side
  enum mirror.
- `crates/verum_kernel/src/soundness/` — the cross-export pipeline
  (`SoundnessExporter`, `LeanBackend`, `CoqBackend`).

The audit gate `--kernel-soundness` drift-checks all three.

### 2.2 Foreign-tool re-check

`verum audit --external-prover-replay` extends the gate with a real
shell-out:

| Backend | Tool | Project | Invocation |
|---------|------|---------|------------|
| Lean 4 (4.29.1+) | `lake build` | `verification/external/lean/` | builds `VerumExternalReplay/KernelSoundness.lean` |
| Coq / Rocq (9.0+) | `coqc -Q . VerumExternalReplay kernel_soundness.v` | `verification/external/coq/` | type-checks the export |

Each backend reports one of four verdicts:

- **`clean`** — backend exited 0 with no diagnostics. The kernel-
  soundness theorem typechecks unconditionally.
- **`iou-only`** — backend exited 0; the only diagnostics are honest
  IOUs (`sorry` warnings in Lean, `Admitted.` lines in Coq) whose
  count matches the corpus's declared admit list. *This is the
  default green state* — the corpus has 27 outstanding admits + 7
  framework-discharged + 4 placeholder-proved.
- **`hard-error`** — backend rejected the export with a real type /
  parse / scoping error. Load-bearing regression. Exits non-zero.
- **`not-available`** — backend binary missing. Advisory unless
  `--strict`; CI uses `--strict`.

## 3. What's NOT verified (honest IOUs)

The four "Proved" lemmas (`K_Var`, `K_Univ`, `K_FwAx`, `K_Pos`) are
discharged via **placeholder axioms** in the emitted file:

```lean
axiom ctx_lookup_sound : ∀ t T, well_typed t T
axiom universe_form_sound : ∀ t T, well_typed t T
axiom axiom_body_typed_in_prop : ∀ t T, well_typed t T
axiom strict_positivity_sound : ∀ t T, well_typed t T
```

These axioms are **vacuously true** — `well_typed` is itself an
opaque axiom. Their job is to discharge the soundness statements *at
the level of theorem statement well-formedness*, not at the level of
meta-theoretic content.

This is a conscious design choice (see `crates/verum_kernel/src/soundness/lean.rs:111-119`):

> Coarse rule predicate placeholders. In a fully fleshed-out
> development these become a `well_typed` relation indexed by 35
> constructors. Here we keep them opaque so the soundness statements
> can be stated uniformly without committing to the elaborator's
> internal `Context` shape.

So `external-prover-replay` verifies:

- ✅ Every emitted theorem statement parses and type-checks in the
  foreign tool.
- ✅ Every `Admitted` / `sorry` carries the same admit reason as the
  Verum corpus (drift-detected).
- ✅ The shape of `CoreTerm`, `CoreType`, `KernelRule` mirrors the
  Rust enums exactly (encoder bug surface).
- ❌ It does **not** verify that the kernel rules are actually sound
  with respect to a denotational model. That's a separate, deeper
  effort tracked under "Kernel meta-theory in Mathlib" in
  `docs/architecture/MSFS-VERIFICATION-ROADMAP.md`.

## 4. The 27 outstanding IOUs

Each admitted lemma names exactly the meta-theory it depends on. The
audit's plain output enumerates every reason verbatim; here they
group by category:

### Cubical (6) — CCHM / HoTT mechanisation
`K_Path_Ty_Form`, `K_Path_Over_Form`, `K_Refl_Intro`, `K_HComp`,
`K_Transp`, `K_Glue`

Discharge plan: port the cubical-type-theory chapter of
[`agda/cubical`](https://github.com/agda/cubical) to a Lean 4 fragment;
or wait for `mathlib4`'s nascent CCHM port. Tracked under
`MSFS-VERIFICATION-ROADMAP.md §4`.

### Refinement (4) — Definition 136.D1 + Lemma 136.L0
`K_Refine`, `K_Refine_Omega`, `K_Refine_Intro`, `K_Refine_Erase`

Discharge plan: formalise the refinement-typing hierarchy + ordinal
modal-depth bound; both are stated in
`crates/verum_kernel/src/eps_mu.rs` but the proofs are admitted at
the Lean side.

### Quotient (3) — equivalence-relation properties
`K_Quot_Form`, `K_Quot_Intro`, `K_Quot_Elim`

Discharge plan: lift `Mathlib.Logic.Equiv` + `Quotient.mk` /
`Quotient.lift` mathlib lemmas through the export.

### Inductive (2) — positivity decision procedure
`K_Inductive`, `K_Elim`

Discharge plan: port Verum's strict-positivity checker
(`crates/verum_kernel/src/positivity.rs`) into a Lean
`Decidable` instance.

### SMT/Axiom (1) — replay correctness
`K_Smt`

Discharge plan: write the property "every cert that
`verum_kernel::replay_smt_cert` accepts denotes a well-typed CoreTerm
derivation" as a Lean predicate over a model of SMT certificates.

### Diakrisis (11) — universe ascent, cohesive modalities, Eps/Mu
`K_Eps_Mu`, `K_Universe_Ascent`, `K_Round_Trip`, `K_Epsilon_Of`,
`K_Alpha_Of`, `K_Modal_Box`, `K_Modal_Diamond`, `K_Modal_Big_And`,
`K_Shape`, `K_Flat`, `K_Sharp`

Discharge plan: these depend on Schreiber DCCT (`schreiber_dcct`
framework, 5 axioms in `core/math/frameworks/`), Shulman 2018 §3, and
Lurie HTT — none has a mature mathlib port today. Tracked separately
under `docs/architecture/diakrisis-bridge-roster.md`.

## 5. Running locally

```bash
# Prerequisites:
#   • Lean 4 via elan: https://leanprover.github.io/get_started/
#   • Rocq / Coq 9.x: brew install rocq  (macOS)
#                     apt install coq    (Debian/Ubuntu, pre-Rocq)

# Default — runs both backends, soft-fails on missing tools.
verum audit --external-prover-replay

# Lean only (faster on a fresh machine — Lake bootstraps quickly).
verum audit --external-prover-replay --backend lean

# CI-grade strict mode.  Fails the gate if any backend isn't
# installed; required reading for `--strict` is "if you can't
# install it, you can't claim it works".
verum audit --external-prover-replay --strict

# JSON for machine consumption.  Output written to
# core/target/audit-reports/external-prover-replay.json + stdout.
verum audit --external-prover-replay --format json
```

## 6. Wiring in CI

The gate is part of `verum audit --bundle` (the umbrella audit). Add
to the GitHub Actions matrix:

```yaml
- name: External-prover replay (Lean + Coq)
  run: |
    # elan + lake
    curl -sSf https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh \
      | sh -s -- -y --default-toolchain leanprover/lean4:v4.29.1
    source $HOME/.elan/env
    # rocq via apt or brew, depending on runner OS
    sudo apt-get install -y coq
    # the gate
    verum audit --external-prover-replay --strict
```

## 7. Maintenance

When **adding** a kernel rule:

1. Append to `KernelRule` enum in `crates/verum_kernel/src/proof_tree.rs`.
2. Append a `RuleSpec` to `canonical_rules()` in
   `crates/verum_kernel/src/soundness/mod.rs` with status set to
   `admitted("<concrete IOU citing meta-theory dependency>")`.
3. Mirror in `core/verify/kernel_soundness/theorems.vr` (drift gate).
4. Run `verum audit --external-prover-replay`. The gate must stay
   green: `iou-only` is fine, `hard-error` means the Lean / Coq
   emission is broken — fix the emitter before merging.

When **promoting** an admit to a real proof:

1. Replace `admitted(...)` with `proved(coq_tactics, lean_tactics)`.
2. Provide a concrete tactic chain that closes the goal against
   the placeholder axioms (or — better — a real proof that doesn't
   rely on them).
3. Rerun the gate. The IOU count decreases; CI doesn't care about
   the absolute number, only about hard-errors.

## 8. Cross-references

- `crates/verum_kernel/src/soundness/mod.rs` — `RuleSpec`, status
  enum, the 38-rule list.
- `crates/verum_kernel/src/soundness/lean.rs`, `.../coq.rs` — emission.
- `verification/external/lean/` — the Lake project this gate drives.
- `verification/external/coq/` — the Coq workspace.
- `docs/architecture/verum-verification-architecture.md` — the broader
  verification strategy this gate is one node of.
- `docs/architecture/MSFS-VERIFICATION-ROADMAP.md` — discharge plans
  for the IOUs.
- `docs/architecture/trusted-kernel.md` — the TCB this gate gives an
  external second opinion on.
