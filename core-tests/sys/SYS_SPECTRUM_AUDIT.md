# core/sys conformance spectrum — 2026-06-11

> ## ✅ PROGRESS UPDATE 2026-06-12 — Class 1 CLOSED & LANDED on main
>
> **Class 1 (bare record-variant pattern + construction) is CLOSED and
> merged to `main`** (`487cef871`, rebased onto the concurrent codegen
> commits, binary + archive rebuilt). **darwin/tls 29→30/0 and darwin/io
> 41→42/0**, validated on the live main build. sys interp is now
> **~41/51 green**.
>
> Fix: `compile_pattern_test` `PatternKind::Record` + `compile_record`
> resolve the variant tag via scrutinee-then-impl-`self` type (descriptor
> scan / imported qualified `Type.Variant`) BEFORE the colliding bare
> `lookup_function_in_scope`; the construction scoping runs BEFORE
> `simple_resolves_to_record` so a real variant beats a plain-record
> collision. `resolve_field_index` gained a qualified-`Parent.Variant`
> branch.
>
> **Still open** (§F): `windows/tls` (1, archive descriptor carries empty
> variant field-names → 2-field bind reads slot 0), `signal` (9),
> `no_runtime` (6, fn-pointer + record-OOB), `bitfield` (5,
> generic-method monomorphization), `darwin/mod` (30, Bug C umbrella),
> `windows/ntstatus` (3), `locking` (2), `io_engine`/`init`/`file_ops`/
> `fs_watch` (1 each). **fn-pointer (Class 2) root cause pinned:**
> `compile_simple_path` auto-calls 0-arg functions, so a bare function
> name in value position (`let t = always_true` / `spawn_sync(always_true)`)
> becomes a call not a func-ref — the proper fix (emit func-ref when an
> `fn`-type is expected) is central/high-regression and was not attempted
> under the current heavy concurrency.
>
> **⚠️ CONCURRENT REGRESSION (not Class 1):** `linux/bpf/{map,mod,program}`
> regressed (record-field round-trip `map_config_*` tests fail / one
> TIMEOUT) between `3858edf52` and `9c641a453` — isolated to the concurrent
> commit `ae4b3d22a` ("route non-intercepted iterator adapters to custom
> .next() for-loop"). bpf is GREEN on `3858edf52`+Class1, broken on
> `9c641a453`+Class1; my Class 1 is in both, so it is not the cause. Flagged
> for the iterator-adapter track to address.

> ## ✅ POST-FIX UPDATE (2026-06-11, later same day)
>
> **Bug A and Bug B are CLOSED** (commits `08ede3518` Bug B + `cee79ec03`
> Bug A, landed on `main`; binary + stdlib archive rebuilt). Re-swept on
> the fixed `main` build:
>
> - **`durability` 3/8-fail → 11/11 PASS.**
> - **`darwin/libsystem` TIMEOUT(hang) → 51/0 PASS** — the hang WAS the
>   Bug B FFI-symbol failure looping; fixing it un-hung the module.
> - **`linux/time` TIMEOUT → 20/0 PASS** (un-hung; helped by the
>   concurrent `946f3d787` lazy-apply-reachability speedup, 84s→4s).
> - **`signal` 10 → 9 fail** (one FFI-routed case fixed).
> - No regressions (every previously-green module still green;
>   `common` 115/0).
> - Whole-suite load is now ~5s/module (was ~30s) via `946f3d787`.
>
> **New module tally: 39/51 leaf modules fully green** (was 37 + 2 hangs).
> Remaining failures are a **heterogeneous long tail**, NOT one cluster
> (see §F): `darwin/mod` (30) is **Bug C** (umbrella specific-item
> re-export under archive lazy-load — OPEN, and in the concurrent
> session's active `lazy-apply reachability` area, so left to that
> track); `io_engine` (1) is method-on-newtype dispatch; the remaining
> singles (`darwin/io`, `darwin/tls`, `fs_watch`, `windows/tls`,
> `locking`, …) are assorted runtime assertion failures; `bitfield` (5),
> `no_runtime` (6), `windows/ntstatus` (3), `signal` (9) are the
> codegen/data cluster. The original root-cause analysis below is
> retained as the fix record.

Full `--interp` sweep of every `core-tests/sys/**` leaf module (51
modules, ~2150 `@test`s), run **per-module** (one `verum test --interp
--filter sys/<m>` process each) so a single interpreter SIGSEGV/hang
isolates to its module instead of aborting the whole suite.

Binary: a stable copy of `target/release/verum` built from `main@293693b0f`
(taken before concurrent-session relinks). The FFI-cluster results below
were **re-confirmed against a freshly re-baked stdlib archive** (built via
`verum_stdlib_precompiler` into an isolated `VERUM_PRECOMPILE_OUT_DIR`,
then embedded) — the durability / cross-module stubs reproduce on the
fresh archive too, so they are **live codegen defects, not stale-archive
artifacts**.

> AOT (`--aot`) is **not** included in this sweep: stdlib module
> resolution under AOT is broken suite-wide (`error<E402>: module
> `core.sys.common` not found` for any program that mounts a stdlib
> module — reproduced fresh this session). AOT cross-tier is gated on
> that separate, pre-existing defect; see §D.

## Totals: ~2010 pass / ~71 fail across 14 modules + 2 hangs

37 of 51 modules are **fully green** under interp. Failures concentrate
in modules that exercise real platform/FFI operations; every pure-data
module (constants, ADTs, bit layouts, error enums) passes.

| module | ok | fail | note |
|---|---:|---:|---|
| cabi | 50 | 0 | clean |
| common | 115 | 0 | clean |
| context_ops | 32 | 0 | clean |
| darwin/errno | 60 | 0 | clean |
| darwin/mach | 32 | 0 | clean |
| darwin/thread | 25 | 0 | clean |
| darwin/time | 11 | 0 | clean |
| embedded | 43 | 0 | clean |
| interrupt | 18 | 0 | clean |
| linux/arch | 22 | 0 | clean |
| linux/auxv | 22 | 0 | clean |
| linux/bpf/error | 57 | 0 | clean |
| linux/bpf/map | 37 | 0 | clean |
| linux/bpf/mod | 21 | 0 | clean |
| linux/bpf/program | 46 | 0 | clean |
| linux/errno | 37 | 0 | clean |
| linux/io | 26 | 0 | clean |
| linux/mem | 26 | 0 | clean |
| linux/mod | 27 | 0 | clean |
| linux/syscall | 32 | 0 | clean |
| linux/thread | 23 | 0 | clean |
| linux/tls | 19 | 0 | clean |
| mmio | 42 | 0 | clean |
| mod | 39 | 0 | clean |
| net_ops | 18 | 0 | clean |
| process_native | 23 | 0 | clean |
| process_ops | 22 | 0 | clean |
| time_ops | 39 | 0 | clean |
| windows/io | 33 | 0 | clean |
| windows/kernel32 | 143 | 0 | clean |
| windows/mod | 35 | 0 | clean |
| windows/ntdll | 68 | 0 | 1 @ignore |
| windows/thread | 49 | 0 | clean |
| windows/time | 55 | 0 | 2 @ignore |
| windows/winsock2 | 43 | 0 | clean |
| **darwin/mod** | 16 | **30** | **§A** umbrella re-export stub (Bug A) |
| **durability** | 3 | **8** | **§A** cross-module fsync stub (Bug A) |
| **signal** | 62 | **10** | §A/§B FFI-backed signal ops |
| **no_runtime** | 37 | **6** | §C |
| **bitfield** | 106 | **5** | §C bit-op width |
| **windows/ntstatus** | 127 | **3** | §C |
| **locking** | 28 | **2** | §A/§B fcntl byte-range locks |
| **darwin/io** | 41 | **1** | §A/§B |
| **darwin/tls** | 29 | **1** | §A/§B |
| **file_ops** | 31 | **1** | §A/§B |
| **fs_watch** | 17 | **1** | §A/§B |
| **init** | 20 | **1** | §A/§B |
| **io_engine** | 43 | **1** | §A/§B |
| **windows/tls** | 60 | **1** | §C |
| **darwin/libsystem** | — | — | **TIMEOUT (>300s)** §E |
| **linux/time** | — | — | **TIMEOUT (>300s)** §E |

---

## §A — Bug A ✅ CLOSED (`cee79ec03`): stdlib→stdlib cross-module calls stubbed to `LOAD_NIL` at precompile (PRIMARY)

> **Fix:** for a rooted module-path method call (`super.X.Y.fn()`) whose
> qualified lookups miss, fall back to the bare name **only** when it
> resolves to a task-#47 globally-unique free-fn stub (stage-3 sentinel
> id range). A stage-3 stub is the single stdlib-wide definition, so
> dispatch is unambiguous and `Call(stub_id)` is patched to the real id
> by name at finalize. `expressions.rs` `compile_method_call`. `full_fsync`
> now compiles to `CALL safe_full_fsync` (was `LOAD_NIL; RET`).

