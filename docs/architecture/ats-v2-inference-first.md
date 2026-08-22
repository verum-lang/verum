# ATS-V-2 — Architecture-as-Types, rethought to the foundation

Status: ACCEPTED DESIGN (T0847), duel discharged 2026-08-22. Owner
mandate: *rethink architecture-as-types to its most fundamental basis,
practically useful for both humans and machines; the current
implementation feels not thought through to the end.* This document is
the synthesis of a two-session design duel — this session's 10-point
diagnosis, holon-e2's adversarial review in three rounds (the two-layer
law's final form, the Evidence bridge, the pin-predicate refinements,
and eight accepted strikes including seccomp monotonicity, capability
rows, the cold-path drill law, and the clean-twin polarity). The full
exchange lives in the T0847 journal. The "widening is rebirth" law
(§4b) is additionally a candidate for the THEORY corpus in its own
right.

## 0. The one-sentence thesis

Architecture must flow the same direction as types: **computed from the
code by the compiler, pinned — not invented — by annotations, judged in
both directions, and enforced at every boundary it can physically
reach.**

## 1. Why ATS-V-1 is not thought through to the end

The April design is a *declarative* layer with post-hoc checking: a
human writes `@arch_module(...)`, the compiler audits the claim
(AP-025 DeclarationDrift). Its own history demonstrates the failure
mode this direction of truth invites: the checker phase sat inert while
2311 unearned `Theorem` stamps accumulated (T0834). The revival made
the corpus honest — but a discipline that needs a gate to stay honest
is categorically weaker than one where dishonesty is unrepresentable.

The same season produced the twin lesson at the intrinsic layer
(T0844/T0846): a declaration that nothing cross-checks WILL drift, in
five surfaces at once. And its cure — one authority, machine-judged
agreement, loud refusal instead of fabricated values — is exactly the
cure ATS-V needs, applied one level up.

## 2. The two-layer law (P1′)

Naive inference-first — "compute everything, declare nothing" — is the
INVERSE of the T0834 defect, not its cure: if the Shape is only what
the body implies, *intent* disappears. Code that quietly grows a
capability nobody wanted gets blessed by its own inference; intent
drift becomes invisible. The honest form is a **two-layer law**:

1. **Inferred Shape** — computed by the compiler for EVERY module, from
   the body, always. An unannotated module is not "unknown"; it is
   *derived*, and the audit says so. Machines can read the architecture
   of any code, annotated or not.
2. **Pinned Shape** — the `@arch_module` annotation, now meaning "this
   is the intent I commit to". Mandatory at trust boundaries (see the
   computable predicate below), optional elsewhere.
3. **The judgment runs in BOTH directions:**
   - `inferred ⊃ pinned` → **capability escalation** (the code does
     something the intent never granted) — error, with the exact delta.
   - `pinned ⊃ inferred` → **dead capability** (a right nobody
     exercises) — a finding of its own class, feeding the rot rules
     (§6). Unused `Network(Outbound)` is a liability, not a comfort.

The pin asks the question; the inference answers it; the judgment
compares. (The house law: one carrier holds the QUESTION, not the
answer.)

**The trust-boundary predicate is itself computed** — a pin is
mandatory iff the module's surface crosses a TRUST-DOMAIN edge, or its
inferred Shape carries any enforceable capability class. Three
refinements make that predicate sound (each closes a hole the naive
form has):

- **Inference is value-flow-aware.** Capabilities ride on VALUES, not
  only on direct calls: a closure or handle built over `Network` CARRIES
  `Network` in its type, and handing it anywhere is a capability flow.
  Without this, a module launders its surface by exporting a
  capability-bearing value while its own call graph stays clean. (This
  is the VBC capability token, lifted into inference.)
- **The boundary is the trust-domain map, not the cog.** A vendored
  third-party cog must not enjoy the same leniency as a first-party
  internal one. The workspace manifest declares which cogs share a
  trust domain (default: every cog is its own domain; the workspace may
  merge). A pin is mandatory where a DOMAIN edge is crossed.
