# Verum's Unified Type Theory

> Central architectural specification. Verum is not a feature
> collection; it is a coherent **stratified refined quantitative
> two-level type theory** with integrated SMT-backed gradual
> verification and model-theoretic protocol discharge.

## Thesis

Every value in Verum is classified by four orthogonal coordinates:

```
(universe_layer, universe_level, quantity, refinement)
```

All surface features — refinement types, dependent types, HoTT /
cubical, CBGR (three-tier references), protocol axioms, tactic DSL —
are *specialisations* of this classification, not parallel
mechanisms.

---

## 1. Landscape

Comparative map of proof-oriented languages across five foundational
pillars — Martin-Löf type theory, cubical / HoTT, quantitative type
theory, refinement + SMT, classical-logic integration:

| System      | MLTT | Cubical/HoTT | QTT | Refinement | SMT | Classical |
|-------------|------|--------------|-----|------------|-----|-----------|
| Coq         | ✓ CIC | partial (HoTT lib) | — | — | — | axiom-only |
| Lean 4      | ✓ CIC | mathlib | — | — | — | axiom |
| Agda        | ✓    | ✓ (cubical-mode) | partial | — | — | — |
| Idris 2     | ✓    | — | **✓** | — | — | — |
| F*          | ✓    | — | — | **✓** | **✓** | classical |
| Isabelle/HOL| simple+poly | — | — | — | partial | **✓** |
| **Verum**   | **✓** | **✓** | **✓** | **✓** | **✓** | **gradual** |

No existing system integrates all five pillars coherently. Verum
does, by stratifying them — each layer handles what it is best at,
and explicit coercion rules police flow across boundaries.

---

## 2. Foundations already present in the language

### 2.1 Two-level type theory (`core/types/two_level.vr`)

```verum
public type Layer is
    | Fibrant    // HoTT / cubical; UIP does NOT hold
    | Strict     // UIP holds, decidable equality
```

Universes are stratified into a fibrant layer supporting path types
and univalence, and a strict layer with uniqueness of identity
proofs. `mix(Strict, _) = Strict` (strictness contagion).

### 2.2 Quantitative type theory (`core/types/qtt.vr`)

```verum
public type Quantity is Zero | One | Many | AtMost { n }
```

Every binding carries a usage quantity — `Zero` (erased), `One`
(linear, must-use-exactly-once), `Many` (unrestricted), `AtMost(n)`
(bounded). Additive algebra: `add(One, One) = AtMost(2)`,
`Many ⊕ _ = Many`.

### 2.3 Refinement types

```verum
type Nat is Int{self >= 0};
fn safe_div(a: Int, b: Int{self != 0}) -> Int { a / b }
```

Base type + SMT-checkable predicate. Subtyping is decidable by the
SMT backend: `Int{p} <: Int{q}` ⟺ `∀x:Int. p(x) → q(x)`.

### 2.4 Cubical HoTT (`core/math/hott.vr`)

Interval `I`, `Path<T>(a, b)`, `Equiv`, `IsContr`, `IsProp`, `IsSet`,
higher inductive types (S¹, Susp, Trunc). Computation rules in the
cubical normaliser.

### 2.5 CBGR three-tier references

```verum
&T           // tier 0: generation-counter-checked (≈ 15 ns)
&checked T   // tier 1: compiler-proven safe   (0 ns)
&unsafe T    // tier 2: manual proof obligation (0 ns)
```

### 2.6 Protocol axioms (`ProtocolItemKind::Axiom`)

```verum
type Group is protocol {
    axiom assoc(a, b, c) ensures (a · b) · c == a · (b · c);
};
```

### 2.7 SMT integration

`verum_smt::ProofSearchEngine::auto_prove(goal)` over Z3 / CVC5.
Linked to the tactic DSL through `convert_tactic()` in
`verum_compiler::phases::proof_verification`.

---

## 3. Synthesis — the unified model

### 3.1 Semantic core

Every value lives at four orthogonal coordinates:

```
(universe_layer ∈ {Fibrant, Strict},
 universe_level ∈ ℕ,
 quantity       ∈ {Zero, One, Many, AtMost(n)},
 refinement     : base → Bool)
```

