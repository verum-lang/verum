#!/bin/bash
# Cross-platform setup script for Verum (Linux/macOS)
# Installs all required dependencies for building Verum

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

echo_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

echo_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

check_command() {
    if command -v "$1" &> /dev/null; then
        echo_info "✓ $1 is installed"
        return 0
    else
        echo_warn "✗ $1 is not installed"
        return 1
    fi
}

detect_os() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        echo "macos"
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        echo "linux"
    else
        echo "unknown"
    fi
}

install_rust() {
    echo_info "Installing Rust toolchain..."
    if ! check_command rustc; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi

    # Update to required version
    rustup update
    rustup default stable

    echo_info "Rust version: $(rustc --version)"
}

install_llvm_macos() {
    echo_info "Installing LLVM on macOS..."

    if ! check_command brew; then
        echo_error "Homebrew is required but not installed"
        echo_error "Install from: https://brew.sh"
        exit 1
    fi

    # Install LLVM (version 21 preferred, but 8-21 supported)
    if ! brew list llvm &>/dev/null; then
        brew install llvm@21 || brew install llvm
    fi

    # Add LLVM to PATH
    LLVM_PATH=$(brew --prefix llvm)
    export PATH="$LLVM_PATH/bin:$PATH"
    export LDFLAGS="-L$LLVM_PATH/lib"
    export CPPFLAGS="-I$LLVM_PATH/include"

    echo_info "LLVM installed at: $LLVM_PATH"
    echo_info "Add to your shell profile (~/.zshrc or ~/.bashrc):"
    echo "  export PATH=\"$LLVM_PATH/bin:\$PATH\""
    echo "  export LDFLAGS=\"-L$LLVM_PATH/lib\""
    echo "  export CPPFLAGS=\"-I$LLVM_PATH/include\""
}

install_llvm_linux() {
    echo_info "Installing LLVM on Linux..."

    if check_command apt-get; then
        # Debian/Ubuntu
        sudo apt-get update
        sudo apt-get install -y llvm-21-dev libllvm21 llvm-21 || \
        sudo apt-get install -y llvm-dev libllvm llvm
    elif check_command yum; then
        # RHEL/CentOS/Fedora
        sudo yum install -y llvm-devel llvm || \
        sudo dnf install -y llvm-devel llvm
    elif check_command pacman; then
        # Arch Linux
        sudo pacman -S --noconfirm llvm
    else
        echo_error "Unsupported package manager"
        echo_error "Please install LLVM manually: https://llvm.org/docs/GettingStarted.html"
        exit 1
    fi
}

install_z3_macos() {
    echo_info "Installing Z3 on macOS..."

    if ! brew list z3 &>/dev/null; then
        brew install z3
    fi

    Z3_PATH=$(brew --prefix z3)
    export PKG_CONFIG_PATH="$Z3_PATH/lib/pkgconfig:$PKG_CONFIG_PATH"

    echo_info "Z3 installed at: $Z3_PATH"
}

install_z3_linux() {
    echo_info "Installing Z3 on Linux..."

    if check_command apt-get; then
        # Debian/Ubuntu
        sudo apt-get install -y libz3-dev z3
    elif check_command yum; then
        # RHEL/CentOS/Fedora
        sudo yum install -y z3-devel z3 || \
        sudo dnf install -y z3-devel z3
    elif check_command pacman; then
        # Arch Linux
        sudo pacman -S --noconfirm z3
    else
        echo_error "Unsupported package manager"
        echo_error "Please install Z3 manually: https://github.com/Z3Prover/z3"
        exit 1
    fi
}

install_dependencies_macos() {
    echo_info "Installing additional macOS dependencies..."

    # Install pkg-config if not present
    if ! check_command pkg-config; then
        brew install pkg-config
    fi

    # Install OpenSSL
    if ! brew list openssl &>/dev/null; then
        brew install openssl
    fi

    # Fix macOS TMPDIR issue
    echo_info "Fixing macOS TMPDIR issue..."
    if [ -f "scripts/fix_macos_tmpdir.sh" ]; then
        source scripts/fix_macos_tmpdir.sh
    fi
}

install_dependencies_linux() {
    echo_info "Installing additional Linux dependencies..."

    if check_command apt-get; then
        sudo apt-get install -y \
            build-essential \
            pkg-config \
            libssl-dev \
            cmake \
            git
    elif check_command yum; then
        sudo yum groupinstall -y "Development Tools" || \
        sudo dnf groupinstall -y "Development Tools"
        sudo yum install -y \
            pkg-config \
            openssl-devel \
            cmake \
            git || \
        sudo dnf install -y \
            pkg-config \
            openssl-devel \
            cmake \
            git
    elif check_command pacman; then
        sudo pacman -S --noconfirm \
            base-devel \
            pkg-config \
            openssl \
            cmake \
            git
    fi
}

verify_installation() {
    echo_info "Verifying installation..."

    local all_good=true

    if ! check_command rustc; then
        echo_error "Rust is not installed"
        all_good=false
    fi

    if ! check_command llvm-config; then
        echo_error "LLVM is not installed or not in PATH"
        all_good=false
    fi

    if ! check_command z3; then
        echo_error "Z3 is not installed or not in PATH"
        all_good=false
    fi

    if ! check_command pkg-config; then
        echo_error "pkg-config is not installed"
        all_good=false
    fi

    if [ "$all_good" = true ]; then
        echo_info "✓ All dependencies are installed"
        return 0
    else
        echo_error "✗ Some dependencies are missing"
        return 1
    fi
}

print_environment_info() {
    echo ""
    echo_info "=== Environment Information ==="
    echo "Rust version: $(rustc --version 2>/dev/null || echo 'not installed')"
    echo "Cargo version: $(cargo --version 2>/dev/null || echo 'not installed')"
    echo "LLVM version: $(llvm-config --version 2>/dev/null || echo 'not installed')"
    echo "Z3 version: $(z3 --version 2>/dev/null || echo 'not installed')"
    echo "pkg-config version: $(pkg-config --version 2>/dev/null || echo 'not installed')"
    echo ""
}

main() {
    echo_info "=== Verum Setup Script ==="
    echo_info "This script will install all dependencies required to build Verum"
    echo ""

    OS=$(detect_os)
    echo_info "Detected OS: $OS"
    echo ""

    if [ "$OS" = "unknown" ]; then
        echo_error "Unsupported operating system: $OSTYPE"
        echo_error "This script supports macOS and Linux only"
        exit 1
    fi

    # Install Rust
    install_rust

    # Install LLVM
    if [ "$OS" = "macos" ]; then
        install_llvm_macos
    else
        install_llvm_linux
    fi

    # Install Z3
    if [ "$OS" = "macos" ]; then
        install_z3_macos
    else
        install_z3_linux
    fi

    # Install other dependencies
    if [ "$OS" = "macos" ]; then
        install_dependencies_macos
    else
        install_dependencies_linux
    fi

    # Verify installation
    echo ""
    verify_installation

    # Print environment info
    print_environment_info

    echo_info "=== Setup Complete ==="
    echo_info "You can now build Verum with: cargo build"
    echo ""

    if [ "$OS" = "macos" ]; then
        echo_warn "Note: On macOS, you may need to set TMPDIR before building:"
        echo "  source scripts/fix_macos_tmpdir.sh"
        echo "  cargo build"
    fi
}

main "$@"
