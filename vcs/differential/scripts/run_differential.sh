#!/usr/bin/env bash
#
# run_differential.sh - Run differential tests between Tier 0 and Tier 3
#
# Usage:
#   ./run_differential.sh [OPTIONS] [TEST_PATH]
#
# Options:
#   -v, --verbose     Show detailed output
#   -q, --quiet       Minimal output
#   -j, --jobs N      Number of parallel jobs (default: number of CPUs)
#   -t, --timeout N   Timeout in seconds per test (default: 30)
#   -o, --output DIR  Output directory for reports (default: ./reports)
#   -f, --format FMT  Report format: text, json, markdown (default: text)
#   --tier0 PATH      Path to interpreter binary
#   --tier3 PATH      Path to AOT binary
#   --only-failed     Only show failed tests
#   --save-outputs    Save stdout/stderr to files
#   --compare-py      Use Python comparison script for semantic equivalence
#   -h, --help        Show this help
#
# Examples:
#   ./run_differential.sh tier-oracle/
#   ./run_differential.sh -v -j 4 cross-impl/edge_cases.vr
#   ./run_differential.sh --only-failed --format markdown tier-oracle/

set -euo pipefail

# Configuration defaults
VERBOSE=0
QUIET=0
JOBS=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
TIMEOUT=30
OUTPUT_DIR="./reports"
FORMAT="text"
TIER0_BIN="${VERUM_INTERPRET:-verum-interpret}"
TIER3_BIN="${VERUM_RUN:-verum-run}"
ONLY_FAILED=0
SAVE_OUTPUTS=0
USE_PYTHON_COMPARE=0
TEST_PATH=""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIFF_DIR="$(dirname "$SCRIPT_DIR")"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -v|--verbose)
            VERBOSE=1
            shift
            ;;
        -q|--quiet)
            QUIET=1
            shift
            ;;
        -j|--jobs)
            JOBS="$2"
            shift 2
            ;;
        -t|--timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -f|--format)
            FORMAT="$2"
            shift 2
            ;;
        --tier0)
            TIER0_BIN="$2"
            shift 2
            ;;
        --tier3)
            TIER3_BIN="$2"
            shift 2
            ;;
        --only-failed)
            ONLY_FAILED=1
            shift
            ;;
        --save-outputs)
            SAVE_OUTPUTS=1
            shift
            ;;
        --compare-py)
            USE_PYTHON_COMPARE=1
            shift
            ;;
        -h|--help)
            head -30 "$0" | tail -28
            exit 0
            ;;
        -*)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
        *)
            TEST_PATH="$1"
            shift
            ;;
    esac
done

# Default test path
if [[ -z "$TEST_PATH" ]]; then
    TEST_PATH="$DIFF_DIR/tier-oracle"
fi

# Ensure output directory exists
mkdir -p "$OUTPUT_DIR"

# Log functions
log_info() {
    if [[ $QUIET -eq 0 ]]; then
        echo -e "${BLUE}[INFO]${NC} $1"
    fi
}

log_success() {
    if [[ $QUIET -eq 0 ]]; then
        echo -e "${GREEN}[PASS]${NC} $1"
    fi
}

log_error() {
    echo -e "${RED}[FAIL]${NC} $1" >&2
}

log_warn() {
    if [[ $QUIET -eq 0 ]]; then
        echo -e "${YELLOW}[WARN]${NC} $1"
    fi
}

log_verbose() {
    if [[ $VERBOSE -eq 1 ]]; then
        echo -e "${BLUE}[DEBUG]${NC} $1"
    fi
}

# Check if binaries exist
check_binaries() {
    local missing=0

    if ! command -v "$TIER0_BIN" &>/dev/null; then
        log_warn "Tier 0 binary not found: $TIER0_BIN"
        log_warn "Set VERUM_INTERPRET or use --tier0"
        missing=1
    fi

    if ! command -v "$TIER3_BIN" &>/dev/null; then
        log_warn "Tier 3 binary not found: $TIER3_BIN"
        log_warn "Set VERUM_RUN or use --tier3"
        missing=1
    fi

    if [[ $missing -eq 1 ]]; then
        log_info "Running in dry-run mode (no actual execution)"
        return 1
    fi

    return 0
}

