# Ordinary build and initial registry field-authority replay

The ordinary CLI build at `e86f2abd77e041e2f29cdcbf30d9629435e64f7e`
passes with automatic standard-library generation enabled. It takes 1,422.990
seconds and embeds the new v54 archive, metadata and symbol graph. The
[review](review.json) binds the retained executable, all five artifacts,
producer identities, original verbose build log and 5,370 source hashes
verified against immutable Git blobs.

The unchanged standalone `WeftRequest` fixture passes in 56.659 seconds,
including all seven public fields, duplicate headers, raw query data, byte
mutation and record construction. The `JsonError` fixture stops before runtime
in 7.269 seconds: four E202 errors reject fields on the returned error payload.
Its directly constructed diagnostic record produces no field error. These
observations distinguish successful direct request-data use from unresolved
nominal ownership on returned values; they do not prove the cause.

The unchanged registry preparation component at
`6c055769bab8deca3787b55e782e7ca61a7ce2c4` stops before runtime in 88.890
seconds with 33 diagnostics. Most reject public request fields as having
unknown declaration visibility; additional decoded-payload, exhaustiveness
and attribution errors remain visible. The credential-file component stops
before runtime in 121.700 seconds with two E400 errors: the success payloads
of `authorize` and `load_credentials` are reported as `AuthenticatedPublisher`
instead of their declared `PublisherAttribution` and `CredentialStore` types.
T1732 owns archive import identity; T1733 owns the distinct payload observation.
No registry fixture or expected output was changed to obtain these results.

Every original receipt and log is retained losslessly. The component wrappers
bind their copied project inputs and the unchanged committed component runner.
The producer fingerprint snapshots and pre-build parser invalidation snapshot
are retained as deterministic tar/gzip archives, with per-member hashes. Large
CLI, archive and producer binary bytes stay at the retained paths identified in
the original build manifest; they are not copied into Git.

The build log includes precompiler warnings, skipped bodies and trap-emitting
intrinsic diagnostics. A successful build is not acceptance of every baked
standard-library body. Earlier non-verbose logs did not capture the same nested
diagnostics, so this record does not classify those warnings as new regressions.
The six external privacy controls and preparation-copy runtime control remain
pending the compiler repair. The unchanged [authentication-bounds component](auth-bounds/review.json) passes
in 543.100 seconds with exact expected output and unchanged source/project/CLI
identities. Its original receipt and inputs are retained separately; it does
not cover credential-file loading or publication preparation. Full VBC socket failures,
native/AOT validation and durable authenticated HTTP publication remain open.
