#!/usr/bin/env python3
"""Gate: a method the stdlib reference documents should be EXECUTED somewhere.

WHY THIS EXISTS, measured 2026-09-07 and not hypothetical. The website
called the `Map` entry API "the canonical way to do insert-or-update".
Copied verbatim into a file and run, it dies:

    error: VBC execution error: Null pointer dereference at Map.entry

It had passed every doc gate, because the ladder stops one rung short:

    parse   every ```verum block parses
    names   a name a doc example uses exists in core/
    check   `verum check` — a TYPE check
    run     ONLY blocks containing `fn main`

The entry API is documented in six fragments and zero runnable
programmes, so nothing ever ran it. Across the site that is 2751 of 2817
blocks — but BLOCKS are the wrong denominator: a method documented in a
fragment may well be exercised by core-tests. The denominator that means
something is DOCUMENTED METHODS.

WHAT THIS COUNTS. For each `stdlib/*.md` page, the method names its
```verum blocks call, and whether any corpus that ACTUALLY RUNS calls
them too.

WHAT COUNTS AS RUNNING, and the distinction is the whole gate. A first
version of this counted every `vcs/specs/**` file and reported 10%
unexercised. That number was wrong, and its own control said so:
`.entry` came back "has evidence" — from
`vcs/specs/core/collections/map_extended_test.vr`, whose directive is

    // @test: typecheck-pass

A spec that only type-checks is not evidence that anything ran. Counting
it made a known-broken method look covered. Only these are executed:

    vcs/specs   @test: run  or  run-interpreter   (not typecheck-pass,
                                                   not parse-pass)
    core-tests  every file, minus the BODY of each `@ignore`d test
    docs/by-example                               (the 22 showcase programmes)

...and, everywhere, minus COMMENTS. Both of those clauses were wrong
until 2026-09-12 and wrong in OPPOSITE directions, which is why the
count looked stable while neither half of it was right:

  * core-tests was filtered by FILE — any file containing the string
    `@ignore` was dropped whole. That discarded 183 of 1263 files. 69 of
    the 183 have no `@ignore` ATTRIBUTE at all (the word appears in a
    comment saying why something is hard); `core-tests/base/data` has
    161 tests and one ignored. Per-test granularity returns 38 rows'
    worth of real evidence: 279 -> 241.
  * comments counted as evidence. A page's method NAMED in prose —
    `// Histogram.observe(5.0) answered 0` sits four lines above the
    only `observe` call, which is `@ignore`d — read as a call. Removing
    them exposes 20 rows that nothing ever ran: 241 -> 261.

The two corrections are independent and both are pinned by `--self-test`,
which names each failure in the words above when either is reverted.

...and minus files that DO NOT COMPILE, listed in
`doc_methods_dead_files.txt`. A dead file is indistinguishable from a
live one by text: it has `@test` functions, carries no `@ignore`, and
its calls sit in the corpus. `core-tests/base/iterator/unit_test.vr` is
215 tests and 215 failures — sixteen typecheck errors around
`peekable()`/`peek()` unifying `Item<ISize>` against `Int` — and it
alone was supplying false evidence for 14 documented methods. The list
is DATA from a real run; regenerate it by running the suite, never by
reasoning about a file.

WHAT COUNTS AS A CALL, and a module-qualified free function is not a
method. The pages write `rfc3339.format_utc(t.unix_seconds, t.nanos)`
and `jcs.canonicalize_str(&raw)`, which `CALL` files as methods, while
`core/` declares them `public fn` and a test calls them by their bare
mounted name — so evidence written `format_utc(0, 0)` was invisible and
six methods sat on the roster with green coverage in core-tests. The
RECEIVER decides and the page says which is which: a receiver naming a
module the page mounts is a module. A blanket "a bare call counts" was
measured first and rejected — `entry(` occurs bare in the corpus, and
`Map.entry` is one of the three known-broken methods this gate proves
itself with, so the blanket rule silences its own controls. The
`--self-test` pins that, in the words above.

KNOWN WEAKNESS, stated rather than hidden. The match is on the METHOD
NAME, not on the receiver's type: `.get_mut(` in a List test counts as
evidence for `Map.get_mut`, which is measurably broken. The gate's own
control reports this — three of four known-broken Map methods come back
unexercised and `get_mut` does not. A per-type version needs the
receiver's static type, which needs the checker; this is the cheap
version that runs with no build.

UNEXERCISED IS NOT BROKEN. It means nobody would notice if it broke.
`Map.entry` is what that looks like when it happens.

TWO HALVES, and the second exists because the first is swap-blind
(T1330). The COUNT answers "did the population grow"; it cannot answer
"did its membership change", and 292 holds just as well when one page
gains coverage and another loses it. `doc_methods_unexercised.txt` names
the population, one `page<TAB>method` per line, and is compared ONLY when
the count agrees with the baseline — that is precisely the state in which
a swap is invisible. A missing roster is a REFUSAL (rc=2), not a pass.
"""
from __future__ import annotations
import contextlib
import io
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# `VERUM_DOCS_DIR` names the whole website `docs/`, which is what CI
# checks out; this gate reads its `stdlib/` subtree.
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = Path(os.environ.get("VERUM_STDLIB_DOCS")
            or (Path(_DOCS_ROOT) / "stdlib" if _DOCS_ROOT
                else REPO.parent / "website" / "docs" / "stdlib"))
