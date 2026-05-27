# `core/base/protocols` — Audit

> Module: `core/base/protocols.vr` — the operator and capability
> protocols that every value type must (or may) implement. The
> single source-of-truth file for `Eq`, `Ord`, `Hash`, `Clone`,
> `Copy`, `Default`, `Debug`, `Display`, `From`, `Into`, `Deref`,
> `AsRef`, `Numeric`, `Add` ... `Shr` etc.

## §1 — Public API surface

### 1.1 Protocols (selected)

| Protocol | Required methods | Notes |
|---|---|---|
| `Eq` | `eq(&self, &Self) -> Bool` | default `.ne` derived from `eq` |
| `Ord` | `cmp(&self, &Self) -> Ordering` | default `.lt` / `.le` / `.gt` / `.ge` / `.max` / `.min` / `.clamp` |
| `Hash` | `hash(&self, &mut Hasher)` | + Hasher protocol with `.write_int`, `.write_bytes`, etc. |
| `Clone` | `clone(&self) -> Self` | independence contract |
| `Copy` | — | marker protocol |
| `Default` | `default() -> Self` | canonical zero / empty value |
| `Debug` | `fmt_debug(&self, &mut Formatter)` | developer-facing |
| `Display` | `fmt(&self, &mut Formatter)` | user-facing |
| `From<T> for U` | `from(T) -> U` | reflexive `From<T> for T` |
| `Into<T>` | `into(self) -> T` | derived from From |
| Operator protocols | Add / Sub / Mul / Div / Mod / Rem / Neg / BitAnd / BitOr / BitXor / Not / Shl / Shr | numeric algebra |

### 1.2 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 73 unit tests | green (after §A fix) |
| `property_test.vr` | 52 algebraic laws | green |
| `integration_test.vr` | integration scenarios | green |
| `try_protocol_agnostic_test.vr` | Try-protocol edge cases | green |
| `regression_test.vr` | 12 active pins | 12 green |

## §2 — Findings landed in this branch

### 2.1 Type-name collision with stdlib through bare-suffix resolution

Pre-fix `test_eq_reflexivity` defined `type Item is { id: Int };` in
the test body. Symptom under `--interp`:

```
field access out of bounds: field index 4 (offset 32+8 = 40)
exceeds object data size 8 type_id=6220 type='Item'
backtrace=[Item.eq@pc=4 <- test.test_eq_reflexivity@pc=35]
```

Root cause: `core.meta.contexts.Item` is a unit type (zero-field
record). The dispatcher's first-suffix-wins root (task #17/#39
manifestation through type-name lookup, not method-name) resolves the
test's local `Item { id: Int }` against the stdlib `Item` instead.
`self.id` field-index resolver then reads at the wrong offset
(0-field type vs 1-field test type), triggering field-OOB.

**Fix in this branch**: renamed test's local type to `EqReflexItem`
(deliberately unique). Pinned in `regression_test.vr §A` as
`regression_unique_typename_avoids_stdlib_collision` so future test
authors know to use unique names until task #17/#39 closes.

### 2.2 Pre-existing unit/property/integration tests largely green

After the `Item` rename, all 73 unit + 52 property + integration tests
pass cleanly under `--interp`. INVENTORY's "2 FAIL TBD" was likely
this single defect surfacing across multiple tests.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.protocols`:

* Every Verum module that uses operator syntax (`==`, `+`, `<`, `.hash()`).
* Every collection type (List / Map / Set / Deque) requires `Eq` +
  `Hash` for its key / element types.
* `f"{x}"` interpolation requires `Display`; `f"{x:?}"` requires `Debug`.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/`.

## §5 — Action items landed in this branch

1. `core-tests/base/protocols/unit_test.vr` — renamed test-local
   `type Item is { id: Int }` to `EqReflexItem` (in
   `test_eq_reflexivity`) to avoid `core.meta.contexts.Item` collision.

2. NEW `core-tests/base/protocols/regression_test.vr` — 12 active pins:
     §A regression_unique_typename_avoids_stdlib_collision
     §B Eq reflexivity (x.eq(&x))
     §C Eq symmetric (a.eq(&b) ⇔ b.eq(&a))
     §D Eq transitive (a==b ∧ b==c ⇒ a==c)
     §E Clone independence (Int + Text)
     §F Default int=0, Default text=empty
     §G Ord total order (lt/le/gt/ge cohere with cmp)
     §G' Ord on equal pair
     §H Maybe.cmp(None, Some(x)) = Less
     §H' Maybe.cmp(Some(3), Some(7)) = Less

3. NEW `core-tests/base/protocols/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Close task #17/#39 type-name dispatch to allow `Item` reuse | multi-day VBC codegen work | task #17/#39 |
| Audit other test files for stdlib-type name collisions | 1h | future task |
| `Hash` protocol exhaustive a==b ⇒ hash(a)==hash(b) coverage | 1h | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
