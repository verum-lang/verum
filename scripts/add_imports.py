#!/usr/bin/env python3
"""
Add missing verum_core imports to Rust files
Spec: CLAUDE.md v6.0-BALANCED Section 3.1
"""

import re
import sys
from pathlib import Path

def needs_import(content, type_name):
    """Check if type is used but not imported"""
    # Check if type is used
    type_pattern = rf'\b{type_name}<'
    if not re.search(type_pattern, content):
        return False

    # Check if already imported from verum_core
    import_pattern = rf'use verum_core::.*\b{type_name}\b'
    if re.search(import_pattern, content):
        return False

    return True

def fix_imports(file_path):
    """Fix imports in a single file"""
    try:
        # Try UTF-8 first
        with open(file_path, 'r', encoding='utf-8') as f:
            content = f.read()
    except UnicodeDecodeError:
        # Try latin-1 as fallback
        try:
            with open(file_path, 'r', encoding='latin-1') as f:
                content = f.read()
        except Exception as e:
            print(f"✗ {file_path}: {e}", file=sys.stderr)
            return False

    original_content = content
    needed_types = []

    # Check which types are needed
    for type_name in ['List', 'Map', 'Set', 'OrderedMap', 'OrderedSet', 'Text', 'Heap', 'Maybe']:
        if needs_import(content, type_name):
            needed_types.append(type_name)

    if not needed_types:
        return False  # No changes needed

    # Find existing verum_core import
    verum_import_match = re.search(r'use verum_core::\{([^}]+)\};', content)

    if verum_import_match:
        # Extend existing import
        existing_imports = verum_import_match.group(1)
        existing_set = set(t.strip() for t in existing_imports.split(','))
        all_imports = existing_set | set(needed_types)
        new_import = ', '.join(sorted(all_imports))
        content = content.replace(
            f'use verum_core::{{{existing_imports}}};',
            f'use verum_core::{{{new_import}}};'
        )
    else:
        # Check for simple single import
        simple_import = re.search(r'use verum_core::(\w+);', content)
        if simple_import:
            existing_type = simple_import.group(1)
            all_imports = set([existing_type]) | set(needed_types)
            new_import = ', '.join(sorted(all_imports))
            content = content.replace(
                f'use verum_core::{existing_type};',
                f'use verum_core::{{{new_import}}};'
            )
        else:
            # Add new import after the first use statement
            new_import = ', '.join(sorted(needed_types))
            import_line = f'use verum_core::{{{new_import}}};\n'

            # Find first 'use' statement
            use_match = re.search(r'^use ', content, re.MULTILINE)
            if use_match:
                # Insert after the first use line
                lines = content.split('\n')
                insert_idx = 0
                for i, line in enumerate(lines):
                    if line.startswith('use '):
                        insert_idx = i
                        break

                lines.insert(insert_idx, import_line.rstrip())
                content = '\n'.join(lines)
            else:
                # No use statements, add at the top after initial comments
                lines = content.split('\n')
                insert_idx = 0
                for i, line in enumerate(lines):
                    if line and not line.startswith('//') and not line.startswith('/*'):
                        insert_idx = i
                        break
                lines.insert(insert_idx, import_line.rstrip())
                content = '\n'.join(lines)

    if content != original_content:
        try:
            with open(file_path, 'w', encoding='utf-8') as f:
                f.write(content)
            return True
        except Exception as e:
            print(f"✗ {file_path}: {e}", file=sys.stderr)
            return False

    return False

def main():
    """Process all files passed as arguments"""
    files_modified = 0
    files_processed = 0

    for file_path in sys.argv[1:]:
        files_processed += 1
        if fix_imports(file_path):
            files_modified += 1
            print(f"✓ {file_path}")

    print(f"\nProcessed {files_processed} files, modified {files_modified}")
    return 0

if __name__ == '__main__':
    sys.exit(main())
