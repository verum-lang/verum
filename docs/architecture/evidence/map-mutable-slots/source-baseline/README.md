The complete production Map source and parsed caller compile, but all seven
controls fail before mutable lookup. Both source and decoded-wire executions
reach the source `Map.insert_internal` body, whose `Slot.hash` read rejects the
flat runtime backing. The owned-get positive fails at the same point.

This is a harness boundary result, not a causal baseline for `get_mut`. The
ordinary compiler diagnostic separately reaches `Map.get_mut` after its read
oracle succeeds. A replacement focused harness must establish the same selected
constructor/insert dispatch before interpreting mutable-slot assertions.

The [manifest](manifest.json) binds the frozen source, unchanged archive files,
raw log, runner and retained executable. No production behavior or assertion
was changed during this run. No AOT, lifetime-verifier or stale-borrow acceptance
is claimed. The broader source/runtime Map representation split remains open.
