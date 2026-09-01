#!/bin/sh
# check_type_parameter_letter_is_irrelevant.sh — the NAME of a type
# parameter must not decide whether a file compiles.
#
# WHY THIS EXISTS.  `implement Bag<T> { … }` — the spelling with no `<T>`
# after `implement` — binds its parameters by extracting them from the
# target type.  The guard was "is this name free?":
#
#     name is uppercase && self.ctx.lookup_type(name).is_none()
#
# and every impl block registered earlier had defined ITS `T` into the
# same global type namespace, with nothing taking it back out.  From the
# second impl onwards `T` looked occupied, so the extraction never ran
# for it.  Measured 2026-09-01 (T1040), two files identical but for one
# letter:
#
#     implement Bag<T>    error<E404>: Ambiguous type … `List<_>`
#     implement Bag<Wq>   clean
#
# WHY A SCRIPT AND NOT A UNIT TEST.  It was tried, twice.  A bare
# `TypeChecker` has no `List`, so the subject dies on
# `TypeNotFound { "List" }` before inference can be ambiguous; and with a
# hand-rolled container the bound parameter is a fresh type VARIABLE, so
# `Vecish<t>` unifies with `Vecish<Int>` and there is no mismatch to
# assert either.  The first attempt asserted an ABSENCE and passed on
# broken code — an assertion that something is missing passes for free
# when the subject never gets that far.  This gate uses the real
# compiler, where the defect actually appears.
#
# Usage:
#   check_type_parameter_letter_is_irrelevant.sh [verum]
#   check_type_parameter_letter_is_irrelevant.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_param_letter: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# `T` is the name every stdlib impl claims first; `Wq` is one nothing
# claims. Everything else about the two files is identical.
write_subject() { # $1 = parameter name, $2 = out file
  cat > "$2" <<VR
module probe.param_letter_$1;

public type Bag<$1> is { items: List<$1> };

implement Bag<$1> {
    public fn all(&self) -> List<$1> { self.items.clone() }
    public fn probe(&self) -> Int { let x = self.all(); x.len() }
}
VR
}

write_subject T "$TMP/claimed.vr"
write_subject Wq "$TMP/unclaimed.vr"

# A control that costs one line and catches every rename that silently
# did nothing — `sed`'s `\b` is unsupported on BSD, and a "control" byte
# identical to its subject agrees with it about everything.
if cmp -s "$TMP/claimed.vr" "$TMP/unclaimed.vr"; then
  printf 'check_param_letter: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

verdict() { # $1 file -> error count, or MUTE
  out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}

a=$(cd "$REPO" && verdict "$TMP/claimed.vr")
b=$(cd "$REPO" && verdict "$TMP/unclaimed.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.param_letter_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  c=$(cd "$REPO" && verdict "$TMP/broken.vr")
  if [ "$b" = "$c" ]; then
    printf 'selftest: FAILED — a clean subject (%s) and a broken one (%s) compare equal\n' "$b" "$c"
    exit 1
  fi
  case "$a$b$c" in
    *MUTE*) printf 'selftest: FAILED — a run printed nothing (%s/%s/%s)\n' "$a" "$b" "$c"; exit 1 ;;
  esac
  printf 'selftest: ok (claimed=%s unclaimed=%s broken=%s)\n' "$a" "$b" "$c"
fi

case "$a$b" in
  *MUTE*)
    printf 'check_param_letter: a run printed NOTHING (T=%s Wq=%s).\n' "$a" "$b"
    printf '  That is a tool failure, not a clean result — check `df -h` and the binary.\n'
    exit 2 ;;
esac

if [ "$a" != "$b" ]; then
  printf 'check_param_letter: the same file compiles differently by the LETTER\n'
  printf 'of its type parameter.\n\n'
  printf '  implement Bag<T>   %s error(s)\n' "$a"
  printf '  implement Bag<Wq>  %s error(s)\n' "$b"
  printf '\nThe two files differ in nothing else. A name another impl block\n'
  printf 'happens to have registered is being read as an existing TYPE (T1040).\n'
  exit 1
fi

printf 'check_param_letter: the parameter name does not change the verdict (%s error(s) both ways)\n' "$a"
exit 0
