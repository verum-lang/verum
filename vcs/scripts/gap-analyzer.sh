#!/usr/bin/env bash
# =============================================================================
# Verum Compliance Suite - Gap Analyzer
# =============================================================================
# NOTE: Requires Bash 4.0+ for associative arrays
# On macOS, install with: brew install bash
# =============================================================================
#
# This script analyzes test results to identify gaps in coverage and
# prioritize fixes based on severity and impact.
#
# Features:
#   1. Identifies spec sections without test coverage
#   2. Analyzes failing tests by category and severity
#   3. Detects regression patterns
#   4. Generates prioritized fix recommendations
#   5. Cross-references with grammar/verum.ebnf coverage
#
# Usage:
#   ./gap-analyzer.sh [options]
#
# Options:
#   -i, --input DIR       Input directory with test results (default: vcs/results/latest)
#   -s, --specs DIR       Specs directory (default: vcs/specs)
#   -o, --output FILE     Output file for analysis report (default: stdout)
#   -f, --format FORMAT   Output format: text, json, markdown (default: text)
#   --grammar FILE        Grammar file to check coverage (default: grammar/verum.ebnf)
#   --severity LEVEL      Minimum severity to report: all, high, critical (default: all)
#   --group-by TYPE       Group results by: level, category, file, error (default: level)
#   -v, --verbose         Verbose output
#   -h, --help            Show this help message
#
# Exit Codes:
#   0 - Analysis completed successfully
#   1 - Critical gaps found
#   2 - High priority gaps found
#   3 - Configuration or runtime error
#
# Examples:
#   ./gap-analyzer.sh                              # Analyze latest results
#   ./gap-analyzer.sh -f markdown -o gaps.md       # Generate markdown report
#   ./gap-analyzer.sh --severity critical          # Show only critical gaps
#   ./gap-analyzer.sh --group-by category          # Group by test category
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Check bash version (requires 4.0+ for associative arrays)
if ((BASH_VERSINFO[0] < 4)); then
    echo "Error: This script requires Bash 4.0 or higher." >&2
    echo "Current version: $BASH_VERSION" >&2
    echo "On macOS, install with: brew install bash" >&2
    exit 3
fi

# Script directory and paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Configuration
INPUT_DIR="$VCS_ROOT/results/latest"
SPECS_DIR="$VCS_ROOT/specs"
OUTPUT_FILE=""
FORMAT="text"
GRAMMAR_FILE="$PROJECT_ROOT/grammar/verum.ebnf"
SEVERITY_LEVEL="all"
GROUP_BY="level"
VERBOSE=0

# Analysis state
declare -A FAILURE_COUNTS
declare -A CATEGORY_FAILURES
declare -A ERROR_PATTERNS
declare -A COVERAGE_GAPS

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# =============================================================================
# Utility Functions
# =============================================================================

print_color() {
    local color="$1"
    local message="$2"
    echo -e "${color}${message}${NC}"
}

log_info() {
    if [ "$VERBOSE" -eq 1 ]; then
        print_color "$CYAN" "[INFO] $1" >&2
    fi
}

log_verbose() {
    if [ "$VERBOSE" -eq 1 ]; then
        print_color "$DIM" "[VERBOSE] $1" >&2
    fi
}

error_exit() {
    print_color "$RED" "[ERROR] $1" >&2
    exit "${2:-3}"
}

