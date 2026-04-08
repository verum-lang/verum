#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Continuous Test Runner
# =============================================================================
#
# This script provides a continuous testing loop that:
#   1. Runs vtest for each level (L0-L4)
#   2. Collects results into structured JSON files
#   3. Generates human-readable reports
#   4. Saves results to vcs/results/ for historical tracking
#
# Usage:
#   ./continuous-test-runner.sh [options]
#
# Options:
#   -i, --interval SECS   Interval between runs in seconds (default: 0, single run)
#   -l, --levels LEVELS   Comma-separated levels to test (default: L0,L1,L2,L3,L4)
#   -p, --parallel N      Number of parallel workers (default: auto)
#   -o, --output-dir DIR  Output directory (default: vcs/results)
#   -n, --max-runs N      Maximum number of runs (default: unlimited with interval)
#   --no-archive          Don't archive to history directory
#   --notify              Enable desktop notifications on failures
#   --webhook URL         Send results to webhook URL
#   --fail-fast           Stop on first critical failure (L0/L1)
#   -v, --verbose         Verbose output
#   -q, --quiet           Quiet mode (minimal output)
#   -h, --help            Show this help message
#
# Exit Codes:
#   0 - All tests passed or completed runs
#   1 - Critical failures (L0/L1 below 100%)
#   2 - Standard failures (L2 below 95%)
#   3 - Configuration or runtime error
#
# Environment Variables:
#   VCS_CONTINUOUS_INTERVAL  Default interval between runs
#   VCS_PARALLEL             Number of parallel workers
#   VCS_WEBHOOK_URL          Webhook URL for notifications
#   VCS_NOTIFY               Enable notifications (1/0)
#
# Examples:
#   ./continuous-test-runner.sh                    # Single run, all levels
#   ./continuous-test-runner.sh -i 300             # Run every 5 minutes
#   ./continuous-test-runner.sh -l L0,L1 -i 60     # Test L0/L1 every minute
#   ./continuous-test-runner.sh -n 10 -i 120      # 10 runs, 2 min apart
#   ./continuous-test-runner.sh --webhook http://... --notify
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory and paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Configuration defaults
INTERVAL="${VCS_CONTINUOUS_INTERVAL:-0}"
LEVELS="L0,L1,L2,L3,L4"
PARALLEL="${VCS_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
OUTPUT_DIR="$VCS_ROOT/results"
MAX_RUNS=0
ARCHIVE=1
NOTIFY="${VCS_NOTIFY:-0}"
WEBHOOK_URL="${VCS_WEBHOOK_URL:-}"
FAIL_FAST=0
VERBOSE=0
QUIET=0
CONFIG="$VCS_ROOT/config/vcs.toml"

# Runtime state
RUN_COUNT=0
LAST_RUN_TIMESTAMP=""
SHOULD_EXIT=0

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# =============================================================================
# Utility Functions
# =============================================================================

print_color() {
    local color="$1"
    local message="$2"
    if [ "$QUIET" -eq 0 ]; then
        echo -e "${color}${message}${NC}"
    fi
}

log_info() {
    print_color "$CYAN" "[INFO] $1"
}

log_success() {
    print_color "$GREEN" "[SUCCESS] $1"
}

log_warning() {
    print_color "$YELLOW" "[WARNING] $1"
}

log_error() {
    print_color "$RED" "[ERROR] $1" >&2
}

log_verbose() {
    if [ "$VERBOSE" -eq 1 ]; then
        print_color "$DIM" "[VERBOSE] $1"
    fi
}

log_debug() {
    if [ "$VERBOSE" -eq 1 ]; then
        print_color "$MAGENTA" "[DEBUG] $1"
    fi
}

