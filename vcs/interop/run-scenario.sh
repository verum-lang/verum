#!/usr/bin/env bash
# vcs/interop/run-scenario.sh — run a single interop scenario against
# a single peer implementation.
#
# Usage:
#   run-scenario.sh <scenario> <impl>
#
# scenario ∈ scenarios.txt, impl ∈ impls.txt.
#
# Delegates to docker-compose under ../interop-runner-containers/ for
# peer binaries. Exits 0 on PASS, non-zero on FAIL. Writes a single
# JSON line to `results/runs.jsonl` of shape:
#   {"scenario": ..., "impl": ..., "ts": ..., "verdict": ..., "duration_ms": ...}

set -euo pipefail

SCENARIO="${1:?usage: run-scenario.sh <scenario> <impl>}"
IMPL="${2:?usage: run-scenario.sh <scenario> <impl>}"

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
RESULTS_DIR="$HERE/results"
mkdir -p "$RESULTS_DIR"

if ! grep -qxF "$SCENARIO" "$HERE/scenarios.txt"; then
    echo "unknown scenario: $SCENARIO" >&2
    exit 2
fi
if ! grep -qxF "$IMPL" "$HERE/impls.txt"; then
    echo "unknown impl: $IMPL" >&2
    exit 2
fi

# The peer-impl orchestration is expected to live adjacent to the
# Verum repo as its own vendored tree. If missing, emit a SKIP so CI
# can tell the infrastructure hasn't shipped yet (distinct from FAIL).
CONTAINERS="$ROOT/../interop-runner-containers"
if [[ ! -d "$CONTAINERS" ]]; then
    ts="$(date -u +%FT%TZ)"
    printf '{"scenario":"%s","impl":"%s","ts":"%s","verdict":"SKIP","duration_ms":0,"reason":"interop-runner-containers not present"}\n' \
        "$SCENARIO" "$IMPL" "$ts" >> "$RESULTS_DIR/runs.jsonl"
    echo "SKIP $SCENARIO/$IMPL (no interop-runner-containers)"
    exit 0
fi

start="$(date +%s%3N)"
verdict="PASS"
if ! "$CONTAINERS/bin/run" \
        --scenario "$SCENARIO" \
        --client warp --server "$IMPL" \
        --also --client "$IMPL" --server warp \
        > "$RESULTS_DIR/${SCENARIO}_${IMPL}.log" 2>&1; then
    verdict="FAIL"
fi
end="$(date +%s%3N)"
ts="$(date -u +%FT%TZ)"
dur=$((end - start))

printf '{"scenario":"%s","impl":"%s","ts":"%s","verdict":"%s","duration_ms":%d}\n' \
    "$SCENARIO" "$IMPL" "$ts" "$verdict" "$dur" >> "$RESULTS_DIR/runs.jsonl"
echo "$verdict $SCENARIO/$IMPL (${dur}ms)"
[[ "$verdict" == "PASS" ]]
