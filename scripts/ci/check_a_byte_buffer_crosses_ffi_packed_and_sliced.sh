#!/usr/bin/env bash
# A byte buffer that reaches the FFI boundary must be PACKED and passed
# as a SUBSLICE.
#
# Two independent defects, documented as rules 1 and 2 of
# docs/architecture/ffi-byte-buffer-contract.md, and they compose:
#
#   let mut buf = [0_u8; 12];        // NaN-boxed List: one 8-byte
#                                    // Value slot per element, so the
#                                    // bytes C reads are STRIDED
#   getsockopt(fd, lvl, opt, &mut buf, &mut len)
#                                    // `&mut arr` where the parameter
#                                    // is `&mut [Byte]` coerces to an
#                                    // EMPTY slice
#
# Measured at core/net/unix.vr:877 (SO_PEERCRED): either alone loses the
# data, together the callee never sees a byte, and nothing errors. The
# correct form is
#
#   let mut buf: [Byte; 12] = [0; 12];
#   getsockopt(fd, lvl, opt, &mut buf[..], &mut len)
#
# NOT EVERY UNANNOTATED BUFFER IS A DEFECT. A buffer that never crosses
# into C is fine as a List. So this gate does not report the
# declaration — it reports a declaration whose NAME is later used in a
# way that only makes sense at the boundary: `&mut name` without `[..]`,
# or `.as_ptr()` / `.as_mut_ptr()` on the raw array.
#
# Usage:
#   scripts/ci/check_a_byte_buffer_crosses_ffi_packed_and_sliced.sh
#   scripts/ci/check_a_byte_buffer_crosses_ffi_packed_and_sliced.sh --selftest
#
# --selftest carries both poles. The normal output is an ABSENCE, and an
# absence passes for free when the finder is broken; and a gate that
# reports every unannotated buffer would be noise with a number attached,
# so the negative control is a buffer that stays inside Verum.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

SELFTEST=0
[ "${1:-}" = "--selftest" ] && SELFTEST=1

python3 - "$SELFTEST" <<'PY'
import os, re, sys

selftest = sys.argv[1] == "1"
ROOT = "core"

DECL = re.compile(r'\blet\s+mut\s+([a-z_][a-z0-9_]*)\s*=\s*\[\s*0_u8\s*;')
# How far after the declaration to look. A buffer is filled and handed
# over inside one function body; 60 lines covers every site measured and
# keeps a later, unrelated `&mut x` in another function out of it.
WINDOW = 60

def uses_at_boundary(lines, start, name):
    """Return the 1-based line numbers where `name` is used in a way
    that only makes sense across the FFI boundary."""
    bad = []
    bare = re.compile(r'&\s*mut\s+' + re.escape(name) + r'\s*(?![\[\.\w])')
    ptr = re.compile(r'\b' + re.escape(name) + r'\s*\.\s*as_(mut_)?ptr\s*\(')
    for j in range(start, min(start + WINDOW, len(lines))):
        code = lines[j].split("//", 1)[0]
        if bare.search(code) or ptr.search(code):
            bad.append(j + 1)
    return bad

def check_text(path, text):
    out = []
    lines = text.split("\n")
    for i, line in enumerate(lines):
        code = line.split("//", 1)[0]
        m = DECL.search(code)
        if not m:
            continue
        name = m.group(1)
        for used in uses_at_boundary(lines, i + 1, name):
            out.append((i + 1, used, name))
    return out

if selftest:
    bad_src = (
        "fn f() {\n"
        "    let mut buf = [0_u8; 12];\n"
        "    getsockopt(fd, a, b, &mut buf, &mut len);\n"
        "}\n"
    )
    bad_ptr = (
        "fn g() {\n"
        "    let mut tv = [0_u8; 16];\n"
        "    setsockopt(fd, a, b, tv.as_ptr() as &unsafe Byte, 16);\n"
        "}\n"
    )
    good_slice = (
        "fn h() {\n"
        "    let mut buf: [Byte; 12] = [0; 12];\n"
        "    getsockopt(fd, a, b, &mut buf[..], &mut len);\n"
        "}\n"
    )
    good_local = (
        "fn k() {\n"
        "    let mut scratch = [0_u8; 32];\n"
        "    scratch[0] = 1;\n"
        "    return scratch.len();\n"
        "}\n"
    )
    r1 = check_text("t.vr", bad_src)
    r2 = check_text("t.vr", bad_ptr)
    r3 = check_text("t.vr", good_slice)
    r4 = check_text("t.vr", good_local)
    print("SELFTEST")
    print(f"  `&mut buf` on an unannotated array reported:  {'yes' if r1 else 'NO — blind'}")
    print(f"  `.as_ptr()` on an unannotated array reported: {'yes' if r2 else 'NO — blind'}")
    print(f"  packed + subslice stayed silent:             {'yes' if not r3 else 'NO — false positive'}")
    print(f"  a buffer that never leaves Verum stayed silent: {'yes' if not r4 else 'NO — noise'}")
    sys.exit(0 if (r1 and r2 and not r3 and not r4) else 1)

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
        for decl, used, name in check_text(p, text):
            findings.append((p, decl, used, name))

print(f"scanned {files} .vr files under {ROOT}/")
if not findings:
    print("check-ffi-byte-buffers: OK")
    sys.exit(0)

print(f"check-ffi-byte-buffers: {len(findings)} buffer use(s) cross the boundary unpacked\n")
for p, decl, used, name in findings:
    print(f"  {p}:{decl} declares `{name}` unannotated, used at :{used}")
print()
print("`[0_u8; N]` is a NaN-boxed List (8-byte-strided Value slots), and")
print("`&mut arr` where the parameter is `&mut [Byte]` coerces to an")
print("EMPTY slice. Neither errors. See rules 1 and 2 of")
print("docs/architecture/ffi-byte-buffer-contract.md.")
sys.exit(1)
PY
