#!/usr/bin/env bash
# GitHub Release Preparation Script for Verum v1.0.0
# Prepares all artifacts needed for GitHub release

set -euo pipefail

# Configuration
VERSION="1.0.0"
TAG="v${VERSION}"
RELEASE_DATE="2025-11-25"
RELEASE_DIR="target/release-artifacts"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo "========================================="
echo "Verum v1.0.0 GitHub Release Preparation"
echo "========================================="
echo ""

# Create release artifacts directory
echo "Creating release artifacts directory..."
mkdir -p "$RELEASE_DIR"
echo -e "${GREEN}✓${NC} Created $RELEASE_DIR"
echo ""

# 1. Create Git Tag
echo "1. Git Tag Creation"
echo "-------------------"
echo "Tag: $TAG"
echo "Date: $RELEASE_DATE"
echo ""

if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo -e "${YELLOW}⚠${NC} Tag $TAG already exists"
    echo "To recreate, delete it first: git tag -d $TAG && git push origin :$TAG"
else
    echo "To create the tag, run:"
    echo "  git tag -a $TAG -m \"Verum v${VERSION} - Production Release\""
    echo "  git push origin $TAG"
fi
echo ""

# 2. Generate Release Notes
echo "2. Release Notes"
echo "----------------"
if [ -f "RELEASE_NOTES_v1.0.md" ]; then
    echo -e "${GREEN}✓${NC} RELEASE_NOTES_v1.0.md exists"
    cp RELEASE_NOTES_v1.0.md "$RELEASE_DIR/RELEASE_NOTES_v1.0.md"
    echo "Copied to $RELEASE_DIR/"
else
    echo -e "${RED}✗${NC} RELEASE_NOTES_v1.0.md not found!"
    echo "Please create it before proceeding."
fi
echo ""

# 3. Generate CHANGELOG
echo "3. Changelog"
echo "------------"
if [ -f "CHANGELOG.md" ]; then
    echo -e "${GREEN}✓${NC} CHANGELOG.md exists"
    cp CHANGELOG.md "$RELEASE_DIR/CHANGELOG.md"
    echo "Copied to $RELEASE_DIR/"
else
    echo -e "${RED}✗${NC} CHANGELOG.md not found!"
    echo "Please create it before proceeding."
fi
echo ""

# 4. Source Archives
echo "4. Source Archives"
echo "------------------"
echo "GitHub will automatically create:"
echo "  - $RELEASE_DIR/verum-v${VERSION}-source.tar.gz"
echo "  - $RELEASE_DIR/verum-v${VERSION}-source.zip"
echo ""
echo "You can also create them manually with:"
echo "  git archive --format=tar.gz --prefix=verum-${VERSION}/ $TAG > $RELEASE_DIR/verum-v${VERSION}-source.tar.gz"
echo "  git archive --format=zip --prefix=verum-${VERSION}/ $TAG > $RELEASE_DIR/verum-v${VERSION}-source.zip"
echo ""

# 5. Binary Packages (placeholders - require actual builds)
echo "5. Binary Packages"
echo "------------------"
echo "The following binaries should be built on their respective platforms:"
echo ""
echo "Linux x86_64:"
echo "  cargo build --release --target x86_64-unknown-linux-gnu"
echo "  tar czf $RELEASE_DIR/verum-v${VERSION}-x86_64-unknown-linux-gnu.tar.gz -C target/x86_64-unknown-linux-gnu/release verum"
echo ""
echo "macOS x86_64:"
echo "  cargo build --release --target x86_64-apple-darwin"
echo "  tar czf $RELEASE_DIR/verum-v${VERSION}-x86_64-apple-darwin.tar.gz -C target/x86_64-apple-darwin/release verum"
echo ""
echo "macOS ARM64:"
echo "  cargo build --release --target aarch64-apple-darwin"
echo "  tar czf $RELEASE_DIR/verum-v${VERSION}-aarch64-apple-darwin.tar.gz -C target/aarch64-apple-darwin/release verum"
echo ""
echo "Windows x86_64:"
echo "  cargo build --release --target x86_64-pc-windows-msvc"
echo "  cd target/x86_64-pc-windows-msvc/release && zip ../../../$RELEASE_DIR/verum-v${VERSION}-x86_64-pc-windows-msvc.zip verum.exe"
echo ""

# 6. Documentation Package
echo "6. Documentation Package"
echo "------------------------"
echo "Building documentation..."
if cargo doc --workspace --all-features --no-deps --quiet; then
    echo -e "${GREEN}✓${NC} Documentation built successfully"

    # Create docs archive
    cd target/doc
    tar czf "../../$RELEASE_DIR/verum-v${VERSION}-docs.tar.gz" .
    cd ../..
    echo -e "${GREEN}✓${NC} Created $RELEASE_DIR/verum-v${VERSION}-docs.tar.gz"
else
    echo -e "${RED}✗${NC} Documentation build failed"
fi
echo ""

# 7. Checksums
echo "7. Checksums"
echo "------------"
echo "Generating SHA256 checksums..."

# Generate checksums for all files in release directory
cd "$RELEASE_DIR"
if ls *.tar.gz *.zip *.md 2>/dev/null | grep -q .; then
    sha256sum *.tar.gz *.zip *.md 2>/dev/null > SHA256SUMS || true
    if [ -f "SHA256SUMS" ]; then
        echo -e "${GREEN}✓${NC} Generated SHA256SUMS"
        cat SHA256SUMS
    fi
