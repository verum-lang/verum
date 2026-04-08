# stdlib/mem Test Suite

Comprehensive test coverage for Verum's CBGR (Counter-Based Generation Reference) memory system.

## Architecture Overview

The CBGR system provides memory safety without garbage collection through:
- **Generation counters**: 32-bit counters tracking allocation lifetimes
- **Epoch tracking**: 16-bit global epoch for wraparound protection
- **Hazard pointers**: Race-free validation-to-dereference
- **Capability-based access control**: Fine-grained permissions

## Test Organization

| File | Module | Coverage |
|------|--------|----------|
| `header_test.vr` | `mem/header` | AllocationHeader, generation/epoch atomics, validation |
| `epoch_test.vr` | `mem/epoch` | EpochManager, EpochCache, callbacks, wraparound |
| `capability_test.vr` | `mem/capability` | Capability flags, attenuation, delegation |
| `thin_ref_test.vr` | `mem/thin_ref` | ThinRef<T>, validation tiers, revocation |
| `fat_ref_test.vr` | `mem/fat_ref` | FatRef<T>, slicing, subslice views |
| `hazard_test.vr` | `mem/hazard` | HazardGuard, acquire/release, retirement |
| `size_class_test.vr` | `mem/size_class` | Size bins, fragmentation, page kinds |
| `segment_test.vr` | `mem/segment` | Segment allocation, slices, abandonment |
| `heap_test.vr` | `mem/heap` | LocalHeap, fast path, cross-thread free |
| `allocator_test.vr` | `mem/allocator` | cbgr_alloc/dealloc/realloc |
| `allocator_protocols_test.vr` | `mem/allocator` | AllocError Display, message content verification | 11 |
| `mem_errors_test.vr` | `mem/mod`, `mem/header` | UseAfterFreeError, RevocationError, ValidationError Debug/Display | 19 |

## CBGR Validation Performance Targets

- Tier 0 (Interpreter): ~15ns full validation
- Tier 1 (Baseline JIT): ~10ns optimized load
- Tier 2 (Optimizing JIT): ~5ns selective validation
- Tier 3 (AOT): ~3ns null check only

## Key Invariants Tested

1. **Generation uniqueness**: Each allocation gets unique generation
2. **Use-after-free detection**: Freed memory invalidates all references
3. **Epoch protection**: Generation wraparound handled via epoch
4. **Capability monotonicity**: Capabilities can only be attenuated
5. **Hazard pointer safety**: No deallocation while hazard active
6. **Cross-thread correctness**: Atomic operations on xthread_free list

## Memory Layout Tested

- AllocationHeader: 32 bytes (size, align, gen, epoch, caps, type_id, flags)
- ThinRef<T>: 16 bytes (ptr, generation, epoch_and_caps)
- FatRef<T>: 32 bytes (ptr, gen, epoch_caps, metadata, offset, reserved)
- PageHeader: 128 bytes (cache-line aligned)
- Segment: 32 MiB with 512 slices of 64 KiB each
