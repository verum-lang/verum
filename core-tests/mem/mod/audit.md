# `core.mem` (module root) — audit findings

> Module under test: `core/mem/mod.vr` (450 LOC).  The `core.mem`
> umbrella manifest re-exports every submodule's public surface PLUS
> declares three pieces of root-level surface:
>
> 1. `UseAfterFreeError` — 5-field record + 3 static constructors
>    (`new` / `null_pointer` / `capability_violation`) + `message()` +
>    Debug + Display + Eq impls.
> 2. `RevocationError` — 4-variant sum + 4 static constructors
>    (`null_pointer` / `capability_violation` / `already_revoked` /
>    `internal_error`) + `message()` + Debug + Display + Eq impls.
> 3. `CbgrTier` — 4-variant sum + `get_execution_tier()` /
>    `set_execution_tier()` global accessors backed by a `static mut`
>    cell.
>
> Test surfaces (this branch):
> `unit_test.vr`, `property_test.vr`, `integration_test.vr`,
> `regression_test.vr`.  All currently-active tests pass `verum test
> --interp`.  3 unit tests + 1 (sister) tests `@ignore`'d on the
> cross-module method-body field-shift defect (§3.1 below).

## 1. Cross-stdlib usage

`core.mem` is the umbrella every consumer mounts.  Direct submodule
mounts (e.g. `core.mem.capability.{CAP_READ}`) work — but for
sanitisation, the recommended-and-tested path is
`mount core.mem.{CAP_READ}` which routes through `mod.vr`'s
`public mount .capability.{...}` re-export.

The module-root surfaces (`UseAfterFreeError` / `RevocationError` /
`CbgrTier`) are not in any submodule — they only live in `mod.vr`
itself.  Every CBGR call site that signals a use-after-free, null,
or revocation error returns one of these types.

| Consumer | Use |
|---|---|
| `core/mem/thin_ref.vr` | `UseAfterFreeError` returned by `ThinRef::deref` on stale generation. |
| `core/mem/fat_ref.vr` | `UseAfterFreeError` + `RevocationError` returned by `FatRef::deref` + `FatRef::revoke`. |
| `core/base/memory.vr` | `Heap<T>` / `Shared<T>` deref failure surfaces both error types. |
| `core/diagnostics/*` | `CbgrTier` consulted to format dump output (`Aot` skips header introspection). |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `UseAfterFreeError` 5-field layout (mod.vr:265) | Order: expected_gen / actual_gen / expected_epoch / actual_epoch / type_name | Drift would break the `expected_gen == GEN_UNALLOCATED` null-pointer routing in `.message()` (mod.vr:319). |
| `RevocationError` 4-variant order (mod.vr:355-360) | Tag 0 = NullPointer, 1 = CapabilityViolation, 2 = AlreadyRevoked, 3 = Internal | The `match` in `.message()` (mod.vr:384) reads the variant tag; any re-ordering must update the match arms. |
| `CbgrTier` 4-variant order (mod.vr:432-437) | Tag 0 = Interpreter, 1 = BaselineJit, 2 = OptimizingJit, 3 = Aot | Re-ordering would shift the global `CURRENT_TIER` initial value (mod.vr:440) — pinned via `regression_test.vr §E`. |
| `static mut CURRENT_TIER: CbgrTier = CbgrTier.Interpreter` (mod.vr:440) | Initial tier value | Drift would silently change the default tier observable via `get_execution_tier()`. |

## 3. Language-implementation gaps

### 3.1 Cross-module instance-method-body field-access shift (OPEN — pinned via 3 @ignore'd tests)

**Defect surface**: `UseAfterFreeError.message()` body reads `self.<field>`
at the WRONG offsets when invoked on an instance constructed in test
code, because the method body's compilation context is `core/mem/mod.vr`
and the precompiled-archive's field layout for `UseAfterFreeError`
isn't fully threaded into `compile_field_access` at the method-body
codegen site.

