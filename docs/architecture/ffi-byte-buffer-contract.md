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

## 0. TL;DR — the seven rules

1. A byte buffer destined for a C `void*` MUST be a **packed `[Byte; N]`**
   (annotated), never a bare `[0_u8; N]`.
2. Hand it to the FFI as a **subslice `&buf[..]`** or an **element address
   `&buf[0]`**. A bare `&buf` / `&mut buf` unsizes to the same thing
   since SLICE-COERCE-ARR-1 (byte arrays) and T1269 (every `[T; N]`) —
   the subslice remains the clearer spelling, not a requirement.
3. `.as_ptr()` / `.as_mut_ptr()` return the packed data pointer on a
   subslice `FatRef` **and, since T1276, on the array itself**. Both
   answered NULL at Tier 1 before that, for two different reasons.
4. `safe_*` wrappers take `&[Byte]` / `&mut [Byte]` and call `.as_mut_ptr()`.
   **Never `transmute` the slice value** to a pointer.
5. `sockaddr_in` is laid out per-platform: BSD/macOS `{ sin_len@0,
   sin_family@1 }`, Linux/Windows `{ sin_family@0 (LE 16-bit) }`.
   `serialize_socket_addr` / `deserialize_socket_addr` must agree with the
   target's kernel on every field offset.
