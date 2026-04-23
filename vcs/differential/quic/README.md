# QUIC Differential Harness — scaffolding

Cross-implementation QUIC wire-format parity harness per
`internal/specs/tls-quic.md` §2.5 / §10.3.

## Planned scope

| Layer | Reference 1 | Reference 2 | Entry point |
|-------|-------------|-------------|-------------|
| QUIC Initial packet (RFC 9001 App A) | quiche 0.22 | ngtcp2 v1 | `initial/` |
| QUIC frame codec | quiche | picoquic | `frame/` |
| QUIC loss detection | quiche | picoquic | `recovery/` |

Each subdir contains:

* `golden/` — byte-exact reference fixtures (hex-text one per line).
* `cases.toml` — test manifest mapping fixtures to seeds.
* `run.sh` — invoke pure-Verum codec, diff against fixture.

## Bootstrap status

Today: KAT vectors already landed under
`vcs/specs/L2-standard/net/quic/rfc9001_*_kat.vr`
(initial secret, retry integrity, ChaCha20 HP). They exercise the
pure-Verum implementation against fixed RFC vectors.

Remaining work before CI gate:

1. Harvest 100+ captures of live handshakes from each reference impl.
2. Wire the harness into `vtest` via a new `differential quic` command.
3. Fail CI on any byte-level divergence in Initial header protection
   or frame round-trip.
