#!/usr/bin/env python3
"""Fail when a `.vr` file calls through a module path whose callee does not exist.

WHY THIS EXISTS
---------------
A call whose receiver is a MODULE PATH is not fully resolved by the
compiler.  Measured on 2026-08-18 (T0806), with a positive control in the
same batch for every line:

    m.absent()            two segments, real module       E100    caught
    "x".absent_method()   missing method on a value       E400    caught
    p.absent_field        missing field on a record       E404    caught
    p.sub.deep.absent()   three segments FROM A VALUE     E404    caught
    fn f(a: Int); f()     local call, wrong arity         E102    caught

    a.b.absent()          three segments, missing LEAF    silent -> nil
    a.zzz.f()             three segments, missing MIDDLE  silent -> nil
    m.f()                 module call, wrong arity        silent -> nil

Nothing diagnoses those three at any stage: the call evaluates to `nil`,
and `nil` satisfies whatever return type was declared.  A typo in a
qualified call is therefore not a compile error but a wrong VALUE, which is
the worst shape a defect can take.

Five of the thirteen calls this gate found are now FIXED, and they were the
live defects rather than dead code:

    core/security/tuf/role_verify.vr    time.rfc3339.to_epoch(expires)
    core/security/sigstore/verify.vr    the same, twice
    core/net/h3/client.vr               core.net.dns.resolve_first(&host)
    core/net/quic/api/client.vr         the same

`core/time/rfc3339.vr` declares no `to_epoch`; `Rfc3339Time` carries a
`unix_seconds` field, which is what all three callers wanted. TUF metadata
expiry had been comparing an Int against nil, reached from the public
`check_not_expired_targets`. `core/net/dns.vr` declares `resolve` and
`resolve_async` and no `resolve_first`; `resolve_async` already pairs each
address with the port, so the callers take the first entry of its list.

Both DNS files checked CLEAN before the fix and clean after — the dead call
produced no diagnostic at all, which is the shape this gate exists for.

WHAT THIS GATE CHECKS — AND WHAT IT DELIBERATELY DOES NOT
---------------------------------------------------------
It reports a module-rooted call whose LEAF NAME is declared nowhere in
`core/`.  That criterion is deliberately coarse, and it is coarse in the
safe direction: a name that appears in no declaration anywhere cannot be
reached by any re-export, however long the chain.

The tempting refinement — "is the leaf declared in the module the path
names?" — is NOT sound to automate here.  Re-export chains run deeper than
two hops and take a braced form as well as a glob:

    public mount runtime.*;
    public mount runtime.time.{ … };

A two-hop follower reported 25 missing names against `core/`, of which
num_cpus, monotonic_nanos and spawn_with_env are provably reachable through
exactly those lines in `core/intrinsics/mod.vr`.  So this gate stays with
the criterion it can defend, and the wrong-module class — a call landing on
a real name in the WRONG module — is out of its reach by construction.

Value-rooted receivers (`self.x.y()`, a local variable) are excluded: the
compiler checks those, and including them turns 459 real candidates into
10244 mostly-irrelevant ones.
"""

from __future__ import annotations

import re
import sys
from collections import defaultdict
from pathlib import Path

CORE = Path(__file__).resolve().parents[2] / "core"

# `root.seg.…​.leaf(` with at least three segments; roots are lower-case, so
# a type-qualified static (`List.new`) is not matched.
CALL = re.compile(r"\b((?:[a-z_][a-z0-9_]*)(?:\.[a-z_][a-z0-9_]*){2,})\s*\(")
# THE SAME CALL WITH A TYPE IN THE MIDDLE — `core.io.file.File.read_to_text(p)`.
# `CALL` above requires every segment to be lower-case, so it stops at
# `core.io.file` (not followed by `(`) and never sees the leaf. That blind
# spot is not hypothetical: `core/security/x509/trust_store.vr:213` called
# `core.io.file.File.read_to_text`, which nothing declares — the free
# function is `read_to_string` and it is not on the type — and the stub it
# became panicked at run time through `TrustStore.system()` and therefore
# through `ClientOptions.with_system_trust()` in the H3 client (T1316).
# Measured when this pattern was added: 57 call sites of this shape reach a
# module root, and exactly one had a leaf declared nowhere — the one above.
TYPED_CALL = re.compile(
    r"\b((?:[a-z_][a-z0-9_]*)(?:\.[a-z_][a-z0-9_]*)+"
    r"\.[A-Z][A-Za-z0-9_]*\.[a-z_][a-z0-9_]*)\s*\(")
