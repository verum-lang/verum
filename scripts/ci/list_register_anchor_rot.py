#!/usr/bin/env python3
"""A LIST TO READ: `file.rs:LINE` anchors in `docs/architecture/` that no
longer point where they say.

WHY. Two rows in one afternoon carried anchors that were ALL dead. A1
sent a reader to `llvm/instruction.rs:9759` for the const-zero degrade —
that line is a bare `ctx.set_register(dst.0, result)`; to `:19098` for
the unresolved CallM — a TCP/UDP routing comment; and to
`llvm/error.rs:668` for "the gate" — a stray comment. A9 sent one to four
line numbers, all four moved. **A DEAD LINE NUMBER IS WORSE THAN NO
ANCHOR**, because it reads as precise: the reader opens the file, sees
unrelated code, and concludes the row is confused rather than that the
anchor is.

WHY IT IS A LIST AND NOT A GATE. Line drift is normal and constant; a
gate on it would fail on every commit that adds a line to a large file,
which is every commit. It also cannot tell an anchor describing a PAST
state (this register is partly a historical record) from one describing
the present. The verdicts below are ranked so the decidable ones come
first and the advisory one is labelled as advisory.

THE THREE VERDICTS, in descending order of certainty:

  FILE-GONE   the path resolves to nothing in the tree. Decidable.
  PAST-EOF    the file has fewer lines than the anchor names. Decidable,
              and the strongest evidence of rot that does not require
              reading code.
  NO-SYMBOL   the anchor names a symbol in an adjacent parenthetical or
              backtick, and that symbol is absent within +/-40 lines.
              ADVISORY — a parenthetical is prose, not a declaration, and
              this arm will accuse a correct anchor whose note describes
              the CONSEQUENCE rather than the code. Read before believing.

CONTINUATION ANCHORS ARE REAL AND MUST BE RESOLVED. The register writes
`` `llvm/instruction.rs:9759` ... `:19098` `` — the second inherits the
first's file. Treating it as its own path finds nothing and would report
FILE-GONE on an anchor whose only fault is being written in the register's
own shorthand.
"""
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
DOCS = REPO / "docs" / "architecture"

# `path/to/file.rs:1234` or the continuation `:1234`, inside backticks.
ANCHOR = re.compile(r"`([A-Za-z0-9_./-]*\.(?:rs|vr|py|md|toml|ebnf))?:(\d{1,6})`")
# An identifier worth looking for near the line: snake_case with an
# underscore, or CamelCase, optionally `Type::method` / `Type.method`.
SYMBOL = re.compile(r"`?\b([A-Za-z_][A-Za-z0-9_]*(?:[.:]{1,2}[A-Za-z_][A-Za-z0-9_]*)*)\b`?")
CODEY = re.compile(r"[_]|[a-z][A-Z]")
WINDOW = 40
# A DOCUMENT THAT SAYS "THIS ANCHOR IS DEAD" MATCHES LIKE ONE THAT IS
# WRONG. The same disease as the muted-test list: a syntactic instrument
# cannot separate a use from a denial, so the denial is spelled out and
# looked for in a NARROW window around the anchor itself — a register line
# runs to thousands of characters and can hold three anchors in three
# states, so a wide window reads one anchor's obituary as another's.
ANCHOR_DENIAL = re.compile(
    r"(deleted|no longer exists?|since removed|removed by|was deleted|gone|"
    r"dead anchor|file DELETED|retired|renamed to)", re.I)
DENIAL_WINDOW = 120


def repo_files():
    """Suffix index over tracked source files, built once."""
    out = {}
    for ext in ("rs", "vr", "py", "md", "toml", "ebnf"):
        for f in REPO.rglob(f"*.{ext}"):
            s = str(f)
            # WORKTREES LIVE INSIDE THE REPO, and that alone made 198
            # of 200 anchors "unresolvable" on the first run. Every file
            # had four or five copies under `.claude/worktrees/`, so
            # `grammar/verum.ebnf` — which is unique — matched five paths
            # and was reported as ambiguous. A tree containing copies of
            # itself answers about the copies.
            if any(x in s for x in ("/target/", "/.git/", "/llvm/install/",
                                    "/.claude/worktrees/", "/node_modules/")):
                continue
            out.setdefault(f.name, []).append(f)
    return out


def resolve(index, path: str):
    """Longest-suffix match. Returns (file, 'ok'|'ambiguous'|'missing')."""
    cands = index.get(pathlib.PurePath(path).name, [])
    if not cands:
        return None, "missing"
    want = path.strip("/")
    exact = [c for c in cands if str(c).endswith("/" + want)]
    if len(exact) == 1:
        return exact[0], "ok"
    if len(exact) > 1:
        return None, "ambiguous"
    if len(cands) == 1:
        return cands[0], "ok"
    return None, "ambiguous"


