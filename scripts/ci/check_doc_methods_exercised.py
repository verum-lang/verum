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
    core-tests  files without @ignore             (180 of 1263 carry it)
    docs/by-example                               (the 22 showcase programmes)

KNOWN WEAKNESS, stated rather than hidden. The match is on the METHOD
NAME, not on the receiver's type: `.get_mut(` in a List test counts as
evidence for `Map.get_mut`, which is measurably broken. The gate's own
control reports this — three of four known-broken Map methods come back
unexercised and `get_mut` does not. A per-type version needs the
receiver's static type, which needs the checker; this is the cheap
version that runs with no build.

UNEXERCISED IS NOT BROKEN. It means nobody would notice if it broke.
`Map.entry` is what that looks like when it happens.
"""
from __future__ import annotations
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
BASELINE = 293  # Lowered by COVERAGE, never by argument — the only way
                # this number is meant to move.
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


def executed_corpus_files() -> list[Path]:
    """Only the files something actually runs."""
    out: list[Path] = []
    specs = REPO / "vcs" / "specs"
    if specs.is_dir():
        for f in specs.rglob("*.vr"):
            m = DIRECTIVE.search(f.read_text(errors="ignore")[:2000])
            if m and m.group(1) in RUN_DIRECTIVES:
                out.append(f)
    ct = REPO / "core-tests"
    if ct.is_dir():
        out += [f for f in ct.rglob("*.vr")
                if "@ignore" not in f.read_text(errors="ignore")]
    bx = REPO / "docs" / "by-example"
    if bx.is_dir():
        out += list(bx.rglob("*.vr"))
    return out


def documented_methods(page: Path) -> set[str]:
    names: set[str] = set()
    for m in BLOCK.finditer(page.read_text(errors="ignore")):
        names |= set(CALL.findall(m.group(1)))
    return names


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

    files = executed_corpus_files()
    blob = "\n".join(f.read_text(errors="ignore") for f in files)

    rows = []
    for page in sorted(DOCS.glob("*.md")):
        names = documented_methods(page)
        if not names:
            continue
        missing = sorted(n for n in names if f".{n}(" not in blob)
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
    if not files:
        print("check-doc-methods-exercised: the EXECUTED corpus is empty — "
              "no spec, core-test or by-example programme was found, so every "
              "method would report unexercised. Refusing to report a count.")
        return 1

    print(f"check-doc-methods-exercised: {unexercised} of {total} documented "
          f"methods are called by nothing that runs (baseline {BASELINE})")
    for name, n, missing in sorted(rows, key=lambda r: -len(r[2]))[:10]:
        if missing:
            print(f"  {len(missing):>4} of {n:<4} {name}")

    if unexercised > BASELINE:
        print(f"  ABOVE BASELINE by {unexercised - BASELINE}. A method with no "
              "execution evidence is one nobody would notice breaking.")
        return 1
    if unexercised < BASELINE:
        print(f"  BELOW baseline by {BASELINE - unexercised} — lower it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
