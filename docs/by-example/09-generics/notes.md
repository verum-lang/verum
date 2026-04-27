# Example 09 — generics

## What it does

Defines a `Pair<A, B>` generic record + a generic `lookup` function
that searches a list of pairs by key, returning a borrowed
reference to the value when found.

## What's new

- `type T<A, B> is { ... };` — generic record with multiple type
  parameters.
- `implement<A, B> T<A, B> { ... }` — generic implementation block.
- `K: Eq` bound — protocol constraint on a type parameter.
- `Maybe<&V>` — optional borrowed reference.
- Multi-parameter generics + bounds.

## The code

```verum
type Pair<A, B> is { first: A, second: B };
```

`A` and `B` are type parameters — placeholders the compiler
replaces with concrete types at each use site. `Pair<Int, Text>`
is a different type from `Pair<Text, Int>`.

```verum
implement<A, B> Pair<A, B> {
    public fn new(a: A, b: B) -> Pair<A, B> {
        Pair { first: a, second: b }
    }
}
```

`implement<A, B>` declares the type parameters fresh in this
block. The block defines methods on `Pair<A, B>` for ALL choices
of `A, B` — the compiler monomorphises per concrete instantiation.

```verum
fn lookup<K: Eq, V>(items: &List<Pair<K, V>>, key: &K) -> Maybe<&V> {
    for item in items {
        if &item.first == key {
            return Some(&item.second);
        }
    }
    None
}
```

Two type parameters: `K` (key) and `V` (value). `K: Eq` requires
keys to support equality comparison — `==` would fail for types
not implementing `Eq`. `V` has no bound — the function works for
any value type.

`Maybe<&V>` is the return type. `Some(&item.second)` borrows the
value out of the matching pair. The borrow's lifetime is tied to
the `&List<Pair<K,V>>` parameter — the caller can't outlive the
list.

```verum
entries.push(Pair.new("alpha".clone(), 1));
```

Type inference resolves `Pair.new("alpha", 1)` to
`Pair<Text, Int>` from the literal types. The `.clone()` is
because `Text.push` consumes the string (Verum doesn't copy `Text`
implicitly — semantic-honesty rule).

## Things to try

1. Add a `Triple<A, B, C>` type and a `lookup3` function that
   matches on the first field.

2. Constrain the value type too: `lookup<K: Eq, V: Clone>` and
   return `Maybe<V>` (cloned) instead of `Maybe<&V>`.

3. Write a generic `swap<A, B>(p: Pair<A, B>) -> Pair<B, A>`.

## Reference

- Generic types: `grammar/verum.ebnf` §2.7 (`generics`).
- Type parameters: `grammar/verum.ebnf` §2.7 (`generic_param`).
- Protocol bounds: `grammar/verum.ebnf` §2.7 (`type_bound`).
- `Eq` protocol: `core/base/protocols.vr`.
- `Maybe<T>`: `core/base/maybe.vr`.
