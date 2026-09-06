#!/usr/bin/env python3
"""Names a documentation example USES that `core/` does not DECLARE.

WHY A PARSE GATE CANNOT SEE THIS.  `check_doc_blocks_parse.py` asks whether a
block parses.  `PostgresDatabase.connect(&db_url)` parses perfectly and names
nothing that exists; so did `supervisor_config()`, `Json(u)` as a call, and
`type Database.ReadOnly`.  Every one of those was live on the site while the
parse baseline stood at zero.  The defect is SEMANTIC — a name in the example
against the names the standard library declares — and it needs its own count.

WHAT IT COUNTS.  A capitalised receiver (`Name.method(...)`) used inside a
```verum block that `core/` does not declare as a type, protocol, function,
constant, static or context.  Capitalised receivers are the highest-signal
class: that is exactly the shape every fictional API on this site has taken.

WHAT IT DELIBERATELY DOES NOT COUNT.
  * lowercase calls — most are the reader's own helper, declared as such in
    the prose (`find_user`, `load_config`, `do_work`), and counting them
    drowns the signal;
  * anything the DOCUMENTATION CORPUS declares, in any block of any page.
    Scope was widened twice, and each widening was forced by a measured
    false positive.  Block-scoped reported 101 names; page-scoped reported
    93; corpus-scoped reports 81.  The last widening is the one that
    matters: `Http` (8 pages) and `ConsoleLogger` (6) are reader-supplied
    CONTEXTS, declared once in `cookbook/http-client.md` and
    `tutorials/context-system.md` and used everywhere else by a convention
    the first of those pages states outright — "`Http` below is a context
    you declare and `provide`, the way every example on this site uses
    it".  Counting them as defects would push the site toward duplicating
    a declaration it deliberately factored out;
  * built-in and primitive names, which are not in `core/` because they are
    in the language.

THE MODIFIER TRAP, measured while calibrating this: a first version matched
`(public )?(fn|type|const)` and reported `serve` as undeclared.  It is
declared — as `async fn serve` — and the pattern simply could not see the
modifier.  The declaration regex therefore admits `public`/`private`,
`async`/`unsafe`/`pure`/`extern`, and `mut`.  A name-existence gate whose
declaration pattern is narrower than the language reports absences that are
its own.
"""
import collections
import io
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DOCS = os.environ.get("VERUM_DOCS_DIR") or os.path.join(
    os.path.dirname(REPO), "website", "docs")
CORE = os.path.join(REPO, "core")

# Every `(name, page)` pair the site carries today.  Lowering it is the work;
# raising it means a doc started naming something the library does not have.
BASELINE = 95

BLOCK = re.compile(r"^```verum(?:[ \t][^\n]*)?\n(.*?)^```", re.M | re.S)
DECL = re.compile(
    r"^\s*(?:public\s+|private\s+)?(?:async\s+|unsafe\s+|pure\s+|extern\s+)*"
    r"(?:fn|type|const|context|static|protocol)\s+(?:mut\s+)?([A-Za-z_]\w*)", re.M)
RECEIVER = re.compile(r"\b([A-Z][A-Za-z0-9]{2,})\s*\.")
MOUNTED = re.compile(r"\bmount\s+[\w.*]*\s*\{([^}]*)\}")
GENERIC = re.compile(r"<([^<>]*)>")

# In the language, not in `core/`.
BUILTIN = {
    "Self", "Int", "Float", "Bool", "Text", "Byte", "Char", "Unit",
    "List", "Map", "Set", "Maybe", "Result", "Heap", "Shared",
    "Some", "None", "Ok", "Err", "Array", "Slice",
    "UInt8", "UInt16", "UInt32", "UInt64", "USize",
    "Int8", "Int16", "Int32", "Int64", "ISize", "Float32", "Float64",
}


def core_declarations():
    out = set()
    for root, _, files in os.walk(CORE):
        for f in files:
            if f.endswith(".vr"):
                text = io.open(os.path.join(root, f), encoding="utf-8",
                               errors="replace").read()
                out |= set(DECL.findall(text))
    return out


def declared_in_blocks(blocks):
    joined = " ".join(blocks)
    out = set(DECL.findall(joined))
    for group in MOUNTED.findall(joined):
        out |= set(re.findall(r"([A-Za-z_]\w*)", group))
    for group in GENERIC.findall(joined):
        out |= set(re.findall(r"\b([A-Z]\w*)\b", group))
    return out


def census():
    declared = core_declarations()
    pages = {}
    for root, _, files in os.walk(DOCS):
        for f in files:
            if not (f.endswith(".md") or f.endswith(".mdx")):
                continue
            path = os.path.join(root, f)
            text = io.open(path, encoding="utf-8", errors="replace").read()
            blocks = BLOCK.findall(text)
            if blocks:
                pages[os.path.relpath(path, DOCS)] = blocks

    # A name the corpus itself introduces is the reader's, not a defect.
    from_docs = set()
    for blocks in pages.values():
        from_docs |= declared_in_blocks(blocks)

    pairs = []
    for rel, blocks in sorted(pages.items()):
        used = set(RECEIVER.findall(" ".join(blocks)))
        for name in sorted(used):
            if name in declared or name in from_docs or name in BUILTIN:
                continue
            pairs.append((name, rel))
    return declared, pairs


def main(argv):
    if not os.path.isdir(DOCS):
        print("check-doc-names-exist: docs not present, skipped")
        return 0
    declared, pairs = census()
    by_name = collections.Counter(n for n, _ in pairs)
    print(f"check-doc-names-exist: {len(pairs)} (name, page) pair(s) name "
          f"something absent from core/'s {len(declared)} declarations "
          f"(baseline {BASELINE})")
    for name, count in by_name.most_common(20):
        pages = sorted({p for n, p in pairs if n == name})
        print(f"    {count:3d} page(s)  {name:22s} e.g. {pages[0]}")
    if "--check" in argv and len(pairs) > BASELINE:
        print(f"check-doc-names-exist: FAIL — {len(pairs)} exceeds {BASELINE}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
