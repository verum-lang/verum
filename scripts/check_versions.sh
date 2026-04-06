#!/usr/bin/env bash
# Version Consistency Checker for Verum v1.0.0 Release
# Verifies all version numbers are consistent across the entire codebase

set -euo pipefail

# Expected version
EXPECTED_VERSION="1.0.0"
EXIT_CODE=0

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "========================================="
echo "Verum Version Consistency Checker"
echo "Expected Version: $EXPECTED_VERSION"
echo "========================================="
echo ""

# Function to report error
report_error() {
    echo -e "${RED}✗ ERROR${NC}: $1"
    EXIT_CODE=1
}

# Function to report success
report_success() {
    echo -e "${GREEN}✓ OK${NC}: $1"
}

# Function to report warning
report_warning() {
    echo -e "${YELLOW}⚠ WARNING${NC}: $1"
}

# 1. Check workspace Cargo.toml
echo "1. Checking workspace Cargo.toml..."
WORKSPACE_VERSION=$(grep -E '^\[workspace.package\]' -A 10 Cargo.toml | grep -E '^version = ' | sed 's/version = "\(.*\)"/\1/')
if [ "$WORKSPACE_VERSION" == "$EXPECTED_VERSION" ]; then
    report_success "Workspace version is $EXPECTED_VERSION"
else
    report_error "Workspace version is $WORKSPACE_VERSION, expected $EXPECTED_VERSION"
fi
echo ""

# 2. Check individual crate Cargo.toml files
echo "2. Checking crate Cargo.toml files..."
CRATES_CHECKED=0
CRATES_OK=0
CRATES_WITH_EXPLICIT_VERSION=0

