#!/usr/bin/env python3
"""
Fix FFI syntax in VCS test files to match the Verum parser.

Key fixes:
1. FFI function signatures need semicolons: fn foo() -> T; not fn foo() -> T
2. Implies operator: ==> should be =>
3. spec fn -> @spec fn or just regular fn
4. ReturnCode with comparison: ReturnCode(result < 0) -> ReturnCode(< 0)
"""
import re
import sys
from pathlib import Path


def fix_ffi_function_syntax(content: str) -> str:
    """
    Fix FFI function declarations to have semicolons.

    Pattern: @extern("C")\n fn name(...) -> Type\n    ensures/requires/memory_effects
    Should be: @extern("C")\n fn name(...) -> Type;\n ensures...;
    """
    lines = content.split('\n')
    result = []
    in_ffi = False
    brace_depth = 0

    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()

        # Track if we're inside an ffi block
        if stripped.startswith('ffi ') and '{' in line:
            in_ffi = True
            brace_depth = 1
            result.append(line)
            i += 1
            continue

        if in_ffi:
            brace_depth += line.count('{') - line.count('}')
            if brace_depth <= 0:
                in_ffi = False

        # Inside FFI block, look for function declarations without semicolons
        if in_ffi and stripped.startswith('fn ') and '->' in stripped:
            # Check if line ends with type (no semicolon)
            if not stripped.endswith(';') and not stripped.endswith('{'):
                # Look ahead to see if next line is a contract clause
                if i + 1 < len(lines):
                    next_stripped = lines[i + 1].strip()
                    if (next_stripped.startswith('ensures') or
                        next_stripped.startswith('requires') or
                        next_stripped.startswith('memory_effects') or
                        next_stripped.startswith('thread_safe') or
                        next_stripped.startswith('errors_via') or
                        next_stripped.startswith('@ownership')):
                        # Add semicolon to function line
                        line = line.rstrip() + ';'

        # Also handle fn declarations that span multiple lines
        if in_ffi and stripped.startswith('fn ') and not stripped.endswith(';') and not stripped.endswith('{') and not stripped.endswith(','):
            # Look for closing paren followed by return type
            if ')' in stripped and '->' not in stripped:
                # Multi-line function, need to find the end
                pass  # Handle later if needed

        result.append(line)
        i += 1

    return '\n'.join(result)


def fix_implies_operator(content: str) -> str:
    """Fix ==> to => for implies operator."""
    return content.replace('==>', '=>')


def fix_spec_fn(content: str) -> str:
    """Fix spec fn declarations."""
    # spec fn name(...) -> Type; => @spec fn name(...) -> Type;
    # Or just make them ghost functions
    content = re.sub(r'^(\s*)spec\s+fn\s+', r'\1@spec fn ', content, flags=re.MULTILINE)
    return content


def fix_return_code_syntax(content: str) -> str:
    """
    Fix ReturnCode syntax.
    ReturnCode(result < 0) with Errno -> errors_via = ReturnCode(< 0) with Errno;
    The 'result' is implicit in ReturnCode patterns.
    """
    # This is complex - the spec says ReturnCode(pattern) where pattern can be:
    # - A value: ReturnCode(SQLITE_OK) means success when result == SQLITE_OK
    # - A comparison: ReturnCode(!= Z_OK) means error when result != Z_OK
    # The test uses result < 0, which should probably be < 0
    content = re.sub(r'ReturnCode\(result\s*([<>=!]+)\s*(\d+)\)', r'ReturnCode(\1 \2)', content)
    return content


def fix_contract_semicolons(content: str) -> str:
    """
    Ensure contract clauses end with semicolons.
    """
    lines = content.split('\n')
    result = []

    for line in lines:
        stripped = line.strip()

        # Contract clauses that should end with semicolons
        contract_keywords = ['ensures ', 'requires ', 'memory_effects =', 'thread_safe =', 'errors_via =']

        for kw in contract_keywords:
            if stripped.startswith(kw) and not stripped.endswith(';') and not stripped.endswith('{'):
                # Check it's not a multi-line expression (ending with operators)
                if not stripped.endswith('&&') and not stripped.endswith('||') and not stripped.endswith(',') and not stripped.endswith('('):
                    line = line.rstrip() + ';'
                    break

        result.append(line)

    return '\n'.join(result)


