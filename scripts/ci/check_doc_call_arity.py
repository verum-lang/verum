#!/usr/bin/env python3
"""Gate: a documented `Type.method(...)` call must pass an argument count
the declaration can accept.

THE HOLE. Four sibling gates ask whether a documented name EXISTS —
`check_doc_method_names` (uppercase receiver), `check_doc_methods_declared`
(the name anywhere in core/), `check_doc_receiver_methods` (the method on
a proven receiver's type), `check_doc_type_shapes` (a type's fields). None
asks whether the CALL could run. `Shared.strong_count(&s)` names a real
method on a real type and cannot be written that way: the method takes
`&self` and no arguments.

Arity is the cheapest divergence detector there is — it needs no type
inference, no receiver resolution, and no build — and it catches the
whole class of documentation transliterated from another language's
calling convention (Rust's `Rc::strong_count(&rc)` is the recurring
one).

WHAT IS COMPARED, and every narrowing here exists because leaving it out
produced a false accusation on a page that was right:

  * ONLY a method declared EXACTLY ONCE across core/. `Type.method`
    resolved by bare name is the wrong-table trap; two declarations of
    one name are two different methods and the doc may mean either.
  * ONLY when the owning type is declared `public`. `stdlib/mesh.md`
    documents a `Node` of its own; core/ has a PRIVATE `Node` in
    `core/net/weft/router.vr` and nothing else. Comparing them accuses
    a correct page of an arity it never claimed.
  * A RANGE, not a number. Verum has default parameter values
    (`fn debug_assert(condition: Bool, message: Text = "…")`,
    `core/intrinsics/control.vr:133`), so the declaration accepts
    `required..=total`. A call short of `total` is legal whenever the
    tail carries defaults.
  * a leading `self` / `&self` / `&mut self` parameter is dropped,
    because `x.method(a)` writes one argument for a two-parameter
    declaration.
  * a call whose argument text contains `...` or `…` is ELIDED on
    purpose and is skipped.
"""
from __future__ import annotations

import collections
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
DOCS = REPO / "internal" / "website" / "docs"

BASELINE = 0

IMPL = re.compile(r"(?:^|\n)[ \t]*implement\s*(?:<[^>]*>)?\s*(.+?)\s*\{")
FN = re.compile(r"\bfn\s+([a-z_]\w*)")
PUBLIC_TYPE = re.compile(r"(?:^|\n)[ \t]*(?:public|pub)\s+type\s+([A-Z]\w*)", re.M)
# A CONTEXT is a second declaration surface for the same bare name, and
# its methods have their own arity. `core/context/standard.vr:275`
# declares `public context Database { fn execute(sql, params) }` while
# `core/database/.../database.vr:190` declares a `Database` TYPE whose
# `execute` takes one argument — so three pages writing the injected
# form were accused of an arity that belongs to the other Database.
# A name carrying both surfaces is ambiguous and is dropped.
CONTEXT_DECL = re.compile(
    r"(?:^|\n)[ \t]*(?:public|pub)\s+context\s+([A-Z]\w*)", re.M)
BLOCK = re.compile(r"```verum\n(.*?)```", re.S)
# A DECLARATION is not a call. Reference pages list signatures as
# `public fn DynamicTable.insert(&mut self, name: Text, value: Text)`,
# and counting `&mut self` as an argument accused eight correct lines on
# one page. The `fn` (with optional visibility / async / meta prefixes)
# is what separates the two.
CALL = re.compile(
    r"(?<!\w)(?<!fn )(?<!fn  )([A-Z]\w*)\.([a-z_]\w*)\s*\(")
DECL_LINE = re.compile(
    r"^[ \t]*(?:public\s+|pub\s+)?(?:async\s+|unsafe\s+|meta\s+|pure\s+)*fn\s")
# A COUNTER-EXAMPLE is written to be wrong. `reference/lint-rules.md`
# shows `let c = Channel.new();  // fires` directly under the rule that
# forbids it, and the next line shows the fix. The comment on the line
# is the page saying so; accusing it would ask the page to delete its
# own teaching.
COUNTER_EXAMPLE = re.compile(
    r"//[^\n]*\b(fires|wrong|bad|refused|rejected|does not compile|"
    r"will not compile|error|counter-example|not shipped|would take|"
    r"do not|don't|does not exist|no such)\b", re.I)
# A self parameter is the WHOLE token — `self`, `&self`, `&mut self`.
# `\b` after `self` also matches the dot in `&self.items`, and that
# made the gate drop a real argument and accuse `cookbook/tui.md`'s
# correct `SelectableList.new(&self.items)` of passing none.
SELF_PARAM = re.compile(
    r"^&?\s*(?:mut\s+|unsafe\s+|checked\s+)*self\s*$")


