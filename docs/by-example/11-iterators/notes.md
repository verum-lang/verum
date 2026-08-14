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

## How many stars does a closure need

`iter()` borrows, so `Self.Item` is `&T`. That single fact decides
every `*` you will write, and the two halves of the rule differ by one
level:

* **Transform closures take the item.** `map`, `filter_map`,
  `for_each`, `fold` hand you `Self.Item` — a `&Int` for a
  `List<Int>` — and arithmetic reads through it:
  `nums.iter().map(|x| x * x)`.
* **Predicate closures take a reference to the item.** `filter`,
  `any`, `all`, `find`, `position`, `take_while` and `skip_while` are
  declared `fn(&Self.Item) -> Bool`, so the parameter is `&&Int` and a
  comparison needs both stars:
  `nums.iter().filter(|n| **n > 12)`.

If a comparison will not typecheck, add a star; if it still will not,
you are on the transform side and should remove one. An explicit `*`
is always allowed where the value is wanted — `|x| *x * *x` and
`|x| x * x` are the same pipeline, and the first reads better in a
dense expression.

## Lazy `.iter().map(f)` vs eager `.map(f)`

Both spellings exist and mean different things:

```verum
let a = nums.iter().map(f);   // MappedIter — nothing computed yet
let b = nums.map(f);          // List<Int>   — computed and allocated
```

Chain from `.iter()` when more transforms follow: the whole chain
fuses into one pass with no intermediate List. Call `.map` directly on
the collection when the container itself is what you want and there is
nothing to fuse.

## Range iterators

`a..b` and `a..=b` are iterators directly — `for i in 0..10` walks
`0..9`, `(1..=6).product()` gives `720`. No need for manual counter
variables.
