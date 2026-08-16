#!/usr/bin/env python3
"""Gate: enumerate free-function (name, arity) collisions across core/.

A collision is two `public fn` declarations that share a name AND an
arity but live in different modules. Bare-name resolution has to CHOOSE
between them, and the choice is not visible at the call site — that is
the root of the bare-name-collision class (T0220 and kin), not a
cosmetic duplication.

The gate is a RATCHET: it fails when the count rises above the frozen
baseline, and it fails when the count drops without the baseline being
lowered. A silently improving number is how a gate stops measuring —
lower the baseline in the same commit that earns it.

Usage:
    check_barename_collisions.py            # enumerate, human-readable
    check_barename_collisions.py --check    # ratchet, exit 1 on drift
    check_barename_collisions.py --scope sqlite   # only the sqlite/native
                                                  # boundary (T0538)
"""

from __future__ import annotations

import argparse
import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
CORE = ROOT / "core"
SQLITE_NATIVE = "database/sqlite/native"
PRELUDE_SOURCE = CORE / "mod.vr"

# Frozen counts, measured 2026-08-11. Lower them in the commit that earns
# it; never raise them without a recorded reason.
# 2026-08-16: 614 -> 615.  `digest/1` became a collision when sha384 and
# sha512 gained the free-function shorthand sha256 has carried since it
# was written (`public fn digest(data) -> [Byte; N]` next to the
# inherent `ShaN.digest`).  Counted deliberately: consumers already
# disambiguate by RENAMING on mount — `core/security/tuf/client.vr`
# writes `sha256.{digest}` beside `sha512.{digest as sha512_digest}` —
# and renaming a function resolves (renaming a `const` does not, which
# is a separate defect).  A family where one of three siblings carries
# the documented shorthand is worse than one counted collision.
BASELINE_ALL = 615
BASELINE_SQLITE = 84
# The PRELUDE scope — the subset a user meets without importing anything.
# This is not a stylistic count: for these names the ambiguity DECIDES which
# implementation runs, and 14 of the 20 prelude math functions currently
# resolve to SQLite's SQL builtins and answer `Relaxed` instead of a number
# (measured 2026-08-11 through `mount core.prelude.*`).
#
# THE NUMBER CHANGED MEANING on 2026-08-16, so it is not comparable with the
# 26 recorded before.  `prelude_named_exports` used to read only the
# explicitly named re-exports of `core/mod.vr`, and that file carries 15
# mount lines of which TWELVE are globs and three name a symbol (List / Map /
# Set).  Those three are TYPE names while this scope keys on
# `(function name, arity)`, so the scope was structurally incapable of
# reporting a collision — it printed "0 collisions [prelude]" against a
# baseline of 26 and read as a clean surface.  With the globs resolved the
# visible surface is 281 names and the honest count is 17.
BASELINE_PRELUDE = 17
# Same populations under the (name, arity, first-param type) key — the
# REUSE question. Duplicated WORK, not merely a shared verb.
BASELINE_ALL_TYPED = 297
# `--kind types`: two TOP-LEVEL type declarations — public OR private —
# sharing a SIMPLE NAME in different modules. Frozen at the measured count.
#
# Not a tidiness metric. One of these pairs cost a whole public function:
# `core/term/render/diff.vr` mounts `core.term.style.Modifier` EXPLICITLY, and
# `Modifier.BOLD` still resolved against the SQLite date grammar's unrelated
# `Modifier` sum — so `write_modifiers` shipped as a panic stub until the
# SQLite type was renamed `DateModifier`. Every remaining pair is the same
# shape, waiting for a resolution order to shift under it.
BASELINE_TYPES = 133
BASELINE_SQLITE_TYPED = 15

# `public fn name(args)` at column 0 — the free-function surface. Methods
# live inside `implement` blocks and are indented, so column-0 anchoring
# is what separates the two without parsing.
DECL = re.compile(r"^public fn (\w+)\s*\(([^)]*)\)")
# Any TOP-LEVEL type declaration, public or private. The compiler's
# layout registry is keyed by SIMPLE NAME and carries no visibility, so a
# private helper collides exactly as hard as a public type: two files each
# declaring a private `SinkInner` sent every field of both through the
# positional GUESS path (T0723). Counting only `public` measured the wrong
# set — this scope was widened after that case.
#
# ONE AUTHORITY for "is this line a type declaration, and what does it
# declare".  This file used to carry its own
# `re.compile(r"^(?:public\s+)?type\s+(\w+)")`, and a second copy of a
# rule is a second chance to disagree with the grammar: `(\w+)` captures
# the MODIFIER, so all 41 of core/'s `type affine …` / `type linear …`
# declarations were read as a type named `affine` or `linear`.  The gate
# duly reported a collision on the name `affine` across 36 files while
# losing the 41 real names.  `check_type_name_collisions.declares_a_type`
# reads the grammar's `type_def` production and is now the only place
# that decision is made.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from check_type_name_collisions import declares_a_type  # noqa: E402