usage() {
    head -n 45 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# =============================================================================
# Data Collection
# =============================================================================

collect_failures() {
    local results_dir="$1"

    log_info "Collecting failures from $results_dir"

    # Find all result JSON files
    local json_files
    json_files=$(find "$results_dir" -name "*.json" -type f 2>/dev/null || true)

    for json_file in $json_files; do
        log_verbose "Processing $json_file"

        if ! command -v jq &> /dev/null; then
            log_info "jq not available, using fallback parsing"
            continue
        fi

        # Extract failures from each file
        local failures
        failures=$(jq -r '.failures[]? | "\(.path)|\(.name)|\(.reason // "unknown")|\(.tier // 0)"' "$json_file" 2>/dev/null || true)

        while IFS='|' read -r path name reason tier; do
            if [ -n "$path" ]; then
                # Extract category from path
                local category
                category=$(echo "$path" | sed -n 's|.*specs/[^/]*/\([^/]*\)/.*|\1|p')
                category="${category:-unknown}"

                # Count by level
                local level
                level=$(echo "$path" | grep -oE 'L[0-4]' | head -1)
                level="${level:-L0}"

                FAILURE_COUNTS["$level"]=$((${FAILURE_COUNTS["$level"]:-0} + 1))
                CATEGORY_FAILURES["$category"]=$((${CATEGORY_FAILURES["$category"]:-0} + 1))

                # Extract error pattern
                local error_type
                error_type=$(echo "$reason" | sed -E 's/^([^:]+):.*$/\1/' | head -c 50)
                ERROR_PATTERNS["$error_type"]=$((${ERROR_PATTERNS["$error_type"]:-0} + 1))
            fi
        done <<< "$failures"
    done
}

# =============================================================================
# Coverage Analysis
# =============================================================================

analyze_spec_coverage() {
    log_info "Analyzing spec coverage..."

    # Get all spec directories (categories)
    local spec_categories
    spec_categories=$(find "$SPECS_DIR" -mindepth 2 -maxdepth 2 -type d 2>/dev/null | sort || true)

    # Count tests per category
    for category_path in $spec_categories; do
        local level category
        level=$(basename "$(dirname "$category_path")")
        category=$(basename "$category_path")

        local test_count
        test_count=$(find "$category_path" -name "*.vr" -type f 2>/dev/null | wc -l)

        if [ "$test_count" -eq 0 ]; then
            COVERAGE_GAPS["$level/$category"]="no_tests"
        elif [ "$test_count" -lt 3 ]; then
            COVERAGE_GAPS["$level/$category"]="low_coverage"
        fi
    done
}

analyze_grammar_coverage() {
    if [ ! -f "$GRAMMAR_FILE" ]; then
        log_info "Grammar file not found: $GRAMMAR_FILE"
        return
    fi

    log_info "Analyzing grammar coverage..."

    # Extract production rules from EBNF
    local grammar_rules
    grammar_rules=$(grep -E '^[a-z_]+\s*=' "$GRAMMAR_FILE" 2>/dev/null | sed 's/\s*=.*//' | sort -u || true)

    # Check which rules have corresponding tests
    for rule in $grammar_rules; do
        # Look for test files that might cover this rule
        local has_test
        has_test=$(find "$SPECS_DIR" -name "*.vr" -exec grep -l "$rule" {} \; 2>/dev/null | head -1 || true)

        if [ -z "$has_test" ]; then
            COVERAGE_GAPS["grammar/$rule"]="no_test"
        fi
    done
}

# =============================================================================
# Priority Calculation
# =============================================================================

calculate_priority() {
    local level="$1"
    local category="$2"
    local failure_count="$3"

    local priority=0

    # Level weight (L0/L1 are critical)
    case "$level" in
        L0) priority=$((priority + 100)) ;;
        L1) priority=$((priority + 80)) ;;
        L2) priority=$((priority + 50)) ;;
        L3) priority=$((priority + 30)) ;;
        L4) priority=$((priority + 10)) ;;
    esac

    # Category weight (memory safety, ownership are critical)
    case "$category" in
        memory-safety) priority=$((priority + 50)) ;;
        ownership) priority=$((priority + 40)) ;;
        cbgr) priority=$((priority + 35)) ;;
        lexer) priority=$((priority + 25)) ;;
        parser) priority=$((priority + 25)) ;;
        types) priority=$((priority + 20)) ;;
        *) priority=$((priority + 10)) ;;
    esac

    # Failure count weight (more failures = higher priority)
    priority=$((priority + failure_count * 2))

    echo "$priority"
}

get_severity() {
    local priority="$1"

    if [ "$priority" -ge 150 ]; then
        echo "CRITICAL"
    elif [ "$priority" -ge 100 ]; then
        echo "HIGH"
    elif [ "$priority" -ge 50 ]; then
        echo "MEDIUM"
    else
        echo "LOW"
    fi
}

should_include_severity() {
    local severity="$1"

    case "$SEVERITY_LEVEL" in
        all)
            return 0
            ;;
        high)
            [ "$severity" = "CRITICAL" ] || [ "$severity" = "HIGH" ]
            ;;
        critical)
            [ "$severity" = "CRITICAL" ]
            ;;
    esac
}

