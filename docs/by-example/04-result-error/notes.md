# Example 04 — result-error

## What it does

Defines a typed error sum, parses a text input through two
fallible functions, and uses `?` to propagate errors out of the
caller without explicit `match` on every step.

## What's new

- `Result<T, E>` — built-in sum type for fallible computations.
  `Ok(T)` for success, `Err(E)` for failure.
- `Maybe<T>` — built-in sum type for optional values. `Some(T)` /
  `None`. (Used implicitly via `s.to_int()`.)
- `?` — error-propagation operator. `expr?` evaluates to the inner
  `Ok` value or short-circuits the enclosing function with the
  inner `Err`.
- `&Text` — borrowed reference. Avoids moving the string into the
  function.
- `return Err(...)` — explicit return.

## The code

```verum
type ParseError is
    | EmptyInput
    | NotANumber(Text)
    | OutOfRange(Int);
```

A typed error — every failure mode is its own variant carrying
the data needed to diagnose it. This is preferable to a single
`Err(Text)` with a stringly-typed message because:

- Callers can match on the kind without parsing strings.
- Adding a new failure mode is a compile error in every match
  that exhaustively handles `ParseError`, surfacing missing
  cases immediately.

```verum
fn parse_positive(s: &Text) -> Result<Int, ParseError> {
    ...
}
```

Returns `Result<Int, ParseError>`. Two construction sites:
`Err(ParseError.<Variant>)` for failures, `Ok(value)` for the
single happy path at the bottom.

```verum
let n = match s.to_int() {
    Some(v) => v,
    None    => return Err(ParseError.NotANumber(s.clone())),
};
```

`s.to_int()` returns `Maybe<Int>`. The match binds the inner
value if `Some`, or short-circuits the function with an `Err`
return if `None`.

```verum
fn double_positive(s: &Text) -> Result<Int, ParseError> {
    let n = parse_positive(s)?;
    Ok(n * 2)
}
```

`expr?` is sugar for the same match-and-early-return pattern.
When `parse_positive(s)` returns `Err(e)`, `?` returns
`Err(e)` from `double_positive`. When it returns `Ok(n)`, `?`
unwraps to `n` and execution continues.

`?` requires the calling function's return type be `Result<_, E>`
where `E` matches (or is convertible from) the inner error.

```verum
match double_positive(s) {
    Ok(n)                          => ...,
    Err(ParseError.EmptyInput)     => ...,
    Err(ParseError.NotANumber(t))  => ...,
    Err(ParseError.OutOfRange(n))  => ...,
}
```

Pattern matching unwraps `Result` AND the nested `ParseError`
variant in a single arm — no explicit `match err { ... }` needed.

## Things to try

1. Add a `LeadingZero` variant for inputs like `"007"` and reject
   them in `parse_positive`.

2. Replace the explicit `match` in `parse_positive` with a chained
   call:

   ```verum
   let n = s.to_int().ok_or(ParseError.NotANumber(s.clone()))?;
   ```

   `Maybe.ok_or(err)` converts `Some(v) -> Ok(v)` and `None -> Err(err)`.

3. Make the function generic over the kind of `Int`:

   ```verum
   fn parse_positive<I: FromStr + Ord>(s: &Text) -> Result<I, ParseError> {
       ...
   }
   ```

   (You'll need to pick `I.zero()` for the zero-comparison; this
   touches the `Numeric` protocol covered in a later example.)

## Reference

- `Result<T, E>`: `core/base/result.vr`.
- `Maybe<T>`: `core/base/maybe.vr`.
- `?` operator: `grammar/verum.ebnf` §4.6 (`try_op`).
- Reference parameters (`&T`): `grammar/verum.ebnf` §2.4
  (`reference_type`).
- Nested-pattern matching: `grammar/verum.ebnf` §3.4
  (`enum_pattern`).
