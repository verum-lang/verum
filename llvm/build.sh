#!/bin/bash
# Build LLVM/LLD/MLIR from source
#
# Usage:
#   ./build.sh                    # Build using llvm.toml config
#   ./build.sh llvmorg-21.0.0     # Build specific tag
#   ./build.sh --clean            # Clean and rebuild
#
# Prerequisites:
#   - CMake 3.20+
#   - Ninja (recommended) or Make
#   - C++ compiler (GCC 9+, Clang 12+, or MSVC 2019+)
#   - ~50GB disk space for build
#   - ~16GB RAM recommended

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="$SCRIPT_DIR/llvm.toml"
SOURCE_DIR="$SCRIPT_DIR/llvm-project"
BUILD_DIR="$SCRIPT_DIR/build"
INSTALL_DIR="$SCRIPT_DIR/install"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1" >&2; }
log_error() { echo -e "${RED}[ERROR]${NC} $1" >&2; }

# Parse llvm.toml to get configuration
parse_config() {
    if [[ ! -f "$CONFIG_FILE" ]]; then
        log_error "Config file not found: $CONFIG_FILE"
        exit 1
    fi

    # Extract values using grep + cut/awk (more reliable than sed on macOS)
    # Format: key = "value" or key = [array]
    LLVM_TAG=$(grep '^tag\s*=' "$CONFIG_FILE" | head -1 | cut -d'"' -f2)
    LLVM_REPO=$(grep '^repo' "$CONFIG_FILE" | head -1 | cut -d'"' -f2)
    LLVM_PROJECTS=$(grep '^projects' "$CONFIG_FILE" | cut -d'[' -f2 | cut -d']' -f1 | tr -d '" ' | tr ',' ';')
    LLVM_TARGETS=$(grep '^targets' "$CONFIG_FILE" | cut -d'[' -f2 | cut -d']' -f1 | tr -d '" ' | tr ',' ';')
    BUILD_TYPE=$(grep '^build_type' "$CONFIG_FILE" | cut -d'"' -f2)
    SHALLOW=$(grep '^shallow' "$CONFIG_FILE" | cut -d'=' -f2 | tr -d ' ')

    # Defaults
    LLVM_TAG="${LLVM_TAG:-llvmorg-21.0.0}"
    LLVM_REPO="${LLVM_REPO:-https://github.com/llvm/llvm-project.git}"
    LLVM_PROJECTS="${LLVM_PROJECTS:-clang;lld;mlir}"
    LLVM_TARGETS="${LLVM_TARGETS:-X86;AArch64;WebAssembly}"
    BUILD_TYPE="${BUILD_TYPE:-Release}"
    SHALLOW="${SHALLOW:-true}"
}

# Clone LLVM source if needed
clone_source() {
    if [[ -d "$SOURCE_DIR" ]]; then
        log_info "LLVM source already exists at $SOURCE_DIR"

        # Check if we need to switch tags
        cd "$SOURCE_DIR"
        CURRENT_TAG=$(git describe --tags --exact-match 2>/dev/null || echo "unknown")
        if [[ "$CURRENT_TAG" != "$LLVM_TAG" ]]; then
            log_info "Switching from $CURRENT_TAG to $LLVM_TAG"
            git fetch --tags --depth 1 origin "refs/tags/$LLVM_TAG:refs/tags/$LLVM_TAG"
            git checkout "$LLVM_TAG"
        fi
        cd "$SCRIPT_DIR"
    else
        log_info "Cloning LLVM from $LLVM_REPO (tag: $LLVM_TAG)"

        if [[ "$SHALLOW" == "true" ]]; then
            git clone --depth 1 --branch "$LLVM_TAG" "$LLVM_REPO" "$SOURCE_DIR"
        else
            git clone --branch "$LLVM_TAG" "$LLVM_REPO" "$SOURCE_DIR"
        fi
    fi
}

# Detect build tool (Ninja preferred)
detect_generator() {
    if command -v ninja &> /dev/null; then
        echo "Ninja"
    else
        log_warn "Ninja not found, using Unix Makefiles (slower)"
        echo "Unix Makefiles"
    fi
}

# Detect number of parallel jobs
detect_jobs() {
    if [[ -f /proc/cpuinfo ]]; then
        # Linux
        nproc
    elif [[ "$(uname)" == "Darwin" ]]; then
        # macOS
        sysctl -n hw.ncpu
    else
        # Fallback
        echo 4
    fi
}

