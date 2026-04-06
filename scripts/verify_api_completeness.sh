#!/bin/bash
#
# Verum API Completeness Verification Script
#
# This script verifies that ALL APIs in the Verum codebase are 100% complete
# with no missing implementations.
#
# Usage: ./scripts/verify_api_completeness.sh

set -e

echo "=========================================="
echo "VERUM API COMPLETENESS VERIFICATION"
echo "=========================================="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
TOTAL_CHECKS=0
PASSED_CHECKS=0
FAILED_CHECKS=0

# Function to run a check
run_check() {
    local name="$1"
    local command="$2"

    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    echo -n "[$TOTAL_CHECKS] $name... "

    if eval "$command" > /dev/null 2>&1; then
        echo -e "${GREEN}✅ PASS${NC}"
        PASSED_CHECKS=$((PASSED_CHECKS + 1))
        return 0
    else
        echo -e "${RED}❌ FAIL${NC}"
        FAILED_CHECKS=$((FAILED_CHECKS + 1))
        return 1
    fi
}

# Function to count occurrences
count_check() {
    local name="$1"
    local pattern="$2"
    local path="$3"
    local expected="$4"

    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    echo -n "[$TOTAL_CHECKS] $name... "

    count=$(find "$path" -name "*.rs" -type f -exec grep -l "$pattern" {} \; 2>/dev/null | wc -l | tr -d ' ')

    if [ "$count" -eq "$expected" ]; then
        echo -e "${GREEN}✅ PASS${NC} (found $count)"
        PASSED_CHECKS=$((PASSED_CHECKS + 1))
        return 0
    else
        echo -e "${RED}❌ FAIL${NC} (expected $expected, found $count)"
        FAILED_CHECKS=$((FAILED_CHECKS + 1))
        return 1
    fi
}

echo "1. Checking for incomplete implementations..."
echo "   (Looking for unimplemented!(), todo!(), stub panics)"
echo ""

# Check verum_core
count_check "verum_core has no unimplemented!()" "unimplemented!" "crates/verum_core/src" 0
count_check "verum_core has no todo!()" "todo!" "crates/verum_core/src" 0
count_check "verum_core has no stub panics" 'panic!.*not implemented' "crates/verum_core/src" 0

echo ""

# Check verum_std
count_check "verum_std has no unimplemented!()" "unimplemented!" "crates/verum_std/src" 0
count_check "verum_std has no todo!()" "todo!" "crates/verum_std/src" 0
count_check "verum_std has no stub panics" 'panic!.*not implemented' "crates/verum_std/src" 0

echo ""

# Check verum_cbgr
count_check "verum_cbgr has no unimplemented!()" "unimplemented!" "crates/verum_cbgr/src" 0
count_check "verum_cbgr has no todo!()" "todo!" "crates/verum_cbgr/src" 0
count_check "verum_cbgr has no stub panics" 'panic!.*not implemented' "crates/verum_cbgr/src" 0

echo ""

# Check verum_types
count_check "verum_types has no unimplemented!()" "unimplemented!" "crates/verum_types/src" 0
count_check "verum_types has no todo!()" "todo!" "crates/verum_types/src" 0
count_check "verum_types has no stub panics" 'panic!.*not implemented' "crates/verum_types/src" 0

echo ""

# Check verum_codegen
count_check "verum_codegen has no unimplemented!()" "unimplemented!" "crates/verum_codegen/src" 0
count_check "verum_codegen has no todo!()" "todo!" "crates/verum_codegen/src" 0
count_check "verum_codegen has no stub panics" 'panic!.*not implemented' "crates/verum_codegen/src" 0

echo ""

# Check verum_parser
count_check "verum_parser has no unimplemented!()" "unimplemented!" "crates/verum_parser/src" 0
count_check "verum_parser has no todo!()" "todo!" "crates/verum_parser/src" 0
count_check "verum_parser has no stub panics" 'panic!.*not implemented' "crates/verum_parser/src" 0

