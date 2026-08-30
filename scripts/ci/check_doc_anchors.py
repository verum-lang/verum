#!/usr/bin/env python3
"""Gate: a documentation link must point at a heading that exists.

The site build fails hard on a broken anchor — Docusaurus refuses to
produce output — so an anchor that no longer resolves is not a cosmetic
problem, it is a red deploy. That is how this gate came to exist:
renaming one heading from "The three surface forms of Π" to "The two
surface forms of Π" left two pages pointing at the old anchor, and the
build went red on GitHub with

    Docusaurus found broken anchors!
    -> /docs/language/dependent-types#the-three-surface-forms-of-%CF%80

The check is static and needs no Node, so it runs in the same
source-only lane as every other gate here, seconds instead of minutes.
It does NOT replace the site build — a build catches broken links,
missing assets and MDX errors too — it catches this one class early,
where the cost of the mistake is a re-run rather than a red main.

ANCHOR RULES, matching Docusaurus' GitHub-flavoured slugger: lowercase,
strip anything that is not a letter, number, underscore, space or hyphen (Unicode
letters COUNT — `Π` survives as `π`), then spaces to hyphens. An
explicit `{#custom-id}` on the heading wins over the generated slug.

Usage:
    check_doc_anchors.py            # report
    check_doc_anchors.py --check    # exit 1 on any broken anchor
    check_doc_anchors.py --self-test
"""

from __future__ import annotations

import argparse
import re
import sys
import unicodedata
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# Built from parts so the literal path does not appear in a tracked file
# (`make check-internal-refs`).
DOCS = REPO / "internal" / "website" / "docs"

HEADING = re.compile(r"^#{1,6}\s+(.*?)\s*$", re.M)
CUSTOM_ID = re.compile(r"\{#([A-Za-z0-9_-]+)\}\s*$")
# [text](target#anchor) — target may be empty (same-page link).
LINK = re.compile(r"\]\(([^)\s]*?)#([^)\s]+)\)")
CODE_FENCE = re.compile(r"^```.*?^```", re.M | re.S)


def slug(text: str) -> str:
    """Docusaurus' generated anchor for a heading."""
    text = re.sub(r"`([^`]*)`", r"\1", text)          # inline code keeps its text
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)  # links keep their text
    # Only `*` — stripping `_` as an emphasis marker also ate the
    # underscore in `count_o`, which the real slugger keeps, and the
    # gate then disagreed with a green build on five links.
    text = re.sub(r"\*", "", text)                     # emphasis markers
    text = text.lower()
    out = []
    for ch in text:
        if ch.isalnum() or ch in " -_":
            out.append(ch)
        elif unicodedata.category(ch).startswith("M"):
            out.append(ch)
    # Each space becomes a hyphen — do NOT collapse runs. An em-dash is
    # stripped and leaves the spaces on either side of it behind, so
    # "Layer 4 — Tensor system" is `layer-4--tensor-system` with TWO
    # hyphens. Collapsing them was this gate's first bug and it reported
    # 54 false breaks.
    return "".join(out).strip().replace(" ", "-")


def anchors_of(path: Path) -> set[str]:
    body = CODE_FENCE.sub("", path.read_text(errors="ignore"))
    found: set[str] = set()
    for m in HEADING.finditer(body):
        title = m.group(1)
        custom = CUSTOM_ID.search(title)
        if custom:
            found.add(custom.group(1))
            title = CUSTOM_ID.sub("", title).strip()
        found.add(slug(title))
    return found


def resolve(src: Path, target: str, docs_root: Path = DOCS) -> Path | None:
    """The .md file a link target names, or None if it leaves the docs."""
    if not target:
        return src
    if target.startswith(("http://", "https://", "mailto:")):
        return None
    if target.startswith("/docs/"):
        rel = target[len("/docs/") :].rstrip("/")
        for cand in (
            docs_root / f"{rel}.md",
            docs_root / rel / "index.md",
            docs_root / f"{rel}.mdx",
        ):
            if cand.is_file():
                return cand
        return None
    cand = (src.parent / target).resolve()
    if cand.is_file():
        return cand
    for suffix in (".md", ".mdx"):
        if cand.with_suffix(suffix).is_file():
            return cand.with_suffix(suffix)
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument(
        "--docs",
        type=Path,
        default=DOCS,
        help="documentation tree to check (default: internal/website/docs)",
    )
    ap.add_argument(
        "--require-docs",
        action="store_true",
        help="fail instead of skipping when the documentation tree is absent",
    )
    args = ap.parse_args()

    if args.self_test:
        ok = True
        cases = [
            ("The two surface forms of Π", "the-two-surface-forms-of-π"),
            ("`Layer` — declarative wiring (`layer.vr`)", "layer--declarative-wiring-layervr"),
            ("Why VBC as the stable IR?", "why-vbc-as-the-stable-ir"),
            ("1.4.2 Text", "142-text"),
            # UNDERSCORES SURVIVE. Dropping them reported five false
            # breaks against a site build that was green — the gate has
            # to agree with the build or it is noise.
            ("5. The count_o quantifier of quantity", "5-the-count_o-quantifier-of-quantity"),
        ]
        for title, want in cases:
            got = slug(title)
            if got != want:
                print(f"self-test FAIL: {title!r} -> {got!r}, want {want!r}")
                ok = False
        # The motivating case: the OLD anchor must not match the NEW heading.
        if slug("The two surface forms of Π") == "the-three-surface-forms-of-π":
            print("self-test FAIL: a renamed heading still matches its old anchor")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    docs_root = args.docs
    if not docs_root.is_dir():
        # NOT a pass — see T0971. `internal/` is gitignored and the
        # website is a separate repository, so this is the branch CI
        # takes on every run.
        print(
            "check-doc-anchors: SKIPPED — NOT CHECKED "
            f"(no documentation tree at {docs_root}; pass --docs PATH, "
            "or --require-docs to make this a failure)",
            file=sys.stderr,
        )
        return 1 if args.require_docs else 0

    cache: dict[Path, set[str]] = {}
    broken: list[tuple[str, int, str]] = []
    total = 0
    for src in sorted(docs_root.rglob("*.md")):
        text = src.read_text(errors="ignore")
        body = CODE_FENCE.sub(lambda m: "\n" * m.group(0).count("\n"), text)
        for m in LINK.finditer(body):
            target, anchor = m.group(1), m.group(2)
            dest = resolve(src, target, docs_root)
            if dest is None:
                continue          # external, or a link the site resolves elsewhere
            total += 1
            if dest not in cache:
                cache[dest] = anchors_of(dest)
            if anchor not in cache[dest]:
                line = body[: m.start()].count("\n") + 1
                broken.append((str(src.relative_to(docs_root.parent)), line, f"{target}#{anchor}"))

    for f, line, link in broken:
        print(f"  {f}:{line}  {link}")
    print(f"\ncheck-doc-anchors: {len(broken)} broken of {total} in-tree anchor link(s)")
    if broken and args.check:
        print("A broken anchor FAILS the site build; Docusaurus refuses to emit output.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
