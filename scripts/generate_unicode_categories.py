#!/usr/bin/env python3
"""Generate the Verum category-code table from Unicode 17.0 DerivedGeneralCategory.txt."""
import argparse
import hashlib
from pathlib import Path

SOURCE = "https://www.unicode.org/Public/17.0.0/ucd/extracted/DerivedGeneralCategory.txt"
SHA256 = "d62e5bab70ca74f099343f71224fa051cb1fdd61a1ab45c0488c44cfc0b6102e"
# Stable codes of the char_general_category scalar intrinsic. Public Verum
# variants are constructed by the normal declaration in core.text.char.
CATEGORIES = "Lu Ll Lt Lm Lo Mn Mc Me Nd Nl No Pc Pd Ps Pe Pi Pf Po Sm Sc Sk So Zs Zl Zp Cc Cf Cs Co Cn".split()
OUTPUT = Path(__file__).resolve().parents[1] / "crates/verum_vbc/src/unicode_category/ucd17.bin"


def generate(data):
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError(f"expected Unicode 17.0 General_Category data from {SOURCE}")
    codes = bytearray([255]) * 0x110000
    for line in data.decode("utf-8").splitlines():
        row = line.partition("#")[0].strip()
        if not row:
            continue
        extent, name = (field.strip() for field in row.split(";"))
        edges = [int(value, 16) for value in extent.split("..")]
        first, last = edges[0], edges[-1]
        if not (0 <= first <= last < len(codes)) or any(v != 255 for v in codes[first:last + 1]):
            raise ValueError(f"overlapping or invalid UCD interval {extent}")
        codes[first:last + 1] = bytes([CATEGORIES.index(name)]) * (last - first + 1)
    if 255 in codes:
        raise ValueError("UCD intervals do not cover every code point")

    # Deduplicate 256-code-point pages. The index and every stored value use
    # one byte, independent of the host and target byte order.
    pages = {}
    index = bytearray()
    for start in range(0, len(codes), 256):
        page = bytes(codes[start:start + 256])
        index.append(pages.setdefault(page, len(pages)))
    table = index + b"".join(pages)
    # Decode through the same two-index layout for every code point.
    if any(table[len(index) + 256 * table[cp >> 8] + (cp & 255)] != code
           for cp, code in enumerate(codes)):
        raise ValueError("packed table differs from UCD")
    return table


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ucd", type=Path, help=SOURCE)
    parser.add_argument("--check", action="store_true", help="compare the checked-in table without writing")
    args = parser.parse_args()
    table = generate(args.ucd.read_bytes())
    if args.check:
        if table != OUTPUT.read_bytes():
            parser.error("the checked-in table differs from UCD")
    else:
        OUTPUT.write_bytes(table)
    print(f"Unicode 17.0: all {0x110000} code points verified, {len(table)} table bytes")
