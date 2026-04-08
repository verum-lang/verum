#!/usr/bin/env python3
"""
compare_outputs.py - Semantic comparison of tier outputs

This script performs semantic comparison of outputs from different execution
tiers, handling acceptable differences like:
- Float precision variations
- Unordered collection output
- Memory address differences
- Whitespace normalization

Usage:
    compare_outputs.py <tier0_output> <tier3_output> [OPTIONS]

Options:
    --float-epsilon N    Float comparison tolerance (default: 1e-10)
    --allow-unordered    Allow unordered comparison for collections
    --normalize-ws       Normalize whitespace
    --strip-ansi         Strip ANSI color codes
    --verbose            Show detailed comparison
    --json               Output results as JSON
    --diff               Show unified diff on mismatch

Exit codes:
    0 - Outputs are semantically equivalent
    1 - Outputs differ
    2 - Error during comparison

Examples:
    compare_outputs.py tier0.txt tier3.txt
    compare_outputs.py tier0.txt tier3.txt --float-epsilon 1e-6 --verbose
    compare_outputs.py tier0.txt tier3.txt --json
"""

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from difflib import unified_diff
from enum import Enum
from pathlib import Path
from typing import Any, List, Optional, Tuple, Union


class DiffType(Enum):
    """Type of difference detected"""
    NONE = "none"
    LINE_COUNT = "line_count"
    EXACT_MISMATCH = "exact_mismatch"
    FLOAT_PRECISION = "float_precision"
    ORDERING = "ordering"
    WHITESPACE = "whitespace"
    ADDRESS = "address"
    TIMESTAMP = "timestamp"


@dataclass
class Difference:
    """A single difference between outputs"""
    line_number: int
    diff_type: DiffType
    expected: str
    actual: str
    acceptable: bool = False
    message: str = ""


@dataclass
class ComparisonResult:
    """Result of comparing two outputs"""
    equivalent: bool
    differences: List[Difference] = field(default_factory=list)
    critical_count: int = 0
    acceptable_count: int = 0

    def to_dict(self) -> dict:
        return {
            "equivalent": self.equivalent,
            "critical_count": self.critical_count,
            "acceptable_count": self.acceptable_count,
            "differences": [
                {
                    "line": d.line_number,
                    "type": d.diff_type.value,
                    "expected": d.expected,
                    "actual": d.actual,
                    "acceptable": d.acceptable,
                    "message": d.message,
                }
                for d in self.differences
            ],
        }


class OutputNormalizer:
    """Normalizes output for comparison"""

    # Pattern for memory addresses
    ADDRESS_PATTERN = re.compile(r'0x[0-9a-fA-F]+')

    # Pattern for ANSI escape codes
    ANSI_PATTERN = re.compile(r'\x1b\[[0-9;]*[a-zA-Z]')

    # Pattern for timestamps
    TIMESTAMP_PATTERN = re.compile(
        r'\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?'
    )

    # Pattern for duration measurements
    DURATION_PATTERN = re.compile(r'\d+(\.\d+)?\s*(ms|us|ns|s)\b')

    def __init__(
        self,
        strip_ansi: bool = True,
        normalize_whitespace: bool = False,
        strip_addresses: bool = True,
        strip_timestamps: bool = True,
        normalize_line_endings: bool = True,
    ):
        self.strip_ansi = strip_ansi
        self.normalize_whitespace = normalize_whitespace
        self.strip_addresses = strip_addresses
        self.strip_timestamps = strip_timestamps
        self.normalize_line_endings = normalize_line_endings

    def normalize(self, text: str) -> str:
        """Normalize text for comparison"""
        result = text

        if self.normalize_line_endings:
            result = result.replace('\r\n', '\n')

        if self.strip_ansi:
            result = self.ANSI_PATTERN.sub('', result)

        if self.strip_addresses:
            result = self.ADDRESS_PATTERN.sub('<ADDRESS>', result)

        if self.strip_timestamps:
            result = self.TIMESTAMP_PATTERN.sub('<TIMESTAMP>', result)
            result = self.DURATION_PATTERN.sub('<DURATION>', result)

        if self.normalize_whitespace:
            # Collapse multiple whitespace to single space
            result = re.sub(r'[ \t]+', ' ', result)
            # Normalize line-by-line
            lines = [line.strip() for line in result.split('\n')]
            result = '\n'.join(lines)

        return result


