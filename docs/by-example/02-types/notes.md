# Example 02 — types

## What it does

Declares variables of every primitive type, defines a 2D `Point`
record, and prints them all using f-strings.

## What's new

- `let name: Type = value;` — variable binding with explicit type.
- `Int`, `Float`, `Text`, `Bool` — built-in primitive types
  (semantic names, not implementation hints).
- `type Name is { field: Type, ... };` — record-type declaration
  (the equivalent of Rust's `struct`).
- `Type { field: value, ... }` — record-literal construction.
- `value.field` — field access.

## The code

```verum
type Point is {
    x: Int,
    y: Int,
};
```

`type T is is { ... };` declares a record type named `T`. Verum
uses `is` rather than `=` so the same keyword carries through to
sum types (`type Foo is A | B`) and protocols (`type T is protocol
{ ... }`).

```verum
let n: Int = 42;
```

`let name: Type = value;` declares an immutable binding. The type
annotation is optional for literals (Verum infers `Int` from `42`)
but spelled out here for clarity.

`Int` is the semantic name for "machine-word integer" — what
languages like Rust call `i64` or C calls `intptr_t`. Verum uses
semantic names so the type tells you what the value MEANS, not
what its bit-layout is.

```verum
let origin = Point { x: 0, y: 0 };
```

Record literals use `TypeName { field: value }` syntax. Field
order doesn't matter: `Point { y: 0, x: 0 }` is the same value.

```verum
print(f"origin = ({origin.x}, {origin.y})");
```

f-strings interpolate expressions inside `{...}`. `origin.x`
accesses the `x` field of the record.

## Things to try

1. Declare a `Color` record with three `UInt8` fields (`r`, `g`,
   `b`) and print one.

2. Make `n` mutable — change `let n` to `let mut n` — and reassign
   it before printing:

   ```verum
   let mut n: Int = 42;
   n = n + 1;
   ```

3. Add a `distance_from_origin` field of type `Float` to `Point`
   and compute it from `x` and `y` (you'll need `core.math.sqrt`;
   look ahead to example 05).

## Reference

- Built-in types: `core/base/primitives.vr`.
- `let` bindings: `grammar/verum.ebnf` §3.1.
- `type` declarations: `grammar/verum.ebnf` §2.5.
- Record literals: `grammar/verum.ebnf` §3.5.
- f-string interpolation: `grammar/verum.ebnf` §1.5.3.
