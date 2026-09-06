#!/usr/bin/env python3
"""A `core/` type declared without `public` must not be named by a public signature.

WHY THIS IS A DEFECT AND NOT A STYLE QUESTION.  A94 made non-`public` stdlib
declarations stop answering in user programs, which is right: a program that
mounts nothing could name 302 private `core/` constants.  But visibility is a
statement about a NAME, and a signature is a statement about a TYPE — so a
public method may return a type the caller is forbidden to name, and then
every method on the result is unreachable.  The diagnostic points at the
method:

    l.iter().transduce(id).fold(0, f)
    error<E400>: no method named `fold` found for type
                 `TransducedIter<ListIter<A>, &A>`

`Iterator.transduce` is a default method of the PUBLIC `Iterator` protocol and
returns `TransducedIter<Self, B>`, and `core/base/iterator.vr` declared that
type without `public` — the only two such declarations in that file, against 54
public ones.  It cost registry-next its whole search module, and nothing in the
tree announced it: the type checker reported a missing METHOD.

WHY IT WAS INVISIBLE FOR SO LONG.  The flag is written by the stdlib BAKE, so
a build served from a cached archive keeps the old, permissive metadata however
new its Rust is.  Six consecutive binaries spanning six hours all reported
`Stdlib precompile cache HIT (blake3 cce44fb820de7fe1)` and all behaved as if
A94 had never landed.  A source-level gate does not depend on a bake and is the
only place this can be checked cheaply.

WHAT IT COUNTS.  A top-level `type X …` in `core/` with no `public`, whose bare
name appears in a signature that is itself public:

  * `public fn …` at top level,
  * `public fn …` inside an `implement` block,
  * any `fn …` declared in a `public type … is protocol { … }` body.

SIGNATURES ARE JOINED ACROSS LINES, and that is not a detail: the first version
of this census matched single lines and reported 8.  It missed
`StatefulTransducedIter`, whose forcing signature — `Iterator.transduce_stateful`
— puts its return type on a continuation line.  Joining raised the answer to 11.
A census that reads only the first line of a declaration undercounts by exactly
the declarations someone took care to format.

WHAT IT DELIBERATELY DOES NOT COUNT: a private type named only in private
signatures, which is the ordinary and correct use of a private type — 194 of
them, and they are none of this gate's business.
"""

import os
import re
import sys
import tempfile

CORE = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__)))), "core")

# A ratchet at zero: the eleven that existed were fixed in the same change
# that installed this, so a first violation is a new one.
BASELINE = 0

PUBLIC_TYPE = re.compile(r"^public type ([A-Z][A-Za-z0-9_]*)")
PRIVATE_TYPE = re.compile(r"^type ([A-Z][A-Za-z0-9_]*)")
PUBLIC_PROTO = re.compile(r"^public type ([A-Z][A-Za-z0-9_]*)(<[^>]*>)? is protocol")
TOP_PUBLIC_FN = re.compile(r"^public\s+(pure\s+|async\s+|const\s+|unsafe\s+)*fn\s")
IMPL_PUBLIC_FN = re.compile(r"^\s{4}public\s+(pure\s+|async\s+|const\s+|unsafe\s+)*fn\s")
PROTO_FN = re.compile(r"^\s{4}(pure\s+|async\s+|const\s+|unsafe\s+)*fn\s")
IDENT = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\b")


def vr_files(root):
    for dirpath, _, files in os.walk(root):
        for fn in sorted(files):
            if fn.endswith(".vr"):
                yield os.path.join(dirpath, fn)


def declarations(root):
    """(private_types, public_type_names) over the tree."""
    public, private = set(), {}
    for path in vr_files(root):
        with open(path, encoding="utf-8", errors="replace") as fh:
            for lineno, line in enumerate(fh, 1):
                m = PUBLIC_TYPE.match(line)
                if m:
                    public.add(m.group(1))
                    continue
                m = PRIVATE_TYPE.match(line)
                if m:
                    private.setdefault(m.group(1), (path, lineno))
    return {k: v for k, v in private.items() if k not in public}, public


def joined_signature(lines, i):
    """The signature starting at `lines[i]`, joined across continuations.

    Stops at the line that closes the parameter list AND carries either the
    return arrow, the opening brace of a body, or the `;` of an abstract
    declaration — whichever comes first, and never more than twelve lines.
    """
    out, depth = [], 0
    for j in range(i, min(i + 12, len(lines))):
        text = lines[j]
        out.append(text.strip())
        depth += text.count("(") - text.count(")")
        closed = depth <= 0 and (")" in text or j > i)
        if closed and ("->" in text or "{" in text or text.rstrip().endswith(";")):
            break
    return " ".join(out)


