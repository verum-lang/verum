#!/usr/bin/env python3
"""Gate: a documentation example that COMPILES is not an example that WORKS.

`check_doc_examples.py` asks whether each self-contained ```verum block
passes `verum check`. That is the right first question and it is not the
last one. Every defect closed in this repository on 2026-09-06 was a
program that type-checked perfectly and then did the wrong thing:

    shared.deref().url            interpreter u      AOT (empty)
    bag.lend_items().len()        interpreter 1      AOT 0
    path.exists()                 interpreter true   AOT SIGSEGV
    map.entry(k).or_insert_with(f)  interpreter NULL dereference

A reader copying any of those from the site gets a clean compile and a
wrong answer, which is the worst failure mode documentation has: the
page looks maintained.

The site's exposure to exactly those idioms, measured the day this gate
was written: `Shared.new` on 18 pages, `as_str()` on 11, `transduce` on
6, `or_insert_with` on 3. So this is not a hypothetical class.

WHAT IT COUNTS. A block with an `fn main` that COMPILES and then fails
to RUN. Compilation failures are deliberately not counted here — they
belong to `check_doc_examples.py` and counting them twice would make one
defect look like two, and make that gate's ratchet unreadable.

TWO MODES, because they cost two different amounts:

  (default)       `verum run` — Tier 0 only. Seconds per example. This
                  is the PR-affordable question: does the example run at
                  all.
  --differential  also `verum build` and run the binary, and compare the
                  two tiers. Minutes per example, so it is a nightly
                  question. A disagreement is a defect even when both
                  sides look plausible, and the interpreter is the side
                  that is right — the same rule the VCS differential
                  specs use.

CONTROLS RUN ON EVERY INVOCATION, both polarities, because a runner that
has silently stopped running things reports a clean sheet:

    a program that must succeed   — if it fails, the harness is broken
    a program that must panic     — if it "passes", the harness is not
                                    observing the exit status at all

The second is the one that matters. A census that only checks the quiet
direction reports zero for whatever reason and reads as a clean bill.

THE EXIT STATUS IS TAKEN FROM THE BINARY, never through a pipe.
`bin 2>&1 | grep …; echo $?` reports the exit of `grep`; it read 0 for a
process killed by SIGSEGV. Every run here captures `returncode` from the
process itself and reports the signal by name.
"""
import argparse
import collections
import hashlib
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS = Path(os.environ.get("VERUM_DOCS_DIR") or (REPO.parent / "website" / "docs"))

BLOCK = re.compile(r"^```verum\n(.*?)^```", re.M | re.S)

# Blocks the documentation deliberately shows as NOT compiling or NOT
# running. Same five spellings `check_doc_examples.py` recognises, plus
# the runtime ones — a page teaching what a panic looks like is correct
# when it panics.
MUST_FAIL = re.compile(
    r"//\s*(COMPILE ERROR|ERROR|error<E\d+>|does not compile|WRONG|BAD|✗|refused"
    r"|panics|PANIC|aborts|deadlocks)",
    re.I,
)

# A program that reads a clock, a socket or the filesystem can fail — or
# BLOCK FOREVER — for reasons that are not the documentation's fault.
# They are skipped rather than excused, so the count means one thing.
#
# NO `\b` ON THE LEFT, and that omission is measured rather than
# stylistic: the first version wrote `\b(listen|accept|…)\b` and reported
# two pages as broken because `__tcp_accept_raw` begins with underscores,
# which are word characters, so the left boundary never matched. The two
# pages teach a TCP echo server; blocking in `accept` is the example
# working. A filter that cannot see the intrinsic spelling of the thing
# it filters reports the correct behaviour as a defect.
ENVIRONMENTAL = re.compile(
    r"(?:^|[^A-Za-z0-9])_*(?:tcp_|udp_)?"
    r"(connect|listen|bind\(|accept|recv|send\(|spawn|Command|getenv|"
    r"read_dir|File\.open|Http\.|Database\.|now\(\)|random|sleep|stdin)"
)

# Every `page#hash` that runs today. Raising it means a page started
# teaching something that no longer works.
# The site marks unshipped work in a dozen explicit phrasings and a
# marked section is HONEST documentation of a plan, not drift. Rather
# than re-derive the list, this reuses `check_doc_names_exist`'s — one
# definition, two readers, and a page that adds a marker stops being
# counted by both gates at once.
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from check_doc_names_exist import drop_unshipped as _drop_unshipped
except Exception:  # the sibling gate is optional; without it nothing is dropped
    def _drop_unshipped(text):
        return text

