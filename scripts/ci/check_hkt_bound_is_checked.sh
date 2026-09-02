#!/bin/sh
# check_hkt_bound_is_checked.sh — a protocol bound on a higher-kinded
# parameter constrains the call, exactly as a bound on an ordinary one.
#
# WHY THIS EXISTS.  `fn use_it<F<_>: Mappable, A>(x: F<A>) -> Int`
# accepted an argument whose type has no `implement Mappable` anywhere.
# Measured 2026-09-02 (T1075), four subjects, one binary:
#
#     implement Mappable for Good<T>, called with Good   0   correct
#     called with Bad, which implements nothing          0   DEFECT
#     same probe with an ordinary bound <T: Mappable>    1   E405
#     HKT function called with an Int                    1   E400
#
# The third line says ordinary bounds ARE checked; the fourth says the
# parameter's SHAPE is checked, so the signature is not opaque.  The
# defect was exactly one thing wide.
#
# ROOT.  In one loop over `func.generics` (infer/modules.rs), the `Type`
# arm does THREE things with a converted bound — registers it on the
# TypeVar for method dispatch, adds a TypeParam to the environment, and
# inserts it into `func_param_protocol_bounds`.  The `HigherKinded` arm
# did the first two.  The third is the load-bearing one: that map
# becomes `scheme.with_protocol_bounds(...)`, and the scheme is what a
# call site checks against.
#
# The tell that named it without a build: the SAME bound written as
# `where type F: Mappable` WAS enforced, because
# `collect_where_clause_bounds` inserts into that map regardless of the
# parameter's kind.  Two spellings of one constraint, one enforced.
# That is why this gate asserts both spellings agree.
#
# WHAT IT ASSERTS, and why each arm is needed:
#   violation  the bound refuses a type that does not implement it
#   valid      it still ADMITS a type that does           (else the fix
#              could be "refuse everything")
#   where      the `where type F: P` spelling agrees with `<F<_>: P>`
#              (the two must not drift apart again)
#
# Usage:
#   check_hkt_bound_is_checked.sh [verum]
#   check_hkt_bound_is_checked.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_hkt_bound: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# Every subject declares the SAME protocol, the SAME two types and the
# SAME implement block.  Only the call's argument and the way the bound
# is spelled vary, so a verdict difference can come from nothing else.
write_case() { # $1 = signature, $2 = argument expression, $3 = out file
  cat > "$3" <<VR
module probe.hkt_bound;

type Mappable is protocol {
    fn mapit(&self) -> Int;
};

type Good<T> is { v: T };

type Bad<T> is { v: T };

implement<T> Mappable for Good<T> {
    fn mapit(&self) -> Int { 1 }
}

$1

public fn go() -> Int {
    let a = $2;
    use_it(a)
}
VR
}

INLINE='fn use_it<F<_>: Mappable, A>(x: F<A>) -> Int { 2 }'
WHERE='fn use_it<F<_>, A>(x: F<A>) -> Int where type F: Mappable { 2 }'

write_case "$INLINE" 'Bad { v: 7 }'  "$TMP/violation.vr"
write_case "$INLINE" 'Good { v: 7 }' "$TMP/valid.vr"
write_case "$WHERE"  'Bad { v: 7 }'  "$TMP/where_violation.vr"

if cmp -s "$TMP/violation.vr" "$TMP/valid.vr"; then
  printf 'check_hkt_bound: violation and valid are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

verdict() { # $1 file -> error count, or MUTE
  out=$(cd "$REPO" && timeout 300 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  n=$(printf '%s' "$out" | grep -oE 'compilation failed with [0-9]+ error' |
        grep -oE '[0-9]+' | tail -1)
  [ -n "$n" ] || n=0
  echo "$n"
}
says_bound() { # $1 file -> 1 when the diagnostic names the reason
  (cd "$REPO" && timeout 300 "$VERUM" check "$1" 2>&1) |
    grep -c "does not implement \`Mappable\`" || true
}

vio=$(verdict "$TMP/violation.vr")
val=$(verdict "$TMP/valid.vr")
whr=$(verdict "$TMP/where_violation.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.hkt_bound_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  brk=$(verdict "$TMP/broken.vr")
  if [ "$brk" = MUTE ] || [ "$brk" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$brk"
    exit 1
  fi
  printf 'selftest: ok — violation=%s valid=%s where=%s broken=%s\n' "$vio" "$val" "$whr" "$brk"
fi

for pair in "violation:$vio" "valid:$val" "where:$whr"; do
  if [ "${pair#*:}" = MUTE ]; then
    printf 'check_hkt_bound: FAILED — subject %s produced no output.\n' "${pair%%:*}"
    exit 1
  fi
done

# The control comes first on purpose: a repair that refuses everything
# would satisfy the violation arm while destroying the feature.
if [ "$val" != 0 ]; then
  printf 'check_hkt_bound: FAILED — a type that DOES implement the bound was refused (%s errors).\n' "$val"
  printf '  `implement<T> Mappable for Good<T>` is present and `use_it(Good{..})`\n'
  printf '  must compile. Refusing both arguments is not a bound check.\n'
  exit 1
fi

if [ "$vio" = 0 ]; then
  printf 'check_hkt_bound: FAILED — `fn use_it<F<_>: Mappable, A>(x: F<A>)` accepted a type\n'
  printf '  with no `implement Mappable` at all.\n'
  printf '  The bound is recorded on the TypeVar for method dispatch but never\n'
  printf '  reaches `func_param_protocol_bounds` — the map that becomes the\n'
  printf '  scheme a call site checks against. See HKT-BOUND-REACHES-THE-SCHEME\n'
  printf '  in infer/modules.rs; the `Type` arm of the same loop does it.\n'
  exit 1
fi

# Refused is necessary but not sufficient: it must be refused for THIS
# reason, or any unrelated breakage in the fixture would satisfy the gate.
if [ "$(says_bound "$TMP/violation.vr")" -eq 0 ]; then
  printf 'check_hkt_bound: FAILED — refused, but not for this reason.\n'
  printf '  The subject reports %s error(s) and none names `Mappable`.\n' "$vio"
  exit 1
fi

# `<F<_>: P>` and `where type F: P` are two spellings of one constraint.
# They were enforced by different code paths and drifted apart once.
if [ "$whr" != "$vio" ]; then
  printf 'check_hkt_bound: FAILED — the two spellings of one bound disagree.\n'
  printf '  <F<_>: Mappable>          : %s error(s)\n' "$vio"
  printf '  where type F: Mappable    : %s error(s)\n' "$whr"
  printf '  They are the same constraint on the same parameter. The inline form\n'
  printf '  is collected in the generics loop, the where form by\n'
  printf '  `collect_where_clause_bounds`; both must insert into\n'
  printf '  `func_param_protocol_bounds`. This disagreement is exactly how\n'
  printf '  T1075 was found.\n'
  exit 1
fi

printf 'check_hkt_bound: ok — violation refused with its own diagnostic (%s), valid admitted (%s), both spellings agree (%s)\n' \
  "$vio" "$val" "$whr"
