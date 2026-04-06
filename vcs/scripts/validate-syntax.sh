#!/bin/bash
# Validates Verum (.vr) files against grammar/verum.ebnf
# Can be used as a pre-commit hook or in CI

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
GRAMMAR_FILE="$PROJECT_ROOT/grammar/verum.ebnf"
PARSER_BIN="$PROJECT_ROOT/target/release/verum-parse"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Options
VERBOSE=0
CHECK_STAGED=0
CI_MODE=0
OUTPUT_FORMAT="console"
FILES=()

usage() {
    cat << EOF
Usage: $(basename "$0") [OPTIONS] [FILES...]

Validate Verum (.vr) file syntax against grammar/verum.ebnf

Options:
    -s, --staged      Check only git staged files
    -v, --verbose     Verbose output
    -c, --ci          CI mode (JSON output, non-zero exit on failure)
    --format FORMAT   Output format: console (default), json, junit
    -h, --help        Show this help message

Examples:
    $(basename "$0") file.vr              # Validate single file
    $(basename "$0") -s                   # Validate staged files
    $(basename "$0") vcs/specs/**/*.vr    # Validate all VCS specs
    $(basename "$0") --ci --format json   # CI mode with JSON output
EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -s|--staged)
                CHECK_STAGED=1
                shift
                ;;
            -v|--verbose)
                VERBOSE=1
                shift
                ;;
            -c|--ci)
                CI_MODE=1
                shift
                ;;
            --format)
                OUTPUT_FORMAT="$2"
                shift 2
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            -*)
                echo "Unknown option: $1"
                usage
                exit 1
                ;;
            *)
                FILES+=("$1")
                shift
                ;;
        esac
    done
}

# Build parser if needed
ensure_parser() {
    if [ ! -f "$PARSER_BIN" ]; then
        if [ "$VERBOSE" -eq 1 ]; then
            echo "Building parser..."
        fi
        (cd "$PROJECT_ROOT" && cargo build --release -p verum_cli --bin verum 2>/dev/null) || {
            # Try alternative binary name
            PARSER_BIN="$PROJECT_ROOT/target/release/verum"
            if [ ! -f "$PARSER_BIN" ]; then
                echo -e "${YELLOW}Parser not available, using fallback validation${NC}"
                PARSER_BIN=""
            fi
        }
    fi
}

