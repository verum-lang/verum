# Standalone residual probe startup boundary

The [probe](probe.vr) compares `?` inside a record initializer with the
same operation in a preceding local binding. Both calls should return the
original `Err("missing")`; the exact expected stdout is recorded in each
receipt.

Both identified ordinary CLI runs fail before `main` in global initialization:
`NullPointerAt { op: "opcode 0x62", site: "ArenaPool.new", pc: 27 }`.
The [comparison](comparison.json), [stage18 receipt](stage18/result.json)
and [stage19 receipt](stage19/result.json) retain the same fixture hash,
executable identities and raw logs. Stdout is empty in both runs. This
startup failure predates the T1684 borrowed-element repair and gives no
inside/outside error-propagation verdict. T1688 owns its diagnosis.

The full registry metadata component separately reaches a rejection oracle.
Its [message-only diagnostic](../registry-metadata-stage19/negative-diagnostic/diagnostic-change.json)
identifies a document missing `description`; that application observation
must not be replaced by the failed standalone probe. T1687 owns the
compiler return-context defect found while inspecting that input.

The [project control](project-stage19/result.json) prepends only a module
header and supplies a cog manifest. It fails at the same pre-entry point,
so standalone file selection does not explain the fault.

The [identified initializer trace](trace-stage19/result.json) records its
extra trace environment and preserves the complete stderr. At
`__tls_init_CHILD_ID_COUNTER`, a call with `Int(1)` and encoded target
`536870955` resolves to `ArenaPool.new` (descriptor 165). The source
initializer in `core/runtime/supervisor.vr` calls `AtomicU64.new(1)`.
This establishes a wrong startup call target; it does not yet determine
which producer or importer mapping introduced it. The
[structured analysis](trace-stage19/analysis.json) records that boundary.
