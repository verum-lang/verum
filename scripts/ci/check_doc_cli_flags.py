#!/usr/bin/env python3
"""Every CLI flag the documentation shows must exist in the binary —
and every flag a doc says does NOT exist must still be absent.

THE DENOMINATOR WAS 22%. Until 2026-09-07 the invocation pattern
required a `$ ` shell prompt, so it read 138 of the 625 `verum …` lines
in the corpus. The other 487 — 385 of them carrying flags, 221 distinct
command+flag pairs — were invisible, which is where a stale flag
survives longest: nobody writes the prompt inside a `bash` fence that is
showing a sequence. Three more sat behind a `- ` bullet and 41 behind a
trailing `\\` continuation, whose flags are precisely the ones a
per-line reader cannot see. All four shapes are read now.

BOTH POLARITIES. A line whose comment says NOT IMPLEMENTED used to be
dropped entirely, which excused it FOREVER: the day the command ships,
the page still tells the reader not to use it and the gate stays green.
Such a line is now checked the other way — the claim must still hold.
Measured 2026-09-07: 3 such lines (`expand-macros`, `doc --search`,
`api --signature`), all three still true.

Measured 2026-09-05 on the website docs: 17 commands, 40 flag mentions,
FIVE flags that no `--help` lists — `check --tier-report` (three times,
in the memory-safety tutorial, with a per-line output format the tool
never printed), `repl --session`, `repl --no-project`, `doc --search`,
`audit --filter`. One more, `api --signature`, was already marked. A
reader following any of them gets `error: unexpected argument`.

The class this catches is not "a typo". It is a flag that WAS real and
was renamed, and prose that kept the old name because nothing executes
prose. The homepage carried the same class in a different shape: a
sample showing `verum analyze --escape` output that the command does
not print.

WHAT IS DEEMED FINE
  * A line whose comment says NOT IMPLEMENTED / does not exist / not
    supported. Documenting a gap is the correct thing to do and must
    not fail the gate; the gap being documented is the point.
  * `-h`, `--help`, `--version` — universal, and not always echoed in
    a subcommand's own help text.

INSTRUMENT CONTROL
  The check runs `<binary> <cmd> --help` and reads the flags out of it.
  A NON-ZERO EXIT means the command is not there (or the help failed)
  and it is reported as unreadable rather than silently contributing
  zero findings. Reading the flag COUNT instead was wrong and was
  measured wrong: an unrecognized subcommand's error text ends with
  "try '--help'", so it parses as one flag and looks readable.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS_ENV = os.environ.get("VERUM_DOCS_DIR")
DOCS = Path(DOCS_ENV) if DOCS_ENV else REPO.parent / "website" / "docs"

# `$ verum <cmd> …` — the flags are every `--flag` on the rest of the
# line, NOT only the ones before the first positional. Caught by this
# file's own self-test: `repl --preload x.vr --skip-verify` reported one
# flag when the pattern required the flags to be contiguous, so a flag
# sitting after a filename was invisible — the exact place a stale flag
# survives longest.
#
# The tail stops at a SPACED pipe or `&&`: `verum doc | grep --color`
# names a flag of grep, not of verum. The space is the discriminator —
# stopping at any `|` truncated `verum extract [--target verum|ocaml|
# lean|coq] [--out DIR]` at the first alternation and hid `--out`, which
# does not exist either. Shell pipelines are spaced; placeholder
# alternations are not.
# A `$ ` prompt anywhere in the line, OR a line that BEGINS with the
# command (optionally behind a `- `/`* `/`> ` marker). The `^` is not
# MULTILINE and the caller feeds one line at a time, so prose that
# merely names a command mid-sentence still does not match — "Run verum
# test --workspace to check everything" has no anchor at position 0 and
# `^` cannot match anywhere else. That case is in the self-test.
# The `(?=\s|$)` is load-bearing: `verum hello.vr` invokes a FILE, not a
# subcommand called `hello`, and without the boundary the name matched up
# to the dot and three pages were reported as naming a missing command.
INVOCATION = re.compile(
    r"(?:\$ |^\s*(?:[-*>]\s+)?)verum ([a-z][a-z0-9-]*)(?=\s|$)"
    r"((?:(?!\s[|&;#])[^\n])*)")
FLAG = re.compile(r"--[a-z][a-z0-9-]*")
EXCUSED = re.compile(
    r"#[^\n]*\b(NOT IMPLEMENTED|does not exist|not implemented|"
    r"not yet supported|not supported)\b", re.I)
UNIVERSAL = {"--help", "--version"}

# A GAP IS DOCUMENTED IN THE LINE'S OWN COMMENT, and that is the whole
# convention. Inferring it from the prose beside the fence was tried and
# abandoned: an admonition that says "none of these four commands
# exists" is easy, but the next one says "`verum doc` … is the nearest
# thing that ships" two clauses after an absence phrase, and the one
# after that lists the REAL roster inside the same parenthesis. Each
# rescue bred the next false positive. The machine-checkable signal is
# the comment; the prose stays for the reader.
# What clap says when the name is not a subcommand at all.
ABSENT_MSG = re.compile(
    r"unrecognized subcommand|unexpected argument|no such subcommand|"
    r"invalid subcommand", re.I)


def binary() -> str:
    return os.environ.get("VERUM_BIN") or str(REPO / "target" / "debug" / "verum")


def logical_lines(text: str) -> list[tuple[int, str]]:
    """(first line number, line) with `\\`-continuations joined.

    41 lines in the corpus end with a backslash, and the flags after the
    break are exactly the ones a per-line reader cannot see. The number
    reported is where the command STARTS, which is where a reader looks.
    """
    lines = text.split("\n")
    out: list[tuple[int, str]] = []
    i = 0
    while i < len(lines):
        start, buf = i + 1, lines[i]
        # Bounded by the file: a run of continuations ends at EOF.
        while buf.rstrip().endswith("\\") and i + 1 < len(lines):
            buf = buf.rstrip()[:-1] + " " + lines[i + 1].strip()
            i += 1
        out.append((start, buf))
        i += 1
    return out


def parse_line(line: str) -> tuple[str, list[str], bool] | None:
    """(subcommand path, flags, is-a-documented-gap), or None.

    ONE parser, used by both the corpus walk and the self-test. They
    used to be two copies of the same logic, which is a gate that can
    pass its own test while doing something else.
    """
    m = INVOCATION.search(line)
    if not m:
        return None
    tail = m.group(2)
    # `--` ends verum's own arguments: `verum run -- --json a.txt`
    # passes `--json` to the PROGRAM. Measured — without this the
    # gate reports the program's flag as a missing verum flag.
    tail = tail.split(" -- ", 1)[0]
    # Leading bare words are a SUBCOMMAND PATH, not arguments:
    # `verum cog-registry publish --manifest` asks about
    # `cog-registry publish`, whose flags `cog-registry --help`
    # does not list. Measured — treating them as one command
    # reported nine flags that all exist, one level down.
    # A SUBCOMMAND PATH IS SINGLE-SPACED. A run of two or more spaces is
    # column alignment, and what follows it is another column:
    # `verum run             shape ok` is a table row reporting that the
    # tier-0 run prints "shape ok", not a call of `verum run shape ok`.
    head = re.split(r"  +", tail)[0]
    words = []
    for w in head.split():
        if w.startswith("-"):
            break
        if not re.fullmatch(r"[a-z][a-z0-9-]*", w):
            break
        words.append(w)
    flags = [f for f in FLAG.findall(tail) if f not in UNIVERSAL]
    return " ".join([m.group(1)] + words), flags, bool(EXCUSED.search(line))


Where = list[tuple[str, int]]


# THE MARKETING HOMEPAGE IS A PAGE. It is `.tsx`, not `.md`, so every
# gate that globs `*.md` silently omits the first page a reader sees —
# the shape `check_doc_names_exist` names in its own census, where
# covering it added a fictional type nobody had counted. Measured
# 2026-09-11: `src/pages/index.tsx` shows `verum test --interp`,
# `verum test --aot`, `verum analyze --escape`, `verum run` and
# `verum build`, none of which this gate had ever read. All five hold
# today; the point is that nothing was checking.
def _pages(docs: Path) -> list[tuple[Path, str]]:
    out = [(p, p.relative_to(docs).as_posix()) for p in sorted(docs.rglob("*.md"))]
    home = docs.parent / "src" / "pages" / "index.tsx"
    if home.is_file():
        out.append((home, "src/pages/index.tsx"))
    return out


def shown(docs: Path) -> tuple[dict[str, dict[str, Where]],
                              list[tuple[str, int, str, list[str]]],
                              dict[str, Where]]:
    """Two populations, read from the same lines.

    First: {command: {flag: [(file, line), …]}} — flags the docs SHOW,
    which must exist.  Second: one (file, line, command, flags) per line
    claiming ABSENCE, judged whole. Third: where each command is shown,
    so a missing one can be reported with a place to fix.
    """
    present: dict[str, dict[str, Where]] = {}
    # A GAP COMMENT MARKS THE LINE, not one flag on it. Keyed by site so
    # the claim can be judged whole: `verum build --release --pgo …
    # # NOT IMPLEMENTED` stays true while `--pgo` is missing, however
    # real `--release` is.
    absent: list[tuple[str, int, str, list[str]]] = []
    cmd_where: dict[str, Where] = {}
    for md, rel in _pages(docs):
        try:
            text = md.read_text(errors="ignore")
        except OSError:
            continue
        rows = logical_lines(text)
        for lineno, line in rows:
            parsed = parse_line(line)
            if parsed is None:
                continue
            path, flags, excused = parsed
            if excused:
                absent.append((rel, lineno, path, flags))
            else:
                # RECORDED EVEN WITH NO FLAGS. A shown command must exist
                # whether or not the line happens to carry a `--flag`, and
                # keying on flags made three of the four wrong `verum
                # cache …` lines invisible while catching the fourth —
                # the one difference between them being a `--older-than`.
                present.setdefault(path, {})
                for fl in flags:
                    present[path].setdefault(fl, []).append((rel, lineno))
                cmd_where.setdefault(path, []).append((rel, lineno))
    return present, absent, cmd_where


def real_flags(bin_path: str, cmd: str) -> set[str] | None:
    """Flags `<cmd> --help` lists, or None when the help is unreadable.

    `cmd` may be a subcommand PATH ("cog-registry publish").
    """
    try:
        p = subprocess.run([bin_path, *cmd.split(), "--help"],
                           capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    # NEITHER THE FLAG COUNT NOR THE EXIT CODE ALONE. Both were tried and
    # both are wrong:
    #
    #   flag count — "no flags means no command" is false. clap answers
    #     an unrecognized subcommand with `error: unrecognized subcommand
    #     'eval' … try '--help'`, which parses as ONE flag and looks
    #     readable. That answer reported two documented gaps as closed.
    #
    #   exit code — sound on its own for every command measured, but it
    #     is a PROXY: it cannot tell "this name is not a subcommand"
    #     from "this help legitimately exited non-zero".
    #
    #     (A claim that `verum cubical primitives --help` prints its help
    #     and exits 2 stood here briefly and was WRONG — the probe passed
    #     "cubical primitives" as ONE argv element, because zsh does not
    #     word-split an unquoted parameter the way bash does. Measured
    #     properly it is rc=0. Struck rather than deleted: the reading was
    #     about the shell, not the binary, and that is worth knowing at
    #     the next probe.)
    #
    # So require BOTH — a non-zero exit AND clap saying in words that the
    # name is not a subcommand. Each alone admits a failure the other
    # catches.
    text = p.stdout + p.stderr
    if p.returncode != 0 and ABSENT_MSG.search(text):
        return None
    found = set(FLAG.findall(text))
    return found or None


SELF_TEST = [
    # (line, (cmd, flags, is-documented-gap) or None)
    ("$ verum audit --bundle", ("audit", ["--bundle"], False)),
    # A subcommand path, not a command plus an argument.
    ("$ verum cog-registry publish --manifest cog.json",
     ("cog-registry publish", ["--manifest"], False)),
    ("$ verum repl --preload x.vr --skip-verify",
     ("repl", ["--preload", "--skip-verify"], False)),
    # A documented gap is now VISIBLE and flagged as such — it is checked
    # the other way, not dropped. Dropping it excused the claim forever.
    ("$ verum api --signature \"fn map\"       # NOT IMPLEMENTED",
     ("api", ["--signature"], True)),
    ("$ verum doc --search x   # does not exist", ("doc", ["--search"], True)),
    # A gap with no flag at all is a claim about the COMMAND.
    ("verum expand-macros src/user.vr   # not implemented",
     ("expand-macros", [], True)),
    # NO PROMPT: the shape that made the gate blind to 487 of 625 lines.
    ("verum eval \"1 + 2 + 3\"              # NOT IMPLEMENTED",
     ("eval", [], True)),
    ("    verum build --emit-vbc app.vr", ("build", ["--emit-vbc"], False)),
    # Behind a list marker.
    ("- verum lint --validate-config", ("lint", ["--validate-config"], False)),
    # Prose that merely names a command is STILL not an invocation: `^`
    # is not MULTILINE, so there is no anchor at "verum" mid-sentence.
    ("Run verum test --workspace to check everything", None),
    ("see verum build --release for details", None),
    # `--help` alone is universal and never reported.
    ("$ verum build --help", ("build", [], False)),
    # Everything after `--` belongs to the program being run, not verum.
    ("$ verum run -- --json /tmp/a.txt", ("run", [], False)),
    ("$ verum run --release -- --json a.txt", ("run", ["--release"], False)),
    # A pipeline's tail names another tool's flags, not verum's.
    ("$ verum lint --format json | jq --raw-output .",
     ("lint", ["--format"], False)),
    # An alternation inside a placeholder is not a pipeline: every flag
    # on the line must still be read.
    ("verum extract [--target verum|ocaml|lean|coq] [--out DIR]",
     ("extract", ["--target", "--out"], False)),
    ("verum doc --format html|markdown|json", ("doc", ["--format"], False)),
    # A FILE, not a subcommand: `verum hello.vr` runs the script.
    ("$ verum hello.vr", None),
    ("$ verum script.vr        # frontmatter wins", None),
    ("$ verum --allow-all untrusted.vr", None),
    # Column alignment, not a subcommand path: this row says what the
    # tier-0 run PRINTS.
    ("verum run             shape ok", ("run", [], False)),
    ("verum cache stats                        # cache hit rates",
     ("cache stats", [], False)),
]

# (text, expected joined lines) — the continuation joiner, tested apart
# because it runs BEFORE the pattern and a break in it makes the pattern
# look innocent.
JOIN_TEST = [
    ("verum build \\\n    --release \\\n    --target x\nnext",
     # The continuation's own indentation is dropped, as a shell drops
     # it: one space joins the pieces.
     [(1, "verum build  --release  --target x"), (4, "next")]),
    ("plain\nlines", [(1, "plain"), (2, "lines")]),
    # A trailing backslash on the LAST line must not run off the end.
    ("verum build \\", [(1, "verum build \\")]),
]


# ── OPTION TABLES ─────────────────────────────────────────────────
# A reference page does not only SHOW invocations; it tabulates a
# command's options under a heading that names the command. Those rows
# are claims of exactly the same kind, and until 2026-09-11 no gate read
# one. Measured that day on `verification/cli-workflow.md` alone: 44
# distinct flags documented, 12 of them accepted by no command at all
# (`--strategy` in six rows, `--counterexample`, `--budget-policy`,
# `--minimize-timeout`, `--show-costs` for the real `--show-cost`,
# `--admits`, `--cone`, `--since`, `--top`, `--by-theory`, `--lifetime`,
# a bare `--json` where `audit` takes `--format json`). The
# troubleshooting table handed out four of them as the FIRST THING TO
# TRY. Four more pages carried the same shape, including one whose
# admonition correctly denied `--pgo` while listing `--opt-level` beside
# it as real — it is not.
#
# NARROW ON PURPOSE. The docstring above records that inferring claims
# from free prose was tried and abandoned; this is not that. A row
# counts only when BOTH hold: the section heading names exactly one
# `verum <cmd>`, and the row's first cell begins with a `--flag`. That
# is a structured claim with a named subject, not a sentence.
# A command PATH, not just a leading token: the options a page
# tabulates often belong to a sub-subcommand (`verum llm-tactic
# propose --theorem`), and attributing them to the parent — which
# carries only `--color/--quiet/--verbose` — reported twelve real
# flags as rejected. `real_flags` already accepts a path.
SECTION_CMD = re.compile(
    r"^#{2,4}\s.*?`verum\s+([a-z][a-z0-9-]*(?:\s+[a-z][a-z0-9-]*){0,2})`")
TABLE_FLAG = re.compile(r"^\|\s*`(--[a-z][a-z0-9-]*)")
# A row, or an admonition standing over the rest of the section, may
# declare the gap. Kept separate from the shared UNSHIPPED vocabulary in
# `check_doc_names_exist`: widening that one changes what several other
# gates skip.
TABLE_EXCUSED = re.compile(
    r"not accepted|does not exist|do not exist|not implemented|"
    r"is not a flag|not shipped|rejects it|is rejected|NOT IMPLEMENTED",
    re.I)
ADMONITION = re.compile(r"^:::[a-z]+(.*)$")
HEADING = re.compile(r"^#{1,6}\s")


def option_table_claims(docs: Path) -> dict[str, dict[str, Where]]:
    """{command: {flag: [(file, line), …]}} read from option tables."""
    claims: dict[str, dict[str, Where]] = {}
    for md, rel in _pages(docs):
        try:
            text = md.read_text(errors="ignore")
        except OSError:
            continue
        cmd, cmd_level = None, 0
        section_excused = False
        for lineno, line in enumerate(text.split("\n"), 1):
            h = HEADING.match(line)
            if h:
                # An admonition's scope ends at the NEXT heading of any
                # level — the same rule `drop_unshipped` uses site-wide.
                # Carrying it into deeper subsections silenced every
                # later table in the section: one `--strategy` caution
                # hid `--solver`, `--timeout` and `--budget` too.
                section_excused = False
                level = len(line) - len(line.lstrip("#"))
                m = SECTION_CMD.match(line)
                # A heading naming two commands is ambiguous about who
                # owns the rows; such a section is skipped, not guessed.
                named = m.group(1) if m and line.count("`verum ") == 1 else None
                if named:
                    cmd, cmd_level = named, level
                elif level <= cmd_level:
                    # A sibling or shallower heading ENDS the section.
                    # A DEEPER one does not: `### Mode flags` under
                    # `## 3. verum verify` still tabulates verify's
                    # options, and treating it as a reset is what made
                    # this read 10 rows where the corpus has ~180.
                    cmd, cmd_level = None, 0
                continue
            adm = ADMONITION.match(line)
            if adm and TABLE_EXCUSED.search(adm.group(1)):
                section_excused = True
                continue
            if cmd is None or section_excused:
                continue
            m = TABLE_FLAG.match(line)
            if not m or TABLE_EXCUSED.search(line):
                continue
            flag = m.group(1)
            if flag in UNIVERSAL:
                continue
            claims.setdefault(cmd, {}).setdefault(flag, []).append((rel, lineno))
    return claims


OPTION_TABLE_TEST = [
    # (markdown, expected {cmd: [flags]})
    ("## 3. `verum verify` — flagship\n"
     "| `--timeout 60` | Per-obligation timeout. |\n",
     {"verify": ["--timeout"]}),
    # a row that declares its own gap is a claim of ABSENCE, not presence
    ("## 3. `verum verify`\n"
     "| `--strategy fast` | **Not accepted (measured 2026-09-11).** |\n",
     {}),
    # an admonition covers the rest of its section
    ("## 3. `verum verify`\n"
     ":::caution `--strategy` is not accepted\n"
     "text\n"
     ":::\n"
     "| `--strategy fast` | Static encoding only. |\n",
     {}),
    # …but not the NEXT section
    ("## 3. `verum verify`\n"
     ":::caution `--strategy` is not accepted\n:::\n"
     "## 5. `verum analyze`\n"
     "| `--escape` | Escape analysis. |\n",
     {"analyze": ["--escape"]}),
    # a heading naming two commands owns nothing
    ("## `verum check` vs `verum verify`\n"
     "| `--whatever` | … |\n",
     {}),
    # prose mentioning a flag is NOT a claim — only table rows are
    ("## 3. `verum verify`\n"
     "Pass `--strategy fast` for a quick pass.\n",
     {}),
    # a table outside any command section is not attributed
    ("## Exit codes\n"
     "| `--timeout` | … |\n",
     {}),
    # A DEEPER SUBHEADING KEEPS THE COMMAND.
    ("## 3. `verum verify`\n"
     "### Mode flags\n"
     "| `--mode static` | SMT. |\n",
     {"verify": ["--mode"]}),
    # A SIBLING HEADING ENDS IT.
    ("## 3. `verum verify`\n"
     "## 4. Exit codes\n"
     "| `--mode static` | SMT. |\n",
     {}),
    # AN ADMONITION ENDS AT THE NEXT HEADING, deeper ones included.
    ("## 3. `verum verify`\n"
     ":::caution `--strategy` is not accepted\n:::\n"
     "### Solver selection\n"
     "| `--solver auto` | Router decides. |\n",
     {"verify": ["--solver"]}),
    # A SUB-SUBCOMMAND PATH IS THE SUBJECT, not its parent.
    ("## `verum llm-tactic propose`\n"
     "| `--theorem NAME` | The theorem to attack. |\n",
     {"llm-tactic propose": ["--theorem"]}),
]


def option_table_self_test() -> int:
    import tempfile
    bad = 0
    for md, want in OPTION_TABLE_TEST:
        with tempfile.TemporaryDirectory() as d:
            Path(d, "p.md").write_text(md)
            got = option_table_claims(Path(d))
        flat = {c: sorted(f) for c, f in got.items()}
        if flat != {c: sorted(f) for c, f in want.items()}:
            bad += 1
            print(f"FAIL option-table {md!r} -> {flat}, expected {want}",
                  file=sys.stderr)
    return bad


def self_test() -> int:
    bad = 0
    for line, want in SELF_TEST:
        got = parse_line(line)
        if got is not None:
            got = (got[0], got[1], got[2])
        if got != want:
            bad += 1
            print(f"FAIL {line!r} -> {got}, expected {want}", file=sys.stderr)
    for text, want in JOIN_TEST:
        got = logical_lines(text)
        if got != want:
            bad += 1
            print(f"FAIL join {text!r} -> {got}, expected {want}", file=sys.stderr)
    bad += option_table_self_test()
    if bad:
        print(f"self-test: {bad} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"self-test: {len(SELF_TEST)} parse + {len(JOIN_TEST)} join + "
          f"{len(OPTION_TABLE_TEST)} option-table case(s) OK")
    return 0


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


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bin_path = binary()
    if not Path(bin_path).exists():
        return _skip_or_fail(sys.argv, f"check-doc-cli-flags: no verum binary at {bin_path}")
    if not DOCS.is_dir():
        return _skip_or_fail(sys.argv, f"check-doc-cli-flags: no docs directory at {DOCS}")

    present, absent, cmd_where = shown(DOCS)
    # Option-table rows are the same kind of claim as an invocation, and
    # are folded into the same population so one verdict covers both.
    tabled = option_table_claims(DOCS)
    table_claims = 0
    for cmd, flags in tabled.items():
        for fl, where in flags.items():
            table_claims += 1
            present.setdefault(cmd, {}).setdefault(fl, []).extend(where)
        cmd_where.setdefault(cmd, []).extend(next(iter(flags.values()), []))
    missing: list[tuple[str, str, list[str]]] = []
    stale: list[tuple[str, str | None, list[str]]] = []
    unreadable: list[str] = []
    checked = 0

    # `--help` is one subprocess per command; the two populations share
    # commands, so ask once.
    help_of: dict[str, set[str] | None] = {}
    for cmd in sorted(set(present) | {a[2] for a in absent}):
        help_of[cmd] = real_flags(bin_path, cmd)

    for cmd, flags in sorted(present.items()):
        if help_of[cmd] is None:
            unreadable.append(cmd)
            continue
        for fl, where in sorted(flags.items()):
            checked += 1
            if fl not in help_of[cmd]:
                missing.append((cmd, fl, sorted({f"{f}:{n}" for f, n in where})))

    # THE OTHER POLARITY. A line saying a thing does not exist is a claim,
    # and a claim that has come true in reverse is worse than a missing
    # one: the reader is told not to use a working command.
    for rel, lineno, cmd, flags in sorted(absent):
        checked += 1
        real = help_of[cmd]
        if real is None:
            continue  # the command still does not exist — claim holds
        if flags and not all(f in real for f in flags):
            continue  # at least one flag is still missing — claim holds
        what = f"{cmd} {' '.join(flags)}".strip()
        stale.append((what, None, [f"{rel}:{lineno}"]))

    if unreadable:
        print(f"[fail] {len(unreadable)} command(s) the docs invoke and the "
              f"binary does not have:")
        for cmd in unreadable:
            where = sorted({f"{f}:{n}" for f, n in cmd_where.get(cmd, [])})
            print(f"    verum {cmd}    — {', '.join(where[:3]) or 'shown with a flag'}")
        print("\nEither the subcommand is gone, or `--help` failed. Both are\n"
              "findings: a flag cannot be checked against a help text that is\n"
              "not there, so this is reported rather than counted as clean.")
        return 1

    if stale:
        print(f"[fail] {len(stale)} documented gap(s) the binary has since closed:")
        for cmd, fl, where in stale:
            what = f"verum {cmd} {fl}" if fl else f"verum {cmd}"
            print(f"    {what}    — {', '.join(where[:3])}")
        print("\nThe page says this does not exist and it now does. Drop the\n"
              "NOT IMPLEMENTED comment and any note beside it, and show what\n"
              "the command actually prints.")
        return 1

    if missing:
        print(f"[fail] {len(missing)} flag(s) the docs show and the binary rejects:")
        for cmd, fl, where in missing:
            print(f"    verum {cmd} {fl}    — {', '.join(where[:3])}")
        print("\nRun the command to see what it does have. If the flag is gone,\n"
              "name the replacement; if it never existed, say so in the line's\n"
              "own comment (`# NOT IMPLEMENTED`) — a documented gap is checked\n"
              "the other way, not excused.")
        return 1

    print(f"check-doc-cli-flags: {len(help_of)} command(s), {checked} claim(s) "
          f"({table_claims} from option tables) "
          f"({sum(len(v) for v in present.values())} shown, "
          f"{len(absent)} documented gaps), 0 unknown")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
