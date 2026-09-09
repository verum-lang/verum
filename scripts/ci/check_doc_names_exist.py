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
# THE COUNT BECAME A ROSTER, 2026-09-09 (T1330).  A bare `BASELINE = 16`
# said how many fictional names the site was allowed to carry and never
# WHICH, so the shape it could not report is the SWAP: fix one page, let
# another invent a name, and `len(pairs)` is unchanged and the gate passes.
# Measured the same day on a sibling gate of identical construction
# (`check_platform_call_parity.py`): population held at one, membership
# replaced, and the count ratchet printed `[ok] … none new` over a tree
# that had just acquired the defect.
#
# This gate is where a swap is MOST likely, because its population lives in
# a different repository than the ratchet: the site is edited by commits
# that never touch this file, so a fix and a regression can land in the
# same week without either being visible here as a number.
#
# The key is `(name, page)`, which is what the census already produced.
#
# HOW THE POPULATION GOT HERE, kept because each removal was a real fix
# and the next reader should not re-litigate them:
#
#   22 -> 21  2026-09-08.  Measured 21 while the ratchet said 22, and a
#             ratchet standing one above its own count admits a
#             twenty-second fictional name without saying so — the entry
#             route A97 documents for all five defects it found.  The
#             composition moved rather than shrank: covering
#             `src/pages/index.tsx` added `UartRegisters`.
#   21 -> 18  2026-09-08.  `MockResolver` left cookbook/dns.md (there is
#             no mock resolver and `Resolver` is not a context);
#             `FileSigner` left cookbook/quic-server.md and
#             tutorials/h3-service.md (no `sign.vr`, and `CertSigner` has
#             no implementation anywhere in core/).
#   18 -> 16  2026-09-09.  `OpenFlag` left architecture-types/
#             orthogonality.md — the direct-write example named
#             `sys.io.open` / `OpenFlag.WriteOnly` and neither exists (the
#             flags are module-level `O_WRONLY` / `O_CREAT`); the block now
#             calls `core.io.file.write_bytes`, verified by RUNNING it.
#             `Colour` left architecture/overview.md — one letter from the
#             stdlib's real `Color`, in a fragment of two bare `match`es;
#             the block now declares its own `Shade` and is a program.
#
# ALL SIXTEEN ARE READER-OWNED EXAMPLE TYPES — FakeDatabase, MyError,
# MemoryFs, AiClient, RecordingLogger, UartRegisters and the like.  That is
# the floor A97 describes, and it does not come down by editing pages: it
# comes down only if an example stops inventing a type, which usually makes
# the example worse.  So the roster is not a to-do list; it is a statement
# that these sixteen are deliberate and a seventeenth would not be.
KNOWN = {
    ("AiClient", "cookbook/resilience.md"),
    ("Analytics", "language/context-system.md"),
    ("FakeDatabase", "cookbook/testing-recipes.md"),
    ("FakeDatabase", "guides/testing-best-practices.md"),
    ("FraudClient", "architecture-types/orthogonality.md"),
    ("HttpServer", "language/context-system.md"),
    ("MemoryFs", "cookbook/file-io.md"),
    ("MyAuditLog", "stdlib/database.md"),
    ("MyError", "stdlib/base.md"),
    ("RecordingLogger", "cookbook/testing-recipes.md"),
    ("ScopeInfo", "verification/tactic-dsl.md"),
    ("SessionInvariant", "reference/grammar-ebnf.md"),
    ("Sodium", "language/ffi.md"),
    ("SqlParser", "language/meta/literal-handlers.md"),
    ("SqlParser", "language/meta/token-api.md"),
    ("UartRegisters", "src/pages/index.tsx"),
}