usage() {
    head -n 48 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

error_exit() {
    log_error "$1"
    exit "${2:-3}"
}

# Get current timestamp in ISO format
get_timestamp() {
    date -u +"%Y-%m-%dT%H:%M:%SZ"
}

# Get timestamp for filenames (filesystem safe)
get_file_timestamp() {
    date +"%Y%m%d-%H%M%S"
}

# =============================================================================
# Signal Handling
# =============================================================================

cleanup() {
    log_info "Cleaning up..."
    SHOULD_EXIT=1
}

trap cleanup SIGINT SIGTERM

# =============================================================================
# Dependency Checks
# =============================================================================

check_dependencies() {
    local missing=()

    if ! command -v cargo &> /dev/null; then
        missing+=("cargo (Rust toolchain)")
    fi

    if ! command -v jq &> /dev/null; then
        log_warning "jq not found - some reporting features will be limited"
    fi

    if ! command -v bc &> /dev/null; then
        log_warning "bc not found - percentage calculations may be limited"
    fi

    if [ ${#missing[@]} -gt 0 ]; then
        error_exit "Missing required dependencies: ${missing[*]}" 3
    fi
}

# =============================================================================
# vtest Binary Management
# =============================================================================

get_vtest_binary() {
    local binary="$VCS_ROOT/runner/vtest/target/release/vtest"

    if [ ! -f "$binary" ]; then
        log_info "Building vtest..."
        if ! (cd "$VCS_ROOT/runner/vtest" && cargo build --release 2>&1); then
            error_exit "Failed to build vtest" 3
        fi
    fi

    echo "$binary"
}

# =============================================================================
# Test Execution
# =============================================================================

run_level_tests() {
    local level="$1"
    local output_file="$2"
    local vtest
    vtest=$(get_vtest_binary)

    log_verbose "Running $level tests..."

    local args=(
        "run"
        "--level" "$level"
        "--parallel" "$PARALLEL"
        "--format" "json"
        "--output" "$output_file"
    )

    if [ -f "$CONFIG" ]; then
        args+=("--config" "$CONFIG")
    fi

    if [ "$VERBOSE" -eq 1 ]; then
        args+=("--verbose")
    fi

    # Run vtest and capture exit code
    local exit_code=0
    if ! (cd "$VCS_ROOT" && "$vtest" "${args[@]}" 2>&1); then
        exit_code=$?
        log_verbose "vtest exited with code $exit_code for $level"
    fi

    return $exit_code
}

run_differential_tests() {
    local output_file="$1"
    local vtest
    vtest=$(get_vtest_binary)

    log_verbose "Running differential tests..."

    local args=(
        "run"
        "--differential"
        "--tier" "0,3"
        "--parallel" "$PARALLEL"
        "--format" "json"
        "--output" "$output_file"
    )

    if [ -f "$CONFIG" ]; then
        args+=("--config" "$CONFIG")
    fi

    local exit_code=0
    if ! (cd "$VCS_ROOT" && "$vtest" "${args[@]}" 2>&1); then
        exit_code=$?
    fi

    return $exit_code
}

# =============================================================================
# Result Processing
# =============================================================================

extract_results() {
    local json_file="$1"

    if [ ! -f "$json_file" ]; then
        echo "0|0|0|0|0"
        return
    fi

    local total passed failed skipped duration
    total=$(jq -r '.summary.total // 0' "$json_file" 2>/dev/null || echo "0")
    passed=$(jq -r '.summary.passed // 0' "$json_file" 2>/dev/null || echo "0")
    failed=$(jq -r '.summary.failed // 0' "$json_file" 2>/dev/null || echo "0")
    skipped=$(jq -r '.summary.skipped // 0' "$json_file" 2>/dev/null || echo "0")
    duration=$(jq -r '.summary.duration_ms // 0' "$json_file" 2>/dev/null || echo "0")

    echo "$total|$passed|$failed|$skipped|$duration"
}

calculate_pass_rate() {
    local passed="$1"
    local total="$2"

    if [ "$total" -eq 0 ]; then
        echo "0.0"
        return
    fi

    if command -v bc &> /dev/null; then
        echo "scale=2; $passed * 100 / $total" | bc
    else
        # Fallback for systems without bc
        echo "$((passed * 100 / total))"
    fi
}

check_compliance() {
    local level="$1"
    local pass_rate="$2"

    local required status
    case "$level" in
        L0|L1)
            required=100
            ;;
        L2)
            required=95
            ;;
        L3)
            required=90
            ;;
        L4|DIFF)
            required=0
            ;;
        *)
            required=0
            ;;
    esac

    if command -v bc &> /dev/null; then
        if [ "$(echo "$pass_rate >= $required" | bc -l 2>/dev/null)" -eq 1 ]; then
            status="PASS"
        else
            status="FAIL"
        fi
    else
        local pass_int="${pass_rate%.*}"
        if [ "$pass_int" -ge "$required" ]; then
            status="PASS"
        else
            status="FAIL"
        fi
    fi

    echo "$required|$status"
}

