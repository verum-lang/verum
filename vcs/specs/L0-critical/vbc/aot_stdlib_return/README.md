# AOT stdlib-return layout mismatch regressions

Three tests guarding the fix for a bug where mounting a stdlib module
whose path was not in `clear_non_compilable_stdlib_modules`'s `ALWAYS_INCLUDE`
allowlist caused the AOT backend to silently emit unresolved calls and
SIGSEGV at runtime.

## Original symptoms

- `Rect.new(5,10,80,24)` compiled, but user-side `r.x` read from an absolute
  constant address (`mov w8, #0x728 ; ldr x19, [x8]` in the disassembly of
  `_verum_main`) → SIGSEGV on the first field access.
- A user-side struct literal `Rect { x, y, width, height }` read correctly
  from user code, but stdlib methods like `area()` saw zeros — the stdlib
  method had no body, so LLVM optimized it to return the zero-initialized
  stack slot.
- A user-defined `LocalRect` of identical shape worked end-to-end — proving
  the bug was tied to the stdlib → user codegen handshake, not to records in
  general.

## Root cause

`Pipeline::clear_non_compilable_stdlib_modules` ran after type-checking and
kept only a hand-maintained allowlist of stdlib module paths
(`core.collections.list`, `core.sync.mutex`, ...). Any mounted module
outside the list (here `core.term.layout.*`) was dropped from `self.modules`
before `compile_ast_to_vbc` called `collect_imported_stdlib_modules`, so the
impl-block ASTs never reached VBC codegen. The stdlib function stayed in
the qualified-function registry (from type-check), which meant the user's
`Call Rect.new` emitted a reference to a function id with no body. LLVM
declared the function as extern, saw no callers could observe side effects,
and const-folded the "return value" to an arbitrary pointer-shaped constant.

## Fix

`clear_non_compilable_stdlib_modules` now takes the user `&Module`, walks
its `mount` items, and retains every stdlib module that matches (or is a
submodule of, or is an ancestor of) any user-mounted path — in addition to
the original `ALWAYS_INCLUDE` list. With the fix, `core.term.layout`, its
`rect` / `flex` / `constraint` / `grid` / ... submodules survive the cull,
their impl blocks reach `compile_module_items_lenient`, and LLVM emits real
function bodies for the stdlib methods.

## Tests

- `rect_field_access.vr` — direct field reads on a stdlib-returned struct.
- `rect_method_on_literal.vr` — stdlib method on a user-built literal.
- `local_rect_baseline.vr` — identical-shape user type (baseline; never
  broken; guards against over-aggressive future culling).
