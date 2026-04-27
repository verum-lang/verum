# Example 03 — pattern-match

## What it does

Defines a `Shape` sum type with three variants, computes each
variant's area in a `match` expression, and demonstrates pattern
guards (`Rectangle(w, h) if w == h`).

## What's new

- `type T is | Variant1(T1) | Variant2(T2, T3);` — sum-type
  declaration (the equivalent of Rust's `enum`).
- `match expr { pattern => result, ... }` — exhaustive pattern
  matching.
- `_` — wildcard pattern (matches but binds nothing).
- `if cond` after a pattern — pattern guards.
- `for x in collection { ... }` — iteration loop.
- `[a, b, c]` — array literal.
- `fn name(param: T) -> R { ... }` — explicit return type.

## The code

```verum
type Shape is
    | Circle(Float)
    | Rectangle(Float, Float)
    | Square(Float);
```

A sum type with three variants. Each variant carries data: a
single `Float` for `Circle` and `Square`, two `Float`s for
`Rectangle`. The `|` is read "or" — a `Shape` is *either* a
Circle, *or* a Rectangle, *or* a Square.

```verum
match s {
    Circle(r) => 3.14159 * r * r,
    Rectangle(w, h) => w * h,
    Square(side) => side * side,
}
```

`match` runs the first arm whose pattern matches `s`. Each arm
binds the variant's data (`r`, `(w, h)`, `side`) and evaluates an
expression to produce the match's result. Patterns are exhaustive
— the compiler refuses to compile a match that doesn't cover
every variant.

```verum
match s {
    Circle(_) => "a circle".clone(),
    Rectangle(w, h) if w == h => "a square ...".clone(),
    Rectangle(_, _) => "a rectangle".clone(),
    Square(_) => "a square".clone(),
}
```

Pattern guards (`if w == h`) refine a match. They run only when
the pattern itself matches; an unmatched guard falls through to
the next arm. Exhaustiveness is checked AS IF guards weren't
there — i.e. you still need a `Rectangle(_, _)` arm without a
guard, even if the guarded `Rectangle(w, h) if w == h` arm covers
the equal-side case.

`_` ignores the bound value. Use it when you only care that the
variant matched, not what it carries.

```verum
let shapes = [
    Circle(2.0),
    Rectangle(3.0, 4.0),
    Square(5.0),
    Rectangle(2.5, 2.5),
];

for s in shapes {
    print(f"{describe(s)} has area {area(s)}");
}
```

`[a, b, c]` is an array literal. The for-in loop iterates each
element and binds it to `s`.

## Things to try

1. Add a `Triangle(Float, Float, Float)` variant (sides a, b, c)
   and use Heron's formula to compute its area.

2. Try removing the `Square(_) => ...` arm. The compiler should
   reject the match as non-exhaustive — read the error message.

3. Replace the four-arm `describe` with two arms that share a
   common sub-pattern using the `or` pattern syntax (look ahead
   to example 04 if you haven't seen it).

## Reference

- `match` expressions: `grammar/verum.ebnf` §4.6.
- Sum types: `grammar/verum.ebnf` §2.5 (`variant_list`).
- Pattern guards: `grammar/verum.ebnf` §4.6 (`match_arm`).
- `for-in` loops: `grammar/verum.ebnf` §4.5.
- Array literals: `grammar/verum.ebnf` §3.5.
