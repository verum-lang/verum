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
import contextlib
import io
import os
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
# 617 since T0822: `ctx_push_frame/0` and `ctx_pop_frame/0` are now declared in BOTH
# core/sys/darwin/tls.vr and core/sys/windows/tls.vr.  That is the tree's established
# shape for platform code — `get_context_slots/0` is same-named in all three platform
# modules, and 59 of the collisions counted here have ALL their participants under
# core/sys/<platform>/ — and the hazard this gate names does not apply to them: they
# are never called by bare name (common.vr calls them qualified, `super.darwin.tls.…`)
# and they sit behind mutually exclusive @cfg arms, so bare-name resolution is never
# asked to CHOOSE between them.
BASELINE_ALL = 617
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
BASELINE_TYPES = 132
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
    # THE ROSTER HALVES, both pinned by a defect they actually had.
    bad = 0

    # 1. `want=None` means THE EXPECTATION IS ABSENT and must REFUSE.
    #    An earlier draft fell back to the default roster here and
    #    compared 132 colliding type names against 617 colliding
    #    function names — rc=1 where the honest answer is rc=2. The
    #    polarity run caught it; this keeps it caught.
    buf = io.StringIO()
    with contextlib.redirect_stderr(buf):
        absent_rc = check_membership({("x", 0): {"a.vr"}}, None, "<absent>")
    if absent_rc != 2:
        print("self-test: an ABSENT expectation must refuse (rc=2), not diff "
              f"— got rc={absent_rc}")
        print(buf.getvalue().rstrip()[:400])
        bad += 1

    # 2. `scope_expectation` must FILTER, not echo. A row whose files are
    #    all outside sqlite/native has no business in the sqlite scope.
    probe = {
        "only_outside/1": {"base/a.vr", "text/b.vr"},
        "spans_boundary/1": {"base/a.vr", SQLITE_NATIVE + "/c.vr"},
    }
    derived = {
        "/".join(str(k) for k in key): mods
        for key, mods in collisions(
            {(lab.rpartition("/")[0], int(lab.rpartition("/")[2])): f
             for lab, f in probe.items()}, "sqlite").items()
    }
    if set(derived) != {"spans_boundary/1"}:
        print(f"self-test: the sqlite derivation is not filtering — got "
              f"{sorted(derived)}")
        bad += 1

    if bad:
        print(f"self-test: {bad} FAILED")
        return 1
    print(f"[ok] self-test: {len(SELF_TEST_ARITY) + 2} extractor case(s) hold, "
          "plus the roster refusal and the scope derivation")
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


# A free function whose name is also a METHOD name is a third collision
# axis, and neither of the two above sees it.  `--kind functions` keys on
# `^public fn` — column-anchored and public-only — so it counts neither
# private helpers nor methods at all.
#
# The axis is not hypothetical.  `core/database/sqlite/native/alter/engine.vr`
# declares a private `fn push(dst: &mut Text, s: &Text)` and calls it; the
# call resolves to `Text.push(&mut self, ch: Char)` instead and reports
# "push expects 1 argument(s), got 2" — 78 such errors across four files,
# none of them visible to this gate (T0798).
#
# Measured when this mode was added: 638 public and 138 private free
# functions carry a name that some method also carries.  Behind several of
# them sits plain duplication — 12 declarations of `push_text` in 6 distinct
# bodies, 6 of `append_text` in 2, 8 of `read_u32_be` in 6 — and behind
# `is_digit`, 12 declarations in EIGHT distinct bodies, which is divergence
# rather than copying.
#
# Reported, not ratcheted.  A ratchet needs a number someone can act on in
# one commit, and this one is a research surface until the resolution defect
# it exposes is settled; freezing it now would only make the next honest
# measurement fail the build.  Same reasoning as `check-rings-census`.
METHOD_DECL = re.compile(
    r"^\s*(?:public\s+|pub\s+)?(?:pure\s+|async\s+|unsafe\s+|meta(?:\(\d+\))?\s+|cofix\s+)*"
    r"fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(\s*&?\s*(?:mut\s+)?self\b"
)
FREE_DECL_ANY_VIS = re.compile(
    r"^(?:public\s+|pub\s+)?(?:pure\s+|async\s+|unsafe\s+|meta(?:\(\d+\))?\s+|cofix\s+)*"
    r"fn\s+(\w+)\s*(?:<[^>]*>)?\s*\("
)