# The two questions this script answers are NOT the same, and conflating
# them overstates the reuse problem threefold:
#
#   (name, arity)         — a RESOLUTION question. Bare-name resolution
#                           has to choose, so this is the collision count.
#   (name, arity, T0)     — a REUSE question. Same name over a DIFFERENT
#                           first-parameter type is a generic verb
#                           (`mode_name(&BeginMode)` vs
#                           `mode_name(&SecureMode)`), not duplicated work.
#
# Measured 2026-08-11: 614 by the first key, 297 by the second.
#
# THE TYPED KEY IS NOT PROOF OF DUPLICATION EITHER — checked on its own
# largest cluster, 2026-08-12. `is_known_only/1/Int64` appears in five
# sqlite modules and reads like five copies of one function. The bodies
# differ: each ANDs against a different flag mask
# (`f_nosavepoint()|f_invert()|f_ignorenoop()|f_fknoaction()` in
# changeset_apply_policy, `well_known_mask()` in open_v2_flags_api, and so
# on). Same verb, same carrier type, different DOMAIN — and the domain is
# distinguished by the MODULE, which no key over (name, arity, type) can
# see.
#
# So treat 297 as an upper bound on duplicated work, and READ THE BODIES
# before merging anything this script pairs up. Kin: register entry C9,
# where three math modules share 18 names and turned out to be three
# deliberate CONTRACTS (high-precision / zero-libc / correctly-rounded),
# not three copies.


def param_list(line: str) -> str | None:
    """The full parameter list of a `public fn` line, balanced.

    `([^)]*)` stops at the FIRST `)`, which is inside the parameter for
    any function-typed argument: `fn assert_panics(f: fn() -> Unit, msg:
    Text)` reads as the single parameter `f: fn(`.  37 of core/'s 10268
    column-0 `public fn` declarations take one, and each was keyed under
    the wrong arity — a `(name, arity)` ratchet cannot afford that.
    """
    i = line.find("(")
    if i < 0:
        return None
    depth = 0
    for j in range(i, len(line)):
        if line[j] == "(":
            depth += 1
        elif line[j] == ")":
            depth -= 1
            if depth == 0:
                return line[i + 1 : j]
    return None


def split_params(params: str) -> list[str]:
    """Split on TOP-LEVEL commas only.

    Two things nest inside a Verum parameter list and both carry commas:
    a generic argument (`Map<Text, Int>`) and a default value, which may
    be a string literal (`msg: Text = "expected panic, got none"`).
    Splitting on every comma counts those as extra parameters.
    """
    # `->` carries a `>` that is NOT a closing angle bracket.  Left in,
    # `f: fn() -> Unit, msg: Text` drops to depth -1 at the arrow and the
    # following comma stops counting as top level — the parameter list
    # reads as one parameter.  Blank the arrows first (same length, so
    # nothing else shifts).
    params = params.replace("->", "~~")
    out: list[str] = []
    depth = 0
    quote = ""
    cur = []
    for ch in params:
        if quote:
            cur.append(ch)
            if ch == quote:
                quote = ""
            continue
        if ch in "\"'":
            quote = ch
            cur.append(ch)
            continue
        if ch in "([{<":
            depth += 1
        elif ch in ")]}>":
            depth -= 1
        if ch == "," and depth == 0:
            out.append("".join(cur))
            cur = []
            continue
        cur.append(ch)
    out.append("".join(cur))
    return [p.strip() for p in out if p.strip()]


def arity(params: str) -> int:
    return len(split_params(params))


def module_public_surface(dotted: str, depth: int = 2) -> set[str]:
    """Public names a `core/` module exports, following its own re-exports.

    `dotted` is the path after `super.` — `base.panic` resolves to
    `core/base/panic.vr`, else `core/base/panic/mod.vr`.  Collected: its
    own column-0 `public fn` / type declarations, its braced and
    single-name re-exports, and (while `depth` lasts) the surface behind
    its own globs.  Bounded rather than complete: two hops is what the
    prelude actually uses, and an unbounded walk would make this script a
    module resolver.
    """
    if depth < 0:
        return set()
    rel = dotted.replace(".", "/")
    for cand in (CORE / f"{rel}.vr", CORE / rel / "mod.vr"):
        if cand.is_file():
            path = cand
            break
    else:
        return set()
    try:
        src = path.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return set()
    names: set[str] = set()
    for line in src.splitlines():
        m = DECL.match(line)
        if m:
            names.add(m.group(1))
        t = declares_a_type(line)
        if t:
            names.add(t)
    names |= mount_named_exports(src)
    # `public mount .sub.*;` / `public mount super.x.y.*;` inside the module.
    for g in re.finditer(r"public mount (\.?)([\w.]+)\.\*\s*;", src):
        sub = g.group(2)
        child = f"{dotted}.{sub}" if g.group(1) == "." else sub
        names |= module_public_surface(child, depth - 1)
    return names


