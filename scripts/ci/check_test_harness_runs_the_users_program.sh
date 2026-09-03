#!/bin/sh
# check_test_harness_runs_the_users_program.sh — the conformance harness
# and `verum run` execute the same program.
#
# WHY THIS EXISTS.  T0732: `vtest` drives `Pipeline::run_for_test`, and
# what ships to users is `run_interpreter`.  Whenever the two sequences
# drift, a spec is green on a pipeline nobody runs — the suite stops
# being evidence, silently, and stays that way until someone runs a file
# by hand.  One drift was measured on 2026-09-03, and one CANDIDATE was
# measured and refuted — both are recorded, because the refuted one is
# what a reader would otherwise re-derive from the source and "fix":
#
#   1. GLOBAL CONSTRUCTORS.  `phase_interpret_for_test` skipped them, on
#      a comment claiming ctors are "primarily FFI library initializers
#      … that fail on macOS".  They are also `@thread_local` static
#      initializers and the CBGR allocator's LOCAL_HEAP/CURRENT_HEAP
#      bootstrap.  Four lines:
#
#          @thread_local static COUNTER: Int = 9;
#          fn main() { print(COUNTER); }
#
#          verum run   ->  9
#          harness     ->  ()      Stdout mismatch, Expected 9
#
#      `()` is `Value::default()` — the uninitialised TLS slot, read
#      because nothing ran the initializer.  A program that takes an
#      ordinary REFERENCE agrees under both paths, which is why the gap
#      survived: it needs a subject whose value comes from a ctor.
#
#   2. MACRO EXPANSION — LOOKED LIKE A DRIFT, IS NOT ONE.  `run_check_only`
#      and `run_for_test` register meta declarations and expand macros;
#      `run_interpreter` does not, which reads like the same class.  It
#      is not.  Measured with a per-run nonce so the content-keyed VBC
#      cache could not answer:
#
#          meta fn six() -> Int { 1 + 2 + 3 }
#          fn main() { print(@six()); }
#
#          verum check   warns E0410, no error
#          verum run     prints `nil`, warns E0410
#          the harness   prints `nil`, FAILED
#
#      All three AGREE, and adding the two phases to `run_interpreter`
#      changed nothing.  The first reading came from counting ERRORS
#      under `check` (0) against RAW OUTPUT under `run` (a warning) —
#      two different units, which is how a non-divergence reads as one.
#      `@meta_fn()` evaluating to `nil`, and the parser refusing every
#      user-declared `meta` against a static KNOWN_META_FUNCTIONS list,
#      are real defects of the meta system, not of harness parity.
#
# WHAT IT ASSERTS, and why each arm is load-bearing:
#   ctor_run   a `@thread_local static` reads its initialiser under
#              `verum run`
#   ctor_test  …and under the harness, when a vtest binary is present
#   control    a program with no static prints the same under both — so
#              a failure names the static, not the harness in general
#   macro      the two paths AGREE on a user macro.  Both counts are
#              non-zero today; what this watches is them parting, which
#              is what one path expanding macros and the other not would
#              produce.  Asserting zero here would gate a defect this
#              file does not own.
#   sites      the source property, checked always: all three execution
#              entry points call `run_global_ctors`.  This arm is what
#              keeps the gate from going quiet where no vtest binary was
#              built — a skip would read as "checked" when nothing was.
#
# Usage:
#   check_test_harness_runs_the_users_program.sh [verum] [vtest]
#   check_test_harness_runs_the_users_program.sh --selftest [verum] [vtest]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_harness_parity: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# EVERY PROBE CARRIES A NONCE, and it is not decoration.
#
# `verum run` caches compiled VBC keyed on the program's CONTENT, not on
# its path, and answers from that cache WITHOUT running the pipeline.
# Measured 2026-09-03 on one file in a fresh temp directory:
#
#     first run   cached VBC: no    unknown meta-function: 1
#     second run  cached VBC: yes   unknown meta-function: 0
#
# So a gate whose probe text is fixed measures the cache from its second
# CI run onward, and reports green for a compiler it never invoked. The
# nonce makes the content unique per invocation, which is the only thing
# a content-keyed cache cannot survive. (It also means this gate observes
# the diagnostic-suppression above: a repair that keeps the pipeline but
# drops its diagnostics on a cache hit is a separate defect, not one this
# gate can see.)
NONCE=$$_$(date +%s 2>/dev/null || echo 0)

