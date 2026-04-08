#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Quick Test Runner
# =============================================================================
#
# Runs only L0 critical tests for fast CI feedback (< 5 minutes).
# Use this for PR validation and pre-commit checks.
#
# Usage:
#   ./run_quick_tests.sh [options]
#
# Options:
#   --parallel N       Number of parallel workers (default: auto)
#   --timeout MS       Timeout per test in milliseconds (default: 5000)
#   --output-dir DIR   Output directory for reports
#   --fail-fast        Stop on first failure (default: true)
#   --include-l1       Also run L1 core tests
#   --filter PATTERN   Only run tests matching pattern
#   --verbose          Enable verbose output
#   -h, --help         Show this help message
#
# Exit Codes:
#   0 - All tests passed
#   1 - Tests failed
#   2 - Build or setup failure
#
# Environment Variables:
#   VCS_PARALLEL          Override parallel workers
#   VCS_QUICK_TIMEOUT     Override timeout (default: 5000)
#   CI                    Set to 'true' in CI environments
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Configuration
PARALLEL="${VCS_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
TIMEOUT="${VCS_QUICK_TIMEOUT:-5000}"
OUTPUT_DIR="${VCS_OUTPUT_DIR:-$VCS_ROOT/reports}"
CONFIG="${VCS_CONFIG:-$VCS_ROOT/config/vcs.toml}"
FAIL_FAST=1
INCLUDE_L1=0
FILTER=""
VERBOSE=0

# Tool paths
VTEST="$VCS_ROOT/runner/vtest/target/release/vtest"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# CI mode
if [ "${CI:-}" = "true" ]; then
    RED=''
    GREEN=''
    YELLOW=''
    CYAN=''
    NC=''
fi

# Logging
log_info() { echo -e "${CYAN}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_error() { echo -e "${RED}[FAIL]${NC} $1" >&2; }

# Usage
usage() {
    head -n 35 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --parallel)
                PARALLEL="$2"
                shift 2
                ;;
            --timeout)
                TIMEOUT="$2"
                shift 2
                ;;
            --output-dir)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            --fail-fast)
                FAIL_FAST=1
                shift
                ;;
            --no-fail-fast)
                FAIL_FAST=0
                shift
                ;;
            --include-l1)
                INCLUDE_L1=1
                shift
                ;;
            --filter)
                FILTER="$2"
                shift 2
                ;;
            --verbose)
                VERBOSE=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done
}

# Build vtest if needed
build_vtest() {
    if [ ! -f "$VTEST" ]; then
        log_info "Building vtest..."
        cargo build --release --manifest-path "$VCS_ROOT/runner/vtest/Cargo.toml"
    fi

    if [ ! -f "$VTEST" ]; then
        log_error "vtest binary not found"
        exit 2
    fi
}

# Run quick tests
run_quick_tests() {
    local levels="L0"
    if [ "$INCLUDE_L1" -eq 1 ]; then
        levels="L0,L1"
    fi

    local args=(
        run
        --level "$levels"
        --parallel "$PARALLEL"
        --timeout "$TIMEOUT"
        --format junit
        --output "$OUTPUT_DIR/quick-results.xml"
        --config "$CONFIG"
    )

    if [ "$FAIL_FAST" -eq 1 ]; then
        args+=(--fail-fast)
    fi

    if [ -n "$FILTER" ]; then
        args+=(--filter "$FILTER")
    fi

    if [ "$VERBOSE" -eq 1 ]; then
        args+=(--verbose)
    fi

    local start_time
    start_time=$(date +%s)

    local exit_code=0
    "$VTEST" "${args[@]}" || exit_code=$?

    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))

    # Parse results
    local total=0
    local failed=0
    local passed=0

    if [ -f "$OUTPUT_DIR/quick-results.xml" ]; then
        total=$(grep -oP 'tests="\K[0-9]+' "$OUTPUT_DIR/quick-results.xml" | head -1 || echo "0")
        failed=$(grep -oP 'failures="\K[0-9]+' "$OUTPUT_DIR/quick-results.xml" | head -1 || echo "0")
        passed=$((total - failed))
    fi

    echo ""
    echo "Quick Test Results ($levels):"
    echo "  Total:    $total"
    echo "  Passed:   $passed"
    echo "  Failed:   $failed"
    echo "  Duration: ${duration}s"
    echo ""

    return $exit_code
}

# Main function
main() {
    parse_args "$@"

    mkdir -p "$OUTPUT_DIR"
    build_vtest

    echo ""
    log_info "VCS Quick Test Suite"
    log_info "Parallel: $PARALLEL, Timeout: ${TIMEOUT}ms"
    echo ""

    local start_time
    start_time=$(date +%s)

    if run_quick_tests; then
        local end_time
        end_time=$(date +%s)
        log_success "Quick tests PASSED in $((end_time - start_time))s"
        exit 0
    else
        local end_time
        end_time=$(date +%s)
        log_error "Quick tests FAILED in $((end_time - start_time))s"
        exit 1
    fi
}

# Run main
main "$@"