# =============================================================================
# Report Generation
# =============================================================================

generate_run_summary() {
    local run_dir="$1"
    local timestamp="$2"
    local summary_file="$run_dir/summary.json"

    log_verbose "Generating run summary..."

    local levels_json="{"
    local first=1
    local overall_status="PASS"
    local total_tests=0
    local total_passed=0
    local total_failed=0
    local total_duration=0

    # Process each level result file
    for level in L0 L1 L2 L3 L4; do
        local level_file="$run_dir/${level,,}-results.json"
        if [ -f "$level_file" ]; then
            local results
            results=$(extract_results "$level_file")
            IFS='|' read -r total passed failed skipped duration <<< "$results"

            local pass_rate
            pass_rate=$(calculate_pass_rate "$passed" "$total")

            local compliance
            compliance=$(check_compliance "$level" "$pass_rate")
            IFS='|' read -r required status <<< "$compliance"

            if [ $first -eq 0 ]; then
                levels_json+=","
            fi
            first=0

            levels_json+="\"$level\":{\"total\":$total,\"passed\":$passed,\"failed\":$failed,\"skipped\":$skipped,\"duration_ms\":$duration,\"pass_rate\":$pass_rate,\"required\":$required,\"status\":\"$status\"}"

            total_tests=$((total_tests + total))
            total_passed=$((total_passed + passed))
            total_failed=$((total_failed + failed))
            total_duration=$((total_duration + duration))

            # Update overall status
            if [ "$status" = "FAIL" ]; then
                case "$level" in
                    L0|L1|L2)
                        overall_status="FAIL"
                        ;;
                    L3)
                        if [ "$overall_status" = "PASS" ]; then
                            overall_status="WARN"
                        fi
                        ;;
                esac
            fi
        fi
    done

    # Process differential results if present
    local diff_file="$run_dir/differential-results.json"
    if [ -f "$diff_file" ]; then
        local results
        results=$(extract_results "$diff_file")
        IFS='|' read -r total passed failed skipped duration <<< "$results"

        local pass_rate
        pass_rate=$(calculate_pass_rate "$passed" "$total")

        if [ $first -eq 0 ]; then
            levels_json+=","
        fi

        levels_json+="\"DIFF\":{\"total\":$total,\"passed\":$passed,\"failed\":$failed,\"skipped\":$skipped,\"duration_ms\":$duration,\"pass_rate\":$pass_rate,\"required\":100,\"status\":\"$([ "$failed" -eq 0 ] && echo "PASS" || echo "FAIL")\"}"

        if [ "$failed" -gt 0 ]; then
            overall_status="FAIL"
        fi
    fi

    levels_json+="}"

    # Calculate overall pass rate
    local overall_pass_rate
    overall_pass_rate=$(calculate_pass_rate "$total_passed" "$total_tests")

    # Generate summary JSON
    cat > "$summary_file" << EOF
{
    "timestamp": "$timestamp",
    "run_number": $RUN_COUNT,
    "overall": {
        "status": "$overall_status",
        "total_tests": $total_tests,
        "total_passed": $total_passed,
        "total_failed": $total_failed,
        "pass_rate": $overall_pass_rate,
        "duration_ms": $total_duration
    },
    "levels": $levels_json,
    "config": {
        "parallel": $PARALLEL,
        "levels_tested": "$(echo "$LEVELS" | tr ',' ' ')"
    }
}
EOF

    echo "$overall_status"
}

