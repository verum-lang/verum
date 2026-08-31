#!/usr/bin/env python3
"""Gate: an input the compiler does not understand must not be counted
in favour of the thing being checked.

Four defects found on 2026-08-31 share one shape, and the shape is worse
than any of them alone: a misspelled, unknown or unmatched input makes
the tool report SUCCESS — not silence, success.

    @verfiy(thorough)          typo in the NAME     -> "2 proved, 0 failed"
    @verify(thorugh)           typo in the ARGUMENT -> "2 proved, 0 failed"
    proof no_such_axiom by ..  the axiom is ABSENT  -> "1 proved"
    decreases n  with n + 1    the measure GROWS    -> "2 proved"
    Clock.now() with no using  context UNDECLARED   -> returned a value

The correct spelling of the first case reports `1 proved, 1 FAILED`, so
the machinery works and one character switches it off.

This gate does not test four bugs. It tests the PROPERTY, so it also
catches the fifth case nobody has found yet: every row below feeds the
compiler something it cannot honour, and requires the compiler to SAY
SO — a diagnostic, or a non-zero exit. A row that passes silently is a
defect regardless of which subsystem swallowed it.

RATCHET, not a pass/fail wall. Every row is red today; each is an open
task. The gate fails when a GREEN row goes red (a regression) or when a
RED row turns green without the baseline being updated (an unrecorded
fix — update BASELINE and cite the task). That way the gate lands now,
before any of the four repairs, and each repair flips exactly one row.

    scripts/ci/check_silent_acceptance.py [path-to-verum]
"""
import subprocess
import sys
import tempfile
from pathlib import Path

# name -> (task, source, what a healthy compiler must do)
CASES = [
    # POSITIVE CONTROL, deliberately first and deliberately NOT in the
    # baseline. The same loop as the two typo rows, spelled correctly.
    # It must always be diagnosed; if it ever goes silent, this gate has
    # stopped being able to tell the difference and every "SILENTLY
    # ACCEPTED" line below it is meaningless. A gate that cannot fail on
    # a known-bad input is not evidence, and six red rows would then be
    # six copies of one broken instrument.
    (
        "positive-control",
        "-",
        """@verify(thorough)
pure fn spin(n: Int) -> Int requires n >= 0 {
    let mut i = n;
    let mut acc = 0;
    while i > 0 { acc = acc + 1; }
    acc
}
public fn main() { print(spin(1)); }
""",
        "always be diagnosed — the loop cannot terminate",
    ),
    (
        "unknown-attribute",
        "T1025",
        """@not_a_real_attribute_xyz
pure fn f(n: Int) -> Int { n }
public fn main() { print(f(1)); }
""",
        "reject or warn about an attribute it does not know",
    ),
    (
        "attribute-name-typo",
        "T1025",
        """@verfiy(thorough)
pure fn spin(n: Int) -> Int requires n >= 0 {
    let mut i = n;
    let mut acc = 0;
    while i > 0 { acc = acc + 1; }
    acc
}
public fn main() { print(spin(1)); }
""",
        "not silently drop a verification the author asked for",
    ),
    (
        "attribute-argument-typo",
        "T1025",
        """@verify(thorugh)
pure fn spin(n: Int) -> Int requires n >= 0 {
    let mut i = n;
    let mut acc = 0;
    while i > 0 { acc = acc + 1; }
    acc
}
public fn main() { print(spin(1)); }
""",
        "not silently drop a verification the author asked for",
    ),
    (
        "nonexistent-protocol-axiom",
        "T0989",
        """type Monoid is protocol {
    fn empty() -> Self;
    axiom left_identity(a: Self) ensures true;
};
type Sum is { v: Int };
implement Monoid for Sum {
    fn empty() -> Sum { Sum { v: 0 } }
    proof no_such_axiom_at_all by auto;
}
public fn main() { print("x"); }
""",
        "reject a proof clause naming an axiom the protocol does not declare",
    ),
    (
        "decreases-measure-grows",
        "T1026",
        """pure fn forever(n: Int) -> Int
    requires n >= 0
    decreases n
{
    if n <= 0 { 0 } else { forever(n + 1) }
}
public fn main() { print("x"); }
""",
        "reject a declared termination measure that increases",
    ),
    (
        "undeclared-context-use",
        "T1027",
        """context protocol Clock { fn now(&self) -> Int; }
type FixedClock is { at: Int };
implement Clock for FixedClock { fn now(&self) -> Int { self.at } }
fn sneaky() -> Int { Clock.now() }
public fn main() {
    provide Clock = FixedClock { at: 42 };
    print(sneaky());
}
""",
        "reject a function that uses a context it does not declare",
    ),
]

# Rows known to be silently accepted today. Each cites the open task that
# will flip it. Shrinking this set is the point; growing it needs a reason
# in the commit message.
BASELINE = {
    "unknown-attribute",
    "attribute-name-typo",
    "attribute-argument-typo",
    "nonexistent-protocol-axiom",
    "decreases-measure-grows",
    "undeclared-context-use",
}


def diagnosed(verum: str, src: str, work: Path) -> bool:
    """True when the compiler says something about the file.

    Both commands are consulted because the four defects live in
    different phases: a parser-level refusal shows up in `check`, a
    verification one only in `verify`. Silence from BOTH is the failure
    this gate is named after.
    """
    probe = work / "probe.vr"
    probe.write_text(src)
    for args in (["check", str(probe)], ["verify", str(probe)]):
        try:
            r = subprocess.run(
                [verum, *args], capture_output=True, text=True, timeout=300
            )
        except (subprocess.TimeoutExpired, FileNotFoundError):
            continue
        if r.returncode != 0:
            return True
        blob = r.stdout + r.stderr
        if "error<" in blob or "warning<" in blob:
            return True
        # `verify` reporting a failed obligation is also the compiler
        # saying so — it just says it in the summary rather than as a
        # diagnostic.
        if "failed" in blob and "0 failed" not in blob:
            return True
    return False


def main() -> int:
    verum = sys.argv[1] if len(sys.argv) > 1 else "verum"
    silent, spoke = [], []

    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        for name, task, src, expectation in CASES:
            if diagnosed(verum, src, work):
                spoke.append(name)
                print(f"  {name:28s} diagnosed          ({task})")
            else:
                silent.append(name)
                print(f"  {name:28s} SILENTLY ACCEPTED  ({task}) — must {expectation}")

    regressed = sorted(set(silent) - BASELINE)
    fixed = sorted(set(spoke) & BASELINE)

    print()
    print(f"silent: {len(silent)}/{len(CASES)}   baseline: {len(BASELINE)}")

    if regressed:
        print(
            "\nFAIL: rows that used to be diagnosed are now silent: "
            + ", ".join(regressed),
            file=sys.stderr,
        )
        return 1
    if fixed:
        print(
            "\nFAIL: rows are fixed but still listed in BASELINE: "
            + ", ".join(fixed)
            + "\n  Remove them from BASELINE and cite the task in the commit.",
            file=sys.stderr,
        )
        return 1

    print("check-silent-acceptance: OK (no regression against the baseline)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