else
    echo -e "${YELLOW}⚠${NC} No artifacts found to checksum yet"
fi
cd - > /dev/null
echo ""

# 8. GPG Signature (optional)
echo "8. GPG Signature"
echo "----------------"
if command -v gpg &> /dev/null; then
    echo "To sign the checksums file, run:"
    echo "  cd $RELEASE_DIR && gpg --detach-sign --armor SHA256SUMS"
    echo "This will create SHA256SUMS.asc"
else
    echo -e "${YELLOW}⚠${NC} GPG not found. Skipping signature creation."
    echo "Install GPG to create signatures for release artifacts."
fi
echo ""

# 9. Platform-Specific Packages
echo "9. Platform-Specific Packages"
echo "------------------------------"
echo ""
echo "The following package configurations should be created:"
echo ""
echo "Debian/Ubuntu (.deb):"
echo "  - Use cargo-deb: cargo install cargo-deb"
echo "  - Build: cargo deb --package verum_cli"
echo "  - Output: target/debian/verum_${VERSION}_amd64.deb"
echo ""
echo "Fedora/RHEL (.rpm):"
echo "  - Use cargo-generate-rpm: cargo install cargo-generate-rpm"
echo "  - Build: cargo build --release && cargo generate-rpm"
echo "  - Output: target/generate-rpm/verum-${VERSION}-1.x86_64.rpm"
echo ""
echo "macOS (.pkg):"
echo "  - Use pkgbuild and productbuild tools"
echo "  - Create package identifier: org.verum-lang.verum"
echo ""
echo "Windows (.msi):"
echo "  - Use WiX Toolset: https://wixtoolset.org/"
echo "  - Create installer with verum.exe and documentation"
echo ""
echo "Homebrew Formula:"
echo "  - Create formula at: homebrew-verum/Formula/verum.rb"
echo "  - Include download URLs and SHA256 checksums"
echo ""
echo "Chocolatey Package:"
echo "  - Create package manifest at: chocolatey-verum/verum.nuspec"
echo "  - Include download URLs and checksums"
echo ""
echo "Arch Linux PKGBUILD:"
echo "  - Create PKGBUILD file for AUR submission"
echo "  - Include source URL and integrity checksums"
echo ""

# 10. GitHub Release Creation
echo "10. GitHub Release Creation"
echo "----------------------------"
echo ""
echo "To create the GitHub release:"
echo ""
echo "Option 1: Using GitHub CLI (gh)"
echo "  gh release create $TAG \\"
echo "    --title \"Verum v${VERSION} - Production Release\" \\"
echo "    --notes-file RELEASE_NOTES_v1.0.md \\"
echo "    --latest \\"
echo "    $RELEASE_DIR/*.tar.gz \\"
echo "    $RELEASE_DIR/*.zip \\"
echo "    $RELEASE_DIR/SHA256SUMS \\"
echo "    $RELEASE_DIR/SHA256SUMS.asc"
echo ""
echo "Option 2: Using GitHub Web Interface"
echo "  1. Go to: https://github.com/verum-lang/verum/releases/new"
echo "  2. Choose tag: $TAG"
echo "  3. Release title: Verum v${VERSION} - Production Release"
echo "  4. Copy contents from RELEASE_NOTES_v1.0.md"
echo "  5. Upload all files from $RELEASE_DIR/"
echo "  6. Check 'Set as latest release'"
echo "  7. Click 'Publish release'"
echo ""

# 11. Pre-Release Checklist
echo "11. Pre-Release Checklist"
echo "-------------------------"
echo ""
echo "Before creating the release, verify:"
echo ""
echo "  [ ] All tests pass: cargo test --workspace --all-features"
echo "  [ ] All benchmarks run: cargo bench --workspace"
echo "  [ ] Documentation builds: cargo doc --workspace --all-features"
echo "  [ ] Clippy passes: cargo clippy --workspace --all-features -- -D warnings"
echo "  [ ] Format check passes: cargo fmt --all -- --check"
echo "  [ ] Version checker passes: ./scripts/check_versions.sh"
echo "  [ ] CHANGELOG.md is complete and up-to-date"
echo "  [ ] RELEASE_NOTES_v1.0.md is complete"
echo "  [ ] All crate versions are 1.0.0"
echo "  [ ] Security audit passes: cargo audit"
echo "  [ ] No uncommitted changes: git status"
echo "  [ ] On correct branch: git branch (should be main)"
echo "  [ ] All changes pushed: git push origin main"
echo "  [ ] Tag created and pushed: git push origin $TAG"
echo ""

# Summary
echo "========================================="
echo "SUMMARY"
echo "========================================="
echo ""
echo "Release artifacts directory: $RELEASE_DIR"
echo ""
echo "Files prepared:"
ls -lh "$RELEASE_DIR/" 2>/dev/null || echo "  (no files yet)"
echo ""
echo -e "${GREEN}✓${NC} GitHub release preparation script completed"
echo ""
echo "Next steps:"
echo "  1. Build binaries for all platforms"
echo "  2. Generate platform-specific packages"
echo "  3. Run pre-release checklist"
echo "  4. Create and verify checksums"
echo "  5. Sign checksums with GPG (optional)"
echo "  6. Create GitHub release"
echo "  7. Publish to crates.io: ./scripts/publish_crates.sh --publish"
echo "  8. Announce release"
echo ""
