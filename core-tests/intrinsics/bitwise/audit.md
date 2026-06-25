# `intrinsics/bitwise` audit

Module: `core/intrinsics/bitwise.vr` (~138 LOC) — bitwise intrinsics that map
directly to LLVM / CPU instructions: logical ops, shifts, rotations, funnel
shifts, bit counting, and bit manipulation.

Tests: `unit_test.vr` (API surface), `property_test.vr` (Boolean-algebra +
bit-manip laws), `integration_test.vr` (flag sets / endianness / hashing /
bitfields / pow2), `regression_test.vr` (defect pins).

## 0. Architectural model (load-bearing)

VBC integer Values are **untyped i64 at runtime** (same model as
`intrinsics/arithmetic`).  The **generic** forms (`bitand`/`bitor`/`bitxor`/
`bitnot`/`clz`/`ctz`/`popcnt`/`bswap`/`bitreverse`/`byte_swap_bits`) are
therefore **64-bit-natural**.  Narrow-width semantics live in the dedicated
type-specific intrinsics (`clz_u32`/`ctz_u32`), whose registry entry bakes the
width adjustment into the emitted VBC sequence.

### Two dispatch surfaces (the root cause of every §2 defect)

A bitwise call reaches an opcode by one of two paths:

1. **Call-site bare-name intercept** — `compile_builtin_call`
   (`codegen/expressions.rs`) intercepts a bare `bitand(a, b)` when all args
   are primitive-numeric and lowers it inline via `lookup_intrinsic("bitand")`.
2. **`@intrinsic` body dispatch** — the stdlib wrapper body
   `fn bitand<T>(a, b) { @intrinsic("and", a, b) }` resolves the *LLVM-canonical*
   name (`"and"`) through `lookup_intrinsic` → `compile_intrinsic_call`.

The registry historically held only the **wrapper** names (`bitand`/`bitnot`)
— so the **body** names (`and`/`not`/`lshr`/`ashr`) were unresolved.  Logical
ops with an intercept arm (`bitand`/`bitor`/`bitxor`) worked anyway; ops
without one (`bitnot`) fell through to their body, hit `lookup_intrinsic("not")
→ None`, and emitted `LoadNil` → **`nil`**.  Per the intrinsic-dispatch
contract the **body `@intrinsic` name is authoritative**, so the fix registers
the canonical names rather than relying on the intercept.

## 1. What is verified GREEN (interp + AOT)

**Both tiers via the test harness:** interp **127/127**, AOT **127/127**
(`verum test --aot`).  AOT was unblocked by the systemic `load_project_modules`
fix (§2 SYSTEMIC-AOT-EAGER-CORE-1).


* **Logical** — `bitand` `bitor` `bitxor` `bitnot` (incl. involution, De
  Morgan, distributivity, idempotence/identity/annihilator laws).
* **Shifts** — `shl`, `shr` (arithmetic), `lshr` (logical/zero-fill),
  `ashr` (arithmetic/sign-extend).
* **Rotations** — `rotl` `rotr` (inverse pair, popcnt-invariant) and the
  3-operand funnel shifts `fshl` `fshr`.
* **Bit counting** — `clz` `ctz` `popcnt` at 64-bit; `leading_ones`
  `trailing_ones`; width-correct `clz_u32`/`ctz_u32`; `clz_u64`/`ctz_u64`/
  `popcnt_u64`/`popcnt_u32`.
* **Bit manipulation** — `bswap` `bitreverse` `byte_swap_bits` (all three
  involutions; `bitreverse = bswap ∘ byte_swap_bits`; popcnt-invariant).

## 2. Defects FIXED on this branch (source + crate-level)

### BITWISE-CANONICAL-NAME-1 — `bitnot`/`and`/`or`/`xor` body names unresolved

`bitnot(x)` returned `nil`: no call-site intercept arm, and the body
`@intrinsic("not", x)` had no registry entry (`lookup_intrinsic("not") → None
→ LoadNil`).  The logical binary ops only *appeared* to work because their
intercept arms masked the same body-name gap.

**Fix**: register the LLVM-canonical bitwise names
(`and`→`Band`, `or`→`Bor`, `xor`→`Bxor`, `not`→`Bnot`) in
`intrinsics/registry.rs` as `DirectOpcode` strategies, so the **body** path
resolves independently of the intercept; and add `bitnot` to the call-site
intercept list (`codegen/expressions.rs`) so direct user calls lower inline
too.  Interp handlers (`handle_band/bor/bxor/bnot`) and LLVM `BitwiseOp::*`
already existed — cross-tier parity is immediate.

### BITWISE-SHIFT-LSHR-ASHR-1 — `lshr`/`ashr` had no registry entry

`lshr(x, n)` / `ashr(x, n)` returned `nil` — no registry entry at all (only
`shl`/`shr` existed).  The `Opcode::Ushr` (0x36, logical) opcode, its interp
handler (`handle_ushr`), and its LLVM lowering (`BitwiseOp::Ushr`) all existed
but were unreachable from the intrinsic surface.

**Fix**: register `lshr`→`DirectOpcode(Ushr)` (logical, zero-fill) and
`ashr`→`DirectOpcode(Shr)` (arithmetic, sign-extend); add the missing
`Opcode::Ushr` arm to `emit_intrinsic_direct_opcode` (it previously fell to
`_ => LoadNil`); add both to the call-site intercept.

### BITWISE-CLZ-CTZ-WIDTH-1 — `clz_u32`/`ctz_u32` ignored 32-bit width

`clz_u32(1)` returned `63` (the 64-bit count) instead of `31`.  Both the
source body (`@intrinsic("ctlz", x)`) and the `clz_u32` registry entry routed
to the generic 64-bit `InlineSequenceId::Clz`, so width was lost entirely.