RUN_DIRECTIVES = {"run", "run-interpreter"}
BASELINE = 255  # Lowered by COVERAGE, never by argument — the only way
                # this number is meant to move.
                #   254 -> 255  NOT a regression and not coverage: a
                #               THIRD instrument correction, and the
                #               only one that moves the count UP.
                #               Two changes landed together and pull
                #               opposite ways, so they are stated
                #               separately:
                #                 * the module-receiver rule credits a
                #                   free function called by its bare
                #                   mounted name — six methods had
                #                   green coverage in core-tests that
                #                   the dot-only match could not see
                #                   (254 -> 241, with sixteen new
                #                   tests landing in the same hour);
                #                 * excluding ONE file that does not
                #                   compile takes back 14 (241 -> 255).
                #                   `core-tests/base/iterator/unit_test.vr`
                #                   is 215 tests and 215 failures, and
                #                   every method it named looked
                #                   covered.
                #               The dead-file list is seeded from that
                #               one measured file; a full-suite run
                #               will name the rest, and each addition
                #               moves this number again, upward, on
                #               purpose.
                #   261 -> 254  NOT coverage: the DENOMINATOR was wrong
                #               the same way the corpus was. A comment
                #               inside a ```verum block is PROSE —
                #               `check_doc_names_exist` has always held
                #               that, and the two gates disagreeing
                #               about what a block says WAS the defect.
                #               17 of 1402 documented methods lived only
                #               in a comment; five of those are the page
                #               DENYING the method ("there is no
                #               `.primary()`", "not a `.widths(…)`", "no
                #               `env.argv()`"), and all five were on the
                #               roster, so the gate was demanding
                #               coverage of methods the page says are
                #               absent. cog.md leaves the census
                #               entirely: its one "call" was
                #               `module_data[i].len()` describing a
                #               field.
                #   279 -> 261  NOT coverage, and not an argument
                #               either: the INSTRUMENT was corrected,
                #               in both directions at once. Per-test
                #               granularity gave back 38 (279 -> 241);
                #               removing comments from the corpus took
                #               away 20 that were never real (241 ->
                #               261). 16 pages' methods joined the
                #               roster whose only evidence had been
                #               prose, and 33 left it having been
                #               exercised all along by tests thrown out
                #               with the file that held them. Both
                #               halves measured separately before this
                #               line was written; see the docstring.
                #   278 -> 279  metrics.md `observe`. Its one call moved
                #               under `@ignore` when A118 showed the
                #               histogram cannot sum — a TRUE plus, and
                #               it is what sent me into the corpus
                #               filter above.
                #   293 -> 292  NOT coverage: one method left the
                #               census between two runs, and the
                #               denominator rose 1381 -> 1401 in the
                #               same interval. Recorded rather than
                #               credited — a ratchet standing above
                #               its own count admits the next
                #               regression without saying so.
                #   385 -> 378  vcs/specs/core/io/fs_operations_run.vr
                #   378 -> 372  .../core/base/iterator_adapters_run.vr
                #               .../core/simd/vec_lanes_run.vr
                #   372 -> 366  .../core/random/rng_surface_run.vr
                #   323 -> 299  .../core/term/layout_style_run.vr
                #   The count went UP to 325 first, and a peer's A/B
                #   named the cause before I did: rewriting the term
                #   pages against `core/` replaced invented names with
                #   real ones, so the DENOMINATOR grew 1382 -> 1384 and
                #   two real methods joined the documented set. The
                #   signature that tells this from a regression is that
                #   both numbers move; a regression moves the numerator
                #   alone. Answered with coverage, per the rule at the
                #   top of this comment.
                #   328 -> 323  .../core/text/expand_escape_parse_run.vr
                #   338 -> 328  .../core/collections/btree_deque_multiset_run.vr
                #   341 -> 338  the peer's work, not mine — noted here so
                #               the next lowering does not claim it.
                #   366 -> 341  .../core/term/widget_builders_run.vr
                # The term page rose to 373 first, the same way and for
                # the same reason: correcting it against the `implement`
                # blocks replaced four names declared NOWHERE
                # (Table.widths, DialogButton.primary, Menu.orientation,
                # Scrollbar.new) with real ones, and real names count.
                # 82 of its 132 methods ran nothing; 51 do now.
                # The count had drifted UP to 380 first, and not through
                # anyone's fault: the stdlib reference's random section was
                # corrected twice on 2026-09-08 (methods on the wrong
                # receiver; nine entries that are free functions, not `Rng`
                # methods), and a correction that names REAL methods adds
                # them to the documented set. The ratchet's rule held — the
                # answer was coverage, not a raised baseline.

