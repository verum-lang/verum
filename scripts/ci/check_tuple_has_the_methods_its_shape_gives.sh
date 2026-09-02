#!/bin/sh
# check_tuple_has_the_methods_its_shape_gives.sh — a tuple must not lose
# methods a record of the same components would have, and `check` and
# `run` must agree about it.
#
# WHY THIS EXISTS.  `lookup_method` and `lookup_method_with_args` both
# open with `extract_type_info(ty)?`, the type→method-registry-key
# mapping.  It answers for Int, Text, Array, Slice, variants, named types
# and the EMPTY tuple (canonically `Unit`), and falls through for
# `(A, B)` — so `?` left the whole function and NO method resolved on a
# non-empty tuple.  The "universal methods that work on all types"
# fallback sat at step 2, i.e. BEHIND a guard requiring the registry
# entry it exists to substitute for: unreachable for exactly the set it
# serves.  Measured 2026-09-02:
#
#     let z = Zz { a: 1 };            z.clone()   clean, no @derive needed
#     let p: (Int, Float) = (1, 2.0); p.clone()
#         error<E400>: no method named `clone` found for type `(Int, Float)`
#
#     nine such call sites in core/ under VERUM_STRICT_STDLIB
#
# WHY IT CHECKS `run` AND NOT ONLY `check`.  This tree has a live class
# where the two disagree — `verum check` exits 0 on a program `verum run`
# rejects (T1060/T1062).  A repair that teaches the typechecker to accept
# something the lowering cannot do would move the failure later instead
# of fixing it, and a check-only gate would call that a success.  The
# runtime copies tuples already (a record holding one clones and prints
# its components), so both halves must pass.
#
# SCOPE.  `clone` only.  `to_string` on a tuple needs every component to
# be renderable — a per-element judgement — and is deliberately NOT
# asserted here; asserting it would gate a decision nobody has taken.
#
# Usage:
#   check_tuple_has_the_methods_its_shape_gives.sh [verum]
#   check_tuple_has_the_methods_its_shape_gives.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_tuple_methods: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# The two subjects differ ONLY in whether the components sit in a tuple
# or in a record — the shape whose methods went missing.
cat > "$TMP/tuple.vr" <<'VR'
module probe.tuple_methods;
mount core.prelude.*;

fn main() -> Unit {
    let p: (Int, Float) = (7, 2.5);
    let q = p.clone();
    print(f"{q.0} {q.1}");
}
VR

cat > "$TMP/record.vr" <<'VR'
module probe.record_methods;
mount core.prelude.*;

public type Pair is { a: Int, b: Float };

fn main() -> Unit {
    let p = Pair { a: 7, b: 2.5 };
    let q = p.clone();
    print(f"{q.a} {q.b}");
}
VR

if cmp -s "$TMP/tuple.vr" "$TMP/record.vr"; then
  printf 'check_tuple_methods: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

errcount() { # $1 file -> error count, or MUTE
  out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}
runout() { # $1 file -> the program's last output line, or FAILED
  out=$(timeout 180 "$VERUM" run "$1" 2>&1) || { echo FAILED; return; }
  printf '%s' "$out" | grep -v 'Running\|Compiling\|Finished\|WARN' | tail -1
}

t_chk=$(cd "$REPO" && errcount "$TMP/tuple.vr")
r_chk=$(cd "$REPO" && errcount "$TMP/record.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.tuple_broken;\n\nfn main() -> Unit { no_such_name_xyz(); }\n' > "$TMP/broken.vr"
  b=$(cd "$REPO" && errcount "$TMP/broken.vr")
  if [ "$b" = MUTE ] || [ "$b" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$b"
    exit 1
  fi
  printf 'selftest: ok — tuple=%s record=%s broken=%s\n' "$t_chk" "$r_chk" "$b"
fi

if [ "$r_chk" != 0 ]; then
  printf 'check_tuple_methods: FAILED — the RECORD control does not compile (%s error(s)).\n' "$r_chk"
  printf '  The comparison is only meaningful while the control is clean.\n'
  exit 1
fi

if [ "$t_chk" != "$r_chk" ]; then
  printf 'check_tuple_methods: FAILED — a tuple lost a method its record twin keeps.\n'
  printf '  (Int, Float).clone() : %s error(s)\n' "$t_chk"
  printf '  { a: Int, b: Float } : %s error(s)\n' "$r_chk"
  printf '  Both lookup entry points open with `extract_type_info(ty)?`, which\n'
  printf '  has no arm for a non-empty tuple — so the receiver leaves the\n'
  printf '  function before any shape-based fallback runs. See\n'
  printf '  `lookup_structural_universal_method` in protocol.rs; BOTH\n'
  printf '  `lookup_method` and `lookup_method_with_args` must call it.\n'
  exit 1
fi

# check agreeing is not enough: the lowering has to do it too.
t_run=$(cd "$REPO" && runout "$TMP/tuple.vr")
if [ "$t_run" != "7 2.5" ]; then
  printf 'check_tuple_methods: FAILED — `check` accepts the clone, `run` does not.\n'
  printf '  expected the program to print: 7 2.5\n'
  printf '  got: %s\n' "$t_run"
  printf '  A typechecker taught to accept what the lowering cannot do moves\n'
  printf '  the failure later; that is the T1060/T1062 class, not a fix.\n'
  exit 1
fi

printf 'check_tuple_methods: ok — tuple and record both clean, and the clone runs (%s)\n' "$t_run"
