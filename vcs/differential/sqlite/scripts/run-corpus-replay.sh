#!/usr/bin/env bash
# run-corpus-replay.sh — read every file in corpus/public via loom, diff canonical
# query outputs (COUNT, integrity_check, schema) vs C-SQLite. Divergences reported.
#
# Usage: ./run-corpus-replay.sh [corpus-dir]
set -euo pipefail

DIFF_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CORPUS_DIR="${1:-${DIFF_ROOT}/corpus/public}"
ORACLE_DIR="${DIFF_ROOT}/tier-oracle"

if [[ ! -d "${CORPUS_DIR}" ]]; then
    echo "Corpus missing; run fetch-corpus.sh first" >&2
    exit 1
fi

FAIL=0
TOTAL=0
for db in "${CORPUS_DIR}"/*.sqlite; do
    [[ -e "${db}" ]] || continue
    TOTAL=$((TOTAL + 1))

    C_OUT="$(mktemp)"
    V_OUT="$(mktemp)"
    QUERY="
        SELECT name FROM sqlite_master ORDER BY name;
        PRAGMA integrity_check;
        SELECT COUNT(*) FROM sqlite_master;
    "
    "${ORACLE_DIR}/c-sqlite"    "${db}" "${QUERY}" > "${C_OUT}" 2>/dev/null || true
    "${ORACLE_DIR}/verum-sqlite" "${db}" "${QUERY}" > "${V_OUT}" 2>/dev/null || true

    if ! diff -q "${C_OUT}" "${V_OUT}" >/dev/null; then
        FAIL=$((FAIL + 1))
        echo "DIVERGENCE: ${db}"
        diff -u "${C_OUT}" "${V_OUT}" | head -20
    fi
    rm -f "${C_OUT}" "${V_OUT}"
done

echo "corpus replay: ${TOTAL} databases, ${FAIL} divergences"
[[ "${FAIL}" -eq 0 ]]
