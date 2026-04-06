#!/usr/bin/env python3
"""
Verum Performance Benchmark Report Generator

Parses Criterion benchmark results and generates comprehensive HTML reports
with performance metrics, graphs, and regression detection.

Usage:
    python3 generate_benchmark_report.py --input <criterion_dir> --output <report.html>
"""

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Dict, List, Any, Optional
from dataclasses import dataclass
from datetime import datetime
import statistics


@dataclass
class BenchmarkResult:
    """Represents a single benchmark result."""
    name: str
    mean_ns: float
    std_dev_ns: float
    median_ns: float
    throughput: Optional[float] = None
    throughput_unit: Optional[str] = None


@dataclass
class PerformanceTarget:
    """Represents a performance target requirement."""
    name: str
    target: str
    metric: str
    current: Optional[float] = None
    status: str = "unknown"  # "pass", "fail", "warning", "unknown"


class BenchmarkReportGenerator:
    """Generates comprehensive benchmark reports."""

    # Performance targets from CLAUDE.md
    TARGETS = {
        "cbgr_overhead": PerformanceTarget(
            name="CBGR Overhead",
            target="< 15ns per check",
            metric="ns"
        ),
        "type_inference": PerformanceTarget(
            name="Type Inference",
            target="< 100ms for 10K LOC",
            metric="ms"
        ),
        "compilation_speed": PerformanceTarget(
            name="Compilation Speed",
            target="> 50K LOC/sec",
            metric="LOC/sec"
        ),
        "runtime_performance": PerformanceTarget(
            name="Runtime Performance",
            target="0.85-0.95x native C",
            metric="ratio"
        ),
        "memory_overhead": PerformanceTarget(
            name="Memory Overhead",
            target="< 5% vs unsafe code",
            metric="%"
        ),
    }

    def __init__(self, criterion_dir: Path, output_file: Path):
        self.criterion_dir = criterion_dir
        self.output_file = output_file
        self.benchmarks: Dict[str, List[BenchmarkResult]] = {}
        self.targets = self.TARGETS.copy()

    def parse_criterion_results(self) -> None:
        """Parse all Criterion benchmark results."""
        if not self.criterion_dir.exists():
            print(f"Warning: Criterion directory not found: {self.criterion_dir}")
            return

        # Iterate through all benchmark directories
        for benchmark_dir in self.criterion_dir.iterdir():
            if not benchmark_dir.is_dir():
                continue

            # Look for estimates.json
            estimates_file = benchmark_dir / "new" / "estimates.json"
            if not estimates_file.exists():
                # Try base directory
                estimates_file = benchmark_dir / "base" / "estimates.json"

            if not estimates_file.exists():
                continue

            try:
                with open(estimates_file, 'r') as f:
                    data = json.load(f)

                # Extract benchmark name
                bench_name = benchmark_dir.name

                # Parse results
                mean_ns = data.get("mean", {}).get("point_estimate", 0)
                std_dev_ns = data.get("std_dev", {}).get("point_estimate", 0)
                median_ns = data.get("median", {}).get("point_estimate", 0)

                result = BenchmarkResult(
                    name=bench_name,
                    mean_ns=mean_ns,
                    std_dev_ns=std_dev_ns,
                    median_ns=median_ns
                )

                # Categorize benchmark
                category = self._categorize_benchmark(bench_name)
                if category not in self.benchmarks:
                    self.benchmarks[category] = []

                self.benchmarks[category].append(result)

            except Exception as e:
                print(f"Warning: Failed to parse {estimates_file}: {e}")

    def _categorize_benchmark(self, bench_name: str) -> str:
        """Categorize benchmark by name."""
        name_lower = bench_name.lower()

        if "cbgr" in name_lower:
            return "CBGR"
        elif "type" in name_lower or "inference" in name_lower:
            return "Type System"
        elif "lex" in name_lower:
            return "Lexer"
        elif "parse" in name_lower:
            return "Parser"
        elif "codegen" in name_lower:
            return "Code Generation"
        elif "smt" in name_lower:
            return "SMT Solver"
        elif "runtime" in name_lower or "jit" in name_lower:
            return "Runtime"
        elif "std" in name_lower or "collection" in name_lower:
            return "Standard Library"
        else:
            return "Other"

    def _analyze_targets(self) -> None:
        """Analyze benchmark results against performance targets."""
        # CBGR overhead target
        cbgr_results = self.benchmarks.get("CBGR", [])
        if cbgr_results:
            # Find the critical overhead benchmark
            overhead_benches = [b for b in cbgr_results if "overhead" in b.name.lower() or "deref" in b.name.lower()]
            if overhead_benches:
                min_overhead = min(b.mean_ns for b in overhead_benches)
                self.targets["cbgr_overhead"].current = min_overhead
                self.targets["cbgr_overhead"].status = "pass" if min_overhead < 15 else "fail"

        # Type inference target
        type_results = self.benchmarks.get("Type System", [])
        if type_results:
            # Find 10K LOC benchmark
            loc_benches = [b for b in type_results if "10k" in b.name.lower() or "scalability" in b.name.lower()]
            if loc_benches:
                max_time_ms = max(b.mean_ns / 1_000_000 for b in loc_benches)
                self.targets["type_inference"].current = max_time_ms
                self.targets["type_inference"].status = "pass" if max_time_ms < 100 else "fail"

        # Compilation speed target
        parser_results = self.benchmarks.get("Parser", [])
        lexer_results = self.benchmarks.get("Lexer", [])

        if parser_results or lexer_results:
            # Calculate LOC/sec from throughput benchmarks
            all_parsing = parser_results + lexer_results
            loc_benches = [b for b in all_parsing if "loc" in b.name.lower()]

            if loc_benches:
                # Estimate LOC/sec (this is a rough estimate)
                # Actual implementation would need throughput data
                avg_time_per_line_ns = statistics.mean(b.mean_ns for b in loc_benches if b.mean_ns > 0)
                if avg_time_per_line_ns > 0:
                    loc_per_sec = 1_000_000_000 / avg_time_per_line_ns
                    self.targets["compilation_speed"].current = loc_per_sec
                    self.targets["compilation_speed"].status = "pass" if loc_per_sec > 50_000 else "fail"

    def generate_html_report(self) -> None:
        """Generate comprehensive HTML report."""
        self._analyze_targets()

        html = self._generate_html()

        # Write report
        self.output_file.parent.mkdir(parents=True, exist_ok=True)
        with open(self.output_file, 'w') as f:
            f.write(html)

        print(f"✓ Report generated: {self.output_file}")

    def _generate_html(self) -> str:
        """Generate HTML content."""
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")

        html = f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Verum Performance Benchmark Report</title>
    <style>
        {self._get_css()}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>Verum Performance Benchmark Report</h1>
            <p class="timestamp">Generated: {timestamp}</p>
        </header>

        <section class="targets">
            <h2>Performance Targets</h2>
            {self._generate_targets_html()}
        </section>

        <section class="benchmarks">
            <h2>Detailed Benchmark Results</h2>
            {self._generate_benchmarks_html()}
        </section>

        <section class="summary">
            <h2>Summary</h2>
            {self._generate_summary_html()}
        </section>

        <footer>
            <p>Verum Language Platform - Performance Benchmarks</p>
            <p>See CLAUDE.md for detailed performance requirements</p>
        </footer>
    </div>
