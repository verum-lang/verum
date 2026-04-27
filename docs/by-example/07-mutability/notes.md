# Example 07 — mutability

## What it does

Introduces `let mut`, `&self` / `&mut self` method receivers, and
the `&checked self` Tier-1 reference. Builds a tiny `Counter` type
to demonstrate.

## What's new

- `let mut name = ...;` — mutable binding. Without `mut`,
  reassignment is a compile error.
- `&self` — shared/read-only borrow. Multiple may exist
  simultaneously.
- `&mut self` — exclusive/mutable borrow. Only one alive at a time.
- `&checked self` — Tier-1 borrow (compiler-proven safe; zero CBGR
  overhead at runtime).
- `implement T { ... }` — method-impl block.
- `public fn` — visibility modifier.

## The code

```verum
type Counter is { value: Int };
```

Standard record type — covered in example 02.

```verum
implement Counter {
    public fn increment(&mut self) {
        self.value = self.value + 1;
    }
    ...
}
```

`implement T { ... }` defines methods on type `T`. Inside, `self`
refers to the receiver. The receiver kind controls what the method
can do:

| Receiver       | Tier | Borrow kind         | Allowed                       |
| -------------- | ---- | ------------------- | ----------------------------- |
| `&self`        | 0    | shared              | read fields, call &self / &checked methods |
| `&mut self`    | 0    | exclusive           | read + write fields           |
| `&checked self`| 1    | shared (proven)     | read; zero runtime overhead   |
| `&unsafe self` | 2    | shared (unsafe)     | read; manual safety proof     |

Tier 0 is the default — the compiler inserts CBGR (Counter-Based
Generation Reference) checks at every dereference (~15 ns). Tier 1
is selected when escape analysis proves the borrow can't outlive
the borrowed value — checks compile away. Tier 2 opts out of
checks entirely; the caller proves safety manually.

```verum
let counter = Counter.new();
print(f"... = {counter.current()}");
```

Immutable binding. `counter.increment()` would fail with "cannot
mutate immutable binding" because `&mut self` requires `mut` on
the binding.

```verum
let mut c = Counter.new();
c.increment();
```

Mutable binding. The `mut` is on the binding (the variable), not
on the type. `Counter` itself is the same type either way; `mut`
controls what the BORROWS can do.

## Things to try

1. Try calling `counter.increment()` on the immutable `counter` —
   read the compile error.

2. Add a `decrement(&mut self)` method.

3. Add a `combined(&self, other: &Counter) -> Int` method that
   sums two counters' values. Note that `other` is also `&Counter`,
   not `&mut Counter` — both bindings are shared-borrowed
   simultaneously.

## Reference

- Mutability: `grammar/verum.ebnf` §3.1 (`let_stmt`).
- `implement` blocks: `grammar/verum.ebnf` §2.6 (`impl_block`).
- Self receivers: `grammar/verum.ebnf` §2.6 (`self_param`).
- Reference tiers: `docs/detailed/26-cbgr-implementation.md`.
- Visibility (`public`): `grammar/verum.ebnf` §2 (`visibility`).
