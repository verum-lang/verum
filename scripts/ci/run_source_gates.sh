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

# `--self-test`: the reader's own poles, because a reader that silently
# takes the first line of a continued list reports a SHORTER wall as a
# green one — which is this script's whole subject, applied to itself.
# Three fixtures: a continued list must yield BOTH gates, a junk token
# must REFUSE, and a plain one-line list must still work.
parse_gates() {
    awk '/^gates-source:/{f=1; sub(/^gates-source:[[:space:]]*/, "")}
         f {cont = ($0 ~ /\\[[:space:]]*$/)
            sub(/##.*/, ""); sub(/\\[[:space:]]*$/, "")
            print
            if (!cont) exit}' "$1" | tr ' \t' '\n\n' | grep -v '^$'
}

if [ "${1:-}" = "--self-test" ]; then
    d=$(mktemp -d); bad=0
    printf 'gates-source: check-alpha \\\n            check-beta ## help\n' > "$d/cont"
    printf 'gates-source: check-alpha junk-token\n' > "$d/junk"
    printf 'gates-source: check-a check-b check-c\n' > "$d/plain"
    got=$(parse_gates "$d/cont" | tr '\n' ' ')
    [ "$got" = "check-alpha check-beta " ] || { echo "FAIL cont -> '$got'" >&2; bad=1; }
    got=$(parse_gates "$d/junk" | grep -vc '^check-')
    [ "$got" = "1" ] || { echo "FAIL junk: $got non-gate token(s), expected 1" >&2; bad=1; }
    got=$(parse_gates "$d/plain" | tr '\n' ' ')
    [ "$got" = "check-a check-b check-c " ] || { echo "FAIL plain -> '$got'" >&2; bad=1; }
    rm -rf "$d"
    [ "$bad" = 0 ] && echo "[ok] run-source-gates self-test: 3 case(s) hold" || exit 1
    exit 0
fi

# The dependency list of the `gates-source` target, minus the `##` comment.
#
# LINE CONTINUATIONS ARE PART OF THE LIST. The previous reader was
# `sed -n 's/^gates-source:...//p'`, which takes the FIRST line and
# nothing else — so every gate written after a `\` was invisible to this
# script, and the trailing `\` itself came through as a gate name.
# Measured 2026-09-11: the script listed 33 entries, one of them the
# literal `\`, where make's own dependency list has 36. The four it could
# not see were check-gate-aggregates-invoked, check-register-rows,
# check-test-mounts and check-verdict-phases — and the first of those is
# the gate whose whole job is finding gates nothing runs. `make
# gates-source` was running all 36 the entire time; only this reporter
# was blind, which is the exact failure mode its own header describes,
# one level up.
gates=$(awk '/^gates-source:/{f=1; sub(/^gates-source:[[:space:]]*/, "")}
             f {cont = ($0 ~ /\\[[:space:]]*$/)
                sub(/##.*/, ""); sub(/\\[[:space:]]*$/, "")
                print
                if (!cont) exit}' Makefile |
            tr ' \t' '\n\n' |
            grep -v '^$')

# A LIST THAT PARSED SOMETHING THAT IS NOT A GATE HAS NOT PARSED THE LIST.
# The `\` above reached the runner and was reported as a FAILING gate,
# which reads as a broken check rather than a broken reader. Anything that
# is not a `check-*` name is a parse failure, and a parse failure must
# stop the script rather than be run.
bad=$(printf '%s\n' "$gates" | grep -v '^check-' || true)
if [ -n "$bad" ]; then
    echo "REFUSING TO PASS: parsed a non-gate out of the Makefile's" \
         "gates-source list: $(printf '%s ' $bad)— the reader is wrong," \
         "not the gate." >&2
    exit 2
fi

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
