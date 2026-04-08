#!/usr/bin/env python3
"""
Fix excessive @requires directives in VCS test files.

Rules:
- parse-pass, parse-fail: require only 'parser' (or nothing)
- typecheck-pass, typecheck-fail: require only 'parser', 'type-checker'
- verify-pass, verify-fail: require 'parser', 'type-checker', 'verification'
- run: keep runtime requirements

This script removes runtime features from tests that don't actually need them.
"""

import os
import re
import sys
from pathlib import Path

# Map test type to allowed requires (non-runtime features)
TEST_TYPE_REQUIREMENTS = {
    'parse-pass': set(),  # No runtime requirements needed
    'parse-fail': set(),
    'parse': set(),
    'lex-pass': set(),
    'lex-fail': set(),
    'typecheck-pass': set(),  # Type checker is always available
    'typecheck-fail': set(),
    'typecheck-mixed': set(),
    'compile-only': set(),
    'verify-pass': {'verification'},  # Only verification feature needed
    'verify-fail': {'verification'},
    # For 'run', 'unit', 'run-pass' and other execution types, keep their requirements
}

# Runtime features that are NOT needed for parsing
RUNTIME_FEATURES = {
    'cbgr-runtime', 'simd', 'async-runtime', 'ffi', 'heap',
    'context-system', 'bounds-checking', 'gpu', 'autodiff',
    'derive-macros', 'overflowing-math', 'advanced-types',
    'meta-programming', 'threading', 'jit', 'aot', 'verification',
    'incremental-parser', 'collections-std', 'pattern-matching-advanced'
}

def get_test_type(content):
    """Extract @test directive value."""
    match = re.search(r'@test:\s*(\S+)', content)
    return match.group(1) if match else None

def get_requires(content):
    """Extract @requires directive values."""
    match = re.search(r'@requires:\s*(.+?)(?:\n|$)', content)
    if match:
        return [r.strip() for r in match.group(1).split(',')]
    return []

def fix_requires_for_file(filepath):
    """Fix the @requires directive for a single file."""
    with open(filepath, 'r') as f:
        content = f.read()

    test_type = get_test_type(content)
    if not test_type:
        return False, "No @test directive found"

    # Only fix parse/lex tests, keep others as-is
    if test_type not in TEST_TYPE_REQUIREMENTS:
        return False, f"Test type '{test_type}' - keeping requires as-is"

    current_requires = get_requires(content)
    if not current_requires:
        return False, "No @requires directive"

    allowed = TEST_TYPE_REQUIREMENTS[test_type]

    # Filter out runtime features that aren't needed
    new_requires = [r for r in current_requires if r not in RUNTIME_FEATURES or r in allowed]

    if set(new_requires) == set(current_requires):
        return False, "No changes needed"

    # Remove the @requires line entirely if nothing left
    if not new_requires:
        new_content = re.sub(r'// @requires:.*\n', '', content)
    else:
        new_requires_str = ', '.join(new_requires)
        new_content = re.sub(
            r'// @requires:.*',
            f'// @requires: {new_requires_str}',
            content
        )

    with open(filepath, 'w') as f:
        f.write(new_content)

    return True, f"Fixed: {current_requires} -> {new_requires or '(none)'}"

def main():
    vcs_dir = Path(__file__).parent.parent / 'specs'

    if not vcs_dir.exists():
        print(f"Error: {vcs_dir} does not exist")
        sys.exit(1)

    fixed_count = 0
    skipped_count = 0

    # Find all .vr files
    for vr_file in sorted(vcs_dir.rglob('*.vr')):
        fixed, message = fix_requires_for_file(vr_file)
        if fixed:
            print(f"[FIXED] {vr_file.relative_to(vcs_dir)}: {message}")
            fixed_count += 1
        else:
            skipped_count += 1

    print(f"\n{'='*60}")
    print(f"Fixed: {fixed_count} files")
    print(f"Skipped: {skipped_count} files")
    print(f"{'='*60}")

if __name__ == '__main__':
    main()
