#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Main Test Runner Script
# =============================================================================
#
# This script provides a unified interface for running VCS tests locally.
# It supports all test levels (L0-L4), differential testing, and reporting.
#
# Usage:
#   ./run-vcs.sh [options] [command]
#
# Commands:
#   run       Run test suite (default)
#   fuzz      Run fuzzing tests
#   bench     Run benchmarks
#   diff      Run differential tests
#   report    Generate HTML report
#   check     Run compliance check
#
# Options:
#   -l, --level LEVEL     Test level: L0, L1, L2, L3, L4, all (default: all)
#   -t, --tier TIER       Execution tier: 0, 1, 2, 3, all (default: all)
#   -p, --parallel N      Number of parallel workers (default: auto)
#   -f, --format FORMAT   Output format: console, json, junit, html
#   -o, --output FILE     Output file for results
#   -v, --verbose         Verbose output
#   -q, --quiet           Quiet mode (errors only)
#   --fail-fast           Stop on first failure
#   --timeout MS          Timeout per test in milliseconds
#   --config FILE         Configuration file path
#   -h, --help            Show this help message
#
# Examples:
#   ./run-vcs.sh                    # Run all tests
#   ./run-vcs.sh -l L0              # Run only L0 critical tests
#   ./run-vcs.sh -l L0,L1 -p 8      # Run L0 and L1 with 8 workers
#   ./run-vcs.sh bench --filter cbgr # Run CBGR benchmarks only
#   ./run-vcs.sh diff -t 0,3        # Differential test Tier 0 vs 3
#   ./run-vcs.sh report             # Generate HTML report
#
# Environment Variables:
#   VCS_PARALLEL          Number of parallel workers
#   VCS_TIMEOUT           Default timeout (ms)
#   VCS_CONFIG            Configuration file path
#   VCS_OUTPUT_DIR        Output directory for reports
#   VCS_VERBOSE           Enable verbose mode (1/0)
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Default configuration
COMMAND="run"
LEVEL="all"
TIER="all"
PARALLEL="${VCS_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
FORMAT="${VCS_FORMAT:-console}"
OUTPUT=""
VERBOSE="${VCS_VERBOSE:-0}"
QUIET=0
FAIL_FAST=0
TIMEOUT="${VCS_TIMEOUT:-30000}"
CONFIG="${VCS_CONFIG:-$VCS_ROOT/config/vcs.toml}"
FILTER=""
DURATION="1h"
OUTPUT_DIR="${VCS_OUTPUT_DIR:-$VCS_ROOT/reports}"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Print colored message
print_color() {
    local color="$1"
    local message="$2"
    echo -e "${color}${message}${NC}"
}

# Print usage
usage() {
    head -n 45 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
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
    if [ "$QUIET" -eq 0 ]; then
        print_color "$CYAN" "INFO: $1"
    fi
}

# Print verbose message
verbose() {
    if [ "$VERBOSE" -eq 1 ]; then
        print_color "$BLUE" "VERBOSE: $1"
    fi
}

# Check if required tools are installed
check_dependencies() {
    local missing=()

    # Check for Rust toolchain
    if ! command -v cargo &> /dev/null; then
        missing+=("cargo (Rust toolchain)")
    fi

    # Check for jq (optional but recommended)
    if ! command -v jq &> /dev/null; then
        warn "jq not found - JSON processing will be limited"
    fi

    if [ ${#missing[@]} -gt 0 ]; then
        error "Missing required dependencies: ${missing[*]}"
    fi
}

# Build VCS tools if needed
build_tools() {
    local tools=("vtest" "vfuzz" "vbench")

    for tool in "${tools[@]}"; do
        local tool_dir="$VCS_ROOT/runner/$tool"
        local binary="$tool_dir/target/release/$tool"

        if [ ! -f "$binary" ]; then
            info "Building $tool..."
            (cd "$tool_dir" && cargo build --release) || error "Failed to build $tool"
        fi
    done
}

# Get tool binary path
get_tool_binary() {
    local tool="$1"
    local binary="$VCS_ROOT/runner/$tool/target/release/$tool"

    if [ ! -f "$binary" ]; then
        error "Tool binary not found: $binary. Run with --build or build manually."
    fi

    echo "$binary"
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            run|fuzz|bench|diff|report|check)
                COMMAND="$1"
                shift
                ;;
            -l|--level)
                LEVEL="$2"
                shift 2
                ;;
            -t|--tier)
                TIER="$2"
                shift 2
                ;;
            -p|--parallel)
                PARALLEL="$2"
                shift 2
                ;;
            -f|--format)
                FORMAT="$2"
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
            -q|--quiet)
                QUIET=1
                shift
                ;;
            --fail-fast)
                FAIL_FAST=1
                shift
                ;;
            --timeout)
                TIMEOUT="$2"
                shift 2
                ;;
            --config)
                CONFIG="$2"
                shift 2
                ;;
            --filter)
                FILTER="$2"
                shift 2
                ;;
            --duration)
                DURATION="$2"
                shift 2
                ;;
            --build)
                build_tools
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

# Create output directory
setup_output_dir() {
    mkdir -p "$OUTPUT_DIR"
    verbose "Output directory: $OUTPUT_DIR"
}

