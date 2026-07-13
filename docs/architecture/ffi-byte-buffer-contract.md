# FFI Byte-Buffer Contract

Pinned architectural rules for how byte buffers cross the Verum ↔ C ABI
boundary (Tier-0 interpreter and Tier-1 AOT). Every socket/`recvfrom`/
`getsockname`/`read`/`write` OUT-parameter and every `sockaddr` IN-parameter
depends on these rules. The whole **B1 net-stack cascade** (B1 → B1b → B1c →
B1d) was a series of facets of *this one model being undocumented*.

A re-introduction of any forbidden pattern is a regression. The witnesses
named below are the canary; this document explains *why*.

Reproducible witness (macOS, current codegen):
`scratchpad probe pg_udp_roundtrip.vr` — full inline UDP round-trip through
raw libSystem `socket`/`bind`/`getsockname`/`sendto`/`recvfrom` using the
patterns below → `sent=4`, `recv=4`, payload bytes `112,105,110,103`
(`"ping"`), `a_port` a real ephemeral port → `ROUNDTRIP_OK`.

---

## 0. TL;DR — the five rules

1. A byte buffer destined for a C `void*` MUST be a **packed `[Byte; N]`**
   (annotated), never a bare `[0_u8; N]`.
2. Hand it to the FFI as a **subslice `&buf[..]`** or an **element address
   `&buf[0]`**, never as a bare `&buf` / `&mut buf`.
3. `.as_ptr()` / `.as_mut_ptr()` return the packed data pointer **only on a
   subslice `FatRef`** (a `&[Byte]` param), never on a raw array object.
4. `safe_*` wrappers take `&[Byte]` / `&mut [Byte]` and call `.as_mut_ptr()`.
   **Never `transmute` the slice value** to a pointer.
5. `sockaddr_in` is laid out per-platform: BSD/macOS `{ sin_len@0,
   sin_family@1 }`, Linux/Windows `{ sin_family@0 (LE 16-bit) }`.
   `serialize_socket_addr` / `deserialize_socket_addr` must agree with the
   target's kernel on every field offset.

---

## 1. Two byte representations — packed vs NaN-boxed

> **A `[Byte; N]` *with a type annotation* is a PACKED `TypeId::U8` object:
> `N` contiguous bytes at `OBJECT_HEADER_SIZE`. A bare `[0_u8; N]` /
> `[0; N]` is a NaN-boxed generic `List` (`TypeId::LIST`, 512): 8-byte-
> strided `Value`s. Only the packed form is contiguous ABI bytes.**

* Packed path: `statements.rs` `detect_byte_array_type(ty)` (keys on the
  **type annotation**) → `NewByteArray` (`FfiExtended` sub-op `0x49`) →
  `TypeId::U8`, zeroed contiguous bytes, `mark_byte_array_var`.
* NaN-boxed path: `compile_array` (`expressions.rs`, both `List` and
  `Repeat` arms) → `NewList`. Element `Value`s are 8-byte-strided; the low
  byte of `Value[i]` is `byte[i]`, but `byte[i+1]` lives 8 bytes on, not 1.

Consequence: `let mut b = [0_u8; 128]` is **not** a byte buffer a syscall can
write into — the kernel writes 16 contiguous bytes over `Value[0..1]` and
corrupts the NaN-boxing. Always annotate: `let mut b: [Byte; 128] = [0; 128]`.

## 2. The reserved-stride `FatRef`

> **A slice over a container carries the element STRIDE in `FatRef.reserved`:
> 1/2/4/8 for packed `U8`/`U16`/`U32`/`U64` (and `BYTE_LIST`), 0 for a
> NaN-boxed `Value` array. The data pointer is `base + OBJECT_HEADER_SIZE`
> (packed) or `backing + OBJECT_HEADER_SIZE` (list).**

Canonical constructor: `container_to_slice_fat_ref`
(`interpreter/dispatch_table/handlers/cbgr.rs:949`).
`SliceGet` / `Unslice` / `SliceLen` all honour `reserved` as the stride; a
fixed `*const Value` read would truncate a byte slice to the first element's
tag bits.

## 3. `.as_ptr()` / `.as_mut_ptr()` — subslice only

> **`Unslice` (`cbgr.rs:1615`, emitted by `as_ptr`/`as_mut_ptr` at
> `expressions.rs:10311`/`10336`) returns `fat_ref.ptr()` for a `FatRef`
> and the `BYTE_SLICE` payload ptr for a byte view — but for a RAW array
> object (`is_ptr`, no `FatRef`) it returns the OBJECT BASE (the header),
> NOT the data.**

Therefore `packed_array.as_mut_ptr()` (called directly on a `[Byte; N]`
variable) hands the callee the `ObjectHeader`; a `getsockname` write then
lands on the header and corrupts `header.size` (observed: length reads back
as `16` = `sin_len`, then any subslice/index throws
"index `<ptr>` for list of length 16"). The subslice form is mandatory:

```verum
// WRONG — as_mut_ptr on the raw array → object header
c_getsockname(fd, buf.as_mut_ptr() as &unsafe Byte, len)
// RIGHT — as_mut_ptr on a &mut [Byte] subslice param → packed data ptr
fn get_name(fd: Int, s: &mut [Byte], l: &mut UInt32) -> Int {
    c_getsockname(fd, s.as_mut_ptr() as &unsafe Byte, l)   // s came from &mut buf[..]
}
```

The stdlib `safe_getsockname`/`safe_getsockopt`/`safe_recvfrom` are correct:
they take `&mut [Byte]` params (subslice `FatRef`s) and call `.as_mut_ptr()`.

## 4. `safe_*` wrappers — never `transmute` the slice value

> **A `&[Byte]` value is a `FatRef` (`{ptr, gen, len, reserved}`).
> `transmute(slice)` reinterprets those struct bits as a pointer — it is
> NOT the data pointer. Use `.as_mut_ptr()`.**

This was B1d's primary bug: `safe_recvfrom` did `let p: &unsafe Byte =
transmute(buf)` and handed `recvfrom` the fat-ref struct bits, so the
datagram was written into the descriptor, not the buffer (recv silently
returned a zero-payload). Fixed to `buf.as_mut_ptr()` — matching the
already-correct `safe_getsockname`. (`core/sys/darwin/libsystem.vr`.)

## 5. sockaddr layout is per-platform

> **BSD (macOS/FreeBSD) `struct sockaddr_in` is `{ uint8_t sin_len@0,
> sa_family_t sin_family@1, ... }`. Linux/Windows put the 16-bit
> `sin_family` at offset 0 (little-endian → low byte @0). Port (bytes 2–3,
> big-endian) and address (4–7) are identical.**

`serialize_socket_addr` writes `sin_len=16, AF_INET` on macOS and
`AF_INET, 0` on Linux. `deserialize_socket_addr` MUST read the family from
the target's own offset — reading `[0]` on macOS yields `sin_len` (16), not
`AF_INET`. Both functions in `core/net/udp.vr` and `core/net/tcp.vr`.

---

## RESOLVED: SLICE-COERCE-ARR-1 (task #24, commit 02e838a10) — bare `&arr` now unsizes

Rule 2's footgun is **fixed for byte arrays**: a bare `&arr` / `&mut arr` on a
`[Byte; N]` variable now lowers (in `compile_unary`) to the SAME `RefSlice`
(start 0, len = `Len(arr)`) as `&arr[..]` — a `reserved=1` packed `FatRef`, not
a raw array-object pointer. `recv_from(&mut buf)` (bare) works. Rust unsizes
`&[u8; N]` → `&[u8]` implicitly; Verum now matches for byte arrays. Verified:
`recv_like(&mut buf)`/`dump(&buf)` print; stdlib `recv_from(&mut buf)` →
`BARE_ROUNDTRIP_OK`; subslice forms + tcp `local_addr` unchanged.

The gate is `get_typed_array_elem_size == Some(1)` (byte arrays) — the minimal
non-regressive blast radius. A `&arr as &unsafe Byte` cast is unaffected (it
parses `&(arr as …)`, inner Cast not Path). FOLLOW-UP: broadening the gate to
`Some(_)` so typed `[T; N]` arrays also unsize is a clean coherence extension,
pending a check that typed arrays are tracked and their `&arr[..]` is verified.

A parallel coherence option: route `[0_u8; N]` (byte-literal `Repeat`)
through `NewByteArray` so every byte buffer is packed regardless of
annotation — gated on a full conformance run (it narrows `List<Byte>`
growable semantics).

## Related, non-fatal

* A C OUT `socklen_t*` (`&mut UInt32`) passed *through a Verum `&mut`
  parameter* does not write back to the caller's variable (mutation through
  a passed reference does not propagate one frame up). The net path is
  robust to it: `deserialize_socket_addr` reads fixed offsets, so a stale
  `addr_len` of 128 over a 16-byte sockaddr still parses IPv4 correctly.

## References

* B1d landing: commit `11a6aafb6` (`core/net/udp.vr`, `core/net/tcp.vr`,
  `core/sys/darwin/libsystem.vr`); register row B1d in
  `docs/architecture/tech-debt-register.md`.
* Packed alloc: `crates/verum_vbc/src/codegen/statements.rs` (`0x49`).
* Slice model: `crates/verum_vbc/src/interpreter/dispatch_table/handlers/cbgr.rs`
  (`container_to_slice_fat_ref:949`, `Unslice:1615`, `SliceGet:1667`).
* `ByteArrayElementAddr` (`&buf[0]`, requires `TypeId::U8`):
  `crates/verum_vbc/src/interpreter/dispatch_table/handlers/ffi_extended.rs:478`.