- **The enforceable-class list is derived, never hand-written.** What
  counts as "enforceable" is read off the enforcement machinery itself
  (what the sandbox layer and the VBC gates can actually hold) — one
  table, no parallel hand list to drift.

**Scope statement (deliberate, not a hole):** a pure module with no
capabilities and no domain-crossing exports — however
correctness-critical (a canonical ordering comparator) — is honestly
exempt from the pin obligation. The pin law governs the CAPABILITY
surface; correctness is the jurisdiction of the `@verify` ladder and
the proof courts. The two disciplines meet in the Evidence ledger (§7),
not in each other's obligations.

### Inference through the three hard horizons

- **Dynamic dispatch** — inference is not whole-program. A protocol
  declares the **max-Shape of its implementors**; a call through the
  protocol contributes the protocol's declared bound, and every
  implementor is judged against it. Sound, local, and it gives protocol
  authors an architectural surface they always implicitly had.
- **FFI / extern** — inference cannot see across the horizon, and
  pretending otherwise would be fabrication. An extern call site
  carries a MANDATORY pin, and its provenance is `Cited { source }`,
  never `Computed` — the same `Evidence` enum the proof kernel uses
  (T0841). One provenance discipline from the kernel to the
  architecture: `inferred = Computed`, `extern = Cited`.
- **Meta-stages** — inference runs AFTER expansion (the only honest
  point), and every module carries a SECOND shape: the **build-time
  Shape** (§5).

### The inference algorithm's load-bearing choices

- **The lattice is trivial; the polymorphism is not.** Per-function
  capability sets under union with a monotone fixpoint over the call
  graph is the easy part. The part that decides whether inference LIVES
  is higher-order code: a `fn` value that captured `Network` carries it
  — and without capability polymorphism, every combinator (`map`, the
  whole stdlib of higher-order functions) infers a ⊤-shaped Shape, ⊤
  flows through call sites, and the entire corpus converges to "may do
  anything" — drowning the two-direction judgment in false
  escalations. The committed answer: **capability rows**
  (row-polymorphism over capability sets, the Koka family), generalised
  at `fn` boundaries — a combinator is polymorphic in the capabilities
  of its argument and passes them through, contributing none of its
  own.
- **Mutual recursion** is the ordinary fixpoint (iterate to
  stabilisation over the SCC); named here so nobody rediscovers it as a
  surprise.
- **Incrementality is part of the design, not an optimisation.**
  Inference produces per-module summaries; a summary change invalidates
  dependents' judgments. An arch verdict computed against a stale
  summary is the architectural twin of testing a stale binary — the
  staleness rule must be stated and enforced from the first
  implementation, or the checker grows a race.
- **The escalation judgment has a development mode.** Mid-refactor code
  is legitimately wider than its pin for minutes at a time. The
  BOTH-direction judgment hard-fails at the submit boundary of the §3
  cycle (and in CI); in the editor it is a warning. A law that fights
  the developer on every save gets disabled, and a disabled law is
  worse than none.

## 3. One protocol for asking and diffing (P3+P4)

Machines and humans need the same two verbs, and they are one loop:

```
ask → patch → arch diff → counterfactual pre-flight → submit
```

- **Ask**: "what may code at this path do?" — answered identically by
  three transports of ONE vocabulary: `verum arch query --at <path>`
  (CLI), an LSP request (editors, agents), and the `ArchInfo`
  meta-context (the 15th, through the same `using [...]` machinery —
  metaprograms read the architecture in the language itself).
- **Diff**: `verum arch diff <rev>` — the capability-surface delta of a
  change as a first-class artefact: "this PR widens the surface by
  `Network(Outbound)` in cog X, narrows `Read(File)` in Y". CI gets
  exit codes; reviewers get the sentence they actually need; agents run
  it pre-flight as self-restraint before submitting.

