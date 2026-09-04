#!/usr/bin/env python3
"""Split `no method named X found for type Y` diagnostics by CAUSE.

A census says what is AT RISK; only a per-instance measurement says what
is BROKEN. This tool does the second thing for the largest hidden class
in T1061's strict sweep — 560 diagnostics over 131 types and 152 methods
— by asking three separable questions of each one.

    LEG 1  the type's name is declared MORE THAN ONCE in core/
           -> the bake aliases a method descriptor under the bare
              `{Type}.{method}` key, first-wins BETWEEN UNRELATED TYPES,
              and the loser's method vanishes from the archive (T0458).
              The file is honest; the archive lost the method.

    INTERCEPT  the method name is answered from Rust before the archive
           is consulted (method_dispatch.rs). A "not found" here is
           about DISPATCH, not about metadata, and no bake fix can move
           it.

    ABSENT  the method is declared NOWHERE in core/. The call names
           something that was never written (T1061's `append_byte`
           class). Nothing about the archive is implicated.

    UNEXPLAINED  none of the above. This is the interesting bucket and
           it is the reason the tool prints it separately rather than
           folding it into the largest neighbour.

USAGE
    classify_no_method.py <diagnostics-file> [--repo DIR]

where <diagnostics-file> is the raw stderr+stdout of one or many
`verum check` runs.

TWO TRAPS THIS TOOL IS BUILT AGAINST, both paid for today:

  * `grep 'fn append_byte'` matches `fn append_bytes`, which is how a
    genuinely absent method reads as present. Every declaration probe
    here is ANCHORED with a word boundary and an opening paren.
  * a type-declaration census that keys on the wrong token reports a
    type that is in plain sight as declared zero times. The declaration
    pattern below is checked against a known-present control
    (`Text`, which must come back as exactly 1) before any row is
    printed; if the control fails the tool refuses to classify.
"""

import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

# The Text methods answered from Rust in method_dispatch.rs, so the
# archive never gets a turn. Supplied by the census that measured them;
# a name here cannot be a metadata problem.
INTERCEPTED = {
    "can_read", "can_write", "capabilities", "capacity", "char_len",
    "contains", "ends_with", "epoch", "hash_value", "is_empty",
    "is_null", "is_valid", "len", "parse_bool", "parse_float",
    "parse_int", "parse_int_radix", "raw_ptr", "read", "remove_prefix",
    "remove_suffix", "starts_with", "strip_prefix", "strip_suffix",
    "to_float", "to_int", "to_lowercase", "to_uppercase", "trim",
    "trim_end", "trim_end_matches", "trim_matches", "trim_start",
    "trim_start_matches", "try_to_float", "try_to_int", "write",
}

DIAG = re.compile(r"no method named `([^`]+)` found for type `([^`]+)`")

# LEG TWO. The bake's TYPE slot is separately last-wins, so when one name
# denotes two different IDEAS the archive keeps whichever it saw last and
# anything written against the other stops reducing. It does not show up
# as a missing method — it shows up as the type having the wrong SHAPE:
#
#   cannot index type 'Fd' — only tuple types support .0
#       core/shell/resources.vr:56   public type Fd is { … }     RECORD
#       core/sys/io_engine.vr:203    public type Fd is (Int32);  NEWTYPE
#
#   Type mismatch: expected 'Output<ReadyFuture<Int>>', found 'Int'
#       core/async/future.vr:60      ReadyFuture<T> is { … }     RECORD
#       core/async/select.vr:823     ReadyFuture<T> is Select…   ALIAS
#
# A method-key fix cannot move these, so counting them with leg one would
# make a correct patch look partial.
LEG2 = [
    re.compile(r"cannot index type '([^']+)'"),
    # CAPTURE THE TYPE, NOT THE FIELD. The real diagnostic is
    #     field 'kind' not found on type 'Finding'. Available members: [...]
    # and the first version of this pattern took `kind`, then counted
    # `type kind is` declarations — zero, always — so an entire class
    # fell out of leg two silently.
    #
    # This face is the DANGEROUS one: the message names a field AND
    # lists alternatives, which reads as a rename and invites fixing
    # the caller. When the type name is declared twice with disjoint
    # fields, that "fix" compiles — against the wrong type. Measured
    # by verum-6c: `Finding` (integrity_walker_api vs cli/verify) and
    # `ColumnMeta` (schema_cache vs mysql/binlog_rows), both live in
    # one hour, beside `TbsCertificate` which really HAD been renamed
    # and was a singleton. One grep separates them.
    re.compile(r"field '[^']+' not found on type '([^']+)'"),
    re.compile(r"expected '(?:Output<)?([A-Za-z_][A-Za-z0-9_]*)"),
]


def rg(pattern: str, repo: Path) -> int:
    """Count matching lines under core/. grep, not python-walk: the tree
    is 2561 files and this runs once per distinct name."""
    try:
        out = subprocess.run(
            ["grep", "-rE", "--include=*.vr", pattern, "core/"],
            cwd=repo, capture_output=True, text=True, timeout=120,
        )
        return len([l for l in out.stdout.splitlines() if l.strip()])
    except (OSError, subprocess.SubprocessError):
        return -1