generate_console_report() {
    local run_dir="$1"
    local summary_file="$run_dir/summary.json"

    if [ ! -f "$summary_file" ]; then
        log_warning "No summary file found at $summary_file"
        return
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "        VCS CONTINUOUS TEST RUNNER - RUN #$RUN_COUNT"
    print_color "$CYAN" "================================================================"
    echo ""

    local timestamp overall_status total_tests pass_rate duration
    timestamp=$(jq -r '.timestamp' "$summary_file")
    overall_status=$(jq -r '.overall.status' "$summary_file")
    total_tests=$(jq -r '.overall.total_tests' "$summary_file")
    pass_rate=$(jq -r '.overall.pass_rate' "$summary_file")
    duration=$(jq -r '.overall.duration_ms' "$summary_file")

    print_color "$DIM" "  Timestamp: $timestamp"
    print_color "$DIM" "  Duration:  ${duration}ms"
    echo ""

    # Print level results table
    printf "  ${BOLD}%-8s %-8s %-8s %-8s %-10s %-10s %-8s${NC}\n" \
        "Level" "Total" "Passed" "Failed" "Rate" "Required" "Status"
    printf "  %-8s %-8s %-8s %-8s %-10s %-10s %-8s\n" \
        "------" "------" "------" "------" "--------" "--------" "------"

    for level in L0 L1 L2 L3 L4 DIFF; do
        local level_data
        level_data=$(jq -r ".levels.\"$level\" // null" "$summary_file")

        if [ "$level_data" != "null" ]; then
            local total passed failed rate required status
            total=$(echo "$level_data" | jq -r '.total')
            passed=$(echo "$level_data" | jq -r '.passed')
            failed=$(echo "$level_data" | jq -r '.failed')
            rate=$(echo "$level_data" | jq -r '.pass_rate')
            required=$(echo "$level_data" | jq -r '.required')
            status=$(echo "$level_data" | jq -r '.status')

            local status_color="$GREEN"
            if [ "$status" = "FAIL" ]; then
                status_color="$RED"
            elif [ "$status" = "WARN" ]; then
                status_color="$YELLOW"
            fi

            printf "  %-8s %-8s %-8s %-8s %-10s %-10s ${status_color}%-8s${NC}\n" \
                "$level" "$total" "$passed" "$failed" "${rate}%" "${required}%" "$status"
        fi
    done

    echo ""
    print_color "$CYAN" "----------------------------------------------------------------"

    # Overall result
    local overall_color="$GREEN"
    if [ "$overall_status" = "FAIL" ]; then
        overall_color="$RED"
    elif [ "$overall_status" = "WARN" ]; then
        overall_color="$YELLOW"
    fi

    printf "  ${BOLD}OVERALL:${NC} ${overall_color}%s${NC}  " "$overall_status"
    printf "(${total_tests} tests, ${pass_rate}%% passed)\n"
    print_color "$CYAN" "================================================================"
    echo ""
}

# =============================================================================
# Archive and History
# =============================================================================

