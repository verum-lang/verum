#!/usr/bin/env python3
"""A `verum_x::a::b` citation in the docs must name a module that exists.

WHY THIS AND NOT "does the name appear somewhere". Those are different
questions and each one alone looks clean. Measured 2026-09-07 on the
site docs:

  * this check — 213 citations, SIX unresolved;
  * a whole-tree NAME check on one of the affected pages — 34 distinct
    identifiers, FIVE absent.

The overlap was ONE. `verum_smt::model::extract_model` passes the name
check (`extract_model` occurs in `z3_backend.rs`) and fails this one:
there is no `verum_smt::model` module, and the occurrence is
`advanced_extract_model`, a different function. A name that appears
somewhere is not the name that was cited.

The six: a page documenting a five-stage counterexample pipeline named
`verum_smt::model::extract_model`, `verum_smt::model::desmtify`,
`verum_verification::counterexample::synthesize_contradiction` and
`verum_verification::fix_suggestions` — none of which exist — plus
three citations that merely dropped the `commands::` segment
(`verum_cli::audit::…` for `verum_cli::commands::audit::…`). Two
classes with the same shape: a reader who greps the citation finds
nothing.

THE PATTERN MUST NOT REQUIRE THE PATH TO END THE SPAN. The first
version anchored on the closing backtick, so
`verum_smt::model::extract_model(solver) -> RawModel` — a path with a
call signature inside the same backticks — was invisible. Two of the
three "Implementation:" lines on that page sat in the blind spot;
210/4 became 213/6 when the anchor was relaxed to a lookahead.
"""

import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS_ENV = os.environ.get("VERUM_DOCS_DIR")
DOCS = Path(DOCS_ENV) if DOCS_ENV else REPO.parent / "website" / "docs"

CITE = re.compile(
    r"`(verum_[a-z_]+(?:::[a-z_][a-z0-9_]*)+)(?:::[A-Z][A-Za-z0-9]*)?(?=[`(\s<])"
)


def real_modules(root: Path) -> set[str]:
    """Crate name plus every module path its `src/` tree declares."""
    mods: set[str] = set()
    crates = root / "crates"
    if not crates.is_dir():
        return mods
    for cargo in crates.rglob("Cargo.toml"):
        src = cargo.parent / "src"
        if not src.is_dir():
            continue
        crate = cargo.parent.name
        mods.add(crate)
        for f in src.rglob("*.rs"):
            rel = f.relative_to(src).with_suffix("")
            parts = [p for p in rel.parts if p != "mod"]
            for i in range(1, len(parts) + 1):
                mods.add("::".join([crate] + parts[:i]))
    return mods


def audit(mods: set[str], text: str):
    """Returns (total, [unresolved paths])."""
    total, bad = 0, []
    for m in CITE.finditer(text):
        total += 1
        path = m.group(1)
        if path in mods:
            continue
        # the final segment may be a FUNCTION, not a module
        if "::".join(path.split("::")[:-1]) in mods:
            continue
        bad.append(path)
    return total, bad


def self_test() -> int:
    mods = {"verum_cli", "verum_cli::commands", "verum_cli::commands::audit"}
    fails = []

    total, bad = audit(mods, "see `verum_cli::commands::audit::run` for this")
    if (total, bad) != (1, []):
        fails.append(f"a real path with a trailing fn was flagged: {bad}")

    total, bad = audit(mods, "see `verum_cli::audit::run` for this")
    if bad != ["verum_cli::audit::run"]:
        fails.append(f"a missing module was not flagged: {bad}")

    # the blind spot that cost two findings: a call signature in the span
    total, bad = audit(mods, "Implementation: `verum_cli::nope::f(solver) -> X`.")
    if bad != ["verum_cli::nope::f"]:
        fails.append(f"a path followed by a signature was not read: {bad}")

    # a bare crate name is fine
    total, bad = audit(mods, "the `verum_cli` crate")
    if total != 0:
        fails.append("a bare crate name should not count as a path citation")

    for f in fails:
        print(f"SELF-TEST FAIL: {f}")
    print(f"self-test: {'ok' if not fails else str(len(fails)) + ' failure(s)'}")
    return 1 if fails else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    floor = 0
    if "--min-citations" in sys.argv:
        floor = int(sys.argv[sys.argv.index("--min-citations") + 1])
    if not DOCS.is_dir():
        print(f"check-doc-module-paths: no docs at {DOCS} — set VERUM_DOCS_DIR")
        return 0
    mods = real_modules(REPO)
    if not mods:
        print("check-doc-module-paths: no crates/ found — refusing to report "
              "a clean run against nothing")
        return 1
    total, defects = 0, []
    for md in sorted(DOCS.rglob("*.md")):
        t, bad = audit(mods, md.read_text(errors="ignore"))
        total += t
        for b in bad:
            defects.append(f"{md.relative_to(DOCS)}: {b}")
    if total < floor:
        print(f"check-doc-module-paths: VACUOUS — {total} citation(s) read "
              f"under {DOCS}, floor is {floor}. Zero defects here is a "
              "statement about the gate's reach, not about the corpus.")
        return 1
    print(f"check-doc-module-paths: {total} citation(s), {len(defects)} unresolved")
    for d in defects:
        print(f"  {d}")
    return 1 if defects else 0


if __name__ == "__main__":
    sys.exit(main())
