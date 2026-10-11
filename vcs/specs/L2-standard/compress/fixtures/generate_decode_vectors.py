#!/usr/bin/env python3
"""Independent zlib/gzip input authority; never executes the Verum decoder."""
from __future__ import annotations

import gzip
from pathlib import Path
import struct
import zlib


class Bits:
    def __init__(self):
        self.values = []

    def put(self, value, width):
        self.values.extend((value >> bit) & 1 for bit in range(width))

    def code(self, value, width):
        self.values.extend((value >> bit) & 1 for bit in reversed(range(width)))

    def finish(self):
        values = self.values + [0] * ((-len(self.values)) % 8)
        return bytes(sum(values[i + bit] << bit for bit in range(8))
                     for i in range(0, len(values), 8))


def fixed_symbol(bits, symbol):
    if symbol <= 143:
        bits.code(48 + symbol, 8)
    elif symbol <= 255:
        bits.code(400 + symbol - 144, 9)
    elif symbol <= 279:
        bits.code(symbol - 256, 7)
    else:
        bits.code(192 + symbol - 280, 8)


def fixed_stream(symbols, distance=None, final=True):
    bits = Bits()
    bits.put(int(final), 1)
    bits.put(1, 2)
    for symbol in symbols:
        fixed_symbol(bits, symbol)
    if distance is not None:
        bits.code(distance, 5)
    fixed_symbol(bits, 256)
    return bits.finish()


ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]


def dynamic_header(code_lengths, hlit=257, hdist=1):
    last = max(3, max(i for i, symbol in enumerate(ORDER) if code_lengths[symbol]))
    bits = Bits()
    bits.put(1, 1)
    bits.put(2, 2)
    bits.put(hlit - 257, 5)
    bits.put(hdist - 1, 5)
    bits.put(last - 3, 4)
    for symbol in ORDER[:last + 1]:
        bits.put(code_lengths[symbol], 3)
    return bits


def tiny_dynamic(literal_lengths, distance_lengths):
    # Complete code-length alphabet: 0 -> 0, 1 -> 10, 2 -> 11.
    code_lengths = [0] * 19
    code_lengths[0], code_lengths[1], code_lengths[2] = 1, 2, 2
    bits = dynamic_header(code_lengths, len(literal_lengths), len(distance_lengths))
    for length in literal_lengths + distance_lengths:
        if length == 0:
            bits.code(0, 1)
        else:
            bits.code(length + 1, 2)
    bits.code(0, 1)  # EOB in the valid one-symbol literal tree.
    return bits.finish()


def raw(data, level=9, strategy=zlib.Z_DEFAULT_STRATEGY):
    encoder = zlib.compressobj(level, zlib.DEFLATED, -15, 8, strategy)
    return encoder.compress(data) + encoder.flush()


def member(payload, compressed=None, flags=0, optional=b""):
    header = b"\x1f\x8b\x08" + bytes([flags]) + b"\0\0\0\0\0\xff" + optional
    if flags & 2:
        header += struct.pack("<H", zlib.crc32(header) & 0xffff)
    return header + (raw(payload) if compressed is None else compressed) + struct.pack(
        "<II", zlib.crc32(payload), len(payload) & 0xffffffff)


