#!/bin/bash
# Pre-commit Security Hook for Verum Platform
#
# This script runs automated security checks before each commit.
# Install with: ln -s ../../scripts/pre-commit-security.sh .git/hooks/pre-commit
#
# Override with: git commit --no-verify

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Color output
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

# Counters
ERRORS=0
WARNINGS=0

echo "Running pre-commit security checks..."
echo ""

# ============================================================================
# CHECK 1: Detect hardcoded secrets and credentials
# ============================================================================

echo "▶ Checking for hardcoded secrets..."

SECRET_PATTERNS=(
    'password\s*[:=]'
    'api[_-]?key\s*[:=]'
    'secret\s*[:=]'
    'token\s*[:=]'
    'auth[_-]?token\s*[:=]'
    'private[_-]?key'
)

for pattern in "${SECRET_PATTERNS[@]}"; do
    if git diff --cached | grep -iE "$pattern" 2>/dev/null; then
        echo -e "${RED}✗ ERROR: Potential secret detected!${NC}"
        echo "  Pattern: $pattern"
        echo "  Action: Remove the secret and use environment variables instead"
        ERRORS=$((ERRORS + 1))
    fi
done

if [ $ERRORS -eq 0 ]; then
    echo -e "${GREEN}✓ No hardcoded secrets detected${NC}"
fi
echo ""

# ============================================================================
# CHECK 2: Check for .env files and credential files
# ============================================================================

echo "▶ Checking for credential files..."

CREDENTIAL_FILES=$(git diff --cached --name-only | grep -E '\.env|credentials|secrets|private_key' || echo "")

if [ -n "$CREDENTIAL_FILES" ]; then
    echo -e "${RED}✗ ERROR: Credential files detected in staging!${NC}"
    echo "$CREDENTIAL_FILES"
    echo "  Add these to .gitignore: $CREDENTIAL_FILES"
    ERRORS=$((ERRORS + 1))
else
    echo -e "${GREEN}✓ No credential files staged${NC}"
fi
echo ""

# ============================================================================
# CHECK 3: Check for unsafe blocks without SAFETY comments
# ============================================================================

echo "▶ Checking for documented unsafe code..."

STAGED_RS_FILES=$(git diff --cached --name-only --diff-filter=ACM -- '*.rs' | grep -v test || echo "")

if [ -n "$STAGED_RS_FILES" ]; then
    UNSAFE_COUNT=0
    UNDOCUMENTED=0

    for file in $STAGED_RS_FILES; do
        if [ -f "$file" ]; then
            FILE_UNSAFE=$(git diff --cached "$file" | grep -c "^+.*unsafe {" || echo 0)
            FILE_SAFETY=$(git diff --cached "$file" | grep -c "^+.*// SAFETY" || echo 0)

            if [ "$FILE_UNSAFE" -gt 0 ] && [ "$FILE_SAFETY" -eq 0 ]; then
                echo -e "${YELLOW}⚠ Warning: Unsafe code in $file without SAFETY comments${NC}"
                WARNINGS=$((WARNINGS + 1))
                UNDOCUMENTED=$((UNDOCUMENTED + FILE_UNSAFE))
            fi

            UNSAFE_COUNT=$((UNSAFE_COUNT + FILE_UNSAFE))
        fi
    done

    if [ "$UNSAFE_COUNT" -gt 0 ]; then
        echo "  Unsafe blocks added: $UNSAFE_COUNT"
        if [ "$UNDOCUMENTED" -gt 0 ]; then
            echo "  Undocumented: $UNDOCUMENTED"
            echo "  Add SAFETY comments explaining the safety invariants"
        fi
    fi

    echo -e "${GREEN}✓ Unsafe code check completed${NC}"
else
    echo -e "${GREEN}✓ No Rust files to check${NC}"
fi
echo ""

# ============================================================================
# CHECK 4: Run cargo fmt check
# ============================================================================

echo "▶ Checking code formatting..."

if ! cargo fmt --check 2>/dev/null; then
    echo -e "${YELLOW}⚠ Code formatting issues detected${NC}"
    echo "  Run: cargo fmt"
    WARNINGS=$((WARNINGS + 1))
else
    echo -e "${GREEN}✓ Code formatting OK${NC}"
fi
echo ""

