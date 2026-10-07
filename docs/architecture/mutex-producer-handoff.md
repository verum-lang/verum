# Mutex producer handoff: 2026-10-07 diagnostic

Status: **FAIL; T1602 remains open.** A single ordinary interpreter run shows
that `Mutex.lock()` releases its guard before the caller extracts the returned
Result. This is a diagnostic of the existing implementation, not a repair.
The [self-contained evidence record](mutex-producer-handoff.json) preserves the
exact source, output oracle, command, environment, artifact hashes and excerpts.

## Measurement and first failing boundary

The run used the hash-verified [stage-14 product](platform-acceptance-2026-10-07-stage14.md),
source `fdad59f06114fd2770f3d53058373764284c7c2e`. It completed in **115.70 seconds**,
exit **0**, without a timeout. The CLI and three saved standard-library
artifacts matched their recorded hashes both before and after execution.
No Cargo, CLI bake, AOT run or production edit was part of this diagnostic.

The fixture separates the original chained expression into these steps:

```verum
let outcome = scoped.lock();
print("after_lock");
print(scoped.is_locked());
let guard = outcome.unwrap();
print("after_unwrap");
print(scoped.is_locked());
drop(guard);
print("after_drop");
print(scoped.is_locked());
```

| Observation | Required | Actual |
| --- | --- | --- |
| After `lock()` returns, before `unwrap()` | `true` | `false` |
| After payload extraction | `true` | `false` |
| After explicit drop | `false` | `false` |

The first wrong boundary therefore precedes `unwrap()`. Exit zero does not
make this oracle pass. A script-cache storage warning occurred after the
completion marker; the run was not retried.

## Runtime trace and producer evidence

Existing `VERUM_TRACE_DROPFN` and `VERUM_TRACE_CALLS` diagnostics record this
order between the source markers, at log lines 698–710:

```text
before_lock
FutexLock.lock
FutexLock.try_lock
MutexGuard.drop  (type1593, runtime function14373)
FutexLock.unlock
after_lock
FutexLock.is_locked
false
```

This is a condensed event sequence; the JSON contains the exact lines,
including intervening atomic-ordering calls. A second guard-drop/unlock pair
appears at lines 715–717 during the explicit `drop(guard)` statement.

The same invocation used `VERUM_DUMP_VBC=Mutex`. Its `Mutex.lock` body creates
the guard, binds it to `r4`, and reaches the successful Result branch with:

```text
pc=87  #19  SetVariantData { variant: Reg(9), field: 0, value: Reg(4) }
pc=91  #20  Mov { dst: Reg(10), src: Reg(9) }
pc=94  #21  Mov { dst: Reg(1), src: Reg(10) }
pc=97  #22  Mov { dst: Reg(11), src: Reg(1) }
pc=100 #23  DropRef { src: Reg(4) }
pc=102 #24  Ret { value: Reg(11) }
```

The payload store leaves the named local's cleanup active. There is no
consuming source-clear operation before `DropRef`. This supports the
producer/local-cleanup explanation of the measured early unlock.

**Correlation limit:** the dump precedes runtime function-ID remapping:
`MutexGuard.drop` has dump ID20996 and runtime ID14373. The runtime trace
contains neither caller PCs nor object addresses. Do not describe dump pc100
as an observed execution PC or claim an independent same-object identity
proof. The ordered runtime trace establishes early destructor/unlock activity;
the bytecode excerpt establishes the corresponding structural cleanup gap.

## Remaining ownership and native acceptance

The [resource-mode contract](resource-mode-contract.md) requires a selected
Borrow, Copy or Transfer operation with exact binding/use and declaration
identity. The [value-copy contract](value-copy-contract.md) keeps ordinary
copies, references and Shared values distinct. `MutexGuard` is still an
ordinary record: the existence of its Drop method cannot select a transfer.

T1602's direct named affine local and explicit-return handoffs do not cover
this aggregate payload. The semantic formal roster preserves declaration
facts but does not itself implement consuming call or aggregate cleanup.
See the [native lifecycle gap](native-drop-lifecycle-gap.md) for those limits.
Suppressing the local Drop alone would not establish receiving-wrapper cleanup
or exactly-once consuming extraction; discarded wrappers must also be tested.

Native was **not rerun**. The separate [stage-13 native record](platform-acceptance-2026-10-07-stage13.md)
retains `true,true,true` against required `true,false,false`: cleanup was
missing there. This interpreter diagnostic neither repairs nor replaces that
native failure, and does not close T1538, T1540 or T1602.
