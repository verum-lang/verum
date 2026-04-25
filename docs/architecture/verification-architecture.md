# Verum Unified Verification Architecture (VUVA)

*Authoritative architectural specification for Verum's verification, dependent-types, formal-proofs, and meta-system layers. Synthesised from: the MSFS / Verum-MSFS preprints (Sereda 2026), the Diakrisis canonical primitive + theorem corpus (96 base theorems + 18 NO-results + maximality theorems 103.T–106.T establishing $\mathrm{Diakrisis} \in \mathcal{L}_{\mathrm{Cls}}^{\top}$ + Dual-AFN-T 109.T) and its `09-applications/01-verum-integration.md` + `11-noesis/*` integration plan, and the existing Verum specs 03/09/12/13/17.*

*This document is the single source of truth for all subsequent work on `verum_types::refinement`, `verum_smt`, `verum_kernel`, `verum_verification`, `core.verify`, `core.proof`, `core.math.*`, `core.action.*`, and `core.theory_interop`. All four legacy specification documents (09, 12, 13, 17) remain descriptive; when they conflict with VUVA, VUVA wins.*

---

## 0. Executive Summary

**Thesis.** Every existing proof assistant (Coq, Lean, Agda, Isabelle, F★, Dafny, Liquid Haskell, Mizar, Metamath, Idris) wires a single Rich-foundation — one R-S point in the classifying 2-stack $\mathfrak{M}$ — into its kernel. Verum is architected as a **foundation-neutral host for $\mathfrak{M}$** itself: foundations are first-class data attached through `@framework(name, "citation")`, reductions between foundations are explicit stdlib functors, and the kernel enforces exactly one load-bearing rule imported from Diakrisis: **T-2f\* depth-stratified comprehension** (realised as the `K-Refine` typing rule). On top of that minimal kernel Verum supports the nine-strategy `@verify(...)` ladder, three equivalent refinement forms (inline / declarative / sigma-type), full dependent types (Π, Σ, Path, HITs, quotients, quantitative modalities), a staged meta-system, and a dual OC/DC stdlib that lets an $\mathcal{E}$-enactment view co-exist with every articulation. Every discharge strategy that descends to SMT returns a CoreTerm certificate; the kernel re-checks that certificate and never trusts the solver. Cross-assistant export to Lean / Coq / Agda / Dedukti / Metamath is a first-class capability, not an afterthought.

**Differentiators vs Coq / Lean / Agda / Isabelle / F★ / Dafny / Liquid Haskell.**

| Capability | Coq | Lean 4 | Agda | Isabelle | F★ | Dafny | Liquid H | **Verum (VUVA)** |
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
- **Backward compatibility**: the three refinement forms in `03-type-system.md` (inline `T{pred}`, declarative `T where name`, sigma `x:T where P(x)`) remain the canonical surface for users. VUVA extends them into the dependent / proof world without breaking the simple-case ergonomics.
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

Grammar (`grammar/verum.ebnf` already supports these; VUVA pins the semantics):

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

This is the single rule that distinguishes Verum's kernel from a plain CCHM + refinements engine.

### 2.5 Certificate-based trust (LCF pattern)

**Target architecture** (end-state, not current reality): every tactic, SMT backend, elaborator output, or external proof is coerced to CoreTerm before the kernel accepts it. The trusted computing base:

- `verum_kernel` — the CoreTerm type checker with `K-Refine`, strict normalisation, universe-consistency, positivity check. Target ≤ 5 000 LOC (see §17 Q8 for current budget).
- Nothing else. `verum_smt`, `verum_tactics`, `verum_elaborator`, `verum_certificate` all emit CoreTerms and let the kernel re-check.

**Current state (Phase 1 baseline)**: `verum_types::refinement` discharges refinements outside the kernel and trusts the SMT solver's Sat/Unsat verdict directly. Kernel re-check for SMT certificates is **Phase 2/3 work** (tasks B2/B3/B4 in §16). Until then, the trusted base includes `verum_smt` alongside `verum_kernel`, and the LCF discipline is aspirational for `@verify(certified)` only. This is tracked as VUVA §17 Q9 (`kernel-ownership-migration`).

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

Implementation `core/theory_interop.vr` is already partially landed; VUVA stabilises the API.

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

**K-SmtRecheck** (certificate verification):

