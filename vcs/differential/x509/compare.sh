#!/usr/bin/env bash
# vcs/differential/x509/compare.sh — X.509 DER parity harness.
#
# Usage:
#   compare.sh <layer> [impl1,impl2,...]
#
# <layer> ∈ {golden, chain, ocsp}
#
# Implementations: rustls-webpki, openssl.

set -euo pipefail

LAYER="${1:?usage: compare.sh <golden|chain|ocsp> [impl1,impl2,...]}"
IMPLS="${2:-rustls-webpki,openssl}"

HERE="$(cd "$(dirname "$0")" && pwd)"
LAYER_DIR="$HERE/$LAYER"
if [[ ! -d "$LAYER_DIR" ]]; then
    echo "no fixtures for $LAYER yet — scaffolding only, skipping" >&2
    exit 0
fi

FAIL=0
for fixture in "$LAYER_DIR"/*.{der,pem,hex}; do
    [[ -f "$fixture" ]] || continue
    name="$(basename "$fixture")"
    IFS=',' read -ra IMPL_ARR <<< "$IMPLS"
    for impl in "${IMPL_ARR[@]}"; do
        runner="$LAYER_DIR/runners/$impl.sh"
        [[ -x "$runner" ]] || continue
        if ! "$runner" < "$fixture" > /dev/null 2>&1; then
            echo "DIVERGE x509/$LAYER/$name vs $impl" >&2
            FAIL=$((FAIL + 1))
        fi
    done
done

if [[ $FAIL -ne 0 ]]; then
    echo "$FAIL divergence(s) in x509.$LAYER" >&2
    exit 1
fi
echo "x509.$LAYER: OK (impls: $IMPLS)"
