#!/bin/bash
# run_fuzz.sh - Start a fuzzing campaign for Verum
#
# This script runs comprehensive fuzz testing against the Verum compiler.
# It supports multiple fuzzing backends and provides progress reporting.
#
# Usage:
#   ./run_fuzz.sh [OPTIONS]
#
# Options:
#   -t, --target TARGET   Fuzzing target: lexer, parser, typecheck, codegen, all (default: all)
#   -d, --duration SECS   Duration in seconds (default: 3600 = 1 hour)
#   -j, --jobs N          Number of parallel jobs (default: number of CPUs)
#   -s, --seed DIR        Seed corpus directory (default: seeds/)
#   -o, --output DIR      Output directory for crashes (default: crashes/)
#   -c, --coverage        Enable coverage-guided fuzzing
#   --sanitizers          Enable sanitizers (ASan, UBSan)
#   --minimize            Minimize crash corpus after fuzzing
#   -v, --verbose         Verbose output
#   -h, --help            Show this help message

set -euo pipefail

# Default configuration
TARGET="all"
DURATION=3600
JOBS=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
SEED_DIR="seeds"
OUTPUT_DIR="crashes"
COVERAGE=false
SANITIZERS=false
MINIMIZE=false
VERBOSE=false

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FUZZ_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_ROOT="$(dirname "$(dirname "$FUZZ_DIR")")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -t|--target)
            TARGET="$2"
            shift 2
            ;;
        -d|--duration)
            DURATION="$2"
            shift 2
            ;;
        -j|--jobs)
            JOBS="$2"
            shift 2
            ;;
        -s|--seed)
            SEED_DIR="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -c|--coverage)
            COVERAGE=true
            shift
            ;;
        --sanitizers)
            SANITIZERS=true
            shift
            ;;
        --minimize)
            MINIMIZE=true
            shift
            ;;
        -v|--verbose)
            VERBOSE=true
            shift
            ;;
        -h|--help)
            head -30 "$0" | tail -n +2 | sed 's/^# //'
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate target
case $TARGET in
    lexer|parser|typecheck|codegen|all)
        ;;
    *)
        log_error "Invalid target: $TARGET"
        log_info "Valid targets: lexer, parser, typecheck, codegen, all"
        exit 1
        ;;
esac

# Create output directories
mkdir -p "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR/lexer"
mkdir -p "$OUTPUT_DIR/parser"
mkdir -p "$OUTPUT_DIR/typecheck"
mkdir -p "$OUTPUT_DIR/codegen"
mkdir -p "$OUTPUT_DIR/reports"

# Display configuration
log_info "Fuzzing Campaign Configuration"
log_info "==============================="
log_info "Target:      $TARGET"
log_info "Duration:    ${DURATION}s"
log_info "Jobs:        $JOBS"
log_info "Seed Dir:    $SEED_DIR"
log_info "Output Dir:  $OUTPUT_DIR"
log_info "Coverage:    $COVERAGE"
log_info "Sanitizers:  $SANITIZERS"
log_info "Minimize:    $MINIMIZE"
echo

# Build the fuzzer
build_fuzzer() {
    log_info "Building fuzzer..."

    local BUILD_FLAGS=""

    if [[ "$COVERAGE" == "true" ]]; then
        BUILD_FLAGS="$BUILD_FLAGS --features coverage"
    fi

    if [[ "$SANITIZERS" == "true" ]]; then
        export RUSTFLAGS="-Zsanitizer=address,undefined"
    fi

    cd "$PROJECT_ROOT"

    if cargo build --release -p verum_fuzz $BUILD_FLAGS; then
        log_success "Fuzzer built successfully"
    else
        log_error "Failed to build fuzzer"
        exit 1
    fi
}

