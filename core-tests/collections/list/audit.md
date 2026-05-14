# `core.collections.list` — Audit

Conformance review for `core/collections/list.vr` (List<T> — the canonical
dynamic-array primitive). Section structure follows the project audit
template: cross-stdlib usage; crate-side hardcodes; language-implementation
gaps; defect inventory.

## Status

**partial** — Unit / property / integration tests cover the API surface
that the runtime intercepts (allocation, push/pop, get/first/last,
contains, set, get_or, swap, insert, remove, swap_remove, clear, truncate,
reverse, sort, capacity management). Methods that fall through to the
stdlib body and depend on direct `self.<field>` access are pinned in
`regression_test.vr` §B until the runtime memory-layout drift closes
(see `Open defects` §2 below).

## 1. Cross-stdlib usage

`List<T>` is the spine of Verum's data-flow vocabulary — referenced by
**every** other collection, every iterator pipeline, every Maybe-returning
search path, every text-formatting builder. Highest-frequency call sites:

| Site | Shape | Notes |
|---|---|---|
| `core/base/maybe.vr:671` | `let mut collected: List<T> = List.with_capacity(lo);` | Maybe→List collector hot path. |
| `core/base/error.vr:148` | `Backtrace { frames: List.new() }` | Error chain backtrace assembly. |
| `core/base/env.vr:140` | `let mut res = List.with_capacity(count);` | Env varlist sizing. |
| `core/base/semver.vr:168,174,251,293,436,464,479,487` | `let mut <out>: List<T> = [];` | Pre-release / build-meta / IDs / byte-slice assembly. |
| `core/base/string_distance.vr:78,79,117` | `let mut <buf>: List<Int> = [];` | DP-table row allocation in edit-distance algorithms. |
| `core/net/weft/metrics.vr:104` | `let bounds = List.from([5, 15, …]);` | Histogram bucket table. |
| `core/net/weft/tls.vr:216` | `List.from(["h2", "http/1.1"])` | ALPN protocol list. |
| `core/verify/kernel_soundness/rules.vr:81-280+` | `List.from([…])` per kernel rule | Inference-rule premise tables. |

The `List.from(...)` call sites in `core/verify/kernel_soundness/rules.vr`
and `core/net/weft/{metrics,tls}.vr` compile under stdlib-internal mode
but fail with `CodegenError::UndefinedFunction("List.from")` at every
user-side call site — pinned as regression §A.

## 2. Crate-side hardcodes

Searches across `crates/` for direct manipulation of `List` heap layout:

| Path | Line(s) | What it does |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs` | 1035-1150, 3694-3776, 8058-8211 | Static-constructor intercepts (List.new / List.with_capacity), instance-method intercepts (len/is_empty/push/contains/pop/insert/remove/clear/swap/reverse/sort), and the new §2 layout-bypass intercepts (set/get_or/truncate/swap_remove) added in this branch. |
| `crates/verum_vbc/src/interpreter/dispatch_table/mod.rs` | 935-980 | `get_array_length` / `get_array_element` canonical readers — hardcode `[len@0, cap@1, backing_ptr@2]`. |
| `crates/verum_vbc/src/types.rs` (TypeId::LIST) | — | `TypeId(0x201) = 513` reserved for the well-known List shape; the runtime intercepts gate on this. |

**Drift surface**: the field order in `crates/verum_vbc/.../mod.rs:931-948`
is the authoritative runtime layout. The stdlib type declaration
`type List<T> is { ptr, len, cap }` (`core/collections/list.vr:69`)
encodes a *different* field order. Codegen resolves `self.<field>` via
the type-decl order, so unintercepted stdlib methods read from the wrong
slots. Tracked as §2 below.

## 3. Language-implementation gaps

| Gap | Impact | Fundamental fix |
|---|---|---|
| Cross-module function-name table omits non-intercepted stdlib static constructors | `List.from`, `List.from_elem`, `List.of` raise `UndefinedFunction` from user code despite stdlib-internal use compiling cleanly. | Same class as Tasks #24/#25/#26. Either fold these into the dispatch-table intercept block, or close the gap in the user-side function-id remap that drops constructor IDs absent from any user-side body Call instruction. |
| Stdlib type field order != runtime heap layout | Every unintercepted `&mut self` / `&self` method that reads internal fields misroutes — SIGSEGV (on pointer dereference), infinite loop (on len-vs-cap condition), or silent data corruption (on shift / swap arithmetic). | Three options: (a) re-declare stdlib type as `{ len, cap, ptr }` to match runtime; (b) add a Rust-side intercept for every field-accessing method (set, get_or, swap_remove, truncate, retain, fill, resize, …); (c) move runtime allocation into a stdlib code path that uses the canonical record literal `List { ptr, len, cap }` so codegen field offsets agree on a single source of truth. Option (c) is cleanest but requires changing `List.new()` / `List.with_capacity()` away from runtime-intercept to stdlib-resident with a one-time `alloc_array` call. Intermediate fix landed in this branch: (b) for set / get_or / swap_remove / truncate. |

## 4. Defect inventory

Per `regression_test.vr`:

### §A — Cross-module function-name resolution (3 tests)

* `List.from(array)` — `UndefinedFunction("List.from")`.
* `List.of(value)` — `UndefinedFunction("List.of")`.
* `List.from_elem(value, n)` — `UndefinedFunction("List.from_elem")`.

Same root cause as Tasks #24/#25/#26 close-out: stdlib static methods
that aren't runtime-intercepted require their function IDs to round-trip
into the user-side bytecode through the function-name table. The current
table populates from cross-module Call operands; constructors that don't
appear in any user-side body's Call instruction at precompile time are
omitted.

### §B — Runtime memory-layout drift (3 tests, additional ones folded into §2 above)

* `retain` — SIGSEGV (stdlib body reads `self.ptr` from slot 0 = len-as-pointer).
* `fill` — silent no-op write (same shape, write rather than read).
* `resize` — undefined behaviour combining read and write at wrong slots.

Sort-by-callback is borderline — pinned for parity. The default `sort()`
is fully intercepted and works.

### §C — `contains(&value)` needle CBGR-ref bit comparison (CLOSED)

Pre-fix: every `contains(&primitive)` returned false because the
intercept bit-compared the ref-encoded needle against each element's
NaN-boxed Value. Closed in this branch by auto-derefing the needle
through `decode_cbgr_ref` before the bit-equality compare. Pinned as a
non-`@ignore`d regression to guard against re-regression.

## 5. Action items

1. **Close the runtime memory-layout drift** (§3 / §B). The right fix is
   option (c) — move runtime allocation into a stdlib-resident
   constructor that builds a real `List { ptr, len, cap }` record. Until
   then, every new stdlib method that touches `self.<field>` MUST land
   with a parallel runtime intercept; pin via grep
   `is_list = receiver_type_name.as_deref() == Some("List")` and audit
   each method-name arm.

2. **Close the cross-module constructor name-table gap** (§3 / §A).
   Same fix path as Tasks #24/#25/#26 for general stdlib-call cross-
   module resolution.

3. **Re-enable the property + integration tests in `regression_test.vr`
   §B** once item 1 lands.
