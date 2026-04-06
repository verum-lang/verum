#!/usr/bin/env python3
"""
Fix Rust-style syntax in VCS test files to use correct Verum syntax.
"""
import re
import sys
from pathlib import Path

def fix_type_alias(content: str) -> str:
    """Fix type X = Y; to type X is Y;"""
    # Match: type Name<...> = Type;
    # But NOT: type Name<...> is ... (already correct)
    # Handle nested generics like <F<_>, A>
    pattern = r'^(\s*type\s+\w+(?:<[^;]+?>)?)\s*=\s*([^;]+);'
    replacement = r'\1 is \2;'
    return re.sub(pattern, replacement, content, flags=re.MULTILINE)

def fix_forward_type_decl(content: str) -> str:
    """Fix type X; to type X is ();

    IMPORTANT: Do NOT modify:
    - Associated type declarations inside protocols (type F<_>;)
    - Associated type definitions inside implement blocks (type F<T> is List<T>;)
    """
    lines = content.split('\n')
    result = []
    in_protocol = False
    in_implement = False
    brace_depth = 0

    for line in lines:
        stripped = line.strip()

        # Track if we're inside a protocol
        if 'is protocol' in line:
            in_protocol = True
            in_implement = False
            brace_depth = 0

        # Track if we're inside an implement block
        if line.strip().startswith('implement ') and '{' in line:
            in_implement = True
            in_protocol = False
            brace_depth = 0

        if in_protocol or in_implement:
            brace_depth += line.count('{') - line.count('}')
            if brace_depth <= 0 and '}' in line:
                in_protocol = False
                in_implement = False

        # Only apply fix if NOT inside a protocol or implement block
        if not in_protocol and not in_implement:
            # Match: type Name; or type Name<...>;
            # But NOT: type Name is ... (already has definition)
            pattern = r'^(\s*type\s+\w+(?:<[^;]+?>)?)\s*;$'
            match = re.match(pattern, line)
            if match:
                decl = match.group(1)
                line = f'{decl} is ();'

        result.append(line)

    return '\n'.join(result)

def fix_struct_syntax(content: str) -> str:
    """Fix struct Name { ... } to type Name is { ... };"""
    # This is more complex, skip for now
    return content

def fix_enum_syntax(content: str) -> str:
    """Fix enum Name { A, B } to type Name is A | B;"""
    # This is more complex, skip for now
    return content

def fix_trait_syntax(content: str) -> str:
    """Fix trait Name { ... } to type Name is protocol { ... };"""
    # This is more complex, skip for now
    return content

def fix_impl_syntax(content: str) -> str:
    """Fix impl Name { ... } to implement Name { ... }"""
    # Simple case: impl X { -> implement X {
    content = re.sub(r'\bimpl\s+(\w+)\s*\{', r'implement \1 {', content)
    # impl X for Y { -> implement X for Y {
    content = re.sub(r'\bimpl\s+(\w+)\s+for\s+(\w+)\s*\{', r'implement \1 for \2 {', content)
    return content

def fix_vec_to_list(content: str) -> str:
    """Fix Vec<T> to List<T>"""
    return re.sub(r'\bVec<', r'List<', content)

def fix_string_to_text(content: str) -> str:
    """Fix String to Text"""
    return re.sub(r'\bString\b', r'Text', content)

def fix_box_new(content: str) -> str:
    """Fix Box::new(x) to Heap(x)"""
    return re.sub(r'Box::new\(', r'Heap(', content)

def fix_println(content: str) -> str:
    """Fix println!(...) to print(...)"""
    return re.sub(r'println!\(', r'print(', content)

def fix_format(content: str) -> str:
    """Fix format!("...", x) to f"..." - simplified"""
    # This is complex, skip for now
    return content

def fix_panic(content: str) -> str:
    """Fix panic!(...) to panic(...)"""
    return re.sub(r'panic!\(', r'panic(', content)

def fix_assert_macro(content: str) -> str:
    """Fix assert!(...) to assert(...)"""
    return re.sub(r'assert!\(', r'assert(', content)

def fix_derive_attr(content: str) -> str:
    """Fix #[derive(...)] to @derive(...)"""
    return re.sub(r'#\[derive\(([^)]+)\)\]', r'@derive(\1)', content)

def fix_repr_attr(content: str) -> str:
    """Fix #[repr(...)] to @repr(...)"""
    return re.sub(r'#\[repr\(([^)]+)\)\]', r'@repr(\1)', content)

def fix_turbofish(content: str) -> str:
    """Fix Rust turbofish syntax ::<...> to Verum <...>"""
    # Match: identifier::<type_args>
    # Replace with: identifier<type_args>
    return re.sub(r'(\w+)::<', r'\1<', content)

def fix_file(path: Path, dry_run: bool = True) -> tuple[bool, str]:
    """Fix a single file. Returns (changed, content)."""
    content = path.read_text()
    original = content

    # Apply all fixes
    content = fix_type_alias(content)
    content = fix_forward_type_decl(content)
    content = fix_impl_syntax(content)
    content = fix_vec_to_list(content)
    content = fix_string_to_text(content)
    content = fix_box_new(content)
    content = fix_println(content)
    content = fix_panic(content)
    content = fix_assert_macro(content)
    content = fix_derive_attr(content)
    content = fix_repr_attr(content)
    content = fix_turbofish(content)

    changed = content != original

    if changed and not dry_run:
        path.write_text(content)

    return changed, content

def main():
    import argparse
    parser = argparse.ArgumentParser(description='Fix Rust syntax in VCS test files')
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
                    for i, line in enumerate(content.split('\n')[:20], 1):
                        print(f"    {i}: {line}")
        except Exception as e:
            print(f"Error processing {f}: {e}", file=sys.stderr)

    print(f"\n{'Would change' if args.dry_run else 'Changed'} {changed_count} files")

if __name__ == '__main__':
    main()
