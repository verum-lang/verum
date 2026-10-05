# No-libc AOT and host-tool dependency rules

**Required contracts; implementation audit updated 2026-10-04 (T1585).**
The strict no-libc requirement applies to **programs compiled AOT by Verum**,
including their standard-library/runtime code and emitted libraries or object
files. It excludes dynamically and statically linked libc implementations,
subject to the documented macOS system-ABI boundary below.

The **Verum CLI and its hosted interpreter** have a separate distribution
requirement: they must run on a clean installation of each supported OS
baseline without the user installing extra runtime libraries. They may use
that OS's standard libc and system libraries. Host dependencies do not become
permitted dependencies of generated AOT programs. Conversely, a host use of
Rust `std` or libc is not itself an AOT no-libc violation.

Neither requirement is fully verified today. The audit below separates AOT
implementation gaps from host packaging and OS compatibility gaps.

## The AOT rule

* Linux and FreeBSD runtime services use the supported kernel ABI directly.
  Depending on glibc, musl, their wrappers, or a statically linked copy of
  either violates the AOT contract.
* macOS uses the supported system ABI through `libSystem`. This is the
  documented platform exception to the AOT rule. It does not allow Homebrew
  dependencies in generated programs or claim libSystem contains no C APIs.
* Windows uses the declared OS boundary (`kernel32.dll` and `ntdll.dll`)
  without MSVC CRT or UCRT. Additional OS imports require an explicit
  capability/platform decision; they are not silently covered by this pair.
* Embedded targets use their specified bare-metal facilities.
* Explicit application FFI dependencies must be declared and audited as such.
  An interoperability example linking a foreign library is not evidence that
  the Verum runtime requires that library, or a no-libc conformance sample.

Tools used to build Verum have their own development prerequisites. End users
of the published CLI must not need a compiler toolchain, Homebrew, OpenSSL
packages or a separately installed Visual C++ runtime merely to launch it.
Static inclusion or removal of host dependencies is a packaging option; the
same technique does **not** make statically linked libc acceptable in AOT
output. Supported OS versions must be explicit and tested, not inferred from
the library names present on a developer's machine.

## Per-platform AOT replacement strategy

| Target | Required AOT runtime boundary |
|---|---|
| Linux | Direct target-specific syscalls (`syscall` on x86_64, `svc #0` on AArch64); no libc/CRT. |
| macOS | Supported system APIs through `libSystem`; explicitly selected platform capabilities are reviewed separately. |
| Windows | `kernel32.dll` and `ntdll.dll`; no MSVC CRT or UCRT. |
| FreeBSD | Direct syscalls using that target's syscall convention. |
| Embedded | Bare-metal implementation for the selected board/target. |

Platform selection in emitted LLVM IR reads the **target** triple. It must
not use host `#[cfg(target_os = ...)]` to choose another target's ABI.
On macOS, the supported system-library boundary is intentional; do not turn
that into a claim that direct syscalls are legally prohibited.

## What the AOT rule excludes

No implicit AOT runtime use of libc allocation, I/O, networking, threading,
formatting, parsing, string or memory functions outside the documented target
boundary. Reaching such a dependency through a helper wrapper or statically
linked archive does not remove it. The host interpreter's Rust `std`, libffi
and libc paths are governed by the separate host distribution requirement.
A crate named `libc` may also provide constants and ABI types: a text search
alone cannot decide which uses execute a call.

