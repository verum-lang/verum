#!/usr/bin/env python3
"""
check_no_double_colon.py — gate: no Rust-style `::` in Verum (.vr) sources.

WHY THIS EXISTS
---------------
`grammar/verum.ebnf` is the authoritative Verum grammar and knows only `.`
for path separation; `::` appears there only inside EBNF comments.  The
fast parser (`crates/verum_fast_parser`) tolerates `::` ONLY as
diagnostic-recovery — it rewrites `List::new()` -> `List.new()` in the error
message and hard-rejects turbofish `foo::<T>`.  So every `::` in a .vr file
is a Rust porting artefact.  This gate keeps them out for good.

It converts (or, in --check mode, flags) `::` -> `.` (turbofish `::<T>` ->
`<T>`) everywhere EXCEPT legitimate DATA that merely contains `::`:

  * IPv6 literals            "::1", "2001:db8::/32", "[::1]:443", ip#"::1"
  * SQL casts                oid::text, atttypid::int4    (Postgres/SQLite)
  * SQLite URIs              file::memory:?cache=shared
  * foreign script gen       cli/completion.vr  (PowerShell [T]::new, zsh :::)
  * quasiquote corpus        vcs/**/quote_hygiene/**  (author-owned tokens)
  * negative-syntax tests    vcs/**/fail/**  (`::` IS the thing under test)

Context (code / comment / string) is resolved via the shared state machine
in `vr_gate_scan.py` (T0652), which understands line + block comments,
plain / multiline / raw strings, f/b prefixes, AND char literals — so
string and comment DATA is never mistaken for code, and a char literal
whose content is a quote (`'"'`) can't desync the string-boundary tracking
for everything after it.  That desync was a real, if lucky, gap: it
reproduces on `core/text/text.vr` (a `'"'` in `f.write_char('"')?` swallows
the next ~20 lines into what the old inline scanner thought was a string),
and this script's own former inline copy of the scanner had it too — the
gate `classify()`s ambiguous non-code context as `keep`, so a `::` caught
in a desynced span would have been silently passed rather than flagged.
Checked the corpus for a live victim (any `.vr` file pairing a `'"'`-style
char literal with a genuine, non-data `::`): none exists today — only one
file, `core/shell/permissions.vr`, has both, and its `::` are all IPv6
documentation, correctly excluded either way.  So the gate has been lucky
rather than correct; this closes the gap before luck runs out.

MODES
  --check   (default) CI gate: exit 1 if any `::` is a violation, else 0
  --report  dry run: print every decision grouped for review, no writes
  --apply   rewrite offending `::` -> `.` in place

Pure Python, no build required.  Runs over core/, core-tests/, vcs/.
"""
import os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vr_gate_scan import iter_vr_files, scan_matches  # noqa: E402

# Repo root = two levels up from vcs/scripts/.
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ROOTS = ["core", "core-tests", "vcs"]
DOUBLE_COLON = re.compile("::")

# Whole-file force-keep: foreign-script generators and author-owned quasiquote
# token corpora — the `::` there is DATA, not Verum path syntax.
FORCE_KEEP_FILES = (
    "cli/completion.vr",     # bash/zsh/PowerShell script gen — foreign shell/.NET syntax
    "/quote_hygiene/",       # macro/quasiquote codegen corpus — quote{} token content
)

# Postgres/SQLite cast type names, lowercase (matched case-sensitively so the
# Verum types `Int`/`Text` and variants `::Rejected` are NOT treated as casts).
SQL_TYPES = {
    "text","int2","int4","int8","bigint","smallint","oid","regclass","regtype",
    "regproc","regnamespace","bytea","jsonb","json","boolean","bool","float4",
    "float8","numeric","timestamptz","timestamp","uuid","record","varchar",
    "double","inet","cidr","tsvector","int",
}
HEX = set("0123456789abcdefABCDEF")


def in_data_domain(path):
    p = path.replace(os.sep, "/")
    return ("/net/" in p or "/database/" in p or "/dns" in p or "/quic/" in p
            or "/tls/" in p or "/socket" in p or "postgres" in p or "mysql" in p
            or "sqlite" in p or "/addr" in p or "/cidr" in p or "ipv6" in p
            or "ipv4" in p)


def _rword(text, pos):
    m = re.match(r"[A-Za-z0-9_]+", text[pos+2:])
    return m.group() if m else ""


def _lword(text, pos):
    m = re.search(r"[A-Za-z0-9_]+$", text[:pos])
    return m.group() if m else ""


def _has_nonhex_alpha(s):
    return any(c.isalpha() and c not in HEX for c in s)


def is_identifier_path(text, pos):
    l, r = _lword(text, pos), _rword(text, pos)
    if "_" in l or "_" in r:
        return True
    return _has_nonhex_alpha(l) or _has_nonhex_alpha(r)


def is_ipv6(text, pos):
    b = text[pos-1] if pos > 0 else ""
    a = text[pos+2] if pos+2 < len(text) else ""
    left_ok = (b in HEX) or b in ":[\"'"
    right_ok = (a in HEX) or a in ":]/\"'."
    # Identifier boundaries (Foo::bar, yoneda::embed) can be hex letters
    # (e/a/b/…) — those are NOT IPv6.
    return left_ok and right_ok and not is_identifier_path(text, pos)


