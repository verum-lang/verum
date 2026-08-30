#!/usr/bin/env python3
"""Census: tests that check a FACT (it ran / it parsed) rather than a RESULT.

Two independent instances of this class were measured on 2026-08-30:
96 of 99 tests in `verum_fast_parser/tests/precedence_tests.rs` asserted
only that a string parses (so `test_bitwise_and_before_or` passed
whichever way the operators bound), and 164 verification specs ran no
verifier at all.

Classification of each `#[test]` body:

  COMPARING  — contains assert_eq!/assert_ne!/matches!/assert!(a == b)
               or a comparison inside assert!: it checks a RESULT.
  FACT-ONLY  — the only assertions are "it did not blow up": .unwrap(),
               .expect(), assert!(x.is_ok()), assert!(x.is_some()),
               or a call to a helper whose whole body is one of those.
  NO-ASSERT  — no assertion of any kind.

FACT-ONLY and NO-ASSERT are the class. They are not automatically
wrong — a parse-acceptance test is a legitimate thing to want — but a
test NAMED for a property it never compares is indistinguishable from
a passing one, which is how a wrong precedence ladder survived.
"""

from __future__ import annotations

import re
import sys
from collections import Counter
from pathlib import Path

REPO = Path(sys.argv[1] if len(sys.argv) > 1 else
            Path(__file__).resolve().parents[2])

COMPARING = re.compile(
    r"assert_eq!|assert_ne!|matches!\s*\(|assert!\s*\([^)]*[=<>!]=|"
    r"\.contains\(|assert_matches!|panic!\s*\(\s*\"expected"
)
FACT_ONLY = re.compile(
    r"\.unwrap\(\)|\.expect\(|is_ok\(\)|is_some\(\)|is_err\(\)|is_none\(\)|"
    r"assert_parses|assert_fails|\.is_empty\(\)"
)
ANY_ASSERT = re.compile(r"assert|panic!|unwrap|expect\(")


def test_bodies(src: str):
    """Yield (name, body) for each #[test] function."""
    for m in re.finditer(r"#\[test\][^\n]*\n(?:\s*#\[[^\]]*\]\s*\n)*\s*(?:async\s+)?fn\s+(\w+)", src):
        name = m.group(1)
        start = src.find("{", m.end())
        if start < 0:
            continue
        depth, i = 0, start
        while i < len(src):
            if src[i] == "{":
                depth += 1
            elif src[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        yield name, src[start : i + 1]


def spec_corpus(repo: Path) -> None:
    """The same question asked of `vcs/specs`: does the spec compare a result?"""
    import collections

    kinds: collections.Counter = collections.Counter()
    run_kind: collections.Counter = collections.Counter()
    specs = list((repo / "vcs" / "specs").rglob("*.vr"))
    if not specs:
        print("\nno vcs/specs found — skipping the conformance half")
        return
    for path in specs:
        try:
            t = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        m = re.search(r"@test:\s*([a-z-]+)", t)
        kind = m.group(1) if m else "(none)"
        kinds[kind] += 1
        if kind not in ("run", "run-interpreter"):
            continue
        exits = re.findall(r"@expected-exit:\s*(\d+)", t)
        if "@expected-stdout" in t:
            run_kind["compares stdout"] += 1
        elif "@expected-error" in t:
            run_kind["expects a specific error"] += 1
        elif any(e != "0" for e in exits):
            run_kind["expects a non-zero exit"] += 1
        elif exits:
            run_kind["expects exit 0 — i.e. did not crash"] += 1
        else:
            run_kind["nothing beyond not crashing"] += 1

    total_run = sum(run_kind.values())
    print(f"\n{len(specs)} conformance specs; {total_run} of them RUN a program\n")
    for k, v in run_kind.most_common():
        print(f"  {v:5d}  ({100 * v / total_run:4.1f}%)  {k}")


def main() -> int:
    per_file: dict[Path, Counter] = {}
    totals = Counter()
    worst: list[tuple[int, int, Path]] = []

    for path in sorted(REPO.glob("crates/*/tests/*.rs")):
        try:
            src = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        counts = Counter()
        for _name, body in test_bodies(src):
            if COMPARING.search(body):
                counts["comparing"] += 1
            elif FACT_ONLY.search(body):
                counts["fact_only"] += 1
            elif ANY_ASSERT.search(body):
                counts["fact_only"] += 1
            else:
                counts["no_assert"] += 1
        if not counts:
            continue
        per_file[path] = counts
        totals.update(counts)
        weak = counts["fact_only"] + counts["no_assert"]
        total = sum(counts.values())
        if total >= 10 and weak / total >= 0.6:
            worst.append((weak, total, path))

    grand = sum(totals.values())
    print(f"{grand} #[test] functions across {len(per_file)} files\n")
    print(f"  comparing a RESULT : {totals['comparing']:5d}  "
          f"({100 * totals['comparing'] / grand:.1f}%)")
    print(f"  fact-only          : {totals['fact_only']:5d}  "
          f"({100 * totals['fact_only'] / grand:.1f}%)")
    print(f"  no assertion       : {totals['no_assert']:5d}  "
          f"({100 * totals['no_assert'] / grand:.1f}%)")

    print(f"\nFiles where >=60% of >=10 tests never compare a result "
          f"({len(worst)}):\n")
    for weak, total, path in sorted(worst, reverse=True)[:30]:
        print(f"  {weak:4d}/{total:4d}  {path.relative_to(REPO)}")
    spec_corpus(REPO)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
