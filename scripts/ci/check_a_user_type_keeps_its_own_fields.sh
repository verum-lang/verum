#!/bin/sh
# check_a_user_type_keeps_its_own_fields.sh — a record the user declares
# must keep its own shape, whatever its name happens to be.
#
# WHY THIS EXISTS.  T1108, measured 2026-09-03.  The whole program:
#
#     type NtStatus is { magic: Int };
#     fn main() {
#         let v = NtStatus { magic: 7 };
#         print(v.magic + 1);        // expected 8
#     }
#
#     run   -> 40874348097      and a DIFFERENT number on the next run
#     check -> no diagnostics
#
# The value changes between runs of the same unchanged file, which is
# what proves it is a HEAP ADDRESS and not a wrong integer: `v.magic`
# hands back the record's boxed pointer, and it then takes part in
# integer arithmetic. Nothing on the path from source to that leak
# crosses a check.
#
# The name is the only thing that matters. `core/` declares
# `public type NtStatus is (Int32);` — a NEWTYPE — and the user declared
# a RECORD. The registry keys type identity by SIMPLE NAME (T0458), the
# user's record resolves to the stdlib newtype's shape, and field access
# reads the wrong layout.
#
# THE DISCRIMINATOR, and what it is NOT.  Three population hypotheses
# were refuted by controls before the real one held:
#
#   "the name is declared often in core/"     Thread (4 declarers) and
#                                             Context (3) both WORK
#   "the winner is a protocol"                AbelianGroup, a protocol
#                                             declared once, WORKS
#   "many declarers AND a protocol"           Universe (2 and 1) WORKS
#
# What holds: the user's record breaks when its simple name is that of a
# stdlib NEWTYPE that is MATERIALISED in this program's stdlib closure.
# Kind mismatch is necessary — record-vs-record collisions are harmless,
# which is why Config, Entry, Node, State, Task, Buffer, Event, Thread
# and Context all behave. Materialisation is necessary too: `BlockId`
# and `AllocHandle` are newtypes and are fine, because nothing pulls
# their modules in.
#
# Measured broken, 15 of ~30 probed: CFd, ChildId, CLong, CPid, CUid,
# HostFlavor, KernReturn, MachMsgOption, MachMsgReturn, MachPortRight,
# NtStatus, TaskFlavor, ThreadFlavor, VmInherit, VmProt.
#
# WHY THE ARMS ARE WHAT THEY ARE:
#   arithmetic   the field is read with `+ 1`, never printed as an
#                object. `print(v.magic)` renders the address as `{7}`,
#                which looks close enough to right to pass a careless
#                eye; `+ 1` cannot.
#   two runs     the same file is run TWICE and the two outputs compared
#                to each other, not only to `8`. An address differs
#                between runs, so agreement across runs is a second,
#                independent witness that the value is real.
#   control      a uniquely-named record runs in the same loop. If the
#                control breaks, the subject says nothing about names.
#   check/run    `verum check` must not stay silent on a program `verum
#                run` corrupts. The verdicts have to agree.
#
# Usage:
#   check_a_user_type_keeps_its_own_fields.sh [verum]
#   check_a_user_type_keeps_its_own_fields.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_user_type_fields: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# Content-keyed VBC cache: a fixed probe answers from cache on a second
# CI run and would report the FIRST run's verdict.
NONCE=$$_$(date +%s 2>/dev/null || echo 0)

write_case() { # $1 = type name
  d="$TMP/$1"
  mkdir -p "$d"
  {
    printf '// nonce %s-%s\n' "$NONCE" "$1"
    printf 'type %s is { magic: Int };\n\n' "$1"
    printf 'fn main() {\n'
    printf '    let v = %s { magic: 7 };\n' "$1"
    printf '    print(v.magic + 1);\n}\n'
  } > "$d/one.vr"
}

payload() { # $1 = type name -> the program's last output line
  (cd "$REPO" && timeout 300 "$VERUM" run "$TMP/$1/one.vr" 2>&1) |
    grep -vE '^\s*(Compiling|Checking|Parsing|Finished|Running|Building|Interpreting)' |
    grep -vE '^\[' | tail -1
}

check_errors() { # $1 = type name -> count of error lines from `verum check`
  (cd "$REPO" && timeout 300 "$VERUM" check "$TMP/$1/one.vr" 2>&1) | grep -cE '^error'
}

# One name known to collide, one that cannot. Keep the subject FIRST in
# the file so a reader sees what is being asserted before the plumbing.
SUBJECT=NtStatus
CONTROL=ZzqNoSuchTypeName

write_case "$SUBJECT"
write_case "$CONTROL"

ctl=$(payload "$CONTROL")
sub_a=$(payload "$SUBJECT")
sub_b=$(payload "$SUBJECT")
sub_check=$(check_errors "$SUBJECT")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'selftest: ok — control=%s subject_run1=%s subject_run2=%s subject_check_errors=%s\n' \
    "$ctl" "$sub_a" "$sub_b" "$sub_check"
fi

# The control comes first: if a uniquely-named record is broken, the
# subject proves nothing about name collisions.
if [ "$ctl" != "8" ]; then
  printf 'check_user_type_fields: FAILED — the CONTROL printed `%s`, expected `8`.\n' "$ctl"
  printf '  `type %s is { magic: Int }` collides with nothing. If reading a\n' "$CONTROL"
  printf '  field of a plain record is broken in general, the collision arm\n'
  printf '  below says nothing about names.\n'
  exit 1
fi

if [ "$sub_a" != "8" ]; then
  printf 'check_user_type_fields: FAILED — `%s` printed `%s`, expected `8`.\n' "$SUBJECT" "$sub_a"
  printf '  A record the user declares must keep its own fields. `core/`\n'
  printf '  declares `public type %s is (Int32);` — a NEWTYPE — and type\n' "$SUBJECT"
  printf '  identity is keyed by SIMPLE NAME, so `v.magic` read the wrong\n'
  printf '  layout and handed back the record pointer.\n'
  if [ "$sub_a" != "$sub_b" ]; then
    printf '  The two runs disagreed (`%s` vs `%s`) on an unchanged file: the\n' "$sub_a" "$sub_b"
    printf '  value is a HEAP ADDRESS, not merely a wrong number.\n'
  fi
  printf '  See T1108 and T0458 (module-qualified canonical type identity).\n'
  exit 1
fi

# A correct answer must also be a STABLE one. If the fix made the first
# run right by accident, two runs would still differ.
if [ "$sub_a" != "$sub_b" ]; then
  printf 'check_user_type_fields: FAILED — two runs of one unchanged file printed\n'
  printf '  `%s` then `%s`. A field read is deterministic; a differing value is\n' "$sub_a" "$sub_b"
  printf '  an address. Equality with `8` alone would not have caught this.\n'
  exit 1
fi

# check must not be silent about a program run corrupts.
if [ "$sub_a" = "8" ] && [ "$sub_check" -ne 0 ]; then
  printf 'check_user_type_fields: FAILED — `verum run` is correct but `verum check`\n'
  printf '  reported %s error(s) for the same file. The verdicts must agree.\n' "$sub_check"
  exit 1
fi

printf 'check_user_type_fields: ok — subject %s (%s, stable across runs), control %s (%s), check errors %s\n' \
  "$SUBJECT" "$sub_a" "$CONTROL" "$ctl" "$sub_check"
