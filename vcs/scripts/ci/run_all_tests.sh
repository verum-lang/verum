#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Full Test Suite Runner
# =============================================================================
#
# Runs the complete VCS test suite (L0-L4) with proper error handling,
# reporting, and exit codes suitable for CI environments.
#
# Usage:
#   ./run_all_tests.sh [options]
#
# Options:
#   --parallel N       Number of parallel workers (default: auto)
#   --timeout MS       Timeout per test in milliseconds (default: 30000)
#   --output-dir DIR   Output directory for reports (default: reports/)
#   --format FORMAT    Output format: console, json, junit, html (default: junit)
#   --skip-l3          Skip L3 extended tests
#   --skip-l4          Skip L4 performance tests
#   --fail-fast        Stop on first failure
#   --verbose          Enable verbose output
#   --dry-run          Print commands without executing
#   -h, --help         Show this help message
#
# Exit Codes:
#   0 - All tests passed
#   1 - L0 or L1 tests failed (critical)
#   2 - L2 tests below threshold
#   3 - Differential tests failed
#   4 - L3 tests below threshold (if not skipped)
#   5 - Build or setup failure
#
# Environment Variables:
#   VCS_PARALLEL          Override parallel workers
#   VCS_TIMEOUT           Override timeout
#   VCS_CONFIG            Configuration file path
#   VCS_OUTPUT_DIR        Output directory
#   CI                    Set to 'true' in CI environments
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Configuration defaults
PARALLEL="${VCS_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
TIMEOUT="${VCS_TIMEOUT:-30000}"
TIMEOUT_EXTENDED="${VCS_TIMEOUT_EXTENDED:-600000}"
OUTPUT_DIR="${VCS_OUTPUT_DIR:-$VCS_ROOT/reports}"
FORMAT="junit"
CONFIG="${VCS_CONFIG:-$VCS_ROOT/config/vcs.toml}"
SKIP_L3=0
SKIP_L4=0
FAIL_FAST=0
VERBOSE=0
DRY_RUN=0

# Thresholds
L0_THRESHOLD=100.0
L1_THRESHOLD=100.0
L2_THRESHOLD=95.0
L3_THRESHOLD=90.0

# Tool paths
VTEST="$VCS_ROOT/runner/vtest/target/release/vtest"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# CI mode detection
if [ "${CI:-}" = "true" ]; then
    # Disable colors in CI
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    CYAN=''
    NC=''
fi

# Logging functions
log_info() {
    echo -e "${CYAN}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[FAIL]${NC} $1" >&2
}

log_step() {
    echo ""
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo ""
}

# Usage
usage() {
    head -n 40 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
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
            --format)
                FORMAT="$2"
                shift 2
                ;;
            --skip-l3)
                SKIP_L3=1
                shift
                ;;
            --skip-l4)
                SKIP_L4=1
                shift
                ;;
            --fail-fast)
                FAIL_FAST=1
                shift
                ;;
            --verbose)
                VERBOSE=1
                shift
                ;;
            --dry-run)
                DRY_RUN=1
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

# Execute command (respects dry-run)
run_cmd() {
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] $*"
        return 0
    fi

    if [ "$VERBOSE" -eq 1 ]; then
        echo "[CMD] $*"
    fi

    "$@"
}

# Build vtest if needed
build_vtest() {
    if [ ! -f "$VTEST" ]; then
        log_info "Building vtest..."
        run_cmd cargo build --release --manifest-path "$VCS_ROOT/runner/vtest/Cargo.toml"
    fi

    if [ ! -f "$VTEST" ]; then
        log_error "vtest binary not found: $VTEST"
        exit 5
    fi
}

# Run test level with result checking
run_test_level() {
    local level="$1"
    local threshold="$2"
    local timeout_val="$3"
    local blocking="${4:-true}"
    local output_file="$OUTPUT_DIR/${level,,}-results"

    log_step "$level Tests (threshold: ${threshold}%)"

    local format_ext="xml"
    if [ "$FORMAT" = "json" ]; then
        format_ext="json"
    fi

    local args=(
        run
        --level "$level"
        --parallel "$PARALLEL"
        --timeout "$timeout_val"
        --format "$FORMAT"
        --output "$output_file.$format_ext"
        --config "$CONFIG"
    )

    if [ "$FAIL_FAST" -eq 1 ]; then
        args+=(--fail-fast)
    fi

    if [ "$VERBOSE" -eq 1 ]; then
        args+=(--verbose)
    fi

    local start_time
    start_time=$(date +%s)

    local exit_code=0
    run_cmd "$VTEST" "${args[@]}" || exit_code=$?

    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))

    # Parse results
    local pass_rate=0
    local total=0
    local passed=0
    local failed=0

    if [ "$FORMAT" = "json" ] && [ -f "$output_file.json" ]; then
        pass_rate=$(jq -r '.summary.pass_percentage // 0' "$output_file.json" 2>/dev/null || echo "0")
        total=$(jq -r '.summary.total // 0' "$output_file.json" 2>/dev/null || echo "0")
        passed=$(jq -r '.summary.passed // 0' "$output_file.json" 2>/dev/null || echo "0")
        failed=$(jq -r '.summary.failed // 0' "$output_file.json" 2>/dev/null || echo "0")
    elif [ "$FORMAT" = "junit" ] && [ -f "$output_file.xml" ]; then
        total=$(grep -oP 'tests="\K[0-9]+' "$output_file.xml" | head -1 || echo "0")
        failed=$(grep -oP 'failures="\K[0-9]+' "$output_file.xml" | head -1 || echo "0")
        passed=$((total - failed))
        if [ "$total" -gt 0 ]; then
            pass_rate=$(echo "scale=2; $passed * 100 / $total" | bc)
        fi
    fi

    # Report results
    echo ""
    echo "Results for $level:"
    echo "  Total:     $total"
    echo "  Passed:    $passed"
    echo "  Failed:    $failed"
    echo "  Pass Rate: ${pass_rate}%"
    echo "  Duration:  ${duration}s"
    echo ""

    # Check threshold
    local threshold_met=1
    if (( $(echo "$pass_rate < $threshold" | bc -l) )); then
        threshold_met=0
    fi

    if [ "$threshold_met" -eq 1 ]; then
        log_success "$level tests passed (${pass_rate}% >= ${threshold}%)"
        return 0
    else
        if [ "$blocking" = "true" ]; then
            log_error "$level tests failed (${pass_rate}% < ${threshold}%)"
            return 1
        else
            log_warn "$level tests below threshold (${pass_rate}% < ${threshold}%)"
            return 0
        fi
    fi
}

