# `intrinsics/runtime/tls` audit

Module: `core/intrinsics/runtime/tls.vr` (~155 LOC) — TLS slots, frames,
typed offset access (V-LLSI context backing, ~2ns hot path).

Tests: unit (6) — slot set/get/has/clear round-trips + independence +
idempotent clear + balanced frame push/pop + non-null base.  HIGH slots
(240-244) only: TLS is process-global under an in-process runner, and the
context system owns the low slots — hygiene (clear before finish) is part
of each test.

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
