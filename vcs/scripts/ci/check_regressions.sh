#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Regression Checker
# =============================================================================
#
# Compares current test results with a baseline to detect regressions.
# Used in CI to prevent merging changes that degrade test pass rates.
#
# Usage:
#   ./check_regressions.sh [options]
#
# Options:
#   --current FILE       Current results file (JSON)
#   --baseline FILE      Baseline results file (JSON)
#   --threshold PERCENT  Regression threshold percentage (default: 0)
#   --benchmark-current  Current benchmark results file
#   --benchmark-baseline Baseline benchmark results file
#   --perf-threshold     Performance regression threshold (default: 10)
#   --output FILE        Output comparison report
#   --strict             Fail on any regression
#   --verbose            Enable verbose output
#   -h, --help           Show this help message
#
# Exit Codes:
#   0 - No regressions detected
#   1 - Test pass rate regression detected
#   2 - Performance regression detected
#   3 - Both test and performance regressions
#   4 - Invalid input or configuration
#
# Environment Variables:
#   VCS_BASELINE_DIR      Directory containing baseline files
#   VCS_REGRESSION_STRICT Enable strict mode
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Configuration
CURRENT_FILE=""
BASELINE_FILE=""
THRESHOLD=0
BENCHMARK_CURRENT=""
BENCHMARK_BASELINE=""
PERF_THRESHOLD=10
OUTPUT_FILE=""
STRICT="${VCS_REGRESSION_STRICT:-0}"
VERBOSE=0

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
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[FAIL]${NC} $1" >&2; }

# Usage
usage() {
    head -n 45 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --current)
                CURRENT_FILE="$2"
                shift 2
                ;;
            --baseline)
                BASELINE_FILE="$2"
                shift 2
                ;;
            --threshold)
                THRESHOLD="$2"
                shift 2
                ;;
            --benchmark-current)
                BENCHMARK_CURRENT="$2"
                shift 2
                ;;
            --benchmark-baseline)
                BENCHMARK_BASELINE="$2"
                shift 2
                ;;
            --perf-threshold)
                PERF_THRESHOLD="$2"
                shift 2
                ;;
            --output)
                OUTPUT_FILE="$2"
                shift 2
                ;;
            --strict)
                STRICT=1
                shift
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
                exit 4
                ;;
        esac
    done
}

# Check test regressions
check_test_regression() {
    local current="$1"
    local baseline="$2"
    local threshold="$3"

    if [ ! -f "$current" ]; then
        log_error "Current results file not found: $current"
        return 4
    fi

    if [ ! -f "$baseline" ]; then
        log_warn "Baseline file not found: $baseline"
        log_info "Skipping test regression check (no baseline)"
        return 0
    fi

    # Extract pass rates
    local current_rate
    local baseline_rate

    current_rate=$(jq -r '.summary.pass_percentage // 0' "$current" 2>/dev/null || echo "0")
    baseline_rate=$(jq -r '.summary.pass_percentage // 0' "$baseline" 2>/dev/null || echo "0")

    local diff
    diff=$(echo "$baseline_rate - $current_rate" | bc -l 2>/dev/null || echo "0")

    log_info "Test Pass Rate Comparison:"
    log_info "  Baseline: ${baseline_rate}%"
    log_info "  Current:  ${current_rate}%"
    log_info "  Delta:    ${diff}%"
    log_info "  Threshold: ${threshold}%"
    echo ""

    # Check for regression
    if (( $(echo "$diff > $threshold" | bc -l) )); then
        log_error "Test pass rate regression detected!"
        log_error "Pass rate dropped by ${diff}% (threshold: ${threshold}%)"
        return 1
    fi

    # Check for improvement
    if (( $(echo "$diff < 0" | bc -l) )); then
        local improvement
        improvement=$(echo "0 - $diff" | bc -l)
        log_success "Test pass rate improved by ${improvement}%"
    else
        log_success "No test regression detected"
    fi

    return 0
}

