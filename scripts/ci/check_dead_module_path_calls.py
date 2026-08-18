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
MOUNT = re.compile(r"^\s*(?:public\s+)?mount\s+([A-Za-z_][\w.]*)")
DECL = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)")

# Roots that always name a module rather than a value.
ALWAYS_MODULE_ROOTS = {"core", "super", "cog"}

# Known count at the time the gate landed; the gate ratchets DOWNWARD only.
BASELINE = 8


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
    sources = sorted(CORE.rglob("*.vr"))
    if not sources:
        print(f"check-dead-module-path-calls: no .vr files under {CORE}", file=sys.stderr)
        return 2

    texts = {path: path.read_text(errors="ignore") for path in sources}

    declared: set[str] = set()
    for text in texts.values():
        declared.update(DECL.findall(text))

    findings: dict[str, list[str]] = defaultdict(list)
    for path, text in texts.items():
        roots = module_roots(text)
        for lineno, line in enumerate(text.splitlines(), 1):
            if line.lstrip().startswith("//"):
                continue
            for match in CALL.finditer(line):
                dotted = match.group(1)
                if dotted.split(".")[0] not in roots:
                    continue
                leaf = dotted.rsplit(".", 1)[1]
                if leaf not in declared:
                    rel = path.relative_to(CORE.parent)
                    findings[leaf].append(f"{rel}:{lineno}  {dotted}()")

    total = sum(len(sites) for sites in findings.values())
    if total > BASELINE:
        print(
            f"check-dead-module-path-calls: {total} module-path calls name a "
            f"callee declared nowhere in core/ (baseline {BASELINE}).\n"
            "The compiler does not diagnose these: each evaluates to `nil` and\n"
            "satisfies whatever return type the caller declared.\n",
            file=sys.stderr,
        )
        for leaf in sorted(findings, key=lambda k: (-len(findings[k]), k)):
            print(f"  {leaf}", file=sys.stderr)
            for site in findings[leaf]:
                print(f"      {site}", file=sys.stderr)
        print(
            "\nRepoint each call at the real callee, or declare it. If a call is\n"
            "legitimately unreachable-by-name, deleting it is the honest fix —\n"
            "it does nothing today except return nil.",
            file=sys.stderr,
        )
        return 1

    if total < BASELINE:
        print(
            f"check-dead-module-path-calls: {total} found, below the baseline of "
            f"{BASELINE} — lower BASELINE to {total} so the ground stays held."
        )
        return 1

    print(f"check-dead-module-path-calls: {total} known dead calls, none new")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
