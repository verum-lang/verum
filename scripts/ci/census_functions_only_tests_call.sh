#!/usr/bin/env bash
# Census: `pub fn` defined in src/ whose ONLY callers live in tests/ or
# benches/.
#
# WHY THIS EXISTS. Twice in one day this tree produced the same shape:
# a complete, correct-looking mechanism with its own green test suite,
# in parallel with a second copy that is the one actually running.
#
#   T1073  protocol.rs::check_full_conformance and four siblings — ~13
#          tests, zero production callers; the deciding conformance
#          check lives in infer/decls.rs and is weaker.
#   T1138  verum_lsp's IncrementalState::cache_node — one caller, a
#          test. Because the map is therefore always empty,
#          should_use_incremental() is unconditionally false, so an
#          entire second LSP's incremental path never runs, and a
#          criterion bench measures full parses under incremental names.
#
# A green suite reads as "this rule is enforced". This census asks the
# question the suite cannot: is anything but the suite asking?
#
# ADVISORY, NOT A GATE. A name it prints is a QUESTION, not a defect —
# a genuinely public API, a trait method called through dyn dispatch, or
# a macro-generated call site all land here legitimately. Making it
# blocking would train people to silence it, which is how a census stops
# being read.
#
# Usage:
#   scripts/ci/census_functions_only_tests_call.sh            # census
#   scripts/ci/census_functions_only_tests_call.sh --selftest # controls
#
# The self-test is not decoration. This script's whole output is a list
# of ABSENCES, and an absence claim passes for free when the instrument
# is broken — a typo in the regex, a path that matches nothing, a filter
# that drops everything. --selftest requires the two KNOWN instances
# above to be found, so a silent census can be told from a clean one.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

SELFTEST=0
[ "${1:-}" = "--selftest" ] && SELFTEST=1

python3 - "$SELFTEST" <<'PY'
import os, re, sys, collections

selftest = sys.argv[1] == "1"

DEF_RE  = re.compile(r'\bpub(?:\s*\([^)]*\))?\s+(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([a-z_][a-z0-9_]*)\s*[(<]')
CALL_RE = re.compile(r'(?:\.\s*)?\b([a-z_][a-z0-9_]*)\s*\(')
ANYFN_RE = re.compile(r'\bfn\s+([a-z_][a-z0-9_]*)\s*[(<]')

# Where a file lives decides which column its calls count in. The two
# must be counted APART: summing them is exactly the mistake that lets a
# test suite stand in for a caller.
def bucket(path):
    parts = path.split(os.sep)
    if "tests" in parts or "benches" in parts or "examples" in parts:
        return "test"
    if path.endswith("_test.rs") or path.endswith("_tests.rs"):
        return "test"
    return "src"

defs = {}          # name -> list of (file, line)
calls = collections.defaultdict(lambda: {"src": 0, "test": 0})
defcount = collections.Counter()   # name -> how many fns share this name

files = []
for root, dirs, names in os.walk("crates"):
    dirs[:] = [d for d in dirs if d not in (".git", "target")]
    for n in names:
        if n.endswith(".rs"):
            files.append(os.path.join(root, n))

for path in files:
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        continue
    file_bucket = bucket(path)
    lines = text.split("\n")

    # An inline `#[cfg(test)] mod` lives in a src/ file but is a test.
    # Counting its calls as production is precisely how a mechanism that
    # only its own tests exercise reads as "used" — the first draft of
    # this census made that mistake and its own positive control caught
    # it (protocol.rs: 10 of 11 apparent src callers were inline tests,
    # the eleventh a doc comment).
    in_test_mod = False
    test_depth = 0
    pending_cfg_test = False

    for i, line in enumerate(lines, 1):
        s = line.strip()

        if in_test_mod:
            test_depth += line.count("{") - line.count("}")
            if test_depth <= 0:
                in_test_mod = False
        elif pending_cfg_test:
            if "mod " in s:
                in_test_mod = True
                test_depth = line.count("{") - line.count("}")
                pending_cfg_test = False
            elif s and not s.startswith("#"):
                # `#[cfg(test)]` on something that is not a module (a
                # single fn, a use). Not a region; stop waiting.
                pending_cfg_test = False
        if s.startswith("#[cfg(test)]"):
            pending_cfg_test = True
            continue

        b = "test" if (file_bucket == "test" or in_test_mod) else "src"

        # Comments are not call sites. A doc comment showing intended
        # usage looks exactly like a caller to a regex, and reads as
        # "somebody uses this" — the worst possible false negative for a
        # census of things nobody uses.
        code = line.split("//", 1)[0]
        if not code.strip():
            continue

        if b == "src":
            for m in DEF_RE.finditer(code):
                defs.setdefault(m.group(1), []).append((path, i))
        for m in ANYFN_RE.finditer(code):
            defcount[m.group(1)] += 1

        # Count calls, then subtract definitions on the same line: a
        # definition matches CALL_RE too, and counting it as a caller
        # would make every function look used exactly once.
        for m in CALL_RE.finditer(code):
            calls[m.group(1)][b] += 1
        for m in ANYFN_RE.finditer(code):
            calls[m.group(1)][b] -= 1

