#!/usr/bin/env bash
# =============================================================================
# WASM End-to-End Test Runner
# =============================================================================
#
# Compiles Verum .vr files to WebAssembly (wasm32-wasi) and executes them
# using wasmtime or wasmer.
#
# Usage:
#   ./run_wasm_tests.sh                    # Run all tests
#   ./run_wasm_tests.sh wasm_arithmetic.vr # Run single test
#   WASM_RUNTIME=wasmer ./run_wasm_tests.sh
#
# Requirements:
#   - verum compiler built with target-wasm feature
#   - wasmtime or wasmer installed
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
BUILD_DIR="$SCRIPT_DIR/target/build"
VERUM_BIN="${VERUM_BIN:-$PROJECT_ROOT/target/release/verum}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Counters
TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

# =============================================================================
# Detect WASM runtime
# =============================================================================

detect_wasm_runtime() {
    if [[ -n "${WASM_RUNTIME:-}" ]]; then
        if command -v "$WASM_RUNTIME" &>/dev/null; then
            echo "$WASM_RUNTIME"
            return 0
        fi
        echo "ERROR: Specified WASM_RUNTIME=$WASM_RUNTIME not found" >&2
        return 1
    fi

    # Prefer wasmtime (WASI-native), fall back to wasmer
    if command -v wasmtime &>/dev/null; then
        echo "wasmtime"
    elif command -v wasmer &>/dev/null; then
        echo "wasmer"
    else
        echo ""
    fi
}

# =============================================================================
# Parse test metadata from .vr file
# =============================================================================

parse_test_meta() {
    local file="$1"
    local key="$2"
    grep -m1 "^// @${key}:" "$file" 2>/dev/null | sed "s|^// @${key}: *||" || echo ""
}

# =============================================================================
# Run a single WASM test
# =============================================================================

run_test() {
    local vr_file="$1"
    local test_name="$(basename "$vr_file" .vr)"
    local wasm_file="$BUILD_DIR/${test_name}.wasm"

    TOTAL=$((TOTAL + 1))

    # Parse metadata
    local requires
    requires="$(parse_test_meta "$vr_file" "requires")"
    local expected_exit
    expected_exit="$(parse_test_meta "$vr_file" "expected-exit")"
    local expected_stdout
    expected_stdout="$(parse_test_meta "$vr_file" "expected-stdout")"

    # Check requirements
    if echo "$requires" | grep -q "wasi" && [[ "$RUNTIME" != "wasmtime" ]]; then
        # wasmer supports WASI too, but wasmtime is canonical
        if [[ "$RUNTIME" != "wasmer" ]]; then
            printf "  ${YELLOW}SKIP${NC}  %-30s (requires WASI runtime)\n" "$test_name"
            SKIPPED=$((SKIPPED + 1))
            return 0
        fi
    fi

    # Step 1: Compile to WASM
    printf "  ${CYAN}BUILD${NC} %-30s " "$test_name"

    local compile_output
    if ! compile_output=$("$VERUM_BIN" build "$vr_file" \
        --target wasm32-wasi \
        --output "$wasm_file" \
        --opt-level 1 2>&1); then
        printf "${RED}COMPILE FAIL${NC}\n"
        if [[ -n "${VERBOSE:-}" ]]; then
            echo "    $compile_output" | head -20
        fi
        FAILED=$((FAILED + 1))
        return 0
    fi

    if [[ ! -f "$wasm_file" ]]; then
        printf "${RED}NO OUTPUT${NC}\n"
        FAILED=$((FAILED + 1))
        return 0
    fi

    printf "-> "

    # Step 2: Execute with WASM runtime
    local run_output
    local actual_exit=0

    case "$RUNTIME" in
        wasmtime)
            run_output=$(wasmtime run "$wasm_file" 2>&1) || actual_exit=$?
            ;;
        wasmer)
            run_output=$(wasmer run "$wasm_file" 2>&1) || actual_exit=$?
            ;;
        *)
            printf "${YELLOW}SKIP (no runtime)${NC}\n"
            SKIPPED=$((SKIPPED + 1))
            return 0
            ;;
    esac

    # Step 3: Validate results
    local test_passed=true

    # Check exit code
    if [[ -n "$expected_exit" ]] && [[ "$actual_exit" != "$expected_exit" ]]; then
        test_passed=false
    fi

    # Check stdout
    if [[ -n "$expected_stdout" ]]; then
        if ! echo "$run_output" | grep -qF "$expected_stdout"; then
            test_passed=false
        fi
    fi

    if $test_passed; then
        printf "${GREEN}PASS${NC}  (exit=$actual_exit)\n"
        PASSED=$((PASSED + 1))
    else
        printf "${RED}FAIL${NC}  (exit=$actual_exit, expected=$expected_exit)\n"
        if [[ -n "${VERBOSE:-}" ]]; then
            echo "    stdout: $(echo "$run_output" | head -5)"
            if [[ -n "$expected_stdout" ]]; then
                echo "    expected stdout: $expected_stdout"
            fi
        fi
        FAILED=$((FAILED + 1))
    fi
}

# =============================================================================
# Main
# =============================================================================

main() {
    echo "============================================="
    echo "  Verum WASM End-to-End Test Suite"
    echo "============================================="
    echo ""

    # Check compiler
    if [[ ! -x "$VERUM_BIN" ]]; then
        echo "ERROR: Verum compiler not found at $VERUM_BIN"
        echo "Build with: cargo build --release -p verum_cli --features target-wasm"
        exit 1
    fi

    # Detect runtime
    RUNTIME="$(detect_wasm_runtime)"
    if [[ -z "$RUNTIME" ]]; then
        echo "WARNING: No WASM runtime found (wasmtime or wasmer)"
        echo "Install: curl https://wasmtime.dev/install.sh -sSf | bash"
        echo "Tests will compile but not execute."
        echo ""
    else
        echo "WASM runtime: $RUNTIME ($(command -v "$RUNTIME"))"
        echo ""
    fi

    # Create build directory
    mkdir -p "$BUILD_DIR"

    # Collect test files
    local test_files=()
    if [[ $# -gt 0 ]]; then
        # Run specific tests
        for arg in "$@"; do
            if [[ -f "$SCRIPT_DIR/$arg" ]]; then
                test_files+=("$SCRIPT_DIR/$arg")
            elif [[ -f "$arg" ]]; then
                test_files+=("$arg")
            else
                echo "WARNING: Test file not found: $arg"
            fi
        done
    else
        # Run all .vr files in this directory
        for f in "$SCRIPT_DIR"/wasm_*.vr; do
            [[ -f "$f" ]] && test_files+=("$f")
        done
    fi

    if [[ ${#test_files[@]} -eq 0 ]]; then
        echo "No test files found."
        exit 1
    fi

    echo "Running ${#test_files[@]} test(s)..."
    echo "---------------------------------------------"

    for vr_file in "${test_files[@]}"; do
        run_test "$vr_file"
    done

    echo "---------------------------------------------"
    echo ""
    echo "Results: ${TOTAL} total, ${GREEN}${PASSED} passed${NC}, ${RED}${FAILED} failed${NC}, ${YELLOW}${SKIPPED} skipped${NC}"
    echo ""

    if [[ $FAILED -gt 0 ]]; then
        exit 1
    fi
}

main "$@"
