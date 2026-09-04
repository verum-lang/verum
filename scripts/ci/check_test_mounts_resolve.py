#!/usr/bin/env python3
"""Ratchet on `mount` statements in core-tests that name something core/ never declares.

WHAT THIS CANNOT SEE.  It checks names that ARE mounted.  A file that
never mounts a module at all has no statement to check, and every use of
those names is an `unbound variable` the compiler catches and this gate
does not.  Measured 2026-09-04: core-tests/base/env/integration_test.vr
mounted `core.prelude.*` and `core.collections.list.List` and used seven
names from `core.base.env` — 15 errors, invisible here.  Closing that
would mean resolving every free identifier, which is name resolution,
not a grep.

WHY.  Five conformance tests were found broken this way in one afternoon
(2026-09-04): the stdlib was reshaped and the test kept the old spelling.

    core-tests/security/zk/halo2/circuit  mount ColumnType   -> CircuitColumnType
    core-tests/async/intrinsics           mount Executor     -> ExecutorHandle
    core-tests/async/executor             mount RuntimeConfig-> never existed
    core-tests/math/distributed           ReduceOp.Avg       -> .Mean
    core-tests/net/tls13                  (annotations)

One unresolved name fails the WHOLE file, so a single stale mount hides
every test in it — halo2 reported 35 errors from one bad name.  Nothing
was checking, because a rename lands in core/ and nobody greps
core-tests.

WHAT THIS IS NOT.  It is not a resolver.  A name counts as present if the
module file declares it (type / fn / const / module), lists it as a
variant of a sum type, or re-exports it via its own `mount`.  That is
approximate in the safe direction — over-accepting — and the residue is
frozen in a baseline.  The ratchet is the point: a NEW unresolved name
fails, the existing backlog does not.
"""
import re
import sys
import pathlib
import collections

REPO = pathlib.Path(__file__).resolve().parents[2]
CORE, TESTS = REPO / "core", REPO / "core-tests"
KNOWN = pathlib.Path(__file__).with_name("test_mounts_known_unresolved.txt")

MOUNT_BRACE = re.compile(r"^\s*mount\s+([a-z_][\w.]*)\s*\.\s*\{([^}]*)\}\s*;", re.M | re.S)
MOUNT_ONE = re.compile(r"^\s*mount\s+([a-z_][\w.]*)\.([A-Za-z_]\w*)\s*;", re.M)
DECL = re.compile(
    r"^\s*(?:pub\s+|public\s+)?(?:type|fn|const|module|async\s+fn|unsafe\s+fn)\s+"
    r"(?:affine\s+|linear\s+)?([A-Za-z_]\w*)",
    re.M,
)
# A sum type's VARIANTS are declared by the type, not by a `type`/`fn` line.
# Without this, `Some` / `None` / `Ok` / `Err` read as unresolved and the
# census reported 455 files instead of 164.
TYPEBODY = re.compile(r"^\s*(?:pub\s+|public\s+)?type\s+\w+(?:<[^>]*>)?\s+is\b(.*?);", re.M | re.S)
CAPWORD = re.compile(r"\b([A-Z]\w*)\b")
REEXPORT = re.compile(r"\bmount\b[^;]*?\b(\w+)\b", re.S)
# A `mount { … }` list may carry line comments between its entries.  The
# first version split the brace body on commas and treated the prose as
# names, so a comment about `memcpy_addr` produced entries like
# "and the bare names now belong" — and because the fragments varied
# between runs the ratchet was not even stable.  A name is an identifier
# or it is not a name.
IDENT = re.compile(r"^[A-Za-z_]\w*$")
LINE_COMMENT = re.compile(r"//[^\n]*")

_cache: dict[str, set[str]] = {}


