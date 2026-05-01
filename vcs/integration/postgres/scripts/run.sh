#!/usr/bin/env bash
# =============================================================
# vcs/integration/postgres/scripts/run.sh
# =============================================================
# Drives the full Postgres integration-test cycle:
#   1. docker compose up -d (background; tmpfs data volume)
#   2. wait for healthcheck to flip to "healthy"
#   3. run every tests/t*.vr through `verum run`
#   4. capture pass/fail per test
#   5. docker compose down -v (always, even on failure)
#
# Usage:
#   bash vcs/integration/postgres/scripts/run.sh           # run all
#   bash vcs/integration/postgres/scripts/run.sh t01_*.vr  # run a glob
#
# Exit code:
#   0 on every test passing
#   1 on any test failure
#   2 on docker / harness failure (test never ran)
# =============================================================

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${HERE}/../../../.." && pwd)"
COMPOSE_FILE="${HERE}/../docker-compose.yml"
TESTS_DIR="${HERE}/../tests"
VERUM_BIN="${ROOT}/target/release/verum"

# Glob filter (default all).
GLOB="${1:-t*.vr}"

# Where the runtime captures per-test logs.
RESULTS_DIR="${HERE}/../results"
mkdir -p "${RESULTS_DIR}"

cd "${HERE}/.."

# -----------------------------------------------------------------
# Helpers.
# -----------------------------------------------------------------

log() { printf '[run.sh] %s\n' "$*" >&2; }

cleanup() {
    log "tearing down docker compose (down -v)"
    docker compose -f "${COMPOSE_FILE}" down -v --remove-orphans >/dev/null 2>&1 || true
}

trap cleanup EXIT

# -----------------------------------------------------------------
# 1. Bring up.
# -----------------------------------------------------------------

# Sanity-check the daemon before invoking compose so the failure
# message is actionable instead of a docker.sock connect error.
if ! docker info >/dev/null 2>&1; then
    log "docker daemon not reachable — start Docker / OrbStack and retry"
    log "  on macOS: open -a Docker      (or)    open -a OrbStack"
    log "  on Linux: sudo systemctl start docker"
    exit 2
fi

log "bringing up postgres:16-alpine on 127.0.0.1:55432"
docker compose -f "${COMPOSE_FILE}" up -d --quiet-pull --wait || {
    log "docker compose up failed"
    exit 2
}

# -----------------------------------------------------------------
# 2. Wait for healthcheck — `--wait` already bounded the up call,
#    but double-check via pg_isready loop because the healthcheck
#    grace period can let an unhealthy instance through during the
#    initial boot-up burst.
# -----------------------------------------------------------------

log "verifying readiness via pg_isready"
DEADLINE_S=$(( $(date +%s) + 30 ))
while true; do
    if docker exec spindle-pg-test pg_isready -U spindle_admin -d spindle_test >/dev/null 2>&1; then
        log "postgres ready"
        break
    fi
    if [[ "$(date +%s)" -ge "${DEADLINE_S}" ]]; then
        log "postgres did not become ready in 30s"
        docker compose -f "${COMPOSE_FILE}" logs postgres | tail -50 >&2 || true
        exit 2
    fi
    sleep 0.5
done

# -----------------------------------------------------------------
# 3. Run tests.
# -----------------------------------------------------------------

PASS=0
FAIL=0
SKIP=0
FAIL_NAMES=()

shopt -s nullglob
TESTS=( "${TESTS_DIR}"/${GLOB} )
shopt -u nullglob

if [[ "${#TESTS[@]}" -eq 0 ]]; then
    log "no tests matched glob '${GLOB}' under ${TESTS_DIR}"
    exit 2
fi

for t in "${TESTS[@]}"; do
    name="$(basename "${t}")"
    out="${RESULTS_DIR}/${name%.vr}.log"
    log "running ${name}"
    if "${VERUM_BIN}" run "${t}" >"${out}" 2>&1; then
        PASS=$((PASS + 1))
        log "  ✓ ${name}"
    else
        rc=$?
        if grep -q '^@skip:' "${out}"; then
            SKIP=$((SKIP + 1))
            log "  ~ ${name} (skipped)"
        else
            FAIL=$((FAIL + 1))
            FAIL_NAMES+=("${name}")
            log "  ✗ ${name} (exit ${rc}; tail follows)"
            tail -20 "${out}" | sed 's/^/    /' >&2
        fi
    fi
done

# -----------------------------------------------------------------
# 4. Summary.
# -----------------------------------------------------------------

TOTAL=$((PASS + FAIL + SKIP))
log "summary: ${PASS}/${TOTAL} passed, ${FAIL} failed, ${SKIP} skipped"
if [[ "${FAIL}" -ne 0 ]]; then
    log "failed tests:"
    for n in "${FAIL_NAMES[@]}"; do log "  - ${n}"; done
    exit 1
fi
exit 0
