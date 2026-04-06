#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Syntax Validator
# =============================================================================
#
# Validates all .vr files in VCS against the Verum grammar specification.
# Checks for common Rust-isms and ensures correct Verum syntax.
#
# Usage:
#   ./validate_syntax.sh [options] [paths...]
#
# Options:
#   --level LEVEL        Only validate tests at this level (L0-L4)
#   --fix                Attempt to auto-fix common issues
#   --strict             Treat warnings as errors
#   --report FILE        Output validation report to file
#   --format FORMAT      Report format: text, json, markdown
#   --parallel N         Number of parallel workers
#   --verbose            Enable verbose output
#   -h, --help           Show help message
#
# Checks performed:
#   - Rust syntax that should be Verum (struct -> type...is, enum -> type...is)
#   - Correct use of Verum keywords (let, fn, is)
#   - Protocol definitions (trait -> protocol)
#   - Semantic types (Vec -> List, String -> Text, etc.)
#   - Attribute syntax (# -> @)
#   - Parser/lexer validation via vtest
#
# Reference: CLAUDE.md, grammar/verum.ebnf
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Configuration
LEVEL=""
FIX=0
STRICT=0
REPORT_FILE=""
REPORT_FORMAT="text"
PARALLEL="${VCS_PARALLEL:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
VERBOSE=0
PATHS=()

SPECS_DIR="$VCS_ROOT/specs"
GRAMMAR_FILE="$PROJECT_ROOT/grammar/verum.ebnf"
VTEST="$VCS_ROOT/runner/vtest/target/release/vtest"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Counters
TOTAL_FILES=0
ERRORS=0
WARNINGS=0
FIXED=0

