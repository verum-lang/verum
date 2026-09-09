#!/usr/bin/env python3
"""Gate: a method called on a receiver whose TYPE the block proves must
be declared by that type.

THE HOLE THIS CLOSES. Three sibling gates look at doc method calls and
all three passed `cookbook/arenas.md`, a page whose every example was
built on a `GenerationalArena<T>` slotmap that does not exist:

  * `check_doc_method_names` checks `Type.method(…)` — an UPPERCASE
    receiver. `arena.insert(…)` is lowercase, so it never looked.
  * `check_doc_methods_declared` asks whether the NAME exists anywhere
    in `core/`. `insert`, `get`, `get_mut` and `remove` all do — on
    `Map`, on `List`. The page passed on other types' methods.
  * `list_doc_absent_methods` is deliberately not a gate, and its
    provable lane skipped these for the same reason: the name exists.

The question none of them asked is the one that matters: does the
method exist ON THIS TYPE. That is answerable exactly when the block
PROVES the receiver's type, and then it is not a judgement call.

WHAT COUNTS AS PROOF. Two forms, both syntactic and both local to the
block:

    let arena: GenerationalArena = …          annotation
    let arena = GenerationalArena.new(4096)   constructor whose core
                                              return type IS the type

The second is narrow on purpose. `MemStackAllocator.init(4096)` returns
`Result<MemStackAllocator, AllocError>`, so `init` is NOT a constructor
for this purpose and a receiver bound from it stays unproven. Only a
function declared inside `implement Type` and returning `Type` or
`Self` binds.

Everything unproven is left alone. This gate never guesses.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
DOCS = REPO / "internal" / "website" / "docs"

BASELINE = 0

FENCE = re.compile(r"^```(?:verum|vr)?\n(.*?)^```", re.M | re.S)
CALL = re.compile(r"\b([a-z_]\w*)\.([a-z_]\w*)\s*\(")
ANNOT = re.compile(r"\blet\s+(?:mut\s+)?([a-z_]\w*)\s*:\s*([A-Z]\w*)")
CTOR_BIND = re.compile(
    r"\blet\s+(?:mut\s+)?([a-z_]\w*)\s*=\s*([A-Z]\w*)\s*(?:<[^>=;]*>)?\s*\.\s*([a-z_]\w*)\s*\(")


def ctor_binds(body: str, ctors: dict[str, set[str]]) -> dict[str, str]:
    """`let x = Type.ctor(…);` — and ONLY when that is the whole
    initialiser.

    A chained call changes the type and the binding with it.
    `stdlib/net/weft/overview.md` writes

        let server = WeftApp.new(app).bind("0.0.0.0:8080")?;

    where `bind` answers `Result<Server<_>, Text>`, so `server` is a
    `Server`. Binding on the constructor alone made it a `WeftApp` and
    the gate reported the page's correct `server.serve()`. So: balance
    the constructor's parentheses and require a `;` immediately after.
    A `?`, a `.`, an `await` — anything else — leaves the receiver
    unproven, which is the safe direction.
    """
    out: dict[str, str] = {}
    for m in CTOR_BIND.finditer(body):
        var, ty, ctor = m.group(1), m.group(2), m.group(3)
        if ctor not in ctors.get(ty, ()):
            continue
        i, depth, n = m.end() - 1, 0, len(body)
        while i < n:
            if body[i] == "(":
                depth += 1
            elif body[i] == ")":
                depth -= 1
                if depth == 0:
                    i += 1
                    break
            i += 1
        while i < n and body[i] in " \t":
            i += 1
        if i < n and body[i] == ";":
            out[var] = ty
    return out
PAGE_FN = re.compile(r"\bfn\s+([a-z_]\w*)")
# Same generous filter the sibling gates use: a page that TEACHES a
# name is absent has to write it.
DENIAL = re.compile(
    r"(does not exist|do not exist|not exist|no such|never existed|"
    r"there is no|there are no|has no|have no|is not a|are not|"
    r"not shipped|not implemented|invented|fiction|undeclared|removed|"
    r"renamed|does not|did not)", re.I)
# The page-wide pass needs an UNAMBIGUOUS denial. "does not" alone
# appears in ordinary prose on almost every page.
STRONG_DENIAL = re.compile(
    r"(does not exist|do not exist|never existed|there is no|there are no|"
    r"has no |have no |not shipped|is not declared|declares no)", re.I)

IMPL = re.compile(r"(?:^|\n)[ \t]*implement\s*(?:<[^>]*>)?\s*(.+?)\s*\{")
FN_NAME = re.compile(r"\bfn\s+([a-z_]\w*)")


def signature_return(body: str, at: int) -> str:
    """The return type of the `fn` whose name ends at `at`, or "".

    A regex cannot do this. `fn map<U, F: fn(T) -> U>(self, f: F)`
    carries a `>` INSIDE its generic list — the arrow of a function-typed
    bound — so `<[^>]*>` stops in the middle of the parameters and the
    declaration is silently not seen at all. Measured: `Maybe` came back
    with 42 methods and no `map`, and the gate then reported
    `language/iterators.md`'s correct `m.map(…)` as calling a method
    `Maybe` does not declare.
    """
    i, n = at, len(body)
    # optional generics, angle-balanced
    while i < n and body[i] in " \t":
        i += 1
    if i < n and body[i] == "<":
        depth = 0
        while i < n:
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
        return ""
    depth = 0
    while i < n:
        if body[i] == "(":
            depth += 1
        elif body[i] == ")":
            depth -= 1
            if depth == 0:
                i += 1
                break
        i += 1
    while i < n and body[i] in " \t\n":
        i += 1
    if not body.startswith("->", i):
        return ""
    i += 2
    j = i
    while j < n and body[j] not in "{;":
        j += 1
    return body[i:j].split(" where ")[0].strip()


def block_at(text: str, open_idx: int) -> str:
    """The brace group starting at `open_idx`, or "" if unbalanced."""
    depth = 0
    for j in range(open_idx, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[open_idx + 1:j]
    return ""


def core_surface() -> tuple[dict[str, set[str]], dict[str, set[str]], set[str]]:
    """(methods per type, constructors per type, universal method names).

    A blanket `implement<T> Proto for T` applies to every type, so its
    method names go in the universal set rather than against a name —
    counting them per-type would need real trait resolution, and
    exempting them is the conservative direction for a gate.
    """
    methods: dict[str, set[str]] = {}
    ctors: dict[str, set[str]] = {}
    universal: set[str] = set()
    # A type that Derefs forwards every method of its target, and the
    # target is a type PARAMETER for the ones that matter — `Shared<T>`,
    # `Heap<T>`, `Maybe<T>`. Resolving that needs real inference, so a
    # Deref-ing receiver is left unproven rather than guessed at.
    # Measured: without this, `stopped.load()` on a `Shared<AtomicBool>`
    # was reported five times across the cookbook as a method `Shared`
    # does not declare. It does not — `AtomicBool` does, and that is the
    # whole point of the wrapper.
    derefs: set[str] = set()
    # `implement P for T` gives T every method P declares, including P's
    # DEFAULTS. Measured: `BufReader` implements `Read`, whose default
    # `lines()` is declared in the protocol body and nowhere else, and
    # the gate reported three correct pages for calling it.
    protocols: dict[str, set[str]] = {}
    impl_of: list[tuple[str, str]] = []
    # `protocol` may carry an `extends` list — `type BufRead is protocol
    # extends Read {`. Requiring `protocol {` skipped every such
    # protocol's body, and BufReader then looked as though it had no
    # `lines()` even though it implements BufRead, which declares one.
    proto_head = re.compile(
        r"(?:^|\n)[ \t]*(?:public\s+|pub\s+)?type\s+([A-Z]\w*)"
        r"(?:<[^>]*>)?\s+is\s+protocol\b[^{]*\{")
    for f in CORE.rglob("*.vr"):
        text = f.read_text(errors="ignore")
        for pm in proto_head.finditer(text):
            pbody = block_at(text, pm.end() - 1)
            if pbody:
                protocols.setdefault(pm.group(1), set()).update(
                    FN_NAME.findall(pbody))
        for m in IMPL.finditer(text):
            head = m.group(1)
            body = block_at(text, m.end() - 1)
            if not body:
                continue
            if " for " in head and head.split(" for ", 1)[0].strip().endswith("Deref"):
                t = re.match(r"([A-Z]\w*)", head.rsplit(" for ", 1)[-1].strip())
                if t:
                    derefs.add(t.group(1))
            if " for " in head:
                pname = re.match(r"([A-Z]\w*)", head.split(" for ", 1)[0].strip())
                tname = re.match(r"([A-Z]\w*)", head.rsplit(" for ", 1)[-1].strip())
                if pname and tname:
                    impl_of.append((tname.group(1), pname.group(1)))
            tail = head.rsplit(" for ", 1)[-1].strip()
            name = re.match(r"([A-Z]\w*)", tail)
            if not name:
                # `implement<T> Proto for T` — the receiver is a
                # parameter, so the methods land on everything.
                universal.update(FN_NAME.findall(body))
                continue
            ty = name.group(1)
            for fm in FN_NAME.finditer(body):
                fn_name = fm.group(1)
                methods.setdefault(ty, set()).add(fn_name)
                r = signature_return(body, fm.end())
                if r == ty or r == "Self" or r.startswith(ty + "<"):
                    ctors.setdefault(ty, set()).add(fn_name)
    for ty, proto in impl_of:
        if proto in protocols:
            methods.setdefault(ty, set()).update(protocols[proto])
    return methods, ctors, universal, derefs


def denied_on_page(text: str) -> set[str]:
    """Method names the PAGE denies, anywhere in it.

    A block-local window is not enough. `cookbook/scheduler.md` denies
    `set_missed_tick_behavior` inside a ":::caution Not shipped" block
    and then shows "the shape a configurable policy would take" in a
    fence twenty lines below. The denial IS the page, and a gate that
    cannot see past its own fence would demand the page delete its own
    teaching.

    But a page-wide pass has to be STRICT or it exempts the tree. The
    first version reused the generous block-local filter with a
    200-character window; on `cookbook/arenas.md`, whose `new_region`
    caution contains "does not exist", that swallowed the very control
    case this gate was built to catch. A rule that silences the finding
    you wrote it for is not a rule, it is a hole. So: an unambiguous
    denial phrase, a tight window, and the name has to be in backticks.
    """
    out: set[str] = set()
    for m in STRONG_DENIAL.finditer(text):
        window = text[max(0, m.start() - 110):m.end() + 110]
        out.update(re.findall(r"`([a-z_]\w{2,})(?:\(\))?`", window))
    return out


def scan(methods, ctors, universal, derefs):
    hits, proven = [], 0
    for p in sorted(list(DOCS.rglob("*.md")) + list(DOCS.rglob("*.mdx"))):
        rel = p.relative_to(DOCS).as_posix()
        text = p.read_text(errors="ignore")
        denied = denied_on_page(text)
        for fence in FENCE.finditer(text):
            body = fence.group(1)
            page_fns = set(PAGE_FN.findall(body))
            bound: dict[str, str] = {}
            for var, ty in ANNOT.findall(body):
                if ty in methods:
                    bound[var] = ty
            bound.update(ctor_binds(body, ctors))
            for m in CALL.finditer(body):
                recv, meth = m.group(1), m.group(2)
                ty = bound.get(recv)
                if ty is None or ty in derefs:
                    continue
                proven += 1
                if meth in methods[ty] or meth in universal or meth in page_fns:
                    continue
                if meth in denied:
                    continue
                if DENIAL.search(body[max(0, m.start() - 120):m.end() + 120]):
                    continue
                hits.append((rel, ty, recv, meth))
    return hits, proven


def self_test() -> int:
    bad = 0
    methods = {"Arena": {"alloc", "reset"}, "Map": {"insert", "get"}}
    ctors = {"Arena": {"new"}}
    universal = {"clone"}

    def run(body):
        import types as _t
        page_fns = set(PAGE_FN.findall(body))
        bound = {}
        for var, ty in ANNOT.findall(body):
            if ty in methods:
                bound[var] = ty
        bound.update(ctor_binds(body, ctors))
        out = []
        for m in CALL.finditer(body):
            r, meth = m.group(1), m.group(2)
            ty = bound.get(r)
            if ty is None:
                continue
            if meth in methods[ty] or meth in universal or meth in page_fns:
                continue
            if DENIAL.search(body[max(0, m.start() - 120):m.end() + 120]):
                continue
            out.append(meth)
        return out

    cases = [
        ("a constructor binds the receiver",
         "let a = Arena.new(16);\na.insert(x);\n", ["insert"]),
        ("a real method on that type passes",
         "let a = Arena.new(16);\na.alloc(64);\n", []),
        ("an annotation binds too",
         "let a: Arena = mk();\na.insert(x);\n", ["insert"]),
        ("generic args on the constructor still bind",
         "let a = Arena<Node>.new(16);\na.insert(x);\n", ["insert"]),
        ("a NON-constructor leaves the receiver unproven",
         "let a = Arena.init(16);\na.insert(x);\n", []),
        ("a CHAINED call leaves the receiver unproven",
         "let a = Arena.new(16).into_map();\na.insert(x);\n", []),
        ("a `?` after the constructor leaves it unproven",
         "let a = Arena.new(16)?;\na.insert(x);\n", []),
        ("an unbound receiver is left alone",
         "a.insert(x);\n", []),
        ("a blanket-impl method is exempt",
         "let a = Arena.new(16);\na.clone();\n", []),
        ("a method the BLOCK declares is exempt",
         "fn insert(x: Int) {}\nlet a = Arena.new(16);\na.insert(x);\n", []),
        ("a denial is not an accusation",
         "let a = Arena.new(16);\n// Arena has no insert\na.insert(x);\n", []),
    ]
    for label, body, expect in cases:
        got = run(body)
        if got != expect:
            bad += 1
            print(f"  SELF-TEST FAIL: {label} — expected {expect}, got {got}",
                  file=sys.stderr)
        else:
            print(f"  [ok] {label}")
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir() or not CORE.is_dir():
        print("check-doc-receiver-methods: docs or core/ absent — UNMEASURED.")
        return 0

    methods, ctors, universal, derefs = core_surface()

    # A FLOOR. An instrument that stopped matching prints the same clean
    # line as a clean tree.
    floor = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-proven" and i + 1 < len(sys.argv):
            floor = int(sys.argv[i + 1])

    # Read a page off disk when asked, so the arenas control can be
    # replayed against a historical version without a checkout.
    for i, a in enumerate(sys.argv):
        if a == "--file" and i + 1 < len(sys.argv):
            path = Path(sys.argv[i + 1])
            text = path.read_text(errors="ignore")
            denied = denied_on_page(text)
            hits = []
            for fence in FENCE.finditer(text):
                body = fence.group(1)
                page_fns = set(PAGE_FN.findall(body))
                bound = {}
                for var, ty in ANNOT.findall(body):
                    if ty in methods:
                        bound[var] = ty
                bound.update(ctor_binds(body, ctors))
                for m in CALL.finditer(body):
                    recv, meth = m.group(1), m.group(2)
                    ty = bound.get(recv)
                    if ty is None or ty in derefs or meth in methods[ty] \
                       or meth in universal or meth in page_fns \
                       or meth in denied:
                        continue
                    if DENIAL.search(body[max(0, m.start() - 120):m.end() + 120]):
                        continue
                    hits.append((path.name, ty, recv, meth))
            for h in sorted(set(hits)):
                print("  {}: `{}.{}` — {} declares no `{}`".format(*h[:1], h[2], h[3], h[1], h[3]))
            print(f"{len(set(hits))} finding(s) in {path}")
            return 0

    hits, proven = scan(methods, ctors, universal, derefs)

    if proven < floor:
        print(f"check-doc-receiver-methods: only {proven} proven receiver "
              f"call(s), expected at least {floor} — the binder stopped "
              "matching, or the corpus is gone. A census of nothing is not "
              "a clean census.")
        return 1

    print(f"check-doc-receiver-methods: {len(hits)} of {proven} call(s) on a "
          f"PROVEN receiver name a method its type does not declare "
          f"(baseline {BASELINE}); {len(methods)} core types carry methods")
    for rel, ty, recv, meth in sorted(set(hits))[:20]:
        print(f"  {rel}: `{recv}.{meth}(…)` — {ty} declares no `{meth}`")
    if len(set(hits)) > 20:
        print(f"  … and {len(set(hits)) - 20} more")

    if len(hits) > BASELINE:
        print(f"  ABOVE BASELINE by {len(hits) - BASELINE}. The receiver's "
              "type is proven by the block itself, so this is not a "
              "judgement call — the example cannot run.")
        return 1
    if len(hits) < BASELINE:
        print(f"  BELOW baseline by {BASELINE - len(hits)} — lower it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
