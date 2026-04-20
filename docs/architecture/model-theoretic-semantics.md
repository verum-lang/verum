# Model-Theoretic Semantics of Protocols

> Reference specification for auto-discharge of protocol axioms at
> `implement` sites, aligned with Verum's six design principles.

## The question

Verum lets a protocol carry axioms:

```verum
type Group is protocol {
    type Elem;
    fn unit() -> Self.Elem;
    fn mul(a: Self.Elem, b: Self.Elem) -> Self.Elem;
    fn inv(a: Self.Elem) -> Self.Elem;
    axiom left_unit(x: Self.Elem)   ensures Self.mul(Self.unit(), x) == x;
    axiom left_inv(x: Self.Elem)    ensures Self.mul(Self.inv(x), x) == Self.unit();
    axiom assoc(a: Self.Elem, b: Self.Elem, c: Self.Elem)
        ensures Self.mul(Self.mul(a, b), c) == Self.mul(a, Self.mul(b, c));
};
```

And a user writes:

```verum
implement Group for IntGroup {
    type Elem = Int;
    fn unit()  -> Int        { 0 }
    fn mul(a: Int, b: Int) -> Int { a + b }
    fn inv(a: Int) -> Int    { -a }
}
```

**The question**: what does the compiler do with the protocol's axioms?

## The wrong answers

Three tempting solutions that VIOLATE Verum's principles:

1. **Ignore them.** Axioms are decoration; the impl compiles as long
   as the methods type-check. *Violates:* "Verification is a spectrum"
   — an axiom that is never discharged provides zero verification
   value, and the user has no way to tell whether the implementation
   actually satisfies the theory.

2. **Require explicit proofs always.** Every implement block must
   carry a `proof axiom_name by tactic;` clause for every axiom in
   the protocol. *Violates:* "Zero-cost default" — the ergonomic
   cost of boilerplate for trivially-SMT-discharged axioms (ring
   axioms on `Int`, for example) is unacceptable.

3. **Use classical logic + axiom-of-choice + decidable equality.**
   Treat every axiom as a hypothesis that's "just true." *Violates:*
   "No magic" and "Semantic honesty" — the user writes
   `implement Group for BrokenGroup {}` where `mul = a - b`, and the
   compiler silently accepts the implementation as a group, producing
   an unsound system.

## The reference-grade answer

### Two-phase discharge pipeline

When the compiler processes `implement P for T { items }`:

**Phase 1 — collect obligations.**
For each `ProtocolItemKind::Axiom(axiom_decl)` in P's AST:

1. Substitute `Self` with `T` in the axiom's proposition.
2. Substitute each `Self.M` reference with the concrete item from
   `items` that satisfies M's signature:
   - `Self.Elem` → `T`'s associated-type binding (`Int` in the IntGroup example).
   - `Self.unit()` → the concrete body `{ 0 }` (or, equivalently, a
     Curry-Howard term `IntGroup.unit`).
   - `Self.mul(a, b)` → `a + b`.
3. The resulting proposition is the **obligation**.

Example: `left_unit` with protocol `Group` and model `IntGroup` becomes
`forall x: Int. (0 + x) == x`.

**Phase 2 — discharge.**
For each obligation:

1. Check whether `items` contains an explicit `proof axiom_name by tactic;`
   clause. If present, run the named tactic against the obligation;
   on success the axiom is verified; on failure emit a precise
   diagnostic.
2. Otherwise, invoke `ProofSearchEngine::auto_prove(smt_ctx, obligation)`
   with a default budget (∼500 ms per obligation). On success the
   axiom is verified; on failure, the diagnostic is:

   ```
   error[E502]: implement Group for IntGroup does not satisfy axiom `left_unit`
     --> mymodule.vr:42:11
      |
   42 |   fn mul(a: Int, b: Int) -> Int { a * b }
      |           --------------------   ^^^^^^^^^
      |                                  the model's `mul` does not satisfy
      |                                  `Self.mul(Self.unit(), x) == x`
      |
   help: add an explicit proof if you believe this should hold:
            proof left_unit by ring;
   ```

### Alignment with the six principles

