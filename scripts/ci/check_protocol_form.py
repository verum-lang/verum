#!/usr/bin/env python3
"""Fail when a `.vr` file declares a protocol outside the grammatical form.

WHY THIS EXISTS
---------------
`grammar/verum.ebnf` admits exactly one protocol declaration:

    protocol_def = 'protocol' , protocol_body ;

and `protocol_def` appears only inside `type_def`, so the form is

    type X is protocol { … }

The parser also accepts a bare `public protocol X { … }`, and that form is
not equivalent: the declaration parses, the file checks clean, and the
protocol is NOT EXPORTED.  Every `mount` of it is then a phantom, resolving
to an unrelated same-named symbol if one exists and failing with E401 if it
does not — and neither outcome points at the declaration.

Measured when this gate was written (T0794).  Three files in `core/` used
the bare form against 549 that did not:

    core/database/common/migrations/store.vr:105  public protocol MigrationStore
    core/configuration/format.vr:122              public protocol ConfigFormat
    core/archive/mod.vr:208                       public protocol Archive

Only the FIRST failed to export, which is what makes the form dangerous
rather than merely non-standard.  Each of the three names is declared
exactly once in `core/`, so this was not one symbol shadowing another, and
neither `async fn` nor `&mut self` explains the difference: `AsyncIterator`
has both, in canonical form, and exports.  The bare form works until it
does not, and nothing reports which case you are in.

Repairing the one declaration also cleared three modules that import
through it — `core/database/common/migrations/mod.vr` plus the postgres and
sqlite `migrations/mod.vr` — taking the core/ failure count from 463 to
459 with no new failures.

This gate is textual on purpose.  The compiler cannot report the class: it
accepts both forms, and the difference only shows up as a missing export
somewhere else entirely.

SCOPE
-----
`core/` by default, and `core/` is clean.  Pass roots explicitly to widen it.

`vcs/` is deliberately NOT gated: a sweep found 46 sites there, 19 of them
under `vcs/fuzz/seeds/`, where arbitrary and even ungrammatical input is the
point.  The other 27 are conformance specs that should be converted, but each
carries `@expected-error` / `@expected-error-count` directives, so the change
has to be made against a run of the suite rather than by rewriting text —
tracked separately, not silently normalised here.

Usage:
    check_protocol_form.py [root ...]     (defaults to core/)
"""

from __future__ import annotations

import pathlib
import re
import sys

# `protocol` as the declaration keyword at the start of an item, optionally
# behind a visibility modifier.  The canonical form has `protocol` in the
# middle of the line (`type X is protocol {`), never at the front, so
# anchoring to the start of the item is what separates the two.
BARE_PROTOCOL = re.compile(
    r"^\s*(?:public\s+|pub(?:\([^)]*\))?\s+|internal\s+)?protocol\s+[A-Za-z_]\w*"
)

DEFAULT_ROOTS = ("core",)


def offenders(roots: list[str]) -> list[tuple[str, int, str]]:
    found: list[tuple[str, int, str]] = []
    for root in roots:
        base = pathlib.Path(root)
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.vr")):
            for lineno, line in enumerate(path.read_text(errors="ignore").splitlines(), 1):
                # A doc comment quoting the wrong form is not a declaration.
                if line.lstrip().startswith("//"):
                    continue
                if BARE_PROTOCOL.match(line):
                    found.append((str(path), lineno, line.strip()))
    return found


def main() -> int:
    roots = sys.argv[1:] or list(DEFAULT_ROOTS)
    found = offenders(roots)

    if not found:
        print("check_protocol_form: OK — every protocol uses `type X is protocol`")
        return 0

    for path, lineno, text in found:
        print(f"  {path}:{lineno}  {text}")
    print(
        f"GATE FAIL: {len(found)} protocol declaration(s) outside the grammar "
        "(grammar/verum.ebnf: protocol_def is only reachable through type_def).\n"
        "Write `type X is protocol { … }`.  The bare form parses and checks "
        "clean but may leave the protocol unexported, so every mount of it "
        "becomes a phantom.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
