# Coherent platform acceptance — 2026-10-04, stage 10

This is a measured snapshot at `ca7d1334b23651258500217d805ea4e290d4d4ef`,
not a full-platform readiness claim. The default-feature CLI and ordinary
stdlib build completed in **1,449.36 seconds**, with no bake or verification
bypass. Source and saved artifacts remained fixed through interpreter and
native execution. Stage 9 was interrupted as superseded and supplies no
CLI/archive acceptance.

## Integrated changes and focused checks

- T1579 (`83d55d578`): preserve exact declaration ownership when imported
  aliases shadow local type names; the public compiler regression also joins
  the blocking CI archive step.
- T1581 (`8b67a2262`): route integer print through the existing internal
  writer, correcting ordering against Text/Bool output. Float printing is
  still open under T1581/T1582.
- T1583 (`45f11c439`): preserve absent parameter type spelling through archive
  import/finalization instead of inventing receiver ownership.
- T1580 (`ca7d1334b`): execute a selected user free-function body named
  `hash_value`; retain the genuine primitive native route. This bounded
  repair does not fix that route's pre-existing hash algorithm divergence.

The integrated controls passed **38 VBC**, **17 LLVM/JIT** and **40 source
gates**. The fresh archive passed **5 declaration-owner**, **58 archive** and
**1 explicitly enabled qualified-call** checks. The ordinary public compiler
owner regression passed **1/1**, using the normal cache hit for this exact
stdlib fingerprint. Fresh imported Int/Unit default parameters retain EMPTY
spelling. These focused checks do not replace the full platform suite.

## Exact build identity

| Artifact | SHA-256 |
|---|---|
| CLI | `459bc1a71d6238aee6f0e31b8e738995926a07e1176acec1b65894fcb70b45aa` |
| runtime.vbca | `a29c368523da045948a749d12a2c55d9b7184bb648028a0674afe70090226ae0` |
| runtime.core_metadata | `3e7e454e8e5b648275c824070319a74f0736e7b88da4731a1fb3c73bc2bfb115` |
| runtime.symbol_graph | `565e871b79329a73da4098aea8517d548bd196d37ec6a217084383473e3d6c94` |

The stdlib BLAKE3 input fingerprint is
`7b3bbaec364e3a8d94bf27397a4aca705a3af0bb2c7b79cecf3d364d0b401aa7`,
with schema `v46-2026-09-03-module_level_statics`. All three archive hashes
differ from stage 8; no decoded whole-archive equivalence is claimed for
this snapshot. The [machine-readable record](platform-acceptance-stage10.json)
contains the measured output, verdicts and identities without local paths.

## Actual execution

The combined interpreter program completed in **78.30 seconds**, with all
four core phases passing: collection/factory resolution, panic payload,
eager HTTP error handling and supervisor once initialization. The field
address control passed. The independent list-backing program passed all eight
expected outputs in **47.29 seconds**. The HTTP interpreter gate passed all
five contracts over four connections, including an exact binary body,
header/slow-drip deadlines, cancellation and a genuine zero-capacity read.
The separate Mutex assertion failed even though each process exited zero.

The same reference/scalar/hash source was executed through both tiers:
**46.41 seconds** interpreted and **510.93 seconds** AOT. The native executable
was observed directly, completed all phases with exit zero and had no
interpreter fallback. Its saved binary is 161,928 bytes, SHA-256
`8000bc65645643b51cdbf08ece43793e7122058620d9091fdad0e6f6a0767d8d`.

| Contract | Interpreter | Native AOT |
|---|---|---|
| Plain/Shared AtomicBool identity; AtomicInt 73/91 | PASS | PASS |
| Result predicates; unrelated Int results 9001/9002 in phase order | PASS | PASS |
| Nested field address / pointer control | PASS | PASS |
| Selected user `hash_value(7)` body answers 37 | PASS | PASS |
| Primitive `Int.hash_value(7)` matches declared DefaultHasher | `-1747035056885442531` | FAIL: `5465015992139406178` |
| Mutex guard scope and explicit drop; expected true,false,false | FAIL: false,false,false | FAIL: true,true,true |

The original native runner stays **FAIL**. Its strict whole-output comparison
also fails because the primitive hash value differs, even though the scalar
predicate ordering defect is fixed. T1587 records the algorithm divergence:
the declared/interpreter DefaultHasher uses FxHash over little-endian bytes,
while `verum_generic_hash` uses FNV-1a. This is deterministic, not seed drift.
T1538/T1540 retain the ownership/Drop failure. The interpreter runner also
stays FAIL for Mutex; correct independent phases do not override it.

A separate original inline-module control completed under this same fresh
CLI but executed the wrong root body: `hash_user=77` instead of 37, while
same-leaf sibling results remained 57/77. T1584 tracks root declaration
ownership. The earlier interpretation of an incomplete log as a missing
`main` was withdrawn; it is a wrong-body result, not absent execution.
The later isolated T1584 repair is not included in this snapshot.

Static inspection of that exact native Mach-O reports only
`/usr/lib/libSystem.B.dylib` as a dynamic dependency; its undefined symbols
contain neither `printf` nor `snprintf`. This satisfies the documented Darwin
dynamic-library boundary for this control, without certifying Float output,
other programs, static-library provenance or Linux/Windows output. Behavior
acceptance remains FAIL as recorded above.

## Remaining boundaries

Native Float formatting, native List storage, returned-reference authority
for the actual emitted callee, and aggregate ownership remain open. No full
mutex suite or native HTTP/combined acceptance was rerun here. Static/JIT
success is not counted as a target executable result.

The separate [AOT and host dependency audit](no-libc-architecture.md) describes
strict no-libc for generated AOT programs and clean-supported-OS packaging
for the host CLI/interpreter. The six inspected rolling release assets were
not built from this stage-10 snapshot and must not be conflated with it.
