# The ERGON target: contract dialect, verification gates, and lowering

**Status:** design accepted 2026-07-30; staged in the task pool as T0671–T0677.
**Counterpart:** the FANOS repo's `docs/verum-contract-requirements.md` (G1–G8) — this
document is Verum's answer to it. FANOS takes no cargo dependency on Verum; the
interface is an artefact format plus the conformance suite described there (§6).

## 1. Answers to the open questions

**Q1 — does the compiler have a `--dialect` concept?** No, not today (verified: no
dialect surface in `verum_compiler`, `verum_cli`, or the grammar). The accepted route is
both spellings the requirements anticipate: a CLI flag (`--dialect=contract`) and a
source-level marker, lowered to one internal `Dialect` value carried by the session.
Enforcement is a typed-AST walk (a compiler phase, not a lint): each forbidden construct
produces a compile error naming the construct. The forbidden set is enumerated from the
compiler's own AST/effect tables, never from a hand-maintained parallel list — a
denylist that silently admits one construct is worse than none.

**Q2 — does capability information propagate through the standard library?** The
mechanism exists and is implemented (`crates/verum_types/src/capability.rs`:
attenuation, intersection, propagation through calls). Whether it survives the
*archive/bake* round-trip with enough fidelity for a lowering to read keys off a
FANOS-provided module's signatures is exactly what T0676 verifies with a test before
G4's Option C is declared workable. Do not assume it; measure it.

**Q3 — DIAKRISIS lineage.** Out of scope for the compiler work; flagged to the humans.
Nothing in T0671–T0677 depends on the answer.

**Q4 — prior art for a non-LLVM target.** Two in-tree shapes: `proof_export.rs`
(typed-AST → external proof formats — the right template for a *serialisation* target)
and the VBC archive writer (canonical bytes with a version discipline). The ergon
target follows the proof-export shape: it consumes the **typed AST/HIR**, where
capability types and `decreases` bounds still exist. It must never lower from VBC —
by the time VBC exists, storage keys are dynamic and the static footprint is gone.

## 2. G4 — the design answer: Option C first, Option A as the language's long game

**Option C is accepted as the starting point** (a FANOS-provided module whose eight
effect functions carry signatures naming the keys; the lowering reads the call graph).
Reasons, in the language's own grain:

- It requires nothing from Verum beyond G1–G3, so it cannot block on language design.
- It matches how authors already write (`ledger.transfer(to, amount)`), and semantic
  honesty is a core principle — the key set belongs to the *operation's meaning*,
  which lives in the signature.
- The trust story is unchanged: the chain derives footprints itself; Verum's job is
  only to emit terms the chain accepts, so a call-graph reading is sufficient.

**Option A (parameterised capabilities — `Ledger with [Write(account_of(sender))]`)
remains the long-range expressiveness item**: it would make key-level footprints a
*type-system* fact usable by every backend, not one lowering's convention. It is
deliberately not on the critical path; if pursued later it subsumes C without breaking
it (C's signatures become the sugar).

**A declared footprint will never be added to the artefact** (requirements §1): the
chain derives footprints structurally, and a second source of truth that can disagree
silently is exactly the class of defect this codebase's carried-fact discipline exists
to prevent.

## 3. Stage map (pool tasks)

| Stage | Task | Blocks | What lands |
|---|---|---|---|
| 1 | T0671 | everything | `@verify(thorough)`/`(certified)` wired in the ladder dispatcher; mandatory termination obligations actually gate |
| 2 | T0672 | G6, G2 | `--dialect=contract` + module marker; per-construct determinism errors |
| 3 | T0673 | — | total arithmetic: unproven divisor/overflow are compile errors (refinement obligations) |
| 4 | T0674 | G2 | proved `decreases` bound surfaced as a compile-time constant; "finite, not constant" diagnostic |
| 5 | T0675 | G8 | `--target=ergon`: typed-AST lowering to the ERGON term algebra behind a small documented backend seam |
| 6 | T0676 | — | Option C verified (capability propagation test) + this document kept honest |
| 7 | T0677 | — | byte-identical artefacts, CI-asserted |

G7 (proof export → `Prove`) is deliberately **not** staged: the requirements say not to
build it yet. When FANOS's recursive proof compaction lands, the stable formats
question gets a written answer here first.

Ordering rationale: 1 and 2 are independent and general-purpose language value
(verification and determinism gates useful far beyond contracts); 3–5 consume them.
The byte codec in `fanos-ergon` does not exist yet (verified 2026-07-30 — no
`encode`/`decode` in the crate), so T0675 lands the lowering against the in-memory
term algebra and keeps emission behind the seam until FANOS pins the wire format.

## 4. Hard constraints the lowering enforces (from the requirements, restated as gates)

1. Depth ≤ 3 — ill-typed beyond it, not expensive; the lowering rejects, never trims.
2. No loops/recursion in the emitted term — loops lower to `Seq` of `k` unrollings
   only when T0674 yields a constant `k`; otherwise a plain rejection: *"loop bound is
   not a compile-time constant, so this cannot compile to a contract"*.
3. `Par` only with proven pairwise footprint-disjointness; `Seq` is the safe default.
4. Every state access key static at compile time; a runtime-computed key is a
   front-end rejection with a diagnostic, never a lowering that defers the problem.
5. Reproducibility is a correctness property of the target (T0677), not a nice-to-have:
   the artefact hash is the contract's on-chain identity.