# Check performance regressions
check_perf_regression() {
    local current="$1"
    local baseline="$2"
    local threshold="$3"

    if [ -z "$current" ] || [ ! -f "$current" ]; then
        log_info "No benchmark results to compare"
        return 0
    fi

    if [ -z "$baseline" ] || [ ! -f "$baseline" ]; then
        log_warn "Benchmark baseline not found"
        log_info "Skipping performance regression check"
        return 0
    fi

    log_info "Performance Comparison:"
    echo ""

    local regressions=0
    local improvements=0
    local report=""

    # Compare each benchmark metric
    while IFS= read -r metric; do
        local name
        name=$(echo "$metric" | jq -r '.name')
        local current_val
        current_val=$(echo "$metric" | jq -r '.value')
        local baseline_val
        baseline_val=$(jq -r ".[] | select(.name == \"$name\") | .value" "$baseline" 2>/dev/null || echo "")

        if [ -z "$baseline_val" ] || [ "$baseline_val" = "null" ]; then
            continue
        fi

        local diff_pct
        if [ "$baseline_val" != "0" ]; then
            diff_pct=$(echo "scale=2; ($current_val - $baseline_val) * 100 / $baseline_val" | bc -l 2>/dev/null || echo "0")
        else
            diff_pct="0"
        fi

        local status="OK"
        if (( $(echo "$diff_pct > $threshold" | bc -l) )); then
            status="REGRESSION"
            ((regressions++))
        elif (( $(echo "$diff_pct < -$threshold" | bc -l) )); then
            status="IMPROVED"
            ((improvements++))
        fi

        if [ "$VERBOSE" -eq 1 ] || [ "$status" != "OK" ]; then
            echo "  $name: $baseline_val -> $current_val (${diff_pct}%) [$status]"
        fi

        report+="$name: $status (${diff_pct}%)\n"

    done < <(jq -c '.[]' "$current" 2>/dev/null)

    echo ""

    if [ "$regressions" -gt 0 ]; then
        log_error "Performance regressions detected: $regressions metrics"
        return 2
    elif [ "$improvements" -gt 0 ]; then
        log_success "Performance improved: $improvements metrics, no regressions"
    else
        log_success "No performance regressions detected"
    fi

    return 0
}

# Compare test details
compare_test_details() {
    local current="$1"
    local baseline="$2"

    if [ ! -f "$current" ] || [ ! -f "$baseline" ]; then
        return 0
    fi

    log_info "Detailed Test Comparison:"
    echo ""

    # Find new failures
    local new_failures
    new_failures=$(jq -r '
        .tests[]? | select(.status == "failed") | .name
    ' "$current" 2>/dev/null | sort)

    local old_failures
    old_failures=$(jq -r '
        .tests[]? | select(.status == "failed") | .name
    ' "$baseline" 2>/dev/null | sort)

    local truly_new
    truly_new=$(comm -23 <(echo "$new_failures") <(echo "$old_failures") 2>/dev/null || true)

    if [ -n "$truly_new" ]; then
        log_warn "New test failures:"
        echo "$truly_new" | while read -r test; do
            [ -n "$test" ] && echo "  - $test"
        done
        echo ""
    fi

    # Find fixed tests
    local fixed
    fixed=$(comm -13 <(echo "$new_failures") <(echo "$old_failures") 2>/dev/null || true)

    if [ -n "$fixed" ]; then
        log_success "Fixed tests:"
        echo "$fixed" | while read -r test; do
            [ -n "$test" ] && echo "  + $test"
        done
        echo ""
    fi
}

# Generate report
generate_report() {
    local output="$1"
    local test_result="$2"
    local perf_result="$3"

    if [ -z "$output" ]; then
        return 0
    fi

    local status="PASS"
    if [ "$test_result" -ne 0 ] || [ "$perf_result" -ne 0 ]; then
        status="FAIL"
    fi

    cat > "$output" << EOF
# VCS Regression Check Report

**Date:** $(date -u +"%Y-%m-%d %H:%M:%S UTC")
**Status:** $status

## Test Results

- Test Regression: $([ "$test_result" -eq 0 ] && echo "PASS" || echo "FAIL")
- Performance Regression: $([ "$perf_result" -eq 0 ] && echo "PASS" || echo "FAIL")

## Configuration

- Threshold: ${THRESHOLD}%
- Performance Threshold: ${PERF_THRESHOLD}%
- Strict Mode: $([ "$STRICT" -eq 1 ] && echo "Enabled" || echo "Disabled")

## Files

- Current: $CURRENT_FILE
- Baseline: $BASELINE_FILE
EOF

    log_info "Report generated: $output"
}

# Main function
main() {
    parse_args "$@"

    log_info "VCS Regression Checker"
    echo ""

    local test_result=0
    local perf_result=0

    # Check test regressions
    if [ -n "$CURRENT_FILE" ]; then
        check_test_regression "$CURRENT_FILE" "$BASELINE_FILE" "$THRESHOLD" || test_result=$?

        if [ "$test_result" -eq 0 ]; then
            compare_test_details "$CURRENT_FILE" "$BASELINE_FILE"
        fi
    fi

    # Check performance regressions
    if [ -n "$BENCHMARK_CURRENT" ]; then
        check_perf_regression "$BENCHMARK_CURRENT" "$BENCHMARK_BASELINE" "$PERF_THRESHOLD" || perf_result=$?
    fi

    # Generate report
    if [ -n "$OUTPUT_FILE" ]; then
        generate_report "$OUTPUT_FILE" "$test_result" "$perf_result"
    fi

    # Final result
    echo ""
    local exit_code=0

    if [ "$test_result" -eq 1 ]; then
        exit_code=1
    fi

    if [ "$perf_result" -eq 2 ]; then
        if [ "$exit_code" -eq 1 ]; then
            exit_code=3
        else
            exit_code=2
        fi
    fi

    if [ "$exit_code" -eq 0 ]; then
        log_success "No regressions detected"
    else
        log_error "Regressions detected (exit code: $exit_code)"
    fi

    exit "$exit_code"
}

# Run main
main "$@"