class SemanticComparator:
    """Performs semantic comparison of outputs"""

    # Pattern for floating-point numbers
    FLOAT_PATTERN = re.compile(r'-?\d+\.\d+([eE][+-]?\d+)?')

    def __init__(
        self,
        float_epsilon: float = 1e-10,
        allow_unordered: bool = False,
        normalizer: Optional[OutputNormalizer] = None,
    ):
        self.float_epsilon = float_epsilon
        self.allow_unordered = allow_unordered
        self.normalizer = normalizer or OutputNormalizer()

    def compare(self, expected: str, actual: str) -> ComparisonResult:
        """Compare two outputs for semantic equivalence"""
        # Normalize both outputs
        norm_expected = self.normalizer.normalize(expected)
        norm_actual = self.normalizer.normalize(actual)

        # Quick exact match check
        if norm_expected == norm_actual:
            return ComparisonResult(equivalent=True)

        # Line-by-line comparison
        exp_lines = norm_expected.split('\n')
        act_lines = norm_actual.split('\n')

        differences: List[Difference] = []

        # Check line count
        if len(exp_lines) != len(act_lines):
            differences.append(Difference(
                line_number=0,
                diff_type=DiffType.LINE_COUNT,
                expected=f"{len(exp_lines)} lines",
                actual=f"{len(act_lines)} lines",
                acceptable=False,
                message="Output line counts differ",
            ))

        # Compare each line
        max_lines = max(len(exp_lines), len(act_lines))
        for i in range(max_lines):
            exp_line = exp_lines[i] if i < len(exp_lines) else "<missing>"
            act_line = act_lines[i] if i < len(act_lines) else "<missing>"

            if exp_line == act_line:
                continue

            # Try to determine the type of difference
            diff = self._compare_lines(i + 1, exp_line, act_line)
            if diff is not None:
                differences.append(diff)

        # Determine overall result
        critical_count = sum(1 for d in differences if not d.acceptable)
        acceptable_count = sum(1 for d in differences if d.acceptable)
        equivalent = critical_count == 0

        return ComparisonResult(
            equivalent=equivalent,
            differences=differences,
            critical_count=critical_count,
            acceptable_count=acceptable_count,
        )

    def _compare_lines(
        self, line_num: int, expected: str, actual: str
    ) -> Optional[Difference]:
        """Compare two lines and determine the type of difference"""

        # Check if it's a float precision difference
        if self._is_float_difference(expected, actual):
            return Difference(
                line_number=line_num,
                diff_type=DiffType.FLOAT_PRECISION,
                expected=expected,
                actual=actual,
                acceptable=True,
                message="Float precision difference within tolerance",
            )

        # Check if it's just whitespace
        if expected.strip() == actual.strip():
            return Difference(
                line_number=line_num,
                diff_type=DiffType.WHITESPACE,
                expected=expected,
                actual=actual,
                acceptable=True,
                message="Whitespace difference only",
            )

        # Check if it's an ordering difference (for collections)
        if self.allow_unordered and self._is_ordering_difference(expected, actual):
            return Difference(
                line_number=line_num,
                diff_type=DiffType.ORDERING,
                expected=expected,
                actual=actual,
                acceptable=True,
                message="Collection ordering difference",
            )

        # Generic mismatch
        return Difference(
            line_number=line_num,
            diff_type=DiffType.EXACT_MISMATCH,
            expected=expected,
            actual=actual,
            acceptable=False,
            message="Lines differ",
        )

    def _is_float_difference(self, expected: str, actual: str) -> bool:
        """Check if difference is only in float precision"""
        # Extract all floats from both strings
        exp_floats = self.FLOAT_PATTERN.findall(expected)
        act_floats = self.FLOAT_PATTERN.findall(actual)

        if len(exp_floats) != len(act_floats):
            return False

        if not exp_floats:
            return False

        # Check non-float parts are identical
        exp_non_float = self.FLOAT_PATTERN.sub('<FLOAT>', expected)
        act_non_float = self.FLOAT_PATTERN.sub('<FLOAT>', actual)

        if exp_non_float != act_non_float:
            return False

        # Compare floats with epsilon
        for exp_f, act_f in zip(exp_floats, act_floats):
            try:
                exp_val = float(exp_f)
                act_val = float(act_f)
                if abs(exp_val - act_val) > self.float_epsilon:
                    # Check relative error for large numbers
                    if abs(exp_val) > 1:
                        rel_error = abs(exp_val - act_val) / abs(exp_val)
                        if rel_error > self.float_epsilon:
                            return False
                    else:
                        return False
            except ValueError:
                return False

        return True

    def _is_ordering_difference(self, expected: str, actual: str) -> bool:
        """Check if difference is only in collection ordering"""
        # Try parsing as a collection-like output
        # This is a heuristic for common patterns like:
        # [1, 2, 3] vs [3, 1, 2]
        # {a, b, c} vs {c, a, b}

        # Extract items from bracket/brace delimited lists
        exp_items = self._extract_collection_items(expected)
        act_items = self._extract_collection_items(actual)

        if exp_items is None or act_items is None:
            return False

        # Check if same items, different order
        return sorted(exp_items) == sorted(act_items)

    def _extract_collection_items(self, text: str) -> Optional[List[str]]:
        """Extract items from a collection-like string"""
        # Match patterns like [a, b, c] or {a, b, c}
        match = re.match(r'[\[{]\s*(.+?)\s*[\]}]', text.strip())
        if not match:
            return None

        content = match.group(1)
        items = [item.strip() for item in content.split(',')]
        return items


