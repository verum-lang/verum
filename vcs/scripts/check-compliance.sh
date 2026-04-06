#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Compliance Level Checker
# =============================================================================
#
# This script verifies VCS compliance levels according to the spec.
# It runs all test levels and reports the compliance status.
#
# Usage:
#   ./check-compliance.sh [options]
#
# Options:
#   -l, --level LEVEL     Check specific level: L0, L1, L2, L3, L4, all
#   -o, --output FILE     Output file for compliance report (JSON)
#   -v, --verbose         Verbose output
#   --strict              Fail on any compliance violation
#   --ci                  CI mode - output JUnit XML
#   -h, --help            Show this help message
#
# Exit Codes:
#   0 - All compliance requirements met
#   1 - L0 or L1 tests failed (critical)
#   2 - L2 tests below 95% threshold
#   3 - Differential tests failed
#   4 - Configuration or runtime error
#
# Compliance Requirements (from VCS Spec Section 23.2):
#   L0 (Critical): 100% pass - blocking
#   L1 (Core):     100% pass - blocking
#   L2 (Standard): 95%+ pass - blocking
#   L3 (Extended): 90%+ pass - recommended
#   L4 (Perf):     Advisory only
#   Differential:  100% pass - blocking
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Configuration
LEVEL="all"
OUTPUT=""
VERBOSE=0
STRICT=0
CI_MODE=0
PARALLEL="${VCS_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
CONFIG="$VCS_ROOT/config/vcs.toml"
REPORTS_DIR="$VCS_ROOT/reports/compliance"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Print colored message
print_color() {
    local color="$1"
    local message="$2"
    echo -e "${color}${message}${NC}"
}

# Print usage
usage() {
    head -n 35 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -l|--level)
                LEVEL="$2"
                shift 2
                ;;
            -o|--output)
                OUTPUT="$2"
                shift 2
                ;;
            -v|--verbose)
                VERBOSE=1
                shift
                ;;
            --strict)
                STRICT=1
                shift
                ;;
            --ci)
                CI_MODE=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                echo "Unknown option: $1" >&2
                usage
                exit 4
                ;;
        esac
    done
}

# Get vtest binary
get_vtest() {
    local binary="$VCS_ROOT/runner/vtest/target/release/vtest"
    if [ ! -f "$binary" ]; then
        echo "Building vtest..."
        (cd "$VCS_ROOT/runner/vtest" && cargo build --release) || exit 4
    fi
    echo "$binary"
}

# Run tests for a level and extract pass rate
run_level() {
    local level="$1"
    local vtest
    vtest=$(get_vtest)

    local output_file="$REPORTS_DIR/${level,,}-results.json"
    mkdir -p "$REPORTS_DIR"

    if [ "$VERBOSE" -eq 1 ]; then
        echo "Running $level tests..."
    fi

    # Run tests and capture output
    (cd "$VCS_ROOT" && "$vtest" run \
        --level "$level" \
        --parallel "$PARALLEL" \
        --format json \
        --output "$output_file" \
        --config "$CONFIG" \
        2>&1) || true

    # Extract results
    if [ -f "$output_file" ]; then
        local total passed failed pass_rate
        total=$(jq -r '.summary.total // 0' "$output_file" 2>/dev/null || echo "0")
        passed=$(jq -r '.summary.passed // 0' "$output_file" 2>/dev/null || echo "0")
        failed=$(jq -r '.summary.failed // 0' "$output_file" 2>/dev/null || echo "0")
        pass_rate=$(jq -r '.summary.pass_percentage // 0' "$output_file" 2>/dev/null || echo "0")

        echo "$level|$total|$passed|$failed|$pass_rate"
    else
        echo "$level|0|0|0|0"
    fi
}

# Check compliance for a level
check_level_compliance() {
    local level="$1"
    local pass_rate="$2"
    local required

    case "$level" in
        L0)
            required=100
            ;;
        L1)
            required=100
            ;;
        L2)
            required=95
            ;;
        L3)
            required=90
            ;;
        L4)
            required=0  # Advisory only
            ;;
        *)
            required=0
            ;;
    esac

    local status
    if (( $(echo "$pass_rate >= $required" | bc -l 2>/dev/null || echo "0") )); then
        status="PASS"
    else
        status="FAIL"
    fi

    echo "$required|$status"
}

