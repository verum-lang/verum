# `core.collections.slice` — Audit

Conformance review for `core/collections/slice.vr` — the `&[T]` /
`&mut [T]` view-over-contiguous-memory primitive.  Slice is layer-2
foundational: every other collection's iteration / range / subrange
surface routes through it.  Section structure follows the project
template: cross-stdlib usage; crate-side hardcodes; language-
implementation gaps; defect inventory.

## Status

**regression-only** — the inherent-impl method surface declared in
`implement<T> [T] { ... }` blocks at `slice.vr:58-1137` does not
reach the runtime dispatch table for slice receivers.  At the VBC
runtime, slices carry a `List` runtime kind (slices are not yet a
distinct first-class kind from `List<T>`), so any method NOT also
present in the List impl panics with `method 'List.<name>' not
found on receiver of runtime kind 'List'`.

The "accidentally working" surface — methods that exist on BOTH
`implement<T> [T]` AND `implement<T> List<T>` with compatible
semantics — passes its conformance tests:

| Method | Status | Reason |
|---|---|---|
| `len` / `is_empty`            | green | List has parallel impl |
| `first` / `last`              | green | List has parallel impl |
| `get(i)`                      | green | List has parallel impl |
| `slice(a, b)` / `slice_from` / `slice_to` | green | List has parallel impl |
| `split_at(i)`                 | green | List has parallel impl |
| `min` / `max`                 | green | List has parallel impl with explicit `*x < *min` deref |
| `contains(&v)`                | green | List has runtime-intercepted impl |
| `iter()`                      | green | returns `SliceIter<T>` which has its own dispatch table |

The surface UNIQUE to slice — declared only in `slice.vr` —
panics or miscomputes:

| Method | Failure mode | Pinned in |
|---|---|---|
| `eq_slice(&other)` | dispatch panic                  | §A.1 |
| `cmp_slice(&other)` | dispatch panic                 | §A.3 |
| `to_list()` | dispatch panic                          | §A.2 |
| `is_sorted()` | returns false on sorted input        | §B.1 |
| `starts_with(p)` / `ends_with(s)` | wrong value     | §B.2 |
| `partition_point(pred)` | wrong index               | §B.3 |
| `position(&v)` / `rposition(&v)` | closure-dispatch crash | §C |
| `chunks(n).next()` / `windows(n).next()` | dispatch panic | §D |
| `binary_search(&v)` | compiler SIGBUS                | §E.1 |

## 1. Cross-stdlib usage

Slices are obtained one of two ways across `core/`:

| Site | Shape | Notes |
|---|---|---|
| `core/collections/list.vr:813` | `pub fn as_slice(&self) -> &[T]` | The canonical entry; calls `slice_from_raw_parts` intrinsic. |
| `core/collections/list.vr:820` | `pub fn as_mut_slice(&mut self) -> &mut [T]` | Mutable parallel. |
| `core/text/text.vr:*` | `text.as_bytes() -> &[Byte]` | Byte view of text — exercises `[Byte]`-specialised impl block. |
| `core/sys/common/*.vr` | `Buffer.as_slice() -> &[Byte]` | Syscall I/O staging. |
| `core/encoding/{json,protobuf}/*.vr` | Various | Wire-format byte slices. |

The slice surface that is NOT runtime-intercepted nor backed by a
parallel List method is fundamentally unreachable from user code
today.  This blocks adoption of slice-style APIs across the stdlib;
consumers fall back to `List<T>` everywhere.

## 2. Crate-side hardcodes

Searches across `crates/` for slice-specific dispatch:

