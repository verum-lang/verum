# `core.sys.linux.bpf.program` — implementation audit

## Status: **partial** (full unit + property + integration + regression under `--interp`; AOT pending D1; syscall path deferred to Linux runner)

* Provides:
  * `ProgramType` — 32 variants matching `enum bpf_prog_type` in
    `<linux/bpf.h>` UAPI (`Unspec` through `Syscall`).
  * `AttachType` — 39 variants matching `enum bpf_attach_type`.
  * `ProgramBytecode` record — typed wrapper for the
    `struct bpf_insn` 8-byte little-endian instruction stream that
    `BPF_PROG_LOAD` consumes, plus license / kernel_version / name.
  * `ProgramDescriptor` — file descriptor + program type + name.
  * `Link` — attached-program handle, dropping it detaches.
  * `load_program` / `attach_xdp` / `attach_tc` / `attach_cgroup` /
    `attach_tracepoint` / `attach_kprobe` — kernel-facing functions
    routed via `@intrinsic("verum.bpf.*")` to the bpf() syscall.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| user code | XDP fast-path loaders, tracing/kprobe instrumentation, cgroup-attached filters. |
| `core.sys.linux.bpf.map` | Programs reference maps by FD inside the instruction stream; the userspace loader patches map FDs into `BPF_LD_MAP_FD` instructions before passing the bytecode to the kernel. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 22 `@test`s pinning a representative slice of
   ProgramType (13 of 32 variants — `Unspec`, `SocketFilter`, `Kprobe`,
   `Xdp`, `Tracepoint`, `SchedCls`, `SchedAct`, `CgroupSkb`,
   `RawTracepoint`, `StructOps`, `Lsm`, `SkLookup`, `Syscall`) +
   AttachType (9 of 39 variants — `CgroupInetIngress`,
   `CgroupInetEgress`, `TraceFentry`, `TraceFexit`, `XdpDevmap`,
   `XdpCpumap`, `PerfEvent`, `ModifyReturn`, `LsmMac`) + record-shape
   round-trip for `ProgramBytecode` (including `dual BSD/GPL` license
   acceptance), `ProgramDescriptor`, and `Link` (including the
   `link.fd != link.program_fd` invariant).

2. `property_test.vr` — **exhaustive** discriminant-injectivity laws over
   ALL 32 `ProgramType` variants and ALL 40 `AttachType` variants (each
   total `match → Int` is injective and dense over `[0, N)`, matching
   `enum bpf_prog_type` / `enum bpf_attach_type` ordinal order — closes
   deferred #4 without a generic runner), plus `ProgramBytecode` field
   round-trip across every program type and `Link` fd/attach preservation
   across every attach type. (Note: the prior audit said AttachType had
   39 variants; the source declares 40 — `CgroupInetIngress` … `TraceKprobeMulti`.)
3. `integration_test.vr` — bytecode assembly via `List<Byte>`,
   load→`Result<ProgramDescriptor, BpfError>` shape (pure stub) with
   empty-program/verifier and non-GPL/EPERM rejection paths, an
   attach-point compatibility predicate, and `Link` bookkeeping through
   `List` / `Maybe`.
4. `regression_test.vr` — pins **D6** (`FileDesc.as_raw()` on both
   descriptor and link fds), no-field-drift through the
   `ProgramDescriptor { fd, program_type, name: Text }` 3-field record,
   high-ordinal dispatch (`TraceKprobeMulti` ordinal 39, `Syscall`
   ordinal 31), and `ProgramBytecode` `List<Byte>` + `Text` field
   independence.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `load_program` round-trip via bpf() syscall + verifier path | Requires Linux host with bpf() + CAP_BPF.  Deferred to integration suite on a Linux runner. |
| 2 | `attach_xdp` / `attach_tc` / `attach_cgroup` / `attach_tracepoint` / `attach_kprobe` round-trip | Same gating. |
| 3 | Compatibility matrix — which AttachType is valid for each ProgramType | The defect class `BpfError.InvalidAttachType` exists, but the type system does not currently encode the valid (ProgramType, AttachType) pairs.  A future refinement type or compile-time table would make invalid pairings unrepresentable.  Tracked here for the language-side type-system work. The `integration_test.vr` `attach_compatible` predicate is a runtime stand-in. |
