# `core.sys.common` — implementation audit

## Status: **partial** (architectural fix landed; full FFI surface deferred)

* Type-level surface (OSError, FileDesc, IOVec, MemProt, MapFlags,
  SysMemoryOrdering, SysContextError, FcntlLockKind, Flock) is
  exhaustively covered by `unit_test.vr`, `property_test.vr`,
  `integration_test.vr`, and pinned in `regression_test.vr`.
* Page-arithmetic surface (PAGE_SIZE, page_align_up, page_align_down,
  is_page_aligned) is covered including boundary + algebraic-law sweeps.
* Context capacity constants (MAX_CONTEXT_SLOTS, CONTEXT_STACK_DEPTH)
  + the 5-variant SysContextError algebra are pinned end-to-end.
* The **FFI-adjacent surface** (`os_alloc`, `os_free`, `get_thread_id`,
  `pread`/`pwrite`, `full_fsync`/`data_only_fsync`, `try_lock_region`,
  `file_size`, `truncate`, `access`, `random_bytes`, `ctx_get`/`ctx_set`,
  `init_process_args`/`get_arg`/`get_env`/`set_env`) is **deferred** —
  these require per-platform plumbing (linux/syscall, darwin/libsystem,
  windows/kernel32) and are exercised in the cog-level integration
  surface where the right tier is wired up.
* The user-side `ContextSlots` record (active bitmap + slots array) is
  declared on top of the V-LLSI raw `ctx_*` API and runs in the same
  deferred bucket — pinning would require runtime fixture setup that's
  out-of-scope for the type-level conformance suite.

## 1. Cross-stdlib usage

`core.sys.common` is the foundation that `core.sys.{file_ops,
process_ops, time_ops, net_ops, context_ops, durability, locking}` and
their per-platform implementations (linux / darwin / windows) all
build on. Concretely:

| Consumer | Touches | Notes |
|---|---|---|
| `core/sys/file_ops.vr` | FileDesc, OSError, IOVec, MapFlags, MemProt | Pure thin shim; delegates to per-platform syscalls. |
| `core/sys/process_ops.vr` | OSError | Wraps fork/exec/wait variants. |
| `core/sys/durability.vr` | FileDesc, OSError | Calls into `full_fsync` / `data_only_fsync`. |
| `core/sys/io_engine.vr` | FileDesc, IoError (its own), Fd | Uses FileDesc as the canonical wrapper. |
| `core/sys/linux/*` | OSError, FileDesc, Timespec | Direct-syscall path that materialises an OSError from raw errno. |
| `core/sys/darwin/*` | OSError, FileDesc, Timespec | libSystem-routed path. |
| `core/io/protocols.vr` | OSError (via `from_raw_os_error`) | Cross-platform error funnel. |

