# `search/types` audit

Module: `core/search/types.vr` (265 LOC) — abstract search-index
protocol + data ADTs. Defines `Document`, `SearchFilter`,
`SortDirection`, `SortSpec`, `SearchQuery`, `SearchHit`,
`FacetDistribution`, `SearchResults`, `IndexConfig`, `SearchError`
records/enums + `SearchIndex` protocol.

Tests focus on `SortDirection` (2-variant) + `SearchError`
(7-variant + Display). The full Query/Results/IndexConfig surface
involves nested `Maybe<SearchFilter>` and `JsonValue` which require
broader stdlib stability — deferred to property + integration tests
once dependencies settle.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.search.meilisearch` | implements SearchIndex protocol against Meilisearch HTTP API. |
| Application search code | `index.search(&query).await?` returns `SearchResults`. |

## 2. Crate-side hardcodes

None today. Future Rust-side intercepts (zero-copy result emission
to Meilisearch over HTTP) must preserve `SearchError` variant shapes.

## 3. Language-implementation gaps

### §3.1 `SearchQuery.limit` refinement — 1..=1000

Constraint `Int { >= 1, <= 1000 }` rejects out-of-range values at
construction. Test via `@expected-runtime-panic` once the fixture
is available. Refinement at 1000 is Meilisearch's hard cap; other
backends may have different limits (Elasticsearch up to 10000).
Document the cross-backend semantic.

**Effort:** small + cross-backend doc.

### §3.2 `SearchResults.estimated_total_hits` semantics — "estimated"

Backends report different precision for total-hit counts (Meilisearch
returns an estimate based on facet-count sampling; Elasticsearch
returns the exact total for queries within `track_total_hits` budget).
The field's `Int { >= 0 }` refinement doesn't distinguish "estimate"
from "exact". Add a sibling `Bool track_total_was_exhaustive` for
backends that can report this.

### §3.3 No `SearchError.Eq` impl

Cannot compare two errors. Pattern shared with ContextError /
StorageError / CacheError — add Eq impl following the qualified-
match-arm discipline.

**Effort:** ~30 min + tests.

### §3.4 `SortDirection` has no `reverse` method

Common Ord-helper pattern: `SortDirection.Asc.reverse() ==
SortDirection.Desc`. Add `fn reverse(&self) -> SortDirection`.

**Effort:** ~10 min + 2 tests.

## Action items landed in this branch

* `core-tests/search/types/unit_test.vr` — 21 unit tests covering
  SortDirection 2-variant + SearchError 7-variant + 7 Display
  rendering tests.
* `core-tests/search/types/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `@expected-runtime-panic` tests for SearchQuery.limit refinement | this folder | gated on test fixture |
| Add `SortDirection.reverse(&self) -> SortDirection` | `core/search/types.vr` + 2 tests | 15 min |
| Add `Eq` for SearchError | `core/search/types.vr` + tests | 30 min |
| Add property_test.vr (Display determinism, variant exhaustiveness) | this folder | 30 min |
| Sister tests for `core.search.meilisearch` adapter | `core-tests/search/meilisearch/` | 1 day |
