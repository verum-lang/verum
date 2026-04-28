# 18 — Text Processing

`Text` is Verum's UTF-8-native string type. Every Text value carries
a documented UTF-8 invariant — there's no "byte buffer that might be
text" alternative on the safe path.

## The byte-vs-char distinction

| Operation | Returns |
|---|---|
| `text.len()` | Byte length |
| `text.chars().count()` | Code-point count |
| `text.slice(start, end)` | Byte-indexed slice (panics on non-boundary) |
| `text.chars()` | Iterator over Char (UTF-8 code points) |

Slicing by byte offset is the explicit choice — it makes O(1) slicing
possible and the panic-on-non-boundary contract makes the safety
invariant visible. Char-indexed slicing is `text.chars().take(n).collect()`.

## TextBuilder — what to use instead of `+`

```verum
// AVOID — O(N²) copying:
let mut s = "".clone();
for x in items.iter() { s = s + &x.to_text(); }

// PREFER — amortised O(N):
let mut b = TextBuilder.new();
for x in items.iter() { b.push(&x.to_text()); }
let s = b.into_text();
```

`TextBuilder` is a single growing buffer; `+` between two `Text`
values allocates a fresh Text each call. The difference is benign
for ≤ 5 concatenations, catastrophic for 10k.

## Format literals

`f"..."` is a compile-time-parsed format string with type-checked
interpolations. The format spec follows Python:

```verum
f"{n:03}"      // zero-pad to 3 digits
f"{x:.2}"      // 2 decimal places
f"{n:x}"       // lowercase hex
f"{n:X}"       // uppercase hex
f"{n:b}"       // binary
f"{s:>10}"     // right-align in 10-char field
```

## What's NOT in the safe path

`Text.from_utf8_unchecked` and `as_bytes_mut` exist but require
`unsafe`. They're for FFI / parser-internal usage where the caller
maintains the UTF-8 invariant manually. Most code never needs them.
