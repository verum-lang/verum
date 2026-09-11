#!/usr/bin/env python3
"""Spec assertions a compiler can answer without running what it emitted.

The shape that bit (T1442/T1443): a value derived through a MECHANISM —
a field read, a tuple index, a dereference — bound to a local with no
call in the initialiser, then asserted against an integer literal. The
front end can follow every step, folds the comparison on its own model,
and the assertion verifies the compiler agrees with itself.

    let v = bound.0;          // the mechanism under test
    assert(v == 42);          // FOLDED — passed while v printed an ADDRESS
    assert(v + 0 == 42);      // sees the register — failed correctly

NOT counted, because there is nothing for the fold to get wrong:
  * `let x = 42; assert(x == 42)` — the initialiser IS the literal;
  * an initialiser containing a call — measured not to fold;
  * a binding mutated between definition and assertion (`let mut sum = 0`
    then a loop) — the initialiser does not determine the value;
  * a spec that never RUNS. This one deflated the count by a factor of
    forty and is the reason the filter exists: the first version reported
    124 hits in 47 files with a striking distribution (45 in
    `reference_system`), and then only THREE of those 47 files carry
    `@test: run`. The rest are `typecheck-pass` / `typecheck-fail`, whose
    assertions are never executed, so no fold can hide anything. A census
    that counts unexecuted assertions is measuring the corpus's shape,
    not its risk.
"""
import re, pathlib, sys

ROOT = pathlib.Path(__file__).resolve().parents[0]
SPECS = pathlib.Path("vcs/specs")
ASSERT_LIT = re.compile(r'\bassert\(\s*([a-z_]\w*)\s*(?:==|!=|<|>|<=|>=)\s*(-?\d+)\s*\)')
LET = re.compile(r'\blet\s+(mut\s+)?([a-z_]\w*)\s*(?::[^=]+)?=\s*([^;]+);')
# a mechanism the compiler can follow: field/tuple access or a deref
MECHANISM = re.compile(r'(\.\w+|\.\d+|^\s*\*)')

RUNS = re.compile(r'^//\s*@test:\s*run\b', re.M)


def census(root: pathlib.Path, executed_only: bool = True):
    out = []
    for p in sorted(root.rglob("*.vr")):
        t = p.read_text(errors="replace")
        if executed_only and not RUNS.search(t):
            continue            # its assertions never execute
        asserted = {m.group(1) for m in ASSERT_LIT.finditer(t)}
        if not asserted:
            continue
        for m in LET.finditer(t):
            is_mut, name, init = m.group(1), m.group(2), m.group(3).strip()
            if name not in asserted or is_mut:
                continue
            if "(" in init or "[" in init:
                continue          # a call or an index: not folded
            if not MECHANISM.search(init):
                continue          # a bare literal or name: nothing to get wrong
            out.append((str(p), name, init[:48]))
    return out

def self_test() -> int:
    """Poles: the shape must be caught, and three look-alikes must not."""
    import tempfile, os
    bad = 0
    RUN = "// @test: run\n"
    cases = [
        (RUN + "let v = bound.0;\nassert(v == 42);\n", 1, "the shape"),
        (RUN + "let x = 42;\nassert(x == 42);\n", 0, "initialiser IS the literal"),
        (RUN + "let v = f(bound);\nassert(v == 42);\n", 0, "a call is not folded"),
        (RUN + "let mut sum = 0;\nassert(sum == 42);\n", 0, "mutated between"),
        # AND THE POLE THAT DEFLATED THE COUNT BY FORTY: the same shape in
        # a spec that never runs. Without this the census reports 124
        # where the executed population is 14.
        ("// @test: typecheck-pass\nlet v = bound.0;\nassert(v == 42);\n",
         0, "a spec that never runs"),
    ]
    d = pathlib.Path(tempfile.mkdtemp())
    for i, (src, want, why) in enumerate(cases):
        f = d / f"c{i}.vr"; f.write_text(src)
        got = len(census(d / f"c{i}.vr".replace("c", "")) if False else
                  [c for c in census(d) if c[0].endswith(f"c{i}.vr")])
        if got != want:
            bad += 1
            print(f"FAIL {why}: {got}, expected {want}", file=sys.stderr)
        f.unlink()
    if bad:
        return 1
    print(f"[ok] self-test: {len(cases)} pole(s) hold")
    return 0

if __name__ == "__main__":
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    hits = census(SPECS)
    shape = census(SPECS, executed_only=False)
    print(f"assertions a compiler can answer: {len(hits)} in "
          f"{len({h[0] for h in hits})} EXECUTED spec(s) "
          f"(the same shape appears {len(shape)} times overall, but a spec "
          f"that never runs cannot have an assertion folded out from under it)")
    for h in hits[:20]:
        print(f"    {h[1]:16} = {h[2]:50} {h[0]}")
