# Differential Test Baseline (2026-04-18)

## Coverage

`vcs/differential/cross-impl/` — 37 specs that run the same source
code through both the Tier 0 interpreter and the Tier 1 AOT
compiler, then compare exit codes / stdout / panics.

| Subdir | Specs | Pass | Rate |
|---|---|---|---|
| cross-impl | 37 | 24 | **64.9%** |

The other differential trees (`tier-oracle/`, `tests/`, `runner/`,
`generators/`, `scripts/`) are infrastructure or generators, not
themselves test cases.

## What this baseline says

The interpreter and AOT agree on 24 of 37 sample programs. The 13
divergences are real tier-consistency bugs — code that produces
one answer in the interpreter and a different answer (or a panic)
in the native binary. These are critical for production-readiness:
a user who changes `--interp` ↔ `--aot` should get the same answer
modulo platform-specific details.

This is a regression baseline. Future runs of
`make test-differential` should match or exceed 24/37; any drop
indicates a tier-consistency regression and should fail the CI gate.

## Top-level numbers (other categories not measured)

| Level | Specs | This session's measurement |
|---|---|---|
| L0 (9 of 10 categories) | 595 | 587 / 595 = 98.7% |
| Differential (cross-impl) | 37 | 24 / 37 = 64.9% |
| Bench (micro) | 35 | 28 / 35 = 80% (perf targets) |
| L1 / L2 / L3 / L4 | 1115 + 374 + 328 + 80 | not measured |

## Reproduction

```bash
cargo build --release --bin verum --bin vtest
cd vcs
PASS=0; FAIL=0
for f in differential/cross-impl/*.vr; do
  if target/release/vtest run --quiet "$f" 2>&1 | grep -q "RESULT: PASSED"; then
    PASS=$((PASS+1))
  else
    FAIL=$((FAIL+1))
  fi
done
echo "$PASS passed / $FAIL failed"
```

(Tested per-file because `make test-differential` floods stdout
with stdlib import warnings, drowning out the summary line.)
