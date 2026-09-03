#!/bin/sh
# check_a_generic_type_satisfies_a_bound_its_impl_provides.sh — an
# associated type written as a generic APPLICATION satisfies a protocol
# bound when a generic impl provides it.
#
# WHY THIS EXISTS.  T1098: 40 false E405 diagnostics across a 14-file
# core/ sample, measured 2026-09-03 against a 2026-08-31 binary that
# reported zero:
#
#     core/base/iterator.vr   0 -> 38
#     core/base/maybe.vr      0 ->  2   (the file had been CLEAN)
#
#     error<E405>: `type IntoIter = MaybeIter<T>` does not satisfy the
#                  bound `Iterator` that `IntoIterator` requires
#
# while `core/base/maybe.vr:511` declares, sixteen lines above the
# complaint, `implement<T> Iterator for MaybeIter<T>`.  The same file
# for `Rev<@builtin_interval>` against FIVE `implement … for Rev…`.
#
# ROOT: `make_type_key` renders a type WITH its arguments, and
# `implements_protocol_any` compared whole keys — so the query
# `MaybeIter<T>` never equalled the key the generic impl registered.
# The method's name, its doc ("ignoring type args") and its only caller
# `implements_by_name` ("the ANY-instantiation question") all state the
# opposite of what it did.  The check had been unreachable until a
# neighbouring repair loaded the protocol definitions that open its
# `known` guard, so the wrongness is older than its visibility.
#
# WHAT IT ASSERTS, and why each arm is load-bearing:
#   generic     a generic application satisfies a bound its generic impl
#               provides — the false positive itself
#   concrete    the non-generic spelling still works, so the repair is
#               not "stop asking"
#   absent      a type with NO such impl is STILL refused. Without this
#               the gate passes for a repair that made the predicate
#               unconditionally true, which is exactly how a
#               false-positive fix becomes a false-negative one
#   corpus      `core/base/maybe.vr` — the real file that regressed —
#               reports no E405
#
# Usage:
#   check_a_generic_type_satisfies_a_bound_its_impl_provides.sh [verum]
#   check_a_generic_type_satisfies_a_bound_its_impl_provides.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_generic_bound: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# The probes carry NO suite header on purpose: a file whose first ten
# lines contain `// @test:` is compiled with lenient context resolution,
# and a gate about diagnostics must not opt into a mode that suppresses
# them.
cat > "$TMP/generic.vr" <<'VR'
module probe.generic_bound;

type Holder is protocol {
    type Inner: Shown;
    fn get(&self) -> Self.Inner;
};

type Shown is protocol {
    fn show(&self) -> Int;
};

type Wrap<T> is { v: T };

implement<T> Shown for Wrap<T> {
    fn show(&self) -> Int { 1 }
}

implement<T> Holder for Wrap<T> {
    type Inner = Wrap<T>;
    fn get(&self) -> Wrap<T> { Wrap { v: self.v } }
}
VR

cat > "$TMP/concrete.vr" <<'VR'
module probe.concrete_bound;

type Shown2 is protocol {
    fn show(&self) -> Int;
};

type Holder2 is protocol {
    type Inner: Shown2;
    fn get(&self) -> Self.Inner;
};

type Plain is { v: Int };

implement Shown2 for Plain {
    fn show(&self) -> Int { 1 }
}

implement Holder2 for Plain {
    type Inner = Plain;
    fn get(&self) -> Plain { Plain { v: self.v } }
}
VR

# No `Shown3` impl for `Bare` anywhere: the bound is genuinely unmet.
cat > "$TMP/absent.vr" <<'VR'
module probe.absent_bound;

type Shown3 is protocol {
    fn show(&self) -> Int;
};

type Holder3 is protocol {
    type Inner: Shown3;
    fn get(&self) -> Self.Inner;
};

type Bare is { v: Int };

implement Holder3 for Bare {
    type Inner = Bare;
    fn get(&self) -> Bare { Bare { v: self.v } }
}
VR

count_e405() { # $1 file -> count, or MUTE
  out=$(cd "$REPO" && env VERUM_STRICT_STDLIB=1 timeout 300 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c 'E405' | tr -d ' '
}

gen=$(count_e405 "$TMP/generic.vr")
con=$(count_e405 "$TMP/concrete.vr")
abs=$(count_e405 "$TMP/absent.vr")
corpus=SKIP
if [ -f "$REPO/core/base/maybe.vr" ]; then
  corpus=$(count_e405 "$REPO/core/base/maybe.vr")
fi

if [ "$SELFTEST" -eq 1 ]; then
  printf 'selftest: ok — generic=%s concrete=%s absent=%s maybe.vr=%s\n' \
    "$gen" "$con" "$abs" "$corpus"
fi

for pair in "generic:$gen" "concrete:$con" "absent:$abs"; do
  if [ "${pair#*:}" = MUTE ]; then
    printf 'check_generic_bound: FAILED — subject %s produced no output.\n' "${pair%%:*}"
    exit 1
  fi
done

# The negative control comes FIRST. A repair that made the predicate
# unconditionally true would pass every arm below it.
if [ "$abs" -eq 0 ]; then
  printf 'check_generic_bound: FAILED — a genuinely unmet bound produced no E405.\n'
  printf '  `type Inner = Bare` with no `implement Shown3 for Bare` anywhere must\n'
  printf '  still be refused. Relaxing the ANY-instantiation question must not\n'
  printf '  relax the question of whether ANY impl exists at all.\n'
  exit 1
fi

if [ "$con" -ne 0 ]; then
  printf 'check_generic_bound: FAILED — the CONCRETE spelling reported %s E405.\n' "$con"
  printf '  A non-generic type whose impl is right there worked before this gate\n'
  printf '  existed; if it is broken, the generic arm below says nothing.\n'
  exit 1
fi

if [ "$gen" -ne 0 ]; then
  printf 'check_generic_bound: FAILED — a GENERIC application reported %s E405.\n' "$gen"
  printf '  `type Inner = Wrap<T>` with `implement<T> Shown for Wrap<T>` in the\n'
  printf '  same file must satisfy the bound. `make_type_key` renders the type\n'
  printf '  WITH its arguments; `implements_protocol_any` must compare the base,\n'
  printf '  as its name, its doc and `implements_by_name` all say. See T1098.\n'
  exit 1
fi

if [ "$corpus" != SKIP ] && [ "$corpus" -ne 0 ]; then
  printf 'check_generic_bound: FAILED — core/base/maybe.vr reported %s E405.\n' "$corpus"
  printf '  That file was clean on 2026-08-31 and reported 2 after the check\n'
  printf '  became reachable, both naming `MaybeIter<T>` — whose `Iterator` impl\n'
  printf '  is declared at line 511 of the same file.\n'
  exit 1
fi

printf 'check_generic_bound: ok — generic application accepted (%s), concrete still accepted (%s), unmet bound still refused (%s), maybe.vr clean (%s)\n' \
  "$gen" "$con" "$abs" "$corpus"