# Run differential tests
run_differential() {
    local vtest
    vtest=$(get_vtest)

    local output_file="$REPORTS_DIR/differential-results.json"
    mkdir -p "$REPORTS_DIR"

    if [ "$VERBOSE" -eq 1 ]; then
        echo "Running differential tests..."
    fi

    (cd "$VCS_ROOT" && "$vtest" run \
        --differential \
        --tier 0,3 \
        --parallel "$PARALLEL" \
        --format json \
        --output "$output_file" \
        --config "$CONFIG" \
        2>&1) || true

    if [ -f "$output_file" ]; then
        local total equivalent divergent
        total=$(jq -r '.summary.total // 0' "$output_file" 2>/dev/null || echo "0")
        equivalent=$(jq -r '.summary.equivalent // 0' "$output_file" 2>/dev/null || echo "0")
        divergent=$(jq -r '.summary.divergent // 0' "$output_file" 2>/dev/null || echo "0")

        if [ "$divergent" -eq 0 ]; then
            echo "DIFF|$total|$equivalent|0|PASS"
        else
            echo "DIFF|$total|$equivalent|$divergent|FAIL"
        fi
    else
        echo "DIFF|0|0|0|UNKNOWN"
    fi
}

# Generate compliance report
generate_report() {
    local results=("$@")
    local timestamp
    timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    local overall_status="PASS"
    local blocking_failed=0

    echo "{"
    echo "  \"timestamp\": \"$timestamp\","
    echo "  \"levels\": {"

    local first=1
    for result in "${results[@]}"; do
        IFS='|' read -r level total passed failed pass_rate required status <<< "$result"

        if [ $first -eq 0 ]; then
            echo ","
        fi
        first=0

        echo "    \"$level\": {"
        echo "      \"total\": $total,"
        echo "      \"passed\": $passed,"
        echo "      \"failed\": $failed,"
        echo "      \"pass_rate\": $pass_rate,"
        echo "      \"required\": $required,"
        echo "      \"status\": \"$status\""
        echo -n "    }"

        if [ "$status" = "FAIL" ]; then
            case "$level" in
                L0|L1|L2|DIFF)
                    blocking_failed=1
                    overall_status="FAIL"
                    ;;
                L3)
                    if [ "$STRICT" -eq 1 ]; then
                        overall_status="WARN"
                    fi
                    ;;
            esac
        fi
    done

    echo ""
    echo "  },"
    echo "  \"overall_status\": \"$overall_status\","
    echo "  \"blocking_failed\": $blocking_failed"
    echo "}"
}

# Print summary table
print_summary() {
    local results=("$@")

    echo ""
    print_color "$CYAN" "========================================"
    print_color "$CYAN" "  VCS Compliance Summary"
    print_color "$CYAN" "========================================"
    echo ""

    printf "%-12s %-8s %-8s %-8s %-12s %-10s %-8s\n" \
        "Level" "Total" "Passed" "Failed" "Pass Rate" "Required" "Status"
    printf "%-12s %-8s %-8s %-8s %-12s %-10s %-8s\n" \
        "--------" "------" "------" "------" "----------" "--------" "------"

    local overall_status="PASS"
    local exit_code=0

    for result in "${results[@]}"; do
        IFS='|' read -r level total passed failed pass_rate required status <<< "$result"

        local status_color="$GREEN"
        if [ "$status" = "FAIL" ]; then
            status_color="$RED"
            case "$level" in
                L0|L1)
                    exit_code=1
                    overall_status="FAIL"
                    ;;
                L2)
                    exit_code=2
                    overall_status="FAIL"
                    ;;
                DIFF)
                    exit_code=3
                    overall_status="FAIL"
                    ;;
                L3)
                    if [ "$STRICT" -eq 1 ]; then
                        exit_code=2
                        overall_status="WARN"
                    fi
                    ;;
            esac
        elif [ "$status" = "WARN" ]; then
            status_color="$YELLOW"
        fi

        printf "%-12s %-8s %-8s %-8s %-12s %-10s " \
            "$level" "$total" "$passed" "$failed" "${pass_rate}%" "${required}%"
        print_color "$status_color" "$status"
    done

    echo ""

    if [ "$overall_status" = "PASS" ]; then
        print_color "$GREEN" "Overall: COMPLIANT"
    elif [ "$overall_status" = "WARN" ]; then
        print_color "$YELLOW" "Overall: PARTIALLY COMPLIANT"
    else
        print_color "$RED" "Overall: NON-COMPLIANT"
    fi

    echo ""

    return $exit_code
}

