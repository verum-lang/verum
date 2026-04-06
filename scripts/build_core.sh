#!/usr/bin/env bash
# Build Script for Verum Standard Library
#
# This script compiles the Verum standard library from .vr source files.
# For now, it performs parse and type-checking. Code generation will be added later.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
STDLIB_DIR="${PROJECT_ROOT}/core"
COMPILER="${PROJECT_ROOT}/target/debug/verum"

# Build flags
VERBOSE=0
CHECK_ONLY=1  # For now, only check (no codegen)

# Print usage
usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Build the Verum standard library.

OPTIONS:
    -h, --help       Show this help message
    -v, --verbose    Enable verbose output
    -c, --codegen    Enable code generation (default: check only)
    --release        Use release build of compiler

EXAMPLES:
    $0                # Check core
    $0 -v             # Check with verbose output
    $0 -c             # Check and generate code
    $0 --release -c   # Use release compiler and generate code
EOF
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -h|--help)
                usage
                exit 0
                ;;
            -v|--verbose)
                VERBOSE=1
                shift
                ;;
            -c|--codegen)
                CHECK_ONLY=0
                shift
                ;;
            --release)
                COMPILER="${PROJECT_ROOT}/target/release/verum"
                shift
                ;;
            *)
                echo -e "${RED}Error: Unknown option: $1${NC}" >&2
                usage
                exit 1
                ;;
        esac
    done
}

# Log functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*" >&2
}

# Build the compiler if needed
build_compiler() {
    log_info "Building Verum compiler..."

    local build_type="debug"
    if [[ "$COMPILER" == *"release"* ]]; then
        build_type="release"
    fi

    cd "$PROJECT_ROOT"
    if [[ "$build_type" == "release" ]]; then
        cargo build --release --bin verum
    else
        cargo build --bin verum
    fi

    if [[ ! -x "$COMPILER" ]]; then
        log_error "Compiler not found at: $COMPILER"
        log_info "Did the build succeed?"
        exit 1
    fi

    log_success "Compiler built: $COMPILER"
}

# Check if core directory exists
check_core_dir() {
    if [[ ! -d "$STDLIB_DIR" ]]; then
        log_warn "Standard library directory not found: $STDLIB_DIR"
        log_info "Creating placeholder core directory..."
        mkdir -p "$STDLIB_DIR"

        # Create a simple placeholder module
        cat > "$STDLIB_DIR/core.vr" <<'EOF'
// Verum Standard Library - Core Module
// This is a placeholder until the full core is implemented

// Basic types are built-in:
// - Int, Float, Bool, Text, Char
// - List<T>, Map<K, V>, Set<T>
// - Maybe<T>, Result<T, E>

// Core functions (to be implemented)
fn identity<T>(x: T) -> T {
    x
}

pub fn main() {
    // Placeholder
}
EOF
        log_info "Created placeholder: $STDLIB_DIR/core.vr"
    fi
}

# Compile a single core file
compile_file() {
    local file="$1"
    local rel_path="${file#$STDLIB_DIR/}"

    log_info "Checking: $rel_path"

    local cmd_args=("check" "$file")
    if [[ $VERBOSE -eq 1 ]]; then
        cmd_args+=("-v")
    fi

    if [[ $CHECK_ONLY -eq 0 ]]; then
        # Build mode (when implemented)
        cmd_args=("build" "$file")
        if [[ $VERBOSE -eq 1 ]]; then
            cmd_args+=("-v")
        fi
    fi

    if "$COMPILER" "${cmd_args[@]}"; then
        log_success "✓ $rel_path"
        return 0
    else
        log_error "✗ $rel_path"
        return 1
    fi
}

# Compile all core files
compile_core() {
    log_info "Compiling Verum standard library..."
    log_info "Mode: $([ $CHECK_ONLY -eq 1 ] && echo "CHECK ONLY" || echo "BUILD")"
    echo ""

    # Find all .vr files in core
    local verum_files=()
    while IFS= read -r -d '' file; do
        verum_files+=("$file")
    done < <(find "$STDLIB_DIR" -name "*.vr" -print0 | sort -z)

    if [[ ${#verum_files[@]} -eq 0 ]]; then
        log_warn "No .vr files found in $STDLIB_DIR"
        return 0
    fi

    log_info "Found ${#verum_files[@]} core file(s)"
    echo ""

    local success_count=0
    local fail_count=0

    for file in "${verum_files[@]}"; do
        if compile_file "$file"; then
            ((success_count++))
        else
            ((fail_count++))
        fi
    done

    echo ""
    log_info "Results: $success_count succeeded, $fail_count failed"

    if [[ $fail_count -eq 0 ]]; then
        log_success "All core files compiled successfully!"
        return 0
    else
        log_error "Some core files failed to compile"
        return 1
    fi
}

# Main execution
main() {
    parse_args "$@"

    echo -e "${BLUE}╔═══════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║   Verum Standard Library Build Script   ║${NC}"
    echo -e "${BLUE}╚═══════════════════════════════════════╝${NC}"
    echo ""

    # Build compiler
    build_compiler
    echo ""

    # Check core directory
    check_core_dir
    echo ""

    # Compile core
    if compile_core; then
        echo ""
        log_success "Standard library build complete!"
        exit 0
    else
        echo ""
        log_error "Standard library build failed"
        exit 1
    fi
}

# Run main
main "$@"
