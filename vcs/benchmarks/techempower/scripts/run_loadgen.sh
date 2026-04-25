#!/usr/bin/env bash
# TechEmpower R23 load-gen driver — wrk2 with calibrated rate control.
#
# Usage:
#   ./run_loadgen.sh <scenario> [duration]
#
# Where:
#   <scenario>   one of: plaintext json db_query db_queries fortunes db_updates
#   [duration]   wrk2 duration (e.g. 30s, 5m). Default 30s.
#
# Assumes:
#   * The verum service for $scenario is running on localhost:8080.
#   * wrk2 is installed (https://github.com/giltene/wrk2).
#   * `jq` is installed for JSON parse of the result.
#
# Outputs:
#   $scenario.results.json — wrk2 stats parsed into a flat JSON record:
#     { "scenario": "...", "duration": "...", "rps": ..., "p50_us": ...,
#       "p99_us": ..., "p999_us": ..., "p9999_us": ... }

set -eu -o pipefail

SCENARIO="${1:-plaintext}"
DURATION="${2:-30s}"
URL="http://localhost:8080/${SCENARIO}"
THREADS=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
CONNECTIONS=512
RATE=20000   # Start at 20k req/s; CI bumps per-scenario based on history.

case "$SCENARIO" in
  plaintext)   URL="http://localhost:8080/plaintext"; RATE=200000 ;;
  json)        URL="http://localhost:8080/json";       RATE=100000 ;;
  db_query)    URL="http://localhost:8080/db";          RATE=50000  ;;
  db_queries)  URL="http://localhost:8080/queries?queries=20"; RATE=10000 ;;
  fortunes)    URL="http://localhost:8080/fortunes";    RATE=10000  ;;
  db_updates)  URL="http://localhost:8080/updates?queries=10"; RATE=2000 ;;
  *)
    echo "unknown scenario: $SCENARIO" >&2
    exit 2
    ;;
esac

echo "==> wrk2 load-gen: scenario=$SCENARIO duration=$DURATION rate=$RATE"
wrk -t"$THREADS" -c"$CONNECTIONS" -d"$DURATION" -R"$RATE" \
    --latency \
    "$URL" \
    | tee "${SCENARIO}.raw.txt"

# Parse wrk2 output into a structured JSON record. wrk2 prints
# percentile lines like:
#   50.000%  123.45us
#   99.000%  789.01us
RPS=$(awk '/Requests\/sec:/ { print $2 }' "${SCENARIO}.raw.txt" | head -1)
P50=$(awk '/^ *50\.000%/  { print $2 }' "${SCENARIO}.raw.txt" | head -1)
P99=$(awk '/^ *99\.000%/  { print $2 }' "${SCENARIO}.raw.txt" | head -1)
P999=$(awk '/^ *99\.900%/ { print $2 }' "${SCENARIO}.raw.txt" | head -1)
P9999=$(awk '/^ *99\.990%/ { print $2 }' "${SCENARIO}.raw.txt" | head -1)

cat > "${SCENARIO}.results.json" <<EOF
{
  "scenario": "${SCENARIO}",
  "duration": "${DURATION}",
  "rate_target": ${RATE},
  "rps": "${RPS}",
  "p50_latency": "${P50}",
  "p99_latency": "${P99}",
  "p999_latency": "${P999}",
  "p9999_latency": "${P9999}"
}
EOF

echo "==> wrote ${SCENARIO}.results.json"
cat "${SCENARIO}.results.json"
