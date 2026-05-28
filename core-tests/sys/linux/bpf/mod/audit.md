# `core.sys.linux.bpf` — implementation audit (umbrella module)

## Status: **partial** (under `--interp`; re-export surface complete, syscall path deferred)

* Acts as the entry-point for the four submodules `error`, `program`,
  `map`, and (via mount-import) the `BpfError` typed-error funnel.
* Re-exports every public name through the umbrella so callers can
  `mount core.sys.linux.bpf.{ProgramType, MapType, BpfError}` without
  knowing about the submodule structure.

## 1. Cross-stdlib usage

The umbrella module is the documented entry-point for every BPF caller
inside / outside stdlib.  Direct imports against `core.sys.linux.bpf.error`,
`core.sys.linux.bpf.program`, `core.sys.linux.bpf.map` are supported as
escape hatches but not the canonical surface.

## 2. Action items landed in this branch

1. `unit_test.vr` — 9 `@test`s pinning that every documented re-export
   path resolves at compile time and constructs at runtime via the
   umbrella name (BpfError / ProgramType / AttachType / ProgramDescriptor /
   ProgramBytecode / Link / MapType / MapDescriptor / MapConfig).  A
   regression that drops any re-export from `core/sys/linux/bpf/mod.vr`
   would cause the corresponding `@test` to fail at compile time.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Cross-submodule integration scenarios | The integration_test.vr surface composing programs + maps + Result-funnel will land once a Linux test runner is available. |
| 2 | Function re-exports (`load_program`, `attach_*`, `create_map`, `map_*`) | Cannot exercise on a non-Linux host; deferred to the integration suite. |
