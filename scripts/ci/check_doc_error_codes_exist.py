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
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# Public-file hygiene: this path is the symlink the working checkout
# has; CI has no website tree and takes the SKIPPED branch below.
# The website is a SEPARATE repository (verum-lang/website). This gate
# reads it from a sibling checkout, which is what exists on a developer
# machine; a CI job must check the website out and point
# VERUM_DOCS_DIR at it, or the gate has nothing to measure and says so.
# See A81 in docs/architecture/tech-debt-register.md.
DOCS = Path(os.environ["VERUM_DOCS_DIR"]) if os.environ.get("VERUM_DOCS_DIR") \
    else REPO.parent / "website" / "docs"
REGISTRY = REPO / "crates" / "verum_error" / "src" / "registry.rs"

CODE = re.compile(r"\b([EW]\d{3,4})\b")
FENCE = re.compile(r"^```.*?^```", re.S | re.M)
REGISTRY_CODE = re.compile(r'code:\s*"([EW]\d+)"')
REGISTRY_ENTRY = re.compile(
    r'code:\s*"([EW]\d+)".*?description:\s*"([^"]*)"', re.S)
# The code INDEX page: the one page whose second column is the code's
# meaning rather than a severity or a page-local label.
INDEX_PAGE = "reference/diagnostics.md"
INDEX_ROW = re.compile(r'^\|\s*`?([EW]\d{3,4})`?\s*\|\s*([^|]+?)\s*\|', re.M)
INDEX_FLOOR = 40

# (code, page-suffix) pairs a page names in order to say they are absent.
# The page may differ from the registry when the EMITTER agrees with the
# PAGE — the registry being the one that is wrong.  This gate's own
# failure message has said "check which side the EMITTER agrees with"
# since W005; there was simply no way to record the answer.
#
# Measured 2026-09-10: seven registry descriptions name a condition no
# emit site produces, and the index page was a VERBATIM COPY of them.
# That is why this check was green over all seven — two copies of one
# mistake agree, and their agreement is what made it durable.  The page
# now follows the emitter; `make check-doc-error-code-meaning` is the
# gate that keeps it there.  The registry half is T1386.
#
# An entry goes STALE the moment the registry catches up, and stale is a
# failure: an exemption nobody can see being used is a standing licence.
PAGE_FOLLOWS_EMITTER: dict[str, str] = {
    # Found only after the meaning gate learned to follow a message
    # BUILT IN A VARIABLE — five codes are reachable no other way, and
    # this is the one where the page was wrong. `ambiguous name` reads
    # as an import clash; the emitter is about PROTOCOLS.
    "E105": "emits `ambiguous method call: `m` could refer to multiple "
            "protocols` — the registry's `ambiguous name` names a "
            "different clash",
    "E311": "emits `cannot borrow ... because field ... is already borrowed`",
    "E313": "emits `cannot move ... while it is borrowed`",
    "E501": "emits `invalid refinement predicate`, and at a second site "
            "`meta function ... must be pure but has side effects`",
    "E502": "emits `meta function ... uses runtime context(s) ... not "
            "available at compile time`",
    "E503": "emits `pure function ... has side effects`",
    "E601": "emits `visibility error: '...' is <vis> in module '...'`",
    "E602": "emits `ambiguous name: '...' is imported from multiple modules`",
}

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


# SECOND QUESTION, same corpus: a code can be real and still never
# reach a reader. `E203: module not found` is registered, documented in
# the E2xx table, and emitted by nothing — a `mount` at a module that
# does not exist answers `error<E402>`, which the Type table filed under
# "`Send` bound not satisfied". Three of the four E2xx module codes are
# like that; their conditions were folded into E401/E402 and the module
# namespace was left standing.
#
# The test is a string search for the quoted code outside the registry,
# which is how every emit site names one. Controls run first: E100 and
# E400 must be found (they are emitted constantly), E203 and E202 must
# not. A run where the controls disagree is a broken instrument, not a
# finding.
EMIT_BASELINE = 14
EMIT_CONTROLS = [("E100", True), ("E400", True), ("E203", False), ("E202", False)]


