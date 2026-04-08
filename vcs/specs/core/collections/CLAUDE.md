# core/collections Test Suite

Test coverage for Verum's collection types.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `list_test.vr` | `collections/list` | Basic List operations, construction, access, map, filter, fold, reduce | 160 |
| `btree_test.vr` | `collections/btree` | BTree operations, ordered access | 131 |
| `map_extended_test.vr` | `collections/map` | Entry API, iterators, retain, drain, protocols, helpers, count_by, group_by, map_from_keys | 129 |
| `slice_test.vr` | `collections/slice` | Slice operations, views, transformations | 121 |
| `set_test.vr` | `collections/set` | Set operations, set algebra, retain, drain, capacity | 120 |
| `deque_test.vr` | `collections/deque` | Double-ended queue operations | 97 |
| `collections_protocols_test.vr` | `collections/*` | Protocol implementations across all collections | 85 |
| `set_operators_test.vr` | `collections/set` | Set algebra operators, in-place operations | 82 |
| `heap_test.vr` | `collections/heap` | Heap operations, priority queue, min_heap, heap_sort | 81 |
| `list_split_test.vr` | `collections/list` | split_first/last, is_sorted*, extract_if, fill_with, resize_with, dedup_by_key, chunks_exact* | 78 |
| `slice_extended_test.vr` | `collections/slice` | dedup, dedup_by, sort_unstable, rotate, partition | 72 |
| `list_extended_test.vr` | `collections/list` | Advanced List ops, iterators, transformations | 69 |
| `list_production_test.vr` | `collections/list` | Production-quality List edge cases | 62 |
| `map_test.vr` | `collections/map` | Basic Map operations, insert, get, remove, types, stress | 62 |
| `map_production_test.vr` | `collections/map` | Production-quality Map edge cases | 59 |
| `btree_extended_test.vr` | `collections/btree` | get_key_value, remove_entry, retain, append, split_off, values_mut, Entry API, BTreeSet ops | 57 |
| `slice_split_test.vr` | `collections/slice` | Slice split operations | 53 |
| `list_double_ended_test.vr` | `collections/list` | DoubleEndedIterator for List iterators | 43 |
| `deque_extended2_test.vr` | `collections/deque` | Extended deque operations, shrink_to | 48 |
| `btree_extended2_test.vr` | `collections/btree` | into_keys, into_values, try_insert, extract_if, take, replace | 40 |
| `map_extract_test.vr` | `collections/map` | iter_mut, extract_if, merge, integration tests | 39 |
| `btreeset_ops_test.vr` | `collections/btree` | BTreeSet operations | 38 |
| `set_extended_test.vr` | `collections/set` | extract_if, strict subset/superset, symmetric_difference_update | 36 |
| `list_sort_unstable_test.vr` | `collections/list` | sort_unstable, sort_unstable_by, splice | 34 |
| `map_extended2_test.vr` | `collections/map` | Extended Map operations | 33 |
| `list_remaining_test.vr` | `collections/list` | is_not_empty, reserve, reserve_exact, list(), as_ptr, as_mut_ptr, get_unchecked | 33 |
| `heap_topk_merge_test.vr` | `collections/heap` | Top-K, merge operations | 33 |
| `deque_sort_test.vr` | `collections/deque` | Deque sorting operations | 33 |
| `deque_double_ended_test.vr` | `collections/deque` | DoubleEndedIterator for Deque iterators | 33 |
| `deque_new_methods_test.vr` | `collections/deque` | Deque.set, Deque.reverse, swap_remove_front/back | 31 |
| `slice_split_iter_test.vr` | `collections/slice` | Split, SplitInclusive, SplitN, RSplit, RSplitN iterators | 30 |
| `list_utils_test.vr` | `collections/list` | flatten_list, unzip_list, group_by, interleave_lists | 30 |
| `fused_iterator_test.vr` | `collections/*` | FusedIterator for all collection iterator types | 30 |
| `exact_size_iterator_test.vr` | `collections/*` | ExactSizeIterator for all collection iterator types | 30 |
| `deque_extract_dedup_test.vr` | `collections/deque` | Deque extract_if, dedup | 29 |
| `deque_windowing_test.vr` | `collections/deque` | Deque windows, chunks | 28 |
| `deque_extended_test.vr` | `collections/deque` | drain, split_off, append, resize, resize_with | 28 |
| `btree_conversion_test.vr` | `collections/btree` | from_entries, to_entries, keys_list, values_list, iter_mut | 25 |
| `list_capacity_test.vr` | `collections/list` | shrink_to, partition_point | 24 |
| `list_ord_hash_test.vr` | `collections/list`, `collections/deque` | List cmp_lexicographic, Hash, Deque Ord/Hash | 23 |
| `list_select_test.vr` | `collections/list` | select_nth_unstable, quickselect | 22 |
| `btree_bounds_test.vr` | `collections/btree` | BTreeMap/BTreeSet range bounds | 22 |
| `map_conversion_test.vr` | `collections/map` | Map conversion methods | 21 |
| `btree_protocols_test.vr` | `collections/btree` | BTree protocol implementations (IntoIterator, Eq, Extend, Hash, Index) | 37 |
| `map_set_new_methods_test.vr` | `collections/map`, `collections/set` | Map.update, Map.pop_entry, Set.pop, Set.update | 20 |
| `heap_extended_test.vr` | `collections/heap` | Extended heap operations | 20 |
| `consistency_conversion_test.vr` | `collections/*` | Deque.into_list, BTreeMap.keys_list/values_list, Heap.to_list | 19 |
| `map_utils_test.vr` | `collections/map` | get_or_default, filter_map | 18 |
| `heap_peek_mut_test.vr` | `collections/heap` | BinaryHeap.peek_mut(), PeekMut guard, Slice.fill_default | 18 |
| `map_tryinsert_test.vr` | `collections/map` | Map.try_insert | 17 |
| `list_flat_map_test.vr` | `collections/list` | List.flat_map | 15 |
| `list_chunk_by_test.vr` | `collections/list` | chunk_by with various predicates | 15 |
| `set_conversion_test.vr` | `collections/set` | Set conversion methods | 14 |
| `list_extend_test.vr` | `collections/list` | List Extend from iterators | 12 |
| `btreeset_conversion_test.vr` | `collections/btree` | BTreeSet.to_list, into_list | 12 |
| `btree_iter_mut_test.vr` | `collections/btree` | BTreeMap.iter_mut(), ordered mutable iteration | 12 |
| `slice_new_methods_test.vr` | `collections/slice` | Slice.fill_with, swap_with_slice | 9 |
| `from_conversions_test.vr` | `collections/*` | From<List<T>> protocol for Deque, Set, BTreeSet, BinaryHeap, Map, BTreeMap | 34 |
| `list_join_test.vr` | `collections/list` | join_text, join_ints utility functions with various separators and edge cases | 29 |
| `set_additional_test.vr` | `collections/set` | is_strict_subset, is_strict_superset, pop, update | 27 |

