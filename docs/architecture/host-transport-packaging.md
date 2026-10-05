# Host CLI transport dependencies

The distributed host CLI may use baseline operating-system libraries. This is
separate from the no-libc contract of generated AOT programs. The full host
release acceptance remains execution of each exact asset on a clean, oldest
supported OS image, with its dependency closure available without extra user
installations.

## Git and TLS packaging boundary

The CLI keeps the default HTTPS and SSH capabilities of `git2`, but explicitly
requests `vendored-libgit2` and `vendored-openssl`. Cargo unifies the vendored
`openssl-sys` feature across the CLI's Git/SSH dependencies and the Linux
`reqwest`/`native-tls` dependency. This packages the transport implementation
instead of finding the builder's OpenSSL installation.

`reqwest` keeps its native TLS backend. Darwin still uses Security.framework;
Windows still uses Schannel. Linux keeps native-tls/OpenSSL and its existing
certificate-location probing. No certificate verification bypass, custom
production trust store, or TLS/SSH feature removal is introduced.

The release workflow selects static OpenSSL and vendored libgit2 and clears the
opt-in system-libssh2 probe. Environment overrides can still affect developer
builds, so source feature selection is not a substitute for inspecting the
resulting executable.

`scripts/ci/check_host_tls_dependencies.py` reads the **packaged executable**
from the actual tar/zip asset and records both asset and executable SHA-256.
It shares executable-format inspection with the AOT checker but does not use
its import allowlist. The host gate rejects direct OpenSSL/libgit2/libssh2
imports; macOS also rejects non-system dylib paths because these archives do
not ship a dylib bundle. Unknown formats and inspection failures are errors.
The report is uploaded with the archive and accompanies the release assets.

```sh
python3 scripts/ci/check_host_tls_dependencies.py --asset verum-example-aarch64-apple-darwin.tar.gz --report host-dependencies.json
python3 -m unittest discover -s scripts/ci/tests -p 'test_host_tls_dependencies.py'
```

A gate PASS has a narrow meaning: direct transport imports satisfy this rule.
It is **not** proof of clean-OS compatibility. In particular, Linux GLIBC and
GLIBCXX symbol versions, the complete transitive closure, Windows VC runtime
availability and representative service launches on clean images remain
separate, required release checks. The report lists these outstanding checks;
OS libc/framework imports are not rejected by applying an AOT policy to the
host.

## Measured boundary, 5 October 2026

The locked default-feature CLI graphs for all six release triples were
inspected. Both Darwin graphs reach OpenSSL through libgit2/libssh2, not
reqwest's native TLS backend. Both Linux graphs additionally reach the same
OpenSSL package through native-tls. Neither MSVC graph activates openssl-sys.

An isolated program using the CLI's locked reqwest/git2 versions and features
was compiled on arm64 macOS. With the original Git features its executable
imports Homebrew libssl/libcrypto and the gate rejects it. With the vendored
features the same source imports only macOS system libraries and passes the
transport gate. A localhost HTTPS request with a CA-signed serverAuth certificate
succeeds with the explicitly trusted test CA and fails with an unrelated CA;
Git reports both HTTPS and SSH capabilities enabled. SSH authentication and
remote repository operations were not exercised by this small probe. It is a
transport probe, not a rebuilt Verum release asset.

Re-inspection of all six previously audited release archives reproduces the
four macOS/Linux OpenSSL failures with matching archive/executable digests.
The two Windows assets pass this transport-only rule; their previously
observed VC runtime dependencies remain unresolved full-release acceptance.
The recorded old artifacts were not replaced by the probe results.