archive_results() {
    local run_dir="$1"
    local timestamp="$2"

    if [ "$ARCHIVE" -eq 0 ]; then
        log_verbose "Archiving disabled"
        return
    fi

    local history_dir="$OUTPUT_DIR/history"
    local archive_name="run-$timestamp"
    local archive_dir="$history_dir/$archive_name"

    mkdir -p "$archive_dir"

    # Copy all result files
    cp -r "$run_dir"/* "$archive_dir/" 2>/dev/null || true

    log_verbose "Archived results to $archive_dir"

    # Clean up old archives (keep last 100)
    local archive_count
    archive_count=$(find "$history_dir" -maxdepth 1 -type d -name "run-*" | wc -l)

    if [ "$archive_count" -gt 100 ]; then
        log_verbose "Cleaning up old archives (keeping last 100)..."
        find "$history_dir" -maxdepth 1 -type d -name "run-*" | \
            sort | head -n "$((archive_count - 100))" | \
            xargs rm -rf
    fi
}

update_latest() {
    local run_dir="$1"
    local latest_dir="$OUTPUT_DIR/latest"

    # Clear and update latest directory
    rm -rf "$latest_dir"/*
    mkdir -p "$latest_dir"
    cp -r "$run_dir"/* "$latest_dir/" 2>/dev/null || true

    log_verbose "Updated latest results in $latest_dir"
}

# =============================================================================
# Notifications
# =============================================================================

send_notification() {
    local title="$1"
    local message="$2"
    local urgency="${3:-normal}"

    if [ "$NOTIFY" -eq 0 ]; then
        return
    fi

    # macOS notification
    if command -v osascript &> /dev/null; then
        osascript -e "display notification \"$message\" with title \"$title\"" 2>/dev/null || true
    fi

    # Linux notification (notify-send)
    if command -v notify-send &> /dev/null; then
        notify-send -u "$urgency" "$title" "$message" 2>/dev/null || true
    fi
}

send_webhook() {
    local summary_file="$1"

    if [ -z "$WEBHOOK_URL" ]; then
        return
    fi

    if ! command -v curl &> /dev/null; then
        log_warning "curl not found, cannot send webhook"
        return
    fi

    log_verbose "Sending results to webhook..."

    local payload
    payload=$(cat "$summary_file")

    curl -s -X POST \
        -H "Content-Type: application/json" \
        -d "$payload" \
        "$WEBHOOK_URL" 2>/dev/null || log_warning "Failed to send webhook"
}

# =============================================================================
# Main Test Run
# =============================================================================

execute_run() {
    RUN_COUNT=$((RUN_COUNT + 1))
    local timestamp
    timestamp=$(get_file_timestamp)
    LAST_RUN_TIMESTAMP="$timestamp"

    log_info "Starting run #$RUN_COUNT at $(get_timestamp)"

    # Create run directory
    local run_dir="$OUTPUT_DIR/run-$timestamp"
    mkdir -p "$run_dir"

    local has_critical_failure=0
    local has_standard_failure=0

    # Run tests for each level
    IFS=',' read -ra LEVEL_ARRAY <<< "$LEVELS"
    for level in "${LEVEL_ARRAY[@]}"; do
        level=$(echo "$level" | tr -d ' ' | tr '[:lower:]' '[:upper:]')

        local output_file="$run_dir/${level,,}-results.json"

        log_info "Testing $level..."

        if ! run_level_tests "$level" "$output_file"; then
            log_warning "$level tests encountered errors"
        fi

        # Check for critical failures with fail-fast
        if [ "$FAIL_FAST" -eq 1 ] && [ -f "$output_file" ]; then
            local results
            results=$(extract_results "$output_file")
            IFS='|' read -r total passed failed skipped duration <<< "$results"
            local pass_rate
            pass_rate=$(calculate_pass_rate "$passed" "$total")

            if [ "$level" = "L0" ] || [ "$level" = "L1" ]; then
                if [ "$(echo "$pass_rate < 100" | bc -l 2>/dev/null || echo "1")" -eq 1 ]; then
                    has_critical_failure=1
                    log_error "Critical failure in $level - stopping due to --fail-fast"
                    break
                fi
            fi
        fi
    done

    # Run differential tests if not in fail-fast mode or no critical failure
    if [ "$has_critical_failure" -eq 0 ]; then
        local diff_output="$run_dir/differential-results.json"
        log_info "Testing differential (Tier 0 vs Tier 3)..."
        run_differential_tests "$diff_output" || true
    fi

    # Generate summary
    log_info "Generating summary..."
    local overall_status
    overall_status=$(generate_run_summary "$run_dir" "$(get_timestamp)")

    # Update latest directory
    update_latest "$run_dir"

    # Archive to history
    archive_results "$run_dir" "$timestamp"

    # Generate console report
    generate_console_report "$run_dir"

    # Send notifications
    if [ "$overall_status" = "FAIL" ]; then
        send_notification "VCS Test Failure" "Run #$RUN_COUNT failed - see results for details" "critical"
        has_standard_failure=1
    elif [ "$overall_status" = "WARN" ]; then
        send_notification "VCS Test Warning" "Run #$RUN_COUNT has warnings" "normal"
    fi

    # Send webhook
    local summary_file="$run_dir/summary.json"
    if [ -f "$summary_file" ]; then
        send_webhook "$summary_file"
    fi

    # Clean up run directory (keep in latest and history)
    rm -rf "$run_dir"

    # Determine exit code
    if [ "$has_critical_failure" -eq 1 ]; then
        return 1
    elif [ "$has_standard_failure" -eq 1 ]; then
        return 2
    else
        return 0
    fi
}

# =============================================================================
# Argument Parsing
# =============================================================================

parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -i|--interval)
                INTERVAL="$2"
                shift 2
                ;;
            -l|--levels)
                LEVELS="$2"
                shift 2
                ;;
            -p|--parallel)
                PARALLEL="$2"
                shift 2
                ;;
            -o|--output-dir)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            -n|--max-runs)
                MAX_RUNS="$2"
                shift 2
                ;;
            --no-archive)
                ARCHIVE=0
                shift
                ;;
            --notify)
                NOTIFY=1
                shift
                ;;
            --webhook)
                WEBHOOK_URL="$2"
                shift 2
                ;;
            --fail-fast)
                FAIL_FAST=1
                shift
                ;;
            -v|--verbose)
                VERBOSE=1
                shift
                ;;
            -q|--quiet)
                QUIET=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error_exit "Unknown option: $1" 3
                ;;
        esac
    done
}

# =============================================================================
# Main Entry Point
# =============================================================================

main() {
    parse_args "$@"

    # Check dependencies
    check_dependencies

    # Create output directories
    mkdir -p "$OUTPUT_DIR"/{latest,history,baselines}

    # Print header
    if [ "$QUIET" -eq 0 ]; then
        echo ""
        print_color "$CYAN" "================================================================"
        print_color "$CYAN" "        VCS CONTINUOUS TEST RUNNER"
        print_color "$CYAN" "================================================================"
        echo ""
        print_color "$DIM" "  Levels:    $LEVELS"
        print_color "$DIM" "  Parallel:  $PARALLEL"
        print_color "$DIM" "  Interval:  ${INTERVAL}s"
        print_color "$DIM" "  Max runs:  $([ "$MAX_RUNS" -eq 0 ] && echo "unlimited" || echo "$MAX_RUNS")"
        print_color "$DIM" "  Output:    $OUTPUT_DIR"
        echo ""
    fi

    local exit_code=0

    # Single run mode (interval = 0)
    if [ "$INTERVAL" -eq 0 ]; then
        execute_run
        exit_code=$?
    else
        # Continuous mode
        log_info "Starting continuous testing (interval: ${INTERVAL}s)"
        log_info "Press Ctrl+C to stop"
        echo ""

        while [ "$SHOULD_EXIT" -eq 0 ]; do
            execute_run
            exit_code=$?

            # Check max runs
            if [ "$MAX_RUNS" -gt 0 ] && [ "$RUN_COUNT" -ge "$MAX_RUNS" ]; then
                log_info "Reached maximum runs ($MAX_RUNS)"
                break
            fi

            # Check for fail-fast exit
            if [ "$FAIL_FAST" -eq 1 ] && [ "$exit_code" -eq 1 ]; then
                log_error "Exiting due to critical failure (--fail-fast)"
                break
            fi

            # Wait for next run
            if [ "$SHOULD_EXIT" -eq 0 ]; then
                log_info "Next run in ${INTERVAL}s... (Ctrl+C to stop)"
                sleep "$INTERVAL" &
                wait $! 2>/dev/null || true
            fi
        done
    fi

    # Final summary
    if [ "$QUIET" -eq 0 ]; then
        echo ""
        print_color "$CYAN" "================================================================"
        print_color "$DIM" "  Completed $RUN_COUNT run(s)"
        print_color "$DIM" "  Results saved to: $OUTPUT_DIR/latest/"
        print_color "$CYAN" "================================================================"
        echo ""
    fi

    exit $exit_code
}

# Run main
main "$@"