def method_axis() -> int:
    """Census: free functions whose name is also declared as a method."""
    methods: dict[str, set[str]] = collections.defaultdict(set)
    free_pub: dict[str, list[str]] = collections.defaultdict(list)
    free_priv: dict[str, list[str]] = collections.defaultdict(list)
    for path in CORE.rglob("*.vr"):
        rel = path.relative_to(CORE).as_posix()
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            m = METHOD_DECL.match(line)
            if m:
                methods[m.group(1)].add(rel)
                continue
            if line[:1].isspace():
                continue
            m = FREE_DECL_ANY_VIS.match(line)
            if m:
                bucket = free_pub if line.startswith(("public ", "pub ")) else free_priv
                bucket[m.group(1)].append(f"{rel}:{lineno}")

    clash_pub = {n: v for n, v in free_pub.items() if n in methods}
    clash_priv = {n: v for n, v in free_priv.items() if n in methods}
    print("free functions whose name is also a method name:")
    print(f"  public:  {len(clash_pub)}")
    print(f"  private: {len(clash_priv)}   (invisible to --kind functions)")
    print()
    ranked = sorted(
        list(clash_pub.items()) + list(clash_priv.items()),
        key=lambda kv: -len(kv[1]),
    )[:12]
    for name, sites in ranked:
        print(f"  {name:<18} {len(sites):>2} free decl(s), method in {len(methods[name])} file(s)")
        print(f"       {sites[0]}")
    return 0


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


# The roster paths are overridable so a POLARITY CONTROL can travel the
# gate's exact route without mutating a tracked file — the control that
# runs beside the subject instead of through it is the one that lies.
MEMBERSHIP = pathlib.Path(
    os.environ.get("VERUM_BARENAME_MEMBERSHIP")
    or pathlib.Path(__file__).with_name("barename_collision_membership.txt"))
# `--kind types` asks a DIFFERENT question of a DIFFERENT population —
# two top-level TYPE declarations sharing a simple name — so its key is
# `(name,)` and not `(name, arity)`. One roster describes one population;
# this one gets its own file rather than a scope column in the first.
MEMBERSHIP_TYPES = pathlib.Path(
    os.environ.get("VERUM_BARENAME_TYPES_MEMBERSHIP")
    or pathlib.Path(__file__).with_name("barename_collision_types_membership.txt"))

# THE SCOPES DO NOT NEED THEIR OWN ROSTERS, and that is a property of
# `collisions()` rather than a convenience: it is a PURE FILTER on
# (name, modules) — `prelude` keeps names the prelude exports, `sqlite`
# keeps names declared on both sides of the sqlite/native boundary.
# Every input it reads is already stored in the `all` roster, so the
# sqlite and prelude expectations are DERIVED from it. A separate sidecar
# for each would be a second copy of the same facts, free to disagree.
#
# What the derivation does NOT cover, stated rather than discovered
# later: the prelude's own export list is read live, so a name entering
# or leaving the prelude moves BOTH sides together and shows up as a
# COUNT change, not as membership movement. The roster describes
# collisions; it does not describe the prelude.