The question vocabulary is append-only and stable, under the same
discipline that made AP codes learnable (a model fine-tuned in 2026
still parses the answers in 2031) — with FOSSIL DATES: an entry may be
marked deprecated (never removed, never renumbered), and the mark
carries the date and the successor. Eternal append without a
deprecation story accumulates debris; append-only WITH fossils stays
both stable and navigable.

The protocol carries its own negative fixture in CI: a known-escalating
patch must produce the expected diff and pre-flight verdict on ALL
THREE transports — otherwise the vocabulary drifts between CLI, LSP and
`ArchInfo` while each stays self-consistent (the judge row of the
transport matrix).

## 4. Physical enforcement, honestly scoped (P2)

A declared Shape should HOLD, not merely describe. Verum owns its
entire syscall surface (no-libc), which makes the compilation of
`Shape.exposes` into enforcement uniquely cheap here:

- **Allowlist algebra**: the enforced set = *runtime base* ∪ Σ(module
  deltas). The async runtime's own syscalls belong to the RUNTIME's
  Shape; module judgment judges deltas only.
- **Tier 1**: the entry sequence installs the process filter
  (seccomp-style on Linux via the direct-syscall layer; platform
  analogues where they exist). **Process filters are MONOTONE** — a
  seccomp filter can only narrow, never widen, so "re-derive and
  re-install on hot-reload" is mechanically impossible as first
  drafted. The honest fork, stated explicitly:
  - *(a) Declared-loadable closure*: the entry filter = runtime base ∪
    every DECLARED loadable delta — the loadable set is part of the
    Shape, known before entry. Predictable, but as wide as the
    declaration.
  - *(b) Widening is rebirth* (preferred): a load that legitimately
    needs a new capability class does not mutate the standing process
    — it RESPAWNS one under the new filter. Rights-widening as process
    rebirth composes exactly with rights-as-standings (§6): a new life
    under a new law, never a mutation of the standing one.

  Either way the frozen-boundary defect (a filter installed once while
  later loads go unjudged) is unrepresentable; what is FORBIDDEN is
  pretending a standing filter widened.
- **Tier 0**: the interpreter gates the syscall intrinsic family by the
  running cog's Shape — module-grained enforcement lives HERE.
- **Scope honesty**: the process filter holds the PROCESS boundary;
  module boundaries are Tier-0/tooling territory. The "bribed deputy"
  (a confined module asking a wider-Shaped neighbour over IPC) is not
  pretended away: every channel whose remote end carries a wider Shape
  is recorded as a **delegation edge** in the audit — visibility, not
  prohibition. The edge is recorded at CONNECT time, per connection —
  the remote end is dynamic, and a static per-channel-type record would
  lie about the live topology. Each edge records the capability DELTA
  (what the remote end holds beyond the local one), which makes
  delegation queryable through the §3 vocabulary for free: *"show every
  bridge through which `Network` leaves confinement."*

## 5. Two Shapes per module: run-time and build-time

ATS-V-1 is silent about what a BUILD may do — while metaprograms,
`BuildAssets`, and codegen run with the developer's full ambient
authority. That is precisely the supply-chain surface (network at
build? file writes at build?). ATS-V-2 gives every module a second
shape, `BuildShape`, with the same primitives, the same inference, the
same judgment — and hermetic builds become a *law you can state*
(`BuildShape.exposes == []` for the default profile), not a CI
aspiration.

## 6. Capabilities age: time in the Shape

Two temporal defects ATS-V-1 cannot express:

- **No budgets** — a Shape says *whether* code may block, never *for
  how long*. `may_block`, `may_sleep`, `bounded_by(τ)` become
  capability classes; "time is the last contract".