# Fallback syntax check using basic parsing patterns
fallback_validate_file() {
    local file="$1"
    local errors=()

    # Read the file content
    local content
    content=$(cat "$file")

    # Check for Rust-style syntax that should be Verum syntax
    if echo "$content" | grep -qE '^\s*struct\s+\w+'; then
        errors+=("Line $(grep -n 'struct\s' "$file" | head -1 | cut -d: -f1): Use 'type Name is { ... };' instead of 'struct'")
    fi

    if echo "$content" | grep -qE '^\s*enum\s+\w+'; then
        errors+=("Line $(grep -n 'enum\s' "$file" | head -1 | cut -d: -f1): Use 'type Name is A | B;' instead of 'enum'")
    fi

    if echo "$content" | grep -qE '^\s*trait\s+\w+'; then
        errors+=("Line $(grep -n 'trait\s' "$file" | head -1 | cut -d: -f1): Use 'type Name is protocol { ... };' instead of 'trait'")
    fi

    if echo "$content" | grep -qE '^\s*impl\s+'; then
        errors+=("Line $(grep -n 'impl\s' "$file" | head -1 | cut -d: -f1): Use 'implement Name { ... }' instead of 'impl'")
    fi

    if echo "$content" | grep -qE 'println!\s*\('; then
        errors+=("Line $(grep -n 'println!' "$file" | head -1 | cut -d: -f1): Use 'print(...)' instead of 'println!(...)'")
    fi

    if echo "$content" | grep -qE 'assert!\s*\('; then
        errors+=("Line $(grep -n 'assert!' "$file" | head -1 | cut -d: -f1): Use 'assert(...)' instead of 'assert!(...)'")
    fi

    if echo "$content" | grep -qE 'vec!\s*\['; then
        errors+=("Line $(grep -n 'vec!' "$file" | head -1 | cut -d: -f1): Use 'list![...]' or 'List::from(...)' instead of 'vec![...]'")
    fi

    if echo "$content" | grep -qE 'Box::new\s*\('; then
        errors+=("Line $(grep -n 'Box::new' "$file" | head -1 | cut -d: -f1): Use 'Heap(...)' instead of 'Box::new(...)'")
    fi

    if [ ${#errors[@]} -gt 0 ]; then
        for err in "${errors[@]}"; do
            echo "  $err"
        done
        return 1
    fi

    return 0
}

# Validate a single file
validate_file() {
    local file="$1"
    local result

    if [ ! -f "$file" ]; then
        if [ "$VERBOSE" -eq 1 ]; then
            echo -e "${YELLOW}SKIP${NC}: $file (not found)"
        fi
        return 0
    fi

    # Run parser in check mode if available
    if [ -n "$PARSER_BIN" ] && [ -x "$PARSER_BIN" ]; then
        if result=$("$PARSER_BIN" parse --check "$file" 2>&1); then
            if [ "$VERBOSE" -eq 1 ]; then
                echo -e "${GREEN}PASS${NC}: $file"
            fi
            return 0
        else
            echo -e "${RED}FAIL${NC}: $file"
            echo "$result" | sed 's/^/  /'
            return 1
        fi
    else
        # Use fallback validation
        if fallback_validate_file "$file"; then
            if [ "$VERBOSE" -eq 1 ]; then
                echo -e "${GREEN}PASS${NC}: $file (fallback)"
            fi
            return 0
        else
            echo -e "${RED}FAIL${NC}: $file"
            return 1
        fi
    fi
}

# Get staged .vr files
get_staged_files() {
    git diff --cached --name-only --diff-filter=ACM -- '*.vr' 2>/dev/null || true
}

# JSON output
output_json() {
    local passed=$1
    local failed=$2
    local total=$3
    local results=$4

    cat << EOF
{
  "passed": $passed,
  "failed": $failed,
  "total": $total,
  "success": $([ "$failed" -eq 0 ] && echo "true" || echo "false"),
  "results": $results
}
EOF
}

# JUnit XML output
output_junit() {
    local passed=$1
    local failed=$2
    local total=$3

    cat << EOF
<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="VCS Syntax Validation" tests="$total" failures="$failed" errors="0">
EOF
    # Individual test cases would be added here
    echo "</testsuite>"
}

main() {
    parse_args "$@"

    ensure_parser

    # Determine files to check
    if [ "$CHECK_STAGED" -eq 1 ]; then
        mapfile -t FILES < <(get_staged_files)
        if [ ${#FILES[@]} -eq 0 ]; then
            if [ "$CI_MODE" -eq 1 ]; then
                echo '{"passed": 0, "failed": 0, "total": 0, "success": true}'
            else
                echo "No staged .vr files to validate"
            fi
            exit 0
        fi
    fi

    if [ ${#FILES[@]} -eq 0 ]; then
        echo "No files specified. Use -s for staged files or provide file paths."
        usage
        exit 1
    fi

    local passed=0
    local failed=0
    local total=${#FILES[@]}
    local results="["

    if [ "$CI_MODE" -eq 0 ]; then
        echo ""
        echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        echo -e "${CYAN}  VCS Syntax Validation${NC}"
        echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        echo ""
        echo "Validating $total .vr file(s)..."
        echo ""
    fi

    for file in "${FILES[@]}"; do
        if validate_file "$file"; then
            passed=$((passed + 1))
            results+="{\"file\":\"$file\",\"status\":\"pass\"},"
        else
            failed=$((failed + 1))
            results+="{\"file\":\"$file\",\"status\":\"fail\"},"
        fi
    done

    # Remove trailing comma and close array
    results="${results%,}]"

    if [ "$CI_MODE" -eq 1 ]; then
        case "$OUTPUT_FORMAT" in
            json)
                output_json "$passed" "$failed" "$total" "$results"
                ;;
            junit)
                output_junit "$passed" "$failed" "$total"
                ;;
            *)
                # Console output for CI
                echo "Passed: $passed, Failed: $failed, Total: $total"
                ;;
        esac
    else
        echo ""
        echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        echo -e "${CYAN}  Syntax Validation Summary${NC}"
        echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        echo -e "  Total:  $total"
        echo -e "  Passed: ${GREEN}$passed${NC}"
        echo -e "  Failed: ${RED}$failed${NC}"
    fi

    if [ "$failed" -gt 0 ]; then
        if [ "$CI_MODE" -eq 0 ]; then
            echo ""
            echo -e "${RED}Syntax validation failed${NC}"
        fi
        exit 1
    fi

    if [ "$CI_MODE" -eq 0 ]; then
        echo ""
        echo -e "${GREEN}All files passed syntax validation${NC}"
    fi
    exit 0
}

main "$@"
