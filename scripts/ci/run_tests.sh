#!/usr/bin/env bash
# Run comprehensive test suite for CI
set -euo pipefail

TEST_TYPE="${1:-all}"
VERBOSE="${VERBOSE:-0}"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
  echo -e "${GREEN}[INFO]${NC} $*"
}

log_warn() {
  echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
  echo -e "${RED}[ERROR]${NC} $*"
}

run_test() {
  local name="$1"
  shift
  log_info "Running: $name"

  if [ "$VERBOSE" -eq 1 ]; then
    "$@" || { log_error "$name failed"; return 1; }
  else
    "$@" 2>&1 | tee "/tmp/${name}.log" || { log_error "$name failed"; return 1; }
  fi

  log_info "✓ $name passed"
}

case "$TEST_TYPE" in
  unit)
    log_info "Running unit tests..."
    run_test "unit-tests" cargo test --workspace --lib --verbose -- --nocapture
    ;;

  integration)
    log_info "Running integration tests..."
    run_test "integration-tests" cargo test --workspace --test '*' --verbose -- --nocapture
    ;;

  doc)
    log_info "Running documentation tests..."
    run_test "doc-tests" cargo test --workspace --doc --verbose
    ;;

  all)
    log_info "Running all tests..."
    run_test "unit-tests" cargo test --workspace --lib --verbose -- --nocapture
    run_test "integration-tests" cargo test --workspace --test '*' --verbose -- --nocapture
    run_test "doc-tests" cargo test --workspace --doc --verbose
    ;;

  *)
    log_error "Unknown test type: $TEST_TYPE"
    echo "Usage: $0 {unit|integration|doc|all}"
    exit 1
    ;;
esac

log_info "All tests completed successfully!"
