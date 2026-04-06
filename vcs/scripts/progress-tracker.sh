#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Progress Tracker
# =============================================================================
#
# This script tracks historical test results and generates progress reports
# with trend analysis and text-based visualizations.
#
# Features:
#   1. Tracks historical pass rates over time
#   2. Calculates trend lines and velocity
#   3. Generates text-based progress charts
#   4. Identifies regression patterns
#   5. Estimates completion dates
#
# Usage:
#   ./progress-tracker.sh [options] [command]
#
# Commands:
#   summary     Show summary of current progress (default)
#   history     Show historical data
#   trends      Show trend analysis
#   chart       Generate text-based progress charts
#   compare     Compare two points in time
#   estimate    Estimate time to 100% completion
#
# Options:
#   -i, --input DIR       History directory (default: vcs/results/history)
#   -o, --output FILE     Output file (default: stdout)
#   -f, --format FORMAT   Output format: text, json, csv (default: text)
#   -l, --level LEVEL     Focus on specific level (L0-L4, all)
#   -n, --last N          Show last N data points (default: 10)
#   --since DATE          Show data since DATE (YYYY-MM-DD)
#   --until DATE          Show data until DATE (YYYY-MM-DD)
#   -v, --verbose         Verbose output
#   -h, --help            Show this help message
#
# Exit Codes:
#   0 - Success
#   1 - Regression detected (pass rate decreased)
#   2 - Stalled progress (no improvement in N runs)
#   3 - Configuration or runtime error
#
# Examples:
#   ./progress-tracker.sh                     # Show summary
#   ./progress-tracker.sh chart               # Generate ASCII charts
#   ./progress-tracker.sh trends -l L0        # L0 trend analysis
#   ./progress-tracker.sh history -n 20       # Last 20 data points
#   ./progress-tracker.sh compare 20240101 20240115
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory and paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Configuration
INPUT_DIR="$VCS_ROOT/results/history"
OUTPUT_FILE=""
FORMAT="text"
LEVEL="all"
LAST_N=10
SINCE_DATE=""
UNTIL_DATE=""
COMMAND="summary"
VERBOSE=0

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
    head -n 50 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# =============================================================================
# Data Collection
# =============================================================================

get_run_directories() {
    local dirs
    dirs=$(find "$INPUT_DIR" -maxdepth 1 -type d -name "run-*" 2>/dev/null | sort)

    # Apply date filters
    if [ -n "$SINCE_DATE" ] || [ -n "$UNTIL_DATE" ]; then
        local filtered=""
        for dir in $dirs; do
            local run_date
            run_date=$(basename "$dir" | sed 's/run-//' | cut -d'-' -f1-3 | tr -d '-')

            local include=1

            if [ -n "$SINCE_DATE" ]; then
                local since_clean
                since_clean=$(echo "$SINCE_DATE" | tr -d '-')
                [ "$run_date" -lt "$since_clean" ] && include=0
            fi

            if [ -n "$UNTIL_DATE" ]; then
                local until_clean
                until_clean=$(echo "$UNTIL_DATE" | tr -d '-')
                [ "$run_date" -gt "$until_clean" ] && include=0
            fi

            [ "$include" -eq 1 ] && filtered="$filtered $dir"
        done
        dirs="$filtered"
    fi

    # Apply last N filter
    if [ "$LAST_N" -gt 0 ]; then
        dirs=$(echo "$dirs" | tr ' ' '\n' | tail -n "$LAST_N")
    fi

    echo "$dirs"
}

extract_run_data() {
    local run_dir="$1"
    local summary_file="$run_dir/summary.json"

    if [ ! -f "$summary_file" ]; then
        log_verbose "No summary file in $run_dir"
        return
    fi

    local timestamp run_number total_tests pass_rate status
    timestamp=$(jq -r '.timestamp // ""' "$summary_file" 2>/dev/null)
    run_number=$(jq -r '.run_number // 0' "$summary_file" 2>/dev/null)
    total_tests=$(jq -r '.overall.total_tests // 0' "$summary_file" 2>/dev/null)
    pass_rate=$(jq -r '.overall.pass_rate // 0' "$summary_file" 2>/dev/null)
    status=$(jq -r '.overall.status // "UNKNOWN"' "$summary_file" 2>/dev/null)

    # Extract per-level data
    local l0_rate l1_rate l2_rate l3_rate l4_rate
    l0_rate=$(jq -r '.levels.L0.pass_rate // 0' "$summary_file" 2>/dev/null)
    l1_rate=$(jq -r '.levels.L1.pass_rate // 0' "$summary_file" 2>/dev/null)
    l2_rate=$(jq -r '.levels.L2.pass_rate // 0' "$summary_file" 2>/dev/null)
    l3_rate=$(jq -r '.levels.L3.pass_rate // 0' "$summary_file" 2>/dev/null)
    l4_rate=$(jq -r '.levels.L4.pass_rate // 0' "$summary_file" 2>/dev/null)

    echo "$timestamp|$run_number|$total_tests|$pass_rate|$status|$l0_rate|$l1_rate|$l2_rate|$l3_rate|$l4_rate"
}