BASELINE = 0

TIMEOUT_RUN = 60
TIMEOUT_BUILD = 900

CONTROL_OK = 'fn main() { print("control-ok"); }\n'
CONTROL_PANIC = 'fn main() { panic("control-panic"); }\n'


def verum_binary() -> Path:
    override = os.environ.get("VERUM_BIN")
    if override:
        p = Path(override)
        if not p.is_file():
            raise SystemExit(f"VERUM_BIN={override} is not a file")
        return p
    for c in (REPO / "target" / "release" / "verum", REPO / "target" / "debug" / "verum"):
        if c.is_file():
            return c
    raise SystemExit(
        "no verum binary at target/{release,debug}/verum — pass one with "
        "VERUM_BIN=/path/to/verum"
    )


def signal_name(rc) -> str:
    """Name the death rather than printing a number.

    A negative returncode is a signal on POSIX; 139 and 134 are the same
    thing seen through a shell. `rc=139` in a log has cost this project
    real time because it reads as an ordinary failure — it is a SIGSEGV.

    `None` means the run TIMED OUT, which is a distinct fact and not a
    zero: a documentation example that hangs is broken in a way a reader
    feels immediately.
    """
    if rc is None:
        return f"TIMEOUT (>{TIMEOUT_RUN}s)"
    sig = None
    if rc < 0:
        sig = -rc
    elif rc > 128:
        sig = rc - 128
    if sig is None:
        return f"rc={rc}"
    names = {4: "SIGILL", 6: "SIGABRT", 8: "SIGFPE", 10: "SIGBUS", 11: "SIGSEGV"}
    return f"{names.get(sig, f'signal {sig}')} (rc={rc})"


def run(binary: Path, argv, cwd=None, timeout=TIMEOUT_RUN):
    """Return (rc, combined output). rc comes from the process, not a pipe."""
    try:
        r = subprocess.run(
            [str(binary)] + argv,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=cwd,
        )
        return r.returncode, (r.stdout or "") + (r.stderr or "")
    except subprocess.TimeoutExpired:
        return None, "TIMEOUT"


def compiles(binary: Path, path: Path) -> bool:
    rc, _ = run(binary, ["check", str(path)])
    return rc == 0


def controls(binary: Path, tmp: Path) -> None:
    ok = tmp / "control_ok.vr"
    ok.write_text(CONTROL_OK)
    rc, out = run(binary, ["run", str(ok)])
    if rc != 0 or "control-ok" not in out:
        raise SystemExit(
            f"ABORTING — the positive control did not run (rc={rc}).\n"
            f"This gate cannot distinguish a broken example from a broken "
            f"runner, so it refuses to report a count.\n{out[:800]}"
        )
    bad = tmp / "control_panic.vr"
    bad.write_text(CONTROL_PANIC)
    rc, _ = run(binary, ["run", str(bad)])
    if rc == 0:
        raise SystemExit(
            "ABORTING — the negative control PASSED: a program that panics "
            "was reported as running cleanly. The runner is not observing "
            "the exit status, so a clean sheet from it would mean nothing."
        )
    print("controls: a clean program runs, a panicking program does not.  OK")


def homepage_blocks():
    """The marketing homepage's samples, which live outside `docs/`.

    `src/pages/index.tsx` is the page the owner calls the project's best,
    and it was outside every documentation gate until one was widened to
    reach it — a gate is defined by the FILES IT WALKS, not by the
    question it asks. `check_homepage_examples.py` already extracts these
    and already asks whether they COMPILE; this asks the next question of
    the same blocks rather than growing a third instrument.
    """
    try:
        from check_homepage_examples import HOMEPAGE, blocks as hp_blocks
    except Exception:
        return
    if not HOMEPAGE.is_file():
        return
    src = HOMEPAGE.read_text(encoding="utf-8", errors="replace")
    for _line, body in hp_blocks(src):
        yield HOMEPAGE, body


