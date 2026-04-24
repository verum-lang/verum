#!/usr/bin/env bash
# vcs/differential/tls/compare.sh — wire-format parity harness
# entry point per tls-quic.md §2.5 / §10.3.
#
# Usage:
#   compare.sh <layer> [impl1,impl2,...]
#
# <layer> ∈ {record, handshake, x509}
#
# Implementations per layer (spec §2.5 table):
#   record     : rustls, s2n-tls
#   handshake  : picotls, boringssl
#   x509       : rustls-webpki, openssl
#
# CI gate — exits non-zero on any byte-level divergence in the golden
# fixtures under `<layer>/golden/`. Impls provided via docker tags
# in `<layer>/docker-compose.yml` (per-layer, managed outside this
# script).

set -euo pipefail

LAYER="${1:?usage: compare.sh <record|handshake|x509> [impl1,impl2,...]}"
IMPLS="${2:-}"

HERE="$(cd "$(dirname "$0")" && pwd)"
LAYER_DIR="$HERE/$LAYER"
if [[ ! -d "$LAYER_DIR" ]]; then
    echo "unknown layer: $LAYER (expected record|handshake|x509)" >&2
    exit 2
fi

DEFAULT_IMPLS=""
case "$LAYER" in
    record)    DEFAULT_IMPLS="rustls,s2n-tls" ;;
    handshake) DEFAULT_IMPLS="picotls,boringssl" ;;
    x509)      DEFAULT_IMPLS="rustls-webpki,openssl" ;;
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
        # A concrete runner per impl lives under <layer>/runners/<impl>.sh.
        # If absent, the fixture is documentation only; skip.
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
echo "$LAYER: OK (impls: $IMPLS)"
