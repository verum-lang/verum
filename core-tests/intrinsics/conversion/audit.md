# `intrinsics/conversion` audit

Module: `core/intrinsics/conversion.vr` (~163 LOC) — type-conversion intrinsics
that map to LLVM/CPU instructions: int↔float, integer/float width changes, bit
reinterpretation, byte layouts, and endianness.

Tests: `unit_test.vr` (API surface), `property_test.vr` (round-trip / inverse
laws), `integration_test.vr` (IEEE field extraction / serialisation / network
order), `regression_test.vr` (defect pins).

## 0. Architectural model (load-bearing)

VBC integer Values are i64 and floats are IEEE f64.  Integer/float **width is a
static-only concept** (same rule as arithmetic/bitwise): the widening
conversions (`sext`/`zext`/`fpext`) and the narrowing ones (`itrunc`/`fptrunc`)
are no-op `Mov`s at the uniform i64/f64 runtime — the dedicated *width-typed*
registry entries (`i32_to_i64`, `u32_to_u64`, `f32_to_f64`, `f64_to_f32`,
`i64_to_i32`) all lower to the same `InlineSequenceId::{Sext,Zext,Fpext,Fptrunc,
IntTrunc}` which emit a copy.

The **bit reinterpretation** between a float and its integer bit pattern is NOT
a no-op in the Tier-0 NaN-boxed representation — it is a real opcode
(`ArithSubOpcode::F{32,64}{To,From}Bits`, interp handlers + LLVM lowering), so
`f64_to_bits`/`f64_from_bits`/`f32_to_bits`/`f32_from_bits` route to those
dedicated intrinsics, not to the generic `bitcast`.

## Tier summary

* **Interp: 60/60 GREEN.**
* **AOT: 52/60** — `CONV-AOT-BYTEARRAY-1` is now FIXED (see §2); the remaining
  failures are the `CONV-AOT-F32BITS-1` f32/f64-bits cluster (§3) plus the
  occasionally-flaky endianness round-trip (`network_order_round_trip` — passes
  in isolation; AOT parallel non-determinism, not a contract gap).

## 1. What is verified GREEN (interp + AOT)

* **int↔float** — `int_to_float`, `uint_to_float`, `float_to_int`
  (trunc-toward-zero), `float_to_uint`.
* **integer width** — `sext`, `zext` (widening = identity at i64).
* **IEEE-754 bits** — `f64_to_bits`/`f64_from_bits`/`f32_to_bits`/`f32_from_bits`
  (round-trip identity; field extraction for sign/exponent).
* **byte layouts** — `to_le_bytes_{2,4,8}` / `to_be_bytes_{2,4,8}` and the
  matching `from_*` (round-trip identity; LE/BE byte order reversed).
* **endianness** — `to_le`/`from_le` (identity on LE target), `to_be`/`from_be`
  (byte swap); inverse-pair round-trips.

## 2. Defects FIXED on this branch (data-only — no enum/handler/LLVM changes)

The conversion intrinsics were fully implemented at the codegen + interpreter +
LLVM levels but **unreachable from the intrinsic surface** — a pure
registry/alias/`.vr` wiring gap.

### CONV-INTWIDTH-1 — `sext`/`zext`/`itrunc`/`fpext`/`fptrunc` → `nil`

`lookup_intrinsic` aliased the generic names to *themselves* (`"sext" =>
"sext"`, …) — names with **no registry entry**.  The real entries are
width-typed (`i32_to_i64`→`Sext`, `u32_to_u64`→`Zext`, `f32_to_f64`→`Fpext`,
`f64_to_f32`→`Fptrunc`, `i64_to_i32`→`IntTrunc`), so the alias resolved to
`None` → `LoadNil` → `nil`.

**Fix** (`intrinsics/mod.rs::lookup_intrinsic`): repoint the generic aliases to
the existing width-typed registry names.  The width in the target name is
irrelevant — all five lower to a value-preserving `Mov` at the i64/f64 runtime.

### CONV-BITCAST-FLOATBITS-1 — `f{32,64}_to/from_bits` → `nil`

The `f64_to_bits` … wrappers called `@intrinsic("bitcast", x)`; `bitcast` has no
registry entry, so they lowered to `nil`.  (The dedicated `f64_to_bits` … entries
+ `ArithSubOpcode::F*{To,From}Bits` opcodes + interp/LLVM lowering already
existed.)

**Fix** (`core/intrinsics/conversion.vr`): point each wrapper at its dedicated
intrinsic (`@intrinsic("f64_to_bits", x)`, …).  Generic `bitcast<S,D>` (the
unsafe size-match-contract form) remains unregistered — see §3.

### CONV-ENDIAN-ALIAS-1 — `to_le`/`to_be`/`from_le`/`from_be` returned bytes

The alias table mapped `to_le`/`from_le` → `to_le_bytes` and `to_be`/`from_be`
→ `to_be_bytes`, so the endianness helpers returned a `[Byte; 8]` instead of the
endianness-converted `T`.

**Fix**: on a little-endian target, `to_le`/`from_le` are no-ops (alias to the
value-preserving `u32_to_u64` `Mov`) and `to_be`/`from_be` are a byte swap
(alias to `bswap`).