def blocks(funnel):
    """(page, body) for every self-contained example, in a stable order.

    `funnel` is filled in as we go, because a count is a statement about
    its DENOMINATOR: "one broken example" means nothing until the reader
    can see it came from 22 and not from 220, and each narrowing step is
    a place this instrument could be silently measuring nothing.
    """
    for page in sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx")):
        text = page.read_text(encoding="utf-8", errors="replace")
        text = _drop_unshipped(text)
        for body in BLOCK.findall(text):
            funnel["verum blocks"] += 1
            if "fn main" not in body:
                continue
            funnel["with an fn main"] += 1
            if MUST_FAIL.search(body):
                funnel["  minus shown-as-failing"] += 1
                continue
            if "..." in body or "…" in body:
                funnel["  minus elided"] += 1
                continue
            yield page, body
    for page, body in homepage_blocks():
        funnel["verum blocks"] += 1
        if "fn main" not in body:
            continue
        funnel["with an fn main"] += 1
        if MUST_FAIL.search(body) or "/* body */" in body:
            funnel["  minus shown-as-failing"] += 1
            continue
        if "..." in body or "…" in body:
            funnel["  minus elided"] += 1
            continue
        yield page, body


def key(page: Path, body: str) -> str:
    try:
        rel = page.relative_to(DOCS)
    except ValueError:
        rel = page.name          # the homepage lives outside docs/
    return f"{rel}#{hashlib.blake2s(body.encode()).hexdigest()[:8]}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--differential", action="store_true",
                    help="also build and run under AOT and compare the tiers")
    ap.add_argument("--limit", type=int, default=0,
                    help="stop after N examples (for a quick look, not for a gate)")
    args = ap.parse_args()

    if not DOCS.is_dir():
        print(f"docs directory not found: {DOCS} — set VERUM_DOCS_DIR", file=sys.stderr)
        return 0

    binary = verum_binary()
    print(f"binary: {binary}")

    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        controls(binary, tmp)

        funnel = collections.Counter()
        considered = compiled = 0
        broken = []
        diverged = []
        for i, (page, body) in enumerate(blocks(funnel)):
            if args.limit and i >= args.limit:
                break
            if ENVIRONMENTAL.search(body):
                funnel["  minus environmental"] += 1
                continue
            considered += 1
            work = tmp / f"ex{i}"
            work.mkdir(exist_ok=True)
            src = work / "ex.vr"
            src.write_text(body)
            if not compiles(binary, src):
                # Owned by check_doc_examples.py. Counting it here too
                # would make one defect look like two.
                continue
            compiled += 1
            rc0, out0 = run(binary, ["run", str(src)])
            if rc0 != 0:
                broken.append((key(page, body), signal_name(rc0), out0.strip()[-200:]))
                continue
            if not args.differential:
                continue
            rcb, _ = run(binary, ["build", str(src)], cwd=work, timeout=TIMEOUT_BUILD)
            exe = work / "target" / "release" / "ex"
            if rcb != 0 or not exe.is_file():
                broken.append((key(page, body), "AOT build failed", ""))
                continue
            rc1, out1 = run(exe, [], cwd=work)
            if rc1 != 0:
                broken.append((key(page, body), f"AOT {signal_name(rc1)}", out1.strip()[-200:]))
            elif out1.strip() != out0.strip():
                diverged.append((key(page, body), out0.strip()[:120], out1.strip()[:120]))

        print()
        for k in ("verum blocks", "with an fn main", "  minus shown-as-failing",
                  "  minus elided", "  minus environmental"):
            print(f"{k:36s}: {funnel[k]}")
        print(f"{'self-contained examples considered':36s}: {considered}")
        print(f"{'  of those, compiling':36s}: {compiled}")
        print(f"{'  compiled but do NOT run':36s}: {len(broken)}")
        if args.differential:
            print(f"  run but the TIERS DISAGREE        : {len(diverged)}")

        for k, why, tail in broken:
            print(f"\n  BROKEN  {k}\n          {why}")
            if tail:
                print(f"          {tail}")
        for k, a, b in diverged:
            print(f"\n  DIVERGE {k}\n          tier0: {a}\n          tier1: {b}")

        total = len(broken) + len(diverged)
        print(f"\ntotal: {total}   baseline: {BASELINE}")
        if total > BASELINE:
            print("A page teaches something that no longer works.", file=sys.stderr)
            return 1
        if total < BASELINE:
            print(f"Baseline is stale — lower BASELINE to {total}.", file=sys.stderr)
            return 1
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
