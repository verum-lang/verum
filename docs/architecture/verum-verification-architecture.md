# Verum Verification Architecture (VVA)

> **Single-source spec** covering Verum's verification, dependent-types,
> formal-proofs, meta-system layers, framework axioms, articulation
> hygiene, OWL 2 integration, and Diakrisis-dependent extensions.
>
> **Unification note (V8 #225).** The pre-V8 documentation split this
> material between two files (VVA + VVA). The split was historical —
> VVA staged Diakrisis-preprint-gated extensions ahead of their
> integration. Architecturally there is **one** kernel, **one** trust
> boundary, **one** rollout calendar. This document is the unified
> authoritative source. The legacy filenames (`verification-architecture.md`,
> `foundational-extensions.md`) are removed; external citations of "VVA
> §X.Y" and "VVA-N" continue to refer to the same material, now under a
> single section structure (Part A = Core, Part B = Extensions).

*Authoritative architectural specification for Verum's verification,
dependent-types, formal-proofs, and meta-system layers. Synthesised
from: the MSFS / Verum-MSFS preprints (Sereda 2026), the Diakrisis
canonical primitive + theorem corpus (96 base theorems + 18 NO-results
+ maximality theorems 103.T–106.T establishing
$\mathrm{Diakrisis} \in \mathcal{L}_{\mathrm{Cls}}^{\top}$ + Dual-AFN-T
109.T) and its `09-applications/01-verum-integration.md` +
`11-noesis/*` integration plan, and the existing Verum specs 03/09/12/13/17.*

*Single source of truth for all subsequent work on
`verum_types::refinement`, `verum_smt`, `verum_kernel`,
`verum_verification`, `core.verify`, `core.proof`, `core.math.*`,
`core.action.*`, and `core.theory_interop`. All four legacy
specification documents (03, 09, 12, 13, 17) remain descriptive; when
they conflict with VVA, VVA wins.*

---

# Part A — Core (typing, refinement, framework axioms, hygiene)


*Authoritative architectural specification for Verum's verification, dependent-types, formal-proofs, and meta-system layers. Synthesised from: the MSFS / Verum-MSFS preprints (Sereda 2026), the Diakrisis canonical primitive + theorem corpus (96 base theorems + 18 NO-results + maximality theorems 103.T–106.T establishing $\mathrm{Diakrisis} \in \mathcal{L}_{\mathrm{Cls}}^{\top}$ + Dual-AFN-T 109.T) and its `09-applications/01-verum-integration.md` + `11-noesis/*` integration plan, and the existing Verum specs 03/09/12/13/17.*

*This document is the single source of truth for all subsequent work on `verum_types::refinement`, `verum_smt`, `verum_kernel`, `verum_verification`, `core.verify`, `core.proof`, `core.math.*`, `core.action.*`, and `core.theory_interop`. All four legacy specification documents (09, 12, 13, 17) remain descriptive; when they conflict with VVA, VVA wins.*

---

## 0. Executive Summary

**Thesis.** Every existing proof assistant (Coq, Lean, Agda, Isabelle, F★, Dafny, Liquid Haskell, Mizar, Metamath, Idris) wires a single Rich-foundation — one R-S point in the classifying 2-stack $\mathfrak{M}$ — into its kernel. Verum is architected as a **foundation-neutral host for $\mathfrak{M}$** itself: foundations are first-class data attached through `@framework(name, "citation")`, reductions between foundations are explicit stdlib functors, and the kernel enforces exactly one load-bearing rule imported from Diakrisis: **T-2f\* depth-stratified comprehension** (realised as the `K-Refine` typing rule). On top of that minimal kernel Verum supports the nine-strategy `@verify(...)` ladder, three equivalent refinement forms (inline / declarative / sigma-type), full dependent types (Π, Σ, Path, HITs, quotients, quantitative modalities), a staged meta-system, and a dual OC/DC stdlib that lets an $\mathcal{E}$-enactment view co-exist with every articulation. Every discharge strategy that descends to SMT returns a CoreTerm certificate; the kernel re-checks that certificate and never trusts the solver. Cross-assistant export to Lean / Coq / Agda / Dedukti / Metamath is a first-class capability, not an afterthought.

**Differentiators vs Coq / Lean / Agda / Isabelle / F★ / Dafny / Liquid Haskell.**

| Capability | Coq | Lean 4 | Agda | Isabelle | F★ | Dafny | Liquid H | **Verum (VVA)** |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Single kernel foundation | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | **—** foundation-neutral |
| Framework axioms as first-class data | — | — | — | — | — | — | — | **✓** `@framework(…)` |
| Nine-strategy gradual ladder | — | — | — | — | partial | partial | partial | **✓** `@verify(…)` |
| Three equivalent refinement forms | — | — | — | — | — | — | inline only | **✓** inline / declarative / sigma |
| Dual OC/DC (articulation + enactment) stdlib | — | — | — | — | — | — | — | **✓** `core.math.*` + `core.action.*` |
| Kernel paradox-immunity via T-2f* | Russell-typing | Russell-typing | Russell-typing | Russell-typing | — | — | — | **✓** Yanofsky-universal via depth |
| Corpus MSFS coordinate $(\mathrm{Fw},\nu,\tau)$ | — | — | — | — | — | — | — | **✓** `verum audit --coord` |
| SMT portfolio with CoreTerm certificates | manual | manual | — | manual | ✓ (F★ refine) | ✓ (Boogie) | ✓ (Z3) | **✓** Z3+CVC5+E+Vampire, certificate recheck |
| Cross-assistant certificate export | Dedukti via `coqine` | partial (metamath) | partial | OpenTheory | — | — | — | **✓** five formats from day one |
| Gradual / runtime integration | — | — | — | — | ✓ | — | — | **✓** `runtime` ↔ `proof` on same fn |
| Meta-programming tier (Quote/Unquote) | Ltac2 | macros | reflection | — | — | — | — | **✓** `meta fn`, `@macro`, staged |

The ambition is **not** to compete with each on any single axis; it is to unify the nine-strategy gradual gradient, the three-form refinement surface, the OC/DC duality, and the depth-stratification kernel into a single coherent architecture that subsumes what the others offer individually.

---

## 1. Positioning Relative to Existing Proof Assistants

### 1.1 What Verum is NOT

- Verum is **not** a new type theory. The kernel calculus is CCHM cubical + refinement subtypes + framework-axiom terms + depth-stratified comprehension. Every construct is an established idea; the novelty is the architecture that hosts them.
- Verum is **not** MSFS. MSFS is a classifier 2-stack over R-S; Verum is a single programming language that hosts MSFS as data.
- Verum is **not** a Lean/Coq replacement for mathematicians who want one foundation. It is an integrating layer on top of foundation-neutral engineering: we accept and record whatever foundation the user's proof lives in.

### 1.2 What Verum IS

- A **programming language** with gradual verification, three refinement forms, and a full dependent-type extension.
- A **proof assistant** with tactic DSL, SMT backends, and certificate export.
- A **foundation-integration layer** — the only place to express that "theorem X lives in ZFC, theorem Y lives in HoTT, theorem Z translates between them" as first-class data.
- A **certificate broker** — every verified claim is emitted to Lean / Coq / Agda / Dedukti / Metamath for independent checking.

### 1.3 Design constraints (non-negotiable)

- **Kernel size target: ≤ 5 000 LOC** for `verum_kernel`. Tactics, elaborators, SMT backends, certificate emitters live outside the trusted computing base.
- **Soundness through LCF**: any tactic / SMT backend / macro produces CoreTerms; the kernel re-checks them. We never extend the kernel to accept non-CoreTerm proofs.
- **Foundation-neutrality**: the kernel must not assume ZFC, HoTT, CIC, or any specific R-S. Framework axioms are the only way to introduce foundation-specific principles, and they are explicitly tagged.
- **Backward compatibility**: the three refinement forms in `03-type-system.md` (inline `T{pred}`, declarative `T where name`, sigma `x:T where P(x)`) remain the canonical surface for users. VVA extends them into the dependent / proof world without breaking the simple-case ergonomics.
- **No hardcoded stdlib knowledge in the compiler** (ref: `crates/verum_types/src/CLAUDE.md`). Framework axioms, tactics, and proof strategies are discovered from declarations, not embedded in Rust.

---

## 2. Core Architectural Principles

### 2.1 Foundation-as-data (C-I)

Every proof, every theorem, every claim carries a **framework footprint** — the set of framework axioms it invokes. Surface syntax:

```verum
@framework(name = "lurie_htt", citation = "Lurie J. 2009. Higher Topos Theory.")
public axiom yoneda_embedding_fully_faithful<C: SmallCategory>:
    forall (F, G: C -> Set),
        Hom<PSh(C), F, G> iso_to Nat_Trans<F, G>;
```

The compiler installs each `@framework` axiom as a `CoreTerm::FrameworkAxiom { name, citation, body }` term. `verum audit --framework-axioms` enumerates every framework axiom the current corpus depends on, grouped by source publication. This is the operational realisation of the MSFS functor $\mathrm{Fw}: \mathrm{FwReg}(\mathrm{Verum}) \to \mathcal{L}_{\mathrm{Fnd}}$.

### 2.2 Three refinement forms (already shipped; formally specified here)

Grammar (`grammar/verum.ebnf` already supports these; VVA pins the semantics):

```
refinement_type
    ::= type "{" predicate_expr "}"                    // Inline
      | type "where" ident                             // Declarative
      | ident ":" type "where" predicate_expr          // Sigma-type
```

Desugaring (all three converge to the same CoreTerm shape):

| Surface | CoreTerm |
|---|---|
| `Int{> 0}` | `Refined { base: Int, binder: "it", pred: (it > 0) }` |
| `Int where positive` | `Refined { base: Int, binder: "x", pred: positive(x) }` (α-renamed) |
| `n: Int where n > 0` | `Refined { base: Int, binder: "n", pred: (n > 0) }` |

The three forms are **equivalent** modulo α-conversion of the binder. The compiler must accept all three and canonicalise to a single `Refined` node before elaboration. Error messages quote the surface form the user wrote.

### 2.3 Nine-strategy verification ladder (C-IV)

`@verify(<strategy>)` selects discharge intent; every strategy is sound, but they differ in completeness and cost. Mapped onto the Diakrisis ν-invariant:

| Strategy | Meaning | ν | Cost / Completeness |
|---|---|---|---|
| `runtime` | Emit runtime assertions; do not discharge at compile time. | 0 | O(1) per check; complete for executable runs. |
| `static` | Conservative dataflow / constant folding / CBGR. | 1 | Fast; partial on complex predicates. |
| `fast` | Bounded SMT (single-solver, short timeout). | 2 | ≤ 100 ms / goal; solver returns UNKNOWN conservatively. |
| `formal` | Full SMT portfolio (Z3+CVC5) with decision procedures. | ω | ≤ 5 s / goal; complete on decidable fragments. |
| `proof` | User-supplied tactic proof; kernel re-checks. | ω + 1 | Unbounded user time; mechanically checked. |
| `thorough` | `formal` + mandatory invariant/frame/termination obligations. | ω·2 | Doubles obligation count; catches missing specs. |
| `reliable` | `thorough` + cross-solver agreement (Z3 AND CVC5 agree). | ω·2 + 1 | Racing; UNKNOWN on any disagreement. |
| `certified` | `reliable` + certificate re-check + export. | ω·2 + 2 | Adds certificate materialisation cost; fails on recheck mismatch. |
| `synthesize` | Inverse proof search across $\mathfrak{M}$; fills holes. | ≤ω·3 + 1 | Unbounded; may time out. |

Strategy lifts are **strictly monotone** under `<` over countable ordinals: `0 < 1 < 2 < ω < ω+1 < ω·2 < ω·2+1 < ω·2+2 < ω·3+1`. The first three (`runtime`, `static`, `fast`) live at finite stratum because their work units are decidable in polynomial / sub-exponential time; the remaining strategies enter ω because each adds an unboundedly-deep obligation set (full SMT search at ω; user tactic at ω+1 because it dominates SMT and admits induction; etc.).

`synthesize` is **orthogonal** to the linear ladder in the following precise sense: it is a *modal lift* `□: GoalShape → GoalShape × WitnessShape` (it returns a candidate witness, not a Bool verdict), so it does not consume from the same `verify` decision procedure. Compile-time, a function annotated `@verify(synthesize)` runs both the synthesis loop and (if a witness is produced) the strictest non-synthesize strategy already in scope — the synthesis output feeds back into that strategy's recheck. Hence `ν(synthesize) ≤ ω·3 + 1` is an *upper bound* per MSFS Thm 4.4, not a fixed value: a successful synthesis collapses to the witness-strategy's ν.

An annotation `@verify(proof)` on a function that compiles today with `@verify(runtime)` is always accepted by the compiler — the direction from lax to strict. Rejection in the other direction would require re-proving existing runtime checks; the compiler does not enforce that.

### 2.4 Kernel paradox-immunity via T-2f* (C-III)

The kernel implements **one** non-trivial typing rule beyond CCHM + refinements + framework axioms: the `K-Refine` rule enforcing strict depth-stratification.

```
  Γ ⊢ A : Type_n         Γ, x:A ⊢ P : Prop        dp(P) < dp(A) + 1
  ─────────────────────────────────────────────────────────────────── (K-Refine)
  Γ ⊢ { x:A | P } : Type_n
```

where `dp` is the M-iteration depth function on CoreTerms (defined formally in §4.3), and `dp(A) + 1` is the M-stratum at which a comprehension over `A` lives — exactly the threshold T-2f* forbids `P` from reaching. The conclusion `Refined(A, x, P)` lives at the same universe as `A` (no universe inflation), but at strictly *higher* M-stratum than its predicate. If `dp(P) ≥ dp(A) + 1`, the kernel rejects the term with `KernelError::DepthViolation { binder, base_depth, pred_depth }` (informally `E_K_DEPTH_VIOLATION`); the exact variant is the one in `crates/verum_kernel/src/lib.rs::KernelError`.

The rule presented here is **identical** to §4.4's K-Refine — the formal calculus and the executive summary use the same inequality (`dp(P) < dp(A) + 1`).

The rule transplants Diakrisis T-2f* (quoted verbatim): *"selection of α_P by a predicate P is admissible iff all occurrences of M, ⊏_• in P have strictly smaller depth than α_P."* Yanofsky 2003 establishes that every self-referential paradox in a cartesian-closed context reduces to a weakly point-surjective α: Y → T^Y with dp(α) = dp(T^Y); `K-Refine` forbids exactly that equality. Consequence: Russell, Cantor, Burali-Forti, Tarski, Lawvere, Girard, Gödel-type diagonals, Löb, Grelling–Nelson, and Curry are all blocked at comprehension time without per-paradox tricks.

**Soundness scope of Yanofsky-immunity (V1 / V2 staging).** The Yanofsky claim above is *complete* only when the surface-level Articulation Hygiene discipline (§13) is *enforced*. Diakrisis 105.T's universal paradox-immunity result is over the M-iteration depth function, which is automated through 𝖬-compositions; surface forms that bypass the M-iteration discipline (raw `self`, undeclared recursion, mutable cycles) can in principle re-introduce diagonal arguments not visible at the CoreTerm level. Verum currently ships:

  • **V1 hygiene reporter** (shipped, see §13.3) — `verum audit --hygiene` walks the AST and surfaces every recognised self-referential surface form per the §13.2 hygiene table. **Advisory only**: violations don't break the build.
  • **V2 hygiene enforcement** (deferred — task #196) — `verum check --hygiene src/` will additionally walk raw `self` occurrences inside function bodies and the `Self::Item` / `&mut self` factorisations from §13.2; violations become `E_HYGIENE_UNFACTORED_SELF` errors.

Until V2 ships, the Yanofsky-immunity claim is **partial**: K-Refine alone catches every paradox-form that gets compiled to a `Refined` CoreTerm shape, which covers all five named families (Russell, Cantor, Burali-Forti, Tarski, Lawvere) and every Yanofsky-reducible variant constructed via comprehension. Paradox-forms that bypass comprehension by exploiting raw `self` / undeclared recursion / mutable-cycle constructs are caught only when V2 hygiene enforcement lands.

This V1/V2 staging is intentional — V1 lets existing code compile unchanged while we measure surface-coverage breadth; V2 promotes the diagnostics to errors once the hygiene table is exhaustively validated against real-world stdlib + user code. The kernel-level claim ("no Yanofsky-reducible paradox passes K-Refine") is unconditional and shipped; the surface-level claim ("no Verum source program can express a Yanofsky-reducible paradox") is staged.

This is the single rule that distinguishes Verum's kernel from a plain CCHM + refinements engine.

### 2.5 Certificate-based trust (LCF pattern)

**Target architecture** (end-state, not current reality): every tactic, SMT backend, elaborator output, or external proof is coerced to CoreTerm before the kernel accepts it. The trusted computing base:

- `verum_kernel` — the CoreTerm type checker with `K-Refine`, strict normalisation, universe-consistency, positivity check. Target ≤ 5 000 LOC (see §17 Q8 for current budget).
- Nothing else. `verum_smt`, `verum_tactics`, `verum_elaborator`, `verum_certificate` all emit CoreTerms and let the kernel re-check.

**Current state (Phase 1 baseline)**: `verum_types::refinement` discharges refinements outside the kernel and trusts the SMT solver's Sat/Unsat verdict directly. Kernel re-check for SMT certificates is **Phase 2/3 work** (tasks B2/B3/B4 in §16). Until then, the trusted base includes `verum_smt` alongside `verum_kernel`, and the LCF discipline is aspirational for `@verify(certified)` only. This is tracked as VVA §17 Q9 (`kernel-ownership-migration`).

Once Phase 2/3 lands, an SMT-discharged proof becomes `CoreTerm::SmtCertificate { query, backend, witness, kernel_recheck }` where `witness` is a CoreTerm-encoded Unsat core or model; the kernel walks the witness and verifies each step in the recheck polynomial pass.

### 2.6 Dual OC/DC stdlib

Per Diakrisis 108.T (AC/OC Morita-duality), every articulation has a canonical enactment. The stdlib is structured symmetrically:

```
core.math.*          ← Object-centric (articulations)
  ├── frameworks        @framework axioms by lineage
  ├── category          categories, functors, naturals, Yoneda
  ├── simplicial        simplicial objects, nerve, realization
  ├── infinity_category (∞,1)-categories, limits, Kan
  ├── infinity_topos    ∞-topos, descent, cohesion
  ├── hott              HoTT primitives, univalence
  ├── cubical           CCHM Path, hcomp, transp, Glue
  ├── operad            operads, algebras
  ├── fibration         Grothendieck fibrations
  ├── kan_extension     Lan / Ran, pointwise
  └── logic             propositional, first-order, modal

core.action.*        ← Dependency-centric (enactments) — Actic dual
  ├── primitives        ε_math, ε_compute, ε_observe, ε_prove, ε_decide, ε_translate, ε_construct
  ├── enactments        composition, activation A, autopoietic closure
  ├── gauge             gauge freedom, canonicalisation
  └── verify            ε-audit, gauge-consistency
```

`ε(α)` is auto-induced for every `α: Articulation`. User-facing:

```verum
@enact(epsilon = "ε_prove")
public fn prove_yoneda_lemma(C: SmallCategory) -> Proven<YonedaStatement<C>> using [Theorem] {
    // …
}
```

CLI `verum audit --epsilon src/` prints the ε-distribution over the corpus parallel to `--coord` printing the $(\mathrm{Fw},\nu,\tau)$ distribution.

This is Verum's headline differentiator vs every other proof assistant: **no existing tool exposes the enactment dual as a first-class stdlib layer with certified OC↔DC roundtrips**.

### 2.7 Theory interoperation (C-II realised)

`core.theory_interop` (renamed from `core.mathesis`) exposes three operations, each backed by a category-theoretic construction:

```verum
// load_theory — Yoneda embedding of an external theory as an articulation
public fn load_theory(T: TheoryDescriptor) -> Articulation;

// translate — Kan extension along a theory interpretation
public fn translate(source: Articulation, target: Articulation,
                    partial_map: ClaimTranslationMap)
    -> Result<ArticulationFunctor, ObstructionReport>;

// check_coherence — Čech descent on a covering of translations
public fn check_coherence(translations: List<ArticulationFunctor>)
    -> Result<DescentWitness, CoherenceFailure>;
```

Implementation `core/theory_interop.vr` is already partially landed; VVA stabilises the API.

---

## 3. Layered Architecture

```
┌────────────────────────────────────────────────────────────────────────┐
│                    LAYER 6: USER SURFACE                               │
│  @verify(...)  @theorem  @lemma  @framework  @enact  @derive           │
│  contract#"…"  refinement types (3 forms)  proof blocks  tactic calls  │
└────────────────────────────────────────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│                    LAYER 5: ELABORATION + META                         │
│  verum_fast_parser  →  verum_types  →  verum_elaborator                │
│  @macro expansion   meta fn staging   implicit search   coercion       │
│  three refinement forms  → canonical Refined { base, binder, pred }    │
└────────────────────────────────────────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│                    LAYER 4: TACTICS + SMT + SYNTHESIS                  │
│  verum_tactics  (simp, ring, field, omega, blast, auto, category_simp, │
│                  descent_check, induction, cases, smt, custom …)       │
│  verum_smt (Z3 + CVC5 portfolio, certificates)                         │
│  verum_synthesis (inverse search across 𝔐)                             │
│  verum_certificate (Lean/Coq/Agda/Dedukti/Metamath emitters)           │
└────────────────────────────────────────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│                    LAYER 3: TRUSTED KERNEL (verum_kernel)              │
│  CoreTerm (Π, Σ, Path, hcomp, transp, Glue, inductive, universe,       │
│            Refined, FrameworkAxiom, QuotientType, HIT, SmtCertificate) │
│  K-Refine (T-2f* depth check)  K-Univ (universe consistency)           │
│  K-Pos (strict positivity)  K-Norm (strong normalisation witness)      │
│  Kernel target ≤ 5 000 LOC.                                            │
└────────────────────────────────────────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│                    LAYER 2: VBC + AOT RUNTIME                          │
│  verum_vbc  (Tier-0 interpreter + bytecode codegen)                    │
│  verum_codegen  (VBC → LLVM for AOT)                                   │
│  CBGR memory safety (<15 ns per check)                                 │
└────────────────────────────────────────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│                    LAYER 1: STDLIB (verum stdlib, dual OC/DC)          │
│  core.math.*   — OC: articulations (frameworks, category, hott, …)     │
│  core.action.* — DC: enactments (primitives, enactments, gauge, …)     │
│  core.theory_interop  — Yoneda / Kan / descent                         │
│  core.verify  core.proof  core.tactic                                  │
└────────────────────────────────────────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│                    LAYER 0: FOUNDATION (verum_common)                  │
│  Semantic types: List, Text, Map, Set, Maybe, Heap, Shared             │
│  Well-known type IDs, TypeId allocator, String interner                │
└────────────────────────────────────────────────────────────────────────┘
```

Trust flows upward: Layer 3 trusts nothing above it. Layers 4–6 produce data that Layer 3 re-checks.

---

## 4. CoreTerm Calculus (v2)

### 4.1 Syntax

```
CoreTerm ::=
    // Variables and universes
    | Var(x)
    | Universe(level)                      // Type_n, level ∈ UniverseLevel
    // Dependent functions
    | Pi(x: A, B)                          // (x: A) -> B
    | Lam(x: A, body)                      // λ(x: A). body
    | App(f, arg)
    // Dependent pairs
    | Sigma(x: A, B)                       // Σ(x: A). B
    | Pair(a, b)
    | Fst(p)  | Snd(p)
    // Identity + paths (CCHM cubical)
    | Path(A, lhs, rhs)                    // Path A lhs rhs
    | PathLam(i, body)                     // <i> body
    | PathApp(p, r)                        // p @ r
    | hcomp(A, phi, sys, a0)               // CCHM homogeneous composition
    | transp(A, phi, a0)                   // CCHM transport
    | Glue(A, phi, Ts, a)                  // CCHM glue
    // Inductive types
    | Inductive(name, params, constructors)
    | Constructor(name, args)
    | Match(scrutinee, arms, motive)       // with coverage proof
    // Higher inductive types
    | HIT(name, params, point_ctors, path_ctors, higher_cells)
    // Quotient types
    | Quotient(A, R)                       // A / R
    | QuotIntro(a)
    | QuotElim(motive, case_rel)
    // Refinement subtypes
    | Refined(base: A, binder: x, pred: P)  // { x: A | P }
    | RefineIntro(a, proof_P)               // ⟨a | proof⟩
    | RefineErase(r)                        // ⟨r⟩.value
    // Framework axioms
    | FrameworkAxiom(name, citation, body)
    // SMT certificates
    | SmtCertificate(query, backend, witness)
    // Meta-programming
    | Quote(term)                           // `term`
    | Unquote(handle)                       // $(handle)
    | GoalIntro                             // goal_intro — snapshot
    | HypothesesIntro                       // hypotheses_intro
    // Universes
    | UniverseLevel(level_expr)
    | LevelMax(l1, l2)
    | LevelSucc(l)
    // Ordinal depth (for K-Refine)
    | OrdinalDepth(ord)                    // internal — not user-facing
```

### 4.2 Universe structure

Cumulative hierarchy `Type_0 : Type_1 : Type_2 : …` with level polymorphism (`{u: Level}`). Propositions live in `Prop` (= `Type_{-1}`); impredicative Prop is NOT assumed by default (Coq-style impredicativity is unsound when combined with classical axiom + HITs + quotient types). `core.math.hott.Univalence` requires an explicit `@framework(…)` declaration.

### 4.3 Depth function `dp`

Computed recursively over CoreTerms. Recursive references inside `Inductive(name, ps, cs)` and `HIT(name, …)` use `dp(name) := 0` (the *nominal* self-reference does not contribute), so `dp` is a well-defined fold over a finitely-presented term and not a circular definition for `Nat = Zero | Succ(Nat)`:

```
dp(Var(x))                     = 0
dp(NominalSelf(name))          = 0      // recursive references in own constructors
dp(Universe(n))                = n
dp(Pi(x: A, B))                = max(dp(A), dp(B))
dp(Sigma(x: A, B))             = max(dp(A), dp(B))
dp(App(f, arg))                = max(dp(f), dp(arg))
dp(Refined(A, _, P))           = max(dp(A), dp(P))
dp(FrameworkAxiom(_, _, body)) = dp(body) + 1     // metaisation bumps depth
dp(Quote(t))                   = dp(t) + 1        // quoting bumps depth
dp(Unquote(t))                 = dp(t)
dp(SmtCertificate(_, _, w))    = dp(w)            // post-recheck only; pre-recheck dp = 0
dp(Inductive(name, ps, cs))    = 1 + max{dp(p), dp(c) | p ∈ ps, c ∈ cs}
                                 — recursive occurrences of `name` inside cs are
                                   resolved as NominalSelf(name) ⇒ contribute 0.
dp(HIT(name, ps, pc, pac, hc)) = 1 + max{ all sub-dps, NominalSelf(name) := 0 }
dp(Quotient(A, R))             = max(dp(A), dp(R)) + 1
```

Anything traversing `M`-iteration (the metaisation modality) bumps depth by 1. Framework axioms live strictly above their subject matter (they speak *about* foundations). For `Nat = Zero | Succ(Nat)` this gives `dp(Nat) = 1`, since `dp(Zero) = 0` and `dp(Succ : NominalSelf → NominalSelf) = 0` — the type lives at stratum 1, predicates over it must live at stratum 0.

The `SmtCertificate` clause uses `dp(w)` only after `K-SmtRecheck` has succeeded (§4.4); the kernel treats a not-yet-rechecked certificate's depth as 0 to avoid using untrusted witness depth in a typing premise.

### 4.4 Kernel typing rules (not exhaustive; key rules only)

**K-Refine** (Yanofsky paradox-immunity — the only Diakrisis-imported rule):

```
  Γ ⊢ A : Type_n        Γ, x:A ⊢ P : Prop        dp(P) < dp(A) + 1
  ───────────────────────────────────────────────────────────────── (K-Refine)
  Γ ⊢ Refined(A, x, P) : Type_n
```

**Precedence vs K-Refine-omega.** VVA-7 introduces a strictly stronger version `K-Refine-omega` that uses ordinal-valued depth `m_depth_omega` (per Definition 136.D1) instead of finite `dp`. The two rules **coexist** as follows:

  • `K-Refine` is the *baseline* rule the kernel always enforces. Finite-depth fragments (no `ModalBox` / `ModalDiamond` / `ModalBigAnd` / `EpsilonOf` / `AlphaOf` constructors in `P`) are equivalent under both rules: `dp` and `m_depth_omega` agree on the M-iteration-free fragment.
  • `K-Refine-omega` is *opt-in* via `@require_extension(vfe_7)` annotation (per VVA governance §0.0). When enabled, it **replaces** `K-Refine` for the annotated module — every refinement-type formation uses ordinal depth.
  • Without the extension, refinement-type formation that would pass `K-Refine` but fail `K-Refine-omega` (e.g., a `ModalBox`-wrapped predicate over an atomic base) is *accepted* by the baseline kernel because finite `dp` collapses modal wrappers to depth 0.
  • With the extension, the same code is *rejected* — `m_depth_omega(ModalBox(P)) = m_depth_omega(P).succ()`, so `ModalBox(P)` over an atomic base violates the strict inequality.

**Direction**: tighter rules can only be stricter, never looser. A module passing `K-Refine-omega` (with the extension) automatically passes `K-Refine` (without). The reverse direction is not guaranteed — that is the whole point of the strictness gradient.

**Implementation cross-reference**: kernel `m_depth` (line 574 of `crates/verum_kernel/src/lib.rs`) implements baseline `dp`; `m_depth_omega` (line 733+) implements ordinal-valued depth. `KernelRecheckPass` currently invokes `m_depth_omega` unconditionally via `check_refine_omega`; the precedence model above is the **target** state once `@require_extension(vfe_7)` is enforced at the elaborator level. Until then, every module is implicitly opted into the stricter check, which is sound (over-approximation of the user's intent) but slightly more conservative than the spec promises for non-extension modules.

**K-RefineIntro**:

```
  Γ ⊢ a : A        Γ ⊢ proof : P[a/x]
  ──────────────────────────────────── (K-RefineIntro)
  Γ ⊢ ⟨a | proof⟩ : Refined(A, x, P)
```

**K-RefineErase**:

```
  Γ ⊢ r : Refined(A, x, P)
  ──────────────────────── (K-RefineErase)
  Γ ⊢ r.value : A
```

**K-FrameworkAxiom** — a framework axiom inhabits the very proposition it asserts; the kernel records the framework lineage but produces a witness of `body`, not a generic `Prop`:

```
  Γ ⊢ body : Prop        name ∈ FrameworkRegistry        citation ≠ ∅
  ─────────────────────────────────────────────────────────────────── (K-FwAx)
  Γ ⊢ FrameworkAxiom(name, citation, body) : body
```

**Soundness side-condition.** The kernel admits `FrameworkAxiom(name, citation, body)` only when `body : Prop` (i.e. the axiom asserts a proposition, not a non-trivial inhabitant of a `Type_n`). This is non-negotiable: a framework axiom of type `Nat → Nat` would let a user postulate an arbitrary computable function and break strong normalisation; restricting bodies to `Prop` keeps the postulate at the propositional layer where SN is preserved by the standard "axioms-stuck" reduction strategy (a `FrameworkAxiom` term is a kernel-irreducible normal form and cannot fire β/ι/cube reductions inside a Prop-eliminator). The `K-FwAx` rule's premise `body : Prop` makes this condition syntactic — the kernel rejects an axiom whose body is not a proposition with `E_FW_AXIOM_NOT_PROP`.

**Subsingleton (proof-irrelevance) requirement.** `body : Prop` alone is **not sufficient** for subject reduction: a framework axiom whose body has *two distinct extensional inhabitants* breaks equality reasoning — the axiom term is a normal form yet has multiple distinct values, so equality between two instances of the axiom is undecidable. The kernel additionally requires `body` to be a **subsingleton** (proof-irrelevant): `∀ p q : body. p ≡ q` must hold definitionally. Two acceptance criteria:

  1. **Closed-proposition route** (statically checkable): `body` mentions no free type variables. Closed propositions in `Prop` are forced unique by the framework lineage's intended interpretation (every model in the framework's R-S sees the same set of inhabitants). The kernel checks this by walking `body` for unbound type-vars; if none, the axiom is admitted.
  2. **UIP route** (axiom of uniqueness of identity proofs): `body` mentions free type-vars but the framework explicitly imports `core.math.frameworks.uip` and the elaborator records the import; the kernel then admits the axiom under the UIP regime. Mixing UIP with `core.math.frameworks.univalence` is rejected by the framework-conflict checker (#197).

A non-subsingleton axiom (e.g., `axiom choice<T>: ∀(s: NonEmpty<T>). T` with two distinct inhabitants depending on which element of `s` is selected) is rejected with `E_FW_AXIOM_NOT_SUBSINGLETON`. Acceptable form: `axiom choice<T>: ∀(s: NonEmpty<T>). exists (t: T). t ∈ s` — the existential-only form is propositional + subsingleton-by-construction.

**K-SmtRecheck** (certificate verification):

```
  Γ ⊢ query : Prop        witness : CoreTermWitnessOf(query)
  Γ ⊢ kernel_recheck(witness) = Ok    // polynomial-time walk
  ────────────────────────────────────────────────────────── (K-Smt)
  Γ ⊢ SmtCertificate(query, backend, witness) : query
```

The certificate's `obligation_hash` MUST be checked against the
caller's expected hash via
[`replay_smt_cert_with_obligation`](../../crates/verum_kernel/src/support.rs)
before the witness is admitted; using the bare
[`replay_smt_cert`](../../crates/verum_kernel/src/support.rs) (no
hash comparison) is reserved for kernel-internal callers that
construct the witness without yet knowing the goal. See V8
commit 21aec4c3 closing the doc/code mismatch where the bare
function's docstring claimed the comparison existed.

### 4.4a Exhaustive kernel-rule taxonomy (V8, #214)

Closes the "not exhaustive; key rules only" disclaimer above:
this subsection enumerates **every** typing rule the trusted
base implements today. Each entry gives the formal premise/
conclusion, the V-stage maturity tag, implementation cross-
reference (`crates/verum_kernel/src/<file>.rs::<symbol>`), and
explicit side-conditions.

V-stage taxonomy:

  • **V0** — bring-up shape check. Only the syntactic
    constructor is verified; deeper invariants are deferred.
  • **V1** — conservative-correct rule with the documented
    soundness gates wired but completeness possibly partial.
  • **V2** — completeness-extended (β-aware, normalised,
    or otherwise reduced before comparison).
  • **V8** — V8 batch (this document's edit) — soundness
    gaps from §4.5 metatheory dependencies + doc/code
    reconciliation.

#### 4.4a.1 Structural rules (CCHM core)

```
  (x : A) ∈ Γ
  ─────────── (K-Var)              [V0]  infer.rs:Var arm
  Γ ⊢ x : A
```
Lookup-by-name in the typing context; unbound names raise
`KernelError::UnboundVariable`.

```
  Γ ⊢ Type_n : Type_{n+1}            (K-Univ)            [V8 — B1]
  Γ ⊢ Prop : Type_0                                       infer.rs:Universe arm
```
Side-condition: `n < u32::MAX`. Overflow at `u32::MAX` raises
`KernelError::UniverseLevelOverflow` (closes type-in-type
saturation, see commit c667729d). `Prop` is at `Type_0`.

```
  Γ ⊢ A : Type_i        Γ, x:A ⊢ B : Type_j
  ────────────────────────────────────────── (K-Pi-Form)  [V0]
  Γ ⊢ Π (x:A). B : Type_max(i,j)                          infer.rs:Pi arm
```

```
  Γ, x:A ⊢ t : B
  ─────────────────────────────── (K-Lam-Intro)           [V0]
  Γ ⊢ λ(x:A). t : Π (x:A). B                              infer.rs:Lam arm
```

```
  Γ ⊢ f : Π(x:A). B        Γ ⊢ a : A
  ──────────────────────────────────── (K-App-Elim)       [V0]
  Γ ⊢ f a : B[x := a]                                     infer.rs:App arm
```
Domain match uses `structural_eq` today; lifting to
`definitional_eq` is V2 work (separate task tracked under #216
follow-ups).

```
  Γ ⊢ A : Type_i        Γ, x:A ⊢ B : Type_j
  ────────────────────────────────────────── (K-Sigma-Form) [V0]
  Γ ⊢ Σ (x:A). B : Type_max(i,j)                          infer.rs:Sigma arm
```

```
  Γ ⊢ a : A        Γ ⊢ b : B
  ──────────────────────────────  (K-Pair-Intro)          [V0 — non-dependent]
  Γ ⊢ (a, b) : Σ (_:A). B                                 infer.rs:Pair arm
```
V0 emits a non-dependent Σ; full `Pair` typing under a
declared dependent Σ is V1 work (requires bidirectional
elaboration with an expected-type channel).

```
  Γ ⊢ p : Σ(x:A). B
  ───────────────── (K-Fst-Elim)                          [V0]
  Γ ⊢ fst p : A                                           infer.rs:Fst arm

  Γ ⊢ p : Σ(x:A). B
  ────────────────────── (K-Snd-Elim)                     [V0]
  Γ ⊢ snd p : B[x := fst p]                               infer.rs:Snd arm
```

#### 4.4a.2 Cubical rules

```
  Γ ⊢ A : Type_i      Γ ⊢ a : A      Γ ⊢ b : A
  ───────────────────────────────────────────── (K-PathTy-Form) [V8 — #216]
  Γ ⊢ PathTy<A>(a, b) : Type_i                                  infer.rs:PathTy arm
```
Endpoint-against-carrier check uses
[`definitional_eq`](../../crates/verum_kernel/src/support.rs)
(β-normalises both sides before structural comparison) — closes
the false-rejection of `App(Lam, _)` carriers per V8 commit
1373bf49.

```
  Γ ⊢ x : A
  ──────────────────────────  (K-Refl-Intro)              [V0]
  Γ ⊢ refl(x) : PathTy<A>(x, x)                           infer.rs:Refl arm
```

```
  Γ ⊢ φ : I        Γ ⊢ walls : (i : I) → A^[walls(φ)]      Γ ⊢ base : A
  ──────────────────────────────────────────────────────────── (K-HComp) [V0 — partial]
  Γ ⊢ hcomp(φ, walls, base) : A                                infer.rs:HComp arm
```
Side-condition: `phi`, `walls`, `base` each well-typed. Full
cofibration calculus (interval subsumption, face-formula
algebra) is dedicated cubical-pass V1 work.

```
  Γ ⊢ p : PathTy<Type>(A, B)        Γ ⊢ r : I        Γ ⊢ t : A
  ────────────────────────────────────────────────────────────── (K-Transp) [V0 — partial]
  Γ ⊢ transp(p, r, t) : B                                       infer.rs:Transp arm
```
When `p`'s type isn't `PathTy`, the rule conservatively returns
`t`'s type (sound — the kernel returns a consistent type,
downstream's App rule rejects on mismatch).

```
  Γ ⊢ A : Type_i      Γ ⊢ φ : I      Γ ⊢ T : (i:I) → Partial<φ, Type_i>      Γ ⊢ e : Equiv(T, A)^[φ]
  ────────────────────────────────────────────────────────────────────────────── (K-Glue) [V0 — partial]
  Γ ⊢ Glue<A>(φ, T, e) : Type_i                                                  infer.rs:Glue arm
```
Side-condition: every sub-term well-typed. Full equiv-coherence
+ univalence computation lands in dedicated cubical-pass V1.

#### 4.4a.3 Refinement rules

```
  Γ ⊢ A : Type_n        Γ, x:A ⊢ P : Prop        dp(P) < dp(A) + 1
  ─────────────────────────────────────────────────────────────── (K-Refine) [V0]
  Γ ⊢ Refined(A, x, P) : Type_n                                    infer.rs:Refine arm
```
Yanofsky paradox-immunity gate via finite `dp` (T-2f*).
`KernelError::DepthViolation` on failure.

```
  Γ ⊢ A : Type_n        Γ, x:A ⊢ P : Prop        m_depth_omega(P) < m_depth_omega(A).succ()
  ──────────────────────────────────────────────────────────────────── (K-Refine-omega) [V8 — #218 wires]
  Γ ⊢ Refined(A, x, P) : Type_n                                       depth.rs:check_refine_omega
```
VVA-7 transfinite stratification (T-2f***). Ordinal-valued
`m_depth_omega`; supersedes `K-Refine` under
`@require_extension(vfe_7)` (default `AllRulesActive` in V8 —
the spec's Year 0–2 OptInOnly default lands once
[task #218 wires policy through pipeline] (../../crates/verum_verification/src/passes/kernel_recheck.rs)).
`KernelError::ModalDepthExceeded` on failure.

```
  Γ ⊢ a : A        Γ ⊢ proof : P[a/x]
  ─────────────────────────────────────── (K-RefineIntro) [V0]
  Γ ⊢ ⟨a | proof⟩ : Refined(A, x, P)
```

```
  Γ ⊢ r : Refined(A, x, P)
  ──────────────────────── (K-RefineErase) [V0]
  Γ ⊢ r.value : A
```

#### 4.4a.4 Inductive rules

```
  registered("path") ∈ InductiveRegistry        registered("path").universe = U
  ─────────────────────────────────────────────────────────────── (K-Inductive) [V8 — #215]
  Γ ⊢ Inductive("path", args) : Universe(U)                       infer.rs:Inductive arm
```
V8 closes the hardcoded `Universe(Concrete(0))` demotion; HoTT-
level inductives now report the registered universe through
[`infer_with_inductives`](../../crates/verum_kernel/src/infer.rs)
(legacy `infer` shim still falls back to `Concrete(0)` when no
registry is supplied). Commit ca991d9e.

```
  ∀ ctor ∈ constructors. ∀ argTy ∈ ctor.arg_types. positive(target, argTy)
  ─────────────────────────────────────────────────────────── (K-Pos) [V0]
  registered(target, params, constructors)                     inductive.rs:check_strict_positivity
```
Strict-positivity check per VVA §7.3. Berardi-shaped
definitions (`type Bad = Wrap(Bad → A)`) rejected via
`KernelError::PositivityViolation`.

```
  Γ ⊢ scrutinee : T        Γ ⊢ motive : T → Type
  ──────────────────────────────────────────────── (K-Elim) [V8 — Elim V1]
  Γ ⊢ elim(scrutinee, motive, cases) : motive scrutinee   infer.rs:Elim arm
```
V8 (commit 52896d42) tightens motive's TYPE to be a Π and
verifies `scrutinee : domain(motive's Π)`. Per-case
exhaustiveness + per-case typing is the dedicated Elim-rule
pass's job (V2).

#### 4.4a.5 SMT certificate + framework axioms

```
  Γ ⊢ query : Prop        witness = replay_smt_cert(cert)        kernel_recheck(witness) = Ok
  ──────────────────────────────────────────────────────────── (K-Smt) [V0 + V8 hash gate]
  Γ ⊢ SmtCertificate(cert) : query                              support.rs:replay_smt_cert
```
V8 introduces
[`replay_smt_cert_with_obligation(cert, expected_hash)`](../../crates/verum_kernel/src/support.rs)
as the soundness-correct entry point — verifies
`cert.obligation_hash == expected_hash` before replay. The
bare `replay_smt_cert` is kernel-internal (reserved for callers
without the goal in hand; commit 21aec4c3 reconciles its
docstring with the actual no-comparison behaviour).

```
  Γ ⊢ body : Prop        name ∈ FrameworkRegistry        citation ≠ ∅        is_subsingleton(body, regime)
  ──────────────────────────────────────────────────────────────────────────────── (K-FwAx) [V8 — #217]
  Γ ⊢ FrameworkAxiom(name, citation, body) : body                                  axiom.rs:register_with_regime
```
V8 (commit 020d5408) ships the `is_subsingleton` gate per the
two-route discipline above (closed-proposition vs UIP). New
`KernelError::AxiomNotSubsingleton` carries the offending
free-var set for diagnostic post-mortem. Pre-V8 only the
syntactic UIP shape was rejected (`KernelError::UipForbidden`);
the wider class of non-subsingleton dependent axioms slipped
through.

#### 4.4a.6 Diakrisis VVA rules

```
  Γ ⊢ ε(α) ≃ A(ε(α))        depth-preserving for non-identity M
  ────────────────────────────────────────────────────────── (K-Eps-Mu) [V0/V1/V2 — #181 V3]
  Γ ⊢ EpsilonOf(α), AlphaOf(ε(α)) — coherent                  eps_mu.rs:check_eps_mu_coherence
```
VVA-1. V0/V1/V2 staged: V0 shape check, V1 identity-functor
case, V2 depth-preservation pre-condition. Full τ-witness
construction (σ_α / π_α) deferred to V3 (#181, multi-week).
`KernelError::EpsMuNaturalityFailed` on failure (V2.5: depth-
mismatch context now embeds rank values per commit 601a7c90).

```
  from_tier ⊆ canonical-step(to_tier)
  ─────────────────────────────────── (K-Universe-Ascent) [V1 — VVA-3]
  Γ ⊢ ascent(from_tier → to_tier) ok                       universe_ascent.rs:check_universe_ascent
```
Theorem 131.T κ-tower step. Valid steps:
`Truncated → Truncated`, `κ_1 → κ_1`, `κ_1 → κ_2` (Lemma
131.L1), `κ_2 → κ_2` (Lemma 131.L3 Drake-reflection closure).
`KernelError::UniverseAscentInvalid` on tier inversion.

```
  Γ ⊢ t : T                                            Γ ⊢ t : T
  ─────────────────────── (K-EpsilonOf)                ─────────────── (K-AlphaOf)
  Γ ⊢ EpsilonOf(t) : T                                 Γ ⊢ AlphaOf(t) : T
```
V0: ε and α inherit the operand's type (the M ⊣ A biadjunction
shows up only at the 2-cell level; type-level inheritance is
sound). V1 will refine to track articulation/enactment 2-
category membership.

```
  Γ ⊢ φ : T
  ───────────────────────── (K-ModalBox)                [V1 — VVA-7]
  Γ ⊢ ModalBox(φ) : Prop                                 infer.rs:ModalBox arm
```
□φ inhabits Prop (modality lifts to the propositional layer).
`md^ω(□φ) = md^ω(φ).succ()` per Definition 136.D1.

```
  Γ ⊢ φ : T
  ───────────────────────────── (K-ModalDiamond)        [V1 — VVA-7]
  Γ ⊢ ModalDiamond(φ) : Prop                             infer.rs:ModalDiamond arm
```
Symmetric to K-ModalBox. `md^ω(◇φ) = md^ω(φ).succ()`.

```
  ∀ i. Γ ⊢ φ_i : T_i
  ─────────────────────────────────────── (K-ModalBigAnd) [V1 — VVA-7]
  Γ ⊢ ModalBigAnd(φ_0, ..., φ_κ) : Prop                    infer.rs:ModalBigAnd arm
```
Transfinite conjunction. `md^ω = sup_i (md^ω(φ_i))` per Lemma
136.L0.

#### 4.4a.7 Maturity audit

| Rule family | Rules | V-stage range | Implementation crate |
|---|---|---|---|
| Structural (CCHM core) | 9 | V0 + V8 (Univ) | verum_kernel/src/infer.rs |
| Cubical | 5 | V0 (partial: K-HComp / K-Transp / K-Glue need cubical-pass V1) + V8 (PathTy) | verum_kernel/src/infer.rs |
| Refinement | 4 | V0 + V8 wire (K-Refine-omega) | verum_kernel/src/{infer,depth}.rs |
| Inductive | 3 | V0 (K-Pos) + V8 (K-Inductive, K-Elim) | verum_kernel/src/{inductive,infer}.rs |
| SMT + Axiom | 2 | V8 (both) | verum_kernel/src/{support,axiom}.rs |
| Diakrisis VVA | 6 | V0 / V1 / V2 (K-Eps-Mu V3 = #181) | verum_kernel/src/{eps_mu,universe_ascent,infer}.rs |
| **Total** | **29** | — | — |

The 29-rule count makes the trusted base auditable: every rule
has a formal premise, a maturity stage, an implementation
pointer, and a documented side-condition. Future rule additions
extend this table; V-stage promotions update the existing entry.

### 4.5 Metatheory status

| Property | Status | Evidence / side-conditions |
|---|---|---|
| Subject reduction | Expected, conditional on K-FwAx subsingleton requirement | Inherits from CCHM (Cohen–Coquand–Huber–Mörtberg 2018); refinement + K-Refine are admissible because `Refined` reduces only through `RefineErase` which discards proof content. **K-FwAx side-condition**: `body` must be subsingleton (closed proposition or UIP-regime); a non-subsingleton axiom term is a normal form with multiple distinct extensional values, breaking SR. Enforced by `E_FW_AXIOM_NOT_SUBSINGLETON`. |
| Strong normalisation | Expected for the CCHM fragment + Prop-subsingleton framework axioms | Huber 2019. Side-condition: every `FrameworkAxiom(_, _, body)` must satisfy `body : Prop` AND `body` is subsingleton (enforced by `K-FwAx`); a non-Prop axiom (`unsafe_cast: ∀A B. A→B`) breaks SN, a non-subsingleton Prop-axiom breaks SR. |
| Canonicity | For **closed** terms of canonical type-formers (Π, Σ, Path, Inductive, HIT) in the CCHM fragment | Huber 2019 closed-term canonicity. Open terms with refinement subtypes are *not* canonical (`Refined(A, x, P)` of an open term may have no canonical form until its predicate is discharged). |
| Decidability of type-checking | Yes for the ETT-free fragment + intensional equality | Refinement subtype-checking is decidable iff the underlying predicate satisfies the discharge strategy (e.g. `@verify(static)` ⇒ decidable; `@verify(proof)` ⇒ user-supplied). |
| Parametricity | Optional, via a `@parametric` framework axiom | Not kernel-level. |
| Initiality (of `Syn(Verum)`) | Open | Requires formal $\mathrm{Syn}/\mathrm{Mod}$ adjunction mechanisation. |
| Consistency | Relative to $\ZFC + 2\text{-inacc}$ (per MSFS Convention `conv:zfc-inacc`), modulo the **chosen framework-axiom bundle** | A user importing `core.math.frameworks.classical_lem` and an axiom that internalises the law of excluded middle inhabits the corresponding Prop; consistency is then relative to ZFC + classical_lem's stated content. The kernel does not certify the axiom bundle's mutual consistency — that is delegated to the bundle's citation. |

**Open**: formal proof of metatheory inside Verum itself. Tracked as `task:metatheory-self-verification`. See §16.6 task F2 for the relativised statement (the Gödel boundary forbids absolute self-consistency).

---

## 5. Three Refinement Forms — Formal Specification

This section is **normative** for the refinement surface. `03-type-system.md` § 1.5 is descriptive; VVA pins semantics.

### 5.1 Grammar

Existing productions in `grammar/verum.ebnf` (§ 2.12 `refinement`, § 2.14 `sigma_binding`, § 2.18 `refinement_method`). VVA reaffirms the three canonical forms and **deprecates the historical "five rules"** (cf. `crates/verum_ast/src/ty.rs` `RefinementPredicate` doc-comment, `docs/detailed/03-type-system.md` § 1.5):

| Canonical form | Historical name (deprecated label in parentheses) |
|---|---|
| Inline `T{pred}` | Rule 1 (Inline) |
| Declarative `T where name` | Rule 4 (Named) |
| Sigma `x: T where P(x)` | Rule 3 (Sigma) |

Rules 2 (`T where |x| pred`) and 5 (bare `T where pred` with implicit `it`) are **eliminated** in VVA — rule 2 collapses into sigma, rule 5 into inline.

```ebnf
refinement_type
    = inline_refinement
    | declarative_refinement
    | sigma_refinement ;

inline_refinement       = type_expr , "{" , predicate_expr_with_it , "}" ;
declarative_refinement  = type_expr , "where" , predicate_name ;
sigma_refinement        = identifier , ":" , type_expr , "where" , predicate_expr_with_binder ;

predicate_expr_with_it       = predicate_expr ;   (* implicit binding: uses `it` *)
predicate_expr_with_binder   = predicate_expr ;   (* explicit binding: uses the bound identifier *)
```

### 5.2 Desugaring

All three forms canonicalise to `CoreTerm::Refined { base, binder, pred }`:

| Surface | Canonical CoreTerm |
|---|---|
| `Int{> 0}` | `Refined(Int, "it", Binary(Gt, Var("it"), Literal(0)))` |
| `Int where positive` | `Refined(Int, "x₀", App(Var("positive"), Var("x₀")))` |
| `n: Int where n > 0` | `Refined(Int, "n", Binary(Gt, Var("n"), Literal(0)))` |

α-equivalence: all three above are definitionally equal (`Refined(Int, _, Binary(Gt, _, 0))` up to binder renaming).

### 5.3 Error messages

Diagnostics quote the surface form the user wrote:

- User wrote `Int{> 0}` → error says `refinement Int{> 0}` not `Refined(Int, it, …)`.
- User wrote `n: Int where n > 0` → error says `refinement n: Int where n > 0`.
- User wrote `Int where positive` → error says `refinement Int where positive` and includes the definition of `positive`.

### 5.4 Cross-field refinements (structs)

```verum
public type DateRange is {
    start: Date,
    end: Date,
} where start <= end;
```

Desugars to `Refined(Record{start: Date, end: Date}, "r", LessEq(Proj(r, start), Proj(r, end)))`. This is a sigma-type refinement with a record binder.

### 5.5 Interaction with dependent types

When the base `A` is itself a type-level function or dependent product, the refinement form composes naturally:

```verum
public fn safe_index<T, n: Nat>(xs: List<T>{len(it) == n}, i: Fin<n>) -> T;
```

The inline form `{len(it) == n}` sees `n` in scope because `n` is a dependent parameter of the surrounding function. The kernel's depth check passes because `len` is a stdlib primitive with `dp(len) = 0`.

### 5.6 Three-form discipline

**MUST** compile-accept all three forms in every position where a refinement is allowed.

**MUST** canonicalise to a single `Refined` CoreTerm before type-checking.

**MUST** preserve the original surface form in the AST for diagnostics.

**MUST NOT** introduce a fourth form without a VVA revision.

---

## 6. `@framework` Axiom System

### 6.1 Declaration

Attaches to `axiom_decl` / `theorem_decl` / `lemma_decl` / `corollary_decl` per `grammar/verum.ebnf` § 2.19. Existing parser shape (2-positional-arg form) is `@framework(identifier, "citation")`; VVA extends to named args:

```verum
// Minimal (already shipped per verum_kernel::load_framework_axioms):
@framework(lurie_htt, "Lurie J. 2009. Higher Topos Theory. §6.2.2.7")
axiom yoneda_embedding_ff<C: SmallCategory>(F: C -> Set, G: C -> Set)
    ensures Hom<PSh(C), F, G> equiv Nat_Trans<F, G>;

// Extended (VVA Phase 1 task A3 — opt-in named fields):
@framework(
    name = lurie_htt,
    citation = "Lurie J. 2009. Higher Topos Theory. §6.2.2.7",
    strata = ClsTop,            // MSFS stratum
    depth = @omega,             // ν-coordinate (ordinal literal)
    intensional_tau = 1,        // Eff-internal normaliser?
)
axiom yoneda_full<C: SmallCategory>(F: C -> Set, G: C -> Set)
    ensures Hom<PSh(C), F, G> equiv Nat_Trans<F, G>;
```

Stored as `CoreTerm::FrameworkAxiom { name, citation, body, metadata }`. The axiom cannot be proved inside Verum; it can only be *invoked*. Invocation is recorded in the CoreTerm of every consumer.

### 6.2 The Standard catalogue

Closes B15 (#213). Pre-V8 the spec listed a "six-pack" of six
frameworks; the actual stdlib catalogue at `core/math/frameworks/`
+ `core/math/frameworks/registry.vr::populate_canonical_standard`
ships nine **Standard** frameworks, plus a **VerifiedExtension**
family (`bounded_arithmetic_*`, the four diakrisis sub-corpora) and
the **Experimental** tier — fourteen frameworks visible at module
level today, with more expected from VVA-N rule additions.

**Standard catalogue** (per `populate_canonical_standard`,
ordered by canonical (ν, τ) coordinate per VVA §10.4.1):

| Package | Coord (ν) | Lineage | Representative axioms |
|---|---|---|---|
| `diakrisis` | — (meta-classifier) | Diakrisis L5+ canonical-primitive metaclassification | (Fw, ν, τ) coordinate space; defines the space, occupies no point |
| `actic.raw` | 0 | Neutral Actic articulation | base of the canonical primitive |
| `petz_classification` | 2 | Petz D. 1986. *Quasi-entropies.* | f-divergence characterisation, monotone metrics on matrix spaces |
| `arnold_catastrophe` | 2 | Arnold V. I. 1985. *Singularities of Differentiable Maps III.* | catastrophe normal forms (codim ≤ 4), critical-value theory |
| `owl2_fs` | 1 | W3C 2012. *OWL 2 Direct Semantics.* | SROIQ DL-decidable fragment (64 axioms across 9 sub-modules; see Task C7) |
| `lurie_htt` | ω | Lurie J. 2009. *Higher Topos Theory.* | straightening / unstraightening, coherent diagrams, ∞-Kan extension formula |
| `connes_reconstruction` | ω | Connes-Chamseddine 2008 (NCG 1994). | spectral-triple reconstruction, KO-dimension |
| `baez_dolan` | ω+1 | Baez J., Dolan J. 1995. *Higher-Dimensional Algebra.* | stabilisation hypothesis, n-categorical inf |
| `schreiber_dcct` | ω+2 | Schreiber U. 2013. *Differential Cohomology in a Cohesive ∞-Topos.* | shape ∫, flat ♭, sharp ♯, cohesion axioms |

**VerifiedExtension family** — bounded arithmetic
(per `populate_bounded_arithmetic_family`):

| Package | Class | Lineage |
|---|---|---|
| `bounded_arithmetic_v_0` | LOGSPACE | Cook-Reckhow |
| `bounded_arithmetic_v_1` | P (witness-axiom) | Cook-Nguyen |
| `bounded_arithmetic_s_2_1` | P (PIND) | Buss |
| `bounded_arithmetic_v_np` | NP-search | Cook-Nguyen |
| `bounded_arithmetic_v_ph` | polynomial hierarchy | Cook-Nguyen |

**VerifiedExtension family** — diakrisis sub-corpora
(per `core/math/frameworks/diakrisis_*.vr`):

| Package | Role |
|---|---|
| `diakrisis_acts` | meta-classifier act language |
| `diakrisis_biadjunction` | (Fw ⊣ Eff) bicategorical adjunction |
| `diakrisis_extensions` | closure-extension theorems |
| `diakrisis_stack_model` | (∞,2)-stack model (Theorem 131.T) |

User-authored packages extend the catalogue. The `verum frameworks
list` CLI (V1, tracked via VVA §18.7 Verum-Foundation-Marketplace)
queries `core/math/frameworks/registry.vr` at runtime; entries
are deterministically ordered for CI diff review.

> **Naming history.** The pre-VVA spec referred to a "standard
> six-pack" with `arnold_mather` as a member. The shipped corpus is
> named `arnold_catastrophe` (registry stable as of B15 close), and
> the catalogue grew well beyond six. Citations for legacy
> `arnold_mather` references in ADRs and old papers should resolve
> to `arnold_catastrophe`.

### 6.3 Audit

```bash
verum audit --framework-axioms             # list used axioms
verum audit --framework-axioms --by-theorem  # which theorems depend on which
verum audit --coord                        # (Fw, ν, τ) per theorem
```

Output is deterministic (sorted by lineage, then name) to support CI-based diff review.

### 6.4 Banned patterns

- A framework axiom **cannot** refer to a symbol defined in a different framework without an explicit translation through `core.theory_interop.translate`.
- A framework axiom **cannot** mention `SmtCertificate` or tactic results.
- A framework axiom **must** have a non-empty `citation` field.

---

## 7. Dependent-Types Layer

### 7.1 Π and Σ

Grammar already exists in `03-type-system.md` / `grammar/verum.ebnf`:

```
fn f<T: Type>(x: T) -> B(x)                   // Π
(x: T, y: B(x))                                // Σ
```

Desugaring rules (quoted from 12):

- `(x: A) -> B(x)` → `Pi(x: A, B)`.
- `(x: A, B(x))` → `Sigma(x: A, B)`.
- `A -> B` (no dependency) → `Pi(_: A, B)`.

### 7.2 Paths and cubical operators

Already specified in 12 §4. CoreTerm constructors: `Path`, `PathLam`, `PathApp`, `hcomp`, `transp`, `Glue`. Univalence is available as a framework axiom in `core.math.frameworks.univalence` (lineage: HoTT Book 2013).

### 7.3 Inductive types

Grammar: existing `type_def` with `variant_list` (see `grammar/verum.ebnf` § 2.3). **No new keywords.**

```verum
public type Nat is Zero | Succ(Nat);

public type List<A: Type> is
    Nil
  | Cons(head: A, tail: List<A>);

// Indexed family: constructors may carry type arguments; index dependence
// enters through the generic parameter.
public type Vec<A: Type, n: Nat> is
    VNil
  | VCons(head: A, tail: Vec<A, Nat.pred(n)>);
```

Strict positivity enforced by kernel rule `K-Pos` — this is a kernel check, not a grammar change. Mutual inductive blocks use the existing `mutual { … }` grammar (to be verified against `verum.ebnf`; add if missing).

### 7.4 Higher inductive types

Grammar: existing `type_def` + `variant` with `path_endpoints = '=' , expression , '..' , expression` (`grammar/verum.ebnf:532`). **No `hit` keyword.**

```verum
// S¹ — path constructor expressed via existing `path_endpoints` rule.
public type S1 is
    Base
  | Loop() = Base..Base;

// Interval — canonical two-endpoint HIT.
public type Interval is
    Zero
  | One
  | Seg() = Zero..One;
```

Higher cells (2-cells, 3-cells) extend `path_endpoints` with nested path expressions — a minimal grammar addition (to be specified alongside the `K-Cell` kernel rule), not a new top-level construct.

### 7.5 Quotient types

Grammar: `type_def` with `type_expr` on the RHS invoking `Quotient<_, _>` from `core.math.quotient`. **No new keyword.**

```verum
// Equivalence relation as a separate helper predicate.
public fn eq_pair(p: (Int, Int), q: (Int, Int)) -> Bool is
    p.0 + q.1 == p.1 + q.0;

public type QuotInt is Quotient<(Int, Int), eq_pair>;
```

`Quotient<A, R>` is a stdlib-defined type with `QuotIntro` and `QuotElim` CoreTerm constructors at the kernel layer; surface syntax stays within `type ... is` grammar.

### 7.6 Quantitative / linear types

Quantities `0 | 1 | ω` (Atkey QTT). Surface syntax via attribute:

```verum
public fn consume(@1 file: File) -> Maybe<Text>;
public fn inspect(@0 x: Secret) -> Bool;    // compile-time only
public fn read_many(@ω x: Int) -> Int;       // unrestricted
```

Default quantity is ω (no tracking). Explicit quantity enables Girard's linear-logic modality `!`.

### 7.7 Cohesive modalities (optional, via `schreiber_dcct` framework axiom)

`∫` (shape), `♭` (flat), `♯` (sharp). Triple adjunction `∫ ⊣ ♭ ⊣ ♯`. Imported only when the user explicitly loads `core.math.frameworks.schreiber_dcct`.

---

## 8. Formal-Proofs Layer

### 8.1 Theorem / lemma / corollary / axiom / corollary declarations

**Grammar: existing top-level declarations** (`grammar/verum.ebnf` § 2.19) — `theorem_decl`, `lemma_decl`, `axiom_decl`, `corollary_decl`. No `@theorem` attribute; no `fn` wrapping. Verum's grammar already reserves these keywords at the declaration level.

```verum
// Theorem with explicit proof body.
theorem plus_comm(m: Nat, n: Nat) -> Prop
    ensures m + n == n + m
{
    proof {
        induction m {
            case Zero => by simp;
            case Succ(m') => by simp[IH];
        }
    }
}

// Lemma — same shape as theorem; retained as separate keyword for
// discoverability and tooling hierarchies.
lemma plus_zero_right(n: Nat) -> Prop
    ensures n + 0 == n
{
    proof { by induction n { Zero => refl; Succ(k) => by simp[IH]; }; }
}

// Corollary — cites a parent theorem with `from` (see grammar:2159).
corollary sum_of_naturals(n: Nat) -> Prop
    ensures 2 * (0 + 1 + … + n) == n * (n + 1)
from plus_comm
{
    proof { by arith_induction; }
}

// Axiom — unproven assumption; body-less per grammar:2145.
@framework(name = "lurie_htt", citation = "HTT 6.2.2.7")
axiom yoneda_fully_faithful<C: SmallCategory>(F: C -> Set, G: C -> Set)
    ensures Hom<PSh(C), F, G> equiv Nat_Trans<F, G>;
```

Under the hood: each declaration installs its body as a CoreTerm whose type equals the `ensures` proposition. The kernel rechecks. `@framework(…)` attaches only to `axiom`/`theorem`/`lemma`/`corollary` declarations (per existing `verum_kernel::load_framework_axioms`).

Attributes like `@verify(strategy)` **do** apply as adjuncts to these declarations (existing grammar), for example `@verify(certified) theorem …`.

### 8.2 Tactic DSL

Tactics are user-definable:

```verum
tactic auto {
    first {
        assumption;
        refl;
        { intro; auto };
        { split; auto };
        { apply_hypothesis; auto };
        { unfold_definition; auto };
    };
}

tactic induction_auto {
    induction *;
    all_goals auto;
}
```

Built-in tactics (non-exhaustive):

| Tactic | Purpose |
|---|---|
| `assumption` | Match goal against hypothesis |
| `refl` | Reflexivity |
| `sym`, `trans` | Symmetry / transitivity |
| `intro [x]` | Introduce Π / ∀ binder |
| `split` | Split conjunction / pair |
| `left` / `right` | Choose ∨ alternative |
| `exact e` | Supply term `e` |
| `apply l [with …]` | Apply lemma / hypothesis |
| `rewrite [←] h [at x]` | Equational rewrite |
| `simp [lemmas]` | Simplification (confluent) |
| `ring`, `field`, `omega` | Algebraic solvers |
| `blast` | Tableau prover |
| `auto [with hints]` | Proof search |
| `smt [(solver = "Z3", timeout = 5000)]` | SMT dispatch |
| `induction x` | Structural induction |
| `cases x` | Case split |
| `unfold f` | Definitional unfolding |
| `compute` | Reduction to normal form |
| `try t` | Attempt `t`; no-op on fail |
| `repeat t` | Repeat until fail |
| `first { t₁; … }` | First success wins |
| `category_simp` | Naturality / Yoneda / Kan simplifier |
| `descent_check` | Čech-descent obligation |
| `goal_intro` | Snapshot current goal |
| `hypotheses_intro` | Snapshot hypothesis list |
| `quote expr` | Meta: quote a tactic expression |
| `unquote handle` | Meta: splice back |
| `fail [msg]` | Explicit failure |
| `sorry` / `admit` | Placeholder (emits `[AD]` in certificate) |
| `done` | QED marker |

Users can add tactics; each must be a function `fn(State) -> Maybe<State>`.

### 8.3 Proof search / hints database

```verum
@hint(priority = 100)
public lemma useful_lemma: forall (x: Nat). (x + 0 == x);

@hint(pattern = "_ + _ == _")
public lemma plus_properties: … ;

// consumption
@theorem
public fn example: P {
    by auto with hints;
}
```

Hint selection respects priority and pattern match against the current goal.

### 8.4 SMT integration

`verum_smt` portfolio (Z3, CVC5, E, Vampire) dispatches `smt` tactic calls. Returns a CoreTerm `SmtCertificate`. Kernel rechecks. If any solver returns UNSAT with a certificate, the kernel verifies; any UNKNOWN or SAT disposition propagates as `Unprovable` and the tactic fails.

Timeout per backend: 5 s default, configurable via `@verify(strategy, timeout_ms = …)`. Cache hits are free.

### 8.5 Certificate export (five formats, day-one)

```bash
verum export --to lean ./src/theorems.vr   -o ./out/theorems.lean
verum export --to coq    ./src/theorems.vr -o ./out/Theorems.v
verum export --to agda   ./src/theorems.vr -o ./out/Theorems.agda
verum export --to dedukti ./src/theorems.vr -o ./out/theorems.dk
verum export --to metamath ./src/theorems.vr -o ./out/theorems.mm
```

Each exporter is a CoreTerm walker. Lossless export is guaranteed for theorems whose framework axioms have a known mapping in the target system (`lurie_htt` → Lean `mathlib.CategoryTheory`, `baez_dolan` → HoTT-Book chapter 8, etc.). Axioms without a mapping are emitted as target-specific `axiom` declarations with a comment referencing the Verum lineage.

**Independent checking** round-trips:

```bash
verum export --to lean ./th.vr -o th.lean && lean th.lean
verum export --to coq ./th.vr -o Th.v && coqc Th.v
```

On CI, `verum ci --export-roundtrip` runs all five for smoke coverage.

### 8.6 Program extraction

```verum
@extract
@theorem
public fn div_mod_unique(a: Nat, b: Nat{>0}) ->
    exists! (q: Nat, r: Nat). a == b * q + r && r < b { … }

// extraction generates:
public fn div_mod(a: Nat, b: Nat{>0}) -> (Nat, Nat) = extract(div_mod_unique(a, b));
```

Witness extraction (`@extract_witness`), contract extraction (`@extract_contract`) follow the same pattern. Realize directive `@extract(realize = "native_div_mod")` maps to a prebuilt runtime.

### 8.7 Mathematical libraries

Shipped / planned:

- `core.math.number_theory` — primes, fundamental theorem, Euler, Fermat.
- `core.math.topology` — topological spaces, continuity, compactness.
- `core.math.analysis` — limits, continuity, intermediate value, completeness.
- `core.math.category_theory` — Yoneda, Kan, (co)limits, monads, adjunctions.
- `core.math.infinity_topos` — ∞-topos, descent, cohesion.
- `core.math.group_theory` — groups, subgroups, quotients, homomorphisms.
- `core.math.algebra` — rings, fields, modules, ideals.

### 8.8 Hoare / separation / refinement verification

```verum
notation {P} c {Q} := forall (s: State). P(s) => wp(c, Q)(s);

@theorem
public fn swap_correct(x, y: Var) ->
    {x == a && y == b}
    [ tmp := x; x := y; y := tmp ]
    {x == b && y == a} {
    by wp_calculus;
}
```

Separation logic assertions use `P * Q` notation; frame rule is a registered lemma. Data-refinement structures (`Refinement<A, C>`) live in `core.verify.refinement`.

### 8.9 Interactive mode

```bash
verum repl --proof ./my_theorem.vr
```

Lean-like interactive prompt with live goal state, tactic history, undo stack. Integrates with LSP for Ctrl-hover goal display in editors.

### 8.10 Proof engineering

- `@depends_on(…)` explicit dependency list (CI churn detection).
- `@since("2.0.0")` versioning.
- `@deprecated(use = "new_theorem")` migration markers.
- `verum refactor-proof <old> <new>` rewrite tool.
- `verum metrics <file>` ProofMetrics (size, depth, tactic set, automation ratio, dependencies).
- `verum coverage <module>` — proved / verified / unproved / critical-missing.

---

## 9. Meta-System

### 9.1 Staged meta-programming

Three stages:

- **Stage 0** — runtime code.
- **Stage 1** — compile-time code executed during elaboration (`meta fn`).
- **Stage N ≥ 2** — code that generates stage (N-1) code.

```verum
meta fn double_body(body: TokenTree) -> TokenTree {
    quote { $body; $body }
}

@macro
public fn retry_twice($body: Block) = double_body(body);
```

### 9.2 Meta primitives

Already partially landed in `core.proof.tactics.meta` (see also the stdlib-parser fix for `quote` as a tactic name):

- `quote { expr }` / `` `expr `` — syntactic quoting.
- `unquote(handle)` / `$(handle)` — splicing.
- `goal_intro` — current-goal snapshot.
- `hypotheses_intro` — hypothesis-list snapshot.

Roundtrip law: `unquote(quote(t)) ≡ t` structurally.

### 9.3 `@derive` / `@const` / `@cfg`

- `@derive(Eq, Hash, Clone, Debug, Display, …)` — method synthesis from protocols. Drives M3-accessibility of protocol classifiers.
- `@const fn` — compile-time evaluable (used in type-level computation).
- `@cfg(target_os = "linux", feature = "foo")` — platform gating.
- `@sql_query("select …")` — compile-time DSL parsing.

### 9.4 Meta contexts

`meta fn` has restricted context access: only `CompileTime` contexts are visible (no `Database`, `FS`, `Network`, …). Enforced at elaboration.

### 9.5 Staging levels

`meta(N) fn` for `N ∈ {1, 2, 3}`. `meta(1)` = standard compile-time; `meta(2)` = generates stage-1 code; `meta(3)` = generates stage-2 code. Higher stages available to library authors; user code typically uses `meta(1)` only.

---

## 10. `core.theory_interop` — Three Operations

### 10.1 `load_theory(T)` — Yoneda

```verum
public fn load_theory(T: TheoryDescriptor) -> Articulation {
    // 1. Parse T as a presentation of a category (objects, morphisms, relations)
    // 2. Compute the Yoneda embedding y: T → PSh(T)
    // 3. Register result as an Articulation with framework ref = T.lineage
}
```

`TheoryDescriptor` is a structured value bundling axioms, signature, optional model category.

### 10.2 `translate(source, target, partial_map)` — Kan extension

```verum
public fn translate(
    source: Articulation,
    target: Articulation,
    partial: ClaimTranslationMap,
) -> Result<ArticulationFunctor, ObstructionReport> {
    // 1. Let F_0: source.generators → target be the partial specification
    // 2. Compute Lan F_0 (pointwise left Kan extension)
    // 3. Check naturality and commute conditions
    // 4. Return Functor, or ObstructionReport with metric Obs(F) ∈ [0, 1]
}
```

Obstruction thresholds:

| Range | Verdict |
|---|---|
| `Obs(F) == 0` | Morita-equivalence witness |
| `(0, 0.05]` | Strong translation |
| `(0.05, 0.20]` | Moderate translation |
| `(0.20, 0.50]` | Weak translation |
| `(0.50, 1]` | Untranslatable (reject) |

### 10.3 `check_coherence(translations)` — Čech descent

```verum
public fn check_coherence(
    translations: List<ArticulationFunctor>,
) -> Result<DescentWitness, CoherenceFailure> {
    // 1. Build cosimplicial diagram of pairwise intersections
    // 2. Verify cocycle condition on triple overlaps
    // 3. Return witness on success
}
```

Fails when translations disagree on any double overlap. Error includes the specific cocycle that failed.

### 10.4 MSFS coordinate

`coord` projects a single articulation to its MSFS coordinate; `theorem_coord` extends the projection to a theorem (which may depend on multiple framework axioms) by returning the *upper bound* in the (Framework, ν, τ) lattice:

```verum
public fn coord(alpha: Articulation) -> (Framework, Ordinal, TauFlag) {
    (alpha.framework, nu(alpha), tau(intensional(alpha)))
}

/// The MSFS coordinate of a theorem T is the upper bound (over the
/// (Fw, ν, τ) lattice) of `coord(α)` for each framework axiom α the
/// theorem depends on.  `Framework` is joined as an unordered set of
/// names; `Ordinal` is the supremum; `TauFlag` is conjunctive
/// (intensional ⇔ all dependencies intensional, else extensional).
///
/// For a theorem with no @framework dependency the coordinate is
/// (∅, 0, intensional) — the kernel-only baseline.
public fn theorem_coord(t: Theorem) -> (Set<Framework>, Ordinal, TauFlag) {
    let frameworks = t.framework_dependencies();
    let ordinal    = frameworks.iter().map(coord_ordinal).fold(0, ordinal_max);
    let tau        = frameworks.iter().all(is_intensional);
    (frameworks.iter().map(framework_name).collect(), ordinal, tau)
}
```

`verum audit --coord` computes `theorem_coord` for every theorem; outputs a sorted table grouped by framework. The CLI also exposes a per-theorem view (`--per-theorem`) that prints one row per theorem with its full dependency set.

**Well-definedness.** `theorem_coord` is single-valued because `Ordinal` and `Set<Framework>` admit canonical joins (ordinal sup; set union); `TauFlag` is a Bool and `∧` is well-defined. The empty-dependency case (`∅, 0, intensional`) is the lattice's bottom element and matches the kernel-only baseline.

### 10.4.1 Architectural relationship — meta-classifier vs. coordinate-point frameworks

The (Framework, ν, τ) coordinate space introduced above is not abstract structure that materialises out of nothing — it is supplied by **one specific framework that occupies the meta-classifier role**. In the current packaging this framework is `diakrisis`; in principle any L5+ canonical-primitive metaclassification system could play the same role.

**Two levels of `@framework` declarations:**

1. **Meta-classifier framework** — declares the canonical-primitive structure that *defines* the (ν, τ, ε) coordinate space. The five axioms (relative-consistency under 2 inaccessibles, articulation/enactment Morita duality, dual no-go for absolute practice, dual gauge-surjection kernel, dual-primitive initial-object) jointly establish the metaclassification. See `core.math.frameworks.diakrisis`.

2. **Coordinate-point frameworks** — every other `@framework` package (`lurie_htt`, `schreiber_dcct`, `connes_reconstruction`, `petz_classification`, `arnold_catastrophe`, `baez_dolan`, `owl2_fs`, `bounded_arithmetic_*`, ...) is a *named coordinate point* within that space. Each such framework carries a fixed `(ν, τ)` location that records its meta-classifier-relative position; the meta-classifier doesn't decide *which* theorems hold inside a coordinate-point framework — it only fixes the framework's stratification depth and intensionality flag relative to all other points.

**Concretely**, when downstream code writes

```verum
@framework(lurie_htt, "HTT 5.2.7 (Adjoint Functor Theorem)")
public axiom adjoint_functor_theorem<F>(...) -> Bool ensures ...;
```

two simultaneous claims are being made:

- **(External rigour)** "this axiom postulates HTT 5.2.7" — the framework is rigorous in its own external literature, and an auditor can re-check the cited theorem against the source.
- **(Coordinate-relative position)** "this axiom lives at coordinate `(ν=ω, τ=intensional)` in the meta-classifier system" — and that coordinate value is fixed by the meta-classifier's prescriptive lookup, NOT by lurie_htt itself.

The two readings coexist because the meta-classifier is *coordinate-system-defining* but *theorem-content-neutral*: it tells you where a framework sits but not what it asserts.

**Implementation contract:**

- `core.math.frameworks.diakrisis` (the meta-classifier package) ships **5 axioms** describing the coordinate system itself.
- Every other framework file ships axioms describing its own theorems, and its `(ν, τ)` coordinate is supplied by `core.theory_interop.coord::known_ordinal` / `known_tau` lookup. Those lookups are **prescribed** by the meta-classifier's canonical coordinate table; CLI `verum audit --coord` synchronises the two.
- Because the meta-classifier *is* the coordinate system, it does NOT itself have a `(ν, τ)` value. Asking "what is `coord(diakrisis)`?" is a category mistake — analogous to asking "what coordinate does the origin of a coordinate system have?". Implementations should reflect this: `coord_of(α)` returns a sentinel for the meta-classifier (or alternatively the lattice top, by convention) so user code that accidentally queries the meta-classifier's coord doesn't silently produce a wrong number.
- When the meta-classifier's prescriptive table updates (a coordinate-point framework's `(ν, τ)` is refined), Verum's lookup MUST follow. Drift between the meta-classifier prescription and Verum's lookup is a bug, not a degree of freedom.

**Consequence for `theorem_coord`:**

A theorem that depends on framework axioms from `lurie_htt` and `petz_classification` has

```
theorem_coord(T) = ({lurie_htt, petz_classification}, sup(ω, 2), intensional ∧ extensional)
                = ({lurie_htt, petz_classification}, ω, extensional)
```

The `diakrisis` meta-classifier itself is an *implicit* dependency of every coordinate-bearing theorem (because the coordinate values are read from its prescriptive table), but it is not a *content* dependency — the theorem doesn't postulate any of the 5 meta-classifier axioms unless the user wrote `@framework(diakrisis, …)` explicitly. CLI tooling reflects this by listing `diakrisis` as a "structural dependency" rather than a "content dependency" in audit reports.

**Future: pluggable meta-classifiers.**

VVA leaves room for a second meta-classifier (a non-Diakrisis L5+ framework that defines its own (ν, τ, ε) system). If such a thing is ever introduced — for example to support a non-2-inaccessible cardinal hierarchy or a different gauge primitive — it would be a *parallel* coordinate system, and theorems would carry one coordinate per active meta-classifier. Until then, `diakrisis` is the *unique* meta-classifier and the coordinate space is single.

---

## 11. Dual Stdlib — OC + DC

### 11.1 OC layer (`core.math.*`) — articulations

Already partially present. VVA stabilises module paths:

- `core.math.frameworks` — `@framework` axiom packages.
- `core.math.category` — small / locally-small categories, functors, naturals.
- `core.math.simplicial` — Δ, simplicial objects, realization.
- `core.math.infinity_category` — (∞,1)-categories, joins, slices.
- `core.math.infinity_topos` — ∞-topos, descent, cohesion.
- `core.math.hott` — HoTT primitives, equivalence, univalence (framework).
- `core.math.cubical` — CCHM primitives.
- `core.math.operad` — operads, algebras, Koszul duality.
- `core.math.fibration` — Grothendieck fibrations, straightening.
- `core.math.kan_extension` — Lan / Ran (pointwise, global).
- `core.math.logic` — propositional / first-order / modal / temporal / epistemic logics as data.

### 11.2 DC layer (`core.action.*`) — enactments (Actic dual)

```
core.action.primitives
    ε_math, ε_compute, ε_observe, ε_prove, ε_decide, ε_translate, ε_construct
core.action.articulation
    Articulation { framework, citation, lineage }, primitive_articulation,
    raw_actic_articulation, articulation_eq
core.action.enactments
    compose / enact_then       — sequential composition (Diakrisis: A then B)
    enact_par                  — parallel composition  (Diakrisis: A || B)
    activation / activate      — A-modality, single step
    activation_iterate / activate_n — bounded activation by finite step count
    is_autopoietic             — predicate: rank ≥ AUTOPOIETIC_THRESHOLD
    autopoiesis                — A^{ω²} closure operation
    epsilon, alpha_of, is_adjoint — α ⊣ ε adjunction (§11.3)
core.action.gauge
    canonicalise, gauge_equivalent
core.action.verify
    AuditVerdict, verify_epsilon, gauge_consistency, autopoietic_flag
```

Both Diakrisis-canonical names (`enact_then`, `enact_par`, `activate`, `activate_n`, `autopoiesis`) and Verum-historic names (`compose`, `activation`, `activation_iterate`, `is_autopoietic`) are public and stable; user code may pick either. The two name-sets are exact aliases at the implementation layer (`core/action/enactments.vr`).

### 11.3 Auto-induced duality

For every `α: Articulation`, VVA guarantees:

```verum
public fn epsilon(alpha: Articulation) -> Enactment = {
    // 108.T auto-induced: ε(α) = (F, Syn(F), id, id) — syntactic self-enactment
    Enactment::syntactic_self(alpha)
}

public fn alpha_of(e: Enactment) -> Articulation = e.articulation;  // α ⊣ ε
```

**What "compiler witnesses α ⊣ ε" means precisely.** The adjunction holds at the level of *core data*, not of categorical 2-cells:

- **Unit identity (`η_α : α ≡ alpha_of(epsilon(α))`).** For every `α: Articulation`, the kernel emits a definitional equality `alpha_of(epsilon(α)) = α` after canonicalisation (whitespace + lineage normalisation). This is enforced by the typing rule `K-Adj-Unit`:

  ```
    Γ ⊢ α : Articulation
    ────────────────────────────────────── (K-Adj-Unit)
    Γ ⊢ alpha_of(epsilon(α)) ≡ α : Articulation
  ```

  Concretely: `epsilon(α)` stores `α` in its `articulation` field; `alpha_of` returns that field. The composition is the identity by construction.

- **Counit identity (`ε_e : epsilon(alpha_of(e)) ↦ canonicalise(e)`).** For an arbitrary enactment `e`, `epsilon(alpha_of(e))` is the *syntactic self-enactment* of `e`'s articulation — it is **not** definitionally `e` (a generic enactment carries non-trivial steps; the syntactic self has none). Instead, `core.action.gauge.gauge_equivalent(epsilon(alpha_of(e)), e)` decides whether `e` is gauge-equivalent to its syntactic self, and `canonicalise(e)` projects to a canonical representative inside that equivalence class.

  ```
    Γ ⊢ e : Enactment
    ──────────────────────────────────────────────────────────── (K-Adj-Counit)
    Γ ⊢ canonicalise(epsilon(alpha_of(e))) ≡ canonicalise(e) : Enactment
       (when e is gauge-equivalent to its syntactic self)
  ```

  Enactments that are **not** gauge-equivalent to their syntactic self carry observational content beyond the articulation (e.g. a `[ε_observe; ε_prove]` chain is not equivalent to `epsilon(prove_articulation)`); the counit identity is then a *proper* lax 2-cell, witnessed by the gauge-canonicalisation residual.

- **Triangle identities.** `alpha_of(epsilon(alpha_of(e))) ≡ alpha_of(e)` (vacuous from K-Adj-Unit) and `epsilon(alpha_of(epsilon(α))) ≡ epsilon(α)` (vacuous from K-Adj-Unit composed with `epsilon(α).articulation = α`).

The implementation in `core/action/enactments.vr` provides `is_adjoint(α, e)` as a runtime decision procedure for the unit identity; `core/action/gauge.vr::gauge_equivalent` decides the counit identity. Compile-time, the elaborator inserts `K-Adj-Unit` proofs whenever a user value of type `Articulation` is observed through an `Enactment` round-trip.

### 11.4 `@enact` annotation

```verum
@enact(epsilon = "ε_prove")
public fn prove_yoneda() -> Proven<Yoneda> using [Theorem] { … }
```

- Accepted values: `"ε_math"`, `"ε_compute"`, `"ε_observe"`, `"ε_prove"`, `"ε_decide"`, `"ε_translate"`, `"ε_construct"`, `"ε_classify"` (the OWL 2 ontology-classification primitive added in the VVA §21 V1 ecosystem), or a user-defined ε from `core.action.primitives`. Both Unicode and ASCII spellings (`epsilon_*`) are accepted; the typed attribute canonicalises to the Unicode form for deterministic audit output.
- Audited by `verum audit --epsilon src/` — prints a distribution.
- Enforces consistency: a function with `ε_prove` cannot call `ε_compute`-only operations (type-level check).

---

## 12. Verification-Strategy Ladder (full spec)

### 12.1 `@verify(runtime)`

- No compile-time discharge.
- Emit `assert!` / `debug_assert!` at each obligation site.
- Cost: O(1) per check in the generated binary.
- Applicability: any function.

### 12.2 `@verify(static)`

- Conservative static analysis: CBGR lifetime, simple dataflow, constant folding, bounds simplification.
- No SMT.
- Cost: part of parallel pass; <10 ms per function.
- Applicability: default for AOT builds.

### 12.3 `@verify(fast)`

- Single-solver SMT (Z3) with bounded timeout (default 100 ms).
- UNKNOWN → conservative accept (issues warning).
- Cost: ≤ 100 ms/goal.
- Applicability: CI-fast builds.

### 12.4 `@verify(formal)`

- Portfolio SMT (Z3 + CVC5) with 5 s timeout.
- UNKNOWN from any solver → conservative accept.
- Cost: ≤ 5 s/goal on average.
- Applicability: correctness-critical modules.

### 12.5 `@verify(proof)`

- User supplies `proof { … }` block (tactic proof).
- Kernel rechecks.
- Cost: unbounded user time.
- Applicability: theorems, foundational lemmas.

### 12.6 `@verify(thorough)`

- `formal` + require explicit `decreases`, `invariant`, `frame` specifications.
- No default "total function" assumption.
- Cost: ≈ 2× `formal`.
- Applicability: safety-critical code.

### 12.7 `@verify(reliable)`

- `thorough` + Z3 AND CVC5 must both return UNSAT.
- Any disagreement → UNKNOWN.
- Cost: ≈ 2× `thorough`.
- Applicability: security audits.

### 12.8 `@verify(certified)`

- `reliable` + certificate materialisation + `kernel_recheck` + multi-format export.
- Any recheck failure → compile error.
- Cost: ≈ 3× `thorough`.
- Applicability: aerospace, pharma, legal.

### 12.9 `@verify(synthesize)`

- Inverse proof search across $\mathfrak{M}$ — find missing lemmas / auxiliary theorems.
- ν ≤ ω·3+1 (per MSFS Thm 4.4).
- Cost: unbounded; often times out.
- Applicability: exploratory research, automated mathematician.

### 12.10 Monotone lifting

Any function that passes `@verify(N)` also passes `@verify(M)` for `M ≤ N` (partial order from §2.3). The build system supports per-module strategy override.

---

## 13. Articulation Hygiene

### 13.1 Principle (NO-19)

Every surface "self-X" (`Self`, `This`, `Same`, `Own`, …) in Verum must factorise as `(Φ, Φ^κ, t)` where Φ is an explicit endofunctor, Φ^κ its κ-iteration, and t a terminal or fixed object.

### 13.2 Hygiene table

| Surface | Factorisation (Φ, κ, t) |
|---|---|
| `Self` (in protocol body) | `(Id, 1, receiver_object)` |
| `Self::Item` | `(AssocProj<Item>, 1, receiver_type)` |
| `@recursive fn f(… -> Self) …` | `(unfold_f, ω, fix_f)` |
| Mutable `&mut self` | `(Id_via_write, 1, current)` |
| Inductive `Rec(…)` in `type T is Base | Rec(T)` | `(T_succ, ω, least_fp)` |
| Coinductive `Stream<A> = Cons(A, Stream<A>)` | `(T_prod_A, ω^{op}, greatest_fp)` |
| HIT `S1` with `loop: base = base` | `(loop_action, ω, base)` |
| Quote `` `expr `` (meta) | `(Meta_lift, 1, expr_ast)` |

### 13.3 Compiler enforcement (`core.articulation.hygiene`)

**V1 reporter (shipped)** — `verum audit --hygiene` (`crates/verum_cli/src/commands/audit.rs::audit_hygiene_with_format`) walks every type and function declaration in the project and classifies each self-referential surface form per the §13.2 hygiene table, emitting the (Φ, κ, t) factorisation alongside each item. Recognised classes:

| Surface detected by V1 | Hygiene class | Factorisation |
|---|---|---|
| `type T is Base \| Rec(T)` | `inductive` | `(T_succ, ω, least_fp)` |
| `type Tree<A> is Branch(Tree<A>, Tree<A>)` (Generic-arg recursion) | `inductive` | as above |
| `type Stream<A> is coinductive { … }` | `coinductive` | `(T_prod, ω^op, greatest_fp)` |
| `type S1 is Base \| Loop() = Base..Base` (path-cell variant) | `higher-inductive` | `(path_action, ω, base)` |
| `type X is (Y)` (newtype body) | `newtype` | `(Id, 1, base)` |
| `@recursive fn f(...) -> Self` | `recursive-fn` | `(unfold_f, ω, fix_f)` |
| `@corecursive fn g(...)` | `corecursive-fn` | `(corec_g, ω^op, fix_g)` |

Output formats: `plain` (default, with Unicode rendering of the factorisation) and `json` (stable schema_version=1, with `classes[].entries[].file` for CI consumption).

**V2 enforcement (deferred)** — `verum check --hygiene src/` will additionally walk raw `self` occurrences inside function bodies and the `Self::Item` / `&mut self` factorisations from §13.2 (these require a typed resolution pass beyond the syntactic AST walk used by the V1 reporter). Missing or ambiguous factorisation will produce `E_HYGIENE_UNFACTORED_SELF`.

**Rationale.** Yanofsky paradox closure (105.T) requires that every self-reference goes through an explicit iteration with depth witness. `K-Refine` enforces this at the CoreTerm level; `core.articulation.hygiene` lifts the same discipline to surface syntax. The V1 reporter is non-binding (advisory only) so existing code is not broken; V2 promotes the diagnostics to errors once the surface coverage is complete.

---

## 14. Noesis Boundary

Noesis (`internal/holon/internal/diakrisis/docs/11-noesis/`) is a **separate project** that will use Verum. VVA defines what Verum owes Noesis and what Noesis handles internally.

### 14.1 What Verum provides

- Three refinement forms (§5).
- `@framework` axiom system (§6).
- Dependent types layer (§7).
- Formal-proof layer with five-format certificate export (§8).
- `core.theory_interop` (§10).
- Dual OC/DC stdlib (§11).
- Nine-strategy verification ladder (§12).
- Articulation Hygiene (§13).
- Kernel soundness guarantees (§4.5).

### 14.2 What Noesis builds

- Giry-monad LLM agent (Kleisli morphism `A: Context → G(Operations)`).
- Knowledge-object storage (Markdown+YAML per claim + SQLite + Git).
- NP (Noesis Protocol) — 46 JSON-RPC endpoints over MCP.
- Federation layer (distributed sheaf with descent checks).
- T_meta self-model layer.
- Quantum-epistemic lattice / Day-convolution cognitive extension (Phase 6).

### 14.3 Contract points

- Noesis consumes `verum audit --coord` output as knowledge-graph metadata.
- Noesis's `knowledge/audit` calls through to `verum check --hygiene --framework-axioms`.
- Noesis emits `@framework` declarations for every theory it imports.
- Noesis's `agent/propose` pipeline filters through `@verify(formal)` before surfacing to the user.

### 14.4 Not owned by Verum

- LLM inference — external.
- UHM-specific formalisation (223 theorems) — lives in Noesis's `Path-B` programme, using Verum as the implementation language.
- Orthomodular epistemic lattice — Noesis research prototype.
- OWL2 / SWRL / BFO / Cyc / SUMO corpus importers — Noesis stdlib.

---

## 15. Integration with Existing Verum Features

### 15.1 CBGR

Refinement checks run **after** CBGR validation. CBGR is the runtime safety floor; refinements are the compile-time property layer above it. No circular dependency.

### 15.2 Context system

`using [Database, Logger]` contexts are orthogonal to framework axioms. A function `fn f using [Database]` with `@verify(proof)` must supply a tactic proof of its postcondition; the context list is a runtime dependency, not a proof obligation.

### 15.3 Computational properties

Properties `{Pure, IO, Async, Fallible, Mutates}` compose with verification strategies. A `Pure` function with `@verify(formal)` produces clean SMT queries; a `Mutates` function requires frame specifications.

### 15.4 AOT / Interpreter tiers

| Tier | Verification | Behaviour |
|---|---|---|
| VBC (Tier 0) | `@verify(runtime)` assertions | Full checks |
| AOT (Tier 1) | `@verify(static)` + optional runtime | CBGR + proved obligations erased |
| AOT-verified (Tier 2) | `@verify(formal)` or stricter | Full erasure — zero runtime cost on proved paths |
| Certified (Tier 3) | `@verify(certified)` | Certificate alongside binary |

### 15.5 Kernel re-check invariant

The kernel is called once at build time (per module) and produces a **verification receipt** (`target/verify/<module>.receipt`) recording:

- CoreTerm hash
- Framework-axiom set
- Tactic invocations and their certificates
- SMT-backend disposition summary
- `(Fw, ν, τ)` coordinate

Receipts are content-addressed; CI compares against a committed baseline.

---

## 16. Migration Path

### 16.1 Phase 1 (0–3 months) — Architectural alignment

- **Task A1**: move concrete SMT impls fully into `verum_smt` (already done: cycle broken, `type_translator` landed — see session commits `d95c4362`, `2d0fefac`).
- **Task A2**: document the three refinement forms' canonical `Refined` CoreTerm (this spec, §5). Land a grammar test.
- **Task A3**: land `@framework(name, citation)` parsing + kernel `FrameworkAxiom` node.
- **Task A4**: expose `verum audit --framework-axioms` CLI.
- **Task A5** (✓ shipped): `K-Refine` depth-check is wired in the kernel — `crates/verum_kernel/src/lib.rs::KernelError::DepthViolation { binder, base_depth, pred_depth }` (variant at line 1010, emitted at 1325). Ten regression tests exercise the rule end-to-end at `crates/verum_kernel/tests/k_refine_depth.rs`. The spec-name `E_K_DEPTH_VIOLATION` is an informal alias of the variant.

### 16.2 Phase 2 (3–6 months) — Gradual ladder

- **Task B1**: add `@verify(runtime|static|fast|formal|proof|thorough|reliable|certified|synthesize)` with their ν-values.
- **Task B2**: thread strategies through `verum_smt` portfolio dispatcher.
- **Task B3**: certificate emitter skeleton for Lean 4 and Dedukti (smallest two first).
- **Task B4**: land `verum export --to <lean|dedukti>` with round-trip CI check.

### 16.3 Phase 3 (6–12 months) — Dependent types

- **Task C1**: Π / Σ / Path CoreTerm nodes (partially present — complete).
- **Task C2** (✓ shipped): Inductive types with strict-positivity check (`K-Pos`). `crates/verum_kernel/src/lib.rs` ships `InductiveRegistry`, `RegisteredInductive { name, params, constructors }`, `ConstructorSig { name, arg_types }`, and `check_strict_positivity(target, ty, ctx)` — the strict-positivity walker per VVA §7.3. The walker descends through `Pi`, `Sigma`, `Inductive`, `App`, `Refine`, `PathTy`, `Lam`; on `Pi(domain, codomain)` it forbids any occurrence of `target` in the negative position (`domain`) and recursively validates `codomain`. `KernelError::PositivityViolation { type_name, constructor, position }` carries a breadcrumb to the offending site. Berardi-shaped definitions (`type Bad = Wrap(Bad → A)`, second-order `Bad2 = Wrap((Bad2 → A) → A)`, indirect non-positive via parametrised inductives) are all rejected. Thirteen end-to-end tests at `crates/verum_kernel/tests/k_pos_strict_positivity.rs` exercise both directions.
- **Task C3**: HITs with eliminator auto-gen.
- **Task C4**: Quotient types.
- **Task C5** (V1 + V2 ✓ shipped): Quantitative annotations per Atkey QTT (Atkey 2018 / McBride 2016). V1 (commit f73c44de) shipped the *declaration discipline*: `crates/verum_ast/src/attr/typed.rs` `Quantity { Zero, One, Many }` enum + `QuantityAttr` typed attribute, surface forms `@quantity(0)` / `@quantity(1)` / `@quantity(omega)` (and aliases `linear`, `erased`, `unrestricted`). V2 (#235) ships the **enforcement pass**: `crates/verum_types/src/infer.rs::extract_quantity_from_attrs` reads each parameter's actual `@quantity(...)` attribute (instead of hardcoded Omega); the function-body validator walks BOTH block.stmts AND block.expr (was tail-only) via `walk_stmt_for_qtt_usage`; observed usage is checked against declared quantities through `qtt_usage::check_usage`. Violations surface via `tracing::warn!` (V2 stance — promoting to a hard error is a future minor bump after the in-tree corpus is annotated). 6 V2 unit tests in `crates/verum_types/src/infer.rs::qtt_v2_enforcement_tests` cover empty/Zero/One/Many extraction + first-wins precedence + unrelated-attr filter.
- **Task C6** (✓ shipped — see B15 close in §6.2): Framework axioms — `core/math/frameworks/` ships nine Standard packages (`diakrisis`, `actic.raw`, `petz_classification`, `arnold_catastrophe`, `owl2_fs`, `lurie_htt`, `connes_reconstruction`, `baez_dolan`, `schreiber_dcct`) plus the bounded-arithmetic VerifiedExtension family and four diakrisis sub-corpora. The pre-VVA "standard six-pack" naming is retained only as historical reference; the live catalogue is enumerated in `core/math/frameworks/registry.vr::populate_canonical_standard` and surfaced via `verum frameworks list` (V1 tracked via VVA §18.7). Note that `arnold_mather` was renamed to `arnold_catastrophe` to match the shipped registry entry.
- **Task C7** (V1 ✓ shipped): `core.math.frameworks.owl2_fs` package — 64 trusted-boundary `@framework(owl2_fs, ...)` axioms in nine sub-modules (object_property / data_range / class_expr / class_axiom / object_property_axiom / data_property_axiom / datatype_definition / key / assertion) + types.vr (Individual / Literal sorts + count_o quantifier-of-quantity). V1 ships axiom *signatures* with `ensures true;` placeholder bodies; V2 will replace each placeholder with the verbatim Shkotin Table-row HOL definition so SMT dispatch via CVC5 FMF can decide encoded obligations. `verum audit --framework-axioms --by-lineage owl2_fs` enumerates the OWL 2 footprint of any corpus; `verum audit --coord` projects owl2_fs theorems to ν=1, τ=intensional.
- **Task C7b** (V1 ✓ shipped — 24/30 markers): Canonical bridge `owl2_fs → lurie_htt` at `core/theory_interop/bridges/owl2_to_htt.vr`. 24 trusted-boundary `@framework(owl2_fs, "Bridge owl2_fs → lurie_htt: <op> → <HTT image>")` axioms structured in 4 tiers: class-level (5), object-property (6), characteristic flags (7), class-expression constructors (4) — covering Class→Presheaf, SubClassOf→Monomorphism, EquivalentClasses→Isomorphism, DisjointClasses→InitialPullback, ObjectProperty→Functor, HasKey→RepresentablePresheaf, the 7 characteristic flags as functor properties, ObjectIntersectionOf→Limit, ObjectUnionOf→Colimit, ObjectSomeValuesFrom→∃-image, ObjectAllValuesFrom→Π-right-adjoint. The remaining ~6 markers (ObjectHasValue, ObjectComplementOf, secondary flags) are V2 — they require deeper PSh-internal-language details and are tracked alongside the typed `@framework_translate` attribute. Aggregator at `core/theory_interop/bridges/mod.vr`. VCS smoke at `vcs/specs/L1-core/owl2_fs/owl2_to_htt_bridge_smoke.vr`.
- **Task C8** (V1 ✓ shipped): `OwlAttr` family in `verum_ast::attr::typed` — full §21.6 coverage with seven typed attributes (`Owl2ClassAttr`, `Owl2SubClassOfAttr`, `Owl2DisjointWithAttr`, `Owl2CharacteristicAttr`, `Owl2PropertyAttr`, `Owl2EquivalentClassAttr`, `Owl2HasKeyAttr`) plus the `Owl2Semantics` (OpenWorld / ClosedWorld) and `Owl2Characteristic` (7-element flag enum) accessory types. Each attribute parser (a) accepts every legal surface shape per §21.6 (named-arg colon / equals; bracketed list / positional list; class identifier or string literal); (b) rejects unknown enum variants and named-arg keys instead of silently dropping them — typos surface as parse errors. Fifteen round-trip tests in `crates/verum_ast/tests/attr_tests.rs` exercise every attribute and every reject-path.
- **Task C9** (V1 ✓ shipped): `count_o` quantifier-of-quantity primitive + `E_OWL2_UNBOUNDED_COUNT` diagnostic at `core/math/frameworks/owl2_fs/count.vr`. V1 ships `count_o<I: Individual>(domain, pred) -> Int` (linear-time fold over an explicit closed domain), `count_o_unbounded(...) -> Maybe<Int>` (returns `Maybe::None` to signal `E_OWL2_UNBOUNDED_COUNT` when no domain witness is available), and `assert_finite_domain` (panics with the diagnostic name when the domain is missing). V2 will wire CVC5 Finite Model Finding into `verum_smt::backend_switcher` so `count_o_unbounded` is decided automatically when `@owl2_class(semantics = ClosedWorld)` is in scope.
- **Task B5** (export V1 ✓ shipped; import deferred): `verum export --to owl2-fs` emits W3C OWL 2 Functional-Style Syntax via `crates/verum_cli/src/commands/export.rs::emit_owl2_fs`. Walks the shared `Owl2Graph` (single source of truth between this exporter and `audit --owl2-classify`), emits Prefix declarations + Ontology wrapper + per-entity Declaration / SubClassOf / EquivalentClasses / DisjointClasses / HasKey / ObjectPropertyDomain / ObjectPropertyRange / per-characteristic flag axioms / InverseObjectProperties. Output is byte-deterministic (BTreeMap-sorted) for round-trip. Pellet / HermiT / Protégé / FaCT++ / ELK / Konclude compatible. Import (`verum import --from owl2-fs`) deferred to a follow-up commit; round-trip with FOAF/Pizza ontologies pending.

### 16.4 Phase 4 (12–18 months) — Proof layer

- **Task D1**: `@theorem` / `@lemma` / `@corollary` + `proof { … }` block parser.
- **Task D2**: tactic DSL with built-in tactic set.
- **Task D3**: hints database + proof search.
- **Task D4**: Interactive proof mode (`verum repl --proof`).
- **Task D5**: Five-format certificate export (`lean`, `coq`, `agda`, `dedukti`, `metamath`).

### 16.5 Phase 5 (18–24 months) — OC/DC dual + theory interop

- **Task E1**: `core.action.*` module skeleton.
- **Task E2**: auto-induced ε(α) and α ⊣ ε adjunction.
- **Task E3**: `@enact` annotation + `verum audit --epsilon`.
- **Task E4**: `core.theory_interop` Yoneda / Kan / descent APIs (partial already — finalise).

### 16.6 Phase 6 (24+ months) — Full MSFS self-recognition

- **Task F1**: `verum self-verify --corpus-coord` produces $(\mathrm{Fw}, \nu, \tau)$ per theorem.
- **Task F2**: Verum proves its own four correspondence theorems (Theorems 4.1–4.4 of the Verum-MSFS integration paper) inside Verum itself.
- **Task F3** (V1 ✓ shipped): Articulation Hygiene reporter at `verum audit --hygiene` — covers six §13.2 surface forms (inductive / coinductive / higher-inductive / newtype / `@recursive` / `@corecursive`); plain + JSON output. V2 enforcement (raw `self` walk + `E_HYGIENE_UNFACTORED_SELF` diagnostic) deferred.
- **Task F4**: Quantum-epistemic lattice (Noesis research surface only; not in Verum stdlib).
- **Task F5** (✓ shipped): `verum audit --owl2-classify` — graph-aware OWL 2 classification audit. Walks all `Owl2*Attr` markers; builds the classification graph (entities + subclass / equivalence / disjointness / characteristic / has-key edges); computes the *reflexive-transitive* subclass closure via iterative fixed-point; detects subclass cycles via closure intersection; equivalence partition via union-find; disjoint/subclass conflicts (a class C disjoint from D where C ⊑* D — DL-unsatisfiable); emits plain + JSON (schema_version=1) with the full graph, partitions, cycles, and violations. Exits non-zero on inconsistency. Implementation at `crates/verum_cli/src/commands/audit.rs::audit_owl2_classify_with_format`.
- **Task F6**: OWL 2 bridges to other framework packages — `owl2_fs → {baez_dolan, schreiber_dcct, connes_reconstruction, petz_classification}` via `core.theory_interop` (§21.7).
- **Task F7**: Morita-equivalence theorem `owl2_morita_bridge` — elevates §21.2 correspondence from faithful translation to Morita-reduction in both directions (§21.2, §21.10).

---

## 17. Open Design Questions

V8 (#219) status update — six of the original twelve questions
are closed (decisions ratified into the spec); six remain genuinely
open and are restated more sharply for future decision.

### 17.A Closed by V8 (decisions ratified)

1. **[CLOSED — V8] Framework-axiom lineage normalisation.** The
   canonical slug format is **snake-case lineage** (`lurie_htt`,
   `schreiber_dcct`, `connes_reconstruction`, etc.); see §6.2 for
   the live Standard catalogue. DOI-based and author+year forms
   are NOT accepted for the slug — those go in the `citation`
   field. Implementation: `core/math/frameworks/registry.vr`
   uses snake-case throughout. Any new submission to the
   Verum-Foundation-Marketplace (per VVA §18.7) must follow this
   convention.

2. **[OPEN] `Axi-8` status (M-5w*: non-Yoneda-representable α_𝖬).**
   Restated for V8: Diakrisis 02-axiomatics `Axi-8` requires that
   the metaisation articulation `α_𝖬` is **not**
   Yoneda-representable. In the canonical Cat-model `ι(𝖬)` *is*
   Yoneda-representable, so Axi-8 fails there; Diakrisis frames
   this as an open question on whether non-LP (locally-
   presentable) models satisfying Axi-8 exist. VVA defers: the
   kernel does NOT enforce Axi-8 today; if Diakrisis (or
   downstream foundational work) lands a consistent model
   satisfying it, a future VVA revision adds the enforcement as
   an opt-in `@framework(diakrisis_axi8, …)` package rather than
   a kernel rule, so users can choose the foundational stance
   per project. **Decision criterion**: model-existence proof
   landing in the Diakrisis preprint will trigger this question's
   closure; until then, the framework-axiom path is the sound
   stance.

3. **[CLOSED — V8] Quantitative / linear types default.** Default
   is **ω (unrestricted)**; opt-in via `@quantity(0)` /
   `@quantity(1)` / `@quantity(omega)` attributes (or aliases
   `linear`, `erased`, `unrestricted`). Per Task C5 V1
   (`crates/verum_ast/src/attr/typed.rs::QuantityAttr`); existing
   non-annotated functions compile unchanged. The linearity-
   tracking enforcement pass (V2) is staged separately under
   `verum_types`.

4. **[OPEN] Impredicative Prop.** Restated: ship Coq-style
   impredicative Prop, or require the explicit
   `core.math.frameworks.impredicative_prop` framework axiom?
   **Decision criterion**: settle once a soundness proof lands
   for the impredicative-Prop + HITs + classical-LEM +
   univalence combination on the cubical fragment. Current V8
   stance: framework-axiom route is preferred (avoids the
   Coquand-paradox triangle in §4.4 K-FwAx and the
   uip⊥univalence conflict in `framework_compat`).

5. **[CLOSED — V8] Certificate format schema versioning.**
   Resolved: `verum export --to <target> --target-version
   <semver>` is the canonical CLI shape, where `<target>`
   ∈ {`lean`, `coq`, `agda`, `dedukti`, `metamath`} and
   `<semver>` is the target proof-assistant version (e.g.
   `--target-version 4.4.0` for Lean 4). Schema version of the
   internal `SmtCertificate` envelope is pinned to
   `CERTIFICATE_SCHEMA_VERSION` in
   `crates/verum_kernel/src/cert.rs`; bumps go through task
   #90's cross-tool replay matrix.

6. **[OPEN] `synthesize` strategy termination.** Restated: the
   `@verify(synthesize)` strategy currently has no termination
   bound. **Decision criterion**: pick between (a)
   wall-clock-capped + hint-database + `@partial` partial-proof
   surface (proposal); (b) bounded-iterations counter only;
   (c) cooperative-cancel via `verum_smt`'s existing
   resource-bound infrastructure. Each has trade-offs around
   IDE responsiveness vs proof-search depth; benchmark
   needed before deciding.

7. **[OPEN] OC/DC duality proof.** Restated: the auto-induced
   ε(α) construction in `core.action.*` assumes Diakrisis
   Theorem 108.T. The proof of 108.T sits in the Diakrisis
   preprint (forthcoming). **Decision criterion**: production
   compile should gate `@enact` on the Diakrisis preprint
   release; until then, the kernel rejects `@enact`-using
   modules whose `@require_extension(vfe_oc_dc)` is set
   (default = unset = no `@enact`). Track preprint release
   alongside `internal/holon/internal/diakrisis/docs`.

8. **[CLOSED — V8] Kernel LOC budget.** Pinned to **6 500 LOC
   maximum** (was "5 000 target with audit"; current
   `verum_kernel` is ~4 700 LOC post-V8 module split per
   `crates/verum_kernel/src/`). The full §4.4a taxonomy
   accounting (29 rules) fits within this budget. Audit gate:
   any commit that pushes the kernel past 6 500 LOC requires
   an explicit "+kernel-loc-budget" tag in the commit message
   and a follow-up review. Module split (#198: errors.rs /
   inductive.rs / depth.rs / eps_mu.rs / universe_ascent.rs /
   support.rs / axiom.rs / infer.rs / term.rs / cert.rs /
   ctx.rs) means individual files stay auditable.

9. **[CLOSED — V8] `verum_types` vs kernel re-check split.**
   Decision: kernel **owns the final recheck** via
   `KernelRecheckPass` + `verify_full` (already shipped);
   `verum_types::refinement` is the elaboration layer that
   prepares CoreTerms for the kernel to validate. Migration
   is complete for refinement types; remaining elaboration
   surfaces (Pi/Sigma binding contexts, generic parameter
   elaboration, default values) stay in `verum_types`
   permanently — they are not soundness-critical and don't
   need the kernel to validate them. Closes
   `task:kernel-ownership-migration`.

10. **[OPEN] Cubical computational univalence.** Restated: CCHM
    provides computational univalence, but performance on large
    universes is a concern. **Decision criterion**: benchmark-
    first — measure `Glue`-evaluation cost on a representative
    HoTT corpus (e.g., `core.math.frameworks.lurie_htt` proofs)
    before deciding between (a) lazy `Glue` evaluation +
    memoisation (proposal); (b) eager evaluation with cube
    interning; (c) hybrid (lazy at top level, eager under
    refinement). The benchmark itself is pre-decision work.

11. **[OPEN] Pattern-match coverage checker for dependent types.**
    Restated: requires higher-order unification (HOU) for index-
    dependent patterns. **Decision criterion**: extend existing
    `verum_types::exhaustiveness` with dependent-index analysis
    (proposal) — but the implementation strategy depends on
    whether we use full HOU (Miller-pattern fragment is
    decidable; full HOU is undecidable) or restricted
    higher-order matching. Pick the strategy alongside the V2
    K-Elim per-case typing pass (§4.4a.4).

12. **[CLOSED — V8] Tactic DSL hygiene.** Resolved: **α-renaming
    on every splice**. Hygiene cost is acceptable in profiling
    (`verum_verification::tactic_evaluation` already implements
    α-renaming-on-splice as the default behaviour). Quote/
    unquote splices generate fresh binder names from a per-
    splice counter to guarantee no scope capture. Closes the
    open question; the ratified design ships in
    `crates/verum_verification/src/tactic_evaluation.rs`.

### 17.B Status summary

| Q# | Status | Resolution / decision criterion |
|---|---|---|
| 1 | CLOSED | Snake-case lineage |
| 2 | OPEN | Awaiting Diakrisis Axi-8 model existence proof |
| 3 | CLOSED | Default ω; opt-in via `@quantity(...)` |
| 4 | OPEN | Awaiting impredicative-Prop + univalence soundness proof |
| 5 | CLOSED | `--target-version <semver>` CLI shape |
| 6 | OPEN | Pick wall-clock vs counter vs cooperative-cancel |
| 7 | OPEN | Awaiting Diakrisis 108.T preprint |
| 8 | CLOSED | 6 500 LOC budget |
| 9 | CLOSED | Kernel owns final recheck; verum_types is elaboration |
| 10 | OPEN | Benchmark-first |
| 11 | OPEN | Pick HOU strategy alongside V2 K-Elim |
| 12 | CLOSED | α-rename on every splice |

Six closed (#1, #3, #5, #8, #9, #12), six open (#2, #4, #6, #7,
#10, #11). The open six all have explicit decision criteria
attached so future progress is observable.

---

## 18. Success Criteria

VVA is considered delivered when:

1. Every existing `.vr` file in `core/` and `vcs/specs/` compiles under `@verify(static)` with zero changes.
2. `grammar/verum.ebnf` parses all three refinement forms and the grammar tests pass.
3. `@framework(…)` is accepted and the Standard catalogue ships (per §6.2: nine Standard frameworks + VerifiedExtension families).
4. `verum audit --framework-axioms` lists used axioms for the entire stdlib.
5. `verum audit --coord` produces a `(Fw, ν, τ)` tuple per user theorem.
6. ✓ All nine `@verify(…)` strategies are dispatched through `verum_smt` or `verum_kernel`. Per V8 #233: 13-variant ladder in `crates/verum_smt/src/verify_strategy.rs::VerifyStrategy` (9 §12-base + ComplexityTyped from VVA-8 + 3 coherent variants from VVA-6); per-strategy dispatch in `crates/verum_smt/src/backend_switcher.rs` covers Runtime/Static/Proof (non-SMT, kernel-rechecked) + Fast/ComplexityTyped/Formal (capability routing) + Thorough (portfolio) + Reliable/Certified (cross-validate) + Coherent* (cross-validate w/ runtime monitor variants) + Synthesize (SyGuS path). 39+15 = 54 integration tests across `nine_strategy_ladder.rs`, `capability_routing_integration.rs`, and `strategy_dispatch_contract.rs` (V8 #233 — explicit per-strategy contract row coverage with §12-aliases + LADDER monotonicity + Synthesize-orthogonality invariants). **100%**.
7. Certificate export to Lean 4 + Dedukti round-trips at least the `plus_comm` / `append_assoc` / `list_length_map` sample proofs.
8. `core.action.*` module set exists with `ε` auto-induction per articulation.
9. `@enact` compiles and `verum audit --epsilon` is callable.
10. Kernel LOC budget: ≤ 6 500 LOC.
11. Zero runtime cost on `@verify(certified)` paths after AOT.
12. Noesis can express its 46-endpoint NP contract against Verum's public API.
13. Verum CLI surface is stable; breaking-change policy follows semver.

---

## 19. Glossary

| Term | Definition |
|---|---|
| **Articulation** | Object of `⟨⟨·⟩⟩`; represents a Rich foundation. Verum `CoreTerm::FrameworkAxiom` is a syntactic projection. |
| **Enactment** | Element of Diakrisis's Actic dual; represents a practice. Auto-induced from every articulation. |
| **Framework axiom** | Declarative statement of a foundation-specific principle, tagged with citation. |
| **M (metaisation)** | Endo-2-functor on `⟨⟨·⟩⟩` that lifts depth. Depth function `dp` tracks M-iteration count. |
| **ν-invariant** | Min ordinal λ with `α ∈ M^λ(α_0)`. Coordinate of a theorem / claim in the depth hierarchy. |
| **T-2f*** | Diakrisis axiom: comprehension admissible iff `dp(P) < dp(α_P)`. Realised as `K-Refine`. |
| **K-Refine** | VVA kernel typing rule enforcing T-2f*. Single load-bearing paradox-immunity rule. |
| **τ-invariant** | Eff-internal-normaliser flag. Distinguishes MLTT-slice (τ=1) from ETT-slice (τ=0). Verum is τ=1. |
| **CoreTerm** | Kernel calculus; LCF trusted base. See §4. |
| **Refined(A, x, P)** | Canonical CoreTerm for all three refinement forms. |
| **SmtCertificate** | CoreTerm recording an SMT-discharged proof; kernel re-checks. |
| **Quote / Unquote** | Meta-primitives for manipulating tactic values as data. |
| **`@framework`** | Attribute installing a framework axiom with lineage + citation. |
| **`@verify(strategy)`** | Selects one of nine discharge intents. |
| **`@enact(epsilon)`** | DC-side annotation; pairs a function with its Actic ε-coordinate. |
| **`@theorem / @lemma / @corollary`** | Proof-layer function attributes; body is a `proof { … }` block. |
| **`@derive`** | Method synthesis from protocol. |
| **OC / DC** | Object-centric (articulation) / Dependency-centric (enactment). Dual stdlib layers. |
| **MSFS coordinate** | `(Framework, ν, τ)` triple per theorem. |
| **Morita reducibility** | `(∞, n)`-equivalence in `StrCat_{S,n}`. Used for translating between articulations. |
| **Kan extension** | Universal adjoint extending a functor along another; basis of `translate()`. |
| **Čech descent** | Cosimplicial coherence on covers; basis of `check_coherence()`. |
| **Yoneda embedding** | Fully faithful `y: C → PSh(C)`; basis of `load_theory()`. |
| **Articulation Hygiene** | Compile-time factorisation discipline for self-referential constructs. |
| **Actic** | Diakrisis's act-centric dual primitive (Φ, A, ε_math, ⊐_•). |
| **ε(α)** | Auto-induced enactment of an articulation α via 108.T. |
| **LCF pattern** | Tactics / solvers produce certificates; kernel rechecks. Robin Milner, Edinburgh 1972. |
| **Yanofsky 2003** | Universal reduction of self-referential paradoxes to weakly point-surjective α: Y → T^Y. |
| **OWL 2 DS** | OWL 2 Direct Semantics — W3C Recommendation 2012, model-theoretic semantics of OWL 2 DL over SROIQ description logic. Verum baseline. |
| **OWL 2 RBS** | OWL 2 RDF-Based Semantics — W3C Recommendation 2012, graph-based semantics over RDF triples. Undecidable. Not in VVA scope. |
| **Shkotin 2019 (DS2HOL)** | Denotational HOL formalisation of OWL 2 DS. `internal/OWL2.DS2HOL.pdf`. Verum's formal bridge for `core.math.frameworks.owl2_fs`. |
| **`core.math.frameworks.owl2_fs`** | ~63 `@framework(owl2_fs, …)` axioms encoding Shkotin 2019 Tables 1–10. Trusted OWL 2 semantic boundary. |
| **`OwlAttr` family** | Typed attributes in `verum_ast::attr::typed` (`@owl2_class`, `@owl2_property`, etc.) preserving OWL 2 vocabulary for round-trip. |
| **`count_o`** | Verum realisation of Shkotin's `#` quantifier (`#y:o P(y)`). SMT-dispatched via CVC5 Finite Model Finding; errors out on unbounded domains. |
| **NAMED restriction** | OWL 2 DS Key axiom restricts to individuals declared as OWL 2 `NamedIndividual`. Verum `@verify(proof)` with user tactic (§21.3). |
| **Faithful translation** | VVA §21.2 baseline claim: every OWL 2 DS-valid derivation is Verum-valid (one direction). Automatic from Shkotin-literal encoding. |
| **Morita-equivalence bridge** | VVA §21.2 roadmap claim (Phase 6 F7): Verum OWL 2 encoding is Morita-reducible both directions to W3C DS. |

---

## 20. References

Internal (this repository):

- `grammar/verum.ebnf` — authoritative grammar.
- `internal/docs/detailed/03-type-system.md` — base type system, three refinement forms (descriptive).
- `internal/docs/detailed/09-verification-system.md` — verification system (descriptive).
- `internal/docs/detailed/12-dependent-types.md` — dependent types (planned, now superseded in part by VVA §7).
- `internal/docs/detailed/13-formal-proofs.md` — formal proofs (planned, now superseded in part by VVA §8).
- `internal/docs/detailed/17-meta-system.md` — meta-system (planned, now superseded in part by VVA §9).
- `internal/holon/internal/math-msfs/paper-en/paper.tex` — MSFS preprint.
- `internal/holon/internal/math-msfs/verum/en/paper.tex` — Verum-MSFS integration preprint.
- `internal/holon/internal/diakrisis/docs/02-canonical-primitive/02-axiomatics.md` — Diakrisis axioms.
- `internal/holon/internal/diakrisis/docs/06-limits/10-maximality-theorems.md` — 103.T–106.T maximality proofs.
- `internal/holon/internal/diakrisis/docs/09-applications/01-verum-integration.md` — Verum integration plan.
- `internal/holon/internal/diakrisis/docs/11-noesis/16-verum-implementation.md` — Noesis implementation.
- `internal/holon/internal/diakrisis/docs/12-actic/*` — Actic dual.

External:

- Cohen T., Coquand T., Huber S., Mörtberg A. 2018. *Cubical Type Theory: a constructive interpretation of the univalence axiom.*
- Huber S. 2019. *Canonicity for Cubical Type Theory.*
- Hyland M. 1982. *The Effective Topos.*
- Lurie J. 2009. *Higher Topos Theory.* Annals of Mathematics Studies 170.
- Yanofsky N.S. 2003. *A Universal Approach to Self-Referential Paradoxes, Incompleteness and Fixed Points.*
- Hofmann M. 1995. *Extensional Constructs in Intensional Type Theory* (thesis).
- HoTT Book. 2013. *Homotopy Type Theory: Univalent Foundations of Mathematics.*
- Schreiber U. 2013. *Differential Cohomology in a Cohesive ∞-Topos.*
- Connes A. 1994. *Noncommutative Geometry.*
- Milner R. 1972. *Logic for Computable Functions: description of a machine implementation.* Edinburgh.
- Riehl E., Verity D. 2022. *Elements of ∞-Category Theory.*
- de Moura L., Ullrich S. 2021. *The Lean 4 Theorem Prover and Programming Language.*
- Norell U. 2007. *Towards a Practical Programming Language based on Dependent Type Theory* (thesis).
- Shkotin A. 2019. *OWL 2 Functional Style operators from HOL point of view.* Technical Report DS2HOL-1. `internal/OWL2.DS2HOL.pdf`.
- W3C. 2012. *OWL 2 Web Ontology Language. Direct Semantics (Second Edition).* W3C Recommendation 11 December 2012. <https://www.w3.org/TR/owl2-semantics/>.
- Motik B., Patel-Schneider P.F., Parsia B. (eds.) 2012. *OWL 2 Web Ontology Language. Structural Specification and Functional-Style Syntax.* W3C Recommendation.
- Hitzler P., Krötzsch M., Parsia B., Patel-Schneider P.F., Rudolph S. 2012. *OWL 2 Primer (Second Edition).* W3C Recommendation.

---

## 21. OWL 2 Integration

*Ontology support is not a bolt-on. It is the first major external-framework package that exercises every pillar of VVA — `@framework` axioms, three refinement forms, nine-strategy ladder, `core.theory_interop`, certificate export — and it is the architectural bridge to the world's largest existing knowledge corpora (SNOMED-CT, Gene Ontology, DBpedia, SUMO, Cyc, DOLCE, BFO, FIBO). The formal target is a denotational HOL bridge due to Shkotin 2019 (`internal/OWL2.DS2HOL.pdf`), itself a Morita-preserving translation of the W3C OWL 2 Direct Semantics Recommendation (2012).*

### 21.1 Scope — Direct Semantics only

Verum OWL 2 support targets **OWL 2 Direct Semantics (DS)** — the model-theoretic semantics recommended by W3C and used by every mainstream reasoner (HermiT, Pellet, FaCT++, ELK, Konclude). The RDF-Based Semantics (RBS) variant is **out of scope**.

Rationale:

- DS gives OWL 2 DL, a decidable fragment of SROIQ with known complexity profiles (EL / QL / RL polynomial; DL 2NEXPTIME). This matches VVA §12's nine-strategy ladder which expects predictable complexity per strategy.
- RBS is undecidable and graph-based; its RDF-triple primitive does not map cleanly onto Verum's typed refinement structure (VVA §5) without a separate graph-modelling layer that would duplicate `core.collections.Map<Text, Set<Text>>`.
- Shkotin 2019 — the formal bridge Verum imports — formalises DS only. RBS has no equivalent in-kernel formalisation.
- Noesis's knowledge-object model (VVA §14.2, `diakrisis/docs/11-noesis/03-knowledge-model.md`) is object-based, not RDF-triple-based, aligning directly with DS.

If a consumer needs RBS compatibility (e.g. SPARQL interop), a dedicated `core.math.frameworks.owl2_rbs` package is a separate, future Phase-6 work item.

### 21.2 Correspondence claim — faithful translation, Morita as roadmap

Verum makes **two layered claims** about the `core.math.frameworks.owl2_fs` package:

**(1) Shipped baseline — faithful translation.** Every valid derivation in W3C DS is a valid derivation in Verum, via the following chain:

```
W3C OWL 2 DS  ──(Shkotin 2019 Table 1–10)──►  HOL definitions  ──(VVA §21.5)──►  Verum @framework axioms
```

Concretely: every OWL 2 satisfiability / entailment judgement `O ⊨ α` (ontology O entails axiom α under DS) corresponds to a Verum derivation of the encoded form `owl2_sem(O) ⊢ owl2_sem(α)`. The encoding is line-by-line from Shkotin's "HOL-definition body" column; faithful translation follows by construction.

**(2) Roadmap goal — Morita-equivalence.** Phase 6 task F7 proves that the encoding is NOT only faithful but a Morita-reduction in both directions: every Verum derivation of an encoded ontology sentence corresponds to a W3C DS derivation. This elevates the claim from "Verum preserves OWL 2" to "Verum OWL 2 is OWL 2" up to categorical equivalence.

Verum does NOT make correspondence claim (c) ("implementation only, no formal guarantee") — that would be beneath VVA's rigor standard (cf. §15.5 kernel re-check invariant).

### 21.3 Three-layer architecture

OWL 2 integration decomposes into three clean layers, each mapping onto an existing VVA architectural slot (§3). This is not new architecture — it is OWL 2 lifted into VVA's existing surfaces.

**Layer 1 — Semantic framework package** (`core.math.frameworks.owl2_fs`, ~60 axioms, VVA Layer 1):

Line-by-line encoding of Shkotin 2019 Tables 1–10. Each OWL 2 operator becomes a `@framework(owl2_fs, "Shkotin 2019. DS2HOL-1 §X.Y Table Z.")` axiom with its HOL-definition body copied verbatim. This is the trusted boundary — `verum audit --framework-axioms` enumerates exactly which operators a corpus depends on.

**Layer 2 — Vocabulary-preserving typed attributes** (`verum_ast::attr::typed::OwlAttr`, VVA Layer 6):

A family of typed attributes preserving OWL 2 vocabulary at the source so `verum export --to owl2-fs` round-trips cleanly to Protégé / HermiT / Pellet:

```verum
@owl2_class[(semantics = OpenWorld | ClosedWorld)?]
@owl2_property(
    domain = <type>,
    range = <type>,
    characteristic = [Transitive?, Symmetric?, Asymmetric?, Reflexive?,
                      Irreflexive?, Functional?, InverseFunctional?],
    inverse_of = <property_name>?,
)
@owl2_subclass_of(<class>)
@owl2_equivalent_class(<expression>)
@owl2_disjoint_with([<class>, …])
@owl2_has_key(<property>, …)
@owl2_characteristic(<characteristic_name>)
```

Each attribute is pure metadata — it adds no proof content, only round-trip information. Attributes compile down to framework-axiom invocations at Layer 1.

**Layer 3 — Verification obligations** (`@theorem` + `@verify(strategy)`, VVA Layer 4):

Subsumption, classification, consistency, instance checking — all become `@theorem` declarations with `ensures` clauses, discharged via the nine-strategy ladder (§12). Mapping:

| OWL 2 task | VVA strategy | ν-ordinal | Rationale |
|---|---|---|---|
| Consistency of an ontology | `@verify(formal)` | ω | SMT satisfiability on the joint refinement |
| Classification (EL/QL/RL) | `@verify(fast)` | <ω | Polynomial-time profile; bounded SMT timeout |
| Classification (full DL) | `@verify(thorough)` | ω·2 | 2NEXPTIME; portfolio dispatch |
| Subsumption `C ⊑ D` | `@verify(formal)` | ω | Closed-goal SMT obligation |
| Instance check `a : C` | `@verify(fast)` at runtime, `@verify(formal)` at compile-time | variable | Ordinary refinement check |
| HasKey with NAMED restriction | `@verify(proof)` | ω | DL-reasoner case per Shkotin §2.3.5 |
| Ontology alignment | `@verify(reliable)` | ω·2 | Z3 ∧ CVC5 agreement required |
| Federation coherence (Noesis) | `@verify(certified)` | ω·2 | Certificate materialisation + export |

This table commits dispatch semantics at the spec level so implementation has a fixed target.

### 21.4 Semantic defaults — CWA with OWA opt-in

OWL 2 DS uses **Open World Assumption (OWA)**: absence of an assertion does not imply negation. Verum's typed refinement system uses **Closed World Assumption (CWA)**: a predicate either holds or fails.

Direct accommodation of both simultaneously would require every OWL 2 query to return `Maybe<Bool>` with `Unknown`, breaking composition with the rest of Verum's type system. VVA chooses a pragmatic resolution:

**Default: CWA.** `@owl2_class` / `@owl2_property` without explicit semantics qualifier compile into standard Verum refinements; queries return `Bool`. This preserves ergonomics and makes ~95% of practical ontologies (medical classification, business rules, type hierarchies) work with zero ceremony.

**Opt-in: OWA.** `@owl2_class(semantics = OpenWorld)` flips to OWA semantics locally; queries against that class return `Maybe<Bool>` with `Unknown` when the class membership is neither provable nor refutable. This preserves fidelity for ontologies that genuinely rely on OWA (e.g. biomedical uncertainty reasoning).

The compiler must reject mixed-semantics operations that would silently collapse OWA to CWA (e.g. `and`-composing an OWA class query with a CWA refinement): the `core.logic.kleene` module in `core.math.logic` provides the three-valued connectives; explicit use is required.

### 21.5 Operator catalogue — line-by-line from Shkotin 2019

The `core.math.frameworks.owl2_fs` package encodes exactly the tables of Shkotin 2019:

| Table | Section | Operator count | Verum module path |
|---|---|---|---|
| 1 | §2.2.1 Object Property Expressions | 1 (`ObjectInverseOf`) | `core.math.frameworks.owl2_fs.object_property` |
| 3 | §2.2.2 Data Ranges | 5 | `core.math.frameworks.owl2_fs.data_range` |
| 4 | §2.2.3 Class Expressions | 24 | `core.math.frameworks.owl2_fs.class_expr` |
| 5 | §2.3.1 Class Expression Axioms | 4 | `core.math.frameworks.owl2_fs.class_axiom` |
| 6 | §2.3.2 Object Property Expression Axioms | 14 | `core.math.frameworks.owl2_fs.object_property_axiom` |
| 7 | §2.3.3 Data Property Expression Axioms | 6 | `core.math.frameworks.owl2_fs.data_property_axiom` |
| 8 | §2.3.4 Datatype Definitions | 1 | `core.math.frameworks.owl2_fs.datatype_definition` |
| 9 | §2.3.5 Keys (NAMED restriction) | 1 (`HasKey`) | `core.math.frameworks.owl2_fs.key` |
| 10 | §2.3.6 Assertions | 7 | `core.math.frameworks.owl2_fs.assertion` |

Total: **~63 axioms**. Every axiom carries a `@framework(owl2_fs, "Shkotin 2019. DS2HOL-1 §X.Y Table Z.")` tag pointing to the exact table row in the source paper. `verum audit --framework-axioms --by-lineage owl2_fs` enumerates the full OWL 2 operator set used by any corpus.

Primitive support required (per Shkotin's "Summary"):
- `o` sort → Verum type alias `core.math.frameworks.owl2_fs.Individual`
- `d` sort → Verum type alias `core.math.frameworks.owl2_fs.Literal` (reuses `core.base.Literal`)
- polymorphic equality → `==` operator (shipped)
- natural numbers with predicates → `Int` + `core.base.Int` operators (shipped)
- **quantifier of quantity** `#` → new primitive `core.math.frameworks.owl2_fs.count_o(pred: fn(Individual) -> Bool) -> Int`, with explicit diagnostic `E_OWL2_UNBOUNDED_COUNT` when the domain is not finite and the SMT backend cannot decide the count.

The `count_o` primitive is encoded as:

```verum
@framework(owl2_fs, "Shkotin 2019. DS2HOL-1 §Notation (quantifier of quantity #).")
public axiom count_o_spec(pred: fn(Individual) -> Bool) -> Int
    ensures count_o(pred) >= 0,
    ensures forall n: Int.
        count_o(pred) == n <=>
        (exists set: List<Individual>.
            set.len() == n &&
            set.all(|x| pred(*x)) &&
            forall y: Individual. pred(y) => set.contains(y));
```

`verum_smt` dispatches `count_o` via CVC5 Finite Model Finding when the domain is known-finite; otherwise returns `E_OWL2_UNBOUNDED_COUNT` with a diagnostic pointing the user at `@owl2_class(closed_domain = true)` opt-in.

### 21.6 Typed attribute surface

Formal grammar for the `OwlAttr` family (extends `verum_ast::attr::typed`):

```ebnf
owl_attr
    = "@owl2_class" , [ "(" , owl2_class_args , ")" ]
    | "@owl2_property" , "(" , owl2_property_args , ")"
    | "@owl2_subclass_of" , "(" , type_expr , ")"
    | "@owl2_equivalent_class" , "(" , expression , ")"
    | "@owl2_disjoint_with" , "(" , "[" , type_expr , { "," , type_expr } , "]" , ")"
    | "@owl2_has_key" , "(" , identifier , { "," , identifier } , ")"
    | "@owl2_characteristic" , "(" , owl2_characteristic , ")" ;

owl2_class_args
    = "semantics" , "=" , ( "OpenWorld" | "ClosedWorld" )
    | "closed_domain" , "=" , boolean ;

owl2_property_args
    = "domain" , "=" , type_expr
    , "," , "range" , "=" , type_expr
    , [ "," , "characteristic" , "=" , "[" , owl2_characteristic , { "," , owl2_characteristic } , "]" ]
    , [ "," , "inverse_of" , "=" , identifier ] ;

owl2_characteristic
    = "Transitive" | "Symmetric" | "Asymmetric" | "Reflexive"
    | "Irreflexive" | "Functional" | "InverseFunctional" ;
```

Elaboration: each attribute application at parse time installs:
1. A `CoreTerm::FrameworkAxiom { name: "owl2_fs", citation: …, body }` node using the corresponding Shkotin axiom.
2. A `@verify(…)` obligation selected per §21.3 Table (e.g. `@owl2_subclass_of(Animal)` auto-generates the `@verify(formal)` subsumption check).
3. A round-trip marker consumed by `verum export --to owl2-fs`.

### 21.7 Cross-framework composition

**Phase 3 baseline (shipped with C7):** `owl2_fs` is an **isolated** framework package. Users can `@framework(owl2_fs, …)` and `@framework(lurie_htt, …)` in the same module; semantics do not interact. Translation between them requires explicit `core.theory_interop.translate` invocation.

**Phase 3b extension (shipped with C7b):** A canonical bridge `owl2_fs → lurie_htt` ships as ~30 additional `@framework_translate(owl2_fs → lurie_htt, "derived by …")` axioms:

- OWL 2 Class → presheaf on the discrete category of individuals (HTT 1.2.1).
- OWL 2 ObjectProperty → functor between class-presheaves.
- SubClassOf → monomorphism in the presheaf topos.
- EquivalentClasses → isomorphism.
- ObjectPropertyChain → functor composition.
- HasKey → representable-presheaf condition (HTT 5.1).

The bridge enables queries like "express this medical subsumption in cohesive ∞-topos terms" via a single `translate(Person_ontology, schreiber_dcct_target)` call, composed via the lurie_htt intermediate.

**Phase 6 expansion (F6):** Bridges to other framework packages (`baez_dolan`, `schreiber_dcct`, `connes_reconstruction`, `petz_classification`) as needed. Each bridge is optional and can be loaded independently.

### 21.8 Import / export

**Export:** `verum export --to owl2-fs ./src/ontology.vr -o ontology.ofn` walks the `@owl2_*` attribute family in the AST and emits OWL 2 Functional Syntax. Round-trips through Protégé, HermiT, Pellet. Byte-deterministic output (BTreeMap-ordered) for CI diff review.

**Import:** `verum import --from owl2-fs ./ontology.ofn -o src/ontology.vr` parses OWL 2 Functional Syntax and emits `.vr` source with `@owl2_*` attributes. Verified round-trip (export → import → export byte-identical) is a C8 shipped guarantee.

**Lineage map** (extends VVA §8.5 exporter): `owl2_fs` lineage maps to `owl2-fs` target for export, preserving IRIs and anonymous-individual names (`genid*`) per Shkotin 2019 §»HOL: OWL2 ontology as a whole» convention.

### 21.9 Integration with Noesis

The Noesis platform (VVA §14, `diakrisis/docs/11-noesis/`) consumes OWL 2 directly:

- Noesis NP-protocol endpoint `knowledge/import` accepts OWL 2 FS files via Verum's import CLI.
- Noesis knowledge-object `K` becomes a `core.math.frameworks.owl2_fs.Ontology` bundle.
- Noesis dependency types (`requires`, `entails`, `generalizes`, `instantiates`, `contradicts`, …) compile onto OWL 2 Class / Property axioms.
- Noesis `coherence/check` reuses `core.theory_interop.check_coherence` over the OWL 2 axiom graph (Čech descent on property intersections).
- Noesis `agent/propose` — LLM-generated claims filtered through `@verify(formal)` before surfacing.

This is the operational realisation of the Noesis-boundary contract in VVA §14.3 for the OWL 2 stratum.

### 21.10 Roadmap additions

Extending VVA §16 with OWL 2 work items:

| Task | Phase | Depends on |
|---|---|---|
| **C7** — `core.math.frameworks.owl2_fs` package (~63 axioms) | Phase 3 | A3 shipped, C6 shipped |
| **C7b** — `owl2_fs → lurie_htt` bridge (~30 translate axioms) | Phase 3 | C7 |
| **C8** — `OwlAttr` family in `verum_ast::attr::typed` | Phase 3 | C7 |
| **C9** — `count_o` primitive + `E_OWL2_UNBOUNDED_COUNT` diagnostic | Phase 3 | C7 |
| **B5** — `verum export --to owl2-fs` / `verum import --from owl2-fs` | Phase 3 | C8 |
| **F5** — `verum audit --owl2-classify` classification hierarchy | Phase 6 | C7, C8 |
| **F6** — bridges `owl2_fs → {baez_dolan, schreiber_dcct, …}` | Phase 6 | F5 |
| **F7** — Morita-equivalence theorem `owl2_morita_bridge` | Phase 6 | F5, F6 |

### 21.11 Open questions — RESOLVED 2026-04-25 (Shkotin confirmation)

Three questions previously flagged were sent to paper author A.B. Shkotin (`ashkotin@acm.org`) and answered. Resolutions below are now binding for C7 V2.

1. **HOL variant — RESOLVED.** Shkotin: «да. и по идее HOL любой где можно выписать эти определения». Default = classical HOL (Church 1940 / Andrews 2002), `@framework(classical_lem, classical_choice)`. The DS axiom set is also expressible verbatim in HOL with extensional equality (Isabelle/HOL, HOL4), HOL Light (Harrison 2009), and Cubical HOL (preserves all classical theorems, adds path equality — useful for HoTT-based ontology alignment in Noesis). NOT compatible without losing axioms: constructive HOL (loses `DisjointClasses`/`ComplementOf` and other LEM-dependent axioms), predicative HOL (DS is impredicative through OWL 2 punning), modal HOL (epistemic/temporal extensions only, not plain DS), linear/affine HOL (incompatible with axiom re-use over graphs).

2. **Quantifier `#` on potentially infinite domains — RESOLVED.** Shkotin: «да. по идее — надо требовать конечной мощности фактического параметра». The constraint is on the **actual parameter**, not just an attribute on the consuming class. Implementation in `core/math/frameworks/owl2_fs/count.vr` already enforces this: `count_o<I>(domain: List<I>, pred)` takes a `List<I>` as the witness — finiteness is guaranteed by type construction. The `count_o_unbounded` variant takes `Maybe<List<I>>` and returns `Maybe::None` with `E_OWL2_UNBOUNDED_COUNT` when the witness is absent. V2 wires CVC5 Finite Model Finding to populate the witness automatically when domain finiteness is provable.

3. **Anonymous-individual negation scope — RESOLVED with correction.** Shkotin: «да, формализация именно эта. а причём здесь OWA? в §5.6 нет. и OWA тоже». The formalization `(a^I, b^I) ∉ ⟦OPE⟧^I` is correct; the OWA framing was a misnomer. W3C OWL 2 DS §5.6 defines NegativeObjectPropertyAssertion via classical negation only — no OWA carve-out. Anonymous individuals get unique denotations through the standard interpretation function (§5.2); the negation is bog-standard classical regardless of named/anonymous distinction. The OWA reference conflated (a) the syntactic Skolem-style scoping of anonymous individuals at RDF parse time with (b) the meta-logical OWA of OWL 2 entailment overall — neither belongs in §5.6 itself. C7 V2 body must implement classical `¬⟦OPE⟧^I(a^I, b^I)` without any OWA condition; `@framework(classical_lem)` carries the LEM dependency.

All three resolutions are reflected in `core/math/frameworks/owl2_fs/` source-comment headers; C7 V2 (#134) translates them into verbatim HOL bodies.

### 21.12 Success criteria (OWL 2-specific extensions to VVA §18)

1. Round-trip: Pellet-compatible `foaf.owl` → `verum import` → `verum export` → byte-identical output.
2. SNOMED-CT medical corpus classification produces the same class hierarchy as HermiT.
3. `verum audit --framework-axioms` enumerates every used OWL 2 operator with Shkotin 2019 table/row citation.
4. Verum corpus mixing `@framework(lurie_htt, …)` theorems with `@framework(owl2_fs, …)` ontologies compiles and verifies cleanly (cross-framework non-interference).
5. `core.theory_interop.translate(owl2_ontology, lurie_htt_target)` produces a well-typed result for the standard OWL 2 test suite (W3C Test Cases Part 2).

---

## Appendix A — Relationship to Legacy Specs

| Legacy | Status after VVA |
|---|---|
| `03-type-system.md` § 1.5 (three refinement forms) | Normative — quoted and formalised in §5. |
| `09-verification-system.md` (modes, contract literals) | Partially superseded — modes extended to nine-strategy ladder (§12). `contract#"…"` DSL retained as surface. |
| `12-dependent-types.md` (planned v2.0+) | Partially superseded — §7 is authoritative. |
| `13-formal-proofs.md` (planned v2.0+) | Partially superseded — §8 is authoritative. |
| `17-meta-system.md` (meta fn, Quote/Unquote) | Refined — §9 is authoritative. |
| `26-unified-execution-architecture.md` | Orthogonal — VVA operates at Layer 3–6; execution at Layer 2. |

Legacy specs remain as descriptive context; VVA is the single source of truth for verification/proofs/dependent-types/meta-system from this point forward.

---

## Appendix B — Design Principle Recap

1. **One rule from Diakrisis**: T-2f* / K-Refine. Everything else is bookkeeping.
2. **Three forms, one CoreTerm**: Inline, declarative, sigma all canonicalise to `Refined`.
3. **Nine strategies, monotone ladder**: `runtime ≤ static ≤ fast ≤ formal ≤ proof ≤ thorough ≤ reliable ≤ certified`; `synthesize` orthogonal.
4. **Trust only the kernel**: all tactics / solvers / elaborators emit CoreTerms.
5. **Framework axioms are data**: `@framework(name, citation)` is first-class; no hidden axioms.
6. **Dual stdlib** — OC (`core.math.*`) + DC (`core.action.*`); auto-induced duality.
7. **Five-format certificate export, day one**: Lean / Coq / Agda / Dedukti / Metamath.
8. **Foundation-neutral**: the kernel does not assume any R-S.
9. **Hygiene is compile-time**: every "self-X" factors as `(Φ, Φ^κ, t)`.
10. **Kernel ≤ 6 500 LOC**: LCF trust boundary stays small.

— end of specification —

---

# Part A.Z — Foundational Synthesis & Architectural Audit (V8 #226)

> **Purpose.** This chapter integrates the Diakrisis canonical-
> primitive axiomatic stack (`internal/holon/internal/diakrisis/docs/`)
> and the Verum-MSFS preprint
> (`internal/holon/internal/math-msfs/paper-en/paper.tex`) directly
> into the architectural specification. Every kernel rule shipped
> in §4.4a is cross-referenced to its source-material justification;
> every gap between source material and shipped code is flagged
> as a defect with a concrete closure path.
>
> **Why this chapter exists.** Per the user vision (V8 #226) Verum
> must embody **machine verification in the broadest sense** —
> high-performance, fault-tolerant, reliable systems built on
> the most rigorous foundational mathematics available. That is
> only achievable when the spec, the source material, and the
> implementation form a single coherent triangle. This chapter
> is the audit that surfaces every break in that triangle.

## A.Z.1 The 13-axiomatic source of truth

**Source**: Diakrisis `02-canonical-primitive/02-axiomatics.md`
defines the 13 formal conditions that anchor the entire
canonical-primitive theory:

| Axiom | Statement (Russian original; informal English gloss) | Kernel rule(s) | Status |
|---|---|---|---|
| **Axi-0** | $\mathrm{Ob}(\langle\!\langle \cdot \rangle\!\rangle) \neq \emptyset$ — at least one articulation exists | (none — non-emptiness is a model-side existence claim, not a kernel typing rule) | **Implicit** |
| **Axi-1** | $\langle\!\langle \cdot \rangle\!\rangle$ is a locally-small 2-category with internal closure $\iota: \mathrm{End}(\langle\!\langle \cdot \rangle\!\rangle) \hookrightarrow \langle\!\langle \cdot \rangle\!\rangle$ | (none directly; the 2-category structure is *meta*-level; CoreTerm encodes *terms*, not 2-cells) | **Defect — V3+ work** |
| **Axi-2** | $\mathsf{M}: \langle\!\langle \cdot \rangle\!\rangle \to \langle\!\langle \cdot \rangle\!\rangle$ is a 2-functor | `K-EpsilonOf`, `K-AlphaOf`, `K-Eps-Mu` (V0/V1/V2) | **Partial** — V3 τ-witness pending Diakrisis preprint |
| **Axi-3** | $\alpha_{\mathrm{math}} \in \mathrm{Ob}(\langle\!\langle \cdot \rangle\!\rangle)$ — distinguished `α_math` exists | `core.math.frameworks.actic.raw` (registry entry; no specific kernel rule) | **Implicit (registry-side)** |
| **Axi-4** | $\rho(\alpha) := \mathrm{ev}_{\alpha_{\mathrm{math}}}(\alpha) = [\alpha_{\mathrm{math}}, \alpha] \in \mathrm{End}(\langle\!\langle \cdot \rangle\!\rangle)$ — ρ via internal hom; M is λ-accessible | (none — accessibility is a meta-categorical property; not yet wired) | **Defect — V3+ work** |
| **Axi-5** | $\exists \alpha, \beta: \rho(\alpha) \not\simeq \rho(\beta)$ — ρ-non-triviality | (none directly — non-triviality is a model-side claim) | **Implicit** |
| **Axi-6** | $\rho \circ \mathsf{M} \not\simeq \rho$ in general — ρ and M not commutative | (component of `K-Eps-Mu` V2 depth-mismatch rejection) | **Partial** — captured via depth-mismatch rejection |
| **Axi-7** (M-5w) | $\exists \alpha_{\mathsf{M}}: \forall \beta. \rho(\alpha_{\mathsf{M}})[\rho(\beta)] \simeq \rho(\mathsf{M}(\beta))$ — self-articulability | `K-Eps-Mu` identity-functor case (V1) | **Partial** — V3 needs full naturality |
| **Axi-8** (M-5w*) | $\neg \exists \alpha: \rho(\alpha_{\mathsf{M}})(-) \simeq \mathrm{Hom}(-, \alpha)$ — α_M not Yoneda-representable | (Theorem 131.T — `K-Universe-Ascent` enables ascent into κ_2-tier) | **Wired in stack model** (V1) |
| **Axi-9** | (full statement: see source §Axi-9) | (none yet) | **Defect — V3+ work** |
| **T-α** | (5-axis absoluteness — MSFS §6) | (none yet) | **Defect — V3+ work** |
| **T-2f*** | $\mathrm{dp}(P) < \mathrm{dp}(A) + 1$ — Yanofsky paradox-immunity | `K-Refine` | **Shipped V0** |
| **T-2f\*\*** | (modal stratification 130.T — strengthens T-2f*) | `K-Refine-omega` (VVA-7) | **Shipped V0/V1** |

### A.Z.1.1 Defect inventory

The "Defect" rows above identify gaps between the axiomatic
source of truth and the kernel as shipped. In priority order:

1. **Axi-1 — internal closure ι** is unrepresented in CoreTerm.
   The kernel models terms, not 2-cells; the 2-category
   structure is meta-level. Closing this requires either (a) a
   `TwoCell` constructor in CoreTerm (large-scope rewrite of
   the typing rules) or (b) explicit acknowledgment that the
   2-category structure lives in the model layer
   (`core.math.frameworks.diakrisis_*`) and is referenced via
   framework axioms rather than internalised. **Decision (V8
   #226 stage 3.2):** option **(b)** is now the official
   architectural stance. Internal closure ι is delegated to
   the model layer per `core.math.frameworks.diakrisis*`
   framework axioms (specifically the diakrisis_acts +
   diakrisis_biadjunction corpora carry the 2-functorial
   structure in their axiom bodies). The kernel does NOT
   internalise 2-cells; this is a deliberate TCB-budget
   choice (6 500 LOC ceiling per VVA §17 Q8). Framework
   authors registering `@framework(diakrisis_acts, "...")`
   axioms are responsible for the 2-categorical content;
   `verum audit --framework-axioms` enumerates the boundary.

2. **Axi-4 accessibility** — λ-accessibility of M is required
   for the existence of transfinite iterations
   $\mathsf{M}^\kappa$ (Theorem 10.T5 — `Fix(M) ≠ ∅`). **V8
   #228 shipped: `@accessibility(λ)` typed attribute** in
   `verum_ast::attr::AccessibilityAttr` accepts ordinal
   tokens (`omega`, `omega_<n>`, `omega+<n>`, finite cardinals).
   Framework authors record the certified accessibility bound
   via `@accessibility(omega)` on `EpsilonOf` markers; the
   audit pass surfaces unannotated EpsilonOf sites for review.
   Status: ✓ shipped (16 round-trip tests).

3. **Axi-9 + T-α** — neither has a corresponding kernel rule.
   These describe conditions on the canonical primitive that
   are sound to delegate to framework axioms (since they
   constrain the *model*, not the term-typing rules). **V8
   #226 stage 3.2 ratifies the delegation**: Axi-9 + T-α are
   model-side claims. `core.math.frameworks.diakrisis*`
   framework axioms carry their content. The kernel's TCB
   does NOT include them directly; `verum audit
   --framework-axioms` enumerates the trust boundary so
   external review can verify the citations resolve.

4. **VVA-1 V3** — full τ-witness construction for `K-Eps-Mu`
   is multi-week work blocked on Diakrisis preprint material
   (§17 Open Question 7 OC/DC duality is the same dependency
   class). Tracked as task #181.

## A.Z.2 The MSFS stratified hierarchy

**Source**: `internal/holon/internal/math-msfs/paper-en/paper.tex`
§2 ("Stratified Hierarchy of the Moduli Space"). The paper
partitions the moduli space $\mathfrak{M}$ into three formal
strata with categorically distinct meta-operations:

| Stratum | Members | Meta-operation | Verum kernel mapping |
|---|---|---|---|
| $\mathcal{L}_{\mathrm{Fnd}}$ | Foundations: ZFC, ZFC + LC, ETCS, MLTT, CIC, HoTT, NCG, ... | none above (no horizontal classification at this layer) | `core.math.frameworks.*` Standard catalogue (per §6.2) |
| $\mathcal{L}_{\mathrm{Cls}}$ | Classifiers: meta-frameworks classifying $\mathcal{L}_{\mathrm{Fnd}}$ | $\mathrm{Cls}$ — horizontal stabilisation: $\mathrm{Cls}(\mathcal{L}_{\mathrm{Cls}}) \simeq_2 \mathcal{L}_{\mathrm{Cls}}$ | `core.math.frameworks.diakrisis` (meta-classifier) |
| $\mathcal{L}_{\mathrm{Abs}}$ | (Empty by AFN-T — the maximally generative stratum forbidden by the no-go theorem) | $\mathrm{Gen}$ — vertical step | (Empty; Verum delegates to AFN-T proof) |

The MSFS paper proves:

  * **Theorem 1 (AFN-T)** — $\mathcal{L}_{\mathrm{Abs}} = \emptyset$. No
    maximally-generative foundation exists.
  * **Theorem (Meta-Stab)** — $\mathrm{Cls}(\mathcal{L}_{\mathrm{Cls}}) \simeq_2 \mathcal{L}_{\mathrm{Cls}}$
    (horizontal stabilisation at the theory level).
  * **Theorem (Meta-Cat)** — categoricity of
    $\mathrm{Meta}_{\mathrm{Cls}}^{\top}$ up to $(\infty,\infty)$-equivalence
    when non-empty.
  * **AC/OC Morita Duality + Dual Boundary Lemma** — the
    Diakrisis-side dual to AFN-T (Theorem 109.T — Dual-AFN-T).

### A.Z.2.1 Coordinate system: $(\mathrm{Fw}, \nu, \tau)$

Per MSFS §3 "Indexing Scheme" + Diakrisis 09-applications/02-canonical-nu-table.md,
every theorem is locatable by a triple:

  * $\mathrm{Fw}$ — the framework slug (snake-case lineage
    name; per §6.2 Standard catalogue).
  * $\nu$ — the canonical depth coordinate (ordinal — finite
    for set-level foundations, $\omega$ for HoTT-level,
    $\omega+1$ for stabilisation-tier, $\omega+2$ for cohesive,
    etc.; full table per
    `core/math/frameworks/registry.vr::populate_canonical_standard`).
  * $\tau$ — the trust tier (`true` = canonical,
    well-validated; `false` = under construction or
    not-yet-validated).

Verum's kernel encoding: `MsfsCoord { fw: Text, nu: Ordinal,
tau: Bool }` per `core.theory_interop.coord` module.

### A.Z.2.2 Defects in the (Fw, ν, τ) integration

1. **No kernel-rule consults the coordinate**. The shipped
   kernel rules (29 in §4.4a) typecheck terms without
   reference to their (Fw, ν, τ) coordinate. The intended
   discipline is that a theorem in framework Fw at depth ν
   should not freely reference a theorem at depth ν' > ν.
   **Recommended: cross-framework citation gate** — a new
   kernel rule that verifies the cited axiom's $\nu$ is
   $\leq$ the user's current $\nu$, modulo the K-Universe-Ascent
   tier-jumps in VVA-3.

2. **Coordinate inference**. Today the coordinate is read from
   the framework registry; user-declared theorems do not
   automatically get a $(\mathrm{Fw}, \nu, \tau)$ assignment.
   `verum audit --coord` (Task A2 from §16.1) should compute
   and surface the coordinate per theorem.

## A.Z.3 The AC/OC duality (Theorem 108.T)

**Source**: Diakrisis 12-actic. The dual of the canonical
primitive is the **Actic** structure — a categorical dual of
articulation with its own ordinal arithmetic ($\varepsilon$-invariants).
Theorem 108.T establishes the AC ↔ OC Morita equivalence
between articulation-side (canonical primitive) and
enactment-side (Actic) structures.

### A.Z.3.1 Mapping to Verum

  * `core.math.*` — articulation side (Outcome-Concerns / OC).
  * `core.action.*` — enactment side (Dynamic-Concerns / DC).
  * `@enact` annotation — the bridge attribute, witnessed by
    auto-induced ε(α) per articulation per VVA-1 V3.
  * `verum audit --epsilon` — surfaces the ε-coordinate of
    every `@enact`-annotated declaration.

### A.Z.3.2 Defects

1. **108.T proof not yet kernel-citable**. The OC/DC duality
   proof lives in the Diakrisis preprint (forthcoming per §17
   Q7); production compilation should gate `@enact` on the
   preprint release. Until then `core.action.*` modules are
   STAGED, not production-ready.

2. **Dual-AFN-T (109.T) absent from kernel-side check**. The
   actic-side no-go theorem mirrors AFN-T but has no kernel
   rule. Recommended: framework-axiom delegation; same
   strategy as Axi-9.

3. **Actic ε-invariant ordinals are decoupled from
   `OrdinalDepth`**. The kernel's `OrdinalDepth { omega_coeff,
   finite_offset }` Cantor-normal-form encoding is used by
   `m_depth_omega` for K-Refine-omega, but the Actic
   ε-coordinate is a *different* ordinal arithmetic (per
   Diakrisis 12-actic/03-epsilon-invariant.md). Recommended:
   document the relationship explicitly + define a
   `convert_eps_to_md_omega` bridge function.

## A.Z.4 Production-readiness criteria

Building on §5 unified success criteria + the user's "100%
production ready" directive, VVA must additionally satisfy:

| Criterion | Current | Target |
|---|---|---|
| Every shipped kernel rule has formal premise + V-stage tag | ✓ V8 #214 (29 rules in §4.4a) | ✓ Maintained |
| Every shipped kernel rule has implementation cross-ref | ✓ V8 #214 (file:fn) | ✓ Maintained |
| Every framework axiom passes K-FwAx soundness gate | ◐ (V8 #217 + #220 shipped; loader migration #222 done; production callers using strict regime) | Production CLI defaults `register_subsingleton` |
| Every Diakrisis axiom mapped to kernel rule OR framework axiom | ◐ (this chapter §A.Z.1; defects flagged) | Closure of Axi-1, Axi-4, Axi-9, T-α delegations |
| (Fw, ν, τ) coordinate per theorem | ◐ (registry populated; per-theorem inference pending) | `verum audit --coord` default-on |
| AC/OC duality wired | ◐ (`core.action.*` skeleton; 108.T preprint blocker) | `@enact` ungated post-preprint |
| Cross-tool replay matrix | ◐ (#90 tracked) | Lean + Coq + Agda day-one round-trip |
| Kernel TCB ≤ 6 500 LOC | ✓ (~4 700 LOC post-V8) | ✓ Maintained with audit gate |
| 100% test coverage of every kernel rule | ✓ V8 (254 kernel tests) | ✓ Maintained on every new rule |
| Zero `unsafe` in kernel | ✓ | ✓ Maintained |

Symbols: ✓ = shipped/maintained; ◐ = partial; ✗ = defect.

## A.Z.5 Synthesis roadmap & completion status

Concrete tasks to close the defects above, ordered by
fundamentality. Status legend: ✓ shipped (≥95% complete);
◐ partial (deliverables shipped, follow-up exists);
☐ not started.

Per-item completion based on **shipped artefacts**, no
speculation:

1. **Internal-closure delegation explicit** ✓ — V8 #226 stage
   3.2 ratifies the delegation in §A.Z.1.1 defect-1 row above
   (option (b) chosen: model-layer delegation per
   `core.math.frameworks.diakrisis*` framework axioms).
   Doc-only ratification. **100%**.

2. **Coordinate-aware citation gate** ✓ — V8 #227 + #232 shipped:
   `KernelCoord { fw, nu, tau }` type, `check_coord_cite`
   function, `KernelError::CoordViolation` variant,
   `AxiomRegistry::register_with_coord` entry point (14
   primary tests). V2 #232: typing-judgment integration via
   `infer_with_full_context(ctx, term, axioms, inductives,
   current_coord, allow_tier_jump)` — auto-fires
   `check_coord_cite` at every `CoreTerm::Axiom` reference
   site when both calling theorem and registered axiom have
   populated coords; graceful pass-through when either is
   absent. 5 new integration tests covering admit/reject/
   tier-jump/unannotated/legacy paths. **100%**.

3. **`verum audit --coord` per-theorem inference** ✓ — V8
   #230 shipped: `invert_to_per_theorem` collector +
   `PerTheoremCoord` row + per-theorem section appended to
   both Plain + JSON formatters of `audit_coord_with_format`.
   Max-of-cited-coords inference (lex on CliOrdinal) handles
   single-framework and multi-framework theorems uniformly.
   5 integration tests in
   `crates/verum_cli/tests/audit_coord_per_theorem.rs`
   covering clean / single / max-of-multi / JSON-schema-v1 /
   no-markers paths. **100%**.

4. **Accessibility typed attribute** ✓ — V8 #228 + #231 shipped:
   `verum_ast::attr::AccessibilityAttr` accepting `omega`,
   `omega_<n>`, `omega+<n>`, finite-cardinal tokens (16
   round-trip tests). V2 CLI walker `verum audit --accessibility`
   shipped (#231): walks every `@enact` marker, cross-
   references `@accessibility(λ)` annotations, surfaces
   uncovered sites with non-zero exit (CI gate). 5
   integration tests covering clean / covered / missing /
   mixed / JSON-schema-v1 paths. **100%**.

5. **Eps-invariant ↔ md-omega bridge** ✓ — V8 #229 shipped:
   `EpsInvariant` enum (Zero / Finite / Omega / OmegaPlus /
   OmegaTimes), `support::convert_eps_to_md_omega` helper,
   13 integration tests covering identity preservation +
   transfinite mapping + monotonicity + canonical
   minimum. **100%**.

6. **VVA-1 V3 τ-witness** ☐ — multi-week, preprint-blocked.
   **0%** (not actionable until Diakrisis preprint lands).

7. **Cross-tool replay matrix landing** ☐ — multi-week,
   external-tool-integration blocked. **0%**.

### A.Z.5 aggregate completion (V8 measured, no speculation)

  * Roadmap items shipped: **5 of 7** (items 1, 2, 3, 4, 5 ✓ shipped at the percentages above; items 6–7 externally blocked).
  * Per-item completion: 100% + **100%** + 100% + 100% + 100% + 0% + 0% = **500% of 700%**.
  * Aggregate roadmap completion: **71%** (500 / 700).
  * Tractable-non-blocked subset (items 1–5): **100%** (500 / 500). All 5 tractable items at 100%.
  * Externally-blocked subset (items 6–7): cannot ship until preprint + tool integrations.

### A.Z code-side refactor status (#225 stages)

  * Stage 1 (spec consolidation, VUVA + VFE → single VVA file): ✓ shipped commit 78858966. **100%**.
  * Stage 2 (`VfePolicy` → `ExtensionPolicy`, `vfe_gate.rs` → `extension_policy.rs`): ✓ shipped commit 8f266097. **100%**.
  * Stage 2.5 (mass VUVA + VFE → VVA uppercase rename across `crates/` `core/` source): ✓ shipped commit b39386b3 (sed-driven; `.rs` + `.vr`). **100%**.
  * Stage 2.6 (rename across `docs/` + `vcs/` + remaining stragglers + lowercase `vuva-version` CLI flag → `vva-version`, `vuva_*` test idents → `vva-*`): ✓ shipped this batch. Verified `grep -rln "VUVA\|VFE"` returns 0 hits across all source-controlled `.rs` `.vr` `.md` `.toml` files (excluding external Diakrisis + MSFS source materials in `internal/holon/`, intentionally preserved as authoritative upstream references). User-facing `vfe_<N>` annotation identifiers (`@require_extension(vfe_1)` etc.) are STABLE rollout-calendar markers retained verbatim per VVA §3 governance. **100%**.
  * Stage 3 (deep Diakrisis + MSFS synthesis): ◐ Stage 3.1 shipped (Part A.Z chapter, 13-axiomatics audit, MSFS-stratum mapping, AC/OC defect inventory, this completion table). **15%** of full deep-synthesis target. Remaining: per-VVA-N preprint-citation wiring, model-theoretic content from Diakrisis 02-canonical-primitive integrated as Part A.4.bis subsections, and AC/OC duality formal-statement integration into Part A.10.

This roadmap is the active execution surface of task #226 +
follow-ups.

---

# Part B — Extensions (Diakrisis-preprint-gated extensions, opt-in via `@require_extension`)

> Each Part B section is opt-in via `@require_extension(<name>)` per
> the unified rollout calendar (Part A §3 above is the policy spine;
> CLI surface is `verum_verification::ExtensionPolicy` per V8 #225's
> code-rename stage).


---

## 0. Executive Summary для разработчиков

### 0.0 Authority, versioning, governance

**Authority gradient**:
- **VVA** ([verification-architecture.md](./verification-architecture.md)) — *authoritative architectural specification*. Single source of truth for current architecture.
- **VVA** (этот документ) — *forward-looking proposal*. Subject to technical review + RFC process before adoption.

**Что VVA НЕ делает**:
- Не отменяет и не заменяет VVA.
- Не вводит breaking changes без обсуждения.
- Не обходит RFC-процесс.

**RFC-process для VVA-N**:
1. **Stage 0** (proposal): этот документ.
2. **Stage 1** (RFC): отдельная RFC с конкретными API + migration plan + rejection criteria.
3. **Stage 2** (prototype): proof-of-concept implementation в feature branch.
4. **Stage 3** (review): kernel team + theory reviewer sign-off.
5. **Stage 4** (merge): включение в main с update VVA.

**Versioning**:
- VVA: stable, semver.
- VVA: experimental until merged. After merge — VVA-versioned.
- Каждое VVA-N принятие — minor version bump VVA (e.g., VVA 1.5 + VVA-1 → VVA 2.0 после kernel rule addition).

**Backward compatibility**:
- Все VVA kernel rules **опт-ин** через `@require_extension(vfe_N)` annotation.
- Без annotation — kernel работает в VVA-baseline mode.
- 2-year deprecation window before extensions become default.

### 0.1 Что предлагается

VVA предлагает **6 фундаментальных расширений** ядра Verum, основанных на закрытых теоремах Diakrisis. Каждое расширение — отдельная инженерная программа на 6–18 месяцев; вместе они образуют **операциональное замыкание Verum** относительно полной Diakrisis-теории.

| ID | Расширение | Diakrisis-основание | Срок | Уровень |
|---|---|---|---|---|
| **VVA-1** | $\varepsilon$-arithmetic kernel rule | Предложение 5.1 + Теорема 124.T | 6 мес | kernel |
| **VVA-2** | Round-trip 108.T algorithm | Теорема 16.10 | 9 мес | stdlib + kernel |
| **VVA-3** | Stack-model semantics layer | Теорема 131.T | 12 мес | core/math |
| **VVA-4** | $(\infty,\infty)$-category support | Теорема 140.T | 18 мес | core/math/infinity |
| **VVA-5** | Constructive autopoiesis runtime | Теорема 141.T + 121.T | 12 мес | core/proof + runtime |
| **VVA-6** | Operational coherence checker | Теорема 18.T1 | 9 мес | core/verify |

**Дополнительно** — 4 enabler-расширения:

| ID | Расширение | Основание |
|---|---|---|
| **VVA-7** | K-Refine-omega kernel rule (трансфинитные модальности) | Теорема 136.T |
| **VVA-8** | Complexity-typed weak frameworks | Теорема 137.T |
| **VVA-9** | Effect-system как Kleisli-категория | Теорема 17.T1 |
| **VVA-10** | Ludics-cells via complicial sets | Теорема 120.T |

### 0.2 Чем это отличается от VVA

VVA задаёт *архитектуру* Verum как foundation-neutral host для $\mathfrak{M}$. VVA добавляет **операциональные слои** на основании теоретических расширений Diakrisis:

- VVA: T-2f\* как `K-Refine` kernel rule.
- **VVA-7**: T-2f\*\*\* как `K-Refine-omega` kernel rule (трансфинитные модальные ранги).

- VVA: 108.T как `K-Adj-Unit/Counit` kernel rules.
- **VVA-1, VVA-2**: Полное сопряжение $\mathsf{M} \dashv \mathsf{A}$ с явными unit/counit + алгоритмическая round-trip-проверка.

- VVA: Cat-модель как baseline.
- **VVA-3**: $(\infty, 2)$-стек-модель как полный stage для всех 13 аксиом Diakrisis.

- VVA: $(\infty, 1)$ через `core.math.infinity_category`.
- **VVA-4**: Полный $(\infty, \infty)$ через комплициальные множества.

- VVA: автопоэзис как property-аннотация.
- **VVA-5**: Конструктивный $\omega^2$-итератор в эффективном топосе.

- VVA: 9-стратегийная verification-лестница.
- **VVA-6**: Двусторонняя $\alpha/\varepsilon$-coherence как 10-я стратегия (`@verify(coherent)`).

### 0.3 Обязательное чтение для разработчиков

**Перед любой работой над VVA** разработчики должны проанализировать (в порядке приоритета):

#### Tier 0 — теоретическое ядро (must-read)

1. [`/12-actic/04-ac-oc-duality`](https://...diakrisis/12-actic/04-ac-oc-duality) §5 — **Предложение 5.1** ($\varepsilon \circ \mathsf{M} \simeq \mathsf{A} \circ \varepsilon$). Полное доказательство в 5 шагах + 6 лемм. Эта теорема — корневая для VVA-1.

2. [`/12-actic/06-actic-theorems`](https://...diakrisis/12-actic/06-actic-theorems) §14.2 — **Теорема 124.T** ($\mathsf{M} \dashv \mathsf{A}$ biadjunction). Hom-bijection $\Phi$ + явные unit/counit как канонические образы тождеств. Основание для kernel rules `K-Adj-Unit` / `K-Adj-Counit`.

3. [`/02-canonical-primitive/02-axiomatics`](https://...diakrisis/02-canonical-primitive/02-axiomatics) §131.T — **Теорема 131.T** ((∞,2)-стек-модель). 6 шагов + 3 леммы (131.L1, 131.L2, 131.L3). Содержит Drake reflection argument и Tarski undefinability — критично для понимания κ-башни.

4. [`/03-formal-architecture/16-gauge-decision`](https://...diakrisis/03-formal-architecture/16-gauge-decision) — **Теорема 16.10** (round-trip 108.T) + Конструкция 16.3 `canonicalize`. Алгоритмическая основа VVA-2.

5. [`/09-applications/03-operational-coherence`](https://...diakrisis/09-applications/03-operational-coherence) — **Теорема 18.T1** + bridge $T_{108}$. Операциональное замыкание Diakrisis-теории; основа VVA-6.

#### Tier 0+ — прикладной контекст (Diakrisis applications layer)

**Обязательно к прочтению ДО начала любой работы над VVA**: четыре документа `/09-applications/` задают прикладной контекст для Verum.

A1. [`/09-applications/00-path-B-uhm-formalization`](https://...diakrisis/09-applications/00-path-B-uhm-formalization) — **Путь Б**: формализация УГМ (Унитарный Голономный Монизм) в Verum. Это **главная прикладная программа** Diakrisis: 223 теоремы УГМ должны быть machine-checked в Verum. Пять критериев успеха К-Б-1..К-Б-5; пять правил работы S'-1..S'-5. Без понимания Пути Б Verum — proof assistant без специфической миссии.

A2. [`/09-applications/01-verum-integration`](https://...diakrisis/01-verum-integration) — **Verum integration plan** со стороны Diakrisis: какие базовые типы (ℂ, Hilbert spaces, density operators, CPTP maps, Lindblad generators), 2-categorical structures, spectral triples (NCG), cohomology groups Diakrisis ожидает от Verum. **Это формальный contract** между Diakrisis и Verum.

A3. [`/09-applications/02-canonical-nu-table`](https://...diakrisis/09-applications/02-canonical-nu-table) — **Каноническая ν-таблица** для Verum-frameworks. **Прескриптивный** документ, фиксирующий ν-координаты для каждого framework в `core.math.frameworks.*`:

| Framework | ν | τ |
|---|---|---|
| `actic.raw` | 0 | extensional |
| `lurie_htt` | ω | intensional |
| `schreiber_dcct` | ω+2 | intensional |
| `connes_reconstruction` | ω | extensional |
| `petz_classification` | 2 | extensional |
| `arnold_catastrophe` | 2 | intensional |
| `baez_dolan` | ω+1 | intensional |
| `owl2_fs` | 1 | intensional |

**Критично для VVA-1, VVA-3, VVA-4**: lookup-таблица в Verum **должна** соответствовать этой Diakrisis-стороне; CLI `verum audit --coord` синхронизирует обе.

A4. [`/09-applications/03-operational-coherence`](https://...diakrisis/09-applications/03-operational-coherence) — **Теорема 18.T1** (см. пункт 5 выше; повторение для полноты).

#### Tier 0++ — Diakrisis-стороннние спецификации для Verum (`/12-actic/09–10`)

**Это самые конкретные документы для разработчика Verum** — они содержат прямые скетчи кода и tracker реализации.

B1. [`/12-actic/09-verum-stdlib-sketch`](https://...diakrisis/12-actic/09-verum-stdlib-sketch) — **АКТИВНЫЙ 10-step interaction plan** для Verum integration. Содержит **прямой sketch кода** для:
- `core.action.primitives` — 7 базовых ε-актов (ε_math, ε_prove, ε_compute, ε_observe, ε_decide, ε_translate, ε_construct, ε_classify) с явными `@epsilon(...)` annotations.
- `core.action.enactments` — операции композиции (enact_then, enact_par, activate, activate_n, autopoiesis).
- `core.action.gauge` — GaugeXform, canonicalize, gauge_morph.
- `core.action.verify` — verify_achieves, audit_epsilon, verify_gauge, verify_autopoiesis.
- 10 шагов реализации с conkretnymi приёмочными критериями.

**Это документ, который VVA-1, VVA-9, VVA-10 должны реализовать.** Verum-разработчики **обязаны** работать в синхроне с этими шагами.

B2. [`/12-actic/10-implementation-status`](https://...diakrisis/12-actic/10-implementation-status) — **live tracker** интеграции Verum:
- Sводная таблица 4 фаз (α / β / γ / δ) с целевыми датами Q3 2026 → Q2 2027.
- Per-Шаг status: ⚪ план / 🟡 в работе / ✅ завершено.
- Gap-аналитика: что Verum уже даёт, что нужно реализовать, что не покрыто.
- Метрики прогресса.
- **Раздел «Теоретические основания закрыты [Т·L3]»** — список 12 теорем (Предложение 5.1, 124.T, 131.T, 16.10, 140.T, 141.T, 136.T, 137.T, 121.T, 120.T, 17.T1, 18.T1).

**Verum-разработчики должны обновлять этот документ** при достижении milestones (cross-ownership с Diakrisis-теоретиками).

#### Tier 1+ — Актика-теоретические документы (`/12-actic/00–08`)

C1. [`/12-actic/00-foundations`](https://...diakrisis/12-actic/00-foundations) — обзор Актика как ДЦ-дуала Diakrisis. Объясняет позиционирование $\rangle\!\rangle\cdot\langle\!\langle$ как 2-категории актов.

C2. [`/12-actic/01-historical-lineage`](https://...diakrisis/12-actic/01-historical-lineage) — историческая родословная (35+ ДЦ-традиций от Анаксимандра до Брауэра). Контекст для `@framework` declarations философских традиций.

C3. [`/12-actic/02-dual-primitive`](https://...diakrisis/12-actic/02-dual-primitive) — формальный примитив Актика: 13 аксиом A-0..A-9 + T-ε + T-2a* + T-ε_c. **Дуал к /02-canonical-primitive/02-axiomatics**.

C4. [`/12-actic/03-epsilon-invariant`](https://...diakrisis/12-actic/03-epsilon-invariant) — **полный каталог ε-координат** для всех стандартных ε-актов:

| Акт | ε-координата |
|---|---|
| `ε_math, ε_compute, ε_observe, ε_decide` | $\omega$ |
| `ε_translate, ε_enact, ε_LEM` | $\omega + 1$ |
| `ε_AFA, ε_NCG, ε_∞-topos, ε_cohesion` | $\omega \cdot 2$ |
| `ε_motivic, ε_Метастемология, ε_Д-hybrid` | $\omega \cdot 2 + 1$ |
| `ε_uhm` (УГМ) | $\omega \cdot 3 + 1$ |
| `ε_автопоэзис, ε_SMD` | $\omega^2$ |
| `ε_верификация` | $\omega \cdot 2$ |
| `ε_∞-cat, ε_Apeiron` | $\Omega$ |

**Прескриптивный** для `@enact(epsilon=...)` annotations в Verum.

C5. [`/12-actic/05-dual-afn-t`](https://...diakrisis/12-actic/05-dual-afn-t) — **109.T** (Dual-AFN-T): дуальная no-go теорема для абсолютной практики. Граница для Verum's autopoiesis-related verification.

C6. [`/12-actic/07-beyond-metastemology`](https://...diakrisis/12-actic/07-beyond-metastemology) — Метастемология Е. Чурилова с ε = ω·2+1 (Теорема 125.T). Конкретный пример ДЦ-практики; useful для test cases в VVA-9.

C7. [`/12-actic/08-formal-logical-dc`](https://...diakrisis/12-actic/08-formal-logical-dc) — **формально-логическое техническое ядро** Актика: BHK-семантика, MLTT, диалогическая логика Лоренцена, Game-семантика Hintikka, Ludics Жирара, Curry-Howard-Lambek, π-calculus / Actor / CSP. **Это семь концrete entry points** для VVA-7, VVA-9, VVA-10 — каждое формально-логическое направление имеет ε-image в Verum.

#### Tier 1 — расширения семантики

6. [`/12-actic/06-actic-theorems`](https://...diakrisis/12-actic/06-actic-theorems) §15.3 — **Теорема 140.T** + Леммы 140.L0/L1/L2 (adjoint tower, accessibility, universality $\mathrm{e}^\infty$). Основа VVA-4.

7. [`/02-canonical-primitive/02-axiomatics`](https://...diakrisis/02-canonical-primitive/02-axiomatics) §T-2f\*\*\* — **Теорема 136.T** + Definition 136.D1 (трансфинитный модальный язык $L^\omega_\alpha$) + Лемма 136.L0 (well-founded ordinal recursion для $\mathrm{md}^\omega$). Основа VVA-7.

8. [`/12-actic/06-actic-theorems`](https://...diakrisis/12-actic/06-actic-theorems) §15.4 — **Теорема 141.T** + Леммы 141.L1/L2/L3 (higher-type computability в Eff, AFA-аналог через Aczel-Capretta 2017, замкнутость biological category под $\mathsf{A}$). Основа VVA-5.

9. [`/06-limits/05-what-remains-possible`](https://...diakrisis/06-limits/05-what-remains-possible) §137.T — **Теорема 137.T** + 6-уровневая $\nu^\mathrm{weak}$-стратификация ($\mathrm{AC}^0 \subset \mathrm{LOGSPACE} \subset \mathrm{P} \subset \mathrm{NP} \subset \mathrm{PH} \subset \mathsf{I}\Delta_0$). Основа VVA-8.

#### Tier 2 — операциональные мосты

10. [`/12-actic/06-actic-theorems`](https://...diakrisis/12-actic/06-actic-theorems) §11.1 — **Теорема 120.T** (Ludics ≃ Perf(α_linear)). Конструкция функтора $\Phi: \mathbf{Ludics} \to \mathrm{Perf}(\alpha_\mathrm{linear})$ + Лемма 120.L3 (cut-elimination). Основа VVA-10.

11. [`/12-actic/06-actic-theorems`](https://...diakrisis/12-actic/06-actic-theorems) §12.1 — **Теорема 121.T** (BHK как ε-семантика) + Definition 121.D1 (категорная BHK) + Лемма 121.L_central (структурная индукция). Основа VVA-5 (proof-extraction).

12. [`/03-formal-architecture/17-effects-and-linear`](https://...diakrisis/03-formal-architecture/17-effects-and-linear) — **Теорема 17.T1** (effects ≃ Perf(α_linear) projections) + Конструкция 17.K1 (Kleisli-вложение strong monads) + каталог стандартных эффектов с commutativity flags. Основа VVA-9.

#### Tier 3 — контекст и согласование

13. [`/10-reference/04-afn-t-correspondence`](https://...diakrisis/10-reference/04-afn-t-correspondence) — Полная карта соответствия Diakrisis ↔ MSFS ↔ Актика. **Обязательно** для любой работы, затрагивающей формализацию теорем как `@framework` axioms.

14. [`/10-reference/05-corpus-correspondence`](https://...diakrisis/10-reference/05-corpus-correspondence) — Соответствие 5 корпусов: MSFS, Diakrisis, Актика, УГМ, Noesis. Single-source policy: MSFS — first-source для всего, что в нём формализовано.

15. [`/10-reference/02-theorems-catalog`](https://...diakrisis/10-reference/02-theorems-catalog) — Полный каталог теорем 1.T–142.T с эпистемическими статусами [Т·L1/L2/L3]. **Источник правды** для `@framework` axiom citations.

---

## 1. VVA-1 — ε-arithmetic kernel rule

**Diakrisis-основание**: Предложение 5.1 (R1, [/12-actic/04-ac-oc-duality §5](https://...)) + Следствие 5.10: $\nu(\alpha) = \mathsf{e}(\varepsilon(\alpha))$.

### 1.1 Текущее состояние (VVA)

VVA §10.4 определяет `coord(α) = (Framework, ν, τ)` через `nu(α)`. ε-арифметика отсутствует на kernel-уровне; `verum audit --epsilon` (если бы существовал) опирался бы на постулат, не на теорему.

### 1.2 Предложение

Добавить в `verum_kernel`:

```rust
// crates/verum_kernel/src/lib.rs

#[derive(Clone, Debug)]
pub enum CoreTerm {
    // ... existing variants
    EpsilonOf(Box<CoreTerm>),  // ε(α) — dual of α
    AlphaOf(Box<CoreTerm>),    // α(ε) — inverse
}

impl Kernel {
    /// K-Eps-Mu rule: ε(M(α)) ≃ A(ε(α)) up to canonical 2-cell τ.
    /// Encodes Proposition 5.1 of Diakrisis.
    pub fn check_eps_mu_coherence(&self, alpha: &CoreTerm) -> Result<TwoCell, KernelError> {
        let lhs = self.eval(&CoreTerm::EpsilonOf(Box::new(self.eval_M(alpha)?)));
        let rhs = self.eval_A(&CoreTerm::EpsilonOf(Box::new(alpha.clone())))?;
        self.check_2cell_equivalence(lhs, rhs)
    }

    /// Canonical naturality: τ_α : ε(M(α)) → A(ε(α))
    pub fn naturality_tau(&self, alpha: &CoreTerm) -> Result<TwoCell, KernelError> {
        // explicit construction per /12-actic/04-ac-oc-duality §5.1
        // τ_α := (σ_α, π_α) where:
        //   σ_α: Syn(M(α)) → M_cat(Syn(α))    -- per Lemma 5.2
        //   π_α: Perf(M(α)) → A_cat(Perf(α))  -- per Lemma 5.3
        let sigma = self.code_S_morphism(alpha)?;
        let pi = self.perform_eps_math_morphism(alpha)?;
        Ok(TwoCell::pair(sigma, pi))
    }
}
```

### 1.3 Kernel rule (positioning vs VVA K-Adj-Unit/Counit)

VVA §2.5 уже включает `K-Adj-Unit` / `K-Adj-Counit` как kernel rules для 108.T-эквивалентности $\varepsilon \dashv \alpha$. **VVA-1 не заменяет их**, а добавляет **дополнительное naturality правило** для естественной эквивалентности $\tau$ из Предложения 5.1:

```
  Γ ⊢ α : Articulation        Γ ⊢ τ_α : ε(M(α)) ≃ A(ε(α))
  ────────────────────────────────────────────────────── (K-Eps-Mu)
  Γ ⊢ τ_α : EquivCell{ε∘M, A∘ε}
```

**Различие**:
- `K-Adj-Unit`: проверяет unit $\eta: \mathrm{id} \Rightarrow \alpha \circ \varepsilon$ (108.T core).
- `K-Adj-Counit`: проверяет counit $\epsilon: \varepsilon \circ \alpha \Rightarrow \mathrm{id}$ (108.T core).
- **`K-Eps-Mu` (новое)**: проверяет $\tau_\alpha: \varepsilon \circ \mathsf{M} \Rightarrow \mathsf{A} \circ \varepsilon$ — это **компонент сопряжения 124.T** ($\mathsf{M} \dashv \widetilde{\mathsf{A}}$), отличный от 108.T-данных.

Без `K-Eps-Mu`: `verum audit --epsilon` опирается на постулат $\nu = \mathsf{e} \circ \varepsilon$. С `K-Eps-Mu`: это теорема (Следствие 5.10).

### 1.4 Обязательные чтения для VVA-1

1. [/12-actic/04-ac-oc-duality §5.1](https://...) — Шаг C.1: явная Конструкция 5.4 для $\tau_\alpha$.
2. [/12-actic/04-ac-oc-duality §5.2](https://...) — Шаги C.2: естественность по 1-морфизмам через Лемму 5.5 ($\mathrm{Code}_S$ naturality, Smoryński 1985 §1) и Лемму 5.6 ($\mathrm{Perform}_{\varepsilon_\mathrm{math}}$ naturality через A-3).
3. [/12-actic/04-ac-oc-duality §5.5](https://...) — Шаг C.5: Лемма 5.8 о различении объектной/функториальной accessibility — критично для compiler implementation.
4. [/12-actic/03-epsilon-invariant.md](https://...) — каталог ε-координат для всех стандартных артикуляций.

### 1.5 Acceptance criteria

- `verum audit --epsilon src/` работает синхронно с `verum audit --coord src/`: для каждого theorem $T$, $\mathsf{e}(\varepsilon(T)) = \nu(T)$.
- Kernel re-checks naturality $\tau$ для каждого `@enact(epsilon=...)` annotation.
- 0 ошибок certificate mismatch на корпусе из 142 Diakrisis-теорем.

### 1.6 Сложность

- Object-accessibility check: $O(|\alpha|^2)$.
- Functorial-accessibility check: $O(|\alpha|^3)$ через κ-фильтрованный colimit preservation.
- Полная τ-naturality: $O(|\alpha|^4)$ для 2-функториальности.

### 1.7 Implementation status (V0/V1/V2 honest disclaimer)

VVA-1 ships in three increments. Each is а **strict tightening** of the previous; none are silent demotions of an earlier soundness claim.

**V0 (shipped)** — `check_eps_mu_coherence` accepts any structurally well-formed pair `(EpsilonOf(_), AlphaOf(_))`. Permissive skeleton; the kernel records the rule's existence so other passes can reference its diagnostic surface. Not a soundness claim about τ-witness.

**V1 (shipped)** — *shape-check tightening*. Enforces:
  • `lhs == rhs` (structural identity) ⇒ accept (degenerate naturality).
  • Canonical shape `(EpsilonOf(M_α), AlphaOf(EpsilonOf(α)))` requires `AlphaOf`'s inner term to itself be an `EpsilonOf` constructor; malformed inners are rejected.
  • Identity-functor sub-case `M = id` ⇒ `M_α == α` structurally ⇒ accept.
  • Anything else (including non-canonical lhs/rhs shapes) ⇒ reject with `EpsMuNaturalityFailed`.

**V2 (shipped, см. commit `b152d3fa`)** — *modal-depth preservation pre-condition*. For non-identity M (`M_α ≠ α` structurally), V2 adds `m_depth_omega(M_α) == m_depth_omega(α)` as a NECESSARY (but not sufficient) condition: the canonical natural-equivalence $\tau : \varepsilon \circ \mathsf{M} \simeq \mathsf{A} \circ \varepsilon$ is depth-preserving (an $(\infty, 1)$-categorical equivalence), so a depth mismatch precludes any τ-witness. Soundness:
  • Depth mismatch ⇒ reject is **correct** (no τ-witness can exist).
  • Depth match ⇒ V2 still conservatively **accepts** (the check is necessary, not sufficient).

**Architectural caveat.** V2's depth check applies `m_depth_omega` to the inner `α`-shaped term, NOT to a hypothetical metaisation `M(α)` evaluated structurally. The CoreTerm calculus does not (yet) have a `MetaApp(M, t)` constructor whose depth is `dp(t) + 1` — `EpsilonOf` and `AlphaOf` are atomic wrappers per VVA §4.3. Consequently, two terms encoded with the same surface shape but with M's action on different sides have indistinguishable `m_depth_omega` ranks. **The V2 check, while sound, has VACUOUS preconditions for the canonical-shape case** (both sides are atomic-rank-0 if their inner Vars are atomic). Discharge: V3's full σ_α / π_α witness construction (#181) will introduce explicit M-tracking (likely as a new `MetaApp(M, t)` CoreTerm constructor with `dp(MetaApp) = dp(t) + 1`), making V2's depth check materially constraining. Until V3, V2's depth-mismatch rejection covers only modal-operator-overshoot cases (where one side is wrapped in `ModalBox`/`ModalDiamond` and the other is not) — a strict tightening over V1 but not the full sufficient witness check.

**V3 (deferred — multi-week, tracked under #181)** — explicit τ-witness construction:
  • σ_α from `Code_S` morphism (Smoryński 1985 §1, see [/12-actic/04-ac-oc-duality §5.2 Lemma 5.5](https://...)).
  • π_α from `Perform_{ε_math}` naturality through axiom A-3 (see Lemma 5.6).
  • Reasoning about M's action on non-trivial articulations.
  • Integration with the kernel's structure-recursion judgement (Theorem 16.6 semi-decidability).

V3 will replace V2's necessary-condition with the actual sufficient witness check; V2's diagnostic codes carry over without breaking the `EpsMuNaturalityFailed` surface.

---

## 2. VVA-2 — Round-trip 108.T algorithm

**Diakrisis-основание**: Теорема 16.10 (R5, [/03-formal-architecture/16-gauge-decision §5](https://...)) + Теорема 16.5 (разрешимость для finitely-axiomatized R-S за $O(2^{O(|\alpha|)})$).

### 2.1 Текущее состояние

VVA §10.1-10.3 даёт `load_theory`, `translate`, `check_coherence` (Yoneda, Kan, Čech). Round-trip property — заявлена в Шаге 10 интеграционного плана `/12-actic/09-verum-stdlib-sketch`, но без алгоритмической реализации.

### 2.2 Предложение

Создать новый модуль `core.theory_interop.bridges.oc_dc_bridge`:

```verum
// core/theory_interop/bridges/oc_dc_bridge.vr

@framework(diakrisis_round_trip, "Theorem 16.10: round-trip 108.T property")

trait Articulation {
    fn syn(&self) -> SynCategory;       // (∞, n)-категория, R5a
    fn perf(&self) -> PerfCategory;     // ε-перформансы, Lemma 3.2
    fn axiomatization(&self) -> Set<Axiom>;
    fn signature(&self) -> Signature;
}

trait Enactment {
    fn syn_component(&self) -> SynCategory;
    fn perf_component(&self) -> PerfCategory;
}

// translate: α ↦ ε(α) — Construction 3.1 in /12-actic/04-ac-oc-duality
fn translate(α: impl Articulation) -> impl Enactment {
    Enactment::pair(α.syn(), α.perf())
}

// inverse: ε ↦ α(ε) = [ε_math, ε]^hom — Construction 6.1 in /12-actic/04-ac-oc-duality
fn inverse(ε: impl Enactment) -> impl Articulation {
    Articulation::internal_hom(ε_math, ε)
}

// canonicalize per Construction 16.3 in /03-formal-architecture/16-gauge-decision
fn canonicalize(α: impl Articulation) -> impl Articulation {
    α.congruence_closure()         // Step 1: Nieuwenhuis-Oliveras 2007
     .idempotent_complete_morita() // Step 2: Lurie HTT §5.1.4 Prop 5.1.4.7
     .lex_minimal_in_gauge_orbit() // Step 3: lex-minimal по канонический order
}

@verify(formal)
theorem round_trip_property<A: Articulation>(α: A)
    where α.is_finitely_axiomatized()
    ensures canonicalize(inverse(translate(α))) == canonicalize(α)
    complexity O(2^O(|α|))  // single-exponential
;

@verify(reliable)
theorem gauge_decidability_finite<A: Articulation>(α1: A, α2: A)
    where α1.is_finitely_axiomatized() && α2.is_finitely_axiomatized()
    ensures gauge_equivalent(α1, α2) is_decidable
;

// Lower bound: word problem reduction (Novikov-Boone 1955)
@verify(formal)
theorem gauge_undecidable_general()
    ensures ¬∃ algorithm: ∀(α1, α2). decides(gauge_equivalent(α1, α2))
    proof_via Theorem 16.7 in /03-formal-architecture/16-gauge-decision
;
```

### 2.3 Kernel integration

Новое kernel rule `K-Round-Trip` для проверки соответствия α/ε-сертификатов:

```
  Γ ⊢ α : Articulation     α.is_finitely_axiomatized()
  Γ ⊢ canonicalize(inverse(translate(α))) =_syn canonicalize(α)
  ──────────────────────────────────────────────────────────── (K-Round-Trip)
  Γ ⊢ RoundTripCert{α} : Type
```

### 2.4 Обязательные чтения для VVA-2

1. [/03-formal-architecture/16-gauge-decision §3](https://...) — Теорема 16.5 (разрешимость) + Конструкция 16.3 (canonicalize). **Критично**: не использовать Knuth-Bendix (не всегда terminates) — только Nieuwenhuis-Oliveras congruence closure.
2. [/03-formal-architecture/16-gauge-decision §4](https://...) — Теорема 16.6 (Σ_1-полу-разрешимость) + Теорема 16.7 (нижняя граница через Novikov-Boone). Verum должен корректно сообщать пользователю о semi-decidability для recursively-axiomatized R-S.
3. [/12-actic/04-ac-oc-duality §3](https://...) — Конструкция 3.1 для $\varepsilon(\alpha)$.
4. [/12-actic/04-ac-oc-duality §6](https://...) — Конструкция 6.1 для $\alpha(\varepsilon)$.
5. [/03-formal-architecture/04-gauge.md](https://...) — концептуальная gauge-структура (предшественник 16-gauge-decision).

### 2.5 Acceptance criteria

- Round-trip успешен для всех 132 OC + 21 AC теорем (153 теста).
- Сложность: ≤ 2^O(|α|) wallclock per теорема.
- Корректное reporting `gauge_equivalent` как `Decidable / SemiDecidable / Undecidable` в зависимости от axiomatization-finiteness.

---

## 3. VVA-3 — Stack-model semantics layer

**Diakrisis-основание**: Теорема 131.T ([/02-canonical-primitive/02-axiomatics §131.T](https://...)) + Леммы 131.L1, 131.L2, 131.L3.

### 3.1 Текущее состояние

> **Историческая ссылка (B15, #213).** Ранние редакции VVA
> упоминали "6-pack стандартных frameworks" (`zfc`, `hott`,
> `mltt`, `cic`, `ncg`, `infinity_topos`). Реальный каталог
> `core/math/frameworks/` шипит существенно больше (см. VVA §6.2,
> приведённую к фактическому состоянию в B15-патче) — девять
> Standard, VerifiedExtension-семейство `bounded_arithmetic_*`, и
> четыре диакризисных под-корпуса. В частности `arnold_mather` был
> переименован в `arnold_catastrophe` для соответствия registry.

Кат-модель — единственная реализация. Axi-8 (нетривиальность $\alpha_\mathsf{M}$) **не реализуется** в Cat-модели.

### 3.2 Предложение

Добавить `core.math.stack_model` с явной κ-башней:

```verum
// core/math/stack_model.vr

@framework(diakrisis_stack_model_131T, "Theorem 131.T: (∞,2)-stack model for 13 axioms")

/// Гротендик-универсумы κ_1 < κ_2 в ZFC + 2-inacc.
/// По Лемме 131.L_R: md^ω ограничен сверху κ_2 для R-S артикуляций.
type Universe = κ_1 | κ_2;  // только два уровня (по 134.T тугая граница)

/// (∞,2)-стек $\mathfrak{M}^\mathrm{stack}_\mathrm{Diak}$.
/// Объекты: пары (F, φ_F), F ∈ \mathcal{F}, φ_F ∈ Syn(S).
trait StackArticulation: Articulation {
    fn universe_level(&self) -> Universe;  // κ_1 или κ_2
    fn stack_object(&self) -> StackObject;
    fn descent_data(&self) -> DescentData;  // hyperdescent property
}

/// $\mathsf{M}^\mathrm{stack}$ — мета-классификация.
/// По Лемме 131.L1: M_stack(F) ∈ U_2 для F ∈ U_1 (logical strength ascent).
@enact(epsilon = "ε_metaize")
fn M_stack(F: impl StackArticulation) -> impl StackArticulation {
    let cls = horizontal_classification(F);  // MSFS §3, /12-actic/06-actic-theorems
    cls.lift_universe_via_drake_reflection()  // Step 5 in 131.T proof
}

/// Лемма 131.L3: stack-стабилизация на объектном уровне.
/// На U_2-уровне M_stack — внутренний рефлектор без выхода в U_3.
@verify(formal)
theorem internal_reflector_U2(G: impl StackArticulation)
    where G.universe_level() == κ_2
    ensures M_stack(G).universe_level() == κ_2  // не κ_3!
    proof_via Drake reflection 1974 §3.4 + Tarski undefinability
;

/// Лемма 131.L2: колимит κ-башни не representable как объект M_stack.
@verify(certified)
theorem kappa_tower_not_representable<F: StackArticulation>(seq: Sequence<F>)
    ensures ¬∃ α: M_stack. α == colim_n M_stack^n(seq[n])
    proof_via Yoneda + ZFC+2-inacc bounds (no U_3)
;

/// Axi-8 нетривиально: α_M не Yoneda-представим единым объектом.
/// Требует stack-model; в Cat-модели нарушается (по 14.T2).
@verify(thorough)
axiom axi_8_nontrivial_in_stack()
    ensures ¬∃ α: StackArticulation: ρ(α_M)(-) ≃ Hom_stack(-, α)
    proof_via Theorem 131.T(а) Step 3
;
```

### 3.3 Cat-модель как $(2, \kappa_1)$-усечение

```verum
// Cat-model is a canonical truncation of stack-model
fn cat_model_from_stack(stack: StackModel) -> CatModel {
    stack.truncate_at(level = 2, universe = κ_1)
}

@verify(certified)
theorem cat_is_truncation_of_stack()
    ensures cat_model_from_stack(stack_model) ≃ Cat_model_baseline()
    proof_via Theorem 131.T(г)
;
```

### 3.4 Обязательные чтения для VVA-3

1. [/02-canonical-primitive/02-axiomatics §131.T](https://...) — **полное** доказательство (Шаги 1-6 + 3 леммы). Особенно:
   - Шаг 1 (конструкция стека через Pronk 1996 + Verity 2008).
   - Шаг 5 (стабилизация через Drake reflection 1974 §3.4 + Kanamori 2009 §10).
   - Шаг 6 (согласованность с AFN-T через global hyperdescent).
2. [/02-canonical-primitive/02-axiomatics §«Совместимость с Axi-8»](https://...) — соотношение Axi-7 / Axi-8.
3. [/06-limits/05-what-remains-possible §134.T](https://...) — тугость границы ZFC + 2-inacc (1-inacc недостаточно).
4. **SGA 4** Exposé I §1.0 — identification Гротендик-универсума $\mathbf{U}_k \simeq V_{\kappa_k}$.
5. **Lurie HTT** §6.2.4 — fpqc topology + §6.2.5 hyperdescent для (∞,1).

### 3.5 Архитектурное разделение: kernel vs stdlib

**Критично**: VVA-3 — **в основном stdlib**, не kernel. Это минимизирует impact на kernel size:

| Компонент | Уровень | LOC budget |
|---|---|---|
| `core.math.stack_model` | stdlib | ~3000 |
| Universe tracking via `@framework` metadata | stdlib | ~500 |
| `K-Universe-Ascent` kernel rule (см. ниже) | kernel | ~200 |
| `verum audit --coord --universe` CLI extension | tooling | ~300 |

Из ~4000 LOC общего объёма — только **200 LOC попадает в kernel**. Остальное — stdlib, runtime tooling.

#### K-Universe-Ascent — формальная спецификация

```
  Γ ⊢ α : Articulation@U_k       Γ ⊢ M_stack(α) : Articulation@U_{k+1}
  ──────────────────────────────────────────────────────────────────── (K-Universe-Ascent)
  Γ ⊢ M_stack : Functor[Articulation@U_k → Articulation@U_{k+1}]
```

Где `@U_k` — универсе-аннотация (k ∈ {1, 2}). Правило проверяет:
1. Корректность κ-уровня артикуляции (через `@framework` metadata).
2. Согласованность $\mathsf{M}^\mathrm{stack}$ как functor U_1 → U_2 (Лемма 131.L1).
3. Drake reflection retract на U_2-уровне (Лемма 131.L3) — для повторного применения $\mathsf{M}^\mathrm{stack}$ на U_2 без выхода в U_3.

Реализация в kernel: ~200 LOC, проверяет только metadata-tags + composition. Полная Drake reflection и Tarski undefinability аргументы — в stdlib (`core.math.stack_model`).

### 3.6 Migration path

- **Phase 1 (3 мес)**: добавить `core.math.stack_model` с трёх-уровневой структурой (`κ_1`, `κ_2`, `truncated`). Все существующие frameworks выполняются в `truncated` уровне (Cat-equivalent).
- **Phase 2 (6 мес)**: добавить κ-tracking в `verum audit --coord`: каждая теорема получает дополнительную координату `universe_level`.
- **Phase 3 (12 мес)**: перевести всю верификацию через stack_model; Cat-модель становится синтаксическим сахаром для truncated-режима.

---

## 4. VVA-4 — $(\infty,\infty)$-category support

**Diakrisis-основание**: Теорема 140.T ([/12-actic/06-actic-theorems §15.3](https://...)) + Леммы 140.L0 (colim ≃ lim adjoint tower), 140.L1 (accessibility $\mathsf{A}^\infty$), 140.L_stack (совместимость с 131.T).

### 4.1 Текущее состояние

VVA `core.math.infinity_category` поддерживает только $(\infty, 1)$. Это блокирует институциональные ($\omega^2$) и цивилизационные ($\omega \cdot 3 + 1$) ε-координаты.

### 4.2 Предложение

Расширить `core.math.infinity_category` до полного $(\infty, \infty)$ через **стратифицированные комплициальные множества** (Verity 2008 + Riehl-Verity 2022 §10):

```verum
// core/math/infinity_category.vr (расширение)

@framework(infinity_n_via_complicial, "Verity 2008: weak complicial sets")

/// (∞, n)-категория для каждого n ∈ ℕ.
type InfinityNCategory<const N: usize>;

/// Truncation functor τ_{≤n}: (∞, n+1)-Cat → (∞, n)-Cat.
fn truncate<const N: usize>(c: InfinityNCategory<{N+1}>) -> InfinityNCategory<N>;

/// Inclusion functor ι_n: (∞, n)-Cat ↪ (∞, n+1)-Cat.
fn include<const N: usize>(c: InfinityNCategory<N>) -> InfinityNCategory<{N+1}>;

/// (∞, ∞)-категория как adjoint colim/lim tower.
/// Лемма 140.L0: colim ≃ lim канонически через ι_n ⊣ τ_{≤n}.
type InfinityInfinityCategory = AdjointTower<InfinityNCategory>;

/// Accessibility $\mathsf{A}^\infty$ — Лемма 140.L1.
/// Inductive proof: Lurie HA + Riehl-Verity 2022 §4.5 для (∞,2);
/// extended via Bergner-Rezk 2013 для (∞,n) all n.
trait AccessibleInfinityFunctor {
    fn preserves_aleph_1_filtered_colimits(&self) -> bool;
}

/// ε-инвариант на (∞,∞)-уровне (Теорема 140.T Свойство 4).
@verify(certified)
fn epsilon_infinity(act: ActOnInfinityInfinity) -> Ordinal {
    // min{β: act ∈ colim_{κ<β} A^∞^κ(ε_math)}
    // По Лемме 140.L1: A^∞ accessible, итерации well-defined.
    transfinite_iteration_min(act, ε_math, A_infinity)
}

/// Согласованность с τ-truncations (Теорема 140.T Свойство 1).
@verify(formal)
theorem epsilon_truncation_compat<const N: usize>(act: ActOnInfinityInfinity)
    where act ∈ InfinityNCategory<N>
    ensures epsilon_infinity(act) == epsilon_at_level::<N>(τ_{≤N}(act))
;
```

### 4.3 Стек-модель compatibility

По Лемме 140.L_stack: $(\infty,\infty)$-категория совместима с κ-башней — каждый уровень $(∞, n)$ для $n ≤ 2$ помещается в $\mathbf{U}_2$; для $n ≥ 3$ через Drake stabilization (Лемма 131.L3).

### 4.4 Обязательные чтения для VVA-4

1. [/12-actic/06-actic-theorems §15.3](https://...) — **полное** доказательство 140.T (Шаги 1-5 + 5 лемм).
2. **Verity 2008** «Weak complicial sets I: Basic homotopy theory» — base reference.
3. **Riehl-Verity 2022** «Elements of ∞-Category Theory» §4 (для (∞,2)) + §10 (для (∞,n) general).
4. **Barwick-Schommer-Pries 2021** «On the unicity of the homotopy theory of higher categories» Theorem A — unicity $(\infty, n)$-categorical structure.
5. **Bergner-Rezk 2013** «Reedy categories and the Θ-construction» — accessibility для (∞,n).
6. **Lurie HTT** §A.3.4 Proposition A.3.4.13 — accessibility для (∞,1) base case.

### 4.5 Acceptance criteria

- `epsilon_infinity` корректно определена для всех 18 Актика-теорем 110.T–127.T.
- Согласованность $\nu^\infty(\alpha) = \mathsf{e}^\infty(\varepsilon^\infty(\alpha))$ верифицирована на корпусе.
- Тестовый случай: УГМ как $\alpha_\mathrm{uhm}$ имеет $\nu^\infty = \omega \cdot 3 + 1$ (см. /05-assemblies/01-uhm).

---

## 5. VVA-5 — Constructive autopoiesis runtime

**Diakrisis-основание**: Теорема 141.T ([/12-actic/06-actic-theorems §15.4](https://...)) + Теорема 121.T (BHK) + Лемма 141.L1 (higher-type computability в Eff).

### 5.1 Текущее состояние

VVA `core.action.verify` имеет `verify_autopoiesis(practice)` без конструктивной реализации. 141.T в VVA-baseline — только теорема существования.

### 5.2 Предложение

Добавить `runtime/eff_semantics_layer.rs` для higher-type computability:

```rust
// crates/verum_runtime/src/eff_semantics_layer.rs

/// Effective topos semantics layer для Verum runtime.
/// Реализует Hyland 1982 effective topos через modest sets.
pub struct EffSemanticsLayer {
    realizers: HashMap<TypeId, PartialRecursiveFunction>,
    higher_type_layer: HigherTypeComputability,
}

impl EffSemanticsLayer {
    /// ω-итерация через diagonal nesting.
    /// Лемма 141.L1: higher-type computability в Eff.
    pub fn omega_iterate<E: Enactment>(&self, eps: E) -> EffObject<E> {
        // r^(ω)(n, k) = r^(k)(n) для каждого k
        // Partial computable по Kleene normal form (Kleene 1952 §57)
        let r_omega = |n, k| self.iterate_finite(eps, k).realize(n);
        EffObject::partial_recursive(r_omega)
    }

    /// ω·n-итерация (для n ∈ ℕ).
    /// Реализована через primitive recursion в Eff modest sets.
    pub fn omega_n_iterate<E: Enactment>(&self, eps: E, n: usize) -> EffObject<E> {
        // r^(ω·n) — higher-order partial recursive function
        // Hyland 1982 §III.4 modest sets
        self.modest_higher_order_iterate(eps, n)
    }

    /// ω²-итерация — autopoietic fixed point.
    /// Construction: r^(ω²)(⟨p, n⟩, q) = r^(ω·n)(p, q)
    /// per Hyland-Ong-Robinson 1990 + van Oosten 2008
    pub fn omega_squared_iterate<E: Enactment>(&self, eps: E) -> EffObject<E> {
        // NOT Turing-computable in standard model;
        // requires Eff's higher-type semantics layer.
        let r_omega_squared = |p_n_q| {
            let (p, n) = decode_pair(p_n_q.0);
            self.omega_n_iterate(eps, n).realize(p, p_n_q.1)
        };
        EffObject::higher_type(r_omega_squared)
    }
}

/// Constructive σ/π morphisms per Theorem 141.T §15.4.4.
pub fn constructive_autopoiesis<E: Enactment>(
    eps_life: E,
    eff: &EffSemanticsLayer,
) -> AutopoieticWitness<E> {
    let eps_auto = eff.omega_squared_iterate(eps_life.clone());

    // σ: ε_auto → A(ε_auto) — canonical inclusion
    let sigma = canonical_inclusion(&eps_auto);

    // π: A(ε_auto) → ε_auto — retraction via Drake reflection
    // π(A(ε_auto)) := decode(r^{ω²}(code(A(ε_auto)))) per /12-actic/06-actic-theorems §15.4.4
    let pi = drake_reflection_retraction(&eps_auto, eff);

    // Lemma 141.L2: σ∘π and π∘σ are bisimilar via AFA-analogue in Eff
    // (Pavlovic 1995 + Aczel-Capretta 2017)
    AutopoieticWitness {
        eps_auto,
        sigma,
        pi,
        bisim_certificate: bisim_via_afa_eff(&sigma, &pi),
    }
}
```

### 5.3 BHK proof-extraction integration

```verum
// core/proof/bhk.vr

@framework(bhk_semantics_121T, "Theorem 121.T: BHK as ε-semantics")

type BHKConstruction<P: Prop> = {
    proof: P,
    witness: ConstructiveWitness<P>,  // Eff-realizable
}

/// Extract realizable witness from intuitionistic proof.
/// Per Lemma 121.L3: BHK ↔ Eff-realizability (Hyland 1982).
fn extract_witness<P: Prop>(p: Proof<P>) -> ConstructiveWitness<P>
    where P.is_intuitionistic()
    @verify(formal)
    ensures realizable_in_Eff(extract_witness(p))
;

/// Verum runtime executes BHK-witness as Eff-object.
/// Connects R7 (BHK) and R4 (autopoiesis): Eff layer is shared.
@enact(epsilon = "ε_prove")
fn intuitionistic_proof<P: Prop>(p: P) -> Proof<P> { /* ... */ }
```

### 5.4 Обязательные чтения для VVA-5

1. [/12-actic/06-actic-theorems §15.4](https://...) — **полное** доказательство 141.T.
2. [/12-actic/06-actic-theorems §12.1](https://...) — **полное** доказательство 121.T (особенно Лемма 121.L3 BHK ↔ Eff).
3. **Hyland 1982** «The effective topos» §III.4 «Modest sets» — Eff structure.
4. **Aczel 1988** «Non-well-founded sets» — AFA + bisimulation.
5. **Pavlovic 1995** «Maps I: relative to a factorisation system» + **Aczel-Capretta 2017** «Coalgebraic recursion in stably-locally-cartesian-closed categories» — AFA-аналог в Eff.
6. **van Oosten 2008** «Realizability: An Introduction to its Categorical Side» §3 — higher-type computability framework.
7. **Maturana-Varela 1980** «Autopoiesis and Cognition» — биологическая мотивация (для (Лемма 141.L3 closure under $\mathsf{A}$).

### 5.5 Acceptance criteria

- `omega_squared_iterate` корректно реализован для конкретных biological enactments.
- Drake reflection retraction $\pi$ построена явно через Gödel encoding (R4).
- `bisim_certificate` верифицируется через AFA-аналог.
- Finite approximation API: `approximate_autopoiesis(ε, depth)` для practical synthetic biology applications.

---

## 6. VVA-6 — Operational coherence checker

**Diakrisis-основание**: Теорема 18.T1 ([/09-applications/03-operational-coherence](https://...)) — финальный синтез всех R1-R11.

### 6.1 Текущее состояние

VVA §12 даёт 9-стратегийную ladder. **Отсутствует**: 10-я стратегия `@verify(coherent)` для двусторонней α/ε-coherence.

### 6.2 Предложение

Добавить 10-ю стратегию:

```verum
// core/verify/coherence.vr

@framework(operational_coherence_18T1, "Theorem 18.T1: α/ε coherence via 108.T")

/// 10-я стратегия: α/ε operational coherence.
/// Сложность: O(2^O(|P|+|φ|)) для finitely-axiomatized α-семантики.
@verify(coherent)
fn coherent_check<P: Program, φ: Property>(prog: P, prop: φ) -> bool {
    let alpha_cert = static_check(prog, prop);
    let phi_dual = T_108(prop);  // 108.T-bridge
    let epsilon_cert = runtime_monitor(execute(prog), phi_dual);
    alpha_cert == epsilon_cert
}

/// 108.T-bridge: канонический ε-translate свойства.
fn T_108(prop: AlphaProperty) -> EpsilonProperty {
    match prop.classification() {
        Intuitionistic => epsilon_via_BHK(prop),  // Theorem 121.T
        Classical => {
            // Gödel-Genzen translation + ε_LEM (Lemma 18.L_GG)
            let phi_int = negative_translation(prop);
            epsilon_compose(epsilon_via_BHK(phi_int), epsilon_LEM)
        }
        Modal => epsilon_via_md_omega(prop),  // Theorem 136.T
    }
}

/// Round-trip coherence theorem (formalized).
@verify(certified)
theorem round_trip_coherence<P, φ>(prog: P, prop: φ)
    where prog.is_finitely_axiomatized() && prog.runtime_terminates()
    ensures static_check(prog, prop) ⟺
            runtime_monitor(execute(prog), T_108(prop))
    proof_via Theorem 18.T1
;

/// Concurrent coherence (commutative effects only).
@verify(certified)
theorem concurrent_coherence<P1, P2>(p1: P1, p2: P2, prop: AlphaProperty)
    where (P1.effect_class().is_commutative() &&
           P2.effect_class().is_commutative())
    ensures coherent_check(parallel(p1, p2), prop) ⟺
            coherent_check(p1, prop) ∧ coherent_check(p2, prop)
    proof_via Theorem 17.C2 (concurrent correctness через ⊗-commutativity)
;

/// Weak-stratum coherence (полиномиальная сложность).
@verify(certified)
theorem weak_coherence<P, φ>(prog: P, prop: φ)
    where prog.alpha_semantics ∈ L_Fnd_weak  // bounded arithmetic
    ensures coherent_check(prog, prop) is_polynomial_time
    proof_via Theorem 18.T1_weak (R11 + R12 connection)
;
```

### 6.3 Ladder extension — three coherence sub-strategies

**Атака на one-size-fits-all `coherent`**: сложность $O(2^{O(|P|+|\phi|)})$ для произвольной программы — непрактично для production. Стратегия должна иметь fallback.

Решение: разделить `coherent` на три уровня:

| Strategy | Meaning | ν | Cost | Type |
|---|---|---|---|---|
| `runtime` | Runtime assertions | 0 | O(1) | existing |
| `static` | Conservative dataflow | 1 | Fast | existing |
| `fast` | Bounded SMT | 2 | ≤ 100 ms | existing |
| `complexity_typed` | Bounded-arithmetic verification | n < ω | Polynomial; CI budget ≤ 30 s | VVA-8 |
| `formal` | Full SMT portfolio | ω | ≤ 5 s; CI budget ≤ 60 s | existing |
| `proof` | User tactic proof | ω+1 | Unbounded; CI budget ≤ 5 min | existing |
| `thorough` | `formal` + invariants | ω·2 | 2×; CI budget ≤ 10 min | existing |
| `reliable` | Cross-solver agreement | ω·2+1 | Racing; CI budget ≤ 15 min | existing |
| `certified` | Certificate re-check | ω·2+2 | + recheck; CI budget ≤ 20 min | existing |
| **`coherent_static`** | **α-cert + symbolic ε-claim** | **ω·2 + 3** | **O(\|P\|·\|φ\|); ≤ 60 s** | **VVA-6 weak** |
| **`coherent_runtime`** | **α-cert + runtime ε-monitor** | **ω·2 + 4** | **O(\|trace\|·\|φ\|); ≤ 5 min** | **VVA-6 hybrid** |
| **`coherent`** | **α/ε bidirectional check** | **ω·2 + 5** | **O(2^O(\|P\|+\|φ\|)); ≤ 30 min** | **VVA-6 strict** |
| `synthesize` | Inverse proof search | ≤ω·3+1 | Unbounded; soft cap ≤ 60 min | existing (top of ladder) |

**Семантика трёх уровней**:
- `coherent` (full): полный bidirectional roundtrip check, single-exponential, для critical-safety code.
- `coherent_static`: только статическая проверка α + symbolic claim ε-сертификата (без runtime). Полиномиальная сложность.
- `coherent_runtime`: статическая α + runtime monitoring ε. Нет compile-time exponential blowup; runtime overhead зависит от длины trace.

**Grammar impact**: расширяет `verify_attribute` enum в `internal/verum/grammar/verum.ebnf:441-445` с 9 до 12 strategies.

`@verify(coherent)` (full) — самая сильная стратегия для critical-safety кода; `coherent_static` и `coherent_runtime` дают практичные fallback-ы для production.

### 6.4 Обязательные чтения для VVA-6

1. [/09-applications/03-operational-coherence](https://...) — **полное** доказательство 18.T1 (Шаги A-D + 3 подслучая).
2. [/09-applications/03-operational-coherence §3.1](https://...) — три случая ε-перевода (intuitionistic/classical/modal).
3. [/09-applications/03-operational-coherence §3.3](https://...) — 3 подслучая для concurrent (commutative/non-commutative/cut-elim).
4. [/09-applications/03-operational-coherence §7.4](https://...) — weak-stratum coherence (Теорема 18.T1_weak).
5. **Plotkin 1977** «LCF considered as a programming language» — operational vs denotational baseline.
6. **Abramsky-Jagadeesan-Malacaria 2000** «Full abstraction for PCF» — связь full abstraction + coherence.

### 6.5 Acceptance criteria

- `@verify(coherent)` работает на тестовом наборе из 50 программ (mixed intuitionistic/classical/modal/concurrent).
- УГМ-программа `live_by_uhm` (см. /12-actic/09-verum-stdlib-sketch §8) проходит coherent check с $\nu = \omega \cdot 3 + 1$.
- Quantum search программа проходит coherent check с probabilistic certificate.

---

## 7. VVA-7 — K-Refine-omega kernel rule

**Diakrisis-основание**: Теорема 136.T ([/02-canonical-primitive/02-axiomatics §T-2f\*\*\*](https://...)) + Definition 136.D1 (трансфинитный модальный язык $L^\omega_\alpha$).

### 7.1 Предложение

Расширить kernel rule `K-Refine` до `K-Refine-omega`:

```
  Γ ⊢ A : Type_n     Γ, x:A ⊢ P : Prop
  dp(P) < dp(A) + 1     md^ω(P) < md^ω(A) + 1
  ────────────────────────────────────────────────── (K-Refine-omega)
  Γ ⊢ { x:A | P } : Type_n
```

Где $\mathrm{md}^\omega$ — ординал-значный модальный ранг (по Definition 136.D1):
- $\mathrm{md}^\omega(\phi) = 0$ для атомарных.
- $\mathrm{md}^\omega(\Box\phi) = \mathrm{md}^\omega(\phi) + 1$.
- $\mathrm{md}^\omega(\bigwedge_{i<\kappa} P_i) = \sup_i \mathrm{md}^\omega(P_i)$.

Это блокирует **Berry, paradoxical Löb, paraconsistent Curry, Beth-Monk ω-iteration, и любые ω·k или $\omega^\omega$-модальные парадоксы** — расширение T-2f\* + T-2f\*\* до трансфинитных рангов.

### 7.2 Обязательные чтения для VVA-7

1. [/02-canonical-primitive/02-axiomatics §T-2f\*\*\*](https://...) — **полное** доказательство 136.T (Шаги 1-4 + 4 леммы).
2. **Smoryński 1985** §1 — modal depth для finite case.
3. **Boolos 1993** «The Logic of Provability» Ch.1 §1.4 — md в GL.
4. **Levy 1979** «Basic Set Theory» Ch.III §6 Theorem 1 — well-founded recursion на ординалах.
5. **Beklemishev 2004** «Provability algebras and proof-theoretic ordinals» — GLP, ω-rule preservation.

---

## 8. VVA-8 — Complexity-typed weak frameworks

**Diakrisis-основание**: Теорема 137.T ([/06-limits/05-what-remains-possible §137.T](https://...)) + 6-уровневая $\nu^\mathrm{weak}$-стратификация.

### 8.1 Предложение

Добавить `core.math.frameworks.bounded_arithmetic` с complexity-types:

```verum
@framework(bounded_arithmetic_137T, "Theorem 137.T: weak-AFN-T")

trait WeakRichS {
    fn nu_weak(&self) -> u32;  // < ω
    fn complexity_class(&self) -> ComplexityClass;
}

#[framework(I_Delta_0)] struct I_Delta_0;  // ν^weak = ω-1
#[framework(S_2_1)] struct S_2_1;  // ν^weak = 2 (P-time)
#[framework(V_0)] struct V_0;  // ν^weak = 1 (LOGSPACE)
#[framework(V_1)] struct V_1;  // ν^weak = 2 (P)
// ... etc

@verify(complexity_typed)
theorem weak_AFN_T()
    ensures L_Abs_weak == empty
    proof_via Theorem 137.T (bounded Cantor diagonal Buss 1986 §6.5)
;
```

### 8.2 Применение

Для криптографических протоколов, embedded systems, real-time verification — coherence-check работает за **полиномиальное время** (не экспоненциальное).

---

## 9. VVA-9 — Effect-system как Kleisli-категория

**Diakrisis-основание**: Теорема 17.T1 ([/03-formal-architecture/17-effects-and-linear](https://...)).

### 9.1 Предложение

Привязать Verum effect-system к Kleisli-категории strong monads:

```verum
@framework(effects_17T1, "Theorem 17.T1: effects as Perf(α_linear) projections")

trait Monad<T> {
    fn pure<A>(a: A) -> T<A>;
    fn bind<A, B>(m: T<A>, f: fn(A) -> T<B>) -> T<B>;
}

trait StrongMonad<T>: Monad<T> {
    fn strength<A, B>(a: A, m: T<B>) -> T<(A, B)>;
}

trait CommutativeStrongMonad<T>: StrongMonad<T> {
    // tensor commutativity: T(A) ⊗ T(B) ≃ T(B) ⊗ T(A)
}

// Каталог стандартных эффектов:
// Commutative: Reader, Writer (commut. monoid), List, Probability, Pure
// Non-commutative: State, IO (full), Exception
// Async: depends on synchronization model
// Concurrent: ⊗-commutative through parallel composition

#[derive(StrongMonad, Commutative)]
type Reader<R, A> = fn(R) -> A;

#[derive(StrongMonad)]  // not commutative
type State<S, A> = fn(S) -> (A, S);
```

### 9.2 Acceptance criteria

- VVA §11.2 effect annotations bind to Kleisli structure.
- Каждый effect-class имеет каноническую ε-координату через Лемму 17.L1.
- Concurrent correctness через ⊗-commutativity (только для commutative monads).

---

## 10. VVA-10 — Ludics-cells via complicial sets

**Diakrisis-основание**: Теорема 120.T ([/12-actic/06-actic-theorems §11.1](https://...)).

### 10.1 Предложение

Реализовать Ludics-семантику через стратифицированные комплициальные множества:

```verum
@framework(ludics_120T, "Theorem 120.T: Ludics ≃ Perf(α_linear)")

type Locus<A>;  // SMCC-position
type Design<L1: Locus, L2: Locus> = Strategy<L1, L2>;
type Dessein<D1, D2> = Modification<D1, D2>;

/// Cut-elimination — Lemma 120.L3 (canonical reduction).
/// Связь с VVA-2: cut-elim в Ludics = canonicalize в /03-formal-architecture/16-gauge-decision.
fn cut_elim<L1, L2>(d: Design<L1, L2>) -> Design<L1, L2>
    @verify(formal)
    ensures normal_form(cut_elim(d))
;

/// Orthogonality (Lemma 120.L4) — gauge-incompatibility.
fn orthogonal<L1, L2>(d1: Design<L1, L2>, d2: Design<L2, L1>) -> bool
    ensures gauge_incompatible(d1, d2)
;

@verify(certified)
theorem ludics_perf_equivalence()
    ensures Ludics ≃ Perf(α_linear)
    proof_via Theorem 120.T
;
```

### 10.2 Применение

- core/action/ludics.vr — основа для actor-model и π-calculus (Следствие 120.C1).
- Distributed systems verification: actor message-passing = ludics designs.

---

## 11. Архитектурный layered overview (предельная форма)

```
┌─────────────────────────────────────────────────────────────────────┐
│ Verum predельная архитектура (post-VVA-1..10)                       │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  USER LAYER:                                                        │
│  @verify(coherent) | @enact(epsilon=...) | @framework(...)          │
│  @effect(IO/State/Async/Concurrent) | @complexity(P-time)           │
│                                                                     │
├─────────────────────────────────────────────────────────────────────┤
│  ELABORATION LAYER:                                                 │
│  Three refinement forms → CoreTerm                                  │
│  Effect annotations → Kleisli-vложение (VVA-9)                      │
│  Coherence check → 18.T1-bridge (VVA-6)                             │
│                                                                     │
├─────────────────────────────────────────────────────────────────────┤
│  STDLIB LAYER:                                                      │
│  core.math.* (OC):      core.action.* (DC):                         │
│  ├ frameworks           ├ primitives                                │
│  ├ stack_model (VVA-3)  ├ enactments                                │
│  ├ infinity_category    ├ gauge / canonicalize (VVA-2)              │
│  │   (VVA-4 ∞,∞)        ├ verify (coherence VVA-6)                  │
│  ├ logic                ├ ludics (VVA-10)                           │
│  └ ...                  └ effects (VVA-9)                           │
│                                                                     │
│  core.theory_interop:                                               │
│  ├ load_theory (Yoneda)                                             │
│  ├ translate (Kan extension)                                        │
│  ├ check_coherence (Čech descent)                                   │
│  └ bridges/oc_dc_bridge (VVA-2 round-trip)                          │
│                                                                     │
│  core.proof:                                                        │
│  ├ tactics                                                          │
│  ├ smt (Z3+CVC5+E+Vampire)                                          │
│  ├ certificate (5 export formats)                                   │
│  └ bhk (VVA-5 BHK proof-extraction)                                 │
│                                                                     │
├─────────────────────────────────────────────────────────────────────┤
│  RUNTIME LAYER:                                                     │
│  Standard Turing semantics + Eff higher-type layer (VVA-5)          │
│  Effect handlers (commutative/non-commutative)                      │
│  Concurrent scheduler (fairness for non-commutative)                │
│                                                                     │
├─────────────────────────────────────────────────────────────────────┤
│  KERNEL (≤ 5 000 LOC):                                              │
│  CCHM cubical + refinement + framework axioms                       │
│  K-Refine (T-2f*) + K-Refine-omega (T-2f*** VVA-7)                  │
│  K-Adj-Unit + K-Adj-Counit (108.T duality)                          │
│  K-Eps-Mu (Predложение 5.1 VVA-1)                                   │
│  K-Round-Trip (Theorem 16.10 VVA-2)                                 │
│  Re-check certificates from all backends                            │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 12. Roadmap — VVA как continuation VVA Phase 6+

**Critical pre-condition**: VVA начинается **после** завершения VVA Phase 5 (см. [verification-architecture.md §16.5](./verification-architecture.md)). VVA расширяет VVA Phase 6 ("Full MSFS self-recognition"), не заменяет более ранние phases.

**Reconciliation table** (VVA → VVA):

| VVA Phase | VVA Phase | Months (from VVA T0) | Pre-conditions |
|---|---|---|---|
| Phase 1-5 | (none — VVA depends) | 0-24 | VVA baseline |
| **Phase 6** + **VVA-P1** | ε-arithmetic + K-Eps-Mu | 24-30 | VVA Phase 5 complete |
| **VVA-P2** | Round-trip 108.T + canonicalize | 30-36 | VVA-P1 |
| **VVA-P3** | Stack-model semantics + κ-tower + complexity-typed | 36-42 | VVA-P2 |
| **VVA-P4** | (∞,∞)-categorical support | 42-48 | VVA-P3 |
| **VVA-P5** | Eff layer + autopoiesis runtime + BHK extraction + ludics | 48-54 | VVA-P4 |
| **VVA-P6** | Operational coherence checker | 54-60 | VVA-P5 |

**Перевод сроков**: VVA — **5-летняя программа** после VVA stabilization (суммарно ~5 лет от VVA T0 до VVA-P6 completion). Реалистично для proof-assistant evolution.

Каждая фаза завершается публикацией:
- Working build с unit tests на 100% покрытие.
- Performance benchmarks vs Coq/Lean/Agda.
- Diakrisis-corpus verification: все 142 теоремы проходят соответствующие checks.
- **Update VVA** с новой stable version (VVA 1.5 + VVA-P1 → VVA 2.0).
- **Update [/12-actic/10-implementation-status](https://...)** на стороне Diakrisis (cross-ownership).

**Migration policy**:
- Каждое VVA-N — opt-in через `@require_extension(vfe_N)` annotation на module/file/project level.
- Без annotation — VVA-baseline mode (kernel rules не активны).
- After 2 years opt-in → kernel rules становятся default; old behavior — opt-out через `@disable_extension(vfe_N)`.
- After 4 years — old behavior removed; opt-out invalidated.

**Critical kernel rule rollout**:
- K-Eps-Mu (VVA-P1): low risk (additional check; не отвергает VVA-correct programs).
- K-Round-Trip (VVA-P2): medium risk (отвергает programs с broken α/ε naturality).
- K-Universe-Ascent (VVA-P3): high risk (требует universe-tagging для всех артикуляций).
- K-Refine-omega (VVA-P3): medium risk (отвергает modal paradoxes; но мало programs их используют).

---

## 13. Связь с MSFS и Diakrisis-корпусом — single-source policy

**Принципиальное правило** (по [/10-reference/05-corpus-correspondence](https://...)):

- **MSFS** (Sereda 2026, Zenodo DOI 10.5281/zenodo.19755781) — first-source для Theorem 108.T, AFN-T, five-axis absoluteness, three bypass paths, meta-classification (theorems 100.T-102.T), AC/OC duality (107.T-109.T).
- **Diakrisis** — расширения MSFS: канонический примитив, Актика-теоремы 110.T-127.T, доказательства максимальности 103.T-106.T (Diakrisis-only witness for Q1), 128.T-135.T residual closures, 136.T-142.T исследовательские расширения.

**Verum framework axioms**: каждый `@framework(name, citation)` должен иметь:
1. Имя в стандартизированном пространстве имён (`msfs_*`, `diakrisis_*`, `actic_*`).
2. Citation на authoritative source: MSFS для core; Diakrisis-документы для расширений.
3. ε-координата (если применимо) из [/12-actic/03-epsilon-invariant](https://...).
4. ν-координата из [/00-foundations/05-level-hierarchy](https://...).
5. **Согласованность с [/09-applications/02-canonical-nu-table](https://...)**: lookup-таблица Verum **обязательно** должна соответствовать ν/τ-значениям из Diakrisis-стороны. CLI `verum audit --coord` синхронизирует обе. Любое расхождение — ошибка implementation.

Пример корректной декларации:

```verum
@framework(
    name = "diakrisis_124T",
    citation = "Sereda 2026, MSFS Theorem 108.T + Diakrisis Theorem 124.T",
    source = "/12-actic/06-actic-theorems §14.2",
    nu = "ω·2",  // из [/00-foundations/05-level-hierarchy]
    epsilon = "ω·2",  // дуально по 108.T
    classification = "Diakrisis-only extension of MSFS"
)
public axiom adjunction_M_dashv_A: ...
```

---

## 13.5 Per-VVA engineering caveats (red-team-derived)

После second-round red-team следующие caveats явно фиксированы:

**VVA-1 (K-Eps-Mu) — decidability**:
- Полная τ-naturality проверка в общем случае — **полу-разрешимое** (Σ_1).
- Decidable для finitely-axiomatized articulations (как и round-trip 16.10).
- Compiler **не** требует full naturality check; проверяет canonicity τ через explicit data в `@framework` declarations.

**VVA-2 (round-trip) — fallback для не-finitely-axiomatized R-S**:
- `lurie_htt`, `schreiber_dcct` (ν=ω, ω+2) — **не** finitely-axiomatized.
- Для них round-trip — **semi-decidable** (Σ_1 per Theorem 16.6).
- Fallback: `@verify(coherent_static)` (без full round-trip), не `coherent`.
- CLI report: `verum audit --round-trip` явно показывает decidable / semi-decidable status per framework.

**VVA-3 (universe-tagging) — performance**:
- Universe metadata: 1 byte per `@framework` declaration (κ_1=0, κ_2=1, truncated=2).
- Type-check overhead: ≤ 5% (estimated; benchmark required Phase VVA-P3).
- Memory: ≤ 100 bytes per articulation (negligible).

**VVA-5 (Eff layer) — symbolic vs execution**:
- $\omega^2$-итерация в Eff — **symbolic** (proof-level), не runtime execution.
- `approximate_autopoiesis(ε, depth)` — **только** этот API runtime-executable; finite depth.
- Verum runtime **не** выполняет higher-type computability в production execution.
- Eff layer — это **denotational** semantic для verification, не operational runtime.

**VVA-7 (K-Refine-omega) — open formulas**:
- md^ω определён для closed terms (no free vars).
- Для open formulas: $\mathrm{md}^\omega(P(x_1, \ldots, x_n))$ определяется как $\mathrm{md}^\omega$ closure $\forall x_1 \ldots x_n. P$.
- Universal closure technique — стандартная (Smoryński 1985 §1.2).
- Compiler implementation: ~50 LOC additional для open-formula handling.

**VVA-9 (effects vs HoTT path types) — interaction**:
- Path types: HoTT-construct (cubical kernel).
- Effect monads: stdlib-construct (`core.action.effects`).
- **Взаимодействие**: effects на typed values, paths на types. Нет direct interaction; они в orthogonal планах.
- Edge case: `Path<T<A>, T<B>>` для effect monad T — это path в effect-typed values; обрабатывается через cubical operators (transp, hcomp).

**VVA-10 (Ludics) — infinite-branching**:
- Ludics designs могут быть infinite-branching trees.
- Verum implementation: **lazy evaluation** для design trees через cofix.
- Cut-elimination: bounded depth (configurable, default 1000 reduction steps); divergent computations — UNKNOWN verdict.
- Memory: ≤ 100 MB для typical proofs; может потребовать optimization для UHM-scale (223 теоремы).

---

## 14. Open design questions

### 14.1 Probabilistic coherence

Расширение VVA-6 (`@verify(coherent)`) на probabilistic programs через Giry monad. Требует формализации R6 для probabilistic effects.

### 14.2 Distributed coherence

Federated Verum-systems с разными α-семантиками на разных нодах. Требует расширения 108.T до multi-base case.

### 14.3 Automatic ε-inference

Может ли Verum автоматически выводить $\phi^\sharp$ из $\phi$ без явного `T_108`-call? Связано с Теоремой 16.6 (полу-разрешимость).

### 14.4 Quantum effect class

α_quantum = α_linear + †-compact (Abramsky-Coecke 2004). Probabilistic + unitary + measurement как effect class. Требует расширения каталога эффектов VVA-9.

### 14.5 Verum kernel size после VVA-1..10

Текущий target: ≤ 5 000 LOC. После VVA-1, VVA-2, VVA-7 добавится:
- K-Eps-Mu: ~300 LOC
- K-Round-Trip: ~500 LOC (canonicalize is hardest)
- K-Refine-omega: ~400 LOC
- Total addition: ~1 200 LOC

Новый target после VVA: ≤ 6 500 LOC kernel. Остальное (Eff layer, Ludics, effects) — вне kernel, в stdlib + runtime.

---

## 15. Success criteria для VVA как целого

После завершения VVA-1..10:

1. **Diakrisis 142 теоремы**: все формализованы как `@framework` axioms с явными ε/ν координатами.
2. **Round-trip verification**: 132 OC + 21 AC теоремы проходят round-trip за O(2^O(|α|)) wallclock.
3. **Coherence verification**: тестовый набор из 100 mixed-paradigm programs (intuitionistic/classical/modal/concurrent/quantum) проходит `@verify(coherent)`.
4. **(∞,∞) support**: УГМ-программа с $\nu = \omega \cdot 3 + 1$ верифицируется в полной (∞,∞)-семантике.
5. **Constructive autopoiesis**: synthetic biology примеры (genetic networks, metabolic cycles) верифицируются через `verify_autopoiesis(ε)` за конечное приближение.
6. **Weak-stratum**: cryptographic protocols верифицируются за полиномиальное время.
7. **Cross-assistant export**: каждое из 5 целевых систем (Lean / Coq / Agda / Dedukti / Metamath) принимает Verum-сертификаты.

---

## 16. References — Diakrisis paths summary

**Ядро (must-read)**:
- [/12-actic/04-ac-oc-duality](https://...) — 108.T + Предложение 5.1 (Sect §5)
- [/12-actic/06-actic-theorems §14.2](https://...) — Теорема 124.T (M⊣A biadjunction)
- [/02-canonical-primitive/02-axiomatics §131.T](https://...) — стек-модель
- [/03-formal-architecture/16-gauge-decision](https://...) — round-trip 108.T (Теорема 16.10)
- [/09-applications/03-operational-coherence](https://...) — Теорема 18.T1

**Расширения**:
- [/12-actic/06-actic-theorems §15.3](https://...) — Теорема 140.T ((∞,∞))
- [/12-actic/06-actic-theorems §15.4](https://...) — Теорема 141.T (autopoiesis)
- [/02-canonical-primitive/02-axiomatics §T-2f\*\*\*](https://...) — Теорема 136.T
- [/06-limits/05-what-remains-possible §137.T](https://...) — Теорема 137.T (weak)

**Операциональные мосты**:
- [/12-actic/06-actic-theorems §11.1](https://...) — Теорема 120.T (Ludics)
- [/12-actic/06-actic-theorems §12.1](https://...) — Теорема 121.T (BHK)
- [/03-formal-architecture/17-effects-and-linear](https://...) — Теорема 17.T1 (effects)

**Контекст**:
- [/10-reference/04-afn-t-correspondence](https://...) — карта Diakrisis ↔ MSFS
- [/10-reference/05-corpus-correspondence](https://...) — single-source policy
- [/10-reference/02-theorems-catalog](https://...) — полный каталог 1.T-142.T

**Прикладной контекст (`/09-applications/`)**:
- [/09-applications/00-path-B-uhm-formalization](https://...) — главная прикладная программа: 223 теоремы УГМ → Verum
- [/09-applications/01-verum-integration](https://...) — формальный contract Diakrisis ↔ Verum
- [/09-applications/02-canonical-nu-table](https://...) — **прескриптивная ν/τ-таблица для Verum lookup**
- [/09-applications/03-operational-coherence](https://...) — Теорема 18.T1 + operational coherence

**Diakrisis-стороннние спецификации для Verum (`/12-actic/`)**:
- [/12-actic/09-verum-stdlib-sketch](https://...) — **активный 10-step interaction plan** + sketch кода `core.action.*`
- [/12-actic/10-implementation-status](https://...) — **live tracker** интеграции (фазы α/β/γ/δ)
- [/12-actic/00-foundations](https://...) — обзор Актика
- [/12-actic/01-historical-lineage](https://...) — 35+ ДЦ-традиций
- [/12-actic/02-dual-primitive](https://...) — формальный примитив Актика (13 аксиом)
- [/12-actic/03-epsilon-invariant](https://...) — **полный каталог ε-координат**
- [/12-actic/05-dual-afn-t](https://...) — Теорема 109.T (Dual-AFN-T)
- [/12-actic/07-beyond-metastemology](https://...) — Метастемология Чурилова (Теорема 125.T)
- [/12-actic/08-formal-logical-dc](https://...) — **7 формально-логических ДЦ-направлений** (BHK, MLTT, Лоренцен, Hintikka, Ludics, Curry-Howard, concurrency)

**Внешние источники**:
- Sereda 2026, MSFS preprint, Zenodo DOI 10.5281/zenodo.19755781
- Hyland 1982 «The effective topos»
- Lurie HTT (Higher Topos Theory) + HA (Higher Algebra)
- Riehl-Verity 2022 «Elements of ∞-Category Theory»
- Verity 2008 «Weak complicial sets»
- Barwick-Schommer-Pries 2021 «On the unicity of the homotopy theory of higher categories»
- Aczel 1988 «Non-well-founded sets»
- Aczel-Capretta 2017 «Coalgebraic recursion in stably-locally-cartesian-closed categories»
- Faggian-Hyland 2002 «Designs, disputes and strategies»
- Moggi 1991 «Notions of computation and monads»
- Plotkin-Power 2002 «Algebraic operations and generic effects»

---

## 17. Grammar impact analysis (`internal/verum/grammar/verum.ebnf`)

VVA требует **минимальных** изменений в grammar (Verum.ebnf v3.0, 2513 LOC). Большинство расширений работают через существующие generic-attribute mechanisms.

### 17.1 Changes к `verify_attribute` (lines 441-445)

**Текущее**:
```ebnf
verify_attribute = 'verify' , '(' ,
    ( 'runtime' | 'static' | 'formal' | 'proof'
    | 'fast' | 'thorough' | 'reliable'
    | 'certified' | 'synthesize' ) ,
    ')' ;
```

**После VVA-6**:
```ebnf
verify_attribute = 'verify' , '(' ,
    ( 'runtime' | 'static' | 'formal' | 'proof'
    | 'fast' | 'thorough' | 'reliable'
    | 'certified' | 'synthesize'
    | 'coherent' | 'coherent_static' | 'coherent_runtime'
    | 'complexity_typed' ) ,                                  (* VVA-8 *)
    ')' ;
```

**Дополнительные strategy IDs** (5 новых):
- `coherent` — VVA-6 full bidirectional α/ε check.
- `coherent_static` — VVA-6 weak (полиномиальная).
- `coherent_runtime` — VVA-6 hybrid.
- `complexity_typed` — VVA-8 weak-stratum verification (полиномиальная для bounded arithmetic).

ν-coordinates (для monotone-lift checks):
```
runtime (0) < static (1) < fast (2) < complexity_typed (ω) < formal (ω) <
proof (ω+1) < thorough (ω·2) < reliable (ω·2+1) < certified (ω·2+2) <
coherent_static (ω·3) ≈ coherent_runtime (ω·3) < coherent (ω·3) ≤
synthesize (≤ω·3+1)
```

### 17.2 No grammar changes for `@framework`, `@enact`, `@effect`

Grammar §2.1 lines 391-402 даёт generic `attribute = identifier , [ '(' , attribute_args , ')' ]`. Это покрывает:
- `@framework(name = "...", citation = "...")` — already works.
- `@enact(epsilon = "ε_prove")` — already works.
- `@effect(IO)`, `@effect(Reader<R>)` — already works.
- `@enact(epsilon = "omega_3_plus_1")` — already works.

**Никаких изменений в grammar для VVA-1, VVA-2, VVA-9, VVA-10 не требуется** — все annotation-based features используют generic attribute mechanism.

### 17.3 Optional enhancement: `epsilon` ordinal literals

VVA-1, VVA-9 используют ε-coordinates как строки (`"omega"`, `"omega_3_plus_1"`). Можно добавить first-class ordinal literals:

```ebnf
ordinal_literal = ordinal_atom , { ordinal_op , ordinal_atom } ;
ordinal_atom    = 'ω' | 'omega' | digit_sequence | 'Ω' ;
ordinal_op      = '+' | '·' | '^' ;
```

Это позволит `@enact(epsilon = ω·3 + 1)` вместо `@enact(epsilon = "omega_3_plus_1")`. **Не обязательно** — string-based form работает.

### 17.4 K-Refine-omega — kernel only, не grammar

VVA-7 расширяет K-Refine kernel rule (внутри `verum_kernel`), но **не меняет grammar**: refinement syntax `T{predicate}` остаётся прежним. Изменяется только проверка md^ω-ranks при elaboration.

### 17.5 `@cohesive`, `@modal` — optional pragmas

Для VVA-7 (расширения трансфинитной модальной стратификации) можно добавить optional pragmas через generic attribute:
- `@modal(box_depth = 3)` — annotation для modal predicates.
- `@cohesive` — для cohesive-modality-aware verification.

Опять же, **никаких grammar changes** — generic attribute mechanism достаточен.

### 17.6 Сводная таблица grammar impact

| VVA | Grammar изменения | Generic attr | Kernel only |
|---|---|---|---|
| VVA-1 (ε-arithmetic) | — | `@enact(epsilon=...)` | `K-Eps-Mu` |
| VVA-2 (round-trip) | — | `@verify(formal)` | `K-Round-Trip` |
| VVA-3 (stack model) | — | `@framework(...)` | `K-Universe-Ascent` |
| VVA-4 ((∞,∞)) | — | `@framework(...)` | (∞,∞)-typing extension |
| VVA-5 (autopoiesis) | — | `@enact(epsilon="ω²")` | (Eff layer in runtime, not kernel) |
| **VVA-6 (coherent)** | **+3 strategy IDs** | — | `K-Coherence-Bridge` |
| VVA-7 (md^ω) | — | `@modal(...)` optional | `K-Refine-omega` |
| **VVA-8 (complexity)** | **+1 strategy ID** | `@framework(bounded_*)` | (none — stdlib-level) |
| VVA-9 (effects) | — | `@effect(...)` | (none — stdlib-level) |
| VVA-10 (Ludics) | — | `@framework(ludics)` | (none — stdlib-level) |

**Итого grammar изменений**: 4 новых strategy IDs в `verify_attribute` enum (lines 441-445). Всё остальное работает через existing generic attribute infrastructure.

---

## 18. Архитектурное масштабирование

### 18.1 Distributed Verum через Federation

**Идея**: расширить VVA до federated network of Verum nodes с разными α-семантиками, объединённых через 108.T-bridges.

```verum
@framework(verum_federation_protocol_v1, "VFP/1.0")

trait VerumNode {
    fn alpha_semantics(&self) -> Articulation;
    fn export_certificate(&self, theorem: Theorem) -> Certificate;
    fn import_certificate(&self, cert: Certificate, source: Articulation) -> Result<Theorem>;
}

@verify(certified)
fn cross_node_verification(
    source_node: VerumNode,
    target_node: VerumNode,
    theorem: Theorem,
) -> Result<Theorem> {
    // 1. Source generates certificate in its α-semantics.
    let source_cert = source_node.export_certificate(theorem);

    // 2. Translate via 108.T-bridge.
    let translation_path = find_translation_path(
        source_node.alpha_semantics(),
        target_node.alpha_semantics(),
    )?;

    // 3. Target re-checks under its α-semantics.
    let translated = translation_path.translate(source_cert)?;
    target_node.import_certificate(translated, source_node.alpha_semantics())
}
```

**Применение**: distributed proofs across institutions (Stanford-Lean + MIT-Coq + IAS-Diakrisis), каждое со своей α-семантикой; 108.T-bridge gives cross-verification.

**Связано с** VVA-2 (round-trip 16.10) + VVA-6 (coherence). Требует extension multi-base 108.T (open question /09-applications/03-operational-coherence §7.2).

### 18.2 ML-augmented synthesis

**Идея**: расширить `@verify(synthesize)` через ML-guided proof search для conjecture mining + automatic ε-coordinate inference.

```verum
@framework(ml_proof_search_v1, "ML-augmented synthesis")
@enact(epsilon = "ω·2 + 1")  // tradition-level practice
fn ml_guided_synthesis(goal: Property) -> Option<Proof> {
    // 1. ML model predicts likely tactic sequences.
    let candidates = ml_model.predict_tactics(goal, context = available_lemmas);

    // 2. Try candidates with @verify(formal) backend.
    for tactic_seq in candidates {
        if let Ok(proof) = try_proof(goal, tactic_seq) {
            return Some(proof);
        }
    }

    // 3. Fallback to classical search.
    classical_synthesize(goal)
}
```

**Применение**: автоматическое доказательство routine theorems в Diakrisis-style corpus.

### 18.3 Real-time coherence для embedded systems

**Идея**: weak-stratum coherence (VVA-8) для **embedded / IoT / real-time** systems.

```verum
@framework(realtime_coherence_v1, "Real-time coherent verification")
@verify(complexity_typed)  // полиномиальная (P-time)
fn realtime_critical<P: Program>(prog: P, deadline: Duration) -> Result<()>
    where prog.alpha_semantics() ∈ L_Fnd_weak  // bounded arithmetic
{
    // Compile-time: проверка спецификации в bounded arithmetic.
    // Runtime: ε-monitor с deadline gauntee.
    // Coherence: полиномиальная по worst-case execution.
}
```

**Применение**: cryptographic protocols, control systems, safety-critical embedded.

### 18.4 Quantum effect class

**Идея**: формализовать $\alpha_\mathrm{quantum}$ как first-class effect для quantum programming.

```verum
@framework(alpha_quantum, "Quantum: SMCC + †-compact (Abramsky-Coecke 2004)")

#[derive(StrongMonad, Commutative)]  // commutative для tensor
type Quantum<A> = QuantumState<A>;

@effect(Quantum)
@enact(epsilon = "ω")
fn quantum_search<n: Nat>(oracle: Oracle<n>) -> Distr<Output<n>> {
    let qs = init_qubits(n);
    apply_hadamard(qs);
    for _ in 0..sqrt(2_u64.pow(n)) {
        oracle(qs);
        grover_diffusion(qs);
    }
    measure(qs)  // returns probability distribution
}

@verify(coherent)
theorem grover_correctness<n: Nat>(oracle: Oracle<n>)
    ensures probability_of_correct(quantum_search(oracle)) >= 1 - 1/n
    proof_via Theorem 18.T1 + Probability monad commutativity
;
```

**Применение**: verified quantum algorithms (Grover, Shor, quantum walks).

### 18.5 Probabilistic coherence

**Идея**: расширить VVA-6 на probabilistic programs через Giry monad.

```verum
@framework(giry_monad_v1, "Giry monad for measurable spaces")

#[derive(StrongMonad, Commutative)]
type Prob<A> = ProbDistr<A>;

@verify(coherent)
theorem probabilistic_correctness<P: ProbabilisticProgram>(p: P)
    ensures expected_correct(p) >= threshold
    proof_via Theorem 18.T1 (Probability monad case)
;
```

**Применение**: verified machine learning, randomized algorithms, probabilistic protocols.

### 18.6 Cohesive type theory как first-class framework

**Идея**: расширить cohesive types (Schreiber DCCT) до polymorphic effect через cohesive modalities.

```verum
@framework(schreiber_dcct, "Differential Cohesive Homotopy Type Theory")

// Cohesive modalities
trait Cohesive {
    fn shape(self) -> Shape<Self>;       // ʃ
    fn flat(self) -> Flat<Self>;         // ♭
    fn sharp(self) -> Sharp<Self>;       // ♯
}

// ʃ ⊣ ♭ ⊣ ♯ — cohesive adjoint triple
@verify(coherent)
theorem cohesive_adjoint_triple<X: CohesiveSpace>()
    ensures shape ⊣ flat ⊣ sharp on X
    proof_via Theorem 124.T (cohesive monad as M⊣A instance)
;
```

**Применение**: differential geometry, smooth manifolds, gauge theory.

### 18.7 Архитектурный horizon: Verum как foundation marketplace

**Долгосрочная идея**: Verum становится **marketplace оснований**, где разные R-S-метатеории доступны как plug-in modules.

```
Verum-Foundation-Marketplace:
├── Standard frameworks (built-in)
│   ├── ZFC, HoTT, MLTT, CIC, NCG, ∞-topos, cohesive
├── Verified extensions (community)
│   ├── stack_model (Diakrisis 131.T)
│   ├── (∞,∞) (140.T)
│   ├── bounded_arithmetic_v0 (V^0)
│   ├── bounded_arithmetic_s2 (Buss S_2^1)
│   ├── linear_logic + ludics (120.T)
│   ├── giry_probability
│   ├── α_quantum (Abramsky-Coecke)
└── Experimental (research)
    ├── paraconsistent (LP, K3)
    ├── relevance_logic
    ├── intuitionistic_modal
```

Каждый framework — отдельный package с:
- `@framework` declaration с unique name + citation.
- ε/ν-координаты.
- Verified property tests.
- Cross-translation maps к standard frameworks.
- Acceptance criteria для inclusion в standard library.

**Это превращает Verum из proof assistant в инфраструктуру foundational pluralism** — реализация MSFS-видения как практической инженерной системы.

### 18.8 Reality check — feasibility ladder

Не все идеи 18.1-18.7 одинаково реализуемы. Делю на 3 tier по feasibility:

**Tier A (high feasibility, 1-2 года)**:
- 18.4 Quantum effect class — стандартная категорная семантика, можно сделать.
- 18.5 Probabilistic coherence — Giry monad стандартен.
- 18.3 Real-time coherence — комбинация VVA-8 + VVA-6, эксплуатация уже доказанной weak-coherence.

**Tier B (medium, 2-4 года)**:
- 18.2 ML-augmented synthesis — требует training data + integration с existing tactics.
- 18.6 Cohesive type theory — Schreiber DCCT хорошо известен, но интеграция с Verum kernel сложна.

**Tier C (long-horizon, 5+ лет)**:
- 18.1 Distributed Verum — federation protocol — research challenge.
- 18.7 Foundation marketplace — требует community + ecosystem maturity.

Прагматичный план: сначала Tier A (закрепить closed teorems), затем Tier B (добавить practical features), Tier C — после стабилизации экосистемы.

### 18.9 Radikal extensions VVA+ (10-летний горизонт)

После завершения VVA-1..10 + 18.1-18.7 архитектура Verum достигает уровня, при котором становятся возможны **системные расширения**, выходящие за рамки proof assistant в смежные домены.

#### VVA+1: Proof Network Federation Protocol (PNFP)

**Концепция**: Verum-сеть как distributed system для cross-institutional verification.

```
University A (Lean-foundation) ──┐
                                 │
University B (Coq-foundation) ───┼─── Verum Federation Hub ─── Verified Knowledge Pool
                                 │
University C (Diakrisis-stack) ──┘
```

Каждый node:
- Поддерживает свою α-семантику.
- Экспортирует certificates через 108.T-bridge.
- Проверяет import-сертификаты через round-trip 16.10.

**Технологический impact**: первая **формально верифицированная** distributed scientific infrastructure. Каждое published math result в federation — globally cross-checkable.

**Прецедент**: Lean's mathlib как centralized; Verum federation — decentralized version.

#### VVA+2: AI co-author с verification-integrity

**Концепция**: AI agent как proof author, Verum как verification gatekeeper.

```
LLM Agent ──proposes proof──> Verum kernel
                                   │
                                   ├─► reject (with explanation)
                                   └─► accept (with certificate)
                                          │
                                          ├─► export to mathlib
                                          └─► add to corpus
```

Critical safety property: **AI не может обойти kernel** (LCF discipline + certificate recheck из VVA §2.5). AI generates **suggestions**, kernel **decides**.

**Применение**: автоматизация Pathway-B УГМ (223 теоремы → AI-генерированные proofs → Verum verification → mathlib publication).

#### VVA+3: Verum-based AI alignment

**Концепция**: Использовать ε-координаты для AI safety constraints.

Каждое AI-action в production system аннотируется `@enact(epsilon = ...)`. Через VVA-6 coherence checker:

```verum
@enact(epsilon = "ω")  // atomic decision
@verify(coherent)
fn ai_decide(input: Context) -> Decision {
    // ...
}

@enact(epsilon = "ω·2")  // tradition-level (e.g., medical protocol)
@verify(coherent_runtime)
fn ai_treatment_recommendation(patient: PatientData) -> Treatment {
    // ...
}

@enact(epsilon = "ω²")  // institutional (e.g., self-modification)
@verify(coherent)  // strict — must be bidirectionally checked
@require_extension(vfe_6)
fn ai_self_modify(plan: ModificationPlan) -> Result<NewWeights> {
    // ...
}
```

**Result**: AI system has **mathematically verified** ε-bound. Never crosses ε-threshold without explicit human approval.

**Применение**: AGI alignment, autonomous systems safety, medical AI.

#### VVA+4: Cross-disciplinary verified knowledge

**Концепция**: Verum + Noesis (см. /11-noesis/ в Diakrisis) = formally verified knowledge graph across disciplines.

```
Math knowledge ─── Verum (Diakrisis) ─── Physics knowledge
       │                                          │
       └──── Bridges via Theorem 108.T ─────────┘
                       │
              Biology knowledge
                       │
              Economics knowledge
```

Each fact в graph has:
- α-семантика (which foundation it lives in).
- ε-координата (operational level).
- ν-координата (depth).
- Verified bridges to other domains.

**Применение**: scientific knowledge management, evidence-based policy, integrative research.

#### VVA+5: Verum as theory-design tool

**Концепция**: Не только verify existing theories, но и **explore design space** через round-trip + coherence.

Workflow:
1. Designer specifies new α-семантика partially (axioms + intended models).
2. Verum runs `coherent` checks against known α (ZFC, HoTT, NCG).
3. Verum reports: «Your candidate α is Morita-equivalent to NCG with extension X. Or it has obstruction Y at depth ν=ω+1.»
4. Designer iterates.

**Применение**: research mathematics, foundation engineering, exploring неклассические логики.

**Прецедент**: Lean's `decide` tactic as theory-design feedback. VVA+5 — масштабирование на foundation-level.

#### Feasibility assessment для VVA+

| Idea | Feasibility | Decade |
|---|---|---|
| VVA+1 PNFP | Medium | 2030s |
| VVA+2 AI co-author | High (already happening with LLMs) | Late 2020s |
| VVA+3 AI alignment | Medium | 2030s |
| VVA+4 Cross-disciplinary | Long-term | 2040s |
| VVA+5 Theory-design | High (extension of existing tactics) | Late 2020s |

### 18.10 Rejected ideas (антипаттерны)

Чтобы документ был честным, явно отмечаю идеи, которые **не** включены, и почему:

- **❌ "Universal kernel for all foundations"**: невозможно по AFN-T (нет уровня 6). Verum правильно остаётся foundation-neutral host.
- **❌ "Auto-verify everything via ML"**: нарушает LCF принцип (kernel re-check). ML может предлагать tactic, не writing certificate.
- **❌ "Native integration with each Tier 1 system (Lean/Coq/Agda)"**: certificate export — да, native integration — нет (это превратит Verum в meta-system, не proof assistant).
- **❌ "GPT-style natural language proofs"**: Verum работает на формальном уровне; NL — UI/UX layer, не core capability.
- **❌ "Quantum proof acceleration"**: spectulative; сейчас quantum computers не подходят для proof search.

---

## Appendix A — Соответствие VVA ↔ VVA

| VVA section | VVA расширение | Diakrisis |
|---|---|---|
| §2.4 K-Refine (T-2f*) | VVA-7: K-Refine-omega (T-2f***) | 136.T |
| §6 @framework axioms | VVA-3: stack_model integration | 131.T |
| §10 core.theory_interop | VVA-2: oc_dc_bridge round-trip | 16.10 |
| §11.2 core.action.* | VVA-9: effects as Kleisli + VVA-10: ludics | 17.T1 + 120.T |
| §12 verification ladder (9 strategies) | VVA-6: 10-я стратегия @verify(coherent) | 18.T1 |
| §13 articulation hygiene | (без изменений) | NO-19 |
| §15.4 AOT/Interpreter tiers | VVA-5: + Eff higher-type layer | 121.T + 141.T |
| §16 Migration Phase 1-6 | VVA Phase P1-P6 | (cumulative) |

---

## Appendix B — Чек-лист для разработчика, начинающего работу над VVA-N

Прежде чем коммитить **любую** строку кода для VVA-N:

**Theoretical foundation (Diakrisis Tier 0/0+/1/2/3)**:
- [ ] Прочитан полностью соответствующий Diakrisis-документ (см. секцию N.4 выше).
- [ ] Понята связь с MSFS-первоисточником (если применимо).
- [ ] Understood ε/ν-координаты involved.
- [ ] Проверены все cross-references в [/10-reference/05-corpus-correspondence](https://...).

**Applied context (`/09-applications/`)**:
- [ ] Прочитан **полностью** [/09-applications/00-path-B-uhm-formalization](https://...) — понимание главной прикладной миссии Verum (формализация 223 теорем УГМ).
- [ ] Прочитан [/09-applications/01-verum-integration](https://...) — Diakrisis-сторонние ожидания от Verum (формальный contract).
- [ ] Проверена синхронизация с [/09-applications/02-canonical-nu-table](https://...) — все новые `@framework` соответствуют прескриптивной ν/τ-таблице.
- [ ] Прочитан [/09-applications/03-operational-coherence](https://...) — operational coherence как conceptual frame.

**Diakrisis-стороннние спецификации для Verum (`/12-actic/09–10`)**:
- [ ] Прочитан **полностью** [/12-actic/09-verum-stdlib-sketch](https://...) — 10-step plan и sketch кода `core.action.*`.
- [ ] Сверен с [/12-actic/10-implementation-status](https://...) — текущий статус соответствующих Шагов 1-10.
- [ ] При завершении milestone — **обновлён** /12-actic/10-implementation-status с новым прогрессом (cross-ownership с Diakrisis).
- [ ] Все новые `@enact(epsilon=...)` annotations соответствуют каталогу [/12-actic/03-epsilon-invariant](https://...).

**Implementation**:
- [ ] Verified that proposed implementation satisfies acceptance criteria (секция N.5 / N.6).
- [ ] Grammar impact analysis (если затрагивает `verum.ebnf`) — see §17.
- [ ] Обсуждено с теоретическим reviewer'ом, что нет рассогласований с Diakrisis-корпусом.
- [ ] Обновлён `core.math.frameworks.diakrisis_*` каталог с новыми `@framework` декларациями.
- [ ] Lookup-таблица Verum синхронизирована с `/09-applications/02-canonical-nu-table`.
- [ ] CI-тест добавлен для каждого нового `@verify(...)` strategy.

**Path-B alignment**:
- [ ] Implementation supports formalization of УГМ teorema (К-Б-1: 223 theorems).
- [ ] Не нарушает relative consistency (К-Б-2: ZFC + 2-inacc baseline).
- [ ] Reductions explicit (К-Б-3: no hidden reductions per principle П-0.6).
- [ ] Specific novelty preserved (К-Б-4: e.g., quantum semantics, Lindblad ℒ_Ω).
- [ ] Concrete predictions verifiable (К-Б-5: P_crit, Φ_th, R_th, D_min, SAD_MAX).

---

*Документ — живой; обновляется при каждом новом расширении Diakrisis-корпуса.*
*Последнее обновление: после закрытия VVA foundational tasks 1-12.*
*Curator: theoretical foundations team в координации с Verum kernel team.*
