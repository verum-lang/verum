# `meta/token` audit

Module: `core/meta/token.vr` (~826 LOC) — `TokenStream`, `Token`,
`TokenTree`, `TokenKind`, `Delimiter`, `Spacing`, `Keyword`,
`Literal`, `StringKind`, `Group`, `LexError`.

Tests: 56 unit tests over the pure-data subset that does NOT call
`Span.call_site()` or `TokenStream.from_str` at runtime.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.meta.attribute` | `Attribute.tokens: TokenStream` carries the raw arg tokens |
| `core.meta.quote` | `QuoteBuilder` constructs `TokenStream` programmatically |
| `core.meta.contexts.AstAccess` | every parser fn accepts `TokenStream` |
| `core.meta.diakrisis_attrs` | not used directly |
| `verum_lexer::Token` | parser-side mirror of `TokenKind` |

## 2. Crate-side hardcodes

* `verum_lexer::token::TokenKind` mirrors the 6-variant token kind
  (Ident / Kw / Lit / Punct / DocComment / Eof). MUST stay in sync.
* `verum_lexer::token::Delimiter` mirrors the 4-variant delimiter.
* `verum_lexer::token::Keyword` mirrors the ~38-variant keyword
  enum. Drift here is high-risk: a missing keyword at the AST
  side silently lexes as `Ident`.
* `verum_lexer::token::Literal` mirrors the 6-variant literal
  payload + the 4-variant `StringKind`.
* `verum_lexer::token::Spacing` mirrors the 2-variant spacing
  (`Joint` / `Alone`).

## 3. Language-implementation gaps

### §3.1 Variant-name drift: `TokenTree.Leaf` vs `TokenTree.Token` — CLOSED 2026-05-25

Closed by realigning `core/meta/quote.vr` (13 sites).
See [meta/quote audit §3.1](../quote/audit.md).

### §3.2 Variant-name drift: `TokenKind.Keyword` vs `TokenKind.Kw` — CLOSED 2026-05-25

Closed by realigning `core/meta/quote.vr` (2 sites).
Plus one additional drift uncovered: `TokenTree.Group` → `Grouped`
(2 sites). See [meta/quote audit §3.1](../quote/audit.md).

### §3.3 `Group { delimiter, tokens, span }` field order in TokenStream.wrap

`TokenStream.wrap` builds:

```verum
TokenTree.Grouped(Group {
    delimiter,
    tokens: self,
    span: self.span,
})
```

But the `Group` type declares fields in order
`{ delimiter, tokens, span }` — matches. No drift here; pinned by
`test_group_record_construction_parenthesis`.

### §3.4 `TokenStream.from_str` is intrinsic and not runtime-callable

The lexer-as-meta-fn is `@compiler_intrinsic` — it needs the
verum_lexer crate. At runtime, calling it would either return an
empty `TokenStream` or panic. Not exercised at this layer; lives
in `verum_lexer::tests` instead.

### §3.5 `Token` ctors and `TokenStream.empty()` call `Span.call_site()` internally

This makes the ctors un-callable across modules at runtime due to
the cross-module fn-return record-layout defect (see
[meta/span audit §3.1](../span/audit.md)). All token tests in this
folder construct via **direct record literals** as a result.

Pinned regressions for the ctor paths could be added; deferred
until the cross-module fix lands so the regression-set isn't
artificially inflated.

### §3.6 META-GROUP-XMODULE-1 — cross-module simple-name type collision — CLOSED 2026-07-06

`Group { delimiter, tokens, span }` (8 unit tests) crashed with
`field write out of bounds: field index 3 … data size 24
type='Group'`. Root cause chain (language level, systemic):

1. VBC codegen's type registries (`type_name_to_id`,
   `type_field_layouts`, `type_field_type_names`) were keyed by
   **simple type name** with first-wins across every loaded archive
   module.
2. The reachability-driven archive loader pulls `core.math` (protocol
   `Group`) before `core.meta` for these tests; the protocol-stub
   pass claimed the name `Group` with an EMPTY-fields descriptor.
3. `import_archive_type_with_protocol_remap` then **silently
   dropped** `meta.token.Group` (and `cli.spec.Group`) — bail on
   "name already has a descriptor".
4. Record literals of the dropped type allocated with the literal's
   own field count but resolved field indices through the
   global-intern fallback (`tokens`→3, `span`→15) → out-of-bounds
   `SetF`. Which module won was **load-order dependent** — adding
   test files to the folder shifted the composition and flipped the
   failure (observed 2026-07-06).

Fundamental fix (verum_vbc): module-qualified registry keys
(`"core.meta.Group"`) registered unconditionally alongside the
first-wins simple key; mount-aware re-keying
(`resolve_record_type_key`, driven by a new
`CodegenContext.mounted_types` populated from `mount` decls);
qualified-first type-id remap in `merge_archive_function_bodies`;
benign-homonym downgrade in the type-table health checker (same
simple name × different ids is the designed state when every id has
its own qualified key). Pinned by
`core-tests/meta/token/regression_test.vr` — note the pin is a
**canary**, not an order-forcing pin: validate across multiple suite
compositions.

## Action items landed in this branch

* `core-tests/meta/token/unit_test.vr` — 56 unit tests over
  Delimiter 4-variant + .open/.close + Spacing 2-variant +
  Keyword 16-of-38 representative + StringKind 4-variant +
  TokenKind 6-variant + Literal 6-variant + Literal ctors +
  Token record + TokenTree 2-variant + TokenStream record +
  Group record + LexError record.
* `core-tests/meta/token/property_test.vr` — 25 law tests
  (Delimiter open/close pairing, Literal ctor→variant matrix,
  Token ctor/predicate coherence, TokenKind 6-variant partition).
* `core-tests/meta/token/regression_test.vr` — 4 canary pins for
  META-GROUP-XMODULE-1 (§3.6).
* `core-tests/meta/token/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Regression-pin Token ctor cross-module paths (§3.5) once cross-module fix lands | this folder | 30 min |
| Integration test: build TokenStream via QuoteBuilder + assert round-trip with Display | this folder | 1 h |
| Drift-pinning Rust unit test mirroring Keyword/Delimiter/Literal/TokenKind enums | crates/verum_lexer/src/tests/ | 1 h |
| Property test: `Spacing` 2-valued domain + `Delimiter.open` / `.close` returns matched pair | this folder | 30 min |
