# HTTP/3 + QPACK Differential Harness — scaffolding

Cross-implementation HTTP/3 wire-format parity harness per
`internal/specs/tls-quic.md` §2.5 / §10.3.

## Planned scope

| Layer | Reference 1 | Reference 2 | Entry point |
|-------|-------------|-------------|-------------|
| H3 frames (RFC 9114 §7) | quiche-h3 | ngtcp2/nghttp3 | `frame/` |
| QPACK encoder/decoder (RFC 9204) | quiche | nghttp3 | `qpack/` |
| H3 request flow | curl --http3 | quiche-client | `flow/` |

## Bootstrap status

Stub — HTTP/3 + QPACK are phase 6 of the warp roadmap. Directory
exists to pre-reserve the path for upcoming KAT fixtures (RFC 9204
static table conformance, Huffman round-trip KAT) and interop
captures.
