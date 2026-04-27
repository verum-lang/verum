# Example 08 — protocols

## What it does

Defines a `Drawable` protocol with two methods, implements it for
`Circle` and `Square`, then writes a generic `report` function
that works for any type implementing `Drawable`.

## What's new

- `type T is protocol { ... };` — protocol declaration (Verum's
  name for trait/interface).
- `implement Protocol for Type { ... }` — implementation block.
- `<S: Protocol>` — generic type parameter with a protocol bound.
- `&S` — borrow of a generic-typed value.

## The code

```verum
type Drawable is protocol {
    fn draw(&self) -> Text;
    fn area(&self) -> Float;
};
```

A protocol declares the methods every implementer must provide.
Method signatures only — no bodies. The `&self` receivers mean
implementers receive their value by shared reference.

```verum
implement Drawable for Circle {
    fn draw(&self) -> Text {
        f"Circle(r={self.radius})"
    }
    fn area(&self) -> Float {
        3.14159 * self.radius * self.radius
    }
}
```

`implement P for T { ... }` provides every method P declares for
type T. The compiler checks that ALL methods are present and have
matching signatures — partial implementations fail to compile.

```verum
fn report<S: Drawable>(s: &S) {
    print(f"  {s.draw()}  area = {s.area()}");
}
```

Generic function bound by `Drawable`. The `<S: Drawable>` syntax
declares a type parameter `S` constrained to types that implement
`Drawable`. The compiler monomorphises `report` per concrete `S`
at compile time — there's no runtime dispatch overhead unless you
use `&dyn Drawable` (dynamic dispatch — covered in a later
example).

`&S` is a borrow of a generic-typed value. Inside `report`,
`s.draw()` resolves to the appropriate impl based on the
concrete type the caller passed.

```verum
let c = Circle { radius: 2.0 };
let s = Square { side: 3.0 };
report(&c);   // monomorphised to report::<Circle>
report(&s);   // monomorphised to report::<Square>
```

## Things to try

1. Add a `Triangle` type with sides `a, b, c` and implement
   `Drawable` for it (use Heron's formula for area).

2. Add a `perimeter(&self) -> Float` method to `Drawable` and
   implement it across all three types.

3. Write a function that takes a `List<&dyn Drawable>` and prints
   every shape's area. Note the runtime-dispatch performance
   difference vs the monomorphised version.

## Reference

- Protocol declarations: `grammar/verum.ebnf` §2.5 (`protocol_def`).
- Implementations: `grammar/verum.ebnf` §2.6 (`impl_block`).
- Generic parameters: `grammar/verum.ebnf` §2.7 (`generics`).
- Protocol bounds: `grammar/verum.ebnf` §2.7 (`type_bound`).
- Dynamic dispatch (`&dyn P`): future example.
