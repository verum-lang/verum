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

### §3.1 Variant-name drift: `TokenTree.Leaf` vs `TokenTree.Token` (medium)

`token.vr` defines:

```verum
public type TokenTree is
    | Leaf(Token)
    | Grouped(Group);
```

But `quote.vr` references:

```verum
self.tokens.push(TokenTree.Token(Token.ident_spanned(name, self.span)));
```

`TokenTree.Token` is **not a variant** of TokenTree — it's a name
that aliases the `Token` *type* under the wrong namespace. This
mismatch silently disables `QuoteBuilder.ident` /
`QuoteBuilder.punct` / `QuoteBuilder.int_lit` / etc. at compile
time (the `.push` call cannot resolve).

**Fix path (5 min):** rename `TokenTree.Token` → `TokenTree.Leaf`
in `core/meta/quote.vr` (16 occurrences).

### §3.2 Variant-name drift: `TokenKind.Keyword` vs `TokenKind.Kw` (medium)

`token.vr` defines:

```verum
public type TokenKind is
    | Ident(Text)
    | Kw(Keyword)
    | ...
```

But `quote.vr` references `TokenKind.Keyword(keyword_from_str(kw))`.
Same fix-class as §3.1; rename to `TokenKind.Kw`.

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

## Action items landed in this branch

* `core-tests/meta/token/unit_test.vr` — 56 unit tests over
  Delimiter 4-variant + .open/.close + Spacing 2-variant +
  Keyword 16-of-38 representative + StringKind 4-variant +
  TokenKind 6-variant + Literal 6-variant + Literal ctors +
  Token record + TokenTree 2-variant + TokenStream record +
  Group record + LexError record.
* `core-tests/meta/token/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Rename `TokenTree.Token` → `TokenTree.Leaf` in quote.vr (§3.1) | core/meta/quote.vr | 5 min |
| Rename `TokenKind.Keyword` → `TokenKind.Kw` in quote.vr (§3.2) | core/meta/quote.vr | 5 min |
| Regression-pin Token ctor cross-module paths (§3.5) once cross-module fix lands | this folder | 30 min |
| Integration test: build TokenStream via QuoteBuilder + assert round-trip with Display | this folder | 1 h post-§3.1 |
| Drift-pinning Rust unit test mirroring Keyword/Delimiter/Literal/TokenKind enums | crates/verum_lexer/src/tests/ | 1 h |
| Property test: `Spacing` 2-valued domain + `Delimiter.open` / `.close` returns matched pair | this folder | 30 min |