**Demonstration** (interpolated output under `--interp` 2026-05-28):

```text
let e: UseAfterFreeError = UseAfterFreeError {
    expected_gen:   5,    actual_gen:     6,
    expected_epoch: 1,    actual_epoch:   2,
    type_name:      "Shared<Int>",
};
print(e.message());
// Output: "use-after-free detected for 1: expected gen=5 epoch=5,
//                                         actual gen=6 epoch=6"
```

Decoded shift:

| Field | Logical slot | `.message()` reads slot |
|---|---:|---:|
| `expected_gen`   | 0 | 0 ✓ |
| `actual_gen`     | 1 | 1 ✓ |
| `expected_epoch` | 2 | 0 ❌ (reads expected_gen) |
| `actual_epoch`   | 3 | 1 ❌ (reads actual_gen) |
| `type_name`      | 4 | 2 ❌ (reads expected_epoch) |

Slots 0/1 are correct because they sit at offset 0/8 in the record's
8-byte-slot layout (UInt32 expected_gen + UInt32 actual_gen pack into
the first 8 bytes).  Slots 2/3 (UInt16 epoch fields) sit at offset 8/10
and slot 4 (Text type_name) sits at offset 16.  Method-body codegen
appears to compute field-N as `8*N` (slot-aligned), misaligning the
UInt16 fields and the Text field.

Direct field reads at the test site (`let e: UseAfterFreeError; e.field`)
resolve CORRECTLY because the `let` annotation propagates the type
binding into `compile_field_access` via `variable_type_names`.

**Same root cause** as the `@ignore`'d
`test_use_after_free_error_new_constructor_round_trip_pinned_by_collision`
and `..._null_pointer_uses_gen_unallocated_sentinel_pinned` pins in
`core-tests/mem/thin_ref/unit_test.vr §6` — there, the constructor
output's field READS at the test site land in the global
`intern_field_name` fallback; here, the method body's `self.<field>`
reads land in the same fallback.

**Working subset** (covered by unit/property/integration/regression tests
in this directory):

* Direct record-literal construction + direct field reads (slots 0/1
  consistently work; slots 2/3/4 work via direct field access with
  explicit `let` type annotation, but FAIL via `.message()` / `.eq()`).
* `RevocationError` (sum type — variant match works, no field-shift).
* `CbgrTier` (sum type — variant match works).
* Re-export surface (constants, free functions).

**Pinned `@ignore`'d tests** (3 total in this directory):

| File | Test | Reads via |
|---|---|---|
| `unit_test.vr` | `test_use_after_free_error_message_routes_to_uaf_path_when_gen_nonzero` | `.message()` → `self.type_name` |
| `unit_test.vr` | `test_use_after_free_error_message_contains_type_name`                  | `.message()` → `self.type_name` |
| `unit_test.vr` | `test_use_after_free_error_eq_distinct_on_type_name`                    | `.eq()` → `self.type_name == other.type_name` |

**Fundamental fix surface** (multi-day VBC codegen work — see
`memory/use_after_free_error_field_shift_2026-05-27.md`):

1. `compile_function` in `core/mem/mod.vr` context must propagate the
   enclosing receiver-type binding (`UseAfterFreeError` for the impl
   block) to `variable_type_names[self]` so the method body's
   `compile_field_access` calls resolve field indices via the
   type-aware lookup rather than the global `intern_field_name`
   fallback at `crates/verum_vbc/src/codegen/mod.rs:13687`.
2. `register_impl_function` (codegen/mod.rs:8225) must record the
   receiver's type-name on the function-info entry so cross-module
   method-body field-access can look it up at compile time.
3. `populate_types_from_archive` (mod.rs:4320) must guarantee that the
   archive's `TypeDescriptor.simple_name` for `UseAfterFreeError` is
   bare ("UseAfterFreeError", not "core.mem.UseAfterFreeError") AND
   that `type_field_layouts` is keyed accordingly.

