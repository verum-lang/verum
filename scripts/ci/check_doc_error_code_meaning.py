#!/usr/bin/env python3
"""A documented error code must mean what the compiler PRINTS for it.

WHY THE EMITTER AND NOT THE REGISTRY
------------------------------------
`crates/verum_error/src/registry.rs` is the obvious authority and is the
wrong one. Measured 2026-09-10, seven codes carry a registry description
that no emit site produces, and the site's diagnostics reference is a
VERBATIM COPY of those seven rows:

    code   registry / site said        the compiler prints
    E311   double move                 cannot borrow `x` because field `f`
                                       is already borrowed
    E313   dangling reference          cannot move `x` while it is borrowed
    E501   SMT solver timeout          invalid refinement predicate
    E502   refinement predicate false  meta function uses runtime context(s)
    E503   precondition not satisfied  pure function has side effects
    E601   context conflict            visibility error: 'x' is <vis> in …
    E602   context cycle               ambiguous name: 'x' is imported from
                                       multiple modules

A gate comparing the SITE against the REGISTRY would have been green on
all seven. Two copies of one mistake agree with each other, and their
agreement is the thing that makes the mistake durable: whichever you
check, the other confirms it. The emitter is the only one of the three
a user actually meets.

The registry half is somebody else's task; this gate is about the site.
Both must move, and neither fix implies the other — `verum explain`
reads the registry.

THE RULE IS DELIBERATELY WEAK. A one-line reference description is a
SUMMARY, and a fair summary may share no vocabulary with the message:
`E312` reads "lifetime error" against "does not live long enough —
reference outlives referent", and that is correct English for the same
condition. So the gate asks only for one significant word in common
with SOME emit site for that code, and the summaries that legitimately
share none live on a roster with the reason.

What it therefore catches is not "imprecise" but "about something
else" — which is the whole population it was built from.
"""

from __future__ import annotations

import collections
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CRATES = REPO / "crates"
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"
PAGE = DOCS / "reference" / "diagnostics.md"

CODE_CALL = re.compile(r'\.code\("(E\d{3})"\)')
MESSAGE = re.compile(r'\.message\(\s*(?:format!\(\s*)?"([^"]{4,120})')
ROW = re.compile(r"^\|\s*`(E\d{3})`\s*\|\s*([^|]+?)\s*\|", re.M)
WORD = re.compile(r"[a-z]{4,}")

# Summaries that share no vocabulary with the message and are still
# right. Each needs the reason, because "it reads fine to me" is how the
# seven above survived.
FAIR_SUMMARY: dict[str, str] = {
    "E312": "`lifetime error` against `does not live long enough — reference "
            "outlives referent`: correct English for the same condition",
    "E500": "`contract violated` against `refinement constraint failed`: a "
            "refinement IS the contract this code is about",
}

# Words too generic to count as agreement on their own.
STOP = {
    "error", "cannot", "must", "with", "this", "that", "from", "into", "than",
    "there", "which", "while", "have", "been", "when", "does", "type", "value",
    "name", "used", "help", "here",
}


def emitter_messages() -> dict[str, list[str]]:
    out: dict[str, list[str]] = collections.defaultdict(list)
    for f in sorted(CRATES.rglob("*.rs")):
        if "registry.rs" in str(f) or f"{os.sep}tests{os.sep}" in str(f):
            continue
        try:
            text = f.read_text(errors="replace")
        except OSError:
            continue
        for m in CODE_CALL.finditer(text):
            tail = text[m.end(): m.end() + 320]
            s = MESSAGE.search(tail)
            if s:
                out[m.group(1)].append(s.group(1).split("\\n")[0].strip())
    return out


def documented() -> dict[str, str]:
    if not PAGE.is_file():
        return {}
    return {m.group(1): m.group(2) for m in ROW.finditer(PAGE.read_text(errors="replace"))}


STEM = 5


def disagrees(desc: str, messages: list[str]) -> bool:
    """True when NO word of the summary appears in any emitter message.

    Compared by five-character STEM, not by substring: `E406` reads
    "type inference failure" against "cannot infer lambda type", and
    `inference` is not a substring of `infer`. Stemming un-flags that
    one and leaves every real finding caught — checked against all
    seven before it was adopted, which is the only reason to trust a
    loosening.
    """
    words = {w for w in WORD.findall(desc.lower())} - STOP
    if not words:
        return False
    blob = " ".join(messages).lower()
    stems = {w[:STEM] for w in WORD.findall(blob)}
    return not any(w[:STEM] in stems for w in words)


def self_test() -> int:
    bad = 0
    cases = [
        ("agrees", "ambiguous name — imported from more than one module",
         ["ambiguous name: '{}' is imported from multiple modules: {}"], False),
        ("about something else", "context cycle",
         ["ambiguous name: '{}' is imported from multiple modules: {}"], True),
        ("agrees through one word only", "a pure function has side effects",
         ["pure function `{}` has side effects: {}"], False),
        ("generic words alone are not agreement", "error value type",
         ["ambiguous name: '{}' is imported from multiple modules"], False),
        ("agrees by stem, not substring", "type inference failure",
         ["cannot infer lambda type"], False),
        ("a stem match must be a real one", "context cycle",
         ["cannot infer lambda type"], True),
    ]
    for label, desc, msgs, want in cases:
        got = disagrees(desc, msgs)
        if got != want:
            print(f"self-test: {label}: expected disagrees={want}, got {got}",
                  file=sys.stderr)
            bad += 1

    # The anchor: the code that started this, and the one that is a fair
    # summary. A detector that cannot separate these two has not measured
    # anything.
    if not disagrees("context cycle",
                     ["ambiguous name: '{}' is imported from multiple modules"]):
        print("self-test: anchor E602 not caught", file=sys.stderr)
        bad += 1
    if "E312" not in FAIR_SUMMARY or "E500" not in FAIR_SUMMARY:
        print("self-test: the fair-summary roster lost an entry", file=sys.stderr)
        bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(cases)} case(s), 1 anchor, "
          f"{len(FAIR_SUMMARY)} fair summaries pinned")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not PAGE.is_file():
        print(f"[skip] {PAGE} not present (the site is a separate checkout).")
        return 0

    emit = emitter_messages()
    doc = documented()
    if len(emit) < 20:
        print(f"check-doc-error-code-meaning: only {len(emit)} code(s) have an "
              f"emitter message — the emit shape changed and this gate would "
              f"go quiet rather than green", file=sys.stderr)
        return 2

    comparable = sorted(set(emit) & set(doc))
    off = [c for c in comparable
           if c not in FAIR_SUMMARY and disagrees(doc[c], emit[c])]
    stale = sorted(c for c in FAIR_SUMMARY
                   if c in comparable and not disagrees(doc[c], emit[c]))

    print(f"check-doc-error-code-meaning: {len(doc)} documented code(s), "
          f"{len(emit)} with an emitter message, {len(comparable)} comparable "
          f"— {len(off)} describing something the compiler never prints, "
          f"{len(FAIR_SUMMARY)} fair summaries on the roster")

    for c in off:
        print(f"    + {c}  page: {doc[c]!r}")
        print(f"           emits: {emit[c][0][:80]!r}")
    for c in stale:
        print(f"    - {c} now shares vocabulary with its message; drop it from "
              f"FAIR_SUMMARY so the roster keeps meaning something")
    return 1 if (off or stale) else 0


if __name__ == "__main__":
    sys.exit(main())
