# Native List storage: evidence and implementation boundary

Status: design and measured counterexamples, 2026-10-04. T1575 is open;
T1517 remains open for its interpreter/native shrink-and-regrow acceptance.
This document does not introduce a storage ABI or claim a native fix.

## Measured failures

A source-driven LLVM/JIT probe at `2a655dfa4` produced three failures. It
parsed Verum source and compiled it through public VBC and LLVM APIs, without
injecting descriptors or constructing instructions. The JIT retained the
emitted CBGR allocator and substituted only the lowest allocation/release
boundary with test-owned storage. The allocation-extent assertion ran before
any potentially out-of-bounds element write.

| Source / boundary | Observed | Required |
|---|---|---|
| `List<Byte>.new()` | null handle | valid List object |
| `Byte.size` / `generic_size<Byte>()` where `generic_size<T>() { T.size }` | 1 / 8 | generic substitution must not erase the declared layout query |
| Concrete `List<Byte>` allocation, capacity 40 | CBGR user extent 40 bytes; native `push` uses an `i64` GEP, stride 8 | allocation extent and element addressing must agree |

The third source spells the allocation expression from `List.with_capacity`
with its type argument explicitly resolved. It does **not** show that the
current generic stdlib body already resolves `T.size` to 1. In fact, that body
currently receives the constant 8 before monomorphization. This accidental
agreement with native slot operations cannot establish a storage contract.

A separate control observed `Record.size == 8` for a three-field record.
The field payload occupies more than one slot, but this query is not evidence
for that inline extent. Do not derive value storage from a field count or
assume that a source type-size query describes an inline foreign object.

An earlier actual native run of the existing
`vcs/specs/L0-critical/stdlib-runtime/list_backing_shrink_regrow.vr` failed
before its first output. Disassembly showed the packed-list allocator as
`mov x0, #0; ret`. An LLDB breakpoint at `List.shrink_to_fit` found the caller's
handle already zero; a write watchpoint did not fire before the subsequent
capacity read faulted at address `0x20`. Thus the method did not null the
handle. The source-driven probe independently reproduces the null result.

## Reproduce from source

Use a normally built CLI and matching baked stdlib. These are diagnostic
commands, not an instruction to bypass archive validation or rebuild a shared
checkout. `verum build` supports the emission flags below; output locations
are printed by the command.

Save as `list_storage_probe.vr`:

```verum
mount core.*;

fn generic_size<T>() -> Int { T.size }
fn make_bytes() -> List<Byte> { List<Byte>.new() }

// The concrete form of the allocation used by List.with_capacity.
fn concrete_bytes(capacity: Int) -> List<Byte> {
    let data = unsafe {
        alloc(capacity * Byte.size, Byte.alignment) as &unsafe Byte
    };
    List { len: 0, cap: capacity, ptr: data }
}

fn push_byte(bytes: &mut List<Byte>, value: Byte) {
    bytes.push(value);
}

fn main() {
    print(Byte.size);
    print(generic_size<Byte>());
}
```

```sh
verum run --tier interpret list_storage_probe.vr
verum build list_storage_probe.vr --emit-vbc --emit-llvm --keep-temps
```

Inspect the emitted functions for the constructor helper body, the constant
returned by `generic_size`, the allocation byte count in `concrete_bytes`,
and the element GEP/store in `push_byte`. If an emission mode prunes unused
functions, give each diagnostic function its own caller. The JIT measurement
above compiled these functions as roots and called them directly, so their
observed results do not depend on source-main reachability.

The existing end-to-end fixture can be run separately:

```sh
verum run --tier interpret vcs/specs/L0-critical/stdlib-runtime/list_backing_shrink_regrow.vr
verum run --tier aot vcs/specs/L0-critical/stdlib-runtime/list_backing_shrink_regrow.vr
```

These commands exercise the full CLI. An earlier checker or archive failure
is a separate boundary to record; the focused JIT measurement does not claim
that every full-pipeline stage passed.

An interpreter pass does not satisfy the native half of this acceptance.
The LLVM-only extent diagnostic should refuse the mismatch before executing
a write; adding a packed allocator without changing consumers is unsafe.

## Current authorities and where they disagree

* `verum_common::layout::{LIST_LEN_OFFSET, LIST_CAP_OFFSET, LIST_PTR_OFFSET}`
  and the declaration in `core/collections/list.vr` agree on the object
  fields: `{len@24, cap@32, data@40}` after the 24-byte object header.
  These constants describe the object, not its element encoding.