| `btreeset_compound_ops_test.vr` | `collections/btree` | BTreeSet |=, &=, -=, ^= compound assignment, retain_from_set | 35 |
| `btree_drain_test.vr` | `collections/btree` | BTreeMap/BTreeSet drain(), sorted iteration, size_hint | 23 |
| `btree_functional_test.vr` | `collections/btree`, `collections/set` | BTreeMap contains_value/get_or_default/any/all/find, BTreeSet/Set any/all/find | 37 |
| `heap_extract_shrink_test.vr` | `collections/heap` | BinaryHeap extract_if, shrink_to | 12 |

## Test Count: 2,769 tests total (64 passing files)

## Key Types Tested

### List<T>
Dynamic growable array.

**Construction:**
- `List.new()`, `List.with_capacity(n)`, `List.from([...])`, `List.from_slice()`, `List.from_elem()`, `list()`

**Access:**
- `len()`, `is_empty()`, `is_not_empty()`, `capacity()`, `get(index)`, `[index]`, `first()`, `last()`

**Mutation:**
- `push(value)`, `pop()`, `insert(index, value)`, `remove(index)`, `clear()`, `truncate(len)`
- `extend(iter)`, `append(&mut other)`, `splice(start, end, iter)`
- `extract_if(pred)`, `fill_with(f)`, `resize_with(len, f)`, `dedup_by_key(f)`

**Capacity Management:**
- `shrink_to_fit()`, `shrink_to(min)`, `reserve(n)`, `reserve_exact(n)`

**Search:**
- `binary_search(val)`, `binary_search_by(cmp)`, `partition_point(pred)`

**Splitting & Chunks:**
- `split_first()`, `split_last()`, `chunks_exact(size)`, `chunks_exact_remainder(size)`, `chunk_by(pred)`

**Sorting:**
- `is_sorted()`, `is_sorted_by(cmp)`, `is_sorted_by_key(f)`
- `sort_unstable()`, `sort_unstable_by(cmp)`, `select_nth_unstable(n)`

**Functional:**
- `map()`, `filter()`, `flat_map()`, `for_each()`, `fold()`, `reduce()`

**Iteration:**
- `iter()`, `iter_mut()`, `into_iter()`, DoubleEndedIterator support

### Map<K, V>
Hash-based key-value mapping.

**Construction:**
- `Map.new()`, `Map.from([...])`, `map_from_keys(keys, default)`

**Access:**
- `len()`, `is_empty()`, `get(key)`, `contains_key(key)`, `get_or_default(key, default)`

