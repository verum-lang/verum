#!/usr/bin/env python3
"""A documentation page that describes a configuration struct must
describe one that EXISTS, with the defaults and the enum values it
actually has.

WHY THIS CLASS AND NOT "a typo". A config page is written once against
a real struct and then outlives it. The struct is renamed, a variant is
added, a default is tuned — and nothing executes prose, so the page
keeps its first reading forever. Measured 2026-09-07 across three
pages, every hit of this shape:

  * TWO sections named `SmtBackendConfig`, a struct that does not exist
    in the tree, documenting two DIFFERENT real structs (`Z3Config` and
    `Cvc5Config`). Both had been renamed since the page was written.
  * Owners `SmtContextManager` and `SmtOptimizer`: absent. `SmtBackend`:
    present, as an ENUM in another crate, unrelated to configuration.
  * THREE enum values named that no enum has — `Coinduction`,
    `UpToBisimulation`, `WeightedSum` — and five real variants unnamed.
  * `max_solutions` documented `Some(usize::MAX)`, actually `Some(100)`
    — on two pages, and a third rendered it `-1  # -1 = unbounded`,
    which cannot even deserialize into the field's `Maybe<usize>`.

THREE AXES, REPORTED SEPARATELY, because each has a different fix:
  1. HEADING  — `## Name — …` or ``### `Name` — …``: does that struct
     exist?
  2. OWNER    — ``**Owner**: `path::Type` ``: does that type exist, and
     is it a struct rather than something else wearing the name?
  3. VALUES   — a field whose default is a bare CamelCase identifier:
     do the alternatives the row names match the enum's variants?
  4. DEFAULTS — a documented default against the `impl Default` block.

VACUITY IS REPORTED, NOT SILENTLY PASSED. A page whose headings this
gate cannot parse yields zero of everything, and zero reads as clean.
It says VACUOUS instead. That is not hypothetical: the first version
required `## Name`, and the architecture page writes ``### `Name` ``,
so it reported "0 structs, 0 defects" for a page carrying four.

NORMALISATION. A doc writes the VALUE, the code writes it in Rust
spelling. `Some(30_000)` / `30 000`, `InfiniteStrategy::Hybrid` /
`Hybrid`, `num_cpus::get()` / `cpus()`, `500 * 1024 * 1024` / `500MB`
carry no difference for a reader, and flagging them gave 13 false hits
out of 14 — the rate at which a reader stops reading the tool.
"""

import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS_ENV = os.environ.get("VERUM_DOCS_DIR")
DOCS = Path(DOCS_ENV) if DOCS_ENV else REPO.parent / "website" / "docs"

HEAD = re.compile(r"^#{2,4} `?([A-Z][A-Za-z0-9]+)`? [—-]")
OWNER = re.compile(r"^\*\*Owner\*\*:\s*`([^`]+)`")
ROW = re.compile(r"^\|\s*`([a-z_][a-z0-9_]*)`\s*\|\s*`?([^`|]*)`?\s*\|(.*)$")
WRAPPERS = ("Some", "None", "Maybe", "Option")

# Floor on the number of pages examined; see main().
MIN_PAGES = 0

# A heading that is a config STRUCT ends in `Config` or `Settings`; a
# page has many other `## Word — …` headings and asking the tree about
# every one of them turns prose sections into phantom defects.
def is_config_name(name: str) -> bool:
    # A PREFIX is required: a heading that is the bare word `Config`
    # (stdlib/context.md has one) is prose about configuration, not a
    # struct named `Config`, and asking the tree about it produces a
    # phantom defect on a page that is perfectly correct.
    for suffix in ("Config", "Settings"):
        if name.endswith(suffix) and len(name) > len(suffix):
            return True
    return False


def load_sources(root: Path):
    out = {}
    crates = root / "crates"
    if not crates.is_dir():
        return out
    for p in crates.rglob("*.rs"):
        if "/target/" in str(p):
            continue
        try:
            out[p] = p.read_text(errors="ignore")
        except OSError:
            continue
    return out


