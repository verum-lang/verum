# VBC opcode map — extended-family inventory and the T0852 re-homing

The single reference for WHERE an operation lives on the wire: which
top-level opcode carries it, which sub-op enum names it, which files
handle it on each tier.  `crates/verum_vbc/src/instruction.rs` is the
code authority; this document is the map and the allocation
discipline.  History: `FfiExtended`'s `SystemSubOpcode` pocket had
grown to 119 variants of which 86 had nothing to do with FFI (owner
mandate 2026-08-22, T0852) — Time, Sys, Mach, Sync, CBGR and memory
operations all squatted in the FFI byte space, which is how the
T0425/T0429 byte collisions and the 0x68 cross-session race happened.

## Top-level gateways (envelope format)

All family gateways share one wire shape, decoded by
`dispatch_enveloped`:

```
[opcode:u8] [sub_op:u8] [operand_len:varint] [operands:len bytes]
```

| Opcode | Byte | Sub-op enum | Tier-0 handler file | Tier-1 lowering |
|---|---|---|---|---|
| `MathExtended` | 0x29 | `MathSubOpcode` | `math_extended.rs` | `lower_math_extended` |
| `SimdExtended` | 0x2A | `SimdSubOpcode` | `simd_extended.rs` | `lower_simd_extended` |
| `CharExtended` | 0x2B | `CharSubOpcode` | `char_extended.rs` | `lower_char_extended` |
| `CmpExtended` | 0x4F | `CmpSubOpcode` | `comparison.rs` | `lower_cmp_extended` |
| `CbgrExtended` | 0x78 | `CbgrSubOpcode` | `cbgr.rs` | `lower_cbgr_extended` |
| `TextExtended` | 0x79 | `TextSubOpcode` | `text_extended.rs` | `lower_text_extended` |
| `FfiExtended` | 0xBC | `SystemSubOpcode` | `ffi_extended.rs` | `lower_ffi_extended` |
| `ArithExtended` | 0xBD | `ArithSubOpcode` | `arith_extended.rs` | `lower_arith_extended` |
| `LogExtended` | 0xBE | `LogSubOpcode` | `log_extended.rs` | `lower_log_extended` |
| `MemExtended` | 0xBF | `MemSubOpcode` | `mem_extended.rs` | `lower_mem_extended` |
| `CubicalExtended` | 0xDE | `CubicalSubOpcode` | `cubical.rs` | — (proof kernel) |
| **`TimeExtended`** | **0xF0** | `TimeSubOpcode` | `time_extended.rs` | `lower_time_extended` |
| **`SysExtended`** | **0xF1** | `SysSubOpcode` | `sys_extended.rs` | `lower_sys_extended` |
| **`MachExtended`** | **0xF2** | `MachSubOpcode` | `mach_extended.rs` | `lower_mach_extended` |
| **`SyncExtended`** | **0xF3** | `SyncSubOpcode` | `sync_extended.rs` | `lower_sync_extended` |
| `GpuExtended` | 0xF8 | `GpuSubOpcode` | `gpu.rs` | `lower_gpu_extended` |
| `TensorExtended` | 0xFC | `TensorSubOpcode`¹ | `tensor_extended.rs` | `lower_tensor_extended` |
| `MlExtended` | 0xFD | `MlSubOpcode` | `ml_extended.rs` | `lower_ml_extended` |

¹ `TensorExtended`/`GpuExtended` use per-sub_op structural decoding
without the length prefix; every other gateway is a length-prefixed
blob (`decode_extended_operands`).

Free top-level bytes: **0xF4-0xF7, 0xFE, 0xFF** — the growth reserve.
Allocating one is a pool-visible decision (file a task naming the
byte BEFORE emitting it; the 0x68 race happened because two sessions
grew the same pocket blind).

## The T0852 re-homing (what moved where)

86 `SystemSubOpcode` squatters moved to family gateways.  The wire
bytes changed (dev-stage: no compatibility obligation; the bake and
all `.vbca` archives regenerate from source):

| Family | Old home (FfiExtended sub-op) | New home |
|---|---|---|
| Time (7) | 0x70-0x76 `Time*` | `TimeExtended` 0x00-0x06 |
| Syscalls (6) | 0x80-0x85 `Sys*` | `SysExtended` 0x00-0x05 |
| Entropy/random (2) | 0x47/0x48 `Random*` | `SysExtended` 0x06/0x07 |
| Tier introspection (2) | 0x86/0x87 | `SysExtended` 0x20/0x21 |
| Environment (3) | 0x88-0x8A `Env*` | `SysExtended` 0x30-0x32 |
| Mach kernel (9) | 0x90-0x98 `Mach*` | `MachExtended` 0x00-0x21 |
| Futex (2) | 0xB0/0xB1 | `SyncExtended` 0x00/0x01 |
| Spinlock (4) | 0xB2-0xB5 | `SyncExtended` 0x10-0x13 |
| Waitgroup (6) | 0xB6-0xBB `Waitgroup*` | `SyncExtended` 0x30-0x35 |
| Atomic RMW (1) | 0xBC `AtomicRmw` | `SyncExtended` 0x40 |
| TLS slots (5) | 0x59-0x5D `TlsSlot*F` | `SyncExtended` 0x50-0x54 |
| CBGR allocator (5) | 0xA0-0xA4 `CbgrAlloc*` | `CbgrExtended` 0x60-0x64 |
| CBGR bridge (6) | 0xA5-0xAA + 0xA3 | `CbgrExtended` 0x63, 0x65-0x6A |
| Pointer/deref (12) | 0x60-0x6C `DerefRaw`/`Ptr*` | `MemExtended` 0x10-0x1B |
| Raw load/store (6) | 0x53-0x58 `Raw*` | `MemExtended` 0x20-0x25 |
| Byte/typed arrays (8) | 0x49-0x4E/0x5E/0x5F | `MemExtended` 0x30-0x37 |
| Static-mut cells (2) | 0x52/0x6A `StaticMut*` | `MemExtended` 0x40-0x41 |

What legitimately REMAINS in `FfiExtended` (33 variants — the honest
FFI surface): symbol resolution (`LoadSymbol`/`GetLibrary`/
`IsSymbolResolved`), the `CallFfi*` calling conventions, marshalling
(`*ToC`/`*FromC`), errno handling, the C heap (`CAlloc`..`CMemcmp`),
callbacks and `StructFieldAddr` (C-struct field addressing).  The Mem
wave completed the re-homing: pointer/raw/array/static-mut all live
in `MemExtended`, and the meta-pin `band_exceptions` list is empty.

## Discipline

* One family = one gateway = one sub-op enum = one Tier-0 handler
  file = one `lower_*` function.  A sub-op that "would fit" a
  neighbouring pocket goes in ITS OWN family's reserved space
  instead.
* Every enum owns `from_byte`/`to_byte` and (for the metadata-bearing
  ones) a `meta()` single source of truth with count pins in
  `instruction.rs` tests — bumping a pin is the conscious signal a
  variant landed.
* Byte allocation across sessions: claim the byte in the pool task
  BEFORE emitting it anywhere.
* The executable coherence gate for any migrated family is a
  differential spec that CALLS the surface live (see
  `vcs/specs/L1-core/mem/cbgr_full_surface.vr` for the pattern).