collect_history() {
    local run_dirs
    run_dirs=$(get_run_directories)

    local data=""
    for run_dir in $run_dirs; do
        local run_data
        run_data=$(extract_run_data "$run_dir")
        if [ -n "$run_data" ]; then
            data="$data$run_data\n"
        fi
    done

    echo -e "$data" | sed '/^$/d'
}

# =============================================================================
# Analysis Functions
# =============================================================================

calculate_trend() {
    local data="$1"
    local column="$2"

    if [ -z "$data" ]; then
        echo "0|0"
        return
    fi

    # Extract values for the specified column
    local values
    values=$(echo -e "$data" | cut -d'|' -f"$column" | grep -v '^$')

    if [ -z "$values" ]; then
        echo "0|0"
        return
    fi

    local count=0
    local sum=0
    local first_val=""
    local last_val=""

    while IFS= read -r val; do
        if [ -n "$val" ]; then
            [ -z "$first_val" ] && first_val="$val"
            last_val="$val"
            sum=$(echo "$sum + $val" | bc 2>/dev/null || echo "$sum")
            count=$((count + 1))
        fi
    done <<< "$values"

    if [ "$count" -gt 0 ]; then
        local avg
        avg=$(echo "scale=2; $sum / $count" | bc 2>/dev/null || echo "0")

        local trend
        if [ -n "$first_val" ] && [ -n "$last_val" ]; then
            trend=$(echo "scale=2; $last_val - $first_val" | bc 2>/dev/null || echo "0")
        else
            trend="0"
        fi

        echo "$avg|$trend"
    else
        echo "0|0"
    fi
}

