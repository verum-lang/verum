#!/usr/bin/env python3
"""
check_op_byte_docs.py — gate: .vr doc comments must not lie about VBC op bytes.

WHY THIS EXISTS
---------------
`crates/verum_vbc/src/instruction.rs` is the single source of truth for every
VBC opcode and sub-opcode byte.  `.vr` doc comments routinely restate those
bytes ("VBC Opcode: SyscallLinux (0xE0)", "op: 3=exp, 4=log ...").  Nothing
kept the two in sync, so the bytes rotted: the T0193 tensor campaign found a
comment claiming `3 = log` where the enum says 4, and the 0x45-0x4F opcode
block had long since moved to 0xE0-0xEA while seven `.vr` files still quoted
the old bytes (T0198).

A doc comment that lies about an opcode byte is worse than no comment at all:
the next author writes code against it.  This gate re-derives the
discriminants from the enum on every run and fails if a `.vr` comment
contradicts them.  There is no checked-in copy of the truth to drift.

SCOPE — WHAT IS CHECKED (deliberately narrow)
---------------------------------------------
Only claims written in one of three unambiguous conventions are checked.  A
narrow gate that always tells the truth beats a broad one that guesses: `.vr`
comments are full of prose arithmetic ("Sum = 55", "Max = 12") and of foreign
opcode taxonomies (SQLite VDBE, HTTP/3 QPACK) that must never be matched
against VBC enums.

  C1  `VBC Opcode:` / `VBC Opcodes:` anchor, then one or more
      `Name (0xNN)` or `Name 0xNN` pairs on the same line.
      Checked against the top-level `Opcode` enum.
        // VBC Opcodes: Spawn (0xA0), Await (0xA1), IoSubmit (0xE7)
        /// Atomic load of UInt8 (VBC opcode: AtomicLoad 0xE3 with size=1).

  C2  `EnumName.Variant (0xNN)` or `EnumName.Variant = 0xNN`, where
      EnumName ends in `SubOpcode` or `Op`.  Checked against that enum.
        /// routes through SyncSubOpcode.FutexWait (0x00)

  C3  a listing of `N=name` pairs anchored by the phrase
      `mirrors <EnumName> byte values` anywhere in the same contiguous
      comment block.  Variant names match case-insensitively.  This is the
      convention that carries the tensor unop/binop/reduce tables — the
      exact shape that drifted in T0193.
        /// Element-wise unary operation (op: 0=neg, 1=abs, 2=sqrt, 3=exp,
        /// 4=log ... — mirrors TensorUnaryOp byte values)

WHAT IS NOT CHECKED (and why)
-----------------------------
  * bare bytes with no variant name ("VBC opcode 0xE1") — nothing to bind
    the byte to, so nothing can be verified.  Prefer C1: name the opcode.
  * symbolic tables using macro spellings (`GPU_LAUNCH`, `SIMD_SPLAT`) —
    SCREAMING_SNAKE -> CamelCase is not a total function, so matching would
    have to guess.  (The `core/math/gpu.vr` table was verified by hand under
    T0198; if it is ever re-spelled in C2 form the gate picks it up.)
  * anything outside the three conventions — including every non-VBC opcode
    taxonomy in `core/database/**` and `core/net/**`.

To bring a claim under the gate, rewrite it in C1/C2/C3 form.

MODES
  --check   (default) CI gate: exit 1 on any contradiction, else 0
  --report  list every claim the gate parsed, OK and DRIFT alike, then exit 0

Pure Python, no build required.  Runs over core/, core-tests/, vcs/.
"""
import os
import re
import sys

# Repo root = two levels up from vcs/scripts/.
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TRUTH_SRC = os.path.join(ROOT, "crates", "verum_vbc", "src", "instruction.rs")
ROOTS = ["core", "core-tests", "vcs"]
SKIP_DIRS = {"target", ".claude", ".git", "node_modules"}

# A comment line, capturing the text after the marker.  Verum uses `//`,
# `///` and `//!` exactly as Rust does.
COMMENT_RE = re.compile(r"^\s*(?://!|///?)\s?(.*)$")

# ---- C1: "VBC Opcode[s]: Name (0xNN)" / "... Name 0xNN" --------------------
C1_ANCHOR_RE = re.compile(r"\bVBC\s+[Oo]pcodes?:\s*(.+)$")
C1_PAIR_RE = re.compile(r"\b([A-Z]\w*)\s*(?:\(\s*(0x[0-9A-Fa-f]+)\s*\)|(0x[0-9A-Fa-f]+))")

# ---- C2: "EnumName.Variant (0xNN)" / "EnumName.Variant = 0xNN" -------------
C2_RE = re.compile(
    r"\b(\w+(?:SubOpcode|Op))\.(\w+)\s*(?:\(\s*(0x[0-9A-Fa-f]+)\s*\)|=\s*(0x[0-9A-Fa-f]+))"
)

# ---- C3: "N=name, ..." anchored by "mirrors <Enum> byte values" ------------
C3_ANCHOR_RE = re.compile(r"\bmirrors\s+(\w+)\s+byte\s+values\b")
C3_PAIR_RE = re.compile(r"\b(\d+)\s*=\s*([A-Za-z]\w*)")