No anti-patterns or redundant call sites surfaced.  Every consumer
routes through the canonical re-export point `core.sys.{OSError, ...}`.

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/codegen/mod.rs:~7213` | `process_import_tree` parent-prefix scan for `public mount` re-exports | **landed in this branch** — fix #FUNDAMENTAL |
| `crates/verum_compiler/src/precompile.rs:430` | `scan_module_reexports` populates `metadata.module_reexports` | OK |
| `crates/verum_compiler/src/pipeline/loading.rs:1227` | Typechecker-side re-export propagation through ExportTables | OK |
| `crates/verum_compiler/src/archive_ctx_loader.rs:127` | `merge_module_and_simple_name` canonical key synthesis | OK |
| `crates/verum_common/src/well_known_types.rs` | No OSError / FileDesc / MemProt entries (intentional — not WKTs) | OK |

No additional Rust-side hardcodes for the public surface surfaced.
The `IntCoercible` marker (`core.base.coercion.IntCoercible`) is the
one cross-cutting protocol; it's tested implicitly via every
`fd >= 0` and `flags | mask` site in this folder.

## 3. Language-implementation gaps surfaced by this suite

### 3.1 `mount X.{CONST}` selective re-export resolution (closed in this branch)

* **Symptom (pre-fix)**: `mount core.sys.{PAGE_SIZE}` returned
  `core.mem.allocator.PAGE_SIZE: Int = 65536` instead of
  `core.sys.common.PAGE_SIZE: USize = 4096`. The defect lived in
  `process_import_tree` (vbc/codegen/mod.rs): the resolver fell
  through to bare-name lookup without first scanning the parent-mount
  path's subtree, and the bare-name slot was owned by whichever
  sibling module's `PAGE_SIZE` registered first during archive load.
* **Fix landed**: parent-prefix scan inserted before the bare-name
  fallback. For `mount core.<X>.<Y>.{NAME}`, the resolver now looks
  for any function/const registered under a key starting with
  `core.<X>.<Y>.` (and the `core.`-stripped variant) and ending in
  `.<NAME>`. Shallowest hit wins; alphabetical as deterministic
  tiebreak. This walks the same re-export chain that
  `metadata.module_reexports` records on the typechecker side, but
  without requiring metadata-plumbing into the codegen.
* **Pinned by**: `regression_test.vr::regression_page_size_resolves_via_sys_re_export`.

### 3.2 Variant-tag stability under repeated archive precompile

* **Symptom**: pre-task #22, the SysContextError 5-variant could have
  its match arms mis-routed under repeated archive precompile cycles
  because the codegen variant-tag map was bare-name first-wins.
* **Status**: closed by task #22 (4 commits, see MEMORY entry).  Pinned
  here as a guardrail; the underlying compiler fix is in the codegen
  variant-tag resolution layer (`compile_pattern_test` +
  `compile_variant_constructor_hinted` context-aware tag lookup).

### 3.3 OSError `Eq.ne` blanket-impl dispatch

* **Symptom**: pre-task #11 / #42, the `Eq.ne` blanket impl over
  OSError dispatched into the wrong concrete `eq` when an unrelated
  sibling type was in scope.
* **Status**: closed by task #11 (blanket-impl pre-pass) + task #42
  (strict generic-param classification at `for_type_is_generic_param`).
  Pinned here so any future re-introduction of the dispatch race
  surfaces immediately.

### 3.4 `IntCoercible` propagation through `mount X.{Type}`

* **Symptom**: the `IntCoercible` marker on FileDesc / MemProt /
  MapFlags must propagate through the selective-mount form so the
  `fd >= 0` / `flags | mask` use sites typecheck. Pre-fix, the
  selective form lost the marker because the type-attribute table
  wasn't consulted at re-export time.
* **Status**: closed; pinned by
  `regression_test.vr::regression_filedesc_compares_with_int_via_coercion`.

## 4. Action items landed in this branch

1. **Fix #FUNDAMENTAL** — parent-prefix scan in `process_import_tree`
   (vbc/codegen/mod.rs) so `mount core.sys.{NAME}` finds the canonical
   re-export source (`core.sys.common.NAME`) instead of falling
   through to bare-name lookup.
2. **Comprehensive type-level test suite** — `unit_test.vr` covers
   every type's public API surface; `property_test.vr` pins algebraic
   laws (idempotence, monotonicity, bijection, reflexivity); 
   `integration_test.vr` exercises cross-type combinations with the
   rest of stdlib (List, Maybe, Result); `regression_test.vr` pins
   every closed compiler-side defect.

## 5. Action items deferred

1. **FFI-adjacent surface coverage** — `os_alloc`/`os_free`, `pread`/
   `pwrite`, `try_lock_region`, `file_size`/`truncate`/`access`,
   `random_bytes`, `init_process_args` / `get_env` / `set_env`,
   `ctx_*` runtime API. These need a per-platform fixture (tmp file,
   environment isolation) that's deferred to the cog-level integration
   surface. Estimate: 1 day per group once the test fixture
   infrastructure lands.
2. **ContextSlots high-level type** — declared on top of the V-LLSI
   raw API; pinning requires runtime fixture setup. Deferred with
   the rest of the runtime-bound API.
3. **PageSize type alias** — currently `type PageSize is USize`; no
   methods, no laws to pin beyond what PAGE_SIZE already covers.
   Skipped intentionally.
