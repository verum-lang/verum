#!/usr/bin/env python3
"""Fail when a `Spec:` comment names a document no tracked file provides.

WHY THIS EXISTS
---------------
CLAUDE.md asks spec-tied code to cite a LOGICAL spec name, "never a path",
and `check_no_internal_refs.sh` already ratchets the PATH form
(`Spec: docs/…`) at zero. The bare-name form was unchecked, and it is
three times larger:

    Spec: docs/…            4 citations, gate at baseline 0
    Spec: <name>.md       879 citations, 767 of them resolving to nothing

The two evade different guards for the same reason. `check-internal-refs`
greps tracked files for the literal `internal/`; a citation that writes a
bare filename mentions no directory at all, so a document living only in
the gitignored tree is cited by three tracked files and the gate that
exists to forbid exactly that sees nothing. A rule enforced by matching a
string is evaded by not writing the string — with no intent required.

WHAT THIS CHECKS, AND WHAT IT DELIBERATELY DOES NOT
---------------------------------------------------
A citation naming `<something>.md` / `.ebnf` / `.tex` whose BASENAME no
tracked file provides. Basename rather than path, because the citation is
a logical name and the document may legitimately have moved.

Three things are out of scope on purpose:

* Citations of EXTERNAL specs — `Spec: SQLite C API`, `Spec: Rust RFC
  3637`. They name no file and cannot be resolved in-tree.
* Prose borrowed into the form — `Spec: Empty programs should be valid`.
  Same reason.
* A citation that resolves to a real document which does NOT say what the
  comment claims. Measured 2026-09-09: `infer/expr.rs:4128` cites the
  CBGR three-tier model for "deref on value types is identity", the
  document exists, and nothing in it says that. No existence check can
  catch that, and pretending otherwise would let a green run be read as
  "the citations are true".

A ROSTER, NOT A COUNT: 767 occurrences across 33 names is a wide shallow
rewrite that would collide with every session holding a core/ file, so
this stops the population GROWING rather than demanding it shrink. The
names are written out because a count cannot report a SWAP — one document
restored while another citation goes dangling leaves 33 at 33.
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]

CITATION = re.compile(r"(?://+|/\*+)\s*Spec:\s*([^\n]{1,120})")
NAMED_DOC = re.compile(r"\b([A-Za-z0-9_.-]+\.(?:md|ebnf|tex))\b")

# Documents cited by name that no tracked file provides, measured
# 2026-09-09. Every one of them existed under a public path once; the
# citations were left behind when the documents moved into the gitignored
# tree or were retired. A reader who follows one of these finds nothing
# and cannot tell whether the spec is missing or the name is wrong.
KNOWN = {
    "02-core-semantics.md", "02-introduction.md", "03-type-system.md",
    "04-memory-model.md", "05-syntax-grammar.md", "10-concurrency-model.md",
    "13-formal-proofs.md", "14-module-system.md", "16-context-system.md",
    "17-meta-system.md", "18-advanced-protocols.md", "20-error-handling.md",
    "26-unified-execution-architecture.md", "40-terminal-tui-architecture.md",
    "RUNTIME_CONSOLIDATION_PLAN.md", "SPIFFE-ID.md", "SPIFFE.md",
    "audit1.md", "cbgr-implementation.md", "database.md", "improvements.md",
    "math-spec.md", "meta-audit.md", "metrics-architecture.md",
    "mimalloc-allocator-plan.md", "new-arch-spec.md", "new-features.md",
    "observability.md", "sqlite-native.md", "tls-quic.md",
    "tracing-architecture.md", "verification-architecture.md",
    "verum-ext-2.md",
}


def tracked(*globs: str) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", *globs], cwd=REPO, capture_output=True, text=True
    ).stdout.split("\n")
    return [p for p in out if p]


# A NAME MATCH IS NOT A LINK RESOLUTION, and this set is the measured
# proof. `database.md` exists as `website/docs/stdlib/database.md`, so the
# website lookup below counted four citations as reachable — but they read
#
#     // Spec: database.md §4.1 + §5.5    core/database/mysql/transaction.vr:14
#     // Spec: database.md §6.1.6         core/database/postgres/copy.vr:14
#
# and the site page has ZERO numbered headings, while the document those
# section numbers belong to has 172. The citations mean a different
# document that happens to share a filename.
#
# Counting an ambiguous name as reachable is the expensive direction: a
# reader sent to the site page for §6.1.6 concludes they misread the
# citation. So an ambiguous name is treated as unreachable — a false RED
# at worst, and it costs a comment.
AMBIGUOUS = {"database.md"}


def website_docs() -> set[str]:
    """Document basenames the sibling website checkout provides.

    CLAUDE.md names `website:docs/*` as a legitimate thing to cite, so a
    citation resolving there is not dangling. The checkout is a SIBLING
    and may be absent (CI, a fresh clone); `main` says so out loud rather
    than letting its absence quietly widen the finding."""
    site = REPO.parent / "website"
    if not site.is_dir():
        return set()
    return {
        p.name
        for p in site.rglob("*.md*")
        if "node_modules" not in p.parts
    }


def scan() -> dict[str, list[str]]:
    """Cited document names that nothing a reader can open provides."""
    provided = ({pathlib.Path(p).name for p in tracked()} | website_docs()) - AMBIGUOUS
    found: dict[str, list[str]] = {}
    for rel in tracked("*.rs", "*.vr"):
        try:
            text = (REPO / rel).read_text(errors="ignore")
        except OSError:
            continue
        for m in CITATION.finditer(text):
            doc = NAMED_DOC.search(m.group(1))
            if doc and doc.group(1) not in provided:
                found.setdefault(doc.group(1), []).append(rel)
    return found


def compare(found: set[str], roster: set[str]) -> tuple[list, list]:
    """Split what the tree cites against what the roster claims.

    Separated from the scan so a control can drive it without a
    checkout — the half a count ratchet has and never tests."""
    return sorted(found - roster), sorted(roster - found)


def self_test() -> int:
    """THE SWAP, which is the shape a count cannot report."""
    app, gone = compare({"a.md"}, {"b.md"})
    if not app or not gone:
        print(
            "self-test: a swap of equal size reported nothing — the roster "
            "comparison has degenerated back into a count",
            file=sys.stderr,
        )
        return 1
    if compare({"a.md"}, {"a.md"}) != ([], []):
        print("self-test: an unchanged population reported a difference",
              file=sys.stderr)
        return 1
    # An external citation must NOT be read as a document name.
    if NAMED_DOC.search("SQLite C API — session extension."):
        print("self-test: an external spec name was read as a filename",
              file=sys.stderr)
        return 1
    print(f"[ok] self-test: roster holds {len(KNOWN)} name(s); a same-size "
          f"swap is reported; an external citation is not a filename")
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()

    if not (REPO.parent / "website").is_dir():
        print(
            "note: the sibling `website` checkout is absent, so a citation "
            "that resolves to a site page cannot be recognised here and would "
            "be reported as dangling.",
            file=sys.stderr,
        )
    found = scan()
    total = sum(len(v) for v in found.values())
    appeared, disappeared = compare(set(found), KNOWN)

    print(
        f"check-spec-citation-names: {total} citation(s) name a document no "
        f"tracked file provides, over {len(found)} name(s) "
        f"({len(KNOWN)} on the roster)"
    )
    for name in sorted(found, key=lambda n: (n not in set(appeared), -len(found[n]))):
        mark = "NEW " if name in set(appeared) else "    "
        print(f"  {mark}{len(found[name]):4d}x  {name}  e.g. {found[name][0]}")

    if appeared:
        print(
            f"\n{len(appeared)} name(s) marked NEW: {' '.join(appeared)}\n"
            "A reader who follows one of these finds nothing, and cannot tell\n"
            "whether the spec is missing or the name is wrong. Cite a document\n"
            "the reader can open — docs/architecture/*, grammar/verum.ebnf,\n"
            "website:docs/* — or state the requirement in place (CLAUDE.md).",
            file=sys.stderr,
        )
        return 1
    if disappeared:
        print(
            f"\nThe roster claims {len(disappeared)} name(s) the tree now "
            f"provides or no longer cites: {' '.join(disappeared)}\n"
            "Remove them from KNOWN in this file — the ground gained is "
            "recorded by NAME, not by a smaller number.",
            file=sys.stderr,
        )
        return 1
    print("[ok] spec-citation names: roster exact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
