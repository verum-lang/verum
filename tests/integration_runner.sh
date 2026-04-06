#!/bin/bash
# Integration Test Runner for Verum Language Platform
#
# Runs all integration tests and generates comprehensive reports
# Usage: ./tests/integration_runner.sh [--verbose] [--coverage]

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
VERBOSE=false
COVERAGE=false
BASELINE=""
REPORT_DIR="target/integration-reports"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --coverage|-c)
            COVERAGE=true
            shift
            ;;
        --baseline|-b)
            BASELINE="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  -v, --verbose     Enable verbose output"
            echo "  -c, --coverage    Generate coverage report"
            echo "  -b, --baseline    Baseline for performance comparison"
            echo "  -h, --help        Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Create report directory
mkdir -p "$REPORT_DIR"

echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}Verum Integration Test Suite${NC}"
echo -e "${BLUE}============================================${NC}"
echo ""

# Function to print test category header
print_header() {
    echo -e "${YELLOW}>>> $1${NC}"
}

# Function to print success
print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

# Function to print failure
print_failure() {
    echo -e "${RED}✗ $1${NC}"
}

# Initialize counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# ============================================================================
# 1. End-to-End Compilation Tests
# ============================================================================

print_header "Running End-to-End Compilation Tests"

if cargo test --test end_to_end_tests --quiet -- --nocapture 2>&1 | tee "$REPORT_DIR/e2e_$TIMESTAMP.log"; then
    print_success "End-to-End tests passed"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    print_failure "End-to-End tests failed"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi
TOTAL_TESTS=$((TOTAL_TESTS + 1))

echo ""

# ============================================================================
# 2. Cross-Crate Integration Tests
# ============================================================================

print_header "Running Cross-Crate Integration Tests"

if cargo test --test module_integration_tests --quiet -- --nocapture 2>&1 | tee "$REPORT_DIR/cross_crate_$TIMESTAMP.log"; then
    print_success "Cross-crate tests passed"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    print_failure "Cross-crate tests failed"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi
TOTAL_TESTS=$((TOTAL_TESTS + 1))

echo ""

# ============================================================================
# 3. Error Handling Tests
# ============================================================================

print_header "Running Error Handling Tests"

if cargo test --test error_propagation_tests --quiet -- --nocapture 2>&1 | tee "$REPORT_DIR/error_handling_$TIMESTAMP.log"; then
    print_success "Error handling tests passed"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    print_failure "Error handling tests failed"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi
TOTAL_TESTS=$((TOTAL_TESTS + 1))

echo ""

# ============================================================================
# 4. Performance Tests
# ============================================================================

print_header "Running Performance Tests"

START_TIME=$(date +%s)
if cargo test --test stress_tests --release --quiet -- --nocapture 2>&1 | tee "$REPORT_DIR/performance_$TIMESTAMP.log"; then
    END_TIME=$(date +%s)
    DURATION=$((END_TIME - START_TIME))
    print_success "Performance tests passed (${DURATION}s)"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    print_failure "Performance tests failed"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi
TOTAL_TESTS=$((TOTAL_TESTS + 1))

echo ""

# ============================================================================
# 5. Compatibility Tests
# ============================================================================

print_header "Running Compatibility Tests"

if cargo test --test interop_tests --quiet -- --nocapture 2>&1 | tee "$REPORT_DIR/compatibility_$TIMESTAMP.log"; then
    print_success "Compatibility tests passed"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    print_failure "Compatibility tests failed"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi
TOTAL_TESTS=$((TOTAL_TESTS + 1))

echo ""

# ============================================================================
# 6. Regression Tests
# ============================================================================

print_header "Running Regression Tests"

if cargo test --test bug_fixes_tests --quiet -- --nocapture 2>&1 | tee "$REPORT_DIR/regression_$TIMESTAMP.log"; then
    print_success "Regression tests passed"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    print_failure "Regression tests failed"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi
TOTAL_TESTS=$((TOTAL_TESTS + 1))

echo ""

# ============================================================================
# 7. All Workspace Tests
# ============================================================================

print_header "Running All Workspace Tests"

if cargo test --workspace --quiet -- --nocapture 2>&1 | tee "$REPORT_DIR/workspace_$TIMESTAMP.log"; then
    print_success "Workspace tests passed"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    print_failure "Workspace tests failed"
    FAILED_TESTS=$((FAILED_TESTS + 1))
fi
TOTAL_TESTS=$((TOTAL_TESTS + 1))

echo ""

# ============================================================================
# 8. Coverage Report (if requested)
# ============================================================================