* VBC `compile_method_call` in `codegen/expressions.rs`
  lowers byte-list construction to `MemExtended` sub-op `0x06`. Native
  `lower_mem_extended` only declares `verum_alloc_byte_list_packed` and
  omits the List result marker. The unresolved-function fallback in
  `llvm/vbc_lowering.rs` supplies a null-returning body.
* Ordinary native `NewList` marks the result as a List. Without this fact,
  `lower_ref_mut_family` passes the local handle's address instead of the
  object address expected by the compiled source method. Parameter and
  return paths need the same fact, not only a local constructor fix.
* `RuntimeLowering::lower_list_push`, `lower_list_pop`, native element
  access and `define_list_ir_helpers` address value slots of eight bytes.
  Native capacity allocation and growth use bare `verum_internal_calloc`
  storage, whose underlying allocator is `verum_os_alloc`.
* Source `with_capacity`, `try_with_capacity`, `resize_buffer`,
  `try_resize_buffer`, `free_buffer`, and several movement operations use
  `T.size` / `T.alignment`. Element addresses use typed pointer offsets.
  Their allocation/deallocation lowering uses a CBGR AllocationHeader.
  Bare OS backing cannot be handed to that deallocator as if it had one.
* Packed arrays and foreign byte buffers have their own established
  contract: `Byte.size == 1` and contiguous bytes. See
  [FFI byte-buffer contract](ffi-byte-buffer-contract.md). They cannot be
  repaired by globally changing Byte's size or pointer arithmetic.

## Proposed common boundary

The next implementation should carry an **element storage layout** derived
from exact instantiated declaration identity and the value ABI. Keep these
facts distinct:

| Fact | Purpose |
|---|---|
| Semantic element type | method/protocol dispatch and generic substitution |
| Element encoding | packed scalar bits versus a value/handle slot |
| Storage stride and alignment | extent, index addressing, load/store and bulk copy |
| Backing allocation provenance | the allocator and deallocator contract |
| Container/reference carrier | object handle, element reference, slice or raw data |

For example, a compact native byte buffer can store one byte per element,
while a heap-record value can be an eight-byte handle regardless of the
record's field layout. An erased generic must receive or retain this fact;
it must not invent it from the name `Byte`, a pointer's magnitude, or the
legacy `T.size` default. Nominal TypeId must continue to identify the declared
type rather than silently becoming a storage-kind tag.

A sound implementation must select both the metadata carrier and its runtime
materialization before changing allocation. There is no new agreed wire
field in this document. If an erased List needs runtime layout metadata,
choose an explicit carrier, update serialization/remapping as necessary,
and define its ownership. Do not quietly repurpose reserved header bytes.

The coherent implementation unit then includes:

1. Constructor, parameter, call-return and reference lowering retain the
   same storage/carrier facts. Unknown or conflicting facts remain explicit.
2. Allocation, reserve, shrink, growth and release share one backing contract.
   Reuse the existing target-aware CBGR allocator where that is the chosen
   provenance; no libc or host-target branching is introduced.
3. Source List bodies ask the buffer's storage contract for byte counts,
   alignment and element addresses. General `T.size` remains a semantic
   layout query. A value/handle slot must not be confused with an inline FFI
   object. This also covers fallible constructors and reserve operations.
4. Native push/pop, index read/write, movement helpers, cloning and slices
   consume that same element encoding. Scalar loads/stores encode and decode
   the native register representation explicitly.
5. Raw-pointer and slice APIs state which data layout they expose. A strided
   value buffer must not be presented to FFI as contiguous bytes. A packed
   representation must not reach an eight-byte writer.

The generic type-property loss is a related independent producer defect
(T1576).
Fixing it alone would expose the concrete underallocation in existing List
bodies, so it is not an alternative to this storage boundary. Conversely,
a List repair must not redefine generic type properties globally.

## Acceptance before claiming a repair

Use real source declarations through VBC and LLVM/JIT, then a fresh coherent
interpreter/native CLI. At minimum:

* `List<Byte>.new`, empty shrink, reserve, writes/reads of 0, 255 and 42,
  nonempty shrink, another reserve, and growth past the initial capacity.
* The same operations through parameters and returned lists, including
  `&mut self`; constructor-local markers alone are insufficient.
* `List<Int>` and heap-value records, checking stored values and alias/clone
  behavior required by the existing value-copy contract.
* Allocation extent, stride and matching release provenance agree at every
  transition, including fallible paths. Test physical extent before writes.
* Packed `[Byte; N]` and foreign byte-buffer controls retain stride 1;
  unrelated same-leaf nominal types retain their own declaration identity.
* The existing T1517 fixture completes in both tiers. A known failure cannot
  be excluded by an ignored test or counted as a successful native run.

No production change or native acceptance is recorded by this design commit.
