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

### §3.1 Variant-name drift blocks every QuoteBuilder API — CLOSED 2026-05-25

Originally documented: `core/meta/quote.vr` referenced
`TokenTree.Token(...)` (correct: `Leaf`), `TokenKind.Keyword(kw)`
(correct: `Kw`), and `TokenTree.Group(g)` (correct: `Grouped`).
All three are mechanical typos against the canonical variant
names declared in `core/meta/token.vr`.

**Closed by rewriting all 18 drift sites**:

* `TokenTree.Token(...)` → `TokenTree.Leaf(...)` (13 sites:
  `ident`/`keyword`/`punct_joint`/`punct`/`operator`/`int_lit`/
  `float_lit`/`string_lit`/`char_lit`/`bool_lit` in QuoteBuilder
  + `ident`/`punct`/`keyword` in GroupBuilder)
* `TokenKind.Keyword(...)` → `TokenKind.Kw(...)` (2 sites:
  QuoteBuilder.keyword + GroupBuilder.keyword)
* `TokenTree.Group(...)` → `TokenTree.Grouped(...)` (2 sites:
  QuoteBuilder.group + GroupBuilder.close)

Verified clear via
`grep "TokenTree\.Token\|TokenKind\.Keyword\|TokenTree\.Group\b" core/meta/quote.vr`.

QuoteBuilder API surface is now syntactically valid; runtime
testing still gated on §3.2 (`keyword_from_str`
`@compiler_intrinsic`) and §3.4 (TokenStream cross-module ctors).

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

* `core/meta/quote.vr` — closed §3.1 (18 drift sites realigned).
* `core-tests/meta/quote/unit_test.vr` — 4 unit tests over QuotePart
  3-variant + variant-disjointness.
* `core-tests/meta/quote/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Full QuoteBuilder API tests (chain ident/keyword/punct/lit/group/build, 30+ tests) post-(§3.2+§3.4) cross-module-ctor fix | this folder | 2 h |
| Integration test: QuoteBuilder → TokenStream → Display round-trip | this folder | 1 h |
| Convenience fn tests post-cross-module-fix | this folder | 30 min |
| Property tests for `QuotePart`-list-equality after macro expansion | this folder | 1 h |
