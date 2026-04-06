#!/usr/bin/env python3
"""
Fix incorrectly converted associated type declarations inside protocols.
Converts `type X<_> is ();` back to `type X<_>;` when inside a protocol.
"""
import re
import sys
from pathlib import Path

def fix_file(path: Path, dry_run: bool = True) -> tuple[bool, str]:
    """Fix a single file. Returns (changed, content)."""
    content = path.read_text()
    original = content

    lines = content.split('\n')
    result = []
    in_protocol = False
    brace_depth = 0

    for line in lines:
        # Track if we're inside a protocol
        if 'is protocol' in line:
            in_protocol = True
            brace_depth = 0

        if in_protocol:
            brace_depth += line.count('{') - line.count('}')
            if brace_depth <= 0 and '}' in line:
                in_protocol = False

        # Fix only inside protocols: type X is (); -> type X;
        # Also handles: type X<...> is (); -> type X<...>;
        if in_protocol:
            # Match: type Name is (); or type Name<...> is ();
            pattern = r'^(\s*type\s+\w+(?:<[^>]+>)?)\s+is\s+\(\);'
            match = re.match(pattern, line)
            if match:
                decl = match.group(1)
                line = f'{decl};'

        result.append(line)

    content = '\n'.join(result)
    changed = content != original

    if changed and not dry_run:
        path.write_text(content)

    return changed, content

def main():
    import argparse
    parser = argparse.ArgumentParser(description='Fix protocol associated types')
    parser.add_argument('path', help='File or directory to process')
    parser.add_argument('--dry-run', action='store_true', help='Show changes without writing')
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
        except Exception as e:
            print(f"Error processing {f}: {e}", file=sys.stderr)

    print(f"\n{'Would change' if args.dry_run else 'Changed'} {changed_count} files")

if __name__ == '__main__':
    main()