detect_regressions() {
    local data="$1"

    local regressions=""
    local prev_rate=""

    while IFS='|' read -r timestamp run_number total_tests pass_rate status l0 l1 l2 l3 l4; do
        if [ -n "$prev_rate" ] && [ -n "$pass_rate" ]; then
            local diff
            diff=$(echo "scale=2; $pass_rate - $prev_rate" | bc 2>/dev/null || echo "0")
            if [ "$(echo "$diff < -5" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
                regressions="$regressions$timestamp: $prev_rate% -> $pass_rate% (${diff}%)\n"
            fi
        fi
        prev_rate="$pass_rate"
    done <<< "$data"

    echo -e "$regressions"
}

estimate_completion() {
    local data="$1"
    local target="${2:-100}"

    # Get current rate and trend
    local last_entry
    last_entry=$(echo -e "$data" | tail -1)

    if [ -z "$last_entry" ]; then
        echo "insufficient_data"
        return
    fi

    local current_rate
    current_rate=$(echo "$last_entry" | cut -d'|' -f4)

    local trend_data
    trend_data=$(calculate_trend "$data" 4)
    local trend
    trend=$(echo "$trend_data" | cut -d'|' -f2)

    if [ -z "$current_rate" ] || [ -z "$trend" ]; then
        echo "insufficient_data"
        return
    fi

    # Calculate remaining and velocity
    local remaining
    remaining=$(echo "scale=2; $target - $current_rate" | bc 2>/dev/null || echo "0")

    if [ "$(echo "$remaining <= 0" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
        echo "completed"
        return
    fi

    if [ "$(echo "$trend <= 0" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
        echo "stalled"
        return
    fi

    # Estimate runs to completion
    local count
    count=$(echo -e "$data" | wc -l)

    if [ "$count" -gt 1 ]; then
        local runs_needed
        runs_needed=$(echo "scale=0; $remaining / ($trend / ($count - 1))" | bc 2>/dev/null || echo "unknown")
        echo "$runs_needed runs"
    else
        echo "insufficient_data"
    fi
}

# =============================================================================
# Text Chart Generation
# =============================================================================

generate_bar_chart() {
    local data="$1"
    local title="$2"
    local width="${3:-50}"

    echo ""
    print_color "$BOLD" "  $title"
    print_color "$DIM" "  $(printf '%.0s-' $(seq 1 $((width + 20))))"
    echo ""

    while IFS='|' read -r timestamp run_number total_tests pass_rate status l0 l1 l2 l3 l4; do
        if [ -n "$pass_rate" ]; then
            local bar_length
            bar_length=$(echo "scale=0; $pass_rate * $width / 100" | bc 2>/dev/null || echo "0")

            local bar=""
            for ((i=0; i<bar_length; i++)); do
                bar+="="
            done

            # Color based on pass rate
            local color="$GREEN"
            if [ "$(echo "$pass_rate < 90" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
                color="$RED"
            elif [ "$(echo "$pass_rate < 95" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
                color="$YELLOW"
            fi

            local date_part
            date_part=$(echo "$timestamp" | cut -d'T' -f1 | tail -c 11)

            printf "  %-12s ${color}%-${width}s${NC} %6.1f%%\n" "${date_part:-Run $run_number}" "$bar" "$pass_rate"
        fi
    done <<< "$data"

    echo ""
}

generate_sparkline() {
    local data="$1"
    local column="$2"
    local width="${3:-40}"

    local values
    values=$(echo -e "$data" | cut -d'|' -f"$column" | grep -v '^$' | tail -n "$width")

    local min=100
    local max=0

    while IFS= read -r val; do
        [ "$(echo "$val < $min" | bc 2>/dev/null || echo "0")" -eq 1 ] && min="$val"
        [ "$(echo "$val > $max" | bc 2>/dev/null || echo "0")" -eq 1 ] && max="$val"
    done <<< "$values"

    local range
    range=$(echo "scale=2; $max - $min" | bc 2>/dev/null || echo "1")
    [ "$(echo "$range == 0" | bc 2>/dev/null || echo "0")" -eq 1 ] && range="1"

    local sparkline=""
    local chars=("_" "." "-" "=" "*" "#" "@")

    while IFS= read -r val; do
        local normalized
        normalized=$(echo "scale=0; ($val - $min) * 6 / $range" | bc 2>/dev/null || echo "0")
        [ "$normalized" -gt 6 ] && normalized=6
        [ "$normalized" -lt 0 ] && normalized=0
        sparkline+="${chars[$normalized]}"
    done <<< "$values"

    echo "$sparkline"
}

generate_level_comparison_chart() {
    local data="$1"

    echo ""
    print_color "$BOLD" "  Level Pass Rate Comparison (Latest)"
    print_color "$DIM" "  ------------------------------------------------"
    echo ""

    local last_entry
    last_entry=$(echo -e "$data" | tail -1)

    if [ -z "$last_entry" ]; then
        echo "  No data available"
        return
    fi

    local l0 l1 l2 l3 l4
    l0=$(echo "$last_entry" | cut -d'|' -f6)
    l1=$(echo "$last_entry" | cut -d'|' -f7)
    l2=$(echo "$last_entry" | cut -d'|' -f8)
    l3=$(echo "$last_entry" | cut -d'|' -f9)
    l4=$(echo "$last_entry" | cut -d'|' -f10)

    for level_data in "L0|$l0|100" "L1|$l1|100" "L2|$l2|95" "L3|$l3|90" "L4|$l4|0"; do
        IFS='|' read -r level rate required <<< "$level_data"

        local bar_length
        bar_length=$(echo "scale=0; ${rate:-0} * 40 / 100" | bc 2>/dev/null || echo "0")

        local bar=""
        for ((i=0; i<bar_length; i++)); do
            bar+="="
        done

        local color="$GREEN"
        if [ -n "$rate" ] && [ "$required" -gt 0 ]; then
            if [ "$(echo "$rate < $required" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
                color="$RED"
            fi
        fi

        local marker=""
        if [ "$required" -gt 0 ]; then
            local marker_pos
            marker_pos=$(echo "scale=0; $required * 40 / 100" | bc 2>/dev/null || echo "0")
            marker=" (req: ${required}%)"
        fi

        printf "  %-4s ${color}%-40s${NC} %6.1f%%%s\n" "$level" "$bar" "${rate:-0}" "$marker"
    done

    echo ""
}

# =============================================================================
# Command Implementations
# =============================================================================

cmd_summary() {
    local data
    data=$(collect_history)

    if [ -z "$data" ]; then
        print_color "$YELLOW" "No historical data found in $INPUT_DIR"
        return
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "              VCS PROGRESS TRACKER - SUMMARY"
    print_color "$CYAN" "================================================================"
    echo ""

    local count
    count=$(echo -e "$data" | wc -l)

    local last_entry
    last_entry=$(echo -e "$data" | tail -1)

    local first_entry
    first_entry=$(echo -e "$data" | head -1)

    print_color "$DIM" "  Data points: $count"
    print_color "$DIM" "  Period: $(echo "$first_entry" | cut -d'|' -f1 | cut -d'T' -f1) to $(echo "$last_entry" | cut -d'|' -f1 | cut -d'T' -f1)"
    echo ""

    # Current status
    local current_rate current_status total_tests
    current_rate=$(echo "$last_entry" | cut -d'|' -f4)
    current_status=$(echo "$last_entry" | cut -d'|' -f5)
    total_tests=$(echo "$last_entry" | cut -d'|' -f3)

    print_color "$BOLD" "  Current Status"
    print_color "$DIM" "  ----------------------------------------"
    printf "  %-20s %s\n" "Pass Rate:" "${current_rate}%"
    printf "  %-20s %s\n" "Total Tests:" "$total_tests"
    printf "  %-20s %s\n" "Status:" "$current_status"
    echo ""

    # Trend analysis
    local trend_data
    trend_data=$(calculate_trend "$data" 4)
    local avg_rate trend
    avg_rate=$(echo "$trend_data" | cut -d'|' -f1)
    trend=$(echo "$trend_data" | cut -d'|' -f2)

    print_color "$BOLD" "  Trend Analysis"
    print_color "$DIM" "  ----------------------------------------"
    printf "  %-20s %s%%\n" "Average Rate:" "$avg_rate"

    local trend_color="$GREEN"
    local trend_symbol="+"
    if [ "$(echo "$trend < 0" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
        trend_color="$RED"
        trend_symbol=""
    fi
    printf "  %-20s ${trend_color}%s%s%%${NC}\n" "Trend:" "$trend_symbol" "$trend"

    # Estimate completion
    local estimate
    estimate=$(estimate_completion "$data")
    printf "  %-20s %s\n" "Est. Completion:" "$estimate"
    echo ""

    # Generate level comparison chart
    generate_level_comparison_chart "$data"

    # Check for regressions
    local regressions
    regressions=$(detect_regressions "$data")

    if [ -n "$regressions" ]; then
        print_color "$BOLD" "  Regressions Detected"
        print_color "$DIM" "  ----------------------------------------"
        echo -e "$regressions" | while read -r line; do
            [ -n "$line" ] && print_color "$RED" "  - $line"
        done
        echo ""
    fi

    print_color "$CYAN" "================================================================"
    echo ""
}

cmd_history() {
    local data
    data=$(collect_history)

    if [ -z "$data" ]; then
        print_color "$YELLOW" "No historical data found in $INPUT_DIR"
        return
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "              VCS PROGRESS TRACKER - HISTORY"
    print_color "$CYAN" "================================================================"
    echo ""

    printf "  ${BOLD}%-20s %-8s %-8s %-10s %-8s${NC}\n" \
        "Timestamp" "Tests" "Rate" "Status" "Trend"
    print_color "$DIM" "  $(printf '%.0s-' $(seq 1 60))"

    local prev_rate=""

    while IFS='|' read -r timestamp run_number total_tests pass_rate status l0 l1 l2 l3 l4; do
        if [ -n "$timestamp" ]; then
            local date_part
            date_part=$(echo "$timestamp" | cut -d'T' -f1)

            local trend_indicator=" "
            if [ -n "$prev_rate" ]; then
                local diff
                diff=$(echo "scale=2; $pass_rate - $prev_rate" | bc 2>/dev/null || echo "0")
                if [ "$(echo "$diff > 0" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
                    trend_indicator="${GREEN}+${NC}"
                elif [ "$(echo "$diff < 0" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
                    trend_indicator="${RED}-${NC}"
                else
                    trend_indicator="${DIM}=${NC}"
                fi
            fi

            local status_color="$GREEN"
            [ "$status" = "FAIL" ] && status_color="$RED"
            [ "$status" = "WARN" ] && status_color="$YELLOW"

            printf "  %-20s %-8s %-8s ${status_color}%-10s${NC} %b\n" \
                "$date_part" "$total_tests" "${pass_rate}%" "$status" "$trend_indicator"

            prev_rate="$pass_rate"
        fi
    done <<< "$data"

    echo ""
    print_color "$CYAN" "================================================================"
    echo ""
}

cmd_trends() {
    local data
    data=$(collect_history)

    if [ -z "$data" ]; then
        print_color "$YELLOW" "No historical data found in $INPUT_DIR"
        return
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "              VCS PROGRESS TRACKER - TRENDS"
    print_color "$CYAN" "================================================================"
    echo ""

    # Overall trend
    print_color "$BOLD" "  Overall Pass Rate Trend"
    print_color "$DIM" "  ----------------------------------------"

    local trend_data
    trend_data=$(calculate_trend "$data" 4)
    local avg_rate trend
    avg_rate=$(echo "$trend_data" | cut -d'|' -f1)
    trend=$(echo "$trend_data" | cut -d'|' -f2)

    printf "  Average:  %s%%\n" "$avg_rate"

    local trend_color="$GREEN"
    [ "$(echo "$trend < 0" | bc 2>/dev/null || echo "0")" -eq 1 ] && trend_color="$RED"
    printf "  Change:   ${trend_color}%+.2f%%${NC}\n" "$trend"

    local sparkline
    sparkline=$(generate_sparkline "$data" 4)
    printf "  Sparkline: %s\n" "$sparkline"
    echo ""

    # Per-level trends
    print_color "$BOLD" "  Per-Level Trends"
    print_color "$DIM" "  ----------------------------------------"

    for col in 6 7 8 9 10; do
        local level=""
        case "$col" in
            6) level="L0" ;;
            7) level="L1" ;;
            8) level="L2" ;;
            9) level="L3" ;;
            10) level="L4" ;;
        esac

        local level_trend
        level_trend=$(calculate_trend "$data" "$col")
        local level_avg level_change
        level_avg=$(echo "$level_trend" | cut -d'|' -f1)
        level_change=$(echo "$level_trend" | cut -d'|' -f2)

        local change_color="$GREEN"
        [ "$(echo "$level_change < 0" | bc 2>/dev/null || echo "0")" -eq 1 ] && change_color="$RED"

        local spark
        spark=$(generate_sparkline "$data" "$col" 20)

        printf "  %-4s avg: %6.1f%% change: ${change_color}%+6.2f%%${NC}  %s\n" \
            "$level" "$level_avg" "$level_change" "$spark"
    done

    echo ""
    print_color "$CYAN" "================================================================"
    echo ""
}

cmd_chart() {
    local data
    data=$(collect_history)

    if [ -z "$data" ]; then
        print_color "$YELLOW" "No historical data found in $INPUT_DIR"
        return
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "              VCS PROGRESS TRACKER - CHARTS"
    print_color "$CYAN" "================================================================"

    generate_bar_chart "$data" "Overall Pass Rate Over Time"
    generate_level_comparison_chart "$data"

    print_color "$CYAN" "================================================================"
    echo ""
}

cmd_compare() {
    local date1="${1:-}"
    local date2="${2:-}"

    if [ -z "$date1" ] || [ -z "$date2" ]; then
        error_exit "Usage: progress-tracker.sh compare DATE1 DATE2" 3
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "              VCS PROGRESS TRACKER - COMPARE"
    print_color "$CYAN" "================================================================"
    echo ""

    print_color "$BOLD" "  Comparing $date1 vs $date2"
    print_color "$DIM" "  ----------------------------------------"
    echo ""

    # Find runs closest to the dates
    local run1_dir run2_dir
    run1_dir=$(find "$INPUT_DIR" -maxdepth 1 -type d -name "run-${date1}*" | head -1)
    run2_dir=$(find "$INPUT_DIR" -maxdepth 1 -type d -name "run-${date2}*" | head -1)

    if [ -z "$run1_dir" ]; then
        print_color "$YELLOW" "  No run found for $date1"
        return
    fi

    if [ -z "$run2_dir" ]; then
        print_color "$YELLOW" "  No run found for $date2"
        return
    fi

    local data1 data2
    data1=$(extract_run_data "$run1_dir")
    data2=$(extract_run_data "$run2_dir")

    local rate1 rate2
    rate1=$(echo "$data1" | cut -d'|' -f4)
    rate2=$(echo "$data2" | cut -d'|' -f4)

    local diff
    diff=$(echo "scale=2; $rate2 - $rate1" | bc 2>/dev/null || echo "0")

    local diff_color="$GREEN"
    [ "$(echo "$diff < 0" | bc 2>/dev/null || echo "0")" -eq 1 ] && diff_color="$RED"

    printf "  %-12s %-10s %-10s %-10s\n" "" "$date1" "$date2" "Change"
    print_color "$DIM" "  $(printf '%.0s-' $(seq 1 45))"
    printf "  %-12s %-10s %-10s ${diff_color}%+.2f%%${NC}\n" "Overall" "${rate1}%" "${rate2}%" "$diff"

    # Compare levels
    for col in 6 7 8 9 10; do
        local level=""
        case "$col" in
            6) level="L0" ;;
            7) level="L1" ;;
            8) level="L2" ;;
            9) level="L3" ;;
            10) level="L4" ;;
        esac

        local level_rate1 level_rate2
        level_rate1=$(echo "$data1" | cut -d'|' -f"$col")
        level_rate2=$(echo "$data2" | cut -d'|' -f"$col")

        local level_diff
        level_diff=$(echo "scale=2; $level_rate2 - $level_rate1" | bc 2>/dev/null || echo "0")

        local level_color="$GREEN"
        [ "$(echo "$level_diff < 0" | bc 2>/dev/null || echo "0")" -eq 1 ] && level_color="$RED"

        printf "  %-12s %-10s %-10s ${level_color}%+.2f%%${NC}\n" \
            "$level" "${level_rate1}%" "${level_rate2}%" "$level_diff"
    done

    echo ""
    print_color "$CYAN" "================================================================"
    echo ""
}

