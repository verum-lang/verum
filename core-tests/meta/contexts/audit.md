# `meta/contexts` audit

Module: `core/meta/contexts.vr` (~3742 LOC) — 14 capability-context
declarations (`BuildAssets`, `TypeInfo`, `AstAccess`, `CompileDiag`,
`MetaRuntime`, `MacroState`, `StageInfo`, `Hygiene`, `CodeSearch`,
`ProjectInfo`, `SourceMap`, `Schema`, `DepGraph`, `MetaBench`), 14
context-group aliases (`MetaCore`, `MetaFull`, `MetaTypes`, `MetaSafe`,
`MetaNoIO`, `MetaDerive`, `MetaAttr`, `MetaStaged`, `MetaAnalysis`,
`MetaProject`, `MetaSourced`, `MetaValidated`, `MetaDeps`,
`MetaProfiled`, `MetaTooling`) and ~30 payload data types.

Tests (task CONTEXTS-DATA-COVERAGE-1, 2026-07-06):

| file | @tests | pass | @ignore |
|---|---|---|---|
| `unit_test.vr` | 106 | 102 | 4 |
| `property_test.vr` | 36 | 36 | 0 |

Coverage: every runtime-constructible payload record is constructed and
read back at least once (AssetMetadata, SpanLabel, Suggestion,
DiagnosticBuilder, CacheStats, FunctionSearchResult, TypeSearchResult,
UsageInfo, PatternMatch, ItemInfo, DependencyInfo, SpanMapping,
SchemaError, BenchTimer, BenchResult, BenchStats, StageRecord,
TraceMarker, InvocationId, ParseResult<Int>, MetaParseError,
FormatOptions); every variant of the 6 payload enums
(DiagnosticSeverity, SuggestionKind, UsageContext, ItemKind,
SchemaErrorSeverity, BraceStyle) is matched at least once; every pure
method (`DiagnosticBuilder` fluent setters, `ParseResult`
`is_ok/has_errors/ok/to_result`, `MetaParseError.format`,
`FormatOptions.default/compact`, `InvocationId.to_text`,
`SchemaErrorSeverity`/`SchemaError` `Display`/`Eq`) is exercised.

## 1. Cross-stdlib usage

Meta contexts are compile-time capabilities: `using [Ctx]` clauses on
`meta fn`s. The runtime side of stdlib does not consume them; the data
records are the exchange currency between meta code and the compiler.

| consumer | which types | how |
|---|---|---|
| `core/meta/mod.vr:247-313` | 23 payload types | umbrella re-exports `public mount .contexts.X` (ParseResult, ParseError*, FormatOptions, BraceStyle, CacheStats, StageRecord, TraceMarker, FunctionSearchResult, TypeSearchResult, UsageInfo, UsageContext, PatternMatch, ItemInfo, ItemKind, DependencyInfo, SpanMapping, SchemaError, SchemaErrorSeverity, BenchTimer, BenchResult, BenchStats, DiagnosticBuilder, DiagnosticSeverity). *see §3.6 — `ParseError` does not exist. **Gaps:** SuggestionKind, SpanLabel, Suggestion, AssetMetadata, InvocationId and MetaParseError are NOT re-exported — consumers must mount `core.meta.contexts` directly (as this suite does) |
| `core/meta/mod.vr:451` | `MetaError.ParseFailed(List<ParseError>)` | payload type in the root error enum (§3.6) |
| `core/meta/contexts.vr:3586` | `ParseResult.to_result` → `MetaError.ParseFailed` | contexts → mod back-edge |
| `core/meta/quote.vr` | `AstAccess` | `QuoteBuilder` interpolation requires the `AstAccess` context |
| `core/meta/reflection.vr` | `TypeKind`, `FieldInfo`, … | reflection records are the return surface of `TypeInfo.*`; `TypeSearchResult.kind` embeds `TypeKind` |
| `core/meta/span.vr`, `core/meta/token.vr` | `Span`, `TokenStream` | embedded in 12 of the payload records tested here |
| derive/attr macro sites (`@derive(...)` expansion) | `MetaDerive`, `MetaCore` groups | consume subsets of the 14 contexts at expansion time |

