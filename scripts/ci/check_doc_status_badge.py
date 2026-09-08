#!/usr/bin/env python3
"""Gate: a page that DECLARES a status must RENDER one, and the same one.

WHY THIS EXISTS, measured 2026-09-08. `website:docs/stdlib/mem.md`
carried `status: partial` in its frontmatter and rendered no badge at
all: it imports the per-file `StdlibBadge` family (`LifecycleBadge`,
`TierBadge`, `TestCovBadge`) and the unused `ModuleStatus` default,
never `<StdlibStatus />`. A reader saw a page with no status while the
site's own convention page says every module page carries one.

That is the same class as the fifty invisible statuses fixed on
2026-09-07 — a status recorded where the build reads it and not where
the READER does — with one page left over because it uses a different
component family, so a sweep keyed on the component missed it.

The check is the pair, not either half: frontmatter and rendered badge
must agree. A page with neither is not a module page and is skipped.

FENCED EXAMPLES DO NOT COUNT. The convention pages
(`stdlib/overview.md`, `stdlib/status-convention.md`) SHOW the component
inside an ```mdx block, and reading that as the page's own badge
reported two mismatches that were documentation working correctly. The
fence stripping is what makes this gate honest, and it is in the
self-test.
"""
from __future__ import annotations
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS = Path(os.environ.get("VERUM_STDLIB_DOCS")
            or os.environ.get("VERUM_DOCS_DIR")
            or (REPO.parent / "website" / "docs"))

FRONTMATTER = re.compile(r"\A---\n(.*?)\n---\n", re.S)
STATUS_FM = re.compile(r"^status:\s*(\S+)", re.M)
BADGE = re.compile(r'<StdlibStatus\s[^>]*?status="([^"]+)"', re.S)
FENCE = re.compile(r"^```.*?^```", re.M | re.S)


def strip_fences(text: str) -> str:
    return FENCE.sub("", text)


def self_test() -> int:
    bad = 0
    if BADGE.findall('<StdlibStatus status="partial" />') != ["partial"]:
        bad += 1; print("self-test: a one-line badge is not read")
    if BADGE.findall('<StdlibStatus\n  status="partial"\n  detail="x"\n/>') != ["partial"]:
        bad += 1; print("self-test: a multi-line badge is not read")
    if BADGE.findall(strip_fences('```mdx\n<StdlibStatus status="partial" />\n```\n')) != []:
        bad += 1; print("self-test: a FENCED example still reads as a badge")
    if STATUS_FM.findall("title: mem\nstatus: partial\n") != ["partial"]:
        bad += 1; print("self-test: the frontmatter status is not read")
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-status-badge: {DOCS} not present — UNMEASURED.")
        return 0

    floor = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-pages" and i + 1 < len(sys.argv):
            floor = int(sys.argv[i + 1])

    considered, bad = 0, []
    for f in sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx")):
        text = f.read_text(errors="ignore")
        fm = FRONTMATTER.match(text)
        declared = None
        if fm:
            m = STATUS_FM.search(fm.group(1))
            declared = m.group(1).strip("\"'") if m else None
        rendered = None
        body = strip_fences(text[fm.end():] if fm else text)
        m = BADGE.search(body)
        if m:
            rendered = m.group(1)
        if declared is None and rendered is None:
            continue
        considered += 1
        if declared != rendered:
            bad.append((f.relative_to(DOCS).as_posix(), declared, rendered))

    if considered < floor:
        print(f"check-doc-status-badge: only {considered} page(s) carry a "
              f"status under {DOCS}, expected at least {floor} — the corpus "
              "is missing or the patterns stopped matching.")
        return 1

    print(f"check-doc-status-badge: {len(bad)} of {considered} page(s) "
          "declare a status they do not render (or the reverse)")
    for rel, d, r in bad:
        print(f"  {rel}: frontmatter={d} rendered={r}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
