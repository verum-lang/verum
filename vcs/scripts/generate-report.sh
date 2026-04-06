#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Report Generator
# =============================================================================
#
# This script generates comprehensive HTML reports from VCS test results.
# It aggregates data from multiple test runs and creates visualizations.
#
# Usage:
#   ./generate-report.sh [options]
#
# Options:
#   -i, --input DIR       Input directory containing test results
#   -o, --output FILE     Output HTML file (default: vcs-report.html)
#   -t, --title TITLE     Report title
#   --include-trends      Include historical trend data
#   --include-coverage    Include code coverage data
#   --include-benchmarks  Include benchmark comparisons
#   --format FORMAT       Output format: html, markdown, json
#   -v, --verbose         Verbose output
#   -h, --help            Show this help message
#
# Environment Variables:
#   VCS_REPORT_TITLE      Report title
#   VCS_REPORT_LOGO       Path to logo image
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Configuration
INPUT_DIR="$VCS_ROOT/reports"
OUTPUT_FILE="$VCS_ROOT/reports/vcs-report.html"
TITLE="${VCS_REPORT_TITLE:-Verum Compliance Suite Report}"
INCLUDE_TRENDS=0
INCLUDE_COVERAGE=0
INCLUDE_BENCHMARKS=0
FORMAT="html"
VERBOSE=0

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Print colored message
print_color() {
    local color="$1"
    local message="$2"
    echo -e "${color}${message}${NC}"
}

# Print usage
usage() {
    head -n 28 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
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
            -t|--title)
                TITLE="$2"
                shift 2
                ;;
            --include-trends)
                INCLUDE_TRENDS=1
                shift
                ;;
            --include-coverage)
                INCLUDE_COVERAGE=1
                shift
                ;;
            --include-benchmarks)
                INCLUDE_BENCHMARKS=1
                shift
                ;;
            --format)
                FORMAT="$2"
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
                echo "Unknown option: $1" >&2
                usage
                exit 1
                ;;
        esac
    done
}