cmd_estimate() {
    local data
    data=$(collect_history)

    if [ -z "$data" ]; then
        print_color "$YELLOW" "No historical data found in $INPUT_DIR"
        return
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    print_color "$CYAN" "          VCS PROGRESS TRACKER - COMPLETION ESTIMATE"
    print_color "$CYAN" "================================================================"
    echo ""

    print_color "$BOLD" "  Estimated Time to 100% Pass Rate"
    print_color "$DIM" "  ----------------------------------------"
    echo ""

    # Overall estimate
    local overall_estimate
    overall_estimate=$(estimate_completion "$data" 100)
    printf "  %-12s %s\n" "Overall:" "$overall_estimate"

    # Per-level estimates
    for col in 6 7 8 9 10; do
        local level=""
        local target=""
        case "$col" in
            6) level="L0"; target=100 ;;
            7) level="L1"; target=100 ;;
            8) level="L2"; target=95 ;;
            9) level="L3"; target=90 ;;
            10) level="L4"; target=0 ;;
        esac

        if [ "$target" -gt 0 ]; then
            # Get level-specific data
            local level_data
            level_data=$(echo -e "$data" | awk -F'|' -v col="$col" '{print $1"|"$2"|"$3"|"$col"|"$5}')

            local level_estimate
            level_estimate=$(estimate_completion "$level_data" "$target")
            printf "  %-12s %s (target: %d%%)\n" "$level:" "$level_estimate" "$target"
        fi
    done

    echo ""

    # Velocity calculation
    local trend_data
    trend_data=$(calculate_trend "$data" 4)
    local trend
    trend=$(echo "$trend_data" | cut -d'|' -f2)

    local count
    count=$(echo -e "$data" | wc -l)

    if [ "$count" -gt 1 ]; then
        local velocity
        velocity=$(echo "scale=3; $trend / ($count - 1)" | bc 2>/dev/null || echo "0")

        print_color "$BOLD" "  Velocity Analysis"
        print_color "$DIM" "  ----------------------------------------"
        printf "  %-20s %+.3f%% per run\n" "Current Velocity:" "$velocity"

        if [ "$(echo "$velocity > 0" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
            print_color "$GREEN" "  Status: Improving"
        elif [ "$(echo "$velocity < 0" | bc 2>/dev/null || echo "0")" -eq 1 ]; then
            print_color "$RED" "  Status: Regressing"
        else
            print_color "$YELLOW" "  Status: Stalled"
        fi
    fi

    echo ""
    print_color "$CYAN" "================================================================"
    echo ""
}

