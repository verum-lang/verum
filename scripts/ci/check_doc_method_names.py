#!/usr/bin/env python3
"""Gate: a METHOD a documentation example calls on a `core/` type must exist.

WHAT THE RECEIVER GATE CANNOT SEE. `check_doc_names_exist.py` asks
whether the RECEIVER in `Name.method(...)` is declared. It never asks
about `method`. So `Response.json(user)` passed it for as long as
`Response` existed — and `Response.json` has never existed. Fourteen
distinct fictional method names were live on the site while that gate
stood green, on fourteen pages, including four that a reader would copy:

    Response.json / Response.with_body / .into_ok / read_body_limited
                          an HTTP server cookbook whose four core calls
                          were all invented; the real builders are FREE
                          FUNCTIONS in `core/net/weft/response_ext.vr`
    RateLimiter.token_bucket
                          `RateLimiter` is a PROTOCOL with ONE method,
                          `try_admit`, which DECIDES rather than queues.
                          The page showed `limiter.acquire(1).await`,
                          promising a limiter that parks the task
    File.open_async       async file I/O is a separate type, `AsyncFile`
    TlsClient.new_resumed / TcpStream.connect_happy_eyeballs_async
                          two whole capabilities described as shipped;
                          neither exists
    Linear.new_xavier     the init is Kaiming, and comes from the
                          `Random` CONTEXT, not from a threaded rng
    DynamicTable.set_max_capacity
                          real name `set_capacity`, and it returns a
                          `Result` the doc signature dropped

WHY IT OVER-ADMITS ON PURPOSE. Every `fn <name>` anywhere in `core/`
counts as declared, whatever type it belongs to, plus every `fn` any
documentation block declares. That makes a REPORT a strong claim — the
language has no such method at ALL, on anything — and keeps this gate
free of the type-resolution work a precise version would need. It
under-reports (a method that exists on type A excuses a doc calling it
on type B) and that is the correct direction for a gate: a false
negative costs a defect that some other instrument may catch, a false
positive costs the gate its credibility.

THE FLOOR, measured rather than assumed. Two spans remain and both are
`Database.transactional()`, which is neither a defect nor fixable here:
`transformed_context` is real grammar (`grammar/verum.ebnf`), a parser
conformance spec writes `[Database.transactional(), Cache.scoped()]`,
and the transform position accepts any identifier — the standard library
simply ships no transformer to put in it. The pages show the SHAPE. They
are keyed below rather than silenced, so that the day a transformer
ships, this gate says so.

A page that TEACHES that a name is absent must write that name, so the
denial filter below excludes those. It is deliberately generous: an
over-broad exclusion costs one unreported span, while an under-broad one
turns every honest correction into a gate failure and teaches writers to
delete the explanation instead of the fiction.
"""
import collections
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
DOCS = pathlib.Path(os.environ.get("VERUM_DOCS_DIR") or (REPO.parent / "website" / "docs"))
CORE = REPO / "core"

