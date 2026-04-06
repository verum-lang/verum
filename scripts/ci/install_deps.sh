#!/usr/bin/env bash
# Install system dependencies for CI environments
set -euo pipefail

OS="${1:-unknown}"

echo "Installing dependencies for: $OS"

case "$OS" in
  ubuntu-22.04)
    echo "Installing Ubuntu 22.04 dependencies..."
    sudo apt-get update
    sudo apt-get install -y \
      build-essential \
      pkg-config \
      libssl-dev \
      llvm-14-dev \
      libz3-dev \
      clang-14 \
      cmake \
      git
    llvm-config-14 --version
    z3 --version
    ;;

  ubuntu-24.04)
    echo "Installing Ubuntu 24.04 dependencies..."
    sudo apt-get update
    sudo apt-get install -y \
      build-essential \
      pkg-config \
      libssl-dev \
      llvm-16-dev \
      libz3-dev \
      clang-16 \
      cmake \
      git
    llvm-config-16 --version
    z3 --version
    ;;

  macos)
    echo "Installing macOS dependencies..."
    brew update
    brew install llvm@21 z3 pkg-config openssl cmake

    # Set environment variables
    echo "LLVM_SYS_210_PREFIX=$(brew --prefix llvm@21)" >> "$GITHUB_ENV"
    echo "$(brew --prefix llvm@21)/bin" >> "$GITHUB_PATH"

    llvm-config --version
    z3 --version
    ;;

  windows)
    echo "Installing Windows dependencies..."
    choco install llvm --version=21.1.0 -y
    choco install z3 -y
    choco install cmake -y
    refreshenv
    llvm-config --version || echo "LLVM installed"
    z3 --version || echo "Z3 installed"
    ;;

  *)
    echo "Unknown OS: $OS"
    exit 1
    ;;
esac

echo "Dependencies installed successfully!"
