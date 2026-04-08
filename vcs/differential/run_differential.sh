#!/usr/bin/env bash
#
# run_differential.sh - Compare VBC interpreter vs AOT compiled output
#
# Runs the same .vr program through both Tier 0 (interpreter) and Tier 1 (AOT/LLVM),
# then compares exit codes and stdout. They must match.
#
# Usage:
#   ./run_differential.sh <file.vr>           Run single file
#   ./run_differential.sh <directory>          Run all .vr files in directory
#   ./run_differential.sh                     Run all cross-impl/ tests
#
# Options:
#   -v, --verbose       Show stdout from both tiers on failure
#   -q, --quiet         Only show summary
#   -t, --timeout N     Timeout in seconds per tier (default: 30)
#   -j, --jobs N        Parallel jobs (default: 1, sequential)
#   --save-outputs      Save tier outputs to reports/
#   --only-failed       Only print failing tests
#   -h, --help          Show this help
#
# Exit codes:
#   0   All tests passed
#   1   One or more tests failed or had mismatches
#   2   Usage error

set -euo pipefail

# --------------------------------------------------------------------------
# Defaults
# --------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
VERUM_BIN="${VERUM_BIN:-$PROJECT_ROOT/target/release/verum}"

VERBOSE=0
QUIET=0
TIMEOUT=30
JOBS=1
SAVE_OUTPUTS=0
ONLY_FAILED=0
TEST_PATH=""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# Counters
TOTAL=0
PASSED=0
FAILED=0
ERRORS=0

# --------------------------------------------------------------------------
# Argument parsing
# --------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case $1 in
        -v|--verbose) VERBOSE=1; shift ;;
        -q|--quiet)   QUIET=1; shift ;;
        -t|--timeout) TIMEOUT="$2"; shift 2 ;;
        -j|--jobs)    JOBS="$2"; shift 2 ;;
        --save-outputs) SAVE_OUTPUTS=1; shift ;;
        --only-failed)  ONLY_FAILED=1; shift ;;
        -h|--help) head -25 "$0" | tail -23; exit 0 ;;
        -*) echo "Unknown option: $1" >&2; exit 2 ;;
        *)  TEST_PATH="$1"; shift ;;
    esac
done

# Default: run cross-impl directory
if [[ -z "$TEST_PATH" ]]; then
    TEST_PATH="$SCRIPT_DIR/cross-impl"
fi

# --------------------------------------------------------------------------
# Helpers
# --------------------------------------------------------------------------
log() {
    [[ $QUIET -eq 0 ]] && echo -e "$@"
}

log_pass() {
    [[ $ONLY_FAILED -eq 0 ]] && [[ $QUIET -eq 0 ]] && echo -e "${GREEN}  PASS${NC}  $1"
}

log_fail() {
    echo -e "${RED}  FAIL${NC}  $1"
}

log_mismatch() {
    echo -e "${YELLOW}  MISMATCH${NC}  $1"
}

log_error() {
    echo -e "${RED}  ERROR${NC}  $1"
}

# --------------------------------------------------------------------------
# Check verum binary
# --------------------------------------------------------------------------
if [[ ! -x "$VERUM_BIN" ]]; then
    # Try debug build
    if [[ -x "$PROJECT_ROOT/target/debug/verum" ]]; then
        VERUM_BIN="$PROJECT_ROOT/target/debug/verum"
    else
        echo -e "${RED}Error:${NC} verum binary not found at $VERUM_BIN"
        echo "Build with: cargo build --release -p verum_cli"
        echo "Or set VERUM_BIN=/path/to/verum"
        exit 2
    fi
fi