| Path | Line(s) | What it does |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs` | runtime kind classification | Returns `List` for slice-shaped Object headers — see §3 below. |
| `crates/verum_codegen/src/llvm/intrinsics.rs` | slice_from_raw_parts | Lowering for `as_slice()`. |
| `crates/verum_codegen/src/llvm/types.rs` | slice ABI | LLVM lowering of `&[T]` as `{ ptr, len }` struct. |

**Drift surface**: the codegen ABI treats slices as `{ ptr, len }`
fat pointers; the VBC interpreter treats them as the original
`List<T>` heap object pointer.  This invariant mismatch is the
root cause of every §A defect.

## 3. Language-implementation gaps

| Gap | Impact | Fundamental fix |
|---|---|---|
| `&[T]` is not a distinct runtime kind from `List<T>` | Every method in `implement<T> [T] { ... }` that is NOT also present in the `implement<T> List<T> { ... }` table panics with "method not found" at CallM dispatch time. | One of three options: **(a)** codegen — when the receiver's static type is `&[T]`, emit `Call(Slice.<method>_fid)` directly rather than CallM; **(b)** dispatch — on CallM miss against List, fall through to the Slice method table; **(c)** runtime — split `Slice` from `List` as separate runtime kinds.  Option (a) is the most targeted (zero new runtime concepts) and aligns with how stdlib-internal `Call`s already work. |
| `slice[i]` indexing yields a ref-shape whose `*deref` does not normalise to the underlying value | `is_sorted` (uses bare `self[i] > self[i-1]`) fails; `starts_with` (uses explicit `*a == *b`) also fails, suggesting at least two distinct deref shapes are produced depending on whether the indexed slice came from `as_slice()` or from `slice(a, b)`. | Audit the codegen path for `Index<USize> for [T]` and unify it with `Index<USize> for List<T>` — they MUST produce the same ref-shape so deref normalises to the underlying value uniformly. |
| `slice.position(&value)` mis-resolves to the Iterator default's predicate form | Closure dispatch sees a `&T` argument where a closure is expected; `call_closure_sync` crashes with TypeMismatch. | Mirror the closed List pattern: rename slice's value-form to a distinct symbol (`position_value` / `find_index`), keep `position_by` for the predicate form, and let `position` route through the Iterator protocol. |
| `slice.binary_search(&target)` SIGBUSes the compiler | Any single test exercising the method aborts. | Investigation needed — backtrace localises to interpreter semaphore-wait, suggesting a thread-local trampoline corrupted during monomorphisation.  Triage path: bisect codegen for the `where T: Ord` generic-bound emission. |
| `Chunks<T>` / `Windows<T>` (slice.vr:1331-1444) report runtime kind `List` | `iter.next()` panics on dispatch because the iterator's wrapper struct shape isn't distinguished from a List object header. | Same fix path as §A — first-class runtime kind for these iterator types OR codegen-side static-dispatch. |

## 4. Defect inventory

See `regression_test.vr` for executable pins.

### §A — Dispatch panic on slice-only methods

* §A.1 `slice.eq_slice(&other)` panic
* §A.2 `slice.to_list()` panic
* §A.3 `slice.cmp_slice(&other)` panic

Root: `List` runtime kind / no fallback to slice method table.

### §B — Wrong-value computation on body-level methods

* §B.1 `is_sorted()` returns false on sorted input
* §B.2 `starts_with` / `ends_with` return false on true prefix/suffix
* §B.3 `partition_point` returns wrong index

Root: ref-shape unification on slice indexing.

### §C — Iterator/closure mis-resolution

* §C.1 `position(&value)` → Iterator predicate-form closure-dispatch crash
* §C.2 `rposition(&value)` → same shape

Root: method-name collision between inherent slice impl and
Iterator protocol default.

### §D — Slice iterator wrapper-type dispatch

* §D.1 `chunks(n).next()` panic
* §D.2 `windows(n).next()` panic

Root: same as §A — wrapper types share `List` runtime kind.

### §E — Compiler crashes

* §E.1 `binary_search(&v)` → SIGBUS

Root: TBD.  Likely codegen miscompile of `where T: Ord` bound.

## 5. Action items

### Landed in this branch

1. Complete unit-test surface for the working subset:
   * Section 1 — Properties (len / is_empty / first / last) × empty + non-empty
   * Section 2 — Element access (`get`) × in-bounds / negative / past-end / empty
   * Section 3 — Slicing (`slice` / `split_at`) × middle / full / empty / zero / end
   * Section 4 — Searching (`contains`) × present / absent / empty / first / last
   * Section 5 — Aggregation (`min` / `max`) × single / many / empty
   * Section 6 — Iteration (`iter().next()`) × sum / empty / count
2. Property-test surface — 8 laws (identity slice round-trip, split_at
   concat, get/iter coherence, min ≤ max, contains/iter coherence,
   first=get(0), last=get(len-1), is_empty iff len==0, subrange
   idempotent).
3. Integration tests — 5 cross-type scenarios:
   * List.as_slice → iter().sum
   * subrange → iter → collect round-trip
   * split_at half partition / min coherence
   * empty-slice projection chain
   * contains as filter-discriminator
4. Regression suite — 11 @ignore'd pins (3 §A, 3 §B, 2 §C, 2 §D, 1 §E) +
   5 PASS-GUARD checks for the working surface.

### Deferred (architectural, multi-crate scope)

1. **Close §A** — codegen-side static-dispatch for slice-typed
   receivers.  Requires plumbing through `verum_vbc/codegen` to
   detect `&[T]` static type at call site and emit `Call(fid)`
   rather than `CallM`.  Estimated 2-4 days.
2. **Close §B** — unify slice-indexing ref-shape with List-indexing.
   Likely a single codegen function in `verum_codegen/llvm/intrinsics.rs`
   plus the VBC mirror in `verum_vbc/src/interpreter/dispatch_table/handlers/get_index.rs`.
   Estimated 1-2 days.
3. **Close §C** — source-level rename of value-form to
   `position_value` / `find_index_of`; keep predicate-form on
   `find_position`.  One-line stdlib rename + audit call sites.
4. **Close §D** — fold into §A close-out.
5. **Close §E** — bisect binary_search codegen.  Estimated 1 day.

## 6. Status of dependents

When the slice runtime kind / dispatch issues close, the following
modules immediately benefit:

* `core.text.text` — currently can't use slice's `binary_search` for
  large dictionaries; uses linear scan instead.
* `core.encoding.protobuf` / `core.encoding.json` — byte-slice
  parsing routes through manual loops; would benefit from
  `chunks` / `windows` iterators.
* `core.collections.list` — would deprecate ~20 duplicate
  inherent methods that exist purely to give slice consumers a
  working escape hatch.
