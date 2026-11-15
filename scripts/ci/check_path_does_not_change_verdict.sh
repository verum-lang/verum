#!/bin/sh
# A source program has the same verdict beside core/, inside core/, and
# inside an anchored core/ with mod.vr. Check positive and negative sources:
# equality alone would let a compiler that accepts everything pass.
set -eu
SELFTEST=0
if [ "${1:-}" = --selftest ]; then SELFTEST=1; shift; fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { echo "check_path_verdict: missing executable: $VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
mkdir -p "$TMP/plain" "$TMP/unanchored/core" "$TMP/anchored/core"
printf 'module core;\n' > "$TMP/anchored/core/mod.vr"

verdict() {
  # Preserve the process status: timeouts, signals and empty output cannot
  # masquerade as successful checking or as the expected type error.
  status=0
  timeout 180 "$VERUM" check "$1" > "$TMP/output" 2>&1 || status=$?
  # CliError::CompilationFailed uses exit 101.
  case "$status" in 0|101) ;; *) cat "$TMP/output" >&2; return 2 ;; esac
  grep '^error' "$TMP/output" > "$TMP/errors" || :
  if [ "$2" = valid ]; then
    if [ "$status" -ne 0 ] || [ -s "$TMP/errors" ] || ! grep -q 'Finished' "$TMP/output"; then
      cat "$TMP/output" >&2; return 1
    fi
  elif [ "$2" = invalid ]; then
    if [ "$status" -eq 0 ] || ! grep -q 'Type mismatch.*Int.*Bool' "$TMP/errors"; then
      cat "$TMP/output" >&2; return 1
    fi
  elif [ "$status" -eq 0 ]; then
    [ ! -s "$TMP/errors" ] && grep -q 'Finished' "$TMP/output" || return 1
  else
    [ -s "$TMP/errors" ] || return 1
  fi
  LC_ALL=C sort "$TMP/errors"
}

for kind in valid invalid; do
  value=7
  [ "$kind" = valid ] || value=true
  printf 'module probe.path_verdict;\npublic fn probe() -> Int { %s }\n' "$value" > "$TMP/subject.vr"
  for placement in plain unanchored/core anchored/core; do
    cp "$TMP/subject.vr" "$TMP/$placement/subject.vr"
    verdict "$TMP/$placement/subject.vr" "$kind" > "$TMP/$placement/verdict"
    cmp "$TMP/plain/verdict" "$TMP/$placement/verdict"
  done
  cp "$TMP/plain/verdict" "$TMP/$kind.verdict"
done
# Preserve the original generic-container control as well as the decisive
# return-type control. Its verdict may expose unrelated library defects;
# none may be hidden by moving it into the anchored directory.
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
for placement in plain unanchored/core anchored/core; do
  cp "$TMP/subject.vr" "$TMP/$placement/subject.vr"
  verdict "$TMP/$placement/subject.vr" same > "$TMP/$placement/verdict"
  cmp "$TMP/plain/verdict" "$TMP/$placement/verdict"
done
if [ "$SELFTEST" -eq 1 ]; then
  if cmp -s "$TMP/valid.verdict" "$TMP/invalid.verdict"; then
    echo 'selftest: positive and negative controls are indistinguishable' >&2; exit 1
  fi
  echo 'selftest: valid source accepted, invalid return rejected'
fi
echo 'check_path_verdict: identical diagnostics in all three locations'