# Run fuzzing for a specific target
run_fuzz_target() {
    local target=$1
    local seed_subdir="$SEED_DIR"
    local output_subdir="$OUTPUT_DIR/$target"
    local start_time=$(date +%s)

    log_info "Starting fuzzing: $target"
    log_info "Seed corpus: $seed_subdir"
    log_info "Output: $output_subdir"

    # Check for seed corpus
    if [[ -d "$FUZZ_DIR/$seed_subdir" ]]; then
        local seed_count=$(find "$FUZZ_DIR/$seed_subdir" -name "*.vr" | wc -l)
        log_info "Found $seed_count seed files"
    else
        log_warning "No seed directory found, starting with empty corpus"
        mkdir -p "$FUZZ_DIR/$seed_subdir"
    fi

    # Prepare environment
    export VERUM_FUZZ_TARGET="$target"
    export VERUM_FUZZ_DURATION="$DURATION"
    export VERUM_FUZZ_JOBS="$JOBS"
    export VERUM_FUZZ_SEED_DIR="$FUZZ_DIR/$seed_subdir"
    export VERUM_FUZZ_OUTPUT_DIR="$output_subdir"

    # Run the appropriate fuzzer
    if command -v cargo-fuzz &> /dev/null; then
        # Use cargo-fuzz if available
        log_info "Using cargo-fuzz backend"
        cd "$PROJECT_ROOT"

        timeout "${DURATION}s" cargo fuzz run "fuzz_${target}" \
            -- -jobs="$JOBS" \
               -max_total_time="$DURATION" \
               -artifact_prefix="$output_subdir/" \
            || true
    else
        # Fall back to our custom runner
        log_info "Using custom fuzzer"
        cd "$PROJECT_ROOT"

        cargo run --release -p verum_fuzz --bin vfuzz -- \
            --target "$target" \
            --duration "$DURATION" \
            --jobs "$JOBS" \
            --seed-dir "$FUZZ_DIR/$seed_subdir" \
            --output-dir "$output_subdir" \
            || true
    fi

    local end_time=$(date +%s)
    local elapsed=$((end_time - start_time))
    log_success "Fuzzing $target completed in ${elapsed}s"

    # Count crashes
    local crash_count=$(find "$output_subdir" -name "crash*" -o -name "timeout*" 2>/dev/null | wc -l)
    if [[ $crash_count -gt 0 ]]; then
        log_warning "Found $crash_count crashes/timeouts for $target"
    else
        log_success "No crashes found for $target"
    fi
}

# Generate report
generate_report() {
    log_info "Generating fuzzing report..."

    local report_file="$OUTPUT_DIR/reports/fuzz_report_$(date +%Y%m%d_%H%M%S).txt"

    {
        echo "Verum Fuzzing Report"
        echo "===================="
        echo "Date: $(date)"
        echo "Duration: ${DURATION}s"
        echo "Target: $TARGET"
        echo "Jobs: $JOBS"
        echo ""

        for target in lexer parser typecheck codegen; do
            if [[ -d "$OUTPUT_DIR/$target" ]]; then
                local crash_count=$(find "$OUTPUT_DIR/$target" -name "crash*" 2>/dev/null | wc -l)
                local timeout_count=$(find "$OUTPUT_DIR/$target" -name "timeout*" 2>/dev/null | wc -l)
                echo "$target:"
                echo "  Crashes: $crash_count"
                echo "  Timeouts: $timeout_count"
                echo ""
            fi
        done

        echo "Crash Summary"
        echo "============="
        for crash in $(find "$OUTPUT_DIR" -name "crash*" 2>/dev/null | head -20); do
            echo "File: $crash"
            echo "Size: $(wc -c < "$crash") bytes"
            echo "Preview:"
            head -5 "$crash" 2>/dev/null || xxd "$crash" | head -5
            echo ""
        done

    } > "$report_file"

    log_success "Report saved to: $report_file"
}

# Main execution
main() {
    log_info "Starting Verum Fuzzing Campaign"
    log_info "================================"

    # Build the fuzzer
    build_fuzzer

    # Run fuzzing based on target
    if [[ "$TARGET" == "all" ]]; then
        for t in lexer parser typecheck codegen; do
            run_fuzz_target "$t"
        done
    else
        run_fuzz_target "$TARGET"
    fi

    # Minimize crashes if requested
    if [[ "$MINIMIZE" == "true" ]]; then
        log_info "Minimizing crash corpus..."
        "$SCRIPT_DIR/minimize_crash.sh" --input "$OUTPUT_DIR" --output "$OUTPUT_DIR/minimized"
    fi

    # Generate report
    generate_report

    # Final summary
    echo
    log_success "Fuzzing campaign completed!"
    log_info "Results saved to: $OUTPUT_DIR"

    # Exit with error if crashes were found
    local total_crashes=$(find "$OUTPUT_DIR" -name "crash*" 2>/dev/null | wc -l)
    if [[ $total_crashes -gt 0 ]]; then
        log_warning "Total crashes found: $total_crashes"
        exit 1
    fi
}

main "$@"
