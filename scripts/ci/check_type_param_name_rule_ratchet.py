#!/usr/bin/env python3
"""Ratchet: the "is this identifier a type parameter" rule, counted.

THE RULE. Several places in the compiler decide whether a `Named` type
is an unresolved GENERIC PARAMETER by looking at how the name is SPELT —
short, and uppercase — rather than by consulting a binding. `unify.rs`
states the trade in its own comment: a length test replaced a hardcoded
exclusion list (`"Ok" | "No" | "Eq" | "Fn" | "IO"`), which was the right
call at that site, and the same reasoning was then made independently
elsewhere.

WHY THIS IS A RATCHET AND NOT A BAN. The rule is load-bearing. It is
what lets an unsubstituted `T` unify with a concrete type at a point
where the binding is genuinely gone, and removing the copies without
restoring the bindings would break inference. Nothing here says "delete
them". It says: this population must not grow silently, and the
DISAGREEMENT between the copies must be visible.

THE DISAGREEMENT, which is the actual finding and is why the count alone
would be too weak. The copies do not use the same boundary:

    verum_smt + verum_types (5 sites)   name.len() == 1
    verum_vbc/codegen (6 sites)        name.len() <= 2

The split is exactly by crate, which is what makes it a phase
disagreement rather than scattered inconsistency: everything that
reasons about TYPES uses one boundary and everything that lowers to
BYTECODE uses the other.

So a type named `Io`, `Ex`, `Ok`, or any other two-character name is a
type PARAMETER to the bytecode codegen and a CONCRETE TYPE to the
unifier. Each copy is locally consistent, every test passes, and nothing
in the tree reports that the two phases answer differently — which is
what makes a divergence like this survive.

MEASURED CONSEQUENCE (T1206): `Result<Int, Foo>` misreads a `Text`
payload under AOT and prints an address; `Result<Int, F>` does not.
Thirteen type names, one variable, a clean break at length 2. The
single-letter branch is the one that answers CORRECTLY — so the
heuristic is not the defect there, it is the accidental workaround, and
the ordinary concrete path is the broken one. That is worth knowing
before anyone "fixes" a copy: making the rule stricter would route the
last working case into the broken branch.

THE CLASS: the compiler asks the SPELLING of an identifier a question
only the BINDING can answer — and each place that asks it invents its
own threshold. Siblings: T1191 (a name deciding a binding), A98 (a name
owning a slot).
"""
import collections
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CRATES = REPO / "crates"

LEN_TEST = re.compile(
    r"(?P<what>[A-Za-z_.()\[\]:<>&\s]{0,40}?)\.len\(\)\s*(?P<op>==|<=|<)\s*(?P<n>\d)")
UPPER = re.compile(r"is_(?:ascii_)?uppercase")
# The SUBJECT of the length test must be a name-like binding. An early
# version used `s\b` as one alternative and matched the trailing "s" of
# `args.len() == 1` — a test about ARITY, not about a spelling. The
# self-test's fourth case exists because of that, and it caught it.
NAMEISH = re.compile(
    r"(?:^|[^A-Za-z0-9_])(?:name|ident|tname|oname|s|str|"
    r"[a-z_]*_name|[a-z_]*_ident)$", re.I)

# Raise ONLY with a measurement and a reason. Lowering is the goal.
BASELINE = 11
BASELINE_THRESHOLDS = {"== 1": 5, "<= 2": 6}


def scan(root):
    rows = []
    for f in sorted(root.rglob("*.rs")):
        if "/target/" in str(f):
            continue
        lines = f.read_text(encoding="utf-8", errors="replace").splitlines()
        for i, line in enumerate(lines):
            m = LEN_TEST.search(line)
            if not m:
                continue
            if not UPPER.search("\n".join(lines[i:i + 3])):
                continue
            what = m.group("what").strip()
            # `name.as_str().len()` and `name.chars().count()` are the
            # same subject wearing an accessor; strip the trailing call
            # so the name-likeness test sees the binding.
            what = re.sub(r"(?:\.(?:as_str|to_string|as_ref|name)\(\))+$", "", what)
            if not NAMEISH.search(what):
                continue
            try:
                shown = str(f.relative_to(REPO))
            except ValueError:      # self-test runs on a temp dir
                shown = str(f)
            rows.append((shown, i + 1, f"{m.group('op')} {m.group('n')}"))
    return rows


def self_test() -> int:
    """Both polarities. A pattern that matches nothing ratchets happily."""
    import tempfile
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        root = pathlib.Path(d)
        (root / "hit.rs").write_text(
            "fn f() { if name.len() == 1 && name.chars().all(|c| c.is_uppercase()) {} }\n")
        (root / "miss_nolen.rs").write_text(
            "fn f() { if name.chars().all(|c| c.is_uppercase()) {} }\n")
        (root / "miss_nocase.rs").write_text(
            "fn f() { if name.len() == 1 {} }\n")
        (root / "miss_notaname.rs").write_text(
            "fn f() { if args.len() == 1 && x.is_uppercase() {} }\n")
        got = scan(root)
        names = sorted(pathlib.Path(r[0]).name for r in got)
        if names != ["hit.rs"]:
            print(f"  SELF-TEST FAIL: expected only hit.rs, got {names}", file=sys.stderr)
            bad += 1
        else:
            print("  [ok] 1 site reported from 1 length+case test on a name")
            print("  [ok] 0 reported from a case test with no length test")
            print("  [ok] 0 reported from a length test with no case test")
            print("  [ok] 0 reported from `args.len()` — not a name")
    if BASELINE <= 0:
        print("  SELF-TEST FAIL: baseline must be positive", file=sys.stderr)
        bad += 1
    else:
        print(f"  [ok] baseline is {BASELINE} across {len(BASELINE_THRESHOLDS)} "
          f"threshold(s): {BASELINE_THRESHOLDS}")
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    rows = scan(CRATES)
    n = len(rows)
    thresholds = collections.Counter(r[2] for r in rows)

    if n == 0:
        print("type-param-name-rule: FAIL — scanned the workspace and found ZERO "
              "sites. The rule cannot have vanished without a refactor that would "
              "also have updated this gate; the pattern has gone stale and must not "
              "report OK.", file=sys.stderr)
        return 2

    if n > BASELINE:
        print(f"type-param-name-rule: FAIL — {n} site(s) decide type-parameter-ness "
              f"from a name's spelling, baseline {BASELINE}. New site(s):",
              file=sys.stderr)
        for r in rows:
            print(f"    {r[0]}:{r[1]}   threshold {r[2]}", file=sys.stderr)
        print("\nDo not add an eleventh spelling of this rule. Ask the BINDING: a "
              "type parameter is one that was declared, and a declared parameter "
              "reaches the checker as a TypeVar rather than a `Named`. If the "
              "binding is genuinely gone by the time you need the answer, that "
              "erasure is the defect worth fixing.", file=sys.stderr)
        return 1

    if n < BASELINE:
        print(f"type-param-name-rule: FAIL — {n} site(s), BELOW the baseline of "
              f"{BASELINE}. That is good news the gate cannot accept silently: "
              f"lower the baseline to {n} in this file, in the same commit that "
              f"removed the site(s), so the next regression is measured against "
              f"what is actually there.", file=sys.stderr)
        return 1

    print(f"[ok] type-param-name-rule: {n} site(s) at baseline, "
          f"thresholds {dict(thresholds)}")
    if len(thresholds) > 1:
        print(f"     NOTE: the copies DISAGREE — a two-character type name is a "
              f"type parameter to some phases and a concrete type to others. "
              f"See the module docstring and T1206.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
