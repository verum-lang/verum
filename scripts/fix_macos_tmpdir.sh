#!/bin/bash
# Fix macOS TMPDIR permission issue
# This script ensures TMPDIR points to a user-accessible directory

set -euo pipefail

echo "=== Fixing macOS TMPDIR Issue ==="

# Get the Darwin user temp directory (user-specific)
USER_TMPDIR=$(getconf DARWIN_USER_TEMP_DIR 2>/dev/null || echo "")

if [ -z "$USER_TMPDIR" ]; then
    echo "Warning: Could not get DARWIN_USER_TEMP_DIR, using fallback"
    USER_TMPDIR="/tmp/verum-build-$$"
    mkdir -p "$USER_TMPDIR"
    chmod 700 "$USER_TMPDIR"
fi

# Check if current TMPDIR is accessible
if [ -n "${TMPDIR:-}" ]; then
    if [ ! -w "$TMPDIR" ]; then
        echo "Current TMPDIR ($TMPDIR) is not writable"
        echo "Setting TMPDIR to user-accessible directory: $USER_TMPDIR"
        export TMPDIR="$USER_TMPDIR"
    else
        echo "Current TMPDIR ($TMPDIR) is writable - OK"
    fi
else
    echo "TMPDIR not set, setting to: $USER_TMPDIR"
    export TMPDIR="$USER_TMPDIR"
fi

echo "TMPDIR is now: $TMPDIR"
echo "Testing write access..."
TEST_FILE="$TMPDIR/verum-test-$$"
if echo "test" > "$TEST_FILE" 2>/dev/null; then
    echo "✓ TMPDIR is writable"
    rm -f "$TEST_FILE"
else
    echo "✗ TMPDIR is still not writable - manual intervention required"
    exit 1
fi

echo ""
echo "=== Fix Applied Successfully ==="
echo "Run your build command with this TMPDIR:"
echo "  TMPDIR=\"$TMPDIR\" cargo build"
echo ""
echo "Or add to your shell profile (~/.zshrc or ~/.bashrc):"
echo "  export TMPDIR=\"$USER_TMPDIR\""
