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
SPEC_BASELINE=0
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

# -----------------------------------------------------------------------------
# A third face of the same rule: a citation to a MACHINE-LOCAL memory file.
#
# Sessions keep working notes in a gitignored, per-machine directory. Fourteen
# tracked files had come to cite it — `memory/<slug>.md`, or `MEMORY.md`, or a
# task number that only means something inside it. The `internal/` check above
# cannot see them: the path never contains "internal/".
#
# It is the worse half of the two. An `internal/` reference at least names a
# directory the reader can be told about; `memory/callg_emission_fix_blueprint
# _2026-05-19.md` names a file that exists on exactly one machine, and the
# reader cannot even discover that it is unreachable — it reads like a repo
# path. One of them was load-bearing: the comment it replaced was the only
# statement of why a compiler intercept hard-codes an identity.
#
# BASELINE IS ZERO, deliberately. A ratchet at today's count would make the
# gate certify the fourteen it was written because of. They are fixed in the
# same commit that adds this; the gate only has to keep them from coming back.
#
# Excludes this file (it must name the pattern to forbid it) and .gitignore.
# `memory/` alone is not enough of a key: `core-tests/base/memory/audit.md` is
# a legitimate repo path, and an earlier draft of this check flagged it. The
# slug shape (a date, or a known private prefix) is what separates them.
mem_refs=$(git grep -nE '(^|[^A-Za-z0-9_./-])(MEMORY\.md|memory/[a-z0-9_]+_[0-9]{4}-[0-9]{2}-[0-9]{2}\.md|memory/(feedback|callg|task)[a-z0-9_]*\.md)' \
  -- ':!.gitignore' ':!scripts/ci/check_no_internal_refs.sh' \
  || true)
if [ -n "$mem_refs" ]; then
  echo "FORBIDDEN machine-local memory references in tracked files:" >&2
  echo "$mem_refs" >&2
  echo "Fix: state the requirement in place, or cite a task ID / a source file." >&2
  exit 1
fi
echo "check-memory-refs: OK (baseline 0)"
