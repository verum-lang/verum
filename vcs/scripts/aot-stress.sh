#!/usr/bin/env bash
# AOT stability stress harness.
#
# Iterates `verum build` across every example to detect the residual ~4%
# LLVM stability failure documented in KNOWN_ISSUES.md (Phase 1.3 target).
# Any iteration failure exits non-zero.
#
# Usage:
#   bash vcs/scripts/aot-stress.sh [ITERATIONS=50] [PARALLELISM=8]
#
# Environment:
#   VERUM_BIN   path to verum binary (default: auto-detect release build)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

ITERATIONS="${1:-50}"
PARALLELISM="${2:-8}"

# Locate the verum binary. Prefer a pre-built release binary; otherwise build.
if [[ -n "${VERUM_BIN:-}" ]]; then
  VERUM="$VERUM_BIN"
elif [[ -x "target/release/verum" ]]; then
  VERUM="$ROOT/target/release/verum"
else
  echo "==> Building verum_cli (release)"
  cargo build -p verum_cli --release --locked
  VERUM="$ROOT/target/release/verum"
fi

EXAMPLES=()
while IFS= read -r -d '' f; do
  EXAMPLES+=("$f")
done < <(find "$ROOT/examples" -name '*.vr' -print0)

if [[ ${#EXAMPLES[@]} -eq 0 ]]; then
  echo "==> No examples found under examples/; nothing to stress. Passing."
  exit 0
fi

echo "==> AOT stress: $ITERATIONS iterations × ${#EXAMPLES[@]} examples, parallelism $PARALLELISM"

OUTDIR="$(mktemp -d)"
trap 'rm -rf "$OUTDIR"' EXIT

FAILED=0
for ((i = 1; i <= ITERATIONS; i++)); do
  for example in "${EXAMPLES[@]}"; do
    name="$(basename "$example" .vr)"
    out="$OUTDIR/iter-$i-$name.log"
    if ! "$VERUM" build "$example" --release >"$out" 2>&1; then
      FAILED=$((FAILED + 1))
      echo "!! FAIL iter=$i example=$name (log: $out)"
    fi
  done
  # Every 10 iterations, emit progress.
  if ((i % 10 == 0)); then
    echo "  ... $i/$ITERATIONS done, $FAILED failures so far"
  fi
done

TOTAL=$((ITERATIONS * ${#EXAMPLES[@]}))
echo "==> AOT stress complete: $((TOTAL - FAILED))/$TOTAL passed"

if [[ $FAILED -gt 0 ]]; then
  echo "!! $FAILED failures detected (target: 0)"
  exit 1
fi