def mount_named_exports(src: str) -> set[str]:
    """Explicitly named re-exports in one source: braced lists + singles."""
    names: set[str] = set()
    for block in re.finditer(r"public mount [\w.]*\{([^}]*)\}", src, re.S):
        body = re.sub(r"//[^\n]*", "", block.group(1))
        for name in re.split(r"[,\s]+", body):
            if re.fullmatch(r"[A-Za-z_]\w*", name):
                names.add(name)
    for one in re.finditer(r"public mount [\w.]*\.(\w+)\s*;", src):
        names.add(one.group(1))
    return names


SELF_TEST_ARITY = [
    # (declaration, expected arity) — every shape that broke a naive split
    ('public fn nothing() -> Int {', 0),
    ('public fn one(x: Int) -> Int {', 1),
    ('public fn walk_all(list: &StmtList, visit: fn(Int64)) {', 2),
    ('public fn assert_panics(f: fn() -> Unit, msg: Text = "a, b") {', 2),
    ('public fn get(m: &Map<Text, Int>, k: &Text) -> Maybe<Int> {', 2),
    ('public fn three(a: fn(Int) -> Int, b: Map<Text, List<Int>>, c: Text) {', 3),
    ('public fn dflt(a: Int, b: Text = "x, y, z") {', 2),
]


def self_test() -> int:
    """Check the parameter parser against the shapes that have broken it.

    Every defect this gate has had lived in an extractor and showed up
    only as a moved number: a `(\\w+)` that captured the `affine`
    MODIFIER instead of the type, a prelude scope that resolved no globs
    and so could never report anything, and `([^)]*)` stopping at the
    first `)` — which is INSIDE the parameter for any function-typed
    argument.  A ratchet is a number nobody argues with, so the parser
    behind it ships with its cases.
    """
    bad = 0
    for src, want in SELF_TEST_ARITY:
        got = arity(param_list(src) or "")
        if got != want:
            bad += 1
            print(f"FAIL arity {got} != {want}: {src}", file=sys.stderr)
    for src, want in (
        ("public type affine WalWriter is { a: Int };", "WalWriter"),
        ("    type Output = Result<T, E>;", None),
    ):
        got = declares_a_type(src)
        if got != want:
            bad += 1
            print(f"FAIL type {got!r} != {want!r}: {src}", file=sys.stderr)
    if bad:
        print(f"self-test: {bad} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(SELF_TEST_ARITY) + 2} extractor case(s) hold")
    return 0


def prelude_named_exports() -> set[str]:
    """Every name a user meets from `core/mod.vr` without importing.

    WHY THIS RESOLVES GLOBS NOW.  It used to read only the explicitly
    named re-exports and say so — "the prelude count is a LOWER BOUND".
    But `core/mod.vr` carries 15 mount lines of which TWELVE are globs
    (`public mount super.base.maybe.*;`) and only three name a symbol
    (List / Map / Set).  Those three are TYPE names, and this scope keys
    on `(function name, arity)`, so the lower bound was structurally
    ZERO: the gate could not report a collision no matter what core/
    contained, while printing "0 collisions [prelude]" as though the
    surface were clean.  A bound that cannot move is not a bound.
    """
    try:
        src = PRELUDE_SOURCE.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return set()
    names: set[str] = set()
    for g in re.finditer(r"public mount super\.([\w.]+)\.\*\s*;", src):
        names |= module_public_surface(g.group(1))
    # `public mount super.math.{sin, cos, ...};` — braced lists, possibly
    # spanning lines and carrying `//` comments between entries.
    for block in re.finditer(r"public mount super\.[\w.]+\.\{([^}]*)\}", src, re.S):
        body = re.sub(r"//[^\n]*", "", block.group(1))
        for name in re.split(r"[,\s]+", body):
            if re.fullmatch(r"[A-Za-z_]\w*", name):
                names.add(name)
    # `public mount super.io.print;` — single-name form.
    for one in re.finditer(r"public mount super\.[\w.]+\.(\w+);", src):
        names.add(one.group(1))
    return names


def collect_types() -> dict[tuple, set[str]]:
    """(type name,) -> modules declaring it as a `public type`."""
    found: dict[tuple, set[str]] = collections.defaultdict(set)
    for path in CORE.rglob("*.vr"):
        rel = path.relative_to(CORE).as_posix()
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for line in text.splitlines():
            name = declares_a_type(line)
            if name:
                found[(name,)].add(rel)
    return found


