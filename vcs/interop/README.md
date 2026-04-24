# QUIC Interop Runner — scaffold

Release-gate matrix per `internal/specs/tls-quic.md` §10.4. Warp must
pass the green square on every scenario × every peer implementation
before v1 release.

## Scenarios (rows)

| Scenario | Tag | RFC | Description |
|----------|-----|-----|-------------|
| handshake | `H` | RFC 9000 §7 | 1-RTT full handshake |
| versionneg | `VN` | RFC 9000 §6 | client offers grease, server negotiates v1 |
| retry | `R` | RFC 9000 §8.1.2 | server issues Retry with address-validation token |
| resumption | `RSM` | RFC 8446 §2.2 / RFC 9000 §7.3 | PSK resumption (full 1-RTT with PSK) |
| zerortt | `Z` | RFC 8446 §2.3 | 0-RTT application data |
| blackhole | `B` | RFC 9002 §6 | loss of ALL packets → connection abort via PTO |
| multiconnect | `MC` | — | simultaneous connections share endpoint |
| chacha20 | `C20` | RFC 9001 §5.4.2 | TLS_CHACHA20_POLY1305_SHA256 selected |
| handshakeloss | `HL` | RFC 9002 §6 | Initial/Handshake packet loss + PTO recovery |
| transfer | `T` | — | 1 MiB stream transfer |
| transfercorruption | `TC` | — | transfer with payload bit-flips (peer must reject) |
| transferloss | `TL` | RFC 9002 | transfer with random packet loss |
| http3 | `H3` | RFC 9114 | GET request over HTTP/3 |
| rebinding-addr | `RA` | RFC 9000 §9 | NAT rebinding by address |
| rebinding-port | `RP` | RFC 9000 §9 | NAT rebinding by port |
| v2 | `V2` | RFC 9369 | QUIC v2 (**not v1-gate**, tracked for roadmap) |

## Implementations (columns)

Configured per spec §10.4:

| Impl | Ref | Role (client/server/both) |
|------|-----|----|
| ngtcp2 | https://github.com/ngtcp2/ngtcp2 | both |
| picoquic | https://github.com/private-octopus/picoquic | both |
| quiche | https://github.com/cloudflare/quiche | both |
| msquic | https://github.com/microsoft/msquic | both |
| mvfst | https://github.com/facebook/mvfst | both |
| aioquic | https://github.com/aiortc/aioquic | both |
| s2n-quic | https://github.com/aws/s2n-quic | both |
| neqo | https://github.com/mozilla/neqo | both |
| go-quiche | https://github.com/quic-go/quic-go | both |
| lucky7 | https://github.com/marten-seemann/quic-go-tracer | client |
| kwik | https://github.com/ptrd/kwik | both |
| **warp (us)** | pure-Verum | both |

## Running

```bash
# Full matrix — release-gate:
bash vcs/interop/run-matrix.sh

# Single scenario:
bash vcs/interop/run-scenario.sh handshake quiche

# Single row (warp vs every impl):
bash vcs/interop/run-row.sh handshake
```

## CI gate

For v1 release: every {scenario, impl} cell must be green **except**
`v2` (deferred). A divergent cell is a release blocker.

The runner writes `results/matrix.json` with the shape:

```json
{
  "scenarios": ["handshake", "versionneg", "retry", ...],
  "impls":     ["ngtcp2", "picoquic", "quiche", ...],
  "grid": {
    "handshake": {
      "ngtcp2":   "PASS",
      "picoquic": "PASS",
      "quiche":   "PASS",
      ...
    },
    ...
  }
}
```

## Bootstrap status

Scaffold only. The docker-based peer-impl harness is out-of-tree
(vendored under `../interop-runner-containers/`); this README and
the three entry scripts are the coupling points.