def vectors():
    output = []

    def add(name, api, data, expected=None, error=None, limit=4096, oracle=True):
        if oracle:
            try:
                decoded = gzip.decompress(data) if api.startswith("gzip") else zlib.decompress(data, -15)
            except (OSError, EOFError, zlib.error):
                assert error == "corrupt", name
            else:
                assert expected is not None and decoded == expected, name
        output.append(dict(name=name, api=api, input=list(data),
                           expected=None if expected is None else list(expected),
                           error=error, limit=limit, python_oracle=oracle))

    add("raw_stored_binary", "raw", raw(bytes([0, 255, 128, 1]), 0), bytes([0, 255, 128, 1]), limit=4)
    add("raw_fixed", "raw", raw(b"fixed codes", strategy=zlib.Z_FIXED), b"fixed codes")
    dynamic_data = b"the quick brown fox jumps over the lazy dog " * 64
    dynamic = raw(dynamic_data)
    assert (dynamic[0] >> 1) & 3 == 2
    add("raw_dynamic", "raw", dynamic, dynamic_data, limit=len(dynamic_data))
    add("raw_empty", "raw", raw(b""), b"", limit=1)
    add("raw_overlap", "raw", fixed_stream([65, 285], 0), b"A" * 259, limit=259)
    # A stored non-final block supplies history for a following fixed block.
    cross = bytes([0, 1, 0, 254, 255, 65]) + fixed_stream([285], 0)
    add("raw_cross_block", "raw", cross, b"A" * 259, limit=259)
    # A small compressed input reaches the 32 KiB history boundary, then
    # copies from exactly distance 32768. The expected bytes remain periodic.
    history = Bits()
    history.put(1, 1)
    history.put(1, 2)
    for byte in range(256):
        fixed_symbol(history, byte)
    for _ in range(126):
        fixed_symbol(history, 285)  # length 258
        history.code(15, 5)
        history.put(63, 6)  # distance 193 + 63 = 256
    fixed_symbol(history, 258)  # length 4, completing 32768 bytes
    history.code(15, 5)
    history.put(63, 6)
    fixed_symbol(history, 257)  # length 3
    history.code(29, 5)
    history.put(8191, 13)  # distance 24577 + 8191 = 32768
    fixed_symbol(history, 256)
    maximum_history = bytes(range(256)) * 128 + bytes([0, 1, 2])
    add("raw_maximum_history", "raw", history.finish(), maximum_history, limit=len(maximum_history))
    degenerate = [0] * 257
    degenerate[256] = 1
    add("raw_degenerate_empty_distance", "raw", tiny_dynamic(degenerate, [0]), b"", limit=1)
    add("raw_degenerate_single_distance", "raw", tiny_dynamic(degenerate, [1]), b"", limit=1)
    add("raw_bad_distance", "raw", fixed_stream([257], 0), error="corrupt")
    add("raw_reserved_literal", "raw", fixed_stream([286]), error="corrupt")
    add("raw_reserved_distance", "raw", fixed_stream([65, 257], 30), error="corrupt")
    add("raw_reserved_block", "raw", bytes([7]), error="corrupt")
    add("raw_bad_stored_length", "raw", bytes([1, 1, 0, 255, 255, 65]), error="corrupt")
    add("raw_truncated_stored", "raw", raw(b"abc", 0)[:-1], error="corrupt")
    add("raw_missing_final_block", "raw", bytes([0, 0, 0, 255, 255]), error="corrupt")
    over = [0] * 19
    over[16] = over[17] = over[18] = 1
    add("raw_oversubscribed_tree", "raw", dynamic_header(over).finish(), error="corrupt")
    incomplete = [0] * 19
    incomplete[0] = 1
    add("raw_incomplete_code_tree", "raw", dynamic_header(incomplete).finish(), error="corrupt")
    missing_eob = [0] * 257
    missing_eob[65] = 1
    add("raw_missing_eob", "raw", tiny_dynamic(missing_eob, [0]), error="corrupt")
    incomplete_literal = [0] * 257
    incomplete_literal[65], incomplete_literal[256] = 1, 2
    add("raw_incomplete_literal_tree", "raw", tiny_dynamic(incomplete_literal, [0]), error="corrupt")
    add("raw_incomplete_distance_tree", "raw", tiny_dynamic(degenerate, [2]), error="corrupt")
    add("raw_oversubscribed_distance_tree", "raw", tiny_dynamic(degenerate, [1, 1, 1]), error="corrupt")
    repeats = [0] * 19
    repeats[0] = repeats[16] = 1
    repeat_first = dynamic_header(repeats)
    repeat_first.code(1, 1)
    repeat_first.put(0, 2)
    add("raw_repeat_without_previous", "raw", repeat_first.finish(), error="corrupt")
    repeats[16], repeats[18] = 0, 1
    overflow = dynamic_header(repeats)
    for _ in range(2):
        overflow.code(1, 1)
        overflow.put(127, 7)
    add("raw_repeat_overflow", "raw", overflow.finish(), error="corrupt")
    bad_alphabet = [0] * 19
    bad_alphabet[0] = bad_alphabet[1] = 1
    add("raw_reserved_alphabet_size", "raw", dynamic_header(bad_alphabet, hlit=288).finish(), error="corrupt")
    wrong_eob_bit = bytearray(tiny_dynamic(degenerate, [0]))
    # Locate the final one-bit EOB after the fixed header and length encoding.
    eob_bit = 3 + 5 + 5 + 4 + 18 * 3 + 257 + 1 + 1
    wrong_eob_bit[eob_bit // 8] |= 1 << (eob_bit % 8)
    add("raw_invalid_degenerate_symbol", "raw", bytes(wrong_eob_bit), error="corrupt")
    # Python's raw decompressor intentionally tolerates trailing input; our
    # whole-stream API must refuse it. This is not a claimed Python rejection.
    add("raw_trailing", "raw", raw(b"") + b"\xff", error="corrupt", oracle=False)
    add("raw_output_limit", "raw", raw(b"abc"), b"abc", error="limit", limit=2)
    add("raw_zero_limit", "raw", raw(b""), b"", error="limit", limit=0)
    add("raw_negative_limit", "raw", raw(b""), b"", error="limit", limit=-1)
    add("raw_large_configured_limit", "raw", raw(b"a"), b"a", limit=9223372036854775807)

    data = b"gzip\x00\xff payload"
    normal = member(data)
    add("gzip_binary", "gzip", normal, data, limit=len(data))
    add("gzip_stored", "gzip", member(data, compressed=raw(data, 0)), data)
    add("gzip_dynamic", "gzip", member(dynamic_data, compressed=dynamic), dynamic_data, limit=len(dynamic_data))
    add("gzip_empty", "gzip", member(b""), b"", limit=1)
    optional = struct.pack("<H", 3) + b"\0\xff\x80" + b"source.vr\0" + b"comment\0"
    option_member = member(data, flags=2 | 4 | 8 | 16, optional=optional)
    add("gzip_optional_fhcrc", "gzip", option_member, data)
    add("gzip_text_hint", "gzip", member(data, flags=1), data)
    add("gzip_empty_extra", "gzip", member(data, flags=4, optional=b"\0\0"), data)
    add("gzip_concatenated", "gzip", member(b"ab") + member(b"cd"), b"abcd", limit=4)
    add("gzip_cumulative_limit", "gzip", member(b"ab") + member(b"cd"), b"abcd", error="limit", limit=3)
    corrupt_crc = normal[:-8] + bytes([normal[-8] ^ 1]) + normal[-7:]
    add("gzip_bad_crc", "gzip", corrupt_crc, error="corrupt")
    add("gzip_bad_size", "gzip", normal[:-4] + struct.pack("<I", len(data) + 1), error="corrupt")
    add("gzip_truncated_trailer", "gzip", normal[:-1], error="corrupt")
    add("gzip_late_corruption", "gzip", member(b"first") + corrupt_crc, error="corrupt")
    bad_fhcrc = bytearray(option_member)
    bad_fhcrc[10 + len(optional)] ^= 1
    # Python gzip ignores FHCRC; zlib's gzip mode validates it.
    try:
        zlib.decompress(bytes(bad_fhcrc), 31)
    except zlib.error:
        pass
    else:
        raise AssertionError("bad FHCRC accepted by zlib")
    add("gzip_bad_fhcrc", "gzip", bytes(bad_fhcrc), error="corrupt", oracle=False)
    add("gzip_reserved_flags", "gzip", normal[:3] + b"\xe0" + normal[4:], error="corrupt", oracle=False)
    add("gzip_bad_magic", "gzip", b"\0" + normal[1:], error="corrupt")
    add("gzip_bad_method", "gzip", normal[:2] + b"\0" + normal[3:], error="corrupt")
    add("gzip_missing_member", "gzip", b"", error="corrupt", oracle=False)
    add("gzip_unterminated_name", "gzip", bytes([31, 139, 8, 8]) + b"\0" * 6 + b"name", error="corrupt")
    add("gzip_truncated_extra", "gzip", bytes([31, 139, 8, 4]) + b"\0" * 6 + b"\x05\0ab", error="corrupt")
    add("gzip_truncated_extra_length", "gzip", bytes([31, 139, 8, 4]) + b"\0" * 6 + b"\x05", error="corrupt")
    add("gzip_unterminated_comment", "gzip", bytes([31, 139, 8, 16]) + b"\0" * 6 + b"comment", error="corrupt")
    add("gzip_truncated_fhcrc", "gzip", bytes([31, 139, 8, 2]) + b"\0" * 6 + b"\0", error="corrupt")
    add("gzip_trailing", "gzip", normal + b"\xff", error="corrupt")
    add("gzip_zero_trailing", "gzip", normal + b"\0", error="corrupt", oracle=False)
    add("gzip_zero_limit", "gzip", member(b""), b"", error="limit", limit=0)
    add("gzip_negative_limit", "gzip", member(b""), b"", error="limit", limit=-1)
    # The second stream cannot refer to output from the first member.
    second = member(b"AAA", compressed=fixed_stream([257], 0))
    add("gzip_reset_history", "gzip", member(b"A") + second, error="corrupt")
    add("single_binary", "single", normal, data, oracle=False)
    add("single_empty", "single", member(b""), b"", limit=1, oracle=False)
    add("single_second_member", "single", normal + member(b""), error="corrupt", oracle=False)
    add("single_zero_trailing", "single", normal + b"\0", error="corrupt", oracle=False)
    add("single_zero_limit", "single", member(b""), b"", error="limit", limit=0, oracle=False)
    add("single_negative_limit", "single", member(b""), b"", error="limit", limit=-1, oracle=False)
    return output


def byte_array(data):
    return "[" + ", ".join(f"{byte}_u8" for byte in data) + "]"


def render(group, cases):
    lines = ["// @test: run", "// @tier: all", "// @level: L2", "// @timeout: 600000",
             f"// @expected-stdout: {group} decode: ok", "// @expected-exit: 0",
             "// Generated by fixtures/generate_decode_vectors.py; Python is input authority only.",
             "mount core.compress.{Algorithm, CompressError, decode};",
             "mount core.compress.mod_gzip.{Gzip, Deflate};", ""]
    for case in cases:
        method = {"raw": "Deflate.decode", "gzip": "Gzip.decode", "single": "Gzip.decode_single"}[case["api"]]
        invocation = f"{method}(&input, {case['limit']}, &mut output)"
        if group == "dispatch":
            algorithm = "Gzip" if case["api"] == "gzip" else "Deflate"
            invocation = f"decode(Algorithm.{algorithm}, &input, {case['limit']}, &mut output)"
        lines += [f"fn {case['name']}() {{",
                  f"    let input: [Byte; {len(case['input'])}] = {byte_array(case['input'])};",
                  "    let mut output: List<Byte> = List.new();",
                  "    output.push(90_u8);", "    output.push(165_u8);",
                  f"    let decoded = {invocation};"]
        if case["error"]:
            pattern = "CompressError.OutputTooLarge { limit }" if case["error"] == "limit" else "CompressError.CorruptInput(_)"
            lines += ["    match decoded {", f"        Result.Err({pattern}) => {{"]
            if case["error"] == "limit":
                lines.append(f"            assert(limit == {case['limit']});")
            lines += ["        },", f"        _ => panic(\"{case['name']}: expected {case['error']} refusal\"),", "    };",
                      "    assert(output.len() == 2);", "    assert(output[0] == 90_u8);", "    assert(output[1] == 165_u8);"]
        else:
            expected = case["expected"]
            pattern = expected
            for size in range(1, min(256, len(expected)) + 1):
                repeated = expected[:size] * ((len(expected) + size - 1) // size)
                if expected == repeated[:len(expected)]:
                    pattern = expected[:size]
                    break
            lines += ["    match decoded {", f"        Result.Ok(count) => assert(count == {len(expected)}),",
                      f"        Result.Err(_) => panic(\"{case['name']}: refused valid stream\"),", "    };",
                      f"    assert(output.len() == {len(expected) + 2});",
                      "    assert(output[0] == 90_u8);", "    assert(output[1] == 165_u8);"]
            if pattern:
                lines += [f"    let expected: [Byte; {len(pattern)}] = {byte_array(pattern)};",
                          "    let mut i = 0;", f"    while i < {len(expected)} {{",
                          "        assert(output[i + 2] == expected[i % expected.len()]);",
                          "        i = i + 1;", "    }"]
        lines += ["}", ""]
    lines.append("fn main() {")
    lines += [f"    {case['name']}();" for case in cases]
    lines += [f"    print(\"{group} decode: ok\");", "}", ""]
    return "\n".join(lines)


def main():
    here = Path(__file__).resolve().parent
    cases = vectors()
    for group in ("raw", "gzip", "single"):
        (here.parent / f"{group}_decode.vr").write_text(render(group, [c for c in cases if c["api"] == group]))
    dispatch_cases = [case for case in cases if case["name"] in
                      {"raw_stored_binary", "gzip_binary", "raw_zero_limit", "gzip_negative_limit"}]
    (here.parent / "dispatch_decode.vr").write_text(render("dispatch", dispatch_cases))
    print(f"Prepared {len(cases)} fixed vectors; no Verum execution performed.")


if __name__ == "__main__":
    main()