# --------------------------------------------------------------------------
# Run a single differential test
#   Compares: verum run --tier interpreter <file>
#        vs:  verum run --tier aot <file>
# --------------------------------------------------------------------------
run_one_test() {
    local test_file="$1"
    local test_name
    test_name="$(basename "$test_file" .vr)"

    local tmpdir
    tmpdir="$(mktemp -d)"

    local interp_stdout="$tmpdir/interp_stdout.txt"
    local interp_stderr="$tmpdir/interp_stderr.txt"
    local aot_stdout="$tmpdir/aot_stdout.txt"
    local aot_stderr="$tmpdir/aot_stderr.txt"

    local interp_exit=0
    local aot_exit=0

    # --- Tier 0: interpreter ---
    timeout "$TIMEOUT" "$VERUM_BIN" run --tier interpreter "$test_file" \
        >"$interp_stdout" 2>"$interp_stderr" || interp_exit=$?

    # --- Tier 1: AOT ---
    timeout "$TIMEOUT" "$VERUM_BIN" run --tier aot "$test_file" \
        >"$aot_stdout" 2>"$aot_stderr" || aot_exit=$?

    # Handle timeout (exit 124 on GNU coreutils, 137 on signal)
    if [[ $interp_exit -eq 124 || $interp_exit -eq 137 ]]; then
        log_error "$test_name  (interpreter timed out after ${TIMEOUT}s)"
        ((ERRORS++)) || true
        rm -rf "$tmpdir"
        return 1
    fi
    if [[ $aot_exit -eq 124 || $aot_exit -eq 137 ]]; then
        log_error "$test_name  (AOT timed out after ${TIMEOUT}s)"
        ((ERRORS++)) || true
        rm -rf "$tmpdir"
        return 1
    fi

    # --- Compare ---
    local result="PASS"
    local detail=""

    # 1. Exit codes must match
    if [[ $interp_exit -ne $aot_exit ]]; then
        result="MISMATCH"
        detail="exit code: interpreter=$interp_exit, aot=$aot_exit"
    fi

    # 2. Stdout must match (byte-for-byte)
    if [[ "$result" == "PASS" ]]; then
        if ! diff -q "$interp_stdout" "$aot_stdout" >/dev/null 2>&1; then
            result="MISMATCH"
            detail="stdout differs"
        fi
    fi

    # --- Report ---
    case "$result" in
        PASS)
            ((PASSED++)) || true
            log_pass "$test_name  (interp=$interp_exit, aot=$aot_exit)"
            ;;
        MISMATCH)
            ((FAILED++)) || true
            log_mismatch "$test_name  -- $detail"
            if [[ $VERBOSE -eq 1 ]]; then
                echo "    --- interpreter stdout ---"
                head -20 "$interp_stdout" | sed 's/^/    /'
                echo "    --- aot stdout ---"
                head -20 "$aot_stdout" | sed 's/^/    /'
                if [[ -n "$(cat "$interp_stderr")" ]]; then
                    echo "    --- interpreter stderr ---"
                    head -10 "$interp_stderr" | sed 's/^/    /'
                fi
                if [[ -n "$(cat "$aot_stderr")" ]]; then
                    echo "    --- aot stderr ---"
                    head -10 "$aot_stderr" | sed 's/^/    /'
                fi
            fi
            ;;
    esac

    # Save outputs
    if [[ $SAVE_OUTPUTS -eq 1 ]]; then
        local outdir="$SCRIPT_DIR/reports/$test_name"
        mkdir -p "$outdir"
        cp "$interp_stdout" "$outdir/interp_stdout.txt"
        cp "$interp_stderr" "$outdir/interp_stderr.txt"
        cp "$aot_stdout"    "$outdir/aot_stdout.txt"
        cp "$aot_stderr"    "$outdir/aot_stderr.txt"
        echo "interp_exit=$interp_exit" > "$outdir/metadata.txt"
        echo "aot_exit=$aot_exit" >> "$outdir/metadata.txt"
        echo "result=$result" >> "$outdir/metadata.txt"
    fi

    rm -rf "$tmpdir"
    [[ "$result" == "PASS" ]]
}

# --------------------------------------------------------------------------
# Collect test files
# --------------------------------------------------------------------------
collect_tests() {
    local path="$1"
    if [[ -f "$path" ]]; then
        echo "$path"
    elif [[ -d "$path" ]]; then
        find "$path" -name '*.vr' -type f | sort
    else
        echo "Path not found: $path" >&2
        exit 2
    fi
}

# --------------------------------------------------------------------------
# Main
# --------------------------------------------------------------------------
main() {
    log "${BOLD}Differential Testing: Interpreter (Tier 0) vs AOT (Tier 1)${NC}"
    log "  verum binary: $VERUM_BIN"
    log "  test path:    $TEST_PATH"
    log "  timeout:      ${TIMEOUT}s"
    log ""

    local tests=()
    while IFS= read -r f; do
        tests+=("$f")
    done < <(collect_tests "$TEST_PATH")

    TOTAL=${#tests[@]}
    if [[ $TOTAL -eq 0 ]]; then
        echo "No .vr test files found in $TEST_PATH"
        exit 2
    fi

    log "  found $TOTAL test file(s)"
    log ""

    for test_file in "${tests[@]}"; do
        run_one_test "$test_file" || true
    done

    # --- Summary ---
    echo ""
    echo -e "${BOLD}============================================${NC}"
    echo -e "${BOLD}  Differential Test Summary${NC}"
    echo -e "${BOLD}============================================${NC}"
    echo -e "  Total:    $TOTAL"
    echo -e "  ${GREEN}Passed:   $PASSED${NC}"
    if [[ $FAILED -gt 0 ]]; then
        echo -e "  ${RED}Failed:   $FAILED${NC}"
    else
        echo -e "  Failed:   0"
    fi
    if [[ $ERRORS -gt 0 ]]; then
        echo -e "  ${YELLOW}Errors:   $ERRORS${NC}"
    fi
    echo -e "${BOLD}============================================${NC}"

    [[ $FAILED -eq 0 && $ERRORS -eq 0 ]]
}

main
