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
The user-side intercept — `expressions.rs`, the sites that read `func_info.intrinsic_name`; the line number this document carried had drifted onto an unrelated tuple-index arm — resolved
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

## 7. A slice's canonical dispatch key is `Slice`, not its spelling

> **`implement<T> [T]` registers under the prefix `Slice`.** Every layer
> that resolves a method name MUST normalise a slice spelling — `[T]`,
> `&[T]`, `&mut [T]`, `&checked [T]`, `&unsafe [T]` — to that key before
> it resolves. A layer that skips the normalisation asks for a name no
> layer has ever defined.

### What is true

`core/collections/slice.vr` declares 27 methods in `implement<T> [T]`,
and `extract_impl_type_name_from_type` registers them under `Slice`. The
emitted module has the bodies — `@Slice.len`, `@Slice.get`,
`@Slice.is_empty`, `@Slice.iter`, `@Slice.fmt_debug`, … — and has
**nothing** under the bracket spelling: measured over one whole-stdlib
compile, 4801 `define`s, 273 of them with quoted names (so a quoted name
is visible to the search), and **zero** with a `[` in the name.
`vbc.types` likewise holds no descriptor spelled `[Byte]`.

Two tier-0 layers do the normalisation:

| layer | file | accepts |
|---|---|---|
| VBC codegen | `codegen/expressions.rs` | `[T].m` only — a name STARTING with the bracket |
| interpreter | `interpreter/dispatch_table/handlers/method_dispatch.rs` | `[T].m` **and** the reference spellings |

The interpreter's site carries the reason verbatim: *"Every line-oriented
read went down with it, since `BufRead.read_until` iterates the subslice
`fill_buf` returns."*

### The defect this pins (T1447)

The AOT had no mirror. A receiver whose declared type is spelled
`&[Byte]` keeps that spelling all the way into `lower_call_method`,
where:

- `method_type_prefix` requires the prefix's first character to be
  UPPERCASE, so `&[Byte]` is refused and the receiver's type reads as
  `None`;
- the unresolved-`CallM` fallback builds a runtime type-id switch by
  walking `vbc.types` for named `Type.method` bodies, and a slice has no
  descriptor there, so the receiver matches **no arm at any type id**;
- the default arm aborts by design:

```
PANIC: AOT dispatch fault: no runtime candidate for method
'&[Byte].is_empty' — the receiver's runtime type id matched none of
the arms built from declared implementations.
```

That abort is correct behaviour for the switch. The defect is upstream:
the switch was asked a question no arm set could answer.

### Why it hid for so long

A slice taken from a `List` is ALSO marked a list register, so the
List/Channel/Map intercept answers `len`/`is_empty` for the common
spelling — sub-ranges included, whose length it gets right. Measured at
Tier 1 on a pre-fix binary, `&xs[..]` and `&xs[1..3]` passed to a
`&[Int]` parameter all answer correctly.

Only a slice arriving WITHOUT that mark falls through, and the canonical
such shape is the payload a `?` unwrapped out of a `Result`:

```verum
let available = self.fill_buf()?;   // IoResult<&[Byte]>
if available.is_empty() { … }       // aborted at Tier 1, read at Tier 0
for (i, b) in available.iter().enumerate() { … }   // the next one along
```

A census of one whole-stdlib compile finds exactly three method names
called on a slice receiver — `iter` (7), `is_empty` (7), `len` (4) — and
`read_until` calls two of them one line apart, so a fix that routes some
of them only moves the abort.

### The second leg: the body has to exist

Resolving the name correctly is not enough, and the measurement says so
plainly. With the normalisation alone:

```
tier 0:          1 3  2 0  3 2  4 20  5 3  6 0  7 3  8 0   rc=0
tier 1 (leg 1):  1 3  2 0  3 2  4 20  5 0  …               rc=255
```

The abort is gone — the name resolved and the programme linked — and
rung 5 answers **0** where it must answer 3. A silent zero, because the
call reached a function that has no body.

Body lowering is scoped to the reachable set (`scoped_lowering_enabled`,
the 82GB → 5GB saving), and the by-name closure in
`crates/verum_vbc/src/reachability.rs` drops a candidate whose parent
type is a `Record` that reachable code never constructs. `Slice`'s
descriptor IS a `Record` — but only by synthesis:
`ensure_structural_impl_target_type` materialises it for
`implement<T> [T]`, which has no named type decl, and hard-codes the
kind. A slice is never built by `New`/`NewG` (it comes from `RefSlice`),
so "no construction found" proves nothing about it, and every `Slice.*`
method was pruned.