# Configure CMake
configure() {
    log_info "Configuring LLVM build..."

    local GENERATOR=$(detect_generator)

    log_info "CMake Generator: $GENERATOR"
    log_info "Projects: $LLVM_PROJECTS"
    log_info "Targets: $LLVM_TARGETS"
    log_info "Build Type: $BUILD_TYPE"

    # Size optimization flags
    local SIZE_FLAGS=""
    if [[ "$BUILD_TYPE" == "MinSizeRel" ]]; then
        SIZE_FLAGS="-Os -DNDEBUG"
        log_info "Optimizing for minimal binary size"
    fi

    cmake -G "$GENERATOR" -B "$BUILD_DIR" -S "$SOURCE_DIR/llvm" \
        -DCMAKE_BUILD_TYPE="$BUILD_TYPE" \
        -DCMAKE_INSTALL_PREFIX="$INSTALL_DIR" \
        -DLLVM_ENABLE_PROJECTS="$LLVM_PROJECTS" \
        -DLLVM_TARGETS_TO_BUILD="$LLVM_TARGETS" \
        \
        `# Static library configuration` \
        -DLLVM_BUILD_LLVM_DYLIB=OFF \
        -DLLVM_LINK_LLVM_DYLIB=OFF \
        -DLLVM_STATIC_LINK_CXX_STDLIB=ON \
        \
        `# Disable debugging features` \
        -DLLVM_ENABLE_ASSERTIONS=OFF \
        -DLLVM_ENABLE_EXPENSIVE_CHECKS=OFF \
        -DLLVM_ENABLE_CRASH_OVERRIDES=OFF \
        -DLLVM_ENABLE_BACKTRACES=OFF \
        \
        `# Disable optional dependencies` \
        -DLLVM_ENABLE_ZLIB=OFF \
        -DLLVM_ENABLE_ZSTD=OFF \
        -DLLVM_ENABLE_TERMINFO=OFF \
        -DLLVM_ENABLE_LIBXML2=OFF \
        -DLLVM_ENABLE_LIBEDIT=OFF \
        -DLLVM_ENABLE_LIBPFM=OFF \
        -DLLVM_ENABLE_FFI=OFF \
        \
        `# Disable unnecessary components` \
        -DLLVM_INCLUDE_TESTS=OFF \
        -DLLVM_INCLUDE_BENCHMARKS=OFF \
        -DLLVM_INCLUDE_EXAMPLES=OFF \
        -DLLVM_INCLUDE_DOCS=OFF \
        -DLLVM_INCLUDE_GO_TESTS=OFF \
        -DLLVM_BUILD_TOOLS=ON \
        -DLLVM_BUILD_UTILS=OFF \
        \
        `# MLIR configuration` \
        -DMLIR_ENABLE_BINDINGS_PYTHON=OFF \
        -DMLIR_BUILD_MLIR_C_DYLIB=OFF \
        \
        `# Build optimization` \
        -DLLVM_OPTIMIZED_TABLEGEN=ON \
        `# Note: Cannot use LLD during bootstrap - it is built as part of LLVM` \
        \
        `# Strip symbols from installed libraries` \
        -DCMAKE_INSTALL_DO_STRIP=ON \
        ${SIZE_FLAGS:+-DCMAKE_C_FLAGS="$SIZE_FLAGS" -DCMAKE_CXX_FLAGS="$SIZE_FLAGS"}

    log_success "Configuration complete"
}

# Build LLVM
build() {
    log_info "Building LLVM (this may take 30-60 minutes)..."

    local JOBS=$(detect_jobs)
    log_info "Using $JOBS parallel jobs"

    cmake --build "$BUILD_DIR" -j "$JOBS"

    log_success "Build complete"
}

# Install LLVM
install() {
    log_info "Installing to $INSTALL_DIR..."

    cmake --install "$BUILD_DIR"

    log_success "Installation complete"

    # Verify installation
    if [[ -f "$INSTALL_DIR/bin/llvm-config" ]]; then
        local VERSION=$("$INSTALL_DIR/bin/llvm-config" --version)
        log_success "LLVM $VERSION installed successfully"
    else
        log_error "llvm-config not found after installation"
        exit 1
    fi
}

# Clean build directory
clean() {
    log_info "Cleaning build directory..."
    rm -rf "$BUILD_DIR" "$INSTALL_DIR"
    log_success "Clean complete"
}

# Create tarball for release
package() {
    local OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    local ARCH=$(uname -m)

    # Normalize arch names
    case "$ARCH" in
        x86_64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="arm64" ;;
    esac

    local ARCHIVE_NAME="llvm-${LLVM_TAG#llvmorg-}-${OS}-${ARCH}.tar.xz"

    log_info "Creating release archive: $ARCHIVE_NAME"

    cd "$SCRIPT_DIR"
    tar -cJf "$ARCHIVE_NAME" -C "$INSTALL_DIR" .

    log_success "Archive created: $ARCHIVE_NAME ($(du -h "$ARCHIVE_NAME" | cut -f1))"
}

# Main entry point
main() {
    log_info "LLVM Build Script for Verum"
    log_info "=============================="

    parse_config

    # Handle arguments
    case "${1:-}" in
        --clean)
            clean
            exit 0
            ;;
        --package)
            package
            exit 0
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS] [TAG]"
            echo ""
            echo "Options:"
            echo "  --clean    Clean build directory"
            echo "  --package  Create release tarball"
            echo "  --help     Show this help"
            echo ""
            echo "Examples:"
            echo "  $0                      # Build using llvm.toml config"
            echo "  $0 llvmorg-21.0.0       # Build specific tag"
            exit 0
            ;;
        llvmorg-*)
            LLVM_TAG="$1"
            log_info "Overriding tag to: $LLVM_TAG"
            ;;
    esac

    log_info "Configuration:"
    log_info "  Tag:      $LLVM_TAG"
    log_info "  Projects: $LLVM_PROJECTS"
    log_info "  Targets:  $LLVM_TARGETS"
    log_info "  Type:     $BUILD_TYPE"
    echo ""

    # Check if already installed
    if [[ -f "$INSTALL_DIR/bin/llvm-config" ]]; then
        local INSTALLED_VERSION=$("$INSTALL_DIR/bin/llvm-config" --version)
        log_warn "LLVM $INSTALLED_VERSION already installed at $INSTALL_DIR"
        read -p "Rebuild? [y/N] " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 0
        fi
        clean
    fi

    clone_source
    configure
    build
    install

    log_success "LLVM build complete!"
    log_info "Install directory: $INSTALL_DIR"
    log_info ""
    log_info "To use in Rust builds, ensure this directory exists:"
    log_info "  export VERUM_LLVM_DIR=$INSTALL_DIR"
}

main "$@"