def collect(typed: bool = False) -> dict[tuple, set[str]]:
    """(name, arity[, first-param type]) -> modules declaring it."""
    found: dict[tuple, set[str]] = collections.defaultdict(set)
    for path in CORE.rglob("*.vr"):
        rel = path.relative_to(CORE).as_posix()
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for line in text.splitlines():
            m = DECL.match(line)
            if not m:
                continue
            full = param_list(line)
            params = split_params(full if full is not None else m.group(2))
            key: tuple
            if typed:
                first = params[0].split(":")[-1].strip() if params else "()"
                key = (m.group(1), len(params), first)
            else:
                key = (m.group(1), len(params))
            found[key].add(rel)
    return found


def collisions(found, scope: str) -> dict[tuple[str, int], set[str]]:
    out = {}
    prelude = prelude_named_exports() if scope == "prelude" else set()
    for key, modules in found.items():
        if len(modules) < 2:
            continue
        if scope == "prelude":
            # The name is reachable with no import at all, and more than one
            # module answers to it at the same arity — so which body runs is
            # decided by resolution order, invisibly, at every call site.
            if key[0] not in prelude:
                continue
        if scope == "sqlite":
            # Only the boundary this task is about: declared BOTH inside
            # sqlite/native and outside it.
            inside = any(m.startswith(SQLITE_NATIVE) for m in modules)
            outside = any(not m.startswith(SQLITE_NATIVE) for m in modules)
            if not (inside and outside):
                continue
        out[key] = modules
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="ratchet mode")
    ap.add_argument("--scope", choices=("all", "sqlite", "prelude"), default="all")
    ap.add_argument(
        "--kind",
        choices=("functions", "types"),
        default="functions",
        help="types: `public type` simple-name collisions across modules",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="check the extractors against hand-written cases and exit",
    )
    ap.add_argument(
        "--typed",
        action="store_true",
        help="key on (name, arity, first-param type): duplicated WORK, not a shared verb",
    )
    args = ap.parse_args()

    if getattr(args, "self_test", False):
        return self_test()

    if args.kind == "types":
        found = collect_types()
        coll = {k: v for k, v in found.items() if len(v) > 1}
        for (name,), mods in sorted(coll.items()):
            print(f"{name:28s} {', '.join(sorted(mods))}")
        print(f"\n{len(coll)} colliding type names, public or private [types]")
        if args.check and len(coll) != BASELINE_TYPES:
            direction = "rose above" if len(coll) > BASELINE_TYPES else "dropped below"
            print(
                f"RATCHET: public-type collisions {direction} the baseline "
                f"({len(coll)} vs {BASELINE_TYPES}). A rise adds a name whose "
                f"resolution is invisible at the use site; a drop must lower the "
                f"baseline in the same commit that earns it.",
                file=sys.stderr,
            )
            return 1
        return 0

    found = collect(typed=args.typed)
    coll = collisions(found, args.scope)
    if args.typed:
        if args.scope == "prelude":
            print(
                "--typed is not defined for the prelude scope: the question there is "
                "WHICH BODY RUNS for a bare name, and a differing first-parameter type "
                "does not make that unambiguous.",
                file=sys.stderr,
            )
            return 2
        baseline = BASELINE_SQLITE_TYPED if args.scope == "sqlite" else BASELINE_ALL_TYPED
    elif args.scope == "prelude":
        baseline = BASELINE_PRELUDE
    else:
        baseline = BASELINE_SQLITE if args.scope == "sqlite" else BASELINE_ALL
    count = len(coll)

    if not args.check:
        for key, modules in sorted(coll.items()):
            print("/".join(str(k) for k in key))
            for m in sorted(modules):
                print(f"    {m}")
        print(f"\n{count} colliding (name, arity) pairs [{args.scope}]")
        return 0

    if count > baseline:
        print(
            f"REGRESSION: {count} (name, arity) collisions [{args.scope}], "
            f"baseline {baseline}.",
            file=sys.stderr,
        )
        for key, modules in sorted(coll.items())[:10]:
            label = "/".join(str(k) for k in key)
            print(f"  {label}: {', '.join(sorted(modules))}", file=sys.stderr)
        return 1

    if count < baseline:
        print(
            f"BASELINE STALE: {count} collisions [{args.scope}], baseline "
            f"{baseline}. Lower it in this commit — a gate whose baseline "
            f"drifts above reality stops measuring.",
            file=sys.stderr,
        )
        return 1

    print(f"OK: {count} collisions [{args.scope}], at baseline.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
