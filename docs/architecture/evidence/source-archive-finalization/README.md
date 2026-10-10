# Source-cog archive finalization

The source-cog producers check tar termination, gzip compression and trailer
completion, and the final writer flush before returning an output path. Their
paths, compression levels and tar options are unchanged. A successful flush
does not establish atomic publication or filesystem durability.

The causal baseline `82c39ac4b` passes three controls and fails two: successful
tar termination hides a refused gzip trailer write, and a complete compressed
stream hides the final writer flush error. The fixed source `9d6725c61` passes
all six selected CLI library controls, including the actual publication archive
producer. Neither gate ignores tests. `guard-interruption` preserves a separate
attempt stopped before tests by a conservative CVC5 wrapper guard; it is not a
test failure or an acceptance result. Inspection and the following run establish
that the selected default-only wrapper emits `cvc5_stub`, with native solver
build features disabled.

Each run's JSON binds the exact command, source inputs, executable, log and
inherited precompiled artifacts. The complete logs are losslessly compressed
as `gate.log.gz`; `integration.json` records both compressed and original hashes.
The two executable copies are retained at the locations in the run receipts.
All five standard-library artifact files remained byte-identical to the
successful `7a8c86f90` automatic producer. These focused no-auto tests do not
validate a new bake, ordinary CLI execution, AOT, or registry admission.

`producer-fixture` contains bytes emitted by the actual `create_cog_tarball`
function: a canonical manifest, a short binary file and a binary file requiring
one GNU long-name extension. The compressed output is retained as
`archive.tar.gz`; its original capture name is `archive.vr` in `expected.json`.
The decompressed tar and exact project input files are retained alongside it.
The locked Rust gzip and tar readers verify framing and exact entry contents.
Acceptance by the shared Verum decoders remains a separate execution gate.
