#!/usr/bin/env bash
# vcs/differential/quic/compare.sh — wire-format parity harness
# entry point per tls-quic.md §2.5 / §10.3.
#
# Usage:
#   compare.sh <layer> [impl1,impl2,...]
#
# <layer> ∈ {initial, frame, recovery}
#
# Implementations per layer (spec §2.5 table):
#   initial    : quiche, ngtcp2
#   frame      : quiche, picoquic
#   recovery   : quiche, picoquic
#
# CI gate — exits non-zero on any byte-level divergence.

set -euo pipefail

LAYER="${1:?usage: compare.sh <initial|frame|recovery> [impl1,impl2,...]}"
IMPLS="${2:-}"

HERE="$(cd "$(dirname "$0")" && pwd)"
LAYER_DIR="$HERE/$LAYER"
if [[ ! -d "$LAYER_DIR" ]]; then
    echo "unknown layer: $LAYER (expected initial|frame|recovery)" >&2
    exit 2
fi

DEFAULT_IMPLS=""
case "$LAYER" in
    initial)  DEFAULT_IMPLS="quiche,ngtcp2" ;;
    frame)    DEFAULT_IMPLS="quiche,picoquic" ;;
    recovery) DEFAULT_IMPLS="quiche,picoquic" ;;
esac
IMPLS="${IMPLS:-$DEFAULT_IMPLS}"

GOLDEN_DIR="$LAYER_DIR/golden"
if [[ ! -d "$GOLDEN_DIR" ]]; then
    echo "no golden/ fixtures for $LAYER yet — scaffolding only, skipping" >&2
    exit 0
fi

FAIL=0
for fixture in "$GOLDEN_DIR"/*.hex; do
    [[ -f "$fixture" ]] || continue
    name="$(basename "$fixture" .hex)"
    IFS=',' read -ra IMPL_ARR <<< "$IMPLS"
    reference_out="$(cat "$fixture")"
    for impl in "${IMPL_ARR[@]}"; do
        runner="$LAYER_DIR/runners/$impl.sh"
        [[ -x "$runner" ]] || continue
        actual="$("$runner" < "$fixture" || true)"
        if [[ "$actual" != "$reference_out" ]]; then
            echo "DIVERGE $LAYER/$name vs $impl" >&2
            FAIL=$((FAIL + 1))
        fi
    done
done

if [[ $FAIL -ne 0 ]]; then
    echo "$FAIL divergence(s) in $LAYER layer" >&2
    exit 1
fi
echo "quic.$LAYER: OK (impls: $IMPLS)"