# Run a single test
run_test() {
    local test_file="$1"
    local test_name=$(basename "$test_file" .vr)
    local temp_dir=$(mktemp -d)

    local tier0_stdout="$temp_dir/tier0_stdout.txt"
    local tier0_stderr="$temp_dir/tier0_stderr.txt"
    local tier3_stdout="$temp_dir/tier3_stdout.txt"
    local tier3_stderr="$temp_dir/tier3_stderr.txt"

    local tier0_exit=0
    local tier3_exit=0

    log_verbose "Running test: $test_file"

    # Run Tier 0 (interpreter)
    local tier0_start=$(date +%s%N)
    timeout "$TIMEOUT" "$TIER0_BIN" "$test_file" >"$tier0_stdout" 2>"$tier0_stderr" || tier0_exit=$?
    local tier0_end=$(date +%s%N)
    local tier0_time=$(( (tier0_end - tier0_start) / 1000000 ))

    # Run Tier 3 (AOT)
    local tier3_start=$(date +%s%N)
    timeout "$TIMEOUT" "$TIER3_BIN" "$test_file" >"$tier3_stdout" 2>"$tier3_stderr" || tier3_exit=$?
    local tier3_end=$(date +%s%N)
    local tier3_time=$(( (tier3_end - tier3_start) / 1000000 ))

    # Compare outputs
    local result="pass"
    local diff_output=""

    if [[ $tier0_exit -ne $tier3_exit ]]; then
        result="fail"
        diff_output="Exit codes differ: Tier 0 = $tier0_exit, Tier 3 = $tier3_exit"
    elif [[ $USE_PYTHON_COMPARE -eq 1 ]]; then
        # Use Python script for semantic comparison
        if ! python3 "$SCRIPT_DIR/compare_outputs.py" "$tier0_stdout" "$tier3_stdout" >/dev/null 2>&1; then
            result="fail"
            diff_output=$(python3 "$SCRIPT_DIR/compare_outputs.py" "$tier0_stdout" "$tier3_stdout" 2>&1)
        fi
    else
        # Simple diff comparison
        if ! diff -q "$tier0_stdout" "$tier3_stdout" >/dev/null 2>&1; then
            result="fail"
            diff_output=$(diff -u "$tier0_stdout" "$tier3_stdout" 2>&1 || true)
        fi
    fi

    # Save outputs if requested
    if [[ $SAVE_OUTPUTS -eq 1 ]]; then
        local out_dir="$OUTPUT_DIR/outputs/$test_name"
        mkdir -p "$out_dir"
        cp "$tier0_stdout" "$out_dir/tier0_stdout.txt"
        cp "$tier0_stderr" "$out_dir/tier0_stderr.txt"
        cp "$tier3_stdout" "$out_dir/tier3_stdout.txt"
        cp "$tier3_stderr" "$out_dir/tier3_stderr.txt"
    fi

    # Output result
    if [[ $result == "pass" ]]; then
        if [[ $ONLY_FAILED -eq 0 ]]; then
            log_success "$test_name (T0: ${tier0_time}ms, T3: ${tier3_time}ms)"
        fi
        echo "$test_file,pass,$tier0_time,$tier3_time,$tier0_exit,$tier3_exit"
    else
        log_error "$test_name"
        if [[ $VERBOSE -eq 1 ]]; then
            echo "  Exit codes: T0=$tier0_exit, T3=$tier3_exit"
            echo "  Times: T0=${tier0_time}ms, T3=${tier3_time}ms"
            if [[ -n "$diff_output" ]]; then
                echo "  Diff:"
                echo "$diff_output" | sed 's/^/    /'
            fi
        fi
        echo "$test_file,fail,$tier0_time,$tier3_time,$tier0_exit,$tier3_exit"
    fi

    # Cleanup
    rm -rf "$temp_dir"

    [[ $result == "pass" ]]
}

# Find all test files
find_tests() {
    local path="$1"

    if [[ -f "$path" ]]; then
        echo "$path"
    elif [[ -d "$path" ]]; then
        find "$path" -name "*.vr" -type f | sort
    else
        log_error "Path not found: $path"
        exit 1
    fi
}

# Generate report
generate_report() {
    local results_file="$1"
    local format="$2"
    local output_file="$OUTPUT_DIR/report"

    case $format in
        json)
            output_file="${output_file}.json"
            echo "[" > "$output_file"
            local first=1
            while IFS=, read -r file result tier0_time tier3_time tier0_exit tier3_exit; do
                [[ $first -eq 0 ]] && echo "," >> "$output_file"
                first=0
                cat >> "$output_file" <<EOF
{
  "file": "$file",
  "result": "$result",
  "tier0_time_ms": $tier0_time,
  "tier3_time_ms": $tier3_time,
  "tier0_exit": $tier0_exit,
  "tier3_exit": $tier3_exit
}
EOF
            done < "$results_file"
            echo "]" >> "$output_file"
            ;;

        markdown)
            output_file="${output_file}.md"
            cat > "$output_file" <<EOF