def compare(found, roster):
    """Split what the site has against what the roster claims.

    Separated from the census so a control can drive it without a docs
    tree — the half a count ratchet has and never tests."""
    return sorted(found - roster), sorted(roster - found)


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
#
# 2026-09-09: the list was collected from ONE spelling of the same marker.
# Dumping every `^:::` header on the site — 60 distinct — showed nine more
# that say the same thing in the PLURAL or by counting: "Every builder below
# is one of the four that do not exist", "These two mounts name modules that
# do not exist", "None of these four commands exists", "Twenty-four of these
# do not exist yet", "The TLS half of this tutorial cannot be written today".
# Hence `do(?:es)? not exist`, `none of these`, `cannot be written`.
#
# AND ONE EXCLUSION, which the same dump made necessary. Three headers are
# RETROSPECTIVES — "Three names this page used to list do not exist", "Four
# names on this page were not real", "The method IS checked — this caution
# was stale". Those sections are CORRECT now, and dropping them would blind
# the gate to a future regression on exactly the pages that already had one.
# They are excluded by their own tense. Measured today: excluding them hides
# nothing, because none of the three sections contributes a name to either
# census — so the exclusion costs zero and buys the future case.
UNSHIPPED = re.compile(
    r"^:::[a-z]+(?![^\n]*(?:used to|were not real|was stale))[^\n]*"
    r"(?:not shipped|not implemented|not available|"
    r"do(?:es)? not exist|does not compile|not in the standard library|"
    r"cannot be evaluated|cannot be written|none of these|not yet|"
    r"no snapshot|no linter|planned|describes a design)[^\n]*$", re.I | re.M)
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
    # A BROKEN OWNER GATE IS NOT AN EMPTY HOMEPAGE.  This used to be
    # `except Exception: return []`, which reports the same thing for "the
    # page has no Verum samples" and "the module that reads it could not be
    # loaded" — so renaming or breaking `check_homepage_examples.py` would
    # have silently dropped `src/pages/index.tsx` out of this census with no
    # line of output.  Observed for real: a copy of this gate run from a
    # directory without its sibling reported the homepage's one known pair
    # as GONE, and under the old count ratchet that would have read as
    # 15 <= 16 and passed.
    owner = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                         "check_homepage_examples.py")
    if not os.path.isfile(owner):
        raise SystemExit(
            f"check-doc-names-exist: {owner} is missing — the homepage census "
            "reads its blocks through that gate, and refusing is the only "
            "honest answer. Reporting an empty homepage would look like a "
            "clean page."
        )
    import importlib.util
    spec = importlib.util.spec_from_file_location("homepage_gate", owner)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
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


def self_test():
    """THE SWAP, which is the shape the count ratchet could not report and
    the reason this gate carries a roster.  Population size is 1 in both
    polarities; the membership differs, and a count is satisfied by both."""
    before = {("MyError", "stdlib/base.md")}
    after = {("MyError", "stdlib/database.md")}
    appeared, disappeared = compare(after, before)
    if not appeared or not disappeared:
        print("self-test: a swap of equal size reported nothing — the roster "
              "comparison has degenerated back into a count")
        return 1
    if compare(before, before) != ([], []):
        print("self-test: an unchanged population reported a difference")
        return 1
    print("[ok] self-test: a same-size swap is reported")
    return 0


def main(argv):
    if "--self-test" in argv:
        return self_test()
    if not os.path.isdir(DOCS):
        return _skip_or_fail(argv, "check-doc-names-exist: docs not present")
    declared, pairs = census()
    found = set(pairs)
    appeared, disappeared = compare(found, KNOWN)
    print(f"check-doc-names-exist: {len(pairs)} (name, page) pair(s) name "
          f"something absent from core/'s {len(declared)} declarations "
          f"({len(KNOWN)} on the roster)")
    for name, page in sorted(found):
        mark = "NEW " if (name, page) in set(appeared) else "    "
        print(f"    {mark}{name:22s} {page}")

    if "--check" not in argv:
        return 0
    if appeared:
        print("check-doc-names-exist: FAIL — the pair(s) marked NEW are not on "
              "the roster in this file.\n"
              "A doc example names something core/ does not declare. Fix the\n"
              "page, or add the pair to KNOWN with the reason it is deliberate\n"
              "(a reader-owned example type usually is).")
        return 1
    if disappeared:
        print("check-doc-names-exist: FAIL — the roster claims pair(s) the site "
              "no longer has —")
        for name, page in disappeared:
            print(f"    {name:22s} {page}")
        print("Remove them from KNOWN in this file; the population shrank and\n"
              "the roster has to say so by name, not by a smaller number.")
        return 1
    print(f"check-doc-names-exist: {len(pairs)} known pair(s), roster exact")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