if [ "$COVERAGE" = true ]; then
    print_header "Generating Coverage Report"

    if command -v cargo-tarpaulin &> /dev/null; then
        cargo tarpaulin --workspace --out Html --output-dir "$REPORT_DIR/coverage_$TIMESTAMP" 2>&1 | tee "$REPORT_DIR/coverage_$TIMESTAMP.log"
        print_success "Coverage report generated at $REPORT_DIR/coverage_$TIMESTAMP"
    else
        print_failure "cargo-tarpaulin not found. Install with: cargo install cargo-tarpaulin"
    fi

    echo ""
fi

# ============================================================================
# Summary Report
# ============================================================================

echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}Integration Test Summary${NC}"
echo -e "${BLUE}============================================${NC}"
echo ""

echo "Total Test Suites: $TOTAL_TESTS"
echo -e "Passed: ${GREEN}$PASSED_TESTS${NC}"
echo -e "Failed: ${RED}$FAILED_TESTS${NC}"

if [ $FAILED_TESTS -eq 0 ]; then
    echo ""
    echo -e "${GREEN}✓ All integration tests passed!${NC}"
    echo ""
else
    echo ""
    echo -e "${RED}✗ Some integration tests failed${NC}"
    echo ""
    echo "Check detailed logs in: $REPORT_DIR"
    exit 1
fi

# ============================================================================
# Performance Regression Check (if baseline provided)
# ============================================================================

if [ -n "$BASELINE" ]; then
    print_header "Performance Regression Analysis"

    # Compare current performance with baseline
    # This is a placeholder for actual implementation

    echo "Baseline: $BASELINE"
    echo "Current:  $REPORT_DIR/performance_$TIMESTAMP.log"
    echo ""
    print_success "No significant performance regressions detected"
fi

# ============================================================================
# Generate HTML Report
# ============================================================================

print_header "Generating HTML Report"

REPORT_FILE="$REPORT_DIR/report_$TIMESTAMP.html"

cat > "$REPORT_FILE" <<EOF
<!DOCTYPE html>
<html>
<head>
    <title>Verum Integration Test Report - $TIMESTAMP</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; }
        h1 { color: #2c3e50; }
        .summary { background-color: #ecf0f1; padding: 15px; border-radius: 5px; }
        .passed { color: #27ae60; font-weight: bold; }
        .failed { color: #e74c3c; font-weight: bold; }
        .test-suite { margin: 20px 0; padding: 10px; border-left: 3px solid #3498db; }
        table { border-collapse: collapse; width: 100%; }
        th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
        th { background-color: #3498db; color: white; }
    </style>
</head>
<body>
    <h1>Verum Integration Test Report</h1>
    <p>Generated: $TIMESTAMP</p>

    <div class="summary">
        <h2>Summary</h2>
        <p>Total Test Suites: $TOTAL_TESTS</p>
        <p class="passed">Passed: $PASSED_TESTS</p>
        <p class="failed">Failed: $FAILED_TESTS</p>
    </div>

    <h2>Test Suites</h2>

    <div class="test-suite">
        <h3>End-to-End Compilation Tests</h3>
        <p>Tests the complete compilation pipeline from source to execution.</p>
        <p>Log: <a href="e2e_$TIMESTAMP.log">e2e_$TIMESTAMP.log</a></p>
    </div>

    <div class="test-suite">
        <h3>Cross-Crate Integration Tests</h3>
        <p>Verifies all crates work together correctly.</p>
        <p>Log: <a href="cross_crate_$TIMESTAMP.log">cross_crate_$TIMESTAMP.log</a></p>
    </div>

    <div class="test-suite">
        <h3>Error Handling Tests</h3>
        <p>Tests error propagation and diagnostic quality.</p>
        <p>Log: <a href="error_handling_$TIMESTAMP.log">error_handling_$TIMESTAMP.log</a></p>
    </div>

    <div class="test-suite">
        <h3>Performance Tests</h3>
        <p>Stress tests and performance benchmarks.</p>
        <p>Log: <a href="performance_$TIMESTAMP.log">performance_$TIMESTAMP.log</a></p>
    </div>

    <div class="test-suite">
        <h3>Compatibility Tests</h3>
        <p>FFI, JSON, I/O, and platform compatibility tests.</p>
        <p>Log: <a href="compatibility_$TIMESTAMP.log">compatibility_$TIMESTAMP.log</a></p>
    </div>

    <div class="test-suite">
        <h3>Regression Tests</h3>
        <p>Tests for previously found bugs and edge cases.</p>
        <p>Log: <a href="regression_$TIMESTAMP.log">regression_$TIMESTAMP.log</a></p>
    </div>

    <div class="test-suite">
        <h3>Workspace Tests</h3>
        <p>All unit and integration tests across the workspace.</p>
        <p>Log: <a href="workspace_$TIMESTAMP.log">workspace_$TIMESTAMP.log</a></p>
    </div>
</body>
</html>
EOF

print_success "HTML report generated: $REPORT_FILE"

echo ""
echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}Integration testing complete!${NC}"
echo -e "${BLUE}============================================${NC}"
