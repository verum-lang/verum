#!/usr/bin/env bash
# End-to-end test: spawn the Verum plaintext server, drive it with the
# Rust loadgen, and assert correctness + minimum RPS. Designed to run
# in CI on both interpreter and AOT tiers.
#
# Usage:
#   ./run_e2e.sh [tier] [duration_secs] [concurrency]
#
# Where:
#   [tier]          one of: interpreter | aot. Default interpreter.
#   [duration_secs] loadgen measurement window. Default 5.
#   [concurrency]   in-flight HTTP/1.1 requests. Default 32.
#
# Exit codes:
#   0 — server bound, loadgen ran, errors==0
#   1 — server failed to bind within 30s
#   2 — loadgen reported non-zero errors
#   3 — RPS below floor (set per tier in the script)

set -eu -o pipefail

TIER="${1:-interpreter}"
DURATION="${2:-5}"
CONCURRENCY="${3:-32}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SERVER_VR="${ROOT}/vcs/benchmarks/techempower/plaintext/server.vr"
LOADGEN_DIR="${ROOT}/vcs/benchmarks/techempower/scripts/loadgen-rs"
LOADGEN_BIN="${LOADGEN_DIR}/target/release/loadgen"

# Floor RPS per tier — interpreter is intentionally lax so a fresh
# build doesn't false-fail; AOT is held to a higher bar that exercises
# the whole optimization pipeline. Tune with measured numbers once
# the rig is calibrated.
case "$TIER" in
  interpreter) RPS_FLOOR=100 ;;
  aot)         RPS_FLOOR=20000 ;;
  *)
    echo "unknown tier: $TIER (use interpreter|aot)" >&2
    exit 2
    ;;
esac

# Build loadgen if missing.
if [[ ! -x "$LOADGEN_BIN" ]]; then
  echo "==> building loadgen-rs"
  (cd "$LOADGEN_DIR" && cargo build --release)
fi

# Pick verum_cli flags per tier. The `--aot` flag selects native LLVM
# codegen instead of the default VBC interpreter.
VERUM_FLAGS=("run")
if [[ "$TIER" == "aot" ]]; then
  VERUM_FLAGS+=("--aot")
fi
VERUM_FLAGS+=("$SERVER_VR")

# Start the verum server in the background.
SERVER_LOG="$(mktemp -t verum-server.XXXXXX)"
echo "==> launching verum [$TIER]: $SERVER_VR (log: $SERVER_LOG)"
(cd "$ROOT" && cargo run --release -p verum_cli -- "${VERUM_FLAGS[@]}") \
  > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

cleanup() {
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  echo "==> server log:"
  cat "$SERVER_LOG"
  rm -f "$SERVER_LOG"
}
trap cleanup EXIT

# Wait for the "listening on" line. 30 second cap to surface bind
# failures quickly.
echo "==> waiting for bind"
for i in $(seq 1 30); do
  if grep -q "listening on" "$SERVER_LOG" 2>/dev/null; then
    echo "==> server is up after ${i}s"
    break
  fi
  if grep -q "bind failed" "$SERVER_LOG" 2>/dev/null; then
    echo "==> bind failed:"
    cat "$SERVER_LOG"
    exit 1
  fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "==> server exited before binding"
    cat "$SERVER_LOG"
    exit 1
  fi
  sleep 1
done

if ! grep -q "listening on" "$SERVER_LOG"; then
  echo "==> server never bound"
  exit 1
fi

# Run loadgen against the live socket.
echo "==> running loadgen"
RESULTS=$("$LOADGEN_BIN" \
  --url "http://127.0.0.1:8080/plaintext" \
  --concurrency "$CONCURRENCY" \
  --duration-secs "$DURATION" \
  --warmup-secs 2 \
  --output json)
echo "$RESULTS"

# Parse and gate.
ERRORS=$(echo "$RESULTS" | awk '/"errors":/  { gsub(",",""); print $2 }' | head -1)
RPS=$(echo "$RESULTS"    | awk '/"rps":/     { gsub(",",""); print $2 }' | head -1 | awk '{printf "%d", $1}')

if [[ "${ERRORS:-0}" != "0" ]]; then
  echo "==> FAIL: ${ERRORS} request errors"
  exit 2
fi
if (( RPS < RPS_FLOOR )); then
  echo "==> FAIL: ${RPS} rps below ${RPS_FLOOR} floor for tier=$TIER"
  exit 3
fi

echo "==> PASS: tier=$TIER rps=${RPS} (floor ${RPS_FLOOR}) errors=0"
exit 0
