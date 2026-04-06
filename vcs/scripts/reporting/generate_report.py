#!/usr/bin/env python3
"""
Verum Compliance Suite - Report Generator

Generates comprehensive HTML/JSON/Markdown reports from VCS test results.
Supports aggregating multiple result files and generating visualizations.

Usage:
    python3 generate_report.py [options]

Options:
    --input DIR          Input directory containing result files
    --output FILE        Output file path
    --format FORMAT      Output format: html, json, markdown (default: html)
    --title TITLE        Report title
    --include-trends     Include historical trend data
    --include-coverage   Include coverage data if available
    --baseline FILE      Baseline file for comparison
    --template FILE      Custom HTML template
    --verbose            Enable verbose output
    -h, --help           Show help message

Environment Variables:
    VCS_REPORT_TITLE     Default report title
    VCS_REPORT_LOGO      Path to logo image

Reference: VCS Spec Section 23 - CI/CD Integration
"""

import argparse
import json
import os
import sys
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional, Any
import xml.etree.ElementTree as ET


# Level configuration
LEVELS = {
    'L0': {'name': 'Critical', 'threshold': 100.0, 'blocking': True},
    'L1': {'name': 'Core', 'threshold': 100.0, 'blocking': True},
    'L2': {'name': 'Standard', 'threshold': 95.0, 'blocking': True},
    'L3': {'name': 'Extended', 'threshold': 90.0, 'blocking': False},
    'L4': {'name': 'Performance', 'threshold': 0.0, 'blocking': False},
    'DIFF': {'name': 'Differential', 'threshold': 100.0, 'blocking': True},
}


def parse_junit_xml(filepath: Path) -> Dict[str, Any]:
    """Parse JUnit XML test results."""
    try:
        tree = ET.parse(filepath)
        root = tree.getroot()

        total = int(root.get('tests', 0))
        failures = int(root.get('failures', 0))
        errors = int(root.get('errors', 0))
        skipped = int(root.get('skipped', 0))
        time_taken = float(root.get('time', 0))

        passed = total - failures - errors - skipped
        pass_rate = (passed / total * 100) if total > 0 else 0

        # Extract individual test results
        tests = []
        for testcase in root.iter('testcase'):
            test = {
                'name': testcase.get('name', 'unknown'),
                'classname': testcase.get('classname', ''),
                'time': float(testcase.get('time', 0)),
                'status': 'passed',
            }

            if testcase.find('failure') is not None:
                test['status'] = 'failed'
                test['message'] = testcase.find('failure').get('message', '')
            elif testcase.find('error') is not None:
                test['status'] = 'error'
                test['message'] = testcase.find('error').get('message', '')
            elif testcase.find('skipped') is not None:
                test['status'] = 'skipped'

            tests.append(test)

        return {
            'summary': {
                'total': total,
                'passed': passed,
                'failed': failures + errors,
                'skipped': skipped,
                'pass_percentage': round(pass_rate, 2),
                'time': time_taken,
            },
            'tests': tests,
        }
    except Exception as e:
        print(f"Error parsing {filepath}: {e}", file=sys.stderr)
        return None


def parse_json_results(filepath: Path) -> Dict[str, Any]:
    """Parse JSON test results."""
    try:
        with open(filepath, 'r') as f:
            return json.load(f)
    except Exception as e:
        print(f"Error parsing {filepath}: {e}", file=sys.stderr)
        return None


def collect_results(input_dir: Path) -> Dict[str, Dict[str, Any]]:
    """Collect all result files from input directory."""
    results = {}

    for filepath in input_dir.glob('*'):
        if not filepath.is_file():
            continue

        name = filepath.stem.upper()

        # Determine level from filename
        level = None
        for l in LEVELS:
            if l in name:
                level = l
                break

        if level is None:
            if 'DIFF' in name:
                level = 'DIFF'
            else:
                continue

        if filepath.suffix == '.xml':
            data = parse_junit_xml(filepath)
        elif filepath.suffix == '.json':
            data = parse_json_results(filepath)
        else:
            continue

        if data:
            results[level] = data

    return results