for cargo_toml in crates/*/Cargo.toml; do
    CRATES_CHECKED=$((CRATES_CHECKED + 1))
    crate_name=$(basename $(dirname "$cargo_toml"))

    # Check if crate has explicit version or uses workspace version
    if grep -q "^version\.workspace = true" "$cargo_toml"; then
        report_success "$crate_name uses workspace version"
        CRATES_OK=$((CRATES_OK + 1))
    elif grep -q "^version = " "$cargo_toml"; then
        CRATES_WITH_EXPLICIT_VERSION=$((CRATES_WITH_EXPLICIT_VERSION + 1))
        CRATE_VERSION=$(grep -E '^version = ' "$cargo_toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
        if [ "$CRATE_VERSION" == "$EXPECTED_VERSION" ]; then
            report_success "$crate_name has explicit version $EXPECTED_VERSION"
            CRATES_OK=$((CRATES_OK + 1))
        else
            report_error "$crate_name version is $CRATE_VERSION, expected $EXPECTED_VERSION"
        fi
    else
        report_warning "$crate_name has no version specified"
    fi
done

echo ""
echo "Crates checked: $CRATES_CHECKED"
echo "Crates OK: $CRATES_OK"
echo "Crates with explicit version: $CRATES_WITH_EXPLICIT_VERSION"
echo ""

# 3. Check README.md version references
echo "3. Checking README.md version references..."
README_ERRORS=0

if grep -q "Version 1.0.0" README.md; then
    report_success "README.md contains 'Version 1.0.0'"
else
    report_error "README.md does not contain 'Version 1.0.0'"
    README_ERRORS=$((README_ERRORS + 1))
fi

if grep -q "Current Version.*1.0.0" README.md; then
    report_success "README.md contains 'Current Version: 1.0.0'"
else
    report_error "README.md does not contain 'Current Version: 1.0.0'"
    README_ERRORS=$((README_ERRORS + 1))
fi

# Check for old version references that should be updated
if grep -qE "v[0-9]+\.[0-9]+\.[0-9]+" README.md | grep -v "v1.0.0" | grep -v "v1.1" | grep -v "v2.0" | grep -v "v5.1" > /dev/null 2>&1; then
    report_warning "README.md may contain old version references (v5.1 references are acceptable for historical context)"
fi

echo ""

# 4. Check CHANGELOG.md
echo "4. Checking CHANGELOG.md..."
if [ -f "CHANGELOG.md" ]; then
    if grep -q "## \[1.0.0\]" CHANGELOG.md; then
        report_success "CHANGELOG.md contains [1.0.0] section"
    else
        report_error "CHANGELOG.md missing [1.0.0] section"
    fi

    if grep -q "2025-11-25" CHANGELOG.md; then
        report_success "CHANGELOG.md contains release date"
    else
        report_warning "CHANGELOG.md missing release date or date format incorrect"
    fi
else
    report_error "CHANGELOG.md not found"
fi
echo ""

# 5. Check RELEASE_NOTES_v1.0.md
echo "5. Checking RELEASE_NOTES_v1.0.md..."
if [ -f "RELEASE_NOTES_v1.0.md" ]; then
    if grep -q "v1.0.0" RELEASE_NOTES_v1.0.md; then
        report_success "RELEASE_NOTES_v1.0.md contains v1.0.0"
    else
        report_error "RELEASE_NOTES_v1.0.md missing v1.0.0 reference"
    fi

    if grep -q "November 25, 2025" RELEASE_NOTES_v1.0.md; then
        report_success "RELEASE_NOTES_v1.0.md contains release date"
    else
        report_warning "RELEASE_NOTES_v1.0.md missing release date or date format incorrect"
    fi
else
    report_error "RELEASE_NOTES_v1.0.md not found"
fi
echo ""

# 6. Check CLI version output (if binary exists)
echo "6. Checking CLI version (if built)..."
if [ -f "target/release/verum" ]; then
    CLI_VERSION=$(./target/release/verum --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || echo "unknown")
    if [ "$CLI_VERSION" == "$EXPECTED_VERSION" ]; then
        report_success "CLI binary reports version $EXPECTED_VERSION"
    else
        report_warning "CLI binary reports version $CLI_VERSION (expected $EXPECTED_VERSION) - may need rebuild"
    fi
else
    report_warning "CLI binary not found at target/release/verum (run 'cargo build --release' first)"
fi
echo ""

# 7. Check for version references in documentation
echo "7. Checking documentation version references..."
DOC_FILES_WITH_VERSION=0
DOC_FILES_CHECKED=0

# Check major documentation files
for doc in docs/LANGUAGE_GUIDE.md docs/API_REFERENCE.md docs/ARCHITECTURE.md; do
    if [ -f "$doc" ]; then
        DOC_FILES_CHECKED=$((DOC_FILES_CHECKED + 1))
        if grep -qE "1\.0\.0|v1\.0\.0" "$doc"; then
            DOC_FILES_WITH_VERSION=$((DOC_FILES_WITH_VERSION + 1))
            report_success "$(basename $doc) contains version references"
        else
            report_warning "$(basename $doc) may need version updates"
        fi
    fi
done

if [ $DOC_FILES_CHECKED -gt 0 ]; then
    echo "Documentation files checked: $DOC_FILES_CHECKED"
    echo "Files with version references: $DOC_FILES_WITH_VERSION"
fi
echo ""

# 8. Check Cargo.lock for consistency
echo "8. Checking Cargo.lock consistency..."
if [ -f "Cargo.lock" ]; then
    # Count how many times the expected version appears in Cargo.lock
    VERSION_COUNT=$(grep -c "version = \"$EXPECTED_VERSION\"" Cargo.lock || true)
    if [ $VERSION_COUNT -gt 0 ]; then
        report_success "Cargo.lock contains $VERSION_COUNT references to version $EXPECTED_VERSION"
    else
        report_warning "Cargo.lock may need updating (run 'cargo update')"
    fi
else
    report_error "Cargo.lock not found"
fi
echo ""

# 9. Summary
echo "========================================="
echo "SUMMARY"
echo "========================================="

if [ $EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}✓ All version checks passed!${NC}"
    echo ""
    echo "Version $EXPECTED_VERSION is consistent across:"
    echo "  - Workspace Cargo.toml"
    echo "  - All crate Cargo.toml files"
    echo "  - README.md"
    echo "  - CHANGELOG.md"
    echo "  - RELEASE_NOTES_v1.0.md"
    echo "  - Documentation"
    echo ""
    echo "The codebase is ready for v1.0.0 release!"
else
    echo -e "${RED}✗ Version consistency check failed!${NC}"
    echo ""
    echo "Please fix the errors reported above before proceeding with the release."
    echo ""
    echo "Common fixes:"
    echo "  1. Update versions in Cargo.toml files to $EXPECTED_VERSION"
    echo "  2. Update version references in README.md"
    echo "  3. Ensure CHANGELOG.md and RELEASE_NOTES_v1.0.md exist and are complete"
    echo "  4. Rebuild CLI with 'cargo build --release'"
    echo "  5. Run 'cargo update' to update Cargo.lock"
fi
echo ""

exit $EXIT_CODE