**The dominant FFI-cluster defect.** During stdlib **precompile**
(`StdlibBootstrap`), a call from one stdlib module into another stdlib
module is silently replaced with a nil stub when the callee's function
id isn't yet resolvable at the call site (ordering / two-pass gap).

### Evidence (VBC dump from a freshly-baked archive)

`core/sys/common.vr` `full_fsync` dispatches (macOS branch) to
`super.darwin.libsystem.safe_full_fsync(fd)`. Its compiled body in the
archive is:

```
; fn sys.common.full_fsync(fd: FileDesc) -> Result<(), OSError>  [id=858, regs=2, locals=1]
  0000  LOAD_NIL  r1
  0001  RET       r1
```

The entire body is `LOAD_NIL; RET` — the cross-module call to
`safe_full_fsync` was dropped. `data_only_fsync` is identical.
`sync_directory` keeps its branch *structure* but every cross-module
call inside it (`safe_open`, `safe_fsync`, `safe_close`) is likewise
`LOAD_NIL`. The function therefore returns `Ok(())`/nil for **every**
input, so it never reports `Err` on an invalid fd.

### Proof it is ordering-specific (not a general resolution failure)

A **fresh user compile** of the identical call resolves correctly — the
call is NOT stubbed:

```verum
// /tmp/probe10.vr
mount core.sys.darwin.libsystem.{safe_full_fsync};
mount core.sys.common.FileDesc;
fn my_full_fsync(fd: FileDesc) -> Result<(), OSError> { return safe_full_fsync(fd); }
fn main() { ... my_full_fsync(FileDesc(-1)) ... }
```
→ reaches `safe_full_fsync`'s body (then hits Bug B, below). So the
stubbing is confined to the precompile of the stdlib archive, where
`sys.common` is compiled before/independently of `sys.darwin.libsystem`.

