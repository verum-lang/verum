# Red-Team Round 1 — Architectural Guardrails

Tracks #172. Each `.vr` test pins one architecture-level invariant
that the round-1 attack-vector inventory either confirmed defended
or marked as a partial defense needing a guardrail.

## Coverage

| File | Vector pinned |
|---|---|
| `hash_iter_determinism.vr` | Vector 7.2 — hash-table iteration determinism |
| `text_eq_ignore_case_ascii_fast_path.vr` | Section C — Text.eq_ignore_case ASCII fast-path behavioural contract |
| `set_iter_determinism.vr` | Vector 7.2 (extension to Set<T>) — Set iteration determinism + idempotency + member-only-yield |

## Pending vectors needing harness infrastructure

The round-1 vectors that this directory does NOT yet cover all need
infrastructure beyond a `.vr` test file:

- 1.x refinement-under-concurrency — concurrent-write harness.
- 2.1 unsafe→checked aliasing — aliasing analysis tool.
- 2.2 generation-counter rollover — long-running counter probe.
- 5.1 Z3 timeout policy — Z3 timeout return path review in
  `verum_smt/src/z3_backend.rs`.
- 6.1 generic monomorphization with capability — type-system
  audit, not a unit test.
- 7.1 Tier-0 vs Tier-1 divergence — covered by #196.

See `vcs/red-team/round-1-architecture.md` for the full vector
inventory + status matrix.
