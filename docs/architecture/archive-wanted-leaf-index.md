# Wanted-leaf indexing during archive registration

T1625 reduces alias candidate work in `register_module_filtered` without changing
which names may bind to an archive function. The baseline controls are committed
as `23a2a734c`; the implementation is `bc18588f0` on source base `fdad59f06`.

## Registration contract

The caller's wanted set is indexed once per invocation as borrowed leaf keys and
borrowed complete names. Each bucket preserves the original set's iteration
order. A function visits only the bucket for its own leaf; the existing qualified
owner, whole-segment prefix compatibility, method/free-function distinction,
variant exception, and first-wins checks still decide admission.

This changes candidate selection from `F × W` visits to an index pass over `W`
plus one lookup per accepted function and visits to its matching bucket. It does
not promise linear work when many wanted names share the same leaf, nor bound
other work in archive loading. Names are not cloned into the index. No archive
wire format, compiler API, cache schema, or runtime behavior changes.

## Focused evidence

The controls in
[`wanted_leaf_index.rs`](../../crates/verum_compiler/tests/archive/wanted_leaf_index.rs)
parse source, generate VBC, serialize an archive, reload it, and check registration.
They cover same-leaf owners in both module orders, bare first-wins behavior,
whole-segment aliases in both prefix directions, rejection of foreign and partial
prefixes, and method/free-function separation. Descriptor-only variants retain
qualified names. A separately serialized real-constructor descriptor fixture
checks the existing bare/qualified variant exception. The index control checks
borrowed identity and candidate order across independently seeded wanted sets.

All six focused checks passed, including the explicitly enabled measurement.
The surrounding archive-loader unit group passed **28 tests**, with three
intentionally ignored diagnostics and the separately run measurement. Both
public lazy-loader controls in
[`archive_mount_declaration_owner.rs`](../../crates/verum_compiler/tests/archive_mount_declaration_owner.rs)
passed, including stale alias and foreign same-number function collisions.

The bounded measurement uses 128 source exports (149 VBC descriptors after
synthesis), verifies registration of every descriptor and lookup of all 128
exports, and times registration only. Source parsing and archive encoding are
outside the timer. These are median wall times over five samples in the
unoptimized Rust test profile on macOS arm64, not a full compiler benchmark.

| Wanted names | Baseline median | Indexed median |
| --- | ---: | ---: |
| 385 | 31.313 ms | 2.172 ms |
| 1,153 | 90.433 ms | 2.913 ms |
| 4,225 | 610.818 ms | 6.009 ms |

These observations establish the bounded registration improvement. They do not
establish a whole-CLI speedup, production latency bound, or AOT acceptance.
Fresh ordinary CLI numeric/List compilation and its unchanged output oracle
remain the separate T1625 acceptance step.

## Reproduction and artifact boundary

The isolated compiler test build used `VERUM_NO_AUTO_PRECOMPILE=1` and private
copies of the verified stage14 artifacts produced from `fdad59f06`, with schema
`v52-2026-10-05-semantic-formal-parameters` and source fingerprint
`def6d3e7b7421caf163b63529b2d0edd4bc2a09ec0004eb9cb89854b1cbae868`.
All three copies were SHA-256 checked before use:

| Artifact | SHA-256 |
| --- | --- |
| `runtime.vbca` | `410ff097b455437cab98b651f7799b3129588820925afe98e0de9c3781530a0f` |
| `runtime.core_metadata` | `a92bd89e649e102cdc67d7ad0032513d6448d8915e5744127e6cdf1f1e34ebbb` |
| `runtime.symbol_graph` | `a6678c18498f7519749ca1fdf1d22629b479083f42fa95a15177e8c7acf1c722` |

With those verified artifacts in a private target's `precompiled-stdlib`
directory and the existing LLVM installation configured, the isolated commands
are:

```sh
VERUM_NO_AUTO_PRECOMPILE=1 cargo test -p verum_compiler --lib wanted_leaf_index_tests -- --include-ignored --nocapture --test-threads=1
VERUM_NO_AUTO_PRECOMPILE=1 cargo test -p verum_compiler --lib archive_ctx_loader:: -- --test-threads=1
VERUM_NO_AUTO_PRECOMPILE=1 cargo test -p verum_compiler --test archive_mount_declaration_owner -- --nocapture --test-threads=1
```

Set `CARGO_TARGET_DIR` to that private target; an override LLVM installation may
also require the worktree's local `llvm/install` link because build scripts watch
its `llvm-config` path. The no-precompile setting here scopes an isolated consumer
test only and must not substitute for the ordinary CLI acceptance run.