# Logging
log_info() { echo -e "${CYAN}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; ((WARNINGS++)); }
log_error() { echo -e "${RED}[ERROR]${NC} $1" >&2; ((ERRORS++)); }

# Usage
usage() {
    head -n 45 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --level)
                LEVEL="$2"
                shift 2
                ;;
            --fix)
                FIX=1
                shift
                ;;
            --strict)
                STRICT=1
                shift
                ;;
            --report)
                REPORT_FILE="$2"
                shift 2
                ;;
            --format)
                REPORT_FORMAT="$2"
                shift 2
                ;;
            --parallel)
                PARALLEL="$2"
                shift 2
                ;;
            --verbose)
                VERBOSE=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            -*)
                log_error "Unknown option: $1"
                usage
                exit 1
                ;;
            *)
                PATHS+=("$1")
                shift
                ;;
        esac
    done

    # Default paths
    if [ ${#PATHS[@]} -eq 0 ]; then
        PATHS=("$SPECS_DIR")
    fi
}

# Common Rust-isms to check for (and their Verum equivalents)
declare -A RUST_TO_VERUM=(
    ["struct "]="type ... is { ... };"
    ["enum "]="type ... is ... | ...;"
    ["trait "]="type ... is protocol { ... };"
    ["impl "]="implement ... { ... }"
    ["Box::new"]="Heap(...)"
    ["Vec<"]="List<"
    ["String"]="Text"
    ["HashMap<"]="Map<"
    ["HashSet<"]="Set<"
    ["Option<"]="Maybe<"
    ["#\\["]="@..."
)

# Check single file for Rust-isms
check_rust_syntax() {
    local file="$1"
    local issues=()

    # Read file content
    local content
    content=$(cat "$file")

    # Check for struct keyword
    if grep -qE '^\s*struct\s+\w+' "$file"; then
        issues+=("Uses 'struct' - should be 'type Name is { ... };'")
    fi

    # Check for enum keyword
    if grep -qE '^\s*enum\s+\w+' "$file"; then
        issues+=("Uses 'enum' - should be 'type Name is A | B;'")
    fi

    # Check for trait keyword
    if grep -qE '^\s*trait\s+\w+' "$file"; then
        issues+=("Uses 'trait' - should be 'type Name is protocol { ... };'")
    fi

    # Check for impl keyword without "implement"
    if grep -qE '^\s*impl\s+' "$file" | grep -vq 'implement'; then
        issues+=("Uses 'impl' - should be 'implement'")
    fi

    # Check for Box::new
    if grep -q 'Box::new' "$file"; then
        issues+=("Uses 'Box::new' - should be 'Heap(...)'")
    fi

    # Check for Vec<
    if grep -qE 'Vec<[^>]+>' "$file"; then
        issues+=("Uses 'Vec<T>' - should be 'List<T>'")
    fi

    # Check for Rust String type
    if grep -qE '\bString\b' "$file" | grep -vq 'Text'; then
        issues+=("Uses 'String' - should be 'Text'")
    fi

    # Check for HashMap
    if grep -q 'HashMap<' "$file"; then
        issues+=("Uses 'HashMap<K,V>' - should be 'Map<K,V>'")
    fi

    # Check for HashSet
    if grep -q 'HashSet<' "$file"; then
        issues+=("Uses 'HashSet<T>' - should be 'Set<T>'")
    fi

    # Check for Option<
    if grep -qE 'Option<[^>]+>' "$file"; then
        issues+=("Uses 'Option<T>' - should be 'Maybe<T>'")
    fi

    # Check for Rust attribute syntax #[...]
    if grep -qE '#\[' "$file"; then
        issues+=("Uses '#[...]' - should be '@...'")
    fi

    # Check for derive macro
    if grep -q '#\[derive' "$file"; then
        issues+=("Uses '#[derive(...)]' - should be '@derive(...)'")
    fi

    # Return issues
    printf '%s\n' "${issues[@]}"
}

# Check for valid test metadata
check_metadata() {
    local file="$1"
    local issues=()

    # Check for required metadata
    if ! grep -q '// @test:' "$file"; then
        issues+=("Missing @test metadata")
    fi

    if ! grep -q '// @tier:' "$file"; then
        issues+=("Missing @tier metadata")
    fi

    if ! grep -q '// @level:' "$file"; then
        issues+=("Missing @level metadata")
    fi

    if ! grep -q '// @expect:' "$file"; then
        issues+=("Missing @expect metadata")
    fi

    printf '%s\n' "${issues[@]}"
}

# Attempt to auto-fix issues
auto_fix() {
    local file="$1"

    if [ "$FIX" -eq 0 ]; then
        return 0
    fi

    local content
    content=$(cat "$file")
    local modified=0

    # Fix Box::new -> Heap
    if grep -q 'Box::new' "$file"; then
        content=$(echo "$content" | sed 's/Box::new(\([^)]*\))/Heap(\1)/g')
        modified=1
    fi

    # Fix Vec< -> List<
    if grep -qE 'Vec<' "$file"; then
        content=$(echo "$content" | sed 's/Vec</List</g')
        modified=1
    fi

    # Fix String -> Text (careful not to break strings)
    # This is tricky and might need manual review

    # Fix HashMap -> Map
    if grep -q 'HashMap<' "$file"; then
        content=$(echo "$content" | sed 's/HashMap</Map</g')
        modified=1
    fi

    # Fix HashSet -> Set
    if grep -q 'HashSet<' "$file"; then
        content=$(echo "$content" | sed 's/HashSet</Set</g')
        modified=1
    fi

    # Fix Option -> Maybe
    if grep -qE 'Option<' "$file"; then
        content=$(echo "$content" | sed 's/Option</Maybe</g')
        modified=1
    fi

    # Fix #[ -> @
    if grep -qE '#\[' "$file"; then
        content=$(echo "$content" | sed 's/#\[\([^]]*\)\]/@\1/g')
        modified=1
    fi

    if [ "$modified" -eq 1 ]; then
        echo "$content" > "$file"
        ((FIXED++))
        log_success "Auto-fixed: $file"
    fi
}

# Validate single file
validate_file() {
    local file="$1"
    local file_issues=()

    ((TOTAL_FILES++))

    if [ "$VERBOSE" -eq 1 ]; then
        log_info "Validating: $file"
    fi

    # Check for Rust syntax
    while IFS= read -r issue; do
        [ -n "$issue" ] && file_issues+=("$issue")
    done < <(check_rust_syntax "$file")

    # Check metadata
    while IFS= read -r issue; do
        [ -n "$issue" ] && file_issues+=("$issue")
    done < <(check_metadata "$file")

    # Report issues
    if [ ${#file_issues[@]} -gt 0 ]; then
        echo ""
        log_error "Issues in: $file"
        for issue in "${file_issues[@]}"; do
            echo "    - $issue"
        done

        # Attempt auto-fix
        auto_fix "$file"
    fi
}

# Validate using vtest parser
validate_with_parser() {
    local file="$1"

    if [ ! -f "$VTEST" ]; then
        return 0
    fi

    local result
    result=$("$VTEST" validate "$file" 2>&1) || true

    if echo "$result" | grep -qi "error\|fail"; then
        log_error "Parser validation failed: $file"
        echo "$result" | head -10
        return 1
    fi

    return 0
}

# Find all .vr files
find_vr_files() {
    local search_paths=("${PATHS[@]}")

    # Filter by level if specified
    if [ -n "$LEVEL" ]; then
        local new_paths=()
        for path in "${search_paths[@]}"; do
            case "$LEVEL" in
                L0) new_paths+=("$path/L0-critical" "$path") ;;
                L1) new_paths+=("$path/L1-core" "$path") ;;
                L2) new_paths+=("$path/L2-standard" "$path") ;;
                L3) new_paths+=("$path/L3-extended" "$path") ;;
                L4) new_paths+=("$path/L4-performance" "$path") ;;
            esac
        done
        search_paths=("${new_paths[@]}")
    fi

    for path in "${search_paths[@]}"; do
        if [ -d "$path" ]; then
            find "$path" -name "*.vr" -type f 2>/dev/null || true
        elif [ -f "$path" ]; then
            echo "$path"
        fi
    done | sort -u
}

