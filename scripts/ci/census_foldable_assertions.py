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
    then a loop) — the initialiser does not determine the value.
"""
import re, pathlib, sys

ROOT = pathlib.Path(__file__).resolve().parents[0]
SPECS = pathlib.Path("vcs/specs")
ASSERT_LIT = re.compile(r'\bassert\(\s*([a-z_]\w*)\s*(?:==|!=|<|>|<=|>=)\s*(-?\d+)\s*\)')
LET = re.compile(r'\blet\s+(mut\s+)?([a-z_]\w*)\s*(?::[^=]+)?=\s*([^;]+);')
# a mechanism the compiler can follow: field/tuple access or a deref
MECHANISM = re.compile(r'(\.\w+|\.\d+|^\s*\*)')

def census(root: pathlib.Path):
    out = []
    for p in sorted(root.rglob("*.vr")):
        t = p.read_text(errors="replace")
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
    cases = [
        ("let v = bound.0;\nassert(v == 42);\n", 1, "the shape"),
        ("let x = 42;\nassert(x == 42);\n", 0, "initialiser IS the literal"),
        ("let v = f(bound);\nassert(v == 42);\n", 0, "a call is not folded"),
        ("let mut sum = 0;\nassert(sum == 42);\n", 0, "mutated between"),
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
    print(f"assertions a compiler can answer: {len(hits)} in "
          f"{len({h[0] for h in hits})} file(s)")
    for h in hits[:20]:
        print(f"    {h[1]:16} = {h[2]:50} {h[0]}")
