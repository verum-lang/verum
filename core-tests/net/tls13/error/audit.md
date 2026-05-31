# core-tests/net/tls13/error — audit

`core/net/tls13/error.vr` — RFC 8446 §6 TLS 1.3 alert/handshake error ADT (maps to AlertDescription via to_alert). Representative subset of ~25 variants: 12 unit-variant ctors + Eq + broad pairwise disjointness, plus payload variants InternalError(Text)/IoError(Text) (ctor Eq + distinct-payload). 15 @test GREEN. Qualified Type.Variant (TlsError impls were already fully qualified).

## Deferred
- to_alert() -> AlertDescription mapping + RemoteAlert(AlertDescription) payload (cross-type AlertDescription surface).
