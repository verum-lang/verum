#!/bin/sh
# check_generic_position_does_not_decide_inference.sh — WHERE a generic
# parameter appears in a signature must not decide whether the call's
# result type can be inferred.
#
# WHY THIS EXISTS.  A function scheme loaded from the baked archive is
# built in two places.  `env.rs::metadata_fn_scheme` carries the
# PARAMNAME-CARRY bridge (T0701): a declared generic arrives under TWO
# spellings — the source name (`T`) and the positional placeholder
# (`__generic_0`) — and both must intern to ONE type variable.  The twin
# builder in `modules.rs` mirrors that function down to its comments but
# was missing the bridge, so the two spellings stayed disconnected.
#
# The renderer spells a generic by SOURCE NAME inside a parameter's type
# arguments and by PLACEHOLDER in the return.  Measured 2026-09-01
# (T0741) on the baked `core.math.tensor`:
#
#   mul         params=[a: `&DynTensor<T>`, b: `&DynTensor<T>`]
#               ret=`DynTensor<__generic_0>`          -> DynTensor<_>
#   mul_scalar  params=[x: `&DynTensor<T>`, scalar: `__generic_0`]
#               ret=`DynTensor<__generic_0>`          -> clean
#
# Solving the arguments bound the `T` variable; the return's
# `__generic_0` was a different variable and stayed free, so
# `let z = mul(y, y);` on a fully CONCRETE `&DynTensor<Float>` judged
# `DynTensor<_>` and demanded an annotation.  `mul_scalar` escaped only
# because one of its parameters happened to be rendered in the return's
# spelling — the inference worked BY COINCIDENCE, and the coincidence is
# what this gate removes.
#
# WHY A SCRIPT AND NOT A UNIT TEST.  The defect needs a callee whose
# signature came from the ARCHIVE; a hand-built descriptor in a unit test
# is exactly the input that does not reproduce it, because the test
# author writes both spellings the same way.  Same reasoning as
# check_type_parameter_letter_is_irrelevant.sh.
#
# WHAT IT ASSERTS.  Two calls into the same stdlib module, differing only
# in where the callee's `T` sits, must reach the SAME verdict, and that
# verdict must be "clean".  Agreement alone is not enough: two broken
# subjects also agree.
#
# Usage:
#   check_generic_position_does_not_decide_inference.sh [verum]
#   check_generic_position_does_not_decide_inference.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_generic_position: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# `mul`'s T appears ONLY inside `&DynTensor<T>`; `mul_scalar` also takes
# it bare as `scalar: T`.  Everything else about the two files matches.
cat > "$TMP/nested_only.vr" <<'VR'
module probe.generic_position_nested;
mount core.math.tensor;

public fn probe(y: &DynTensor<Float>) -> Int {
    let z = mul(y, y);
    let _ = z;
    0
}
VR

cat > "$TMP/also_bare.vr" <<'VR'
module probe.generic_position_bare;
mount core.math.tensor;

public fn probe(y: &DynTensor<Float>) -> Int {
    let z = mul_scalar(y, 2.0);
    let _ = z;
    0
}
VR

# A fixture that compares two byte-identical subjects agrees with itself
# about everything and proves nothing.
if cmp -s "$TMP/nested_only.vr" "$TMP/also_bare.vr"; then
  printf 'check_generic_position: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

verdict() { # $1 file -> error count, or MUTE
  out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}

a=$(cd "$REPO" && verdict "$TMP/nested_only.vr")
b=$(cd "$REPO" && verdict "$TMP/also_bare.vr")

if [ "$SELFTEST" -eq 1 ]; then
  # The instrument must be able to come back negative: a subject that IS
  # broken has to score differently from a clean one.  Without this, a
  # checker that reports 0 for everything passes the gate silently.
  printf 'module probe.generic_position_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  c=$(cd "$REPO" && verdict "$TMP/broken.vr")
  if [ "$b" = "$c" ]; then
    printf 'selftest: FAILED — a clean subject (%s) and a broken one (%s) compare equal\n' "$b" "$c"
    exit 1
  fi
  printf 'selftest: ok — clean=%s broken=%s are distinguishable\n' "$b" "$c"
fi

if [ "$a" = MUTE ] || [ "$b" = MUTE ]; then
  printf 'check_generic_position: FAILED — subject produced no output (nested=%s bare=%s).\n' "$a" "$b"
  printf '  The checker said nothing at all; that is a broken run, not a clean one.\n'
  exit 1
fi

if [ "$a" != "$b" ]; then
  printf 'check_generic_position: FAILED — verdict depends on WHERE the generic sits.\n'
  printf '  mul(y, y)          (T only inside &DynTensor<T>): %s error(s)\n' "$a"
  printf '  mul_scalar(y, 2.0) (T also bare as `scalar: T`) : %s error(s)\n' "$b"
  printf '  Both callees are generic over one T and both are called with a\n'
  printf '  fully concrete &DynTensor<Float>. The position of T in the\n'
  printf '  signature is not a fact about the call.\n'
  printf '  Look for the PARAMNAME-CARRY bridge (alias_scope_generic) in the\n'
  printf '  scheme builder that served this call — env.rs has it; its twin\n'
  printf '  in modules.rs went without it once already (T0741).\n'
  exit 1
fi

if [ "$a" != 0 ]; then
  printf 'check_generic_position: FAILED — both subjects report %s error(s).\n' "$a"
  printf '  They agree, but agreement between two broken subjects is not the\n'
  printf '  property this gate exists for. Run the two files by hand:\n'
  printf '    %s check <file>\n' "$VERUM"
  exit 1
fi

printf 'check_generic_position: ok — nested-only and also-bare both clean (%s errors)\n' "$a"
