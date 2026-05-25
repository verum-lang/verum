# `meta/quote` audit

Module: `core/meta/quote.vr` (~517 LOC) — quasi-quotation builder
(QuoteBuilder + GroupBuilder + QuotePart) and convenience helpers
on TokenStream.

Tests: 4 unit tests over the QuotePart 3-variant pure-data surface.
**The rest of the QuoteBuilder API is currently un-callable** due to
the `TokenTree.Token` / `TokenKind.Keyword` variant-name drift
documented in [meta/token audit §3.1 / §3.2](../token/audit.md).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.meta.contexts.AstAccess` | macros that emit code do so by building TokenStreams either via `quote { ... }` (compiler-desugared) or `QuoteBuilder` (programmatic) |
| `core.meta.diakrisis_attrs` | not used |
| `verum_compiler::quasi_quote_desugar` | the `quote { ... }` surface lowering — produces calls to `__quote_impl` |

## 2. Crate-side hardcodes

* `verum_compiler::quasi_quote::QuotePart` mirrors the 3-variant
  Literal / Interpolate / Repeat enum.
* `verum_compiler::quasi_quote::__quote_impl` is `@compiler_intrinsic` —
  the actual desugaring target.

## 3. Language-implementation gaps

### §3.1 Variant-name drift blocks every QuoteBuilder API (HIGH)

```verum
self.tokens.push(TokenTree.Token(Token.ident_spanned(name, self.span)));
```

`TokenTree.Token` is not a valid TokenTree variant (correct name:
`TokenTree.Leaf`). Same drift affects `TokenKind.Keyword` (correct
name: `TokenKind.Kw`).

**Fix:** in `core/meta/quote.vr`, replace:
* `TokenTree.Token` → `TokenTree.Leaf` (16 occurrences)
* `TokenKind.Keyword` → `TokenKind.Kw` (3 occurrences)

5-minute fix; immediately unlocks the entire programmatic QuoteBuilder
surface for testing.

### §3.2 `keyword_from_str` is `@compiler_intrinsic`

Even after §3.1, `QuoteBuilder.keyword("let")` calls
`keyword_from_str` which needs the compile-time keyword table.
At runtime it returns a default `Keyword` (probably the first
variant, `Let`). Tests can pin the runtime behaviour as long as
they don't depend on the keyword being correct.

### §3.3 `Group { delimiter, tokens, span }` ordering matches token.vr

Verified — no drift.

### §3.4 Convenience module-level fns (`empty`, `parens`, `braces`, …)
depend on TokenStream cross-module ctors

`quote.empty()` calls `TokenStream.empty()` which calls
`Span.call_site()` — runs into the cross-module fn-return defect
(see [meta/span audit §3.1](../span/audit.md)). Not exercised here.

## Action items landed in this branch

* `core-tests/meta/quote/unit_test.vr` — 4 unit tests over QuotePart
  3-variant + variant-disjointness.
* `core-tests/meta/quote/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Rename TokenTree.Token → TokenTree.Leaf in quote.vr (§3.1) | core/meta/quote.vr | 5 min |
| Rename TokenKind.Keyword → TokenKind.Kw in quote.vr (§3.1) | core/meta/quote.vr | 5 min |
| Full QuoteBuilder API tests post-§3.1 (chain ident/keyword/punct/lit/group/build, 30+ tests) | this folder | 2 h |
| Integration test: QuoteBuilder → TokenStream → Display round-trip | this folder | 1 h |
| Convenience fn tests post-cross-module-fix | this folder | 30 min |
| Property tests for `QuotePart`-list-equality after macro expansion | this folder | 1 h |