# =============================================================================
# Report Generation - Text Format
# =============================================================================

generate_text_report() {
    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "              VCS GAP ANALYSIS REPORT"
    print_color "$CYAN" "================================================================"
    echo ""
    print_color "$DIM" "  Generated: $(date -u +"%Y-%m-%d %H:%M:%S UTC")"
    print_color "$DIM" "  Input:     $INPUT_DIR"
    echo ""

    # Section 1: Failure Summary by Level
    print_color "$BOLD" "1. FAILURE SUMMARY BY LEVEL"
    print_color "$DIM" "   ----------------------------------------"
    echo ""

    local total_failures=0
    for level in L0 L1 L2 L3 L4; do
        local count="${FAILURE_COUNTS[$level]:-0}"
        total_failures=$((total_failures + count))

        local color="$GREEN"
        if [ "$count" -gt 0 ]; then
            case "$level" in
                L0|L1) color="$RED" ;;
                L2) color="$YELLOW" ;;
                *) color="$BLUE" ;;
            esac
        fi

        printf "   %-8s ${color}%5d failures${NC}\n" "$level:" "$count"
    done
    echo ""
    printf "   %-8s ${BOLD}%5d total${NC}\n" "Total:" "$total_failures"
    echo ""

    # Section 2: Failure Summary by Category
    print_color "$BOLD" "2. FAILURE SUMMARY BY CATEGORY"
    print_color "$DIM" "   ----------------------------------------"
    echo ""

    # Sort categories by failure count
    for category in $(echo "${!CATEGORY_FAILURES[@]}" | tr ' ' '\n' | sort); do
        local count="${CATEGORY_FAILURES[$category]}"
        local color="$YELLOW"
        [ "$count" -gt 10 ] && color="$RED"
        printf "   %-25s ${color}%5d failures${NC}\n" "$category:" "$count"
    done
    echo ""

    # Section 3: Common Error Patterns
    print_color "$BOLD" "3. COMMON ERROR PATTERNS"
    print_color "$DIM" "   ----------------------------------------"
    echo ""

    local pattern_count=0
    for pattern in $(echo "${!ERROR_PATTERNS[@]}" | tr ' ' '\n' | sort); do
        local count="${ERROR_PATTERNS[$pattern]}"
        if [ "$count" -gt 0 ] && [ "$pattern_count" -lt 10 ]; then
            printf "   %-40s %5d occurrences\n" "${pattern:0:40}:" "$count"
            pattern_count=$((pattern_count + 1))
        fi
    done
    [ "$pattern_count" -eq 0 ] && echo "   No error patterns detected"
    echo ""

    # Section 4: Coverage Gaps
    print_color "$BOLD" "4. COVERAGE GAPS"
    print_color "$DIM" "   ----------------------------------------"
    echo ""

    if [ ${#COVERAGE_GAPS[@]} -eq 0 ]; then
        print_color "$GREEN" "   No significant coverage gaps detected"
    else
        for gap in $(echo "${!COVERAGE_GAPS[@]}" | tr ' ' '\n' | sort); do
            local gap_type="${COVERAGE_GAPS[$gap]}"
            local color="$YELLOW"
            [ "$gap_type" = "no_tests" ] && color="$RED"

            printf "   ${color}%-40s %-15s${NC}\n" "$gap" "($gap_type)"
        done
    fi
    echo ""

    # Section 5: Prioritized Fix Recommendations
    print_color "$BOLD" "5. PRIORITIZED FIX RECOMMENDATIONS"
    print_color "$DIM" "   ----------------------------------------"
    echo ""

    local recommendations=()

    # Generate recommendations from failures
    for level in L0 L1 L2 L3 L4; do
        local level_failures="${FAILURE_COUNTS[$level]:-0}"
        if [ "$level_failures" -gt 0 ]; then
            for category in $(echo "${!CATEGORY_FAILURES[@]}" | tr ' ' '\n'); do
                local cat_failures="${CATEGORY_FAILURES[$category]}"
                local priority
                priority=$(calculate_priority "$level" "$category" "$cat_failures")
                local severity
                severity=$(get_severity "$priority")

                if should_include_severity "$severity"; then
                    recommendations+=("$priority|$severity|$level/$category|$cat_failures failures")
                fi
            done
        fi
    done

    # Generate recommendations from coverage gaps
    for gap in $(echo "${!COVERAGE_GAPS[@]}" | tr ' ' '\n'); do
        local level category
        level=$(echo "$gap" | cut -d'/' -f1)
        category=$(echo "$gap" | cut -d'/' -f2)
        local priority
        priority=$(calculate_priority "$level" "$category" 0)
        local severity
        severity=$(get_severity "$priority")

        if should_include_severity "$severity"; then
            local gap_type="${COVERAGE_GAPS[$gap]}"
            recommendations+=("$priority|$severity|$gap|$gap_type")
        fi
    done

    # Sort and display recommendations
    if [ ${#recommendations[@]} -eq 0 ]; then
        print_color "$GREEN" "   No gaps requiring immediate attention"
    else
        printf "   ${BOLD}%-10s %-10s %-30s %s${NC}\n" "Priority" "Severity" "Area" "Issue"
        print_color "$DIM" "   ----------------------------------------"

        for rec in $(printf '%s\n' "${recommendations[@]}" | sort -t'|' -k1 -rn | head -20); do
            IFS='|' read -r priority severity area issue <<< "$rec"

            local severity_color="$GREEN"
            case "$severity" in
                CRITICAL) severity_color="$RED" ;;
                HIGH) severity_color="$YELLOW" ;;
                MEDIUM) severity_color="$BLUE" ;;
            esac

            printf "   %-10s ${severity_color}%-10s${NC} %-30s %s\n" "$priority" "$severity" "$area" "$issue"
        done
    fi
    echo ""

    # Section 6: Suggested Actions
    print_color "$BOLD" "6. SUGGESTED ACTIONS"
    print_color "$DIM" "   ----------------------------------------"
    echo ""

    local l0_failures="${FAILURE_COUNTS[L0]:-0}"
    local l1_failures="${FAILURE_COUNTS[L1]:-0}"
    local l2_failures="${FAILURE_COUNTS[L2]:-0}"

    if [ "$l0_failures" -gt 0 ]; then
        print_color "$RED" "   [BLOCKER] Fix $l0_failures L0 critical failures before release"
    fi

    if [ "$l1_failures" -gt 0 ]; then
        print_color "$RED" "   [BLOCKER] Fix $l1_failures L1 core failures before release"
    fi

    if [ "$l2_failures" -gt 0 ]; then
        print_color "$YELLOW" "   [HIGH] Address $l2_failures L2 standard failures (95% required)"
    fi

    local memory_failures="${CATEGORY_FAILURES[memory-safety]:-0}"
    if [ "$memory_failures" -gt 0 ]; then
        print_color "$RED" "   [CRITICAL] $memory_failures memory safety failures need immediate attention"
    fi

    local ownership_failures="${CATEGORY_FAILURES[ownership]:-0}"
    if [ "$ownership_failures" -gt 0 ]; then
        print_color "$YELLOW" "   [HIGH] $ownership_failures ownership/CBGR failures to investigate"
    fi

    local no_test_gaps
    no_test_gaps=$(echo "${!COVERAGE_GAPS[@]}" | tr ' ' '\n' | while read -r gap; do
        [ "${COVERAGE_GAPS[$gap]}" = "no_tests" ] && echo "$gap"
    done | wc -l)

    if [ "$no_test_gaps" -gt 0 ]; then
        print_color "$YELLOW" "   [MEDIUM] Add tests for $no_test_gaps uncovered spec sections"
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    echo ""
}

