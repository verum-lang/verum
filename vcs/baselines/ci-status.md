# CI Status (2026-04-18, local validation)

> **This is a dated snapshot, not a live gate.** The figures below were
> measured on 2026-04-18 against the `main` of that day. Treat every
> percentage as a claim about then, and re-measure before citing one.
>
> **It also under-reports the pipeline.** `.github/workflows/ci.yml`
> defines `vr-syntax`, `unit`, `vcs-l0-l1`, `differential`,
> `strict-gates`, `lint` and `arch-guards`; the table below covers a
> subset. In particular the `vcs-l0-l1` job runs **both** `make test-l0`
> and `make test-l1` as 100%-required gates, while only L0 rows appear
> here — L1 is gated in CI but unrecorded in this file.
>
> **L2, L3 and L4 are not run by the workflow at all.** Any acceptance
> phrased "measured in CI baseline" is unreachable for those three until
> the workflow covers them.

The local pipeline that the GitHub Actions `.github/workflows/ci.yml`
runs was validated against `main` as of the date above.

| Job                              | Status            |
|---|---|
| `unit` — `cargo build --workspace` with `RUSTFLAGS="-D warnings"` | ✅ green (no warnings outside upstream C/C++ build noise) |
| `unit` — `cargo test --workspace --lib --bins`                    | ✅ green (~3000 tests, 0 failed) |
| `lint` — `cargo clippy --workspace --bins --lib`                  | ✅ green (0 warnings outside upstream noise) |
| `vcs-l0-l1` — L0 lexer/parser/types/builtin-syntax                | ✅ 100% (subset measured) |
| `vcs-l0-l1` — L0 across 9 of 10 categories (587 specs)            | ⚠️ 98.7% (8 known stdlib-API + interpreter static-mut gaps) |
| `differential` — Tier 0 vs Tier **3** (`make test-differential`, `--tier 0,3`) | ⚠️ 64.9% (24/37 — 13 tier-consistency gaps; baseline locked). **24/37, not 24/259**: `@tier:` is an intersection filter, so the 253 specs declaring `@tier: 0` run single-tier and are never compared; only the 6 declaring `@tier: all` cross tiers. |
| `differential-tiers` — interp vs AOT                              | not run in CI — `vcs/scripts/differential-tiers.sh` is wired into `vcs/Makefile` only, against `differential-tiers-baseline.txt` |

> **The repo numbers the AOT tier two different ways, so do not read a
> tier number as identifying a comparison.** `CLAUDE.md` states "VBC
> interpreter (Tier 0) and AOT-compiled binaries (**Tier 1**)";
> `vcs/differential/README.md` states "the interpreter (Tier 0) and AOT
> compiler (**Tier 3**)"; and `vcs/Makefile` invokes `--tier 0,3`. The
> two differential entries above may therefore be the same comparison
> under different numbering — what distinguishes them is that one runs
> in CI and one does not. Reconciling the numbering is a separate
> question and is not settled here.
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
