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


## First implementation boundary: existing List buffers

The instance-storage query uses the existing canonical `LIST` (value slots)
versus `BYTE_LIST` (packed bytes) runtime identity. It does not reinterpret an
arbitrary nominal TypeId as a layout. `list_storage_stride` is declared in
`core.intrinsics.memory` and encoded as `MemExtended.ListStorageStride` in VBC
2.19; unknown identities are refused in both execution paths. The same width
is the alignment for these two existing encodings.

`next_cap`, `resize_buffer`, `try_resize_buffer` and `free_buffer` use that
carrier rather than generic `T.size`. Native ordinary List construction,
growth and clone retain the canonical header and use CBGR backing allocations,
matching the source allocator/reallocator/deallocator contract. Cloning
preserves the source encoding. The generated allocator checks its capacity
limit before multiplying by the width.

The integration control is ordinary source:

```verum
mount core.*;
fn main() {
    let mut values: List<Int> = List<Int>.new();
    values.reserve(3);
    values.push(7);
    values.shrink_to_fit();
    values.reserve(17);
    print(f"int_list_layout_control={values[0]}");
}
```

The same source-method JIT harness returned 7 before typed layout queries,
then refused an unresolved `Generic(0)` query inside `List.resize_buffer`.
The storage query restores 7 using the actual buffer representation. The
focused gates are `verum_vbc --test list_storage_query` and
`verum_codegen --test list_storage_query`; they also cover serialized query
boundaries, unknown encodings, record values, clone and call/return carriers.

This boundary does not enable the native packed-byte constructor or complete
the full acceptance above. Static element selection, raw element access,
movement, iterators and slices still need a coherent packed-storage unit.
The constructor selection now uses the exact declared element identity: a
user nominal named `Byte`, a reference to Byte and a same-spelling generic
parameter no longer select the packed route. This repairs producer identity;
it does not enable the missing native packed allocator. Fresh whole-CLI/native
List acceptance remains required.


## Checked container-owned element operations

The next bounded unit declares unsafe `list_storage_read`,
`list_storage_write` and `list_storage_move` in `core.intrinsics.memory`.
VBC 2.21 carries their register operands in the existing memory-operation
length envelope. They take a List owner and element indices, never an
unqualified raw element pointer. The owner selects the existing 8-byte Value
slot or 1-byte packed representation. Semantic `T.size` is not consulted.

Both execution paths validate the canonical owner shape, the capacity range
and the allocation byte extent before access. A zero-count move still
validates nonempty backing; a legitimate zero-capacity/null owner requires
no backing dereference. Move validates both complete ranges and supports
overlap in either direction. It copies storage bits without element glue.
The unsafe caller remains responsible for initialization, logical length,
exclusive access, and move/drop obligations. Reading does not clone a value.

The focused source→wire→interpreter tests use both canonical encodings,
public declarations, record handles and Float values. Source→LLVM/JIT tests
exercise ordinary value slots and public borrowed-owner calls. A separate
packed-consumer test builds the existing canonical byte carrier with a real
emitted CBGR allocation; this is **not** evidence of a working native packed
constructor. Float32 uses the same value-preserving f64 slot encoding as
native register storage. Unboxed native i128 writes are unsupported and
produce a lowering error rather than truncate; the strict-codegen test
exposes that error independently of the existing lenient function-skip policy.

This unit does not migrate every source List operation or enable the packed
constructor. Raw `ptr.offset` operations, reference-producing iterators and
slice construction still need the same encoding carried through their
consumers. In particular, generic iterator adaptors cannot recover an
original byte address from a preloaded value. Their reference representation
and lifetime proof remain a separate prerequisite. Full acceptance above
requires the coherent native fixture after those consumers are complete.

## Owner-preserving allocation and source value methods

VBC 2.22 adds `list_storage_resize(owner, capacity) -> Bool`. It validates
an existing canonical owner, its initialized length and allocation extent.
The requested capacity must retain every initialized element. Only the
`len * storage_width` prefix moves; spare capacity is not read as elements.
A false result leaves length, capacity, pointer and initialized values
unchanged. Zero capacity is permitted only at zero length. The operation
invalidates element borrows and does not run element glue.

Source `with_capacity` and `try_with_capacity` first create an empty owner,
then use its encoding through the source resize methods. `resize_buffer`
panics on failure, while `try_resize_buffer` returns the declared `AllocError`.
`free_buffer` checks that logical length is already zero. Interpreter growth
allocates tracked backing instead of publishing an untracked raw allocation;
managed old backing remains under the interpreter heap's ownership. Exact
tracked CBGR backing is released through its existing deallocator. An
interior bridge address cannot authorize freeing its enclosing allocation.
Native growth uses the same CBGR header construction for checked and fallible
allocation, with the latter reporting OS allocation failure before changing
the owner. It copies initialized bytes before releasing the old block.

The internal native allocator takes an explicit canonical encoding (512 or
527). Ordinary value-slot construction and cloning reuse it. Source `get`,
`set`, `push`, `pop`, `insert`, `remove`, `swap` and `swap_remove` use the
checked owner-based value operations. The legacy native ListPop opcode
branches before any empty-buffer read and reads nonempty elements using the
same storage encoding. This does not change element clone/drop semantics.

The durable gates are `verum_vbc --test list_source_storage` and
`verum_codegen --test list_storage_query`. The source-method fixtures retain
real List method bodies under distinct names to avoid a method intercept
hiding their behavior. These are public parser/codegen tests, not a full CLI
checker or AOT acceptance. They include reserve, growth, insertion/removal,
clear/truncate, shrink and regrowth for Byte/Int and record slots; allocation-failure controls
check preservation of the old owner. Native tests retain the compiler-owned
checked allocator and exit bodies, substituting only OS allocation/exit
edges. Packed native owners are created by the actual internal allocator,
not by constructing a header in the test.

This is an internal allocation/value-access boundary. The public native
packed constructor remains disconnected: borrowed `get_unchecked`, indexing,
raw-pointer methods, iterators and slices still require an explicit storage
reference contract. Their original address and width cannot be reconstructed
from a preloaded word or from `&T` alone. Whole native List acceptance remains
open until those consumers and the public constructor compose safely.
