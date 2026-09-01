#!/bin/sh
# check_path_does_not_change_verdict.sh — where a file SITS must not
# change what the compiler says about it.
#
# WHY THIS EXISTS.  `is_stdlib_file` was decided by a substring of the
# input path:
#
#     p.contains("/core/") || p.starts_with("core/")
#
# so any file under any directory called `core` was checked under the
# standard library's rules.  Measured 2026-09-01 (T1041): over a 25-file
# sample the flag changed the verdict for 13 of them, in BOTH
# directions — one file reported 52 errors in a scratch `core/`
# directory where the same bytes beside it reported 10.
#
# The direction that matters to a user is the other one.  This gate's
# own subject, on a pre-repair binary, reports:
#
#     in a directory named `core`:  0 errors
#     beside it:                    2 errors
#
# — two real errors HIDDEN because of where the file sits.  A project
# with a `src/core/` had its diagnostics silently suppressed, and
# nothing said so.
#
# The repair anchors on the tree's own marker: an ancestor directory
# named `core` that holds `mod.vr` IS the standard library's root.  This
# gate pins the consequence, which is the part a user can feel: a file
# that is NOT part of the standard library gets one verdict, wherever it
# sits.
#
# Deliberately NOT asserted here: what the mode DOES.  It is not simply
# lenient — several of its arms answer `Type::Unknown`, which surfaces
# later as "not fully determined" — and nobody has characterised that.
# This gate is about WHO the mode applies to.
#
# Usage:
#   check_path_does_not_change_verdict.sh [verum]
#   check_path_does_not_change_verdict.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_path_verdict: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/core" "$TMP/plain"

# The subject needs to EXERCISE the mode, or the comparison passes on a
# file the flag never touched.  A generic container with an iterator is
# what the 2026-09-01 measurement found the flag moving most.
cat > "$TMP/subject.vr" <<'VR'
module probe.path_verdict;

public type Bag<T> is { items: List<T> };

implement<T> Bag<T> {
    public fn holds(&self, v: &T) -> Bool where T: Eq { self.items.contains(v) }
    public fn covers(&self, other: &Bag<T>) -> Bool where T: Eq {
        self.items.iter().all(|v| other.holds(v))
    }
    public fn count(&self) -> Int { self.items.len() }
}
VR
cp "$TMP/subject.vr" "$TMP/core/subject.vr"
cp "$TMP/subject.vr" "$TMP/plain/subject.vr"

# `$TMP/core/` has NO mod.vr, so it is a directory that merely shares
# the standard library's name — which is exactly the case that used to
# be misread.
[ -e "$TMP/core/mod.vr" ] && { echo 'check_path_verdict: fixture is wrong' >&2; exit 2; }

verdict() { # $1 file -> error count, or MUTE
  out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}

a=$(cd "$REPO" && verdict "$TMP/core/subject.vr")
b=$(cd "$REPO" && verdict "$TMP/plain/subject.vr")

if [ "$SELFTEST" -eq 1 ]; then
  # The check must be able to FAIL.  Feed it two files that genuinely
  # differ; if it calls those equal, it is measuring nothing.
  printf 'module probe.pv_other;\n\npublic fn go() -> Int { undefined_name_xyz() }\n' \
    > "$TMP/plain/other.vr"
  c=$(cd "$REPO" && verdict "$TMP/plain/other.vr")
  if [ "$a" = "$c" ]; then
    printf 'selftest: FAILED — a clean subject (%s) and a broken one (%s) compare equal\n' "$a" "$c"
    exit 1
  fi
  case "$a$b$c" in
    *MUTE*) printf 'selftest: FAILED — a run printed nothing (%s/%s/%s)\n' "$a" "$b" "$c"; exit 1 ;;
  esac
  printf 'selftest: ok (subject=%s twin=%s broken=%s)\n' "$a" "$b" "$c"
fi

case "$a$b" in
  *MUTE*)
    printf 'check_path_verdict: a run printed NOTHING (core=%s plain=%s).\n' "$a" "$b"
    printf '  That is a tool failure, not a clean result — check `df -h` and the binary.\n'
    exit 2 ;;
esac

if [ "$a" != "$b" ]; then
  printf 'check_path_verdict: the SAME file got different verdicts by LOCATION.\n\n'
  printf '  in a directory named `core`: %s error(s)\n' "$a"
  printf '  beside it:                   %s error(s)\n' "$b"
  printf '\nNeither copy is part of the standard library — %s/core/ has no mod.vr.\n' "$TMP"
  printf 'Something is deciding "this is stdlib" from the path text rather than\n'
  printf 'from the standard library`s own root (T1041).\n'
  exit 1
fi

printf 'check_path_verdict: location does not change the verdict (%s error(s) both ways)\n' "$a"
exit 0
