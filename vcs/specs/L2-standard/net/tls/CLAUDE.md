# TLS tests — caveats

## Name clash: `TlsError`

There are multiple `TlsError` types in stdlib:

- `core.net.tls.TlsError` — Transport Layer Security errors (HandshakeFailed / InvalidCertificate / ...)
- `core.sys.{linux,darwin,windows}.tls.TlsError` — **T**hread-**L**ocal-**S**torage errors (MmapFailed / ArchPrctlFailed / ...)

In a test file that mounts `core.*`, the `TlsError` identifier resolves
to the system-tls variant (comes first in resolution order), not the
network-tls one. Avoid returning `TlsError` directly from test-level
functions — either use `core.net.tls.TlsError` fully-qualified, or
structure the test to not reference it explicitly.

## @intrinsic stubs

TlsStream operations currently go through `@intrinsic("verum.tls.*")`
hooks that are **not yet resolved in the runtime**. Tests that exercise
actual handshake / read / write behavior will fail at execution time;
keep tests at `typecheck-pass` level until a backend (rustls /
OpenSSL / HACL\*) is wired.
