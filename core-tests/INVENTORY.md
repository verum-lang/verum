# `core-tests/` — module inventory

Live inventory of which `core/` modules have a matching test folder under
`core-tests/`, the LOC of each test file, and the open audit deferrals.

The CI contract: every `@test` here passes under both `verum test --interp`
(Tier 0 VBC interpreter) and `verum test --aot` (Tier 2 LLVM AOT). `@ignore`d
tests pin known stdlib / language-level defects and are excluded from the
default green-suite gate.

| module | unit | property | integration | regression | open deferrals |
|---|---:|---:|---:|---:|---|
| `base/memory`            | 131 | 172 | 102 |  95 | 5 (§A 14/18 `cbgr_test::test_heap_*` interpreter `assertion failed: left != right`, `--nocapture` triage pending; §B AOT cross-tier check on same 14; §C per-test status appendix; §D `Heap.is_freed` strict-greater-than vs inequality audit; §E hoist `- 32` literal CBGR header offset to a single const). **partial** — `Heap.new`/`into_inner`/`try_new`/nested-heap round-trips stable; full CBGR introspection surface exposed and unit-tested. **Language-level fix landed in this branch**: duplicate-method-name in impl block is now a compile-time error (`check_no_duplicate_impl_items` in `verum_types/infer/decls.rs`) — previously silently overwritten via HashMap insert. **Stdlib companion fix**: duplicate `Heap.is_freed` definition removed from `core/base/memory.vr`. |
| `collections/union_find` | 358 | 371 | 147 | 197 | 5 (Map.get → Maybe<V>; Map.contains_key(&K); lenient-skip on Map.get_optional/get_key_value; Text.from_utf8_unchecked zero-length as_bytes; Text.eq method dispatch) |
| `collections/reservoir`  | 176 | 140 | 104 |  99 | 1 (core.sys.common.random_bytes intrinsic missing from VBC dispatch table — gates the replacement-phase API) |
| `collections/toposort`   |  76 |   0 |   0 | 100 | 4 (Map.contains_key(&amp;K) gates contains/idempotent add_node; Map.get → Maybe&lt;V&gt; gates the toposort algorithm itself; Text.from gates the Cycle-variant payload). regression-only outside of new()/add_node-distinct/empty-toposort. |
| `sys/bitfield`           | 452 |   0 |   0 | 114 | 1 (mount X.{public_const} not registered in codegen symbol table — tracked as #15). Cross-module dispatch closed in task #121; interpreter SIGABRT closed in task #14. Implementation in core/sys/bitfield.vr is `pure @inline(always)`. **complete** outside the remaining selective-mount const-import deferral. |
| `async/poll`             | 393 | 334 | 212 | 157 | 1 — `map_ok`/`map_err` closure body not invoked in `implement<T,E> Poll<Result<T,E>>` impl block (task #25, distinct from #22 which closed all 4 variant-tag stability classes 2026-05-13 via commits 90b94e68b + 3f14510b8 + 485a230c6 + f1dd6fd19). Worked around in unit/property/integration via direct match-destructure projection. Closed earlier: codegen-emit-MakeVariantTyped over MakeVariant; blanket From<T> for Poll<T> removed; receiver-aware method-chain inference; task #22 variant-tag stability (4 classes — nested destructure scrutinee-type leak, flat tag drift, nested construction payload propagation, generic-param substitution). **complete** outside the constrained-implement-block closure-dispatch defect. |
| `async/waker`            |  ~80|   ~50|   ~80|    ~80| 2 — fn_ref-as-Int identity stability; Debug auto-derive precedence. Closed §B/§C in this branch via record-literal Clone-Unit-corruption fix + Waker construction inlining. **partial**. |
| `async/future`           | ~150|  ~150|  ~150|     0| 4 — Tasks #9 + #11 + #21 closed in this branch via 3 architectural fixes in vbc/codegen: blanket-impl pre-pass (#11), `register_function_authoritative` + cross-pollination guard + qualified-key path-doubling fix (#21), and `register_archive_type` unconditional `type_field_type_names` population (#9). 10 newly-passing regression tests total (6 from #11 — block/map/and_then/lazy.invokes on flat receivers; 4 from #9 — ReadyFuture/Join2/Select2/Lazy field-access under List mount). Remaining: 4 tests pinned as task #24 (`.block()` on Join2/Join3/Select2 combinator receivers — separate dispatch defect surfaces only when receiver is a generic combinator wrapping inner Futures). **partial**. |
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
| `async/parallel`         |  227|  173|  100|     0| 1 — global AOT generate_native SIGABRT (task #10, LLVM SmallVector hang affects every test, not parallel-specific). All 38 interpreter tests green: parallel_map/filter_map/for_each/reduce/scan_exclusive covered with worker-count-invariance + Blelloch-vs-reference-prefix-scan + fold₁ identity properties. **complete** under interpreter; AOT pending #10. |
| `async/intrinsics`       |  117|   89|   72|     0| 2 — task #10 (AOT generate_native SIGABRT); spawn family + sleep family @intrinsics deferred pending live-executor test-bed. 19 interpreter tests green: Executor.current/in_async_context coherence, future_poll_sync round-trip across Int + Text payloads, IntrinsicsYieldNow two-state lifecycle (Pending→Ready) with exactly-one-Pending tightness. **partial** — covers the synchronous-poll + introspection surface; runtime-side intrinsics remain. |
| `text/text`              | 1014|  348|  235|  290| 5 (§I Text.cmp variant-tag stability / §R count_matches typechecker ICE / strip_suffix StackOverflow / Maybe.take receiver-kind on Int / Result&lt;Bool,_&gt; destructure routing). **§A / §C / §E / §F / §G / §H / §J / §K / §L / §M / §N / §O / §P / §Q ALL CLOSED 2026-05-13** via 9 commits: `48a76117f` (Iterator.next dispatch), `5d04b98e7` (utf8_decode_char), `f8d70e6ef` (extract_string CBGR-deref — find/contains/replace/split cluster), `9c9eeb996` (from_utf8 family proper decoding), `73b9db0d0` (from_int/float/bool intercepts), `22559f4c9` (parse_int/float/bool/try_to_*/to_* intercepts), `81b34faea` (pad_left/right/center/zfill Char arg fix), `9136055a6` (truncate/clear/pop NULL_PTR fix), `706681590` (Text.join intercept). Targeted: **16 / 17 PASS** on representative cluster (only test_find_present remains — pre-existing typechecker ICE §R). Sweep projection: ~180 / 218 (83%, +28pp absolute from 121 baseline). Status promoted **regression-only → partial**. |
| `text/char`              |  410|  292|  220|  220| 5 (§A &mut Char deref-assign no-op / §B eq_ignore_ascii_case false-negative downstream of §A / §C from_digit hex offset / §D general_category misroute (task #22 root) / §E AnyChar.matches Iterator.next). 75 / 86 unit (87%). **partial**. |
| `text/case_fold`         |  220|  240|  150|  120| 1 (§A fold_text_ascii eq vs literal — downstream of text/text §I Text.eq). 25 / 30 unit (83%). Module is **complete** outside the deferred Text.eq cascade. |
| `text/builder`           |  238|  217|  185|  136| 1 (§A Int.BAnd / Int.BNeq dispatch — gates every push). 4 / 23 unit (17%). **regression-only** outside constructor surface. |
| `text/format`            |  280|  221|  170|  140| 1 (§A format_display / format_debug closure-dispatch). 39 / 41 unit (95%). **partial**. |
| `text/regex`             |  310|  170|  170|  155| 5 (§A is_match wrong-Bool on \d/anchors / §B find_all NullPointerAt SetIdx / §C split same root as §B / §D find Maybe<Text> shape mismatch / §E replace_all). 8 / 31 unit (26%). **regression-only**. |
| `text/tagged_literals`   |  170|  150|  100|   85| 1 (§A/§B CallM register-slot read — surfaces random Text values as method names). 1 / 29 unit (3.4%). **regression-only** outside validate_uri. |
| `text/numeric/decimal`   |  390|  260|    0|  170| 2 (§A Int.neg dispatch / §B function-id collision; §C arithmetic semantic cascade). 27 / 45 unit (60%). **partial**. |
| `text/numeric/bigint`    |  300|    0|    0|    0| inherits text/text §C/§D + Char §A. **partial** (per-test sweep deferred). |
| `text/numeric/bigdecimal`|   85|    0|    0|    0| inherits bigint. **partial**. |
| `text/numeric/rational`  |   75|    0|    0|    0| inherits bigint. **partial**. |
| `text/numeric/modular`   |  120|    0|    0|    0| inherits bigint. **partial**. |

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