- **Rights rot** — permissions, once granted, live forever. ATS-V-2
  treats rights as STANDINGS, not grants: a new right enters on
  probation (dead-capability finding if unused after its window); a
  long-unused right is auto-challenged; removal is the default motion.
  Two corrections keep the rot law from eating the COLD PATHS — the
  crash handler's `Exec`, the incident-response module's recovery
  rights, exactly the rights that MUST live while rarely firing:
  - Retention is not a comment; it is **`@retain(reason, drill)`** — a
    cold right stays alive only with a periodically-executed DRILL (a
    fixture that exercises the path). The right lives because the fire
    drill is lived, not because someone once wrote a sentence.
  - Rot clocks tick on **opportunities to use** (executions of the
    module), never on wall-clock releases — otherwise rot punishes
    cold-correct code for the inactivity of its CALLERS.
  The dead-capability direction of the two-layer judgment (§2) is what
  makes all of this computable.
- **Time enforcement is scoped honestly** (the §4 discipline applied to
  §6): there is no seccomp for time. `bounded_by(τ)` is enforced by
  Tier-0 gates and Tier-1 watchdog budgets — stated as such, so the
  temporal rows are never declarations without physics.

## 7. Evidence, not a second logic (P5 resolved)

The structural checker STAYS — it is fast, total, and never times out
(an architectural claim with an SMT timeout would be unreviewable).
What unifies is the **ledger**, not the solver: every arch-checker
verdict is recorded as an `Evidence::Computed` fact; the `@verify`
ladder and the proof kernel can CITE those facts (`Cited { source }`)
wherever an architectural premise enters a proof. One provenance
discipline; two engines; no dogma about one prover.

## 8. The checker guards itself

The T0841/T0834 lesson, generalised: a checker without a negative
control is a future silent liar — and a checker with ONLY a negative
control can be satisfied by screaming at everything. ATS-V-2 ships
**both polarities per enforceable capability class**:

- a **forever-red fixture** — a canonical violating module that MUST
  produce its diagnostic on every run;
- its **clean twin** — a canonical innocent module that MUST pass.

The suite fails when a red fixture goes green AND when a green twin
goes red. (The counterfactual engine's liveness pin covers scenarios;
this covers the checker's own dispatch of every class, in both
directions — the same judge-row discipline §3 demands of the
transports.)

## 9. Conflict seniority

When a profile says `pure-lib` and a legitimate dependency needs a
syscall, SOMETHING must win and the loss must be ledgered. ATS-V-2
carries a seniority register for such conflicts: the resolution (who
yielded, why, decided by whom) is an audit record, not a shrug in a
code review.

## 10. What is deliberately rejected

- **Pure inference with no pins** — erases intent (§2).
- **SMT as the one arch judge** — trades totality for timeouts (§7).
- **Pretending process filters enforce module boundaries** — the
  deputy problem is scoped and ledgered instead (§4).
- **Training-data export as a headline** (P10 of the original list) —
  deferred to last and shipped ADVERSARIALLY (the dataset must carry
  letter-satisfying/intent-breaking negatives, or it trains
  checker-gamers).

## 11. Sequencing (converged priority)

1. **P1′** — two-layer law: inference engine, computable pin
   obligation, both-direction judgment. Foundation for everything.
2. **P3+P4** — the ask/diff protocol (one vocabulary, three
   transports).
3. **§5 + §6 rows in the Shape NOW** — BuildShape and temporal
   capability classes are cheap to lay in the type today and
   ruinous to retrofit.
4. **P2** — physical enforcement, after the Shape model stabilises
   (enforcing an unstable Shape is churn), scoped per §4.
5. **§7** — Evidence bridge.
6. **P7** profiles on top of inference; **P8** provenance (MTAC
   observer bound to real authorship); **P9** counterfactual pre-flight
   API; **P10** adversarial corpus export — in that order.

Each step lands as its own pool task under the T0847 umbrella, with the
forever-red fixtures (§8) accompanying every enforceable class from the
first one shipped.
