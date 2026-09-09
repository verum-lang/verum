#!/usr/bin/env python3
"""Gate: a method a doc example CALLS should be declared somewhere in core/.

WHY THIS EXISTS, measured 2026-09-08 and not hypothetical. Five pages
showed

    Style.new().fg(color).add_modifier(Modifier.Bold)

    error<E400>: no method named `add_modifier` found for type `Style`

Three method names were invented and so was the constant casing.
`add_modifier` and `sub_modifier` are FIELDS of the `Style` record —
`Style.bold()` is `Style { add_modifier: …union(Modifier.BOLD), ..self }`
— which is why the field name reads like an API and is not one.

WHY NO EXISTING GATE SAW IT. The ladder asks three questions and this is
not among them:

    parse    every ```verum block parses
    names    a CAPITALISED receiver exists in core/   <- TYPES, not methods
    check    `verum check` type-checks the block
    run      only blocks with an `fn main`
    exercised whether anything EXECUTES the method     (a separate gate)

`Style` and `Modifier` are both real, so the names gate is silent; the
call is in a fragment, so nothing runs it. A documented method that
exists NOWHERE passed every rung.

WHAT THIS COUNTS. Method names called as `.name(` inside a ```verum
block, against every `fn name` declared in core/.

TWO PATTERN TRAPS, both of which bit me while writing this:

  * `fn NAME(` misses every GENERIC declaration, which is written
    `fn NAME<T: …>(`. A first version reported `shuffle_vec` and `choice`
    absent on that basis and the claim reached a commit message before
    the control refuted it.
  * asking "is it in the MISSING set" is not asking "is it declared" —
    a name no longer USED is in neither set, and the first control read
    that as declared. The control below asks `declared` directly.

UNDECLARED IS NOT ALWAYS WRONG. A reader's own example type has its own
methods (`find_user`, `from_row`, `build_request`), and those are the
floor — the same shape as the type-name gate's twenty-one reader-owned
types. The number goes down by fixing real ones, and the floor is
whatever survives.
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
CORE = REPO / "core"
BASELINE = 45  # Lowered by FIXING, never by argument.
                #    47 ->  45  NOT a fix, the fourth of its kind, and
                #               the authority is the GRAMMAR rather
                #               than a judgement call: a call inside a
                #               `using [...]` clause is a CONTEXT
                #               TRANSFORM (`verum.ebnf:1035`,
                #               `context_transform = '.' , identifier`),
                #               supplied by whoever declares the
                #               context, and the grammar's own example
                #               is `using [Database.transactional()]`.
                #               Asking core/ whether it declares `fn
                #               transactional` is the wrong index.
                #               Exactly two names live only there:
                #               `.transactional` and `.traced`.
                #               `.readonly` from the same chain is not
                #               among them because core/ happens to
                #               declare that name — the gate could only
                #               have been right here by accident.
                #    49 ->  47  the same marker in the site's PLURAL
                #               spelling: `.streaming` under
                #               h3-server.md's ":::warning Every
                #               builder below is one of the four that
                #               do not exist", and `.throttle_filter`
                #               under async.md's "Three combinators on
                #               that list do not exist". The regex knew
                #               only the singular. Nine headers newly
                #               match, three retrospective ones are
                #               excluded by tense — see UNSHIPPED.
                #    57 ->  49  NOT a fix, and the third of its kind: eight
                #               names sit inside a section the page MARKS as
                #               unshipped, and this gate was painting the
                #               pages that said so. `testing-tui.md` opens
                #               its third section with ":::caution Not
                #               shipped / None of this section exists.
                #               `VirtualTerminal`, `ManualRuntime`,
                #               `type_keys`, `expect_row`, `run_one_frame`
                #               and `block_on_with_fake_clock` are absent
                #               from `core/` — measured, not guessed" —
                #               this gate's own finding, written down before
                #               the gate existed. Its sibling
                #               `check_doc_names_exist.py` has honoured
                #               these markers since 1eb0a71c4; this one did
                #               not, so one page was scored twice under two
                #               rules. Verified before landing: all eight
                #               calls fall between an unshipped admonition
                #               and the next `##` heading (scheduler.md:52
                #               after :32, repl.md:278-279 after :265,
                #               dns.md:236 after :217, and five in
                #               testing-tui.md). A false POSITIVE is the
                #               expensive kind here: it teaches readers to
                #               ignore the number.
                #    58 ->  57  measured, not acted on: one name left the
                #               census on its own between two runs.
                #   118 -> 114  the Postgres/MySQL config builders
                #               (with_host, with_port, with_user,
                #                with_database, with_password_from_env)
                #   114 -> 111  H3Response.with_body, OpenOptions.open_async,
                #               ServerOptions.with_cert_pem/with_key_pem
                #    59 ->  58  Table.widths on the widget reference —
                #               the same constructor-argument error the
                #               catalogue carried, one page over.
                #    61 ->  59  TokenStream.as_text_literal (two meta
                #               pages) and H3Server.stats' three readers.
                #    72 ->  61  NOT a fix, the second of its kind: eleven
                #               names are DECLARED by the page that calls
                #               them — `fn from_a` next to `b.from_b()`,
                #               the FFI page's own `sodium_init` extern
                #               block, the UrlLike macro's synthesised
                #               `to_url_query`. A page teaching with an
                #               example type is the floor, not debt, and
                #               mixing the two makes the number mean two
                #               things at once. They are still printed,
                #               under `floor:`, so a real API that starts
                #               being declared on the page instead of in
                #               core/ cannot hide there silently.
                #    83 ->  72  eight pages: the shell command DSLs (sum
                #               types, not builders, and they do not run),
                #               List.group_by vs into_group_map*,
                #               recv_with_timeout and `buf.as_unsafe()`,
                #               Text.url_encode/url_decode, byte_stream/
                #               utf8_chunks/after_at, and the QUIC/H3 TLS
                #               half — parse_cert_chain_pem and FileSigner
                #               are declared nowhere and CertSigner has no
                #               implementation at all.
                #    93 ->  83  NOT a fix: the census stopped counting
                #               names it found in COMMENTS inside the
                #               blocks. Eleven of them, and every one was
                #               a correction naming the wrong name so a
                #               reader would recognise it. Counting those
                #               makes a corrected page look like a
                #               regression — it pushed this gate one
                #               ABOVE its own new baseline the moment the
                #               five pages below landed.
                #   108 ->  93  five pages that taught an API their
                #               library does not have: the cli builder
                #               (App.new/.about/FlagSpec.new/.takes_value),
                #               the server request (req.uri/.read_body),
                #               Response.with_headers/with_body/
                #               Headers.new_with/get_first, Text.to_bytes,
                #               Semaphore.acquire_owned, HttpClient.builder
                #               and its pool/tls/user_agent/max_redirects,
                #               MockResolver.with_a/with_aaaa/with_txt.
                #   111 -> 108  Table.widths, DialogButton.primary,
                #               Menu.orientation — all three on the term
                #               pages, all three refused by `verum check`
                #               once put in a runnable position. Router
                #               and CommandPalette went with them; the
                #               page had a `.navigate(…)` builder on a
                #               type that is declared nowhere.

BLOCK = re.compile(r"```verum\n(.*?)```", re.S)
CALL = re.compile(r"\.([a-z_][a-z0-9_]*)\s*\(")

# A call inside a `using [...]` clause is a CONTEXT TRANSFORM, not a method.
# `grammar/verum.ebnf:1035` is the authority and settles it without a vote:
#
#     context_transform = '.' , identifier , [ '(' , [ transform_args ] , ')' ] ;
#
# — a bare `identifier`, supplied by whoever declares the context, and the
# grammar's own worked example two lines above is
# `using [Database.transactional(), Cache.scoped()]`.  Asking `core/` whether
# it declares `fn transactional` is asking the wrong index: the site was
# quoting the grammar and being scored against the library.
#
# Measured 2026-09-09: exactly two names appear ONLY in this position —
# `.transactional` (language/syntax.md, reference/grammar-ebnf.md) and
# `.traced` (language/context-system.md, `Store.readonly().traced()`).
# `.readonly` from that same chain is NOT among them, because a method of
# that name does exist in `core/` — which is the shape of the false positive
# this removes: the gate could only ever be right by accident here.
USING_CLAUSE = re.compile(r"\busing\s*\[[^\]]*\]")


def calls_outside_using(block: str) -> list[str]:
    """Method names called in `block`, minus the context-transform position."""
    spans = [m.span() for m in USING_CLAUSE.finditer(block)]
    return [m.group(1) for m in re.finditer(r"\.([a-z_][a-z0-9_]*)\s*\(", block)
            if not any(s <= m.start() < e for s, e in spans)]

# A section the page MARKS as unshipped is honest documentation of a plan,
# not rot.  Carried over verbatim from `check_doc_names_exist.py`, which has
# honoured these markers since 1eb0a71c4 — until now the two gates scored the
# same page under two different rules, and the page that wrote down this
# gate's own finding was the one being painted for it.  The phrasings are
# collected from the admonition headers actually in use, not guessed — see
# the longer note in that sibling, which records the 2026-09-09 widening
# (the plural "do not exist", the counting forms, and the tense exclusion
# that keeps retrospective notes countable).  The two regexes are kept
# IDENTICAL on purpose: two gates disagreeing about what "unshipped"
# means is how one page came to be scored under two rules to begin with.
UNSHIPPED = re.compile(
    r"^:::[a-z]+(?![^\n]*(?:used to|were not real|was stale))[^\n]*"
    r"(?:not shipped|not implemented|not available|"
    r"do(?:es)? not exist|does not compile|not in the standard library|"
    r"cannot be evaluated|cannot be written|none of these|not yet|"
    r"no snapshot|no linter|planned|describes a design)[^\n]*$", re.I | re.M)
# The marker scopes to the SECTION, not to the admonition: the illustrative
# blocks follow the `:::` close, so cutting at it would leave every name
# behind.  Measured on the eight names this removes — each falls between an
# unshipped admonition and the next `##`.
NEXT_HEADING = re.compile(r"^##\s", re.M)


def drop_unshipped(text: str) -> str:
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

# A COMMENT inside a ```verum block is prose, and prose in these blocks
# is usually ABOUT the wrong name — "the builder is `.body(..)`, not
# `.with_body(..)`". Counting it puts the corrected name back in the
# census the correction was written to empty, which happened on
# 2026-09-08 and pushed the count one ABOVE its own new baseline.
#
# Stripping is deliberately conservative in the direction that keeps
# calls: a `//` tail is removed only when no quote opens earlier on the
# line, so `let u = "http://x"; a.b()` keeps `b`.
def strip_comments(code: str) -> str:
    out = []
    for line in code.split("\n"):
        stripped = line.lstrip()
        if stripped.startswith("//"):
            continue
        i = line.find("//")
        if i >= 0 and '"' not in line[:i]:
            line = line[:i]
        out.append(line)
    return "\n".join(out)
# `[<(]` — a generic declaration has `<` where a plain one has `(`.
DECL = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)\s*[<(]")

# Measured on 2026-09-08. Absent ones were removed from the docs the same
# day; they stay here because a census with no known answer cannot be
# trusted, and this one caught two of my own mistakes before it ran.
CONTROL = {
    "add_modifier": False, "remove_modifier": False,   # fields, never functions
    "shuffle_vec": True, "choice": True,               # generic free functions
    "uniform_01": True, "bold": True,
}


def declared_names() -> set[str]:
    if not CORE.is_dir():
        return set()
    return set(DECL.findall(
        "\n".join(f.read_text(errors="ignore") for f in CORE.rglob("*.vr"))))


def self_test() -> int:
    bad = 0
    if DECL.findall("public fn shuffle_vec<T: Copy>(key: RandomKey)") != ["shuffle_vec"]:
        print("self-test: the declaration pattern misses a GENERIC fn"); bad += 1
    if DECL.findall("public fn bold(self) -> Style {") != ["bold"]:
        print("self-test: the declaration pattern misses a plain fn"); bad += 1
    if DECL.findall("    add_modifier: Modifier,") != []:
        print("self-test: a FIELD must not read as a declaration"); bad += 1
    if CALL.findall("Style.new().fg(c).add_modifier(M.BOLD)") != ["new", "fg", "add_modifier"]:
        print("self-test: the call pattern misses a chained call"); bad += 1
    if CALL.findall(strip_comments("    // the builder is `.body(..)`, not `.with_body(..)`")) != []:
        print("self-test: a full-line COMMENT still reads as a call"); bad += 1
    if CALL.findall(strip_comments("x.foo()   // and not .bar()")) != ["foo"]:
        print("self-test: a trailing comment still reads as a call"); bad += 1
    if CALL.findall(strip_comments('let u = "http://x"; a.b()')) != ["b"]:
        print("self-test: a URL in a string ate the rest of the line"); bad += 1
    # The floor rule reads DECL over the same blocks the calls come from.
    if DECL.findall("fn from_a() { b.from_b(); }") != ["from_a"]:
        print("self-test: a page-local declaration is not seen"); bad += 1
    # A context transform is not a method call (grammar/verum.ebnf:1035).
    if calls_outside_using("fn tx() using [Database.transactional()] {}") != []:
        print("self-test: a context transform still reads as a method call"); bad += 1
    if calls_outside_using("fn a() using [Store.readonly().traced()] { x.real() }") != ["real"]:
        print("self-test: the using narrowing swallowed a call OUTSIDE the clause"); bad += 1
    if calls_outside_using("y.keep()") != ["keep"]:
        print("self-test: a plain call was lost with no using clause present"); bad += 1
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-methods-declared: {DOCS} not present — the docs live "
              "in a sibling checkout; UNMEASURED rather than passing on an "
              "absent input.")
        return 0

    declared = declared_names()
    if not declared:
        print("check-doc-methods-declared: core/ is not present — UNMEASURED.")
        return 0

    # THE CONTROL, asked as "is it declared", never as "is it missing":
    # a name that is no longer used is in neither set, and reading that
    # as declared is how the first version of this passed while wrong.
    wrong = [n for n, want in CONTROL.items() if (n in declared) != want]
    if wrong:
        print("control FAILED for: " + ", ".join(wrong))
        print("  A census whose known answers are wrong says nothing about "
              "its unknowns. Fix the patterns before reading any count.")
        return 1
    print(f"control: {len(CONTROL)}/{len(CONTROL)} known answers correct")

    pages: dict[str, set[str]] = {}
    # A page that DECLARES the helper it calls is not documenting an
    # absent API — it is teaching with an example type, and that is the
    # floor. Tracked separately so the debt number means one thing.
    self_declared: dict[str, set[str]] = {}
    pages_scanned = 0
    for f in sorted(list(DOCS.rglob("*.md")) + list(DOCS.rglob("*.mdx"))):
        pages_scanned += 1
        text = drop_unshipped(f.read_text(errors="ignore"))
        blocks = [strip_comments(m.group(1)) for m in BLOCK.finditer(text)]
        page_decls: set[str] = set()
        for b in blocks:
            page_decls |= set(DECL.findall(b))
        rel = f.relative_to(DOCS).as_posix()
        for b in blocks:
            for name in calls_outside_using(b):
                if name not in declared:
                    pages.setdefault(name, set()).add(rel)
                    if name in page_decls:
                        self_declared.setdefault(name, set()).add(rel)

    floor = sorted(n for n in pages
                   if n in self_declared and pages[n] <= self_declared[n])
    for n in floor:
        del pages[n]

    # A FLOOR, for the reason this file's neighbours carry one: a gate
    # whose input went missing prints the same clean line as a clean
    # corpus. `--min-pages` turns "I found nothing" into a failure.
    floor_pages = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-pages" and i + 1 < len(sys.argv):
            floor_pages = int(sys.argv[i + 1])
    if pages_scanned < floor_pages:
        print(f"check-doc-methods-declared: only {pages_scanned} page(s) "
              f"scanned under {DOCS}, expected at least {floor_pages} — the "
              "corpus is missing or the glob stopped matching. A census of "
              "nothing is not a clean census.")
        return 1

    total = len(pages)
    print(f"check-doc-methods-declared: {total} method name(s) called by a doc "
          f"example are declared nowhere in core/ (baseline {BASELINE})")
    print(f"  floor: {len(floor)} more are declared by the page that calls "
          f"them — reader-owned example helpers, not debt "
          f"({', '.join(floor[:6])}{', …' if len(floor) > 6 else ''})")
    for name, ps in sorted(pages.items(), key=lambda kv: -len(kv[1]))[:10]:
        print(f"  {len(ps):>2} page(s)  .{name:<22} e.g. {sorted(ps)[0]}")

    if total > BASELINE:
        print(f"  ABOVE BASELINE by {total - BASELINE}. A method that exists "
              "nowhere is one a reader cannot call.")
        return 1
    if total < BASELINE:
        print(f"  BELOW baseline by {BASELINE - total} — lower it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
