#!/bin/bash
# Test script for Verum Type System features
# Run from project root: bash examples/tests/features/RUN_TYPE_SYSTEM_TESTS.sh

set -e

echo "=========================================="
echo "Verum Type System Feature Tests"
echo "=========================================="
echo ""
echo "Testing grammar specification compliance"
echo "Test Date: $(date)"
echo ""

BASE_PATH="/Users/taaliman/projects/luxquant/axiom/examples/tests/features"

# Test 1: Inline Refinement Types
echo "=========================================="
echo "Test 1: Inline Refinement Types"
echo "File: test_refinement_inline.vr"
echo "Syntax: type Name is BaseType{predicate};"
echo "=========================================="
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet --release -- run \
    "${BASE_PATH}/test_refinement_inline.vr" \
    2>&1 | head -30 || echo "❌ Test failed"
echo ""

# Test 2: Where Clause Refinements
echo "=========================================="
echo "Test 2: Where Clause Refinements"
echo "File: test_refinement_where.vr"
echo "Syntax: type Name is BaseType where |x| pred;"
echo "=========================================="
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet --release -- run \
    "${BASE_PATH}/test_refinement_where.vr" \
    2>&1 | head -30 || echo "❌ Test failed"
echo ""

# Test 3: Sigma Type Refinements
echo "=========================================="
echo "Test 3: Sigma Type Refinements"
echo "File: test_sigma_types.vr"
echo "Syntax: type Name is x: BaseType where pred;"
echo "=========================================="
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet --release -- run \
    "${BASE_PATH}/test_sigma_types.vr" \
    2>&1 | head -30 || echo "❌ Test failed"
echo ""

# Test 4: Extended Variant Types
echo "=========================================="
echo "Test 4: Extended Variant Types"
echo "File: test_variant_types_extended.vr"
echo "Syntax: type Name is | Variant { fields } | ...;"
echo "=========================================="
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet --release -- run \
    "${BASE_PATH}/test_variant_types_extended.vr" \
    2>&1 | head -30 || echo "❌ Test failed"
echo ""

# Test 5: Protocol Definitions
echo "=========================================="
echo "Test 5: Protocol Definitions"
echo "File: test_protocol_basic.vr"
echo "Syntax: type Protocol is protocol { methods };"
echo "=========================================="
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet --release -- run \
    "${BASE_PATH}/test_protocol_basic.vr" \
    2>&1 | head -30 || echo "❌ Test failed"
echo ""

# Test 6: Newtype Definitions
echo "=========================================="
echo "Test 6: Newtype Definitions"
echo "File: test_newtype.vr"
echo "Syntax: type Name is (BaseType);"
echo "=========================================="
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet --release -- run \
    "${BASE_PATH}/test_newtype.vr" \
    2>&1 | head -30 || echo "❌ Test failed"
echo ""

echo "=========================================="
echo "Tests Complete"
echo "See TYPE_SYSTEM_TEST_RESULTS.md for details"
echo "=========================================="
