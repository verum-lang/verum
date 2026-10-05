# Platform acceptance: 2026-10-05, stage 12

Status: **not accepted**. These results belong to the frozen build below;
subsequent source fixes do not change its verdicts.

## Build identity

Source: `dcfa0fa0a6b308d65f3ed17d1ac08a0a70be7b0a`.
Ordinary `cargo build --locked -p verum_cli --bin verum`, default stdlib bake,
cache and SMT, completed in 1875.41 seconds. Schema:
`v50-2026-10-05-list-storage-access`; source fingerprint:
`be7c410382345d3365377c9dd0586e548a56dda3daebed9235194ffb41fe00b4`.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Host CLI | 493780520 | `861edd9cbe97139ecc3acb186277da8471c467a21981947e5a74043e7fd0466a` |
| runtime.vbca | 29796130 | `1d7408b40aa434807ac52806e338a321980aa447515a90a0d593c0d15923ebb9` |
| runtime.core_metadata | 78604634 | `daae66501d5c60d6d9b5565264eba2fe5114e290131b72364d92effc7fc285f1` |
| runtime.symbol_graph | 5914480 | `6ec34a0195ee7f9418d5b953af874d9c6361343246818c05dca7561714b9ed4f` |

## Actual execution

* Numeric/List interpreter control: **PASS**, exit 0 in 586.24 seconds.
  All 17 expected lines match, including Float32/Float64 formatting,
  signed zero, small and large decimal values, integer/Boolean output
  ordering and a List<Int> value. Source SHA-256:
  `eb3fccab7c2c65ad6777b9c63bdbf837569733f8afa019a32b01c549dc86dc40`.
  This does not exercise packed Byte allocation or resizing an empty List.
* Returned-reference interpreter control: **PASS**, exit 0 in 40.92 seconds.
  All seven expected lines match for nested field/method projections,
  mutable and scalar references, mounted Once/OnceLock, and Supervisor.
  Source SHA-256:
  `e03fe68943aaf03be6550f25cb7ed85975867a2e11155f0561077d2287b998ce`.
  Existing SMT reflection warnings remain in the log; verification was
  not disabled.
* Native-control source on the interpreter: **FAIL** before execution,
  exit 1 in 8.70 seconds. Three E400 errors reject `load`/`swap` through
  `core.base.memory.Shared<core.sync.atomic.AtomicBool>`: the constructor
  now retains its owner but the Deref impl is still indexed by a bare key.
  No Mutex or atomic runtime verdict follows from this run. Source SHA-256:
  `918adcc5d66a70f18aed78f031c6912bd14b9f34fa1acf523e1f7e79c752efd3`.
* Numeric/List AOT: **FAIL**, timeout after 1800.07 seconds, exit -9.
  No native launch or runtime output was observed. The same source was
  used as the interpreter control. Log SHA-256:
  `37b20798d912a7e1c71054563d5ba80ba1f84250a657c6689e7fd8af6f4875ac`.
  This is a compilation-time failure, not a numeric runtime mismatch.
  The returned-reference AOT control was not run on this snapshot.

## Follow-up source corrections

T1606 retains the exact metadata declaration owner when publishing Deref
impls, without guessing from the leaf name. The frozen metadata reproduces
the failure with the old checker and passes the focused source correction;
a fresh CLI is still required.

A one-second sample during the timed-out AOT compile found all 101 sampled
worker stacks in the GetVariantData type-name search. Each outer descriptor
scan called a name lookup that rescanned the same type table. T1608 builds an
index once for each immutable LLVM input module and preserves the original
first-TypeId and first-name selection. Two identity/order controls pass.
The correction has not yet been measured in an ordinary rebuilt CLI; no
end-to-end speedup or AOT acceptance is claimed from the sample.

## Dependency boundary and outstanding acceptance

The exact macOS host CLI passes the direct-import inspector: all ten imports
are baseline system libraries/frameworks; none is a Homebrew/OpenSSL dylib.
This is not a clean-OS or published-release test and establishes no Linux
GLIBC or Windows runtime baseline. The [dated release audit](no-libc-architecture.md)
remains applicable to its measured artifacts.

The stage-11 runtime allocator repair has not yet passed a fresh native
execution. Native Drop/Mutex lifetime, aggregate ownership transfer, packed
List reference carriers and complete platform/release conformance remain
open. Prepared controls and source/JIT tests are not substitutes for those
acceptance runs.
