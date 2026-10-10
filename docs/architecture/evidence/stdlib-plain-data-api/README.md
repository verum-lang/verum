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
