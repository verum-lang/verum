# Legacy worktree handoff — 2026-10-05

T1590 records the user-authorized completion of five dormant worktrees.
The review used clean main `a4bac4ad9`. Before changes, all six modified
source/document files and ten untracked build logs were copied with their
original timestamps and SHA-256 verified; binary Git patches and original
branch/HEAD identities were also retained. The original branches remain
unchanged. Four separate snapshot commits preserve the exact historical
source diffs. These are recovery snapshots, not production fixes.

| Worktree | Disposition | Remaining verification |
|---|---|---|
| `verum-wt-lang` | The single uncommitted audit row is preserved on `codex/legacy-verum-wt-lang-snapshot`. Its constructor-ambiguity finding is recorded below. | T1593 owns a current reproduction and exact-owner diagnostic repair; no fresh runtime result is claimed here. |
| `t0129-subslice` | Despite its directory name, the diff is an abandoned reachability-pruning prototype. Preserved on `codex/legacy-t0129-subslice-snapshot`; not merged over the newer body-only lowering. | Existing T0279 records the conservative CallM/foreign-record concern and requires current reachability/behavior controls before adoption. |
| `t0165-bigint` | `resolve_receiver_qualified_method` is already byte-identical in main and its call site is active. Preserved on `codex/legacy-t0165-bigint-snapshot`; no duplicate implementation merged. | T0165's complete Rational acceptance remains separate from the already-landed dispatch helper. |
| `t0172-packedbyte` | No source changes remained. All six build logs were retained in the verified handoff archive and removed from the working directory. | No new packed-array execution claim follows from historical build logs. |
| `t0425-ffi` | All five byte-address header operations already have equivalent arithmetic in main. Preserved on `codex/legacy-t0425-ffi-snapshot`; four build logs retained in the archive. | T0425 was already closed; this comparison is source equivalence, not a new CBGR runtime certification. |

All five worktrees are clean after this handoff. No old implementation was
silently discarded or treated as a new passing fix. The snapshot branches
retain their original base history and must not be merged wholesale into
current main.

## Preserved constructor-ambiguity observation

The historical 2026-09-03 audit described two visible sum types that both
declare `Pending`. Bare `Pending` selected the last declaration; reversing
declaration order changed its owner. The proposed E431 ambiguity diagnostic
was documented but absent from both the diagnostic registry and emit sites.
The 2026-10-05 source review still found no E431 implementation, but did not
rerun that historical program. T1593 requires a fresh reproduction, explicit
`A.Pending` / `B.Pending` controls, expected-type ownership and ordinary/strict
language-law behavior before claiming resolution.

## Why the old pruning patch was not adopted

Its removal of both declarations and bodies predates the current strategy
that retains declarations and limits body lowering. Replaying it would also
reintroduce obsolete linkage-name roots and assumptions about missing-edge
fallbacks. The remaining concern about records originating in FFI or raw
conversion without `New`/`NewG` deserves a focused current test. Preserving
that concern under T0279 does not validate the discarded pruning strategy or
establish a current RSS improvement.