MOUNT = re.compile(r"^\s*(?:public\s+)?mount\s+([A-Za-z_][\w.]*)")
DECL = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)")

# Roots that always name a module rather than a value.
ALWAYS_MODULE_ROOTS = {"core", "super", "cog"}

# THE COUNT BECAME A ROSTER, 2026-09-09 (T1320).  `BASELINE = 7` said how
# many dead calls were tolerated and never WHICH, so the one shape it could
# not report was the SWAP: repoint one call, introduce another, and the
# total is still 7 and the gate still prints `none new`.  Measured on a
# scratch tree the same day against the sibling gate
# (`check_platform_call_parity.py`, identical construction): with the
# population held at one and its membership replaced, the count ratchet
# printed `[ok] … none new` — a sentence that was false as written.
#
# The turnover is not hypothetical either.  Two entries left this
# population today (`thread_yield`, both platforms, e88d25fd4) and the
# gate's whole reaction was to demand a smaller number — a request to edit
# the INSTRUMENT, which is the opposite of what a ratchet is for.
#
# The key is `(dotted-path, file)` and carries NO line number: a roster
# keyed on positions goes red when an unrelated edit above shifts a line,
# which teaches the reader to re-baseline without looking.
#
# The five that remain, and why each is still open:
#
#   core.time.rfc3339.format_iso8601_basic   core/storage/s3/client.vr
#   core.runtime.env.random_u8               core/net/weft/tracing.vr
#   core.shell.stream.stream_lines           core/shell/command.vr
#   sys.windows.time.query_performance_counter_ns  core/mem/segment.vr
#   sys.windows.thread.thread_join           core/runtime/thread.vr
#
# The last two are Windows-only and also on the roster of
# `check_platform_call_parity.py`, which reaches them from the other side;
# `thread_join` needs its call FORM changed to a method
# (`core/sys/windows/thread.vr:281` declares `join`), not a new
# declaration.  See T1320.
KNOWN: set[tuple[str, str]] = {
    ("core.time.rfc3339.format_iso8601_basic", "core/storage/s3/client.vr"),
    ("core.runtime.env.random_u8", "core/net/weft/tracing.vr"),
    ("core.shell.stream.stream_lines", "core/shell/command.vr"),
    ("sys.windows.time.query_performance_counter_ns", "core/mem/segment.vr"),
    ("sys.windows.thread.thread_join", "core/runtime/thread.vr"),
}


def compare(
    found: set[tuple[str, str]],
    roster: set[tuple[str, str]],
) -> tuple[list, list]:
    """Split what the tree has against what the roster claims.

    Separated from the scan so a control can drive it without a tree —
    the half the count ratchet had, and never tested."""
    return sorted(found - roster), sorted(roster - found)


