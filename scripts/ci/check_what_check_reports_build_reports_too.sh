#!/bin/sh
# check_what_check_reports_build_reports_too.sh — `verum check` must not
# report a problem that `verum build` stays silent about, and must not
# stay silent about one `build` reports. On the same source.
#
# WHY THIS EXISTS.  T1118, measured 2026-09-03 on `internal/registry-next`,
# a 22-file cog:
#
#     command   purity errors   refinement errors   total
#     check           5                0              7
#     run             0               21             22
#     build           0               21             22
#
# The two sets are DISJOINT. Neither command sees what the other sees, so
# a developer fixes five errors under `check`, gets a clean run, calls
# `build`, and meets twenty-one different ones — none of which `check`
# had ever shown. Fixing those, `check` then shows five more.
#
# That is stronger than the invariant we had been stating. "check ⊆ build"
# allows `check` to be WEAKER; it does not allow the two to disagree. Here
# neither set is a subset of the other.
#
# TWO CAUSES, and the gate keeps them apart because the repairs differ:
#
#   check sees, build does not — `phase_context_validation` is called from
#     `check_project` under `if self.mode == CompilationMode::Check`, so
#     the purity judgement exists only in the check driver. `build` never
#     asks. Repair: move the phase into the shared list.
#
#   build sees, check does not — the same file checked ALONE reports the
#     refinement errors (measured: 2 of 2 on
#     `protocol/revocation.vr`), so the phase is reachable from `check`.
#     Over a cog it is not reached: the run ends at the first failing
#     phase instead of collecting. Repair: accumulate across phases —
#     `run_check_only` already does exactly this for the meta/typecheck
#     pair, with a comment saying why, so the pattern exists and was
#     applied once.
#
# WHAT THE FIXTURE CARRIES: one defect of each class, so a repair that
# closes only one direction cannot pass.
#
# Usage:
#   check_what_check_reports_build_reports_too.sh [verum]
#   check_what_check_reports_build_reports_too.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_verdict_agreement: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT
NONCE=$$_$(date +%s 2>/dev/null || echo 0)

mkdir -p "$TMP/probe/src"
cat > "$TMP/probe/verum.toml" <<'TOML'
[cog]
name        = "probe"
version     = "0.1.0"
authors     = []
license     = "MIT"
keywords    = []
categories  = []

[language]
profile = "application"

[dependencies]
stdlib = "0.1"
TOML

# Defect 1 — a purity violation, the class `check` sees and `build` does
# not. Defect 2 — a plain type error, which every driver should see.
# Keeping them in SEPARATE files means neither can mask the other by
# ending its file's compilation early.
{
  printf '// nonce %s\n' "$NONCE"
  printf 'module probe.impure;\n\n'
  printf 'public pure fn writes_through_a_param(out: &mut List<Int>) -> Int {\n'
  printf '    out.push(1);\n'
  printf '    0\n}\n'
} > "$TMP/probe/src/impure.vr"
{
  printf '// nonce %s\n' "$NONCE"
  printf 'module probe.mistyped;\n\n'
  printf 'public fn mistyped() -> Int {\n'
  printf '    let t: Text = "x";\n'
  printf '    t\n}\n'
} > "$TMP/probe/src/mistyped.vr"
{
  printf '// nonce %s\n' "$NONCE"
  printf 'mount probe.impure.{writes_through_a_param};\n'
  printf 'mount probe.mistyped.{mistyped};\n\n'
  printf 'fn main() {\n    print(mistyped());\n}\n'
} > "$TMP/probe/src/main.vr"

errset() { # $1 = subcommand -> sorted, normalised error texts
  (cd "$TMP" && timeout 400 "$VERUM" "$1" probe 2>&1) |
    grep -E '^error' |
    grep -v 'compilation failed with' |
    sed -E 's/[0-9]+/N/g' | sort -u
}

chk=$(errset check)
bld=$(errset build)
n_chk=$(printf '%s' "$chk" | grep -c . || true)
n_bld=$(printf '%s' "$bld" | grep -c . || true)
only_chk=$(printf '%s\n' "$chk" | grep -vxF "$bld" 2>/dev/null | grep -c . || true)

if [ "$SELFTEST" -eq 1 ]; then
  printf 'selftest: ok — check reported %s, build reported %s, check-only %s\n' \
    "$n_chk" "$n_bld" "$only_chk"
fi

# The fixture must be able to fail: if NEITHER command reports anything,
# the probe compiled clean and proves nothing about agreement.
if [ "$n_chk" -eq 0 ] && [ "$n_bld" -eq 0 ]; then
  printf 'check_verdict_agreement: FAILED — neither command reported anything.\n'
  printf '  The fixture carries a `pure` function that writes through an `&mut`\n'
  printf '  parameter and a function returning `Text` where `Int` is declared.\n'
  printf '  A probe that cannot come back positive measures nothing.\n'
  exit 1
fi

if [ "$only_chk" -ne 0 ]; then
  printf 'check_verdict_agreement: FAILED — %s diagnostic(s) appear under `check`\n' "$only_chk"
  printf '  and NOT under `build`:\n'
  printf '%s\n' "$chk" | grep -vxF "$bld" | sed 's/^/    /'
  printf '  `check` answers "would this build". A problem it reports and the\n'
  printf '  build does not means one of the two is lying, and the developer\n'
  printf '  cannot tell which. See T1118.\n'
  exit 1
fi

printf 'check_verdict_agreement: ok — check %s, build %s, nothing check-only\n' \
  "$n_chk" "$n_bld"
