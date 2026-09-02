#!/bin/sh
# check_context_is_not_a_type_bound.sh — a `context` declared without
# `protocol` must be refused where a type bound is expected.
#
# WHY THIS EXISTS.  Contexts and protocol bounds are different
# mechanisms: a context is runtime dependency injection (`using [X]`), a
# bound is a compile-time constraint.  CLAUDE.md keeps them apart on
# purpose.  The compiler mixed them in silence — measured 2026-09-02
# (T1076):
#
#     public context Clocky { fn now_ms(&self) -> Int; }
#     public fn takes<T: Clocky>(x: T) -> Int { 1 }
#       -> clean; the context was accepted as an ordinary bound
#     takes(Zed { a: 1 })
#       -> E405: type `Zed` does not implement `Clocky`
#
# The author learned about it at the CALL rather than the declaration,
# and the message asked them to implement a context — advice that cannot
# be followed.
#
# `ContextResolver::validate_as_type_bound` was written for exactly this
# and had ONE mention in the tree: its own definition.  A stub elsewhere
# computed the same condition with a comment for a body ("emit warning
# but don't block … Full error would require span info not available
# here").
#
# WHY A GATE AND NOT A CORPUS MEASUREMENT.  `core/` declares 33
# injectable contexts and uses NONE of them as a bound — the radius is
# zero, so no sweep can show this working or broken.  Without a gate the
# repair is unverifiable: there is no subject in the tree.  The gate IS
# the subject.
#
# Usage:
#   check_context_is_not_a_type_bound.sh [verum]
#   check_context_is_not_a_type_bound.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_context_bound: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# The two files differ ONLY in the `protocol` keyword: one declares an
# injectable-only context, the other a context that is also a protocol
# and is therefore legitimate as a bound.  Everything else — the method,
# the bound, the function — is identical, so a verdict difference can
# only come from that keyword.
cat > "$TMP/injectable.vr" <<'VR'
module probe.context_bound_injectable;
mount core.prelude.*;

public context Clocky {
    fn now_ms(&self) -> Int;
}

public fn takes<T: Clocky>(x: T) -> Int { 1 }
VR

cat > "$TMP/protocol.vr" <<'VR'
module probe.context_bound_protocol;
mount core.prelude.*;

public context protocol Clocky {
    fn now_ms(&self) -> Int;
}

public fn takes<T: Clocky>(x: T) -> Int { 1 }
VR

if cmp -s "$TMP/injectable.vr" "$TMP/protocol.vr"; then
  printf 'check_context_bound: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

verdict() { # $1 file -> error count, or MUTE
  out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}
says_context_bound() { # $1 file -> 1 when the diagnostic names the reason
  timeout 180 "$VERUM" check "$1" 2>&1 |
    grep -c 'cannot be used as a type bound' || true
}

inj=$(cd "$REPO" && verdict "$TMP/injectable.vr")
proto=$(cd "$REPO" && verdict "$TMP/protocol.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.context_bound_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  broken=$(cd "$REPO" && verdict "$TMP/broken.vr")
  if [ "$broken" = MUTE ] || [ "$broken" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$broken"
    exit 1
  fi
  printf 'selftest: ok — injectable=%s protocol=%s broken=%s\n' "$inj" "$proto" "$broken"
fi

if [ "$inj" = MUTE ] || [ "$proto" = MUTE ]; then
  printf 'check_context_bound: FAILED — a subject produced no output (%s / %s).\n' "$inj" "$proto"
  exit 1
fi

# The `context protocol` form is the CONTROL: it must stay legal.  A
# repair that refuses both would pass an "injectable is rejected" test
# while breaking the feature the keyword exists for.
if [ "$proto" != 0 ]; then
  printf 'check_context_bound: FAILED — `context protocol` was refused as a bound (%s errors).\n' "$proto"
  printf '  That form exists PRECISELY to be usable as both a bound and an\n'
  printf '  injectable context; refusing it trades one wrong verdict for another.\n'
  exit 1
fi

if [ "$inj" = 0 ]; then
  printf 'check_context_bound: FAILED — an injectable-only context was accepted as a type bound.\n'
  printf '  `public context Clocky { … }`  then  `fn takes<T: Clocky>(…)`  compiled clean.\n'
  printf '  A context is runtime dependency injection; a bound is a compile-time\n'
  printf '  constraint. Accepting one for the other defers the error to the call\n'
  printf '  site, where the message asks the author to implement a context.\n'
  printf '  See `ContextResolver::validate_as_type_bound` and its call in\n'
  printf '  `convert_type_bounds_to_protocol_bounds` (infer/env.rs).\n'
  exit 1
fi

# Rejected is necessary but not sufficient: it must be rejected FOR THIS
# REASON.  Any unrelated breakage would otherwise satisfy the gate.
if [ "$(cd "$REPO" && says_context_bound "$TMP/injectable.vr")" -eq 0 ]; then
  printf 'check_context_bound: FAILED — refused, but not for this reason.\n'
  printf '  The subject reports %s error(s) and none says "cannot be used as a\n' "$inj"
  printf '  type bound". A gate keyed on "did it fail" would pass on any\n'
  printf '  unrelated breakage in the fixture.\n'
  exit 1
fi

printf 'check_context_bound: ok — injectable refused with its own diagnostic (%s), `context protocol` still legal (%s)\n' \
  "$inj" "$proto"