6. **When C READS THE BYTES, a reference to a scalar or a record is
   never the right pointer.** `&val` on an `Int32` local is the address
   of a NaN-boxed register slot; `&rec` on a record is the address of
   its `ObjectHeader`. Serialise into a packed `[Byte; N]` and pass
   `&buf[..]`. The qualifier is load-bearing: when C only CARRIES the
   pointer and hands it back (`pthread_create`'s `void* arg`,
   `CreateThread`'s parameter), the object address is exactly right and
   the round trip works — those sites are correct and must not be
   "fixed". Ask what the callee DOES with the pointer, not what its
   type says.
7. **An FFI wrapper takes `&[Byte]` / `&mut [Byte]`, never `&unsafe
   Byte`.** Taking the raw pointer forces every CALLER to produce it,
   and the call site is the one position where `.as_ptr()` returns the
   object header (rule 3). Take the slice and produce the pointer
   inside.

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

### 1b. The two TIERS do not allocate it the same way (T1192, T1220)

The paragraph above describes the INTERPRETER's packed object and reads
as if it were universal. It is not, and two rules of this contract lean
on the difference.

| | interpreter | AOT |
|---|---|---|
| allocation | `heap.alloc` with the U8 heap type id | `verum_cbgr_allocate(bytes)` |
| what precedes the data | an **ObjectHeader**, carrying a `type_id` and a size | a 32-byte **AllocationHeader**, carrying `{base_offset, total}` |
| pointer handed out | the object base (data at `OBJECT_HEADER_SIZE`) | the **user address** — the header is BEHIND it |
| filled with the init value | yes | **no, until T1220** |

Two consequences, each one a measured defect rather than a caution:

* **Nothing reachable from an AOT byte-array pointer says how long it
  is.** Forward offsets 0, 24 and 32 all land in DATA, and the header
  that exists carries no `type_id` and no length. Every probe in
  `lower_len` was written against the interpreter's shape, so at Tier 1
  they read the array's own bytes and called the result a length —
  `[Byte; 16]` answered 16 at Tier 0 and **0** at Tier 1, rc=0, no
  diagnostic (T1192, fixed by answering from the frontend where `N` is
  part of the type).
* **"Zeroed contiguous bytes" is the interpreter's guarantee.** The AOT
  `NewByteArray` arm ignored the init operand entirely, so `[7; 4]` read
  as zeros. `[0; N]` was correct BY ACCIDENT — a fresh allocation
  already reads zero — which is why the defect was invisible in the
  shape that uses it most, a zeroed scratch buffer (T1220).

* **The frontend answer had to be given at EVERY site that asks, and
  T1192 gave it at one.** `a.len()` folded to a constant; `&a[..]` did
  not, and its omitted end still lowered to `Len { arr }` — the same
  probe of the same headerless block, now feeding the FatRef's LENGTH.
  So the count arrived as 0 inside every callee taking `&[Byte]`, which
  is a correct read of a correctly built FatRef that was built with a
  zero. Measured in one binary: `f(&a[0..12])` = 12 and `f(&a[..])` = 0,
  literal bounds emitting no `Len` at all. The typed half was never
  covered — `[Int; 5]` answered **0** even for a plain local `.len()`,
  because only byte arrays recorded a count. T1269 records the count for
  every `[T; N]`, including an array PARAMETER (`&[Byte; 12]`, whose N is
  in its own declaration), and folds it at all six asking sites:
  `.len()`, `is_empty()`, the free `len(x)`, `&a[..]`, `a[..]` and a bare
  `&a`.
* **A `[T; N]` PARAMETER is marked as a LIST register at Tier 1**
  (`vbc_lowering.rs`, the `TypeRef::Array` arm — the descriptor erases
  `&`), and a caller writing `&a` hands over a 24-byte slice cell
  `{data@0, len@8, elem@16}`. The list arm of `lower_len` read `@24` and
  `@32` — the second past a cell's allocation — so `&[Byte; 12]`
  answered 0 while its `&[Byte]` twin answered 12 from the SAME FatRef.
  That arm now goes through `emit_container_view`, the one authority
  over {cell | stamped-pack | unstamped-list}, which branches rather
  than selecting; a real list is unaffected because its word 0 is an
  `ObjectHeader` type_id, far below the heap floor. `VERUM_NO_LEN_CELLVIEW`
  restores the old select.

Neither is a reason to prefer the NaN-boxed form: the packed form is
still the only contiguous ABI bytes. It is a reason not to reason about
a Tier-1 byte buffer from the interpreter's layout.

Consequence: `let mut b = [0_u8; 128]` is **not** contiguous ABI bytes — the
kernel would write 16 contiguous bytes over `Value[0..1]` and corrupt the
NaN-boxing. Always annotate: `let mut b: [Byte; 128] = [0; 128]`.

**But that consequence does NOT reach every call, and the difference is
measured, not argued.** On Tier 0 an unannotated buffer that reaches a
syscall *through a slice* survives, because the interpreter's FFI marshaller
packs it into scratch for C and writes the result back element-by-element
(`interpreter/dispatch_table/handlers/ffi_extended.rs:1641`, the
`TypeId::LIST` branch). Three independent measurements, all healthy:

| site | form | result |
|---|---|---|
| `float_to_string(x, &mut buf, n)` | Verum callee | `f"{x:.2}"` → `3.14` |
| `src.read(&mut buf)` (chunked copy) | protocol → syscall | 14 bytes, identical |
| `sys_read(fd, &mut tmp)` (stdin) | `safe_read` → `read(2)` | the real line, right length |

So the rule to enforce is rule 3, not this one: the writeback repairs a
STRIDE, and nothing can repair a pointer that was already wrong — which is
what `.as_ptr()` on the array variable produces. Annotating remains correct
hygiene (it avoids a per-call pack/unpack) and is required for the AOT path,
which does **not** carry this writeback and is not measured here.

**MEASURED 2026-09-11 (T1403) — "not measured here" is no longer the
state of knowledge, and what it hid is worse than a gap in scope.** The
AOT's missing write-back is not benign. A scalar `&mut` argument is
STRIPPED by the emitter (`codegen/expressions.rs:7856`, so Tier 0's
marshaller can allocate storage for the value) and Tier 1 then
`inttoptr`s that value into a pointer. What the callee receives is
whatever the caller's variable held:

| caller's slot | what C gets | what happens |
|---|---|---|
| `0` | `NULL` | legal for `time(NULL)` — writes nothing, silently |
| `123456` | address `123456` | SIGSEGV (measured, exit 139) |

and a `&mut <record>` hands over the object BASE, so the callee writes
over the 24-byte header while the caller reads fields at `+24`.

Ordinary stdlib surfaces are affected, not corner cases:
`core.io.fs.metadata` and `exists` SIGSEGV under `verum build` while
correct under `verum run`; `safe_clock_gettime` returns `Ok` with
`tv_sec = 0`.

**This also re-reads the gate history in the paragraph below.** The first
edition's `&mut name` findings — "33 findings, 30+ of them healthy code"
— were healthy AT TIER 0 and wrong at Tier 1. They were not false
positives; they were TIER-CONDITIONAL ones, and the reason that could not
be seen is that the Tier-1 half of the measurement did not exist. A gate
phrased "correct under the interpreter, returns zeros when built" would
have been right.

The gate is deliberately NOT re-widened yet: `core/` itself holds six
such sites (`libsystem.vr:1382,1498,1532,1570`, `io/fs.vr:684,699`), so
widening it today reports the stdlib rather than protecting it. The fix
comes first; T1403 carries the six-shape acceptance table.

Gate: `scripts/ci/check_a_byte_buffer_crosses_ffi_packed_and_sliced.sh`
reports rule 3 only. Its first edition also reported `&mut name` on the
strength of the paragraph above and produced 33 findings, 30+ of them
healthy code.

## 2. The reserved-stride `FatRef`

> **A slice over a container carries the element STRIDE in `FatRef.reserved`:
> 1/2/4/8 for packed `U8`/`U16`/`U32`/`U64` (and `BYTE_LIST`), 0 for a
> NaN-boxed `Value` array. The data pointer is `base + OBJECT_HEADER_SIZE`
> (packed) or `backing + OBJECT_HEADER_SIZE` (list).**

Canonical constructor: `container_to_slice_fat_ref`
(`interpreter/dispatch_table/handlers/cbgr.rs`, the function of that name — the line number this anchor used to carry had drifted onto an unrelated bridge-store arm, and `SliceGet` with it).
`SliceGet` / `Unslice` / `SliceLen` all honour `reserved` as the stride; a
fixed `*const Value` read would truncate a byte slice to the first element's
tag bits.

## 3. `.as_ptr()` / `.as_mut_ptr()` — both spellings, since T1276

> **`Unslice` returns `fat_ref.ptr()` for a `FatRef` and the `BYTE_SLICE`
> payload ptr for a byte view. For a RAW array object it USED TO return
> the OBJECT BASE (the header), not the data — and at Tier 1 it returned
> NULL. Both spellings now address the array's own first byte.**

### RESOLVED (T1276): the bare form answers the data pointer

Measured 2026-09-08, and BOTH spellings were broken at Tier 1 — the one
this section called wrong and the one it called right:

    buf.as_ptr()          tier 0: 30787343192   tier 1: 0
    (&buf[..]).as_ptr()   tier 0: 30787343192   tier 1: 0

Two separate causes, one per spelling, each hidden behind the other
while both answered zero:

* The SUBSLICE form reached `Unslice` (CBGR 0x04), which guessed the
  pointer's offset statically — a slice-marked register was assumed to
  be a Pack and read at 24, while the canonical producer emits a 24-byte
  cell `{data@0, len@8, elem@16}` whose 24 is past the allocation. The
  neighbouring `SliceLen` (0x05) had retired the identical guess long
  before, and 0x08/0x09 with it; 0x04 was the last one guessing. It now
  goes through the same `emit_container_view`, which reproduces the old
  answer exactly where the old answer was right (a genuine Pack still
  reads 24). A Text register keeps the flat read deliberately: its word
  0 is a data pointer only while it is non-empty.
* The BARE form never reached `Unslice` at all. A packed array carries
  the type name `List`, so "the receiver defines its own `as_ptr`" held
  and the call dispatched to `List.as_ptr`, which reads `self.ptr` at
  `LIST_PTR_OFFSET` — 24 bytes into a HEADERLESS allocation. Named by a
  `VERUM_AOT_TRACE_CALLM` line rather than by reading code:
  `method="List.as_ptr" recv_r4 type_name=Some("List")`. It now lowers
  to the same `RefSlice`+`Unslice` pair that `&buf[..].as_ptr()` emits,
  using the N and the stride the frontend already holds.

Pinned by `vcs/specs/L0-critical/vbc/array-as-ptr-addresses-its-data.vr`,
which proves the pointer addresses DATA without dereferencing it: the
step from `&buf[..]` to `&buf[1..]` is exactly one byte, which a
header-addressing pointer cannot satisfy. (It avoids a raw read on
purpose — `ptr_read::<Byte>` reads two bytes at Tier 0 today, so a
dereference would diverge for an unrelated reason.)

WHAT THE NULL COST: `open(2)` answered errno 14 (EFAULT) for every
path-taking syscall on darwin — nine callers go through `copy_path_nul`
— so `File.create` failed and nothing could be written at Tier 1
(T1192). `File.create` now succeeds; the file layer's remaining Tier-1
failure is the `FileDesc(Int)` newtype reading its `.0` as an address,
which is a different row.

### The history this section was written for

`packed_array.as_mut_ptr()` (called directly on a `[Byte; N]` variable)
used to hand the callee the `ObjectHeader`; a `getsockname` write then
landed on the header and corrupted `header.size` (observed: length reads
back as `16` = `sin_len`, then any subslice/index throws
"index `<ptr>` for list of length 16"). The subslice form was mandatory
and is still the clearer spelling in a signature that takes `&mut [Byte]`:

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
Since T1276 the bare form is correct too, so this is a style preference
rather than a rule — but a signature that says `&mut [Byte]` still
documents the intent better than one that says `&mut [Byte; N]`.

## 4. `safe_*` wrappers — never `transmute` the slice value

> **A `&[Byte]` value is a `FatRef` (`{ptr, gen, len, reserved}`).
> `transmute(slice)` reinterprets those struct bits as a pointer — it is
> NOT the data pointer. Use `.as_mut_ptr()`.**

This was B1d's primary bug: `safe_recvfrom` did `let p: &unsafe Byte =
transmute(buf)` and handed `recvfrom` the fat-ref struct bits, so the
datagram was written into the descriptor, not the buffer (recv silently
returned a zero-payload). Fixed to `buf.as_mut_ptr()` — matching the
already-correct `safe_getsockname`. (`core/sys/darwin/libsystem.vr`.)

## 4b. Scalars and records are not buffers (T1135)

> **`&scalar as &unsafe Byte` and `&record as &unsafe Byte` both hand C
> a heap address that is not the data.** A scalar local lives in a
> NaN-boxed register slot — eight tagged bytes where the callee reads
> four clean ones. A record's address is its `ObjectHeader`.

Measured 2026-09-04. `safe_setsockopt` took `optval: &unsafe Byte`, so
each of its four callers produced the pointer itself, and all four
produced the wrong one in three different shapes:

```verum
safe_setsockopt(fd, level, optname, &val as &unsafe Byte, 4)          // scalar
safe_setsockopt(fd, SOL_SOCKET, optname, timeval.as_ptr() as ..., 16) // raw array (rule 3)
safe_setsockopt(fd, IPPROTO_IP, IP_ADD_MEMBERSHIP, &mreq as ..., 8)   // record
```

`core/sys/linux/syscall.vr` mirrors all four. The consequence is not a
crash: on both platforms `SO_NOSIGPIPE`, `SO_KEEPALIVE`, `SO_BROADCAST`,
`SO_SNDBUF`, `SO_RCVBUF`, socket timeouts and IPv4 multicast join/leave
wrote header bytes into the option and reported success.

**Only the scalar form fails loudly**, because the marshaller has no
conversion for a boxed register slot and refuses it — `unsupported
conversion from Unknown to Ptr`. The array and record forms produce a
pointer, so they are accepted. The loud failure is the lucky one, and
an acceptance that checks "no FFI error" cannot see the other two.

**So the acceptance for any SET across this boundary is a GET.** Set the
option, read it back, compare. A write that lands in the wrong bytes
reports success on every channel except the read.

The repair is rule 7: the wrapper takes `&[Byte]` and calls `.as_ptr()`
itself. `safe_getsockopt` already had that shape; the asymmetry between
get and set was the defect.

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

The gate WAS `get_typed_array_elem_size == Some(1)` (byte arrays) — the minimal
non-regressive blast radius. A `&arr as &unsafe Byte` cast is unaffected (it
parses `&(arr as …)`, inner Cast not Path).

**FOLLOW-UP DONE (T1269): the gate is now `Some(_)`, every `[T; N]`.** The
narrow gate was itself producing a wrong answer, not merely declining to
improve one: a bare `&ai` on a `[Int; 5]` did not unsize, so the callee got
the array value and `buf.len()` answered **1** — a third wrong answer beside
the 0 that `&ai[..]` gave. The stride is no longer assumed: it comes from the
same `get_typed_array_elem_size` the range arm already trusted and travels as
the fifth `RefSlice` operand. `VERUM_NO_TYPED_ARRAY_UNSIZE` restores the
byte-only guard, so both polarities are reachable from one binary.

A parallel coherence option: route `[0_u8; N]` (byte-literal `Repeat`)
through `NewByteArray` so every byte buffer is packed regardless of
annotation — gated on a full conformance run (it narrows `List<Byte>`
growable semantics).

### What "RESOLVED" did NOT cover, measured 2026-09-06 (T1213)

The COERCION was fixed; what the coerced value meant at Tier 1 was not.
`&arr` and `&arr[..]` both produced a `RefSlice`, and AOT then indexed
that slice EIGHT bytes at a time:

    let mut arr: [Byte; 8] = [0 as Byte; 8];
    arr[0] = 65 as Byte;  arr[1] = 66 as Byte;
    arr[1]              tier 0: 66   tier 1: 66
    (&arr[..])[1]       tier 0: 66   tier 1:  0

Width predicted before the run and matched to the bit: `[Byte; 8]`
filled 1..8 read `0x0807060504030201` at index 0.

**The stride was being decided by the array's own CONTENT.** The AOT
`RefSlice` arm classified its source by reading the source's first word
as a `type_id` — and for a packed `[Byte; N]` that word is the DATA.
Three arrays differing only in byte 0 took three arms and gave three
different wrong answers; the `TypeId.U8` and `TypeId.U16` arms compute
`data = src + OBJECT_HEADER_SIZE` on a pointer that already points at
the data, reading 24 bytes past a 16-byte array. Byte buffers hold
input, so the steering value is routinely external.

Fixed in `92482af33`: `RefSlice` carries the declaration's stride as a
fifth operand and AOT skips the runtime classification when it is
present. Two gaps remain and are tracked on the debt register's
A-DATASTRIDE row — the hint is keyed by variable NAME, so `self.buf[..]`
still probes; and the WRITE side is a separate arm.

THE LESSON FOR THIS DOCUMENT: "RESOLVED" was written about the
representation the FRONTEND emits. The gate that guards this contract,
`check_a_byte_buffer_crosses_ffi_packed_and_sliced.sh`, says in its own
header "Tier 1 / AOT is not measured", and `core-tests/INVENTORY.md` had
carried the symptom as `#48 slice-elemsize` since July. A resolution
claimed at one tier should say which tier.

Regression guard:
`vcs/specs/L0-critical/vbc/byte-array-slice-reads-one-byte.vr`
(differential, tiers 0 and 1; verified to fail six of its seven lines
on the unfixed compiler).

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
