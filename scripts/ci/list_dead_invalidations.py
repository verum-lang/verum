#!/usr/bin/env python3
"""A LIST TO READ: `clear_*` / `reset_*` / `forget_*` methods on the
codegen contexts that NOTHING CALLS, and whether the state they exist to
drop is dropped anywhere else.

WHY. A dead invalidation is worse than a missing one, because it reads as
a guarantee. `VbcCodegenContext::clear_byte_array_vars` has zero callers;
anyone auditing `byte_array_vars` sees a `clear_` method and stops. The
set is keyed by variable NAME and the context is per-MODULE — `reset()`
is the module boundary, and there is no per-function one — so a
`let buf: [Byte; 16]` in one function marks the name `buf` for every
later function in that module (T1219).

MEASURED at the time of writing: 7 of 14 invalidations have no callers.

AND THE FIRST THING IT DID WAS DISPROVE THE CLAIM IT WAS WRITTEN FOR.
I had filed T1219 saying `byte_array_vars` LEAKS across functions,
because `clear_byte_array_vars` has no callers and `reset()` is the
per-module boundary. The second question below answers it: the field is
dropped one more time, in `begin_function`, INLINE — so there IS a
per-function boundary and nothing leaks. What is true is narrower and
still worth fixing: the method misleads an auditor into thinking the
clearing goes through it, and it misled me.

TWO QUESTIONS, NOT ONE, and the second is what makes this readable:

    is the METHOD called?   a `clear_x()` nobody calls
    is the FIELD dropped?   a `self.x.remove(...)` / `.clear()` somewhere
                            else — because a field can be invalidated
                            without its named method

A method with no callers whose FIELD is dropped elsewhere is DEAD CODE
(delete it, or route the other site through it). A method with no callers
whose field is dropped NOWHERE is a STATE LEAK — the interesting row.

WHY A LIST AND NOT A GATE. Some of these are deliberate: a fact may be
designed to live for the whole module, and a `clear_` written for a
future caller is a decision, not a defect. The list cannot tell those
apart and does not try. It reports the pair of answers and leaves the
reading to a person.
"""
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CONTEXTS = [
    REPO / "crates" / "verum_vbc" / "src" / "codegen" / "context.rs",
    REPO / "crates" / "verum_codegen" / "src" / "llvm" / "context.rs",
]
CRATES = [REPO / "crates" / "verum_vbc" / "src", REPO / "crates" / "verum_codegen" / "src"]

DECL = re.compile(
    r"^[ \t]*pub(?:\(crate\))?[ \t]+fn[ \t]+"
    r"((?:clear|reset|unmark|forget|drop)_[a-z0-9_]*)[ \t]*\(", re.M)
# The body of a one- or two-line clear names the field it drops.
FIELD = re.compile(r"self\.([a-z_][a-z0-9_]*)\s*\.\s*(?:clear|remove|take)\s*\(")


def call_count(sources, name: str) -> int:
    """`.name(` anywhere — a DECLARATION is `fn name(`, which this cannot
    match, so the definition never counts as its own caller."""
    pat = re.compile(r"\.\s*" + re.escape(name) + r"\s*\(")
    return sum(len(pat.findall(s)) for s in sources.values())


def field_dropped_elsewhere(sources, field: str, own_body: str) -> int:
    r"""Count `self.<field>.clear()/remove()/take()` OUTSIDE the method's
    own body.

    THE FIRST VERSION SUBTRACTED NOTHING and so reported every dead
    method as "dropped 1x elsewhere" — counting the method's OWN body as
    another site, which is the instrument measuring itself. It extracted
    the body with a regex ending at `\n    \}`; that is one indentation
    convention out of several, and when it failed to match the
    subtraction silently became zero. The body is now passed IN, from the
    same window the field name was read from, so the two can never
    disagree."""
    pat = re.compile(r"self\.\s*" + re.escape(field) + r"\s*\.\s*(?:clear|remove|take)\s*\(")
    total = sum(len(pat.findall(s)) for s in sources.values())
    return total - len(pat.findall(own_body))


