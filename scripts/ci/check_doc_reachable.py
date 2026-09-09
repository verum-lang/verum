#!/usr/bin/env python3
"""Gate: every documentation page must be reachable from the site itself.

A page nobody links to and the sidebar does not list exists only for a
search engine. It is not "extra documentation" — it is documentation
that cannot be found, and it drifts unnoticed because nobody reading the
site ever sees it.

WHY THIS EXISTS, measured 2026-09-09 on a tree that looked healthy:

  * `stdlib/overview.md`'s "Top-level modules" table listed 26 of the 52
    directories under `core/`. Twenty-five of the missing ones ALREADY
    HAD a written page — half the standard library was reachable only
    by guessing a URL.
  * `language/language-laws.md` (141 lines) and
    `architecture-types/ats-v2-direction.md` (78 lines) were in no
    sidebar and linked from no page.
  * `architecture-types/primitives/{cve,corpus}.md` (390 lines between
    them) were reachable only through one relative link in a table,
    while the sidebar category above them was labelled "Eight
    architectural primitives" and the page's own prose miscounted which
    was ninth.
  * `stdlib/theory_interop.md` is a SECOND, different page about a
    module that already has one, and only the other is reachable.

REACHABLE means: named in `sidebars.ts`, or linked from some other page
by an absolute `/docs/...` link or a relative `./x.md` one. Index and
overview pages are exempt because a category link names them
structurally.
"""
from __future__ import annotations
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SITE = Path(os.environ.get("VERUM_SITE_DIR") or (REPO.parent / "website"))
DOCS = Path(os.environ.get("VERUM_DOCS_DIR") or (SITE / "docs"))
SIDEBARS = SITE / "sidebars.ts"

LINK = re.compile(r"\]\(([^)\s]+)\)")

# KEYED, not silenced — the sibling gates' rule: a bare number carries no
# owner, so a floor cannot be told from a debt. Each key names the task
# that removes it.
KNOWN_UNREACHABLE = {
    "stdlib/theory_interop":
        "T1278 — a SECOND, different page for a module that already has "
        "`stdlib/theory-interop.md`. Not a stale copy: it carries "
        "sections the live page lacks (module layout, architecture "
        "stack, quick start, status, foundational alignment), so it "
        "cannot be deleted without a merge. Remove this key when T1278 "
        "closes; the page must then be gone, not merely linked.",
}
ID = re.compile(r"'([A-Za-z0-9][A-Za-z0-9_\-/]*)'")
EXEMPT_LEAF = {"index", "overview"}


def page_ids(docs: Path) -> set[str]:
    out = set()
    for p in list(docs.rglob("*.md")) + list(docs.rglob("*.mdx")):
        out.add(p.relative_to(docs).with_suffix("").as_posix())
    return out


def reachable(docs: Path, sidebars: Path) -> set[str]:
    seen: set[str] = set()
    if sidebars.is_file():
        seen |= set(ID.findall(sidebars.read_text(errors="ignore")))
    for p in list(docs.rglob("*.md")) + list(docs.rglob("*.mdx")):
        for m in LINK.finditer(p.read_text(errors="ignore")):
            u = m.group(1).split("#", 1)[0].strip()
            if not u or u.startswith(("http", "mailto:")):
                continue
            if u.startswith("/docs/"):
                seen.add(u[len("/docs/"):].rstrip("/"))
                continue
            try:
                tgt = (p.parent / u).resolve().relative_to(docs.resolve())
            except (ValueError, OSError):
                continue
            seen.add(tgt.with_suffix("").as_posix())
    return seen


def self_test() -> int:
    bad = 0
    if ID.findall("items: ['a/b', 'c-d/e_f'],") != ["a/b", "c-d/e_f"]:
        bad += 1; print("self-test: sidebar ids are not read")
    if LINK.findall("see [x](./y.md) and [z](/docs/q/r)") != ["./y.md", "/docs/q/r"]:
        bad += 1; print("self-test: links are not read")
    if LINK.findall("[a](https://x/y.md)")[0].startswith("http") is not True:
        bad += 1; print("self-test: an external link is not recognised")
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-reachable: {DOCS} not present — UNMEASURED.")
        return 0

    floor = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-pages" and i + 1 < len(sys.argv):
            floor = int(sys.argv[i + 1])

    pages = page_ids(DOCS)
    if len(pages) < floor:
        print(f"check-doc-reachable: only {len(pages)} page(s) under {DOCS}, "
              f"expected at least {floor} — the corpus is missing.")
        return 1
    if SIDEBARS.is_file() is False:
        print(f"check-doc-reachable: no sidebars.ts at {SIDEBARS} — every page "
              "would look unreachable. Refusing to report a count.")
        return 1

    seen = reachable(DOCS, SIDEBARS)
    lost = sorted(p for p in pages
                  if p not in seen and p.rsplit("/", 1)[-1] not in EXEMPT_LEAF)
    keyed = [p for p in lost if p in KNOWN_UNREACHABLE]
    lost = [p for p in lost if p not in KNOWN_UNREACHABLE]

    # A key that no longer applies is itself a defect: it would hide the
    # page's return. Say so rather than passing quietly.
    #
    # Only judge a key when we are looking at ITS corpus — a fixture or a
    # different docs tree has neither the page nor its neighbours, and
    # reading that as "the key went stale" makes the gate unusable
    # anywhere but one directory. Measured: the first version failed on
    # its own three-page self-test fixture for exactly that reason.
    def in_this_corpus(key: str) -> bool:
        parent = key.rsplit("/", 1)[0] + "/" if "/" in key else ""
        return any(pg.startswith(parent) for pg in pages)

    stale = [k for k in KNOWN_UNREACHABLE
             if in_this_corpus(k) and (k not in pages or k in seen)]
    if stale:
        print("check-doc-reachable: keyed page(s) no longer unreachable — "
              "remove the key: " + ", ".join(sorted(stale)))
        return 1

    print(f"check-doc-reachable: {len(lost)} of {len(pages)} page(s) are "
          "reachable from neither the sidebar nor any other page "
          f"({len(keyed)} keyed)")
    for p in lost:
        print(f"  {p}")
    for p in keyed:
        print(f"  [keyed] {p}: {KNOWN_UNREACHABLE[p]}")
    return 1 if lost else 0


if __name__ == "__main__":
    sys.exit(main())
