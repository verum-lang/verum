#!/bin/bash
# Documentation Coverage Checker for Verum Platform
# Finds all public items without documentation and generates a report

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Core crates to check (in priority order)
CORE_CRATES=(
    "verum_cbgr"
    "verum_types"
    "verum_std"
    "verum_runtime"
    "verum_compiler"
    "verum_codegen"
    "verum_context"
    "verum_verification"
    "verum_smt"
    "verum_modules"
    "verum_ast"
    "verum_parser"
    "verum_lexer"
    "verum_core"
)

echo -e "${BLUE}==================================================================${NC}"
echo -e "${BLUE}  Verum Platform - Documentation Coverage Report${NC}"
echo -e "${BLUE}==================================================================${NC}"
echo ""

TOTAL_WARNINGS=0
OUTPUT_FILE="target/doc_coverage_report.txt"
mkdir -p target

# Clear previous report
echo "Documentation Coverage Report - $(date)" > "$OUTPUT_FILE"
echo "================================================================" >> "$OUTPUT_FILE"
echo "" >> "$OUTPUT_FILE"

# Function to check a single crate
check_crate() {
    local crate_name=$1
    local crate_path="crates/$crate_name"

    if [ ! -d "$crate_path" ]; then
        echo -e "${YELLOW}⚠  Crate not found: $crate_name${NC}"
        return
    fi

    echo -e "${BLUE}Checking $crate_name...${NC}"

    # Run cargo rustdoc with missing-docs warning
    local warnings_output
    warnings_output=$(cargo rustdoc -p "$crate_name" --quiet -- \
        -W missing_docs \
        -W rustdoc::broken_intra_doc_links \
        -W rustdoc::invalid_codeblock_attributes \
        2>&1 | grep -E "warning:|error:" || true)

    if [ -z "$warnings_output" ]; then
        echo -e "${GREEN}✓  $crate_name - 100% documented${NC}"
        echo "$crate_name: 100% documented (0 warnings)" >> "$OUTPUT_FILE"
        echo "" >> "$OUTPUT_FILE"
        return 0
    fi

    # Count warnings
    local warning_count
    warning_count=$(echo "$warnings_output" | grep -c "warning:" || echo 0)

    TOTAL_WARNINGS=$((TOTAL_WARNINGS + warning_count))

    echo -e "${RED}✗  $crate_name - $warning_count undocumented items${NC}"
    echo "" >> "$OUTPUT_FILE"
    echo "=== $crate_name ===" >> "$OUTPUT_FILE"
    echo "Undocumented items: $warning_count" >> "$OUTPUT_FILE"
    echo "" >> "$OUTPUT_FILE"
    echo "$warnings_output" >> "$OUTPUT_FILE"
    echo "" >> "$OUTPUT_FILE"

    # Extract specific undocumented items
    echo "$warnings_output" | grep "warning: missing documentation" | head -20

    echo ""
}

# Check each core crate
for crate in "${CORE_CRATES[@]}"; do
    check_crate "$crate"
done

# Summary
echo -e "${BLUE}==================================================================${NC}"
echo -e "${BLUE}  Summary${NC}"
echo -e "${BLUE}==================================================================${NC}"
echo ""

if [ $TOTAL_WARNINGS -eq 0 ]; then
    echo -e "${GREEN}✓  All crates have 100% documentation coverage!${NC}"
    echo "" >> "$OUTPUT_FILE"
    echo "SUMMARY: All crates have 100% documentation coverage!" >> "$OUTPUT_FILE"
    exit 0
else
    echo -e "${RED}✗  Total undocumented items: $TOTAL_WARNINGS${NC}"
    echo -e "${YELLOW}   Full report saved to: $OUTPUT_FILE${NC}"
    echo "" >> "$OUTPUT_FILE"
    echo "SUMMARY: Total undocumented items: $TOTAL_WARNINGS" >> "$OUTPUT_FILE"
    exit 1
fi
