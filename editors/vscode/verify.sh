#!/bin/bash
# Verification script for Verum VS Code Extension
# Run this to ensure all files are present and valid

set -e

echo "======================================"
echo "Verum VS Code Extension Verification"
echo "======================================"
echo ""

# Color codes
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Track results
ERRORS=0
WARNINGS=0

# Function to check file exists
check_file() {
    if [ -f "$1" ]; then
        echo -e "${GREEN}✓${NC} Found: $1"
    else
        echo -e "${RED}✗${NC} Missing: $1"
        ((ERRORS++))
    fi
}

# Function to check file is valid JSON
check_json() {
    if command -v jq &> /dev/null; then
        if jq empty "$1" 2>/dev/null; then
            echo -e "${GREEN}✓${NC} Valid JSON: $1"
        else
            echo -e "${RED}✗${NC} Invalid JSON: $1"
            ((ERRORS++))
        fi
    else
        echo -e "${YELLOW}⚠${NC} Cannot validate JSON (jq not installed): $1"
        ((WARNINGS++))
    fi
}

# Function to check TypeScript syntax
check_typescript() {
    if [ -f "$1" ]; then
        # Basic syntax check - file should exist and be non-empty
        if [ -s "$1" ]; then
            echo -e "${GREEN}✓${NC} TypeScript file: $1"
        else
            echo -e "${RED}✗${NC} Empty TypeScript file: $1"
            ((ERRORS++))
        fi
    fi
}

echo "1. Checking Core Files"
echo "----------------------"
check_file "package.json"
check_file "tsconfig.json"
check_file "language-configuration.json"
check_file "README.md"
check_file "CHANGELOG.md"
check_file "LICENSE"
check_file "IMPLEMENTATION.md"
check_file "QUICKSTART.md"
check_file ".gitignore"
check_file ".vscodeignore"
check_file ".eslintrc.json"
echo ""

echo "2. Checking Source Files"
echo "------------------------"
check_file "src/extension.ts"
check_file "src/refinementValidator.ts"
check_file "src/codeActionProvider.ts"
check_file "src/inlayHintsProvider.ts"
echo ""

echo "3. Checking Syntax Files"
echo "------------------------"
check_file "syntaxes/verum.tmLanguage.json"
echo ""

echo "4. Validating JSON Files"
echo "------------------------"
check_json "package.json"
check_json "tsconfig.json"
check_json "language-configuration.json"
check_json ".eslintrc.json"
check_json "syntaxes/verum.tmLanguage.json"
echo ""

echo "5. Validating TypeScript Files"
echo "-------------------------------"
check_typescript "src/extension.ts"
check_typescript "src/refinementValidator.ts"
check_typescript "src/codeActionProvider.ts"
check_typescript "src/inlayHintsProvider.ts"
echo ""

echo "6. Checking Package.json Structure"
echo "-----------------------------------"
if command -v jq &> /dev/null; then
    # Check required fields
    NAME=$(jq -r '.name' package.json)
    VERSION=$(jq -r '.version' package.json)
    MAIN=$(jq -r '.main' package.json)
    ENGINES=$(jq -r '.engines.vscode' package.json)

    if [ "$NAME" = "verum-language" ]; then
        echo -e "${GREEN}✓${NC} Package name: $NAME"
    else
        echo -e "${RED}✗${NC} Invalid package name: $NAME"
        ((ERRORS++))
    fi

    if [ "$VERSION" = "1.0.0" ]; then
        echo -e "${GREEN}✓${NC} Version: $VERSION"
    else
        echo -e "${YELLOW}⚠${NC} Version: $VERSION (expected 1.0.0)"
        ((WARNINGS++))
    fi

    if [ "$MAIN" = "./out/extension.js" ]; then
        echo -e "${GREEN}✓${NC} Main entry: $MAIN"
    else
        echo -e "${RED}✗${NC} Invalid main entry: $MAIN"
        ((ERRORS++))
    fi

    echo -e "${GREEN}✓${NC} VS Code engine: $ENGINES"

    # Check for required contributions
    LANGS=$(jq -r '.contributes.languages | length' package.json)
    GRAMMARS=$(jq -r '.contributes.grammars | length' package.json)
    COMMANDS=$(jq -r '.contributes.commands | length' package.json)
    KEYBINDINGS=$(jq -r '.contributes.keybindings | length' package.json)

    echo -e "${GREEN}✓${NC} Languages: $LANGS"
    echo -e "${GREEN}✓${NC} Grammars: $GRAMMARS"
    echo -e "${GREEN}✓${NC} Commands: $COMMANDS"
    echo -e "${GREEN}✓${NC} Keybindings: $KEYBINDINGS"
else
    echo -e "${YELLOW}⚠${NC} Cannot validate package.json structure (jq not installed)"
    ((WARNINGS++))
fi
echo ""

echo "7. Code Statistics"
echo "------------------"
TS_LINES=$(find src -name "*.ts" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')
JSON_LINES=$(find . -maxdepth 2 -name "*.json" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')
MD_LINES=$(find . -maxdepth 1 -name "*.md" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')

echo "TypeScript: $TS_LINES lines"
echo "JSON: $JSON_LINES lines"
echo "Documentation: $MD_LINES lines"
echo ""

echo "8. Directory Structure"
echo "----------------------"
if command -v tree &> /dev/null; then
    tree -L 2 -I 'node_modules|out' --charset ascii
else
    find . -type d -name "node_modules" -prune -o -type d -name "out" -prune -o -type d -print | head -10
fi
echo ""

echo "======================================"
echo "Verification Summary"
echo "======================================"
if [ $ERRORS -eq 0 ] && [ $WARNINGS -eq 0 ]; then
    echo -e "${GREEN}✓ All checks passed!${NC}"
    echo ""
    echo "Extension is ready for:"
    echo "  - Development (npm install && npm run compile)"
    echo "  - Packaging (npm run package)"
    echo "  - Publishing (npm run publish)"
elif [ $ERRORS -eq 0 ]; then
    echo -e "${YELLOW}⚠ All checks passed with $WARNINGS warnings${NC}"
    echo ""
    echo "Extension is functional but review warnings above"
else
    echo -e "${RED}✗ Found $ERRORS errors and $WARNINGS warnings${NC}"
    echo ""
    echo "Please fix errors before proceeding"
    exit 1
fi

echo ""
echo "======================================"
echo "Next Steps"
echo "======================================"
echo "1. Install dependencies: npm install"
echo "2. Compile TypeScript:   npm run compile"
echo "3. Run in dev mode:      Press F5 in VS Code"
echo "4. Package extension:    npm run package"
echo "5. Read docs:            cat README.md"
echo ""

exit 0
