#!/bin/sh
# check_compile_footprint.sh — ratchet on what a compile costs before it
# has done any work for the user.
#
# WHY THIS EXISTS.  Measured 2026-08-15 on the then-current binary:
#
#   verum --version .....................    8 MB   0.01 s
#   verum check --parse-only hello.vr ...    8 MB   0.01 s
#   verum check <missing file> ..........    8 MB   0.01 s
#   verum check bad.vr (syntax error) ... 721 MB   0.22 s
#   verum check empty.vr ................ 1059 MB   0.57 s
#   verum check hello.vr ................ 1062 MB   0.55 s
#
# An EMPTY file cost the same as a real one, and a file that does not even
# parse cost 721 MB — the whole stdlib was materialised before the parser
# looked at the source.  That is what makes N concurrent compilers (a test
# sweep, a build farm, an editor's check loop) exhaust a machine.
#
# The two numbers this gate holds are the ones a user can feel:
#
#   * EMPTY   — the floor of any compile.  Nothing about an empty file
#               justifies materialising a standard library.
#   * BROKEN  — a file with a syntax error.  This is the editor's inner
#               loop; it must cost about what `--parse-only` costs, because
#               that is all the work that can possibly be useful.
#
# Thresholds are a RATCHET, not a target: lower them when the number drops,
# never raise them to make a red run green.  Raising one is a decision to
# ship a regression and belongs in a commit message that says so.
#
# Usage: scripts/ci/check_compile_footprint.sh [verum-binary]
set -u

# Limits set from MEASUREMENT, not from a wish: 2026-08-15 the binary
# built at 07:24:51 peaked at 604 MB on the empty file and 8 MB on the
# broken one.  The headroom is deliberately small — a ratchet that sits
# far above the real number cannot catch the regression it exists for.
VERUM="${1:-target/release/verum}"
EMPTY_MAX_MB="${EMPTY_MAX_MB:-700}"
BROKEN_MAX_MB="${BROKEN_MAX_MB:-24}"

[ -x "$VERUM" ] || { printf 'check_compile_footprint: no binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

work="${TMPDIR:-/tmp}/verum_footprint.$$"
mkdir -p "$work" || exit 2
trap 'rm -rf "$work"' EXIT
: > "$work/empty.vr"
printf 'fn main( {\n' > "$work/broken.vr"

peak_mb() {  # peak_mb <file>
  out=$(cd "$work" && /usr/bin/time -l "$VERUM" check "$1" 2>&1 >/dev/null)
  bytes=$(printf '%s\n' "$out" | awk '/peak memory footprint/{print $1}')
  # macOS reports bytes; GNU time reports "Maximum resident set size (kbytes)".
  [ -n "$bytes" ] || bytes=$(printf '%s\n' "$out" | awk '/Maximum resident set size/{print $NF * 1024}')
  awk -v b="${bytes:-0}" 'BEGIN{printf "%.0f", b/1048576}'
}

status=0
empty_mb=$(peak_mb empty.vr)
broken_mb=$(peak_mb broken.vr)

printf 'compile footprint: empty=%s MB (limit %s)  syntax-error=%s MB (limit %s)\n' \
  "$empty_mb" "$EMPTY_MAX_MB" "$broken_mb" "$BROKEN_MAX_MB"

if [ "$empty_mb" -gt "$EMPTY_MAX_MB" ]; then
  printf 'FAIL: an empty file costs %s MB (limit %s).\n' "$empty_mb" "$EMPTY_MAX_MB" >&2
  status=1
fi
if [ "$broken_mb" -gt "$BROKEN_MAX_MB" ]; then
  printf 'FAIL: a file that does not parse costs %s MB (limit %s) — work is being done before the parse decides there is anything to do.\n' \
    "$broken_mb" "$BROKEN_MAX_MB" >&2
  status=1
fi
exit "$status"