The IR says it before the programme runs: `@Slice.len`, `@Slice.iter`
and `@Slice.is_empty` are `declare`s, and 268 calls to `@Slice.len` sit
in the module — dead only because their callers were pruned too. The
linked binary carries no `Slice` symbol at all.

So a structural impl target (`Slice`, `Array`, `Tuple`) belongs in the
conservatively-included class that pass already names for variants and
newtypes: **a parent whose construction the pass cannot observe at all.**

### Pin

- Normalisation: `lower_call_method`, immediately after `method_name_str`
  is read from the string table — the one point where every producer of a
  qualified name converges, which is where the interpreter does it too.
  `type_name_is_slice` strips all three CBGR reference spellings (`&mut`,
  `&checked`, `&unsafe`; stripping `&` alone leaves `mut [Byte]`, which
  names nothing) and excludes `[T; N]`, a different value shape with its
  own length authority.
- `receiver_type_name` is normalised the same way, so every strategy
  that builds `format!("{type}.{method}")` asks a question the module can
  answer.
- Consequence: all 27 declared slice methods dispatch, not the three that
  happen to be called today.
- Reachability: `crates/verum_vbc/src/reachability.rs`, the by-name
  candidate closure — a structural impl target is never excluded by the
  "record never constructed" rule, because its descriptor is synthesised
  and its instances never pass through `New`/`NewG`.
- Cost of that inclusion, measured on one probe back-to-back with the
  pre-fix binary: **87s → 75s**. Un-pruning `Slice`/`Array`/`Tuple`
  methods did not cost AOT compile time here; the difference is load
  noise, and it runs in the favourable direction.
- Spec: `vcs/specs/L0-critical/stdlib-runtime/`
  `a_method_on_a_slice_dispatches_at_both_tiers.vr` — eight rungs at both
  tiers; rungs 5 and 6 are the `?` shape, the only pair that fails
  pre-fix. Every assertion adds `+ 0` so the front end's constant model
  cannot answer it.
- Trace: `VERUM_AOT_TRACE_CALLM=1` prints the dispatch decision per
  `CallM`, including the resolved target.

## Generic-arithmetic object arm (T0499)

The integer arithmetic opcodes (`AddI`/`SubI`/`MulI`/`DivI`/`ModI`/
`NegI`) are POLYMORPHIC over erased operands. Arm order is pinned:

1. both-inline-int fast path (zero-cost);
2. reference resolve (CBGR refs);
3. float arm (T0497 — either operand a real NaN-box float);
4. 128-bit arm (T0272 — boxed Int128/UInt128 full width);
5. string-concat arm (`AddI` only — small OR heap Texts);
6. **object arm** — a heap-record operand dispatches to its type's
   operator method (`Complex.add`) resolved through the guarded
   header probe in
   `interpreter/dispatch_table/handlers/object_dispatch.rs` (the ONE
   runtime-type-resolution authority; the Eq/Ord fallback delegates
   to it). Dispatch pushes the method's call frame in-loop (the
   `handle_eqg` shape — no nested execution), memoised per
   `(type_id, method)` including negative results;
7. integer-extract fallback (legacy tag-robust arm — unchanged).

Codegen counterpart: `compile_binary`'s operator-method route
normalises the receiver type with `strip_generic_args` (function keys
are generic-stripped — `Complex.add`, never `Complex<Float>.add`),
same for unary `neg`.

Static-call witness chain: a bare-path static call carries generic
witnesses derived (in priority order) from an explicit TypeExpr
receiver, an alias instantiation target, the binding-annotation hint
(`let m: Matrix<Complex<Float>> = Matrix.zeros(…)`), or the
enclosing-impl identity relay (`Matrix.zeros(n, n)` inside
`implement<T> Matrix<T>` stages `[Generic(0..k)]`, resolved through
the caller frame's witness table at runtime). Without a witness the
callee's `LoadT Generic` loads nil — the "zeros() matrix full of
nils" class.

Tier-1 (AOT) parity: the object arm is interpreter-side; the AOT
`AddI` lowering keeps integer semantics for erased operands. Mono
specialization is the intended Tier-1 route for generic user-type
arithmetic; until it covers this shape, generic `+` on user records
under `--aot` remains a known gap (tracked with T0499's residuals).

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