def fix_inline_ffi_contracts(content: str) -> str:
    """
    Fix inline FFI contracts that appear on same/next line as function.

    Pattern:
        fn malloc(size: c_size) -> *mut c_void
            ensures result == null || is_valid_heap_ptr(result, size)
            memory_effects = Allocates;

    Should be:
        fn malloc(size: c_size) -> *mut c_void;
        ensures result == null || is_valid_heap_ptr(result, size);
        memory_effects = Allocates;
    """
    lines = content.split('\n')
    result = []
    i = 0

    while i < len(lines):
        line = lines[i]
        stripped = line.strip()

        # Check for @extern followed by fn declaration
        if stripped.startswith('@extern'):
            result.append(line)
            i += 1

            # Look for fn declaration
            if i < len(lines):
                fn_line = lines[i]
                fn_stripped = fn_line.strip()

                if fn_stripped.startswith('fn '):
                    # Collect the full function signature (may span multiple lines)
                    fn_parts = [fn_line]

                    # Check if it spans multiple lines (doesn't have closing paren and return type)
                    while i + 1 < len(lines) and not (')' in fn_stripped and ('->' in fn_stripped or fn_stripped.endswith(';'))):
                        i += 1
                        fn_parts.append(lines[i])
                        fn_stripped = lines[i].strip()

                    # Now fn_parts contains the complete function signature
                    full_fn = '\n'.join(fn_parts)

                    # Check if it ends with semicolon
                    if not full_fn.rstrip().endswith(';'):
                        # Add semicolon
                        fn_parts[-1] = fn_parts[-1].rstrip() + ';'

                    result.extend(fn_parts)
                    i += 1

                    # Process contract clauses that follow
                    while i < len(lines):
                        contract_line = lines[i]
                        contract_stripped = contract_line.strip()

                        # Check if this is a contract clause
                        if (contract_stripped.startswith('ensures') or
                            contract_stripped.startswith('requires') or
                            contract_stripped.startswith('memory_effects') or
                            contract_stripped.startswith('thread_safe') or
                            contract_stripped.startswith('errors_via') or
                            contract_stripped.startswith('@ownership')):

                            # Collect multi-line clause
                            clause_parts = [contract_line]

                            # Check if clause continues (ends with &&, ||, etc.)
                            while (contract_stripped.endswith('&&') or
                                   contract_stripped.endswith('||') or
                                   contract_stripped.endswith('(') or
                                   contract_stripped.endswith(',')) and i + 1 < len(lines):
                                i += 1
                                clause_parts.append(lines[i])
                                contract_stripped = lines[i].strip()

                            # Ensure ends with semicolon
                            full_clause = '\n'.join(clause_parts)
                            if not full_clause.rstrip().endswith(';'):
                                clause_parts[-1] = clause_parts[-1].rstrip() + ';'

                            result.extend(clause_parts)
                            i += 1
                        else:
                            # Not a contract clause, stop
                            break
                    continue
                else:
                    # Not a function declaration after @extern
                    pass
            continue

        result.append(line)
        i += 1

    return '\n'.join(result)


def fix_file(path: Path, dry_run: bool = True) -> tuple[bool, str]:
    """Fix a single file. Returns (changed, content)."""
    content = path.read_text()
    original = content

    # Apply all fixes
    content = fix_implies_operator(content)
    content = fix_spec_fn(content)
    content = fix_return_code_syntax(content)
    content = fix_inline_ffi_contracts(content)
    content = fix_contract_semicolons(content)

    changed = content != original

    if changed and not dry_run:
        path.write_text(content)

    return changed, content


def main():
    import argparse
    parser = argparse.ArgumentParser(description='Fix FFI syntax in VCS test files')
    parser.add_argument('path', help='File or directory to process')
    parser.add_argument('--dry-run', action='store_true', help='Show changes without writing')
    parser.add_argument('--verbose', '-v', action='store_true', help='Show all changes')
    args = parser.parse_args()

    path = Path(args.path)

    if path.is_file():
        files = [path]
    else:
        files = list(path.rglob('*.vr'))

    changed_count = 0
    for f in files:
        try:
            changed, content = fix_file(f, dry_run=args.dry_run)
            if changed:
                changed_count += 1
                print(f"{'Would change' if args.dry_run else 'Changed'}: {f}")
                if args.verbose:
                    print(f"  New content preview:")
                    for i, line in enumerate(content.split('\n')[:30], 1):
                        print(f"    {i}: {line}")
        except Exception as e:
            print(f"Error processing {f}: {e}", file=sys.stderr)

    print(f"\n{'Would change' if args.dry_run else 'Changed'} {changed_count} files")


if __name__ == '__main__':
    main()
