# core-tests/net/tls13/error — audit

`core/net/tls13/error.vr` — RFC 8446 §6 TLS 1.3 alert/handshake error ADT (maps to AlertDescription via to_alert). Representative subset of ~25 unit variants: ctors + Eq + broad pairwise disjointness (12 GREEN + 1 @ignore). Qualified Type.Variant.

## Pinned defect
- BAREVAR-ADT-1: `TlsError.InternalError` Eq mis-dispatches (bare-variant first-wins collision with tls13/alert + other stdlib `InternalError` variants). `test_tls_error_internal_error` @ignore`d. Fix = qualify TlsError impl match arms.

## Deferred
- to_alert() -> AlertDescription mapping + RemoteAlert/IoError payload variants (cross-type surface).
