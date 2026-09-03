#!/bin/sh
# check_a_mounted_call_never_compiles_to_unit.sh — a call to a function
# the user's own module declares and mounts must reach that function, or
# fail loudly. It must never compile to `()`.
#
# WHY THIS EXISTS.  T0907, measured 2026-09-03.  Two files, no stdlib
# involvement in the source:
#
#     module verifier;
#     public fn verify(n: Int) -> Result<Verified, Text> {
#         Result.Ok(Verified { authority_id: n, role: 1 })
#     }
#
#     mount verifier.{Verified, verify};
#     fn main() {
#         match verify(3) {
#             Ok(v)  => print(v.authority_id),
#             Err(_) => print(999),
#         }
#     }
#
#     verum run   -> 999          the Err arm, from a function that
#                                 returns Ok unconditionally
#     verum check -> 0 errors
#     verum build -> 0 errors
#
# The user's function is never entered (a `print` inside it stays
# silent) and `print(verify(3))` prints `()`.  The TYPE CHECKER is
# right — annotating `let w: Int = verify(3)` gives
# `expected 'Int', found 'Ok(Verified) | Err(Text)'` — so the two layers
# disagree and codegen is the wrong one.
#
# MECHANISM — measured, not inferred.  `VERUM_TRACE_QKEY` names the
# whole chain in two lines:
#
#     [qkey] bind_mounted_function alias='verify' key='verifier.verify'
#     [qkey] decl-yield verifier.verify mine=33895 holder=Some(33349)
#
# and the healthy spelling differs in exactly one value — the holder is
# the declaration ITSELF:
#
#     [qkey] decl-yield vf6.verify mine=33895 holder=Some(33895)
#
# That the holder 33349 is FOREIGN was measured rather than assumed:
# padding the user's module with three extra declarations walked `mine`
# 33895 -> 33898 while `holder` stayed 33349, which rules out the
# reading that it is a second pass over the same declaration.
#
# The ownership judgement (`verum_vbc/src/codegen/mod.rs`, search
# `decl-yield`) asked two questions of a holder — is it a mount?  is it
# an extern stub? — and read every other holder as the one legitimate
# case: a second LOCAL declaration of this module, i.e. an arity
# overload, which keeps source-order precedence.  A third case exists:
# a foreign module holding the key.  It was read as the first, so the
# user's own declaration yielded its own name.  The explicit mount then
# found the stranger on the ladder's FIRST probe and bound the bare
# alias to it authoritatively, and the call ran the stdlib function.
#
# The leaf-qualified key `verifier.verify` is not in the baked archive
# (7 of its 8 `verifier.verify` strings are fully qualified, none are
# leaf) — it is created in-process at load.  Which loader writes it is
# still unnamed; the ownership rule is repaired independently of that,
# because a user module may not lose its own name to the leaf of
# somebody else's path however the leaf key arrives.
#
# EARLIER OBSERVATION, kept because it shows the two failure shapes.
# The module name `verifier` is the simple name of three core/ modules.
# Vary ONLY the function name and the outcome tracks how many core/
# modules declare it:
#
#     verify                                5 modules   silent 999
#     verify_batch                          2 modules   silent 999
#     verifier_config_for_code_signing      1 module    LOUD
#     verifier_config_for_tls_server_auth   1 module    LOUD (predicted)
#
# LOUD is `VBC codegen error (user bodies): wrong number of arguments
# for …: expected 0, found 1` — the contested bare slot resolves to
# nothing and becomes Unit; the uncontested one resolves to a stranger
# and its arity check fires.  So a working refusal already exists on one
# branch and is missing on the other.
#
# WHAT IT ASSERTS, and why each arm is load-bearing:
#   contested    the pair (colliding module, contested name) RUNS the
#                user's function — prints 3, not 999 and not a stub
#   uncontested  the pair (colliding module, uncontested name) also
#                runs it. Without this arm a repair that only silences
#                the loud branch would pass
#   control      a NON-colliding module name works, before and after —
#                so a failure names the collision, not mounting at large
#   loud         a genuinely wrong call is still refused: calling the
#                stdlib's own `verify` with one argument must remain an
#                error, so the fix cannot be "stop checking calls"
#
# Usage:
#   check_a_mounted_call_never_compiles_to_unit.sh [verum]
#   check_a_mounted_call_never_compiles_to_unit.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_mounted_call: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# Content-keyed VBC cache: a fixed probe answers from cache on the
# second CI run. See check_test_harness_runs_the_users_program.sh.
NONCE=$$_$(date +%s 2>/dev/null || echo 0)

