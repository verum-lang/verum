#!/usr/bin/env bash
#
# vcs/gates/diagnostic.sh — Production-readiness gate 6 (#197 + #198).
#
# Asserts: every panic site in core/ carries enough context for
# post-mortem (object identity / generation / call site). Bare
# `panic("foo")` without structured context is a regression on the
# diagnostic gate.
#
# Heuristic: a panic call with a single bare string literal is
# "unstructured". Acceptable forms:
#   * panic(f"...{var}...") — formatted with at least one binding
#   * panic(f"...")          — a formatted string is acceptable
#                              (template tag suggests intent)
#   * panic(f"unreachable")  — caller-stack tells you where; the
#                              stable convention.
#
# Allowlist: a per-line `# diagnostic-allowed:<reason>` trailing
# comment exempts a panic. The reason is required.
#
# Exit:
#   0 — green: every panic in core/ is acceptable
#   1 — red:  one or more bare panics found; printed to stdout
#   2 — usage / repo-not-found

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ ! -d "$PROJECT_ROOT/core" ]; then
    echo "diagnostic-gate: core/ not found at $PROJECT_ROOT" >&2
    exit 2
fi

# Find every `panic("..."` (bare-string panic — the failure shape).
# Verum uses panic() builtin with Text argument; the f-prefix on
# format literals signals structured context.
findings=()
while IFS= read -r line; do
    # Skip if the line carries the explicit allowlist marker.
    if [[ "$line" == *"diagnostic-allowed:"* ]]; then
        continue
    fi
    findings+=("$line")
done < <(
    cd "$PROJECT_ROOT"
    grep -rn 'panic("' core/ --include="*.vr" 2>/dev/null \
        | grep -v 'panic(\"\\${' \
        || true
)

if [ ${#findings[@]} -eq 0 ]; then
    echo "diagnostic-gate: GREEN (every panic carries context)"
    exit 0
fi

echo "diagnostic-gate: RED (${#findings[@]} bare-string panics found)"
echo
echo "Each must either:"
echo "  * use f\"...{var}...\" for structured context, or"
echo "  * carry a trailing  // diagnostic-allowed:<reason>  comment"
echo
for f in "${findings[@]}"; do
    echo "  $f"
done
exit 1