def module_roots(text: str) -> set[str]:
    """Roots that name a module in this file: the always-module ones plus
    every alias a `mount` introduces (both its first and last segment)."""
    roots = set(ALWAYS_MODULE_ROOTS)
    for line in text.splitlines():
        m = MOUNT.match(line)
        if m:
            segments = m.group(1).split(".")
            roots.add(segments[0])
            roots.add(segments[-1])
    return roots


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    sources = sorted(CORE.rglob("*.vr"))
    if not sources:
        print(f"check-dead-module-path-calls: no .vr files under {CORE}", file=sys.stderr)
        return 2

    texts = {path: path.read_text(errors="ignore") for path in sources}

    declared: set[str] = set()
    for text in texts.values():
        declared.update(DECL.findall(text))

    findings: dict[tuple[str, str], list[str]] = defaultdict(list)
    for path, text in texts.items():
        roots = module_roots(text)
        rel = str(path.relative_to(CORE.parent))
        for lineno, line in enumerate(text.splitlines(), 1):
            if line.lstrip().startswith("//"):
                continue
            for match in list(CALL.finditer(line)) + list(TYPED_CALL.finditer(line)):
                dotted = match.group(1)
                if dotted.split(".")[0] not in roots:
                    continue
                leaf = dotted.rsplit(".", 1)[1]
                if leaf not in declared:
                    findings[(dotted, rel)].append(f"{rel}:{lineno}")

    total = sum(len(sites) for sites in findings.values())
    appeared, disappeared = compare(set(findings), KNOWN)
    bad = bool(appeared) or bool(disappeared)

    if "--list" in sys.argv or bad:
        stream = sys.stderr if bad else sys.stdout
        print(
            f"check-dead-module-path-calls: {total} module-path call(s) name a "
            f"callee declared nowhere in core/ ({len(KNOWN)} on the roster).\n"
            "The compiler does not diagnose these: each evaluates to `nil` and\n"
            "satisfies whatever return type the caller declared.\n",
            file=stream,
        )
        for key in sorted(findings, key=lambda k: (k[0] not in {a[0] for a in appeared}, k)):
            dotted, _ = key
            mark = "NEW " if key in set(appeared) else "    "
            print(f"  {mark}{dotted}()", file=stream)
            for site in findings[key]:
                print(f"          {site}", file=stream)

    if appeared:
        print(
            "\nThe call(s) marked NEW are not on the roster in this file.\n"
            "Repoint each at the real callee, or declare it. If a call is\n"
            "legitimately unreachable-by-name, deleting it is the honest fix —\n"
            "it does nothing today except return nil. If it has to stay, add it\n"
            "to KNOWN with the reason.",
            file=sys.stderr,
        )
        return 1

    if disappeared:
        print(
            "check-dead-module-path-calls: the roster claims call(s) the tree no "
            "longer has —\n"
            + "".join(f"  {dotted}()  {rel}\n" for dotted, rel in disappeared)
            + "Remove them from KNOWN in this file; the population shrank and the\n"
            "roster has to say so by name, not by a smaller number.",
            file=sys.stderr,
        )
        return 1

    print(
        f"check-dead-module-path-calls: {total} known dead call(s), roster exact"
    )
    return 0


def self_test() -> int:
    """Known answers for both call shapes, because a widened pattern that
    swallows real calls is worse than the blind spot it removes."""
    bad = 0
    cases = [
        # (line, pattern, expected dotted matches)
        ("core.time.rfc3339.to_epoch(x)", CALL, ["core.time.rfc3339.to_epoch"]),
        ("core.io.file.File.read_to_text(p)", TYPED_CALL,
         ["core.io.file.File.read_to_text"]),
        # The lower-case pattern must NOT half-match the typed form: it
        # stops at `core.io.file`, which is not followed by `(`.
        ("core.io.file.File.read_to_text(p)", CALL, []),
        # A type-qualified static is not a module path and never was.
        ("List.new()", CALL, []),
        ("List.new()", TYPED_CALL, []),
        # Two segments is not enough for either.
        ("m.absent()", CALL, []),
        ("m.absent()", TYPED_CALL, []),
        # A generic on the way through must not be read as a segment.
        ("core.a.b.Type.method(x)", TYPED_CALL, ["core.a.b.Type.method"]),
    ]
    for line, pattern, want in cases:
        got = [m.group(1) for m in pattern.finditer(line)]
        if got != want:
            name = "CALL" if pattern is CALL else "TYPED_CALL"
            print(f"self-test: {name} on {line!r} gave {got}, wanted {want}")
            bad += 1
    # THE SWAP, which is the shape the count ratchet could not report and
    # the reason this gate carries a roster.  Population size is 1 in both
    # polarities; the membership differs.  A count is satisfied by both.
    before = {("sys.windows.thread.thread_join", "core/runtime/thread.vr")}
    after = {("sys.windows.thread.thread_detach", "core/runtime/thread.vr")}
    appeared, disappeared = compare(after, before)
    if not appeared or not disappeared:
        print(
            "self-test: a swap of equal size reported nothing — the roster "
            "comparison has degenerated back into a count"
        )
        bad += 1
    if compare(before, before) != ([], []):
        print("self-test: an unchanged population reported a difference")
        bad += 1

    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


if __name__ == "__main__":
    raise SystemExit(main())