- `Int{>= 0}` in strict layer at level 0 with quantity `Many` is
  `(Strict, 0, Many, λx. x ≥ 0)`.
- `Path<S1>(base, base)` in fibrant layer at level 0 with quantity
  `Many` and no refinement is `(Fibrant, 0, Many, ⊤)`.
- `&mut Mutex<T>` with quantity `One` is a linear reference that
  must be used exactly once.

### 3.2 Coercion rules — the soundness gate

Values may flow between classifications only by these rules:

#### Layer flow

```
Fibrant ↪ Strict     (allowed — forget path structure)
Strict  ⊬ Fibrant     (forbidden — cannot transplant UIP into HoTT)
```

#### Level flow (cumulativity)

```
Type(i) ↪ Type(j)   when i ≤ j
```

#### Quantity flow

```
Many ↪ One     (forbidden — an unrestricted value cannot be linear)
One  ↪ Many    (allowed — a value that *may* be used once can be
                used arbitrarily when we relax the linearity claim)
Zero ↪ *  at type level;   * ↪ Zero only via explicit erasure
```

#### Refinement flow

```
Int{p} <: Int{q}     ⟺     SMT ⊢ p → q
```

### 3.3 Protocols as theories + models

`type T is protocol { methods; axioms }` is a **signature plus
theory** in the model-theoretic sense:

```
⟦type T is protocol { methods, axioms }⟧
  =  ⟨signature(methods), {M | ∀axiom ∈ axioms. M ⊨ axiom}⟩
```

A protocol type denotes a *category of models*. `implement T for
Concrete { ... }` picks a specific model. The discharge pipeline
(see `model-theoretic-semantics.md`) checks `Concrete ⊨ axiom` for
every axiom — this is classical model theory.

### 3.4 Tactic DSL as a proof monad

Tactics live in a monad `TacticM`:

```
TacticM<A> ≈ ProofState -> Result<(A, ProofState), TacticError>
```

- `let x = e; rest` ≈ monadic bind.
- `match x { p => t }` ≈ monadic case analysis.
- `try { t } else { f }` ≈ `MonadError.recover`.
- `fail(msg)` ≈ `throwError msg`.
- `ring`, `smt`, `auto` are atomic tactics invoking `auto_prove` on
  the SMT backend.

The tactic DSL, the SMT backend, and the cubical kernel all speak
the same `ProofGoal` / `VerificationResult` vocabulary. No
translation hops between verification surfaces.

### 3.5 Gradual verification spectrum

Each function chooses its verification strength:

```
@verify(none)       // no checks
@verify(runtime)    // runtime asserts
@verify(static)     // type checker only
@verify(formal)     // SMT-discharged (default)
@verify(certified)  // externally checkable proof artifact
```

This is not a separate verification system; it is a parameter on
the same typing rules. The refinement `Int{p}` becomes a runtime
assert under `runtime` and an SMT goal under `formal`.

---

## 4. Five architectural rules

### Rule 1. Stratification over unification

Do not unify Fibrant and Strict into a single theory. Cubical
computation stays in Fibrant; SMT stays in Strict; bridges are
explicit layer-coercion.

**Implementation consequence.** `verum_types::ty::Universe` carries
a `layer: Option<Layer>`. Default is Strict. Fibrant is forced by
explicit `@layer(fibrant)` or by the presence of a `Path<T>(_)` in
the type.

### Rule 2. Refinement declared eagerly, SMT invoked lazily

The refinement predicate lives as an unresolved expression AST. The
SMT backend is invoked only when subtyping, contract obligations,
or runtime-check emission demands it.

**Implementation consequence.** `Type::Refined { base, predicate:
Expr }` does not contain Z3 AST. Compilation proceeds normally;
SMT goals are generated on demand.

### Rule 3. Model theory as a first-class construction

`implement` is not sugar over a dispatch table. It is an
existential attestation that a given type is a model of a given
theory.

**Implementation consequence.** The pipeline runs
`collect_impl_obligations` + `verify_impl_axioms` for every
`ImplKind::Protocol` — no bypass.

### Rule 4. QTT as optimisation, not restriction

Zero-quantity values are automatically erased at the AOT tier.
Linear values compile to stack allocation with automatic destructor
after last use. `Many` lives on the heap.

