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
    # SECOND POSITIVE CONTROL, also not in the baseline. Loop invariant
    # preservation is one of the seven capabilities that genuinely work
    # (debt register section H) and the only one with no gate of its own,
    # because the showcase chapter that would have carried it was drafted
    # and deleted — the postcondition half is still open (T0905). A false
    # invariant must stay refused; if it ever goes quiet, a working
    # guarantee has been lost with nothing else watching.
    (
        "false-loop-invariant",
        "-",
        """@verify(thorough)
pure fn g(n: Int) -> Int requires n >= 0 {
    let mut i = 0;
    let mut u = 0;
    while i < n
        invariant i >= 0
        invariant u < 0
        decreases n - i
    { u = u + 1; i = i + 1; }
    u
}
public fn main() { print(g(3)); }
""",
        "always be diagnosed — the invariant is false on entry",
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
    # Declaration ORDER decides whether purity is checked at all. The
    # registry showcase already gates `pure fn b() { impure_helper() }`
    # and it passes — but only because that probe happens to declare the
    # helper ABOVE its caller. Move the helper below and the same program
    # is accepted (T0985). A gate that tests one spelling of a two-sided
    # rule is testing the side that works, which is why this row exists
    # beside the showcase's.
    #
    # BOUNDED, so nobody over-generalises from it: the showcase's other
    # rejection rows were re-measured with their declarations moved BELOW
    # the use — affine use-after-move (E310), the const-generic width
    # mismatch (E400) and `spawn` inside a `pure fn` (E503) all still
    # fire. Order-sensitivity here is a property of purity crossing a
    # CALL, not of checking in general, and one row is the right number.
    #
    # FIXED on lang/grammar-authority and not on main. When that branch
    # lands (T1024) this row starts being diagnosed, the gate fails with
    # "fixed but still listed in BASELINE", and the landing commit is the
    # one that removes it. That failure is the ratchet working, not a
    # regression.
    (
        "pure-calls-impure-declared-later",
        "T0985",
        """pure fn caller() -> Int { helper() }
fn helper() -> Int { print("io"); 1 }
public fn main() { print(caller()); }
""",
        "refuse a pure function calling an impure one regardless of declaration order",
    ),
    # A file DECLARES that it must fail, and does not. `verum check`
    # consults `@expected-error` only when errors exist: when the
    # expected error is ABSENT it prints an ordinary success line and
    # exits 0, the same exit code as a file whose expected error WAS
    # found. Two opposite outcomes, one verdict.
    #
    # Found by running a conformance spec directly, without vtest, and
    # nearly reading "both binaries pass" off two zeros that meant
    # opposite things. The official runner may well check this; measured
    # here only for `verum check`, which is what a person reaches for
    # when the runner is not built.
    # Protocol conformance checks the method NAME and not its SIGNATURE,
    # so a generic bound compiled against the protocol reaches a method
    # that takes and returns something else. This row is the day's only
    # instance where the un-honoured input reaches EXECUTION as a wrong
    # value rather than stopping at a verdict (T1029).
    #
    # The positive control for it lives in the same task, not here: an
    # implementation MISSING a method is refused with E405, so the
    # conformance phase runs and this is a gap in what it compares.
    (
        "protocol-signature-mismatch",
        "T1029",
        """type Greeter is protocol {
    fn greet(&self, times: Int) -> Int;
};
type P is { tag: Int };
implement Greeter for P {
    fn greet(&self, name: Text) -> Bool { true }
}
fn use_it<T: Greeter>(g: &T) -> Int { g.greet(42) }
public fn main() { print(use_it(&P { tag: 1 })); }
""",
        "refuse an implementation whose method signature differs from the protocol's",
    ),
    # `verum check` accepts a program the interpreter then panics on.
    # Found by verum-2b (T1010): the SHORT mount spelling does not merely
    # fail to help, it REPLACES a working binding with one that never
    # resolves — the same program with no mount at all runs correctly,
    # and with the full path runs correctly.
    #
    # Baseline taken on both binaries before landing T1024: identical
    # panic on main and on lang/grammar-authority, so the branch neither
    # fixes nor worsens it.
    #
    # COUPLING, stated because it is the one row that has it: this probe
    # needs `math.tactics.TacticProp` to exist in the baked stdlib. If it
    # is renamed the row starts being diagnosed for the wrong reason and
    # the gate will say "fixed but still in BASELINE" — noisy, but it
    # fails toward someone looking rather than away.
    (
        "short-mount-passes-check",
        "T1010",
        """mount math.tactics.{TacticProp};
fn main() { let x = TacticProp(1); print("ok"); }
""",
        "refuse, or resolve, a mount path it cannot bind — not accept and panic at run time",
    ),
    (
        "unmet-expected-error",
        "T1028",
        """// @test: typecheck-fail
// @expected-error: E503
// @description: declares it must be refused, and is not

pure fn clean(n: Int) -> Int { n + 1 }
public fn main() { print(clean(1)); }
""",
        "refuse a file whose declared @expected-error never appeared",
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
BASELINE: set[str] = set()
# EMPTY, and that is the point: every row in this file is now diagnosed.
# The ratchet started at eight silent rows (T1025 x3, T1026, T1027,
# T1028, T1029, T0989). An addition here is a REGRESSION, not a
# baseline — a row may only enter with a task id and a measurement.

# Rows whose stated expectation has TWO satisfying arms — "refuse, OR
# resolve" — and whose second arm lives at run time.
#
# `short-mount-passes-check` is the one: its expectation reads "refuse,
# or resolve, a mount path it cannot bind — not accept and panic at run
# time". `probe()` runs only `check` and `verify`, so it could see the
# first arm and never the second, and reported the row SILENT for a
# program that compiles AND runs correctly. Measured 2026-09-01 on a
# binary carrying T1010's repair (e0052cf26): five runs, exit 0, "ok"
# printed each time.
#
# Counting a clean compile as "silently accepted" is right for every
# other row here, because every other row describes a program that must
# NOT be accepted. This one describes a mount that must WORK.
RESOLVED_IF_IT_RUNS = {
    "short-mount-passes-check",
}


def probe(verum: str, src: str, work: Path, name: str = "") -> str:
    """One of "diagnosed", "resolved", "silent", "mute".

    MUTE is a THIRD outcome and exists because of a measured failure, not
    a hypothesis: with the volume full, `verum check` stopped printing
    entirely — no "Checking", no "error", no "Finished" — and every
    sweep written as `errors == 0 -> PASS` turned green. A peer's 372-file
    sweep reported 34 newly-passing files; re-measured, three were real.

    A tool that cannot speak is indistinguishable from a program with
    nothing wrong, unless absence of output is checked for separately.
    So silence with no sign of life is never counted as either verdict:
    it fails the gate, because a run with mutes has not answered its
    question.
    """
    probe_file = work / "probe.vr"
    probe_file.write_text(src)
    alive = False
    for args in (["check", str(probe_file)], ["verify", str(probe_file)]):
        try:
            r = subprocess.run(
                [verum, *args], capture_output=True, text=True, timeout=300
            )
        except (subprocess.TimeoutExpired, FileNotFoundError):
            continue
        blob = r.stdout + r.stderr
        if any(k in blob for k in ("Checking", "Verifying", "Finished", "Summary")):
            alive = True
        if r.returncode != 0:
            return "diagnosed"
        if "error<" in blob or "warning<" in blob:
            return "diagnosed"
        # `verify` reporting a failed obligation is also the compiler
        # saying so — it just says it in the summary rather than as a
        # diagnostic.
        if "failed" in blob and "0 failed" not in blob:
            return "diagnosed"
    if not alive:
        return "mute"
    # The second arm, for the rows that have one: a clean compile is the
    # right answer when the program is supposed to WORK, and only running
    # it can tell.
    if name in RESOLVED_IF_IT_RUNS:
        try:
            r = subprocess.run(
                [verum, "run", str(probe_file)],
                capture_output=True,
                text=True,
                timeout=300,
            )
        except (subprocess.TimeoutExpired, FileNotFoundError):
            return "silent"
        blob = r.stdout + r.stderr
        if not any(k in blob for k in ("Running", "ok", "error", "panic")):
            return "mute"
        if r.returncode == 0 and "panic" not in blob:
            return "resolved"
    return "silent"


def main() -> int:
    verum = sys.argv[1] if len(sys.argv) > 1 else "verum"
    silent, spoke, mute = [], [], []

    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        for name, task, src, expectation in CASES:
            verdict = probe(verum, src, work, name)
            if verdict == "mute":
                mute.append(name)
                print(f"  {name:28s} MUTE — the tool printed nothing  ({task})")
            elif verdict == "diagnosed":
                spoke.append(name)
                print(f"  {name:28s} diagnosed          ({task})")
            elif verdict == "resolved":
                spoke.append(name)
                print(f"  {name:28s} resolved at run    ({task})")
            else:
                silent.append(name)
                print(f"  {name:28s} SILENTLY ACCEPTED  ({task}) — must {expectation}")

    if mute:
        print(
            "\nFAIL: the tool printed nothing for: "
            + ", ".join(sorted(mute))
            + "\n  A mute run has not answered the question."
            f"\n  First: does `{verum}` exist and run? The binary is a POSITIONAL"
            "\n  argument — a flag like `--verum path` becomes the path itself,"
            "\n  every probe raises FileNotFoundError, and EVERY row goes mute."
            "\n  Then: `df -h` and `sysctl vm.swapusage` — a full volume silences"
            "\n  the compiler outright, which is why this outcome exists.",
            file=sys.stderr,
        )
        return 1

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
