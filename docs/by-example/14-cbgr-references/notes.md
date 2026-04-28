# 14 — CBGR Three-Tier References

CBGR (Counter-Based Generation Reference) is Verum's memory-safety
system: a fat reference layout that catches use-after-free at runtime
without garbage collection. References come in three tiers depending
on what the compiler can prove.

## The three tiers

| Tier | Syntax | Overhead | Safety basis |
|---|---|---|---|
| 0 | `&T` | ~15ns / deref | Runtime CBGR check |
| 1 | `&checked T` | 0ns | Compiler-proven |
| 2 | `&unsafe T` | 0ns | Manually proven |

## Layout

`&T` is 16 bytes:

```
+--------+--------+--------+
| ptr    | gen    | epoch  |
| 8 B    | 4 B    | 4 B    |
+--------+--------+--------+
```

- `ptr` — raw pointer to the referent.
- `gen` — generation counter; matches the referent's allocation. If
  the referent is freed, the slot's gen is bumped, and any old `&T`
  reference's stale gen no longer matches → check fails.
- `epoch` — coarser temporal scope, e.g. for region/arena tracking.

A dereference compares `gen` against the live slot's `gen` and panics
on mismatch. Modern hardware: ~15ns including the conditional branch.

## When to use which tier

- **`&T` (default)**: 99% of code. The 15ns overhead disappears in
  the noise of any real workload, and the runtime check is your
  insurance against the entire use-after-free class.
- **`&checked T`**: hot paths where escape analysis succeeds.
  Mostly local references that don't escape the function. The
  compiler will refuse to accept `&checked` if it can't prove
  safety — the annotation is a contract, not a hint.
- **`&unsafe T`**: FFI boundaries, custom allocators, hand-rolled
  data structures whose invariants exceed the type system's
  vocabulary. Always paired with an `unsafe { ... }` block that
  documents the SAFETY argument inline.

## Three lints to know

- `cbgr/elide-tier0-on-checkable` — suggests upgrading `&T` to
  `&checked T` when escape analysis succeeds.
- `cbgr/no-unsafe-without-safety-comment` — denies `unsafe` blocks
  without an explanatory comment.
- `cbgr/no-mixed-tiers` — denies passing `&unsafe T` where `&T` is
  expected without an explicit cast.