# ============================================================================
# CHECK 5: Run cargo clippy on staged files (if available)
# ============================================================================

echo "▶ Running clippy lint (development mode)..."

if command -v cargo &> /dev/null; then
    if cargo clippy --all-targets -- -D warnings 2>/dev/null; then
        echo -e "${GREEN}✓ Clippy checks passed${NC}"
    else
        echo -e "${YELLOW}⚠ Clippy warnings detected (non-blocking)${NC}"
        WARNINGS=$((WARNINGS + 1))
    fi
else
    echo -e "${YELLOW}⚠ Cargo not found, skipping clippy${NC}"
fi
echo ""

# ============================================================================
# CHECK 6: Check for common security anti-patterns
# ============================================================================

echo "▶ Scanning for security anti-patterns..."

ANTI_PATTERNS=(
    'eval('
    'exec('
    'system('
    'shell_exec('
    'passthru('
)

PATTERN_FOUND=0
for pattern in "${ANTI_PATTERNS[@]}"; do
    if git diff --cached -- '*.rs' | grep -i "$pattern" 2>/dev/null; then
        echo -e "${YELLOW}⚠ Potential security issue: $pattern${NC}"
        PATTERN_FOUND=1
        WARNINGS=$((WARNINGS + 1))
    fi
done

if [ $PATTERN_FOUND -eq 0 ]; then
    echo -e "${GREEN}✓ No common security anti-patterns found${NC}"
fi
echo ""

# ============================================================================
# CHECK 7: Verify no transmute without safety docs
# ============================================================================

echo "▶ Checking transmute operations..."

TRANSMUTE_COUNT=$(git diff --cached -- '*.rs' | grep -c "transmute" || echo 0)
if [ "$TRANSMUTE_COUNT" -gt 0 ]; then
    SAFE_TRANSMUTE=$(git diff --cached -- '*.rs' | grep -B2 "transmute" | grep -c "SAFETY" || echo 0)
    if [ "$TRANSMUTE_COUNT" -gt "$SAFE_TRANSMUTE" ]; then
        echo -e "${YELLOW}⚠ Transmute without SAFETY comments detected${NC}"
        echo "  Add SAFETY comments explaining lifetime/alignment invariants"
        WARNINGS=$((WARNINGS + 1))
    else
        echo -e "${GREEN}✓ Transmute operations documented${NC}"
    fi
else
    echo -e "${GREEN}✓ No transmute operations${NC}"
fi
echo ""

# ============================================================================
# CHECK 8: Check for debugging statements
# ============================================================================

echo "▶ Checking for debugging statements..."

DEBUG_PATTERNS=(
    'println!'
    'dbg!'
    'unwrap()'
    'expect('
)

DEBUG_COUNT=0
for pattern in "${DEBUG_PATTERNS[@]}"; do
    PATTERN_COUNT=$(git diff --cached -- '*.rs' | grep -c "$pattern" || echo 0)
    if [ "$PATTERN_COUNT" -gt 0 ]; then
        DEBUG_COUNT=$((DEBUG_COUNT + PATTERN_COUNT))
    fi
done

if [ "$DEBUG_COUNT" -gt 0 ]; then
    echo -e "${YELLOW}⚠ Debug statements found: $DEBUG_COUNT${NC}"
    echo "  Review before committing: println!, dbg!, unwrap(), expect()"
    WARNINGS=$((WARNINGS + 1))
else
    echo -e "${GREEN}✓ No obvious debug statements${NC}"
fi
echo ""

# ============================================================================
# SUMMARY
# ============================================================================

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Security Check Summary"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ $ERRORS -gt 0 ]; then
    echo -e "${RED}✗ ERRORS: $ERRORS${NC}"
    echo ""
    echo "Commit BLOCKED due to security issues."
    echo ""
    echo "To override and commit anyway (NOT RECOMMENDED):"
    echo "  git commit --no-verify"
    echo ""
    exit 1
fi

if [ $WARNINGS -gt 0 ]; then
    echo -e "${YELLOW}⚠ WARNINGS: $WARNINGS${NC}"
    echo ""
    echo "Review warnings above before committing."
    echo ""
    echo "To commit anyway:"
    echo "  git commit"
    echo ""
fi

echo -e "${GREEN}✓ Security checks passed${NC}"
echo ""
echo "Proceeding with commit..."
exit 0
