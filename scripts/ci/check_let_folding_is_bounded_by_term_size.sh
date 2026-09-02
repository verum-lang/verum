#!/bin/sh
# check_let_folding_is_bounded_by_term_size.sh — folding `let` bindings
# for refinement reflection must be bounded by the size of the term it
# builds, not by the number of statements it walks.
#
# WHY THIS EXISTS.  `fold_let_bindings` stores every binding ALREADY
# SUBSTITUTED, so `let a_i = a_{i-1} + a_{i-1}` doubles the stored term
# at each step.  Measured 2026-09-02 (T1081), one binary:
#
#     bindings   peak RSS      wall
#        8         534 MB      0.32 s
#       12         534 MB      0.34 s
#       16         855 MB      0.48 s
#       20        6028 MB      6.30 s
#
# The real victim was `core/database/sqlite/native/builtins/math_fns.vr`
# — 343 lines of stdlib that reached the 24 GB memory ceiling and was
# therefore NEVER CHECKED.  `check_core_compiles` had been reporting it
# for as long as it existed, and it read as "one more red file".
#
# A guard was already there — `let mut budget: usize = 256` — and its
# own comment claimed it "refuses a body that doubles its term on every
# step".  It cannot: twenty statements are twenty against 256 whether
# they build forty nodes or a million.  The counter was denominated in
# STATEMENTS; the quantity that grows is NODES.  That is why the second
# budget exists and why this gate is written in the same unit.
#
# WHAT IT ASSERTS, and why each arm is load-bearing:
#   subject   a 24-binding doubling chain completes under a 2 GB ceiling
#   fixture   the SAME file with the term budget lifted must still trip
#             that ceiling — otherwise the subject proves nothing, since
#             a fixture that stopped exploding would pass on any build
#   quiet     an ordinary 8-binding chain is NOT refused: the guard must
#             not pay for itself by declining normal code
#   verify    a reflection-dependent spec still verifies, so the repair
#             cannot be "switch reflection off"
#
# Usage:
#   check_let_folding_is_bounded_by_term_size.sh [verum]
#   check_let_folding_is_bounded_by_term_size.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_let_folding: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

write_chain() { # $1 = number of bindings, $2 = out file
  {
    printf 'module probe.let_chain_%s;\n\n' "$1"
    printf 'fn chain(x: Float64) -> Float64 {\n'
    printf '    let a0: Float64 = x;\n'
    i=1
    while [ "$i" -le "$1" ]; do
      p=$((i - 1))
      printf '    let a%s: Float64 = a%s + a%s;\n' "$i" "$p" "$p"
      i=$((i + 1))
    done
    printf '    a%s\n}\n' "$1"
  } > "$2"
}

write_chain 24 "$TMP/deep.vr"
write_chain 8  "$TMP/shallow.vr"

# 2 GB is comfortably above this toolchain's ~750 MB floor for a
# single-file check and far below the 24 GB the unbounded fold reached.
CEIL=2000

run_deep() { # $1 = extra env assignment or empty -> exit code
  if [ -n "$1" ]; then
    (cd "$REPO" && env "$1" VERUM_MEMORY_CEILING_MB=$CEIL \
      timeout 400 "$VERUM" check "$TMP/deep.vr" >/dev/null 2>&1)
  else
    (cd "$REPO" && env VERUM_MEMORY_CEILING_MB=$CEIL \
      timeout 400 "$VERUM" check "$TMP/deep.vr" >/dev/null 2>&1)
  fi
  echo $?
}

bounded=$(run_deep "")
unbounded=$(run_deep "VERUM_FOLD_TERM_BUDGET=999999999")

refused_shallow=$( (cd "$REPO" && env VERUM_TRACE_FOLDSIZE=1 \
  timeout 200 "$VERUM" check "$TMP/shallow.vr" 2>&1) | grep -c 'foldsize] REFUSED' || true)

SPEC="vcs/specs/L0-critical/verification/two_calls_to_one_pure_function_are_equal.vr"
if [ -f "$REPO/$SPEC" ]; then
  spec_errs=$( (cd "$REPO" && timeout 300 "$VERUM" check "$SPEC" 2>&1) |
    grep -c '^error' || true)
else
  spec_errs=SKIP
fi

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.let_chain_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  brk=$( (cd "$REPO" && timeout 200 "$VERUM" check "$TMP/broken.vr" 2>&1) |
    grep -c '^error' || true)
  if [ "$brk" -eq 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file reported no error\n'
    exit 1
  fi
  printf 'selftest: ok — bounded=%s unbounded=%s shallow_refusals=%s spec_errors=%s broken=%s\n' \
    "$bounded" "$unbounded" "$refused_shallow" "$spec_errs" "$brk"
fi

# The fixture check comes FIRST. If the unbounded run no longer explodes,
# the subject below passes for the wrong reason and this gate would go
# quietly vacuous — the failure mode it was written to avoid.
if [ "$unbounded" -eq 0 ]; then
  printf 'check_let_folding: FAILED — the fixture no longer explodes without the budget.\n'
  printf '  A 24-binding doubling chain completed under a %s MB ceiling with\n' "$CEIL"
  printf '  VERUM_FOLD_TERM_BUDGET lifted. Either the fold stopped substituting\n'
  printf '  eagerly (then this gate needs a new fixture) or the env override\n'
  printf '  stopped working (then the gate measures nothing at all).\n'
  exit 1
fi

if [ "$bounded" -ne 0 ]; then
  printf 'check_let_folding: FAILED — a 24-binding chain exceeded a %s MB ceiling (exit %s).\n' \
    "$CEIL" "$bounded"
  printf '  Each `let a_i = a_{i-1} + a_{i-1}` doubles the stored term because\n'
  printf '  fold_stmts keeps every binding already substituted. The statement\n'
  printf '  budget cannot see that: twenty statements are twenty whether they\n'
  printf '  build forty nodes or a million. See T1081 and `expr_size_capped`\n'
  printf '  in verum_smt/src/expr_to_smtlib.rs.\n'
  exit 1
fi

if [ "$refused_shallow" -ne 0 ]; then
  printf 'check_let_folding: FAILED — an ordinary 8-binding chain was refused (%s times).\n' \
    "$refused_shallow"
  printf '  Measured when the budget was introduced: 213 core/ files produced\n'
  printf '  ZERO refusals. A guard that declines ordinary code buys its bound\n'
  printf '  by making the verifier prove less everywhere.\n'
  exit 1
fi

if [ "$spec_errs" != SKIP ] && [ "$spec_errs" -ne 0 ]; then
  printf 'check_let_folding: FAILED — a reflection-dependent spec reported %s error(s).\n' \
    "$spec_errs"
  printf '  %s\n' "$SPEC"
  printf '  Proving two calls to a pure function equal needs the body reflected.\n'
  printf '  Bounding the fold must not be achieved by folding nothing.\n'
  exit 1
fi

printf 'check_let_folding: ok — deep chain bounded (fixture still explodes unbounded), shallow chain untouched, reflection intact\n'