# =============================================================================
# Report Generation - JSON Format
# =============================================================================

generate_json_report() {
    local timestamp
    timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    cat << EOF
{
    "timestamp": "$timestamp",
    "input_directory": "$INPUT_DIR",
    "failure_summary": {
        "by_level": {
            "L0": ${FAILURE_COUNTS[L0]:-0},
            "L1": ${FAILURE_COUNTS[L1]:-0},
            "L2": ${FAILURE_COUNTS[L2]:-0},
            "L3": ${FAILURE_COUNTS[L3]:-0},
            "L4": ${FAILURE_COUNTS[L4]:-0}
        },
        "by_category": {
EOF

    local first=1
    for category in $(echo "${!CATEGORY_FAILURES[@]}" | tr ' ' '\n' | sort); do
        [ $first -eq 0 ] && echo ","
        first=0
        printf '            "%s": %d' "$category" "${CATEGORY_FAILURES[$category]}"
    done

    cat << EOF

        },
        "error_patterns": {
EOF

    first=1
    for pattern in $(echo "${!ERROR_PATTERNS[@]}" | tr ' ' '\n' | sort | head -10); do
        [ $first -eq 0 ] && echo ","
        first=0
        local escaped_pattern
        escaped_pattern=$(echo "$pattern" | sed 's/"/\\"/g')
        printf '            "%s": %d' "$escaped_pattern" "${ERROR_PATTERNS[$pattern]}"
    done

    cat << EOF

        }
    },
    "coverage_gaps": [
EOF

    first=1
    for gap in $(echo "${!COVERAGE_GAPS[@]}" | tr ' ' '\n' | sort); do
        [ $first -eq 0 ] && echo ","
        first=0
        printf '        {"area": "%s", "type": "%s"}' "$gap" "${COVERAGE_GAPS[$gap]}"
    done

    cat << EOF

    ],
    "recommendations": [
EOF

    # Generate recommendations
    local recommendations=()
    for level in L0 L1 L2 L3 L4; do
        local level_failures="${FAILURE_COUNTS[$level]:-0}"
        if [ "$level_failures" -gt 0 ]; then
            for category in $(echo "${!CATEGORY_FAILURES[@]}" | tr ' ' '\n'); do
                local cat_failures="${CATEGORY_FAILURES[$category]}"
                local priority
                priority=$(calculate_priority "$level" "$category" "$cat_failures")
                local severity
                severity=$(get_severity "$priority")
                recommendations+=("$priority|$severity|$level/$category|$cat_failures failures")
            done
        fi
    done

    first=1
    for rec in $(printf '%s\n' "${recommendations[@]}" | sort -t'|' -k1 -rn | head -10); do
        [ $first -eq 0 ] && echo ","
        first=0
        IFS='|' read -r priority severity area issue <<< "$rec"
        printf '        {"priority": %d, "severity": "%s", "area": "%s", "issue": "%s"}' \
            "$priority" "$severity" "$area" "$issue"
    done

    cat << EOF

    ]
}
EOF
}

