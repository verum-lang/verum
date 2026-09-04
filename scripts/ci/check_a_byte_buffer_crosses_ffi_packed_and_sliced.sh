#!/usr/bin/env bash
# Taking the RAW POINTER of an unannotated byte array and handing it to
# C reads the object header, not the data.
#
# THE NARROW FORM IS THE WHOLE GATE, and the first edition of this file
# was much wider — it also reported `&mut name` without `[..]`, on the
# strength of rule 2's footgun in
# docs/architecture/ffi-byte-buffer-contract.md. That produced 33
# findings, and measurement said 30+ of them were healthy code:
#
#   float_to_string(self, &mut buf, n)   -> f"{x:.2}" prints 3.14
#   src.read(&mut buf)  (chunked copy)   -> 14 bytes copied, identical
#   sys_read(fd, &mut tmp)  (stdin)      -> the real line, right length
#
# Two mechanisms make that form safe, and both are recorded in the
# contract itself:
#
#   * commit 02e838a10 (task #24) made a bare `&arr` / `&mut arr` on a
#     byte array unsize to the SAME packed `RefSlice` as `&arr[..]`.
#     The empty-slice coercion this gate was built to catch NO LONGER
#     EXISTS.
#   * the interpreter's FFI marshaller has a `TypeId::LIST` writeback
#     (interpreter/dispatch_table/handlers/ffi_extended.rs:1641): a
#     NaN-boxed array is packed into scratch for C and written back
#     element-by-element afterwards. So even a strided buffer that
#     reaches a syscall through a SLICE survives on Tier 0. (Tier 1 /
#     AOT is not measured — see the contract.)
#
# What is still a defect is rule 3: `.as_ptr()` / `.as_mut_ptr()`
# applied to the ARRAY VARIABLE returns the object base, and no
# writeback can repair a pointer that was wrong before the call. The
# correct form takes the subslice first:
#
#   let mut buf: [Byte; 12] = [0; 12];
#   c_fn(buf[..].as_ptr() as &unsafe Byte, 12)
#
# A gate that reports 30 healthy sites teaches people to ignore it, so
# this one reports only the form that is measurably still wrong.
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
    """Return the 1-based line numbers where `name` has its RAW POINTER
    taken directly — `name.as_ptr()`, not `name[..].as_ptr()`.

    `&mut name` is deliberately NOT matched: it unsizes to a packed
    slice since 02e838a10, and three independent measurements found it
    healthy. See the header."""
    bad = []
    ptr = re.compile(r'\b' + re.escape(name) + r'\s*\.\s*as_(mut_)?ptr\s*\(')
    for j in range(start, min(start + WINDOW, len(lines))):
        code = lines[j].split("//", 1)[0]
        if ptr.search(code):
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
    good_bare = (
        "fn m() {\n"
        "    let mut buf = [0_u8; 64];\n"
        "    let n = float_to_string(x, &mut buf, 2);\n"
        "}\n"
    )
    r1 = check_text("t.vr", bad_src)
    r2 = check_text("t.vr", bad_ptr)
    r3 = check_text("t.vr", good_slice)
    r4 = check_text("t.vr", good_local)
    r5 = check_text("t.vr", good_bare)
    print("SELFTEST")
    print(f"  `.as_ptr()` on an unannotated array reported: {'yes' if r2 else 'NO — blind'}")
    print(f"  packed + subslice stayed silent:              {'yes' if not r3 else 'NO — false positive'}")
    print(f"  a buffer that never leaves Verum stayed silent: {'yes' if not r4 else 'NO — noise'}")
    print(f"  bare `&mut buf` stayed silent (02e838a10):    {'yes' if not r5 else 'NO — stale rule'}")
    print(f"  ...and so did the same form at a syscall:     {'yes' if not r1 else 'NO — stale rule'}")
    print()
    print("  The positive pole is `.as_ptr()` alone. `&mut buf` is now a")
    print("  NEGATIVE control in two flavours — a Verum callee and a")
    print("  syscall — because both were MEASURED healthy, and a gate")
    print("  whose only pole is an absence passes for free.")
    sys.exit(0 if (r2 and not r1 and not r3 and not r4 and not r5) else 1)

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

print(f"check-ffi-byte-buffers: {len(findings)} raw pointer(s) taken of an unpacked array\n")
for p, decl, used, name in findings:
    print(f"  {p}:{decl} declares `{name}` unannotated, raw pointer taken at :{used}")
print()
print("`[0_u8; N]` is a NaN-boxed List (8-byte-strided Value slots), and")
print("`.as_ptr()` on the ARRAY VARIABLE returns the object base rather")
print("than the data — so C reads the header. Taking the subslice first")
print("fixes both at once:")
print()
print("    let mut buf: [Byte; N] = [0; N];")
print("    c_fn(buf[..].as_ptr() as &unsafe Byte, N)")
print()
print("This does NOT apply to `&mut buf`, which unsizes correctly since")
print("02e838a10. See rules 1 and 3 of")
print("docs/architecture/ffi-byte-buffer-contract.md.")
sys.exit(1)
PY
