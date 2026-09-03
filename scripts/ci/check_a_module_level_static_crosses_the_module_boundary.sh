#!/bin/sh
# check_a_module_level_static_crosses_the_module_boundary.sh — a
# `public static` is importable, and usable, from another module.
#
# WHY THIS EXISTS.  It did not, and the whole declaration form was
# inert across a module boundary.  Measured 2026-09-03 (T1088): NONE of
# the stdlib's five `public static` declarations could be mounted —
# GLOBAL_EPOCH, GLOBAL_HAZARD_DOMAIN, CAP_AUDIT_ENABLED,
# CAP_AUDIT_NEXT_SEQ, CAP_AUDIT_RING — while `EPOCH_MAX`, a `public
# const` seventy lines from the first of them in the SAME FILE,
# imported cleanly.
#
# The three mount forms disagreed, which is why it went unnoticed:
#
#     mount core.mem.epoch.{GLOBAL_EPOCH};   error<E401> cannot find
#     mount core.mem.epoch.*;                accepted…
#     let e = GLOBAL_EPOCH;                  …then error<E100> unbound
#
# The braced form validates each named item against the module surface
# and told the truth; the glob form does not validate and deferred the
# same truth to the use site.  A gate that checked only the glob form
# would have been green throughout — which is why THIS one checks both
# forms AND a use.
#
# ROOT: a `const` reaches the module surface because codegen LOWERS it
# to a zero-argument function (`register_constant_with_value`), so it
# lands in `metadata.functions`.  A `static` cannot take that route —
# the constant-function path re-executes the initialiser on every read,
# so a write from one frame is invisible to the next — and codegen gives
# it a TLS slot instead.  A slot is a codegen-time structure no wire
# format carried, so the declaration reached the archive as nothing but
# an interned string.  `CoreMetadata::statics` is the channel that was
# missing.
#
# WHAT IT ASSERTS, and why each arm is load-bearing:
#   braced    the explicit-item mount resolves
#   glob      the glob mount resolves AND the name is usable — without
#             the use, the glob arm passes on a build where nothing
#             works, since the glob form never validated anything
#   const     `EPOCH_MAX` still imports — the control that separates
#             "statics now work" from "this module stopped being
#             checked"
#   absent    a name the module does NOT declare is still refused, so
#             the fix cannot be "accept every item in a mount list"
#
# Usage:
#   check_a_module_level_static_crosses_the_module_boundary.sh [verum]
#   check_a_module_level_static_crosses_the_module_boundary.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_static_boundary: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# The VBC cache is keyed on CONTENT, so a fixed probe answers from cache
# on the second CI run and measures nothing. See
# check_test_harness_runs_the_users_program.sh for the measurement.
NONCE=$$_$(date +%s 2>/dev/null || echo 0)

probe() { # $1 = name, $2 = mount line, $3 = body line
  {
    printf '// nonce %s — a fixed probe would answer from the VBC cache\n' "$NONCE"
    printf 'module probe.static_boundary_%s;\n\n' "$1"
    printf '%s\n\n' "$2"
    printf 'public fn go() -> Int {\n%s\n}\n' "$3"
  } > "$TMP/$1.vr"
}

probe braced 'mount core.mem.epoch.{GLOBAL_EPOCH};' '    let _e = GLOBAL_EPOCH;
    0'
probe glob   'mount core.mem.epoch.*;'              '    let _e = GLOBAL_EPOCH;
    0'
probe konst  'mount core.mem.epoch.{EPOCH_MAX};'    '    let _m = EPOCH_MAX;
    0'
probe absent 'mount core.mem.epoch.{NO_SUCH_ITEM_ZZ};' '    0'

verdict() { # $1 file -> error count, or MUTE
  out=$(cd "$REPO" && timeout 300 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error<'
}

br=$(verdict "$TMP/braced.vr")
gl=$(verdict "$TMP/glob.vr")
ko=$(verdict "$TMP/konst.vr")
ab=$(verdict "$TMP/absent.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.static_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  brk=$(verdict "$TMP/broken.vr")
  if [ "$brk" = MUTE ] || [ "$brk" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$brk"
    exit 1
  fi
  printf 'selftest: ok — braced=%s glob=%s const=%s absent=%s broken=%s\n' \
    "$br" "$gl" "$ko" "$ab" "$brk"
fi

for pair in "braced:$br" "glob:$gl" "const:$ko" "absent:$ab"; do
  if [ "${pair#*:}" = MUTE ]; then
    printf 'check_static_boundary: FAILED — subject %s produced no output.\n' "${pair%%:*}"
    exit 1
  fi
done

# The control comes first. If the const stopped importing, the subjects
# below would be measuring something else entirely.
if [ "$ko" != 0 ]; then
  printf 'check_static_boundary: FAILED — the CONST control reported %s error(s).\n' "$ko"
  printf '  `mount core.mem.epoch.{EPOCH_MAX}` worked before statics did; a\n'
  printf '  change that costs it is not a fix for this.\n'
  exit 1
fi

if [ "$br" != 0 ]; then
  printf 'check_static_boundary: FAILED — the BRACED mount of a static reported %s error(s).\n' "$br"
  printf '  `mount core.mem.epoch.{GLOBAL_EPOCH}` must resolve. The braced form\n'
  printf '  validates each named item against the module surface, which is built\n'
  printf '  by `own_surface_functions` from `metadata.functions` AND\n'
  printf '  `metadata.statics`. If the sidecar was baked without the `statics`\n'
  printf '  field the map is empty — check that PRECOMPILE_SCHEMA_VERSION was\n'
  printf '  bumped, or the bake cache will serve a stale sidecar behind a green\n'
  printf '  build.\n'
  exit 1
fi

if [ "$gl" != 0 ]; then
  printf 'check_static_boundary: FAILED — the GLOB mount plus a USE reported %s error(s).\n' "$gl"
  printf '  `mount core.mem.epoch.*` never validated its items, so this arm is\n'
  printf '  about the USE: `let _e = GLOBAL_EPOCH;` must bind. Pre-fix this was\n'
  printf '  error<E100> unbound variable, with no complaint at the mount.\n'
  exit 1
fi

# Without this arm the gate passes for "accept every item in a mount list".
if [ "$ab" = 0 ]; then
  printf 'check_static_boundary: FAILED — mounting a name the module does not declare compiled clean.\n'
  printf '  `mount core.mem.epoch.{NO_SUCH_ITEM_ZZ}` must still be refused.\n'
  printf '  Publishing statics into the surface must ADD names, not stop the\n'
  printf '  surface being checked.\n'
  exit 1
fi

printf 'check_static_boundary: ok — braced mount (%s), glob mount and use (%s), const control (%s), absent item still refused (%s)\n' \
  "$br" "$gl" "$ko" "$ab"