def is_sql_cast(text, pos):
    w = _rword(text, pos)
    if not w or w not in SQL_TYPES:      # case-sensitive: `::text` yes, `::Text`/`::Int` no
        return False
    after = text[pos+2+len(w): pos+3+len(w)]
    return after != "("                  # `::text` is a cast, `x::name(` is a call


def is_sqlite_uri(text, pos):
    return text[max(0, pos-5):pos].endswith("file") and text[pos+2:pos+8].startswith("memory")


def classify(ctx, path, text, pos):
    """Return 'convert' or 'keep' for the `::` at pos."""
    p = path.replace(os.sep, "/")
    if "/fail/" in p:                    # negative-syntax test corpus
        return "keep"
    if any(f in p for f in FORCE_KEEP_FILES):
        return "keep"
    b = text[pos-1] if pos > 0 else ""
    a2 = text[pos+2] if pos+2 < len(text) else ""
    if b in "]:" or a2 == ":":           # PowerShell [T]::new, zsh :::, regex (?:: — foreign syntax
        return "keep"
    if ctx == "code":
        return "convert"
    if is_ipv6(text, pos) or is_sql_cast(text, pos) or is_sqlite_uri(text, pos):
        return "keep"
    if ctx in ("line", "block"):         # comments hold no protected data beyond the quoted forms above
        return "convert"
    if in_data_domain(path):             # net/database string literal = IPv6/SQL/URI data
        return "keep"
    if is_identifier_path(text, pos):
        return "convert"
    return "keep"                        # ambiguous non-domain string -> safe keep


def scan(text):
    """Yield (pos, ctx, line) for each '::'. ctx in code|line|block|string.

    Thin wrapper over the shared `vr_gate_scan.scan_matches` state machine
    (T0652) — see that module and the file docstring above for what
    changed and why (char-literal awareness, so `'"'` can't desync string
    tracking for the rest of the file).
    """
    for pos, ctx, line, _match in scan_matches(text, DOUBLE_COLON):
        yield (pos, ctx, line)


def rewrite(text, path):
    out, decisions, last = [], [], 0
    for pos, ctx, line in scan(text):
        action = classify(ctx, path, text, pos)
        decisions.append((line, ctx, action, text[max(0, pos-30):pos+30].split("\n")[0]))
        if action == "keep":
            continue
        out.append(text[last:pos])
        nxt = text[pos+2] if pos+2 < len(text) else ""
        out.append("" if nxt == "<" else ".")          # turbofish `::<` -> `<`, else `::` -> `.`
        last = pos + 2
    out.append(text[last:])
    return "".join(out), decisions


def iter_vr():
    """Thin wrapper over the shared `vr_gate_scan.iter_vr_files` (T0652)."""
    return iter_vr_files(ROOT, ROOTS)


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "--check"
    tot_conv = tot_keep = changed = 0
    by_ctx = {}
    violations, code_c, string_c = [], [], []
    for path in iter_vr():
        try:
            text = open(path, encoding="utf-8").read()
        except Exception:
            continue
        if "::" not in text:
            continue
        new, decisions = rewrite(text, path)
        rel = os.path.relpath(path, ROOT)
        for line, ctx, action, snip in decisions:
            if action == "convert":
                tot_conv += 1
                by_ctx[ctx] = by_ctx.get(ctx, 0) + 1
                violations.append((rel, line, ctx, snip.strip()))
                if ctx == "code":
                    code_c.append((rel, line, snip.strip()))
                elif ctx == "string":
                    string_c.append((rel, line, snip.strip()))
            else:
                tot_keep += 1
        if mode == "--apply" and new != text:
            open(path, "w", encoding="utf-8").write(new)
            changed += 1

    if mode == "--check":
        if violations:
            print(f"GATE FAIL: {len(violations)} Rust-style `::` in .vr sources — use `.` (grammar/verum.ebnf).")
            print("Fix locally:  python3 vcs/scripts/check_no_double_colon.py --apply")
            for rel, line, ctx, snip in violations[:100]:
                print(f"  {rel}:{line} [{ctx}]  …{snip}…")
            if len(violations) > 100:
                print(f"  … and {len(violations) - 100} more")
            sys.exit(1)
        print("GATE OK: no Rust-style `::` in .vr sources (IPv6/SQL/URI/script DATA excluded).")
        sys.exit(0)

    print(f"=== SUMMARY ===  convert={tot_conv}  keep(data)={tot_keep}  by-context={by_ctx}")
    if mode == "--apply":
        print(f"files rewritten: {changed}")
        return
    print(f"\n=== CODE-context conversions (parse-affecting): {len(code_c)} ===")
    for rel, line, snip in code_c:
        print(f"  {rel}:{line}  {snip}")
    print(f"\n=== STRING-context conversions (semantic review): {len(string_c)} ===")
    for rel, line, snip in string_c:
        print(f"  {rel}:{line}  {snip}")


if __name__ == "__main__":
    main()