def generate_html_report(
    results: Dict[str, Dict[str, Any]],
    title: str,
    output_path: Path,
    baseline: Optional[Dict] = None,
) -> None:
    """Generate HTML report."""

    timestamp = datetime.utcnow().strftime('%Y-%m-%d %H:%M:%S UTC')

    # Calculate totals
    total_tests = sum(r.get('summary', {}).get('total', 0) for r in results.values())
    total_passed = sum(r.get('summary', {}).get('passed', 0) for r in results.values())
    total_failed = sum(r.get('summary', {}).get('failed', 0) for r in results.values())
    overall_rate = (total_passed / total_tests * 100) if total_tests > 0 else 0

    # Generate level rows
    level_rows = []
    for level, config in LEVELS.items():
        data = results.get(level, {}).get('summary', {})

        total = data.get('total', 0)
        passed = data.get('passed', 0)
        failed = data.get('failed', 0)
        pass_rate = data.get('pass_percentage', 0)

        threshold = config['threshold']
        blocking = config['blocking']

        if threshold > 0:
            if pass_rate >= threshold:
                status = 'PASS'
                status_class = 'pass'
            else:
                status = 'FAIL'
                status_class = 'fail'
        else:
            status = 'INFO'
            status_class = 'info'

        level_rows.append({
            'level': level,
            'name': config['name'],
            'total': total,
            'passed': passed,
            'failed': failed,
            'pass_rate': pass_rate,
            'threshold': f"{threshold}%" if threshold > 0 else 'Advisory',
            'blocking': 'Yes' if blocking else 'No',
            'status': status,
            'status_class': status_class,
        })

    # HTML template
    html = f'''<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <style>
        :root {{
            --bg-primary: #1a1a2e;
            --bg-secondary: #16213e;
            --bg-card: #0f3460;
            --text-primary: #eaeaea;
            --text-secondary: #a0a0a0;
            --accent-green: #4ecca3;
            --accent-red: #e74c3c;
            --accent-yellow: #f1c40f;
            --accent-blue: #3498db;
        }}

        * {{ margin: 0; padding: 0; box-sizing: border-box; }}

        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background-color: var(--bg-primary);
            color: var(--text-primary);
            line-height: 1.6;
        }}

        .container {{ max-width: 1400px; margin: 0 auto; padding: 2rem; }}

        header {{
            background: linear-gradient(135deg, var(--bg-secondary), var(--bg-card));
            padding: 2rem;
            border-radius: 12px;
            margin-bottom: 2rem;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.3);
        }}

        header h1 {{ font-size: 2.5rem; margin-bottom: 0.5rem; }}
        header .meta {{ color: var(--text-secondary); font-size: 0.9rem; }}

        .summary-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1.5rem;
            margin-bottom: 2rem;
        }}

        .summary-card {{
            background: var(--bg-card);
            padding: 1.5rem;
            border-radius: 10px;
            text-align: center;
            box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
        }}

        .summary-card h3 {{
            font-size: 0.9rem;
            color: var(--text-secondary);
            margin-bottom: 0.5rem;
            text-transform: uppercase;
            letter-spacing: 1px;
        }}

        .summary-card .value {{ font-size: 2.5rem; font-weight: bold; }}
        .summary-card .value.pass {{ color: var(--accent-green); }}
        .summary-card .value.fail {{ color: var(--accent-red); }}
        .summary-card .value.info {{ color: var(--accent-blue); }}

        .section {{
            background: var(--bg-card);
            padding: 2rem;
            border-radius: 10px;
            margin-bottom: 2rem;
        }}

        .section h2 {{
            margin-bottom: 1.5rem;
            padding-bottom: 0.5rem;
            border-bottom: 2px solid var(--bg-secondary);
        }}

        table {{
            width: 100%;
            border-collapse: collapse;
        }}

        th, td {{
            padding: 1rem;
            text-align: left;
            border-bottom: 1px solid var(--bg-secondary);
        }}

        th {{
            background: var(--bg-secondary);
            font-weight: 600;
            text-transform: uppercase;
            font-size: 0.85rem;
            letter-spacing: 1px;
        }}

        tr:hover {{ background: rgba(255, 255, 255, 0.05); }}

        .status-badge {{
            display: inline-block;
            padding: 0.25rem 0.75rem;
            border-radius: 20px;
            font-size: 0.8rem;
            font-weight: 600;
            text-transform: uppercase;
        }}

        .status-badge.pass {{ background: rgba(78, 204, 163, 0.2); color: var(--accent-green); }}
        .status-badge.fail {{ background: rgba(231, 76, 60, 0.2); color: var(--accent-red); }}
        .status-badge.info {{ background: rgba(52, 152, 219, 0.2); color: var(--accent-blue); }}

        .progress-bar {{
            width: 100%;
            height: 8px;
            background: var(--bg-secondary);
            border-radius: 4px;
            overflow: hidden;
        }}

        .progress-bar .fill {{
            height: 100%;
            border-radius: 4px;
            transition: width 0.3s ease;
        }}

        .progress-bar .fill.pass {{ background: var(--accent-green); }}
        .progress-bar .fill.fail {{ background: var(--accent-red); }}
        .progress-bar .fill.info {{ background: var(--accent-blue); }}

        footer {{
            text-align: center;
            padding: 2rem;
            color: var(--text-secondary);
            font-size: 0.9rem;
        }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>{title}</h1>
            <div class="meta">Generated: {timestamp} | VCS Version: 1.0</div>
        </header>

        <div class="summary-grid">
            <div class="summary-card">
                <h3>Total Tests</h3>
                <div class="value info">{total_tests}</div>
            </div>
            <div class="summary-card">
                <h3>Passed</h3>
                <div class="value pass">{total_passed}</div>
            </div>
            <div class="summary-card">
                <h3>Failed</h3>
                <div class="value fail">{total_failed}</div>
            </div>
            <div class="summary-card">
                <h3>Pass Rate</h3>
                <div class="value {'pass' if overall_rate >= 95 else 'fail'}">{overall_rate:.1f}%</div>
            </div>
        </div>

        <div class="section">
            <h2>Test Level Results</h2>
            <table>
                <thead>
                    <tr>
                        <th>Level</th>
                        <th>Name</th>
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
'''

    for row in level_rows:
        html += f'''
                    <tr>
                        <td><strong>{row["level"]}</strong></td>
                        <td>{row["name"]}</td>
                        <td>{row["total"]}</td>
                        <td>{row["passed"]}</td>
                        <td>{row["failed"]}</td>
                        <td>{row["pass_rate"]}%</td>
                        <td>{row["threshold"]}</td>
                        <td><span class="status-badge {row["status_class"]}">{row["status"]}</span></td>
                        <td>
                            <div class="progress-bar">
                                <div class="fill {row["status_class"]}" style="width: {row["pass_rate"]}%"></div>
                            </div>
                        </td>
                    </tr>
'''

    html += '''
                </tbody>
            </table>
        </div>

        <div class="section">
            <h2>Compliance Requirements</h2>
            <table>
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
                        <td><span class="status-badge info">RECOMMENDED</span></td>
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

        <footer>
            <p>Verum Compliance Suite (VCS) - Reference: VCS Spec Section 23</p>
            <p>Generated by generate_report.py</p>
        </footer>
    </div>
</body>
</html>
'''

    with open(output_path, 'w') as f:
        f.write(html)


