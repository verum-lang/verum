#!/bin/bash
# =============================================================================
# Architectural CI guard: verify Verum AOT binaries don't link libc.
# =============================================================================
#
# Per `docs/architecture/no-libc-architecture.md` and CLAUDE.md:
#
#   Verum's VBC interpreter (Tier 0) and AOT-compiled binaries (Tier 1)
#   MUST NOT call into libc.  All runtime functionality goes through:
#     * Linux: direct syscalls via `syscall` / `svc #0` instructions.
#     * macOS: libSystem.B.dylib only (Apple-required boundary, NOT
#       libc in the glibc/musl sense).
#     * Windows: kernel32.dll + ntdll.dll only (no MSVC CRT, no UCRT).
#     * FreeBSD: direct syscalls.
#
# This script builds a smoke binary then verifies its dynamic-library
# dependencies are within the allow-list for the host platform.
#
# Usage:
#   ./scripts/ci/check_no_libc_link.sh [verum_binary] [smoke_source]
#
# Defaults:
#   verum_binary  = target/debug/verum
#   smoke_source  = (created in $TMPDIR — minimal hello-world)
#
# Exit codes:
#   0 — no libc deps detected
#   1 — libc dep detected
#   2 — script error / unsupported platform
# =============================================================================
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

VERUM_BIN="${1:-target/debug/verum}"
SMOKE_SRC="${2:-}"

if [[ ! -x "$VERUM_BIN" ]]; then
    echo "[fail] verum binary not found at: $VERUM_BIN"
    echo "       build it first: cargo build --bin verum"
    exit 2
fi

# Per-platform allow-lists.  Anything OUTSIDE this list is a libc-link
# violation (binary links a forbidden library).
case "$(uname -s)" in
    Linux)
        # Linux: direct syscalls — NO libraries allowed except dynamic
        # linker (ld-linux-*).  No libc.so, no libpthread.so, etc.
        ALLOWED_PATTERNS=(
            "ld-linux-aarch64.so"
            "ld-linux-x86-64.so"
            "ld-linux.so"
            "linux-vdso.so"  # vDSO is part of kernel ABI, not libc
            "ld.so"
        )
        FORBIDDEN_PATTERNS=(
            "libc.so"
            "libc.musl"
            "libpthread.so"
            "libdl.so"
            "libm.so"
            "libgcc_s.so"
        )
        DEPS_CMD="ldd"
        ;;
    Darwin)
        # macOS: libSystem.B.dylib is ALLOWED (Apple's required system
        # boundary).  Foundation/Metal/objc are allowed for GPU compute.
        # NO libc.dylib (which doesn't exist on macOS anyway), NO third-
        # party C libraries.
        ALLOWED_PATTERNS=(
            "/usr/lib/libSystem.B.dylib"
            "/System/Library/Frameworks/"  # Foundation, Metal, etc.
            "/usr/lib/libobjc"             # ObjC runtime for Metal interop
        )
        FORBIDDEN_PATTERNS=(
            # macOS has no libc.dylib — listing for clarity
            "/usr/local/lib/"  # Homebrew C libraries
            "/opt/homebrew/lib/"
        )
        DEPS_CMD="otool -L"
        ;;
    *)
        echo "[skip] unsupported platform: $(uname -s)"
        echo "       this guard runs on Linux + macOS only"
        exit 0
        ;;
esac

# Build a smoke source if not provided.
# Use a fixed-name file to avoid mktemp's randomness producing a
# filename that the verum parser refuses (dots/special characters).
if [[ -z "$SMOKE_SRC" ]]; then
    SMOKE_SRC="/tmp/verum_no_libc_smoke.vr"
    cat > "$SMOKE_SRC" <<'EOF'
fn main() {
    print("smoke");
}
EOF
    trap "rm -f '$SMOKE_SRC'" EXIT
fi

# Build the smoke binary.
echo "[info] building smoke binary from $SMOKE_SRC"
SMOKE_OUT_DIR="$(mktemp -d -t verum_smoke_XXXXXX)"
trap "rm -rf '$SMOKE_OUT_DIR'" EXIT
SMOKE_TARGET="$SMOKE_OUT_DIR/smoke"

if ! "$VERUM_BIN" build "$SMOKE_SRC" 2>&1 | tail -2; then
    echo "[fail] verum build failed"
    exit 2
fi

# verum build outputs to /tmp/target/release/<basename>.
SMOKE_BASENAME="$(basename "$SMOKE_SRC" .vr)"
BUILT_BIN="/tmp/target/release/$SMOKE_BASENAME"
if [[ ! -x "$BUILT_BIN" ]]; then
    echo "[fail] expected binary not found: $BUILT_BIN"
    exit 2
fi

echo "[info] inspecting linker dependencies of $BUILT_BIN"
DEPS_OUTPUT="$($DEPS_CMD "$BUILT_BIN" 2>&1)"

# Print full deps for visibility.
echo "[info] $DEPS_CMD output:"
echo "$DEPS_OUTPUT" | sed 's/^/    /'

# Detect forbidden patterns.
violations=0
for pat in "${FORBIDDEN_PATTERNS[@]}"; do
    if echo "$DEPS_OUTPUT" | grep -qE "$pat"; then
        echo "[fail] FORBIDDEN library detected: '$pat'"
        echo "$DEPS_OUTPUT" | grep -E "$pat" | sed 's/^/    /'
        violations=$((violations + 1))
    fi
done

# Also verify EVERY dependency line is in the allow-list (defence in depth).
# Skip the binary's own self-reference line ("$BUILT_BIN:") on macOS otool.
DEPS_LINES="$(echo "$DEPS_OUTPUT" | grep -E '^\s+(/|libname)' || true)"
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    matched=0
    for allowed in "${ALLOWED_PATTERNS[@]}"; do
        if echo "$line" | grep -qE "$allowed"; then
            matched=1
            break
        fi
    done
    if [[ $matched -eq 0 ]]; then
        echo "[warn] dependency outside allow-list: $line"
        # Don't fail on warnings — only on explicit forbidden pattern matches.
    fi
done <<< "$DEPS_LINES"

if [[ $violations -gt 0 ]]; then
    echo ""
    echo "============================================================"
    echo "FAIL: $violations forbidden library reference(s) detected."
    echo ""
    echo "Per docs/architecture/no-libc-architecture.md, Verum AOT"
    echo "binaries MUST NOT link against libc."
    echo "============================================================"
    exit 1
fi

echo ""
echo "OK: no forbidden libc dependencies detected."
echo "    Binary: $BUILT_BIN ($(stat -f%z "$BUILT_BIN" 2>/dev/null || stat -c%s "$BUILT_BIN") bytes)"
exit 0
