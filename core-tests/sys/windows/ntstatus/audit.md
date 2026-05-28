# `core.sys.windows.ntstatus` — implementation audit

## Status: **complete** (under `--interp`; constant + severity-dispatch + classifier surface)

* Provides 50+ NTSTATUS constants spanning the four severity domains
  (Success, Information, Warning, Error) plus the NT-canonical helper
  predicates `is_success` / `is_information` / `is_warning` / `is_error`,
  the bit-field accessors `code` / `facility` / `status_code`, the
  textual `name()` lookup table, the Win32 conversion path
  `to_win32_error`, and the classifier helpers `is_retryable` /
  `is_not_found` / `is_access_denied`.
* Reference: Microsoft `ntstatus.h` (Windows DDK).  NTSTATUS layout —
  bits 31-30 severity, bit 29 customer, bit 28 reserved, bits 27-16
  facility, bits 15-0 status-code — is fixed by the Win32 ABI.

## 1. Cross-stdlib usage

`core.sys.windows.ntstatus` is consumed by every Windows-side stdlib
module that wraps `ntdll.dll` / `kernel32.dll`:

| Caller | Use |
|---|---|
| `core.sys.windows.ntdll` | All `Nt*` wrappers funnel raw NTSTATUS through `is_success` to decide Ok/Err. |
| `core.sys.windows.kernel32` | `GetLastError` round-trips against `to_win32_error` for callers that surface Win32 codes. |
| `core.sys.windows.io` | IOCP completion routing keys on `is_retryable` (STATUS_PENDING) for retry loops. |
| `core.sys.common` | OSError funnel from raw NTSTATUS to the cross-platform typed error layer. |
| `core.io.fs` | Path-resolution errors classified via `is_not_found` / `is_access_denied`. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 50 `@test`s covering the constant surface, severity
   dispatch over the documented success / information / warning / error
   arms, bit-field accessors (`code` / `facility` / `status_code`),
   `to_win32_error` round-trip, `name()` lookup table on every documented
   arm + an unknown sample, and every classifier predicate.
2. `property_test.vr` — 11 algebraic laws:
   * severity dispatch is mutually exclusive (exactly one of the four
     `is_*` predicates is true for every documented status);
   * severity windows are total over sampled error / success / warning /
     information slices (every value in the slice classifies into the
     window the slice represents);
   * top-level helpers (`nt_success` / `nt_error` / `nt_warning` /
     `nt_information`) agree with their method-dispatch counterparts;
   * `code()` returns `self.0`;
   * classifier predicates (`is_retryable` / `is_not_found` /
     `is_access_denied`) are pairwise disjoint;
   * `to_win32_error` equals `status_code()` for error-severity codes and
     is zero for success-severity codes;
   * `name()` is total — returns a non-empty Text for every input,
     including 5 random unknown samples;
   * `facility()` / `status_code()` agree with the manual bit-mask formula;
   * `is_retryable` is true ONLY for `STATUS_PENDING`;
   * `is_not_found` covers exactly the documented file-resolution arms;
   * `name()` is injective over the documented arm set.
3. `integration_test.vr` — 15 cross-stdlib scenarios spanning the
   Result-typed syscall funnel, the retry-loop coordinator, the
   PathError dispatch table, batch error counting via `List<NtStatus>`,
   and the first-error scan via `Maybe<NtStatus>`.
4. `regression_test.vr` — 12 `@test`s pinning the defect classes
   below.

## 3. Defects

### §A — UInt32 → Int32 cast preserves bit pattern but not sign at comparison time **[CLOSED via stdlib bit-extraction; VBC codegen fix landed]**

