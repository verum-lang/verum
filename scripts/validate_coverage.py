#!/usr/bin/env python3
"""
Coverage Validation Script for Verum Language Platform

This script parses LCOV coverage reports and validates that coverage
meets the specified threshold. It provides detailed analysis of:
- Per-file coverage
- Per-crate coverage
- Coverage gaps (uncovered lines)
- Trend analysis (if historical data available)

Usage:
    python scripts/validate_coverage.py coverage/lcov.info --threshold 95
    python scripts/validate_coverage.py coverage/ --all-crates --threshold 95
    python scripts/validate_coverage.py coverage/ --json --output report.json
"""

import argparse
import json
import sys
import re
from pathlib import Path
from typing import Dict, List, Tuple, Optional
from dataclasses import dataclass, asdict
from collections import defaultdict

# ANSI color codes
class Colors:
    RED = '\033[0;31m'
    GREEN = '\033[0;32m'
    YELLOW = '\033[1;33m'
    BLUE = '\033[0;34m'
    MAGENTA = '\033[0;35m'
    CYAN = '\033[0;36m'
    BOLD = '\033[1m'
    NC = '\033[0m'  # No Color


@dataclass
class FileCoverage:
    """Coverage information for a single file"""
    path: str
    lines_found: int
    lines_hit: int
    functions_found: int
    functions_hit: int
    branches_found: int
    branches_hit: int

    @property
    def line_coverage_percent(self) -> float:
        if self.lines_found == 0:
            return 100.0
        return (self.lines_hit / self.lines_found) * 100

    @property
    def function_coverage_percent(self) -> float:
        if self.functions_found == 0:
            return 100.0
        return (self.functions_hit / self.functions_found) * 100

    @property
    def branch_coverage_percent(self) -> float:
        if self.branches_found == 0:
            return 100.0
        return (self.branches_hit / self.branches_found) * 100


@dataclass
class CrateCoverage:
    """Aggregated coverage for a crate"""
    name: str
    files: List[FileCoverage]
    lines_found: int
    lines_hit: int
    functions_found: int
    functions_hit: int
    branches_found: int
    branches_hit: int

    @property
    def line_coverage_percent(self) -> float:
        if self.lines_found == 0:
            return 100.0
        return (self.lines_hit / self.lines_found) * 100

    @property
    def function_coverage_percent(self) -> float:
        if self.functions_found == 0:
            return 100.0
        return (self.functions_hit / self.functions_found) * 100

    @property
    def branch_coverage_percent(self) -> float:
        if self.branches_found == 0:
            return 100.0
        return (self.branches_hit / self.branches_found) * 100


class LcovParser:
    """Parser for LCOV trace files"""

    def __init__(self, lcov_file: Path):
        self.lcov_file = lcov_file
        self.files: List[FileCoverage] = []

    def parse(self) -> List[FileCoverage]:
        """Parse LCOV file and return list of file coverages"""

        if not self.lcov_file.exists():
            raise FileNotFoundError(f"LCOV file not found: {self.lcov_file}")

        with open(self.lcov_file, 'r') as f:
            content = f.read()

        # Split by end_of_record markers
        records = content.split('end_of_record')

        for record in records:
            if not record.strip():
                continue

            file_cov = self._parse_record(record)
            if file_cov:
                self.files.append(file_cov)

        return self.files

    def _parse_record(self, record: str) -> Optional[FileCoverage]:
        """Parse a single LCOV record"""

        lines = record.strip().split('\n')

        # Extract file path
        sf_line = next((l for l in lines if l.startswith('SF:')), None)
        if not sf_line:
            return None

        file_path = sf_line[3:]  # Remove 'SF:' prefix

        # Initialize counters
        lines_found = 0
        lines_hit = 0
        functions_found = 0
        functions_hit = 0
        branches_found = 0
        branches_hit = 0

        for line in lines:
            if line.startswith('LF:'):  # Lines found
                lines_found = int(line[3:])
            elif line.startswith('LH:'):  # Lines hit
                lines_hit = int(line[3:])
            elif line.startswith('FNF:'):  # Functions found
                functions_found = int(line[4:])
            elif line.startswith('FNH:'):  # Functions hit
                functions_hit = int(line[4:])
            elif line.startswith('BRF:'):  # Branches found
                branches_found = int(line[4:])
            elif line.startswith('BRH:'):  # Branches hit
                branches_hit = int(line[4:])

        return FileCoverage(
            path=file_path,
            lines_found=lines_found,
            lines_hit=lines_hit,
            functions_found=functions_found,
            functions_hit=functions_hit,
            branches_found=branches_found,
            branches_hit=branches_hit,
        )


