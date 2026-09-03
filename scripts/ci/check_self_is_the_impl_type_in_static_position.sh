#!/bin/sh
# check_self_is_the_impl_type_in_static_position.sh — `Self.method()`
# inside an `implement T { … }` block means `T.method()`.
#
# WHY THIS EXISTS.  It did not.  Measured 2026-09-03 (T1087), one file,
# two lines differing in nothing else:
#
#     implement Opt2 {
#         public fn new() -> Self { Opt2 { n: 0 } }
#         public fn default() -> Self { Self.new() }   E400 no method
#         public fn default() -> Self { Opt2.new() }   clean
#     }
#
# `try_resolve_path_static_call` builds the lookup key from the
# receiver path's first segment.  It has an arm for `PathSegment::
# SelfValue` — the VALUE keyword `self` — and capital `Self` is lexed as
# an ordinary `Name`, so it fell through and the key became the literal
# `Self.new`, which no registry holds.  The neighbouring arm is the
# proof of intent: the rule was written for one of the two spellings.
#
# THE SAME SUBSTITUTION EXISTS TWICE MORE IN THE TREE — `resolve_type_name`
# for `Self` in type position and `SELF-NEWTYPE-CTOR-1` for the
# constructor form `Self(v)`.  This was the third spelling of one rule
# and the one nobody wrote, which is why the gate asserts the PROPERTY
# ("Self in static position is the impl type") rather than the case.
#
# WHAT IT ASSERTS, and why each arm is load-bearing:
#   self_static   `Self.new()` resolves
#   named_static  `Opt2.new()` still resolves — a repair that broke the
#                 ordinary spelling would otherwise pass
#   absent        `Self.no_such_method()` is still REFUSED, so the fix
#                 cannot be "resolve anything on Self"
#
# Usage:
#   check_self_is_the_impl_type_in_static_position.sh [verum]
#   check_self_is_the_impl_type_in_static_position.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_self_static: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

write_case() { # $1 = the call expression, $2 = out file
  cat > "$2" <<VR
module probe.self_static;

type Opt2 is { n: Int };

implement Opt2 {
    public fn new() -> Self { Opt2 { n: 0 } }
    public fn default() -> Self { $1 }
}
VR
}

write_case 'Self.new()'            "$TMP/self_static.vr"
write_case 'Opt2.new()'            "$TMP/named_static.vr"
write_case 'Self.no_such_method()' "$TMP/absent.vr"

if cmp -s "$TMP/self_static.vr" "$TMP/named_static.vr"; then
  printf 'check_self_static: the two subjects are IDENTICAL — the fixture is broken\n' >&2
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

sf=$(verdict "$TMP/self_static.vr")
nm=$(verdict "$TMP/named_static.vr")
ab=$(verdict "$TMP/absent.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.self_static_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  brk=$(verdict "$TMP/broken.vr")
  if [ "$brk" = MUTE ] || [ "$brk" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$brk"
    exit 1
  fi
  printf 'selftest: ok — self=%s named=%s absent=%s broken=%s\n' "$sf" "$nm" "$ab" "$brk"
fi

for pair in "self_static:$sf" "named_static:$nm" "absent:$ab"; do
  if [ "${pair#*:}" = MUTE ]; then
    printf 'check_self_static: FAILED — subject %s produced no output.\n' "${pair%%:*}"
    exit 1
  fi
done

# The control comes first: a repair that broke `Opt2.new()` would
# otherwise be hidden behind a passing subject.
if [ "$nm" != 0 ]; then
  printf 'check_self_static: FAILED — `Opt2.new()` reported %s error(s).\n' "$nm"
  printf '  The ordinary spelling of a static call must keep working; a fix\n'
  printf '  for `Self` that costs the named form is not a fix.\n'
  exit 1
fi

if [ "$sf" != 0 ]; then
  printf 'check_self_static: FAILED — `Self.new()` reported %s error(s).\n' "$sf"
  printf '  Inside `implement Opt2 { … }`, `Self.new()` is `Opt2.new()`.\n'
  printf '  See SELF-STATIC-RECEIVER in infer/modules.rs: the lookup key is\n'
  printf '  built from the receiver path, whose `SelfValue` arm resolves the\n'
  printf '  lowercase keyword while capital `Self` arrives as a plain Name.\n'
  exit 1
fi

# Without this arm the gate passes for "resolve anything on Self".
if [ "$ab" = 0 ]; then
  printf 'check_self_static: FAILED — `Self.no_such_method()` compiled clean.\n'
  printf '  Substituting the impl type for `Self` must not also make every\n'
  printf '  method name resolve. Silence here means the gate above proves\n'
  printf '  nothing.\n'
  exit 1
fi

printf 'check_self_static: ok — Self.new resolves (%s), Opt2.new still resolves (%s), an absent method is still refused (%s)\n' \
  "$sf" "$nm" "$ab"
