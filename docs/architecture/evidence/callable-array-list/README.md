# Callable array-to-List return conversion

A callable declared to return `List<T>` now copies a proved local packed numeric
array into a fresh growable List. Explicit returns, function tails, closure
returns and known empty arrays use the callable's own structural return target.
Returning a fixed array keeps its existing storage. Existing List results are
not reinterpreted as packed arrays merely because an intermediate declaration
has a fixed-array type.

The proof comes from emitted `NewByteArray`, `NewTypedArray` and `NewList`
instructions, including canonical multi-byte register operands and a known
nonnegative element count. Moves carry proof; overwrites invalidate it. Float
storage must use width four or eight. Calls, control-flow boundaries and Drop
glue discard proof when their effects are unknown. Callable compilation saves
and restores the enclosing facts. Deferred code has a separate instruction
stream, so its future allocations and assignments cannot change current facts.

The conversion reuses the existing packed-array copy emitter. It does not add a
bytecode field, change the declared numeric APIs, or establish a universal array
calling convention. In particular, declaration types and the old inferred local
array marks are not physical storage evidence.

## Verification

The original nine interpreter controls passed three and failed six before the
repair. The expanded callable suite passes all fifteen controls after the
repair, including the two deferred-buffer cases found during review. Five proof
unit controls cover wide registers, zero length, malformed operands, invalid
float geometry, unknown counts, overwritten registers and Drop effects.

Six native controls execute both source VBC and decoded serialized VBC through
production LLVM lowering and JIT. They verify length, indexing and growth for
byte, wider integer, floating and empty-array results, and retain List-backed
fixed-array results. The host allocation substrate is bounded and kept live
through each probe; this is not allocator-lifecycle or ordinary AOT acceptance.
The native gate retains all twelve emitted IR files.

The unfiltered VBC library gate passes 2,080 tests, fails none, and retains
one pre-existing T0839 ignored coverage-report test. It takes 510.091 seconds
including compilation (480.52 seconds in the test executable).
Four adjacent integration binaries pass another 71 controls in 33.265 seconds:
return targets, affine handoff, packed field normalization and array methods.
These and the native gate use the same frozen source `31713d3c3`.
`acceptance-manifest.json` pins the retained receipts, raw logs, IR and runners.
The original filtered unit receipt has an overbroad runner scope label; its
command and five-test result are the authority, not that label.

`baseline.json` and `emitted-results.json` preserve the initial causal gate and
first thirteen-control checkpoint. `review-cases.json` preserves both deferred
failures, the malformed-geometry failure, the repaired fifteen-control result,
and the initial unit-harness compile failure separately. Compressed logs retain
their exact original bytes and SHA-256 identities. Scope labels for filtered
unit commands are corrected in the retained evidence; the exact commands and
raw logs are unchanged.

## Remaining work

Unknown call results, forwarded parameters and control-flow joins do not gain
packed-storage proof from their declarations. Their required conversions and
the shared argument/result contracts remain open under T1700 and T1704. The
endian producer work remains held until those boundaries agree. Signed narrow-element decoding requires separate verification under T1706;
the measured integer materialization case is UInt32. The numeric
method-owner repair is independent and is not part of this tested source.

These gates do not rebuild the ordinary CLI or embedded standard library and
do not establish SHA-256, registry authentication, publication, service, FFI or
no-libc acceptance. The deferred-source controls establish compiler-state
isolation; they are not complete deferred-cleanup lifecycle tests.
