# Example 05 — collections

## What it does

Builds a `List<Int>`, sums it, builds a `Map<Text, Int>` of word
counts, and shows a small functional-style transform via
`iter().map(...).collect()`.

## What's new

- `mount core.collections.{List, Map};` — the `mount` system.
  Verum doesn't auto-import everything; you opt in to the symbols
  you want.
- `List<T>` — semantic name for "growable array". (Other languages
  call this `Vec`, `ArrayList`, or `vector`.)
- `Map<K, V>` — semantic name for "associative key→value
  mapping". (Other languages call this `HashMap`, `dict`, or
  `unordered_map`.)
- `let mut name: Type = ...;` — mutable binding.
- `value.method()` chains and the `Iterator` protocol.
- `|x| expr` — closure literal.
- `for (k, v) in &map { ... }` — destructuring iteration.

## The code

```verum
mount core.collections.{List, Map};
```

Brings `List` and `Map` into scope. `mount` is Verum's import
mechanism — all dependencies are explicit, no transitive
auto-imports. Pattern: `mount module.path.{Symbol1, Symbol2, ...};`.

```verum
let mut nums: List<Int> = List.new();
nums.push(1);
nums.push(2);
```

`let mut` declares a mutable binding. Without `mut`, `nums.push(1)`
would fail to compile because `push` mutates the List.

`List.new()` is the standard constructor. `List.with_capacity(n)`
exists for performance when you know the size in advance — it
pre-allocates the backing buffer so subsequent `push` calls don't
re-allocate.

```verum
for n in nums {
    total = total + n;
}
```

`for-in` loops iterate. By value here — each `n` is an owned
`Int` because `Int` is a `Copy` type. For a `List<NonCopyType>`,
write `for n in &nums { ... }` to iterate by reference.

```verum
let prev = match counts.get(&w.clone()) {
    Some(n) => *n,
    None    => 0,
};
counts.insert(w.clone(), prev + 1);
```

`Map.get(&K)` returns `Maybe<&V>` (a reference into the map).
`*n` dereferences the borrowed `&Int` into an owned `Int`. The
`w.clone()` creates an owned `Text` because `insert` consumes
the key.

```verum
for (word, n) in &counts {
    print(f"  {word} -> {n}");
}
```

`for (k, v) in &map` iterates by reference, destructuring each
`(K, V)` entry into named bindings. Iteration order is
insertion-order (Verum's `Map` preserves it; this is a documented
contract, unlike Rust's `HashMap`).

```verum
let doubled: List<Int> = nums.iter().map(|n| n * 2).collect();
```

`nums.iter()` produces an iterator. `.map(|n| n * 2)` lazily
transforms each element. `.collect()` materialises the iterator
back into a `List<Int>`.

`|n| n * 2` is a closure — an inline anonymous function. The
parameter list is between the `|` bars; the body is the expression
that follows.

## Things to try

1. Replace `match counts.get(&w.clone()) { ... }` with the
   one-line `entry`-style API when it ships.

2. Build a `Map<Text, List<Text>>` — list of authors per year — and
   iterate it.

3. Use `nums.iter().filter(|n| **n > 1).sum()` to sum only
   elements > 1. (`filter` takes a predicate; `sum` is the
   standard fold.)

## Reference

- `mount` system: `grammar/verum.ebnf` §2 (`mount_stmt`).
- `List<T>`: `core/collections/list.vr`.
- `Map<K, V>`: `core/collections/map.vr`.
- Iterator protocol: `core/base/iterator.vr`.
- Closure syntax: `grammar/verum.ebnf` §3.6 (`closure_expr`).
- `for-in`: `grammar/verum.ebnf` §4.5.
