#!/usr/bin/env python3
"""A LIST TO READ: numeric claims on the site about things the repository
can COUNT.

NOT A GATE, and it cannot become one. Mapping "37-file dispatch table"
to `ls crates/verum_vbc/src/interpreter/dispatch_table/handlers/*.rs`
takes judgement, and a wrong mapping would fail a page for being right.
What a script CAN do is find the claims and put them where somebody
looks — which is the step that was missing, since every one of the five
below was found by reading a page for another reason.

FIVE FOUND BY HAND ON 2026-09-06, four of them on the ROADMAP — the
page a reader uses to decide whether to try the language:

    ~0.93 ns per CBGR check     the cited bench reports 1.69 ns; the
                                figure predates a re-measurement that is
                                recorded in CLAUDE.md
    37-file dispatch table      62 files
    1506 / 1507 checks (99.93%) the suite is 7136 spec files and that
                                denominator matches no level and no total
    60–70% cache hit rate       cites no run; the only figure in the tree
                                is `> 90%` written as a TARGET in a
                                module comment
    97 VDBE opcodes             89

Four of five were wrong, and the fifth was unsourced. That ratio is why
this list exists: a number on a documentation page has no reader who can
check it, so nothing corrects it, and it ages silently while the prose
around it stays true.

WHAT IT MATCHES. A digit-group followed by a countable noun. It is
deliberately noisy about VERSIONS and DATES (filtered) and deliberately
quiet about percentages with no noun, which are usually rates rather
than counts and need their own judgement.

HOW TO USE IT. For each row, ask two questions in order:
  1. does the repository contain the thing being counted?
  2. does the page say WHEN it was counted?
A claim that fails (2) is stale by construction the day the thing
changes, whatever its current value.
"""
import collections
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
DOCS = pathlib.Path(os.environ.get("VERUM_DOCS_DIR") or (REPO.parent / "website" / "docs"))

NOUNS = (r"files?|opcodes?|tests?|checks?|specs?|modules?|crates?|gates?|"
         r"functions?|types?|variants?|passes|phases?|rules?|handlers?|"
         r"instructions?|targets?|backends?|adapters?")
CLAIM = re.compile(rf"\b([0-9][0-9,]{{0,6}})[ -]+({NOUNS})\b", re.I)
# A version (`v0.32`, `3.17`), a date, an RFC or a code span are not counts.
NOISE = re.compile(r"(v?\d+\.\d+|20\d\d|RFC ?\d+|E\d{3})")
FENCE = re.compile(r"^```.*?^```", re.M | re.S)


def main() -> int:
    if not DOCS.is_dir():
        print(f"docs directory not found: {DOCS} — set VERUM_DOCS_DIR", file=sys.stderr)
        return 0
    pages = sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx"))
    rows = []
    for p in pages:
        text = p.read_text(encoding="utf-8", errors="replace")
        # Prose only: a count inside a code block is usually sample output.
        for lineno, line in enumerate(FENCE.sub("", text).splitlines(), 1):
            if NOISE.search(line):
                continue
            for m in CLAIM.finditer(line):
                rows.append((str(p.relative_to(DOCS)), lineno,
                             f"{m.group(1)} {m.group(2)}", line.strip()[:72]))

    by_page = collections.Counter(r[0] for r in rows)
    print(f"pages scanned            : {len(pages)}")
    print(f"countable claims found   : {len(rows)} across {len(by_page)} page(s)\n")
    print("Pages carrying the most, which is where to start — a page that "
          "counts many things\nis a page whose author was measuring, and "
          "measurements age:\n")
    for page, n in by_page.most_common(12):
        print(f"  {n:>3}  {page}")
    print("\nEvery claim, page-ordered:\n")
    for page, lineno, claim, ctx in rows:
        print(f"  {page}:{lineno}")
        print(f"      {claim:<18} {ctx}")
    print("\nNot a gate: a wrong claim and a correct one are the same text. "
          "Read them.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
