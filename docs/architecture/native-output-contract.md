# Native output writer contract

Status: focused source/LLVM/JIT and object checks, 2026-10-05. This describes
the shared generated writer; it is not a complete AOT dependency certificate
or evidence of execution on Windows. Generated AOT follows the
[no-libc contract](no-libc-architecture.md). Host tooling may use baseline OS
libraries under that separate packaging contract.

## One platform boundary

`RuntimeLowering::get_or_declare_write` emits the low-level
`verum_internal_write(fd, bytes, count) -> Int`. `verum_os_write` delegates to
it instead of maintaining another platform implementation. Linux uses the
existing target-specific syscall emitter; Darwin uses its supported system
`write` ABI. Windows uses `GetStdHandle` and synchronous `WriteFile`.

The Windows branch supports the declared standard descriptors 0, 1 and 2.
Other values return -1: there is no owned runtime fd-to-HANDLE table yet, and
an arbitrary integer must not be reinterpreted as a HANDLE. A missing/invalid
standard handle also returns -1 rather than reporting fictional bytes written.
Counts outside the nonnegative signed return range fail. A low-level call
clamps its request to DWORD maximum and returns the actual count on success
or -1 on failure. These signatures and synchronous result semantics follow
[GetStdHandle](https://learn.microsoft.com/en-us/windows/console/getstdhandle)
and [WriteFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-writefile).

The generated Windows imports carry the existing platform-external provenance
attribute as well as DLL import storage. This keeps the final bodyless-function
pass from replacing a real OS import with a default-return stub. Tests verify
that both imports remain declarations after full source lowering.

## Progress and presentation

`verum_internal_write_all` calls the common low-level writer until every byte
is written. It advances only by positive progress no greater than the remaining
count. A negative count, error, zero progress or invalid reported extent returns
-1. Every successful iteration reduces the remaining count; no-progress output
cannot spin forever. A zero-length buffer succeeds without a write call.

`verum_internal_puts` uses this loop for the string and its newline. It returns
0 only after both complete, otherwise -1, and it does not append a newline after
a failed body write. Numeric print uses its bounded formatter buffer and the
same writer as Text and Bool. The internal NUL scan uses volatile byte loads
so LLVM cannot synthesize a libc `strlen` during optimization.

The language's existing Unit-returning `print` lowering still ignores the final
puts status. This change prevents ordinary short writes from truncating output;
it does not establish a language policy for reporting terminal I/O failure.
General Windows file handles, asynchronous writes, interrupted Unix write retry,
and 32-bit targets require their own contracts and acceptance.

## Evidence

Two focused regressions failed before the writer correction and pass afterward:
full source lowering retains the shared Windows writer/import authority, and
optimized strlen retains its own byte scan. The JIT executes the generated
writer/puts logic with only the two Windows OS entry points mocked. Cases cover
stdout/stderr/stdin selection, missing and invalid handles, unsupported fd,
empty output, partial progress, zero progress, failure before/after progress,
newline failure, DWORD chunk boundaries and invalid counts. This is host
execution of target-neutral logic, not a Windows OS run.

The source mixed Text/Bool/Int/Float redirected-output gate and integer boundary
gates remain green. Six optimized source objects preserve their actual source
entry points. `llvm-nm --undefined-only` reports no imports for x86_64/AArch64
Linux, only `write` for Darwin, and `GetStdHandle`/`WriteFile` for Windows.
The x86_64 Windows Float object additionally retains `_fltused`, separately
tracked for an owned ABI provider. There are no printf-family or strlen imports.
The isolated integer-only Float kernel remains independently free of imports.

Repository gates: `crates/verum_codegen/tests/common_print_writer.rs`,
`float_print_writer.rs`, `integer_print_writer.rs` and
`float_format_contract.rs`. They run in the integration job’s codegen test step;
final coherent CLI/native and target-system acceptance remain separate.
