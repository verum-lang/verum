# Intrinsic Dispatch Contract

Pinned architectural rules for stdlib intrinsic dispatch, `static mut`
backing storage, and CBGR reference encoding. Each rule below has at
least one regression test in `core-tests/` or `crates/verum_*/tests/`
and is load-bearing for production semantics.

A re-introduction of any forbidden pattern is a regression. The pin
tests are the canary; this document explains *why*.

## 1. Body `@intrinsic` macro authority

> **Every public stdlib function whose body is a single `@intrinsic("...",
> args)` macro call MUST NOT also appear in `register_stdlib_intrinsics`'s
> table at `crates/verum_vbc/src/codegen/mod.rs:2422`.**

The body's macro form is the authoritative dispatch declaration. The
table is reserved for non-body-bearing aliases (Duration / Instant /
time / Stopwatch / PerfCounter / DeadlineTimer impl methods) where
there is no parallel Verum body.

### Why

Pre-fix the table pre-registered a stub-shape `FunctionInfo` with
`id = next_func_id++`, empty body, and `intrinsic_name = "<bare>"`.
The user-side intercept at `expressions.rs:4608` resolved
`func_info.intrinsic_name` and dispatched the inline emit, but the
collision with the body-decl produced register corruption in some
cases (most visibly: `clz(1 as UInt64) = ()` instead of 63 — see
[E3 closure note](#references)).

### Pin

- `core-tests/intrinsics/bitwise/regression_test.vr` —
  `regression_clz_u64_*`, `regression_ctz_u64_*`, `regression_popcnt_u64_*`,
  `regression_rotl_u64_*`, `regression_rotr_u64_*`.

## 2. LLVM-canonical alias requirement

> **Every bodyless `@intrinsic("llvm.<name>.<width>")` declaration in
> stdlib MUST have a matching alias in `lookup_intrinsic`** so user-side
> dispatch resolves the LLVM-canonical name to the correct registry
> entry.

Current alias map covers `ctlz` / `cttz` / `ctpop` / `bswap` /
`bitreverse` at i16/i32/i64 widths in
`crates/verum_vbc/src/intrinsics/mod.rs::lookup_intrinsic`'s alias
table.

### Why

`core/math/bits.vr` declares bodyless wrappers like
`@intrinsic("llvm.ctlz.i64") public fn clz(x: UInt64) -> UInt32;`.
Without an alias, `lookup_intrinsic("llvm.ctlz.i64")` returned `None`
and the user-side `compile_call` fell through to a raw `Call` to the
bodyless function → executed an empty body → returned
`Value::default() = Unit`. The bare-name `clz` slot won by
first-wins, shadowing the bitwise.vr generic wrapper.

### Pin

- Same regression-test set as Rule 1.

## 3. Call-site arity matches intrinsic param_count

> **Every `@intrinsic("...", args)` macro call site MUST have arity
> matching the registry's `param_count`.**

The arity-mismatch silent r0-padding in `emit_arith_extended_*` helpers
is a latent miscompile for any future intrinsic that lands in the
registry with non-default arity.

### Why

Pre-fix `core/intrinsics/bitwise.vr::rotl` body was
`@intrinsic("fshl", x, x, n)` — 3-arg funnel-shift call. The
user-side `compile_imported_intrinsic_call` saw `args.len() = 2` at
the wrapper call site, but the intrinsic's `param_count = 3` and the
`emit_arith_extended_ternary` helper padded the missing operand with
`r0` (an uninitialised function-local), silently producing garbage.

### Pin

- `core-tests/intrinsics/bitwise/regression_test.vr::regression_rotl_u64_*`
  / `regression_rotr_u64_*` — these would silently pass with garbage
  results pre-fix; the assertion `assert_eq(rotl(1, 3), 8)` is the
  drift gate.

## 4. `static mut` backing-cell architecture

> **Every `static mut X: T = init;` declaration is backed by a
> process-wide heap-allocated cell with a stable byte address.
> `&X as *T` resolves to that stable address via
> `SystemSubOpcode::StaticMutAddr` (0x52).**

The TLS-slot mechanism (`tls_slots: HashMap<usize, Value>`) is reserved
for `@thread_local static` declarations only — `static mut` without
`@thread_local` has process-wide semantics that TLS cannot express.

### Why

Pre-fix `static mut CAP_AUDIT_ENABLED: UInt8 = 0;` was lowered to a
TLS slot holding an 8-byte NaN-boxed `Value`. Taking
`&CAP_AUDIT_ENABLED as *mut UInt8` fell through `compile_cast`'s
generic catch-all, producing a register-encoded CBGR-Ref bit-pattern.
The Tier-0 `handle_atomic_store` then extracted `ptr_val.as_i64() as
usize` from the bit-pattern → `0xFFFF_FFFD_FFFF_FFFD` garbage
address → SIGSEGV at `stlrb` (ARM64 store-release byte).

### Implementation invariants

- `crates/verum_vbc/src/instruction.rs::SystemSubOpcode::StaticMutAddr =
  0x52` — sibling of `StructFieldAddr = 0x4F`. Operand layout
  `dst:reg, slot_lo:u8, slot_hi:u8`. The slot id is reused from
  `register_thread_local`.
- `crates/verum_vbc/src/interpreter/state.rs::InterpreterState::
  static_mut_cells: HashMap<u16, Box<UnsafeCell<u64>>>`. `Box`'s heap
  allocation gives a stable address across `HashMap` rehashes for the
  lifetime of the `InterpreterState`; 8-byte cell aligned-to-8 covers
  every scalar `static mut` (UInt8/16/32/64/Bool/Int/Float).
- Codegen detection: `try_compile_static_mut_addr` in
  `crates/verum_vbc/src/codegen/expressions.rs::compile_cast` (sibling
  of `try_compile_struct_field_addr`).
- Tier-1 LLVM lowering: extern call to
  `verum_static_mut_cell_addr(slot) -> *mut u8`.

### Open work

- Cells are currently fixed 8 bytes; arrays/records >8 bytes are not
  supported via this path and continue to use TLS storage.
- Non-zero initializers need a `__static_mut_init_<X>` ctor that
  emits `StaticMutAddr + DerefMutRaw size=sizeof(T)`. Cell currently
  defaults to zero, which matches every audit-ring/allocator scalar in
  the current codebase.
- Plain `STATIC_MUT_NAME = expr` writes and `let v = STATIC_MUT_NAME`
  reads still route through the TLS slot. Mixed atomic + non-atomic
  access risks storage divergence — tracked as a follow-up.

### Pin

- `core-tests/mem/cap_audit_ring/unit_test.vr::test_audit_*`
  (previously `@ignore`'d for SIGSEGV).

## 5. CBGR-ref tag decode-bound-check

> **`is_cbgr_ref(val)` MUST validate the decoded `abs_index` against a
> register-file ceiling.** A bare value-range check
> (`val.as_i64() < -2^32`) collides with large negative user-code
> integers — those values decode to garbage abs_index that overflows
> `Registers::get_absolute`.

### Why

The CBGR-ref encoding packs `(abs_index, generation)` into a negative
inline-Int payload. The negative-int range `(-2^47, -2^32)` overlaps
with legitimate user-code values like `-10_000_000_000`. Pre-fix
`is_cbgr_ref` returned `true` for any such value, and downstream
`decode_cbgr_ref` extracted a garbage abs_index that crashed
`Registers::get_absolute` (`index out of bounds: the len is 1024 but
the index is 1420103679`).

### Implementation

- `CBGR_REF_ABS_INDEX_MAX = 1 << 24` in
  `crates/verum_vbc/src/interpreter/dispatch_table/handlers/cbgr_helpers.rs`.
- `is_cbgr_ref` decodes the abs_index and rejects values above the
  ceiling.

### Architectural follow-up

The value-range "tag" is fundamentally a workaround. A future revision
should allocate a true NaN-box tag (`TAG_CBGR_REF`) in
`verum_vbc::value::nanbox` and remove the range overlap entirely.
Tracked as Task F4.

## 6. Three-tier reference model dispatch

> **Every reference-shape unwrap site MUST handle all THREE shapes**:
> CBGR-register-ref (negative inline-Int payload), ThinRef (heap
> 16-byte struct), heap-interior-pointer (`Value::from_ptr` for
> `cbgr_mutable_ptrs`).

### Implementation

Inheritance from CBGR architecture docs:

| Tier | Syntax | Overhead | Use Case |
|------|--------|----------|----------|
| 0 | `&T` | ~15ns | Default, full CBGR protection |
| 1 | `&checked T` | 0ns | Compiler-proven safe |
| 2 | `&unsafe T` | 0ns | Manual safety proof |

### Pin

- `dispatch_method_call`, `handle_get_index`, `handle_set_index` —
  each has three parallel arms for the three shapes (Task #24 fix).

## References

- Task #25 [E3] — body @intrinsic vs table authority, LLVM-canonical
  alias coverage. Closed 2026-05-17.
- Task #26 [E2] — `static mut` backing cell + `StaticMutAddr` opcode.
  Closed 2026-05-17.
- Task #13 [A1] — `is_cbgr_ref` bound-check tightening. Closed
  2026-05-17.
- Task #17 [B2] — funnel-shift 3-operand opcode (FunnelShiftLeft 0x57,
  FunnelShiftRight 0x58).
- Task #24 — interior-field-ref auto-deref across fn boundaries (the
  three-shape unwrap rule).

## Performance budget

Per the CBGR spec, these dispatch paths target:

| Operation | Budget |
|---|---|
| CBGR check | <15 ns |
| Intrinsic dispatch | 1 cycle (DirectOpcode) — 20 cycles (InlineSequence) |
| `StaticMutAddr` lookup | 1 HashMap probe (~30 ns) lazy-allocate on first call; stable thereafter |
| Atomic load/store via cell | <30 ns (1 hash + 1 atomic op) |

Drift here is a regression. Re-run `cargo bench -p verum_vbc --bench
production_targets` after any change to the dispatch path.
