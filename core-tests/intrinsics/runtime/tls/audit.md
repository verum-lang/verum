# `intrinsics/runtime/tls` audit

Module: `core/intrinsics/runtime/tls.vr` (~155 LOC) — TLS slots, frames,
typed offset access (V-LLSI context backing, ~2ns hot path).

Tests: unit (6) — slot set/get/has/clear round-trips + independence +
idempotent clear + balanced frame push/pop + non-null base.  HIGH slots
(240-244) only: TLS is process-global under an in-process runner, and the
context system owns the low slots — hygiene (clear before finish) is part
of each test.

## Resolution (2026-07-04)

Slot trio FIXED — suite 6/6 interp: the quartet rode
`DirectOpcode(TlsGet/TlsSet)` whose emission writes DST FIRST, so TlsSet
read the destination register index as the SLOT (writes landed in slot
<temp-reg>); `has` returned the stored pointer where Bool was promised;
`clear`'s value operand was missing.  Now `SystemSubOpcode` 0x59-0x5C
with the declared shapes — and a DEDICATED `user_tls_slots` store:
probing found the context system's own table already populating high
slots (245 held a context object), so user slots get their own map
(the AOT twin `__verum_tls_slots` thread_local global has the same
isolation).  AOT residual: tls_get_base arm (task #5).

## Coverage decisions

* `tls_read_ptr/write_ptr/read_i32/write_i32/read_usize/write_usize`
  (byte-offset typed access) — NOT suite-driven: they address raw offsets
  into the TLS block; a wrong offset silently corrupts the context
  system's live state for the whole test process.  Needs a
  subprocess-isolated harness (same class as the cbgr global mutators).
* Frame push/pop tested as a balanced pair only; nesting semantics with
  live `provide` blocks are the context suite's domain
  (`core-tests/context/*`).

## Crate-side drift surfaces

* `TlsGet (0x4E)` / `TlsSet (0x4F)` opcodes; `PushContext (0xB3)` /
  `PopContext (0xB4)`.
* Doc drift FIXED in website f6d9a6c: `tls_slot_get` returns
  `*const Byte`, not `Maybe<Int>`.

## Action items

* Subprocess harness for typed offset access — deferred.
