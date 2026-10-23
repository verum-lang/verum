#!/usr/bin/env python3
"""Audit the pinned real CLI capture and reproduce its Verum regression fixture.

This is fixture preparation, not a second production tar implementation. Python's
independent gzip/tar readers and a fixed-capture header inventory check provenance;
only execution of the generated .vr file tests the shared Verum strict reader.
"""
from pathlib import Path
import argparse
import hashlib
import io
import json
import tarfile
import zlib

ROOT = Path(__file__).resolve().parent
CAPTURE = ROOT / "cli-source-cog"
OUTPUT = ROOT.parent / "cli_source_cog_test.vr"
TAR_SHA256 = "725882b126da1685c512a7b98ef71ae1c0b85f17b740b0fd2ce0830eb505ba09"
GZIP_SHA256 = "1641cd4640ea51391fe321386742faf05f6214164044341c25b1b86210917952"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def audit():
    raw = (CAPTURE / "archive.tar").read_bytes()
    compressed = (CAPTURE / "archive.tar.gz").read_bytes()
    expected = json.loads((CAPTURE / "expected.json").read_text())
    assert digest(raw) == TAR_SHA256 == expected["tar"]["sha256"]
    assert digest(compressed) == GZIP_SHA256 == expected["compressed"]["sha256"]
    assert len(raw) == expected["tar"]["size"] == 5120
    assert len(compressed) == expected["compressed"]["size"] == 313
    decoder = zlib.decompressobj(31)
    decoded = decoder.decompress(compressed) + decoder.flush()
    assert decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail
    assert decoded == raw
    entries = {entry["path"]: entry for entry in expected["entries"]}
    assert len(entries) == len(expected["entries"]) == 3
    actual = {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
        for entry in archive:
            assert entry.isfile() and entry.name not in actual
            content = archive.extractfile(entry).read()
            assert content == (CAPTURE / "input" / entry.name).read_bytes()
            assert len(content) == entries[entry.name]["size"]
            assert digest(content) == entries[entry.name]["sha256"]
            actual[entry.name] = content
    assert set(actual) == set(entries)
    actual_inputs = {
        path.relative_to(CAPTURE / "input").as_posix()
        for path in (CAPTURE / "input").rglob("*") if path.is_file()
    }
    assert actual_inputs == set(entries)
    # Inspect the exact fixed GNU capture, not a general alternate parser.
    headers = []
    pos = 0
    while pos < len(raw) and any(raw[pos:pos + 512]):
        header = raw[pos:pos + 512]
        assert len(header) == 512 and header[257:265] == b"ustar  \0"
        size = int(header[124:136].rstrip(b"\0 ").lstrip(b" "), 8)
        checksum = int(header[148:156].rstrip(b"\0 ").lstrip(b" "), 8)
        assert checksum == sum(header[:148]) + 8 * 32 + sum(header[156:])
        end = pos + 512 + size
        next_header = end + (-size % 512)
        assert next_header <= len(raw) and not any(raw[end:next_header])
        headers.append({"offset": pos, "kind": chr(header[156]), "size": size})
        pos = next_header
    assert headers == [
        {"offset": 0, "kind": "0", "size": 87},
        {"offset": 1024, "kind": "0", "size": 8},
        {"offset": 2048, "kind": "L", "size": 145},
        {"offset": 3072, "kind": "0", "size": 7},
    ]
    assert len(raw) - pos == 1024 and not any(raw[pos:])
    assert raw[2560:2705].endswith(b"\0") and b"\0" not in raw[2560:2704]
    return raw, actual, headers


def chunks(raw):
    # Omit zero-filled spans from source without changing a single capture byte.
    # Merge nearby nonzero runs to keep the resulting fixture readable.
    positions = [i for i, value in enumerate(raw) if value]
    spans = []
    start = positions[0]
    previous = start
    for pos in positions[1:]:
        if pos - previous > 16:
            spans.append((start, raw[start:previous + 1]))
            start = pos
        previous = pos
    spans.append((start, raw[start:previous + 1]))
    rebuilt = bytearray(len(raw))
    for start, data in spans:
        rebuilt[start:start + len(data)] = data
    assert rebuilt == raw and digest(rebuilt) == TAR_SHA256
    return spans


def byte_literal(data, indent="        "):
    return "\n".join(
        indent + ", ".join(f"0x{byte:02X}_u8" for byte in data[i:i + 12]) + ","
        for i in range(0, len(data), 12)
    )


def render(raw, entries):
    lines = [
        "// Generated from the pinned actual CLI source-cog capture; do not edit bytes.",
        "// Reproduce/check with fixtures/prepare_cli_source_cog.py.",
        f"// Raw tar SHA256: {TAR_SHA256}",
        "// This control has no gzip, filesystem extraction or registry admission claim.",
        "mount core.archive.tar.{EntryType, TarEntry, TarReadLimits, read_archive_strict};",
        "mount core.collections.list.List;", "",
        "fn copy_segment(out: &mut List<Byte>, offset: Int, segment: &List<Byte>) {",
        "    let mut i = 0;",
        "    while i < segment.len() { out.set(offset + i, segment[i]); i = i + 1; }",
        "}", "", "fn captured_tar() -> List<Byte> {",
        "    let mut out: List<Byte> = List.new();", "    let mut i = 0;",
        f"    while i < {len(raw)} {{ out.push(0_u8); i = i + 1; }}",
    ]
    for index, (offset, data) in enumerate(chunks(raw)):
        lines.extend([
            f"    let segment_{index}: List<Byte> = [", byte_literal(data), "    ];",
            f"    copy_segment(&mut out, {offset}, &segment_{index});",
        ])
    lines.extend([
        "    out", "}", "",
        "fn assert_entry(entries: &List<TarEntry>, path: &Text, expected: &List<Byte>) {",
        "    let mut matched = 0;", "    let mut i = 0;",
        "    while i < entries.len() {", "        let entry = &entries[i];",
        "        if *entry.path() == *path {", "            matched = matched + 1;",
        "            assert(entry.entry_type() is EntryType.RegularFile);",
        "            let content: &List<Byte> = entry.content();",
        "            assert(content.len() == expected.len());", "            let mut j = 0;",
        "            while j < expected.len() { assert(content[j] == expected[j]); j = j + 1; }",
        "        }", "        i = i + 1;", "    }", "    assert(matched == 1);", "}", "",
        "@test", "fn actual_cli_archive_preserves_exact_paths_and_binary_contents() {",
        "    let bytes = captured_tar();", "    assert(bytes.len() == 5120);",
        "    let limits = TarReadLimits {", "        max_input_bytes: 5120, max_entries: 3,",
        "        max_entry_bytes: 87, max_total_content_bytes: 102,",
        "        max_extensions: 1, max_extension_bytes: 145,",
        "        max_text_bytes: 144, max_trailing_padding_bytes: 0,", "    };",
        "    let entries = read_archive_strict(bytes.as_slice(), &limits).unwrap();",
        "    assert(entries.len() == 3);",
    ])
    for index, (name, data) in enumerate(sorted(entries.items())):
        # The actual fixture is ASCII. JSON escaping therefore matches Verum's
        # ordinary quoted-string subset without introducing Unicode syntax.
        assert name.isascii()
        lines.extend([
            f"    let expected_{index}: List<Byte> = [", byte_literal(data), "    ];",
            f"    assert_entry(&entries, &{json.dumps(name)}.to_text(), &expected_{index});",
        ])
    lines.extend([
        "}", "", "fn main() {",
        "    actual_cli_archive_preserves_exact_paths_and_binary_contents();",
        '    print("tar-cli-producer: 1 passed");', "}", "",
    ])
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="regenerate the Verum fixture")
    args = parser.parse_args()
    raw, entries, headers = audit()
    generated = render(raw, entries)
    if args.write:
        OUTPUT.write_text(generated)
    else:
        assert OUTPUT.read_text() == generated, "fixture drift; regenerate explicitly"
    print(json.dumps({
        "capture_tar_sha256": digest(raw), "fixture_sha256": digest(generated.encode()),
        "headers": headers, "exact_entries": len(entries),
        "scope": "Python capture and reproduction checks; this script does not execute Verum",
    }, indent=2))


if __name__ == "__main__":
    main()