# THE AUTHORITY, not my own regex. scripts/ci/check_type_name_collisions.py
# carries `declares_a_type`, written against the grammar, and it knows
# something no pattern of mine did: `type` introduces THREE different
# things in Verum and only one is a nominal declaration —
#
#     type Point is { … };        a type_def, declares `Point`
#     type Item;                  an associated type inside a protocol
#     type Alias is Other;        (and aliases, which are declarations)
#
# My own pattern counted associated-type lines as declarations AND was
# blind to `type Foo<T> is`. The peer's census had the identical blind
# spot and undercounted duplicated names 113 vs 132. Importing beats
# re-deriving; the controls below still gate the import, because an
# authority can be broken too.
_GATE = None


def _gate(repo: Path):
    global _GATE
    if _GATE is None:
        import importlib.util
        path = repo / "scripts/ci/check_type_name_collisions.py"
        spec = importlib.util.spec_from_file_location("collisions_gate", path)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        _GATE = mod
    return _GATE


def type_decl_count(name: str, repo: Path) -> int:
    """How many .vr files under core/ DECLARE a nominal type of this name."""
    try:
        g = _gate(repo)
    except Exception as exc:
        print(f"PROBE-FAILED: cannot load the collisions gate: {exc}")
        raise SystemExit(2)
    n = 0
    for f in (repo / "core").rglob("*.vr"):
        try:
            for line in f.read_text(encoding="utf-8", errors="replace").splitlines():
                if g.declares_a_type(line) == name:
                    n += 1
        except OSError:
            continue
    return n


def method_decl_count(name: str, repo: Path) -> int:
    # `fn foo(` anchored: `fn append_byte(` must NOT match `fn append_bytes(`.
    return rg(rf"\bfn\s+{re.escape(name)}\s*\(", repo)


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        return 2
    repo = Path(".")
    if "--repo" in sys.argv:
        repo = Path(sys.argv[sys.argv.index("--repo") + 1])

    # CONTROL FIRST. A declaration probe that silently matches nothing
    # would classify every type as a singleton and every method as
    # absent — a tidy, entirely wrong table.
    control = type_decl_count("Text", repo)
    if control != 1:
        print(f"PROBE-FAILED: `type Text is` counted {control} times, expected 1.")
        print("The declaration pattern does not match this tree; refusing to classify.")
        return 2
    # A GENERIC control, because the non-generic one passed while the
    # pattern was blind to `type Foo<T> is`.
    control_g = type_decl_count("Maybe", repo)
    if control_g < 1:
        print(f"PROBE-FAILED: `type Maybe<T> is` counted {control_g}, expected >=1.")
        print("The pattern does not see GENERIC declarations; refusing to classify.")
        return 2
    control_m = method_decl_count("ends_with_char", repo)
    if control_m < 1:
        print(f"PROBE-FAILED: `fn ends_with_char(` counted {control_m}, expected >=1.")
        return 2
    print(f"controls OK  (Text={control}  Maybe<T>={control_g}  fn ends_with_char(={control_m})\n")

    text = Path(args[0]).read_text(encoding="utf-8", errors="replace")
    pairs = DIAG.findall(text)
    if not pairs:
        print("no `no method named` diagnostics in that file — nothing to classify.")
        print("(That is a statement about the INPUT, not about the tree.)")
        return 0

    tcache: dict[str, int] = {}
    mcache: dict[str, int] = {}
    buckets: Counter[str] = Counter()
    rows: list[tuple[str, str, str, int, int]] = []

    for method, ty in pairs:
        base = ty.split("<")[0].strip()
        if base not in tcache:
            tcache[base] = type_decl_count(base, repo)
        if method not in mcache:
            mcache[method] = method_decl_count(method, repo)
        td, md = tcache[base], mcache[method]

        if method in INTERCEPTED:
            b = "INTERCEPT"
        elif td > 1:
            b = "LEG1-dup-type"
        elif md == 0:
            b = "ABSENT"
        else:
            b = "UNEXPLAINED"
        buckets[b] += 1
        rows.append((b, ty, method, td, md))

    # LEG TWO, counted separately and never folded into leg one: a
    # method-key fix cannot move a wrong-SHAPE diagnostic, so mixing the
    # two would make a correct patch read as partial.
    leg2_hits: Counter[str] = Counter()
    for rx in LEG2:
        for name in rx.findall(text):
            base = name.split("<")[0].strip()
            if base not in tcache:
                tcache[base] = type_decl_count(base, repo)
            if tcache[base] > 1:
                leg2_hits[base] += 1

    total = sum(buckets.values())
    print(f"{total} diagnostics, {len(set(pairs))} distinct (type, method) pairs\n")
    for b, n in buckets.most_common():
        print(f"  {b:<16} {n:>5}   {100*n/total:>5.1f}%")
    if leg2_hits:
        print("LEG TWO (wrong SHAPE, not a missing method — a method-key")
        print("fix cannot move these; the bare TYPE slot is last-wins):")
        for name, n in leg2_hits.most_common(10):
            print(f"  {name:<24} {n:>4}   declared {tcache[name]}x in core/")
        print()
    else:
        print("LEG TWO: no shape diagnostics naming a duplicated type name.")
        print("(An absence — read it only if the input contained such")
        print(" diagnostics at all.)")
        print()
    print("UNEXPLAINED is the bucket to read — it is neither a duplicate")
    print("type, nor an intercepted name, nor a method that was never")
    print("written, so it is a fourth cause nobody has characterised.")
    print()
    seen = set()
    for b, ty, method, td, md in rows:
        if b != "UNEXPLAINED":
            continue
        k = (ty, method)
        if k in seen:
            continue
        seen.add(k)
        print(f"  {ty}.{method}    type-decls={td} method-decls={md}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
