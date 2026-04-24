#!/usr/bin/env bash
# vcs/interop/run-row.sh — run a single scenario against every impl.
#
# Usage:
#   run-row.sh <scenario>

set -euo pipefail

SCENARIO="${1:?usage: run-row.sh <scenario>}"
HERE="$(cd "$(dirname "$0")" && pwd)"

PASS=0
FAIL=0
SKIP=0
while read -r impl; do
    [[ -z "$impl" ]] && continue
    if bash "$HERE/run-scenario.sh" "$SCENARIO" "$impl" > /dev/null; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
done < "$HERE/impls.txt"

echo "$SCENARIO: pass=$PASS fail=$FAIL skip=$SKIP"
[[ $FAIL -eq 0 ]]