def generate_diff(expected: str, actual: str) -> str:
    """Generate a unified diff between expected and actual"""
    exp_lines = expected.splitlines(keepends=True)
    act_lines = actual.splitlines(keepends=True)

    diff = unified_diff(
        exp_lines,
        act_lines,
        fromfile='Tier 0 (expected)',
        tofile='Tier 3 (actual)',
        lineterm='',
    )

    return ''.join(diff)


def format_result(result: ComparisonResult, verbose: bool = False) -> str:
    """Format comparison result for display"""
    lines = []

    if result.equivalent:
        lines.append("Outputs are SEMANTICALLY EQUIVALENT")
        if result.acceptable_count > 0:
            lines.append(f"  ({result.acceptable_count} acceptable differences)")
    else:
        lines.append("Outputs DIFFER")
        lines.append(f"  Critical differences: {result.critical_count}")
        lines.append(f"  Acceptable differences: {result.acceptable_count}")

    if verbose and result.differences:
        lines.append("")
        lines.append("Differences:")
        for diff in result.differences:
            status = "OK" if diff.acceptable else "!!"
            lines.append(f"  [{status}] Line {diff.line_number}: {diff.diff_type.value}")
            lines.append(f"       Expected: {diff.expected[:60]}...")
            lines.append(f"       Actual:   {diff.actual[:60]}...")
            if diff.message:
                lines.append(f"       Note: {diff.message}")

    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(
        description="Semantic comparison of tier outputs",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("Usage:")[0],
    )

    parser.add_argument(
        "tier0_output",
        help="Path to Tier 0 output file",
    )
    parser.add_argument(
        "tier3_output",
        help="Path to Tier 3 output file",
    )
    parser.add_argument(
        "--float-epsilon",
        type=float,
        default=1e-10,
        help="Float comparison tolerance (default: 1e-10)",
    )
    parser.add_argument(
        "--allow-unordered",
        action="store_true",
        help="Allow unordered comparison for collections",
    )
    parser.add_argument(
        "--normalize-ws",
        action="store_true",
        help="Normalize whitespace",
    )
    parser.add_argument(
        "--strip-ansi",
        action="store_true",
        default=True,
        help="Strip ANSI color codes (default: true)",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Show detailed comparison",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Output results as JSON",
    )
    parser.add_argument(
        "--diff",
        action="store_true",
        help="Show unified diff on mismatch",
    )

    args = parser.parse_args()

    # Read input files
    try:
        tier0_path = Path(args.tier0_output)
        tier3_path = Path(args.tier3_output)

        tier0_content = tier0_path.read_text()
        tier3_content = tier3_path.read_text()
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(2)
    except Exception as e:
        print(f"Error reading files: {e}", file=sys.stderr)
        sys.exit(2)

    # Create normalizer
    normalizer = OutputNormalizer(
        strip_ansi=args.strip_ansi,
        normalize_whitespace=args.normalize_ws,
    )

    # Create comparator
    comparator = SemanticComparator(
        float_epsilon=args.float_epsilon,
        allow_unordered=args.allow_unordered,
        normalizer=normalizer,
    )

    # Compare
    result = comparator.compare(tier0_content, tier3_content)

    # Output results
    if args.json:
        print(json.dumps(result.to_dict(), indent=2))
    else:
        print(format_result(result, verbose=args.verbose))

        if args.diff and not result.equivalent:
            print("\n" + "=" * 60)
            print("Unified Diff:")
            print("=" * 60)
            print(generate_diff(tier0_content, tier3_content))

    # Exit code
    sys.exit(0 if result.equivalent else 1)


if __name__ == "__main__":
    main()
