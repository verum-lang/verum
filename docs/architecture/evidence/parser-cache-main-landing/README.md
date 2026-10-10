# Main-based parser and archive cache acceptance

This landing combines the declaration-owned generic alias repair with parser
inputs in the automatic standard-library archive cache. It carries no pending
record-field visibility consumer implementation.

The [manifest](manifest.json) binds the exact main-based source, commands,
source hashes, retained test executables and logs. On that source, the twelve
parser/checker controls and eight compiler controls pass. The compiler control
uses `compile_core` to write a new minimal archive and reads it with the actual
archive and metadata decoders: `Identity<T>` retains parameter zero,
`Pick<L, R>` retains parameter one, and an unrelated marker stays a variant.

The cache controls preserve stable inputs and schema semantics, distinguish a
missing parser file from an empty one, cover the complete parser source roster,
and execute the actual disabled build script in isolated child processes. Both
values `1` and `0` retain the flag's presence-based no-auto meaning. Their
original causal failures and the earlier harness compilation failure remain in
[the cache evidence](../parser-archive-cache-identity/README.md).

The included historical parser receipts preserve the original harness failures,
corrected five-pass/seven-fail baseline, twelve-pass repair, and unfiltered
4,231-test parser gate. Those results retain their original source identities;
they are not a fresh full-suite run on the main-based candidate. The unrelated
field-policy consumer gate is not carried into this landing.

Both new gates use existing warm targets, one build job, offline dependencies,
disabled incremental compilation and disabled automatic precompilation. The
compiler target's five inherited standard-library files remain byte-identical
to the separately recorded e86 producer. They are not a rebake by these tests.
Ordinary CLI, automatic whole-library production, registry execution and native
or AOT acceptance remain separate. Cargo environment-only build-script reruns
are tracked separately from the directly executed no-auto control.

Raw logs and receipts are stored as deterministic gzip files. Each entry binds
both the stored file and its exact uncompressed bytes. Release receipts record
that each leased target was idle after its gate; test binaries remain outside
the targets at the recorded paths.