# Differential Test Report

Generated: $(date -Iseconds)

## Summary

| Metric | Value |
|--------|-------|
| Total Tests | $(wc -l < "$results_file") |
| Passed | $(grep -c ',pass,' "$results_file" || echo 0) |
| Failed | $(grep -c ',fail,' "$results_file" || echo 0) |

## Results

| File | Result | T0 Time (ms) | T3 Time (ms) | Speedup |
|------|--------|--------------|--------------|---------|
EOF
            while IFS=, read -r file result tier0_time tier3_time tier0_exit tier3_exit; do
                local speedup="N/A"
                if [[ $tier3_time -gt 0 ]]; then
                    speedup=$(echo "scale=2; $tier0_time / $tier3_time" | bc)
                fi
                echo "| \`$file\` | $result | $tier0_time | $tier3_time | ${speedup}x |" >> "$output_file"
            done < "$results_file"
            ;;

        *)
            output_file="${output_file}.txt"
            {
                echo "Differential Test Report"
                echo "========================"
                echo "Generated: $(date)"
                echo ""
                echo "Summary:"
                echo "  Total:  $(wc -l < "$results_file")"
                echo "  Passed: $(grep -c ',pass,' "$results_file" || echo 0)"
                echo "  Failed: $(grep -c ',fail,' "$results_file" || echo 0)"
                echo ""
                echo "Results:"
                while IFS=, read -r file result tier0_time tier3_time tier0_exit tier3_exit; do
                    printf "  %-50s %s (T0: %5dms, T3: %5dms)\n" "$file" "$result" "$tier0_time" "$tier3_time"
                done < "$results_file"
            } > "$output_file"
            ;;
    esac

    log_info "Report written to: $output_file"
}

# Main execution
main() {
    log_info "Differential Testing: Tier 0 vs Tier 3"
    log_info "Test path: $TEST_PATH"
    log_info "Parallel jobs: $JOBS"
    log_info "Timeout: ${TIMEOUT}s"

    # Check binaries
    local dry_run=0
    if ! check_binaries; then
        dry_run=1
    fi

    # Find test files
    local tests=()
    while IFS= read -r file; do
        tests+=("$file")
    done < <(find_tests "$TEST_PATH")

    log_info "Found ${#tests[@]} test files"

    if [[ ${#tests[@]} -eq 0 ]]; then
        log_warn "No test files found"
        exit 0
    fi

    # Dry run mode
    if [[ $dry_run -eq 1 ]]; then
        log_info "Dry run - listing tests only:"
        for test in "${tests[@]}"; do
            echo "  $test"
        done
        exit 0
    fi

    # Results file
    local results_file="$OUTPUT_DIR/results.csv"
    : > "$results_file"

    # Run tests
    local passed=0
    local failed=0
    local start_time=$(date +%s)

    # Parallel execution using xargs or sequential
    if [[ $JOBS -gt 1 ]] && command -v parallel &>/dev/null; then
        log_verbose "Using GNU parallel"
        export -f run_test log_verbose log_success log_error
        export TIER0_BIN TIER3_BIN TIMEOUT VERBOSE QUIET ONLY_FAILED SAVE_OUTPUTS USE_PYTHON_COMPARE OUTPUT_DIR SCRIPT_DIR
        export RED GREEN YELLOW BLUE NC

        printf '%s\n' "${tests[@]}" | parallel -j "$JOBS" run_test {} >> "$results_file"
    else
        for test in "${tests[@]}"; do
            if run_test "$test" >> "$results_file"; then
                ((passed++))
            else
                ((failed++))
            fi
        done
    fi

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))

    # Count results from file
    passed=$(grep -c ',pass,' "$results_file" 2>/dev/null || echo 0)
    failed=$(grep -c ',fail,' "$results_file" 2>/dev/null || echo 0)
    local total=$((passed + failed))

    # Print summary
    echo ""
    log_info "============================================"
    log_info "SUMMARY"
    log_info "============================================"
    log_info "Total:    $total"
    log_success "Passed:   $passed"
    if [[ $failed -gt 0 ]]; then
        log_error "Failed:   $failed"
    else
        log_info "Failed:   0"
    fi
    log_info "Duration: ${duration}s"
    log_info "============================================"

    # Generate report
    generate_report "$results_file" "$FORMAT"

    # Exit with appropriate code
    [[ $failed -eq 0 ]]
}

main
