#!/bin/sh
# check_call_through_fn_bound_has_the_declared_result.sh — calling a type
# parameter bounded by a function type yields the type that bound declares.
#
# WHY THIS EXISTS.  It did not.  Measured 2026-09-03 (T0755), two files
# differing in one thing and nothing else:
#
#     fn via_bound<U, F: fn(Int) -> List<U>>(f: F) -> Int {
#         let inner = f(1);        error<E404>: `inner` is `_`
#         inner.len()
#     }
#     fn via_direct<U>(f: fn(Int) -> List<U>) -> Int {
#         let inner = f(1);        clean
#         inner.len()
#     }
#
# The two spellings mean the same thing and only one worked.  `bind_var`
# PERMITS a bounded parameter to take a function shape — the bound
# promised one — but the rigid-variable table recorded only THAT a bound
# exists, never BY WHAT: `Map<TypeVar, (Text, bool)>`.  So the call site
# invented a signature out of fresh variables, unification had nothing to
# narrow it with, and the result type stayed unsolved.
#
# The stdlib pays for it directly.  `core/collections/deque.vr` declares
#
#     public fn flat_map<U, F: fn(&T) -> List<U>>(&self, f: F) -> Deque<U>
#
# and `let inner = f(item);` inside it is the same E404 — which is also
# why this gate matters beyond the two probe files: the diagnostic is
# SWALLOWED under `core/`'s lenient mode, so it read as one more line in
# a known-failures baseline rather than as a defect with a root.
#
# WHAT IT ASSERTS, and why each arm is load-bearing:
#   bound     the bounded spelling compiles clean
#   direct    the direct spelling still compiles clean — a repair that
#             cost the working spelling is not a repair
#   wrong     a call whose ARGUMENT contradicts the bound is still
#             REFUSED, so the fix cannot be "make the callee Unknown";
#             without this arm the gate passes for a repair that simply
#             stopped checking calls through bounded parameters
#   unbounded `fn g<T>(t: T) { t(1) }` — calling an UNBOUNDED parameter
#             is still refused, so the fix cannot be "any type variable
#             is callable"
#
# Usage:
#   check_call_through_fn_bound_has_the_declared_result.sh [verum]
#   check_call_through_fn_bound_has_the_declared_result.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_fn_bound_call: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/bound.vr" <<'VR'
module probe.fn_bound_call;

public fn via_bound<U, F: fn(Int) -> List<U>>(f: F) -> Int {
    let inner = f(1);
    inner.len()
}
VR

cat > "$TMP/direct.vr" <<'VR'
module probe.fn_direct_call;

public fn via_direct<U>(f: fn(Int) -> List<U>) -> Int {
    let inner = f(1);
    inner.len()
}
VR

# The argument contradicts the declared parameter type. If the repair
# made the callee Unknown instead of reading its bound, this compiles
# clean and the gate above proves nothing.
cat > "$TMP/wrong.vr" <<'VR'
module probe.fn_bound_wrong_arg;

public fn via_bound_wrong<U, F: fn(Int) -> List<U>>(f: F) -> Int {
    let inner = f(Text.from("not an Int"));
    inner.len()
}
VR

# An UNBOUNDED parameter promises nothing, so calling it is an error and
# must stay one.
cat > "$TMP/unbounded.vr" <<'VR'
module probe.fn_unbounded_call;

public fn call_unbounded<T>(t: T) -> Int {
    let inner = t(1);
    0
}
VR

if cmp -s "$TMP/bound.vr" "$TMP/direct.vr"; then
  printf 'check_fn_bound_call: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

verdict() { # $1 file -> error count, or MUTE
  out=$(cd "$REPO" && timeout 300 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  n=$(printf '%s' "$out" | grep -c '^error<')
  echo "$n"
}

bd=$(verdict "$TMP/bound.vr")
dr=$(verdict "$TMP/direct.vr")
wr=$(verdict "$TMP/wrong.vr")
ub=$(verdict "$TMP/unbounded.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.fn_bound_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  brk=$(verdict "$TMP/broken.vr")
  if [ "$brk" = MUTE ] || [ "$brk" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$brk"
    exit 1
  fi
  printf 'selftest: ok — bound=%s direct=%s wrong=%s unbounded=%s broken=%s\n' \
    "$bd" "$dr" "$wr" "$ub" "$brk"
fi

for pair in "bound:$bd" "direct:$dr" "wrong:$wr" "unbounded:$ub"; do
  if [ "${pair#*:}" = MUTE ]; then
    printf 'check_fn_bound_call: FAILED — subject %s produced no output.\n' "${pair%%:*}"
    exit 1
  fi
done

# The control comes first: a repair that broke the direct spelling would
# otherwise hide behind a passing subject.
if [ "$dr" != 0 ]; then
  printf 'check_fn_bound_call: FAILED — the DIRECT spelling reported %s error(s).\n' "$dr"
  printf '  `fn via_direct<U>(f: fn(Int) -> List<U>)` worked before this gate\n'
  printf '  existed; a fix for the bounded spelling that costs it is not a fix.\n'
  exit 1
fi

if [ "$bd" != 0 ]; then
  printf 'check_fn_bound_call: FAILED — the BOUNDED spelling reported %s error(s).\n' "$bd"
  printf '  `fn via_bound<U, F: fn(Int) -> List<U>>(f: F)` calling `f(1)` must\n'
  printf '  produce `List<U>`, exactly as the direct spelling does. The bound is\n'
  printf '  carried in `RigidVar::fn_bound` (verum_types/src/unify.rs) and read\n'
  printf '  where `func_ty` is normalised in infer/expr.rs; if either half is\n'
  printf '  gone the result type falls back to a fresh variable and surfaces as\n'
  printf '  `error<E404>: Ambiguous type`.\n'
  exit 1
fi

# Without this arm the gate passes for "stop checking calls through a
# bounded parameter at all".
if [ "$wr" = 0 ]; then
  printf 'check_fn_bound_call: FAILED — an argument contradicting the bound compiled clean.\n'
  printf '  `f(Text.from("not an Int"))` against `F: fn(Int) -> List<U>` must be\n'
  printf '  refused. Reading the bound has to CHECK the call, not merely stop\n'
  printf '  asking questions about it.\n'
  exit 1
fi

if [ "$ub" = 0 ]; then
  printf 'check_fn_bound_call: FAILED — calling an UNBOUNDED parameter compiled clean.\n'
  printf '  `fn call_unbounded<T>(t: T) { t(1) }` promises nothing about `T`, so\n'
  printf '  the call is an error. If this passes, the repair made every type\n'
  printf '  variable callable rather than reading the declared bound.\n'
  exit 1
fi

printf 'check_fn_bound_call: ok — bounded spelling clean (%s), direct still clean (%s), bad argument refused (%s), unbounded call refused (%s)\n' \
  "$bd" "$dr" "$wr" "$ub"