| Principle | How the two-phase discharge respects it |
|-----------|------------------------------------------|
| **Semantic honesty** | An `implement` block that doesn't discharge the theory's axioms is *not a model* of the theory. The compiler rejects it. |
| **Verification spectrum** | The user can pick where on the spectrum each axiom sits: `proof X by runtime;` for lazy check, `proof X by smt;` for formal, `proof X by certified;` for externally checkable. |
| **No hidden state** | Every proof obligation is explicit in the diagnostic; the user always knows what's been verified and what's been auto-discharged. |
| **Zero-cost default** | Axioms that SMT closes in milliseconds require zero user-level annotation. The boilerplate cost scales with the *hardness* of the theory, not with its size. |
| **No magic** | The obligation-generation step is a mechanical substitution, visible in the diagnostic trace. No classical-logic-induced soundness hole. |
| **Radical expressiveness** | Verum is the first mainstream language where `implement Group for T { … }` **IS a model-theoretic claim** — not mere interface conformance. |

### Why this is *the* reference solution

1. **Conservative over classical systems.** In Coq, `Class Group` +
   `Instance IntGroup` requires explicit field-by-field proof; there's
   no auto-discharge. Verum's auto-discharge via SMT reduces user
   burden *without* compromising soundness — the proof object is
   still generated, just mechanically.

2. **More expressive than HOL systems.** In Isabelle, `locale group`
   + `interpretation` carries locale obligations but discharge is
   manual. Verum lifts that to the language level AND integrates with
   the spectrum of verification strategies (runtime → SMT → certified).

3. **More sound than dependent-type-only systems.** In Lean 4, a
   `class Group` + `instance` can silently accept unsound
   implementations because Lean doesn't SMT-check the instance by
   default. Verum enforces it.

4. **Compatible with refinement types.** Because obligations flow
   through the existing `ProofSearchEngine::auto_prove` interface, the
   same SMT backend that handles `Int{x > 0}` discharges `left_unit`.
   One proof engine, uniform treatment.

5. **Compatible with HoTT/cubical.** Obligations are `Expr`s in the
   AST; for cubical obligations (path equality), auto_prove routes to
   the cubical tactic. No special casing.

## Implementation blueprint

### AST additions

- `ProtocolItemKind::Axiom(AxiomDecl)` — a protocol item carrying
  an axiom declaration.
- `AxiomDecl.proposition` synthesised from the axiom's `ensures`
  clauses (canonical: conjunction of ensures; with requires: the
  implication `R → E`).

### Phase 1 — obligation collection

New function `collect_impl_obligations(impl_decl, protocol_decl) -> List<ProofObligation>`
where:

```rust
pub struct ProofObligation {
    pub axiom_name: Ident,
    pub proposition: Expr,       // Self-substituted
    pub origin_span: Span,        // axiom decl span
    pub impl_span: Span,          // impl block span
}
```

The substitution walks the proposition's `Expr` tree, replacing:
- `Path(self)` → `Path(impl_decl.for_type)`
- `Path(Self, M)` → the M-named item in `impl_decl.items`
- Field access `self.x` threads through identically.

### Phase 2 — discharge

New function `verify_impl_axioms(engine, smt_ctx, obligations, impl_items) -> Result<List<Certificate>, Diagnostic>`:

```rust
for obligation in obligations {
    if let Some(explicit_proof) = impl_items.find_proof_for(obligation.axiom_name) {
        engine.execute_tactic(&explicit_proof.tactic, obligation.proposition)?;
    } else {
        engine.auto_prove(smt_ctx, &obligation.proposition)?;
    }
}
```

### New grammar — `proof name by tactic;` inside impl blocks

Extend `impl_item` in `grammar/verum.ebnf`:

```ebnf
impl_item = function_decl
          | associated_type_decl
          | associated_const_decl
          | proof_clause ;
proof_clause = 'proof' , identifier , 'by' , tactic_expr , ';' ;
```

Parser: new branch in `parse_impl_item` recognising `proof` keyword
at the head of an impl item. AST: new `ImplItemKind::Proof(ProofClause)`.

### Pipeline integration

In `verum_compiler::pipeline`:

1. After type registration, walk `ItemKind::Impl` blocks.
2. For each `ImplKind::Protocol { protocol, for_type, .. }`:
   a. Look up the protocol's AST by name.
   b. Call `collect_impl_obligations`.
   c. Call `verify_impl_axioms`.