BLOCK = re.compile(r"```verum\n(.*?)```", re.S)
CALL = re.compile(r"\.([a-z_][a-z0-9_]*)\s*\(")
DIRECTIVE = re.compile(r"^// @test: *([a-z-]+)", re.M)
# A MODULE-QUALIFIED FREE FUNCTION LOOKS EXACTLY LIKE A METHOD and is
# not one. The pages write `rfc3339.format_utc(t.unix_seconds, t.nanos)`
# and `jcs.canonicalize_str(&raw)`, so `CALL` above files both as
# methods — while `core/` declares them `public fn`, and a test calls
# them by their bare mounted name. Evidence written `format_utc(0, 0)`
# was invisible: six methods stayed on the roster with green coverage
# sitting in core-tests (measured 2026-09-12).
#
# The receiver decides, and the page says which is which: a receiver
# that matches a MODULE this page mounts is a module, and the function
# is free. Everything else is a value, and a method stays a method.
# Deliberately NOT a blanket "a bare call counts" — that was measured
# first and it silences the gate's own controls, because `entry(` occurs
# bare in the corpus and `Map.entry` is one of the three known-broken
# methods this gate proves itself with.
MOUNT = re.compile(r"^\s*(?:public\s+)?mount\s+([A-Za-z_][\w.]*)", re.M)
QUALIFIED_CALL = re.compile(r"\b([a-z_][a-z0-9_]*)\s*\.\s*([a-z_][a-z0-9_]*)\s*\(")

# THE ROSTER. A count answers "did the population grow"; it cannot answer
# "did its membership change". 292 pairs held steady while one page gains a
# covered method and another loses one is a SWAP, and the count prints
# `baseline 292` over it. The roster is a sidecar rather than 292 lines of
# Python for the same reason `barename_collision_membership.txt` is: a
# roster this size belongs beside the gate, not inside it.
#
# KEY = (page, method name). Deliberately NOT a line number — this
# population lives in a DIFFERENT REPOSITORY than the gate, edited by
# commits that never touch this file, so a positional key would redden on
# unrelated prose edits and teach the reader to re-baseline without looking.
ROSTER = Path(os.environ.get("VERUM_DOC_METHODS_ROSTER")
              or (Path(__file__).resolve().parent
                  / "doc_methods_unexercised.txt"))

# A FILE THAT DOES NOT COMPILE IS NOT EXECUTION, and it looks exactly
# like one that does: it has `@test` functions, no `@ignore`, and its
# calls sit in the corpus text. Measured 2026-09-12,
# `core-tests/base/iterator/unit_test.vr` is 215 tests and 215 failures
# — 16 typecheck errors around `peekable()`/`peek()` unifying
# `Item<ISize>` against `Int` — so every method it names looked covered
# and none of them ran. Nothing a text scan can see distinguishes the
# two, so the list is DATA, written from a real run, one path per line.
# Regenerate it by running the suite, not by reasoning.
DEAD = Path(os.environ.get("VERUM_DOC_METHODS_DEAD")
            or (Path(__file__).resolve().parent
                / "doc_methods_dead_files.txt"))