spec() { # $1 = name, $2 = expected stdout, $3 = body
  {
    printf '// @test: run\n// @tier: 0\n// @level: L0\n// @expected-stdout: %s\n' "$2"
    printf '// nonce %s — see the note above: a fixed probe measures the VBC cache.\n' "$NONCE"
    printf '%s\n' "$3"
  } > "$TMP/$1.vr"
}

spec statik 9 '@thread_local static COUNTER: Int = 9;

fn main() {
    print(COUNTER);
}'

spec plain 9 'fn main() {
    let counter: Int = 9;
    print(counter);
}'

spec macro 4 'meta ident(x: tt) {
}

fn main() {
    let a = @ident(4);
    print(4);
}'

if cmp -s "$TMP/statik.vr" "$TMP/plain.vr"; then
  printf 'check_harness_parity: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

# ---------------------------------------------------------------- ctors
# The source property. Checked FIRST and unconditionally: it is the only
# arm that cannot be silenced by a missing binary, and the drift it
# names is the one that produced `()` above.
CTOR_SITES=$(grep -c 'run_global_ctors()' \
  "$REPO/crates/verum_compiler/src/pipeline/interpreter.rs" \
  "$REPO/crates/verum_compiler/src/pipeline/dispatch.rs" 2>/dev/null |
  awk -F: '{s+=$2} END{print s+0}')

# ------------------------------------------------------------ behaviour
run_out() { # $1 file -> last non-status line of `verum run`
  (cd "$REPO" && timeout 300 "$VERUM" run "$1" 2>&1) |
    grep -vE '^\s*(Compiling|Checking|Parsing|Finished|Running|Building)' |
    grep -vE '^(warning|error|note|help|  *[0-9]* *│|  *-->|   *│)' |
    tail -1
}

statik_run=$(run_out "$TMP/statik.vr")
plain_run=$(run_out "$TMP/plain.vr")
macro_run=$( (cd "$REPO" && timeout 300 "$VERUM" run "$TMP/macro.vr" 2>&1) |
  grep -c 'unknown meta-function' )
macro_check=$( (cd "$REPO" && timeout 300 "$VERUM" check "$TMP/macro.vr" 2>&1) |
  grep -c 'unknown meta-function' )

# The harness leg, when a vtest binary exists. Its absence leaves the
# arms above, which are checked either way — this is not a skip.
VTEST="${2:-}"
if [ -z "$VTEST" ] || [ ! -x "$VTEST" ]; then
  VTEST=""
  for cand in "$REPO/target/release/vtest" "$REPO/vcs/runner/vtest/target/release/vtest"; do
    [ -x "$cand" ] && VTEST="$cand" && break
  done
fi
harness_verdict="(no vtest binary)"
harness_plain="(no vtest binary)"
if [ -n "$VTEST" ]; then
  harness_verdict=$( (cd "$REPO" && timeout 400 "$VTEST" run "$TMP/statik.vr" 2>&1) |
    grep -oE 'RESULT: [A-Z]+' | tail -1 | sed 's/RESULT: //')
  harness_plain=$( (cd "$REPO" && timeout 400 "$VTEST" run "$TMP/plain.vr" 2>&1) |
    grep -oE 'RESULT: [A-Z]+' | tail -1 | sed 's/RESULT: //')
fi