# =============================================================================
# Output Format Handlers
# =============================================================================

generate_json_output() {
    local data
    data=$(collect_history)

    local timestamp
    timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    cat << EOF
{
    "generated": "$timestamp",
    "data_points": $(echo -e "$data" | wc -l),
    "history": [
EOF

    local first=1
    while IFS='|' read -r ts run_number total_tests pass_rate status l0 l1 l2 l3 l4; do
        [ $first -eq 0 ] && echo ","
        first=0
        printf '        {"timestamp": "%s", "run": %s, "tests": %s, "pass_rate": %s, "status": "%s", "levels": {"L0": %s, "L1": %s, "L2": %s, "L3": %s, "L4": %s}}' \
            "$ts" "${run_number:-0}" "${total_tests:-0}" "${pass_rate:-0}" "${status:-UNKNOWN}" \
            "${l0:-0}" "${l1:-0}" "${l2:-0}" "${l3:-0}" "${l4:-0}"
    done <<< "$data"

    local trend_data
    trend_data=$(calculate_trend "$data" 4)
    local avg trend
    avg=$(echo "$trend_data" | cut -d'|' -f1)
    trend=$(echo "$trend_data" | cut -d'|' -f2)

    cat << EOF

    ],
    "analysis": {
        "average_pass_rate": $avg,
        "trend": $trend,
        "estimate": "$(estimate_completion "$data")"
    }
}
EOF
}

