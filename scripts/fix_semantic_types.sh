#!/usr/bin/env bash
# Spec: CLAUDE.md v6.0-BALANCED Section 3.1 - Semantic Type Enforcement
# Fix semantic type violations across all Verum crates

set -euo pipefail

AXIOM_ROOT="/Users/taaliman/projects/luxquant/axiom"
cd "$AXIOM_ROOT"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}===== Verum Semantic Type Migration =====${NC}"
echo -e "${YELLOW}Spec: CLAUDE.md v6.0-BALANCED Section 3.1${NC}"
echo ""

# Exclude these crates/files (they're allowed to use std types directly)
EXCLUDE_PATHS=(
    "crates/verum_core/src/lib.rs"
    "crates/verum_core/src/conversions.rs"
    "crates/chumsky"
    "target/"
    "experiments/"
    ".git/"
)

# Build exclusion pattern for find
EXCLUDE_PATTERN=""
for path in "${EXCLUDE_PATHS[@]}"; do
    EXCLUDE_PATTERN="$EXCLUDE_PATTERN -not -path '*/$path/*'"
done

echo -e "${YELLOW}Step 1: Finding files with semantic type violations...${NC}"

# Find all Rust source files excluding protected paths
find crates -name "*.rs" -type f \
    -not -path "*/chumsky/*" \
    -not -path "*/target/*" \
    -not -path "*/verum_core/src/lib.rs" \
    -not -path "*/verum_core/src/conversions.rs" \
    > /tmp/verum_files_to_fix.txt

TOTAL_FILES=$(wc -l < /tmp/verum_files_to_fix.txt)
echo -e "${GREEN}Found $TOTAL_FILES files to process${NC}"
echo ""

# Counter for changes
FILES_MODIFIED=0

echo -e "${YELLOW}Step 2: Applying semantic type replacements...${NC}"

while IFS= read -r file; do
    # Skip if file doesn't exist or isn't readable
    [[ ! -f "$file" ]] && continue

    # Create backup
    cp "$file" "${file}.bak"

    MODIFIED=0

    # Fix type alias usage in declarations and signatures
    # Vec<T> -> List<T>
    if sed -i.tmp 's/\bVec</List</g' "$file" 2>/dev/null; then
        if ! cmp -s "$file" "${file}.tmp"; then
            MODIFIED=1
        fi
        rm -f "${file}.tmp"
    fi

    # HashMap<K,V> -> Map<K,V>  (including spaced variants)
    if sed -i.tmp 's/\bHashMap\s*</Map</g' "$file" 2>/dev/null; then
        if ! cmp -s "$file" "${file}.tmp"; then
            MODIFIED=1
        fi
        rm -f "${file}.tmp"
    fi

    # HashSet<T> -> Set<T>
    if sed -i.tmp 's/\bHashSet</Set</g' "$file" 2>/dev/null; then
        if ! cmp -s "$file" "${file}.tmp"; then
            MODIFIED=1
        fi
        rm -f "${file}.tmp"
    fi

    # BTreeMap<K,V> -> OrderedMap<K,V>
    if sed -i.tmp 's/\bBTreeMap</OrderedMap</g' "$file" 2>/dev/null; then
        if ! cmp -s "$file" "${file}.tmp"; then
            MODIFIED=1
        fi
        rm -f "${file}.tmp"
    fi

    # BTreeSet<T> -> OrderedSet<T>
    if sed -i.tmp 's/\bBTreeSet</OrderedSet</g' "$file" 2>/dev/null; then
        if ! cmp -s "$file" "${file}.tmp"; then
            MODIFIED=1
        fi
        rm -f "${file}.tmp"
    fi

    # Fix imports - remove std collection imports and replace with verum_core imports
    # This is more complex and needs careful handling

    # Remove: use std::collections::{HashMap, ...}
    if grep -q "use std::collections::" "$file"; then
        # Extract what's being imported
        if grep -q "HashMap" "$file" && grep -q "use std::collections::" "$file"; then
            # Need to add verum_core::Map import if not present
            if ! grep -q "use verum_core::.*Map" "$file"; then
                # Add import after other verum_core imports or at the top
                if grep -q "use verum_core::" "$file"; then
                    # Extend existing verum_core import
                    sed -i.tmp 's/use verum_core::{/use verum_core::{Map, /' "$file"
                else
                    # Add new import after use statements
                    sed -i.tmp '/^use /a\
use verum_core::Map;
' "$file"
                fi
                MODIFIED=1
            fi
        fi

        # Remove the std::collections imports that are now covered
        sed -i.tmp '/^use std::collections::{.*HashMap.*};$/d' "$file"
        sed -i.tmp '/^use std::collections::{.*HashSet.*};$/d' "$file"
        sed -i.tmp '/^use std::collections::{.*BTreeMap.*};$/d' "$file"
        sed -i.tmp '/^use std::collections::{.*BTreeSet.*};$/d' "$file"
        sed -i.tmp '/^use std::collections::HashMap;$/d' "$file"
        sed -i.tmp '/^use std::collections::HashSet;$/d' "$file"
        sed -i.tmp '/^use std::collections::BTreeMap;$/d' "$file"
        sed -i.tmp '/^use std::collections::BTreeSet;$/d' "$file"

        rm -f "${file}.tmp"
        MODIFIED=1
    fi

    # Remove: use std::vec::Vec
    if grep -q "use std::vec::Vec" "$file"; then
        sed -i.tmp '/^use std::vec::Vec;$/d' "$file"
        rm -f "${file}.tmp"
        MODIFIED=1
    fi

    # Fix aliased imports: HashMap as StdHashMap -> remove
    if grep -q "HashMap as StdHashMap\|Vec as StdVec\|HashSet as StdHashSet" "$file"; then
        echo -e "${YELLOW}Warning: Found aliased std types in $file - manual review needed${NC}"
        MODIFIED=1
    fi

    if [[ $MODIFIED -eq 1 ]]; then
        # Verify the file still compiles syntactically (basic check)
        if ! rustfmt --check "$file" 2>/dev/null; then
            echo -e "${RED}Warning: Formatting issues in $file after changes${NC}"
        fi

        FILES_MODIFIED=$((FILES_MODIFIED + 1))
        echo -e "${GREEN}✓${NC} Fixed: $file"
    else
        # No changes, remove backup
        rm -f "${file}.bak"
    fi

done < /tmp/verum_files_to_fix.txt

echo ""
echo -e "${GREEN}===== Migration Complete =====${NC}"
echo -e "Files processed: ${TOTAL_FILES}"
echo -e "Files modified: ${FILES_MODIFIED}"
echo ""

if [[ $FILES_MODIFIED -gt 0 ]]; then
    echo -e "${YELLOW}Next steps:${NC}"
    echo "1. Review changes with: git diff"
    echo "2. Run tests: cargo test --workspace"
    echo "3. Run clippy: cargo clippy --workspace -- -D warnings"
    echo "4. If all passes, remove backups: find crates -name '*.bak' -delete"
    echo ""
fi

# Cleanup
rm -f /tmp/verum_files_to_fix.txt

echo -e "${GREEN}Done!${NC}"
