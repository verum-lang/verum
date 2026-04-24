# Warp performance benchmarks — per tls-quic.md §11

Release-gate performance targets for the pure-Verum TLS 1.3 + QUIC +
HTTP/3 + X.509 stack. Each benchmark is an `@test: benchmark` Verum
program that reports throughput / latency under the `vbench` runner.

## Targets (spec §11)

| Benchmark | Target | Reference |
|-----------|--------|-----------|
| TLS 1.3 handshake latency | ≤ 1.1 × rustls | 1-RTT full handshake |
| TLS record AEAD seal (AES-128-GCM) | ≥ 3 GB/s | server-grade x86_64 w/o AES-NI; ≥ 10 GB/s w/ AES-NI |
| TLS record AEAD open (ChaCha20-Poly1305) | ≥ 1.5 GB/s | without SIMD |
| QUIC packet process (1-RTT, 1350 B) | ≤ 1.5 µs | rustls+quiche ~1.3 µs |
| QUIC throughput (single-conn, localhost) | ≥ 40 Gbps | matches quiche |
| QUIC throughput (100K connections) | ≥ 10 Gbps total | 100K idle + 1K active @ 10 Mbps |
| QUIC connections/sec (server accept) | ≥ 50K | Ice Lake EC2 c5n.4xlarge |
| X.509 chain validate (3-cert LE chain) | ≤ 100 µs | OpenSSL ~80 µs |
| DER parse single cert | ≤ 20 µs | rustls-webpki ~15 µs |
| Memory per idle QUIC connection | ≤ 4 KiB | quiche ~3.2 KiB |
| Memory per active QUIC connection | ≤ 64 KiB | steady state |

## Running

```bash
cd vcs
make bench-net                               # full net bench suite
cargo run -p vbench -- run \
    vcs/benchmarks/net/tls/record_aead_seal.vr
```

Results land in `vcs/results/benchmarks/net/*.json` with the
`vbench` runner's standard shape:

```json
{
  "benchmark": "tls_record_aead_seal_aes128",
  "target":    "≥ 3 GB/s",
  "measured":  { "throughput_gbps": 3.8, "iterations": 10000 },
  "verdict":   "PASS"
}
```

## Layout

| Path | Target |
|------|--------|
| `tls/handshake_latency.vr` | TLS 1.3 1-RTT handshake |
| `tls/record_aead_seal_aes128.vr` | TLS record AEAD seal (AES-128-GCM) |
| `tls/record_aead_seal_chacha20.vr` | TLS record AEAD seal (ChaCha20-Poly1305) |
| `tls/record_aead_open_chacha20.vr` | TLS record AEAD open (ChaCha20-Poly1305) |
| `quic/packet_process_1rtt.vr` | QUIC 1-RTT packet process |
| `quic/throughput_localhost.vr` | QUIC throughput over loopback |
| `quic/accept_rate.vr` | QUIC server accept/s |
| `quic/idle_connection_memory.vr` | bytes per idle connection |
| `x509/der_parse_single.vr` | DER parse latency (1 cert) |
| `x509/chain_validate_le.vr` | Let's Encrypt 3-cert chain |
| `h3/request_rtt.vr` | GET request round-trip |

Each file lives as `@test: benchmark` with clear `@expected-performance`
tags so `vbench` can auto-verify against `§11` targets and
regression-fail the CI gate.

## Status

Scaffold only — runners emit structure + expected-performance tags.
The measurement implementations ship incrementally as each pure-Verum
code-path stabilises against the AOT backend (spec Phase 7).
