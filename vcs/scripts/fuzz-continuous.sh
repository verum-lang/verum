#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Continuous Fuzzing Script
# =============================================================================
#
# This script runs continuous fuzzing campaigns for VCS targets. It manages
# corpus rotation, crash minimization, and reporting.
#
# Usage:
#   ./fuzz-continuous.sh [options]
#
# Options:
#   -t, --targets TARGETS   Fuzz targets: all, lexer, parser, cbgr, types, smt
#   -d, --duration DURATION Duration per cycle (default: 1h)
#   -c, --cycles N          Number of cycles (default: infinite)
#   -p, --parallel N        Number of parallel fuzzers (default: auto)
#   --corpus DIR            Corpus directory
#   --crashes DIR           Crashes output directory
#   --minimize              Minimize crash files after each cycle
#   --report                Generate report after each cycle
#   --notify                Send notifications on crashes
#   -v, --verbose           Verbose output
#   -h, --help              Show this help message
#
# Environment Variables:
#   VCS_FUZZ_TARGETS        Default targets
#   VCS_FUZZ_DURATION       Default duration
#   VCS_FUZZ_PARALLEL       Number of parallel workers
#   VCS_SLACK_WEBHOOK       Slack webhook URL for notifications
#
# Example:
#   ./fuzz-continuous.sh -t all -d 4h -c 6 --minimize --report
#   ./fuzz-continuous.sh -t cbgr -d 8h --notify
#
# Reference: VCS Spec Section 23.4 - Continuous Fuzzing
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Default configuration
TARGETS="${VCS_FUZZ_TARGETS:-all}"
DURATION="${VCS_FUZZ_DURATION:-1h}"
CYCLES=0  # 0 = infinite
PARALLEL="${VCS_FUZZ_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
CORPUS_DIR="$VCS_ROOT/fuzz/corpus"
CRASHES_DIR="$VCS_ROOT/fuzz/crashes"
MINIMIZE=0
GENERATE_REPORT=0
NOTIFY=0
VERBOSE=0
SLACK_WEBHOOK="${VCS_SLACK_WEBHOOK:-}"

# Timestamps
START_TIME=$(date +%s)
CYCLE_COUNT=0
TOTAL_CRASHES=0

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
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

# Print error and exit
error() {
    print_color "$RED" "ERROR: $1" >&2
    exit 1
}

# Print warning
warn() {
    print_color "$YELLOW" "WARNING: $1" >&2
}

# Print info
info() {
    print_color "$CYAN" "INFO: $1"
}

# Print verbose
verbose() {
    if [ "$VERBOSE" -eq 1 ]; then
        print_color "$BLUE" "VERBOSE: $1"
    fi
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -t|--targets)
                TARGETS="$2"
                shift 2
                ;;
            -d|--duration)
                DURATION="$2"
                shift 2
                ;;
            -c|--cycles)
                CYCLES="$2"
                shift 2
                ;;
            -p|--parallel)
                PARALLEL="$2"
                shift 2
                ;;
            --corpus)
                CORPUS_DIR="$2"
                shift 2
                ;;
            --crashes)
                CRASHES_DIR="$2"
                shift 2
                ;;
            --minimize)
                MINIMIZE=1
                shift
                ;;
            --report)
                GENERATE_REPORT=1
                shift
                ;;
            --notify)
                NOTIFY=1
                shift
                ;;
            -v|--verbose)
                VERBOSE=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error "Unknown option: $1"
                ;;
        esac
    done
}

# Convert duration string to seconds
duration_to_seconds() {
    local duration="$1"
    local value="${duration%[hms]}"
    local unit="${duration: -1}"

    case "$unit" in
        h) echo $((value * 3600)) ;;
        m) echo $((value * 60)) ;;
        s) echo "$value" ;;
        *) echo "$duration" ;;
    esac
}