# =============================================================================
# Report Generation - Markdown Format
# =============================================================================

generate_markdown_report() {
    local timestamp
    timestamp=$(date -u +"%Y-%m-%d %H:%M:%S UTC")

    cat << EOF
# VCS Gap Analysis Report

**Generated:** $timestamp
**Input:** $INPUT_DIR

---

## 1. Failure Summary by Level

| Level | Failures | Status |
|-------|----------|--------|
EOF

    for level in L0 L1 L2 L3 L4; do
        local count="${FAILURE_COUNTS[$level]:-0}"
        local status=":white_check_mark:"
        if [ "$count" -gt 0 ]; then
            case "$level" in
                L0|L1) status=":x: BLOCKING" ;;
                L2) status=":warning: HIGH" ;;
                *) status=":information_source:" ;;
            esac
        fi
        echo "| $level | $count | $status |"
    done

    cat << EOF

## 2. Failure Summary by Category

| Category | Failures |
|----------|----------|
EOF

    for category in $(echo "${!CATEGORY_FAILURES[@]}" | tr ' ' '\n' | sort); do
        echo "| $category | ${CATEGORY_FAILURES[$category]} |"
    done

    cat << EOF

## 3. Coverage Gaps

EOF

    if [ ${#COVERAGE_GAPS[@]} -eq 0 ]; then
        echo ":white_check_mark: No significant coverage gaps detected"
    else
        echo "| Area | Gap Type |"
        echo "|------|----------|"
        for gap in $(echo "${!COVERAGE_GAPS[@]}" | tr ' ' '\n' | sort); do
            echo "| $gap | ${COVERAGE_GAPS[$gap]} |"
        done
    fi

    cat << EOF

## 4. Prioritized Fix Recommendations

| Priority | Severity | Area | Issue |
|----------|----------|------|-------|
EOF

    local recommendations=()
    for level in L0 L1 L2 L3 L4; do
        local level_failures="${FAILURE_COUNTS[$level]:-0}"
        if [ "$level_failures" -gt 0 ]; then
            for category in $(echo "${!CATEGORY_FAILURES[@]}" | tr ' ' '\n'); do
                local cat_failures="${CATEGORY_FAILURES[$category]}"
                local priority
                priority=$(calculate_priority "$level" "$category" "$cat_failures")
                local severity
                severity=$(get_severity "$priority")
                recommendations+=("$priority|$severity|$level/$category|$cat_failures failures")
            done
        fi
    done

    for rec in $(printf '%s\n' "${recommendations[@]}" | sort -t'|' -k1 -rn | head -15); do
        IFS='|' read -r priority severity area issue <<< "$rec"
        local badge=""
        case "$severity" in
            CRITICAL) badge=":x:" ;;
            HIGH) badge=":warning:" ;;
            MEDIUM) badge=":large_blue_circle:" ;;
            *) badge=":white_circle:" ;;
        esac
        echo "| $priority | $badge $severity | \`$area\` | $issue |"
    done

    cat << EOF

