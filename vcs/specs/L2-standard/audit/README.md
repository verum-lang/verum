# L2-Standard — Audit-Gate Pins

VCS regression pins for `verum audit --*` gates landed across tasks
#154 / #162 / #163.  Each `.vr` file documents the manifest shape
+ live MSFS headline for one audit gate so any silent drift surfaces
in code review (and, once `vtest` gains an `@audit-cmd:` directive,
in CI).

## Coverage

| File | Audit gate | Task | Pinned headlines |
|---|---|---|---|
| `kernel_v0_roster_pin.vr`     | `verum audit --kernel-v0-roster`     | #154 | 10 rules / 4 Proved / 6 Admitted |
| `foundation_profiles_pin.vr`  | `verum audit --foundation-profiles`  | #163 | 10-profile classifier / 5 distinct foundations populated / 388 citations / coherent |
| `codegen_attestation_pin.vr`  | `verum audit --codegen-attestation`  | #162 | 6 passes / 0 of 6 attested baseline |

## Why type-check pins (not integration runs)

Each gate runs in Rust (`crates/verum_cli/src/commands/audit.rs`)
and walks `.vr` files on disk; spawning the `verum` binary or
linking against `verum_cli` from `vtest` is not yet wired into the
VCS runner.  These pins document the manifest invariants in source
form so any drift forces a lockstep update with the Rust manifest —
the regression discipline the gates exist to enforce.

## Upgrade path

Once `vcs/runner/vtest/` gains an `// @audit-cmd: <command>`
directive (TODO referenced in each `.vr` file's docstring), each
pin should:

1. Switch `// @test: typecheck-pass` → `// @test: integration`.
2. Add `// @audit-cmd: verum audit --<gate> --format json`.
3. Replace the `assert(N == N)` placeholder calls with assertions
   on the JSON output's `pass_count` / `proved_count` / `by_foundation`
   keys.

Until then these files serve as canonical, parseable, in-tree
documentation of each gate's V0 contract.