```
  Γ ⊢ query : Prop        witness : CoreTermWitnessOf(query)
  Γ ⊢ kernel_recheck(witness) = Ok    // polynomial-time walk
  ────────────────────────────────────────────────────────── (K-Smt)
  Γ ⊢ SmtCertificate(query, backend, witness) : query
```

### 4.5 Metatheory status

| Property | Status | Evidence / side-conditions |
|---|---|---|
| Subject reduction | Expected | Inherits from CCHM (Cohen–Coquand–Huber–Mörtberg 2018); refinement + K-Refine are admissible because `Refined` reduces only through `RefineErase` which discards proof content. |
| Strong normalisation | Expected for the CCHM fragment + Prop-only framework axioms | Huber 2019. Side-condition: every `FrameworkAxiom(_, _, body)` must satisfy `body : Prop` (enforced by `K-FwAx`); a non-Prop axiom (`unsafe_cast: ∀A B. A→B`) would break SN. |
| Canonicity | For **closed** terms of canonical type-formers (Π, Σ, Path, Inductive, HIT) in the CCHM fragment | Huber 2019 closed-term canonicity. Open terms with refinement subtypes are *not* canonical (`Refined(A, x, P)` of an open term may have no canonical form until its predicate is discharged). |
| Decidability of type-checking | Yes for the ETT-free fragment + intensional equality | Refinement subtype-checking is decidable iff the underlying predicate satisfies the discharge strategy (e.g. `@verify(static)` ⇒ decidable; `@verify(proof)` ⇒ user-supplied). |
| Parametricity | Optional, via a `@parametric` framework axiom | Not kernel-level. |
| Initiality (of `Syn(Verum)`) | Open | Requires formal $\mathrm{Syn}/\mathrm{Mod}$ adjunction mechanisation. |
| Consistency | Relative to $\ZFC + 2\text{-inacc}$ (per MSFS Convention `conv:zfc-inacc`), modulo the **chosen framework-axiom bundle** | A user importing `core.math.frameworks.classical_lem` and an axiom that internalises the law of excluded middle inhabits the corresponding Prop; consistency is then relative to ZFC + classical_lem's stated content. The kernel does not certify the axiom bundle's mutual consistency — that is delegated to the bundle's citation. |

**Open**: formal proof of metatheory inside Verum itself. Tracked as `task:metatheory-self-verification`. See §16.6 task F2 for the relativised statement (the Gödel boundary forbids absolute self-consistency).

---

## 5. Three Refinement Forms — Formal Specification

This section is **normative** for the refinement surface. `03-type-system.md` § 1.5 is descriptive; VUVA pins semantics.

### 5.1 Grammar

Existing productions in `grammar/verum.ebnf` (§ 2.12 `refinement`, § 2.14 `sigma_binding`, § 2.18 `refinement_method`). VUVA reaffirms the three canonical forms and **deprecates the historical "five rules"** (cf. `crates/verum_ast/src/ty.rs` `RefinementPredicate` doc-comment, `docs/detailed/03-type-system.md` § 1.5):

| Canonical form | Historical name (deprecated label in parentheses) |
|---|---|
| Inline `T{pred}` | Rule 1 (Inline) |
| Declarative `T where name` | Rule 4 (Named) |
| Sigma `x: T where P(x)` | Rule 3 (Sigma) |

Rules 2 (`T where |x| pred`) and 5 (bare `T where pred` with implicit `it`) are **eliminated** in VUVA — rule 2 collapses into sigma, rule 5 into inline.

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

**MUST NOT** introduce a fourth form without a VUVA revision.

---

## 6. `@framework` Axiom System

### 6.1 Declaration

Attaches to `axiom_decl` / `theorem_decl` / `lemma_decl` / `corollary_decl` per `grammar/verum.ebnf` § 2.19. Existing parser shape (2-positional-arg form) is `@framework(identifier, "citation")`; VUVA extends to named args:

