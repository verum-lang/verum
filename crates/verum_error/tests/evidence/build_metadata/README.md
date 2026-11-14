# Build metadata invalidation evidence

The shared `verum_error` crate has no build script or Git/timestamp inputs.
The application supplies `BuildIdentity` through `CrashReporterConfig`; the
CLI produces that identity in its own build script and uses it for both crash
reports and verbose version output.

[The probe](../../build_metadata_probe.py) copies the actual
[CLI build script](../../../../verum_cli/build.rs) and identity module into a
small executable that links the actual `verum_error` and `verum_common` crates.
One small intermediate consumer tests transitive Cargo freshness. The probe
checks real panic reports, captured metadata and Cargo artifact reuse in ordinary,
packed-ref, detached, linked and Git-free source trees. It also checks tracked,
staged and newly created CLI source inputs, path whitespace and explicit rejection
of path spellings that Cargo cannot preserve in watch directives.

From the repository root, choose a new evidence directory and an existing
session-private target on the development disk:

```sh
python3 crates/verum_error/tests/build_metadata_probe.py \
  --evidence-dir /Volumes/AGI-Dev/work/verum/build-metadata-check \
  --target-dir /Volumes/AGI-Dev/work/verum/existing-private-target
```

All Cargo invocations are offline and locked. No actual `verum_types`, CLI,
LLVM, Z3, automatic precompile or AOT build runs. The intermediate fixture is
representative; it is not evidence that the actual type checker was rebuilt.
The actual common/error dependencies must stay fresh after a Git ref update,
while the executable must publish the new SHA in its crash report.

Exit 0 means all measured metadata and reuse requirements pass; exit 1 records
violated requirements; exit 2 means the harness could not complete. The output
directory contains the fixture repositories, raw Cargo logs, crash reports and
result JSON; the target is reused. The probe refuses to overwrite evidence.
Repeated-pair waits cross the timestamp's one-second resolution without changing
source or filesystem timestamps.

## Retained baseline

The following evidence was produced by the earlier error-crate build script
and the earlier dependency-free probe. Those retained inputs and results have
not been rewritten as results of the current application-owned implementation.

[result.json](result.json) records source revision
`70ab902f0e642e365a9b2a531376f1d1802a8add`, build-script and harness hashes,
fixture input hashes, captured metadata, executable hashes/mtimes, environment
overrides and the hash of every raw log. The copied build script stayed unchanged
through all 11 successful Cargo test commands. Cargo and rustc were both 1.95.0;
the three fixture targets occupied 6.1 MiB together.

The overall probe correctly exits 1: seven assertions fail across five stages.

| Control | Observed result |
| --- | --- |
| Linked, detached worktree: identical invocation | Rebuilds; executable identity and embedded timestamp change. The second fingerprint trace identifies the absent `../../.git/HEAD` as `MissingFile`. |
| Ordinary checkout: identical invocation | Reuses the executable and timestamp. |
| Ordinary checkout: edit compiled `src/lib.rs` | The executed source marker changes, but the embedded dirty flag remains `clean` instead of `dirty`. |
| Commit that source edit | The embedded revision remains the initial revision instead of the new commit. |
| Advance the symbolic ref with an empty commit | The embedded revision remains stale. The text of `.git/HEAD` is unchanged. |
| Switch the ordinary checkout to detached HEAD, then repeat | Metadata refreshes on the HEAD-file change; the next identical invocation reuses the executable. |
| Git-free tree: identical invocation | Revision and dirty state correctly say `unknown`, but the absent HEAD watch causes another rebuild and timestamp change. |

The decisive causal trace is
[linked-identical.stderr.log](logs/linked-identical.stderr.log). Source-marker
and metadata output for the dirty-source counterexample is
[ordinary-source-dirty.stdout.log](logs/ordinary-source-dirty.stdout.log).
These controls establish defects in this build script, not that it is the sole
cause of every workspace rebuild.

An earlier exploratory invocation failed its rustdoc phase because inherited
`TMPDIR` pointed outside the permitted sandbox. Its original output is retained
in [setup-trial](setup-trial/result.json); it is not a successful control. The
final probe pins `TMPDIR` inside its evidence directory, and every Cargo command
in the baseline table completes successfully.

## Provenance and reuse contract

Preserve both artifact reuse and truthful provenance. In ordinary and linked
checkouts, attached and detached HEAD modes must reuse the executable and build
timestamp on an identical invocation. Actual ref changes must refresh the
embedded revision. Changed compiled source must report its current revision and
dirty state; committing or reverting that change must refresh the corresponding
metadata. Cover staged edits and newly created untracked paths before claiming
the existing repository-wide dirty-state contract is preserved. Git-free builds
must retain explicit `unknown` provenance and reuse unchanged output.

Resolving the real HEAD path alone cannot satisfy this boundary. Git provides
[`rev-parse --git-path`](https://git-scm.com/docs/git-rev-parse) for repository
path resolution, but symbolic ref and dirty-state inputs need their own bounded
authority. Cargo's
[`rerun-if-changed` contract](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rerun-if-changed)
also makes watching an entire directory recursive. Do not replace the missing
watch with recursive `.git` churn, force timestamps, or freeze metadata merely
to obtain a cache hit.

The lightweight probe does not qualify a complete compiler rebuild. Full
compiler-test freshness requires a separate unchanged invocation on a warm
target with its own Cargo and executable identities.