3. Merge diagnostics into the existing error channel.

### Opt-out

For gradual migration, tests can annotate the impl block:

```verum
@verify(none)
implement Group for IntGroup { … }
```

This suppresses obligation generation — the implementation is taken
on faith. Equivalent to classical `trust_me` annotations. `@verify(formal)`
is the default; `@verify(runtime)` emits runtime asserts for every
axiom; `@verify(certified)` additionally emits externally-checkable
proof artifacts.

## Non-goals

- **Full dependent-type elaboration.** If an axiom references a
  dependent type (`axiom lift(f: Self.Elem)` where `Elem: Type(u)`),
  the discharge pipeline assumes the universe-polymorphism
  infrastructure has resolved the universe upstream. No universe
  inference happens during discharge.
- **Incremental re-verification.** On source change, discharged
  obligations re-run from scratch. Proof caching is a performance
  concern outside this specification.
- **Arbitrary user-defined tactics at the impl level.** Built-in
  tactics (`auto`, `ring`, `smt`, `simp`, etc.) plus their combinators
  suffice for discharge-site usage. Complex proof strategies belong
  in top-level `theorem` bodies.

## Consequences

- Morphism protocols (`Hom<A, B>` with preservation axioms) flow
  through the same pipeline — `implement Hom<A, B> for Projection`
  auto-discharges preservation obligations uniformly.
- Axiom-constrained algorithms (e.g. a graph routine that requires
  `DAG.acyclic_directed`) can soundly invoke the axiom as a
  hypothesis because the `implement` has already discharged it.
- External libraries translated into Verum emit concrete
  `implement P for T` blocks whose original proof content becomes
  explicit `proof X by …;` clauses.

## Call graph

```
pipeline.phase_verify()
  └─> verify_theorem_proofs(module)
  └─> verify_impl_axioms_for_module(module)
        └─> find_protocol_decl(module, name)
        └─> proof_verification::verify_impl_axioms(impl_decl, protocol_decl)
              └─> proof_obligations::collect_impl_obligations(impl_decl, protocol_decl)
                    └─> ImplBindings::from_impl_items(for_type, items)
                    └─> substitute_self_in_expr(axiom.proposition, for_type, bindings)
              └─> for each obligation:
                    ├─> find_proof_clause_for(impl_decl, axiom_name)?
                    │     YES → convert_tactic() → engine.execute_tactic()
                    │     NO  → engine.auto_prove(smt_ctx, proposition)
                    └─> record verified / unverified in ImplVerificationReport
        └─> emit diagnostics for unverified axioms
```

## Diagnostic form

Failed obligations surface through the standard diagnostic channel:

```
model verification: `implement Group for IntGroup` does not
discharge axiom `left_unit` (auto_prove could not close the
obligation: <timeout / saturation / UNSAT>)
```

Severity is controlled by the session option
`model_verification_level ∈ {Off, Warn, Error}`. The default in
stable releases is `Error` — a model that fails to discharge its
theory's axioms is not a model, and the compiler rejects it.

## Coverage

`substitute_self_in_expr` recurses through the expression shapes
that appear in axioms:

- `Path` — the primary substitution target (Self / Self.X).
- `Binary` / `Unary` — algebraic connectives.
- `Call` / `MethodCall` / `Field` — user-defined operations and
  their dotted references.
- `If` / `Match` — conditional axioms.

Extending to new expression kinds as the axiom surface grows is a
straightforward addition to the walker; the substitution is purely
syntactic.

## Source layout

| Layer | File |
|-------|------|
| AST | `crates/verum_ast/src/decl.rs` — `ProtocolItemKind::Axiom`, `ImplItemKind::Proof` |
| Parser | `crates/verum_fast_parser/src/proof.rs` + `decl.rs` — `axiom … ensures …`, `proof X by tactic;` |
| Substitution + collection | `crates/verum_types/src/proof_obligations.rs` |
| Discharge | `crates/verum_compiler/src/phases/proof_verification.rs` — `verify_impl_axioms` |
| Pipeline hook | `crates/verum_compiler/src/pipeline.rs` — `verify_impl_axioms_for_module` |
| Grammar | `grammar/verum.ebnf` §2.19 — protocol axioms, proof_clause |
