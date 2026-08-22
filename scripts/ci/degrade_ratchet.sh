#!/usr/bin/env bash
# Semantic-debt ratchet («идея 7», 2026-08-22): the count of
# codegen degrades (unresolved generic calls — every one a place the
# static resolver gave up and the runtime-type-switch or a trap took
# over) may only go DOWN.
#
# The budget lives in scripts/ci/degrade_budget.txt. A run that
# EXCEEDS it fails ("you added semantic debt"); a run strictly BELOW
# it also fails with instructions to tighten the budget — the ratchet
# clicks, it never rests loose.
#
# Usage: scripts/ci/degrade_ratchet.sh <verum-binary>
set -euo pipefail
VERUM="${1:?usage: degrade_ratchet.sh <verum-binary>}"
BUDGET_FILE="$(dirname "$0")/degrade_budget.txt"
BUDGET="$(cat "$BUDGET_FILE")"

# The fixed probe corpus: programs that pull substantial stdlib
# surface through AOT. Extend deliberately; every addition re-baselines.
PROBES=(
  "vcs/specs/L0-critical/vbc/e2e/aot/1097_arena_allocator.vr"
)

REPORT="$(mktemp)"
export VERUM_DEGRADE_REPORT="$REPORT"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK" "$REPORT"' EXIT
for p in "${PROBES[@]}"; do
  # Cache-bust: the AOT object cache keys on CONTENT, so an unchanged
  # probe skips lowering entirely and a skipped lowering writes NO
  # degrade report — the counter would read 0 and the ratchet would
  # demand a lie (the probe-cache trap, institutionalised away).
  # Salt a COPY with a unique comment; the original tree is untouched.
  salted="$WORK/$(basename "$p")"
  { cat "$p"; echo "// ratchet-salt $$-$RANDOM"; } > "$salted"
  "$VERUM" build "$salted" >/dev/null 2>&1 || {
    echo "degrade-ratchet: probe failed to BUILD: $p" >&2
    exit 2
  }
done

TOTAL=$(python3 - "$REPORT" <<'PY'
import json, sys
total = 0
with open(sys.argv[1]) as f:
    for line in f:
        line = line.strip()
        if line:
            total += json.loads(line)["reachable"]
print(total)
PY
)

echo "degrade-ratchet: reachable degrades = $TOTAL, budget = $BUDGET"
if [ "$TOTAL" -gt "$BUDGET" ]; then
  echo "degrade-ratchet: FAIL — semantic debt grew ($TOTAL > $BUDGET)." >&2
  echo "  Every unresolved generic call is a site where static resolution" >&2
  echo "  gave up. Fix the type-carry, don't raise the budget." >&2
  exit 1
elif [ "$TOTAL" -lt "$BUDGET" ]; then
  echo "degrade-ratchet: debt went DOWN ($TOTAL < $BUDGET) — click the ratchet:" >&2
  echo "  echo $TOTAL > $BUDGET_FILE   (commit it with your change)" >&2
  exit 1
fi
echo "degrade-ratchet: OK (at budget)"
