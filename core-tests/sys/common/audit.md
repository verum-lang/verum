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

### 3.4 Sized-integer `==/!=` recursing through method dispatch (closed in this branch)

* **Symptom (pre-fix)**: a file with both `assert_eq(F_RDLCK, 0 as
  Int16)` and `assert(F_RDLCK != F_WRLCK)` would crash the second
  assert with `StackOverflow { depth: 16384 }`. The pattern
  generalises to ANY sized-integer comparison (`Int8/Int16/Int32/
  Int64/UInt8..UInt128/USize/ISize/Byte`) on cross-module-const
  operands when an Eq-instantiating context is present in the same
  compilation unit.
* **Root cause**: `compile_binary` (`verum_vbc/src/codegen/
  expressions.rs:~1953`) tested `is_primitive` against ONLY the
  three TypeKind primitives (`Int`, `Bool`, `Char`). Sized aliases
  carry `TypeKind::Path("Int16")` etc., so the test missed them and
  routed `==/!=` through `CmpG` → method dispatch. The
  `assert_eq<T: Eq>` instantiation materialises an `Int16.eq`
  wrapper that captures the default `PartialEq.ne` body's
  `!self.eq(other)`; the subsequent `Int16 != Int16` recursed into
  the wrapper's `self.eq(other)` arm.
* **Two-arm fix landed in commit `0b17c7579`**:
    1. `is_primitive` now consults
       `verum_common::well_known_types::type_names::is_numeric_type`
       (NUMERIC_ALIAS_MATRIX-pinned) so all sized aliases route
       through direct `CmpI` / `CmpF` opcodes;
    2. `extract_expr_type_name(Path)` now probes `func_info.is_const
       + return_type_name` for const declarations so cross-module
       const operands propagate their declared primitive type up
       through the primitive-name probe.
* **Architectural rule pinned**: every primitive comparison op MUST
  emit the direct opcode for primitive operands; every Path
  expression resolving to a const declaration MUST propagate the
  declared type through `extract_expr_type_name`.

### 3.5 EqG protocol_id=0 falling through to deep_value_eq (closed in this branch)

* **Symptom (pre-fix)**: `Some(OSError(2, "X")).eq(&Some(OSError(2,
  "Y")))` returned FALSE even though `OSError.eq` compares only
  `code` (ignoring `message`). Caused `test_oserror_list_count_
  distinct_codes` to fail with `AssertionFailed`. Generalises to
  every `Maybe<Record>.eq` / `Result<Record, _>.eq` / `List<Record>
  .eq` / `Map<K, Record>.eq` / `Set<Record>.eq` site whose record
  type overrides field-by-field equality.
* **Root cause**: blanket-impl bodies like `<T: Eq> Eq for Maybe<T>
  { fn eq(&self, other) { match (self,other) { (Some(a),Some(b)) =>
  a == b, ... } } }` emit `a == b` as `CmpG { protocol_id: 0 }`
  because T is generic at codegen time. `handle_eqg` (`verum_vbc/
  src/interpreter/dispatch_table/handlers/comparison.rs:168`) fell
  through to `deep_value_eq` (recursive structural comparison) when
  protocol_id == 0, ignoring the receiver's runtime TypeId.
* **Fix landed in commit `c8e39850c`**: new
  `runtime_type_name_for_eq(Value, &State) -> Option<String>`
  helper reads heap `ObjectHeader.type_id`, looks up the type's
  name in `state.module.types`, and dispatches through
  `<TypeName>.eq` if such a function exists. Only falls back to
  `deep_value_eq` when no Eq impl is registered for the runtime
  TypeId. Mirrors the architectural rule pinned for Display
  dispatch (task #9 / #10) but for runtime equality dispatch.
* **Architectural rule pinned**: every runtime dispatch site that
  receives a generic-typed value MUST consult the heap
  ObjectHeader to recover the concrete TypeId before falling back
  to structural comparison. Same pattern applies to Ord/Hash
  blanket impls if their dispatch chains regress.

### 3.6 `*list.get(i)` broken idiom — test migration

* **Symptom**: `let xi: USize = *samples.get(i as USize);` silently
  compiles (Maybe declares `Deref<Target=T>` so the type-checker
  accepts) but at runtime `*Maybe<T>` returns identity (per
  `interpreter/dispatch_table/handlers/cbgr.rs:244` — auto-unwrap
  would break `match *self` for sum types like IpAddr).
  `IntCoercible` accepts the assignment regardless, silently
  corrupting test data.
* **Status**: language-level fix tracked as task #5 (deeper
  `*Maybe<T>` codegen alignment with the user-declared Deref impl
  required). Test idiom migration landed in this branch — 30 sites
  across 10 sys-test files migrated to canonical
  `list.get(i).unwrap()` / `list[i]`. Closes the `law_page_align_
  up_monotone` / `law_page_align_down_monotone` failures.

### 3.4 `IntCoercible` propagation through `mount X.{Type}` (legacy)

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