class CoverageAnalyzer:
    """Analyzer for coverage data"""

    def __init__(self, files: List[FileCoverage]):
        self.files = files

    def group_by_crate(self) -> Dict[str, CrateCoverage]:
        """Group file coverages by crate"""

        crates: Dict[str, List[FileCoverage]] = defaultdict(list)

        for file_cov in self.files:
            # Extract crate name from path
            # Format: /path/to/crates/verum_*/src/...
            match = re.search(r'/crates/(verum_\w+)/', file_cov.path)
            if match:
                crate_name = match.group(1)
            else:
                crate_name = 'unknown'

            crates[crate_name].append(file_cov)

        # Aggregate coverage per crate
        result = {}
        for crate_name, files in crates.items():
            lines_found = sum(f.lines_found for f in files)
            lines_hit = sum(f.lines_hit for f in files)
            functions_found = sum(f.functions_found for f in files)
            functions_hit = sum(f.functions_hit for f in files)
            branches_found = sum(f.branches_found for f in files)
            branches_hit = sum(f.branches_hit for f in files)

            result[crate_name] = CrateCoverage(
                name=crate_name,
                files=files,
                lines_found=lines_found,
                lines_hit=lines_hit,
                functions_found=functions_found,
                functions_hit=functions_hit,
                branches_found=branches_found,
                branches_hit=branches_hit,
            )

        return result

    def find_gaps(self, threshold: float) -> List[FileCoverage]:
        """Find files below coverage threshold"""

        return [f for f in self.files if f.line_coverage_percent < threshold]

    def overall_coverage(self) -> Tuple[float, float, float]:
        """Calculate overall line, function, and branch coverage"""

        total_lines_found = sum(f.lines_found for f in self.files)
        total_lines_hit = sum(f.lines_hit for f in self.files)

        total_functions_found = sum(f.functions_found for f in self.files)
        total_functions_hit = sum(f.functions_hit for f in self.files)

        total_branches_found = sum(f.branches_found for f in self.files)
        total_branches_hit = sum(f.branches_hit for f in self.files)

        line_cov = (total_lines_hit / total_lines_found * 100) if total_lines_found > 0 else 100.0
        func_cov = (total_functions_hit / total_functions_found * 100) if total_functions_found > 0 else 100.0
        branch_cov = (total_branches_hit / total_branches_found * 100) if total_branches_found > 0 else 100.0

        return line_cov, func_cov, branch_cov


def print_coverage_table(crates: Dict[str, CrateCoverage], threshold: float):
    """Print formatted coverage table"""

    print(f"\n{Colors.BOLD}Per-Crate Coverage Report{Colors.NC}")
    print("=" * 80)
    print(f"{'Crate':<25} {'Lines':>12} {'Functions':>12} {'Branches':>12} {'Status':>10}")
    print("-" * 80)

    for crate_name in sorted(crates.keys()):
        crate = crates[crate_name]

        line_cov = crate.line_coverage_percent
        func_cov = crate.function_coverage_percent
        branch_cov = crate.branch_coverage_percent

        # Color based on threshold
        if line_cov >= threshold:
            color = Colors.GREEN
            status = "✓ PASS"
        elif line_cov >= threshold - 5:
            color = Colors.YELLOW
            status = "⚠ CLOSE"
        else:
            color = Colors.RED
            status = "✗ FAIL"

        print(f"{crate_name:<25} {color}{line_cov:>11.2f}%{Colors.NC} "
              f"{func_cov:>11.2f}% {branch_cov:>11.2f}% {color}{status:>10}{Colors.NC}")

    print("=" * 80)


def print_gap_analysis(gaps: List[FileCoverage], threshold: float):
    """Print files with coverage gaps"""

    if not gaps:
        print(f"\n{Colors.GREEN}✓ No files below {threshold}% threshold{Colors.NC}")
        return

    print(f"\n{Colors.YELLOW}Files Below {threshold}% Threshold:{Colors.NC}")
    print("-" * 80)

    # Sort by coverage (lowest first)
    gaps_sorted = sorted(gaps, key=lambda f: f.line_coverage_percent)

    for file_cov in gaps_sorted[:20]:  # Show top 20 worst
        gap = threshold - file_cov.line_coverage_percent
        uncovered = file_cov.lines_found - file_cov.lines_hit

        # Shorten path for display
        path_parts = Path(file_cov.path).parts
        if 'crates' in path_parts:
            idx = path_parts.index('crates')
            short_path = '/'.join(path_parts[idx:idx+3])
        else:
            short_path = file_cov.path[-50:]

        print(f"  {Colors.RED}•{Colors.NC} {short_path}")
        print(f"    Coverage: {file_cov.line_coverage_percent:.2f}% "
              f"(need +{gap:.2f}%, {uncovered} uncovered lines)")


