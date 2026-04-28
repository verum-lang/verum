# 21 — Macros

Verum's macro system uses `@`-prefixed names — never `!` suffix
(unlike Rust's `println!`). Every compile-time construct is `@name`:

```verum
@derive(Eq)         // user-facing — synthesise impls
@const              // user-facing — compile-time evaluate
@cfg(...)           // user-facing — conditional compilation
@inline(always)     // user-facing — inlining hint
@intrinsic("...")   // stdlib-internal — bind to compiler intrinsic
@verify(...)        // stdlib-internal — assert verification properties
```

## Built-in functions vs macros

These are FUNCTIONS, not macros (no `@`):

| Function | Purpose |
|---|---|
| `print(s)` | Write to stdout |
| `println(s)` | Write + newline |
| `panic(msg)` | Abort with message |
| `assert(cond)` | Runtime assertion |
| `unreachable()` | Mark unreachable branch |

Macros (with `@`) are compile-time. The two cannot be confused —
the syntax keeps them apart.

## Hygiene

User-defined `@!my_macro(...)` macros are **hygienic**: identifiers
introduced inside the macro body don't collide with identifiers in
the call site. This rules out a whole class of C-preprocessor
breakage (`#define max(a, b) ...` aliasing user-named `a` / `b`).

## Format literals are NOT macros

`f"x = {x}"` is a literal kind, not a macro. The compiler parses the
template, generates the equivalent of `Text.format(...)` automatically,
and type-checks every `{expr}` against the surrounding scope. No
runtime parser, no dynamic dispatch.

## Common derive impls

| Derived | Generated method(s) |
|---|---|
| `Eq` | `eq`, `ne`, `==` operator |
| `Ord` | `cmp`, `<` `<=` `>` `>=` operators |
| `Hash` | `hash<H>(&self, h: &mut H)` |
| `Debug` | `fmt_debug` (used by `{:?}`) |
| `Clone` | `clone(&self) -> Self` |
| `Default` | `default() -> Self` |

`@derive(Eq, Hash, Debug)` is the most common combo for value types
that go into `Map` / `Set` / log lines.