# Main function
main() {
    parse_args "$@"

    # Create reports directory
    mkdir -p "$REPORTS_DIR"

    print_color "$CYAN" "Verum Compliance Suite - Compliance Check"
    echo ""

    # Collect results
    local results=()

    # Run test levels
    if [ "$LEVEL" = "all" ] || [ "$LEVEL" = "L0" ]; then
        local l0_result
        l0_result=$(run_level "L0")
        IFS='|' read -r _ total passed failed pass_rate <<< "$l0_result"
        IFS='|' read -r required status <<< "$(check_level_compliance "L0" "$pass_rate")"
        results+=("L0|$total|$passed|$failed|$pass_rate|$required|$status")
    fi

    if [ "$LEVEL" = "all" ] || [ "$LEVEL" = "L1" ]; then
        local l1_result
        l1_result=$(run_level "L1")
        IFS='|' read -r _ total passed failed pass_rate <<< "$l1_result"
        IFS='|' read -r required status <<< "$(check_level_compliance "L1" "$pass_rate")"
        results+=("L1|$total|$passed|$failed|$pass_rate|$required|$status")
    fi

    if [ "$LEVEL" = "all" ] || [ "$LEVEL" = "L2" ]; then
        local l2_result
        l2_result=$(run_level "L2")
        IFS='|' read -r _ total passed failed pass_rate <<< "$l2_result"
        IFS='|' read -r required status <<< "$(check_level_compliance "L2" "$pass_rate")"
        results+=("L2|$total|$passed|$failed|$pass_rate|$required|$status")
    fi

    if [ "$LEVEL" = "all" ] || [ "$LEVEL" = "L3" ]; then
        local l3_result
        l3_result=$(run_level "L3")
        IFS='|' read -r _ total passed failed pass_rate <<< "$l3_result"
        IFS='|' read -r required status <<< "$(check_level_compliance "L3" "$pass_rate")"
        results+=("L3|$total|$passed|$failed|$pass_rate|$required|$status")
    fi

    if [ "$LEVEL" = "all" ] || [ "$LEVEL" = "L4" ]; then
        local l4_result
        l4_result=$(run_level "L4")
        IFS='|' read -r _ total passed failed pass_rate <<< "$l4_result"
        IFS='|' read -r required status <<< "$(check_level_compliance "L4" "$pass_rate")"
        results+=("L4|$total|$passed|$failed|$pass_rate|$required|$status")
    fi

    # Run differential tests
    if [ "$LEVEL" = "all" ]; then
        local diff_result
        diff_result=$(run_differential)
        IFS='|' read -r _ total equiv divergent status <<< "$diff_result"
        results+=("DIFF|$total|$equiv|$divergent|100|100|$status")
    fi

    # Generate JSON report if requested
    if [ -n "$OUTPUT" ]; then
        generate_report "${results[@]}" > "$OUTPUT"
        echo "Compliance report written to: $OUTPUT"
    fi

    # Print summary
    print_summary "${results[@]}"
    local exit_code=$?

    # Save report
    local report_file="$REPORTS_DIR/compliance-$(date +%Y%m%d-%H%M%S).json"
    generate_report "${results[@]}" > "$report_file"

    if [ "$VERBOSE" -eq 1 ]; then
        echo "Full report saved to: $report_file"
    fi

    exit $exit_code
}

# Run main
main "$@"