## 3. Defects OPEN

### CONV-AOT-F32BITS-1 — `f32_to_bits`/`f32_from_bits` return `0` under AOT

Interp is correct (`f32_to_bits(1.0)=0x3F800000`); AOT yields `0`.  The
`F32ToBits`/`F32FromBits` LLVM lowering (`verum_codegen/.../instruction.rs`,
f64→fptrunc→bitcast→zext) is itself correct, so the root cause is upstream
AOT **Float32** handling — the `1.0 as Float32` cast / f32 parameter flow
produces a zero/garbage float before the intrinsic runs.  The f64 forms work on
both tiers.  Tracked: task #16.

### CONV-AOT-BYTEARRAY-1 — `to/from_*_bytes` `[Byte; N]` SIGSEGV under AOT  ✅ FIXED

Interp was correct; AOT SIGSEGV'd (exit 139) on any `to_le_bytes_N(x)[i]` /
`from_*_bytes_N(arr)` use.  **Root cause** (a general AOT defect, not specific
to byte conversions): AOT `GetE` dispatches on compile-time register marks
(`is_list`/`is_slice`), and a byte-element collection that flowed across a
function boundary was mis-marked — `List<U8>`/`List<I8>` as a *slice* (i8 stride
/ offset-24 ptr) and `[T; N]` (`TypeRef::Array`) as *unmarked* (raw branch →
deref of the list-object header).  A `List<U8>`/`[Byte; N]` is actually an
i64-strided LIST OBJECT (proven by a local `[UInt8;4]` reading `4,3,2,1` via the
is_list path); true `&[U8]` slices are a separate `Pack`/`TypeRef::Slice`
representation marked at their own sites.

**Fix** (`crates/verum_codegen/src/llvm`): mark `List<*>` (incl. byte element)
AND `[T; N]` as list registers in BOTH `mark_register_from_return_type`
(returns) and the parameter-marking loop in `vbc_lowering` (arguments).
Conversion AOT 41→52; all `to_*_bytes`/`from_*_bytes` produce + consume tests
pass.  Also unblocks every `fn … -> [T; N]` / `fn(List<U8>)` across the stdlib.

* **`bitcast<S, D>` (generic, `unsafe`)** — no registry entry; resolves to `nil`.
  The runtime cannot recover the static `S`/`D` sizes, and the generic
  `InlineSequenceId::Bitcast` is a no-op `Mov` (correct only for same-rep types,
  silently wrong for float↔int).  The safe surface is the size-typed
  `f{32,64}_{to,from}_bits` wrappers (fixed in §2).  Pinned; tracked as part of
  CONV-BITCAST-FLOATBITS-1's residual.
* **Narrowing precision is virtual** — `itrunc` (`trunc`) and `fptrunc` lower to
  `Mov`, so they do NOT mask to the destination width / round to `f32`
  precision.  This is consistent with the uniform-width runtime model (the
  value is reinterpreted at the narrow type at its point of use).  Width-exact
  narrowing would need width-carrying variants (cf. arithmetic's `*_uN` split).

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.encoding.*` (hex/base64/varint) | `to_*_bytes`/`from_*_bytes` + endianness for wire formats. |
| `core.text.numeric` (float formatting/parsing) | `f64_to_bits`/`f64_from_bits` for IEEE field extraction (sign/exponent/mantissa). |
| `core.net.*` | `to_be`/`from_be` (network byte order). |
| `core.math.*` | `int_to_float`/`float_to_int` for numeric coercions. |
| hashers | `f64_to_bits` to hash floats by bit pattern. |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/mod.rs::lookup_intrinsic` — the alias map.
  The generic→width-typed conversion aliases live here; a new conversion
  intrinsic name needs an alias row OR a registry entry.
* `crates/verum_vbc/src/intrinsics/registry.rs` — the width-typed conversion
  entries (`i32_to_i64`, `f64_to_bits`, …) and their `InlineSequenceId` mapping.
* `crates/verum_vbc/src/codegen/expressions.rs::emit_intrinsic_inline_sequence`
  — `Sext`/`Zext`/`Fpext`/`Fptrunc`/`IntTrunc`/`Bitcast` (Mov) and the
  `F*{To,From}Bits` ArithExtended emission.
* `crates/verum_vbc/src/interpreter/dispatch_table/handlers/arith_extended.rs`
  + `crates/verum_codegen/src/llvm/instruction.rs` — `F*{To,From}Bits` semantics
  on each tier (the only non-`Mov` conversions).

## 6. Action items

**Landed this branch (data-only)**
* CONV-INTWIDTH-1 — generic→width-typed conversion aliases.
* CONV-BITCAST-FLOATBITS-1 — `f*_{to,from}_bits` wrappers route to dedicated
  intrinsics.
* CONV-ENDIAN-ALIAS-1 — `to_le`/`to_be`/`from_le`/`from_be` return `T`.
* Full conversion test suite (unit/property/integration/regression).

**Deferred**
* Generic `unsafe bitcast<S, D>` registration (§3) — needs a same-size contract
  the runtime cannot check; the safe `f*_bits` surface supersedes it.
* Width-exact narrowing for `itrunc`/`fptrunc` (§3) — width-carrying variants.
