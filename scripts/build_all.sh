#!/bin/bash
# Comprehensive build script for Verum (Linux/macOS)
# Builds all crates with proper error handling and diagnostics

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BUILD_TYPE="${BUILD_TYPE:-release}"
PARALLEL_JOBS="${PARALLEL_JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
RUN_TESTS="${RUN_TESTS:-yes}"
RUN_BENCHES="${RUN_BENCHES:-no}"
GEN_DOCS="${GEN_DOCS:-no}"

echo_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

echo_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

echo_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

echo_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Fix macOS TMPDIR issue if on macOS
fix_tmpdir_if_needed() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        echo_info "Detected macOS, checking TMPDIR..."

        if [ -n "${TMPDIR:-}" ] && [ ! -w "$TMPDIR" ]; then
            echo_warn "TMPDIR ($TMPDIR) is not writable"

            # Get user-accessible temp directory
            USER_TMPDIR=$(getconf DARWIN_USER_TEMP_DIR 2>/dev/null || echo "/tmp")

            if [ -w "$USER_TMPDIR" ]; then
                echo_info "Setting TMPDIR to: $USER_TMPDIR"
                export TMPDIR="$USER_TMPDIR"
            else
                echo_error "Cannot find writable temp directory"
                echo_error "Please run: source scripts/fix_macos_tmpdir.sh"
                exit 1
            fi
        else
            echo_info "TMPDIR is OK: ${TMPDIR:-<not set>}"
        fi
    fi
}