def parse_truth(path):
    """Re-derive {enum: {Variant: value}} from the instruction.rs enums."""
    try:
        lines = open(path, encoding="utf-8").read().split("\n")
    except OSError as exc:
        sys.stderr.write(f"check_op_byte_docs: cannot read truth source: {exc}\n")
        sys.exit(2)

    enums, i = {}, 0
    enum_re = re.compile(r"^pub enum (\w+)\s*\{")
    variant_re = re.compile(r"^\s*(\w+)\s*=\s*(0x[0-9A-Fa-f]+|\d+)\s*,")
    while i < len(lines):
        m = enum_re.match(lines[i])
        if not m:
            i += 1
            continue
        name, variants, depth, j = m.group(1), {}, 1, i + 1
        while j < len(lines) and depth > 0:
            line = lines[j]
            depth += line.count("{") - line.count("}")
            if depth <= 0:
                break
            vm = variant_re.match(line)
            if vm:
                variants[vm.group(1)] = int(vm.group(2), 0)
            j += 1
        if variants:
            enums[name] = variants
        i = j + 1
    return enums


def comment_blocks(path):
    """Yield (start_line, [(lineno, text), ...]) for contiguous comment runs."""
    try:
        lines = open(path, encoding="utf-8", errors="replace").read().split("\n")
    except OSError:
        return
    block = []
    for n, raw in enumerate(lines, 1):
        m = COMMENT_RE.match(raw)
        if m:
            block.append((n, m.group(1)))
        elif block:
            yield block
            block = []
    if block:
        yield block


def iter_vr_files():
    for root in ROOTS:
        base = os.path.join(ROOT, root)
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
            for fn in sorted(filenames):
                if fn.endswith(".vr"):
                    yield os.path.join(dirpath, fn)


def collect_claims(path, truth):
    """Return [(lineno, convention, enum, name, claimed, real_or_None, text)]."""
    out = []
    for block in comment_blocks(path):
        # C3 anchor applies to the whole contiguous block.
        c3_enum = None
        for _, text in block:
            m = C3_ANCHOR_RE.search(text)
            if m:
                c3_enum = m.group(1)
                break

        for lineno, text in block:
            # --- C1 ---
            anchor = C1_ANCHOR_RE.search(text)
            if anchor:
                for pm in C1_PAIR_RE.finditer(anchor.group(1)):
                    name = pm.group(1)
                    claimed = int(pm.group(2) or pm.group(3), 0)
                    real = truth.get("Opcode", {}).get(name)
                    out.append((lineno, "C1", "Opcode", name, claimed, real, text))

            # --- C2 ---
            for cm in C2_RE.finditer(text):
                enum, name = cm.group(1), cm.group(2)
                claimed = int(cm.group(3) or cm.group(4), 0)
                if enum not in truth:
                    continue  # not a VBC enum — out of scope
                out.append((lineno, "C2", enum, name, claimed, truth[enum].get(name), text))

            # --- C3 ---
            if c3_enum and c3_enum in truth:
                lowered = {k.lower(): v for k, v in truth[c3_enum].items()}
                for pm in C3_PAIR_RE.finditer(text):
                    claimed, name = int(pm.group(1)), pm.group(2)
                    if name.lower() not in lowered:
                        continue  # prose word, not a variant of this enum
                    out.append(
                        (lineno, "C3", c3_enum, name, claimed, lowered[name.lower()], text)
                    )
    return out


def main():
    argv = sys.argv[1:]
    report = "--report" in argv
    truth = parse_truth(TRUTH_SRC)
    if "Opcode" not in truth:
        sys.stderr.write(
            "check_op_byte_docs: no `Opcode` enum found in "
            f"{os.path.relpath(TRUTH_SRC, ROOT)} — truth parser is out of date.\n"
        )
        return 2

    violations, parsed = [], 0
    for path in iter_vr_files():
        for lineno, conv, enum, name, claimed, real, text in collect_claims(path, truth):
            parsed += 1
            rel = os.path.relpath(path, ROOT)
            if real is None:
                violations.append(
                    (rel, lineno, conv, f"{enum}.{name} is not a variant of {enum}",
                     f"0x{claimed:02X}", "no such variant", text)
                )
            elif real != claimed:
                violations.append(
                    (rel, lineno, conv, f"{enum}.{name}",
                     f"0x{claimed:02X} ({claimed})", f"0x{real:02X} ({real})", text)
                )
            elif report:
                print(f"OK    {rel}:{lineno} [{conv}] {enum}.{name} = 0x{real:02X}")

    if report:
        print(f"\nparsed {parsed} op-byte claims; {len(violations)} contradict the enum")
        return 0

    if violations:
        sys.stderr.write(
            f"check_op_byte_docs: {len(violations)} .vr doc comment(s) contradict "
            f"{os.path.relpath(TRUTH_SRC, ROOT)}\n\n"
        )
        for rel, lineno, conv, what, claimed, real, text in violations:
            sys.stderr.write(
                f"  {rel}:{lineno} [{conv}] {what}\n"
                f"      claimed: {claimed}\n"
                f"      real   : {real}\n"
                f"      line   : {text.strip()[:100]}\n\n"
            )
        sys.stderr.write(
            "The enum is the single source of truth — fix the comment, not the enum.\n"
            "Run with --report to list every claim the gate parses.\n"
        )
        return 1

    print(f"check_op_byte_docs: OK — {parsed} op-byte claims match the enum")
    return 0


if __name__ == "__main__":
    sys.exit(main())
