#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Nightly Test Runner
# =============================================================================
#
# Runs the complete nightly test suite including:
# - Full test suite (L0-L4)
# - Extended differential tests
# - Fuzzing (configurable duration)
# - Performance benchmarks
# - Regression analysis
#
# Usage:
#   ./run_nightly.sh [options]
#
# Options:
#   --parallel N         Number of parallel workers (default: auto)
#   --fuzz-duration DUR  Fuzzing duration (default: 2h)
#   --output-dir DIR     Output directory for reports
#   --skip-fuzz          Skip fuzzing tests
#   --skip-bench         Skip benchmarks
#   --skip-coverage      Skip coverage generation
#   --notify             Send notifications on completion
#   --verbose            Enable verbose output
#   -h, --help           Show this help message
#
# Environment Variables:
#   VCS_PARALLEL          Override parallel workers
#   VCS_FUZZ_DURATION     Override fuzz duration
#   SLACK_WEBHOOK_URL     Slack notification webhook
#   DISCORD_WEBHOOK_URL   Discord notification webhook
#   EMAIL_RECIPIENTS      Comma-separated email addresses
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Configuration
PARALLEL="${VCS_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 8)}"
FUZZ_DURATION="${VCS_FUZZ_DURATION:-2h}"
TIMEOUT="30000"
TIMEOUT_EXTENDED="600000"
OUTPUT_DIR="${VCS_OUTPUT_DIR:-$VCS_ROOT/reports/nightly}"
CONFIG="${VCS_CONFIG:-$VCS_ROOT/config/vcs.toml}"
SKIP_FUZZ=0
SKIP_BENCH=0
SKIP_COVERAGE=0
NOTIFY=0
VERBOSE=0

# Tool paths
VTEST="$VCS_ROOT/runner/vtest/target/release/vtest"
VFUZZ="$VCS_ROOT/runner/vfuzz/target/release/vfuzz"
VBENCH="$VCS_ROOT/runner/vbench/target/release/vbench"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Logging
log_info() { echo -e "${CYAN}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[FAIL]${NC} $1" >&2; }
log_step() {
    echo ""
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo ""
}

# Usage
usage() {
    head -n 45 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --parallel)
                PARALLEL="$2"
                shift 2
                ;;
            --fuzz-duration)
                FUZZ_DURATION="$2"
                shift 2
                ;;
            --output-dir)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            --skip-fuzz)
                SKIP_FUZZ=1
                shift
                ;;
            --skip-bench)
                SKIP_BENCH=1
                shift
                ;;
            --skip-coverage)
                SKIP_COVERAGE=1
                shift
                ;;
            --notify)
                NOTIFY=1
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
                exit 1
                ;;
        esac
    done
}

# Build tools
build_tools() {
    log_info "Building VCS tools..."

    for tool in vtest vfuzz vbench; do
        local tool_dir="$VCS_ROOT/runner/$tool"
        local binary="$tool_dir/target/release/$tool"

        if [ ! -f "$binary" ]; then
            log_info "Building $tool..."
            cargo build --release --manifest-path "$tool_dir/Cargo.toml"
        fi
    done
}

# Run full test suite
run_full_tests() {
    log_step "Full Test Suite (L0-L4)"

    local args=()
    if [ "$VERBOSE" -eq 1 ]; then
        args+=(--verbose)
    fi

    "$SCRIPT_DIR/run_all_tests.sh" \
        --parallel "$PARALLEL" \
        --timeout "$TIMEOUT" \
        --output-dir "$OUTPUT_DIR" \
        --format json \
        "${args[@]}" || true

    return 0  # Continue even if tests fail
}

# Run extended differential tests
run_differential() {
    log_step "Extended Differential Tests"

    mkdir -p "$OUTPUT_DIR/differential"

    # All tier combinations
    for combo in "0,1" "1,2" "2,3" "0,2" "0,3"; do
        local tier1="${combo%,*}"
        local tier2="${combo#*,}"

        log_info "Testing Tier $tier1 vs Tier $tier2..."

        "$VTEST" run \
            "$VCS_ROOT/differential/" \
            --tier "$combo" \
            --parallel "$PARALLEL" \
            --format json \
            --output "$OUTPUT_DIR/differential/tier-${tier1}-vs-${tier2}.json" \
            --config "$CONFIG" || true
    done

    log_success "Differential tests completed"
}

# Run fuzzing
run_fuzzing() {
    if [ "$SKIP_FUZZ" -eq 1 ]; then
        log_info "Fuzzing skipped"
        return 0
    fi

    log_step "Fuzzing ($FUZZ_DURATION)"

    mkdir -p "$VCS_ROOT/fuzz/crashes" "$VCS_ROOT/fuzz/artifacts"

    "$VFUZZ" run \
        --targets all \
        --duration "$FUZZ_DURATION" \
        --parallel "$PARALLEL" \
        --corpus "$VCS_ROOT/fuzz/seeds/" \
        --crashes "$VCS_ROOT/fuzz/crashes/" \
        --config "$CONFIG" || true

    # Check for crashes
    local crash_count=0
    if [ -d "$VCS_ROOT/fuzz/crashes" ]; then
        crash_count=$(find "$VCS_ROOT/fuzz/crashes" -type f 2>/dev/null | wc -l)
    fi

    if [ "$crash_count" -gt 0 ]; then
        log_warn "Fuzzing found $crash_count crashes"
        ls -la "$VCS_ROOT/fuzz/crashes/" || true
    else
        log_success "No crashes found during fuzzing"
    fi

    # Copy crash artifacts
    if [ "$crash_count" -gt 0 ]; then
        mkdir -p "$OUTPUT_DIR/crashes"
        cp -r "$VCS_ROOT/fuzz/crashes/"* "$OUTPUT_DIR/crashes/" 2>/dev/null || true
    fi
}