**Implementation consequence.** CBGR's three-tier references are
QTT quantities in disguise: `&T` is a `Many` reference with a
runtime generation counter; `&checked T` is a `One` reference in a
compiler-proven scope; `&unsafe T` is a manual `Zero` obligation.

### Rule 5. Tactic as monad, not macro

Tactics are first-class values in the proof monad. Composition is
ordinary monadic bind. `ring` and `auto` are methods on the same
`ProofState`.

**Implementation consequence.** Both direct `auto_prove(goal)` and
`apply_ring → try_ring → auto_prove` must give the same result on
the same goal (confluence).

---

## 5. Mapping surface features to the theory

| Feature | Classification / rule |
|---------|------------------------|
| Model-theoretic check of axioms at `implement` | Rule 3 |
| Tactic engine (`ring`, `auto`, `smt` in one monad) | Rule 5 |
| Runtime refinement assert under `@verify(runtime)` | Rule 2 |
| Runtime representation of Pi/Sigma/Witness values | Rule 4 (QTT erasure) |
| Quotient type `T / R` with `Q.of` / `q.rep` projections | Fibrant-universe HIT |
| Graph algorithms (BFS, Dijkstra, …) | `(Strict, 0, Many, ⊤)` |
| Morphism coherence (`Hom<A, B>`) | Rule 3 (special case) |
| Stdlib-agnostic type system | Rule 1 (stratification) |

### Runtime-side realisation

Each surface feature has a concrete lowering on the Tier-0
interpreter. The table is not the spec (the rules above are) but it
keeps the implementation honest:

| Feature | Tier-0 lowering |
|---------|------------------|
| `Int { it > 0 }` parameter | `Assert { cond, message_id }` at fn entry |
| `Int { it > 0 }` return | `Assert` at each `Ret` site (tail-expr + `return expr;`) |
| `Π(x: T). U(x)` at runtime | `MakePi` opcode → 2-slot heap record tagged `TypeId::PI (524)` |
| `Σ(x: T). U(x)` at runtime | `MakeSigma` → tagged `TypeId::SIGMA (525)` |
| Refined value with proof hash | `MakeWitness` → tagged `TypeId::WITNESS (526)` |
| `type Q is T / R` construction | `Q.of(rep)` → identity `Mov` at Tier-0 |
| `type Q is T / R` projection | `q.rep()` → identity `Mov` at Tier-0 |

All three dependent-type packagings share a 2-slot layout
compatible with the variant-payload offset convention, so
`GetVariantData` field 0 / 1 acts as a projection primitive until
dedicated projection opcodes land.

---

## 6. Alignment with the six design principles

| Principle | How the unified theory realises it |
|-----------|------------------------------------|
| **Semantic honesty** | A type is `(layer, level, qty, refinement)`, not a byte-count. `List<T>` names a category of models. |
| **Verification spectrum** | Direct consequence of Rule 2 — `@verify(...)` switches SMT between compile-time goal and runtime assert. |
| **No hidden state** | QTT gives explicit resource tracking. Layer coercion prevents unsoundness. Axioms are visible. |
| **Zero-cost default** | Rule 4 + layer erasure: zero-quantity, type-level values are erased at Tier 1 automatically. |
| **No magic** | Every step is mechanical. SMT is invoked only via explicit tactics or verified contracts. Cubical reduction follows documented rules. |
| **Radical expressiveness** | All five pillars (MLTT + HoTT + QTT + Refinement + SMT) live in a single theory with classical-gradual coexistence. |

---

## 7. What makes Verum unique

Verum is the first language where proof-oriented and
system-oriented concepts live in one stratified type theory, not
in two or three parallel systems with bridges.

Concretely:

- **No embedded proof system** — there is a single language.
- **No unsafe world** — `unsafe` is a QTT quantity with an explicit
  proof obligation.
- **No runtime vs. compile-time divide** — this is a `@verify(...)`
  attribute on the same code.
- **No pure vs. effectful divide** — this is the `using [...]`
  context clause.
- **No strict vs. lazy divide** — this is a layer annotation.

For every new language question: place it in `(layer, level,
quantity, refinement)`, apply the five rules. The answer is
concrete, local, and preserves soundness.
