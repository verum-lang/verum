#!/usr/bin/env bash
# AOT determinism + regression gate (AOT-DETERMINISM-GATE-1).
#
# Motivation: the 2026-07-06 meta conformance wave found two AOT failure
# shapes that a single-run suite cannot distinguish or catch:
#   1. NONDETERMINISM — a test set that changes run-to-run (historically
#      caused by type-id collisions and uninitialized Pack headers: the
#      failing-test SET was load-order/ASLR dependent).
#   2. SLOW REGRESSION — new AOT failures landing while a long parity
#      effort (META-AOT-PARITY-1) still tolerates a known-failure set.
#
# This gate runs the AOT suite N times (default 2) over a filter
# (default "meta/"), then:
#   * FAILS (exit 2) if the failing-test set differs BETWEEN runs
#     (nondeterminism — always a bug, never acceptable);
#   * FAILS (exit 1) if the (stable) failing set contains tests NOT in
#     the checked-in baseline (regression against the parity frontier);
#   * WARNS (exit 0) if baseline entries now PASS (progress — tighten
#     the baseline with --update-baseline).
#
# Usage:
#   bash vcs/scripts/aot-determinism-gate.sh [--filter meta/] [--runs 2]
#       [--baseline core-tests/meta/AOT_BASELINE.txt]
#       [--update-baseline]      # rewrite baseline from this run's result
#       [--parse-only LOG ...]   # dev aid: parse saved suite logs instead
#                                # of running (validates flip detection)
#
# Environment:
#   VERUM_BIN   path to verum binary (default: target/release/verum)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

FILTER="meta/"
RUNS=2
BASELINE="core-tests/meta/AOT_BASELINE.txt"
UPDATE_BASELINE=0
PARSE_ONLY=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --filter) FILTER="$2"; shift 2 ;;
    --runs) RUNS="$2"; shift 2 ;;
    --baseline) BASELINE="$2"; shift 2 ;;
    --update-baseline) UPDATE_BASELINE=1; shift ;;
    --parse-only) shift; while [[ $# -gt 0 && "$1" != --* ]]; do PARSE_ONLY+=("$1"); shift; done ;;
    *) echo "unknown arg: $1" >&2; exit 64 ;;
  esac
done

if [[ -n "${VERUM_BIN:-}" ]]; then
  VERUM="$VERUM_BIN"
else
  VERUM="$ROOT/target/release/verum"
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/aot-gate.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# Extract the sorted failing-test list from a suite log.
extract_failures() { # $1 = log file, $2 = out file
  # Harness lines look like: "test meta/x/unit_test::name ... FAILED (12.3s)"
  sed -n 's/^test \([^ ]*\) \.\.\. FAILED.*/\1/p' "$1" | LC_ALL=C sort -u > "$2"
}

declare -a FAIL_FILES=()

if [[ ${#PARSE_ONLY[@]} -gt 0 ]]; then
  echo "==> parse-only mode over ${#PARSE_ONLY[@]} saved log(s)"
  i=0
  for log in "${PARSE_ONLY[@]}"; do
    i=$((i+1))
    extract_failures "$log" "$WORK/fails.$i"
    FAIL_FILES+=("$WORK/fails.$i")
    echo "    run $i: $(wc -l < "$WORK/fails.$i" | tr -d ' ') failures ($log)"
  done
else
  [[ -x "$VERUM" ]] || { echo "verum binary not found at $VERUM" >&2; exit 66; }
  for ((i=1; i<=RUNS; i++)); do
    echo "==> AOT run $i/$RUNS (filter: $FILTER)"
    # The suite exits non-zero when tests fail; the gate's own verdict
    # comes from set comparison, so tolerate the exit code here.
    "$VERUM" test --aot --filter "$FILTER" > "$WORK/run.$i.log" 2>&1 || true
    extract_failures "$WORK/run.$i.log" "$WORK/fails.$i"
    FAIL_FILES+=("$WORK/fails.$i")
    echo "    $(tail -n 2 "$WORK/run.$i.log" | head -n 1)"
  done
fi

# --- Verdict 1: determinism across runs ---------------------------------
FIRST="${FAIL_FILES[0]}"
NONDET=0
for f in "${FAIL_FILES[@]:1}"; do
  if ! cmp -s "$FIRST" "$f"; then
    NONDET=1
    echo ""
    echo "!! NONDETERMINISM: failing-test set differs between runs"
    echo "-- only in run 1:"
    comm -23 "$FIRST" "$f" | sed 's/^/     /'
    echo "-- only in run $(basename "$f" | sed 's/fails\.//'):"
    comm -13 "$FIRST" "$f" | sed 's/^/     /'
  fi
done
if [[ $NONDET -eq 1 ]]; then
  echo ""
  echo "GATE: FAIL (nondeterministic AOT results — see diff above)"
  exit 2
fi
echo "==> determinism: OK (identical failing set across ${#FAIL_FILES[@]} run(s))"

# --- Verdict 2: regression vs baseline ----------------------------------
if [[ $UPDATE_BASELINE -eq 1 ]]; then
  {
    echo "# AOT known-failure baseline (AOT-DETERMINISM-GATE-1)."
    echo "# One test id per line. Regenerate: vcs/scripts/aot-determinism-gate.sh --update-baseline"
    echo "# Updated: $(date -u +%Y-%m-%dT%H:%M:%SZ) filter=$FILTER"
    cat "$FIRST"
  } > "$BASELINE"
  echo "==> baseline updated: $BASELINE ($(wc -l < "$FIRST" | tr -d ' ') entries)"
  exit 0
fi

if [[ ! -f "$BASELINE" ]]; then
  echo "==> no baseline at $BASELINE — treating current failing set as informational"
  echo "    (create one with --update-baseline once the set is triaged)"
  exit 0
fi

grep -v '^#' "$BASELINE" | LC_ALL=C sort -u > "$WORK/baseline"
NEW_FAILS="$(comm -13 "$WORK/baseline" "$FIRST" || true)"
FIXED="$(comm -23 "$WORK/baseline" "$FIRST" || true)"

if [[ -n "$FIXED" ]]; then
  echo "==> progress: $(echo "$FIXED" | wc -l | tr -d ' ') baseline entries now PASS:"
  echo "$FIXED" | sed 's/^/     /'
  echo "    (tighten with --update-baseline)"
fi

if [[ -n "$NEW_FAILS" ]]; then
  echo ""
  echo "!! NEW AOT failures not in baseline:"
  echo "$NEW_FAILS" | sed 's/^/     /'
  echo ""
  echo "GATE: FAIL (AOT regression vs $BASELINE)"
  exit 1
fi

echo "GATE: PASS"