generate_csv_output() {
    local data
    data=$(collect_history)

    echo "timestamp,run_number,total_tests,pass_rate,status,L0,L1,L2,L3,L4"

    while IFS='|' read -r ts run_number total_tests pass_rate status l0 l1 l2 l3 l4; do
        echo "$ts,$run_number,$total_tests,$pass_rate,$status,$l0,$l1,$l2,$l3,$l4"
    done <<< "$data"
}

# =============================================================================
# Argument Parsing
# =============================================================================

parse_args() {
    # Extract command if present
    case "${1:-}" in
        summary|history|trends|chart|compare|estimate)
            COMMAND="$1"
            shift
            ;;
    esac

    while [[ $# -gt 0 ]]; do
        case $1 in
            -i|--input)
                INPUT_DIR="$2"
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
            -l|--level)
                LEVEL="$2"
                shift 2
                ;;
            -n|--last)
                LAST_N="$2"
                shift 2
                ;;
            --since)
                SINCE_DATE="$2"
                shift 2
                ;;
            --until)
                UNTIL_DATE="$2"
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
                # Handle compare command arguments
                if [ "$COMMAND" = "compare" ]; then
                    break
                fi
                error_exit "Unknown option: $1" 3
                ;;
        esac
    done

    # Store remaining args for compare command
    COMPARE_ARGS=("$@")
}

