# Warp network fuzz corpus — TLS 1.3 / QUIC / HTTP/3 / X.509

Verum-native fuzz runners against the pure-Verum parsers on every wire
surface warp exposes. Each runner is a standalone `.vr` program that:

1. Builds an initial seed vector from a canonical RFC test case.
2. Mutates it with the shared `FuzzRng` (xorshift64, deterministic).
3. Feeds the mutant bytes into the parser under test.
4. Asserts that every invocation terminates with `Ok(_)` or `Err(_)` —
   never a panic, SIGSEGV, or out-of-bounds access.

Every runner tags `@test: wip` so the main vtest collector leaves it
alone; runners are invoked manually or from the nightly fuzz script.

## Layout

| Path | Parser under test | Seeds (RFC ref) |
|------|-------------------|-----------------|
| `tls13/handshake/runner.vr` | `core.net.tls13.handshake.messages::decode_handshake` | RFC 8448 §A.1 ClientHello |
| `quic/packet/runner.vr` | `core.net.quic.packet::{parse_long, parse_short}` | RFC 9001 §A.2 Initial, short-header dummy |
| `quic/frame/runner.vr` | `core.net.quic.frame::decode_all_frames` | RFC 9000 §19 PADDING/PING/ACK/CRYPTO |
| `h3/frame/runner.vr` | `core.net.h3.frame::decode_frame` | RFC 9114 §7 DATA/HEADERS/SETTINGS/GOAWAY |
| `h3/qpack/runner.vr` | `core.net.h3.qpack.decoder::decode_field_section` | RFC 9204 §3.2 indexed-static `:method GET` |
| `x509/der/runner.vr` | `core.encoding.der::DerReader` | minimal SEQUENCE, SEQUENCE{OID,OCTET STRING} |

Each runner carries its own inline `FuzzRng` (xorshift64) so they are
self-contained and can be invoked individually.

## Running

```bash
verum run vcs/fuzz/net/tls13/handshake/runner.vr
verum run vcs/fuzz/net/quic/packet/runner.vr
verum run vcs/fuzz/net/quic/frame/runner.vr
verum run vcs/fuzz/net/h3/frame/runner.vr
verum run vcs/fuzz/net/h3/qpack/runner.vr
verum run vcs/fuzz/net/x509/der/runner.vr
```

A 30-minute nightly campaign script is the CI entry point; crashes
become permanent corpus entries under the adjacent `crash_seeds/`
directories.

## Invariants guarded

| Runner | Invariant |
|--------|-----------|
| tls13/handshake | `decode_handshake` never panics on any byte input; truncation/overflow returns `CodecError::Truncated` / `CodecError::Overflow` |
| quic/packet | `parse_long` / `parse_short` never panics; malformed headers return `PacketError` |
| quic/frame | `decode_all_frames` never panics; malformed frames return `FrameError` |
| h3/frame | `decode_frame` never panics; unknown frame types pass through (RFC 9114 §9) |
| h3/qpack | `decode_field_section` never panics; out-of-range static-table index returns `QpackDecodeError` |
| x509/der | `DerReader.read_tag_length` never panics; indefinite-length / tag-overflow return `DerError` |

## Relationship to Rust-side scaffolding

The README at `vcs/fuzz/harnesses/net/README.md` outlines Rust fuzz
harnesses (AFL++ / honggfuzz) that drive these same parser entry
points via the verum_vbc interpreter. The Verum-side runners are
self-contained (no external binary); the Rust-side harnesses are the
throughput path. Both agree on which parsers are in-scope and on the
"never panic" invariant.