**Mutation:**
- `insert(key, value)`, `remove(key)`, `clear()`, `update(key, f)`, `pop_entry(key)`
- `entry(key)` — Entry API (or_insert, or_insert_with, or_default, and_modify)
- `extract_if(pred)`, `merge(other, resolve)`, `filter_map(f)`, `try_insert(key, value)`

**Iteration:**
- `keys()`, `values()`, `iter()`, `iter_mut()` with DoubleEndedIterator support

**Helpers:**
- `count_by(iter, key_fn)`, `group_by(iter, key_fn)`

### Set<T>
Hash-based unique element collection.

**Construction:**
- `Set.new()` + `.insert()` calls

**Set Operations:**
- `union(other)`, `intersection(other)`, `difference(other)`, `symmetric_difference(other)`
- `is_subset(other)`, `is_superset(other)`, `is_strict_subset(other)`, `is_strict_superset(other)`, `is_disjoint(other)`

**In-Place Set Operations:**
- `extend_from_set(other)`, `retain_from_set(other)`, `remove_from_set(other)`, `symmetric_difference_update(other)`

**Mutation:**
- `insert(value)`, `remove(value)`, `clear()`, `pop()`, `update(value)`, `extract_if(pred)`

### BTree<K, V> / BTreeSet<T>
Balanced tree with ordered keys.

**Construction:**
- `BTree.new()`, `BTreeMap.from_entries(list)`

**Access:**
- `len()`, `is_empty()`, `get(key)`, `get_key_value(key)`, `first_entry()`, `last_entry()`, `range(from, to)`
- `[key]` (Index<K> operator)

**Mutation:**
- `insert(key, value)`, `remove(key)`, `remove_entry(key)`, `clear()`
- `retain(pred)`, `append(other)`, `split_off(key)`, `extract_if(pred)`, `try_insert(key, value)`
- Entry API: `entry(key)`, `or_insert`, `or_insert_with`, `or_default`, `and_modify`, `key()`

**Protocols:**
- `Eq`, `Clone`, `Default`, `Debug`, `Display`, `Hash`, `Index<K>`, `FromIterator`, `IntoIterator`, `Extend`

**Conversion:**
- `to_entries()`, `keys_list()`, `values_list()`, `into_keys()`, `into_values()`

**BTreeSet-specific:**
- `take(value)`, `replace(value)`, `to_list()`, `into_list()`
- `is_strict_subset(other)`, `is_strict_superset(other)`, `extract_if(pred)`

### Heap<T>
Binary heap (priority queue).

**Construction:**
- `Heap.new()`, `Heap.max_heap()`, `min_heap(arr)`, `heap(arr)`

**Access:**
- `len()`, `is_empty()`, `peek()`, `peek_mut()` → PeekMut guard (value, replace, pop)

**Mutation:**
- `push(value)`, `pop()`, `push_pop(value)`, `heap_sort(&mut list)`

**Conversion:**
- `to_list()`, `into_list()`, `iter()`

### Deque<T>
Double-ended queue.

**Construction:**
- `Deque.new()`, `Deque.with_capacity(n)`

**Access:**
- `len()`, `is_empty()`, `front()`, `back()`, `[index]`, `set(index, value)`

**Mutation:**
- `push_front(value)`, `push_back(value)`, `pop_front()`, `pop_back()`
- `clear()`, `drain()`, `split_off(at)`, `append(other)`, `reverse()`
- `resize(len, value)`, `resize_with(len, f)`, `swap_remove_front(i)`, `swap_remove_back(i)`
- `extract_if(pred)`, `dedup()`, `shrink_to_fit()`, `shrink_to(min_capacity)`

**Windowing:**
- `windows(size)`, `chunks(size)`

**Iteration:**
- `iter()`, `into_iter()`, DoubleEndedIterator support

**Conversion:**
- `into_list()`, `to_list()`

### Slice<T>
View into contiguous memory.

**Access:**
- `len()`, `is_empty()`, `get(index)`, `[index]`, `first()`, `last()`

**Subslicing:**
- `slice(from, to)`, `split_at(index)`, `chunks(size)`, `windows(size)`

**Search:**
- `contains(value)`, `binary_search(value)`, `starts_with(other)`, `ends_with(other)`

**Split iterators:**
- `split(pred)`, `split_inclusive(pred)`, `splitn(n, pred)`, `rsplit(pred)`, `rsplitn(n, pred)`

**Mutation:**
- `fill_with(f)`, `fill_default()`, `swap_with_slice(other)`, `dedup()`, `dedup_by(f)`
- `sort_unstable()`, `rotate_left(n)`, `rotate_right(n)`

## Test Status

All 57 test files currently **PASSING**.

## Known Limitations

- `Set.from([...])` does NOT exist — use `Set.new()` + `.insert()` calls
- `SetDrain<T>` doesn't implement IntoIterator — use manual `drain.next()` loop
- `List<List<&Int>>` nested generics in collect cause parse errors — use count() or manual next()