## 2. Crate-side hardcodes

The Rust provisioning lives in two trees:

* `crates/verum_compiler/src/meta/builtins/` — per-context intrinsic
  method dispatch (the `@compiler_provided` bodies):
  * `build_assets.rs` — BuildAssets (`load`, `load_text`, `exists`,
    `list_dir`, `metadata` → constructs `AssetMetadata` field order)
  * `reflection.rs` + `type_props.rs` — TypeInfo (fields_of /
    variants_of / …)
  * `code_gen.rs` — AstAccess (parse_* / emit / pretty_print)
  * `code_search.rs` — CodeSearch (constructs FunctionSearchResult /
    TypeSearchResult / UsageInfo / PatternMatch / ItemInfo)
  * `project_info.rs` — ProjectInfo (constructs DependencyInfo)
  * `source_map.rs` — SourceMap (constructs SpanMapping)
  * `schema.rs` — Schema (constructs SchemaError; the schema builders'
    `_inner` handles)
  * `dep_graph.rs` — DepGraph
  * `meta_bench.rs` — MetaBench (constructs BenchTimer / BenchResult /
    BenchStats)
  * `stage_info.rs` — StageInfo (constructs StageRecord / TraceMarker)
  * `runtime.rs` — MetaRuntime + MacroState (constructs CacheStats)
  * `context_requirements.rs` — hardcodes which context each builtin
    requires (the `using [...]` enforcement); context-name strings
    (`"MetaCore"` at :149/:562 as the implicit always-available
    group) must stay in sync with the group aliases in contexts.vr
  * `tier0/` — always-available primitive builtins (arithmetic /
    collections / code_gen; no context needed); `tier1/` —
    context-gated builtins (constraint_reflection, diagnostics,
    structural_reflection, type_introspection)
* `crates/verum_compiler/src/meta/contexts/` — the context *state*
  the builtins read: `build_config.rs`, `diagnostics.rs`
  (DiagnosticBuilder/severity mirror), `execution_state.rs`,
  `security.rs` (BuildAssets path-traversal rules),
  `type_introspection.rs`.

Every payload record constructed on the Rust side (AssetMetadata,
CacheStats, FunctionSearchResult, TypeSearchResult, UsageInfo,
PatternMatch, ItemInfo, DependencyInfo, SpanMapping, SchemaError,
BenchTimer/BenchResult/BenchStats, StageRecord, TraceMarker) is a
**field-name drift surface** against contexts.vr — the Rust builders
populate name-keyed `ConstValue::Map`s and silently misalign if the
.vr record changes. The variant tag orders of the 6 enums
(DiagnosticSeverity 4, SuggestionKind 3, UsageContext 6, ItemKind 7,
SchemaErrorSeverity 3, BraceStyle 3) are likewise mirrored. The unit
tests in this folder pin the .vr side of each shape.

**Verified drift (already live):**
`crates/verum_compiler/src/meta/builtins/code_search.rs:188`
(`make_function_result`) builds `{name, return_type, attributes}` —
contexts.vr `FunctionSearchResult` declares
`{name, module, is_public, span, attributes, signature}`; the Rust
shape carries a field the .vr record doesn't have (`return_type`) and
omits four it does. `make_type_result` (`code_search.rs:204`) likewise
builds `{name, protocols, attributes}` vs the declared
`{name, module, is_public, span, attributes, kind}`. Any meta fn doing
`result.module` / `result.signature` on a live CodeSearch result gets
a missing field.

## 3. Language-implementation gaps

### §3.1 The 14 context methods are all @compiler_provided / @compiler_intrinsic

Every method on every context is implemented in Rust at the compiler
level; at Tier-0 runtime they return stub values. This is correct
architecture (contexts are compile-time capabilities) but means this
folder covers only the payload data shapes, not context behaviour.
Context behaviour is exercised at `crates/verum_compiler/tests/` and
`vcs/specs/L2-standard/meta/`.