# =============================================================================
# Main Entry Point
# =============================================================================

main() {
    parse_args "$@"

    # Validate input directory
    if [ ! -d "$INPUT_DIR" ]; then
        error_exit "History directory not found: $INPUT_DIR" 3
    fi

    # Check for jq dependency
    if ! command -v jq &> /dev/null; then
        error_exit "jq is required for JSON parsing. Please install jq." 3
    fi

    # Handle format-specific output
    if [ "$FORMAT" = "json" ]; then
        local output
        output=$(generate_json_output)
        if [ -n "$OUTPUT_FILE" ]; then
            echo "$output" > "$OUTPUT_FILE"
        else
            echo "$output"
        fi
        exit 0
    fi

    if [ "$FORMAT" = "csv" ]; then
        local output
        output=$(generate_csv_output)
        if [ -n "$OUTPUT_FILE" ]; then
            echo "$output" > "$OUTPUT_FILE"
        else
            echo "$output"
        fi
        exit 0
    fi

    # Execute command
    case "$COMMAND" in
        summary)
            cmd_summary
            ;;
        history)
            cmd_history
            ;;
        trends)
            cmd_trends
            ;;
        chart)
            cmd_chart
            ;;
        compare)
            cmd_compare "${COMPARE_ARGS[@]}"
            ;;
        estimate)
            cmd_estimate
            ;;
        *)
            error_exit "Unknown command: $COMMAND" 3
            ;;
    esac

    exit 0
}

# Run main
main "$@"
