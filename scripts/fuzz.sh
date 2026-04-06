#!/bin/bash

# Fuzzing script for Verum language components
# Usage: ./scripts/fuzz.sh <crate> <target> [duration]
#
# Examples:
#   ./scripts/fuzz.sh verum_parser fuzz_target_1 3600
#   ./scripts/fuzz.sh verum_lexer fuzz_target_1 7200
#   ./scripts/fuzz.sh verum_cbgr fuzz_target_allocate_deallocate 1800

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

CRATE="${1:-}"
TARGET="${2:-}"
DURATION="${3:-3600}"  # Default 1 hour

if [ -z "$CRATE" ] || [ -z "$TARGET" ]; then
    echo "Usage: $0 <crate> <target> [duration_seconds]"
    echo ""
    echo "Available Crates and Targets:"
    echo "  verum_parser:"
    echo "    - fuzz_target_1 (module parsing)"
    echo ""
    echo "  verum_lexer:"
    echo "    - fuzz_target_1 (tokenization)"
    echo ""
    echo "  verum_cbgr:"
    echo "    - fuzz_target_allocate_deallocate (memory allocation patterns)"
    echo "    - fuzz_target_concurrent_access (concurrent memory access)"
    echo "    - fuzz_target_wraparound (epoch wraparound safety)"
    echo "    - fuzz_target_capabilities (generation reference capabilities)"
    echo ""
    echo "Examples:"
    echo "  $0 verum_parser fuzz_target_1 3600"
    echo "  $0 verum_lexer fuzz_target_1 7200"
    echo "  $0 verum_cbgr fuzz_target_allocate_deallocate 1800"
    exit 1
fi

# Validate crate exists
CRATE_PATH="$PROJECT_ROOT/crates/$CRATE"
if [ ! -d "$CRATE_PATH" ]; then
    echo "Error: Crate '$CRATE' not found at $CRATE_PATH"
    exit 1
fi

# Check for fuzz directory
if [ ! -d "$CRATE_PATH/fuzz" ]; then
    echo "Error: Fuzzing not initialized for $CRATE"
    echo "Run: cd $CRATE_PATH && cargo fuzz init"
    exit 1
fi

echo "========================================================================"
echo "Fuzzing Configuration"
echo "========================================================================"
echo "Crate:      $CRATE"
echo "Target:     $TARGET"
echo "Duration:   ${DURATION}s ($(( DURATION / 60 )) minutes)"
echo "Corpus:     $CRATE_PATH/fuzz/corpus/$TARGET"
echo "Artifacts:  $CRATE_PATH/fuzz/artifacts/$TARGET"
echo "========================================================================"
echo ""

cd "$CRATE_PATH"

# Create corpus directory if it doesn't exist
mkdir -p "fuzz/corpus/$TARGET"

echo "Starting fuzzer in $(pwd)..."
echo "Press Ctrl+C to stop"
echo ""

# Run cargo fuzz with time limit
# ASAN_OPTIONS controls AddressSanitizer behavior
export ASAN_OPTIONS="quarantine_size_mb=10:halt_on_error=1"
export UBSAN_OPTIONS="print_stacktrace=1"

cargo +nightly fuzz run "$TARGET" \
    --release \
    -- \
    -max_total_time="$DURATION" \
    -timeout=10 \
    -rss_limit_mb=4096 \
    -artifact_prefix="fuzz/artifacts/$TARGET/"

echo ""
echo "========================================================================"
echo "Fuzzing Complete"
echo "========================================================================"

# Check if any crashes were found
ARTIFACTS_DIR="fuzz/artifacts/$TARGET"
if [ -d "$ARTIFACTS_DIR" ] && [ -n "$(ls -A "$ARTIFACTS_DIR" 2>/dev/null)" ]; then
    echo "WARNING: Crashes found in $ARTIFACTS_DIR"
    echo "To reproduce a crash:"
    echo "  cargo +nightly fuzz run $TARGET fuzz/artifacts/$TARGET/crash-*"
    echo ""
    echo "To add as a regression test:"
    echo "  cp fuzz/artifacts/$TARGET/crash-* tests/fuzz_regression_*"
    exit 1
else
    echo "No crashes found! ✓"
    exit 0
fi
