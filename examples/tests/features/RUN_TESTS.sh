#!/bin/bash
# Test script for three-tier reference system
# Run from project root: bash examples/tests/features/RUN_TESTS.sh

set -e

echo "=================================="
echo "Three-Tier Reference System Tests"
echo "=================================="
echo ""

# Test 1: Simple reference creation
echo "Test 1: Simple reference creation"
echo "File: test_three_tier_refs_simple.vr"
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet -- run \
    /Users/taaliman/projects/luxquant/axiom/examples/tests/features/test_three_tier_refs_simple.vr \
    2>&1 | head -15 || true
echo ""

# Test 2: Managed reference dereferencing
echo "Test 2: Managed reference dereferencing"
echo "File: test_ref_deref.vr"
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet -- run \
    /Users/taaliman/projects/luxquant/axiom/examples/tests/features/test_ref_deref.vr \
    2>&1 | head -15 || true
echo ""

# Test 3: Checked reference dereferencing
echo "Test 3: Checked reference dereferencing"
echo "File: test_ref_checked_deref.vr"
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet -- run \
    /Users/taaliman/projects/luxquant/axiom/examples/tests/features/test_ref_checked_deref.vr \
    2>&1 | head -15 || true
echo ""

# Test 4: Unsafe reference dereferencing
echo "Test 4: Unsafe reference dereferencing"
echo "File: test_ref_unsafe_deref.vr"
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet -- run \
    /Users/taaliman/projects/luxquant/axiom/examples/tests/features/test_ref_unsafe_deref.vr \
    2>&1 | head -15 || true
echo ""

# Test 5: Comprehensive three-tier test
echo "Test 5: Comprehensive three-tier test"
echo "File: test_three_tier_refs.vr"
RUSTFLAGS="-A warnings" cargo run -p verum_cli --quiet -- run \
    /Users/taaliman/projects/luxquant/axiom/examples/tests/features/test_three_tier_refs.vr \
    2>&1 | head -15 || true
echo ""

echo "=================================="
echo "Tests Complete"
echo "See THREE_TIER_REFERENCE_TEST_REPORT.md for details"
echo "=================================="
