#!/bin/bash

# Run Async/Await Feature Tests for Verum
# This script runs all async/await related tests and reports results

echo "========================================"
echo "Verum Async/Await Feature Tests"
echo "========================================"
echo ""

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

TESTS_PASSED=0
TESTS_FAILED=0

# Function to run a test
run_test() {
    local test_name=$1
    local test_file=$2

    echo "Running: $test_name"
    echo "----------------------------------------"

    output=$(cargo run --bin verum -- file run "$test_file" 2>&1)
    exit_code=$?

    # Check if test succeeded (exit code 0 and no ERROR in output)
    if [ $exit_code -eq 0 ] && ! echo "$output" | grep -q "\[ERROR\]"; then
        echo -e "${GREEN}✅ PASS${NC}"
        ((TESTS_PASSED++))
        # Show output (filter out warnings)
        echo "$output" | grep -v "^warning:" | tail -10
    else
        echo -e "${RED}❌ FAIL${NC}"
        ((TESTS_FAILED++))
        # Show error
        echo "$output" | grep -v "^warning:" | tail -5
    fi

    echo ""
}

# Run all tests
echo "=== Async Function Tests ==="
run_test "test_async_basic" "examples/tests/features/test_async_basic.vr"

echo "=== Spawn Expression Tests ==="
run_test "test_spawn" "examples/tests/features/test_spawn.vr"

echo "=== Async Closure Tests ==="
run_test "test_async_closure" "examples/tests/features/test_async_closure.vr"

echo "=== Try/Recover Tests ==="
run_test "test_try_simple" "examples/tests/features/test_try_simple.vr"

echo "=== Defer Statement Tests ==="
run_test "test_defer_simple" "examples/tests/features/test_defer_simple.vr"

# Summary
echo "========================================"
echo "Test Summary"
echo "========================================"
echo -e "Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo "Total Tests: $((TESTS_PASSED + TESTS_FAILED))"

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${YELLOW}Some tests failed. See ASYNC_AWAIT_TEST_RESULTS.md for details.${NC}"
    exit 1
fi
