# Example 06 — functions

## What it does

Demonstrates the four function forms Verum supports: standard
block-bodied, expression-bodied, default-parameter, and
Unit-returning. Each is called with various argument patterns.

## What's new

- `fn name(p: T) -> R { ... }` — block body.
- `fn name(p: T) -> R = expr;` — expression body.
- `fn name(p: T = default) -> R { ... }` — default parameter values.
- `fn name(p: T) { ... }` — implicit Unit return type.
- Trailing-default argument omission at call sites.

## The code

```verum
fn add(a: Int, b: Int) -> Int {
    a + b
}
```

Block-bodied function. The last expression in the block (without
a trailing `;`) is the return value — `a + b` here. No `return`
needed.

```verum
fn double(x: Int) -> Int = x * 2;
```

Expression-bodied function. Single-expression bodies don't need
`{}` — the `= expr;` form is equivalent and slightly less noisy
for one-liners. Both forms produce identical bytecode.

```verum
fn greet(name: Text = "World", excited: Bool = false) -> Text {
    ...
}
```

Default parameter values. The compiler synthesises overloads for
each suffix-default combination, so callers can omit trailing
arguments. Defaults are evaluated at call time, not at function
definition time (unlike Python).

```verum
fn announce(msg: Text) {
    print(f"[ANNOUNCE] {msg}");
}
```

No explicit `-> R` means the function returns `Unit` (the empty
tuple, written `()`). Useful for side-effecting helpers.

## Things to try

1. Add a `triple` expression-bodied function and use it in `main`.

2. Make `add` work for any numeric type using a protocol bound (look
   ahead to example 09 for generics).

3. Add a default to `add`'s second parameter (`b: Int = 1`) and call
   it with one argument.

## Reference

- Function declarations: `grammar/verum.ebnf` §2.1 (`function_def`).
- Expression bodies: `grammar/verum.ebnf` §2.1 (`function_body`).
- Default parameters: `grammar/verum.ebnf` §2.1 (`function_param`).
- Unit type (`()`): `core/base/primitives.vr`.
