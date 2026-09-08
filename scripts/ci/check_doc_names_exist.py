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
#
# 22 -> 21 on 2026-09-08: measured 21, and a ratchet standing one above its
# own count is a ratchet that admits a twenty-second fictional name without
# saying so — which is exactly the entry route A97 documents for all five
# defects it found. The composition of the floor moved rather than shrank:
# covering `src/pages/index.tsx` added `UartRegisters`, and one of A97's
# original survivors is gone.
BASELINE = 18  # 21 -> 18 on 2026-09-08: `MockResolver` left
               # cookbook/dns.md (there is no mock resolver and
               # `Resolver` is not a context), and `FileSigner` left
               # cookbook/quic-server.md and tutorials/h3-service.md
               # (no `sign.vr` module, and `CertSigner` has no
               # implementation anywhere in core/).
               #
               # All 18 that remain are reader-owned example types —
               # FakeDatabase, MyError, MemoryFs, AiClient,
               # RecordingLogger, UartRegisters and the like. That is
               # the floor A97 describes, and it does not come down by
               # editing pages: it comes down only if an example stops
               # inventing a type, which usually makes the example
               # worse.

BLOCK = re.compile(r"^```verum(?:[ \t][^\n]*)?\n(.*?)^```", re.M | re.S)
DECL = re.compile(
    # `pub` is Verum, not a Rust leak: `visibility = ( 'public' | 'pub' )`
    # at `grammar/verum.ebnf:497`.  Reading only `public` made the gate
    # report `SerializeBuf` as undeclared while `tutorials/protocols.md`
    # declares it two lines above the use, and nearly bought a 173-line
    # rewrite of 33 pages that would have changed nothing.
    r"^\s*(?:public\s+|pub(?:\(\w+\))?\s+|private\s+)?"
    r"(?:async\s+|unsafe\s+|pure\s+|extern\s+)*"
    r"(?:fn|type|const|context|static|protocol)\s+(?:mut\s+)?([A-Za-z_]\w*)", re.M)
RECEIVER = re.compile(r"\b([A-Z][A-Za-z0-9]{2,})\s*\.")
# A comment inside a block is PROSE, and prose ends sentences with a period.
# Measured: every `KiB.` on the site is "32 KiB." or "2.5-4.9 KiB." in a
# comment or a bullet, and reading them as a receiver invented a defect.
LINE_COMMENT = re.compile(r"//[^\n]*")
BLOCK_COMMENT = re.compile(r"/\*.*?\*/", re.S)
# A STRING is data, not code.  `db.query("SELECT ...")` made the gate report
# `SELECT` as an undeclared receiver: the regex saw the identifier, a space
# and the first dot of the ellipsis.  Tagged literals (`sql#"""…"""`,
# `rx#"…"`) carry the same trap and are stripped with them.
TAGGED_STRING = re.compile(r"[a-z_]*#\"\"\".*?\"\"\"|[a-z_]*#\"[^\"]*\"", re.S)
PLAIN_STRING = re.compile(r"\"[^\"\n]*\"")
# `Base..Base` is a range, not a receiver.
RANGE_OP = re.compile(r"\.\.")
# A section the page MARKS as unshipped is honest documentation of a plan,
# not rot.  `stdlib/term/guides/testing-tui.md` opens its third section with
# ":::caution Not shipped / None of this section exists. `VirtualTerminal`,
# `ManualRuntime`, … are absent from `core/` — measured, not guessed", which
# is this gate's own finding, written down before the gate existed.  Counting
# it would punish the page for being explicit.  14 such admonitions exist.
# The site marks unshipped work in a dozen phrasings, all of them explicit.
# Collected from the admonition headers actually in use rather than guessed:
# "Not shipped", "does not exist", "Not available", "not in the standard
# library", "does not compile", "cannot be evaluated today", "No snapshot
# helper", "not available", "describes a design".
UNSHIPPED = re.compile(
    r"^:::[a-z]+[^\n]*(?:not shipped|not implemented|not available|"
    r"does not exist|does not compile|not in the standard library|"
    r"cannot be evaluated|not yet|no snapshot|no linter|planned|"
    r"describes a design)[^\n]*$", re.I | re.M)
# The marker scopes to the SECTION, not to the admonition.  Measured on
# `testing-tui.md`: its ":::caution Not shipped / None of this section
# exists" closes after the prose and the illustrative blocks follow it, so
# cutting at the `:::` removed 475 characters and left every name behind.
NEXT_HEADING = re.compile(r"^##\s", re.M)


def drop_unshipped(text):
    """Drop each marked section, from its marker to the next heading."""
    out, pos = [], 0
    for m in UNSHIPPED.finditer(text):
        if m.start() < pos:
            continue
        nxt = NEXT_HEADING.search(text, m.end())
        stop = nxt.start() if nxt else len(text)
        out.append(text[pos:m.start()])
        pos = stop
    out.append(text[pos:])
    return "".join(out)


