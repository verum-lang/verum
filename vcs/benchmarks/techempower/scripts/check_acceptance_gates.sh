#!/usr/bin/env bash
# Acceptance-gate harness — read the six results.json files and
# enforce the targets from net-framework.md §1.1.
#
# Exit 0 iff every gate passes, otherwise exit 1 with a summary.

set -eu -o pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

FAIL=0

# Gate 1: plaintext RPS / core
echo "==> Gate 1: plaintext RPS"
if [[ -f plaintext.results.json ]]; then
  rps=$(jq -r '.rps' plaintext.results.json | tr -d 'kKmM' )
  if (( $(echo "$rps >= 4000000" | bc -l) )); then
    echo "    PASS — plaintext RPS = $rps"
  else
    echo "    FAIL — plaintext RPS = $rps (need ≥ 4M)"
    FAIL=1
  fi
else
  echo "    SKIP — plaintext.results.json missing"
fi

# Gate 2: p99.9 echo latency < 200µs (we use plaintext as the echo).
echo "==> Gate 2: p99.9 < 200µs"
if [[ -f plaintext.results.json ]]; then
  p999=$(jq -r '.p999_latency' plaintext.results.json)
  case "$p999" in
    *us) us="${p999%us}";  ;;
    *ms) us=$(echo "${p999%ms} * 1000" | bc -l) ;;
    *)   us=999999 ;;
  esac
  if (( $(echo "$us < 200" | bc -l) )); then
    echo "    PASS — p99.9 = ${p999}"
  else
    echo "    FAIL — p99.9 = ${p999} (need < 200µs)"
    FAIL=1
  fi
fi

# Gate 3: each scenario has a results.json (i.e., the load-gen ran).
echo "==> Gate 3: every scenario produced output"
for s in plaintext json db_query db_queries fortunes db_updates; do
  if [[ -f "${s}.results.json" ]]; then
    echo "    OK — $s"
  else
    echo "    FAIL — ${s}.results.json missing"
    FAIL=1
  fi
done

if [[ $FAIL -eq 0 ]]; then
  echo
  echo "==> ALL ACCEPTANCE GATES PASSED"
  exit 0
else
  echo
  echo "==> ACCEPTANCE GATES FAILED"
  exit 1
fi