A defensive fallback in `resolve_field_index` was attempted (commit
`ab8e707f4`) but REGRESSED 3 previously-GREEN record-literal tests with
`field write out of bounds: field index 5` and was reverted in commit
`585728904`.  The correct fundamental fix must preserve the 4-way
cache-consistency invariant `(type_name_to_id, self.types,
type_field_layouts, type_field_type_names)` simultaneously.

### 3.2 `RevocationError` variant-match field-access works

In contrast to §3.1, `RevocationError.message()` returns correct output
because the body uses a `match self { ... }` pattern that destructures
the variant payload:

```verum
match self {
    RevocationError.NullPointer { type_name } => f"...{type_name}...",
    ...
}
```

The variant destructure routes through the per-variant payload accessor
rather than `intern_field_name`-based field access, so it sidesteps the
defect.  All 4 RevocationError `_message_*` tests pass GREEN.

### 3.3 `CbgrTier` set/get round-trip works

The `static mut CURRENT_TIER: CbgrTier` global cell is backed correctly
(per the round-trip-pin in `integration_test.vr §3`) and `set_execution_tier`
/ `get_execution_tier` form an identity pair across all 4 variants.

### 3.4 Umbrella-mount dispatch collision: `has_capability` (OPEN — pinned via 1 @ignore'd test)

**Defect surface**: When `has_capability` is mounted via the umbrella
(`mount core.mem.{has_capability}` routing through `mod.vr`'s
`public mount .capability.{has_capability}` re-export), a 2-arg call
`has_capability(flags, cap)` is dispatched to the SAME-NAME 2-arg method
`AllocationHeader.has_capability(&self, cap)` defined at
`core/mem/header.vr:636`.

Demonstration:

```text
let flags: UInt16 = CAP_OWNED;
assert(has_capability(flags, CAP_READ));
// Runtime: NullPointerAt { op: "opcode 0x78",
//                          site: "AllocationHeader.load_capabilities",
//                          pc: 0 }
```

The first UInt16 argument (CAP_OWNED) is re-interpreted as a
`&AllocationHeader` pointer (= null), which faults the moment the
method body calls `self.load_capabilities()`.

**Direct submodule mount works**: `mount core.mem.capability.{has_capability}`
resolves to the free function correctly (proven by 29 GREEN tests in
`core-tests/mem/capability/unit_test.vr`).  The defect is specific to
umbrella-mount dispatch, not bare-name dispatch.

**Pinned `@ignore`'d test** (1 in this directory):

| File | Test | Symptom |
|---|---|---|
| `unit_test.vr` | `test_reexport_has_capability_call_pinned_by_collision` | `has_capability(CAP_OWNED, CAP_READ)` → NullPointerAt at AllocationHeader.load_capabilities |

**Fundamental fix surface**:

1. The dispatcher's bare-name lookup must distinguish free-fn-arity-N
   from impl-block-method-arity-(N-1)-plus-receiver dispatch.  A 2-arg
   free fn call CANNOT match a 1-arg `&self` impl-block method's
   2-position name lookup; the receiver-arg position must be type-checked
   against the actual argument's type before the method is selected.
2. Or — umbrella-re-exported free fns must carry their canonical
   source-module identity in their function-id key, so a same-name
   impl-block method on an unrelated type cannot shadow them.

**Attempted fixes 2026-05-28** (4 attempts, ALL reverted — defect class
beyond single-session scope):

Investigation identified the root cause: `register_impl_function` at
`codegen/mod.rs:8127-8150` `filter_map`s out `&self` / `&mut self`
params from `param_type_names`, so for `AllocationHeader.has_capability
(&self, cap: UInt16)`:
* `param_count = 2` (param_names = ["self", "cap"])
* `param_type_names = ["UInt16"]` (only 1 entry — self filtered out)