# Extract test results from JSON files
extract_results() {
    local results_json="[]"

    for file in "$INPUT_DIR"/*.json; do
        if [ -f "$file" ]; then
            local filename
            filename=$(basename "$file")

            if [[ "$filename" =~ ^l[0-4] ]] || [[ "$filename" =~ ^diff ]]; then
                if [ "$VERBOSE" -eq 1 ]; then
                    echo "Processing: $file"
                fi

                local level total passed failed pass_rate
                level=$(echo "$filename" | grep -oE '^l[0-4]|^diff' | tr '[:lower:]' '[:upper:]')
                total=$(jq -r '.summary.total // 0' "$file" 2>/dev/null || echo "0")
                passed=$(jq -r '.summary.passed // 0' "$file" 2>/dev/null || echo "0")
                failed=$(jq -r '.summary.failed // 0' "$file" 2>/dev/null || echo "0")
                pass_rate=$(jq -r '.summary.pass_percentage // 0' "$file" 2>/dev/null || echo "0")

                results_json=$(echo "$results_json" | jq --arg level "$level" \
                    --argjson total "$total" \
                    --argjson passed "$passed" \
                    --argjson failed "$failed" \
                    --argjson pass_rate "$pass_rate" \
                    '. + [{"level": $level, "total": $total, "passed": $passed, "failed": $failed, "pass_rate": $pass_rate}]')
            fi
        fi
    done

    echo "$results_json"
}

# Generate HTML header
html_header() {
    local timestamp
    timestamp=$(date -u +"%Y-%m-%d %H:%M:%S UTC")

    cat << EOF
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>$TITLE</title>
    <style>
        :root {
            --bg-primary: #1a1a2e;
            --bg-secondary: #16213e;
            --bg-card: #0f3460;
            --text-primary: #eaeaea;
            --text-secondary: #a0a0a0;
            --accent-green: #4ecca3;
            --accent-red: #e74c3c;
            --accent-yellow: #f1c40f;
            --accent-blue: #3498db;
        }

        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
            background-color: var(--bg-primary);
            color: var(--text-primary);
            line-height: 1.6;
        }

        .container {
            max-width: 1400px;
            margin: 0 auto;
            padding: 2rem;
        }

        header {
            background: linear-gradient(135deg, var(--bg-secondary), var(--bg-card));
            padding: 2rem;
            border-radius: 12px;
            margin-bottom: 2rem;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.3);
        }

        header h1 {
            font-size: 2.5rem;
            margin-bottom: 0.5rem;
        }

        header .meta {
            color: var(--text-secondary);
            font-size: 0.9rem;
        }

        .summary-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1.5rem;
            margin-bottom: 2rem;
        }

        .summary-card {
            background: var(--bg-card);
            padding: 1.5rem;
            border-radius: 10px;
            text-align: center;
            box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
        }

        .summary-card h3 {
            font-size: 0.9rem;
            color: var(--text-secondary);
            margin-bottom: 0.5rem;
            text-transform: uppercase;
            letter-spacing: 1px;
        }

        .summary-card .value {
            font-size: 2.5rem;
            font-weight: bold;
        }

        .summary-card .value.pass { color: var(--accent-green); }
        .summary-card .value.fail { color: var(--accent-red); }
        .summary-card .value.warn { color: var(--accent-yellow); }
        .summary-card .value.info { color: var(--accent-blue); }

        .results-table {
            width: 100%;
            border-collapse: collapse;
            background: var(--bg-card);
            border-radius: 10px;
            overflow: hidden;
            margin-bottom: 2rem;
        }

        .results-table th,
        .results-table td {
            padding: 1rem;
            text-align: left;
            border-bottom: 1px solid var(--bg-secondary);
        }

        .results-table th {
            background: var(--bg-secondary);
            font-weight: 600;
            text-transform: uppercase;
            font-size: 0.85rem;
            letter-spacing: 1px;
        }

        .results-table tr:hover {
            background: rgba(255, 255, 255, 0.05);
        }

        .status-badge {
            display: inline-block;
            padding: 0.25rem 0.75rem;
            border-radius: 20px;
            font-size: 0.8rem;
            font-weight: 600;
            text-transform: uppercase;
        }

        .status-badge.pass {
            background: rgba(78, 204, 163, 0.2);
            color: var(--accent-green);
        }

        .status-badge.fail {
            background: rgba(231, 76, 60, 0.2);
            color: var(--accent-red);
        }

        .status-badge.warn {
            background: rgba(241, 196, 15, 0.2);
            color: var(--accent-yellow);
        }

        .progress-bar {
            width: 100%;
            height: 8px;
            background: var(--bg-secondary);
            border-radius: 4px;
            overflow: hidden;
        }

        .progress-bar .fill {
            height: 100%;
            border-radius: 4px;
            transition: width 0.3s ease;
        }

        .progress-bar .fill.pass { background: var(--accent-green); }
        .progress-bar .fill.fail { background: var(--accent-red); }
        .progress-bar .fill.warn { background: var(--accent-yellow); }

        .section {
            background: var(--bg-card);
            padding: 2rem;
            border-radius: 10px;
            margin-bottom: 2rem;
        }

        .section h2 {
            margin-bottom: 1.5rem;
            padding-bottom: 0.5rem;
            border-bottom: 2px solid var(--bg-secondary);
        }

        .chart-container {
            width: 100%;
            height: 300px;
            margin: 1rem 0;
        }

        footer {
            text-align: center;
            padding: 2rem;
            color: var(--text-secondary);
            font-size: 0.9rem;
        }

        @media (max-width: 768px) {
            .container {
                padding: 1rem;
            }

            header h1 {
                font-size: 1.8rem;
            }

            .summary-grid {
                grid-template-columns: repeat(2, 1fr);
            }
        }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>$TITLE</h1>
            <div class="meta">
                Generated: $timestamp | VCS Version: 1.0
            </div>
        </header>
EOF
}

# Generate summary section
html_summary() {
    local results="$1"

    local total_tests passed_tests failed_tests overall_rate
    total_tests=$(echo "$results" | jq '[.[].total] | add // 0')
    passed_tests=$(echo "$results" | jq '[.[].passed] | add // 0')
    failed_tests=$(echo "$results" | jq '[.[].failed] | add // 0')

    if [ "$total_tests" -gt 0 ]; then
        overall_rate=$(echo "scale=1; $passed_tests * 100 / $total_tests" | bc)
    else
        overall_rate="0"
    fi

    local status_class="pass"
    if [ "$failed_tests" -gt 0 ]; then
        status_class="fail"
    fi

    cat << EOF
        <div class="summary-grid">
            <div class="summary-card">
                <h3>Total Tests</h3>
                <div class="value info">$total_tests</div>
            </div>
            <div class="summary-card">
                <h3>Passed</h3>
                <div class="value pass">$passed_tests</div>
            </div>
            <div class="summary-card">
                <h3>Failed</h3>
                <div class="value fail">$failed_tests</div>
            </div>
            <div class="summary-card">
                <h3>Pass Rate</h3>
                <div class="value $status_class">${overall_rate}%</div>
            </div>
        </div>
EOF
}

# Generate results table
html_results_table() {
    local results="$1"

    cat << EOF
        <div class="section">
            <h2>Test Level Results</h2>
            <table class="results-table">
                <thead>
                    <tr>
                        <th>Level</th>
                        <th>Total</th>
                        <th>Passed</th>
                        <th>Failed</th>
                        <th>Pass Rate</th>
                        <th>Required</th>
                        <th>Status</th>
                        <th>Progress</th>
                    </tr>
                </thead>
                <tbody>
EOF

    # Process each level
    echo "$results" | jq -r '.[] | [.level, .total, .passed, .failed, .pass_rate] | @tsv' | while IFS=$'\t' read -r level total passed failed pass_rate; do
        local required status_class status_text

        case "$level" in
            L0|L1)
                required="100%"
                if [ "$(echo "$pass_rate == 100" | bc)" -eq 1 ]; then
                    status_class="pass"
                    status_text="PASS"
                else
                    status_class="fail"
                    status_text="FAIL"
                fi
                ;;
            L2)
                required="95%"
                if [ "$(echo "$pass_rate >= 95" | bc)" -eq 1 ]; then
                    status_class="pass"
                    status_text="PASS"
                else
                    status_class="fail"
                    status_text="FAIL"
                fi
                ;;
            L3)
                required="90%"
                if [ "$(echo "$pass_rate >= 90" | bc)" -eq 1 ]; then
                    status_class="pass"
                    status_text="PASS"
                else
                    status_class="warn"
                    status_text="WARN"
                fi
                ;;
            L4|DIFF)
                required="Advisory"
                status_class="info"
                status_text="INFO"
                ;;
        esac

        cat << EOF
                    <tr>
                        <td><strong>$level</strong></td>
                        <td>$total</td>
                        <td>$passed</td>
                        <td>$failed</td>
                        <td>${pass_rate}%</td>
                        <td>$required</td>
                        <td><span class="status-badge $status_class">$status_text</span></td>
                        <td>
                            <div class="progress-bar">
                                <div class="fill $status_class" style="width: ${pass_rate}%"></div>
                            </div>
                        </td>
                    </tr>
EOF
    done

    cat << EOF
                </tbody>
            </table>
        </div>
EOF
}

# Generate compliance section
html_compliance() {
    cat << EOF
        <div class="section">
            <h2>Compliance Requirements</h2>
            <table class="results-table">
                <thead>
                    <tr>
                        <th>Level</th>
                        <th>Description</th>
                        <th>Requirement</th>
                        <th>Blocking</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td><strong>L0</strong></td>
                        <td>Critical - Semantic correctness, memory safety</td>
                        <td>100% pass</td>
                        <td><span class="status-badge fail">BLOCKING</span></td>
                    </tr>
                    <tr>
                        <td><strong>L1</strong></td>
                        <td>Core - Type system, basic constructs</td>
                        <td>100% pass</td>
                        <td><span class="status-badge fail">BLOCKING</span></td>
                    </tr>
                    <tr>
                        <td><strong>L2</strong></td>
                        <td>Standard - Standard library, common patterns</td>
                        <td>95%+ pass</td>
                        <td><span class="status-badge fail">BLOCKING</span></td>
                    </tr>
                    <tr>
                        <td><strong>L3</strong></td>
                        <td>Extended - GPU, dependent types, advanced features</td>
                        <td>90%+ pass</td>
                        <td><span class="status-badge warn">RECOMMENDED</span></td>
                    </tr>
                    <tr>
                        <td><strong>L4</strong></td>
                        <td>Performance - Timing characteristics</td>
                        <td>Advisory</td>
                        <td><span class="status-badge pass">ADVISORY</span></td>
                    </tr>
                    <tr>
                        <td><strong>DIFF</strong></td>
                        <td>Differential - Tier 0 == Tier 3 equivalence</td>
                        <td>100% equivalent</td>
                        <td><span class="status-badge fail">BLOCKING</span></td>
                    </tr>
                </tbody>
            </table>
        </div>
EOF
}

# Generate footer
html_footer() {
    cat << EOF
        <footer>
            <p>Verum Compliance Suite (VCS) - Reference: VCS Spec Section 23</p>
            <p>Generated by generate-report.sh</p>
        </footer>
    </div>
</body>
</html>
EOF
}

# Generate markdown report
generate_markdown() {
    local results="$1"
    local timestamp
    timestamp=$(date -u +"%Y-%m-%d %H:%M:%S UTC")

    cat << EOF
# $TITLE

Generated: $timestamp

## Summary

| Level | Total | Passed | Failed | Pass Rate | Required | Status |
|-------|-------|--------|--------|-----------|----------|--------|
EOF

    echo "$results" | jq -r '.[] | [.level, .total, .passed, .failed, .pass_rate] | @tsv' | while IFS=$'\t' read -r level total passed failed pass_rate; do
        local required status
        case "$level" in
            L0|L1) required="100%"; [ "$(echo "$pass_rate == 100" | bc)" -eq 1 ] && status="PASS" || status="FAIL" ;;
            L2) required="95%"; [ "$(echo "$pass_rate >= 95" | bc)" -eq 1 ] && status="PASS" || status="FAIL" ;;
            L3) required="90%"; [ "$(echo "$pass_rate >= 90" | bc)" -eq 1 ] && status="PASS" || status="WARN" ;;
            *) required="Advisory"; status="INFO" ;;
        esac
        echo "| $level | $total | $passed | $failed | ${pass_rate}% | $required | $status |"
    done

    cat << EOF

## Compliance Requirements

| Level | Description | Requirement | Blocking |
|-------|-------------|-------------|----------|
| L0 | Critical - Semantic correctness, memory safety | 100% pass | Yes |
| L1 | Core - Type system, basic constructs | 100% pass | Yes |
| L2 | Standard - Standard library, common patterns | 95%+ pass | Yes |
| L3 | Extended - GPU, dependent types, advanced features | 90%+ pass | Recommended |
| L4 | Performance - Timing characteristics | Advisory | No |
| DIFF | Differential - Tier 0 == Tier 3 equivalence | 100% equivalent | Yes |

---

*Verum Compliance Suite (VCS) - Reference: VCS Spec Section 23*
EOF
}

# Main function
main() {
    parse_args "$@"

    if [ ! -d "$INPUT_DIR" ]; then
        echo "Error: Input directory not found: $INPUT_DIR" >&2
        exit 1
    fi

    print_color "$CYAN" "Generating VCS Report..."

    # Extract results
    local results
    results=$(extract_results)

    if [ "$results" = "[]" ]; then
        print_color "$YELLOW" "Warning: No test results found in $INPUT_DIR"
        results='[{"level":"N/A","total":0,"passed":0,"failed":0,"pass_rate":0}]'
    fi

    # Generate report based on format
    case "$FORMAT" in
        html)
            {
                html_header
                html_summary "$results"
                html_results_table "$results"
                html_compliance
                html_footer
            } > "$OUTPUT_FILE"
            ;;
        markdown|md)
            generate_markdown "$results" > "${OUTPUT_FILE%.html}.md"
            OUTPUT_FILE="${OUTPUT_FILE%.html}.md"
            ;;
        json)
            echo "$results" | jq '.' > "${OUTPUT_FILE%.html}.json"
            OUTPUT_FILE="${OUTPUT_FILE%.html}.json"
            ;;
        *)
            echo "Unknown format: $FORMAT" >&2
            exit 1
            ;;
    esac

    print_color "$GREEN" "Report generated: $OUTPUT_FILE"
}

# Run main
main "$@"
