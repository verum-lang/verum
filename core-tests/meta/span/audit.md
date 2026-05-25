# `meta/span` audit

Module: `core/meta/span.vr` (~440 LOC) — source spans, locations,
ranges, and multi-spans for the metaprogramming layer.

Tests: 25 unit tests covering SpanFlags 3-Bool record +
MetaSpan direct-field construction + .is_synthetic/.is_expansion +
Eq-by-(id,hygiene) + Span alias + SourceLocation 4-field record
(direct construction only) + SpanRange 2-field record + MultiSpan
record.

Plus 5 `@ignore`-pinned regressions in `regression_test.vr` for
the cross-module ctor return-value field-access OOB defect class.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.meta.token` | every Token / TokenStream / TokenTree / Group carries a `Span` |
| `core.meta.attribute` | Attribute / AttributeArg carry a `Span` |
| `core.meta.reflection` | FieldInfo / VariantInfo / FunctionInfo / etc. carry `Span` |
| `core.meta.quote` | QuoteBuilder / GroupBuilder / QuotePart carry `Span` |
| `core.meta.contexts` | DiagnosticBuilder / SpanLabel / SpanMapping carry `Span` |
| `core.meta.tactic` | not directly used (tactic algebra is span-free) |
| `verum_compiler::lower::span_table` | mirror of `MetaSpan.id` registry |
| `verum_ast::span` | parser-side `Span` mirror |

## 2. Crate-side hardcodes

| site | hardcode |
|---|---|
| `verum_ast::span::Span` | `{ start: usize, end: usize, file_id: u32 }` — the parser's span, **not** layout-compatible with stdlib `MetaSpan`. The crossing is handled by `verum_compiler::lower::span_table` which translates AST-level Span → stdlib MetaSpan via a side table when emitting AST data into meta-fns. Any reshape of `MetaSpan` requires updating the lowering. |
| `verum_compiler::derives::span_call_site` | emits a synthesised `MetaSpan { id: 0, hygiene: 0, flags: {…} }` when desugaring `@derive(...)` — pinned by `test_meta_span_synthetic_predicate_true`. |
| `verum_vbc::interpreter::builtins::span` | runtime-side intrinsics for `.call_site()` / `.def_site()` / `.mixed_site()` / `.start()` / `.end()` / `.join()` / `.subspan()` / `.source_text()` / `.resolved_at_call_site()` / `.resolved_at_def_site()` / `.location()` — all return synthesised zero-filled values at runtime (no compile-time registry); the surface is meaningful only during macro expansion. |

## 3. Language-implementation gaps

### §3.1 Cross-module record-return field-access OOB (high)

Every cross-module factory function that returns a record value
fails on the receiver-side `.field` access path:

```text
let loc = SourceLocation.new(Text.from("foo.vr"), 1, 1, 0);
assert(loc.file == "foo.vr");
// runtime panic:
//   field access out of bounds: field index 12 (offset 96+8 = 104)
//   exceeds object data size 32
```

Field index 12 is well beyond SourceLocation's 4 fields — the
codegen has fallen through to the *global intern-id fallback* for
the field name `file`. The record object is sized correctly (32
bytes = 4 × 8) but the field-resolution path discards the type and
defaults to a synthesised offset based on the literal field-name
intern position.

**Defect class** is the same root as `[[enactment_field_access_oob_2026-05-24]]`
(action/gauge tests, `let c = canonicalise(e); c.steps.len()`)
and `[[btree_pattern_match_ref_generic_class]]` (pattern-match
through-ref/through-Heap loses generic-record-arg type).

**Pinned regressions:**
- `regression_source_location_new_cross_module_return`
- `regression_span_range_new_cross_module_return`
- `regression_span_range_single_cross_module_return`
- `regression_multi_span_empty_cross_module_return`
- `regression_multi_span_from_span_cross_module_return`

All `@ignore`d until the cross-module fn-return path preserves
type layout through `compile_field_access` (verum_vbc/src/codegen/expressions.rs).

**Effort:** multi-day VBC codegen work — same fix unlocks ~30
sister regressions across `core.action`, `core.btree`, and the
meta-system as a whole.

**Workaround discipline pinned in `unit_test.vr` header:** use
**direct record construction** (`SourceLocation { file: ..., ... }`)
at the call site, never the cross-module `.new(...)` ctor, until
the defect closes.

### §3.2 Compiler-intrinsic ctors return zero-filled at runtime (low)

`Span.call_site()` / `Span.def_site()` / `Span.mixed_site()` /
`Span.synthetic()` are `@compiler_intrinsic`. At runtime (Tier 0
interpreter) they return `MetaSpan { id: 0, hygiene: 0, flags: {…} }`
because there is no live span registry. This is **correct** —
spans are a compile-time concept — but it means runtime tests
of `.start()` / `.end()` / `.location()` / `.source_text()` are
not meaningful and live at the verum_compiler test layer instead.

### §3.3 `SpanRange.to_span()` calls `.join()` which is intrinsic (low)

`SpanRange.to_span()` is non-intrinsic but its body is
`self.start.join(self.end)` — `MetaSpan.join` is intrinsic, so
calling `to_span()` at runtime returns whatever the intrinsic
stub yields (typically a zero-Span). Not testable at this layer.

## Action items landed in this branch

* `core-tests/meta/span/unit_test.vr` — 25 unit tests over
  SpanFlags + MetaSpan + Span alias + SourceLocation + SpanRange +
  MultiSpan via **direct record construction** (works around the
  cross-module fn-return defect).
* `core-tests/meta/span/regression_test.vr` — 5 `@ignore` regressions
  pinning the cross-module ctor return-value field-access OOB defect
  class.
* `core-tests/meta/span/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Cross-module fn-return record layout preservation (§3.1) | verum_vbc/src/codegen/expressions.rs `compile_field_access` + `compile_method_call` | multi-day VBC |
| Live span-registry intrinsics for runtime tests of `.start`/`.end`/`.location` (§3.2) | verum_vbc/src/interpreter/builtins/span.rs | 1 day |
| Property tests for `MultiSpan.iter` once §3.1 closes (every `(MetaSpan, Maybe<Text>)` pair surfaces correctly) | this folder | 30 min |