def emitted_codes(registry_path: Path) -> set[str]:
    """Codes named as a string literal anywhere in crates/ but the registry."""
    blob = []
    for f in sorted((REPO / "crates").rglob("*.rs")):
        if f == registry_path:
            continue
        blob.append(f.read_text(errors="ignore"))
    text = "\n".join(blob)
    return {c for c in re.findall(r'"([EW]\d{3,4})"', text)}


# THIRD QUESTION, same corpus and the one that motivated this section:
# a code can be real, emitted, AND cited under the wrong MEANING.
#
# Measured 2026-09-10.  A user file mounting one simple type name from two
# modules printed `error<E602>: ambiguous name: …`, and the index page said
# `| E602 | context cycle |` — faithfully, because that is what the registry
# says.  `context cycle` has no emitter anywhere in the tree; `ambiguous
# name` already owns E105.  A reader looking up the code they were shown
# read about a different subsystem, and the two existing questions above
# both stayed green: the code EXISTS and something DOES emit it.
#
# Scope is deliberately the index page alone.  Run over every table on the
# site the same comparison reports 17 rows, of which the majority are not
# defects: `language/patterns.md` puts the SEVERITY in the second column
# ("error" / "warning") and `verification/performance.md` uses short labels
# ("Missing bound vars").  A gate that flags a table for having a label
# column stops being read.  The index page is the one place whose second
# column is a definition and whose reader is looking a code UP.
#
# A page text that is a PREFIX of the registry's is accepted: the registry
# grew an "; also …" clause from a later measurement, so the page is
# incomplete rather than wrong.
def registry_meanings(path: Path) -> dict[str, str]:
    return {m.group(1): m.group(2)
            for m in REGISTRY_ENTRY.finditer(path.read_text(errors="ignore"))}


def normalise_meaning(text: str) -> str:
    """Compare meanings, not markdown.

    The page writes `**module not found**; also \u0060Send\u0060 not
    implemented` where the registry writes the same sentence plain.
    Comparing raw strings makes that a false positive, and a false positive
    over formatting is the expensive kind.
    """
    t = text.strip().lower().replace("**", "").replace("`", "")
    for dash in ("\u2014", "\u2013", "\u2212"):
        t = t.replace(dash, "-")
    return re.sub(r"\s+", " ", t).rstrip(". ")