```verum
// Minimal (already shipped per verum_kernel::load_framework_axioms):
@framework(lurie_htt, "Lurie J. 2009. Higher Topos Theory. §6.2.2.7")
axiom yoneda_embedding_ff<C: SmallCategory>(F: C -> Set, G: C -> Set)
    ensures Hom<PSh(C), F, G> equiv Nat_Trans<F, G>;

// Extended (VUVA Phase 1 task A3 — opt-in named fields):
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

### 6.2 The standard six-pack

Ships in `core.math.frameworks`:

| Package | Lineage | Representative axioms |
|---|---|---|
| `lurie_htt` | Lurie J. 2009. *Higher Topos Theory.* | straightening / unstraightening, coherent diagrams, ∞-Kan extension formula |
| `schreiber_dcct` | Schreiber U. 2013. *Differential Cohomology in a Cohesive ∞-Topos.* | shape ∫, flat ♭, sharp ♯, cohesion axioms |
| `connes_reconstruction` | Connes A. 1994. *Noncommutative Geometry.* | spectral-triple reconstruction, KO-dimension |
| `petz_classification` | Petz D. 1986. *Quasi-entropies.* | f-divergence characterisation |
| `arnold_mather` | Arnold–Mather. *Dynamical Systems III.* | critical-value theory |
| `baez_dolan` | Baez J., Dolan J. 1995. *Higher-Dimensional Algebra.* | stabilisation, hypothesis H |

36 axioms in total. User-authored packages extend this.

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

---

## 11. Dual Stdlib — OC + DC

### 11.1 OC layer (`core.math.*`) — articulations

Already partially present. VUVA stabilises module paths:

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

For every `α: Articulation`, VUVA guarantees:

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

- Accepted values: `"ε_math"`, `"ε_compute"`, `"ε_observe"`, `"ε_prove"`, `"ε_decide"`, `"ε_translate"`, `"ε_construct"`, `"ε_classify"` (the OWL 2 ontology-classification primitive added in the VUVA §21 V1 ecosystem), or a user-defined ε from `core.action.primitives`. Both Unicode and ASCII spellings (`epsilon_*`) are accepted; the typed attribute canonicalises to the Unicode form for deterministic audit output.
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

Noesis (`internal/holon/internal/diakrisis/docs/11-noesis/`) is a **separate project** that will use Verum. VUVA defines what Verum owes Noesis and what Noesis handles internally.

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
- **Task C2**: Inductive types with positivity check.
- **Task C3**: HITs with eliminator auto-gen.
- **Task C4**: Quotient types.
- **Task C5**: Quantitative annotations.
- **Task C6**: Framework axioms: ship the standard six-pack (`lurie_htt`, `schreiber_dcct`, `connes_reconstruction`, `petz_classification`, `arnold_mather`, `baez_dolan`).
- **Task C7** (V1 ✓ shipped): `core.math.frameworks.owl2_fs` package — 64 trusted-boundary `@framework(owl2_fs, ...)` axioms in nine sub-modules (object_property / data_range / class_expr / class_axiom / object_property_axiom / data_property_axiom / datatype_definition / key / assertion) + types.vr (Individual / Literal sorts + count_o quantifier-of-quantity). V1 ships axiom *signatures* with `ensures true;` placeholder bodies; V2 will replace each placeholder with the verbatim Shkotin Table-row HOL definition so SMT dispatch via CVC5 FMF can decide encoded obligations. `verum audit --framework-axioms --by-lineage owl2_fs` enumerates the OWL 2 footprint of any corpus; `verum audit --coord` projects owl2_fs theorems to ν=1, τ=intensional.
- **Task C7b**: Canonical bridge `owl2_fs → lurie_htt` (~30 translate axioms) so OWL 2 corpora automatically receive ∞-topos / categorical interpretations (§21.7).
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

1. **Framework-axiom lineage normalisation.** What is the canonical slug format? Proposal: snake-case lineage (`lurie_htt`, `schreiber_dcct`). Alternatives: full author+year, DOI-based. Decision deferred to Task A3.
2. **`Axi-8` status (M-5w*: non-Yoneda-representable α_𝖬).** Diakrisis 02-axiomatics `Axi-8` requires that the metaisation articulation `α_𝖬` is **not** Yoneda-representable. In the canonical Cat-model `ι(𝖬)` *is* Yoneda-representable, so Axi-8 fails there; Diakrisis frames this as an open question on whether non-LP (locally-presentable) models satisfying Axi-8 exist. VUVA defers: the kernel does NOT enforce Axi-8 today; if Diakrisis (or downstream foundational work) lands a consistent model satisfying it, a future VUVA revision adds the enforcement as an opt-in `@framework(diakrisis_axi8, …)` package rather than a kernel rule, so users can choose the foundational stance per project.
3. **Quantitative / linear types default.** Ship as opt-in (`@1 x: T`) vs opt-out (default ω is current). Proposal: opt-in via attribute; default remains ω.
4. **Impredicative Prop.** Ship Coq-style impredicative Prop, or require framework axiom (`core.math.frameworks.impredicative_prop`)? Proposal: latter, for soundness with HITs + classical axiom + univalence.
5. **Certificate format schema versioning.** Lean evolves fast; export needs target-version pinning. Proposal: `verum export --to lean --target-version 4.4.0`.
6. **`synthesize` strategy termination.** No bound, no proved termination. Proposal: run with wall-clock cap + hint database; surface best-effort partial proofs in a `@partial` theorem.
7. **OC/DC duality proof.** The auto-induced ε(α) assumes 108.T. The proof of 108.T sits in the Diakrisis preprint (forthcoming). Production compile should gate `@enact` on the Diakrisis release.
8. **Kernel LOC budget.** Target 5 000 LOC. Current `verum_kernel` is ~4 000 LOC (per `crates/verum_kernel/src/`). Adding `K-Refine` + framework-axiom + SMT-certificate nodes may push over; VUVA budget allows up to 6 500 LOC with mandatory audit.
9. **`verum_types` vs kernel re-check split.** Today, `verum_types::refinement` does refinement checking outside the kernel. Long-term, kernel should own the final recheck; `verum_types` hosts elaboration. Task track: `kernel-ownership-migration`.
10. **Cubical computational univalence.** CCHM provides it, but performance on large universes is a concern. Pragmatic path: lazy `Glue` evaluation + memoisation.
11. **Pattern-match coverage checker** for dependent types: requires higher-order unification. Proposal: extend existing `verum_types::exhaustiveness` with dependent-index analysis.
12. **Tactic DSL hygiene.** Quote/unquote macro hygiene: freshness of binders, scope capture. Proposal: α-renaming on every splice; runtime cost acceptable.

---

## 18. Success Criteria

VUVA is considered delivered when:

1. Every existing `.vr` file in `core/` and `vcs/specs/` compiles under `@verify(static)` with zero changes.
2. `grammar/verum.ebnf` parses all three refinement forms and the grammar tests pass.
3. `@framework(…)` is accepted and the six-pack ships.
4. `verum audit --framework-axioms` lists used axioms for the entire stdlib.
5. `verum audit --coord` produces a `(Fw, ν, τ)` tuple per user theorem.
6. All nine `@verify(…)` strategies are dispatched through `verum_smt` or `verum_kernel`.
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
| **K-Refine** | VUVA kernel typing rule enforcing T-2f*. Single load-bearing paradox-immunity rule. |
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
| **OWL 2 RBS** | OWL 2 RDF-Based Semantics — W3C Recommendation 2012, graph-based semantics over RDF triples. Undecidable. Not in VUVA scope. |
| **Shkotin 2019 (DS2HOL)** | Denotational HOL formalisation of OWL 2 DS. `internal/OWL2.DS2HOL.pdf`. Verum's formal bridge for `core.math.frameworks.owl2_fs`. |
| **`core.math.frameworks.owl2_fs`** | ~63 `@framework(owl2_fs, …)` axioms encoding Shkotin 2019 Tables 1–10. Trusted OWL 2 semantic boundary. |
| **`OwlAttr` family** | Typed attributes in `verum_ast::attr::typed` (`@owl2_class`, `@owl2_property`, etc.) preserving OWL 2 vocabulary for round-trip. |
| **`count_o`** | Verum realisation of Shkotin's `#` quantifier (`#y:o P(y)`). SMT-dispatched via CVC5 Finite Model Finding; errors out on unbounded domains. |
| **NAMED restriction** | OWL 2 DS Key axiom restricts to individuals declared as OWL 2 `NamedIndividual`. Verum `@verify(proof)` with user tactic (§21.3). |
| **Faithful translation** | VUVA §21.2 baseline claim: every OWL 2 DS-valid derivation is Verum-valid (one direction). Automatic from Shkotin-literal encoding. |
| **Morita-equivalence bridge** | VUVA §21.2 roadmap claim (Phase 6 F7): Verum OWL 2 encoding is Morita-reducible both directions to W3C DS. |

