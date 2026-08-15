#!/bin/sh
# diff_kill_switch.sh — differential A/B over a corpus, using ONE binary.
#
# An optimisation guarded by a kill switch makes its own control available:
# the same binary can be asked for the new answer and the old one.  That is
# strictly better than comparing two builds, where a difference can always
# be blamed on the build rather than the change (and regularly is —
# see the "A/B killed by a reused artifact" class).
#
# Usage:
#   scripts/ci/diff_kill_switch.sh VERUM_NO_STDLIB_SCOPE <corpus-dir> [limit] [verum-binary]
#
# For every `.vr` under <corpus-dir>, runs `verum check` twice — once with
# the named variable unset (new behaviour) and once with it set to 1 (old
# behaviour) — and compares stdout+stderr+exit code byte for byte.
#
# Exit status is the number of files whose answer CHANGED, so a caller can
# gate on `-eq 0`.  Every difference is named by file and dumped in full,
# because "N differences" without the names is not actionable.
set -u

SWITCH="${1:?usage: diff_kill_switch.sh SWITCH_ENV_VAR corpus-dir [limit] [verum]}"
CORPUS="${2:?corpus directory required}"
LIMIT="${3:-0}"
VERUM="${4:-target/release/verum}"

[ -x "$VERUM" ] || { printf 'diff_kill_switch: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
[ -d "$CORPUS" ] || { printf 'diff_kill_switch: no corpus at %s\n' "$CORPUS" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

report=$(mktemp)/  2>/dev/null || report=/tmp/diff_kill_switch.$$
report="${TMPDIR:-/tmp}/diff_kill_switch.$$"
: > "$report"

# Time is NOT part of the answer, and it leaks into the output twice.
#
#  1. "Finished checking f.vr in 3.46s" — the duration line.
#  2. "2026-08-15T04:33:50.898716Z WARN …" — the tracing timestamp on
#     every diagnostic the compiler logs through `tracing`.
#
# Both differ between two runs of identical work, so comparing them
# verbatim reports a behaviour change on every file that takes measurable
# time or logs a warning.  Measured, in two rounds: the first version of
# this harness called 13 of 80 files CHANGED (all of them the duration
# alone), and the second called 26 of 200 (all of them the timestamp
# alone).  A differential check whose noise floor is that high does not
# answer the question it was built for.
#
# Normalise time, and ONLY time — anything else normalised here would be
# a difference this harness is meant to catch.
normalise() {
    sed -E 's/ in [0-9]+\.[0-9]+s/ in <t>/g
            s/[0-9]+\.[0-9]+ms/<t>ms/g
            s/[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]+Z/<ts>/g'
}

files=$(find "$CORPUS" -name '*.vr' | sort)
[ "$LIMIT" -gt 0 ] 2>/dev/null && files=$(printf '%s\n' "$files" | head -"$LIMIT")

total=0
differ=0
for f in $files; do
  total=$((total + 1))
  dir=$(dirname "$f")
  base=$(basename "$f")
  new=$(cd "$dir" && { timeout 120 "$VERUM" check "$base" 2>&1; printf 'EXIT=%s' "$?"; } | normalise)
  old=$(cd "$dir" && { timeout 120 env "$SWITCH=1" "$VERUM" check "$base" 2>&1; printf 'EXIT=%s' "$?"; } | normalise)
  if [ "$new" != "$old" ]; then
    differ=$((differ + 1))
    printf 'CHANGED %s\n' "$f"
    {
      printf '\n===== %s =====\n--- %s unset (new) ---\n%s\n--- %s=1 (old) ---\n%s\n' \
        "$f" "$SWITCH" "$new" "$SWITCH" "$old"
    } >> "$report"
  fi
done

printf '\ndiff_kill_switch %s: %d file(s) checked, %d changed\n' "$SWITCH" "$total" "$differ"
[ "$differ" -gt 0 ] && printf 'full diffs: %s\n' "$report"
exit "$differ"
