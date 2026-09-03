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
    "phase_verify": ("run_check_only", "run_interpreter"),
    # INTENDED: `run_for_test` reaches phase_verify through validate_module.
    "phase_safety_gate": ("run_check_only", "run_for_test", "run_interpreter"),
    # INTENDED: all three now run it. Was DEFECT T1101 — `verum check`
    # skipped the safety gate, inert on the default permissive [safety]
    # configuration and a silent hole under a restrictive one. Closed
    # 2026-09-03; `run_check_only` takes it in `run_interpreter`'s order,
    # before type checking, so the diagnostic arrives at the same point in
    # both commands.
    "phase_cbgr_analysis": ("run_check_only", "run_for_test", "run_interpreter"),
    # INTENDED: all three now run it. Was DEFECT T1101 — `verum check`
    # skipped CBGR analysis outright, so a borrow the shipped path refuses
    # passed `check` clean. Closed 2026-09-03; it runs last, after
    # dependency analysis, for the same reason it runs last in
    # `run_interpreter` — it reads the types the checker has just settled.
    "phase_interpret_with_args": ("run_interpreter",),
    # INTENDED: only the run path executes.
    "phase_interpret_for_test": ("run_for_test",),
    # INTENDED: the harness variant, with output capture.
    "phase_meta_evaluation": ("run_check_only",),
    # INTENDED: meta evaluation is a check-time service.
    "phase_context_validation": ("run_check_only",),
    # DEFECT T1095, closed 2026-09-03 by 9c0d94e12 — and the row still
    # reads as one caller, which needs both halves said or the next
    # reader draws the wrong conclusion from it.
    #
    # WHY ONE NAME IS CORRECT HERE: the other two entry points reach
    # context validation INDIRECTLY — `run_interpreter` through
    # `phase_interpret`, `run_for_test` through `validate_module` — and
    # this gate reads direct `self.phase_*(` calls out of a body. Every
    # row means "who calls it here", never "who runs it". Read the other
    # way, this row looks like a two-thirds gap that does not exist.
    #
    # WHY THE ROW IS STILL MARKED DEFECT: single-file `verum check
    # <file>` ran no context validation at all, so `using [!Ctx]` — a
    # documented compile-time guarantee — was enforced by `verum build`
    # and ignored by `verum check`. `check_project` had been fixed first
    # and that was not enough: the single-file path is a THIRD entry
    # point, and "fixed the project path" read as "fixed check".
    #
    # The measurement that made it visible is the one to copy: a
    # 120-file corpus scored "0 violations" BEFORE the fix, and the
    # known-bad control INSIDE the same run also scored 0 — the clean
    # result came from not checking. After: 120 files 0, control 1.
    # Same number, opposite meaning.
    #
    # This row is also the first live demonstration that this gate
    # works: it went red on main the moment that one line landed.
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
    # A DUPLICATE KEY IN `EXPECTED` IS CHECKED ELSEWHERE, ON PURPOSE.
    #
    # This file briefly carried its own duplicate-key check, written after
    # the table lost half a commit to one: two sessions each recorded
    # `phase_context_validation` within one window, and Python keeps the
    # LAST key silently, so the earlier (better) annotation became dead
    # text that still read as authoritative.
    #
    # `scripts/ci/check_gate_tables_have_no_duplicate_keys.py` now does it
    # for EVERY dict and set in EVERY gate script — by parsing them with
    # `ast`, not by grepping, because what makes something a duplicate is
    # the parser. It runs in CI through `make gates-source`. The local
    # version matched only `"phase_*":` rows, so it was both redundant and
    # strictly weaker: a duplicate of any other key here would have passed
    # it and been caught by the general one anyway.
    #
    # Two checks for one property is how the weaker of them ends up giving
    # false comfort. Kept here as the reason the general gate exists.
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
    # A green here means "nothing MOVED", never "the entry points agree",
    # and the two read alike unless the difference is printed. The gate's
    # own name says `agree`, so a summary that stops at "unchanged" hands
    # the reader the stronger claim for free — the shape catalogued as a
    # gate legitimising the drift it does not check. Name the gaps in the
    # success line so the reader can never mistake a ratchet for parity.
    total = len(ENTRY_POINTS)
    partial = sorted(k for k, v in actual.items() if len(v) < total)
    print(f"check_phase_parity: ok — {n} phases, mapping unchanged across "
          f"{total} entry points")
    if partial:
        verdicts = _row_verdicts()
        defects = [k for k in partial if verdicts.get(k, "").startswith("DEFECT")]
        print(f"  NOT parity: {len(partial)} of {n} phases run by some entry "
              f"points and not others.")
        for k in partial:
            missing = [e for e, _ in ENTRY_POINTS if e not in actual[k]]
            print(f"    [{verdicts.get(k, 'UNANNOTATED')}] {k}: "
                  f"missing from {', '.join(missing)}")
        if defects:
            print(f"  {len(defects)} of those are marked DEFECT and are OPEN "
                  f"work, not design.")
    return 0


def _row_verdicts():
    """Read this file's own EXPECTED annotations: phase -> INTENDED / DEFECT <T>.

    A bare list of asymmetries cries wolf — most of them are by design, and a
    reader who cannot tell design from debt learns to skip the whole block.
    The verdicts are already written beside the rows; parse them rather than
    make the reader open the file to find out which lines matter.
    """
    import re
    src = Path(__file__).read_text(encoding="utf-8").splitlines()
    verdicts, current = {}, None
    for line in src:
        m = re.match(r'\s*"(phase_[a-z_]+)":', line)
        if m:
            current = m.group(1)
            continue
        if current is None:
            continue
        s = line.strip()
        if not s.startswith("#"):
            if s.startswith("}"):
                break
            continue
        m = re.match(r'#\s*(INTENDED|DEFECT\s+\S+?)[:,]', s)
        if m and current not in verdicts:
            verdicts[current] = m.group(1).replace("  ", " ")
    return verdicts


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