# Get vfuzz binary
get_vfuzz() {
    local binary="$VCS_ROOT/runner/vfuzz/target/release/vfuzz"

    if [ ! -f "$binary" ]; then
        info "Building vfuzz..."
        (cd "$VCS_ROOT/runner/vfuzz" && cargo build --release) || error "Failed to build vfuzz"
    fi

    echo "$binary"
}

# Check if cargo-fuzz is available
check_cargo_fuzz() {
    if ! command -v cargo &> /dev/null; then
        error "cargo not found"
    fi

    if ! cargo +nightly fuzz --help &> /dev/null 2>&1; then
        warn "cargo-fuzz not found or nightly not installed"
        info "Installing cargo-fuzz..."
        cargo install cargo-fuzz --locked || warn "Failed to install cargo-fuzz"
    fi
}

# Get list of fuzz targets
get_targets() {
    local targets="$1"

    if [ "$targets" == "all" ]; then
        echo "lexer parser cbgr types smt"
    else
        echo "$targets" | tr ',' ' '
    fi
}

# Run a single fuzz cycle
run_fuzz_cycle() {
    local target="$1"
    local duration_sec="$2"
    local cycle_num="$3"
    local cycle_crashes_dir="$CRASHES_DIR/cycle-$cycle_num/$target"

    mkdir -p "$cycle_crashes_dir"
    mkdir -p "$CORPUS_DIR/$target"

    local vfuzz
    vfuzz=$(get_vfuzz)

    info "Cycle $cycle_num: Fuzzing $target for $(duration_to_seconds "$DURATION")s..."

    local start=$(date +%s)

    # Run the fuzzer
    timeout "$duration_sec" "$vfuzz" run \
        --targets "$target" \
        --duration "$DURATION" \
        --parallel "$PARALLEL" \
        --corpus "$CORPUS_DIR/$target" \
        --crashes "$cycle_crashes_dir" \
        --config "$VCS_ROOT/config/vcs.toml" \
        2>&1 | tee "$CRASHES_DIR/cycle-$cycle_num/$target.log" || true

    local end=$(date +%s)
    local runtime=$((end - start))

    # Count crashes
    local crash_count
    crash_count=$(find "$cycle_crashes_dir" -type f 2>/dev/null | wc -l || echo "0")

    if [ "$crash_count" -gt 0 ]; then
        print_color "$RED" "Cycle $cycle_num: Found $crash_count crash(es) in $target"
        TOTAL_CRASHES=$((TOTAL_CRASHES + crash_count))
    else
        print_color "$GREEN" "Cycle $cycle_num: No crashes in $target (${runtime}s)"
    fi

    echo "$crash_count"
}

# Run cargo-fuzz for a target
run_cargo_fuzz() {
    local target="$1"
    local duration_sec="$2"
    local cycle_num="$3"

    cd "$VCS_ROOT/fuzz"

    local fuzz_target="${target}_fuzz"

    info "Cycle $cycle_num: cargo-fuzz $fuzz_target for ${duration_sec}s..."

    mkdir -p "artifacts/$target"

    timeout "$duration_sec" cargo +nightly fuzz run "$fuzz_target" \
        --jobs "$PARALLEL" \
        -- \
        -max_len=16384 \
        -max_total_time="$duration_sec" \
        -artifact_prefix="artifacts/$target/" \
        -print_final_stats=1 \
        2>&1 | tee "reports/${target}_cycle${cycle_num}.log" || true

    local crash_count
    crash_count=$(find "artifacts/$target" -type f 2>/dev/null | wc -l || echo "0")

    if [ "$crash_count" -gt 0 ]; then
        # Copy crashes to crashes directory
        mkdir -p "$CRASHES_DIR/cycle-$cycle_num/$target"
        cp "artifacts/$target"/* "$CRASHES_DIR/cycle-$cycle_num/$target/" 2>/dev/null || true
        TOTAL_CRASHES=$((TOTAL_CRASHES + crash_count))
    fi

    echo "$crash_count"
}

# Minimize crash files
minimize_crashes() {
    local cycle_num="$1"
    local cycle_dir="$CRASHES_DIR/cycle-$cycle_num"

    if [ ! -d "$cycle_dir" ]; then
        return
    fi

    info "Minimizing crashes from cycle $cycle_num..."

    cd "$VCS_ROOT/fuzz"

    for target_dir in "$cycle_dir"/*/; do
        if [ -d "$target_dir" ]; then
            local target
            target=$(basename "$target_dir")
            local fuzz_target="${target}_fuzz"

            for crash in "$target_dir"*; do
                if [ -f "$crash" ] && [ ! -f "${crash}.min" ]; then
                    verbose "Minimizing: $crash"
                    timeout 300 cargo +nightly fuzz tmin "$fuzz_target" "$crash" \
                        -o "${crash}.min" 2>/dev/null || true
                fi
            done
        fi
    done

    print_color "$GREEN" "Crash minimization complete"
}

