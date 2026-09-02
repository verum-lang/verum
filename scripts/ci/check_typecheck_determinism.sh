#!/usr/bin/env bash
# Fail if `verum check` gives a file two different verdicts.
#
# The type checker answered `core/logic/kripke.vr` with 0 errors on some runs
# and 20 on others, from one binary, on one unedited file (T0927). Nothing
# noticed for weeks: every gate in this tree runs a file ONCE, and one run
# cannot see a verdict that varies.
#
# HOW MANY RUNS. T0927 fired in roughly one run of five. A three-run probe
# catches a 20%-flipper 38% of the time and a 10%-flipper 27% — so the first
# sweep this gate's author ran reported "151 of 151 stable" and meant nothing.
# RUNS defaults to 15, which catches a 20%-flipper 96% of the time and a
# 10%-flipper 80%. Lowering it does not make the gate cheaper, it makes it
# quieter.
#
# THE SELF-TEST IS NOT OPTIONAL. A green run of this gate says "no file
# varied", which is also what a broken detector says. `--selftest` feeds the
# same comparison a source that is KNOWN to vary and fails if the gate calls
# it stable. CI runs it before the sweep; that is the whole reason the green
# is worth reading.
#
# Usage:
#   scripts/ci/check_typecheck_determinism.sh [--runs N] [--sample FILE] [--bin PATH]
#   scripts/ci/check_typecheck_determinism.sh --selftest
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNS="${RUNS:-15}"
SAMPLE_LIST="$REPO_ROOT/scripts/ci/typecheck_determinism_sample.txt"
VERUM_BIN="${VERUM_CLI:-}"
SELFTEST=0

while [ $# -gt 0 ]; do
  case "$1" in
    --runs)     RUNS="$2"; shift 2 ;;
    --sample)   SAMPLE_LIST="$2"; shift 2 ;;
    --bin)      VERUM_BIN="$2"; shift 2 ;;
    --selftest) SELFTEST=1; shift ;;
    -h|--help)  sed -n '2,28p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# Count the error lines a command produced. One place, so the sweep and the
# self-test are measured by the SAME code — a self-test that exercises a
# different comparison proves nothing about the sweep.
verdict_of() { "$@" 2>&1 | grep -c '^error'; }

# A subject inside `core/` must be measured under the SAME rules as user
# code, or the sweep measures the lenient answer and calls it stable.
#
# Measured 2026-09-02 on `core/net/cidr.vr`, one binary, no rebuild:
#
#     lenient:  0 0 0 0 0 0        <- stable, and silent
#     strict:   5 2 5 5 2 2        <- the verdict itself moves
#
# `IpAddr.contains` resolves on some runs and not others; leniently an
# unmatched method answers `Type::Unknown` instead of an error (T1044),
# so the variation cannot surface at all.  Without this the gate reports
# a file as deterministic precisely when it cannot see the defect —
# green because it is blind, not because the property holds.
#
# `VERUM_STRICT_STDLIB` was added the same day (2d37f3721); before it the
# question could not be asked.  Reproduced independently by a second
# session on its own tree and binary.
verum_check_of() { # $1 = binary, $2 = repo-relative .vr path
  case "$2" in
    core/*) VERUM_STRICT_STDLIB=1 verdict_of "$1" check "$2" ;;
    *)      verdict_of "$1" check "$2" ;;
  esac
}

# Run `verdict_of` RUNS times over one subject; echo "STABLE <v>" or
# "VARY <v1>/<v2>/...".
#
# The first argument names the MEASURING function (`verdict_of` for the
# self-test's raw command, `verum_check_of` for a sweep subject that may
# need the stdlib-strict environment).  It used to call `verdict_of`
# unconditionally, so passing a wrapper made the wrapper's NAME the
# command — bash ran `verum_check_of <bin> <file>` through `verdict_of`,
# which found no such executable, counted zero `error` lines, and
# reported every subject as a stable 0.  Caught because `core/net/cidr.vr`
# — measured by hand as varying 5/2 — came back "ok (0/0/0/…)".
verdicts_for() {
  local measure="$1"; shift
  local seen="" all=""
  local i v
  for ((i = 0; i < RUNS; i++)); do
    v="$("$measure" "$@")"
    all="$all$v/"
    case "/$seen/" in
      */"$v"/*) ;;
      *) seen="$seen$v/" ;;
    esac
  done
  # `seen` holds one entry per DISTINCT verdict.
  if [ "$(printf '%s' "$seen" | tr -cd '/' | wc -c)" -eq 1 ]; then
    echo "STABLE ${all%/}"
  else
    echo "VARY ${all%/}"
  fi
}

if [ "$SELFTEST" = "1" ]; then
  echo "== self-test: does this gate notice a verdict that varies?"
  # A subject that prints one `error` line about half the time. If the sweep
  # below can be trusted, this must come back VARY.
  probe="$(mktemp)"
  cat >"$probe" <<'PROBE'
#!/usr/bin/env bash
if [ $(( RANDOM % 2 )) -eq 0 ]; then echo "error: synthetic"; fi
exit 0
PROBE
  chmod +x "$probe"
  # 15 coin flips call it stable with probability 2 * 0.5^15 ~= 6e-5.
  result="$(verdicts_for verdict_of "$probe")"
  rm -f "$probe"
  echo "   $result"
  case "$result" in
    VARY*) echo "== self-test PASSED: the gate can see a varying verdict"; exit 0 ;;
    *) echo "== self-test FAILED: the gate called a varying subject stable." >&2
       echo "   Do NOT trust a green sweep from this build." >&2
       exit 1 ;;
  esac
fi

if [ -z "$VERUM_BIN" ]; then
  VERUM_BIN="$REPO_ROOT/target/release/verum"
fi
if [ ! -x "$VERUM_BIN" ]; then
  echo "no verum binary at $VERUM_BIN (pass --bin PATH or set VERUM_CLI)" >&2
  exit 2
fi
if [ ! -f "$SAMPLE_LIST" ]; then
  echo "no sample list at $SAMPLE_LIST" >&2
  exit 2
fi

echo "== typecheck determinism: $RUNS runs per file"
echo "   binary: $VERUM_BIN"
echo "   sample: $SAMPLE_LIST"

varied=0
checked=0
while IFS= read -r rel; do
  case "$rel" in ''|\#*) continue ;; esac
  [ -f "$REPO_ROOT/$rel" ] || { echo "   SKIP (gone) $rel"; continue; }
  checked=$((checked + 1))
  line="$(cd "$REPO_ROOT" && verdicts_for verum_check_of "$VERUM_BIN" "$rel")"
  case "$line" in
    VARY*) varied=$((varied + 1)); echo "   VARY   $rel  ${line#VARY }" ;;
    # Named as they pass, not only when they fail: this sweep is minutes long
    # and a silent one is indistinguishable from a hung one — which cost this
    # session a diagnosis earlier today.
    *) echo "   ok     $rel  (${line#STABLE })" ;;
  esac
done <"$SAMPLE_LIST"

echo "== $checked files, $varied with a varying verdict"
if [ "$varied" -gt 0 ]; then
  echo "FAIL: the same binary gave the same file more than one verdict." >&2
  echo "      A gate keyed on an error count cannot be trusted while this holds." >&2
  exit 1
fi
echo "OK"