def brace_body(text: str, open_idx: int) -> str:
    depth = 0
    for j in range(open_idx, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[open_idx + 1:j]
    return ""


def param_text(body: str, at: int) -> str | None:
    """The text between the parentheses of the fn whose name ends at `at`.

    Generics are angle-balanced first: `fn map<U, F: fn(T) -> U>(…)`
    carries both a `>` and a `(` inside its type parameter list, so a
    regex stops in the wrong place — the same trap that made a sibling
    gate read `Maybe` as having no `map`.
    """
    i, n = at, len(body)
    while i < n and body[i] in " \t":
        i += 1
    if i < n and body[i] == "<":
        depth = 0
        while i < n:
            # `->` inside a generic list is an ARROW, not a closing
            # angle. `fn map<U, F: fn(T) -> U>` closed the list at the
            # arrow and the scan then looked for `(` at ` U>` and gave
            # up — measured by the self-test below before this line
            # existed.
            if body[i] == "-" and i + 1 < n and body[i + 1] == ">":
                i += 2
                continue
            if body[i] == "<":
                depth += 1
            elif body[i] == ">":
                depth -= 1
                if depth == 0:
                    i += 1
                    break
            i += 1
    while i < n and body[i] in " \t\n":
        i += 1
    if i >= n or body[i] != "(":
        return None
    start, depth = i, 0
    while i < n:
        if body[i] == "(":
            depth += 1
        elif body[i] == ")":
            depth -= 1
            if depth == 0:
                return body[start + 1:i]
        i += 1
    return None


def _scan(s: str, on_sep=None):
    """Walk `s` tracking bracket depth and skipping STRING literals.

    A quoted argument is the reason this is not a one-line loop:
    `AppBuilder.new("wave", "Greet a value, the Verum way.")` has a comma
    inside a string and read as three arguments, and `TextSpan.raw(" (")`
    has an unbalanced `(` inside one and read as three. Both accused a
    correct page.
    """
    depth, i, n, out = 0, 0, len(s), []
    while i < n:
        ch = s[i]
        if ch in "\"'":
            q = ch
            i += 1
            while i < n:
                if s[i] == "\\":
                    i += 2
                    continue
                if s[i] == q:
                    i += 1
                    break
                i += 1
            continue
        if ch in "(<[{":
            depth += 1
        elif ch in ")>]}":
            depth -= 1
            if depth < 0 and on_sep is None:
                return i
        if on_sep is not None and ch == "," and depth == 0:
            out.append(i)
        i += 1
    return out if on_sep is not None else -1


def split_top(s: str) -> list[str]:
    cuts = _scan(s, on_sep=True)
    parts, prev = [], 0
    for c in cuts:
        parts.append(s[prev:c])
        prev = c + 1
    parts.append(s[prev:])
    return [x.strip() for x in parts if x.strip()]


def arity_of(params: str) -> tuple[int, int]:
    """(required, total) with a leading self dropped and defaults counted."""
    parts = split_top(params)
    if parts and SELF_PARAM.match(parts[0]):
        parts = parts[1:]
    total = len(parts)
    defaulted = sum(1 for p in parts if "=" in p.split(":", 1)[-1])
    return (total - defaulted, total)


def core_surface() -> dict[str, tuple[int, int, str]]:
    sites: dict[str, list[tuple[int, int, str]]] = collections.defaultdict(list)
    public_types: set[str] = set()
    context_names: set[str] = set()
    for f in CORE.rglob("*.vr"):
        text = f.read_text(errors="ignore")
        public_types.update(PUBLIC_TYPE.findall(text))
        context_names.update(CONTEXT_DECL.findall(text))
        for m in IMPL.finditer(text):
            head = m.group(1).rsplit(" for ", 1)[-1].strip()
            tm = re.match(r"([A-Z]\w*)", head)
            if not tm:
                continue
            body = brace_body(text, m.end() - 1)
            for fm in FN.finditer(body):
                params = param_text(body, fm.end())
                if params is None:
                    continue
                lo, hi = arity_of(params)
                sites[f"{tm.group(1)}.{fm.group(1)}"].append(
                    (lo, hi, str(f.relative_to(REPO))))
    return {
        k: v[0]
        for k, v in sites.items()
        if len(v) == 1
        and k.split(".", 1)[0] in public_types
        and k.split(".", 1)[0] not in context_names
    }


def call_args(body: str, open_paren: int) -> str | None:
    """The text between the call's parentheses, strings skipped."""
    depth, i, n = 0, open_paren, len(body)
    while i < n:
        ch = body[i]
        if ch in "\"'":
            q = ch
            i += 1
            while i < n:
                if body[i] == "\\":
                    i += 2
                    continue
                if body[i] == q:
                    i += 1
                    break
                i += 1
            continue
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return body[open_paren + 1:i]
        i += 1
    return None


def scan(known):
    hits, compared = [], 0
    for p in sorted(list(DOCS.rglob("*.md")) + list(DOCS.rglob("*.mdx"))):
        rel = p.relative_to(DOCS).as_posix()
        for blk in BLOCK.finditer(p.read_text(errors="ignore")):
            raw = blk.group(1)
            # Comments are blanked, not deleted, so offsets stay aligned
            # with `raw` — the counter-example marker lives IN the
            # comment, and stripping it first hid the very line it
            # excuses.
            body = re.sub(r"//[^\n]*", lambda m: " " * len(m.group(0)), raw)
            for m in CALL.finditer(body):
                key = f"{m.group(1)}.{m.group(2)}"
                if key not in known:
                    continue
                line_start = body.rfind("\n", 0, m.start()) + 1
                line_end = body.find("\n", m.start())
                # THREE lines of lead-in, not one: a "not shipped" note
                # sits ABOVE the block it disowns as often as beside it,
                # and a one-line window made the gate demand that a page
                # delete an example it had already labelled.
                win_start = line_start
                for _ in range(3):
                    win_start = raw.rfind("\n", 0, max(win_start - 1, 0)) + 1
                    if win_start <= 0:
                        break
                line = raw[win_start:line_end if line_end != -1 else len(raw)]
                if DECL_LINE.match(body[line_start:m.start() + 1]):
                    continue
                if COUNTER_EXAMPLE.search(line):
                    continue
                args = call_args(body, m.end() - 1)
                if args is None or "..." in args or "…" in args:
                    continue
                parts = split_top(args)
                # A SIGNATURE written without the `fn` keyword is the
                # other half of the declaration class: reference pages
                # list `Hsl.to_rgb(&self) -> Rgb` and
                # `TcpStream.shutdown(self, Shutdown.Write)` in a plain
                # code block. Dropping a self-shaped first argument
                # makes both read as the declarations they are, and
                # costs nothing on a real call — no call passes `self`
                # explicitly.
                if parts and SELF_PARAM.match(parts[0]):
                    parts = parts[1:]
                compared += 1
                n = len(parts)
                lo, hi, src = known[key]
                if n < lo or n > hi:
                    hits.append((rel, key, n, lo, hi, src))
    return hits, compared


def self_test() -> int:
    bad = 0
    cases = [
        ("a plain pair", "a: Int, b: Text", (2, 2)),
        ("a leading &self is not an argument", "&self, a: Int", (1, 1)),
        ("a leading self is not an argument", "self, a: Int", (1, 1)),
        ("&mut self likewise", "&mut self", (0, 0)),
        ("a default makes the tail optional",
         "a: Int, b: Text = \"x\"", (1, 2)),
        ("a comma inside a generic is not a separator",
         "a: Map<Text, Int>, b: Int", (2, 2)),
        ("empty is zero", "", (0, 0)),
        ("`&self.items` is an ARGUMENT, not a self parameter",
         "&self.items", (1, 1)),
        ("a string comma does not split", '"a, b", c', (2, 2)),
        ("an unbalanced paren inside a string does not count",
         '" (", x', (2, 2)),
    ]
    for label, params, expect in cases:
        got = arity_of(params)
        if got != expect:
            bad += 1
            print(f"  SELF-TEST FAIL: {label} — expected {expect}, got {got}",
                  file=sys.stderr)
        else:
            print(f"  [ok] {label}")
    # the generic-list trap the sibling gate was measured on
    body = "fn map<U, F: fn(T) -> U>(self, f: F) -> Maybe<U> { }"
    at = body.index("map") + 3
    if param_text(body, at) != "self, f: F":
        bad += 1
        print(f"  SELF-TEST FAIL: a `>` inside the generic list breaks the "
              f"parameter scan — got {param_text(body, at)!r}", file=sys.stderr)
    else:
        print("  [ok] a `>` inside the generic list does not break the scan")
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir() or not CORE.is_dir():
        print("check-doc-call-arity: docs or core/ absent — UNMEASURED.")
        return 0

    floor = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-calls" and i + 1 < len(sys.argv):
            floor = int(sys.argv[i + 1])

    known = core_surface()
    hits, compared = scan(known)

    if compared < floor:
        print(f"check-doc-call-arity: only {compared} comparable call(s), "
              f"expected at least {floor} — the corpus or the parameter scan "
              "is gone. A census of nothing is not a clean census.")
        return 1

    print(f"check-doc-call-arity: {len(set(hits))} of {compared} documented "
          f"call(s) pass an argument count the declaration cannot accept "
          f"(baseline {BASELINE}); {len(known)} singly-declared public methods")
    for rel, key, n, lo, hi, src in sorted(set(hits))[:20]:
        rng = f"{lo}" if lo == hi else f"{lo}..{hi}"
        print(f"  {rel}: {key} called with {n}, declares {rng}   [{src}]")
    if len(set(hits)) > 20:
        print(f"  … and {len(set(hits)) - 20} more")

    if len(set(hits)) > BASELINE:
        print(f"  ABOVE BASELINE by {len(set(hits)) - BASELINE}. The reader "
              "copies the call and the compiler refuses it.")
        return 1
    if len(set(hits)) < BASELINE:
        print(f"  BELOW baseline by {BASELINE - len(set(hits))} — lower it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