### Reproduction (interp, embedded archive)

```
$ verum run --interp probe_full_fsync.vr   # full_fsync(FileDesc(-1))
full_fsync(-1) = Ok        # WRONG — must be Err(EBADF)
```

### Affected tests
- `durability/*` (8) — `full_fsync` / `data_only_fsync` invalid-fd Err funnel.
- `darwin/mod/test_umbrella_*` (30) — umbrella re-exports
  (`core.sys.darwin.{O_RDWR, MAP_ANON, ENOENT, …}`). The ~16 that pass
  are exactly those whose true value is `0`/nil-equivalent (`O_RDONLY=0`,
  `PROT_NONE=0`, `ONCE_INIT=0`, …); every non-zero constant fails
  because the re-export collapses to the nil stub.
- `locking` / `darwin/io` / `file_ops` / `fs_watch` / `init` /
  `io_engine` / `darwin/tls` / `signal` (partial) — the individual
  failing tests are the ones routing through a cross-module stdlib call.

### Fix surface
Precompile cross-module call resolution must be **two-pass** (register
all stdlib function ids first, resolve call targets second) so a forward
reference to a sibling stdlib module binds instead of stubbing. This is
the `CLASS-1` topo-sort item. Until then, NO `@cfg`-dispatched
`sys.common` wrapper that delegates to a platform backend works in the
interpreter.

---

