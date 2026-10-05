# Producer-owned value-use receipts

The first T1599 vertical publishes observations of operations that the common
source producer already emits. It does not change `Clone`, `Mov`, `DropRef`,
argument ownership, returned-reference representation, or destructor execution.
T1538/T1540 lifecycle acceptance therefore remains open.

## Authority and current implementation

`RegisterAllocator` allocates a function-local `BindingId` for each parameter or
local declaration. Names and recycled temporary registers are not identities.
Nested function/closure/spawn/generator compilation snapshots the outer recorder
alongside its instruction buffer. Branch compilation does not restore that
snapshot. Each retained emission gets a separate `ValueUseId`.

`VbcCodegen::compile_function` publishes regular parameter types only after the
current declaration's exact generic-ID roster is established and before the
prologue. `compile_let` publishes a supported explicit annotation, or propagates
an already proven direct initializer fact. The existing signature resolver
supplies semantic `TypeRef`; the shared `resource_discipline_with` query consumes
exact descriptor IDs. Unsupported types, unresolved generics and missing legacy
metadata remain unknown. Cached definition text is not an authority.

`CodegenContext::emit` records:

- `Clone`: the current value-copy operation, only for proven unrestricted
  non-reference source values. It says nothing about a fresh allocation; a
  shared handle may still use its existing sharing policy.
- `Mov` of a proven reference: borrow forwarding, without referent ownership.
- Direct `Call`/`CallG` arguments and `Ret`: their exact source binding and use
  site when available. By-value ownership handoff remains unknown; a proven
  reference can be classified as borrow forwarding.

Affine, linear and unresolved source uses never receive Copy or Transfer
permission. `Transfer` and carrier-copy vocabulary are reserved and are not
emitted by this stage. Reference classification is semantic evidence, not a
native pointer/address representation rule.

## Persistence and existing CFG consumer

VBC minor 20 adds an optional function-descriptor receipt tail. Missing older
metadata is unknown. The receipt plan is sealed against canonical body bytes
and semantic signature/use facts. The reader verifies instruction indices,
ordered use IDs, opcode/operand/site compatibility and current exact resource
discipline. Stale body, signature or receipt facts are declined.

Source finalization reseals only its existing one-to-one instruction operand-ID
map. Archive import validates the original body first, remaps semantic TypeRefs
with that archive's exact type map, and reseals the unchanged instruction
positions. Unaccounted rewrites invalidate the plan by its seals. The linker
explicitly discards plans because it does not yet relocate their source facts;
specializations do not inherit permission from an old generic plan.

`VbcModule::value_use_cfg` constructs the existing CBGR `ControlFlowGraph` with
separate `value_uses` events. It supports ordinary direct branches; exceptional
and suspending control flow is declined. Events are not converted into the
older mutable-reference-use/deallocation approximation. No escape analysis,
CBGR reference ticket or destructor decision consumes them as cleanup authority.

A plan retains at most 16,384 uses. Exhaustion discards the whole plan. CFG
materialization decodes at most 65,536 instructions before retaining another
one. Missing/declined facts never silently become transfer permission.

## Next executable ownership step

Local initialization currently calls `binds_a_copy_of_a_place`, emits `Clone`
into a temporary, and then `compile_pattern_bind` allocates the destination
binding. This means the copy observation can precede its destination ID. The
next producer operation must explicitly join those identities and choose a
semantic duplication/transfer operation. Replacing `Clone` with `Mov` alone is
incorrect: lexical cleanup would still own both registers.

Direct call argument packing emits `Mov` into a consecutive range; the receipt
can identify the source, but exact selected parameter consumption and callee
cleanup obligations still need a joint contract. A call result is not currently
published as a newly owned binding merely because its nominal type is known.

`compile_return` and block-tail preservation identify direct source operands.
`compile_block` saves the result in a fresh register before `exit_scope` emits
cleanup. The next stage must atomically transfer the cleanup obligation from
the exact source binding to the escaping result; reference forwarding must not
acquire referent ownership. Aggregate fields, variant payloads and call-return
values need their own producer operations and obligation identities. Current
receipts do not reconstruct them from pointer equality, type names, `drop_fn`
or a later native register mark.

Only after those operations and CFG joins carry real ownership obligations can
both interpreter and LLVM enable the same cleanup decisions. Receipts alone do
not close early/duplicate Drop, MutexGuard lifetime or aggregate-result escape
failures.