def find_item(blobs, kind: str, name: str):
    pat = re.compile(rf"pub {kind} {re.escape(name)}\b")
    return [p for p, s in blobs.items() if pat.search(s)]


def _balanced(s: str, start: int) -> str:
    depth, i = 1, start
    while i < len(s) and depth:
        depth += {"{": 1, "}": -1}.get(s[i], 0)
        i += 1
    return s[start : i - 1]


def struct_fields(blobs, name: str):
    pat = re.compile(rf"pub struct {re.escape(name)}\s*\{{")
    for _, s in blobs.items():
        m = pat.search(s)
        if not m:
            continue
        body = _balanced(s, m.end())
        return {
            fm.group(1): fm.group(2).strip()
            for fm in re.finditer(r"pub ([a-z_][a-z0-9_]*)\s*:\s*([^,\n]+)", body)
        }
    return None


def enum_variants(blobs, name: str):
    pat = re.compile(rf"pub enum {re.escape(name)}\s*\{{")
    for p, s in blobs.items():
        m = pat.search(s)
        if not m:
            continue
        out = []
        for line in _balanced(s, m.end()).splitlines():
            line = line.strip()
            if not line or line.startswith("//") or line.startswith("#["):
                continue
            vm = re.match(r"([A-Z][A-Za-z0-9_]*)", line)
            if vm:
                out.append(vm.group(1))
        return out, p
    return None, None


def impl_default(blobs, name: str):
    pat = re.compile(r"impl Default for " + re.escape(name) + r"\b")
    for p, s in blobs.items():
        m = pat.search(s)
        if not m:
            continue
        b = s.find("Self {", m.end())
        if b < 0:
            continue
        body = _balanced(s, b + 6)
        out = {}
        # A field line may carry a trailing `// comment`; anchoring on
        # `,\s*$` silently skipped every field written that way and
        # reported all of them as absent from the impl (19 such rows in
        # the first run, every one an artefact).
        for fm in re.finditer(
            r"^\s*([a-z_][a-z0-9_]*)\s*:\s*(.+?),\s*(?://.*)?$", body, re.M
        ):
            out[fm.group(1)] = fm.group(2).strip()
        return p, out
    return None, None


def norm(v: str) -> str:
    v = v.strip()
    m = re.match(r"^`([^`]*)`", v)
    if m:
        v = m.group(1)
    for sp in (" ", " ", " ", " "):
        v = v.replace(sp, "")
    v = v.replace("_", "").rstrip(".")
    m = re.fullmatch(r"Duration::from_?secs\((.*)\)", v)
    if m:
        try:
            secs = eval(m.group(1), {"__builtins__": {}})  # digits and * only
            if isinstance(secs, int) and secs % 86400 == 0:
                return f"{secs // 86400}days"
        except Exception:
            pass
    # ORDER MATTERS, and getting it wrong manufactures thirteen
    # disagreements out of fourteen. The code writes `Maybe::Some(30000)`
    # and the page writes `30 000`; unwrapping `Some(...)` BEFORE
    # stripping the `Maybe::` path leaves the code side as
    # `Maybe::Some(30000)` (no match) and the page side as `30000`.
    v = re.sub(r"^(?:Maybe|Option)::", "", v)
    m = re.fullmatch(r"Some\((.*)\)", v)
    if m:
        v = m.group(1)
    v = re.sub(r"^(?:Self|[A-Z][A-Za-z0-9]*)::", "", v)
    v = v.replace("numcpus::get()", "cpus()").replace("num_cpus::get()", "cpus()")
    m = re.fullmatch(r"(\d+)\*1024\*1024", v)
    if m:
        v = f"{m.group(1)}MB"
    return v