Then in `type_aware_lookup`'s type filter, `param_type_names.iter().
zip(arg_type_names.iter()).all(...)` TRUNCATES to the shorter sequence.
For a 2-arg bare-name call with `arg_type_names = [Some("UInt16"),
Some("UInt16")]`, the method's `param_type_names = ["UInt16"]` zips
to `[("UInt16", "UInt16")]` and `.all()` returns true — `arg[1]` is
never checked. So the method INCORRECTLY passes the type filter.

Four targeted fixes were attempted to filter out impl-block methods
from bare-name dispatch (using `info.param_names.first() == "self"`
as the marker):

1. **v1**: post-filter at `type_aware_lookup` after type_matched
   (didn't fire — type_matched.len() == 1 case already chose method)
2. **v2**: pre-filter at `type_aware_lookup` arity_matches +
   `lookup_function_with_arity_in_scope`
3. **v3**: v2 + `module_qualified_lookup` filter
4. **v4**: v3 + `process_import_tree` umbrella mount registration
   filter at `codegen/mod.rs:7632` (prefer non-method when both free
   fn and method candidates exist)

After EACH rebuild (~25 min per cycle), the test
`test_reexport_has_capability_call_pinned_by_collision` still failed
identically with `NullPointerAt at AllocationHeader.load_capabilities`.
The dispatcher must be selecting `AllocationHeader.has_capability`
via a path NOT covered by any of the 4 filters — most likely via the
stdlib-precompile-emitted bytecode itself (the precompile baked the
function ID into the Call instruction during stdlib's own compilation
phase BEFORE the user-side dispatcher runs).

All 4 fix attempts were reverted (git stash dropped 2026-05-28T22:55).
Fundamental fix requires deeper investigation of:
1. Stdlib precompile's Call-emission path during stdlib's own bytecode
   compilation (NOT the user-side compile_call)
2. `register_function_authoritative` ordering during archive load
3. How `core.mem.has_capability` (umbrella alias) gets its FunctionId

This is multi-day VBC codegen surgery and beyond the scope of this
session.

**Sister defects in the same class** (out of scope here; tracked
across `memory/`):

* `[[btree_pattern_match_ref_generic_class]]` — same-name impl-method
  shadows free fn in collections.
* `io_fundamental_fixes_session_2026-05-24` — `read` / `write` / `close`
  free fns shadowed by File/Socket methods, closed via mount-scope-aware
  `find_function_by_unique_bare_suffix` (`crates/verum_vbc/src/codegen/mod.rs`).

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/mod.vr` module-root surface | `core-tests/mem/mod/{unit,property,integration,regression}_test.vr` | New 4-file suite + this audit |
| 2 | Cross-module method-body field-shift defect documented for `UseAfterFreeError` | §3.1 above | 3 `@ignore`'d tests pin the defect surface; fundamental fix tracked in `memory/use_after_free_error_field_shift_2026-05-27.md` |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Fundamental fix for §3.1 — propagate receiver type binding to method-body `compile_field_access` | multi-day VBC codegen | open (cross-module record-return field-access OOB defect class) |
| §B | Cross-tier divergence sweep: run the four test files under `--aot` and ensure exit-code parity with `--interp` | 1 hour wall-clock (AOT compile times) | pending |
| §C | Extend `Debug` / `Display` `.fmt(...)` tests for `UseAfterFreeError` and `RevocationError` once §A closes | ~30 min | open (gated on §A) |

## Session 2026-07-05 — §3.4 umbrella-dispatch pin closed; 87/87/0

`has_capability(CAP_OWNED, CAP_READ)` via the `core.mem.{…}` umbrella
now routes to the `core.mem.capability` free fn: the `public mount`
re-export traversal in `compile_mount_import` (parent-subtree scan,
module-parent-before-type-parent tie-break, landed in the interim
waves) resolves the binding authoritatively.  Pin un-@ignore'd; the
module is **87 passed / 0 failed / 0 ignored**.