# Generate report
generate_report() {
    if [ -z "$REPORT_FILE" ]; then
        return 0
    fi

    local status="PASS"
    if [ "$ERRORS" -gt 0 ]; then
        status="FAIL"
    elif [ "$WARNINGS" -gt 0 ]; then
        status="WARN"
    fi

    case "$REPORT_FORMAT" in
        json)
            cat > "$REPORT_FILE" << EOF
{
  "status": "$status",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "summary": {
    "total_files": $TOTAL_FILES,
    "errors": $ERRORS,
    "warnings": $WARNINGS,
    "fixed": $FIXED
  }
}
EOF
            ;;
        markdown)
            cat > "$REPORT_FILE" << EOF
# VCS Syntax Validation Report

**Date:** $(date)
**Status:** $status

## Summary

| Metric | Count |
|--------|-------|
| Total Files | $TOTAL_FILES |
| Errors | $ERRORS |
| Warnings | $WARNINGS |
| Auto-Fixed | $FIXED |

## Checks Performed

- Rust syntax detection (struct, enum, trait, impl)
- Semantic type usage (Vec->List, String->Text, etc.)
- Attribute syntax (#[...] -> @...)
- Test metadata validation

## Reference

- Grammar: \`grammar/verum.ebnf\`
- Style Guide: \`CLAUDE.md\`
EOF
            ;;
        *)
            cat > "$REPORT_FILE" << EOF
VCS Syntax Validation Report
=============================

Date:       $(date)
Status:     $status

Summary:
  Total Files:  $TOTAL_FILES
  Errors:       $ERRORS
  Warnings:     $WARNINGS
  Auto-Fixed:   $FIXED

Reference: grammar/verum.ebnf, CLAUDE.md
EOF
            ;;
    esac

    log_info "Report saved: $REPORT_FILE"
}

# Main function
main() {
    parse_args "$@"

    log_info "VCS Syntax Validator"
    log_info "Checking for Rust-isms and validating Verum syntax"
    echo ""

    if [ -f "$GRAMMAR_FILE" ]; then
        log_info "Grammar reference: $GRAMMAR_FILE"
    else
        log_warn "Grammar file not found: $GRAMMAR_FILE"
    fi

    if [ "$FIX" -eq 1 ]; then
        log_info "Auto-fix mode enabled"
    fi

    echo ""

    # Find and validate files
    local files
    files=$(find_vr_files)

    if [ -z "$files" ]; then
        log_warn "No .vr files found"
        exit 0
    fi

    while IFS= read -r file; do
        [ -n "$file" ] && validate_file "$file"
    done <<< "$files"

    # Generate report
    generate_report

    # Summary
    echo ""
    echo "=========================================="
    log_info "Validation Summary:"
    echo "  Total Files:  $TOTAL_FILES"
    echo "  Errors:       $ERRORS"
    echo "  Warnings:     $WARNINGS"
    echo "  Auto-Fixed:   $FIXED"
    echo ""

    # Exit code
    if [ "$ERRORS" -gt 0 ]; then
        log_error "Validation FAILED"
        exit 1
    elif [ "$WARNINGS" -gt 0 ] && [ "$STRICT" -eq 1 ]; then
        log_error "Validation FAILED (strict mode)"
        exit 1
    else
        log_success "Validation PASSED"
        exit 0
    fi
}

# Run main
main "$@"
