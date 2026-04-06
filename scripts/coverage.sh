#!/bin/bash
#
# Comprehensive Test Coverage Script for Verum Language Platform
#
# This script runs cargo-tarpaulin to measure test coverage across all crates
# and validates that coverage meets the 95% threshold.
#
# Usage:
#   ./scripts/coverage.sh                    # Run coverage for all crates
#   ./scripts/coverage.sh --crate verum_cbgr # Run coverage for specific crate
#   ./scripts/coverage.sh --html             # Generate HTML report
#   ./scripts/coverage.sh --ci               # CI mode (strict validation)

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
COVERAGE_THRESHOLD=95
TIMEOUT=600  # 10 minutes timeout per crate
OUTPUT_DIR="coverage"
HTML_REPORT=false
CI_MODE=false
SPECIFIC_CRATE=""

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --html)
            HTML_REPORT=true
            shift
            ;;
        --ci)
            CI_MODE=true
            shift
            ;;
        --crate)
            SPECIFIC_CRATE="$2"
            shift 2
            ;;
        --threshold)
            COVERAGE_THRESHOLD="$2"
            shift 2
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --html                Generate HTML coverage report"
            echo "  --ci                  CI mode (strict validation, non-interactive)"
            echo "  --crate CRATE        Run coverage for specific crate only"
            echo "  --threshold N        Set coverage threshold (default: 95)"
            echo "  --help               Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run with --help for usage information"
            exit 1
            ;;
    esac
done

