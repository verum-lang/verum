#!/usr/bin/env python3
"""Gate: an error code cited in the documentation must exist in the registry.

`crates/verum_error/src/registry.rs` is the authority on which
diagnostic codes the compiler has — `registry_covers_every_emitted_code`
already pins that every emitted code appears there. Nothing pinned the
other direction for prose: a page could name a code that no namespace
has, and it read exactly like a code that does.

That is how this gate came to exist. Measured 2026-09-03, the site
cited twelve codes the registry does not contain. Two of them were the
codes for the errors the page was about:

    docs/stdlib/context.md   E3050 / E3051 / E3052   (direct /
                             transitive / conflicting negative-context
                             violations)

The real ones are E611, E609 and E608. `E3050` was not merely absent —
it survived in three doc comments inside `crates/verum_types` that
describe variants which emit E611, and the page copied them. A reader
following the citation into `verum --explain E3050` gets nothing.

Also found, same run: `E806: scope violation` (no such code; the
nearest real thing is the runtime `ContextError::ScopeViolation`
variant, a different layer), and `E805 / E807` from a table in
`core/context/error.vr` that names four DI codes, three of which do not
exist and the fourth of which — E808 — means something else entirely
("duplicate `provide` for one context").

WHAT COUNTS AS A CITATION: a bare `Exxx` / `Wxxx` token in prose or in
a table. Codes inside a fenced code block are NOT citations — a macro
author picking `.code("E9001")` for their own diagnostic is writing
their code, not naming the compiler's, and that was a real false
positive on the first run of this check.

ALLOWLIST: a page may deliberately name a code that does not exist, to
say that it does not (`docs/language/language-laws.md` does exactly
this for E431). Those go in the allowlist below, WITH the page, so the
exemption cannot silently widen to another file.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# Public-file hygiene: this path is the symlink the working checkout
# has; CI has no website tree and takes the SKIPPED branch below.
DOCS = REPO / "internal" / "website" / "docs"
REGISTRY = REPO / "crates" / "verum_error" / "src" / "registry.rs"

CODE = re.compile(r"\b([EW]\d{3,4})\b")
FENCE = re.compile(r"^```.*?^```", re.S | re.M)
REGISTRY_CODE = re.compile(r'code:\s*"([EW]\d+)"')

# (code, page-suffix) pairs a page names in order to say they are absent.
ALLOWED = {
    ("E431", "language/language-laws.md"),
    ("E3050", "stdlib/context.md"),
    ("E3051", "stdlib/context.md"),
    ("E3052", "stdlib/context.md"),
    ("E806", "stdlib/context.md"),
    # universes.md keeps its three invented codes inside a marked
    # caution, so a reader searching for `E1103` learns it does not
    # exist. The diagnostics themselves are still owed — when they land
    # and get registry entries, these three rows come out.
    ("E1103", "language/universes.md"),
    ("E1104", "language/universes.md"),
    ("W1105", "language/universes.md"),
    ("E4102", "language/meta/macro-kinds.md"),
    ("W501", "verification/tactic-dsl.md"),
}


def registry_codes(path: Path) -> set[str]:
    return set(REGISTRY_CODE.findall(path.read_text(errors="ignore")))


def citations(text: str) -> list[tuple[int, str]]:
    """Codes cited in prose. Code fences are blanked, keeping line numbers."""
    body = FENCE.sub(lambda m: "\n" * m.group(0).count("\n"), text)
    out = []
    for i, line in enumerate(body.split("\n"), 1):
        for code in CODE.findall(line):
            out.append((i, code))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--docs", type=Path, default=DOCS)
    ap.add_argument("--registry", type=Path, default=REGISTRY)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--require-docs", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        ok = True
        # A citation in prose is seen.
        if citations("the checker emits E605 here") != [(1, "E605")]:
            print("self-test FAIL: a prose citation was not seen")
            ok = False
        # A code inside a fence is NOT a citation — the motivating false
        # positive (a macro author's own `.code("E9001")`).
        if citations('```verum\n.code("E9001")\n```\n') != []:
            print("self-test FAIL: a fenced code read as a citation")
            ok = False
        # Line numbers survive the fence blanking.
        if citations('```\nx\n```\nE605\n') != [(4, "E605")]:
            print("self-test FAIL: fence blanking moved line numbers")
            ok = False
        # The motivating case: E3050 must not be mistaken for a real code.
        if not args.registry.is_file():
            print("self-test FAIL: registry not found — cannot check the case")
            ok = False
        else:
            real = registry_codes(args.registry)
            if not real:
                print("self-test FAIL: the registry parsed to zero codes")
                ok = False
            if "E605" not in real:
                print("self-test FAIL: E605 missing — the parse is wrong")
                ok = False
            if "E3050" in real:
                print("self-test FAIL: E3050 is in the registry after all")
                ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    if not args.registry.is_file():
        print(f"check-doc-error-codes: registry not found at {args.registry}",
              file=sys.stderr)
        return 1
    real = registry_codes(args.registry)
    if not real:
        print("check-doc-error-codes: the registry parsed to ZERO codes — "
              "the check would pass vacuously", file=sys.stderr)
        return 1

    docs_root = args.docs
    if not docs_root.is_dir():
        print(
            "check-doc-error-codes: SKIPPED — NOT CHECKED "
            f"(no documentation tree at {docs_root}; pass --docs PATH, "
            "or --require-docs to make this a failure)",
            file=sys.stderr,
        )
        return 1 if args.require_docs else 0

    bad: list[str] = []
    cited = 0
    pages = 0
    for src in sorted(docs_root.rglob("*.md")):
        text = src.read_text(errors="ignore")
        hits = citations(text)
        if hits:
            pages += 1
        rel = str(src.relative_to(docs_root))
        for line, code in hits:
            cited += 1
            if code in real:
                continue
            if any(code == c and rel.endswith(p) for c, p in ALLOWED):
                continue
            bad.append(f"  {rel}:{line}: {code} is in no error-code registry")

    print(f"check-doc-error-codes: {cited} citations over {pages} pages, "
          f"{len(real)} codes in the registry, {len(bad)} unknown")
    if bad:
        for b in sorted(set(bad)):
            print(b, file=sys.stderr)
        print("\nThe authority is crates/verum_error/src/registry.rs. If the "
              "code is real, add it there; if the page means to say a code "
              "does NOT exist, add the (code, page) pair to ALLOWED.",
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