**Symptom.** `STATUS_BUFFER_OVERFLOW.is_success()` returned TRUE on the
unfixed implementation.  The `is_success` body was `self.0 >= 0`, where
`self.0` is the Int32 backing the newtype.  For a const initialiser
`NtStatus(0x80000005_u32 as Int32)`, the VBC Int payload was
+2147483653 (UInt32 numeric value) rather than -2147483643 (Int32
two's-complement), so the signed comparison evaluated as
`2147483653 >= 0 → true`, mis-classifying every Warning- and Error-severity
NTSTATUS as success.

**Isolation chain.** Three probes (all pinned in `regression_test.vr` §A):

| Probe | Behaviour pre-fix | Diagnoses |
|---|---|---|
| `assert(STATUS_BUFFER_OVERFLOW.0 < 0)` | FAIL | The Int32 value's signedness is lost when read from the const. |
| `let x: Int32 = 0x80000005_u32 as Int32; assert(x < 0)` | FAIL | Defect lives in the cast `UInt32 → Int32`, not in const evaluation. |
| `let x: Int32 = -2147483643_i32; assert(x < 0)` | PASS | Negative literals work — only the cast path mis-encodes. |
| `assert_eq(0x80000005_u32 as Int32, -2147483643_i32)` | FAIL | The bit pattern is preserved (so `assert_eq`'s value-equality matched) but the value's signed interpretation drifts. |

**Root cause.** `compile_cast` in `crates/verum_vbc/src/codegen/expressions.rs`
collapsed every integer type (`Int8`, `Int16`, `Int32`, `Int64`,
`UInt8`..`UInt128`, `Byte`, …) to a single `TypeKind::Int` via
`primitive_path_ident_to_typekind`.  The same-discriminant arm at the
fall-through then returned the source register **unchanged** even when
source and target differed in width or signedness.  NaN-boxed Int values
carry a 48-bit signed payload, so a UInt32 value above 2^31 was loaded
as a positive 48-bit integer and `< 0` legitimately returned false.

**Fix.** Two-layer:

(a) **VBC codegen** (`crates/verum_vbc/src/codegen/expressions.rs` —
    integer-integer cast arm).  Detect signedness / width drift via the
    AST-level type-name surface (`infer_expr_type_name` /
    `type_to_simple_name`).  When source is unsigned and target is
    signed at width ≤ 4 bytes, emit `Shl(48-bits) + Shr(48-bits)` to
    sign-extend bit (width*8 - 1) into the 48-bit payload.  When source
    is signed and target is unsigned at width ≤ 4 bytes, emit
    `And(mask)` to zero-extend.  Widths 8 bytes and above fit the
    payload directly and need no re-encoding.

(b) **Stdlib** (`core/sys/windows/ntstatus.vr::is_success`).  Replaced
    the signed `self.0 >= 0` with the canonical NT_SUCCESS bit-31 form
    `((self.0 as UInt32) >> 31) == 0`.  This matches Microsoft's
    `NT_SUCCESS(s) := ((NTSTATUS)(s) >= 0)` macro by extracting the
    high bit directly, never relying on the codegen to interpret a
    negative-Int32-stored-as-positive-UInt32 correctly.  The stdlib fix
    is defensive even after the VBC codegen fix lands — the bit-extract
    form is independently sound.

**Verification.** `regression_int32_*` probes now PASS under `--interp`
post-rebuild.  Severity dispatch over the entire documented arm set is
GREEN.

### §A.bis — Method dispatch loses receiver type for `mount X.{CONST}` imports without the type

**Surfaced during isolation probes for §A.**

`mount core.sys.windows.ntstatus.{STATUS_UNSUCCESSFUL}` imports the
constant value but does NOT bring `NtStatus.is_error` / `is_success` /
etc. impl-block methods into the caller's method-dispatch namespace.
At the call site, `STATUS_UNSUCCESSFUL.is_error()` panics with:

```
method 'NtStatus.is_error' not found on receiver of runtime kind `Int`
```

The dispatch tries to resolve the method against the runtime type of
the receiver — for newtype tuples whose declared type isn't in the
caller's mount-imported namespace, the receiver kind decays to the
underlying primitive (`Int` for `NtStatus = (Int32)`), and dispatch
fails to find the method because the impl block is keyed on the named
newtype, not on `Int`.

**Workaround.** Import the type alongside the constants:
`mount core.sys.windows.ntstatus.{NtStatus, STATUS_UNSUCCESSFUL}`.

**Fundamental fix surface.** The dispatch table should resolve methods
via the receiver's static declared type (the const initialiser
declares it as `NtStatus`), not the runtime-tag of the underlying
primitive.  This is the same class of defect as receiver-method
intercepts in `method_dispatch.rs` that key on `is_int()` + bare
method name and mis-fire for single-field-record-unboxed records (see
related work in [[duration_single_field_record_unboxing_2026-05-27]]
and [[btree_pattern_match_ref_generic_class]]).

Tests in this suite import `NtStatus` alongside the const surface as
the workaround.

### §B — STATUS_PENDING / STATUS_TIMEOUT are Success-domain by NT convention

**Pinned by.** `regression_status_pending_is_success_domain` /
`regression_status_success_zero_is_success_domain`.

**What it pins.** The NT_SUCCESS convention covers `severity = 0` AND
`severity = 1`. Both `STATUS_PENDING` (0x00000103) and `STATUS_TIMEOUT`
(0x00000102) live in the Success domain even though their numeric value
is non-zero. Any future edit that reads `is_success` as "severity ==
exactly 0" would mis-classify both and silently break every retry-loop
coordinator that treats `nt_success(STATUS_PENDING)` as "Ok, retry".

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `is_information` / `is_warning` / `is_error` `@inline` expansion across CONST initialiser at AOT | Surface checked only under `--interp` so far.  The full AOT path through the cast + shift sequence needs a dedicated run once the codegen-emitted sequence stabilises in production. |
| 2 | `name()` exhaustive sweep | The lookup table has ~44 documented arms.  The pairwise-distinct property test covers the small documented subset; full sweep awaits compile-time match-coverage in property_test. |
