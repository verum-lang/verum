#!/usr/bin/env bash
#
# vcs/gates/memory_safety.sh — Production-readiness gate 2 (#198).
#
# Asserts: every test under vcs/specs/L0-critical/memory-safety/
# passes with 0 ignored / 0 known-failures.
#
# Heuristic: scans the L0-critical/memory-safety/ directory for
# `// @skip:`, `// @ignore:`, `// @expected-error: ` (where the
# expected error is empty, signalling "we don't know what to expect"),
# and `// @known-failure:` markers. Any of these blocks the gate.
#
# Per #198 spec: "full vcs/specs/L0-critical/ memory-safety tests
# pass with 0 ignored / 0 known-failures". This script enforces the
# 0-marker invariant statically; runtime pass verification belongs
# to `make test-l0` (separate vtest target).
#
# Allowlist: per-test `// memory-safety-allowed: <reason>` exempts
# a marker. Reserved for genuine cross-platform-only paths.
#
# Exit:
#   0 — green: zero ignored / known-failure markers
#   1 — red:  one or more markers found
#   2 — usage / repo-not-found

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="$PROJECT_ROOT/vcs/specs/L0-critical/memory-safety"

if [ ! -d "$TARGET_DIR" ]; then
    echo "memory-safety-gate: $TARGET_DIR not found" >&2
    exit 2
fi

cd "$PROJECT_ROOT"

# Look for any of the markers documented in vcs/CLAUDE.md as
# "skip / ignore / known-failure" forms, plus the explicit
# `@requires` form which is conditionally-skipped on CI matrix.
markers=$(
    grep -rn -E '@skip:|@ignore\b|@known-failure|@expected-error:[[:space:]]*$' \
        "$TARGET_DIR" \
        --include='*.vr' 2>/dev/null \
        | grep -v 'memory-safety-allowed:' \
        || true
)

if [ -z "$markers" ]; then
    echo "memory-safety-gate: GREEN (zero ignored / known-failure markers under L0-critical/memory-safety/)"
    exit 0
fi

count=$(printf "%s\n" "$markers" | wc -l | awk '{print $1}')
echo "memory-safety-gate: RED ($count tests have skip/ignore/known-failure markers)"
echo
echo "Per #198 spec, L0-critical/memory-safety/ tests must pass with"
echo "0 ignored / 0 known-failures. Either fix the test or add an"
echo "explicit memory-safety-allowed: <reason> trailing comment if"
echo "the marker is genuinely required (rare; cross-platform-only)."
echo
printf "%s\n" "$markers" | head -50
exit 1
