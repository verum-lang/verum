#!/usr/bin/env bash
#
# vcs/gates/aot_lenient_skip.sh — Production-readiness gate (#176 + #198 gate 2 partial).
#
# AOT lenient-skip ratchet — driven by #176. The skip count is a
# runtime metric from a full-corpus AOT pass, NOT a static grep
# (the skip records are emitted into the runtime telemetry, not
# the source). This gate consumes a snapshot file at
# `vcs/aot_skip_snapshot.txt` produced by the latest CI run.
#
# Snapshot format: a single integer line — the total skip count
# observed during the most recent full-corpus AOT pass. The ratchet
# baseline is recorded in `vcs/aot_skip_baseline.txt` (the value
# below which the next ratchet step has not yet landed).
#
# Per #176 phase A: each waivered skip is enumerated in
# `vcs/aot_skip_waivers.toml` with a tracking task or
# known-resolution-WIP. This gate doesn't parse the TOML; that's
# the next ratchet phase. For now, it pins:
#   * snapshot ≤ baseline (no regression)
#   * snapshot file exists (forces CI to record one)
#
# Exit:
#   0 — green: snapshot ≤ baseline
#   1 — red:  snapshot > baseline OR snapshot missing
#   2 — usage / repo-not-found

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ ! -d "$PROJECT_ROOT/vcs" ]; then
    echo "aot-skip-gate: vcs/ not found at $PROJECT_ROOT" >&2
    exit 2
fi

cd "$PROJECT_ROOT"

baseline_file="vcs/aot_skip_baseline.txt"
snapshot_file="vcs/aot_skip_snapshot.txt"

if [ ! -f "$baseline_file" ]; then
    echo "aot-skip-gate: RED ($baseline_file missing)"
    echo
    echo "Per #176, the ratchet baseline must be recorded at"
    echo "$baseline_file as a single integer line. Initial"
    echo "value: ~107 (the count at #176 ratchet start)."
    exit 1
fi

baseline=$(cat "$baseline_file" | tr -d '[:space:]')

if [ ! -f "$snapshot_file" ]; then
    echo "aot-skip-gate: RED ($snapshot_file missing)"
    echo
    echo "Per #176, the latest full-corpus AOT pass must record"
    echo "its skip count at $snapshot_file. CI is responsible"
    echo "for producing this file before invoking the gate."
    echo "Run the AOT pass via 'make ci-full' or equivalent."
    exit 1
fi

snapshot=$(cat "$snapshot_file" | tr -d '[:space:]')

if [ "$snapshot" -le "$baseline" ]; then
    if [ "$snapshot" -lt "$baseline" ]; then
        echo "aot-skip-gate: GREEN ($snapshot < baseline $baseline — ratchet ready to advance)"
        echo
        echo "Skip count is below the recorded baseline. To advance"
        echo "the ratchet, update $baseline_file to $snapshot."
    else
        echo "aot-skip-gate: GREEN ($snapshot = baseline $baseline)"
    fi
    exit 0
fi

echo "aot-skip-gate: RED (snapshot $snapshot > baseline $baseline)"
echo
echo "Skip count regressed past the #176 baseline. Either:"
echo "  * Fix the underlying defect that introduced new skips"
echo "    (preferred — drives count toward zero)."
echo "  * Add a waiver entry to vcs/aot_skip_waivers.toml with a"
echo "    tracking task ID, then update $baseline_file."
echo
echo "Per-class triage:"
echo "  * 'missing intrinsic'    → depends on #168"
echo "  * 'wrong number of args' → arity-disambiguation guardrail"
echo "  * 'undefined variable'   → ratchet / variant-alias guardrails"
exit 1