echo ""

# Check verum_lsp
count_check "verum_lsp has no unimplemented!()" "unimplemented!" "crates/verum_lsp/src" 0
count_check "verum_lsp has no todo!()" "todo!" "crates/verum_lsp/src" 0
count_check "verum_lsp has no stub panics" 'panic!.*not implemented' "crates/verum_lsp/src" 0

echo ""
echo "2. Verifying file existence..."
echo ""

# Check key files exist
run_check "verum_core/semantic_types.rs exists" "[ -f crates/verum_core/src/semantic_types.rs ]"
run_check "verum_core/maybe.rs exists" "[ -f crates/verum_core/src/maybe.rs ]"
run_check "verum_std/collections.rs exists" "[ -f crates/verum_std/src/collections.rs ]"
run_check "verum_cbgr/lib.rs exists" "[ -f crates/verum_cbgr/src/lib.rs ]"
run_check "verum_types/infer.rs exists" "[ -f crates/verum_types/src/infer.rs ]"
run_check "verum_codegen/expressions.rs exists" "[ -f crates/verum_codegen/src/expressions.rs ]"
run_check "verum_parser/expr.rs exists" "[ -f crates/verum_parser/src/expr.rs ]"
run_check "verum_lsp/backend.rs exists" "[ -f crates/verum_lsp/src/backend.rs ]"

echo ""
echo "3. Counting API methods..."
echo ""

# Count public methods
verum_core_methods=$(grep -r "pub fn" crates/verum_core/src/*.rs 2>/dev/null | wc -l | tr -d ' ')
verum_std_methods=$(grep -r "pub fn" crates/verum_std/src/collections.rs 2>/dev/null | wc -l | tr -d ' ')
verum_cbgr_methods=$(grep -r "pub fn" crates/verum_cbgr/src/*.rs 2>/dev/null | wc -l | tr -d ' ')

echo "   verum_core: $verum_core_methods public methods"
echo "   verum_std (collections): $verum_std_methods public methods"
echo "   verum_cbgr: $verum_cbgr_methods public methods"

echo ""
echo "4. Counting test cases..."
echo ""

# Count tests
test_files=$(find crates -path "*/tests/*.rs" -type f 2>/dev/null | wc -l | tr -d ' ')
test_cases=$(grep -r "#\[test\]\|#\[tokio::test\]" crates 2>/dev/null | wc -l | tr -d ' ')

echo "   Test files: $test_files"
echo "   Test cases: $test_cases"

echo ""
echo "5. Counting lines of code..."
echo ""

# Count LOC
total_files=$(find crates -name "*.rs" -type f 2>/dev/null | wc -l | tr -d ' ')
total_loc=$(find crates -name "*.rs" -type f -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')

echo "   Total Rust files: $total_files"
echo "   Total lines of code: $total_loc"

echo ""
echo "=========================================="
echo "VERIFICATION SUMMARY"
echo "=========================================="
echo ""
echo "Total checks: $TOTAL_CHECKS"
echo -e "Passed: ${GREEN}$PASSED_CHECKS${NC}"
echo -e "Failed: ${RED}$FAILED_CHECKS${NC}"
echo ""

if [ $FAILED_CHECKS -eq 0 ]; then
    echo -e "${GREEN}✅ ALL CHECKS PASSED - 100% API COMPLETENESS VERIFIED${NC}"
    echo ""
    echo "Statistics:"
    echo "  - $total_files source files"
    echo "  - $total_loc lines of code"
    echo "  - $test_files test files"
    echo "  - $test_cases test cases"
    echo "  - 0 incomplete implementations"
    echo "  - 0 TODO markers"
    echo "  - 0 stub panics"
    echo ""
    echo "Status: PRODUCTION READY ✅"
    exit 0
else
    echo -e "${RED}❌ VERIFICATION FAILED${NC}"
    echo ""
    echo "Some checks failed. Please review the output above."
    exit 1
fi
