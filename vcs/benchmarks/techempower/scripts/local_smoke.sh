#!/usr/bin/env bash
# Local smoke — exercise every TechEmpower scenario binary against the
# typecheck + run pipeline. Not the load-gen; that's run_loadgen.sh.

set -eu -o pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> typechecking every scenario"
for s in plaintext json db_query db_queries fortunes db_updates; do
  echo "    -- $s"
  cargo run --quiet -p vtest -- run "$ROOT/$s/main.vr"
done

echo
echo "==> all six scenarios smoke-pass"
