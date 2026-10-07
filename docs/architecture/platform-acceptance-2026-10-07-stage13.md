# Platform acceptance: 2026-10-07, stage 13

Status: **not accepted**. The immutable source and artifacts below predate
follow-up corrections; their failures remain failures after those corrections.
Machine-readable identities, source/log hashes and oracle results are in the
[stage-13 record](platform-acceptance-2026-10-07-stage13.json).

## Frozen build and source gates

Source: `4e93cfa61102cbc871c05d352ffd690463a47a3f`.
Ordinary `cargo build --locked -p verum_cli --bin verum` used default features,
stdlib baking, cache and SMT, completing in 1626.85 seconds. Cache schema:
`v51-2026-10-05-list-storage-resize`; source fingerprint:
`dace3efb594aba70e29f73d79ad60dbc88f57b0a27c6a6f95bb0a4138ee1a479`.
The CLI is 494146664 bytes, SHA-256
`d4a40596c2f2b81a320c99498e6e648aa8b9c556f79a77be43fcc3eb09de4b24`.

The parser/types suites passed 8106 tests with three existing ignores;
LLVM/codegen passed 511 tests. VBC passed 2060 library tests (one existing
ignore) and 147 selected integration tests with codegen enabled. The ordinary
embedded-metadata Deref-owner test also passed with the frozen archive. These
source checks do not replace the following public CLI executions.

## Actual interpreter and native execution

| Control | Interpreter | Native AOT |
| --- | --- | --- |
| Atomics, predicates, field addresses, hash body selection and output order | PASS, completed in 68.29 s | PASS, compiled and launched in 764.17 s |
| Mutex lifetime in that same control | FAIL: `false,false,false` | FAIL: `true,true,true` |
| Root function versus same-leaf inline-module siblings | Declaration identity and output order PASS, 66.26 s; the retained Mutex check still fails | Not run |
| Numeric/List control | FAIL after valid numeric and Int-list prefixes, 667.05 s | Compilation timeout, 1800.07 s; no native launch or output |
| Returned-reference/Once/Supervisor control | FAIL at Supervisor name after earlier reference and Once checks, 150.90 s | Not run |
| Declaration properties through inline-module aliases | FAIL before output, 119.41 s | Not run |

Mutex requires `true,false,false`: the lock must remain held inside its scope
and be released after scope exit or explicit drop. A process exit of zero
does not override the failed lifecycle oracle. T1538/T1540 and ownership
handoff remain open.

The native control now reaches user entry, completes both direct and shared
atomic operations, selects the user's `hash_value` body (37), and produces
the same primitive hash as the interpreter (-1747035056885442531). This is
bounded execution evidence for the allocator startup correction (T1603),
qualified Deref metadata (T1606), and hash parity (T1587), not full runtime
acceptance. The root/sibling fixture separately verifies T1584 through the
ordinary public compiler.

## Retained failures and follow-up work

* **T1575:** the Byte-list resize/swap sequence panics with `List storage range
  exceeds capacity`. The ordinary List method interceptor treats a Byte-list
  owner as an inline array during swap/reverse and corrupts its metadata.
  A checked common-storage correction passes causal and neighboring source
  controls; it still needs an ordinary rebuilt CLI and native acceptance.
* **T1613:** lowercase value `self.inner.config.name` is classified as an
  associated type property. Earlier returned-reference and Once checks pass,
  but Supervisor name returns the wrong value. The source correction preserves
  the distinction between value `self` and type `Self`; fresh CLI is pending.
* **T1614:** `mount wide as short` cannot resolve the root inline namespace
  in the ordinary compiler. Direct checker tests that supply a synthetic
  registry entry do not cover this pipeline. Source/public-pipeline recovery
  is tracked separately.
* **T1608:** removing the quadratic type-name lookup does not establish
  numeric AOT completion: the new frozen binary still hits the timeout.
  T1581/T1582/T1591 numeric native acceptance remains open.

The initial native-control harness lacked its baseline log, and the initial
inline-root summary runner had a Python syntax error. Neither launched a
product run. Corrected reruns produced the results above; harness failures
are not counted as compiler failures or passing acceptance.

## Dependency boundary

The exact macOS host CLI has ten direct system imports and no Homebrew/OpenSSL
path. This is an import inspection, not a clean-system or release certification.
The generated native control is 162296 bytes, SHA-256
`cf714cdb45dfbfe14697c27b7d63b5e7dfd4097ecd01e66e9b1c7da95bb8f1cf`;
its AOT dependency inspection passes with only the allowed libSystem import.
This establishes neither static libc absence nor Linux/Windows runtime coverage.

Strict no-libc remains an AOT/runtime requirement with the macOS system-ABI
exception. Host CLI compatibility remains a separate clean-OS release gate.
