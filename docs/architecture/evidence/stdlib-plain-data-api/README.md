# Standard-library request and diagnostic field policies

The source change explicitly exposes the seven `WeftRequest` transport fields
and four `JsonError` diagnostic fields. They contain application data; the
records do not establish authentication or parser provenance.

On committed integration `bf09017ffb7a0493e547a5d5162f9ab97ac8e1bc`, all four
`stdlib_plain_data_visibility` tests passed. Each parses the complete real
source, retains the exact selected record declaration and owner, and checks
the produced and archive-decoded descriptors. The public fields survive;
the six `Parser` fields and `PathParam.value` remain private. Committed inputs
and inherited standard-library artifacts stayed unchanged.

`wire-validation.json` binds the exact runner, raw command log and receipts
to deterministic gzip copies. The retained test executable is identified by
hash in the result receipt; it is not embedded in the repository. The earlier
`source-preparation.json` remains the historical syntax-only receipt.

This gate accepts declaration and archive policy. It does not execute HTTP
or JSON bodies, prove compiler enforcement of those policies, rebuild the
standard library, or establish ordinary CLI, registry or AOT acceptance.
The two prepared Verum runtime fixtures require a fresh ordinary build.

The independent main landing uses frozen source
`41be1f4679357179fb1ddf56cda91d2118a5a563` on top of `042cd5bc8`.
Its five source/test files are byte-identical to the original reviewed changes.
The selected parser, VBC library and test fingerprints were preserved and
invalidated together before rebuilding that exact dependency chain.

All four controls passed in the Cargo execution. Its wrapper then failed
while interpreting verbose package-description text as an executable path;
the Cargo process exit was not persisted. That failure and its complete log
remain in `landing-cargo-result.json.gz`. One separately authorized direct
run of the exact retained executable passed the same four controls with
exit code zero. These are four unique controls, not eight. No Cargo rebuild
or source change separates the two executions.

`landing-validation.json` binds both runs, the cache preparation and the
retained fingerprint metadata. Source and inherited archive identities were
unchanged, and the test target was explicitly released. The declaration-only
scope and unexecuted ordinary runtime fixtures described above still apply.