def generate_json_report(
    results: Dict[str, Dict[str, Any]],
    output_path: Path,
) -> None:
    """Generate JSON report."""
    report = {
        'generated': datetime.utcnow().isoformat(),
        'version': '1.0',
        'levels': {},
        'summary': {
            'total': 0,
            'passed': 0,
            'failed': 0,
        },
    }

    for level, data in results.items():
        summary = data.get('summary', {})
        report['levels'][level] = {
            'name': LEVELS.get(level, {}).get('name', level),
            'total': summary.get('total', 0),
            'passed': summary.get('passed', 0),
            'failed': summary.get('failed', 0),
            'pass_percentage': summary.get('pass_percentage', 0),
            'threshold': LEVELS.get(level, {}).get('threshold', 0),
            'blocking': LEVELS.get(level, {}).get('blocking', False),
        }

        report['summary']['total'] += summary.get('total', 0)
        report['summary']['passed'] += summary.get('passed', 0)
        report['summary']['failed'] += summary.get('failed', 0)

    total = report['summary']['total']
    passed = report['summary']['passed']
    report['summary']['pass_percentage'] = round(passed / total * 100, 2) if total > 0 else 0

    with open(output_path, 'w') as f:
        json.dump(report, f, indent=2)


def generate_markdown_report(
    results: Dict[str, Dict[str, Any]],
    title: str,
    output_path: Path,
) -> None:
    """Generate Markdown report."""
    timestamp = datetime.utcnow().strftime('%Y-%m-%d %H:%M:%S UTC')

    total_tests = sum(r.get('summary', {}).get('total', 0) for r in results.values())
    total_passed = sum(r.get('summary', {}).get('passed', 0) for r in results.values())
    overall_rate = (total_passed / total_tests * 100) if total_tests > 0 else 0

    md = f'''# {title}

Generated: {timestamp}

## Summary

| Metric | Value |
|--------|-------|
| Total Tests | {total_tests} |
| Passed | {total_passed} |
| Failed | {total_tests - total_passed} |
| Pass Rate | {overall_rate:.1f}% |

## Results by Level

| Level | Name | Total | Passed | Failed | Pass Rate | Required | Status |
|-------|------|-------|--------|--------|-----------|----------|--------|
'''

    for level, config in LEVELS.items():
        data = results.get(level, {}).get('summary', {})
        total = data.get('total', 0)
        passed = data.get('passed', 0)
        failed = data.get('failed', 0)
        pass_rate = data.get('pass_percentage', 0)
        threshold = config['threshold']

        if threshold > 0:
            status = 'PASS' if pass_rate >= threshold else 'FAIL'
        else:
            status = 'INFO'

        threshold_str = f"{threshold}%" if threshold > 0 else 'Advisory'

        md += f"| {level} | {config['name']} | {total} | {passed} | {failed} | {pass_rate}% | {threshold_str} | {status} |\n"

    md += '''
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
'''

    with open(output_path, 'w') as f:
        f.write(md)


