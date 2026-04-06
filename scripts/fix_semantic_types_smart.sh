#!/usr/bin/env bash
# Spec: CLAUDE.md v6.0-BALANCED Section 3.1 - Semantic Type Enforcement
# Smart semantic type migration with import management

set -euo pipefail

AXIOM_ROOT="/Users/taaliman/projects/luxquant/axiom"
cd "$AXIOM_ROOT"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}===== Smart Semantic Type Migration =====${NC}"

# Step 1: Fix type usages (Vec -> List, HashMap -> Map, etc.)
echo -e "${YELLOW}Step 1: Replacing type usages...${NC}"

find crates -name "*.rs" -type f \
    -not -path "*/chumsky/*" \
    -not -path "*/target/*" \
    -not -path "*/verum_core/src/lib.rs" \
    -not -path "*/verum_core/src/conversions.rs" \
    -not -path "*/verum_core/tests/conversions_tests.rs" \
    -exec sed -i '' \
        -e 's/\bVec</List</g' \
        -e 's/\bHashMap</Map</g' \
        -e 's/\bHashSet</Set</g' \
        -e 's/\bBTreeMap</OrderedMap</g' \
        -e 's/\bBTreeSet</OrderedSet</g' \
        {} \;

echo -e "${GREEN}✓ Type usages replaced${NC}"

# Step 2: Fix import statements
echo -e "${YELLOW}Step 2: Fixing import statements...${NC}"

# Remove direct std collection imports
find crates -name "*.rs" -type f \
    -not -path "*/chumsky/*" \
    -not -path "*/target/*" \
    -not -path "*/verum_core/src/lib.rs" \
    -not -path "*/verum_core/src/conversions.rs" \
    -not -path "*/verum_core/tests/conversions_tests.rs" \
    -exec sed -i '' \
        -e '/^use std::collections::HashMap;$/d' \
        -e '/^use std::collections::HashSet;$/d' \
        -e '/^use std::collections::BTreeMap;$/d' \
        -e '/^use std::collections::BTreeSet;$/d' \
        -e '/^use std::vec::Vec;$/d' \
        -e '/^use std::string::String;$/d' \
        {} \;

# Handle multi-import lines more carefully
find crates -name "*.rs" -type f \
    -not -path "*/chumsky/*" \
    -not -path "*/target/*" \
    -not -path "*/verum_core/src/lib.rs" \
    -not -path "*/verum_core/src/conversions.rs" \
    -not -path "*/verum_core/tests/conversions_tests.rs" \
    -exec sed -i '' \
        -e 's/HashMap as StdHashMap/Map/g' \
        -e 's/HashSet as StdHashSet/Set/g' \
        -e 's/Vec as StdVec/List/g' \
        {} \;

echo -e "${GREEN}✓ Import statements cleaned${NC}"

# Step 3: Ensure verum_core imports exist where needed
echo -e "${YELLOW}Step 3: Adding missing verum_core imports...${NC}"

# This requires checking each file to see if it uses List/Map/Set but doesn't import them
# We'll do this with a Python script for better control

cat > /tmp/add_imports.py << 'EOF'
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
    with open(file_path, 'r') as f:
        content = f.read()

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
        # Add new import after the first use statement
        new_import = ', '.join(sorted(needed_types))
        import_line = f'use verum_core::{{{new_import}}};\n'

        # Find first 'use' statement
        use_match = re.search(r'^use ', content, re.MULTILINE)
        if use_match:
            # Insert after the module-level comment if present
            lines = content.split('\n')
            insert_idx = 0
            for i, line in enumerate(lines):
                if line.startswith('use '):
                    insert_idx = i
                    break

            lines.insert(insert_idx, import_line.rstrip())
            content = '\n'.join(lines)
        else:
            # No use statements, add after comments
            content = import_line + content

    if content != original_content:
        with open(file_path, 'w') as f:
            f.write(content)
        return True

    return False

# Process files
files_modified = 0
for file_path in sys.argv[1:]:
    if fix_imports(file_path):
        files_modified += 1
        print(f"✓ {file_path}")

print(f"\nModified {files_modified} files")
EOF

# Get list of files to process
find crates -name "*.rs" -type f \
    -not -path "*/chumsky/*" \
    -not -path "*/target/*" \
    -not -path "*/verum_core/src/lib.rs" \
    -not -path "*/verum_core/src/conversions.rs" \
    -not -path "*/verum_core/tests/conversions_tests.rs" \
    > /tmp/files_to_process.txt

# Run Python script
python3 /tmp/add_imports.py $(cat /tmp/files_to_process.txt)

echo -e "${GREEN}✓ Imports added where needed${NC}"

# Cleanup
rm -f /tmp/add_imports.py /tmp/files_to_process.txt

echo ""
echo -e "${GREEN}===== Migration Complete =====${NC}"
echo ""
echo -e "${YELLOW}Next steps:${NC}"
echo "1. cargo fmt --all"
echo "2. cargo build --workspace"
echo "3. cargo test --workspace"
echo "4. cargo clippy --workspace -- -D warnings"

