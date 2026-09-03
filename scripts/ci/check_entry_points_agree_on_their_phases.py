#!/usr/bin/env python3
"""Every pipeline entry point runs the same phases, or says why not.

WHY THIS EXISTS.  Three separate defects in one day, all the same shape:
a phase that one entry point runs and another does not.

  * T0732 — the conformance harness (`run_for_test`) did not call
    `run_global_ctors`, so a `@thread_local static` read as `()` under
    vtest while `verum run` printed 9.  Every spec was green on a
    pipeline no user executes.
  * T1095 — `check_project` is a hand-assembled list, so the context
    validation phase added to `build` was never inherited: a forbidden
    context was enforced by `verum build` and silently ignored by
    `verum check`.
  * T1101 — `run_check_only` does not call `phase_cbgr_analysis`.
    `verum check` reports zero diagnostics for

        let r: &Int = { let v: Int = 5; &v };  print(*r);

    which `verum run` catches at runtime as a CBGR use-after-free.

None of the three was found by a test.  All three were found by the same
two-command trick: grep `self.phase_*(` out of the two entry-point bodies
and `comm` the sorted lists.  This file makes that trick a gate.

WHAT IT ASSERTS.  Not equality — the entry points legitimately differ
(`run_check_only` does not interpret; `run_for_test` captures output).
It asserts that the CURRENT mapping "phase -> which entry points call it"
is exactly the one recorded below.  A phase added to one entry point and
not the others changes the mapping and fails here, so the divergence has
to be looked at and written down rather than discovered a month later by
a user.

This is a ratchet, and a ratchet legitimises what it records.  The
recorded state below is therefore annotated: each divergence says whether
it is INTENDED or a KNOWN DEFECT with its task id.  Do not add a row
without one of those two words.

Usage:  check_entry_points_agree_on_their_phases.py [--update]
"""

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DISPATCH = REPO / "crates/verum_compiler/src/pipeline/dispatch.rs"
INTERP = REPO / "crates/verum_compiler/src/pipeline/interpreter.rs"

ENTRY_POINTS = [
    ("run_check_only", DISPATCH),
    ("run_interpreter", DISPATCH),
    ("run_for_test", INTERP),
]

# phase -> sorted tuple of entry points that call it.
#
# INTENDED   — the difference is by design, and the reason is stated.
# DEFECT <T> — the difference is a known defect; the task owns closing it.
EXPECTED = {
    "phase_load_source": ("run_check_only", "run_for_test", "run_interpreter"),
    "phase_parse": ("run_check_only", "run_for_test", "run_interpreter"),
    "phase_type_check": ("run_check_only", "run_for_test", "run_interpreter"),
    "phase_dependency_analysis": ("run_check_only", "run_for_test", "run_interpreter"),
    # DEFECT T1095, closed 2026-09-03: `verum check <file>` ran no
    # context validation, so `using [!Ctx]` — a documented compile-time
    # guarantee — was reported by `build` and not by `check`. The single
    # -file path is a THIRD entry point; `check_project` had been fixed
    # first and that was not enough. Recorded as the one-caller shape it
    # has TODAY: `run_interpreter` and `run_for_test` reach the phase
    # through `phase_context_validation`'s other call sites in
    # pipeline.rs, not from their own bodies, so this row is not yet the
    # three-way it should read as.
    "phase_context_validation": ("run_check_only",),
    "phase_verify": ("run_check_only", "run_interpreter"),
    # INTENDED: `run_for_test` reaches phase_verify through validate_module.
    "phase_safety_gate": ("run_for_test", "run_interpreter"),
    # DEFECT T1101: `verum check` skips the safety gate. Inert on the default
    # permissive [safety] configuration, observable under a restrictive one.
    "phase_cbgr_analysis": ("run_for_test", "run_interpreter"),
    # DEFECT T1101: `verum check` skips CBGR analysis outright.
    "phase_interpret_with_args": ("run_interpreter",),
    # INTENDED: only the run path executes.
    "phase_interpret_for_test": ("run_for_test",),
    # INTENDED: the harness variant, with output capture.
    "phase_meta_evaluation": ("run_check_only",),
    # INTENDED: meta evaluation is a check-time service.
    "phase_context_validation": ("run_check_only",),
    # INTENDED that only ONE name appears, and it is worth reading twice:
    # the other two entry points reach context validation INDIRECTLY —
    # `run_interpreter` through `phase_interpret`, `run_for_test` through
    # `validate_module` — and this gate sees direct `self.phase_*(` calls
    # only. The row therefore records "who calls it in its own body", not
    # "who runs it".
    #
    # The direct call here is T1095's fix (9c0d94e12): single-file
    # `verum check <file>` ran neither, so a forbidden context was
    # enforced by `verum build` and ignored by `verum check`. The measure
    # that made it visible is worth copying: a 120-file corpus scored
    # "0 violations" BEFORE the fix, and the known-bad control inside the
    # same run also scored 0 — the clean result came from not checking.
    # After: 120 files 0, control 1. Same number, opposite meaning.
    #
    # This row is also the first live demonstration that the gate works:
    # it went red on main the moment that one line landed.
    "phase_ats_v": (),
    # INTENDED: reached through validate_module, never called directly here.
}

CALL = re.compile(r"self\.(phase_[a-z_0-9]+)\(")
FNDEF = re.compile(r"^    (?:pub(?:\([a-z()]+\))? )?(?:async )?fn (\w+)")


def body(path: Path, name: str):
    lines = path.read_text().splitlines()
    start = None
    for i, line in enumerate(lines):
        m = FNDEF.match(line)
        if m and m.group(1) == name:
            start = i
            break
    if start is None:
        return None
    for j in range(start + 1, len(lines)):
        if FNDEF.match(lines[j]):
            return lines[start:j]
    return lines[start:]


def main(argv):
    update = "--update" in argv
    actual = {}
    for name, path in ENTRY_POINTS:
        b = body(path, name)
        if b is None:
            print(f"check_phase_parity: FAILED — entry point `{name}` not found in {path.name}.")
            print("  The gate cannot compare what it cannot locate; if the function was")
            print("  renamed, rename it here too rather than deleting the row.")
            return 1
        for phase in set(CALL.findall("\n".join(b))):
            actual.setdefault(phase, set()).add(name)

    actual = {k: tuple(sorted(v)) for k, v in actual.items()}
    for k in EXPECTED:
        actual.setdefault(k, ())

    if update:
        for phase in sorted(actual):
            print(f'    "{phase}": {actual[phase]!r},')
        return 0

    bad = []
    for phase in sorted(set(actual) | set(EXPECTED)):
        want = EXPECTED.get(phase)
        got = actual.get(phase, ())
        if want is None:
            bad.append(f"  NEW phase `{phase}` called by {got} — not recorded here.")
        elif want != got:
            bad.append(f"  `{phase}`: recorded {want}, actual {got}.")

    if bad:
        print("check_phase_parity: FAILED — the entry points' phase mapping changed.")
        for line in bad:
            print(line)
        print()
        print("  A phase that one entry point runs and another does not is how")
        print("  T0732, T1095 and T1101 each happened: the suite, `verum check`")
        print("  and `verum run` executed different programs and every one of them")
        print("  reported success. Update EXPECTED in this file, and write INTENDED")
        print("  with a reason or DEFECT with a task id beside the row — a bare")
        print("  update turns this gate into a record of drift rather than a check")
        print("  on it. `--update` prints the current mapping to paste.")
        return 1

    n = len(actual)
    print(f"check_phase_parity: ok — {n} phases, mapping unchanged across "
          f"{len(ENTRY_POINTS)} entry points")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