# Generate cycle report
generate_cycle_report() {
    local cycle_num="$1"
    local report_file="$CRASHES_DIR/cycle-$cycle_num/report.md"

    info "Generating report for cycle $cycle_num..."

    cat > "$report_file" << EOF
# Fuzzing Cycle Report: Cycle $cycle_num

**Date:** $(date -u +"%Y-%m-%d %H:%M:%S UTC")
**Duration:** $DURATION
**Targets:** $TARGETS
**Parallel Workers:** $PARALLEL

## Results by Target

| Target | Crashes | Corpus Size |
|--------|---------|-------------|
EOF

    for target in $(get_targets "$TARGETS"); do
        local crash_count corpus_size
        crash_count=$(find "$CRASHES_DIR/cycle-$cycle_num/$target" -type f 2>/dev/null | wc -l || echo "0")
        corpus_size=$(find "$CORPUS_DIR/$target" -type f 2>/dev/null | wc -l || echo "0")
        echo "| $target | $crash_count | $corpus_size |" >> "$report_file"
    done

    cat >> "$report_file" << EOF

## Summary

- **Cycle:** $cycle_num
- **Total Crashes (this cycle):** $(find "$CRASHES_DIR/cycle-$cycle_num" -type f ! -name "*.log" ! -name "*.md" ! -name "*.min" 2>/dev/null | wc -l || echo "0")
- **Total Crashes (all time):** $TOTAL_CRASHES

EOF

    print_color "$GREEN" "Report generated: $report_file"
}

# Send notification
send_notification() {
    local message="$1"

    if [ -n "$SLACK_WEBHOOK" ]; then
        curl -s -X POST -H 'Content-type: application/json' \
            --data "{\"text\":\"$message\"}" \
            "$SLACK_WEBHOOK" > /dev/null 2>&1 || true
    fi

    # Also log to file
    echo "[$(date -u +"%Y-%m-%d %H:%M:%S UTC")] $message" >> "$CRASHES_DIR/notifications.log"
}

# Print status banner
print_banner() {
    echo ""
    print_color "$CYAN" "========================================"
    print_color "$CYAN" "  VCS Continuous Fuzzing Campaign"
    print_color "$CYAN" "========================================"
    echo ""
    print_color "$BLUE" "Targets:      $TARGETS"
    print_color "$BLUE" "Duration:     $DURATION per cycle"
    print_color "$BLUE" "Cycles:       $([ "$CYCLES" -eq 0 ] && echo "infinite" || echo "$CYCLES")"
    print_color "$BLUE" "Parallel:     $PARALLEL"
    print_color "$BLUE" "Corpus:       $CORPUS_DIR"
    print_color "$BLUE" "Crashes:      $CRASHES_DIR"
    print_color "$BLUE" "Minimize:     $([ "$MINIMIZE" -eq 1 ] && echo "yes" || echo "no")"
    print_color "$BLUE" "Reports:      $([ "$GENERATE_REPORT" -eq 1 ] && echo "yes" || echo "no")"
    print_color "$BLUE" "Notify:       $([ "$NOTIFY" -eq 1 ] && echo "yes" || echo "no")"
    echo ""
}

