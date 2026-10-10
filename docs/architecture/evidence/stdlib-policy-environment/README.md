# Stdlib policy environment dependencies

The compiler build script reports both `VERUM_NO_AUTO_PRECOMPILE` and `DOCS_RS`
as Cargo environment dependencies before any policy branch or early return.
The existing presence-based policy remains: either variable disables automatic
refresh when present, including the value `0`. File and checksum dependencies
are still reported while refresh is disabled.

The [manifest](manifest.json) binds the paired source revisions, raw logs,
retained test executables and inherited product identities. The source pairs
differ only by the two environment directives and their comment.

The baseline executes the actual build-script entry point through a small,
dependency-free Cargo fixture. Its initial automatic build runs once; an
identical build stays fresh. Setting `VERUM_NO_AUTO_PRECOMPILE=1` incorrectly
stays fresh, leaves the invocation counter at one instead of two, and retains
the previous automatic-policy output. This is the intended failing control.

The fixed full `stdlib_cache_identity` target passes all eight tests. Across
seven policy states, each environment change runs the script exactly once,
and each identical repeat stays fresh. Both variables are checked with values
`1`, `0`, and absent, preserving both disabled-to-enabled and enabled-to-disabled
transitions. Cargo's verbose logs identify the changed variable. The requested
fingerprint trace filter produced no additional trace-level output; the evidence
does not claim otherwise.

The fixture invokes the compiled production entry point without changing its
logic. It supplies sentinel archive bytes and the real shared checksum to
select cache-hit or disabled policy, while an empty child executable search
path prevents an accidental baker. Those sentinels are deliberately not valid
archives. These results establish policy invalidation and cache reuse, not
archive validity, a successful stdlib bake, CLI execution, registry acceptance
or AOT support.

The complete tests and gate inputs remained unchanged during execution. The
five inherited archive files and all retained stage22 product files have the
same hashes before and after both gates. The Native target was returned idle
after the fixed gate. Raw files are retained losslessly as gzip members; binary
paths and hashes are recorded in each receipt without adding executables to Git.

Run the existing strict-CI target with automatic baking disabled:

```sh
VERUM_NO_AUTO_PRECOMPILE=1 cargo test --offline --locked \
  -p verum_compiler --test stdlib_cache_identity -- --test-threads=1
```

`VERUM_CACHE_POLICY_EVIDENCE_DIR` optionally names a new directory for the
individual Cargo invocation logs. The test refuses to reuse an existing one.

Local main `e0c9e20b6` passes the complete eight-test target in 111.393 seconds
(including compilation). All seven policy transitions and unchanged repeats
pass on that exact committed source. Its 14,738 source bindings, original
logs and retained executable were verified; all inherited archive files and
the pinned stage22 product remained unchanged. This closes the environment
invalidation repair without claiming an automatic bake or registry runtime.
