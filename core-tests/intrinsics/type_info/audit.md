# `intrinsics/type_info` audit

Module: `core/intrinsics/type_info.vr` (~89 LOC) — type-
information intrinsics.

Tests: 47 unit tests covering the **canonical type-property
surface** (T.size, T.bits, T.alignment, T.stride, T.name, T.id)
across 13 primitive types (Int / Int8 / Int16 / Int32 / Int64 /
UInt8 / UInt16 / UInt32 / UInt64 / Byte / Float32 / Float64 /
Bool).

## 1. Why type-property form

Per the module docs: type properties are the **canonical**
surface; the legacy meta-fn form (`size_of<T>()`,
`align_of<T>()`, etc.) is `@deprecated`. Each legacy fn carries
the directive "Use T.size type property instead".

Type properties evaluate at compile time and are essentially
free (~2-7s per test, vs. ~25-50s for runtime-dispatched
legacy fn calls). The compile-time resolution is also why
type-property form works under --interp where the legacy
meta-fn form returns 0 (see audit §2).

## 2. Legacy meta-fn form returns zero — documented limitation

`size_of<Int>() = 0` under --interp via legacy meta-fn dispatch
(probed in round-8 investigation). The legacy form is
`@deprecated` and not load-bearing — Verum's canonical API is
the type-property form, which works correctly. Pinned at
[[intrinsic_meta_fn_returns_zero_2026-05-24]] for the legacy-
path tracking; not a blocker for stdlib type-info coverage.

## 3. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mem.allocator` | T.size + T.alignment for layout. |
| `core.collections.bloom` | T.size for cell sizing. |
| `core.text.numeric.BigInt` | T.bits for word-size detection. |
| Application generic code | T.name for diagnostic rendering. |

## 4. Crate-side hardcodes

* Primitive-type size table MUST agree with the LLVM target
  data layout for the configured triple.
* `Int = Int64` on the default target — pinned.
* `Float = Float64` (Float32 and Float64 are explicit
  alternative names).
* `Byte = UInt8` semantically — size = 1.
* `T.bits = T.size * 8` for every primitive — invariant pinned
  in this branch via 8 consistency tests.
* `T.alignment <= T.size` for every primitive — invariant
  pinned via 5 tests.
* `T.stride >= T.size` for every primitive — invariant pinned
  via 3 ge-size tests.

## 5. Language-implementation gaps

### §5.1 Property test on T.size / T.bits / T.alignment invariants

* ∀primitive T. T.bits == T.size * 8
* ∀primitive T. T.alignment <= T.size
* ∀primitive T. T.stride >= T.size
* ∀primitive T. T.stride is a multiple of T.alignment

Currently pinned per-type; generalize via property test.

**Effort:** ~30 min.

### §5.2 Type-ID uniqueness exhaustive sweep

3 pairwise-distinct-id tests landed (Int vs Float64, Int32 vs
Int64, Int vs Bool). Exhaustive: all 13×12/2 = 78 unique-ID
checks. Currently sampled.

### §5.3 Generic-type properties

`List<Int>.size`, `Maybe<Int>.size`, `Heap<Int>.size`, etc.
Verifying that generic-type-property dispatch works for
parameterised types. Gated on whether the property-resolution
path supports generic params.

### §5.4 Fix legacy meta-fn dispatch under --interp

`size_of<T>()` returns 0. Either (a) fix the @intrinsic
dispatch to resolve the type-size at runtime, OR (b) drop the
legacy meta-fn form entirely from stdlib. Task-level discussion
needed.

## Action items landed in this branch

* `core-tests/intrinsics/type_info/unit_test.vr` — 47 unit
  tests over the canonical type-property surface for 13
  primitive types.
* `core-tests/intrinsics/type_info/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test on size/bits/alignment invariants | this folder | 30 min |
| Type-ID uniqueness exhaustive sweep (78 pairs) | this folder | 30 min |
| Generic-type properties (List<Int>.size etc.) | this folder | 1h (gated on resolution) |
| Fix or drop legacy meta-fn dispatch | `core/intrinsics/type_info.vr` + VBC | multi-day |
| Sister tests for `core.intrinsics.{arithmetic,bitwise,conversion,control,float,memory,simd}` | sister folders | 1 week total |