def module_names(path: str) -> set[str] | None:
    """Names `core.a.b` provides, or None when no such file exists."""
    if path in _cache:
        return _cache[path] or None
    parts = [p for p in path.split(".") if p]
    if parts and parts[0] == "core":
        parts = parts[1:]
    base = CORE.joinpath(*parts) if parts else CORE
    files = [f for f in (base.with_suffix(".vr"), base / "mod.vr") if f.exists()]
    names: set[str] = set()
    for f in files:
        t = f.read_text(errors="replace")
        names |= set(DECL.findall(t))
        names |= set(REEXPORT.findall(t))
        for body in TYPEBODY.findall(t):
            names |= set(CAPWORD.findall(body))
    _cache[path] = names
    return names or None


def unresolved() -> dict[str, list[str]]:
    out: dict[str, list[str]] = collections.defaultdict(list)
    for tf in sorted(TESTS.rglob("*_test.vr")):
        t = tf.read_text(errors="replace")
        items = [
            (m.group(1), LINE_COMMENT.sub("", m.group(2)).split(","))
            for m in MOUNT_BRACE.finditer(t)
        ]
        items += [(m.group(1), [m.group(2)]) for m in MOUNT_ONE.finditer(t)]
        for mod, names in items:
            have = module_names(mod)
            if have is None:
                continue  # module path not on disk — a different defect
            for raw in names:
                n = raw.strip().rstrip(",")
                if not IDENT.match(n):
                    continue
                if n not in have:
                    out[str(tf.relative_to(REPO))].append(f"{mod}.{n}")
    return out


def controls() -> bool:
    """Both poles, run before any number is believed."""
    maybe = module_names("core.base.maybe") or set()
    ok = True
    if not ({"Some", "None"} <= maybe):
        print("control FAILED: Some/None do not resolve in core.base.maybe", file=sys.stderr)
        ok = False
    if "ZzNoSuchNameEver" in maybe:
        print("control FAILED: a bogus name resolved", file=sys.stderr)
        ok = False
    tensor = module_names("core.math.tensor") or set()
    if "Mean" not in tensor:  # a VARIANT, not a declaration — the 455-vs-164 case
        print("control FAILED: variant `Mean` not seen in core.math.tensor", file=sys.stderr)
        ok = False
    return ok


def pairs(u: dict[str, list[str]]) -> list[str]:
    return sorted(f"{f}\t{n}" for f, ns in u.items() for n in ns)


def main() -> int:
    if not controls():
        print("REFUSING to report: a control failed", file=sys.stderr)
        return 2
    u = unresolved()
    now = set(pairs(u))
    if "--write-baseline" in sys.argv:
        KNOWN.write_text(
            "\n".join(
                [
                    "# `mount` names in core-tests that core/ does not appear to declare.",
                    "# Approximate by design — see the module docstring.  A line REMOVED",
                    "# is a test repaired or a false positive eliminated; a line ADDED is",
                    "# a test mounting a name the stdlib no longer has, and one such name",
                    "# fails the whole file.",
                    "#",
                    "# Generated by: scripts/ci/check_test_mounts_resolve.py --write-baseline",
                ]
                + sorted(now)
            )
            + "\n"
        )
        print(f"[ok] baseline written: {len(now)} pair(s) over {len(u)} file(s)")
        return 0
    known = set()
    if KNOWN.exists():
        known = {
            ln.rstrip("\n")
            for ln in KNOWN.read_text().splitlines()
            if ln.strip() and not ln.startswith("#")
        }
    new = sorted(now - known)
    gone = sorted(known - now)
    if gone:
        print(f"[ok] {len(gone)} mount(s) now resolve — re-record with --write-baseline")
    if new:
        print(f"\n[fail] {len(new)} core-tests mount(s) name something core/ does not declare:")
        for line in new[:20]:
            f, n = line.split("\t", 1)
            print(f"    {n}\n        mounted by {f}")
        if len(new) > 20:
            print(f"    … and {len(new) - 20} more")
        print(
            "\nOne unresolved name fails the WHOLE file, so this hides every test\n"
            "in it.  Either the stdlib was renamed and the test was not, or the\n"
            "name never existed.  Fix the test, or re-record the baseline in a\n"
            "commit that says why."
        )
        return 1
    print(f"[ok] test-mount ratchet holds: {len(now)} known unresolved, none new")
    return 0


if __name__ == "__main__":
    sys.exit(main())