---

## 20. References

Internal (this repository):

- `grammar/verum.ebnf` — authoritative grammar.
- `internal/docs/detailed/03-type-system.md` — base type system, three refinement forms (descriptive).
- `internal/docs/detailed/09-verification-system.md` — verification system (descriptive).
- `internal/docs/detailed/12-dependent-types.md` — dependent types (planned, now superseded in part by VUVA §7).
- `internal/docs/detailed/13-formal-proofs.md` — formal proofs (planned, now superseded in part by VUVA §8).
- `internal/docs/detailed/17-meta-system.md` — meta-system (planned, now superseded in part by VUVA §9).
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

*Ontology support is not a bolt-on. It is the first major external-framework package that exercises every pillar of VUVA — `@framework` axioms, three refinement forms, nine-strategy ladder, `core.theory_interop`, certificate export — and it is the architectural bridge to the world's largest existing knowledge corpora (SNOMED-CT, Gene Ontology, DBpedia, SUMO, Cyc, DOLCE, BFO, FIBO). The formal target is a denotational HOL bridge due to Shkotin 2019 (`internal/OWL2.DS2HOL.pdf`), itself a Morita-preserving translation of the W3C OWL 2 Direct Semantics Recommendation (2012).*

### 21.1 Scope — Direct Semantics only