def self_test() -> int:
    bad = 0

    def check(label, got, want):
        nonlocal bad
        if got == want:
            print(f"  [ok] {label}")
        else:
            print(f"  SELF-TEST FAIL: {label} -> {got!r} (wanted {want!r})",
                  file=sys.stderr)
            bad += 1

    check("a declaration is found",
          DECL.findall("    pub fn clear_marks(&mut self) {\n"), ["clear_marks"])
    check("a non-invalidation name is not",
          DECL.findall("    pub fn set_marks(&mut self) {\n"), [])
    check("`pub(crate)` counts",
          DECL.findall("    pub(crate) fn reset_x(&mut self) {\n"), ["reset_x"])
    check("the DEFINITION is not its own caller",
          call_count({"a": "pub fn clear_marks(&mut self) {}"}, "clear_marks"), 0)
    check("a real call counts",
          call_count({"a": "self.ctx.clear_marks();"}, "clear_marks"), 1)
    check("the field a clear drops is recovered",
          FIELD.findall("self.byte_array_vars.clear();"), ["byte_array_vars"])
    check("a method's OWN body is not counted as another site — the bug "
          "that reported every dead method as 'dropped 1x elsewhere'",
          field_dropped_elsewhere({"a": "self.xs.clear();"}, "xs",
                                  "self.xs.clear();"), 0)
    check("...but a genuine second site still counts",
          field_dropped_elsewhere({"a": "self.xs.clear();\nself.xs.remove(1);"},
                                  "xs", "self.xs.clear();"), 1)
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    sources = {}
    for root in CRATES:
        for f in root.rglob("*.rs"):
            sources[str(f)] = f.read_text(errors="replace")
    if not sources:
        print("dead-invalidations: FAIL — scanned 0 source files.", file=sys.stderr)
        return 2

    rows = []
    for c in CONTEXTS:
        if not c.is_file():
            print(f"dead-invalidations: FAIL — {c} not found; the file moved and "
                  f"this list is measuring nothing.", file=sys.stderr)
            return 2
        text = c.read_text(errors="replace")
        for m in DECL.finditer(text):
            name = m.group(1)
            # The window stops at the method's closing brace so a
            # neighbouring method's body cannot be read as this one's.
            window = text[m.end():m.end() + 400]
            end = window.find("\n    }")
            body = window[:end] if end >= 0 else window
            fm = FIELD.search(body)
            rows.append((c.parts[-4], name, call_count(sources, name),
                         fm.group(1) if fm else None, body))

    if not rows:
        print("dead-invalidations: FAIL — 0 invalidation methods found. The "
              "naming convention changed; this is not a clean result.",
              file=sys.stderr)
        return 2

    dead = [r for r in rows if r[2] == 0]
    print(f"invalidation methods on the codegen contexts : {len(rows)}")
    print(f"  with NO callers                            : {len(dead)}\n")

    leaks, deadcode = [], []
    for crate, name, _, field, body in dead:
        elsewhere = field_dropped_elsewhere(sources, field, body) if field else -1
        (deadcode if elsewhere > 0 else leaks).append((crate, name, field, elsewhere))

    named = [r for r in leaks if r[2]]
    unnamed = [r for r in leaks if not r[2]]
    print(f"STATE LEAK — nothing calls the method AND the field it drops is "
          f"dropped nowhere else ({len(named)}):")
    for crate, name, field, _ in sorted(named):
        print(f"   {crate:<14} {name:<30} field `{field}`")
    print(f"\nUNCLASSIFIED — no caller, and the body drops no Rust field, so "
          f"neither question applies ({len(unnamed)}). READ THESE: a `clear_`"
          f" may EMIT code rather than drop state, and then its siblings are "
          f"the question. `clear_exception_value` was the first, and it emits "
          f"an LLVM store of zero into an alloca — measured, its whole family "
          f"(`store_` / `load_` / `clear_exception_value`) has ZERO callers, "
          f"so it is an unused feature and not a leak at all:")
    for crate, name, _, _ in sorted(unnamed):
        print(f"   {crate:<14} {name}")
    print(f"\nDEAD CODE — nothing calls the method, but the field IS dropped "
          f"elsewhere ({len(deadcode)}). Route that site through the method or "
          f"delete it; either way the method currently misleads:")
    for crate, name, field, n in sorted(deadcode):
        print(f"   {crate:<14} {name:<30} field `{field}` dropped {n}x elsewhere")
    print("\nNever gate on these numbers: a fact may be DESIGNED to live for a "
          "whole module, and a `clear_` written for a future caller is a "
          "decision. This list reports the two answers and leaves the reading "
          "to a person.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
