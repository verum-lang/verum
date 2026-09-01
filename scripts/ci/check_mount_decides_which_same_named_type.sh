#!/bin/sh
# check_mount_decides_which_same_named_type.sh — when several stdlib
# modules declare a type with the SAME name, the file's own `mount` must
# decide which one a qualified constructor means.
#
# WHY THIS EXISTS.  `core/` declares FOUR public types named
# `Capability` (core/mem/capability.vr, core/logic/separation.vr,
# core/architecture/types.vr, and a protocol in
# core/database/common/capability.vr).  Measured 2026-09-02 (T0755),
# with the intended one mounted:
#
#     mount core.mem.capability.{Capability};
#     let c: Capability = Capability.Read;
#
#     error<E400>: Type mismatch:
#       expected 'Read(Unit) | Write(Unit) | Execute(Unit) | …'
#       found    'Read(ResourceTag) | Exec(ExecTarget) | …'
#
# BOTH sides are types named `Capability`.  The ANNOTATION went through
# mount authority and got the mounted one; the CONSTRUCTOR took the flat
# `Type.Variant` env slot, which is written with an unguarded
# `insert_mono` — last-write-wins across every same-named type — and got
# another module's.  Two registries, opposite winners, in one line of
# user code.
#
# The mount-authority strategy for constructor syntax already existed
# (`Strategy 1.5 / MOUNT-TYPE-AUTHORITY-1` in `infer/expr.rs`) and its
# own comment names this exact failure; it simply sat AFTER the flat
# strategy that answers first.
#
# WHAT IT ASSERTS.  Two files that differ only in whether the mounted
# type's name is contested must both compile.  Agreement is not enough
# on its own — two broken subjects agree — so a clean verdict is
# required, and the selftest proves the checker can still say "no".
#
# WHY A SCRIPT AND NOT A UNIT TEST.  The defect needs several stdlib
# modules registering the same type name in a particular order; a
# hand-built TypeChecker registers whatever the test author writes, in
# the order they write it, which is the input that does NOT reproduce
# it.  Same reasoning as check_type_parameter_letter_is_irrelevant.sh.
#
# Usage:
#   check_mount_decides_which_same_named_type.sh [verum]
#   check_mount_decides_which_same_named_type.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_mount_decides: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# `Capability` is declared by four modules; `IOInterest` by one.
# Everything else about the two files matches.
cat > "$TMP/contested.vr" <<'VR'
module probe.mount_decides_contested;
mount core.prelude.*;
mount core.mem.capability.{Capability};

public fn probe() -> Int {
    let c: Capability = Capability.Read;
    let _ = c;
    0
}
VR

cat > "$TMP/uncontested.vr" <<'VR'
module probe.mount_decides_uncontested;
mount core.prelude.*;
mount core.sys.io_engine.{IOInterest};

public fn probe() -> Int {
    let c: IOInterest = IOInterest.Read;
    let _ = c;
    0
}
VR

# A fixture comparing two byte-identical subjects proves nothing.
if cmp -s "$TMP/contested.vr" "$TMP/uncontested.vr"; then
  printf 'check_mount_decides: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

verdict() { # $1 file -> error count, or MUTE
  out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}

a=$(cd "$REPO" && verdict "$TMP/contested.vr")
b=$(cd "$REPO" && verdict "$TMP/uncontested.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.mount_decides_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  c=$(cd "$REPO" && verdict "$TMP/broken.vr")
  if [ "$b" = "$c" ]; then
    printf 'selftest: FAILED — a clean subject (%s) and a broken one (%s) compare equal\n' "$b" "$c"
    exit 1
  fi
  printf 'selftest: ok — clean=%s broken=%s are distinguishable\n' "$b" "$c"
fi

if [ "$a" = MUTE ] || [ "$b" = MUTE ]; then
  printf 'check_mount_decides: FAILED — a subject produced no output (contested=%s uncontested=%s).\n' "$a" "$b"
  printf '  Saying nothing is a broken run, not a clean one.\n'
  exit 1
fi

if [ "$a" != "$b" ]; then
  printf 'check_mount_decides: FAILED — a contested type name changes the verdict.\n'
  printf '  Capability.Read  (four modules declare `Capability`): %s error(s)\n' "$a"
  printf '  IOInterest.Read  (one module declares it)           : %s error(s)\n' "$b"
  printf '  Both files mount the type they use and annotate with the same\n'
  printf '  name they construct. How many OTHER modules happen to reuse the\n'
  printf '  name is not a fact about this file.\n'
  printf '  Look for a strategy that answers from the flat `Type.Variant`\n'
  printf '  env slot BEFORE mount authority (Strategy 1 vs Strategy 1.5 in\n'
  printf '  infer/expr.rs). That slot is last-write-wins (T0755).\n'
  exit 1
fi

if [ "$a" != 0 ]; then
  printf 'check_mount_decides: FAILED — both subjects report %s error(s).\n' "$a"
  printf '  They agree, but agreement between two broken subjects is not the\n'
  printf '  property this gate exists for. Run them by hand:\n'
  printf '    %s check <file>\n' "$VERUM"
  exit 1
fi

printf 'check_mount_decides: ok — contested and uncontested both clean (%s errors)\n' "$a"
