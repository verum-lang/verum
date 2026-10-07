# Platform acceptance: 2026-10-07, stage 14

Status: **not accepted**. Four complete public interpreter controls pass;
two others retain the known premature MutexGuard cleanup failure. The
[machine-readable record](platform-acceptance-2026-10-07-stage14.json) contains
the exact source snapshots, output oracles, artifact identities and log hashes.
The frozen source is `fdad59f06114fd2770f3d53058373764284c7c2e`.

## Ordinary build and source checks

`cargo build --locked -p verum_cli --bin verum` completed in 2654.25 seconds
with default features, automatic standard-library baking, cache and SMT.
The archive was rebuilt. Its schema is
`v52-2026-10-05-semantic-formal-parameters`; source fingerprint:
`def6d3e7b7421caf163b63529b2d0edd4bc2a09ec0004eb9cb89854b1cbae868`.
The CLI is 495337656 bytes, SHA-256
`0e488832a40517d260e50947a8788548484c4da931f49b92e4c1c7734858719a`.

The full parser/type-checker tests passed 8127 checks, with three existing
ignores. VBC passed 2063 library checks and 95 selected integration checks,
with one existing library ignore. The native code-generation library passed
318 checks. These are 10603 passing source checks; they are separate from
the following executions through the ordinary CLI.

## Actual interpreter results

Each source and expected output was retained unchanged from the earlier
control, except that primitive hashing now also runs the existing durable
`vcs/specs/L0-critical/stdlib-runtime/primitive_hash_parity.vr` fixture.
Runs used the frozen CLI and checked its hash before launch. Results below
use the measured product process time, excluding the outer Python runner.

| Control | Result | Seconds |
| --- | --- | ---: |
| Returned references, Once and Supervisor | PASS; all seven expected lines | 146.87 |
| Declaration properties through inline-module aliases | PASS; `true,16,8,8,16` between the expected markers | 135.42 |
| Primitive hashing, DefaultHasher, Map and Set | PASS; all eleven expected lines, including integer endpoints | 133.98 |
| Numeric formatting and Int/Byte List storage | PASS; all twenty expected lines | 642.85 |
| Atomics, predicates, field addresses, hash selection and order | Those oracles PASS; Mutex lifetime FAIL | 69.46 |
| Root function versus inline-module sibling identity | Identity/order PASS; Mutex lifetime FAIL | 69.60 |

The List control now completes Byte swap/reverse and empty shrink/regrow,
reporting `byte_list_values=42,255` and `byte_list_empty_regrow=37 len=1`.
The prior capacity panic remains a recorded stage-13 failure. This establishes
the bounded interpreter storage correction; native packed constructors and
borrowed List consumers remain under T1575/T1450.

The returned-reference control now obtains `name=root` from Supervisor, and
the declaration control resolves `mount wide as short` through the actual
compiler. T1613 and T1614 meet their bounded acceptance criteria. Together
with the source/archive tests, the fresh structural-reference generic call
also completes T1596. Conditional array-count work T1617 and fixed-array
method dispatch T1615 are later changes and are absent from this artifact.

Both Mutex controls exit zero but report `false,false,false`; the required
sequence is `true,false,false`. Their overall verdict remains FAIL. Native
owned Drop, argument/aggregate ownership handoff and MutexGuard lifetime
remain open under T1538/T1540/T1602.

## Native and dependency boundary

Native AOT was not rerun for this revision. The last actual native results
remain in the [stage-13 record](platform-acceptance-2026-10-07-stage13.md):
the scalar/atomic control executes, Mutex lifetime fails, and numeric/List
compilation times out before native launch. The stage-14 interpreter results
do not promote any of those native verdicts.

The exact new host CLI passes the direct-import check with ten macOS system
libraries. Transitive dependencies, oldest-supported OS execution and the
Linux/Windows release baselines still require separate acceptance. Strict
no-libc remains an AOT/runtime requirement, with the documented macOS system
ABI exception; baseline system libraries remain allowed in the host CLI.

## Measured compilation costs

A five-second sample during the standard-library bake observed 164 of 836
main-thread stacks under function export, including 126 under its deep
registry clone. T1622 addresses discarded metadata copies while preserving
canonical aliases and stub replacement.

A separate five-second sample during numeric/List compilation observed all
2858 worker-thread stacks in filtered archive registration. Its wanted-name
loop computes path prefixes and formatted suffixes before rejecting unequal
leaves. T1625 indexes candidates without changing the existing owner and
first-wins rules. These are observations of the sampled phases. They do not
establish whole-build percentages, memory attribution or an end-to-end
speedup. Both optimizations remain outside the frozen stage-14 source.
