# Audit — `core/base/memory.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/memory.vr` (1845 lines) |
| Tests | `core-tests/base/memory/` — `cbgr_test.vr` (936 LOC, migrated), `unit_test.vr` (NEW, ~140 LOC, Heap/Shared/Cow/swap surface), `property_test.vr` (NEW, ~190 LOC, round-trip + swap involution + replace/take laws), `integration_test.vr` (NEW, ~110 LOC, nested heap, collections, allocator try_alloc) |
| Hardcodes in `crates/` | Critical — Heap/Shared TypeIds (HEAP=519, SHARED=520) hardcoded; allocator paths via verum_codegen |

## §1  TypeId hardcoding (drift surface)

`Heap` and `Shared` are well-known types with hardcoded `TypeId` values
in:

```text
crates/verum_vbc/src/codegen/mod.rs   — well_known_types mapping (HEAP=519, SHARED=520)
crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs:692
                                       — heap allocator stamps SHARED on Shared boxes
```

Background (project memory `project_heap_shared_typeid_repair_2026-05-06.md`):
prior commit `a8d51d48` aliased both Heap and Shared to `TypeId::PTR=14`,
which collapsed them in `metadata.types`. The fix bound them to dedicated
TypeIds (HEAP=519, SHARED=520).

**Drift surface:** if memory.vr ever splits Shared into Shared+Weak with
distinct layouts, or adds a third smart-pointer type, the well_known
TypeId mapping must be updated. There's no automated check.

**Action item (deferred):** mirror the variant-layout-pinning pattern.
Add a `SMART_POINTER_TYPE_IDS: &[(&str, u32)] = &[("Heap", 519),
("Shared", 520), …]` constant and a unit test that pins it.

## §2  CBGR integration

CBGR (capability-based generation refs) is the memory-safety substrate.
`memory.vr` types interact with CBGR via:
- `&T` references carry CBGR check (Tier 0, ~15ns overhead)
- `&checked T` references — compiler-proven safe (Tier 1, 0ns)
- `&unsafe T` — manual safety proof (Tier 2, 0ns)

Tests in `cbgr_test.vr` cover the runtime checks. The unit/property
tests added here cover the API surface but cannot directly observe
CBGR validation — that requires runtime introspection that's
outside this audit's scope.

## §3  Allocator try_* family

`try_alloc` / `try_alloc_zeroed` / `try_realloc` return
`Result<_, AllocError>` per project memory and per `mod.vr:243-246`.
This is the typed-OOM pattern (#65, #68) that lets collections
distinguish "out of memory" from "logic error".

`integration_test.vr §6` exercises the happy path. Negative-path
testing (intentionally exhausting memory to force OOM) is complex
and requires runtime support that's currently not test-friendly.

## §4  Cross-stdlib usage

`Heap` and `Shared` are pervasive — every type that needs heap
allocation uses one. `Weak` provides cycle-breaking refs. `Cow`
enables borrowed-or-owned transitions for read-mostly data.

Idiomatic patterns observed:
- `Heap::new(x)` for owned heap allocation
- `Shared::new(x)` for refcounted shared ownership
- `Cow::borrowed(&x)` then `.into_owned()` on first mutation
- `Pin<T>` for self-referential structures

## §5  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/memory_cbgr_test.vr` →
       `core-tests/base/memory/cbgr_test.vr` (vtest frontmatter stripped)
- [x]  Add `unit_test.vr` covering Heap/Shared construction + clone,
       Cow borrowed/owned, ManuallyDrop, replace/take/swap,
       MaybeUninit
- [x]  Add `property_test.vr` covering round-trip laws, swap
       involution, replace/take contracts, Heap clone independence,
       Shared value-observation; plus @property samples and
       @test_case truth tables
- [x]  Add `integration_test.vr` covering List<Heap<T>>, Map<K, Shared>,
       Cow paths, nested Heap, try_alloc happy path
- [x]  Add this audit document

## §6  Action items deferred (not landed)

1. **SMART_POINTER_TYPE_IDS canonical constant** — close the §1 drift
   surface. ~30 LOC in well_known_types + matrix-pinning test.
2. **Negative-path try_alloc testing** — requires test-mode allocator
   that simulates OOM. Architecture-level work, ~200 LOC.
3. **CBGR aliasing-violation observable tests** — verify that
   `&mut T` + `&T` simultaneous borrows produce a runtime error in
   Tier 0 and a compile error in Tier 1+. Currently CBGR contract is
   tested only via lower-level runtime tests.
4. **Pin<T> projection contracts** — Pin's promise is "pinned data
   does not move"; testing this requires self-referential examples
   that exercise the move-blocking path.