def code_only(text):
    text = BLOCK_COMMENT.sub("", text)
    text = LINE_COMMENT.sub("", text)
    text = TAGGED_STRING.sub('""', text)
    text = PLAIN_STRING.sub('""', text)
    return RANGE_OP.sub(" ", text)
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


VARIANTS = re.compile(r"\btype\s+\w+(?:<[^>]*>)?\s+is\s+([^;{]*)")


def core_declarations():
    """Everything `core/` declares — INCLUDING sum-type variants.

    The variant rule was written for doc blocks first and not applied here,
    and the asymmetry invented a defect: `NetworkError` is declared at
    `core/net/dns.vr:175` as `| NetworkError(Text)`, and a scan that only
    looked for `type`/`fn`/`const` reported the library's own name as
    missing from the library.
    """
    out = set()
    for root, _, files in os.walk(CORE):
        for f in files:
            if f.endswith(".vr"):
                text = io.open(os.path.join(root, f), encoding="utf-8",
                               errors="replace").read()
                out |= set(DECL.findall(text))
                for body in VARIANTS.findall(text):
                    out |= set(re.findall(r"\b([A-Z]\w*)", body))
    return out


def declared_in_blocks(blocks):
    joined = " ".join(blocks)
    out = set(DECL.findall(joined))
    # `type S1 is Base | Loop();` declares `Base` and `Loop` as surely as
    # the type itself — a sum type's variants are names the page introduces.
    for body in VARIANTS.findall(joined):
        out |= set(re.findall(r"\b([A-Z]\w*)", body))
    for group in MOUNTED.findall(joined):
        out |= set(re.findall(r"([A-Za-z_]\w*)", group))
    for group in GENERIC.findall(joined):
        out |= set(re.findall(r"\b([A-Z]\w*)\b", group))
    return out


HOMEPAGE = os.path.join(os.path.dirname(DOCS), "src", "pages", "index.tsx")


def homepage_samples():
    """The marketing homepage's Verum samples, via the gate that owns them.

    The owner asks for that page to be the site's best, and it sat outside
    this census: it is `.tsx`, not `.md`.  Reusing
    `check_homepage_examples.blocks` rather than grabbing template literals
    matters — a crude grab returned `LANGUAGE` and `React` as undeclared
    Verum receivers, which is TypeScript.
    """
    if not os.path.isfile(HOMEPAGE):
        return []
    try:
        import importlib.util
        spec = importlib.util.spec_from_file_location(
            "homepage_gate",
            os.path.join(os.path.dirname(os.path.abspath(__file__)),
                         "check_homepage_examples.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
    except Exception:
        return []
    src = io.open(HOMEPAGE, encoding="utf-8", errors="replace").read()
    return [body for _, body in mod.blocks(src)]


def census():
    declared = core_declarations()
    pages = {}
    for root, _, files in os.walk(DOCS):
        for f in files:
            if not (f.endswith(".md") or f.endswith(".mdx")):
                continue
            path = os.path.join(root, f)
            text = drop_unshipped(
                io.open(path, encoding="utf-8", errors="replace").read())
            blocks = BLOCK.findall(text)
            if blocks:
                pages[os.path.relpath(path, DOCS)] = blocks
    home = homepage_samples()
    if home:
        pages["src/pages/index.tsx"] = home

    # A name the corpus itself introduces is the reader's, not a defect.
    from_docs = set()
    for blocks in pages.values():
        from_docs |= declared_in_blocks(blocks)

    pairs = []
    for rel, blocks in sorted(pages.items()):
        used = set(RECEIVER.findall(code_only(" ".join(blocks))))
        for name in sorted(used):
            if name in declared or name in from_docs or name in BUILTIN:
                continue
            pairs.append((name, rel))
    return declared, pairs


# A SKIP IS A VERDICT ABOUT NOTHING. In CI these gates run with an
# explicit flag, and a missing input there is not "nothing to check" —
# it is the checkout step having failed, the docs directory having
# moved, or the binary not having been built. Reporting OK in that
# state is the shape `check_gate_verdict_carries_a_quantity` exists to
# prevent, one level up: not a verdict without a number, but a verdict
# without a subject. Six gates could do it; measured 2026-09-06.
#
# Locally, with no flag, skipping stays correct — a source-only clone
# has no website beside it and should not fail for that.
def _skip_or_fail(argv, what: str) -> int:
    """Report a missing input, and decide whether that is fatal.

    Returns the exit code AND says which of the two happened, because a
    line reading "skipped" beside a non-zero exit is a verdict that
    misdescribes itself.
    """
    import os as _os
    import sys as _sys

    fatal = any(a in argv for a in ("--check", "--ratchet")) or bool(
        _os.environ.get("CI")
    )
    if fatal:
        print(
            f"{what} — REFUSING to report OK: this run was asked to CHECK, so a "
            f"missing input is a failed checkout or an unbuilt binary, not "
            f"'nothing to do'.",
            file=_sys.stderr,
        )
        return 2
    print(f"{what}, skipped (local run; pass --check to make this fatal)")
    return 0


def main(argv):
    if not os.path.isdir(DOCS):
        return _skip_or_fail(argv, "check-doc-names-exist: docs not present")
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
