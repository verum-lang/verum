#!/usr/bin/env bash
# vcs/differential/h3/compare.sh — HTTP/3 + QPACK parity harness.
#
# Usage:
#   compare.sh <layer> [impl1,impl2,...]
#
# <layer> ∈ {frame, qpack}
#
# Implementations: quiche-h3, nghttp3 (ngtcp2's H3 impl).

set -euo pipefail

LAYER="${1:?usage: compare.sh <frame|qpack> [impl1,impl2,...]}"
IMPLS="${2:-quiche-h3,nghttp3}"

HERE="$(cd "$(dirname "$0")" && pwd)"
LAYER_DIR="$HERE/$LAYER"
if [[ ! -d "$LAYER_DIR" ]]; then
    echo "no fixtures for $LAYER yet — scaffolding only, skipping" >&2
    exit 0
fi

FAIL=0
GOLDEN_DIR="$LAYER_DIR/golden"
if [[ -d "$GOLDEN_DIR" ]]; then
    for fixture in "$GOLDEN_DIR"/*.hex; do
        [[ -f "$fixture" ]] || continue
        name="$(basename "$fixture" .hex)"
        IFS=',' read -ra IMPL_ARR <<< "$IMPLS"
        for impl in "${IMPL_ARR[@]}"; do
            runner="$LAYER_DIR/runners/$impl.sh"
            [[ -x "$runner" ]] || continue
            if ! "$runner" < "$fixture" > /dev/null 2>&1; then
                echo "DIVERGE h3/$LAYER/$name vs $impl" >&2
                FAIL=$((FAIL + 1))
            fi
        done
    done
fi

if [[ $FAIL -ne 0 ]]; then
    echo "$FAIL divergence(s) in h3.$LAYER" >&2
    exit 1
fi
echo "h3.$LAYER: OK (impls: $IMPLS)"