def dead_files() -> set[str]:
    if not DEAD.is_file():
        return set()
    return {
        line.split("#", 1)[0].strip()
        for line in DEAD.read_text(encoding="utf8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    } - {""}


def membership(rows) -> list[str]:
    """The population as identities, one per line: `page<TAB>method`."""
    return sorted(f"{page}\t{name}"
                  for page, _n, missing in rows for name in missing)


def read_roster() -> list[str] | None:
    if not ROSTER.is_file():
        return None
    return sorted(l for l in ROSTER.read_text(encoding="utf8").splitlines()
                  if l.strip() and not l.startswith("#"))


def compare(current: list[str], roster: list[str]) -> int:
    """Report a SWAP. Equal sizes with different members must NOT pass —
    that degenerate form is what the self-test falsifies."""
    cur, ros = set(current), set(roster)
    if cur == ros:
        return 0
    appeared = sorted(cur - ros)
    vanished = sorted(ros - cur)
    print(f"  MEMBERSHIP MOVED while the count held at {len(current)}: "
          f"{len(appeared)} newly unexercised, {len(vanished)} no longer.")
    for line in appeared[:20]:
        page, name = line.split("\t")
        print(f"    + {page:<28} .{name}()   documented, nothing runs it")
    if len(appeared) > 20:
        print(f"    + … {len(appeared) - 20} more")
    for line in vanished[:20]:
        page, name = line.split("\t")
        print(f"    - {page:<28} .{name}()   now exercised — delete this row")
    if len(vanished) > 20:
        print(f"    - … {len(vanished) - 20} more")
    print("  Regenerate with --write-membership ONLY after reading the list: "
          "a `+` row is a regression, a `-` row is the coverage this "
          "ratchet exists to collect.")
    return 1


IGNORE_ATTR = re.compile(r"^[ \t]*@ignore\b", re.M)
FN_HEADER = re.compile(r"^[ \t]*(?:pub(?:lic)?[ \t]+)?(?:async[ \t]+)?fn\b", re.M)


def strip_comments(text: str) -> str:
    """A method NAMED in a comment is not a method something RUNS.

    Measured 2026-09-12 on `core-tests/metrics/histogram/unit_test.vr`,
    whose header explains the defect it pins with the words
    `Histogram.observe(5.0)` — prose, in a `//` line, four lines above
    the only test that calls `observe`, which is `@ignore`d. Without
    this the sentence describing the hole counts as evidence the hole
    is covered.

    STRING LITERALS ARE LEFT ALONE, deliberately: `f"{x.len()}"` runs
    `len`, so stripping strings would delete real evidence to remove
    imaginary evidence. Newlines survive comment removal so the
    line-anchored patterns below still see the file's line structure.
    """
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        if text[i] == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1])
            i = j + 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j == -1 else j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j == -1 else j + 2
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


def _body_end(text: str, open_brace: int) -> int:
    """Index just past the `}` matching the `{` at `open_brace`."""
    depth, i, n = 0, open_brace, len(text)
    while i < n:
        c = text[i]
        if c == '"':
            i += 1
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return n


def strip_ignored_tests(text: str) -> str:
    """Drop the BODY of every `@ignore`d test, KEEPING its neighbours.

    THE GRANULE OF IGNORING IS A TEST, and this used to be a file: any
    core-tests file containing the string `@ignore` was dropped whole.
    Measured 2026-09-12 that discarded 183 of 1263 files, and the two
    ways it was wrong compound:

      * 69 of those 183 carry no `@ignore` ATTRIBUTE at all — the word
        appears in a comment saying why something is hard.
        `core-tests/base/env/unit_test.vr` has 76 running tests and was
        excluded by the phrase `// @ignore: arg(0) returns Maybe.None`.
      * of the rest, `core-tests/base/data/unit_test.vr` has 161 tests
        and ONE ignored; 160 tests' worth of evidence went out with it.

    Expects comment-stripped input, so a commented `@ignore` cannot
    reach the attribute pattern.
    """
    spans: list[tuple[int, int]] = []
    for m in IGNORE_ATTR.finditer(text):
        fn = FN_HEADER.search(text, m.end())
        if not fn:
            continue
        brace = text.find("{", fn.end())
        if brace == -1:
            continue
        spans.append((m.start(), _body_end(text, brace)))
    if not spans:
        return text
    merged: list[list[int]] = []
    for a, b in sorted(spans):
        if merged and a <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], b)
        else:
            merged.append([a, b])
    out, prev = [], 0
    for a, b in merged:
        out.append(text[prev:a])
        prev = b
    out.append(text[prev:])
    return "".join(out)


