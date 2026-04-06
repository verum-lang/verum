#!/bin/bash
# Verum CLI Integration Test Runner
# Run all test programs through different execution tiers

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERUM_CLI="${SCRIPT_DIR}/../../target/release/verum"
TEST_DIR="${SCRIPT_DIR}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
PASSED=0
FAILED=0
SKIPPED=0

# Test result storage
declare -a FAILED_TESTS=()

# Function to run a single test
run_test() {
    local test_file="$1"
    local tier="$2"
    local test_name=$(basename "$test_file" .vr)

    echo -n "  [$tier] $test_name... "

    if [ ! -f "$VERUM_CLI" ]; then
        echo -e "${YELLOW}SKIPPED${NC} (CLI not built)"
        ((SKIPPED++))
        return 0
    fi

    # Run the test
    if "$VERUM_CLI" run --tier "$tier" "$test_file" > /tmp/test_output.txt 2>&1; then
        echo -e "${GREEN}PASSED${NC}"
        ((PASSED++))
    else
        echo -e "${RED}FAILED${NC}"
        ((FAILED++))
        FAILED_TESTS+=("[$tier] $test_file")
        cat /tmp/test_output.txt | head -20
    fi
}

# Function to run parser-only test (just check if it parses)
run_parse_test() {
    local test_file="$1"
    local test_name=$(basename "$test_file" .vr)

    echo -n "  [parse] $test_name... "

    if [ ! -f "$VERUM_CLI" ]; then
        echo -e "${YELLOW}SKIPPED${NC} (CLI not built)"
        ((SKIPPED++))
        return 0
    fi

    # Run check (parse + type check)
    if "$VERUM_CLI" check "$test_file" > /tmp/test_output.txt 2>&1; then
        echo -e "${GREEN}PASSED${NC}"
        ((PASSED++))
    else
        echo -e "${RED}FAILED${NC}"
        ((FAILED++))
        FAILED_TESTS+=("[parse] $test_file")
        cat /tmp/test_output.txt | head -20
    fi
}

echo "=========================================="
echo "Verum CLI Integration Test Suite"
echo "=========================================="
echo ""

# Check if CLI exists
if [ ! -f "$VERUM_CLI" ]; then
    echo -e "${YELLOW}WARNING: verum CLI not built. Run 'cargo build --release -p verum_cli' first.${NC}"
    echo "Running parse-only tests using cargo..."
    echo ""
fi

# Run single-file tests
echo "=== Single-File Tests ==="

for category in basic types collections refinements cbgr contexts operators patterns async modules protocols errors; do
    category_dir="${TEST_DIR}/single/${category}"
    if [ -d "$category_dir" ]; then
        echo ""
        echo "--- $category ---"
        for test_file in "$category_dir"/*.vr; do
            if [ -f "$test_file" ]; then
                run_parse_test "$test_file"
            fi
        done
    fi
done

# Run multi-module tests
echo ""
echo "=== Multi-Module Tests ==="
for project_dir in "${TEST_DIR}"/multimodule/*/; do
    if [ -d "$project_dir" ] && [ -f "${project_dir}/Verum.toml" ]; then
        project_name=$(basename "$project_dir")
        echo ""
        echo "--- $project_name ---"
        echo -n "  [build] $project_name... "

        if [ ! -f "$VERUM_CLI" ]; then
            echo -e "${YELLOW}SKIPPED${NC} (CLI not built)"
            ((SKIPPED++))
        else
            if (cd "$project_dir" && "$VERUM_CLI" build --tier 0) > /tmp/test_output.txt 2>&1; then
                echo -e "${GREEN}PASSED${NC}"
                ((PASSED++))
            else
                echo -e "${RED}FAILED${NC}"
                ((FAILED++))
                FAILED_TESTS+=("[build] $project_dir")
            fi
        fi
    fi
done

# Summary
echo ""
echo "=========================================="
echo "Test Summary"
echo "=========================================="
echo -e "Passed:  ${GREEN}$PASSED${NC}"
echo -e "Failed:  ${RED}$FAILED${NC}"
echo -e "Skipped: ${YELLOW}$SKIPPED${NC}"
echo ""

if [ ${#FAILED_TESTS[@]} -gt 0 ]; then
    echo "Failed tests:"
    for test in "${FAILED_TESTS[@]}"; do
        echo "  - $test"
    done
    exit 1
fi

echo "All tests passed!"
exit 0
