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
| `sys/bitfield`           | 452 |   0 |   0 | 114 | 3 (cross-module free-fn dispatch silently returns Unit at --interp; mount X.{public_const} not registered in codegen symbol table; SIGABRT in cbgr::handle_drop_ref on full-suite run). regression-only at runtime; implementation in core/sys/bitfield.vr is `pure @inline(always)` and the suite turns green when the dispatch defect closes. |
| `async/poll`             | 393 | 334 | 212 | 157 | 0 — full Poll<T> surface conformance-tested. Closed in this branch: codegen-emit-MakeVariantTyped over MakeVariant for user sum types (Poll/LocalPair Debug fixed); blanket From<T> for Poll<T> removed (overlap with From<Maybe<T>>); receiver-aware method-chain inference lifted ahead of hardcoded MAYBE_RETURNING_METHODS table. **complete**. |

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