## §B — Bug B ✅ CLOSED (`08ede3518`): archive FFI symbols not carried into the consuming module on body-merge

> **Fix:** `merge_archive_function_bodies` now imports each referenced
> archive `FfiSymbol` (dedup by name via `ffi_function_map`), its owning
> library, and `@repr(C)` layouts into `self`, then rewrites the leading
> `symbol_idx:u32` operand of every `CallFfi*` sub-op (except
> `CallFfiIndirect`). `codegen/mod.rs` `import_archive_ffi_symbol`.
> Validated: user program calling archive `safe_full_fsync(FileDesc(-1))`
> returns `Err(EBADF)` (was "FFI symbol not found").

Independent of Bug A, even when a body **does** reach an FFI call, the
call fails: `merge_archive_function_bodies`
(`crates/verum_vbc/src/codegen/mod.rs:16620`) remaps `func_id`,
`type_id`, `const_id`, and `string_id` references in copied archive
bodies — but **not** the `ffi_symbols` table, the `ffi_layouts`, the
library list, or the `CallFfiC` symbol-index operand. The baked-in
symbol index then indexes the *consuming* module's (shorter/different)
`ffi_symbols` table.

### Evidence
`safe_full_fsync` emits `FfiExtended { sub_op: 16 /*CallFfiC*/, operands:
[10, …] }` (symbol 10 = `fcntl`); `safe_getpid` emits `operands: [28, …]`
(symbol 28 = `getpid`). At runtime in a consuming module:

```
$ verum run --interp probe10.vr      # user calls archive safe_full_fsync
error: FFI runtime error: FFI symbol not found: FfiSymbolId(10)
$ verum run --interp probe_getpid.vr # user calls archive safe_getpid
getpid = 73847                        # works (idx 28 happens to resolve)
```

`module.get_ffi_symbol(10)` returns `None` in the consuming module — the
archive's `fcntl` entry was never imported. (`@ffi_name("...")` is also
ignored — a uniquely-renamed extern dlsym's the Verum name, not the
`@ffi_name` target — a smaller sibling defect.)