def executed_corpus() -> list[tuple[Path, str]]:
    """Only what actually runs, as (path, the text that runs)."""
    out: list[tuple[Path, str]] = []
    specs = REPO / "vcs" / "specs"
    if specs.is_dir():
        for f in specs.rglob("*.vr"):
            text = f.read_text(errors="ignore")
            m = DIRECTIVE.search(text[:2000])
            if m and m.group(1) in RUN_DIRECTIVES:
                out.append((f, strip_comments(text)))
    ct = REPO / "core-tests"
    if ct.is_dir():
        dead = dead_files()
        for f in ct.rglob("*.vr"):
            if str(f.relative_to(REPO)) in dead:
                continue
            out.append((f, strip_ignored_tests(
                strip_comments(f.read_text(errors="ignore")))))
    bx = REPO / "docs" / "by-example"
    if bx.is_dir():
        out += [(f, strip_comments(f.read_text(errors="ignore")))
                for f in bx.rglob("*.vr")]
    return out


def documented_methods(page: Path) -> set[str]:
    """The methods a page's examples CALL — in code, not in prose.

    A COMMENT INSIDE A BLOCK IS PROSE, which is what
    `check_doc_names_exist` has always held (see its LINE_COMMENT), and
    the two gates disagreeing about what a block says was itself the
    defect: this census demanded coverage for names that gate never
    checked exist. Measured 2026-09-12, 17 of 1402 documented methods
    live only in a comment, and FIVE of them are the page DENYING the
    method:

        term.md:427  // ... are a CONSTRUCTOR argument, not a `.widths(…)`
        term.md:515  // There is no `.primary()` on a ...
        cli.md:211   // ... no `env.argv()` and no `env.is_json_mode()`.
        async.md:567 // `Http.get_streaming` and `body.next_chunk()` do
                     //  not exist

    All five sat on the roster, so the gate was asking for coverage of
    four methods the page says are absent — work that cannot be done and
    would be wrong if it could. The remaining twelve are real claims
    written in prose; they lose their census row, and the answer to that
    is to put the call in code, where a claim belongs and where every
    other gate can see it.
    """
    names: set[str] = set()
    for m in BLOCK.finditer(page.read_text(errors="ignore")):
        names |= set(CALL.findall(strip_comments(m.group(1))))
    return names


def module_qualified_names(page: Path) -> set[str]:
    """Names this page calls on a MODULE receiver — free functions.

    Evidence for these may be written without the dot, because that is
    how a caller who mounted the name spells it. See MOUNT above.
    """
    text = page.read_text(errors="ignore")
    modules: set[str] = set()
    free: set[str] = set()
    for m in BLOCK.finditer(text):
        body = strip_comments(m.group(1))
        for path in MOUNT.findall(body):
            modules.update(seg for seg in path.split(".") if seg)
    for m in BLOCK.finditer(text):
        body = strip_comments(m.group(1))
        for recv, name in QUALIFIED_CALL.findall(body):
            if recv in modules:
                free.add(name)
    return free


def is_exercised(name: str, free: set[str], blob: str) -> bool:
    """Does anything that runs call `name`?"""
    if f".{name}(" in blob:
        return True
    if name in free:
        return re.search(r"(?<![\w.])" + re.escape(name) + r"\s*\(", blob) is not None
    return False


