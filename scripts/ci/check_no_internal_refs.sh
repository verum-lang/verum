#!/usr/bin/env bash
# Gate: tracked (publicly visible) files must not reference the internal/
# directory. Standing directive (2026-07-16): state requirements in place, or
# cite a public doc (docs/architecture/*, grammar/verum.ebnf, website:docs/*) —
# never link internal specs. Introduced as task T0142.
set -eu
cd "$(git rev-parse --show-toplevel)"
# Pattern: dir-like (internal/<seg>/) or file-like (internal/<name>.<ext>)
# references; plain English like "internal/protected" does not match.
# Allowlist:
#   .gitignore                 — the ignore rule for internal/ itself
#   k_arch_v_alignment.rs      — the kernel gate names the pattern to forbid it
violations=$(git grep -nE '(^|[^A-Za-z0-9_.])internal/([A-Za-z0-9_-]+/|[A-Za-z0-9_-]+\.(md|pdf|vr|rs|tex|toml|json))' \
  -- ':!.gitignore' \
     ':!crates/verum_kernel/tests/k_arch_v_alignment.rs' \
     ':!scripts/ci/check_no_internal_refs.sh' \
  || true)
if [ -n "$violations" ]; then
  echo "FORBIDDEN internal/-directory references in tracked files:" >&2
  echo "$violations" >&2
  echo "Fix: state the requirement in place or cite a public doc (see CLAUDE.md)." >&2
  exit 1
fi
echo "check-internal-refs: OK"

# -----------------------------------------------------------------------------
# The same rule's other face: a `Spec:` citation that resolves nowhere.
#
# CLAUDE.md asks for a LOGICAL spec name, "never a path", and this is why.
# Every cited spec still exists — inside the gitignored tree this script
# forbids naming — and 256 citations were left pointing at the public paths
# they used to have. So a citation can be neither repointed (that would name
# the forbidden directory) nor followed (what it does name stopped being a
# file), and the check above cannot see it because the dead path does not
# mention that directory at all.
#
# A ratchet, not a sweep: 256 across 228 files is a wide shallow rewrite that
# would collide with every session holding a core/ file. The gate stops it
# growing; the register row (A54) carries the fix.
#
# It reads TRACKED files only, because `git grep` does. A control that adds
# a dangling citation to a NEW file therefore reports nothing and looks like
# a dead gate — append it to a tracked file instead. (Verified both ways:
# rc=1 on a tracked file, back to rc=0 when reverted.)
SPEC_BASELINE=4
spec_dangling=$(git grep -hoE '(//+|/\*+)[[:space:]]*Spec:[[:space:]]*docs/[^[:space:]#,;]+' \
    -- '*.vr' '*.rs' 2>/dev/null \
  | sed -E 's|.*Spec:[[:space:]]*||' | sort -u \
  | while read -r p; do [ -e "$p" ] || echo "$p"; done)
n_spec=$(git grep -cE '(//+|/\*+)[[:space:]]*Spec:[[:space:]]*docs/' -- '*.vr' '*.rs' 2>/dev/null \
  | awk -F: '{s+=$2} END {print s+0}')
n_dangling=0
for p in $spec_dangling; do
  c=$(git grep -cE "Spec:[[:space:]]*${p//./\\.}" -- '*.vr' '*.rs' 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
  n_dangling=$((n_dangling + c))
done
echo "check-spec-citations: $n_dangling of $n_spec \`Spec: docs/…\` citations resolve to no file (baseline $SPEC_BASELINE)"
if [ "$n_dangling" -gt "$SPEC_BASELINE" ]; then
  echo "A new unresolvable Spec: citation. Cite a LOGICAL spec name (CLAUDE.md), not a path." >&2
  exit 1
fi
