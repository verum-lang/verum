# Audit — `core/base/cell.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/cell.vr` (626 lines) |
| Tests | NEW — `unit_test.vr` (~90 LOC), `property_test.vr` (~110 LOC, round-trip / replace / update / borrow / once) |
| Hardcodes in `crates/` | RefCell borrow-counting integrated with CBGR runtime |

## §1  Interior mutability and CBGR

The cell types let you mutate through `&T`, sidestepping CBGR's
default "shared ⇒ immutable" rule. The contract:

- **Cell<T>**: only `Copy` types; no aliased reference into the value
  is exposed; safe by construction.
- **RefCell<T>**: dynamic borrow-counting; `borrow()` and
  `borrow_mut()` panic at runtime if rules are violated. Safe via
  enforcement.
- **OnceCell<T>**: write-once; subsequent writes are no-ops.
- **LazyCell<T>**: like OnceCell with an init closure.

CBGR cannot statically prove RefCell safe, so RefCell *requires*
runtime checks. That's the trade-off.

## §2  Aliasing-violation observable tests (deferred)

The most important contract for RefCell is "double-mut-borrow
panics." Today our property tests don't directly exercise this —
they'd require `assert_panics` around a test function that holds a
borrow_mut and tries to borrow_mut again. This is a stretch test
that should be added in a follow-up.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/cell/`
- [x]  `unit_test.vr` — Cell get/set/replace/take/update,
       RefCell borrow/borrow_mut, OnceCell get_or_init
- [x]  `property_test.vr` — set/get round-trip @property,
       replace contract @property, update composition,
       RefCell observability, OnceCell single-set,
       @test_case increment truth table
- [x]  This audit document

## §4  Action items deferred

1. **RefCell double-borrow-mut panic test** — pin the contract via
   `assert_panics`.
2. **LazyCell init-once-then-replay test** — same shape as OnceCell.
3. **Cell.swap exchange** — paired-cells swap operation if exposed.
4. **Concurrent access** — Cell/RefCell are Send+!Sync; verify the
   marker bounds are correct in the type-checker.