def audit_page(blobs, page: Path):
    """Returns (defects, heads, checked_enums) — heads==0 means VACUOUS."""
    defects = []
    heads = checked = 0
    doc: dict[str, dict[str, tuple[str, str]]] = {}
    owners: list[tuple[str, str]] = []
    cur = None
    for line in page.read_text(errors="ignore").splitlines():
        h = HEAD.match(line)
        if h:
            cur = h.group(1) if is_config_name(h.group(1)) else None
            if cur:
                heads += 1
                doc.setdefault(cur, {})
                if not find_item(blobs, "struct", cur):
                    defects.append(f"heading `{cur}` names no struct in the tree")
            continue
        if cur is None:
            continue
        o = OWNER.match(line)
        if o:
            ty = o.group(1).split("::")[-1].strip()
            if not find_item(blobs, "struct", ty):
                kind = "an enum" if find_item(blobs, "enum", ty) else "absent"
                defects.append(f"{cur}: owner `{o.group(1)}` is {kind}, not a struct")
            owners.append((cur, ty))
            continue
        r = ROW.match(line)
        if r:
            doc[cur].setdefault(r.group(1), (r.group(2).strip(), r.group(3)))

    for struct, fields in doc.items():
        if not fields:
            continue
        _, impl = impl_default(blobs, struct)
        sfields = struct_fields(blobs, struct)
        for f, (default, rest) in fields.items():
            if impl and f in impl and norm(default) != norm(impl[f]):
                defects.append(
                    f"{struct}.{f}: page says {norm(default)!r}, "
                    f"impl Default says {norm(impl[f])!r}"
                )
            if not re.fullmatch(r"`?[A-Z][A-Za-z0-9]*`?", default.strip()):
                continue
            if not sfields or f not in sfields:
                continue
            ety = re.sub(r"^(?:Maybe|Option)<|>$", "", sfields[f]).strip()
            variants, _ = enum_variants(blobs, ety)
            if variants is None:
                continue
            checked += 1
            named = re.findall(r"`([A-Z][A-Za-z0-9_]*)`", rest)
            named = [n for n in named if n in variants or n not in WRAPPERS]
            ghosts = [n for n in named if n not in variants]
            if ghosts:
                defects.append(
                    f"{struct}.{f}: names {', '.join(ghosts)} — "
                    f"{ety} has {', '.join(variants)}"
                )
    return defects, heads, checked


SELF_TEST_PAGE = """\
## GhostConfig — one that does not exist

**Owner**: `nowhere::GhostOwner`.

| Field | Default | Effect |
|-------|--------:|--------|
| `mode` | `Fast` | `Fast` / `Nonexistent`. |
"""


def self_test() -> int:
    blobs = {
        Path("fake.rs"): (
            "pub struct RealConfig { pub mode: Mode }\n"
            "pub enum Mode { Fast, Slow }\n"
            "impl Default for RealConfig { fn default() -> Self { "
            "Self { mode: Mode::Fast, } } }\n"
        )
    }
    import tempfile

    fails = []
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / "ghost.md"
        p.write_text(SELF_TEST_PAGE)
        defects, heads, _ = audit_page(blobs, p)
        if heads != 1:
            fails.append(f"heading not parsed: heads={heads}")
        if not any("names no struct" in x for x in defects):
            fails.append("a heading naming an absent struct was not flagged")
        if not any("owner" in x for x in defects):
            fails.append("an absent owner was not flagged")

        # the same page, but naming the struct that DOES exist
        p2 = Path(d) / "real.md"
        p2.write_text(SELF_TEST_PAGE.replace("GhostConfig", "RealConfig")
                      .replace("`nowhere::GhostOwner`", "`x::RealConfig`"))
        defects2, heads2, checked2 = audit_page(blobs, p2)
        if heads2 != 1:
            fails.append(f"real heading not parsed: heads={heads2}")
        if checked2 != 1:
            fails.append(f"enum row not checked: checked={checked2}")
        if not any("Nonexistent" in x for x in defects2):
            fails.append("a ghost enum value was not flagged")
        if any("names no struct" in x for x in defects2):
            fails.append("a real struct was reported absent")

    # normalisation must not manufacture disagreements
    for a, b in [
        ("Some(30 000)", "Some(30000)"),
        ("30 000", "Some(30000)"),
        ("`Hybrid`", "InfiniteStrategy::Hybrid"),
        ("cpus()", "num_cpus::get()"),
        ("500MB", "500 * 1024 * 1024"),
        ("30days", "Duration::from_secs(30 * 24 * 60 * 60)"),
        ("default_strategies()", "Self::default_strategies()"),
    ]:
        if norm(a) != norm(b):
            fails.append(f"normalisation split {a!r} from {b!r}: "
                         f"{norm(a)!r} != {norm(b)!r}")
    for a, b in [
        ("30 000", "Maybe::Some(30000)"),
        ("8 192", "Maybe::Some(8192)"),
        ("None", "Maybe::None"),
    ]:
        if norm(a) != norm(b):
            fails.append(f"normalisation split {a!r} from {b!r}: "
                         f"{norm(a)!r} != {norm(b)!r}")
    # …and must not erase a REAL disagreement
    if norm("Some(usize::MAX)") == norm("Some(100)"):
        fails.append("normalisation erased a real default mismatch")
    if norm("30 000") == norm("Maybe::Some(5000)"):
        fails.append("normalisation erased a real timeout mismatch")
    # a bare `Config` heading is prose, a prefixed one is a struct
    if is_config_name("Config"):
        fails.append("a bare `Config` heading was taken for a struct")
    if not is_config_name("QEConfig"):
        fails.append("a prefixed Config heading was not recognised")

    for f in fails:
        print(f"SELF-TEST FAIL: {f}")
    print(f"self-test: {'ok' if not fails else str(len(fails)) + ' failure(s)'}")
    return 1 if fails else 0


