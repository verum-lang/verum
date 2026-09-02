#!/usr/bin/env python3
"""Count the diagnostics that live BEHIND a narrow early return (T1078).

`check_protocol_impl_is_complete` (verum_types/src/infer/decls.rs)
opens with

    let Some((required_set, declared)) =
        self.protocols_with_known_defaults.get(&proto_ident).cloned()
    else { return; };

whose precondition — the protocol's SOURCE was seen — is correct for
the METHOD-COMPLETENESS check it was written for, and narrower than
several other checks that later moved into the same function.  T1074's
associated-type bound check inherited it and was silent for every
archive-loaded protocol until it was moved ahead of the return.

This gate does not judge whether a tenant belongs there: only a reader
knows whether a given check needs a source-seen protocol.  It fails
when the COUNT changes, so that moving in — or out — is a deliberate,
reviewed act rather than a side effect.

    python3 scripts/ci/check_early_return_tenants.py
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGET = ROOT / "crates" / "verum_types" / "src" / "infer" / "decls.rs"
FUNCTION = "fn check_protocol_impl_is_complete"
GUARD = "self.protocols_with_known_defaults.get(&proto_ident)"
# Diagnostics, not helpers: what a tenant EMITS is what makes its
# silence cost a user something.
EMIT = re.compile(r"push_diagnostic_for|report_[a-z_]+\(")
EXPECTED = 2


def main() -> int:
    lines = TARGET.read_text(encoding="utf-8").split("\n")

    fn_at = next((i for i, l in enumerate(lines) if FUNCTION in l), None)
    if fn_at is None:
        print(f"early-return-tenants: FAIL — `{FUNCTION}` not found in {TARGET.name}.")
        print("  The gate cannot locate its subject; rename it here or restore the function.")
        return 1

    guard_at = next((i for i in range(fn_at, len(lines)) if GUARD in lines[i]), None)
    if guard_at is None:
        print("early-return-tenants: FAIL — the guard this gate watches is gone.")
        print(f"  Looked for `{GUARD}` after line {fn_at + 1}.")
        print("  If the early return was removed, delete this gate and say so in the commit.")
        return 1

    # The function's own closing brace: the first line that is exactly
    # four spaces and `}` after the guard.
    end_at = next(
        (i for i in range(guard_at + 1, len(lines)) if lines[i] == "    }"),
        len(lines),
    )

    tenants = [
        (i + 1, lines[i].strip())
        for i in range(guard_at, end_at)
        if EMIT.search(lines[i])
    ]

    if len(tenants) == EXPECTED:
        print(
            f"early-return-tenants: OK — {len(tenants)} diagnostic(s) behind the "
            f"source-seen guard (expected {EXPECTED})."
        )
        return 0

    verb = "moved in" if len(tenants) > EXPECTED else "left"
    print(
        f"early-return-tenants: FAIL — {len(tenants)} diagnostic(s) behind the "
        f"guard, expected {EXPECTED}: something {verb}."
    )
    print(
        "  The guard's precondition is 'this protocol's SOURCE was seen', which is\n"
        "  narrower than most checks need — a tenant that does not need it is\n"
        "  SILENT for every archive-loaded protocol, with no diagnostic to notice."
    )
    for line_no, text in tenants:
        print(f"    decls.rs:{line_no}  {text[:88]}")
    print(f"  If the move is intended, update EXPECTED in {pathlib.Path(__file__).name}.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
