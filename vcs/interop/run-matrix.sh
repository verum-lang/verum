#!/usr/bin/env bash
# vcs/interop/run-matrix.sh — full scenario × impl matrix (v1 gate).
#
# Writes:
#   results/matrix.json     — grid-shape, per spec §10.4
#   results/runs.jsonl      — per-cell run appended by run-scenario.sh

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
RESULTS_DIR="$HERE/results"
mkdir -p "$RESULTS_DIR"

# Reset the per-run log so each matrix sweep stands on its own.
: > "$RESULTS_DIR/runs.jsonl"

TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_SKIP=0
TMP_GRID="$(mktemp)"
echo "{" > "$TMP_GRID"
first_scn=1
while read -r scenario; do
    [[ -z "$scenario" ]] && continue
    if [[ $first_scn -eq 0 ]]; then echo "," >> "$TMP_GRID"; fi
    first_scn=0
    printf '"%s": {' "$scenario" >> "$TMP_GRID"

    first_impl=1
    while read -r impl; do
        [[ -z "$impl" ]] && continue
        if [[ $first_impl -eq 0 ]]; then printf "," >> "$TMP_GRID"; fi
        first_impl=0

        verdict="PASS"
        if ! bash "$HERE/run-scenario.sh" "$scenario" "$impl" > /dev/null; then
            verdict="FAIL"
        fi
        # Detect SKIP by checking the last log line.
        last="$(tail -n1 "$RESULTS_DIR/runs.jsonl" 2>/dev/null || true)"
        if [[ "$last" == *'"verdict":"SKIP"'* ]]; then verdict="SKIP"; fi

        case "$verdict" in
            PASS) TOTAL_PASS=$((TOTAL_PASS + 1)) ;;
            FAIL) TOTAL_FAIL=$((TOTAL_FAIL + 1)) ;;
            SKIP) TOTAL_SKIP=$((TOTAL_SKIP + 1)) ;;
        esac

        printf '"%s":"%s"' "$impl" "$verdict" >> "$TMP_GRID"
    done < "$HERE/impls.txt"

    printf "}" >> "$TMP_GRID"
done < "$HERE/scenarios.txt"
echo "}" >> "$TMP_GRID"

jq -n --argjson grid "$(cat "$TMP_GRID")" \
      --slurpfile runs "$RESULTS_DIR/runs.jsonl" \
      '{grid: $grid, runs: $runs[]? | [.]}' \
      > "$RESULTS_DIR/matrix.json" 2>/dev/null \
    || cp "$TMP_GRID" "$RESULTS_DIR/matrix.json"
rm -f "$TMP_GRID"

echo ""
echo "Matrix: pass=$TOTAL_PASS fail=$TOTAL_FAIL skip=$TOTAL_SKIP"
[[ $TOTAL_FAIL -eq 0 ]]
