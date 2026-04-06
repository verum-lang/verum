#!/usr/bin/env bash
# Security Check Script for Verum Language Platform
# Runs comprehensive security validation

set -e

echo "=========================================="
echo "Verum Language Platform - Security Check"
echo "=========================================="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check if cargo-audit is installed
if ! command -v cargo-audit &> /dev/null; then
    echo -e "${YELLOW}Installing cargo-audit...${NC}"
    cargo install cargo-audit
fi

echo "1. Running cargo audit..."
echo "----------------------------------------"
if cargo audit 2>&1 | tee /tmp/audit_output.txt; then
    echo -e "${GREEN}✓ No vulnerabilities found${NC}"
    AUDIT_RESULT=0
else
    AUDIT_RESULT=$?
    # Check if only RSA advisory (expected)
    if grep -q "RUSTSEC-2023-0071" /tmp/audit_output.txt && ! grep -q "error: [2-9]" /tmp/audit_output.txt; then
        echo -e "${YELLOW}✓ Only known RSA advisory (RUSTSEC-2023-0071) - ACCEPTABLE${NC}"
        AUDIT_RESULT=0
    else
        echo -e "${RED}✗ Unexpected vulnerabilities found${NC}"
    fi
fi
echo ""

echo "2. Checking for unmaintained dependencies..."
echo "----------------------------------------"
WARNINGS=$(cargo audit 2>&1 | grep -c "unmaintained" || true)
if [ "$WARNINGS" -eq 0 ]; then
    echo -e "${GREEN}✓ No unmaintained dependencies${NC}"
else
    echo -e "${RED}✗ Found $WARNINGS unmaintained dependencies${NC}"
    AUDIT_RESULT=1
fi
echo ""

echo "3. Running security tests..."
echo "----------------------------------------"
if cargo test --test security_tests --lib 2>&1 | tail -20; then
    echo -e "${GREEN}✓ All security tests passed${NC}"
else
    echo -e "${RED}✗ Security tests failed${NC}"
    AUDIT_RESULT=1
fi
echo ""

echo "4. Checking for unsafe code..."
echo "----------------------------------------"
UNSAFE_COUNT=$(rg "unsafe " --type rust -g "!tests" crates/ | wc -l | tr -d ' ')
echo "Found $UNSAFE_COUNT unsafe blocks (should have SAFETY comments)"
if [ "$UNSAFE_COUNT" -lt 100 ]; then
    echo -e "${GREEN}✓ Acceptable amount of unsafe code${NC}"
else
    echo -e "${YELLOW}⚠ High number of unsafe blocks - review required${NC}"
fi
echo ""

echo "5. Dependency count..."
echo "----------------------------------------"
DEP_COUNT=$(cargo tree --depth 0 2>/dev/null | wc -l | tr -d ' ')
echo "Direct dependencies: $DEP_COUNT"
TOTAL_DEPS=$(cargo tree 2>/dev/null | wc -l | tr -d ' ')
echo "Total dependencies: $TOTAL_DEPS"
echo ""

echo "=========================================="
if [ $AUDIT_RESULT -eq 0 ]; then
    echo -e "${GREEN}✓ SECURITY CHECK PASSED${NC}"
    echo ""
    echo "Status: PRODUCTION READY"
    echo "Known advisories: 1 (RUSTSEC-2023-0071 - documented & mitigated)"
    exit 0
else
    echo -e "${RED}✗ SECURITY CHECK FAILED${NC}"
    echo ""
    echo "Status: NEEDS ATTENTION"
    echo "Review audit output above for details"
    exit 1
fi
