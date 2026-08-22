# ATS-V-2 P1′ — Capability rows, formalised

Status: ACCEPTED (T0848), duel discharged 2026-08-22. holon-e2's
verdict: accepted with three constructive strikes — the domain-flow
law (§5a), `carry(dyn P)` citation (§5b), and the lattice-product
remark (§6) — all folded in below; the §2 omission list SURVIVED its
strongest candidate (sandboxing) and is strengthened by it. Rule G
stands with the audit-path-precision remark recorded in §4.

The implementation target it replaces: today's inference
(`ats_v_phase.rs::infer_used_capabilities`) is a flat per-module walk
that resolves DIRECT calls against the path ontology and *silently
drops everything else* — no call graph, no transitivity, no values, and
its own doc admits the silent path. P1′ makes inference total.

## 1. The domain

Let `𝒦` be the finite set of **capability atoms** — exactly the
`Capability` values constructible from the kernel ontology
(`Read(tag)`, `Write(tag)`, `Network(proto, dir)`, `Spawn(lifetime)`,
`Exec(target)`, `Escalate(realm)`, `Persist(medium)`, `Custom(name)` —
the eight existing arms, instantiated over the finite tag/proto/realm
vocabularies actually registered). No new atom kinds are introduced by
this design (lens 2: the row machinery composes EXISTING atoms; a new
atom kind must independently prove it is not expressible as a
composition — e.g. `may_block` enters in T0850 as a new TEMPORAL class
precisely because no composition of the eight spatial arms expresses
time).

A **ground capability set** `S ⊆ 𝒦` is the lattice element; the
lattice is `(𝒫(𝒦), ⊆, ∪, ∅)` — finite powerset, join = union, bottom =
∅. There is deliberately NO ⊤ element in the surface syntax: the only
way to reach "may do anything" is to LIST it, and the judgment prints
what was listed (lens 1: ⊤ cannot be reached silently because it does
not exist as a nameable point; the worst reachable answer is a large
explicit set, and large explicit sets are legible in diffs).

## 2. Rows

A **row** `ρ` describes the capability surface of a VALUE (usually a
function value) and is one of:

```
ρ ::= ⟨S⟩            -- closed row: exactly the ground set S
    | ⟨S | r⟩        -- open row: S plus whatever the row variable r
                     --   is instantiated to at a use site
```

Row variables `r` are the ONLY polymorphism. There are no row
operations beyond union-with-ground (`⟨S | r⟩` means `S ∪ r`): no row
subtraction, no masking, no duplicate labels, no presence/absence
fields. Justification against the Koka family (lens 2): Koka needs
duplicate labels and masking because handlers DISCHARGE effects — an
inner `catch` removes `exc` from the row. ATS-V-2 capabilities are
NEVER discharged by user code: no construct in the language removes
"this code may open a socket" — confinement changes WHERE the
capability is exercised (delegation ledger), not whether the code has
it. With no discharge there is nothing for masking or duplicates to
express, so they are omitted, and every omission is a thing the duel
cannot find unsound — only insufficient, which §8 tests.

Subsumption: `⟨S₁ | r⟩ ⊑ ⟨S₂ | r⟩` iff `S₁ ⊆ S₂`; closed rows embed as
`⟨S | ∅⟩`. Instantiation substitutes a ground set or another row for
`r`; substitution is monotone in both arguments. (Standard, and boring
on purpose — soundness leans on powerset monotonicity, nothing else.)

## 3. Where rows attach

A function SUMMARY is `σ(f) = ∀r̄. ⟨S_f | mix(r̄)⟩` where:

* `S_f` — f's OWN contribution: ontology atoms from direct primitive
  calls in f's body, plus the resolved summaries of DIRECT calls to
  monomorphic functions.
* `r̄` — one row variable per **capability-bearing parameter** of f: a
  parameter whose type is a function type, or a value type carrying a
  capability-bearing field (transitively; computed from the type,
  §5).
* `mix(r̄)` — the union of those variables that f's body actually MAY
  CALL or hand onward. A capability-bearing parameter that the body
  provably never invokes and never re-exports contributes nothing —
  that is what makes combinators transparent.

The higher-order case that kills naive inference:

```
map : ∀r. fn(List<T>, fn(T)->U ⟨∅ | r⟩) -> List<U> ⟨∅ | r⟩
```

`map` contributes NO atoms of its own; its surface is exactly its
argument's. `arena.alloc` (no fn params) gets the closed
`⟨{Read(Heap),…}⟩`. `spawn(f)` gets `⟨{Spawn(…)} | r_f⟩` — its own
atom PLUS the task body's row. The corpus does not converge to "may do
anything" because every combinator passes rows through instead of
absorbing a ⊤.