**Fix**: new width-aware inline sequences emitted from the authoritative
`codegen/expressions.rs::emit_intrinsic_inline_sequence`:

* `ClzU32` = `clz64(x) - 32` — the 32-bit operand is zero-extended in the i64
  carrier, so the generic clz over-counts by exactly 32 (and `clz64(0)=64 → 32`,
  the correct width-32 zero result).
* `CtzU32` = `ctz64(x | (1<<32))` — the guard bit caps the count at 32 for an
  all-zero low word; any genuine low-32 trailing run (≤31) dominates it.

Registry `clz_u32`/`ctz_u32` re-pointed to `ClzU32`/`CtzU32`; the source bodies
now call `@intrinsic("clz_u32"/"ctz_u32", x)`.  Both compose existing 64-bit
ops, so they lower identically on Tier-0 and Tier-1.

### BITWISE-BYTE-SWAP-BITS-1 — `byte_swap_bits` unregistered

`byte_swap_bits(x)` (reverse the bit order *within* each byte, byte positions
unchanged) returned `nil` — no registry entry.

**Fix**: new `InlineSequenceId::ByteSwapBits` emitting `bswap(bitreverse(x))`.
A full bit-reverse equals a byte-order reverse composed with a per-byte
bit-reverse; `bswap` is its own inverse, so applying it to `bitreverse(x)`
cancels the byte-order flip and leaves the per-byte bit-reverse.  Composed from
two existing unary opcodes — no new dispatch arm on either tier.

### SYSTEMIC-AOT-EAGER-CORE-1 — `verum test --aot` compiled the whole `core` crate

Not a bitwise defect, but the blocker that kept this (and every other)
`core-tests` suite from passing under `--aot`.  `load_project_modules`
(`crates/verum_compiler/src/pipeline/loading.rs`) walks up to the first
`verum.toml` and eager-loads **all** sibling `.vr` files of that cog.  The test
harness writes its merged file into `core/target/test/` — inside the `core`
cog — so the walk resolved to the core root and pulled **every** core module
body (including ones unreachable from the test) into native codegen, which then
aborted on the first undefined stdlib leaf function (`sha512_digest`,
`fs_current_dir`, `equiv_inv_coherence_law`, …).  `verum run/build --aot` of the
same mounts on a file *outside* the cog compiled cleanly in ~15s.

**Fix**: in `BuildMode::Normal`, skip eager project-loading when the project
root is the stdlib `core` cog (`[cog] name == "core"`) — core is served by the
embedded precompiled archive, so `mount core.*` resolves without compiling
core's source.  Bitwise AOT suite: 0/127 (4407s, all undefined-fn aborts) →
**127/127 GREEN (322s)**.

## 3. Defects OPEN

None specific to this module.  The cross-cutting
`INTRINSIC-NESTED-CALL-DISPATCH-1` (arithmetic audit §2.6 — nested
intrinsic-over-intrinsic dispatch) is avoided here by `let`-splitting composed
expressions in `property_test.vr`; it is not re-pinned in this suite.

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.base.primitives` | backs `UInt*`/`Int*` bit-operator methods (`&`, `\|`, `^`, `~`, `<<`, `>>`, `count_ones`, `leading_zeros`, …). |
| `core.math.bits` | re-exports `clz`/`ctz`/`popcnt`/`rotl`/`rotr`/`bswap` for the numeric bit-twiddling layer. |
| hashers (`nanoid`, `snowflake`, `collections.*`) | `rotl` + `bitxor` mixing; `bswap` for endianness; `popcnt` for sparse-set sizes. |
| `core.encoding.*` | `bswap` / `to_*_bytes` endianness conversion; bitfield extract via `lshr` + `bitand`. |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/registry.rs` — the intrinsic table.  The
  canonical-name vs wrapper-name split (and the width-aware `clz_u32`/`ctz_u32`
  entries) live here.
* `crates/verum_vbc/src/intrinsics/mod.rs::lookup_intrinsic` — the LLVM-name
  alias map (`ctlz`→`clz`, `llvm.ctlz.iN`→`clz`, …).
* `crates/verum_vbc/src/codegen/expressions.rs` —
  `emit_intrinsic_direct_opcode` (the `Ushr` arm) and
  `emit_intrinsic_inline_sequence` (the authoritative `ClzU32`/`CtzU32`/
  `ByteSwapBits` width/composition emission), plus the bare-name intercept list.
* `crates/verum_vbc/src/intrinsics/{codegen.rs,lowering.rs}` — the
  intermediate-IR / MLIR lowering paths emit the *base* op for the new variants
  (approximation precedent: `Ilog2`/`Fshl`); the precise emission is the VBC
  path above.
* AOT parity: `crates/verum_codegen/src/llvm/instruction.rs` lowers every
  `BitwiseOp` (incl. `Ushr`); the composed inline sequences reduce to opcodes
  it already handles.

## 6. Action items

**Landed this branch (source + crate-level)**
* BITWISE-CANONICAL-NAME-1 — register `and`/`or`/`xor`/`not`; intercept `bitnot`.
* BITWISE-SHIFT-LSHR-ASHR-1 — register `lshr`/`ashr`; add `Ushr` emit arm.
* BITWISE-CLZ-CTZ-WIDTH-1 — width-aware `ClzU32`/`CtzU32` sequences.
* BITWISE-BYTE-SWAP-BITS-1 — `ByteSwapBits` = `bswap ∘ bitreverse`.
* SYSTEMIC-AOT-EAGER-CORE-1 — `load_project_modules` no longer eager-loads the
  stdlib `core` cog; unblocks `--aot` for the whole `core-tests` suite.
* Full bitwise test suite (unit/property/integration/regression).
* **Both tiers GREEN via `verum test`: interp 127/127, AOT 127/127.**
