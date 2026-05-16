# `core.collections.trie` — Audit

`core/collections/trie.vr` — `Trie<V>`, prefix tree with O(|key|)
access, byte-slice keyed.

## Status

**regression-only** — Trie's populated-state surface routes through
`Map.get_mut` for the mutator path (`insert` / `remove`), which
fails with the wrapper-iter / Map-method dispatch class:

```
method 'Map.get_mut' not found on receiver of runtime kind `Map`
```

Working empty-state surface: new / get-absent / contains-absent /
is_empty / len — 3 unit + 3 property + 2 integration + 3 PASS-GUARDs.
Populated-state pinned in `regression_test.vr` §A.

## Action items

### Landed in this branch

1. 3 unit + 3 property + 2 integration + 6 regressions (3 PASS-
   GUARDs + 3 @ignore'd defect-pins).

### Deferred

* `Map.get_mut` dispatch close-out — unblocks `Trie.remove` /
  `Trie.insert`'s mutator path.  Same architectural class as
  multiset §B / slice §D.
