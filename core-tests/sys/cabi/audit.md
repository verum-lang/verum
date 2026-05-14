# `core.sys.cabi` — implementation audit

## Status: **complete**

* Every public alias (`CInt`, `CUInt`, `CLong`, `CULong`, `CSize`,
  `CSSize`, `COff`, `CMode`, `CPid`, `CUid`, `CGid`, `CClockId`,
  `CSockLen`, `CFd`) is covered by `unit_test.vr` via round-trip
  construction + tuple-access tests.
* Every published const (`CFD_STDIN`, `CFD_STDOUT`, `CFD_STDERR`)
  has a value pin and is locked under the parent-prefix scan
  resolution path (`regression_test.vr` §A).
* `property_test.vr` pins the sign domain (signed vs unsigned),
  round-trip identity at boundary values, and the partitioning of
  the three standard descriptor sentinels.

## 1. Cross-stdlib usage

`core.sys.cabi` is consumed by every `extern { ... }` declaration in
`core/sys/{linux,darwin,windows}` and by every FFI helper:

| Consumer | Touches | Notes |
|---|---|---|
| `core/sys/linux/syscall.vr` | CInt, CSize, COff, CFd | Raw-syscall ABI. |
| `core/sys/darwin/libsystem.vr` | CInt, CSSize, CSize, COff, CFd, CClockId | libSystem stubs. |
| `core/sys/windows/kernel32.vr` | None (uses Windows-specific aliases) | Windows takes its own typedef route. |
| `core/sys/common.vr` | FileDesc / Flock | Verum-internal newtypes that wrap `CFd` semantics with stronger types. |
| `core/io/protocols.vr` | CFd | `core.io.fs` open / read / write thin shims. |
| `core/net/*` | CInt, CSize, CSockLen | Socket-API plumbing. |

No anti-patterns surfaced. Every C-side function declaration uses the
appropriate alias.

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_codegen/src/llvm/ffi.rs` | C-ABI type → LLVM type mapping (`i32` for `Int32` etc.) | OK |
| `crates/verum_vbc/src/types.rs` | TypeId reservations for primitive widths | OK |
| `crates/verum_common/src/well_known_types.rs` | No cabi entries (intentional — newtype wrappers, not WKTs) | OK |

No additional Rust-side hardcodes for the cabi alias names surfaced.

## 3. Language-implementation gaps surfaced by this suite

### 3.1 Selective const re-export resolution (closed by #FUNDAMENTAL)

* **Symptom (pre-fix)**: `mount core.sys.cabi.{CFD_STDIN}` could fail
  to resolve the constant because the parent-prefix scan in
  `process_import_tree` was missing.
* **Status**: **closed** by the parent-prefix scan landed in
  `vbc/codegen/mod.rs` `process_import_tree`. Pinned by
  `regression_test.vr` §A.

### 3.2 Newtype tuple-access width preservation (closed by `61ed2cdaa`)

* **Symptom (pre-fix)**: tuple-newtype const initialisers
  (`public const CFD_STDIN: CFd = CFd(0 as Int32)`) widened the
  underlying Int32 to Int64 during VBC codegen because the
  VarTypeKind classifier didn't unwrap the reference layer for the
  const path.
* **Status**: **closed** by commit `61ed2cdaa fix(vbc/codegen,
  precompile): unwrap reference type for const VarTypeKind
  classification`. Pinned by `regression_test.vr` §B.

## 4. Action items landed in this branch

1. **`unit_test.vr`** — 22 @tests covering every public alias and
   sentinel constant.
2. **`property_test.vr`** — 9 algebraic-law @tests pinning sign
   domain, round-trip identity, and CFD_* partitioning.
3. **`regression_test.vr`** — 5 @tests pinning the two closed
   compiler-side defects relevant to the C-ABI alias surface.

## 5. Action items deferred

None — the C-ABI alias surface is value-only (no methods, no protocol
impls) and fully covered. The runtime ABI behaviour (extern function
calls actually crossing the boundary) is tested in the broader
`core/sys/{linux,darwin,windows}` integration surface where the per-
platform plumbing is available.
