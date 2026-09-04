#!/usr/bin/env bash
# Every `super.<a>.<b>...` call in core/ must name a module that exists.
#
# WHY. `core/sys/common.vr`'s Windows arm of `random_bytes` calls
#
#     super.windows.bcrypt.BCryptGenRandom(...)
#     ...
#     return Result.Ok(());
#
# and `core/sys/windows/bcrypt.vr` does not exist. A dotted module-path
# call that resolves to nothing is SILENT in this compiler (register row
# A38), so on Windows that function writes NOTHING into the caller's
# buffer and reports success. The heap's pointer-encoding keys are then
# seeded with zeros, and every layer above certifies that seeding
# worked. The security property is not degraded — it is absent and
# self-certifying (T0807).
#
# Nothing about that needed a compiler. It needed `ls`.
#
# RESOLUTION RULE. For a file `core/a/b.vr` the module is `core.a.b` and
# `super` is `core.a`; for `core/a/mod.vr` the module is `core.a` and
# `super` is `core`. A call `super.X.Y.Z` may mean module `<super>.X.Y`
# with function `Z`, or module `<super>.X` with `Y.Z` being a type and
# its method — the grammar does not say which from the call site alone.
# So this gate is deliberately CONSERVATIVE: it fails only when NO
# prefix of the path resolves to a file. One resolving prefix is enough
# to stay silent, which means it cannot cry wolf about a well-formed
# call whose shape it merely fails to parse.
#
# Usage:
#   scripts/ci/check_a_super_path_call_names_a_file_that_exists.sh
#   scripts/ci/check_a_super_path_call_names_a_file_that_exists.sh --selftest
#
# --selftest is not decoration. This gate's normal output is an ABSENCE,
# and an absence claim passes for free when the finder is broken — a
# wrong path root, a regex that matches nothing, a resolution rule that
# accepts everything. The self-test plants a call whose module cannot
# exist and requires it to be reported.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

SELFTEST=0
[ "${1:-}" = "--selftest" ] && SELFTEST=1

python3 - "$SELFTEST" <<'PY'
import os, re, sys

selftest = sys.argv[1] == "1"
ROOT = "core"
# `super` may repeat — `super.super.x` climbs two levels. Capture the
# run of `super.` prefixes separately from the path that follows.
CALL = re.compile(r'\b((?:super\.)*)super((?:\.[A-Za-z_][A-Za-z0-9_]*)+)')

def module_of(path):
    """core/a/b.vr -> ['core','a','b'];  core/a/mod.vr -> ['core','a']"""
    parts = path[:-3].split(os.sep)          # drop .vr
    if parts[-1] == "mod":
        parts = parts[:-1]
    return parts

