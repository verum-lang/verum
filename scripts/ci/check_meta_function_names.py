#!/usr/bin/env python3
"""An `@name(...)` in EXPRESSION position that the compiler does not know
types as `Unit` — silently, with only a parser warning.

Root, one line, `crates/verum_types/src/infer/expr.rs`:

    _ => Type::unit(),

the catch-all of the match on a meta-function name.  Upstream of it the
parser (`crates/verum_fast_parser/src/expr.rs`) pushes an `E0410`
*warning* for a name outside `KNOWN_META_FUNCTIONS` and then builds
`ExprKind::MetaFunction` anyway, so nothing refuses.  Downstream, the
expression has type `Unit`: a zero-byte object.

What that cost, measured: `core/io/file.vr` built its `fstat(2)` buffer
with `unsafe { @zeroed() }`.  `@zeroed` is not a meta-function, so the
buffer was `Unit`, so `File.size()` handed the FFI a zero-byte object and
returned `size ERR {Other, None}`; a direct field read on the same buffer
panicked with `field index 11 ... exceeds object data size 0
type_id=16777230 type='?'`.  Data size 0 is the signature of `Unit`, not
of a mis-laid struct — reading it as a layout defect sent one session
after `@repr(C)`, which was the wrong fix.

WHY A GATE AND NOT THE WARNING.  `verum check` against a `core/` carrying
three live `@zeroed()` calls returns rc=0 and prints ZERO `E0410` lines:
the stdlib path swallows the parser's attribute warnings entirely.  The
defect is invisible to every other instrument in the tree.

TWO ROSTERS, BOTH READ FROM THE COMPILER, never transcribed:

  * `KNOWN_META_FUNCTIONS` in the parser — what does not warn.
  * the arms of the meta-function match in type inference — what actually
    gets a type.

A name must miss BOTH to be a violation.  Keying on the parser roster
alone paints five correct sites red (`@size_of`, `@align_of`, `@asm`
warn but are typed), and a false plus costs more than a false minus.

TWO SHAPES REACH VALUE POSITION, and the second was a blind spot in this
gate's first draft:

  (A) the meta-call is not the first token on its line — an argument, a
      `let` initialiser, an operand.
  (B) the meta-call IS first on its line and the next non-blank line
      starts with `}` — it is the TAIL of a block, so its `Unit` is the
      block's value, and for a function body that is the return value.
      A declaration attribute is never followed by `}`: there would be
      nothing left for it to attribute.  Checked against every such site
      in `core/` — all of them are `@unreachable` or `@builtin_*`, none
      a plausible attribute.

Shape (B) is the same failure `check-cfg-block-tail` (T0805) gates for
`@cfg` blocks, with a different producer.  Sixteen `@builtin_*` sites in
`core/math/` are function bodies consisting of one unknown meta-call, so
each of those functions returns `Unit` whatever its signature says.

DECLARATION ATTRIBUTES ARE NOT THIS.  `@derive`, `@inline`, `@ffi`,
`@thread_local` and 75 other names reach the *attribute* parser, which
has its own list; there are 4000+ such occurrences in `core/`.  The
discriminator is position: a declaration attribute is the FIRST token on
its line.  Anything else is an expression.

KNOWN LIMITATION, with its exposure measured and enumerated in FULL —
there are TWO doors, not one, and naming half an exposure is the failure
this gate keeps finding elsewhere. The variant exclusion matches only a
BARE `|` before the name, so both of these read as expressions:

  * a compact declaration putting two attributed variants on ONE line,
    `type X is | @a(0) A | @b(1) B;` — the SECOND attribute;
  * a FIRST variant carrying an attribute with NO leading pipe,
        type X is
            @a(0) A
            | @a(1) B;
    which `variant_list = [ '|' ] , variant , { '|' , variant }`
    (grammar/verum.ebnf:793) permits, because the leading pipe is
    optional and `variant = { attribute } , identifier , …` (:826) puts
    the attribute before the name.

Neither shape occurs in `core/` — measured at zero for both. And for the
first door: across every `.vr` file exactly ONE `@name` is preceded by
a prefix ending in `|`, and it is a lambda body —
`core/math/autodiff.vr:217`, `let pullback = |seed| @vbc(...)`. Widening
the rule to "the prefix ends with `|`" would silence that lambda, which is
a genuine expression position, so the narrow rule is the correct one here
and the limitation is recorded rather than traded away.

SCOPE IS `core/`, and that is measured rather than convenient: across
every tracked `.vr` file the population is 345 sites, of which 310 are in
`vcs/` — a conformance suite where an unknown meta-function can be a
deliberate test input — and exactly 35 are in `core/`, the shipped
standard library.  Widening to `vcs/` would need each of those 310
triaged first.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
PARSER = REPO / "crates/verum_fast_parser/src/expr.rs"
INFER = REPO / "crates/verum_types/src/infer/expr.rs"
CORE = REPO / "core"

# Roster keyed on IDENTITY (the name), never on line numbers: a file that
# grows by ten lines must not move this table, and a name that is SWAPPED
# for another must not pass by keeping the total.
KNOWN: dict[str, int] = {
    "bitcast": 3,
    "stack_alloc": 3,
    "builtin_path_app": 2,
    "syscall": 2,
    "transport_roundtrip": 2,
    "builtin_absurd": 1,
    "builtin_ap": 1,
    "builtin_apd": 1,
    "builtin_construct_sheaf_topos": 1,
    "builtin_glue": 1,
    "builtin_hcomp": 1,
    "builtin_i0": 1,
    "builtin_i1": 1,
    "builtin_interval": 1,
    "builtin_interval_eq": 1,
    "builtin_interval_join": 1,
    "builtin_interval_meet": 1,
    "builtin_interval_rev": 1,
    "builtin_path": 1,
    "builtin_refl": 1,
    "builtin_sigma_path": 1,
    "builtin_sym": 1,
    "builtin_theory_category": 1,
    "builtin_trans": 1,
    "builtin_transport": 1,
    "builtin_unglue": 1,
    "const_slot_for": 1,
    "frame_address": 1,
    "llm_oracle": 1,
    "zero": 1,
}
# Derived, never written beside the table: a hand-typed total has drifted
# from its own roster before.
BASELINE = sum(KNOWN.values())

STRING = re.compile(r'"(?:[^"\\]|\\.)*"')
META = re.compile(r"@([A-Za-z_][A-Za-z0-9_]*)")
# The whole line is one meta-call and nothing else.
ALONE = re.compile(r"^\s*@[A-Za-z_][A-Za-z0-9_]*\s*(\(.*\))?\s*;?\s*$")

# Floors for the two extractions.  An instrument that cannot find its
# input must get STRICTER, not softer: a silent empty roster would make
# every name "known" and this gate would go green on a broken tree.
MIN_PARSER_NAMES = 20
MIN_INFER_NAMES = 10


def parser_roster() -> set[str]:
    m = re.search(
        r"const KNOWN_META_FUNCTIONS: &\[&str\] = &\[(.*?)\n\];",
        PARSER.read_text(),
        re.S,
    )
    if not m:
        return set()
    return set(re.findall(r'"([A-Za-z_][A-Za-z0-9_]*)"', m.group(1)))


def inference_roster() -> set[str]:
    lines = INFER.read_text().splitlines()
    start = None
    for i, line in enumerate(lines):
        if "let ExprKind::MetaFunction { name, args } = &expr.kind" in line:
            start = i
            break
    if start is None:
        return set()
    names: set[str] = set()
    for line in lines[start : start + 420]:
        if re.match(r'^            "', line):
            names |= set(re.findall(r'"([a-z_0-9]+)"', line.split("=>")[0]))
    return names


def scan(root: pathlib.Path, accepted: set[str]) -> dict[str, list[str]]:
    """Expression-position `@name` occurrences whose name is in neither roster."""
    found: dict[str, list[str]] = {}
    for path in sorted(root.rglob("*.vr")):
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        src = text.splitlines()
        for lineno, line in enumerate(src, 1):
            if line.lstrip().startswith("//"):
                continue
            code = STRING.sub('""', line.split("//")[0])
            for m in META.finditer(code):
                if m.group(1) in accepted:
                    continue
                before = code[: m.start()].strip()
                # A VARIANT attribute in a sum-type declaration:
                #     public type GpioMode is
                #         | @value(0b0000) Input
                #         | @value(0b0001) OutputPushPull
                # `variant_list = [ '|' ] , variant , { '|' , variant }`
                # (grammar/verum.ebnf:793). The only other production that
                # opens with `|` is `lambda_expr = '|' , param_list_lambda ,
                # '|' , expression`, whose params are bare identifiers, so a
                # `@` cannot appear there — the exclusion is exact, not a
                # heuristic. Ten sites in core/ have this shape (`@value` x9
                # in sys/bitfield.vr, `@default` in runtime/supervisor.vr)
                # and every one of them was a FALSE POSITIVE in this gate's
                # first roster.
                if before == "|":
                    continue
                if before == "":
                    # First token on the line. A declaration attribute —
                    # UNLESS the call is ALONE on its line and the block
                    # closes right after it, in which case the call is the
                    # block's tail and its Unit is the block's value.
                    #
                    # "Alone on its line" is load-bearing: `@inline public
                    # fn fail() -> ExitCode { ... }` and `@bits(4) reserved:
                    # UInt8,` are attributes whose declaration sits on the
                    # SAME line, and both can be followed by a closing
                    # brace. Without this clause the gate paints them red.
                    if not ALONE.match(code):
                        continue
                    nxt = next(
                        (
                            l
                            for l in src[lineno:]
                            if l.strip() and not l.lstrip().startswith("//")
                        ),
                        "",
                    )
                    # A bare `}` or `},` closes a BLOCK (function body, match
                    # arm). `};` closes a type declaration, so what preceded
                    # it was a field, not a tail.
                    if not re.match(r"\}\s*,?\s*$", nxt.strip()):
                        continue
                rel = path.relative_to(REPO) if path.is_relative_to(REPO) else path
                found.setdefault(m.group(1), []).append(f"{rel}:{lineno}")
    return found


def self_test() -> int:
    """Pin the detector: it must fire on the shape it exists for, stay
    silent on the shape it must not paint, and know a declaration
    attribute from an expression."""
    import tempfile

    accepted = {"cfg", "size_of", "derive"}
    cases = [
        ("let x: T = unsafe { @nosuch() };", True, "expression meta-call"),
        ("    let n = @size_of(T);", False, "accepted name"),
        ("@derive(Debug)", False, "declaration attribute, first token"),
        ("// let x = @nosuch();", False, "comment line"),
        ('    print("@nosuch()");', False, "inside a string literal"),
        ("    let y = @cfg(a) { @nosuch(1) };", True, "nested in an accepted call"),
        ("    | @nosuch(0b01) Medium", False, "variant attribute after a bare pipe"),
    ]
    # Shape (B) needs two lines, so it is pinned separately.
    multi = [
        ("    @nosuch()\n}", True, "block tail: next line closes the block"),
        ("    @nosuch(x)\n},", True, "block tail of a match arm"),
        ("@nosuch(x)\npublic fn f() -> Int { 1 }", False, "attribute: a declaration follows"),
        ("    @size_of(T)\n}", False, "block tail with an accepted name"),
        ("    @nosuch public fn f() -> Int { 1 }\n}", False,
         "attribute with its declaration on the SAME line"),
        ("    @nosuch(4) field: UInt8,\n};", False, "field attribute before a type close"),
    ]
    failures = 0
    with tempfile.TemporaryDirectory() as td:
        for i, (src, want, why) in enumerate(cases + multi):
            d = pathlib.Path(td) / f"c{i}"
            d.mkdir()
            (d / "m.vr").write_text(src + "\n")
            got = bool(scan(d, accepted))
            if got != want:
                print(f"  SELF-TEST FAIL ({why}): want fire={want} got={got} :: {src}")
                failures += 1
    if failures:
        print(f"[FAIL] detector self-test: {failures} case(s)")
        return 2
    print(f"[ok] detector self-test: {len(cases) + len(multi)} cases")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true", help="pin the detector and exit")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    rc = self_test()
    if rc:
        return rc

    parser_names = parser_roster()
    infer_names = inference_roster()
    if len(parser_names) < MIN_PARSER_NAMES:
        print(
            f"[FAIL] KNOWN_META_FUNCTIONS yielded {len(parser_names)} names "
            f"(< {MIN_PARSER_NAMES}) — the parser roster moved or "
            f"{PARSER.relative_to(REPO)} changed shape. Refusing to judge."
        )
        return 2
    if len(infer_names) < MIN_INFER_NAMES:
        print(
            f"[FAIL] the inference match yielded {len(infer_names)} names "
            f"(< {MIN_INFER_NAMES}) — {INFER.relative_to(REPO)} changed shape. "
            f"Refusing to judge."
        )
        return 2

    found = scan(CORE, parser_names | infer_names)
    total = sum(len(v) for v in found.values())

    new_names = sorted(set(found) - set(KNOWN))
    gone_names = sorted(set(KNOWN) - set(found))
    moved = sorted(
        n for n in set(found) & set(KNOWN) if len(found[n]) != KNOWN[n]
    )

    print(
        f"meta-function names accepted by the compiler: "
        f"{len(parser_names)} parser + {len(infer_names)} inference "
        f"= {len(parser_names | infer_names)} distinct"
    )
    print(f"untyped expression meta-calls in core/: {total} (roster {BASELINE})")

    if new_names:
        print(f"\n[FAIL] {len(new_names)} name(s) NOT in the roster — each types as Unit:")
        for n in new_names:
            for loc in found[n][:6]:
                print(f"    @{n} at {loc}")
            if len(found[n]) > 6:
                print(f"    ... and {len(found[n]) - 6} more")
        print(
            "\n  A meta-function the compiler does not know is not a lint: the\n"
            "  expression is Unit, a zero-byte object. Spell it as something\n"
            "  that exists, or add it to both the parser roster and the\n"
            "  inference match."
        )
        return 1

    if moved:
        print(f"\n[FAIL] {len(moved)} roster name(s) changed count — new sites appeared:")
        for n in moved:
            print(f"    @{n}: roster {KNOWN[n]}, found {len(found[n])}")
            for loc in found[n]:
                print(f"        {loc}")
        return 1

    if gone_names:
        print(f"\n[ok] {len(gone_names)} roster name(s) fixed since the baseline: "
              f"{', '.join('@' + n for n in gone_names)}")
        print("  Lower the roster in this file to lock the win in.")

    print(f"\n[ok] no new untyped meta-calls in core/ ({total} known)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
