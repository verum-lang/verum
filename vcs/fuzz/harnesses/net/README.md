# Warp fuzzing harnesses — TLS 1.3 / QUIC / HTTP/3 / X.509

Rust fuzz-target-crate harnesses that feed raw-byte inputs (from AFL++
or honggfuzz) into the pure-Verum parsers via the verum_vbc
interpreter.

## Harness list

| Path | Parser under test | Panic-surface cleared? |
|------|-------------------|------------------------|
| `tls13/handshake.rs` | `core.net.tls13.handshake.codec.decode_handshake` | spec §6 — no panic on truncation / overflow |
| `tls13/record.rs` | `core.net.tls13.record.aead::open` | spec §6 — AEAD tag mismatch → typed error |
| `quic/packet.rs` | `core.net.quic.packet::parse` | spec §7 |
| `quic/frame.rs` | `core.net.quic.frame::parse_frames` | spec §7 |
| `h3/frame.rs` | `core.net.h3.frame::parse_frame` | spec §10 (phase 6) |
| `h3/qpack.rs` | `core.net.h3.qpack.decoder::decode_field_section` | RFC 9204 §4.5 |
| `x509/der.rs` | `core.encoding.der::DerReader` | RFC 5280 — reject malformed cert |

## Seed corpus

One harness entry ↔ one `vcs/fuzz/seeds/net/<subdir>/` with:

- `valid/` — 100+ real-world captures that MUST parse + re-encode.
- `malformed/` — 1000+ truncation / field-overflow / encoding-corruption
  cases that MUST error without panic.

Initial seeds (this commit) cover the canonical RFC vectors:

- TLS 1.3 handshake: RFC 8448 App A/B/C ClientHello/ServerHello/EE/Cert/CV/Fin.
- QUIC packet: RFC 9001 App A Initial packets.
- X.509: 50+ real-world certs from Let's Encrypt / DigiCert / Amazon / Google chains.
- QPACK: RFC 9204 §A static-table vectors.

## CI gate

```bash
./vcs/fuzz/scripts/run_net.sh 30m    # 30-minute nightly fuzz per harness
```

Fail CI on any non-zero crash count. Regressions land in `crash_seeds/`
and become permanent corpus entries.
