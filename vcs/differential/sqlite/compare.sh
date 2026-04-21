#!/usr/bin/env bash
# compare.sh — run one SQL scenario against both C-SQLite and loom, diff results.
#
# Usage: ./compare.sh <scenario.sql>
#
# Exit codes:
#   0 — identical output
#   1 — output diverges (logged)
#   2 — one of the engines crashed / non-zero exit
#   3 — usage error

set -euo pipefail

SCENARIO="${1:-}"
if [[ -z "${SCENARIO}" || ! -f "${SCENARIO}" ]]; then
    echo "usage: compare.sh <scenario.sql>" >&2
    exit 3
fi

DIFF_ROOT="$(cd "$(dirname "$0")" && pwd)"
ORACLE_DIR="${DIFF_ROOT}/tier-oracle"
C_SQLITE="${ORACLE_DIR}/c-sqlite"
V_SQLITE="${ORACLE_DIR}/verum-sqlite"

if [[ ! -x "${C_SQLITE}" || ! -x "${V_SQLITE}" ]]; then
    echo "error: missing tier-oracle binaries; run vcs pre-gate" >&2
    exit 2
fi

WORK_DIR="$(mktemp -d -t loom-diff-XXXXXX)"
trap "rm -rf ${WORK_DIR}" EXIT

C_DB="${WORK_DIR}/c.db"
V_DB="${WORK_DIR}/v.db"
C_OUT="${WORK_DIR}/c.out"
V_OUT="${WORK_DIR}/v.out"

# Run both engines on the same SQL:
"${C_SQLITE}" -bail -batch "${C_DB}" < "${SCENARIO}" > "${C_OUT}" 2>&1
"${V_SQLITE}" -bail -batch "${V_DB}" < "${SCENARIO}" > "${V_OUT}" 2>&1

# File-format round-trip: V reads C's file; C reads V's file.
"${V_SQLITE}" "${C_DB}" "SELECT 'ok'" > /dev/null
"${C_SQLITE}" "${V_DB}" "SELECT 'ok'" > /dev/null

# Diff query outputs:
if ! diff -u "${C_OUT}" "${V_OUT}"; then
    echo "DIVERGENCE on ${SCENARIO}" >&2
    echo "  C output:      ${C_OUT}" >&2
    echo "  V output:      ${V_OUT}" >&2
    exit 1
fi

# Diff file-format bytes (after vacuum to canonicalise free list):
"${C_SQLITE}" "${C_DB}" "VACUUM" > /dev/null
"${V_SQLITE}" "${V_DB}" "VACUUM" > /dev/null
if ! cmp "${C_DB}" "${V_DB}"; then
    echo "FILE FORMAT DIVERGENCE on ${SCENARIO}" >&2
    exit 1
fi

echo "OK ${SCENARIO}"