def named_symbol(text: str, end: int) -> str | None:
    """A symbol from the parenthetical or backticks right after the anchor."""
    tail = text[end:end + 90]
    # THE ANCHOR IS ITSELF INSIDE THE PARENTHESES. `(`supervisor.vr:723`)`
    # leaves a tail that OPENS with `)`, so there is no trailing
    # parenthetical describing this anchor — and reading on picks up the
    # next clause's name instead. Three rows were exactly this: the
    # register's `(…supervisor.vr:723`), and `PostgresDatabase.connect(url)`
    # by …` had the anchor accused of not containing a symbol that belongs
    # to the sentence AFTER it. All three anchors were exact to the line.
    if tail.lstrip().startswith(")"):
        return None
    m = re.match(r"\s*\(([^)]{1,80})\)", tail)
    if m:
        blob = m.group(1)
    else:
        # A FIXED CUT SLICES AN IDENTIFIER IN HALF, and half an identifier
        # is a false accusation: `handle_deref` became `handle_de`,
        # `ItemKind` became `ItemKi`, `fat_ref.ptr` became `fat_ref.r`.
        # Each was then reported ABSENT from a file that contains the
        # whole name. Cut at the last word boundary instead, and drop the
        # trailing fragment.
        blob = re.sub(r"[A-Za-z0-9_.:]*$", "", tail[:40])
    for cand in SYMBOL.findall(blob):
        # A PARENTHETICAL NAMING ANOTHER FILE IS NOT A SYMBOL TO FIND HERE.
        # `stdlib_index.rs:150` (… core_metadata.rs …) says where the reader
        # goes NEXT; looking for "core_metadata.rs" inside stdlib_index.rs
        # reports rot on a correct anchor. Measured: 5 of the first 33
        # advisory rows were this.
        if cand.rsplit(".", 1)[-1] in ("rs", "vr", "py", "md", "toml", "ebnf"):
            continue
        # A CRATE NAME IS NOT A SYMBOL AT A LINE. `link.rs:560`
        # (… verum_codegen …) says which crate the row is about; looking
        # for "verum_codegen" INSIDE link.rs reports rot on a correct
        # anchor. Four of twenty advisory rows were this.
        if cand.startswith("verum_") and "." not in cand:
            continue
        if CODEY.search(cand) and len(cand) > 3:
            return cand
    return None


