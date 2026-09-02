#!/bin/sh
# check_strict_stdlib_flag_reaches_core.sh — `VERUM_STRICT_STDLIB=1` must
# make a file inside the stdlib anchor verdict-identical to the same
# bytes outside it.
#
# WHY THIS EXISTS.  `verum check` picks a lenient mode from the PATH
# alone: an ancestor directory named `core` that holds `mod.vr`.  Until
# 2026-09-02 there was no switch, so "what does `core/` look like under
# user rules" could not be asked at all.  Two defects were living in
# that gap — a mounted enum's inherent `implement` silently not loading
# (T0755), and a receiver-first `Display.fmt(self, f)` call form the
# language does not have, in 33 files, which the mode accepted because
# it does not count arguments (T1059; chasing it down surfaced
# transposed arguments in RSA-PSS signature verification).
#
# WHAT IT ASSERTS.  Three placements of ONE subject:
#
#     outside the anchor                      strict   -> N errors
#     inside  the anchor                      lenient  -> may differ
#     inside  the anchor + VERUM_STRICT_STDLIB=1       -> must equal N
#
# The third is the gate.  The second is printed, not asserted: the
# mode's EFFECT is a separate open question (T1044), and a gate that
# demanded equality there would be red for a reason this flag does not
# fix.
#
# THE FIXTURE HAS TO BE ABLE TO DISCRIMINATE, and this is where its
# predecessor failed.  `check_path_does_not_change_verdict.sh` compares
# two placements too — but it creates its `core` directory WITHOUT
# `mod.vr`, so the anchor never fires and both sides land in the same
# mode; and its subject is a self-contained generic container that
# mounts no stdlib, so it would not discriminate even with the anchor
# right.  Green, and unable to be anything else.  Hence the two
# structural checks below: the anchor file is written, and the lenient
# placement is REQUIRED to differ from the strict one — if it ever
# stops differing, this fixture has gone vacuous the same way and says
# so instead of passing.
#
# Usage:
#   check_strict_stdlib_flag_reaches_core.sh [verum]
#   check_strict_stdlib_flag_reaches_core.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_strict_stdlib: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/plain" "$TMP/anchored/core"

# The anchor is a `core` directory holding `mod.vr` — without this file
# the placement is not anchored at all and the comparison is vacuous.
printf 'module core.mod;\n' > "$TMP/anchored/core/mod.vr"
[ -f "$TMP/anchored/core/mod.vr" ] || {
  printf 'check_strict_stdlib: could not write the anchor file\n' >&2
  exit 2
}

# A subject that EXERCISES the mode: a mounted stdlib enum whose
# inherent `implement` block carries the method being called. Measured
# 2026-09-02 — clean inside the anchor, E400 outside it.
write_subject() { # $1 = out file
  cat > "$1" <<'VR'
module probe.strict_stdlib;
mount core.prelude.*;
mount core.mem.capability.{Capability, Read, Write};

public fn probe() -> Int {
    let c: Capability = Read;
    let _ = c.to_bit();
    0
}
VR
}
write_subject "$TMP/plain/subject.vr"
write_subject "$TMP/anchored/core/subject.vr"

verdict() { # $1 file [$2 = strict] -> error count, or MUTE
  if [ "${2:-}" = strict ]; then
    out=$(VERUM_STRICT_STDLIB=1 timeout 180 "$VERUM" check "$1" 2>&1)
  else
    out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  fi
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}

plain=$(cd "$REPO" && verdict "$TMP/plain/subject.vr")
lenient=$(cd "$REPO" && verdict "$TMP/anchored/core/subject.vr")
strict=$(cd "$REPO" && verdict "$TMP/anchored/core/subject.vr" strict)

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.strict_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/plain/broken.vr"
  broken=$(cd "$REPO" && verdict "$TMP/plain/broken.vr")
  if [ "$broken" = MUTE ] || [ "$broken" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$broken"
    exit 1
  fi
  printf 'selftest: ok — plain=%s lenient=%s strict=%s broken=%s\n' \
    "$plain" "$lenient" "$strict" "$broken"
fi

for v in "$plain" "$lenient" "$strict"; do
  [ "$v" = MUTE ] && {
    printf 'check_strict_stdlib: FAILED — a placement produced no output (%s/%s/%s).\n' \
      "$plain" "$lenient" "$strict"
    exit 1
  }
done

if [ "$plain" = "$lenient" ]; then
  printf 'check_strict_stdlib: FAILED — the FIXTURE has gone vacuous.\n'
  printf '  Anchored (%s) and plain (%s) agree, so this subject no longer\n' "$lenient" "$plain"
  printf '  exercises the mode and the flag check below would pass for free.\n'
  printf '  Either the mode stopped applying (good — retire this gate) or the\n'
  printf '  subject stopped reaching it (bad — pick one that does).\n'
  exit 1
fi

if [ "$strict" != "$plain" ]; then
  printf 'check_strict_stdlib: FAILED — VERUM_STRICT_STDLIB did not reach the anchor.\n'
  printf '  outside the anchor            : %s error(s)\n' "$plain"
  printf '  inside, mode on               : %s error(s)\n' "$lenient"
  printf '  inside, VERUM_STRICT_STDLIB=1 : %s error(s)   (expected %s)\n' "$strict" "$plain"
  printf '  The flag is what makes core/ measurable under user rules; if it\n'
  printf '  stops working, that measurement silently reverts to the lenient\n'
  printf '  answer while still looking like a strict sweep.\n'
  exit 1
fi

printf 'check_strict_stdlib: ok — anchored+flag (%s) equals plain (%s); mode itself still differs (%s)\n' \
  "$strict" "$plain" "$lenient"