def self_test() -> int:
    """Prove each half fires before any count is believed."""
    bad = 0
    if not DIRECTIVE.search("// @test: run\n"):
        print("self-test: the directive pattern does not match `run`"); bad += 1
    if DIRECTIVE.search("// @test: typecheck-pass\n").group(1) in RUN_DIRECTIVES:
        print("self-test: typecheck-pass must NOT count as executed"); bad += 1
    if CALL.findall("m.entry(key).or_insert(0)") != ["entry", "or_insert"]:
        print("self-test: the call pattern misses a chained call"); bad += 1
    if BLOCK.findall("```verum\nfn main() {}\n```") != ["fn main() {}\n"]:
        print("self-test: the block pattern does not extract a verum block"); bad += 1

    # THE MEMBERSHIP HALF, proved by FAILING. A `compare()` degenerated
    # back into a size check ("equal sizes, no difference") passes every
    # count assertion above and dies exactly here, by name — which is the
    # whole defect this gate was carrying.
    a = ["a.md\tfoo", "b.md\tbar"]
    swapped = ["a.md\tfoo", "b.md\tbaz"]
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        same_size_swap = compare(a, swapped)
        identical = compare(a, list(a))
    if same_size_swap == 0:
        print("self-test: compare() PASSED a same-size swap — the roster is "
              "decorative and the gate is still count-only")
        print(buf.getvalue().rstrip())
        bad += 1
    if identical != 0:
        print("self-test: compare() rejected an identical population")
        print(buf.getvalue().rstrip())
        bad += 1
    if membership([("p.md", 3, ["z", "a"])]) != ["p.md\ta", "p.md\tz"]:
        print("self-test: membership() is not the sorted (page, name) key"); bad += 1

    # THE CORPUS HALF. Both of these were live defects on 2026-09-12 and
    # both are silent: they change what counts as evidence, and the count
    # they produce looks exactly as plausible as the right one.
    two_tests = (
        "// a header mentioning @ignore: arg(0) is broken\n"
        "@test\n"
        "fn lives() {\n"
        "    x.alive();\n"
        "}\n"
        "\n"
        "@test\n"
        '@ignore("2^47")\n'
        "fn skipped() {\n"
        "    if true { y.dead(); }\n"
        "}\n"
        "\n"
        "@test\n"
        "fn also_lives() {\n"
        "    z.also_alive();\n"
        "}\n"
    )
    kept = strip_ignored_tests(strip_comments(two_tests))
    if ".alive(" not in kept or ".also_alive(" not in kept:
        print("self-test: stripping an @ignore'd test took its NEIGHBOURS "
              "with it — the file-level filter is back"); bad += 1
    if ".dead(" in kept:
        print("self-test: an @ignore'd test's calls still count as "
              "execution evidence"); bad += 1
    if "@ignore: arg(0)" in kept:
        print("self-test: a COMMENT survived comment-stripping"); bad += 1
    # A commented mention must not disqualify the file: the old filter
    # dropped 69 files that contain no @ignore attribute at all.
    only_a_mention = "// see @ignore below\n@test\nfn t() { a.b(); }\n"
    if ".b(" not in strip_ignored_tests(strip_comments(only_a_mention)):
        print("self-test: a file merely MENTIONING @ignore lost its "
              "coverage"); bad += 1
    # THE DOC SIDE OF THE SAME RULE. A page that DENIES a method was
    # putting it in the census and demanding coverage for it.
    denial = "```verum\n// There is no `.primary()` on a Dialog.\nd.body(x);\n```"
    got = set()
    for b in BLOCK.finditer(denial):
        got |= set(CALL.findall(strip_comments(b.group(1))))
    if "primary" in got:
        print("self-test: a method the page says does NOT exist is still "
              "counted as documented"); bad += 1
    if "body" not in got:
        print("self-test: comment-stripping ate the block's real call"); bad += 1

    # THE MODULE-RECEIVER RULE, both polarities. A blanket "a bare call
    # counts" was measured first and silences this gate's own controls.
    free = {"format_utc"}
    corpus = "let s = format_utc(0, 0);\nlet e = m.entry(k);\nentry(x);\n"
    if not is_exercised("format_utc", free, corpus):
        print("self-test: a module-qualified free function is not credited "
              "with its bare call — coverage in core-tests stays invisible")
        bad += 1
    if is_exercised("nowhere", free, corpus):
        print("self-test: a name nothing calls reports as exercised"); bad += 1
    # `entry` occurs BARE in that corpus and must still not count: it is
    # a method on a value, and it is one of the three known-broken
    # controls this gate proves itself with.
    if is_exercised("entry", set(), corpus.replace("m.entry(k);", "")):
        print("self-test: a bare call credits a VALUE method — the "
              "known-broken controls would go silent"); bad += 1

    # A method named only in prose is not a method anything runs.
    if ".observe(" in strip_comments("// Histogram.observe(5.0) answered 0\n"):
        print("self-test: a call written in a comment counts as evidence")
        bad += 1
    # ... but a call inside a format literal DOES run, and must survive.
    if ".len(" not in strip_comments('print(f"{s.len()}");\n'):
        print("self-test: stripping comments ate a string literal"); bad += 1
    # A `//` inside a string is not a comment.
    if "http" not in strip_comments('let u = "http://x";\n'):
        print("self-test: a URL in a string was read as a comment"); bad += 1
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-methods-exercised: {DOCS} not present — "
              "the docs live in a sibling checkout; "
              "reporting UNMEASURED rather than passing on an absent input.")
        return 0

    corpus = executed_corpus()
    blob = "\n".join(text for _f, text in corpus)

    rows = []
    for page in sorted(DOCS.glob("*.md")):
        names = documented_methods(page)
        if not names:
            continue
        free = module_qualified_names(page)
        missing = sorted(n for n in names if not is_exercised(n, free, blob))
        rows.append((page.name, len(names), missing))

    total = sum(n for _, n, _ in rows)
    unexercised = sum(len(m) for _, _, m in rows)

    # THE CONTROL. Methods measured broken on 2026-09-07 must come back
    # unexercised — a census with no known answer cannot be trusted, and
    # this one has three known answers plus one known miss.
    known_broken = ["entry", "get_key_value", "remove_entry"]
    held = [n for n in known_broken if f".{n}(" not in blob]
    print(f"control: {len(held)}/{len(known_broken)} known-broken methods "
          f"report as unexercised ({', '.join(known_broken)})")
    if len(held) != len(known_broken):
        print("  CONTROL FAILED — the census says a known-broken method is "
              "covered, so its zeros mean nothing. Fix before reading counts.")
        return 1

    # A FLOOR, same reason as the sibling gates carry one: an input that
    # went missing prints the same clean line as a clean corpus.
    floor_pages = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-pages" and i + 1 < len(sys.argv):
            floor_pages = int(sys.argv[i + 1])
    if len(rows) < floor_pages:
        print(f"check-doc-methods-exercised: only {len(rows)} stdlib page(s) "
              f"with documented methods under {DOCS}, expected at least "
              f"{floor_pages} — the corpus is missing or the pattern stopped "
              "matching. A census of nothing is not a clean census.")
        return 1
    if not corpus:
        print("check-doc-methods-exercised: the EXECUTED corpus is empty — "
              "no spec, core-test or by-example programme was found, so every "
              "method would report unexercised. Refusing to report a count.")
        return 1

    print(f"check-doc-methods-exercised: {unexercised} of {total} documented "
          f"methods across {len(rows)} page(s) are called by nothing that "
          f"runs (baseline {BASELINE})")
    for name, n, missing in sorted(rows, key=lambda r: -len(r[2]))[:10]:
        if missing:
            print(f"  {len(missing):>4} of {n:<4} {name}")

    current = membership(rows)

    if "--write-membership" in sys.argv:
        ROSTER.write_text(
            "# Population of `check-doc-methods-exercised`: one line per\n"
            "# (stdlib reference page, method name) that NOTHING THAT RUNS\n"
            "# calls. Generated by `--write-membership`; a `-` row in a gate\n"
            "# failure is coverage that was won and this file must lose.\n"
            + "\n".join(current) + "\n", encoding="utf8")
        print(f"wrote {len(current)} rows to {ROSTER}")
        return 0

    if unexercised > BASELINE:
        print(f"  ABOVE BASELINE by {unexercised - BASELINE}. A method with no "
              "execution evidence is one nobody would notice breaking.")
        return 1

    roster = read_roster()
    if roster is None:
        print(f"  NO ROSTER at {ROSTER}. The count alone cannot report a swap, "
              "so this gate has nothing to check membership against. "
              "Refusing rather than passing — regenerate with "
              "`--write-membership`.")
        return 2

    if unexercised < BASELINE:
        print(f"  BELOW baseline by {BASELINE - unexercised} — lower it.")
        won = sorted(set(roster) - set(current))
        for line in won[:25]:
            page, name = line.split("\t")
            print(f"    - {page:<28} .{name}()   now exercised")
        if len(won) > 25:
            print(f"    - … {len(won) - 25} more")
        return 0

    # The count AGREES with the baseline. That is precisely the state in
    # which a swap is invisible, so the membership question is asked here
    # and not earlier.
    return compare(current, roster)


if __name__ == "__main__":
    sys.exit(main())
