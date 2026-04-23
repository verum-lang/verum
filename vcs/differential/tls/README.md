# TLS 1.3 Differential Harness — scaffolding

Cross-implementation TLS 1.3 wire-format parity harness per
`internal/specs/tls-quic.md` §2.5 / §10.3.

## Planned scope

| Layer | Reference 1 | Reference 2 | Entry point |
|-------|-------------|-------------|-------------|
| Record layer | rustls 0.23 | s2n-tls 1.5 | `record/` |
| Handshake | picotls | BoringSSL | `handshake/` |
| X.509 parser | rustls-webpki | OpenSSL 3.x | `x509/` |

## RFC 8448 KAT fixtures (Phase 2 exit gate)

Paired companion under `vcs/specs/L2-standard/net/tls13/rfc8448_*`.
Appendix A / B / C handshake transcripts become golden vectors here
once the handshake driver produces byte-exact wire output.

## Bootstrap status

Partial — see `vcs/specs/L2-standard/net/tls13/` which already runs
integration tests (typecheck, keyschedule KATs, post-handshake auth,
resumption / 0-RTT antireplay). The differential harness comes online
once the typed session layer (`core.net.tls13.session.{TlsClient,TlsServer}`)
is fed a complete handshake transcript end-to-end (phase 2 step 3
per spec §12).