# Print final summary
print_summary() {
    local end_time=$(date +%s)
    local total_runtime=$((end_time - START_TIME))
    local hours=$((total_runtime / 3600))
    local minutes=$(((total_runtime % 3600) / 60))

    echo ""
    print_color "$CYAN" "========================================"
    print_color "$CYAN" "  Fuzzing Campaign Summary"
    print_color "$CYAN" "========================================"
    echo ""
    print_color "$BLUE" "Total Runtime:    ${hours}h ${minutes}m"
    print_color "$BLUE" "Cycles Completed: $CYCLE_COUNT"
    print_color "$BLUE" "Total Crashes:    $TOTAL_CRASHES"
    echo ""

    if [ "$TOTAL_CRASHES" -gt 0 ]; then
        print_color "$YELLOW" "Crashes found! Review: $CRASHES_DIR"
    else
        print_color "$GREEN" "No crashes found during this campaign."
    fi
}

# Handle interrupt
cleanup() {
    echo ""
    print_color "$YELLOW" "Interrupt received, cleaning up..."
    print_summary
    exit 0
}

trap cleanup SIGINT SIGTERM

# Main function
main() {
    parse_args "$@"

    # Setup directories
    mkdir -p "$CORPUS_DIR"
    mkdir -p "$CRASHES_DIR"

    # Print banner
    print_banner

    # Check dependencies
    check_cargo_fuzz

    local duration_sec
    duration_sec=$(duration_to_seconds "$DURATION")

    info "Starting continuous fuzzing campaign..."

    if [ "$NOTIFY" -eq 1 ]; then
        send_notification "VCS Fuzzing Campaign Started - Targets: $TARGETS, Duration: $DURATION/cycle"
    fi

    # Main fuzzing loop
    while true; do
        CYCLE_COUNT=$((CYCLE_COUNT + 1))

        print_color "$MAGENTA" ""
        print_color "$MAGENTA" "========== Cycle $CYCLE_COUNT =========="
        print_color "$MAGENTA" ""

        local cycle_start=$(date +%s)
        local cycle_crashes=0

        # Run fuzzing for each target
        for target in $(get_targets "$TARGETS"); do
            local target_crashes
            target_crashes=$(run_fuzz_cycle "$target" "$duration_sec" "$CYCLE_COUNT")
            cycle_crashes=$((cycle_crashes + target_crashes))
        done

        local cycle_end=$(date +%s)
        local cycle_runtime=$((cycle_end - cycle_start))

        info "Cycle $CYCLE_COUNT complete: ${cycle_runtime}s, $cycle_crashes crash(es)"

        # Minimize crashes if requested
        if [ "$MINIMIZE" -eq 1 ] && [ "$cycle_crashes" -gt 0 ]; then
            minimize_crashes "$CYCLE_COUNT"
        fi

        # Generate report if requested
        if [ "$GENERATE_REPORT" -eq 1 ]; then
            generate_cycle_report "$CYCLE_COUNT"
        fi

        # Send notification if crashes found
        if [ "$NOTIFY" -eq 1 ] && [ "$cycle_crashes" -gt 0 ]; then
            send_notification "VCS Fuzzing Alert: Cycle $CYCLE_COUNT found $cycle_crashes crash(es)! Total: $TOTAL_CRASHES"
        fi

        # Check if we should stop
        if [ "$CYCLES" -gt 0 ] && [ "$CYCLE_COUNT" -ge "$CYCLES" ]; then
            info "Completed $CYCLES cycles, stopping..."
            break
        fi

        # Brief pause between cycles
        sleep 5
    done

    # Final summary
    print_summary

    if [ "$NOTIFY" -eq 1 ]; then
        send_notification "VCS Fuzzing Campaign Complete - $CYCLE_COUNT cycles, $TOTAL_CRASHES total crashes"
    fi

    # Exit with error if crashes found
    if [ "$TOTAL_CRASHES" -gt 0 ]; then
        exit 1
    fi
}

# Run main
main "$@"
