#!/usr/bin/env bash
# Verum v1.0.0 Crates.io Publishing Script
# Publishes crates in correct dependency order

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Expected version
EXPECTED_VERSION="1.0.0"

# Dry run mode (default: true, set to false with --publish flag)
DRY_RUN=true

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --publish)
            DRY_RUN=false
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --dry-run     Check what would be published (default)"
            echo "  --publish     Actually publish to crates.io"
            echo "  --help        Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

echo "========================================="
echo "Verum v1.0.0 Crates.io Publishing"
if [ "$DRY_RUN" = true ]; then
    echo -e "${YELLOW}MODE: DRY RUN (use --publish to actually publish)${NC}"
else
    echo -e "${RED}MODE: LIVE PUBLISHING${NC}"
fi
echo "========================================="
echo ""

# Function to publish a crate
publish_crate() {
    local crate_name=$1
    local crate_path="crates/$crate_name"

    echo ""
    echo -e "${BLUE}Publishing: $crate_name${NC}"
    echo "----------------------------------------"

    # Check if crate directory exists
    if [ ! -d "$crate_path" ]; then
        echo -e "${RED}✗ ERROR: Crate directory not found: $crate_path${NC}"
        return 1
    fi

    # Check if Cargo.toml exists
    if [ ! -f "$crate_path/Cargo.toml" ]; then
        echo -e "${RED}✗ ERROR: Cargo.toml not found in $crate_path${NC}"
        return 1
    fi

    # Verify version
    local version
    if grep -q "^version\.workspace = true" "$crate_path/Cargo.toml"; then
        version=$EXPECTED_VERSION
        echo "Version: $version (workspace)"
    else
        version=$(grep -E '^version = ' "$crate_path/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
        echo "Version: $version (explicit)"
    fi

    if [ "$version" != "$EXPECTED_VERSION" ]; then
        echo -e "${RED}✗ ERROR: Version mismatch. Expected $EXPECTED_VERSION, got $version${NC}"
        return 1
    fi

    # Check if already published
    if cargo search "$crate_name" --limit 1 | grep -q "^$crate_name = \"$version\""; then
        echo -e "${YELLOW}⚠ WARNING: $crate_name $version already published to crates.io${NC}"
        echo "Skipping..."
        return 0
    fi

    # Run cargo package to verify
    echo "Running cargo package..."
    if ! cargo package --manifest-path "$crate_path/Cargo.toml" --quiet; then
        echo -e "${RED}✗ ERROR: cargo package failed for $crate_name${NC}"
        return 1
    fi
    echo -e "${GREEN}✓ Package build successful${NC}"

    # Publish or dry-run
    if [ "$DRY_RUN" = false ]; then
        echo "Publishing to crates.io..."
        if cargo publish --manifest-path "$crate_path/Cargo.toml"; then
            echo -e "${GREEN}✓ Successfully published $crate_name $version${NC}"
            # Wait a bit to let crates.io index the package
            echo "Waiting 30 seconds for crates.io to index..."
            sleep 30
        else
            echo -e "${RED}✗ ERROR: Failed to publish $crate_name${NC}"
            return 1
        fi
    else
        echo -e "${YELLOW}[DRY RUN] Would publish $crate_name $version${NC}"
    fi

    return 0
}

# Publishing order based on dependency graph
# Layer 0: Foundation (no dependencies within Verum)
LAYER_0=(
    "verum_core"
    "verum_derive"
)

# Layer 1: Core Infrastructure
LAYER_1=(
    "verum_cbgr"
    "verum_std"
)

# Layer 2: Parsing and AST
LAYER_2=(
    "verum_error"
    "verum_ast"
    "verum_lexer"
    "verum_parser"
    "verum_diagnostics"
)

# Layer 3: Type System and Analysis
LAYER_3=(
    "verum_types"
    "verum_smt"
    "verum_modules"
)

# Layer 4: Execution and Runtime
LAYER_4=(
    "verum_context_macros"
    "verum_context"
    "verum_runtime"
    "verum_codegen"
    "verum_interpreter"
    "verum_resolve"
    "verum_verification"
)

# Layer 5: Tools
LAYER_5=(
    "verum_compiler"
    "verum_lsp"
    "verum_cli"
)

# Layer 6: Testing (optional, not published to crates.io)
LAYER_6=(
    # "verum_integration_tests"  # Not published
)

echo "Publishing order:"
echo ""
echo "Layer 0 (Foundation):"
for crate in "${LAYER_0[@]}"; do
    echo "  - $crate"
done
echo ""
echo "Layer 1 (Core Infrastructure):"
for crate in "${LAYER_1[@]}"; do
    echo "  - $crate"
done
echo ""
echo "Layer 2 (Parsing and AST):"
for crate in "${LAYER_2[@]}"; do
    echo "  - $crate"
done
echo ""
echo "Layer 3 (Type System and Analysis):"
for crate in "${LAYER_3[@]}"; do
    echo "  - $crate"
done
echo ""
echo "Layer 4 (Execution and Runtime):"
for crate in "${LAYER_4[@]}"; do
    echo "  - $crate"
done
echo ""
echo "Layer 5 (Tools):"
for crate in "${LAYER_5[@]}"; do
    echo "  - $crate"
done
echo ""

if [ "$DRY_RUN" = true ]; then
    echo -e "${YELLOW}This is a DRY RUN. No crates will be published.${NC}"
    echo "Use --publish flag to actually publish to crates.io"
else
    echo -e "${RED}WARNING: This will publish crates to crates.io!${NC}"
    echo "Press Ctrl+C within 10 seconds to cancel..."
    sleep 10
fi
echo ""

# Counters
TOTAL_CRATES=0
PUBLISHED_CRATES=0
FAILED_CRATES=0
SKIPPED_CRATES=0

# Publish Layer 0
echo "========================================="
echo "Publishing Layer 0: Foundation"
echo "========================================="
for crate in "${LAYER_0[@]}"; do
    TOTAL_CRATES=$((TOTAL_CRATES + 1))
    if publish_crate "$crate"; then
        PUBLISHED_CRATES=$((PUBLISHED_CRATES + 1))
    else
        FAILED_CRATES=$((FAILED_CRATES + 1))
        echo -e "${RED}Failed to publish $crate. Stopping.${NC}"
        exit 1
    fi
done

# Publish Layer 1
echo ""
echo "========================================="
echo "Publishing Layer 1: Core Infrastructure"
echo "========================================="
for crate in "${LAYER_1[@]}"; do
    TOTAL_CRATES=$((TOTAL_CRATES + 1))
    if publish_crate "$crate"; then
        PUBLISHED_CRATES=$((PUBLISHED_CRATES + 1))
    else
        FAILED_CRATES=$((FAILED_CRATES + 1))
        echo -e "${RED}Failed to publish $crate. Stopping.${NC}"
        exit 1
    fi
done

# Publish Layer 2
echo ""
echo "========================================="
echo "Publishing Layer 2: Parsing and AST"
echo "========================================="
for crate in "${LAYER_2[@]}"; do
    TOTAL_CRATES=$((TOTAL_CRATES + 1))
    if publish_crate "$crate"; then
        PUBLISHED_CRATES=$((PUBLISHED_CRATES + 1))
    else
        FAILED_CRATES=$((FAILED_CRATES + 1))
        echo -e "${RED}Failed to publish $crate. Stopping.${NC}"
        exit 1
    fi
done

# Publish Layer 3
echo ""
echo "========================================="
echo "Publishing Layer 3: Type System and Analysis"
echo "========================================="
for crate in "${LAYER_3[@]}"; do
    TOTAL_CRATES=$((TOTAL_CRATES + 1))
    if publish_crate "$crate"; then
        PUBLISHED_CRATES=$((PUBLISHED_CRATES + 1))
    else
        FAILED_CRATES=$((FAILED_CRATES + 1))
        echo -e "${RED}Failed to publish $crate. Stopping.${NC}"
        exit 1
    fi
done

# Publish Layer 4
echo ""
echo "========================================="
echo "Publishing Layer 4: Execution and Runtime"
echo "========================================="
for crate in "${LAYER_4[@]}"; do
    TOTAL_CRATES=$((TOTAL_CRATES + 1))
    if publish_crate "$crate"; then
        PUBLISHED_CRATES=$((PUBLISHED_CRATES + 1))
    else
        FAILED_CRATES=$((FAILED_CRATES + 1))
        echo -e "${RED}Failed to publish $crate. Stopping.${NC}"
        exit 1
    fi
done

# Publish Layer 5
echo ""
echo "========================================="
echo "Publishing Layer 5: Tools"
echo "========================================="
for crate in "${LAYER_5[@]}"; do
    TOTAL_CRATES=$((TOTAL_CRATES + 1))
    if publish_crate "$crate"; then
        PUBLISHED_CRATES=$((PUBLISHED_CRATES + 1))
    else
        FAILED_CRATES=$((FAILED_CRATES + 1))
        echo -e "${RED}Failed to publish $crate. Stopping.${NC}"
        exit 1
    fi
done

# Summary
echo ""
echo "========================================="
echo "PUBLISHING SUMMARY"
echo "========================================="
echo "Total crates processed: $TOTAL_CRATES"
echo "Successfully published: $PUBLISHED_CRATES"
echo "Failed: $FAILED_CRATES"
echo "Skipped (already published): $SKIPPED_CRATES"
echo ""

if [ $FAILED_CRATES -eq 0 ]; then
    if [ "$DRY_RUN" = true ]; then
        echo -e "${GREEN}✓ Dry run completed successfully!${NC}"
        echo "All crates are ready to be published."
        echo ""
        echo "To actually publish, run:"
        echo "  $0 --publish"
    else
        echo -e "${GREEN}✓ All crates published successfully!${NC}"
        echo ""
        echo "Next steps:"
        echo "1. Verify crates on crates.io (may take a few minutes to appear)"
        echo "2. Check documentation built on docs.rs"
        echo "3. Test installation: cargo install verum_cli --version $EXPECTED_VERSION"
        echo "4. Create GitHub release"
        echo "5. Announce release to community"
    fi
else
    echo -e "${RED}✗ Publishing failed!${NC}"
    echo "Please fix the errors and try again."
    exit 1
fi
