#!/usr/bin/env python3
"""
VCS Syntax Compliance Checker

Verifies that .vr files use correct Verum syntax, not Rust syntax.

Checks for:
1. Turbofish syntax `::<` - should use `<` instead
2. Double colon paths `::` - should use `.` for paths
3. Rust-style attributes `#[...]` - should use `@` prefix
4. Rust keywords (struct, enum, impl, trait) - should use Verum equivalents
5. Rust macros with `!` suffix - should use `@` prefix or built-in functions

Usage:
    python check_syntax_compliance.py [directory]

    If no directory specified, checks vcs/specs/
"""

import os
import re
import sys
from pathlib import Path
from typing import List, Tuple, NamedTuple

class Issue(NamedTuple):
    file: str
    line: int
    column: int
    pattern: str
    message: str
    suggestion: str

# Patterns to check (pattern, message, suggestion)
PATTERNS = [
    # Turbofish syntax
    (r'::<',
     "Rust turbofish syntax found",
     "Use `func<T>()` instead of `func::<T>()`"),

    # Rust attributes
    (r'#\[(?!cfg\b)',  # Allow #[cfg...] in comments for now
     "Rust attribute syntax found",
     "Use `@attribute(...)` instead of `#[attribute(...)]`"),

    # Rust struct keyword (not in comments)
    (r'(?<!//)(?<!/// )\bstruct\s+\w+\s*\{',
     "Rust `struct` keyword found",
     "Use `type Name is { ... };` instead"),

    # Rust enum keyword
    (r'(?<!//)(?<!/// )\benum\s+\w+\s*\{',
     "Rust `enum` keyword found",
     "Use `type Name is A | B;` instead"),

    # Rust trait keyword
    (r'(?<!//)(?<!/// )\btrait\s+\w+',
     "Rust `trait` keyword found",
     "Use `type Name is protocol { ... };` instead"),

    # Rust impl keyword (without 'implement')
    (r'(?<!//)(?<!/// )\bimpl\s+(?!ement)',
     "Rust `impl` keyword found",
     "Use `implement` instead of `impl`"),

    # Rust-style macros (but allow assert, print, panic which are built-ins)
    (r'(?<!@)\b(?!assert|print|panic|format|unreachable)\w+!\s*\(',
     "Rust-style macro invocation found",
     "Use `@macro(...)` or built-in function syntax"),

    # Rust Box::new
    (r'\bBox::new\b',
     "Rust `Box::new` found",
     "Use `Heap(...)` instead"),

    # Rust Vec<T>
    (r'\bVec<',
     "Rust `Vec<T>` found",
     "Use `List<T>` instead"),

    # Rust String (as type, not in strings)
    (r':\s*String\b',
     "Rust `String` type found",
     "Use `Text` instead"),

    # Rust HashMap
    (r'\bHashMap<',
     "Rust `HashMap` found",
     "Use `Map<K, V>` instead"),

    # Rust HashSet
    (r'\bHashSet<',
     "Rust `HashSet` found",
     "Use `Set<T>` instead"),

    # Rust Option (as type)
    (r':\s*Option<',
     "Rust `Option<T>` type found",
     "Use `Maybe<T>` instead"),

    # Rust println!/eprintln!
    (r'\bprintln!\s*\(|\beprintln!\s*\(',
     "Rust `println!` macro found",
     "Use `print(...)` or `eprint(...)` built-in functions"),

    # Rust format!
    (r'\bformat!\s*\(',
     "Rust `format!` macro found",
     "Use `f\"...\"` format string literal"),

    # Rust panic!
    (r'\bpanic!\s*\(',
     "Rust `panic!` macro found",
     "Use `panic(...)` built-in function"),

    # Rust assert!
    (r'\bassert!\s*\(',
     "Rust `assert!` macro found",
     "Use `assert(...)` built-in function"),

    # Rust assert_eq!
    (r'\bassert_eq!\s*\(',
     "Rust `assert_eq!` macro found",
     "Use `assert_eq(...)` built-in function"),
]

def check_file(filepath: Path) -> List[Issue]:
    """Check a single file for syntax compliance issues."""
    issues = []

    try:
        content = filepath.read_text()
    except Exception as e:
        print(f"Warning: Could not read {filepath}: {e}", file=sys.stderr)
        return issues

    lines = content.split('\n')

    for line_num, line in enumerate(lines, 1):
        # Skip pure comment lines for some checks
        stripped = line.strip()
        is_comment = stripped.startswith('//')

        for pattern, message, suggestion in PATTERNS:
            # Skip some patterns in comments
            if is_comment and pattern in [r'::<']:
                # Still flag turbofish even in comments as documentation
                pass

            for match in re.finditer(pattern, line):
                issues.append(Issue(
                    file=str(filepath),
                    line=line_num,
                    column=match.start() + 1,
                    pattern=match.group(),
                    message=message,
                    suggestion=suggestion
                ))

    return issues

def check_directory(directory: Path) -> List[Issue]:
    """Check all .vr files in a directory recursively."""
    all_issues = []

    for vr_file in directory.rglob("*.vr"):
        issues = check_file(vr_file)
        all_issues.extend(issues)

    return all_issues

def main():
    if len(sys.argv) > 1:
        directory = Path(sys.argv[1])
    else:
        # Default to vcs/specs from project root
        script_dir = Path(__file__).parent
        directory = script_dir.parent / "specs"

    if not directory.exists():
        print(f"Error: Directory {directory} does not exist", file=sys.stderr)
        sys.exit(1)

    print(f"Checking syntax compliance in: {directory}")
    print("-" * 60)

    issues = check_directory(directory)

    if not issues:
        print("No syntax compliance issues found!")
        sys.exit(0)

    # Group by file
    by_file = {}
    for issue in issues:
        if issue.file not in by_file:
            by_file[issue.file] = []
        by_file[issue.file].append(issue)

    # Print issues
    total = len(issues)
    files_with_issues = len(by_file)

    for filepath, file_issues in sorted(by_file.items()):
        rel_path = Path(filepath).relative_to(directory.parent) if filepath.startswith(str(directory.parent)) else filepath
        print(f"\n{rel_path}:")
        for issue in file_issues:
            print(f"  Line {issue.line}:{issue.column}: {issue.message}")
            print(f"    Found: `{issue.pattern}`")
            print(f"    Fix: {issue.suggestion}")

    print("\n" + "-" * 60)
    print(f"Total: {total} issues in {files_with_issues} files")

    # Return non-zero if issues found
    sys.exit(1 if issues else 0)

if __name__ == "__main__":
    main()
