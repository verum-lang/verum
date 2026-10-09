# Merged array storage and declared numeric bodies (T1704)

Frozen production source: `fa6bf9a613da39d9fd1adbff353f3ec417249b02`, tree
`512ad85f3be46e40f82b14477da16a7531540588`.

The new `codex/array-storage-integration` branch merges held array-storage
checkpoint `9ad15cfb5c2aab6e4d121205467a0e72b4e3f580` with numeric-body and
socket integration `f3006f403ab059532cf45917057528683caed650`. Their common
ancestor is `31713d3c3047aa1e4e449a494f323c28e702d7e3`.
`codex/sha256-consuming-receiver` remains at the held checkpoint.

The merge resolutions preserve the final source-owner authority:

- `declared_numeric_receiver_method` retains the incoming `has_source_body`
  condition. The declared-function registration test retains the incoming
  exact export-key lookup across function-ID compaction.
- The earlier synchronous numeric-body interception was removed from
  `dispatch_primitive_method`; the [explicit removal diff](removed-synchronous-intercept.patch)
  records its 37 lines. The incoming `handle_call_method` implementation
  retains ordinary call frames, reference adaptation and autodiff argument
  provenance. Exact declared numeric bodies are selected before the
  primitive fallback.
- Relative to the incoming method dispatcher, the only remaining changes
  are the held twelve builtin endian byte-allocation arms; see the
  [complete delta](incoming-builtin-allocation-delta.patch). The source
  implementation and its selected result remain separate from this builtin
  fallback. Native source-owner parity still requires execution tests.

The [source identity audit](source-identity.json) records all 43 Rust files
changed from the common ancestor. Forty-one exactly match at least one
parent. The other two combine the incoming owner logic with the held endian
implementation: the expression emitter and interpreter method dispatcher.
Mechanical comparisons verify those two combinations. All 71 native source
and selected test files in the audited group are byte-identical to the held
parent. No SHA loop, argument ABI, unknown-call or signed-element scope was
added.

The [source-only gate receipt](source-gates.json) and [raw log](source-gates.log)
record a clean, unchanged merged checkout and exit zero in 24.310 seconds for
privacy, complete reference/citation checks, panic/diagnostic ratchets,
early-return tenants and duplicate emitters. The citation-name gate preserves
its existing roster of 33 names and 767 citations; it does not claim those
legacy citations are repaired.

No Cargo build, interpreter fixture, LLVM execution, JIT or AOT test ran for
this merge. The held checkpoint's 36 focused controls and the incoming
branch's tests retain their original source identities; neither result is
an execution gate for this combined tree. Selected-owner native controls,
the integrated full VBC gate, fresh ordinary compiler/archive, SHA-256 and
registry authentication replay remain pending. The deliberately refused
loop, unknown argument and mixed-producer cases remain outside acceptance.

[manifest.json](manifest.json) pins every file in this evidence directory.
