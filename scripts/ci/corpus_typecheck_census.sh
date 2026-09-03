#!/usr/bin/env bash
# CORPUS-TYPECHECK-CENSUS-1 (T1073): run `verum check` over every `.vr`
# in a tree and emit one row per file.
#
# WHY THIS EXISTS. Measured 2026-09-03: `core/` has 2560 files and NOT
# ONE gate that typechecks them. Every existing core/ gate in the
# Makefile — str-alias, barename-collisions, arch-attestation,
# type-name-collisions, protocol-form, constant-time-duplication — is
# syntactic, a grep. None runs the compiler.
#
# The cost of that showed up the same day: nine crypto call sites,
# covering EVERY signature verification in the tree (TLS 1.3
# certificate-verify, WebAuthn attestation and assertion, TUF role
# verification, Sigstore, X.509 chain validation) called
# `P256Signature.from_der` — a static method on a type that is
# `[Byte; 64]`, an array alias, which cannot carry one. Nothing was
# measuring, so nothing said so.
#
# A conformance spec did NOT catch it and could not have:
# `vcs/specs/L1-core/security/p256.vr` passes and uses the CORRECT
# spelling. It pins the library's SURFACE; what broke were the CALLERS.
# A gate on the producer says nothing about the consumer.
#
# THREE QUANTITIES PER FILE, and each earns its place:
#
#   rc     the exit code.
#   N      taken from the `compilation failed with N error(s)` summary
#          line, NOT from counting `^error<`: 318 diagnostics in this
#          tree print with no code at all, so a line-grep is a silent
#          lower bound.
#   parse  errors whose text says `Parse error`. Without this column an
#          error count is NOT a monotone quality measure — a parse
#          failure truncates the file and every later diagnostic
#          disappears, so BREAKING a file makes its count FALL. A sweep
#          with auto-revert keyed on "did the count drop" scored a
#          broken `core/text/text.vr` as its best result that morning.
#   mute   `yes` when the output shows no sign of work at all — no
#          `Checking`, no `Finished`, no `error`. Without it, "0" and
#          "the instrument did not answer" are the same reading.
#
# The three-quantity shape and the summary-line rule are verum-2b's,
# from the T1061 campaign; the parse column is what today cost them a
# broken file and an hour of misattribution.
#
# Usage:
#   scripts/ci/corpus_typecheck_census.sh <verum-binary> <out.tsv> [root]
#   scripts/ci/corpus_typecheck_census.sh <verum-binary> <out.tsv> core --sample 120
#
# Output: TSV, one row per file, plus a TOTAL line. Read it with
# `corpus_typecheck_ratchet.py`.

set -uo pipefail

BIN="${1:-}"
OUT="${2:-}"
ROOT="${3:-core}"
SAMPLE=0
if [ "${4:-}" = "--sample" ]; then SAMPLE="${5:-120}"; fi

if [ -z "$BIN" ] || [ -z "$OUT" ]; then
  echo "usage: $0 <verum-binary> <out.tsv> [root] [--sample N]" >&2
  exit 2
fi
if [ ! -x "$BIN" ]; then
  echo "no such binary: $BIN" >&2
  exit 2
fi

cd "$(dirname "$0")/../.."

# Deterministic order, so a sample is the SAME sample between runs and
# two censuses are comparable. `sort` is the whole of the determinism.
FILES=$(find "$ROOT" -name '*.vr' | sort)
if [ "$SAMPLE" -gt 0 ]; then
  FILES=$(printf '%s\n' "$FILES" | head -n "$SAMPLE")
fi

: > "$OUT"
total_n=0
total_parse=0
total_mute=0
total_files=0
total_failing=0

while IFS= read -r f; do
  [ -z "$f" ] && continue
  out=$(timeout 200 "$BIN" check "$f" 2>&1)
  rc=$?
  n=$(printf '%s' "$out" | grep -oE 'compilation failed with [0-9]+ error' | grep -oE '[0-9]+' | tail -1)
  [ -z "$n" ] && n=0
  parse=$(printf '%s' "$out" | grep -c 'Parse error')
  mute=no
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || mute=yes
  printf '%s\t%s\t%s\t%s\t%s\n' "$rc" "$n" "$parse" "$mute" "$f" >> "$OUT"
  total_files=$((total_files + 1))
  total_n=$((total_n + n))
  total_parse=$((total_parse + parse))
  [ "$mute" = yes ] && total_mute=$((total_mute + 1))
  [ "$n" -gt 0 ] && total_failing=$((total_failing + 1))
done <<< "$FILES"

printf 'TOTAL\tfiles=%s\terrors=%s\tparse=%s\tmute=%s\tfailing=%s\n' \
  "$total_files" "$total_n" "$total_parse" "$total_mute" "$total_failing" >> "$OUT"

echo "corpus census: files=$total_files errors=$total_n parse=$total_parse mute=$total_mute failing=$total_failing"
echo "  written to $OUT"