if [ "$SELFTEST" -eq 1 ]; then
  printf 'fn main() {\n    print(no_such_name_xyz());\n}\n' > "$TMP/broken.vr"
  brk=$( (cd "$REPO" && timeout 200 "$VERUM" run "$TMP/broken.vr" 2>&1) | grep -c 'error' )
  if [ "$brk" -eq 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file ran without an error\n'
    exit 1
  fi
  printf 'selftest: ok — statik_run=%s plain_run=%s macro run/check=%s/%s sites=%s harness=%s harness_plain=%s broken=%s\n' \
    "$statik_run" "$plain_run" "$macro_run" "$macro_check" "$CTOR_SITES" \
    "$harness_verdict" "$harness_plain" "$brk"
fi

if [ "$CTOR_SITES" -lt 3 ]; then
  printf 'check_harness_parity: FAILED — only %s execution entry point(s) call run_global_ctors.\n' \
    "$CTOR_SITES"
  printf '  There are three: phase_interpret (the `verum run` path),\n'
  printf '  run_compiled_vbc (the cached-VBC path) and phase_interpret_for_test\n'
  printf '  (what vtest drives). A path that skips them runs `main` before the\n'
  printf '  static initialisers have run, and a `@thread_local static` reads as\n'
  printf '  `()` — `Value::default()` out of an untouched slot. See T0732.\n'
  exit 1
fi

# The control comes first: if the static-free program is wrong, the
# subject below would fail for a reason that has nothing to do with ctors.
if [ "$plain_run" != "9" ]; then
  printf 'check_harness_parity: FAILED — the CONTROL printed `%s`, expected `9`.\n' "$plain_run"
  printf '  `let counter: Int = 9; print(counter)` involves no static. If this is\n'
  printf '  wrong, the subject below proves nothing about initialisers.\n'
  exit 1
fi

if [ "$statik_run" != "9" ]; then
  printf 'check_harness_parity: FAILED — `verum run` printed `%s` for a static, expected `9`.\n' \
    "$statik_run"
  printf '  `@thread_local static COUNTER: Int = 9` must be initialised before\n'
  printf '  `main` reads it. `()` here is `Value::default()` from a slot no\n'
  printf '  constructor wrote.\n'
  exit 1
fi

if [ "$macro_run" != "$macro_check" ]; then
  printf 'check_harness_parity: FAILED — a user macro warns %s time(s) under `verum run` and %s under `verum check`.\n' \
    "$macro_run" "$macro_check"
  printf '  The counts must MATCH. They are both non-zero today, because the\n'
  printf '  parser refuses every user-declared `meta` against a static\n'
  printf '  KNOWN_META_FUNCTIONS list — a real defect, but not this one. What\n'
  printf '  this arm watches is the two paths DIVERGING on it, which is what\n'
  printf '  would happen if one of them started expanding macros and the other\n'
  printf '  did not.\n'
  exit 1
fi

if [ -n "$VTEST" ]; then
  if [ "$harness_plain" != "PASSED" ]; then
    printf 'check_harness_parity: FAILED — the harness could not run the CONTROL (%s).\n' \
      "$harness_plain"
    printf '  With the control failing, the subject below says nothing about ctors.\n'
    exit 1
  fi
  if [ "$harness_verdict" != "PASSED" ]; then
    printf 'check_harness_parity: FAILED — the harness reported %s on the file `verum run` prints 9 for.\n' \
      "$harness_verdict"
    printf '  Expected stdout 9, and the harness produced `()` when\n'
    printf '  `phase_interpret_for_test` skipped `run_global_ctors`. The suite and\n'
    printf '  the shipped path must execute the same program — that is T0732.\n'
    exit 1
  fi
fi

printf 'check_harness_parity: ok — static under run=%s harness=%s, control=%s/%s, macro run/check=%s/%s (equal), ctor entry points=%s\n' \
  "$statik_run" "$harness_verdict" "$plain_run" "$harness_plain" "$macro_run" "$macro_check" "$CTOR_SITES"
