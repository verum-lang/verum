# Selected array result storage checkpoint (T1704)

The frozen source is `b6d0dff764543e856f92ea1bf2de0f6b6ca2d64a`, tree
`0b917a6bafd72a5a879eb7ba156c049ca9602990`. This is a held integration
checkpoint, not a main-branch or registry runtime acceptance statement.

| Gate | Result | Receipt |
| --- | --- | --- |
| Parsed source and serialized VBC interpreter controls | 6 passed, 0 failed | [Final source gate](vbc-source-final/result.json) |
| Shared emitted-storage proof units | 9 passed, 0 failed | [Proof gate](vbc-proof-cleanup-fixed/result.json) |
| Native selected-body and instruction-transfer units | 5 passed, 0 failed | [Native proof gate](native-proof-final/result.json) |
| Parsed source and decoded-wire LLVM/JIT controls | 16 passed, 0 failed | [Native consumer gate](native-consumers-final/result.json) |
| Source privacy, complete reference/citation gate, panic, early-return and duplicate-emitter ratchets | passed | [Source gates](source-gates/result.json) |

The nine shared units ran at `dda41dd00`. Their implementation and test files
are byte-identical to the final source; the [identity record](shared-proof-source-identity.json)
verifies both files. The other final executable gates ran at `b6d0dff76`.
The four test-executable hashes were rechecked before the exclusive target
was returned to the coordinator; see [handoff](target-handoff.json).

Frontend array calls retain representation-neutral indexing. Native lowering
uses the selected LLVM callable and its completed source body to distinguish
an actual List from an actual packed allocation, even when their declared
array signatures are identical. Source-body presence alone establishes no
storage proof. Ambiguous descriptor ownership, bodyless or pre-existing native
targets, unsupported control flow and unknown argument storage supply no proof.
FunctionContext keeps this state private and advances its facts once per VBC
instruction. Indexed reads, writes and length use the same physical authority.

The controls exercise inferred and explicit bindings, both declaration orders,
List versus packed returns, indexed mutation and length, byte/UInt32/Float
geometry, canonical wide registers, interpreter allocation-bound refusals,
native checked-load IR, and exact source/wire decoding. Positive native cases
and the reachability pair execute both routes; other deliberate native
refusals currently test parsed-source modules. The complete-module reachability pair shows that an
unused array-parameter helper permits an independent main, whereas calling
that helper surfaces the unproved-storage error before JIT execution.
No-main library builds and conservative reachability roots remain broader
than this simple program control.

Earlier failures are retained, not replaced by the final result:

- `982b24482` did not compile its new interpreter fixture because two calls
  needed the existing `Shared::into_arc()` API boundary. No tests executed.
- At `5f47257b2`, the native gate passed ten controls and refused six positive
  consumers before JIT. Failure-only body diagnostics at `0268d3351` showed
  compiler-emitted cleanup after the final indexed access and one scalar
  `CvtToI`; they did not show a hidden branch. The initial attempt to use the
  existing VBC dump flag emitted no body in this harness and is recorded
  separately.
- `dda41dd00` treats `DropRef` like a returning call only in the caller's flow
  classifier. Actual transfer still clears every storage fact, and completed
  return-body summaries still reject drop glue. `CvtToI` explicitly invalidates
  its destination. The negative controls remain enabled.

Native execution uses the test's bounded host allocation substrate. This
establishes the specified LLVM lowering and JIT assertions, not allocator
lifecycle, native executable linking, the AOT no-libc invariant, or a fresh
standard-library archive. All inherited archive, metadata, symbol-graph,
checksum and schema bytes stayed unchanged in every Cargo gate.

Remaining acceptance boundaries are explicit:

- Final T1701 source-owner/native parity is not included. This branch retains
  earlier T1701 pieces and the source-body presence API, not the later final
  dispatch repair. Native numeric interception must still be tested against
  genuinely selected source bodies and different declared return contracts.
- The native loop control intentionally expects refusal. SHA-256 finalization
  loops over returned bytes, so no repaired ordinary SHA-256 or registry
  authentication result follows from this checkpoint.
- Fixed-array parameter/forwarder storage, unknown or mixed call results,
  general call-boundary materialization and CFG meets remain unproved. T1700
  retains the broader Array-to-List boundary work. Signed narrow semantic
  decoding is a separate T1706 concern; physical width does not establish it.
- The required integrated full VBC gate, ordinary product rebuild, registry
  replay and AOT acceptance remain pending. The held endian baseline evidence
  is unchanged.

Each receipt retains its exact command, source and executable identity,
explicit environment overrides, deadline, logs and source/artifact checks.
`run_gate.py` is the exact bounded runner used for these Cargo checks. Text
logs and LLVM IR are losslessly gzipped; their original uncompressed names
remain in the receipts. [manifest.json](manifest.json) maps stored files to
both original and compressed hashes. No source filtering, declaration
retyping, ownership renaming or successful-publication acknowledgement was
used to make these controls pass.