# $1 = subdir, $2 = module name, $3 = function name
build_case() {
  d="$TMP/$1"
  mkdir -p "$d"
  {
    printf '// nonce %s-%s\n' "$NONCE" "$1"
    printf 'module %s;\n\n' "$2"
    printf 'public type Verified is { authority_id: Int, role: Int };\n\n'
    printf 'public fn %s(n: Int) -> Result<Verified, Text> {\n' "$3"
    printf '    Result.Ok(Verified { authority_id: n, role: 1 })\n}\n'
  } > "$d/$2.vr"
  {
    printf '// nonce %s-%s\n' "$NONCE" "$1"
    printf 'mount %s.{Verified, %s};\n\n' "$2" "$3"
    printf 'fn main() {\n    match %s(3) {\n' "$3"
    printf '        Ok(v) => print(v.authority_id),\n'
    printf '        Err(_) => print(999),\n    }\n}\n'
  } > "$d/main.vr"
}

run_case() { # $1 = subdir -> last payload line, or the first error line
  out=$( (cd "$REPO" && timeout 300 "$VERUM" run "$TMP/$1/main.vr" 2>&1) )
  err=$(printf '%s' "$out" | grep -m1 -E '^error' | cut -c1-70)
  if [ -n "$err" ]; then
    printf 'ERR:%s' "$err"
    return
  fi
  printf '%s' "$out" |
    grep -vE '^\s*(Compiling|Checking|Parsing|Finished|Running|Building)' |
    grep -vE '^\[' | tail -1
}

build_case contested   verifier verify
build_case uncontested verifier verifier_config_for_code_signing
build_case control     vfprobe  verify

co=$(run_case contested)
un=$(run_case uncontested)
ct=$(run_case control)

# A genuinely wrong call must stay an error: the stdlib's own `verify`
# takes four arguments.
mkdir -p "$TMP/loud"
{
  printf '// nonce %s-loud\n' "$NONCE"
  printf 'mount core.security.zk.stark.verifier.{verify};\n\n'
  printf 'fn main() {\n    let r = verify(3);\n    print(1);\n}\n'
} > "$TMP/loud/main.vr"
loud=$( (cd "$REPO" && timeout 300 "$VERUM" check "$TMP/loud/main.vr" 2>&1) | grep -c '^error<E102>')

if [ "$SELFTEST" -eq 1 ]; then
  printf 'selftest: ok — contested=%s uncontested=%s control=%s loud_arity_errors=%s\n' \
    "$co" "$un" "$ct" "$loud"
fi

# The control comes first: if a non-colliding module name is broken, the
# subjects say nothing about collisions.
if [ "$ct" != "3" ]; then
  printf 'check_mounted_call: FAILED — the CONTROL printed `%s`, expected `3`.\n' "$ct"
  printf '  A module named `vfprobe` collides with nothing. If mounting is\n'
  printf '  broken in general, the collision arms below prove nothing.\n'
  exit 1
fi

if [ "$loud" -eq 0 ]; then
  printf 'check_mounted_call: FAILED — calling the stdlib `verify` with one argument was accepted.\n'
  printf '  `core.security.zk.stark.verifier.verify` takes four. E102 must fire.\n'
  printf '  Without this, the arms below pass for a build that stopped checking\n'
  printf '  calls at all.\n'
  exit 1
fi

if [ "$co" != "3" ]; then
  printf 'check_mounted_call: FAILED — a CONTESTED name printed `%s`, expected `3`.\n' "$co"
  printf '  `module verifier` + `fn verify`: five core/ modules declare `verify`,\n'
  printf '  so the bare slot is contested, resolves to nothing, and the call\n'
  printf '  compiles to `()` — `match` then takes the Err arm and prints 999.\n'
  printf '  The type checker resolves the SAME call correctly, so this is a\n'
  printf '  disagreement between layers, not a missing type. See T0907 and\n'
  printf '  the ownership judgement in verum_vbc/src/codegen/mod.rs — search\n'
  printf '  `decl-yield`; run again with VERUM_TRACE_QKEY=1 to see which of\n'
  printf '  decl-fresh / decl-takeback{,-stub,-foreign} / decl-yield fired.\n'
  exit 1
fi

if [ "$un" != "3" ]; then
  printf 'check_mounted_call: FAILED — an UNCONTESTED name printed `%s`, expected `3`.\n' "$un"
  printf '  `module verifier` + `fn verifier_config_for_code_signing`: one core/\n'
  printf '  module declares it, so the bare slot resolves to that stranger and\n'
  printf '  its arity check fires — pre-fix this was `VBC codegen error … \n'
  printf '  expected 0, found 1`. Loud is better than silent and still wrong:\n'
  printf '  the call belongs to the module the file mounted.\n'
  exit 1
fi

printf 'check_mounted_call: ok — contested (%s), uncontested (%s), control (%s), stdlib arity still refused (%s)\n' \
  "$co" "$un" "$ct" "$loud"