Not testable at runtime, by design: the six `@compiler_type` unit
placeholders (MetaExpr/Type/Item/Pattern/Statement/MetaBlock), the five
opaque schema-builder records (`_inner: ()` + bodyless methods),
`BenchTimer.stop()/elapsed()` (bodyless), `DiagnosticBuilder.emit()`
(@compiler_intrinsic), and the `ToTokens` protocol (meta fn only).

### §3.2 META-CTX-ELEM-INFER-1 — unannotated locals from cross-module
record internals misresolve field indices — OPEN (workaround in place)

`let l = b.spans[0];` where `b: DiagnosticBuilder` (stdlib record)
leaves `l`'s static type unresolved; subsequent field reads fall back
to a dynamic lookup that resolves against the WRONG table:

* `l.label` returned the SPAN record; `l.is_primary` read the label
  Text (truthy); `l.span` panicked
  `field access out of bounds: field index 15 … exceeds object data
  size 24 type='SpanLabel'`.
* Same class for nested reads: `r.remaining.tokens` on
  `ParseResult<Int>` panicked with `field index 3 … data size 16
  type='TokenStream'` (index 3 is ParseResult's `success` slot — the
  outer record's index leaked onto the inner object).

**Workaround (used throughout this suite):** annotate the hop —
`let l: SpanLabel = b.spans[0];` / `let rem: TokenStream =
r.remaining;` — every read is then correct. Fix belongs in
verum_types (element/field type propagation through cross-module
record fields) or the VBC dynamic-field fallback.

### §3.3 META-CTX-BUILDER-CODE-METHODFIELD-1 — stdlib method sharing its
field's name returns a corrupt value cross-module — OPEN (1 @ignore)

`DiagnosticBuilder.code(mut self, code: Text) -> Self` collides with
the `code: Maybe<Text>` field. Called cross-module at Tier-0 the
invocation "succeeds" but the returned value is corrupt: any later
field read (or chained method) null-derefs (`opcode 0x62`), e.g.
`fresh_builder().code(t).primary_span(…)` died inside `primary_span`.
A LOCALLY-declared record with an identically-shaped `code`
method/field collision works correctly — the defect is specific to
archive-compiled/cross-module method dispatch. All non-colliding
builder methods (`error`, `warning`, `note`, `help`, `suggest`,
`primary_span`, `secondary_span`) work.

Pinned by `@ignore` `test_diagnostic_builder_code_sets_code_only`; the
field itself is covered via direct assignment
(`test_diagnostic_builder_code_field_direct_assignment`), and
`test_diagnostic_builder_full_chain` deliberately omits `.code(...)`.

### §3.4 META-CTX-FSTRING-NEWLINE-1 — `\n` in f-strings is emitted as a
literal 2-char sequence — OPEN (3 @ignore)

Language-wide inconsistency, isolated to a 3-line control:

```verum
f"x\ny".len()  // == 4  (backslash + n emitted literally)
"x\ny".len()   // == 3  (escape processed to a real newline)
```

`MetaParseError.format()` is built from f-strings
(`f"{msg}\nExpected: …"`), so its multi-line output currently carries
literal `\n` sequences. Three exact-equality tests pin the intended
real-newline contract and are `@ignore`d; three composition tests
(`starts_with` message prefix + `contains` section lines) cover the
pure logic escape-agnostically and pass. Fix belongs in the f-string
lexer/lowering (escape processing parity with plain string literals);
when it lands, drop the three `@ignore`s.

### §3.5 ParseResult "ok" divergence — is_ok() vs to_result() disagree —
PINNED (API wart, working as coded)

`is_ok()` = `success && errors.is_empty()`;
`to_result()` = `Ok` iff `success && value.is_some()`. On the cell
(value=None, success=true, errors=[]), `is_ok()` returns `true` while
`to_result()` returns `Err(ParseFailed([]))` — an empty error list.
Pinned by `law_parse_result_is_ok_vs_to_result_divergence_pinned`.
Suggested stdlib alignment: `is_ok` should also require
`value.is_some()` (or `to_result` should treat that cell as Ok-less
success explicitly).

### §3.6 MOD-REEXPORT-PARSEERROR-1 — `mod.vr` re-exports a name contexts.vr
does not define — OPEN

`core/meta/mod.vr:250` — `public mount .contexts.ParseError;` — but
contexts.vr defines `MetaParseError` (renamed at some point); there is
no `ParseError` anywhere in contexts.vr, and `MetaParseError` itself
is NOT re-exported (the rename was never propagated to the umbrella).
The root error enum still references the stale name:
`MetaError.ParseFailed(List<ParseError>)` (`mod.vr:451`). The resolver
accepts this silently (lenient re-export) — `ParseFailed` payloads are
effectively untyped at the .vr level. Either restore a
`public type ParseError is MetaParseError;` alias in contexts.vr or
rename the re-export + enum payload to MetaParseError.

### §3.7 Historical: cross-module record construction — CLOSED

The May-2026 revision of this audit deferred all payload-record tests
to "once the cross-module record ctor defect closes". It has closed:
all 22 runtime-constructible payload records now construct, read back,
and (where implemented) dispatch Display/Eq correctly cross-module —
including f-string Display of enums and records
(`f"{SchemaErrorSeverity.Error}" == "error"`,
`f"{schema_error}" == "schema error: boom"`). Only the §3.2 annotation
caveat remains.

## Action items landed in this branch

* `core-tests/meta/contexts/unit_test.vr` — expanded 32 → 106 tests:
  the 6 payload enums (kept verbatim) + construction/readback/pure
  methods for all 22 runtime-constructible payload records, including
  DiagnosticBuilder's fluent surface, ParseResult<Int> predicates and
  conversions, MetaParseError.format composition, FormatOptions
  presets, and Display pins for SchemaErrorSeverity / SchemaError /
  MetaParseError.
* `core-tests/meta/contexts/property_test.vr` — NEW, 36 laws:
  exactly-one-variant partition for all 6 enums; SchemaErrorSeverity
  Eq reflexive/symmetric/transitive/variant-agreement (exhaustive
  3/9/27); SchemaError Eq span-exclusion + per-field discrimination;
  DiagnosticBuilder last-write-wins / count / order / kind-partition /
  independence / chain-vs-stepwise; ParseResult exhaustive 2x2x2 truth
  table incl. the §3.5 divergence pin; FormatOptions determinism +
  axes; InvocationId.to_text f-string agreement / determinism /
  injectivity; MetaParseError.format sectional composition (2x2).
* `core-tests/meta/contexts/audit.md` — this file (rewritten).

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Fix cross-module method/field name-collision dispatch (§3.3), then de-@ignore `test_diagnostic_builder_code_sets_code_only` | verum_vbc / verum_compiler method dispatch | 0.5-1 day |
| Process escapes in f-strings (§3.4), then de-@ignore the 3 format exact-equality tests | verum_lexer / f-string lowering | 0.5 day + sweep for code relying on literal `\n` |
| Element/nested type propagation for cross-module record fields (§3.2) — removes the annotation requirement | verum_types | 1-2 days |
| Align `ParseResult.is_ok` with `to_result` (§3.5) | core/meta/contexts.vr | 15 min + re-run suite |
| Restore/rename the `ParseError` re-export (§3.6) | core/meta/mod.vr + contexts.vr | 15 min |
| `ParseResult<TokenStream>` with a real parsed payload (needs `AstAccess.parse_expr_recovery` at compile time) | vcs/specs L2 + verum_compiler tests | 0.5 day |
| Cross-tier `--aot` validation of this folder | this folder | gated on stdlib AOT build task |
| Drift-pin the Rust-side record shapes (§2) with mirror tests | crates/verum_compiler | 1 day |
