# `core.collections.list` — Audit

Conformance review for `core/collections/list.vr` (List<T> — the canonical
dynamic-array primitive). Section structure follows the project audit
template: cross-stdlib usage; crate-side hardcodes; language-implementation
gaps; defect inventory.

## Status

**partial** — Unit / property / integration coverage now spans both the
runtime-intercepted API surface (allocation, push/pop, get/first/last,
contains, set, get_or, swap, insert, remove, swap_remove, clear,
truncate, reverse, sort, capacity management) AND the stdlib-body path
that was previously gated by the memory-layout drift (retain, resize,
rotate_left, dedup — see unit_test.vr §10 + regression_test.vr §B).

**Architectural fix landed in this branch**: the stdlib `type List<T>`
field decl was reordered from `{ ptr, len, cap }` to `{ len, cap, ptr }`
so codegen field-index agrees with the VBC runtime intercept's slot
allocation `[len@0, cap@1, backing_ptr@2]`. Eliminates the entire
class of "unintercepted method reads wrong slot" defects.

Residual defects are now in adjacent layers (Clone-dispatch for
`value.clone()` inside `fill`; ref-deref-as-value comparison inside
`is_sorted` / `sort_by`) — pinned in regression_test.vr §D.

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
| Stdlib type field order != runtime heap layout — **CLOSED** | (Pre-fix) Every unintercepted `&mut self` / `&self` method that reads internal fields misrouted — SIGSEGV on pointer dereference, infinite loop on len-vs-cap condition, silent data corruption on shift / swap arithmetic. | **Landed**: re-declared stdlib type as `{ len, cap, ptr }` to match runtime intercept layout (option a). Codegen field-index now equals runtime slot index across the whole API surface. Verified by 11 newly-passing tests for retain / resize / rotate_left / dedup in unit_test.vr §10 and 4 active (non-`@ignore`d) guardrails in regression_test.vr §B. The Rust-side intercepts for set / get_or / swap_remove / truncate remain as defense-in-depth: they bypass the stdlib body entirely and serve as a guarantee that the interpreter is correct independent of any future stdlib regression. |
| Clone-dispatch defect surfaced by `List.fill(value)` — value.clone() returns wrong Value for Int. Tracked separately in regression §D. | `xs.fill(0)` writes a wrong NaN-boxed Value into every slot. | Out of scope for the layout fix. Trace the Int Clone dispatch in `verum_vbc/intrinsics/registry.rs`. |
| Ref-deref-as-value compare defect surfaced by `List.is_sorted` / `List.sort_by`. Tracked separately in regression §D. | `a > b` where `a, b: &Int` compares pointer addresses, not Int values. | Out of scope for the layout fix. Audit codegen for `*x > *y` vs `x > y` resolution on `&T`. |

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

4. **Close the for/range iterator-closure dispatch class for non-
   intercepted methods** (§E new). The stdlib bodies of `find_by`,
   `position_by`, `starts_with`, `ends_with`, `windows`, `chunks`,
   `partition_by`, `chunk_by`, `dedup_by` all use
   `for i in range(0, self.len)` which depends on iterator-internal
   closures being registered in the user-side function table. From
   stdlib-internal call sites the closures are present; from user-side
   bytecode they aren't and the call lowers to
   `TypeMismatch { expected: "closure", got: "non-pointer" }`. Two fix
   paths: (a) migrate each method body to the `while + self.get(i)`
   pattern that `index_of` uses (already validated to work everywhere);
   (b) close the iterator-closure cross-module remap. Path (a) is the
   minimum-invasive fundamental fix and is symmetric with the
   already-validated `index_of`/`position`/`rposition` pattern in
   commit chain `c14e4ca87` lineage. Pinned @ignore in
   `unit_test.vr` Sections 17 / 18 / 20 / 27 / 28.

### §E — Bi-modal `position` intercept fixed (CLOSED 2026-05-26)

The runtime `position` intercept at
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs:9243`
previously took `arg[0]` as a closure unconditionally — the
iterator-protocol shape. This routed every value-form
`xs.position(&value)` call (sister of `contains`) into
`call_closure_sync(value, _)` which panics with
`TypeMismatch { expected: "closure", got: "non-pointer" }`.

Fundamental fix: the intercept now branches on `arg0.is_func_ref()`.
Func-ref → predicate-form (original semantics); non-func-ref →
value-form mirroring the `contains` intercept (auto-deref CBGR ref
needle, bitwise compare against each element).

Same defect class as task #17/#39 (mount-scope-aware lookup): a single
name-only dispatch arm trying to serve two semantic sister types
without arg-shape discrimination. The bi-modal-by-arg-shape pattern
documented here is the load-bearing template for closing similar
defects in `find`, `find_by`, `position_by`, etc.
