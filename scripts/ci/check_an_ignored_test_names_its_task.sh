#!/usr/bin/env bash
# An `@ignore`d test must name the task that owns it.
#
# A disabled test is invisible in every green run: the suite reports
# success over a population that excludes it, so an absence and a pass
# are indistinguishable at the level anyone reads. Measured 2026-09-04,
# core-tests/ held 368 `@ignore` attributes and 336 of them (91%, over
# 102 files) carried no `T####` anywhere near.
#
# A RATCHET, NOT A ZERO. The 336 are frozen in
# scripts/ci/ignored_tests_without_a_task.txt; this gate fails only on a
# NEW one. A gate demanding zero here lands red on 336 sites and teaches
# everyone to ignore it — which is the defect it exists to cure.
# Removing a line is a test that acquired an owner or was deleted;
# ADDING one is a decision to disable a test with no record of why, and
# belongs in a commit message that says so.
#
# Usage:
#   scripts/ci/check_an_ignored_test_names_its_task.sh
#   scripts/ci/check_an_ignored_test_names_its_task.sh --selftest
#   scripts/ci/check_an_ignored_test_names_its_task.sh --write-baseline
#
# THE SELFTEST CARRIES THREE POLES because this finder was keyed wrong
# four times before it was right, and each wrong key produced a
# plausible number:
#
#   * `@ignore` appearing in PROSE must not count. The word occurs in
#     comments ("@ignore'd pins gated on the SIGSEGV class",
#     "transition from @ignore'd-SIGSEGV to GREEN"), which inflated a
#     count from 368 to 610.
#   * a site whose task is named TWO LINES UP must count as carried —
#     the reference is rarely on the attribute's own line.
#   * a genuinely bare site must be reported, or the gate is an
#     assertion of absence and passes for free.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

MODE="check"
[ "${1:-}" = "--selftest" ] && MODE="selftest"
[ "${1:-}" = "--write-baseline" ] && MODE="baseline"

python3 - "$MODE" <<'PY'
import os, re, sys

mode = sys.argv[1]
ROOT = "core-tests"
BASELINE = "scripts/ci/ignored_tests_without_a_task.txt"
# How far above the attribute to look for the owning task. Measured:
# the reference is almost never on the attribute's own line, and 10
# lines covers every carried site in the tree without reaching into an
# unrelated neighbouring test.
LOOKBACK = 10
IGNORE = re.compile(r'^\s*@ignore\b')
TASK = re.compile(r'\bT\d{4}\b')


def uncarried(text):
    """1-based line numbers of `@ignore` attributes with no task nearby."""
    out = []
    lines = text.split("\n")
    for i, line in enumerate(lines):
        # PROSE STRIPPED FIRST — the word appears in comments.
        if not IGNORE.search(line.split("//", 1)[0]):
            continue
        ctx = " ".join(lines[max(0, i - LOOKBACK):i + 3])
        if not TASK.search(ctx):
            out.append(i + 1)
    return out


if mode == "selftest":
    prose = (
        "// @ignore'd pins gated on the precompile-cascade SIGSEGV class\n"
        "// transition from @ignore'd-SIGSEGV to GREEN under --interp\n"
        "@test\nfn t() {}\n"
    )
    carried_above = (
        "// Blocked by T0620: terminate() bit-62 write is masked away.\n"
        "// Un-ignore once that lands.\n"
        "@ignore\n@test\nfn t() {}\n"
    )
    bare = "@ignore\n@test\nfn law_accept_key_rfc_vector() {}\n"
    r1, r2, r3 = uncarried(prose), uncarried(carried_above), uncarried(bare)
    print("SELFTEST")
    print(f"  `@ignore` in PROSE not counted:        {'yes' if not r1 else 'NO — inflates'}")
    print(f"  a task named two lines up = carried:   {'yes' if not r2 else 'NO — false positive'}")
    print(f"  a genuinely bare site reported:        {'yes' if r3 else 'NO — blind'}")
    sys.exit(0 if (not r1 and not r2 and r3) else 1)

found = []
for dirpath, _dirs, names in os.walk(ROOT):
    for n in sorted(names):
        if not n.endswith(".vr"):
            continue
        p = os.path.join(dirpath, n)
        try:
            text = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        for ln in uncarried(text):
            found.append(f"{p}:{ln}")

if mode == "baseline":
    with open(BASELINE, "w", encoding="utf-8") as fh:
        fh.write("# `@ignore`d tests in core-tests/ that name no owning task.\n")
        fh.write("#\n")
        fh.write("# A line REMOVED is a test that acquired an owner or was deleted.\n")
        fh.write("# A line ADDED is a decision to disable a test with no record of\n")
        fh.write("# why, and belongs in a commit message that says so.\n")
        fh.write("#\n")
        fh.write("# Regenerate: scripts/ci/check_an_ignored_test_names_its_task.sh --write-baseline\n")
        for f in found:
            fh.write(f + "\n")
    print(f"baseline written: {len(found)} site(s)")
    sys.exit(0)

known = set()
if os.path.exists(BASELINE):
    with open(BASELINE, encoding="utf-8") as fh:
        known = {l.strip() for l in fh if l.strip() and not l.startswith("#")}

now = set(found)
new = sorted(now - known)
gone = sorted(known - now)

print(f"scanned {ROOT}/: {len(now)} `@ignore` site(s) with no task named")
if gone:
    print(f"  {len(gone)} baseline line(s) no longer apply — regenerate with --write-baseline")
    for g in gone[:5]:
        print(f"    gone: {g}")
if not new:
    print("check-ignored-tests: OK")
    sys.exit(0)

print(f"\ncheck-ignored-tests: {len(new)} NEW disabled test(s) name no task\n")
for f in new:
    print(f"  {f}")
print()
print("A disabled test is invisible in every green run — an absence and a")
print("pass read alike. Name the task that owns it (T####) within ten")
print("lines, or delete the test.")
sys.exit(1)
PY
