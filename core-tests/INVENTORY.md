# `core-tests/` — module inventory

Live inventory of which `core/` modules have a matching test folder under
`core-tests/`, the LOC of each test file, and the open audit deferrals.

The CI contract: every `@test` here passes under both `verum test --interp`
(Tier 0 VBC interpreter) and `verum test --aot` (Tier 2 LLVM AOT). `@ignore`d
tests pin known stdlib / language-level defects and are excluded from the
default green-suite gate.

| module | unit | property | integration | regression | open deferrals |
|---|---:|---:|---:|---:|---|
| `collections/union_find` | 358 | 371 | 147 | 197 | 5 (Map.get → Maybe<V>; Map.contains_key(&K); lenient-skip on Map.get_optional/get_key_value; Text.from_utf8_unchecked zero-length as_bytes; Text.eq method dispatch) |
| `collections/reservoir`  | 176 | 140 | 104 |  99 | 1 (core.sys.common.random_bytes intrinsic missing from VBC dispatch table — gates the replacement-phase API) |
| `collections/toposort`   |  76 |   0 |   0 | 100 | 4 (Map.contains_key(&amp;K) gates contains/idempotent add_node; Map.get → Maybe&lt;V&gt; gates the toposort algorithm itself; Text.from gates the Cycle-variant payload). regression-only outside of new()/add_node-distinct/empty-toposort. |
| `sys/bitfield`           | 452 |   0 |   0 | 114 | 1 (mount X.{public_const} not registered in codegen symbol table — tracked as #15). Cross-module dispatch closed in task #121; interpreter SIGABRT closed in task #14. Implementation in core/sys/bitfield.vr is `pure @inline(always)`. **complete** outside the remaining selective-mount const-import deferral. |
| `async/poll`             | 393 | 334 | 212 | 157 | 1 — `map_ok`/`map_err` on `Poll<Result<T,E>>` constrained-implement-block dispatch no-op (task #22, worked around in unit/property/integration via direct match-destructure projection — the algebraic identity is pinned without crossing the broken dispatcher). Closed earlier in branch: codegen-emit-MakeVariantTyped over MakeVariant for user sum types (Poll/LocalPair Debug fixed); blanket From<T> for Poll<T> removed (overlap with From<Maybe<T>>); receiver-aware method-chain inference lifted ahead of hardcoded MAYBE_RETURNING_METHODS table. **complete** outside the constrained-implement-block dispatch defect (which spans the type-system invariants surface, separate timeline). |
| `async/waker`            |  ~80|   ~50|   ~80|    ~80| 2 — fn_ref-as-Int identity stability; Debug auto-derive precedence. Closed §B/§C in this branch via record-literal Clone-Unit-corruption fix + Waker construction inlining. **partial**. |
| `async/future`           | ~150|  ~150|  ~150|     0| 15 — FutureExt blanket-impl-derived methods (block/map/and_then) not dispatched onto ReadyFuture (task #11); free-function `join`/`select` mount-resolution collides with 7-10 stdlib overloads of the same simple name (task #21, worked around in unit_test §4-5 via direct Join2/Join3/Select2 record-literal construction). **partial**. |
| `async/backoff`          |  ~120|   ~30|   ~30|     0| 7 — Backoff.<ctor> blocked by upstream CSPRNG intrinsic gap shared with reservoir. **partial**. |
| `async/task`             |  ~70|   ~40|   ~50|     0| 2 — TaskId.new atomic counter + JoinError Debug; both gated by upstream defects. **partial**. |
| `async/diagnostics`      |  ~90|   ~30|   ~30|     0| 0 — TaskLifecycleState 6-variant + TaskInfo. Pure data-type module. **partial** (carries no runtime tests). |
| `async/cancellation`     |  ~70|   ~20|   ~30|     0| 0 — CancelReason 4-variant + Aborted(Text) payload + List bookkeeping + CancellationError record-literal construction + Aborted-payload round-trip (extended in this commit). Timeout{deadline} arm deferred. **partial**. |
| `async/channel`          |  ~60|   ~30|   ~30|     0| 0 — TrySendError + TryRecvError variant algebra. **partial**. |
| `async/broadcast`        |  ~110|   ~40|   ~20|     0| 0 — BroadcastRecvError + TryRecvResult<T> + LagPolicy. **partial**. |
| `async/select`           |  ~70|   ~30|   ~20|     0| 0 — Either<A,B> + SelectError + race-outcome. **partial**. |
| `async/nursery`          |  ~60|   ~10|   ~20|     0| 0 — NurseryErrorBehavior 3-policy. **partial**. |
| `async/spawn_config`     | ~150|   ~30|   ~30|     0| 0 — RestartPolicy + IsolationLevel + Priority 4-rank. **partial**. |
| `async/spawn_with`       |  ~60|   ~10|   ~30|     0| 0 — CircuitState 3-variant breaker lifecycle. **partial**. |

## Status legend

When adding new modules to this index, mark each with a status keyword:

| status | meaning |
|---|---|
| **complete** | All public APIs covered by unit tests; algebraic laws pinned by property tests; cross-stdlib integration verified; audit findings landed or routed. |
| **partial** | Subset of the API surface covered. Reasons for partial coverage cited in the module's `audit.md`. |
| **regression-only** | Module is gated by upstream defects and no public-API tests pass yet — only `@ignore`d regressions exist to lock the bug shapes. |

For the website API reference (see `internal/website/`) we lift the same
keyword onto each module page so consumers see at a glance whether the API
is conformance-tested.

## How to update

When you finish a module:

1. Append a single-line row to the table above with the four LOC counts and a
   one-line summary of `audit.md` deferrals.
2. Do not restructure the table — append-only keeps the diff small for parallel
   PRs.
3. Update `internal/website/docs/stdlib/<module>.md` with the same status
   keyword.