### Fix surface
`merge_archive_function_bodies` must, alongside the existing remap
tables, build an `ffi_symbol_remap: archive_idx → consumer_idx`
(importing+dedup'ing each archive `FfiSymbol` by `(library, name,
signature)` into `self.ffi_symbols`, importing `ffi_layouts`, and
unioning the `libraries`), then rewrite the symbol-index operand of
every FFI-call `FfiExtended` sub-op (`CallFfiC`/`CallFfiVariadic`/the
ABI-specific `CallFfi*`/`CallFfiIndirect`) in the copied bodies. This
mirrors the existing `CreateCallback` func_id rewrite at
`codegen/mod.rs:15694`.

This fix is **validatable without an archive rebuild** (it is on the
user-compile / body-merge path) and would make every FFI-backed stdlib
function callable from user programs and from cross-module stdlib code.

---

## §C — Codegen / data cluster (independent of FFI)

These fail in the **test bodies' own** freshly-compiled code (not the
archive), so they are ordinary codegen/runtime defects:

- `bitfield` (5) — `(v & field_mask) >> offset` bit-op width mismatch
  (catalogued previously, §26 of the defect catalogue).
- `no_runtime` (6) — to triage.
- `windows/ntstatus` (3) — to triage (most ntstatus closed previously;
  these are residual).
- `windows/tls` (1) — to triage.

## §D — AOT stdlib module resolution (suite-wide blocker for `--aot`)

`verum build --aot` (and `--emit-vbc`, which forces AOT) reports
`error<E402>: module core.sys.common not found` for any program that
`mount`s a stdlib submodule. Reproduced fresh this session. Until AOT
stdlib module resolution is fixed, the `--aot` half of the
interp+aot CI contract cannot be met for sys (or any module). Tracked
separately from the sys suite.

## §E — Hangs (TIMEOUT > 300s)

`darwin/libsystem` and `linux/time` time out under interp. `libsystem`'s
discovered tests are pure constants (`test_*_is_zero`, …) which should
not hang — the stall is in a `property_test`/`integration_test` (a
blocking FFI call such as `safe_read` on a live fd, or a runaway
property loop). Needs per-test isolation. A hang is worse than a
failure (it wedges the whole-suite run), so these gate the default
green-suite contract and should be triaged first among §C/§E.

---

## §F — Remaining long tail (post Bug A/B) — precise root causes, 2026-06-11

After Bug A/B, 12 modules still fail. Each was root-caused this session to
a **distinct deep defect class** (NOT one cluster). They are documented
here as actionable specs with reproductions and fix paths. None is a
simple test bug — every one is a real VBC codegen / interpreter / precompile
defect. **NB: `crates/verum_vbc/src/codegen/expressions.rs` is under active
concurrent-session edit (uncommitted WIP + `8d8214d83`), so any fix in that
file must coordinate with that track.**

### Class 1 — bare record-variant pattern/ctor resolves to a colliding tag at precompile
> **PARTIALLY FIXED on branch `sys-tail-fixes2` (commit `475240b96`),
> pending merge** (blocked: concurrent session holds uncommitted WIP in
> `codegen/expressions.rs` + `interpreter/.../pattern_matching.rs` — the
> exact files — so a fast-forward would clobber their work). **darwin/tls
> 29→30/0 and darwin/io 41→42/0 CLOSED & validated.** Fixes: (1)
> `compile_pattern_test` `PatternKind::Record` resolves the variant tag via
> scrutinee-then-impl-`self` type (descriptor scan / imported qualified
> `Type.Variant`) BEFORE the colliding bare `lookup_function_in_scope`;
> (2) `compile_record` computes the impl/return-type-scoped tag BEFORE
> `simple_resolves_to_record` so `Other` in `from_errno`→`DarwinIoDriverError`
> is the variant, not an untyped `NEW Other` plain record. **`windows/tls`
> (1) STILL OPEN** — the 2-field `AllocationFailed{code,size}` bind reads
> `size` from slot 0; `resolve_field_index` got a qualified-`Parent.Variant`
> branch but `find_variant_in_type_descriptors` returns an empty field list
> for the *archive-loaded* descriptor in the consuming program (variant
> field names not carried on archive descriptors) — deeper archive-metadata
> follow-up.

Modules: `darwin/io` (1), `darwin/tls` (1), `windows/tls` (1), `signal` (some Eq).
Repro: `SysTlsError.InvalidSlot{slot:7}.eq(&same)` → **false** (must be true),
while `SysTlsError.AllocationFailed{code:5}.eq(&same)` → true. VBC evidence:
in the archived `SysTlsError.eq`, the `AllocationFailed` arm emits
`IsVar tag=2` (correct) but the `InvalidSlot` arm emits **`IsVar tag=913`**
(should be 3). Root: `InvalidSlot` is multi-defined (`SysTlsError.InvalidSlot`
*and* `SysContextError.InvalidSlot`, …). At **precompile** time, when a
stdlib `Eq` impl body compiles, the bare pattern `InvalidSlot` resolves via
`lookup_function_in_scope` to the first-wins colliding entry (tag 913)
because neither the type-descriptor scan (`self.types`) nor the qualified
`ctx.functions["SysTlsError.InvalidSlot"]` entry is populated for the
type-being-compiled at that point (STEP 3.6 registers the qualified ctor in
`global_function_registry` with the correct tag, but it does not reach the
per-module codegen's pattern resolution for tuple-match inner patterns;
`match (self, other)` sets `match_scrutinee_type` to the TUPLE, not the
element type). Qualified **construction** uses the descriptor scan (works:
tag 3) — so construction and bare-pattern disagree → arm never matches →
`eq` drops to `_ => false`. Fix paths (tried resolution-site tiers in
`compile_pattern_test`/`compile_record` — ineffective because both sources
return None during precompile): (a) seed `global_function_registry`'s
qualified `Type.Variant` entries into each per-module codegen's
`ctx.functions` before STEP 4 body compile; or (b) propagate tuple-element
types into inner patterns' `match_scrutinee_type`; or (c) the documented
workaround — qualify the variant names in the affected stdlib `Eq`/`match`
impls (`SysTlsError.InvalidSlot{…}`), as already done for `ContextError`.

### Class 2 — function-pointer call of a `fn`-typed parameter
Modules: `no_runtime` (4). Repro: `spawn_sync<T>(task: fn()->T){ task() }`
called with a bare fn name → `TypeMismatch { expected: "closure", got:
"non-pointer", operation: "call_closure" }`. Calling a `fn`-typed parameter
holding a `FuncRef` (NaN-boxed func id, not a heap closure pointer) routes
to the closure-call path which expects a pointer. Fix: the call-a-value path
must dispatch on FuncRef vs closure-pointer.

### Class 3 — interior mutation through a shared `&self` address-of-field
Modules: `signal` (flag set/clear, several). `SignalFlag.set(&self)` does
`atomic_store(&self.value as &mut Int, 1, …)`; `is_set()` then reads 0.
A method taking `&self` (shared) that mutates a field via an
`as &mut`-cast address-of-field does not persist to the caller's value —
the small-record receiver is copied for `&self`, so the store lands on the
copy. Fix: address-of-field on a by-ref receiver must alias the original
(or `set` should take `&mut self` / the field be a Cell/atomic cell).

### Class 4 — method-on-unboxed-newtype dispatch loses the receiver type
Modules: `io_engine` (1), `bitfield` (5), `windows/ntstatus` (1).
`EngineDuration.as_micros()` / `BitfieldElement<UInt8>.to_bits()` /
`Int.bitnot()` panic "method not found on receiver of runtime kind `Int`,
N candidates". The newtype/generic receiver unboxes to `Int` at runtime, so
runtime-kind dispatch can't pick among same-named impls. `bitfield`'s
`byte$to_bits` additionally reports "No registered function ends with the
bare method name" — a generic-method **monomorphization** gap (the mangled
`byte$to_bits` is never registered). Fix: carry the static receiver type to
the call site for direct dispatch; register monomorphized generic-method
names.

### Class 5 — wrong-type method dispatch on collect/Display
Modules: `init` (1: `Maybe.message` on a Maybe receiver), `signal` (2:
`FFIAbi.from_iter` on a List collect). A `.collect()` / `.message()` call
dispatches to the wrong type's method (the receiver's static type is lost or
mis-inferred). Same receiver-type-tracking family as Class 4.

### Class 6 — record field-OOB / data cluster
`no_runtime` (1: `IntPair` field index 2 exceeds size-16 object),
`windows/ntstatus` (2 asserts), `locking` (2 asserts), `fs_watch` (1),
`file_ops` (1), `darwin/io`/etc. residual asserts — per-test triage needed;
some are record-layout/codegen, some may be assertion-logic.

### Bug C — umbrella specific-item re-export under archive lazy-load
Modules: `darwin/mod` (30, one cascading compile error). `mount
core.sys.darwin.{is_retryable}` + call → `undefined function: is_retryable`,
but `core.sys.darwin.errno.{is_retryable}` (direct) and `core.sys.darwin.*`
(glob) both work. `is_retryable` is re-exported into the darwin umbrella via
`public mount .errno.{…}` and is multi-defined across sibling platform
modules, so the specific-item mount neither lazy-loads the re-export target
module nor binds the bare name. **Lives in the archive
lazy-reachability + re-export-metadata path — the subsystem the concurrent
`946f3d787` work touches — left to that track to avoid collision.**

## Cross-tier contract status

Per `core-tests/INVENTORY.md`, the CI contract is "every `@test` passes
under both `--interp` and `--aot`". For `core/sys` today:

- **interp** (post Bug A+B, 2026-06-11): **39/51 modules green**; 12 fail,
  0 hang (both former TIMEOUTs resolved). Remaining failures = the 6 deep
  classes + Bug C catalogued in §F, each a separate focused follow-up.
- **aot**: blocked suite-wide by §D.

The single highest-leverage fix is **Bug A** (two-pass precompile
cross-module resolution): it alone clears the durability + darwin/mod
clusters (38 tests) and most of the single-test FFI failures. **Bug B**
is the necessary follow-on so the un-stubbed calls actually execute.