# Run tests
cmd_run() {
    local vtest
    vtest=$(get_tool_binary "vtest")

    info "Running VCS tests (Level: $LEVEL, Tier: $TIER)"

    local args=()
    args+=("run")

    if [ "$LEVEL" != "all" ]; then
        args+=("--level" "$LEVEL")
    fi

    if [ "$TIER" != "all" ]; then
        args+=("--tier" "$TIER")
    fi

    args+=("--parallel" "$PARALLEL")
    args+=("--timeout" "$TIMEOUT")
    args+=("--format" "$FORMAT")
    args+=("--config" "$CONFIG")

    if [ -n "$OUTPUT" ]; then
        args+=("--output" "$OUTPUT")
    fi

    if [ "$FAIL_FAST" -eq 1 ]; then
        args+=("--fail-fast")
    fi

    if [ "$VERBOSE" -eq 1 ]; then
        args+=("--verbose")
    fi

    verbose "Executing: $vtest ${args[*]}"

    (cd "$VCS_ROOT" && "$vtest" "${args[@]}")
}

# Run fuzzing tests
cmd_fuzz() {
    local vfuzz
    vfuzz=$(get_tool_binary "vfuzz")

    info "Running VCS fuzzing (Duration: $DURATION)"

    local args=()
    args+=("run")
    args+=("--duration" "$DURATION")
    args+=("--parallel" "$PARALLEL")
    args+=("--config" "$CONFIG")

    if [ -n "$FILTER" ]; then
        args+=("--filter" "$FILTER")
    fi

    if [ -n "$OUTPUT" ]; then
        args+=("--output" "$OUTPUT")
    fi

    verbose "Executing: $vfuzz ${args[*]}"

    (cd "$VCS_ROOT" && "$vfuzz" "${args[@]}")
}

# Run benchmarks
cmd_bench() {
    local vbench
    vbench=$(get_tool_binary "vbench")

    info "Running VCS benchmarks"

    local args=()
    args+=("run")
    args+=("--config" "$CONFIG")

    if [ -n "$FILTER" ]; then
        args+=("--category" "$FILTER")
    fi

    args+=("--format" "$FORMAT")

    if [ -n "$OUTPUT" ]; then
        args+=("--output" "$OUTPUT")
    fi

    verbose "Executing: $vbench ${args[*]}"

    (cd "$VCS_ROOT" && "$vbench" "${args[@]}")
}

# Run differential tests
cmd_diff() {
    local vtest
    vtest=$(get_tool_binary "vtest")

    info "Running differential tests (Tiers: $TIER)"

    local args=()
    args+=("run")
    args+=("--differential")

    if [ "$TIER" != "all" ]; then
        args+=("--tier" "$TIER")
    else
        args+=("--tier" "0,3")  # Default to Tier 0 vs Tier 3
    fi

    args+=("--parallel" "$PARALLEL")
    args+=("--config" "$CONFIG")
    args+=("--format" "$FORMAT")

    if [ -n "$OUTPUT" ]; then
        args+=("--output" "$OUTPUT")
    fi

    verbose "Executing: $vtest ${args[*]}"

    (cd "$VCS_ROOT" && "$vtest" "${args[@]}")
}

# Generate report
cmd_report() {
    local vtest
    vtest=$(get_tool_binary "vtest")

    info "Generating VCS report"

    local output_file="${OUTPUT:-$OUTPUT_DIR/vcs-report.html}"

    local args=()
    args+=("report")
    args+=("--format" "html")
    args+=("--output" "$output_file")
    args+=("--config" "$CONFIG")

    verbose "Executing: $vtest ${args[*]}"

    (cd "$VCS_ROOT" && "$vtest" "${args[@]}")

    print_color "$GREEN" "Report generated: $output_file"
}

# Run compliance check
cmd_check() {
    info "Running VCS compliance check"

    # Run the check-compliance script
    "$SCRIPT_DIR/check-compliance.sh" "$@"
}

# Print summary header
print_header() {
    if [ "$QUIET" -eq 0 ]; then
        echo ""
        print_color "$CYAN" "========================================"
        print_color "$CYAN" "  Verum Compliance Suite (VCS)"
        print_color "$CYAN" "========================================"
        echo ""
        print_color "$BLUE" "Command:    $COMMAND"
        print_color "$BLUE" "Level:      $LEVEL"
        print_color "$BLUE" "Tier:       $TIER"
        print_color "$BLUE" "Parallel:   $PARALLEL"
        print_color "$BLUE" "Format:     $FORMAT"
        print_color "$BLUE" "Config:     $CONFIG"
        echo ""
    fi
}

# Main function
main() {
    parse_args "$@"

    # Check dependencies
    check_dependencies

    # Build tools if needed
    build_tools

    # Setup output directory
    setup_output_dir

    # Print header
    print_header

    # Execute command
    case $COMMAND in
        run)
            cmd_run
            ;;
        fuzz)
            cmd_fuzz
            ;;
        bench)
            cmd_bench
            ;;
        diff)
            cmd_diff
            ;;
        report)
            cmd_report
            ;;
        check)
            cmd_check
            ;;
        *)
            error "Unknown command: $COMMAND"
            ;;
    esac

    print_color "$GREEN" "Done!"
}

# Run main
main "$@"