# A module need not be a FILE. `core/mod.vr:535` declares
# `public module prelude { … }` inline, and core.prelude is a published
# surface that fifteen files mount. Without this index the gate reported
# forty-five false positives, all of them that one module.
INLINE = set()
def index_inline_modules(root):
    decl = re.compile(r'^\s*(?:public\s+)?module\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{')
    for dirpath, dirs, names in os.walk(root):
        for n in names:
            if not n.endswith(".vr"):
                continue
            fp = os.path.join(dirpath, n)
            parts = fp[:-3].split(os.sep)
            if parts[-1] == "mod":
                parts = parts[:-1]
            try:
                text = open(fp, encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            for line in text.split("\n"):
                m = decl.match(line)
                if m:
                    INLINE.add(tuple(parts + [m.group(1)]))

def exists(mod_parts):
    """A module exists if <p>.vr or <p>/mod.vr is a file, or if some
    file declares it INLINE (`public module X { … }`)."""
    p = os.sep.join(mod_parts)
    if os.path.isfile(p + ".vr") or os.path.isfile(os.path.join(p, "mod.vr")):
        return True
    return tuple(mod_parts) in INLINE

def check_text(path, text):
    """Return the list of (line, call) whose every prefix is unresolvable."""
    bad = []
    sup = module_of(path)[:-1]               # `super` = parent module
    if not sup:
        return bad
    for i, line in enumerate(text.split("\n"), 1):
        code = line.split("//", 1)[0]
        if "super" not in code:
            continue
        for m in CALL.finditer(code):
            climbs = m.group(1).count("super") + 1      # this `super` too
            segs = [s for s in m.group(2).split(".") if s]
            if not segs:
                continue
            # A MODULE segment is lower-case; a TYPE is capitalised.
            # `super.TcpStream` is a re-exported type, not a module path,
            # and flagging it would drown the real findings — `core/net/
            # mod.vr` alone re-exports eight. Require the first segment
            # to look like a module.
            if not segs[0][0].islower():
                continue
            # A call INTO another module needs at least module + name.
            # A single segment — `super.spawn`, `super.bold` — is a
            # re-export of a function from the parent module, which is
            # not a path and cannot name a missing file. Without this
            # the gate reports fifty of them and buries the one finding
            # it exists for.
            #
            # EXCEPT in the brace-mount form, where ONE segment IS the
            # module: `mount super.elementary.{sqrt, log};`. Skipping
            # those hid a real finding — core/random/deterministic.vr
            # mounts a module that does not exist at :45 and calls into
            # it at :559, and only the call was reported.
            brace_mount = (code.lstrip().startswith("mount")
                           and code[m.end():].lstrip().startswith(".{"))
            if len(segs) < 2 and not brace_mount:
                continue
            base = module_of(path)[:-climbs]
            if not base:
                continue
            # WHICH PREFIX IS THE MODULE. The first version of this
            # gate accepted "any prefix resolves", and that made it
            # blind to the case it was written for:
            # `super.windows.bcrypt.BCryptGenRandom` passed because
            # `core/sys/windows` exists, even though `bcrypt` does not.
            # A gate that cannot see its own motivating example is
            # worse than none.
            #
            # The last segment's CASE says what it is. Capitalised, it
            # is a function or a type, so everything before it must be
            # a module and must resolve exactly. Lower-case, it may be
            # a function (module = all but last) or a method on a type
            # (module = all but last two), so either resolving is
            # enough.
            # A `mount` names a MODULE, not a call: every segment is
            # part of the path and none of them is a function.
            # `mount super.rules.k_var.{k_var_sound};` was reported as
            # missing because the gate dropped `k_var` and looked for a
            # `rules` module — and `rules/` has no `mod.vr`, so it does
            # not resolve as one even though `rules/k_var.vr` is there.
            # Two mount forms, and only one makes every segment part of
            # the module:
            #   mount super.rules.k_var.{k_var_sound};   module = rules.k_var
            #   mount super.scope.Scope;                 module = scope
            # The brace is the discriminator. Reading `mount` alone and
            # taking every segment turned 13 findings into 161, all of
            # them the second form.
            if code.lstrip().startswith("mount"):
                # `.{` and `.*` are the same case: a brace list and a glob
                # both import FROM a module, so every captured segment is
                # part of the path. Reading only `.{` flagged
                # `mount core.collections.*;` whose module plainly exists.
                rest = code[m.end():].lstrip()
                brace = rest.startswith(".{") or rest.startswith(".*")
                candidates = [segs] if brace else [segs[:-1]]
            elif segs[-1][0].isupper():
                candidates = [segs[:-1]]
            else:
                candidates = [segs[:-1], segs[:-2]]
            if not any(c and exists(base + c) for c in candidates):
                bad.append((i, m.group(0)))
    return bad

if selftest:
    index_inline_modules(ROOT)
    # POSITIVE CONTROL — a module that cannot exist, from a real file's
    # position, must be reported.
    planted = "fn f() { super.zzqnosuch.alsonot.call(1); }\n"
    got = check_text(os.path.join("core", "sys", "common.vr"), planted)
    print("SELFTEST")
    print(f"  planted unresolvable call reported: {'yes' if got else 'NO — the finder is blind'}")
    # NEGATIVE CONTROL — a call that DOES resolve must not be reported.
    ok = "fn f() { super.linux.syscall.getrandom(b, 0); }\n"
    got2 = check_text(os.path.join("core", "sys", "common.vr"), ok)
    print(f"  resolvable call stayed silent:      {'yes' if not got2 else 'NO — false positive'}")
    sys.exit(0 if (got and not got2) else 1)

# ---------------------------------------------------------------- absolute
# The same file-existence rule, applied to `mount core.a.b...`. A move
# breaks relative imports; a rename or a deletion breaks absolute ones,
# and both are silent. Same four path forms, same conservative
# resolution — only the base differs, and here it is the tree root.
ABS = re.compile(r'\bcore((?:\.[A-Za-z_][A-Za-z0-9_]*)+)')

def check_absolute(path, text):
    bad = []
    for i, line in enumerate(text.split("\n"), 1):
        code = line.split("//", 1)[0]
        if "core." not in code:
            continue
        stripped = code.lstrip()
        if not stripped.startswith("mount"):
            continue
        for m in ABS.finditer(code):
            segs = [x for x in m.group(1).split(".") if x]
            if not segs:
                continue
            rest = code[m.end():].lstrip()
            brace = rest.startswith(".{") or rest.startswith(".*")
            if brace:
                candidates = [segs]
            elif segs[-1][0].isupper():
                candidates = [segs[:-1]]
            else:
                candidates = [segs[:-1], segs[:-2]]
            if not any(c and exists(["core"] + c) for c in candidates):
                bad.append((i, m.group(0)))
    return bad

index_inline_modules(ROOT)

files, findings = 0, []
for dirpath, dirs, names in os.walk(ROOT):
    for n in names:
        if not n.endswith(".vr"):
            continue
        p = os.path.join(dirpath, n)
        files += 1
        try:
            text = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        for line, call in check_text(p, text):
            findings.append((p, line, call))
        for line, call in check_absolute(p, text):
            findings.append((p, line, call))

print(f"scanned {files} .vr files under {ROOT}/")
if not findings:
    print("check-super-paths: OK — every super-path call names a module that exists")
    sys.exit(0)

print(f"check-super-paths: {len(findings)} call(s) name a module that does not exist\n")
for p, line, call in findings:
    print(f"  {p}:{line}  {call}")
print()
print("Such a call resolves to nothing SILENTLY (register row A38): the")
print("callee is never entered, out-params are never written, and the")
print("caller's success path runs anyway.")
sys.exit(1)
PY
