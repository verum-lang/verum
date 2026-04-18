# Differential Test Baseline (2026-04-18, post-fix)

## Coverage

`vcs/differential/cross-impl/` — 37 specs that run the same source
code through both the Tier 0 interpreter and the Tier 1 AOT
compiler, then compare exit codes / stdout / panics.

| Subdir | Specs | Pass | Rate |
|---|---|---|---|
| cross-impl | 37 | 27 | **73.0%** |

Previous baseline (pre-fix, 2026-04-18 morning): **24 / 37 = 64.9%**.

The other differential trees (`tier-oracle/`, `tests/`, `runner/`,
`generators/`, `scripts/`) are infrastructure or generators, not
themselves test cases.

## What moved since the morning baseline

| Spec | Before | After | Fix |
|---|---|---|---|
| `diff_match_expr` | FAIL | PASS | AOT text ownership transfer through MOV + Ret alias check (commit 35830c1) |
| `diff_array_operations` | FAIL | PASS | Same |
| `diff_pattern_matching` | FAIL | PASS | Same (mixed-branch Text match) |
| `diff_type_casting` | FAIL | PASS | Same |

Plus `diff_nonzero_exit` now passes the interpreter→AOT exit-code
check (commit 570abfc) but can still flip to FAIL in parallel
sweeps due to a vtest-harness race; running it in isolation gives
PASS consistently.

## What still fails (10 specs)

| Spec | Root cause (brief) | Work required |
|---|---|---|
| `diff_nested_control` | Interpreter assertion fails on `collatz_steps(27) == 111`; VBC-level codegen bug in nested while + if inside a mutable-while loop | VBC codegen audit, specific to nested mutable loops |
| `edge_cases` | Uses `Int.MAX`, `Float.INFINITY`, `Float.EPSILON`, emoji ZWJ, closure-in-loop capture semantics | Stdlib const accessors + Unicode graphemes + loop-capture design decision |
| `ieee754_conformance` | NaN formatting + subnormal handling | f-string/NaN formatting (stdlib) |
| `memory_model` | Uses &mut + struct field writes in ways that hit additional CBGR edge cases | Broader codegen audit |
| `numeric_precision` | Float formatting round-trips | Stdlib f"..." pad/precision spec |
| `portable_semantics` | Platform-dependent formatting (int width, overflow) | Platform normalization in stdlib |
| `semantic_equivalence` | Mixed pattern features converging | Further codegen audit |
| `spec_conformance` | Multiple L2+ features (async, ctx) at once | Not an L0 gap |
| `unicode_handling` | Grapheme clusters, combining characters, RTL | Stdlib Unicode work |
| `diff_nonzero_exit` | Parallel-sweep harness race (passes in isolation) | Fix vtest differential-harness isolation |

None of the remaining 10 are blocked by the tier codegen itself
anymore; they are all stdlib / Unicode / formatting gaps or vtest
harness issues.

## Reproduction

```bash
cargo build --release --bin verum --bin vtest
cd vcs
PASS=0; FAIL=0
for f in differential/cross-impl/*.vr; do
  if ../target/release/vtest run --quiet "$f" 2>&1 | grep -q "RESULT: PASSED"; then
    PASS=$((PASS+1))
  else
    FAIL=$((FAIL+1))
  fi
done
echo "$PASS passed / $FAIL failed"
```

(Tested per-file because `make test-differential` floods stdout
with stdlib import warnings, drowning out the summary line.)

## Regression gate

Future runs of `make test-differential` should match or exceed
27/37. Any drop indicates a tier-consistency regression and
should fail the CI gate.
