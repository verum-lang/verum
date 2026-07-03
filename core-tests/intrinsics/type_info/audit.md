# `intrinsics/type_info` audit

Module: `core/intrinsics/type_info.vr` (~89 LOC) — type-information
intrinsics.  Canonical surface: the type properties `T.size` / `T.bits` /
`T.alignment` / `T.stride` / `T.name` / `T.id` / `T.min` / `T.max`; the
legacy meta-fn forms (`size_of<T>()`, …) are `@deprecated`.

Tests (2026-07-03): unit (69) + property (16) + regression (8) +
integration (9).  The property suite exhausts the 17-primitive domain per
law (bits/size coherence, stride laws, power-of-two alignment, exact
canonical names, 120-pair id distinctness, two's-complement min/max).

## 1. Resolution model (as implemented)

`crates/verum_vbc/src/codegen/expressions.rs`:

* Primitives — `resolve_type_property` (bits from the `type_names`
  registry; size = bits/8; alignment = min(size, 16); stride = size;
  min/max/is_signed).
* Reference types — `try_resolve_ref_type_property` (ThinRef 16 / FatRef
  24 / checked 8/16 / unsafe 8/16).
* Named user/generic types — `layout_property_for_named`: registered
  record → `num_fields * 8` (NaN-boxed slot model), else `.name` echo,
  else **silent 8-slot fallback**.

## 2. Defects found this pass

### TYPEINFO-USERTYPE-SIZE-1 (task #8, OPEN) — user records fall to the 8-fallback

`type Tri is {a: Int, b: Int, c: Int}; Tri.size` → 8 on BOTH tiers
(expected 24 per the documented slot model).  `.name` resolves through the
same branch (proving the branch fires); `type_field_count` misses at
body-compile despite Pass-2 registration.  `VERUM_TRACE_TYPEPROP`
instrumentation added to `layout_property_for_named` to pinpoint the map
state.  Pinned red in regression_test.vr
(`regression_user_record_size_is_fieldcount_times_8`).  The silent
fallback itself is the architectural weakness: unknown NAMED types must be
a compile-time error; only genuine unconstrained generic params
legitimately answer 8.

### TYPEINFO-ID-CANON-1 (task #9, OPEN) — three disjoint type-identity notions

* `T.id` is name-derived: aliases DIVERGE (`Int.id != Int64.id`,
  `Float.id != Float64.id`, `Byte.id != UInt8.id`, `USize.id != UInt64.id`)
  while the docs pin `Int = Int64`, `Byte = UInt8`.
* legacy `type_id<T>()` under interp emits the generic param NAME as a
  string (see §3).
* archive `TypeDescriptor` ids are a third space (#27 re-homing).
The property suite deliberately pins only the uncontested core
(determinism + distinct names ⇒ distinct ids); the alias relation is
pinned NOWHERE until the canonicalisation decision.

### TYPEINFO-PROP-LAYER-DRIFT-1 (task #10, OPEN) — checker/codegen allowlist drift

`Int.is_signed` → type-checker error "Associated constant is_signed not
found" while codegen fully implements it.  One property registry must feed
both layers (well_known_types-style).  `is_signed` is therefore untested.

### Unit-type property silence

`().size` silently evaluates to `()` (no resolution, no error) — noted for
the property-registry rework.

## 3. Legacy meta-fn forms — root cause identified (task #14)

`emit_intrinsic_compile_time_constant`: `size_of`/`align_of` emit a
hard-coded `LoadI 8` placeholder; `type_name`/`type_id` emit the
UNINSTANTIATED generic-param name as a string constant.  The interpreter
compiles ONE dynamic body per generic — no substitution reaches the
`@intrinsic` arm (ARCH VBC-GENERIC-INSTANTIATION-1, task #14; same root as
the transmute identity, MEM-TRANSMUTE-FLOAT-1).

## 4. Generic-type properties (§5.3 of the previous audit — ANSWERED)

`List<Int>.size == 24` (3-slot header), `Maybe<Int>.size == 8`,
argument-independence (`List<Int>.size == List<Bool>.size`) — pinned in
integration.  The shallow record-slot model is the contract
(`cbgr_allocate(T.size, T.alignment)`-style consumers stay layout-correct).

## 5. Raw-view representation boundary (cross-module finding)

Reading a LITERAL `List<UInt64>` element through `as_mut_ptr` + atomic
load observes the tier's REPRESENTATION (NaN-box tags under interp, raw
i64 under AOT).  Portable raw-cell code must be store-first — pinned as
the pattern in integration_test.vr; the full story lives in the
memory/atomic audits (LIST-ASPTR-HEADER-1, task #12).

## 6. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mem.allocator` | `T.size` + `T.alignment` for layout. |
| `core.collections.bloom` | `T.size` cell sizing. |
| `core.text.numeric.BigInt` | `T.bits` word-size detection. |
| generic diagnostics | `T.name` rendering (f-string agreement pinned). |

## 7. Action items

**Landed this branch**
* property/regression/integration suites (33 new tests) + unit extension
  (+22: USize/ISize/Char/Int128/Float alias, min/max surface).
* `VERUM_TRACE_TYPEPROP` instrumentation.
* Tasks #8/#9/#10/#14 filed with root-cause analysis.

**Deferred (tracked)**
| Item | Task |
|---|---|
| user-record size resolution + fallback→error | #8 |
| type-identity canonicalisation | #9 |
| property-registry unification (is_signed, unit type) | #10 |
| generic-instantiation visibility at VBC codegen | #14 |