def main():
    parser = argparse.ArgumentParser(description='Validate code coverage for Verum')

    parser.add_argument('input', type=Path,
                       help='LCOV file or directory containing LCOV files')
    parser.add_argument('--threshold', type=float, default=95.0,
                       help='Coverage threshold percentage (default: 95)')
    parser.add_argument('--all-crates', action='store_true',
                       help='Process all crates in directory')
    parser.add_argument('--json', action='store_true',
                       help='Output results as JSON')
    parser.add_argument('--output', type=Path,
                       help='Output file for JSON results')
    parser.add_argument('--fail-below', type=float,
                       help='Exit with error if coverage below this threshold')

    args = parser.parse_args()

    # Collect LCOV files
    lcov_files = []

    if args.input.is_file():
        lcov_files.append(args.input)
    elif args.input.is_dir():
        if args.all_crates:
            lcov_files.extend(args.input.rglob('lcov.info'))
        else:
            # Look for combined report
            combined = args.input / 'combined.info'
            if combined.exists():
                lcov_files.append(combined)
            else:
                lcov_files.extend(args.input.glob('*/lcov.info'))

    if not lcov_files:
        print(f"{Colors.RED}✗ No LCOV files found in {args.input}{Colors.NC}")
        sys.exit(1)

    print(f"{Colors.BLUE}Processing {len(lcov_files)} LCOV file(s)...{Colors.NC}")

    # Parse all files
    all_files = []
    for lcov_file in lcov_files:
        try:
            parser = LcovParser(lcov_file)
            files = parser.parse()
            all_files.extend(files)
            print(f"{Colors.GREEN}✓{Colors.NC} Parsed {lcov_file} ({len(files)} files)")
        except Exception as e:
            print(f"{Colors.RED}✗{Colors.NC} Failed to parse {lcov_file}: {e}")

    if not all_files:
        print(f"{Colors.RED}✗ No coverage data found{Colors.NC}")
        sys.exit(1)

    # Analyze coverage
    analyzer = CoverageAnalyzer(all_files)

    line_cov, func_cov, branch_cov = analyzer.overall_coverage()
    crates = analyzer.group_by_crate()
    gaps = analyzer.find_gaps(args.threshold)

    # Print results
    print(f"\n{Colors.BOLD}Overall Coverage{Colors.NC}")
    print("=" * 80)
    print(f"Line Coverage:     {line_cov:>6.2f}%")
    print(f"Function Coverage: {func_cov:>6.2f}%")
    print(f"Branch Coverage:   {branch_cov:>6.2f}%")
    print("=" * 80)

    print_coverage_table(crates, args.threshold)
    print_gap_analysis(gaps, args.threshold)

    # JSON output
    if args.json:
        result = {
            'overall': {
                'line_coverage': line_cov,
                'function_coverage': func_cov,
                'branch_coverage': branch_cov,
            },
            'crates': {
                name: {
                    'line_coverage': crate.line_coverage_percent,
                    'function_coverage': crate.function_coverage_percent,
                    'branch_coverage': crate.branch_coverage_percent,
                    'lines_found': crate.lines_found,
                    'lines_hit': crate.lines_hit,
                }
                for name, crate in crates.items()
            },
            'threshold': args.threshold,
            'passed': line_cov >= args.threshold,
        }

        output_file = args.output or Path('coverage_report.json')
        with open(output_file, 'w') as f:
            json.dump(result, f, indent=2)

        print(f"\n{Colors.GREEN}✓ JSON report saved to {output_file}{Colors.NC}")

    # Summary
    print(f"\n{Colors.BOLD}Summary{Colors.NC}")
    print("-" * 80)

    passed_crates = sum(1 for c in crates.values() if c.line_coverage_percent >= args.threshold)
    total_crates = len(crates)

    print(f"Total files:        {len(all_files)}")
    print(f"Files below threshold: {len(gaps)}")
    print(f"Crates passed:      {passed_crates}/{total_crates}")

    fail_threshold = args.fail_below if args.fail_below is not None else args.threshold

    if line_cov >= fail_threshold:
        print(f"\n{Colors.GREEN}✓ Coverage {line_cov:.2f}% meets threshold of {fail_threshold}%{Colors.NC}")
        sys.exit(0)
    else:
        gap = fail_threshold - line_cov
        print(f"\n{Colors.RED}✗ Coverage {line_cov:.2f}% below threshold of {fail_threshold}% (need +{gap:.2f}%){Colors.NC}")
        sys.exit(1)


if __name__ == '__main__':
    main()