# A helper that exists TO BE called by tests is not a finding. Filter
# it, but COUNT what was filtered and say so below: a census that
# quietly drops a category reads as "nothing there" for that category.
HELPER_NAME = re.compile(r'(^|_)(test|mock|fixture|dummy|stub)(_|$)')
HELPER_FILE = re.compile(r'(^|/)(test|tests|testing|test_utils|test_support)\.rs$')

findings, helpers = [], []
for name, sites in sorted(defs.items()):
    c = calls[name]
    if c["src"] > 0 or c["test"] <= 0:
        continue
    row = (name, sites, c["test"], defcount[name])
    if HELPER_NAME.search(name) or any(HELPER_FILE.search(s[0].replace(os.sep, "/")) for s in sites):
        helpers.append(row)
    else:
        findings.append(row)

if selftest:
    # POSITIVE CONTROLS. Both are real, both were confirmed by hand.
    # If the census cannot see them it is not clean, it is blind.
    want = {"cache_node", "check_full_conformance"}
    found = {f[0] for f in findings}
    missing = want - found
    print("SELFTEST — known instances the census must find:")
    for w in sorted(want):
        print(f"  {'FOUND  ' if w in found else 'MISSING'}  {w}")
    # NEGATIVE CONTROL. A name called constantly from src/ must NOT be
    # listed; without this the script could 'pass' by listing everything.
    noisy = [n for n in ("len", "clone", "push") if n in found]
    print(f"  negative control (len/clone/push must be absent): "
          f"{'FAIL: ' + ', '.join(noisy) if noisy else 'ok'}")
    print(f"  census size: {len(findings)} names (+{len(helpers)} test helpers) "
          f"over {len(files)} files")
    sys.exit(1 if missing or noisy else 0)

print(f"census: {len(files)} .rs files under crates/")
print(f"        {len(defs)} distinct `pub fn` names defined in src/")
print(f"        {len(findings) + len(helpers)} of them are called ONLY from tests/benches")
print(f"        {len(helpers)} of those are named or placed as test helpers "
      f"and are listed separately at the end, not dropped")
print()
print("Each line is a QUESTION, not a verdict. Read it as: what asks for")
print("this outside the suite that tests it? Two answers seen so far that")
print("are NOT defects: an opt-in cargo feature whose whole block is off")
print("by default (verum_vbc's `metal`), and an API meant for embedders.")
print("Sorted by test-call count: a mechanism with many tests and no")
print("caller is the shape worth reading first.")
print()
findings.sort(key=lambda r: -r[2])
helpers.sort(key=lambda r: -r[2])
for name, sites, ntest, ndef in findings:
    amb = f"  [{ndef} fns share this name]" if ndef > 1 else ""
    where = sites[0][0] + ":" + str(sites[0][1])
    extra = f" (+{len(sites)-1} more definitions)" if len(sites) > 1 else ""
    print(f"  {name:<44} {ntest:>3} test call(s)   {where}{extra}{amb}")

print()
print(f"-- {len(helpers)} named or placed as test helpers (listed, not dropped) --")
for name, sites, ntest, ndef in helpers:
    where = sites[0][0] + ":" + str(sites[0][1])
    print(f"  {name:<44} {ntest:>3} test call(s)   {where}")
PY
