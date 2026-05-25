# `meta/contexts` audit

Module: `core/meta/contexts.vr` (~3742 LOC) — 14 capability-context
declarations (`BuildAssets`, `TypeInfo`, `AstAccess`, `CompileDiag`,
`MetaRuntime`, `MacroState`, `StageInfo`, `Hygiene`, `CodeSearch`,
`ProjectInfo`, `SourceMap`, `Schema`, `DepGraph`, `MetaBench`) and
their ~40 payload data types.

Tests: 26 unit tests over the pure-data enum types
(DiagnosticSeverity / SuggestionKind / UsageContext / ItemKind /
SchemaErrorSeverity / BraceStyle).

## 1. Cross-stdlib usage

Every `@derive(...)` macro, every `@meta_macro` definition, and
every `quote { ... }` expansion site consumes some subset of these
contexts. The runtime side of stdlib does not use contexts (they
are compile-time-only).

| consumer | how |
|---|---|
| `core.meta.mod` | re-exports the 20+ payload types via `public mount .contexts.X` |
| `core.meta.quote` | `QuoteBuilder.interpolate` requires `AstAccess` ctx (a context!) |
| `core.meta.reflection` | reflection types are returned from `TypeInfo.*` ctx accessors |
| `verum_compiler::contexts` | the actual implementation of all 14 contexts |

## 2. Crate-side hardcodes

* `verum_compiler::contexts::DiagnosticSeverity` mirrors 4-variant.
* `verum_compiler::contexts::SuggestionKind` mirrors 3-variant.
* `verum_compiler::contexts::UsageContext` mirrors 6-variant.
* `verum_compiler::contexts::ItemKind` mirrors 7-variant.
* `verum_compiler::contexts::SchemaErrorSeverity` mirrors 3-variant.
* `verum_compiler::contexts::BraceStyle` mirrors 3-variant.

All payload types (CacheStats, FunctionSearchResult,
TypeSearchResult, UsageInfo, PatternMatch, ItemInfo, DependencyInfo,
SpanMapping, SchemaError, BenchTimer, BenchResult, BenchStats,
StageRecord, TraceMarker, AssetMetadata, ParseResult,
MetaParseError, FormatOptions, DiagnosticBuilder, SpanLabel,
Suggestion) have Rust-side mirrors in `verum_compiler::contexts::*`
that the lowering populates before handing to a `meta fn`.

## 3. Language-implementation gaps

### §3.1 The 14 context **methods** are all @compiler_provided / @compiler_intrinsic

Every method on every context type is implemented in Rust at the
compiler level; at runtime (Tier 0 interp) they return stub values.
This is **correct architecture** — contexts are compile-time
capabilities — but means runtime tests in this folder cover only
the **payload data shapes**, not the context behaviour.

The full context method contracts are exercised at:
- `crates/verum_compiler/tests/contexts/*.rs` (Rust side)
- `vcs/specs/L2-standard/meta/contexts/*.vr` (Verum side, vtest)

### §3.2 Cross-module fn-return defect blocks payload-record construction

CacheStats, FunctionSearchResult, TypeSearchResult, UsageInfo,
DependencyInfo, FormatOptions, BenchResult — these are record
types with multiple Span / Maybe<Text> / Map<…> fields. Direct
record construction at the test site is feasible but lengthy
because the records each have ~7-15 fields. Property tests for
these records once the cross-module ctor return defect closes (see
[meta/span audit §3.1](../span/audit.md)).

### §3.3 `ParseResult<T>` is generic — testing requires a concrete T

`ParseResult<T>` is generic over the parsed value. Testing
`.is_ok` / `.has_errors` / `.ok` / `.to_result` requires a
concrete `T`. Easiest pick: `ParseResult<TokenStream>`. Deferred
until the cross-module record ctor returns + TokenStream construction
work in cross-module context.

### §3.4 Five `unit-type` placeholders: MetaExpr / Type / Item / Pattern /
Statement / MetaBlock are all `type X is ();`

These are opaque tokens returned from `AstAccess.parse_*`. The
unit type makes them un-introspectable at runtime; the actual
AST representation lives in `verum_ast`. No direct surface to
test here.

## Action items landed in this branch

* `core-tests/meta/contexts/unit_test.vr` — 26 unit tests over the
  6 enum data types: DiagnosticSeverity 4-variant + SuggestionKind
  3-variant + UsageContext 6-variant + ItemKind 7-variant +
  SchemaErrorSeverity 3-variant + BraceStyle 3-variant.
* `core-tests/meta/contexts/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property-test `Display` implementations for the 6 enum data types | this folder | 30 min |
| Tests for the 21 payload record types (CacheStats, FunctionSearchResult, ...) once cross-module record ctor returns work | this folder | 2-3 days |
| `ParseResult<T>` `.is_ok` / `.has_errors` round-trip tests with concrete `T = TokenStream` (§3.3) | this folder | 1 h after §3.2 |
| Integration tests for the 14 context dispatchers exercised at compile time (vcs/specs/L2 + crates/verum_compiler/tests) — drop drift-pinning macros mirroring this folder's enum-variant assertions | verum_compiler | 1 day |
