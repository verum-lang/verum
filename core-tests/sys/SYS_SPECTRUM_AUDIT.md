# core/sys conformance spectrum — 2026-06-11

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

## §A — Bug A: stdlib→stdlib cross-module calls stubbed to `LOAD_NIL` at precompile (PRIMARY)

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

## §B — Bug B: archive FFI symbols not carried into the consuming module on body-merge

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

## Cross-tier contract status

Per `core-tests/INVENTORY.md`, the CI contract is "every `@test` passes
under both `--interp` and `--aot`". For `core/sys` today:

- **interp**: 37/51 modules green; 14 fail + 2 hang, dominated by Bug A.
- **aot**: blocked suite-wide by §D.

The single highest-leverage fix is **Bug A** (two-pass precompile
cross-module resolution): it alone clears the durability + darwin/mod
clusters (38 tests) and most of the single-test FFI failures. **Bug B**
is the necessary follow-on so the un-stubbed calls actually execute.
