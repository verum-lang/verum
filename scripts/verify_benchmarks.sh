#!/usr/bin/env bash
#
# Quick Benchmark Verification Script
#
# Verifies that all benchmark suites compile and can run in test mode
# Use this to quickly check benchmark infrastructure without running full benchmarks

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo -e "${BOLD}${BLUE}"
echo "╔════════════════════════════════════════════════════════════╗"
echo "║         Verum Benchmark Infrastructure Verification        ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo -e "${NC}\n"

cd "$PROJECT_ROOT"

# Track results
total=0
passed=0
failed=0

# Test function
test_benchmark() {
    local crate=$1
    local bench=$2

    ((total++))

    echo -e "${BLUE}Testing: ${crate}::${bench}${NC}"

    if cargo bench --package "$crate" --bench "$bench" -- --test > /dev/null 2>&1; then
        echo -e "${GREEN}✓ PASS${NC}\n"
        ((passed++))
        return 0
    else
        echo -e "${RED}✗ FAIL${NC}\n"
        ((failed++))
        return 1
    fi
}

echo -e "${BOLD}Verifying critical benchmarks...${NC}\n"

# Test critical benchmarks
test_benchmark "verum_cbgr" "cbgr_overhead_bench"
test_benchmark "verum_types" "type_checking_bench"
test_benchmark "verum_lexer" "lexer_bench"
test_benchmark "verum_parser" "parser_bench"

echo -e "\n${BOLD}${BLUE}"
echo "╔════════════════════════════════════════════════════════════╗"
echo "║                    Verification Summary                    ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo -e "${NC}"

echo "Total:  $total"
echo -e "${GREEN}Passed: $passed${NC}"

if [[ $failed -gt 0 ]]; then
    echo -e "${RED}Failed: $failed${NC}"
    exit 1
else
    echo -e "\n${GREEN}${BOLD}✓ All critical benchmarks verified!${NC}\n"
    echo "Run full benchmarks with:"
    echo "  ./scripts/run_benchmarks.sh --all"
    exit 0
fi