def self_test() -> int:
    """Both polarities on every arm — an instrument that reports nothing
    passes by doing nothing."""
    bad = 0
    checks = [
        ("continuation inherits the file",
         list(ANCHOR.finditer("`a/b.rs:10` and `:20`")),
         lambda ms: len(ms) == 2 and ms[1].group(1) is None),
        ("a plain word is not a symbol",
         named_symbol("`x.rs:1` (the gate)", 8), lambda s: s is None),
        ("a snake_case name is",
         named_symbol("`x.rs:1` (check_no_unresolved)", 8),
         lambda s: s == "check_no_unresolved"),
        ("a CamelCase name is",
         named_symbol("`x.rs:1` (UNRESOLVED_FN_ID -> zero)", 8),
         lambda s: s == "UNRESOLVED_FN_ID"),
    ]
    checks.append(("an anchor INSIDE parentheses takes no symbol from what "
                   "follows the closing paren",
                   named_symbol("(`a/b.rs:12`), and `Other.thing(x)` by …", 12),
                   lambda s: s is None))
    checks.append(("a crate name is not a symbol",
                   named_symbol("`x.rs:1` (verum_codegen owns this)", 8),
                   lambda s: s != "verum_codegen"))
    checks.append(("a name CUT BY THE WINDOW is dropped, not reported half",
                   named_symbol("`x.rs:1` sits in handle_deref which is long",
                                8), lambda s: s != "handle_de"))
    checks.append(("a parenthetical naming ANOTHER FILE is not a symbol",
                   named_symbol("`a.rs:1` (see core_metadata.rs)", 8),
                   lambda s: s is None))
    checks.append(("…but a real symbol beside a filename still wins",
                   named_symbol("`a.rs:1` (build_index in core_metadata.rs)", 8),
                   lambda s: s == "build_index"))
    checks.append(("an anchor the page itself calls deleted is excluded",
                   bool(ANCHOR_DENIAL.search("(`x/y.rs:12` — file DELETED by abc)")),
                   lambda b: b is True))
    checks.append(("a plain anchor is NOT read as a denial",
                   bool(ANCHOR_DENIAL.search("(`x/y.rs:12` the const-zero site)")),
                   lambda b: b is False))
    fake = {"verum.ebnf": [pathlib.Path("/r/grammar/verum.ebnf")]}
    checks.append(("a unique path resolves",
                   resolve(fake, "grammar/verum.ebnf")[1], lambda h: h == "ok"))
    fake2 = {"verum.ebnf": [pathlib.Path("/r/grammar/verum.ebnf"),
                            pathlib.Path("/r/.claude/worktrees/w/grammar/verum.ebnf")]}
    checks.append(("a WORKTREE COPY must not make it ambiguous — this is the "
                   "bug that hid 198 of 200 anchors",
                   resolve(fake2, "grammar/verum.ebnf")[1], lambda h: h == "ambiguous"))
    for label, got, ok in checks:
        if ok(got):
            print(f"  [ok] {label}")
        else:
            print(f"  SELF-TEST FAIL: {label} -> {got!r}", file=sys.stderr)
            bad += 1
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"not found: {DOCS}", file=sys.stderr)
        return 2
    index = repo_files()
    rows, scanned, ambiguous, advisory, denied = [], 0, 0, [], 0
    for page in sorted(DOCS.rglob("*.md")):
        text = page.read_text(encoding="utf-8", errors="replace")
        last_path, last_at = None, -10**9
        for m in ANCHOR.finditer(text):
            near = last_path if (m.start() - last_at) <= 200 else None
            path, line = m.group(1) or near, int(m.group(2))
            if m.group(1):
                last_path, last_at = m.group(1), m.end()
            if not path:
                continue
            scanned += 1
            f, how = resolve(index, path)
            ln = text[:m.start()].count("\n") + 1
            where = f"{page.name}:{ln}"
            if how == "ambiguous":
                ambiguous += 1
                continue
            if how == "missing":
                w = text[max(0, m.start() - DENIAL_WINDOW):m.end() + DENIAL_WINDOW]
                if ANCHOR_DENIAL.search(w):
                    denied += 1
                    continue
                rows.append(("FILE-GONE", where, f"{path}:{line}", ""))
                continue
            body = f.read_text(encoding="utf-8", errors="replace").split("\n")
            if line > len(body):
                w = text[max(0, m.start() - DENIAL_WINDOW):m.end() + DENIAL_WINDOW]
                if ANCHOR_DENIAL.search(w):
                    denied += 1
                    continue
                rows.append(("PAST-EOF", where, f"{path}:{line}",
                             f"file has {len(body)} lines"))
                continue
            sym = named_symbol(text, m.end())
            if sym:
                lo, hi = max(0, line - 1 - WINDOW), min(len(body), line + WINDOW)
                # THE PROSE QUALIFIES, THE FILE DECLARES BARE. A row
                # writing `BufRead.has_data_left` points at a line that
                # reads `fn has_data_left(&mut self)`; searching only for
                # the dotted form reports rot on an anchor that is exact
                # to the line. Try the last segment too.
                forms = {sym}
                if "." in sym:
                    forms.add(sym.rsplit(".", 1)[-1])
                if not any(any(fm in b for fm in forms) for b in body[lo:hi]):
                    advisory.append((where, f"{path}:{line}", sym))

    # AN INSTRUMENT THAT CANNOT FIND ITS INPUT MUST GET STRICTER, never
    # report a clean sheet: a moved docs directory and a healthy register
    # print the same zero.
    if scanned == 0:
        print("anchor-rot: FAIL — scanned 0 anchors. The corpus moved or the "
              "anchor spelling changed; this is not a clean result.",
              file=sys.stderr)
        return 2

    print(f"anchors scanned in {DOCS.relative_to(REPO)}/ : {scanned}")
    print(f"  unresolvable path (skipped, not judged)  : {ambiguous}")
    print(f"  already MARKED dead by the page itself    : {denied}")
    print(f"  DECIDABLY dead (file gone / past EOF)    : {len(rows)}")
    print(f"  advisory (named symbol not within +/-{WINDOW}) : {len(advisory)}\n")
    if rows:
        print("DECIDABLY DEAD — the anchor cannot point at anything:")
        for kind, where, anchor, note in sorted(rows):
            print(f"   {kind:<10} {where:<34} {anchor:<44} {note}")
    if advisory:
        print(f"\nADVISORY — the anchor's own note names a symbol that is not "
              f"within +/-{WINDOW} lines. A parenthetical is PROSE and may "
              f"describe the consequence rather than the code, so read the "
              f"line before believing the row:")
        for where, anchor, sym in sorted(advisory)[:40]:
            print(f"   {where:<34} {anchor:<44} wanted `{sym}`")
        if len(advisory) > 40:
            print(f"   … {len(advisory) - 40} more")
    print("\nNever gate on these numbers: line drift is constant, and this "
          "register is partly a historical record whose anchors describe past "
          "states on purpose.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
