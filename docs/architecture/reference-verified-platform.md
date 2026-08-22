# The Reference Verified Platform

*Design document — joint draft (holon: purpose, audit, roadmap;
verum session: language contract, subsystem architecture). Born
2026-08-22 from the owner's mandate: «we are building the
reference verifiable systems language of the future — we set the
industry standards». Status: DRAFT under joint revision; the
facts base is the triad audit
(`omega/reference/audits/audit-2026-08-22-verification-triad.md`).*

---

## 1. Purpose — what the mathematical houses ARE to the platform

A verifiable systems language cannot outsource its own
foundations. Verum carries them as **verified artifacts inside
the platform itself**:

- **math-msfs — the metatheory of foundation choice.** The
  machine-checked map of the space of foundations and its edge:
  the platform's official answer to *what the kernel stands on*
  (ZFC + 2 inaccessibles), *what that costs*, and *what changing
  it would mean*. The mechanical joint already exists in the
  language: the Foundation profile of `@arch_module` plus
  AP-005/AP-019 (foundation drift / downgrade) make «what we
  stand on» a checkable declaration of every cog — msfs is the
  load-bearing document behind that check. The bootstrap loop is
  deliberate: the msfs corpus is verified by an audit bundle
  that itself stands on the foundation msfs justifies
  (bootstrapping honesty).
- **diakrisis — the multi-foundation subsystem.** The formal
  articulation of foundations (gauge classes ZFC / HoTT / CIC /
  ∞-Topos and their transitions) is not philosophy on the side:
  it is the layer that lets ONE corpus stand over DIFFERENT
  back-end foundations. Its engineering projection is the
  cross-format emitter matrix: **each external checker (Coq,
  Lean, Agda, Isabelle, Dedukti) is a gauge class, and a theorem
  that survives translation into five checkers is gauge
  coherence measured by execution.** The T0838 deep design
  (TheoremSpec v2) is therefore the subsystem's data model, not
  a gate's tail.
- **math-foundations — the pedagogical-foundational layer.** 18
  sections from apeiron to autogeny, with gateways into msfs /
  diakrisis / UHM. Its python instruments (13 verifiers: PNT,
  zeta, simplicial, …) become a LEGAL verification class —
  the numeric-witness rung of the @verify ladder (below formal,
  above structural). Its verum corpus is strictly selective
  (load-bearing [Т] theorems), never a bulk scaffold: growth
  only together with proofs (the 2311-stamp scar).

## 2. Contract — the canon of a reference corpus

Every corpus claiming the reference bar satisfies, in order of
load-bearing weight:

1. `@arch_module` on every cog with an honest lifecycle:
   Definition until proofs exist; Theorem ONLY with the full CVE
   triple. This is a live compiler gate (AT-2 / AP-010), not a
   convention.
2. Content/aggregator split (msfs 11+4 is the model): an
   aggregator sets boundaries and claims no proofs.
3. The CVE triple of every theorem module: **C**onstructive = a
   named principal construction; **V**erifiable = a rung of the
   @verify ladder (formal at minimum for load-bearing, certified
   for [Т]); **E**xecutable = a make target of the corpus audit.
4. `verify` in CI: the [Т] status requires 100% of goals proved,
   otherwise demotion to Conditional with NAMED assumptions.
   («41.2% with one root» does not live at the reference bar.)
5. The full L4 audit bundle (all 20 gates), hard core:
   kernel-series, differential kernel (+lean twin), signatures,
   apply_graph, proof_term_library (adversarial ≥ 10; schema law
   «accept old, emit new»), arch_corpus with 0 dependency cycles
   (mounts point at declaring modules; re-exports keep old
   spellings alive — the T0836 lesson), cross_format.
6. RU/EN 1:1 of the human texts; corpus-check in CI.
7. Profile strictness (candidate, §4E): `profile = "research"`
   implies verify is mandatory in check.

## 3. Architecture — the subsystems (language side)

- **(A) Foundations Backend Registry.** The five external
  checkers as first-class objects: pinned version, capability
  manifest («what is expressible»), and the gauge class from
  diakrisis. One registry read by roundtrip, replay, and future
  verify routing.
- **(B) TheoremSpec v2 = spec + cited.** `cited: Vec<(name,
  per_backend_sig, foundation_requirements)>` — one data
  structure that closes the T0838 deep half, carries the
  diakrisis gauge data, and retires the E401 emission class
  permanently. The 36×5 emitter matrix is the subsystem's first
  silicon.
- **(C) Transparency artifacts.** `proof-honesty.json` +
  `bundle.json` become versioned, published release artifacts;
  their schemas freeze under «accept old, emit new».
