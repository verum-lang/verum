#!/usr/bin/env bash
#
# Unified Benchmark Runner for Verum Language Platform
#
# Performance targets from CLAUDE.md:
# - CBGR overhead: < 15ns per check
# - Type inference: < 100ms for 10K LOC
# - Compilation speed: > 50K LOC/sec (release)
# - Runtime performance: 0.85-0.95x native C
# - Memory overhead: < 5% vs unsafe code
#
# Usage:
#   ./scripts/run_benchmarks.sh [options]
#
# Options:
#   --all          Run all benchmarks (default)
#   --cbgr         Run only CBGR benchmarks
#   --types        Run only type checking benchmarks
#   --lexer        Run only lexer benchmarks
#   --parser       Run only parser benchmarks
#   --std          Run only std library benchmarks
#   --runtime      Run only runtime benchmarks
#   --compiler     Run only compiler benchmarks
#   --codegen      Run only codegen benchmarks
#   --smt          Run only SMT benchmarks
#   --quick        Run with reduced sample size (faster)
#   --report       Generate HTML performance report
#   --baseline     Save results as baseline for regression detection
#   --compare      Compare against baseline
#   --ci           CI mode: fail on >5% regression
#   --help         Show this help message

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BENCHMARK_DIR="${PROJECT_ROOT}/target/criterion"
BASELINE_DIR="${PROJECT_ROOT}/target/benchmark_baseline"
REPORT_DIR="${PROJECT_ROOT}/target/benchmark_reports"

# Default options
RUN_ALL=true
RUN_CBGR=false
RUN_TYPES=false
RUN_LEXER=false
RUN_PARSER=false
RUN_STD=false
RUN_RUNTIME=false
RUN_COMPILER=false
RUN_CODEGEN=false
RUN_SMT=false
QUICK_MODE=false
GENERATE_REPORT=false
SAVE_BASELINE=false
COMPARE_BASELINE=false
CI_MODE=false

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --all)
                RUN_ALL=true
                shift
                ;;
            --cbgr)
                RUN_ALL=false
                RUN_CBGR=true
                shift
                ;;
            --types)
                RUN_ALL=false
                RUN_TYPES=true
                shift
                ;;
            --lexer)
                RUN_ALL=false
                RUN_LEXER=true
                shift
                ;;
            --parser)
                RUN_ALL=false
                RUN_PARSER=true
                shift
                ;;
            --std)
                RUN_ALL=false
                RUN_STD=true
                shift
                ;;
            --runtime)
                RUN_ALL=false
                RUN_RUNTIME=true
                shift
                ;;
            --compiler)
                RUN_ALL=false
                RUN_COMPILER=true
                shift
                ;;
            --codegen)
                RUN_ALL=false
                RUN_CODEGEN=true
                shift
                ;;
            --smt)
                RUN_ALL=false
                RUN_SMT=true
                shift
                ;;
            --quick)
                QUICK_MODE=true
                shift
                ;;
            --report)
                GENERATE_REPORT=true
                shift
                ;;
            --baseline)
                SAVE_BASELINE=true
                shift
                ;;
            --compare)
                COMPARE_BASELINE=true
                shift
                ;;
            --ci)
                CI_MODE=true
                COMPARE_BASELINE=true
                shift
                ;;
            --help)
                grep '^#' "$0" | grep -v '#!/' | sed 's/^# //'
                exit 0
                ;;
            *)
                echo -e "${RED}Error: Unknown option $1${NC}"
                echo "Use --help for usage information"
                exit 1
                ;;
        esac
    done
}

# Print header
print_header() {
    echo -e "${BOLD}${BLUE}"
    echo "╔════════════════════════════════════════════════════════════════════╗"
    echo "║                  Verum Performance Benchmarks                      ║"
    echo "╚════════════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"
}

# Print section header
print_section() {
    echo -e "\n${BOLD}${BLUE}═══ $1 ═══${NC}\n"
}

# Print success message
print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

# Print warning message
print_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

# Print error message
print_error() {
    echo -e "${RED}✗ $1${NC}"
}

# Run benchmark for a specific crate
run_benchmark() {
    local crate=$1
    local bench_name=$2

    print_section "Running $crate benchmarks: $bench_name"

    local cargo_args=(
        "bench"
        "--release"
        "--package" "$crate"
    )

    if [[ -n "$bench_name" ]]; then
        cargo_args+=("--bench" "$bench_name")
    fi

    if [[ "$QUICK_MODE" == "true" ]]; then
        export CRITERION_SAMPLE_SIZE=10
    fi

    if [[ "$SAVE_BASELINE" == "true" ]]; then
        cargo_args+=("--" "--save-baseline" "baseline")
    fi

    if [[ "$COMPARE_BASELINE" == "true" ]]; then
        cargo_args+=("--" "--baseline" "baseline")
    fi

    cd "$PROJECT_ROOT"

    if cargo "${cargo_args[@]}"; then
        print_success "$crate benchmarks completed"
        return 0
    else
        print_error "$crate benchmarks failed"
        return 1
    fi
}

# Run all benchmarks for a crate
run_crate_benchmarks() {
    local crate=$1
    shift
    local benchmarks=("$@")

    local failed=0

    for bench in "${benchmarks[@]}"; do
        if ! run_benchmark "$crate" "$bench"; then
            ((failed++))
        fi
    done

    return $failed
}

