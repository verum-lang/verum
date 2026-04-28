#!/usr/bin/env bash
#
# vcs/gates/documentation.sh — Production-readiness gate 5 (#198).
#
# Asserts: every `public fn` / `public type` / `public axiom` in core/
# has a doc comment immediately above (`///`-prefix lines).
#
# Per #198 spec, the bar is higher (every public API has a doc comment
# AND an example AND a contract section). This MVP gate enforces the
# baseline — every public surface has SOME doc comment. Future
# extensions tighten to require example blocks + contract sections.
#
# Allowlist: prepend `// doc-allowed:<reason>` immediately above the
# public surface to exempt it. Should be reserved for trivial
# constructors / internal-but-public-because-cog-export cases.
#
# Exit:
#   0 — green: every public surface has a doc comment
#   1 — red:  one or more bare public surfaces; printed to stdout
#   2 — usage / repo-not-found

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ ! -d "$PROJECT_ROOT/core" ]; then
    echo "documentation-gate: core/ not found at $PROJECT_ROOT" >&2
    exit 2
fi

cd "$PROJECT_ROOT"

# State machine across each .vr file:
#  - has_doc tracks whether the most recent non-blank, non-comment
#    line was a `///` doc-comment.
#  - On a `public fn` / `public type` / `public axiom` opener,
#    require has_doc; otherwise emit a finding.
output=$(
    while IFS= read -r f; do
        awk -v file="$f" '
            BEGIN { has_doc = 0; }
            /doc-allowed:/        { has_doc = 1; next; }
            /^[[:space:]]*\/\/\// { has_doc = 1; next; }
            /^[[:space:]]*public[[:space:]]+(fn|type|axiom|theorem)\b/ {
                if (!has_doc) {
                    printf("%s:%d: undocumented public surface: %s\n",
                           file, NR, $0)
                }
                has_doc = 0
                next
            }
            /^[[:space:]]*$/        { next }
            /^[[:space:]]*\/\//     { next }
            { has_doc = 0 }
        ' "$f"
    done < <(find core/ -name "*.vr" -type f 2>/dev/null)
)

if [ -z "$output" ]; then
    echo "documentation-gate: GREEN (every public surface in core/ has a doc comment)"
    exit 0
fi

count=$(printf "%s\n" "$output" | wc -l | awk '{print $1}')
echo "documentation-gate: RED ($count undocumented public surfaces)"
echo
echo "Each public fn/type/axiom/theorem in core/ must have a"
echo "/// doc comment immediately above. Future extensions also"
echo "require an example block + contract section per #198 spec."
echo
echo "Allowlist via  // doc-allowed:<reason>  on the line above"
echo "(rare — reserve for trivial constructors)."
echo
# Print only first 50 findings to keep output manageable; full
# count is in the header.
printf "%s\n" "$output" | head -50
if [ "$count" -gt 50 ]; then
    echo
    echo "... ($((count - 50)) more findings)"
fi
exit 1