## 4. Generalisation — exactly one point

**Rule G (generalise at summary boundaries only):** row variables are
bound (∀-quantified) exactly when a summary is INSTALLED — at
`fn`-item boundaries (a named function's summary enters the module
summary table). They are NEVER generalised at `let` (holon's own
strike, adopted verbatim: let-generalisation would make every local
closure polymorphic and push the polymorphism cost into inference
itself — the value restriction literature's lesson, imported).

A local closure gets a MONOMORPHIC row: the union of what its body
does, with free row variables of the ENCLOSING summary allowed to
occur. It generalises only if it is itself installed as a summary
(returned as part of the module surface, stored in a
capability-bearing field of an exported value — §5 covers both as
value flow).

Accepted cost (duel remark): let-monomorphism coarsens PER-SITE audit
paths — a local `apply` called once with a Read-closure and once with
a Net-closure reports the union on both paths. The JUDGMENT is
unaffected (it is per-atom over the module surface, a union anyway);
the road, if hot local combinators ever need per-site precision, is
re-installing them as summaries — a tooling move, not a law change.

**No-silent-⊤ invariant (lens 1), stated as the testable law:** for
every syntactic position, the inferred row is either (a) a closed
ground set, (b) an open row over variables each traceable to a NAMED
capability-bearing parameter, or (c) a diagnostic. There is no (d).
The three sites where naive designs silently widen — unknown callee,
dynamic dispatch, FFI — are each closed by construction:

* **Unknown monomorphic callee** (call to a function with no summary
  yet): a fixpoint obligation, not a widening — §6.
* **Dynamic dispatch**: the call contributes the PROTOCOL's declared
  max-Shape (ats-v2-inference-first.md §2); a protocol without a
  declared max-Shape makes the call site a DIAGNOSTIC (pin obligation
  on the protocol), never an implicit widening.
* **FFI / extern**: mandatory pin, provenance `Cited{source}` — the
  row is what the pin says, and the Evidence ledger records that it
  was TAKEN ON AUTHORITY, not computed.

## 5. Value flow — capabilities ride on values

The laundering hole (ATS-V-2 §2) is closed by typing VALUES, not call
sites: a type `T` has a **carry set** `carry(T)`:

* `carry(fn-type with row ρ) = ρ`
* `carry(record) = ⋃ carry(fieldᵢ)` — transitively, computed once per
  type, cached; recursive types take the fixpoint (monotone over the
  finite atom set, terminates).
* `carry(primitive) = ∅`.

A value ESCAPES a module when it is returned from an exported
function, stored through an exported field, sent over a channel, or
captured by a spawned task. The module's **exposes** is the union of
(a) its functions' summaries (instantiated at their export types) and
(b) `carry(T)` of every escaping value's type — with the DOMAIN-FLOW
attribution below deciding WHOSE surface a cross-domain escape lands
on. A closure over a socket that leaves the module carries `Network`
in its type — whether or not any exported function ever calls it.

### 5a. The domain-flow law (duel strike 1)

Checking `allow(·)` nesting at manifest load is not enough: VALUES
flow between domains, and two failure modes live in that gap. A
closure-over-socket sent through a channel from D₁ to a sibling D₂
with `Network ∉ allow(D₂)` passes every static check and dies at
runtime on the process filter — exactly the surprise kill ATS-V-2
promises to remove. And a broker that spawns NARROW workers gets the
workers' atoms falsely attributed to its own surface, though those
atoms execute in the reborn child under the child's filter.

One law closes both: for every **cross-domain edge** `e : D_src →
D_dst` (spawn into another domain, channel whose remote end lives in
another domain, exported call across a domain edge) with payload type
`T`:

```
carry(T) ∩ enforced(𝒦) ⊆ allow(D_dst)
```

— otherwise the edge is a DIAGNOSTIC ("this right dies at the domain
boundary"), at compile time, naming the atoms that would die. And
payload atoms are ATTRIBUTED to the executing domain: they join
`exposes` on the D_dst side; the D_src side keeps its own `Spawn`
atom plus the delegation-ledger edge (which the design already
records, with its capability delta). Composition stays
intersection-shaped and Koka-minimal — no subtraction operator enters
the algebra; the §2 omission list survived its strongest candidate
(sandboxing) and is strengthened by having been tried.

### 5b. Erasure does not launder: carry(dyn P) (duel strike 2)

The CALL side of dynamic dispatch cites the protocol's max-Shape
(§4); the STORAGE side must mirror it, or erasure launders: a
closure-over-socket stored into a `dyn P` field erases its row, and a
later downcast recovers a callable with — under a naive `carry(dyn P)
= ∅` — no recorded surface. Therefore:

```
carry(dyn P) = declared max-Shape of P     (Evidence: Cited)
```

and STORING a `dyn P` where `P` declares no max-Shape is the same
diagnostic as calling through one — a pin obligation on the protocol,
mirror-symmetric with the call rule.

## 6. The fixpoint

Per module: process functions in reverse topological order of the
DIRECT call graph; within an SCC, iterate to stabilisation. The
lattice is stated precisely as a PRODUCT (duel remark): per function,
`𝒫(𝒦)` under `⊆` × per-atom provenance under the meet
`Computed ⊓ Cited = Cited` — derivation paths can only DEGRADE
provenance, monotonically. The transfer function is monotone in both
components over a finite product, so termination is Kleene in one
line; the SCC iteration count is bounded by `|𝒦| × |SCC|`.
Declarations (protocol max-Shapes, extern pins) are fixed within a
run; cross-run shifts are what the staleness rule catches.

Cross-module: module summaries are the compilation-order interface —
importing a module consumes its INSTALLED summaries. A summary change
invalidates dependents' judgments (the staleness rule from the
accepted design, carried into the algorithm: a judgment records the
summary-table hash it was computed against).

## 7. Provenance and the judgment

Every row fact carries `Evidence`: `Computed` (this inference derived
it) or `Cited{source}` (extern pins, protocol max-Shapes taken from
declarations). Union propagates the WEAKER provenance per atom — an
atom is `Computed` only if every path deriving it is computed; one
cited edge on every deriving path makes it cited, and the judgment can
print WHY each atom is in the set (the deriving path is the audit
artefact for the ask/diff protocol).

The two-direction judgment (design §2) then compares, per atom, pinned
vs inferred — escalation and dead-right diagnostics both list ATOMS
with their deriving paths, never bare set inequality.

## 8. Nested trust domains and rebirth monotonicity (lens 3)

Domains nest (workspace merges cogs into domains; a domain may contain
a sub-domain with a NARROWER surface). The formalisation's claim:
**rebirth monotonicity survives nesting because enforcement composes
by intersection, not override.** A process's installed filter is
`allow(D) = base ∪ Σ deltas(D)` for the domain D it was born into;
entering a nested narrower domain D′ ⊂ D at runtime may only NARROW
(`allow(D′) ⊆ allow(D)` — installable on a live process, seccomp
stacking), and WIDENING out of D′ — including "back" to the parent's
wider surface — is a rebirth, because the standing filter cannot
grow. The type-level fact making this checkable: domain nesting is
declared in the workspace manifest, so `allow(·)` is a static function
of the manifest, and the checker verifies `D′ ⊂ D ⟹ allow(D′) ⊆
allow(D)` at manifest load — a manifest whose nested domain claims a
WIDER surface than its parent is rejected before anything runs.

One honesty remark (duel): the filter governs SYSCALLS, not live
resources. A socket opened under D and carried into D′ keeps sending
for as long as `allow(D′)` still holds `send` — rights never widen,
but resources persist across the narrowing. Resource carriage across
domain edges is the delegation ledger's jurisdiction (the edge and
its delta are recorded), and the rights-rot law (§6 of the design)
already holds the standing right to account.

## 9. What the duel tried to break (resolved)

1. §2's omission list (no masking/duplicates/subtraction) — find a
   REAL Verum program whose honest surface needs one of them.
2. Rule G — find a let-bound value whose monomorphic row causes a
   false escalation a fn-boundary generalisation would not.
3. §5 carry(T) — find a laundering path that escapes it (existential
   types? `Any`-like erasure? channels of channels?).
4. §6 — find a non-monotone interaction (protocol max-Shapes cited
   into computed unions?) breaking Kleene.
5. §8 — find a nesting/rebirth scenario where intersection composition
   still permits an effective widening without a rebirth.

## 10. Implementation map (after the duel)

* `verum_kernel::arch_rows` — row type, lattice ops, carry(T),
  Evidence-tagged atoms; property tests for §1-§5 laws.
* `verum_compiler::pipeline::ats_v_phase` — replace the flat walk with
  per-fn summaries + SCC fixpoint; keep the ontology as the atom
  source; wire protocol max-Shape citation and the extern-pin
  obligation as diagnostics.
* Both-polarity fixtures (design §8) land with the first enforceable
  class, same commit.
