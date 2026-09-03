#!/usr/bin/env bash
# SUMMARY-LINE-ONE-SPELLING-1 (T1073): the compilation-failure summary
# must have exactly ONE producer.
#
# Measured 2026-09-03: the same fact was spelled four ways.
#
#     verum_diagnostics/emitter.rs   "compilation failed with N error"/"errors"
#     verum_compiler/session.rs      "compilation failed with N error(s)"
#     verum_cli/commands/check.rs    "N type errors found"      (project mode)
#     the same file's CliError       "N type errors"
#
# `verum check <file>` printed the first; `verum check` on a PROJECT
# printed the third. Every corpus counter in this tree keys on the
# canonical wording — `check_hkt_bound_is_checked.sh`,
# `check_group_mount_item_is_not_a_module.sh`, and the ad-hoc sweeps
# both sessions ran today — so all of them read ZERO on a project-mode
# failure. A zero meaning "the instrument did not answer" is
# indistinguishable from a zero meaning "no errors", which is the
# failure this repo has paid for more than once.
#
# The gate is structural, not behavioural: it counts PRODUCERS, so it
# cannot be fooled by a run that happens not to fail.

# `set -e` matters here: this gate first printed [ok] while `mapfile`
# (bash 4, absent from macOS's bash 3.2) failed on every line — a green
# verdict from a script that never ran its own check. Errors abort.
set -euo pipefail
cd "$(dirname "$0")/../.."

# The one producer, by definition.
HELPER='crates/verum_diagnostics/src/emitter.rs'

# Any other site that formats the wording itself.
offenders=$(grep -rn 'compilation failed with' crates/ --include='*.rs' 2>/dev/null \
  | grep -v "^${HELPER}:" || true)

# The project path must not reintroduce its own wording.
typeerr=$(grep -rnE '"\{\} type error' crates/verum_cli/src --include='*.rs' 2>/dev/null || true)

fail=0
if [ -n "$offenders" ]; then
  echo "GATE FAIL: summary-line-one-spelling: a site formats the summary wording"
  echo "  instead of calling verum_diagnostics::compilation_failure_summary():"
  echo "$offenders" | sed 's/^/    /'
  fail=1
fi
if [ -n "$typeerr" ]; then
  echo "GATE FAIL: summary-line-one-spelling: the project path prints its own"
  echo "  '{} type error...' summary; corpus counters key on the canonical line:"
  echo "$typeerr" | sed 's/^/    /'
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "  A summary line is only useful if there is exactly one of it."
  exit 1
fi

echo "[ok] summary-line: one producer (verum_diagnostics::compilation_failure_summary)"