# WHY A ROSTER AND NOT ONLY A COUNT.
#
# The count baselines below answer "did the population grow". They cannot
# answer "did its MEMBERSHIP change", and a swap of equal size prints
# `OK: 617 collisions [all], at baseline.` A file that stops colliding and
# another that starts are two separate events, and the one that starts is
# the one worth reading.
#
# The roster lives in a sidecar rather than inline because 617 entries would
# be most of this file — the tree already keeps baselines that way
# (`panic_surface_baseline.txt`, `core_compile_known_failures.txt`).
#
# It reports WHICH names moved, not merely that something did. A digest
# alone would say "something changed" and reproduce, one level up, the
# problem this exists to close.


def membership_lines(coll: dict) -> list[str]:
    """`name/arity<TAB>file,file,...`, sorted, one line per colliding key."""
    out = []
    for key, modules in sorted(coll.items()):
        label = "/".join(str(k) for k in key)
        out.append(f"{label}\t{','.join(sorted(modules))}")
    return out


def read_membership(path: pathlib.Path = None) -> dict[str, set[str]] | None:
    path = path or MEMBERSHIP
    if not path.is_file():
        return None
    got: dict[str, set[str]] = {}
    for line in path.read_text().splitlines():
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        label, _, files = line.partition("\t")
        got[label] = set(f for f in files.split(",") if f)
    return got


def scope_expectation(scope: str) -> dict[str, set[str]] | None:
    """The sqlite / prelude expectation, DERIVED from the `all` roster by
    the same filter the live side goes through."""
    want = read_membership(MEMBERSHIP)
    if want is None:
        return None
    as_coll: dict[tuple[str, int], set[str]] = {}
    for label, files in want.items():
        name, _, arity = label.rpartition("/")
        if not arity.isdigit():
            continue
        as_coll[(name, int(arity))] = files
    return {
        "/".join(str(k) for k in key): mods
        for key, mods in collisions(as_coll, scope).items()
    }