# Run differential tests
run_differential() {
    log_step "Differential Tests (Tier 0 vs Tier 3)"

    local output_file="$OUTPUT_DIR/differential-results.xml"

    local args=(
        run
        "$VCS_ROOT/differential/"
        --tier 0,3
        --parallel "$PARALLEL"
        --format junit
        --output "$output_file"
        --config "$CONFIG"
    )

    local exit_code=0
    run_cmd "$VTEST" "${args[@]}" || exit_code=$?

    if [ "$exit_code" -eq 0 ] && [ -f "$output_file" ]; then
        if grep -q 'failures="0"' "$output_file" 2>/dev/null; then
            log_success "Differential tests passed - Tier 0 and Tier 3 are semantically equivalent"
            return 0
        fi
    fi

    log_error "Differential tests failed - Tier 0 and Tier 3 produce different results"
    return 1
}

# Generate summary report
generate_summary() {
    log_step "Test Suite Summary"

    echo ""
    echo "Configuration:"
    echo "  Parallel Workers: $PARALLEL"
    echo "  Timeout:          ${TIMEOUT}ms"
    echo "  Output Directory: $OUTPUT_DIR"
    echo "  Config File:      $CONFIG"
    echo ""

    echo "Results Files:"
    for f in "$OUTPUT_DIR"/*.xml "$OUTPUT_DIR"/*.json; do
        if [ -f "$f" ]; then
            echo "  $(basename "$f")"
        fi
    done
    echo ""
}

# Main function
main() {
    parse_args "$@"

    # Setup
    mkdir -p "$OUTPUT_DIR"
    build_vtest

    log_step "VCS Full Test Suite"

    log_info "Parallel Workers: $PARALLEL"
    log_info "Output Directory: $OUTPUT_DIR"
    log_info "Config File:      $CONFIG"
    echo ""

    local overall_exit=0

    # L0 Critical (100% required, blocking)
    if ! run_test_level "L0" "$L0_THRESHOLD" "$TIMEOUT" "true"; then
        overall_exit=1
        if [ "$FAIL_FAST" -eq 1 ]; then
            log_error "L0 Critical tests failed - stopping"
            exit 1
        fi
    fi

    # L1 Core (100% required, blocking)
    if ! run_test_level "L1" "$L1_THRESHOLD" "$TIMEOUT" "true"; then
        overall_exit=1
        if [ "$FAIL_FAST" -eq 1 ]; then
            log_error "L1 Core tests failed - stopping"
            exit 1
        fi
    fi

    # L2 Standard (95%+ required, blocking)
    FORMAT=json  # Use JSON for pass rate calculation
    if ! run_test_level "L2" "$L2_THRESHOLD" "$TIMEOUT" "true"; then
        [ "$overall_exit" -eq 0 ] && overall_exit=2
        if [ "$FAIL_FAST" -eq 1 ]; then
            log_error "L2 Standard tests failed - stopping"
            exit 2
        fi
    fi
    FORMAT=junit

    # Differential tests (100% required, blocking)
    if ! run_differential; then
        [ "$overall_exit" -eq 0 ] && overall_exit=3
        if [ "$FAIL_FAST" -eq 1 ]; then
            log_error "Differential tests failed - stopping"
            exit 3
        fi
    fi

    # L3 Extended (90%+ required, optional)
    if [ "$SKIP_L3" -eq 0 ]; then
        FORMAT=json
        if ! run_test_level "L3" "$L3_THRESHOLD" "$TIMEOUT_EXTENDED" "false"; then
            log_warn "L3 Extended tests below threshold (non-blocking)"
        fi
        FORMAT=junit
    else
        log_info "L3 Extended tests skipped"
    fi

    # L4 Performance (advisory, optional)
    if [ "$SKIP_L4" -eq 0 ]; then
        log_step "L4 Performance Tests (advisory)"
        FORMAT=json
        run_test_level "L4" "0" "$TIMEOUT_EXTENDED" "false" || true
        FORMAT=junit
    else
        log_info "L4 Performance tests skipped"
    fi

    # Generate summary
    generate_summary

    # Final status
    echo ""
    if [ "$overall_exit" -eq 0 ]; then
        log_success "VCS Full Test Suite PASSED"
    else
        log_error "VCS Full Test Suite FAILED (exit code: $overall_exit)"
    fi

    exit "$overall_exit"
}

# Run main
main "$@"
