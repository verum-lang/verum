#!/bin/sh
# check_group_mount_item_is_not_a_module.sh — a name a group mount asks
# for is an ITEM of that module, and a type is not a submodule just
# because it has methods.
#
# WHY THIS EXISTS.  `mount core.{Result};` was refused with E402
# "module `core.Result` not found" while `mount core.{Maybe};` was
# clean — same form, same prefix, same arity.  Measured 2026-09-02
# (T1077).
#
# The group-mount site (infer/modules.rs, MountTreeKind::Nested) asks
# first whether `<prefix>.<Item>` is ITSELF a module, and glob-imports it
# when the answer is yes:
#
#     let sub_path = format!("{}.{}", module_path, item_name);
#     if let Some(m) = self.mount_module_target(&sub_path, registry) {
#         self.import_all_from_module(&m, registry)?;   // refusal
#         continue;                                     // or silence
#     }
#
# `mount_module_target`'s last authority is a RANGE QUERY: "does the
# metadata index hold any key with the prefix `<n>.`".  Method keys are
# spelled `Result.map`, `Maybe.unwrap`, `Map.insert` — so every type with
# methods answers yes, and `mount core.{Result}` resolved as though the
# author had written `mount core.Result.*`.  Two namespaces (module
# paths, `Type.method`) share one key space, and the coarser one wins.
#
# Measured over the archive: Result 218 method keys, Maybe 370, Map 237,
# Ordering 24 — all diverted; Add / Clone / Iterator have 0 and were
# never diverted.  Perfect separation on the count, which is what named
# the mechanism.  `Maybe` survived only because a later gate happened to
# find "known items" for `core.Maybe` — the same false path, not a
# different one.
#
# THE THREE SUBJECTS.  A gate keyed only on "Result is clean" would pass
# for a repair that stops resolving group mounts at all, and one keyed
# only on "nonexistent is refused" would pass for a repair that changes
# nothing.  Both sides are therefore checked, plus the real-submodule
# case the diverted path exists to serve.
#
# Usage:
#   check_group_mount_item_is_not_a_module.sh [verum]
#   check_group_mount_item_is_not_a_module.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_group_mount: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# `Result` is a TYPE with methods: the item form must bind the type.
cat > "$TMP/type_item.vr" <<'VR'
module probe.group_mount_type_item;
mount core.{Result};

public fn p() -> Int { 0 }
VR

# `collections` is a REAL submodule: the "is it a module" path exists for
# this case and must keep working.
cat > "$TMP/real_submodule.vr" <<'VR'
module probe.group_mount_real_submodule;
mount core.{collections};

public fn p() -> Int { 0 }
VR

# Nothing of this name exists in any form: the mount must still be
# refused, and refused as a missing ITEM (E401), not a missing module.
cat > "$TMP/absent.vr" <<'VR'
module probe.group_mount_absent;
mount core.{ZzzNotAThingAnywhere};

public fn p() -> Int { 0 }
VR

verdict() { # $1 file -> error count, or MUTE when the binary said nothing
  out=$(cd "$REPO" && timeout 300 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  n=$(printf '%s' "$out" | grep -oE 'compilation failed with [0-9]+ error' |
        grep -oE '[0-9]+' | tail -1)
  [ -n "$n" ] || n=0
  echo "$n"
}
codes() { # $1 file -> the distinct E4xx codes it raised
  (cd "$REPO" && timeout 300 "$VERUM" check "$1" 2>&1) |
    grep -oE 'E4[0-9][0-9]' | sort -u | tr '\n' ' '
}

t=$(verdict "$TMP/type_item.vr")
s=$(verdict "$TMP/real_submodule.vr")
a=$(verdict "$TMP/absent.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.group_mount_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  b=$(verdict "$TMP/broken.vr")
  if [ "$b" = MUTE ] || [ "$b" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$b"
    exit 1
  fi
  printf 'selftest: ok — type=%s submodule=%s absent=%s broken=%s\n' "$t" "$s" "$a" "$b"
fi

for pair in "type_item:$t" "real_submodule:$s" "absent:$a"; do
  if [ "${pair#*:}" = MUTE ]; then
    printf 'check_group_mount: FAILED — subject %s produced no output.\n' "${pair%%:*}"
    exit 1
  fi
done

if [ "$t" != 0 ]; then
  printf 'check_group_mount: FAILED — `mount core.{Result};` reported %s error(s).\n' "$t"
  printf '  Codes: %s\n' "$(codes "$TMP/type_item.vr")"
  printf '  `Result` is a TYPE. The group-mount site asked whether `core.Result`\n'
  printf '  is a module, and `mount_module_target` answered yes because the\n'
  printf '  metadata index holds keys with the prefix `Result.` — its METHODS.\n'
  printf '  See MOUNT-GROUP-ITEM-IS-NOT-A-MODULE in infer/modules.rs.\n'
  exit 1
fi

if [ "$s" != 0 ]; then
  printf 'check_group_mount: FAILED — `mount core.{collections};` reported %s error(s).\n' "$s"
  printf '  Codes: %s\n' "$(codes "$TMP/real_submodule.vr")"
  printf '  This is the CONTROL for the opposite side: a group mount naming a\n'
  printf '  real submodule must still glob-import it. A repair that refuses the\n'
  printf '  module path outright trades one wrong verdict for another.\n'
  exit 1
fi

# Without this arm the gate is vacuous: deleting the resolution check
# entirely would satisfy both tests above.
if [ "$a" = 0 ]; then
  printf 'check_group_mount: FAILED — `mount core.{ZzzNotAThingAnywhere};` compiled clean.\n'
  printf '  A name that exists in no form must still be refused. Passing it means\n'
  printf '  the gate above proves nothing: silence is not resolution.\n'
  exit 1
fi

case " $(codes "$TMP/absent.vr") " in
  *' E401 '*) ;;
  *)
    printf 'check_group_mount: FAILED — absent name refused, but not as a missing ITEM.\n'
    printf '  Codes: %s (expected E401).\n' "$(codes "$TMP/absent.vr")"
    printf '  E402 here would mean the compiler is still reading `core.<Item>` as a\n'
    printf '  module path — the very confusion this gate exists to pin down. The\n'
    printf '  author would be told to fix a module name they never wrote.\n'
    exit 1
    ;;
esac

printf 'check_group_mount: ok — type item clean (%s), real submodule clean (%s), absent name refused as E401 (%s)\n' \
  "$t" "$s" "$a"
