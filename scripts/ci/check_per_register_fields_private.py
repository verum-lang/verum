#!/usr/bin/env python3
"""Gate: no per-register fact in `FunctionContext` may be `pub`.

WHY THE SHAPE AND NOT THE INSTANCES. T1194 counted sixteen per-register
fields that survive `set_register` and proposed auditing the clear-list.
An audit is a statement about today's fields; this is a statement about
what a field can BE. A private field has ONE choke point — its setter —
so no caller can acquire the wrong write order. A `pub` field has as
many orders as it has callers, and no clear-list audit can see them all.

MEASURED, and this is the whole argument: of the ten per-register fields
that survived `set_register`, exactly ONE was `pub` —
`loadt_generic_regs` — and it was the only one written directly from
`instruction.rs` (four sites). It was therefore the only one whose write
could PRECEDE `set_register`, which meant the obvious repair — adding
`remove(&reg)` to the clear-list — would have silently undone its own
`insert` and broken the feature it exists for. Nine mechanical clears
and one landmine, and the landmine was identifiable in one command:
which of them is `pub`.

WHAT IT WOULD HAVE CAUGHT. `lower_call_method` const-folds a zero-arg
`default` / `zero` / `one` on a register the `LoadT` arm marked as a
generic type reference. VBC reuses register numbers. A register that
once held a `LoadT(Generic)` and is later reused for a real receiver
kept the mark — so the call answered with a constant instead of
dispatching.

WHAT IT DELIBERATELY DOES NOT COUNT. Fields keyed by something other
than a register (`(reg, field_idx)` pairs, name maps, the alloca
bookkeeping) are not per-register FACTS in this sense: they are not read
as "what does register N currently hold". The pattern below keys on the
declared type — `Map<u16, …>` or `Set<u16>` — because that is what
makes a field answerable about a register, and `registers` /
`register_slots` are exempt by name because they ARE the storage rather
than a fact about it.
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONTEXT = REPO / "crates" / "verum_codegen" / "src" / "llvm" / "context.rs"

# `pub <name>: HashMap<u16, …>` / `HashSet<u16>`, however the type is
# spelled (`std::collections::` prefixed or imported).
PUB_PER_REG = re.compile(
    r"^\s*pub\s+([a-z_][a-z0-9_]*)\s*:\s*(?:std::collections::)?"
    r"(?:HashMap|HashSet|BTreeMap|BTreeSet)\s*<\s*u16\s*[,>]",
    re.M,
)

# The value storage itself, and the alloca-mode bookkeeping that mirrors
# it. Clearing these in the setter would be nonsense, and they are read
# as storage rather than as facts.
EXEMPT = {
    "registers",
    "register_slots",
    "alloca_registers",
    "alloca_register_types",
}


def offenders(text: str):
    return [m.group(1) for m in PUB_PER_REG.finditer(text) if m.group(1) not in EXEMPT]


def self_test() -> int:
    """Both polarities, because a pattern that matches nothing passes."""
    bad = 0
    positive = "    pub sticky_marks: HashMap<u16, String>,\n"
    if offenders(positive) != ["sticky_marks"]:
        print("  SELF-TEST FAIL: a pub per-register field was not reported",
              file=sys.stderr)
        bad += 1
    else:
        print("  [ok] a `pub` per-register field IS reported")

    negative = "    sticky_marks: HashMap<u16, String>,\n"
    if offenders(negative):
        print("  SELF-TEST FAIL: a private field was reported", file=sys.stderr)
        bad += 1
    else:
        print("  [ok] a private per-register field is NOT reported")

    exempt = "    pub registers: HashMap<u16, BasicValueEnum<'ctx>>,\n"
    if offenders(exempt):
        print("  SELF-TEST FAIL: the storage itself was reported", file=sys.stderr)
        bad += 1
    else:
        print("  [ok] the value storage is exempt by name")

    unrelated = "    pub struct_string_fields: HashMap<(u16, u32), bool>,\n"
    if offenders(unrelated):
        print("  SELF-TEST FAIL: a (reg, field) map was reported", file=sys.stderr)
        bad += 1
    else:
        print("  [ok] a field keyed by more than a register is NOT a per-register fact")
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not CONTEXT.is_file():
        print(f"context.rs not found at {CONTEXT}", file=sys.stderr)
        return 2
    found = offenders(CONTEXT.read_text(encoding="utf-8"))
    if found:
        print(
            f"per-register-privacy: FAIL — {len(found)} per-register field(s) are "
            f"`pub` in FunctionContext:",
            file=sys.stderr,
        )
        for name in found:
            print(f"    {name}", file=sys.stderr)
        print(
            "\nA per-register fact must be private with a setter, so its write "
            "ORDER relative to `set_register` is enforced in one place rather "
            "than agreed by every caller. `set_register` clears these facts; a "
            "mark applied BEFORE the store is a mark immediately erased, and a "
            "mark that survives a store is the T1167 / T1194 stale-fact class.",
            file=sys.stderr,
        )
        return 1
    print("[ok] per-register-privacy: no per-register fact is `pub`")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