Verum OWL 2 support targets **OWL 2 Direct Semantics (DS)** — the model-theoretic semantics recommended by W3C and used by every mainstream reasoner (HermiT, Pellet, FaCT++, ELK, Konclude). The RDF-Based Semantics (RBS) variant is **out of scope**.

Rationale:

- DS gives OWL 2 DL, a decidable fragment of SROIQ with known complexity profiles (EL / QL / RL polynomial; DL 2NEXPTIME). This matches VUVA §12's nine-strategy ladder which expects predictable complexity per strategy.
- RBS is undecidable and graph-based; its RDF-triple primitive does not map cleanly onto Verum's typed refinement structure (VUVA §5) without a separate graph-modelling layer that would duplicate `core.collections.Map<Text, Set<Text>>`.
- Shkotin 2019 — the formal bridge Verum imports — formalises DS only. RBS has no equivalent in-kernel formalisation.
- Noesis's knowledge-object model (VUVA §14.2, `diakrisis/docs/11-noesis/03-knowledge-model.md`) is object-based, not RDF-triple-based, aligning directly with DS.

If a consumer needs RBS compatibility (e.g. SPARQL interop), a dedicated `core.math.frameworks.owl2_rbs` package is a separate, future Phase-6 work item.

### 21.2 Correspondence claim — faithful translation, Morita as roadmap

Verum makes **two layered claims** about the `core.math.frameworks.owl2_fs` package:

**(1) Shipped baseline — faithful translation.** Every valid derivation in W3C DS is a valid derivation in Verum, via the following chain:

```
W3C OWL 2 DS  ──(Shkotin 2019 Table 1–10)──►  HOL definitions  ──(VUVA §21.5)──►  Verum @framework axioms
```

Concretely: every OWL 2 satisfiability / entailment judgement `O ⊨ α` (ontology O entails axiom α under DS) corresponds to a Verum derivation of the encoded form `owl2_sem(O) ⊢ owl2_sem(α)`. The encoding is line-by-line from Shkotin's "HOL-definition body" column; faithful translation follows by construction.

**(2) Roadmap goal — Morita-equivalence.** Phase 6 task F7 proves that the encoding is NOT only faithful but a Morita-reduction in both directions: every Verum derivation of an encoded ontology sentence corresponds to a W3C DS derivation. This elevates the claim from "Verum preserves OWL 2" to "Verum OWL 2 is OWL 2" up to categorical equivalence.

Verum does NOT make correspondence claim (c) ("implementation only, no formal guarantee") — that would be beneath VUVA's rigor standard (cf. §15.5 kernel re-check invariant).

### 21.3 Three-layer architecture

OWL 2 integration decomposes into three clean layers, each mapping onto an existing VUVA architectural slot (§3). This is not new architecture — it is OWL 2 lifted into VUVA's existing surfaces.

**Layer 1 — Semantic framework package** (`core.math.frameworks.owl2_fs`, ~60 axioms, VUVA Layer 1):

Line-by-line encoding of Shkotin 2019 Tables 1–10. Each OWL 2 operator becomes a `@framework(owl2_fs, "Shkotin 2019. DS2HOL-1 §X.Y Table Z.")` axiom with its HOL-definition body copied verbatim. This is the trusted boundary — `verum audit --framework-axioms` enumerates exactly which operators a corpus depends on.

**Layer 2 — Vocabulary-preserving typed attributes** (`verum_ast::attr::typed::OwlAttr`, VUVA Layer 6):

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

**Layer 3 — Verification obligations** (`@theorem` + `@verify(strategy)`, VUVA Layer 4):

Subsumption, classification, consistency, instance checking — all become `@theorem` declarations with `ensures` clauses, discharged via the nine-strategy ladder (§12). Mapping:

