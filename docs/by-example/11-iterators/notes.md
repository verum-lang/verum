# 11 — Iterators

Iterators are Verum's universal traversal abstraction: a value that
yields a sequence of items one at a time, with composable transforms
that build pipelines and only run on demand.

## The two-phase model

1. **Build phase** — `.iter()`, `.filter()`, `.map()`, `.take()`,
   `.skip()`, `.zip()`, `.enumerate()`, etc. compose lazily; no
   work happens yet.
2. **Consume phase** — `.collect()`, `.sum()`, `.count()`,
   `.fold()`, `.for_each()`, `for x in iter { ... }` drive the
   pipeline and produce a result.

## Why lazy

Lazy pipelines avoid allocating intermediate Lists between every
transform. `nums.iter().map(f).filter(g).take(n)` walks `nums` once
and stops as soon as `n` items have passed `g(f(x))` — it doesn't
build the full mapped list, then the full filtered list, then take
the prefix.

## Common terminal operations

| Operation | Description |
|---|---|
| `collect()` | Materialise into a `List<T>` (or `Map`, `Set`, ...) |
| `sum()` | Numeric sum |
| `product()` | Numeric product |
| `count()` | Number of items |
| `fold(init, f)` | Generic left fold |
| `any(p)` / `all(p)` | Predicate over the sequence |
| `min()` / `max()` | Extremum |
| `for x in iter` | Side-effecting traversal |

## Common build operations

| Operation | Description |
|---|---|
| `filter(p)` | Keep items where `p` is true |
| `map(f)` | Transform each item |
| `take(n)` / `skip(n)` | Window over the sequence |
| `enumerate()` | Yield `(index, item)` |
| `zip(other)` | Pair items with another iterator |
| `flat_map(f)` | Flatten nested iteration |
| `chain(other)` | Concatenate two iterators |

## Range iterators

`a..b` and `a..=b` are iterators directly — `for i in 0..10` walks
`0..9`, `(1..=6).product()` gives `720`. No need for manual counter
variables.