# Run benchmarks
run_benchmarks() {
    if [ "$SKIP_BENCH" -eq 1 ]; then
        log_info "Benchmarks skipped"
        return 0
    fi

    log_step "Performance Benchmarks"

    mkdir -p "$OUTPUT_DIR/benchmarks"

    "$VBENCH" run \
        --suite all \
        --iterations 1000 \
        --warmup 100 \
        --config "$CONFIG" \
        --format json \
        --output "$OUTPUT_DIR/benchmarks/results.json" || true

    # Compare to baseline if exists
    local baseline="$VCS_ROOT/baselines/benchmark-baseline.json"
    if [ -f "$baseline" ]; then
        log_info "Comparing to baseline..."

        "$VBENCH" compare \
            --current "$OUTPUT_DIR/benchmarks/results.json" \
            --baseline "$baseline" \
            --threshold 10.0 \
            --format markdown \
            --output "$OUTPUT_DIR/benchmarks/comparison.md" || true

        if grep -q "REGRESSION" "$OUTPUT_DIR/benchmarks/comparison.md" 2>/dev/null; then
            log_warn "Performance regression detected"
        fi
    else
        log_info "No baseline found - skipping comparison"
    fi

    # Validate thresholds
    "$VBENCH" validate \
        --thresholds "$VCS_ROOT/config/thresholds.toml" \
        --results "$OUTPUT_DIR/benchmarks/results.json" || true

    log_success "Benchmarks completed"
}

# Generate reports
generate_reports() {
    log_step "Generating Reports"

    # HTML report
    "$VCS_ROOT/scripts/generate-report.sh" \
        --input "$OUTPUT_DIR" \
        --output "$OUTPUT_DIR/nightly-report.html" \
        --title "VCS Nightly Report - $(date +%Y-%m-%d)" \
        --format html || true

    # JSON summary
    "$VCS_ROOT/scripts/generate-report.sh" \
        --input "$OUTPUT_DIR" \
        --output "$OUTPUT_DIR/nightly-summary.json" \
        --format json || true

    log_success "Reports generated"
}

# Send notifications
send_notifications() {
    if [ "$NOTIFY" -eq 0 ]; then
        return 0
    fi

    log_step "Sending Notifications"

    local status="SUCCESS"
    local color="good"

    # Check for failures
    if find "$OUTPUT_DIR" -name "*.json" -exec grep -q '"failed":.*[1-9]' {} \; 2>/dev/null; then
        status="FAILED"
        color="danger"
    fi

    local message="VCS Nightly Suite: $status ($(date +%Y-%m-%d))"

    # Slack notification
    if [ -n "${SLACK_WEBHOOK_URL:-}" ]; then
        log_info "Sending Slack notification..."
        curl -s -X POST -H 'Content-type: application/json' \
            --data "{\"text\": \"$message\", \"attachments\": [{\"color\": \"$color\"}]}" \
            "$SLACK_WEBHOOK_URL" || true
    fi

    # Discord notification
    if [ -n "${DISCORD_WEBHOOK_URL:-}" ]; then
        log_info "Sending Discord notification..."
        curl -s -X POST -H 'Content-type: application/json' \
            --data "{\"content\": \"$message\"}" \
            "$DISCORD_WEBHOOK_URL" || true
    fi

    log_success "Notifications sent"
}

# Generate summary
print_summary() {
    log_step "Nightly Suite Summary"

    echo "Date:            $(date)"
    echo "Duration:        ${DURATION}s"
    echo "Output Dir:      $OUTPUT_DIR"
    echo ""

    echo "Test Results:"
    for f in "$OUTPUT_DIR"/*.json; do
        if [ -f "$f" ]; then
            local name
            name=$(basename "$f" .json)
            local passed
            passed=$(jq -r '.summary.passed // 0' "$f" 2>/dev/null || echo "?")
            local total
            total=$(jq -r '.summary.total // 0' "$f" 2>/dev/null || echo "?")
            echo "  $name: $passed/$total"
        fi
    done
    echo ""

    echo "Artifacts:"
    ls -la "$OUTPUT_DIR/" 2>/dev/null || echo "  (none)"
}

# Main function
main() {
    parse_args "$@"

    # Setup
    local start_time
    start_time=$(date +%s)

    mkdir -p "$OUTPUT_DIR"
    build_tools

    log_step "VCS Nightly Test Suite"
    log_info "Date:          $(date)"
    log_info "Parallel:      $PARALLEL"
    log_info "Fuzz Duration: $FUZZ_DURATION"
    log_info "Output Dir:    $OUTPUT_DIR"

    # Run all stages
    run_full_tests
    run_differential
    run_fuzzing
    run_benchmarks
    generate_reports

    local end_time
    end_time=$(date +%s)
    DURATION=$((end_time - start_time))

    print_summary
    send_notifications

    log_success "Nightly suite completed in ${DURATION}s"
}

# Run main
main "$@"