def census(root):
    private, public = declarations(root)
    hits = []
    for path in vr_files(root):
        with open(path, encoding="utf-8", errors="replace") as fh:
            lines = fh.read().splitlines()
        in_public_protocol, protocol = False, None
        for i, line in enumerate(lines):
            m = PUBLIC_PROTO.match(line)
            if m:
                in_public_protocol, protocol = True, m.group(1)
            elif in_public_protocol and line.startswith("};"):
                in_public_protocol, protocol = False, None
            if TOP_PUBLIC_FN.match(line):
                where = "public fn"
            elif IMPL_PUBLIC_FN.match(line):
                where = "public method"
            elif in_public_protocol and PROTO_FN.match(line):
                where = f"protocol {protocol}"
            else:
                continue
            signature = joined_signature(lines, i)
            for name in sorted(set(IDENT.findall(signature))):
                if name in private:
                    hits.append((name, private[name], where, path, i + 1))
    return private, public, hits


FIXTURE_FORCED = """\
public type Feed is protocol {
    fn open(&self) -> Int;
    fn tapped<B>(self, k: B) -> TappedFeed<Self, B> {
        TappedFeed { source: self, mark: k }
    }
};

type TappedFeed<I: Feed, B> is { source: I, mark: B };
"""

FIXTURE_CLEAN = """\
public type Feed is protocol {
    fn open(&self) -> Int;
};

type InternalOnly is { n: Int };

implement Feed for InternalOnly {
    fn open(&self) -> Int { self.n }
}

fn helper(x: &InternalOnly) -> Int { x.n }
"""


def self_test():
    """The instrument must fire on a known violation AND stay quiet on a
    private type used privately.  A census with only the second half reports
    zero for whatever reason and reads as a clean bill."""
    ok = True
    with tempfile.TemporaryDirectory() as tmp:
        forced = os.path.join(tmp, "forced")
        clean = os.path.join(tmp, "clean")
        os.makedirs(forced)
        os.makedirs(clean)
        with open(os.path.join(forced, "a.vr"), "w", encoding="utf-8") as fh:
            fh.write(FIXTURE_FORCED)
        with open(os.path.join(clean, "b.vr"), "w", encoding="utf-8") as fh:
            fh.write(FIXTURE_CLEAN)

        _, _, hits = census(forced)
        names = {h[0] for h in hits}
        if "TappedFeed" not in names:
            print("  self-test FAIL: a private type returned by a public "
                  "protocol default was NOT reported")
            ok = False
        else:
            print("  self-test ok: the forced violation is reported")

        _, _, hits = census(clean)
        if hits:
            print(f"  self-test FAIL: a private type used only privately was "
                  f"reported: {sorted({h[0] for h in hits})}")
            ok = False
        else:
            print("  self-test ok: a private type used privately is not reported")
    return ok


def main(argv):
    if "--self-test" in argv:
        return 0 if self_test() else 1
    if not os.path.isdir(CORE):
        # ALWAYS fatal: `core/` is IN this repository. Its absence is a
        # broken checkout, never 'nothing to check', so there is no local
        # mode in which skipping is the right answer.
        print(f"private-types: core/ not found at {CORE} — the checkout is "
              f"broken; refusing to report OK.", file=sys.stderr)
        return 2
    private, public, hits = census(CORE)
    names = sorted({h[0] for h in hits})
    print(f"check-private-types-off-public-surface: {len(names)} private type(s) "
          f"named by a public signature (baseline {BASELINE}); "
          f"{len(private)} private and {len(public)} public declarations in core/")
    for name in names:
        _, (dpath, dline), where, upath, uline = next(h for h in hits if h[0] == name)
        rel_d = os.path.relpath(dpath, os.path.dirname(CORE))
        rel_u = os.path.relpath(upath, os.path.dirname(CORE))
        print(f"    {name:26s} declared {rel_d}:{dline}")
        print(f"    {'':26s} named by {where} at {rel_u}:{uline}")
    if "--check" in argv and len(names) > BASELINE:
        print(f"check-private-types-off-public-surface: FAIL — {len(names)} "
              f"exceeds {BASELINE}. Give the type `public`, or stop naming it "
              f"in a public signature; a type a caller receives is public API.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