FN = re.compile(r"\bfn\s+([a-z_][A-Za-z0-9_]*)\s*[<(]")
TYPE_DECL = re.compile(r"\b(?:public\s+|pub\s+)?type\s+([A-Z][A-Za-z0-9_]*)")
FENCE = re.compile(r"^```verum\n(.*?)^```", re.M | re.S)
CALL = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\.([a-z_][A-Za-z0-9_]*)\s*\(")
DENIAL = re.compile(
    r"(does not exist|do not exist|not exist|no such|never existed|"
    r"is not a|are not|not shipped|not implemented|invented|fiction|"
    r"undeclared|removed|renamed|until 2026|previously|earlier version|"
    r"was not real|were not real|does not|did not)", re.I)

# KEYED, not counted. A bare number cannot see a SWAP — one fiction
# repaired while another appears keeps the total unchanged — and a
# number carries no owner, so nobody can tell a floor from a debt.
KNOWN = {
    "Database.transactional":
        "NOT A DEFECT — the floor. `transformed_context` is real grammar "
        "and `vcs/specs/parser/success/contexts/context_groups.vr` writes "
        "`[Database.transactional(), Cache.scoped()]`. The transform "
        "position takes any identifier; `core/` ships no transformer. The "
        "pages show the shape. Remove this key when a transformer ships.",
}


def harvest_core():
    fns, types = set(), set()
    for f in CORE.rglob("*.vr"):
        t = f.read_text(encoding="utf-8", errors="replace")
        fns.update(FN.findall(t))
        types.update(TYPE_DECL.findall(t))
    return fns, types


def scan(core_fns, core_types, pages):
    doc_fns, blocks = set(), 0
    texts = {}
    for p in pages:
        texts[p] = p.read_text(encoding="utf-8", errors="replace")
        for m in FENCE.finditer(texts[p]):
            blocks += 1
            doc_fns.update(FN.findall(m.group(1)))

    spans = denials = 0
    hits = collections.defaultdict(set)
    for p in pages:
        for m in FENCE.finditer(texts[p]):
            body = m.group(1)
            for mm in CALL.finditer(body):
                recv, meth = mm.group(1), mm.group(2)
                if recv not in core_types:
                    continue
                spans += 1
                if meth in core_fns or meth in doc_fns:
                    continue
                if DENIAL.search(body[max(0, mm.start() - 160):mm.end() + 160]):
                    denials += 1
                    continue
                hits[f"{recv}.{meth}"].add(str(p))
    return spans, denials, blocks, hits


def self_test() -> int:
    """Both polarities, because a pattern that matches nothing passes."""
    bad = 0
    core_fns, core_types = {"open", "connect"}, {"File", "Foo"}
    tmp = pathlib.Path(os.environ.get("TMPDIR", "/tmp")) / "vr_method_gate_selftest"
    tmp.mkdir(parents=True, exist_ok=True)

    cases = [
        ("a fictional method is reported",
         "```verum\nFile.open_async(p)\n```\n", {"File.open_async"}),
        ("a real method is not",
         "```verum\nFile.open(p)\n```\n", set()),
        ("a non-core receiver is left to the receiver gate",
         "```verum\nWidget.frobnicate(p)\n```\n", set()),
        ("a denial is not an accusation",
         "```verum\n// File.open_async does not exist\nFile.open_async(p)\n```\n", set()),
        ("a method the DOCS declare is admitted",
         "```verum\nfn open_async(p: Text) {}\nFile.open_async(p)\n```\n", set()),
    ]
    for label, body, expect in cases:
        f = tmp / "case.md"
        f.write_text(body, encoding="utf-8")
        _, _, _, hits = scan(core_fns, core_types, [f])
        got = set(hits)
        if got != expect:
            print(f"  SELF-TEST FAIL: {label} — expected {expect or '{}'}, got {got or '{}'}",
                  file=sys.stderr)
            bad += 1
        else:
            print(f"  [ok] {label}")
    if not KNOWN:
        print("  SELF-TEST FAIL: the baseline is empty; it must name its floor",
              file=sys.stderr)
        bad += 1
    else:
        print(f"  [ok] all {len(KNOWN)} baseline entr(y/ies) carry a reason")
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        # A SKIP IS A VERDICT ABOUT NOTHING, and this gate shipped with the
        # defect it was written beside: `return 0` here means a failed
        # website checkout reads as a clean documentation tree. Fixed in
        # the same commit that fixed it in five neighbours — including
        # this one, which I had just written.
        import os as _os
        fatal = "--check" in sys.argv or bool(_os.environ.get("CI"))
        print(
            f"doc-method-names: docs directory not found: {DOCS} — set "
            f"VERUM_DOCS_DIR" + (
                ". REFUSING to report OK: a missing input under --check/CI is "
                "a failed checkout, not an empty finding."
                if fatal else " (skipped; pass --check to make this fatal)"
            ),
            file=sys.stderr if fatal else sys.stdout,
        )
        return 2 if fatal else 0
    core_fns, core_types = harvest_core()
    pages = sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx"))
    spans, denials, blocks, hits = scan(core_fns, core_types, pages)

    # THE DENOMINATOR IS PART OF THE VERDICT. `[ok]` is reachable by
    # scanning nothing — a moved docs dir, a changed fence marker — and
    # reads identically to a real pass. `check_gate_verdict_carries_a_
    # quantity` exists for this.
    if spans == 0:
        print(f"doc-method-names: FAIL — scanned {len(pages)} page(s), {blocks} "
              f"```verum block(s) and found ZERO calls on a core-declared type. "
              f"The docs moved, or the fence marker changed; either way this gate "
              f"is measuring nothing and must not report OK.", file=sys.stderr)
        return 2

    found, known = {}, {}
    for name, where in hits.items():
        (known if name in KNOWN else found)[name] = where

    if found:
        print(f"doc-method-names: FAIL — {len(found)} method name(s) called on a "
              f"`core/` type that neither `core/` nor the documentation declares:",
              file=sys.stderr)
        for name in sorted(found):
            for page in sorted(found[name]):
                print(f"    {name:<38} {pathlib.Path(page).name}", file=sys.stderr)
        print("\nEither the name is wrong (read the real one off `core/` and fix the "
              "page), or the capability is not shipped (say so with a "
              "`:::caution Not shipped` block naming what IS available). If the page "
              "is TEACHING that the name is absent, the denial wording is what "
              "excuses it — say plainly that it does not exist.", file=sys.stderr)
        return 1

    repaired = sorted(set(KNOWN) - set(known))
    if repaired:
        print(f"doc-method-names: FAIL — {len(repaired)} baseline entr(y/ies) no "
              f"longer occur and must be deleted from KNOWN:", file=sys.stderr)
        for name in repaired:
            print(f"    {name}", file=sys.stderr)
        print("\nA baseline that outlives its subject is how a gate goes quietly "
              "false-green: the count still matches and the entry describes "
              "something that no longer happens.", file=sys.stderr)
        return 1

    print(f"[ok] doc-method-names: 0 undeclared method(s) in {spans} call(s) on "
          f"core-declared types, across {blocks} ```verum block(s) in {len(pages)} "
          f"page(s) ({denials} excluded as denials, {len(known)} keyed)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
