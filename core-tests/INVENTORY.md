# `core-tests/` — module inventory

Live inventory of which `core/` modules have a matching test folder under
`core-tests/`, the LOC of each test file, and the open audit deferrals.

The CI contract: every `@test` here passes under both `verum test --interp`
(Tier 0 VBC interpreter) and `verum test --aot` (Tier 2 LLVM AOT). `@ignore`d
tests pin known stdlib / language-level defects and are excluded from the
default green-suite gate.

| module | unit | property | integration | regression | audit | open deferrals |
|---|---:|---:|---:|---:|---:|---|
| `collections/union_find` | 358 | 371 | 147 | 197 | 229 | 5 (Map.get → Maybe<V>; Map.contains_key(&K); lenient-skip on Map.get_optional/get_key_value; Text.from_utf8_unchecked zero-length as_bytes; Text.eq method dispatch) |

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
