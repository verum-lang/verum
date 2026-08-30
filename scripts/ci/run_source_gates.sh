#!/usr/bin/env bash
# Run every source-only gate, keep going past failures, print a summary.
#
# `make gates-source` stops at the FIRST failing gate, so the gates listed
# after it are never run — and a gate that did not run is indistinguishable
# from a green one in the output. Measured 2026-08-30: the wall had five
# failures, and only the first was visible; the other four surfaced only by
# invoking each target by hand.
#
# The gate list is READ FROM THE MAKEFILE, not copied here. A second copy
# of the list would drift the moment someone adds a gate to one of them —
# the same defect this script exists to expose, one level up.
#
#   scripts/ci/run_source_gates.sh              # run all, summarise
#   scripts/ci/run_source_gates.sh --list       # just print the gate list
#
# Exit status: 1 if any gate failed, 0 if all passed.

set -uo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO"

# The dependency list of the `gates-source` target, minus the `##` comment.
gates=$(sed -n 's/^gates-source:[[:space:]]*//p' Makefile |
            sed 's/##.*//' |
            tr ' ' '\n' |
            grep -v '^$')

if [ -z "$gates" ]; then
    echo "REFUSING TO PASS: no gates parsed out of the Makefile's" \
         "gates-source target — the list moved and this script went blind." >&2
    exit 2
fi

if [ "${1:-}" = "--list" ]; then
    echo "$gates"
    exit 0
fi

count=0
failed=0
declare -a failures=()

for gate in $gates; do
    count=$((count + 1))
    if out=$(make "$gate" 2>&1); then
        printf '  %-38s ok\n' "$gate"
    else
        printf '  %-38s FAIL\n' "$gate"
        failed=$((failed + 1))
        failures+=("$gate")
        # First meaningful line of the failure, for the summary.
        detail=$(printf '%s\n' "$out" |
                     grep -vE '^(python3|bash|make(\[|:))' |
                     grep -v '^[[:space:]]*$' |
                     head -1)
        printf '  %-38s   %s\n' "" "${detail:0:100}"
    fi
done

echo
if [ "$failed" -eq 0 ]; then
    echo "source gates: all $count green"
    exit 0
fi

echo "source gates: $failed of $count FAILED — ${failures[*]}"
exit 1
