#!/usr/bin/env bash
#
# vcs/gates/soundness.sh — Production-readiness gate 7 (#198).
#
# Asserts: every `unsafe` block in core/ has a `// SAFETY:` comment
# above it that names the specific invariant it relies on AND points
# to where that invariant is established.
#
# Heuristic: scan core/ for `unsafe {` and `unsafe fn`; for each, check
# that the immediately preceding non-blank line(s) include a
# `// SAFETY:` marker.
#
# Allowlist: prepend `// soundness-allowed:<reason>` on the line above
# the unsafe block to exempt it (rare — should be reserved for
# generated code or trivially-bounded blocks).
#
# Exit:
#   0 — green: every unsafe block has a SAFETY: comment
#   1 — red:  one or more bare unsafe blocks; printed to stdout
#   2 — usage / repo-not-found

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ ! -d "$PROJECT_ROOT/core" ]; then
    echo "soundness-gate: core/ not found at $PROJECT_ROOT" >&2
    exit 2
fi

cd "$PROJECT_ROOT"

# Single-pass scan capturing findings. State machine: track whether
# the most recent non-blank, non-pure-comment line included a
# // SAFETY: or // soundness-allowed: marker; an unsafe opener
# without that prior marker is a violation.
output=$(
    while IFS= read -r f; do
        awk -v file="$f" '
            BEGIN { has_safety = 0; }
            /soundness-allowed:/ { has_safety = 1; next; }
            /\/\/ *SAFETY:/      { has_safety = 1; next; }
            /unsafe[[:space:]]*\{/ || /unsafe[[:space:]]+fn/ {
                if (!has_safety) {
                    printf("%s:%d: bare unsafe block (no // SAFETY: above): %s\n",
                           file, NR, $0)
                }
                has_safety = 0
                next
            }
            /^[[:space:]]*$/ { next }
            /^[[:space:]]*\/\// { next }
            { has_safety = 0 }
        ' "$f"
    done < <(find core/ -name "*.vr" -type f 2>/dev/null)
)

if [ -z "$output" ]; then
    echo "soundness-gate: GREEN (every unsafe block has a // SAFETY: comment)"
    exit 0
fi

count=$(printf "%s\n" "$output" | wc -l | awk '{print $1}')
echo "soundness-gate: RED ($count bare unsafe blocks found)"
echo
echo "Each must have a // SAFETY: comment immediately above naming"
echo "the invariant it relies on, OR a // soundness-allowed:<reason>"
echo "marker (rare)."
echo
echo "$output"
exit 1
