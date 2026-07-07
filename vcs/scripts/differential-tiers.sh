#!/usr/bin/env bash
#
# differential-tiers.sh — Differential interp-vs-AOT conformance gate.
#
# Runs each core-test under BOTH tiers — the VBC interpreter (Tier 0) and the
# AOT native backend (Tier 1) — and fails on ANY per-test divergence: a test
# that passes on one tier but fails, crashes, or disappears on the other.
#
# WHY THIS EXISTS
#   Tier divergence is a kernel incident.  Without a systematic per-test
#   cross-tier check, an entire compiler subsystem can silently miscompile —
#   or never run at all — while the interpreter stays green.  That is exactly
#   how the AOT generic-monomorphization pass went undetected as *permanently
#   dead*: every program's `future.poll()`-style generic protocol call
#   SIGSEGV'd under --aot while --interp printed the right answer, and nothing
#   compared the two.  This gate turns that class of silent drift into an
#   immediate, localized failure.
#
# USAGE
#   vcs/scripts/differential-tiers.sh [FILTER]
#     FILTER   optional core-test filter, e.g. "async/", "control/", "text/"
#
# ENV
#   VERUM_BIN   path to the `verum` binary (default: `verum` on PATH)
#   STRICT_IGNORED=1   treat an `ignored`-vs-`ok` mismatch as a divergence too
#                      (default: an `ignored` test on either tier is skipped,
#                      since a documented single-tier @ignore is intentional)
#
# EXIT STATUS
#   0  every test agrees across tiers (no divergence)
#   1  one or more divergences were found (details printed)
#   2  usage / environment error
#
set -uo pipefail

VERUM="${VERUM_BIN:-verum}"
FILTER="${1:-}"

if ! command -v "$VERUM" >/dev/null 2>&1 && [ ! -x "$VERUM" ]; then
    echo "error: verum binary not found (set VERUM_BIN or put 'verum' on PATH)" >&2
    exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Run one tier, emitting "NAME<TAB>STATUS" for every reported test.
# --test-threads 1 keeps output deterministic and ordered; a mid-run crash
# (e.g. an AOT SIGSEGV) simply truncates the list, so any interp test with no
# AOT line surfaces below as a MISSING divergence — which is the point.
run_tier() {
    local tier="$1"
    rm -rf "${HOME}/.verum/script-cache" 2>/dev/null || true
    if [ -n "$FILTER" ]; then
        "$VERUM" test "$tier" --test-threads 1 --filter "$FILTER" 2>&1
    else
        "$VERUM" test "$tier" --test-threads 1 2>&1
    fi | sed -nE 's/^test ([^ ]+) \.\.\. (ok|FAILED|ignored).*/\1\t\2/p'
}

echo "[diff-tiers] filter='${FILTER:-<all>}'"
echo "[diff-tiers] running interpreter (Tier 0)…"
run_tier --interp | LC_ALL=C sort > "$WORK/interp.txt"
echo "[diff-tiers] running AOT (Tier 1)…"
run_tier --aot    | LC_ALL=C sort > "$WORK/aot.txt"

echo "[diff-tiers] interp reported $(wc -l < "$WORK/interp.txt" | tr -d ' ') test(s); aot reported $(wc -l < "$WORK/aot.txt" | tr -d ' ')."

# Full outer join on the test name; fill absent sides with MISSING.
join -t "$(printf '\t')" -a1 -a2 -e MISSING -o '0,1.2,2.2' \
    "$WORK/interp.txt" "$WORK/aot.txt" > "$WORK/joined.txt"

STRICT_IGNORED="${STRICT_IGNORED:-0}" awk -F'\t' '
    {
        name = $1; itier = $2; atier = $3;
        if (itier == atier) next;
        # A documented single-tier @ignore is intentional unless STRICT.
        if (ENVIRON["STRICT_IGNORED"] != "1" && (itier == "ignored" || atier == "ignored")) next;
        printf "DIVERGENCE  %-64s interp=%-8s aot=%-8s\n", name, itier, atier;
        d++;
    }
    END {
        if (d > 0) { printf "\n[diff-tiers] %d divergence(s) — TIER DIVERGENCE (kernel incident)\n", d; exit 1 }
        else       { print  "[diff-tiers] no divergence — tiers agree"; exit 0 }
    }
' "$WORK/joined.txt"