def check_membership(coll: dict, want: dict[str, set[str]] | None = None,
                     source: str = None) -> int:
    """Compare the live membership against the roster. Absent roster is a
    REFUSAL, not a pass: an instrument that cannot find its input gets
    stricter."""
    # NO FALLBACK HERE, and the reason is a defect this very function
    # shipped for one polarity run: `want=None` means THE EXPECTATION IS
    # ABSENT, and reading the default roster instead compared 132 colliding
    # TYPE names against 617 colliding FUNCTION names — rc=1 with a
    # 749-line diff where the honest answer was rc=2, "your roster is
    # missing". Every caller names its own expectation.
    source = source or MEMBERSHIP.name
    if want is None:
        print(
            f"REFUSING TO PASS: {source} is missing. The count baseline "
            f"alone is blind to a swap of equal size. Regenerate it with "
            f"--write-membership in a commit that says why.",
            file=sys.stderr,
        )
        return 2
    have = {
        "/".join(str(k) for k in key): set(mods) for key, mods in coll.items()
    }
    new = sorted(set(have) - set(want))
    gone = sorted(set(want) - set(have))
    moved = sorted(k for k in set(have) & set(want) if have[k] != want[k])
    if not (new or gone or moved):
        print(f"OK: membership matches the roster ({len(have)} names).")
        return 0
    if new:
        print(f"MEMBERSHIP: {len(new)} name(s) newly colliding:", file=sys.stderr)
        for k in new[:10]:
            print(f"  + {k}: {', '.join(sorted(have[k]))}", file=sys.stderr)
    if gone:
        print(f"MEMBERSHIP: {len(gone)} name(s) no longer colliding:", file=sys.stderr)
        for k in gone[:10]:
            print(f"  - {k}", file=sys.stderr)
    if moved:
        print(
            f"MEMBERSHIP: {len(moved)} name(s) kept their count but CHANGED FILES "
            f"— the swap a count cannot see:",
            file=sys.stderr,
        )
        for k in moved[:10]:
            print(f"  ~ {k}", file=sys.stderr)
            print(f"      was: {', '.join(sorted(want[k]))}", file=sys.stderr)
            print(f"      now: {', '.join(sorted(have[k]))}", file=sys.stderr)
    print(
        "\n  Every line above is a real change to which bodies a bare name can "
        "reach.\n  Regenerate with --write-membership in the commit that earns it.",
        file=sys.stderr,
    )
    return 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="ratchet mode")
    ap.add_argument("--scope", choices=("all", "sqlite", "prelude"), default="all")
    ap.add_argument(
        "--kind",
        choices=("functions", "types", "methods"),
        default="functions",
        help=(
            "types: `public type` simple-name collisions across modules; "
            "methods: free functions whose name is also a method (census, never fails)"
        ),
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
    ap.add_argument(
        "--write-membership",
        action="store_true",
        help=(
            "regenerate the membership roster from the tree. Run it in the same "
            "commit that earns the change, never to silence a failure you have "
            "not read."
        ),
    )
    args = ap.parse_args()

    if getattr(args, "self_test", False):
        return self_test()

    if args.kind == "methods":
        return method_axis()

    if args.kind == "types":
        found = collect_types()
        coll = {k: v for k, v in found.items() if len(v) > 1}
        if args.write_membership:
            MEMBERSHIP_TYPES.write_text("\n".join([
                "# Membership roster for check_barename_collisions.py "
                "--check --kind types.",
                "# One line per colliding SIMPLE TYPE NAME: the name, a TAB, then",
                "# every module that declares a top-level type by it. The count",
                "# baseline answers 'did the population grow'; this answers 'did",
                "# its membership change' — a rename that trades one collision",
                "# for another holds the count and moves these lines.",
                "# Regenerate with: --check --kind types --write-membership",
            ] + membership_lines(coll)) + "\n")
            print(f"wrote {MEMBERSHIP_TYPES.name}: {len(coll)} names")
            return 0
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
        if args.check:
            # The count agreed — the state in which a SWAP is invisible.
            return check_membership(coll, read_membership(MEMBERSHIP_TYPES),
                                    MEMBERSHIP_TYPES.name)
        return 0

    found = collect(typed=args.typed)
    coll = collisions(found, args.scope)

    if args.write_membership:
        if args.scope != "all" or args.typed:
            print(
                f"--write-membership is not defined for --scope {args.scope}"
                f"{' --typed' if args.typed else ''}: the sqlite and prelude "
                "expectations are DERIVED from the `all` roster by the same "
                "filter the live side uses, so writing them would be a second "
                "copy of the same facts, free to disagree. Regenerate "
                f"{MEMBERSHIP.name} instead.",
                file=sys.stderr,
            )
            return 2
        header = [
            "# Membership roster for check_barename_collisions.py --check.",
            "# One line per colliding (name, arity): the name, a TAB, then every",
            "# file that declares it. The count baselines in the script answer",
            "# 'did the population grow'; this answers 'did its membership change',",
            "# which a count of equal size cannot.",
            "# Regenerate with: check_barename_collisions.py --check --write-membership",
        ]
        MEMBERSHIP.write_text("\n".join(header + membership_lines(coll)) + "\n")
        print(f"wrote {MEMBERSHIP.name}: {len(coll)} names")
        return 0
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

    # The count agreed. That is exactly the state in which a SWAP is
    # invisible, so the membership question is asked HERE and not earlier.
    if args.typed:
        # The typed axis carries counts alone, and deliberately: the
        # Makefile runs `--check`, `--scope sqlite`, `--scope prelude` and
        # `--kind types`, never `--typed`. Its baselines document a
        # measurement rather than gate one, and a roster nothing runs is a
        # file that rots. Say so instead of shipping it.
        return 0
    if args.scope == "all":
        return check_membership(coll, read_membership(MEMBERSHIP), MEMBERSHIP.name)
    return check_membership(
        coll, scope_expectation(args.scope),
        f"{MEMBERSHIP.name} (filtered to --scope {args.scope})")


if __name__ == "__main__":
    sys.exit(main())