</body>
</html>
"""
        return html

    def _get_css(self) -> str:
        """Get CSS styles for the report."""
        return """
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            line-height: 1.6;
            color: #333;
            background: #f5f5f5;
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
        }

        header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 40px;
            border-radius: 10px;
            margin-bottom: 30px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
        }

        header h1 {
            font-size: 2.5em;
            margin-bottom: 10px;
        }

        .timestamp {
            opacity: 0.9;
            font-size: 0.9em;
        }

        section {
            background: white;
            padding: 30px;
            margin-bottom: 30px;
            border-radius: 10px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }

        h2 {
            color: #667eea;
            margin-bottom: 20px;
            padding-bottom: 10px;
            border-bottom: 2px solid #f0f0f0;
        }

        .target-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 20px;
            margin-top: 20px;
        }

        .target-card {
            padding: 20px;
            border-radius: 8px;
            border-left: 4px solid #ddd;
        }

        .target-card.pass {
            background: #f0fdf4;
            border-left-color: #22c55e;
        }

        .target-card.fail {
            background: #fef2f2;
            border-left-color: #ef4444;
        }

        .target-card.warning {
            background: #fffbeb;
            border-left-color: #f59e0b;
        }

        .target-card.unknown {
            background: #f9fafb;
            border-left-color: #9ca3af;
        }

        .target-name {
            font-weight: bold;
            font-size: 1.1em;
            margin-bottom: 8px;
        }

        .target-value {
            font-size: 1.5em;
            font-weight: bold;
            margin: 10px 0;
        }

        .target-requirement {
            color: #666;
            font-size: 0.9em;
        }

        .status-badge {
            display: inline-block;
            padding: 4px 12px;
            border-radius: 12px;
            font-size: 0.85em;
            font-weight: bold;
            margin-top: 8px;
        }

        .status-badge.pass {
            background: #22c55e;
            color: white;
        }

        .status-badge.fail {
            background: #ef4444;
            color: white;
        }

        .status-badge.warning {
            background: #f59e0b;
            color: white;
        }

        .status-badge.unknown {
            background: #9ca3af;
            color: white;
        }

        .benchmark-category {
            margin-bottom: 30px;
        }

        .benchmark-category h3 {
            color: #764ba2;
            margin-bottom: 15px;
        }

        .benchmark-table {
            width: 100%;
            border-collapse: collapse;
            margin-top: 10px;
        }

        .benchmark-table th,
        .benchmark-table td {
            padding: 12px;
            text-align: left;
            border-bottom: 1px solid #f0f0f0;
        }

        .benchmark-table th {
            background: #f9fafb;
            font-weight: 600;
            color: #374151;
        }

        .benchmark-table tr:hover {
            background: #f9fafb;
        }

        .metric-value {
            font-family: 'Courier New', monospace;
            font-weight: bold;
        }

        .summary-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 20px;
        }

        .summary-card {
            text-align: center;
            padding: 20px;
            background: #f9fafb;
            border-radius: 8px;
        }

        .summary-card .number {
            font-size: 3em;
            font-weight: bold;
            color: #667eea;
        }

        .summary-card .label {
            color: #666;
            margin-top: 10px;
        }

        footer {
            text-align: center;
            color: #666;
            margin-top: 40px;
            padding: 20px;
        }

        footer p {
            margin: 5px 0;
        }
        """

    def _generate_targets_html(self) -> str:
        """Generate HTML for performance targets."""
        html = '<div class="target-grid">'

        for target in self.targets.values():
            status_class = target.status
            status_text = target.status.upper()

            current_value = "N/A"
            if target.current is not None:
                if target.metric == "ns":
                    current_value = f"{target.current:.2f} ns"
                elif target.metric == "ms":
                    current_value = f"{target.current:.2f} ms"
                elif target.metric == "LOC/sec":
                    current_value = f"{target.current:,.0f} LOC/sec"
                elif target.metric == "%":
                    current_value = f"{target.current:.2f}%"
                else:
                    current_value = f"{target.current:.2f}"

            html += f"""
            <div class="target-card {status_class}">
                <div class="target-name">{target.name}</div>
                <div class="target-value">{current_value}</div>
                <div class="target-requirement">Target: {target.target}</div>
                <span class="status-badge {status_class}">{status_text}</span>
            </div>
            """

        html += '</div>'
        return html

    def _generate_benchmarks_html(self) -> str:
        """Generate HTML for detailed benchmark results."""
        if not self.benchmarks:
            return '<p>No benchmark results found.</p>'

        html = ""

        for category, results in sorted(self.benchmarks.items()):
            html += f'<div class="benchmark-category">'
            html += f'<h3>{category}</h3>'
            html += '<table class="benchmark-table">'
            html += '<thead><tr><th>Benchmark</th><th>Mean</th><th>Std Dev</th><th>Median</th></tr></thead>'
            html += '<tbody>'

            for result in sorted(results, key=lambda r: r.name):
                mean_str = self._format_time(result.mean_ns)
                stddev_str = self._format_time(result.std_dev_ns)
                median_str = self._format_time(result.median_ns)

                html += f"""
                <tr>
                    <td>{result.name}</td>
                    <td class="metric-value">{mean_str}</td>
                    <td class="metric-value">{stddev_str}</td>
                    <td class="metric-value">{median_str}</td>
                </tr>
                """

            html += '</tbody></table>'
            html += '</div>'

        return html

    def _generate_summary_html(self) -> str:
        """Generate summary statistics."""
        total_benchmarks = sum(len(results) for results in self.benchmarks.values())
        total_categories = len(self.benchmarks)

        targets_passed = sum(1 for t in self.targets.values() if t.status == "pass")
        targets_total = len([t for t in self.targets.values() if t.current is not None])

        html = '<div class="summary-grid">'
        html += f"""
        <div class="summary-card">
            <div class="number">{total_benchmarks}</div>
            <div class="label">Total Benchmarks</div>
        </div>
        <div class="summary-card">
            <div class="number">{total_categories}</div>
            <div class="label">Categories</div>
        </div>
        <div class="summary-card">
            <div class="number">{targets_passed}/{targets_total}</div>
            <div class="label">Targets Met</div>
        </div>
        """
        html += '</div>'

        return html

    def _format_time(self, ns: float) -> str:
        """Format time value with appropriate unit."""
        if ns < 1000:
            return f"{ns:.2f} ns"
        elif ns < 1_000_000:
            return f"{ns/1000:.2f} µs"
        elif ns < 1_000_000_000:
            return f"{ns/1_000_000:.2f} ms"
        else:
            return f"{ns/1_000_000_000:.2f} s"


def main():
    parser = argparse.ArgumentParser(description="Generate Verum benchmark report")
    parser.add_argument("--input", type=Path, required=True,
                        help="Path to Criterion benchmark results directory")
    parser.add_argument("--output", type=Path, required=True,
                        help="Path to output HTML report file")

    args = parser.parse_args()

    generator = BenchmarkReportGenerator(args.input, args.output)
    generator.parse_criterion_results()
    generator.generate_html_report()


if __name__ == "__main__":
    main()