- **(D) Verify diagnostics name the goal.** E0319 prints the
  goal, its position, and the nearest missing fact — a proof
  debt one can act on. Two open (D) items, both found by the
  first mf-corpus verify (2026-08-22): (D1) *predicate goals
  over record witnesses await their reducer* — an applied
  axiom's ensures matching the goal verbatim still leaves the
  goal unproved (the green class today is primitive
  refinements, e.g. Int bounds); prime suspects per the code
  map: requires-hypotheses not asserted into the context as
  uninterpreted applications after the apply peel, or the goal
  form gate before try_apply — diagnosis in progress by
  instrumented replay, and corpora deliberately do NOT work
  around it with field-level goal forms (a workaround would
  mask the hypothesis-channel bug from the lint). Root
  CANDIDATE (T0842, from the reflection WARN read as
  evidence): reflection runs a SINGLE pass — P calling a
  not-yet-reflected Q is rejected instead of iterating to a
  fixpoint over the call graph (the same closure-by-set-not-
  by-pass class as transitive re-exports/T0566 and the removed
  MAX_DEPTH); if confirmed, both (D1) layers share one root —
  an unreflected aggregator is an opaque app with no defining
  axiom, hence unproved under verbatim ensures. Field-level
  aggregator conjunctions are then a SYMPTOMATIC alignment to
  single-pass reflection; the canon lands after the code
  verdict; (D2) *the
  prover does not explain itself* — proof_search has zero
  tracing today; minimal tracing lands with the E0319 work so
  a future «unproved» always names its break point.
- **(E) Strictness profiles as a gated ladder** declared in
  `verum.toml`: `research` (verify mandatory) →
  `load-bearing` (full L4 bundle mandatory). Semver-like: the
  profile is a public promise, and CI enforces it.

## 4. Bar — the industry standard, each plank with its scar

- **(i) Determinism of the whole chain.** Every toolchain pinned
  by an authoritative file: rust pinned, lean pinned — provers
  NOT yet (apt coq 8.x vs local Rocq 9.1 is a compat patch over
  a missing pin). Canon: prover pins per corpus (deb/opam
  versions in a file).
- **(ii) Schema evolution law** «accept old, emit new» for ALL
  certificate formats (scar: FV-19 silently invalidated 13
  canonical .vproof).
- **(iii) A gate that has never been green is a statement, not a
  gate.** Four born-broken gates were exposed in one day (replay
  since birth; cross-format since birth; 30-min budgets under a
  65-min build; a roundtrip report never written). Canon:
  **gate-manifest** — CI publishes, per gate, the date of first
  green and last green (certificate transparency of the CI
  itself).
- **(iv) Autopsy standard.** Any runner crash must leave a stack
  (the T0837 arming), in CI canon.
- **(v) Honesty as a scalable discipline.** The reference
  platform makes a dishonest status INEXPRESSIBLE (live AT-2),
  not merely frowned upon (scar: 2311 Theorem stamps demoted).
- **(vi) Provenance tiers** (from diakrisis methodology): five
  origin tiers of a claim (derived / checked / constructed /
  cited / inherited), the fossil audit (transmission graph →
  components with no incoming checks → a deadline to meet a
  check), the clock trap (checking a corpus by its own outputs
  measures drift, not truth) and the echo trap (a citation
  repeated N times is one citation).

## 5. Federation — how the houses connect

- diakrisis → msfs: git submodule + `verum.toml` path onto the
  submodule (sha = version; reproducible today, migrates to the
  cog registry later without structural change). The current
  path+symlink chain is the fragility the 10×E401 day priced.
- Inter-corpus mounts ONLY through a corpus's public root API —
  never deep paths.
- mf gateways (sections 14/15/16) acquire machine links once the
  mf corpus exists: the executable leg of a gateway claim is the
  target corpus's make goal.

## 6. Roadmap (phases; the audit is the facts SSOT)

- **F0 — language debts (verum session, in flight):** T0838
  emitter matrix (the last msfs L4 gate; deep half = (B));
  T0837 runner segv; verify diagnostics (D); T0839 tail; T0835
  rename (owner's naming pending).
- **F1 — msfs to the reference bar:** afnt_alpha's 2 goals
  (prove or Conditional with named assumptions); Never→Never_;
  after T0838 — the L4 bundle fully green; profile strictness.
- **F2 — diakrisis to the canon:** CVE triples for all theorem
  modules (the msfs 11+4 campaign as the template); submodule
  federation; a MACHINE catalog↔corpus correspondence lint
  (121 catalog / 137 corpus / «142» claimed — reconcile);
  verify + bundle in CI; EN layer of the texts; enrichment:
  close N-04a and N-10 or mark them programs honestly.
- **F3 — the selective mf corpus:** G-49/50/51 ([Т]-closed: PNT
  with π(10⁷) exact, Sobolev–Tonelli, …) plus load-bearing
  XIV/XVIII, each with a full triple; python instruments as the
  numeric-witness rung; gateway machine links.
- **F4 — the standard itself:** this document graduates from
  DRAFT when F0–F2 land; the gate-manifest, prover pins, and
  transparency artifacts ship with it. The platform then states
  its bar publicly: *a corpus is what its gates prove, a gate is
  what its manifest shows, and a foundation is what its
  metatheory verifies.*