| OWL 2 task | VUVA strategy | ν-ordinal | Rationale |
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

Direct accommodation of both simultaneously would require every OWL 2 query to return `Maybe<Bool>` with `Unknown`, breaking composition with the rest of Verum's type system. VUVA chooses a pragmatic resolution:

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

**Lineage map** (extends VUVA §8.5 exporter): `owl2_fs` lineage maps to `owl2-fs` target for export, preserving IRIs and anonymous-individual names (`genid*`) per Shkotin 2019 §»HOL: OWL2 ontology as a whole» convention.

### 21.9 Integration with Noesis

The Noesis platform (VUVA §14, `diakrisis/docs/11-noesis/`) consumes OWL 2 directly:

- Noesis NP-protocol endpoint `knowledge/import` accepts OWL 2 FS files via Verum's import CLI.
- Noesis knowledge-object `K` becomes a `core.math.frameworks.owl2_fs.Ontology` bundle.
- Noesis dependency types (`requires`, `entails`, `generalizes`, `instantiates`, `contradicts`, …) compile onto OWL 2 Class / Property axioms.
- Noesis `coherence/check` reuses `core.theory_interop.check_coherence` over the OWL 2 axiom graph (Čech descent on property intersections).
- Noesis `agent/propose` — LLM-generated claims filtered through `@verify(formal)` before surfacing.

This is the operational realisation of the Noesis-boundary contract in VUVA §14.3 for the OWL 2 stratum.

### 21.10 Roadmap additions

Extending VUVA §16 with OWL 2 work items:

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

### 21.11 Open questions deferred to implementation

Three questions flagged for confirmation during C7 implementation (should be resolved via e-mail to paper author `ashkotin@acm.org` before merging C7, or accepted as documented defaults):

1. **HOL variant** — which HOL system is Shkotin's target (Church / HOL4 / HOL Light / Isabelle-HOL)? Default assumption: classical HOL (so Verum OWL 2 package depends on `@framework(classical_lem, "W3C OWL 2 DS. 2012.")`).

2. **Quantifier `#` on potentially infinite domains** — Shkotin does not specify. Default: `count_o` requires `@owl2_class(closed_domain = true)` when domain finiteness is not inferable by the SMT backend.

3. **Anonymous-individual negation scope** — `NegativeObjectPropertyAssertion(OPE :a _:b)` is `¬OPE(a, b_anon)` or `¬∃b_anon. OPE(a, b_anon)`? Default: OWA-semantically `¬∃b_anon. OPE(a, b_anon)` (matches W3C DS §5.6).

All three are documented in `core/math/frameworks/owl2_fs/README.md` at C7 ship time with explicit citation of the default choice.

### 21.12 Success criteria (OWL 2-specific extensions to VUVA §18)

1. Round-trip: Pellet-compatible `foaf.owl` → `verum import` → `verum export` → byte-identical output.
2. SNOMED-CT medical corpus classification produces the same class hierarchy as HermiT.
3. `verum audit --framework-axioms` enumerates every used OWL 2 operator with Shkotin 2019 table/row citation.
4. Verum corpus mixing `@framework(lurie_htt, …)` theorems with `@framework(owl2_fs, …)` ontologies compiles and verifies cleanly (cross-framework non-interference).
5. `core.theory_interop.translate(owl2_ontology, lurie_htt_target)` produces a well-typed result for the standard OWL 2 test suite (W3C Test Cases Part 2).

---

## Appendix A — Relationship to Legacy Specs

| Legacy | Status after VUVA |
|---|---|
| `03-type-system.md` § 1.5 (three refinement forms) | Normative — quoted and formalised in §5. |
| `09-verification-system.md` (modes, contract literals) | Partially superseded — modes extended to nine-strategy ladder (§12). `contract#"…"` DSL retained as surface. |
| `12-dependent-types.md` (planned v2.0+) | Partially superseded — §7 is authoritative. |
| `13-formal-proofs.md` (planned v2.0+) | Partially superseded — §8 is authoritative. |
| `17-meta-system.md` (meta fn, Quote/Unquote) | Refined — §9 is authoritative. |
| `26-unified-execution-architecture.md` | Orthogonal — VUVA operates at Layer 3–6; execution at Layer 2. |

Legacy specs remain as descriptive context; VUVA is the single source of truth for verification/proofs/dependent-types/meta-system from this point forward.

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
