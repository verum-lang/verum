# Flex-layout regressions — **all green** ✅

Regression suite for Tier-0 VBC bugs that previously prevented the
full `core.term.layout.FlexLayout.compute` pipeline from running.

## Suite

| File | Scope | Status |
|---|---|---|
| `with_capacity_ctor.vr` | `List/Map/Set.with_capacity(n)` return real built-in collections (not flat stdlib records) | ✅ passes |
| `compute_three_panes.vr` | `FlexLayout.row().compute(area, &items)` on a toolbar/main/sidebar layout | ✅ passes |

## Root cause

`List.new()` was already special-cased in `compile_method_call` to emit
the `NewList` opcode, which the interpreter binds to the
`TypeId::LIST` handler with proper heap layout (`len`, `cap`, `data`
slots). `List.with_capacity(n)` wasn't — it fell through to the stdlib's
compiled function body, which builds a plain record and returns it.
The record had no `TypeId::LIST` header, so every later
`len`/`push`/`[i]` call either:

* read garbage from the wrong offset (`len() = 51173805664`), or
* segfaulted because the element pointer was uninitialised.

Cascading effect: `FlexLayout.compute` opens with
`List.with_capacity(n)` for its `hyp_sizes` vector, so the solver
SIGSEGV'd on the first `.push(clamped)`.

## Fix

`crates/verum_vbc/src/codegen/expressions.rs` — extend the builtin
collection-constructor fast-path in `compile_method_call` to handle
`<Collection>.with_capacity(n)` in addition to `<Collection>.new()`:

* Emit `NewList` / `NewMap` / `NewSet` directly
* Evaluate the capacity argument for its side effects, then drop it
  (the VBC heap auto-grows, so the hint has no runtime effect)

Applies to `List`, `Map`, and `Set` — all three have a
`with_capacity(n)` in the stdlib that would otherwise miscompile.
