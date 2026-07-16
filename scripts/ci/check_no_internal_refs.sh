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
#   llvm/instruction.rs        — one comment, peer-dirty at gate introduction (T0143)
#   vbc codegen/expressions.rs — one comment, peer-dirty at gate introduction (T0143)
violations=$(git grep -nE '(^|[^A-Za-z0-9_.])internal/([A-Za-z0-9_-]+/|[A-Za-z0-9_-]+\.(md|pdf|vr|rs|tex|toml|json))' \
  -- ':!.gitignore' \
     ':!crates/verum_kernel/tests/k_arch_v_alignment.rs' \
     ':!crates/verum_codegen/src/llvm/instruction.rs' \
     ':!crates/verum_vbc/src/codegen/expressions.rs' \
     ':!scripts/ci/check_no_internal_refs.sh' \
  || true)
if [ -n "$violations" ]; then
  echo "FORBIDDEN internal/-directory references in tracked files:" >&2
  echo "$violations" >&2
  echo "Fix: state the requirement in place or cite a public doc (see CLAUDE.md)." >&2
  exit 1
fi
echo "check-internal-refs: OK"