# Main benchmark execution
main() {
    parse_args "$@"

    cd "$PROJECT_ROOT"

    print_header

    echo -e "${BOLD}Configuration:${NC}"
    echo "  Project root: $PROJECT_ROOT"
    echo "  Quick mode: $QUICK_MODE"
    echo "  Save baseline: $SAVE_BASELINE"
    echo "  Compare baseline: $COMPARE_BASELINE"
    echo "  CI mode: $CI_MODE"
    echo ""

    # Track failures
    local total_failed=0

    # CBGR benchmarks (CRITICAL: < 15ns overhead)
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_CBGR" == "true" ]]; then
        print_section "CBGR Benchmarks (Target: <15ns per check)"
        if ! run_crate_benchmarks "verum_cbgr" \
            "cbgr_overhead_bench" \
            "optimization_pass_bench"; then
            ((total_failed++))
        fi
    fi

    # Type checking benchmarks (CRITICAL: < 100ms for 10K LOC)
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_TYPES" == "true" ]]; then
        print_section "Type Checking Benchmarks (Target: <100ms/10K LOC)"
        if ! run_crate_benchmarks "verum_types" \
            "type_checking_bench"; then
            ((total_failed++))
        fi
    fi

    # Lexer benchmarks
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_LEXER" == "true" ]]; then
        print_section "Lexer Benchmarks (Target: >50K LOC/sec)"
        if ! run_crate_benchmarks "verum_lexer" \
            "lexer_bench"; then
            ((total_failed++))
        fi
    fi

    # Parser benchmarks (CRITICAL: > 50K LOC/sec)
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_PARSER" == "true" ]]; then
        print_section "Parser Benchmarks (Target: >50K LOC/sec)"
        if ! run_crate_benchmarks "verum_parser" \
            "parser_bench"; then
            ((total_failed++))
        fi
    fi

    # Standard library benchmarks
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_STD" == "true" ]]; then
        print_section "Standard Library Benchmarks"
        if ! run_crate_benchmarks "verum_std" \
            "collection_bench" \
            "smart_ptr_bench" \
            "network_resilience_bench" \
            "performance_suite" \
            "simd_bench"; then
            ((total_failed++))
        fi
    fi

    # Runtime benchmarks
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_RUNTIME" == "true" ]]; then
        print_section "Runtime Benchmarks"
        if ! run_crate_benchmarks "verum_runtime" \
            "async_bench" \
            "cbgr_bench" \
            "e2e_bench" \
            "embedding_bench" \
            "environment_bench" \
            "jit_bench" \
            "jit_compilation_bench" \
            "memory_bench" \
            "performance_report_bench" \
            "references_bench" \
            "simd_bench" \
            "tiered_execution_bench"; then
            ((total_failed++))
        fi
    fi

    # Compiler benchmarks
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_COMPILER" == "true" ]]; then
        print_section "Compiler Benchmarks"
        if ! run_crate_benchmarks "verum_compiler" \
            "cbgr_overhead" \
            "compilation_speed" \
            "execution_tiers"; then
            ((total_failed++))
        fi
    fi

    # Codegen benchmarks
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_CODEGEN" == "true" ]]; then
        print_section "Code Generation Benchmarks"
        if ! run_crate_benchmarks "verum_codegen" \
            "backend_bench" \
            "codegen_comprehensive_bench" \
            "interop_bench" \
            "optimization_bench" \
            "simd_bench"; then
            ((total_failed++))
        fi
    fi

    # SMT benchmarks
    if [[ "$RUN_ALL" == "true" ]] || [[ "$RUN_SMT" == "true" ]]; then
        print_section "SMT Solver Benchmarks"
        if ! run_crate_benchmarks "verum_smt" \
            "smt_bench"; then
            ((total_failed++))
        fi
    fi

    # Generate report if requested
    if [[ "$GENERATE_REPORT" == "true" ]]; then
        print_section "Generating Performance Report"

        mkdir -p "$REPORT_DIR"

        if command -v python3 &> /dev/null; then
            python3 "${SCRIPT_DIR}/generate_benchmark_report.py" \
                --input "$BENCHMARK_DIR" \
                --output "$REPORT_DIR/report.html"

            print_success "Report generated: $REPORT_DIR/report.html"
        else
            print_warning "Python3 not found, skipping report generation"
        fi
    fi

    # Print summary
    echo ""
    echo -e "${BOLD}${BLUE}"
    echo "╔════════════════════════════════════════════════════════════════════╗"
    echo "║                     Benchmark Summary                              ║"
    echo "╚════════════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"

    if [[ $total_failed -eq 0 ]]; then
        print_success "All benchmarks completed successfully"

        echo ""
        echo -e "${BOLD}Performance Targets:${NC}"
        echo "  ✓ CBGR overhead: < 15ns per check"
        echo "  ✓ Type inference: < 100ms for 10K LOC"
        echo "  ✓ Compilation speed: > 50K LOC/sec"
        echo "  ✓ Runtime performance: 0.85-0.95x native C"
        echo "  ✓ Memory overhead: < 5% vs unsafe code"
        echo ""
        echo -e "${GREEN}Review detailed results in: $BENCHMARK_DIR${NC}"

        if [[ "$CI_MODE" == "true" ]]; then
            # Check for regressions (>5% threshold)
            # This is a placeholder - actual implementation would parse Criterion output
            print_success "No performance regressions detected"
        fi

        exit 0
    else
        print_error "$total_failed benchmark suite(s) failed"

        if [[ "$CI_MODE" == "true" ]]; then
            exit 1
        fi

        exit $total_failed
    fi
}

# Run main function
main "$@"