## 5. Suggested Actions

EOF

    local l0_failures="${FAILURE_COUNTS[L0]:-0}"
    local l1_failures="${FAILURE_COUNTS[L1]:-0}"
    local l2_failures="${FAILURE_COUNTS[L2]:-0}"

    if [ "$l0_failures" -gt 0 ]; then
        echo "- :x: **BLOCKER:** Fix $l0_failures L0 critical failures before release"
    fi
    if [ "$l1_failures" -gt 0 ]; then
        echo "- :x: **BLOCKER:** Fix $l1_failures L1 core failures before release"
    fi
    if [ "$l2_failures" -gt 0 ]; then
        echo "- :warning: **HIGH:** Address $l2_failures L2 standard failures (95% required)"
    fi

    local memory_failures="${CATEGORY_FAILURES[memory-safety]:-0}"
    if [ "$memory_failures" -gt 0 ]; then
        echo "- :x: **CRITICAL:** $memory_failures memory safety failures need immediate attention"
    fi

    cat << EOF

---

*Generated by VCS Gap Analyzer*
EOF
}

# =============================================================================
# Argument Parsing
# =============================================================================

parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -i|--input)
                INPUT_DIR="$2"
                shift 2
                ;;
            -s|--specs)
                SPECS_DIR="$2"
                shift 2
                ;;
            -o|--output)
                OUTPUT_FILE="$2"
                shift 2
                ;;
            -f|--format)
                FORMAT="$2"
                shift 2
                ;;
            --grammar)
                GRAMMAR_FILE="$2"
                shift 2
                ;;
            --severity)
                SEVERITY_LEVEL="$2"
                shift 2
                ;;
            --group-by)
                GROUP_BY="$2"
                shift 2
                ;;
            -v|--verbose)
                VERBOSE=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error_exit "Unknown option: $1" 3
                ;;
        esac
    done
}

# =============================================================================
# Main Entry Point
# =============================================================================

main() {
    parse_args "$@"

    # Validate input directory
    if [ ! -d "$INPUT_DIR" ]; then
        error_exit "Input directory not found: $INPUT_DIR" 3
    fi

    # Collect data
    collect_failures "$INPUT_DIR"
    analyze_spec_coverage
    analyze_grammar_coverage

    # Generate report
    local output
    case "$FORMAT" in
        json)
            output=$(generate_json_report)
            ;;
        markdown|md)
            output=$(generate_markdown_report)
            ;;
        text|*)
            output=$(generate_text_report)
            ;;
    esac

    # Write output
    if [ -n "$OUTPUT_FILE" ]; then
        echo "$output" > "$OUTPUT_FILE"
        log_info "Report written to $OUTPUT_FILE"
    else
        echo "$output"
    fi

    # Determine exit code based on gaps found
    local l0_failures="${FAILURE_COUNTS[L0]:-0}"
    local l1_failures="${FAILURE_COUNTS[L1]:-0}"

    if [ "$l0_failures" -gt 0 ] || [ "$l1_failures" -gt 0 ]; then
        exit 1
    fi

    local l2_failures="${FAILURE_COUNTS[L2]:-0}"
    if [ "$l2_failures" -gt 0 ]; then
        exit 2
    fi

    exit 0
}

# Run main
main "$@"
