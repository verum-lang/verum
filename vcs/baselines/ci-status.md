# CI Status (2026-04-18, local validation)

The local pipeline that the GitHub Actions `.github/workflows/ci.yml`
runs has been validated against the current `main`.

| Job                              | Status            |
|---|---|
| `unit` — `cargo build --workspace` with `RUSTFLAGS="-D warnings"` | ✅ green (no warnings outside upstream C/C++ build noise) |
| `unit` — `cargo test --workspace --lib --bins`                    | ✅ green (~3000 tests, 0 failed) |
| `lint` — `cargo clippy --workspace --bins --lib`                  | ✅ green (0 warnings outside upstream noise) |
| `vcs-l0-l1` — L0 lexer/parser/types/builtin-syntax                | ✅ 100% (subset measured) |
| `vcs-l0-l1` — L0 across 9 of 10 categories (587 specs)            | ⚠️ 98.7% (8 known stdlib-API + interpreter static-mut gaps) |
| `differential` — Tier 0 vs Tier 1 cross-impl                      | ⚠️ 64.9% (24/37 — 13 tier-consistency gaps; baseline locked) |
| `bench` — micro                                                   | ⚠️ 28/35 = 80% (perf-target misses, pre-existing) |

## What "ready to merge" means right now

The unit / build / clippy / lint pipeline is reliably green —
those gates would block any regression on existing pass rates.

Per-category L0 / differential / bench baselines are committed under
`vcs/baselines/`:

- `l0-baseline-final.md` (587/595 = 98.7%)
- `differential-baseline.md` (24/37 = 64.9%)
- `bench-baseline-micro.json` + `.md` (28/35 perf targets)

`make bench-compare`, `make test-l0`, and `make test-differential`
diff against these and fail the gate on regression.

## Known not-yet-green (tracked, not blocking this baseline)

- **2 mmio specs** (`readonly_write_fail`, `writeonly_read_fail`) —
  type-checker doesn't filter impl blocks by mode-parameter
  instantiation. Per-instantiation impl-block dispatch is required.
- **6 cbgr specs** — depend on `Epoch.advance()` mutating a
  `static mut` counter. Static-mut writes don't propagate across
  frames in the Tier 0 interpreter (AOT honors them). The Epoch
  facade is wired and types/dispatches cleanly; the runtime
  monotonicity invariant needs an interpreter intrinsic for the
  global counter (or a thread-local cell).
- **13 differential specs** — interpreter and AOT diverge on exit
  code / stdout for some patterns. Each needs individual triage.
- **7 micro benchmarks** — pre-existing performance-target misses
  (e.g., large-allocation throughput). Not regressions from this
  session's runtime/codegen fixes.

## How to push to GitHub

This baseline is local. To submit:

```bash
git push origin main
```

Then watch the GitHub Actions tab for the `unit`, `lint`,
`vcs-l0-l1`, and `differential` jobs to all turn green. The session
hasn't pushed because of authorization — the user should review
the local commits first (`git log origin/main..HEAD`) and push when
ready.
