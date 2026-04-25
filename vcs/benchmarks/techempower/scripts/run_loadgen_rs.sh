#!/usr/bin/env bash
# TechEmpower R23 load-gen — Rust hyper backend.
#
# Wraps the in-tree `verum-techempower-loadgen` binary at
# vcs/benchmarks/techempower/scripts/loadgen-rs/. Builds it on demand
# (release mode) and invokes the closed-loop driver.
#
# Usage:
#   ./run_loadgen_rs.sh <scenario> [duration_secs] [concurrency]
#
# Where:
#   <scenario>      one of: plaintext json db_query db_queries fortunes db_updates
#   [duration_secs] measurement window; default 10
#   [concurrency]   in-flight requests; default 64
#
# Output:
#   ${scenario}.results.json — JSON record produced by the loadgen.

set -eu -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADGEN_DIR="${SCRIPT_DIR}/loadgen-rs"
BIN="${LOADGEN_DIR}/target/release/loadgen"

SCENARIO="${1:-plaintext}"
DURATION="${2:-10}"
CONCURRENCY="${3:-64}"

case "$SCENARIO" in
  plaintext)   PATHX="/plaintext" ;;
  json)        PATHX="/json" ;;
  db_query)    PATHX="/db" ;;
  db_queries)  PATHX="/queries?queries=20" ;;
  fortunes)    PATHX="/fortunes" ;;
  db_updates)  PATHX="/updates?queries=10" ;;
  *)
    echo "unknown scenario: $SCENARIO" >&2
    exit 2
    ;;
esac

URL="http://127.0.0.1:8080${PATHX}"

if [[ ! -x "$BIN" ]]; then
  echo "==> building loadgen-rs"
  (cd "$LOADGEN_DIR" && cargo build --release)
fi

echo "==> loadgen scenario=$SCENARIO duration=${DURATION}s concurrency=$CONCURRENCY"
"$BIN" \
  --url "$URL" \
  --concurrency "$CONCURRENCY" \
  --duration-secs "$DURATION" \
  --warmup-secs 2 \
  --output json \
  | tee "${SCENARIO}.results.json"

echo "==> wrote ${SCENARIO}.results.json"