def index_disagreements(docs_root: Path, meanings: dict[str, str]):
    """(rows_compared, [(code, page_text, registry_text)])."""
    page = docs_root / INDEX_PAGE
    if not page.is_file():
        return 0, []
    compared, bad = 0, []
    stale = []
    for code, text in INDEX_ROW.findall(page.read_text(errors="ignore")):
        if code not in meanings:
            continue          # question one already owns the absent case
        compared += 1
        p, r = normalise_meaning(text), normalise_meaning(meanings[code])
        agrees = p == r or r.startswith(p)
        if code in PAGE_FOLLOWS_EMITTER:
            if agrees:
                stale.append(code)
            continue
        if agrees:
            continue
        bad.append((code, text.strip(), meanings[code]))
    return compared, bad, stale


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
        # THIRD QUESTION's instrument.
        if normalise_meaning("**module not found**; also `Send` not implemented") \
                != normalise_meaning("module not found; also Send not implemented"):
            print("self-test FAIL: markdown emphasis is not normalised away")
            ok = False
        if normalise_meaning("type not found \u2014 no declaration") \
                != "type not found - no declaration":
            print("self-test FAIL: an em dash is not folded to a hyphen")
            ok = False
        if normalise_meaning("context cycle") == normalise_meaning("ambiguous name"):
            print("self-test FAIL: the E602 pair compares equal")
            ok = False
        if not normalise_meaning(
                "recursive type without indirection; also wrong number of type "
                "arguments").startswith(
                normalise_meaning("recursive type without indirection")):
            print("self-test FAIL: the prefix rule does not hold")
            ok = False
        if INDEX_ROW.findall("| `E602` | context cycle | yes |\n") \
                != [("E602", "context cycle")]:
            print("self-test FAIL: an index row is not read")
            ok = False
        if INDEX_ROW.findall("| `E0xx` | Parse | Lexing |\n") != []:
            print("self-test FAIL: the range table read as a code row")
            ok = False
        if args.registry.is_file():
            m = registry_meanings(args.registry)
            if m.get("E105") != "ambiguous name":
                print("self-test FAIL: registry descriptions are not parsed "
                      f"(E105 read as {m.get('E105')!r})")
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

    # Second question: cited AND registered, but nothing emits it.
    emitted = emitted_codes(args.registry)
    for code, want in EMIT_CONTROLS:
        if (code in emitted) != want:
            print(f"emit-control FAILED: {code} should "
                  f"{'be' if want else 'not be'} found at an emit site — "
                  "the instrument is wrong, not the documentation",
                  file=sys.stderr)
            return 1
    cited_codes = {c for _, c in
                   ((l, c) for src in sorted(docs_root.rglob("*.md"))
                    for l, c in citations(src.read_text(errors="ignore")))}
    dead = sorted(c for c in cited_codes if c in real and c not in emitted)
    print(f"check-doc-error-codes-emitted: {len(dead)} cited code(s) that "
          f"nothing emits (baseline {EMIT_BASELINE})")
    for c in dead:
        print(f"    {c}")

    meanings = registry_meanings(args.registry)
    compared, disagree, stale_follow = index_disagreements(docs_root, meanings)
    print(f"check-doc-error-codes-meaning: {len(disagree)} of {compared} "
          f"index row(s) disagree with the registry "
          f"({len(PAGE_FOLLOWS_EMITTER)} following the emitter by roster)")
    for code, page_t, reg_t in disagree:
        print(f"    {code}\n        page     = {page_t!r}"
              f"\n        registry = {reg_t!r}")

    if bad:
        for b in sorted(set(bad)):
            print(b, file=sys.stderr)
        print("\nThe authority is crates/verum_error/src/registry.rs. If the "
              "code is real, add it there; if the page means to say a code "
              "does NOT exist, add the (code, page) pair to ALLOWED.",
              file=sys.stderr)
        return 1
    if compared < INDEX_FLOOR:
        print(f"\nonly {compared} row(s) on {INDEX_PAGE} named a registered "
              f"code, expected at least {INDEX_FLOOR}. The page or its table "
              "shape is gone, so the meaning check measured almost nothing — "
              "refusing rather than passing.", file=sys.stderr)
        return 1
    if stale_follow:
        print(f"\n{len(stale_follow)} code(s) on the PAGE_FOLLOWS_EMITTER "
              f"roster now AGREE with the registry: "
              f"{', '.join(sorted(stale_follow))}. The registry caught up, so "
              "the exemption has nothing left to exempt — delete those rows.",
              file=sys.stderr)
        return 1
    if disagree:
        print(f"\n{len(disagree)} row(s) on {INDEX_PAGE} give a meaning the "
              "registry does not. The reader looked the code UP, so this page "
              "is where a wrong meaning costs most. Check which side the "
              "EMITTER agrees with before editing either — measured once, it "
              "was the registry that was wrong (W005 read `SelfShadowing` as "
              "being about the `self` keyword).", file=sys.stderr)
        return 1
    if len(dead) != EMIT_BASELINE:
        direction = "above" if len(dead) > EMIT_BASELINE else "below"
        print(f"\n{len(dead)} cited-but-unemitted code(s), {direction} the "
              f"baseline of {EMIT_BASELINE}. A code the documentation names "
              "and the compiler never produces sends a reader to a page for a "
              "message they cannot have seen. Adjust EMIT_BASELINE in a commit "
              "that says which code changed and why.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