def main():
    parser = argparse.ArgumentParser(
        description='VCS Report Generator',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )

    parser.add_argument('--input', '-i', type=Path, required=True,
                       help='Input directory containing result files')
    parser.add_argument('--output', '-o', type=Path, required=True,
                       help='Output file path')
    parser.add_argument('--format', '-f', choices=['html', 'json', 'markdown'],
                       default='html', help='Output format')
    parser.add_argument('--title', '-t', type=str,
                       default=os.environ.get('VCS_REPORT_TITLE', 'VCS Test Report'),
                       help='Report title')
    parser.add_argument('--baseline', type=Path, default=None,
                       help='Baseline file for comparison')
    parser.add_argument('--verbose', '-v', action='store_true',
                       help='Enable verbose output')

    args = parser.parse_args()

    if not args.input.is_dir():
        print(f"Error: Input directory not found: {args.input}", file=sys.stderr)
        sys.exit(1)

    if args.verbose:
        print(f"Input directory: {args.input}")
        print(f"Output file: {args.output}")
        print(f"Format: {args.format}")

    # Collect results
    results = collect_results(args.input)

    if not results:
        print("Warning: No result files found", file=sys.stderr)
        results = {}

    # Load baseline if provided
    baseline = None
    if args.baseline and args.baseline.exists():
        with open(args.baseline) as f:
            baseline = json.load(f)

    # Generate report
    args.output.parent.mkdir(parents=True, exist_ok=True)

    if args.format == 'html':
        generate_html_report(results, args.title, args.output, baseline)
    elif args.format == 'json':
        generate_json_report(results, args.output)
    elif args.format == 'markdown':
        generate_markdown_report(results, args.title, args.output)

    print(f"Report generated: {args.output}")


if __name__ == '__main__':
    main()