# Print banner
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Verum Coverage Analysis${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Check if cargo-tarpaulin is installed
if ! command -v cargo-tarpaulin &> /dev/null; then
    echo -e "${YELLOW}cargo-tarpaulin not found. Installing...${NC}"
    cargo install cargo-tarpaulin
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Define core crates to test (in dependency order)
CORE_CRATES=(
    "verum_core"
    "verum_cbgr"
    "verum_std"
    "verum_ast"
    "verum_lexer"
    "verum_parser"
    "verum_types"
    "verum_smt"
    "verum_runtime"
    "verum_compiler"
    "verum_lsp"
)

# If specific crate specified, only test that one
if [ -n "$SPECIFIC_CRATE" ]; then
    CORE_CRATES=("$SPECIFIC_CRATE")
fi

# Function to run coverage for a single crate
run_crate_coverage() {
    local crate=$1
    local crate_dir="crates/$crate"

    if [ ! -d "$crate_dir" ]; then
        echo -e "${RED}✗ Crate directory not found: $crate_dir${NC}"
        return 1
    fi

    echo -e "${BLUE}▶ Running coverage for $crate...${NC}"

    local output_format="--out Lcov --output-dir $OUTPUT_DIR"

    if [ "$HTML_REPORT" = true ]; then
        output_format="--out Html --out Lcov --output-dir $OUTPUT_DIR/$crate"
    fi

    # Run tarpaulin
    cargo tarpaulin \
        --manifest-path "$crate_dir/Cargo.toml" \
        --all-features \
        --timeout "$TIMEOUT" \
        $output_format \
        --exclude-files "*/tests/*" \
        --exclude-files "*/benches/*" \
        --exclude-files "*/examples/*" \
        --verbose \
        2>&1 | tee "$OUTPUT_DIR/${crate}_coverage.log"

    local exit_code=${PIPESTATUS[0]}

    if [ $exit_code -eq 0 ]; then
        echo -e "${GREEN}✓ Coverage analysis completed for $crate${NC}"
        return 0
    else
        echo -e "${RED}✗ Coverage analysis failed for $crate${NC}"
        return 1
    fi
}

# Function to parse coverage percentage from log
get_coverage_percentage() {
    local crate=$1
    local log_file="$OUTPUT_DIR/${crate}_coverage.log"

    if [ ! -f "$log_file" ]; then
        echo "0"
        return
    fi

    # Extract coverage percentage from tarpaulin output
    # Format: "XX.XX% coverage"
    local coverage=$(grep -oP '\d+\.\d+(?=% coverage)' "$log_file" | tail -1)

    if [ -z "$coverage" ]; then
        echo "0"
    else
        echo "$coverage"
    fi
}

# Track overall results
declare -A coverage_results
failed_crates=()
low_coverage_crates=()
total_crates=0
passed_crates=0

# Main coverage loop
echo ""
echo -e "${BLUE}Running coverage for ${#CORE_CRATES[@]} crate(s)...${NC}"
echo ""

for crate in "${CORE_CRATES[@]}"; do
    total_crates=$((total_crates + 1))

    if run_crate_coverage "$crate"; then
        coverage=$(get_coverage_percentage "$crate")
        coverage_results[$crate]=$coverage

        # Check if coverage meets threshold
        if (( $(echo "$coverage >= $COVERAGE_THRESHOLD" | bc -l) )); then
            echo -e "${GREEN}✓ $crate: ${coverage}% (meets threshold)${NC}"
            passed_crates=$((passed_crates + 1))
        else
            echo -e "${YELLOW}⚠ $crate: ${coverage}% (below threshold of ${COVERAGE_THRESHOLD}%)${NC}"
            low_coverage_crates+=("$crate:$coverage")
        fi
    else
        echo -e "${RED}✗ $crate: Coverage analysis failed${NC}"
        failed_crates+=("$crate")
        coverage_results[$crate]="ERROR"
    fi

    echo ""
done

# Generate summary report
echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Coverage Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Calculate average coverage
total_coverage=0
valid_crates=0

for crate in "${!coverage_results[@]}"; do
    coverage="${coverage_results[$crate]}"

    if [ "$coverage" != "ERROR" ]; then
        total_coverage=$(echo "$total_coverage + $coverage" | bc -l)
        valid_crates=$((valid_crates + 1))
    fi
done

if [ $valid_crates -gt 0 ]; then
    avg_coverage=$(echo "scale=2; $total_coverage / $valid_crates" | bc -l)
else
    avg_coverage=0
fi

echo "Total crates tested: $total_crates"
echo "Passed threshold: $passed_crates"
echo "Below threshold: ${#low_coverage_crates[@]}"
echo "Failed: ${#failed_crates[@]}"
echo ""
echo -e "Average coverage: ${BLUE}${avg_coverage}%${NC}"
echo -e "Coverage threshold: ${BLUE}${COVERAGE_THRESHOLD}%${NC}"
echo ""

# Detailed results table
echo "Per-Crate Results:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
printf "%-25s %10s %10s\n" "Crate" "Coverage" "Status"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

for crate in "${CORE_CRATES[@]}"; do
    coverage="${coverage_results[$crate]:-N/A}"

    if [ "$coverage" = "ERROR" ]; then
        status="${RED}FAILED${NC}"
    elif [ "$coverage" = "N/A" ]; then
        status="${YELLOW}SKIPPED${NC}"
    elif (( $(echo "$coverage >= $COVERAGE_THRESHOLD" | bc -l) )); then
        status="${GREEN}PASS${NC}"
    else
        status="${YELLOW}BELOW${NC}"
    fi

    printf "%-25s %9s%% " "$crate" "$coverage"
    echo -e "$status"
done

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Show crates below threshold
if [ ${#low_coverage_crates[@]} -gt 0 ]; then
    echo -e "${YELLOW}Crates below ${COVERAGE_THRESHOLD}% threshold:${NC}"
    for item in "${low_coverage_crates[@]}"; do
        IFS=':' read -r crate coverage <<< "$item"
        gap=$(echo "$COVERAGE_THRESHOLD - $coverage" | bc -l)
        echo -e "  • $crate: ${coverage}% (need +${gap}%)"
    done
    echo ""
fi

# Show failed crates
if [ ${#failed_crates[@]} -gt 0 ]; then
    echo -e "${RED}Failed crates:${NC}"
    for crate in "${failed_crates[@]}"; do
        echo -e "  • $crate"
        echo "    See log: $OUTPUT_DIR/${crate}_coverage.log"
    done
    echo ""
fi

# Generate combined LCOV report if multiple crates
if [ ${#CORE_CRATES[@]} -gt 1 ]; then
    echo "Generating combined coverage report..."

    # Merge LCOV files
    lcov_files=()
    for crate in "${CORE_CRATES[@]}"; do
        lcov_file="$OUTPUT_DIR/$crate/lcov.info"
        if [ -f "$lcov_file" ]; then
            lcov_files+=("$lcov_file")
        fi
    done

    if [ ${#lcov_files[@]} -gt 0 ]; then
        # Combine LCOV files
        echo "Merging ${#lcov_files[@]} LCOV files..."

        # Use lcov to combine if available
        if command -v lcov &> /dev/null; then
            lcov_args=()
            for file in "${lcov_files[@]}"; do
                lcov_args+=("-a" "$file")
            done

            lcov "${lcov_args[@]}" -o "$OUTPUT_DIR/combined.info" 2>/dev/null || true
            echo -e "${GREEN}✓ Combined coverage report: $OUTPUT_DIR/combined.info${NC}"
        fi
    fi
fi

# Generate HTML report if requested
if [ "$HTML_REPORT" = true ]; then
    echo ""
    echo "HTML coverage reports generated in: $OUTPUT_DIR/"
    echo "Open coverage/index.html in your browser"
fi

# CI mode validation
if [ "$CI_MODE" = true ]; then
    echo ""
    echo -e "${BLUE}CI Mode: Validating results...${NC}"

    # Fail if any crate failed
    if [ ${#failed_crates[@]} -gt 0 ]; then
        echo -e "${RED}✗ CI FAILED: ${#failed_crates[@]} crate(s) failed coverage analysis${NC}"
        exit 1
    fi

    # Fail if any crate below threshold
    if [ ${#low_coverage_crates[@]} -gt 0 ]; then
        echo -e "${RED}✗ CI FAILED: ${#low_coverage_crates[@]} crate(s) below ${COVERAGE_THRESHOLD}% threshold${NC}"
        exit 1
    fi

    # Fail if average below threshold
    if (( $(echo "$avg_coverage < $COVERAGE_THRESHOLD" | bc -l) )); then
        echo -e "${RED}✗ CI FAILED: Average coverage ${avg_coverage}% below ${COVERAGE_THRESHOLD}% threshold${NC}"
        exit 1
    fi

    echo -e "${GREEN}✓ CI PASSED: All coverage requirements met${NC}"
    exit 0
fi

# Interactive summary
echo ""
if [ ${#failed_crates[@]} -eq 0 ] && [ ${#low_coverage_crates[@]} -eq 0 ]; then
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}  ✓ All crates meet the ${COVERAGE_THRESHOLD}% threshold!${NC}"
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    exit 0
else
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${YELLOW}  ⚠ Some crates need improvement${NC}"
    echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    exit 1
fi