LLVM memory intrinsics are **not sufficient proof**. Ordinary `llvm.memcpy`
can lower to an external call. LLVM provides a separate
[`llvm.memcpy.inline` guarantee](https://llvm.org/docs/LangRef.html#llvm-memcpy-inline-intrinsic),
with its own constraints. Inspect final objects and binaries, and supply
Verum-owned implementations where the backend needs a helper. Renaming a
wrapper `verum_internal_*` does not establish the origin of its callees.

## Verification

Keep two independent acceptance records: **AOT dependency conformance** and
**host CLI clean-OS compatibility**. Record source revision, target, build
flags, asset and executable SHA-256, and dependencies/symbols. Static
inspection does not execute a foreign-target executable:

```sh
# Linux: inspect the host CLI and generated AOT program separately.
llvm-readelf --program-headers --dynamic --version-info ./verum
llvm-readelf --program-headers --dynamic --version-info ./program
llvm-nm --undefined-only ./program.o

# macOS: system and external dynamic dependencies.
otool -L ./verum
otool -L ./program
nm -u ./program.o

# Windows: PE imports, including api-ms-win-crt-* forwarding DLLs.
llvm-readobj --coff-imports ./verum.exe
llvm-readobj --coff-imports ./program.exe
```

In **Linux AOT output**, `libc.so.6`, a musl dependency, or versioned `GLIBC_*`
imports refute no-libc conformance. A static binary or an empty dynamic
import list does **not** prove absence of statically linked libc: inspect
linked members/symbol provenance and execute representative runtime paths.
Likewise, macOS `libSystem` linkage alone says nothing about a Linux build.

For the **host CLI**, resolve the complete dependency closure against a clean
supported OS image with no developer packages or third-party runtimes. Test
the oldest declared OS baseline, startup, interpretation, compilation and
package/network paths. A version smoke on a dependency-rich CI builder is
insufficient. System libc is allowed; missing packages and unavailable symbol
versions are release compatibility defects.

Updated 2026-10-05 (T1589): `scripts/ci/check_no_libc_link.sh` builds a smoke
in a unique temporary workspace, or inspects an existing executable with
`--artifact PATH`. The inspector selects ELF, Mach-O or PE from the artifact,
not the host. Unknown imports fail; missing tools, tool errors/diagnostics,
malformed output and unsupported formats produce a non-success result. Linux
controls permit no dynamic runtime or userspace loader; Darwin controls allow
exactly libSystem; Windows controls allow kernel32/ntdll, including inspection
of delayed imports. Explicit application FFI/capability libraries need a
separate audit and are not silently exempted by this default-runtime gate.

Fourteen source-only regression tests cover both poles and smoke isolation.
The real stage-10 Darwin AOT control passes this dynamic boundary; all six
host CLI artifacts from the audit below are rejected by the AOT policy, as
expected. They were inspected without execution. A fresh isolated Darwin
smoke also builds and passes inspection. These are not new Linux/Windows
execution results or proof about statically linked libc.

The release matrix in `.github/workflows/build-verum.yml` builds GNU Linux
and MSVC Windows CLI targets and runs `--version`. The host transport packaging
change also inspects the exact packaged executable for external Git/OpenSSL
dependencies before upload; see [its measured scope](host-transport-packaging.md).
That narrow gate does not certify Linux symbol-version baselines, Windows
redistributable availability, or clean-OS execution. Host packaging and AOT
conformance remain separate. Full AOT runtime and static provenance coverage
remains open.

## Source implementation status (2026-10-05)

The original 2026-10-04 audit used integration revision `ca7d1334b`. The
following rows include the subsequent numeric-output and host-packaging
source corrections. Focused LLVM/JIT and object checks establish their
stated scope; coherent CLI execution and clean-system release acceptance
remain separate requirements.

| Boundary | Evidence and remaining work |
|---|---|
| Native integer print | Uses the owned integer formatter and [common target-aware writer](native-output-contract.md). Source/LLVM/JIT boundary and ordering controls pass. |
| Native Float print | Both ordinary print arms now use the owned formatter and common writer. Focused mixed-output and six-target object checks pass without printf-family imports. Fresh CLI and target-system acceptance remain separate; see the [formatting evidence](native-float-formatting-contract.md). |
| Native Float-to-Text | The shared integer-based Ryū kernel now handles default f64 Display/Debug, including tiny/large finite values and signed zero, under focused numerical and object checks. Explicit precision, source-only conversion and remaining presentation modes retain their own acceptance requirements. |
| Host interpreter networking | `interpreter/dispatch_table/handlers/net_runtime.rs` uses Rust `std::net` and direct `libc` socket calls on Unix. This is allowed by the host policy when provided by the supported OS; it does not authorize libc calls in emitted AOT code. |
| Standard-library terminal paths | `core/term/raw/termios.vr` and `core/term/event/source.vr` declare `@ffi("libc")`, including a Linux `poll` path. Audit selected AOT objects and replace forbidden target dependencies. macOS terminal APIs through the permitted libSystem boundary are a separate case; source declarations alone do not establish every emitted call. |
| Host foreign-library plumbing | `ffi/platform/linux.rs` uses `libc::dlopen`, `dlsym`, `dlclose`, `mmap` and `munmap`. OS-provided host loader/allocation services are allowed; user-selected FFI libraries remain explicit application dependencies. |
| Emitted low-level helpers | Direct-syscall and internal allocation/I/O helpers exist. Check each selected target and final emitted object; old source-level replacement records alone do not certify all reachable paths. |
| Float parsing / DNS / exceptions | Prior Linux `strtod`, native `getaddrinfo`, and setjmp migration work landed. Parsing parity and native target execution are separate checks; these should not be listed as entirely absent implementations. |
| Linker fallback | The Linux no-libc linker configuration exists, but the cc-driver `VERUM_NOSTDLIB_CC_DRIVER` path is opt-in. Audit the selected AOT link route and compiler-generated library calls. |
| Shipped Rust/LLVM CLI | GNU/MSVC host triples are allowed. Actual imports, versions and clean-OS runs must establish availability without additional installations; AOT dependency evidence cannot establish this. |

### Published dev artifact audit

All six rolling `dev` assets updated on 2026-10-04 at 13:53 UTC were
downloaded, verified against their GitHub SHA-256 digests, and inspected
without executing the binaries. The observed tag pointed to `c75c7e888`.
The [machine-readable evidence](no-libc-dev-artifacts-2026-10-04.json) records
each asset URL, update time, archive/executable SHA-256 and complete direct
import list. A rolling tag can move; the hashes identify this observation.

| Published host CLI | Actual imports and compatibility implications |
|---|---|
| Linux x86_64 and AArch64 | `libc.so.6`, `libm.so.6`, `libgcc_s.so.1`, `libstdc++.so.6`, `libssl.so.3`, `libcrypto.so.3`, and the architecture's GNU loader; versioned imports include `GLIBC_2.39`. libc itself is allowed, but the old glibc 2.31 minimum is false. Availability of every other library must be checked on clean supported distributions. |
| macOS x86_64 and AArch64 | Homebrew `libssl.3.dylib` / `libcrypto.3.dylib` under `/usr/local/opt/openssl@3` and `/opt/homebrew/opt/openssl@3` respectively is a confirmed external dependency to remove from the distributed CLI. System frameworks, `libc++`, `libiconv`, `libz` and libSystem are allowed if available at the supported OS baseline. |
| Windows x86_64 and AArch64 | `api-ms-win-crt-*` imports, `MSVCP140.dll` and `VCRUNTIME140.dll`; x86_64 also imports `VCRUNTIME140_1.dll`. UCRT is an OS component on Windows 10 and later; VC runtime redistributables are a separate dependency whose availability cannot be assumed on a clean OS. |

Microsoft documents the distinction between
[OS-provided UCRT](https://learn.microsoft.com/en-us/cpp/windows/universal-crt-deployment?view=msvc-170)
and [runtime libraries to redistribute](https://learn.microsoft.com/en-us/cpp/windows/determining-which-dlls-to-redistribute?view=msvc-170).
No Windows clean-OS execution was performed in this audit.

The old universal "glibc >= 2.31", "self-contained CLI", and "macOS CLI links
only libSystem" statements do not describe these files. `GLIBC_2.39` is an
observed symbol-version requirement, not a newly approved minimum OS or a
complete compatibility test. Host packaging needs repair and clean-baseline
verification. It is independent of proving strict no-libc in AOT output.

## Compiler-rt audit (#28, 2026-07-15, darwin arm64)

The following is preserved historical evidence for the named Darwin probes.
It does not certify today's CLI, another target, or every generated program.


Method: built representative AOT binaries with `verum build --keep-temps`
(a basic-arithmetic probe, an `Int128` div/rem + `Float`↔`Int128`
probe, and a `Float` div/convert probe) and ran `nm` on the emitted
`.o` objects and linked binaries.  The bigint conformance suite is
*not* a compiler-rt exercise: Verum's `BigInt` is
`{ sign: Bool, digits: List<Int> }` (base-10⁹ chunks over `Int` =
i64) — it never touches a native 128-bit integer.

Findings (facts):

* **Zero compiler-rt builtin symbols** (`__divti3`, `__udivti3`,
  `__modti3`, `__multi3`, `__floattidf`, `__fixdfti`, `__ashlti3`,
  …) appear in *any* object or binary — including the `Int128`
  probe.  Confirmed by `nm` over defined **and** undefined symbols.
* `Int128` arithmetic collapses to **i64** under the uniform-i64
  register model: the probe's division lowers to native
  `sdiv x8, x8, x9` (64-bit registers), never a `bl ___divti3`.
  Float↔int conversions are all f64↔i64 (`fcvtzs`/`scvtf`, inline).
  (The `Int128` probe therefore also mis-computes / crashes on
  large values — a *separate* correctness gap, not a link concern.)
* Every remaining undefined symbol in the objects
  (`memcpy`/`memmove`/`memset`/`bzero`/`strlen`,
  `sin`/`cos`/`exp`/`log`/`pow`, `pthread_*`, `clock_gettime`,
  `nanosleep`, `mmap`, `write`, `kqueue`, `__error`, …) is provided
  by **libSystem** on macOS (acceptable per the architecture rule).
* Empirical link test: `cc probe.o <darwin-flags> -nostdlib -lSystem`
  produces a working binary that `otool -L` shows depends on
  **libSystem.B.dylib only** — identical to the default link.  Adding
  the compiler-rt builtins archive (`libclang_rt.osx.a`) on top is
  **inert**: no new dylib dependency, no pulled members.

Conclusion: blocker (a) is **empirically closed on darwin**.  A
`-nostdlib -lSystem` cc-driver link is correct today; the compiler-rt
builtins archive is wired as an opt-in, target-aware fallback
(`NoLibcConfig::compiler_rt_builtins_archive`) so the link stays
correct the day codegen *does* emit an i128 libcall — without
hand-authoring soft-int IR that nothing currently calls.  The
in-tree `llvm/install` ships no compiler-rt; the locator falls back
to the host `clang` resource dir and the Apple CommandLineTools /
Xcode toolchains (`libclang_rt.osx.a`).

Strategy rejected: **IR soft-int helpers** (`platform_ir.rs`).  With
zero builtin symbols referenced today, emitting `__divti3` &c. in IR
would be speculative dead code, contradicting the "only really-used
symbols" rule.  Revisit only if/when the register model gains a real
native i128 and a survey shows which specific builtins the backend
then lowers to a libcall on each target.

## Why this matters

For generated AOT programs, removing libc removes that dependency's ABI and
deployment constraints. Kernel versions, architecture, CPU features and
explicitly selected OS capabilities still bound compatibility. The runtime
remains responsible for allocation, I/O, parsing and formatting correctness.
Performance claims need measurements of the actual target and path.

For the host CLI, the goal is installation without dependency setup on every
supported OS baseline. A system-library dependency is acceptable only when
that baseline actually supplies the required ABI. Neither goal promises
compatibility with every OS version or every future system.

## Owner / mechanism

Codegen/runtime maintainers own AOT dependency and behavior acceptance;
release maintainers own host packaging and clean-OS compatibility. Every
external runtime symbol must identify its provider and target. Record these
acceptances separately, keep implementation gaps visible, and do not convert
a build-machine dependency into an installation requirement to hide it.