def main() -> int:
    global MIN_PAGES
    if "--self-test" in sys.argv:
        return self_test()
    if "--min-pages" in sys.argv:
        MIN_PAGES = int(sys.argv[sys.argv.index("--min-pages") + 1])
    if not DOCS.is_dir():
        print(f"check-doc-config-structs: no docs at {DOCS} — set VERUM_DOCS_DIR")
        return 0
    blobs = load_sources(REPO)
    if not blobs:
        print("check-doc-config-structs: no crates/ sources found — refusing to "
              "report a clean run against nothing")
        return 1
    total_defects, pages, vacuous = [], 0, []
    for md in sorted(DOCS.rglob("*.md")):
        text = md.read_text(errors="ignore")
        # `HEAD` is anchored with `^` and used per LINE elsewhere; a
        # whole-file `search` without re.M matches only at offset 0 and
        # skipped every page. The gate printed "0 pages, 0 defects" for
        # a corpus with thirteen config sections — a clean zero that
        # meant the pre-filter never found its input.
        if not any(HEAD.match(l) for l in text.splitlines()):
            continue
        defects, heads, _ = audit_page(blobs, md)
        if heads == 0:
            # Config-shaped headings were present but none named a
            # `*Config` / `*Settings` struct — nothing to check here,
            # and saying so beats a silent skip only when the page
            # LOOKED like a config page. It did not; move on.
            continue
        pages += 1
        for d in defects:
            total_defects.append(f"{md.relative_to(DOCS)}: {d}")
    # A floor, not a zero-check. `docs/` legitimately carries no config
    # sections, so vacuity there is a fact about the corpus; on the site
    # it would mean the gate stopped reading its input. The CALLER says
    # which it is, the way the other doc gates carry a baseline.
    if pages < MIN_PAGES:
        print(f"check-doc-config-structs: VACUOUS — {pages} page(s) with a "
              f"`## <Name>Config — …` section under {DOCS}, floor is "
              f"{MIN_PAGES}. Zero defects here is a statement about the "
              "gate's reach, not about the corpus.")
        return 1
    if pages == 0:
        print(f"check-doc-config-structs: no config sections under {DOCS} "
              "(no floor set) — nothing examined")
        return 0
    print(f"check-doc-config-structs: {pages} page(s) with config sections, "
          f"{len(total_defects)} defect(s)")
    for d in total_defects:
        print(f"  {d}")
    if vacuous:
        print("VACUOUS pages (headings unparsed):")
        for v in vacuous:
            print(f"  {v}")
    return 1 if total_defects else 0


if __name__ == "__main__":
    sys.exit(main())