check_dependencies() {
    echo_step "Checking dependencies..."

    local missing_deps=()

    if ! command -v rustc &> /dev/null; then
        missing_deps+=("rust")
    fi

    if ! command -v llvm-config &> /dev/null; then
        missing_deps+=("llvm")
    fi

    if ! command -v z3 &> /dev/null; then
        missing_deps+=("z3")
    fi

    if [ ${#missing_deps[@]} -gt 0 ]; then
        echo_error "Missing dependencies: ${missing_deps[*]}"
        echo_error "Run 'scripts/setup.sh' to install all dependencies"
        return 1
    fi

    echo_info "✓ All dependencies are available"

    # Print versions
    echo_info "Rust: $(rustc --version)"
    echo_info "LLVM: $(llvm-config --version)"
    echo_info "Z3: $(z3 --version | head -1)"
}

clean_build() {
    echo_step "Cleaning build artifacts..."
    cargo clean
    echo_info "✓ Clean complete"
}

build_workspace() {
    echo_step "Building Verum workspace..."

    local build_flags=()

    if [ "$BUILD_TYPE" = "release" ]; then
        build_flags+=("--release")
        echo_info "Building in RELEASE mode"
    else
        echo_info "Building in DEBUG mode"
    fi

    build_flags+=("-j" "$PARALLEL_JOBS")

    echo_info "Parallel jobs: $PARALLEL_JOBS"
    echo_info "Build flags: ${build_flags[*]}"

    # Build with proper error handling
    if cargo build "${build_flags[@]}" 2>&1 | tee build.log; then
        echo_info "✓ Build successful"
        return 0
    else
        echo_error "✗ Build failed"
        echo_error "Check build.log for details"
        return 1
    fi
}

run_tests() {
    if [ "$RUN_TESTS" != "yes" ]; then
        echo_info "Skipping tests (RUN_TESTS=$RUN_TESTS)"
        return 0
    fi

    echo_step "Running tests..."

    local test_flags=()

    if [ "$BUILD_TYPE" = "release" ]; then
        test_flags+=("--release")
    fi

    test_flags+=("-j" "$PARALLEL_JOBS")

    if cargo test "${test_flags[@]}" --workspace 2>&1 | tee test.log; then
        echo_info "✓ All tests passed"
        return 0
    else
        echo_error "✗ Some tests failed"
        echo_error "Check test.log for details"
        return 1
    fi
}

run_benches() {
    if [ "$RUN_BENCHES" != "yes" ]; then
        echo_info "Skipping benchmarks (RUN_BENCHES=$RUN_BENCHES)"
        return 0
    fi

    echo_step "Running benchmarks..."

    if cargo bench --workspace 2>&1 | tee bench.log; then
        echo_info "✓ Benchmarks complete"
        return 0
    else
        echo_warn "Some benchmarks failed"
        return 1
    fi
}

generate_docs() {
    if [ "$GEN_DOCS" != "yes" ]; then
        echo_info "Skipping documentation (GEN_DOCS=$GEN_DOCS)"
        return 0
    fi

    echo_step "Generating documentation..."

    if cargo doc --workspace --no-deps 2>&1 | tee doc.log; then
        echo_info "✓ Documentation generated"
        echo_info "Open: target/doc/verum_cli/index.html"
        return 0
    else
        echo_warn "Documentation generation had warnings"
        return 0 # Don't fail on doc warnings
    fi
}

print_build_summary() {
    echo ""
    echo_info "=== Build Summary ==="
    echo "Build type: $BUILD_TYPE"
    echo "Parallel jobs: $PARALLEL_JOBS"
    echo "Tests: $RUN_TESTS"
    echo "Benchmarks: $RUN_BENCHES"
    echo "Documentation: $GEN_DOCS"
    echo ""

    if [ "$BUILD_TYPE" = "release" ]; then
        local binary_path="target/release/verum"
    else
        local binary_path="target/debug/verum"
    fi

    if [ -f "$binary_path" ]; then
        echo_info "Binary location: $binary_path"
        echo_info "Binary size: $(du -h "$binary_path" | cut -f1)"
    fi

    echo ""
}

print_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -c, --clean          Clean build artifacts before building"
    echo "  -r, --release        Build in release mode (default)"
    echo "  -d, --debug          Build in debug mode"
    echo "  -t, --test           Run tests after building (default: yes)"
    echo "  -T, --no-test        Skip tests"
    echo "  -b, --bench          Run benchmarks"
    echo "  -D, --docs           Generate documentation"
    echo "  -j, --jobs N         Number of parallel jobs (default: auto)"
    echo "  -h, --help           Show this help message"
    echo ""
    echo "Environment variables:"
    echo "  BUILD_TYPE=release|debug"
    echo "  RUN_TESTS=yes|no"
    echo "  RUN_BENCHES=yes|no"
    echo "  GEN_DOCS=yes|no"
    echo "  PARALLEL_JOBS=N"
    echo ""
    echo "Examples:"
    echo "  $0                    # Build in release mode with tests"
    echo "  $0 -d -t              # Build in debug mode with tests"
    echo "  $0 -c -r -b -D        # Clean, release build, bench, and docs"
}

main() {
    local clean_first=false

    # Parse arguments
    while [[ $# -gt 0 ]]; then
        case $1 in
            -c|--clean)
                clean_first=true
                shift
                ;;
            -r|--release)
                BUILD_TYPE="release"
                shift
                ;;
            -d|--debug)
                BUILD_TYPE="debug"
                shift
                ;;
            -t|--test)
                RUN_TESTS="yes"
                shift
                ;;
            -T|--no-test)
                RUN_TESTS="no"
                shift
                ;;
            -b|--bench)
                RUN_BENCHES="yes"
                shift
                ;;
            -D|--docs)
                GEN_DOCS="yes"
                shift
                ;;
            -j|--jobs)
                PARALLEL_JOBS="$2"
                shift 2
                ;;
            -h|--help)
                print_usage
                exit 0
                ;;
            *)
                echo_error "Unknown option: $1"
                print_usage
                exit 1
                ;;
        esac
    done

    echo_info "=== Verum Build Script ==="
    echo ""

    # Check if we're in the right directory
    if [ ! -f "Cargo.toml" ]; then
        echo_error "Cargo.toml not found. Please run this script from the project root."
        exit 1
    fi

    # Fix TMPDIR if needed
    fix_tmpdir_if_needed

    # Check dependencies
    check_dependencies || exit 1

    # Clean if requested
    if [ "$clean_first" = true ]; then
        clean_build
    fi

    # Build
    echo ""
    if ! build_workspace; then
        echo_error "Build failed"
        exit 1
    fi

    # Run tests
    echo ""
    if ! run_tests; then
        echo_error "Tests failed"
        exit 1
    fi

    # Run benchmarks
    echo ""
    run_benches

    # Generate documentation
    echo ""
    generate_docs

    # Print summary
    print_build_summary

    echo_info "=== Build Complete ==="
}

main "$@"
