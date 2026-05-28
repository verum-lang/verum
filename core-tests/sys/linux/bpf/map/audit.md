# `core.sys.linux.bpf.map` — implementation audit

## Status: **partial** (under `--interp`; ADT + record shape, CRUD deferred)

* Provides `MapType` (30 variants matching `enum bpf_map_type` in
  `<linux/bpf.h>` UAPI), `MapConfig` / `MapDescriptor` records, and
  the kernel-facing CRUD primitives `create_map`, `map_lookup`,
  `map_update`, `map_delete`, `map_iterate`.
* The CRUD operations route through `@intrinsic("verum.bpf.*")` to the
  `bpf(BPF_*)` syscall and require a Linux host with CAP_BPF or
  CAP_SYS_ADMIN.  They cannot be exercised on macOS / Windows.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.sys.linux.bpf.program` | Programs receive map FDs via the `BPF_PROG_LOAD` instruction-stream rewriting; `MapDescriptor.fd` is the value the userspace loader passes. |
| user code | `Hash` / `LpmTrie` / `RingBuf` / `Sockmap` / `Devmap` / `Xskmap` are the map types exercised by XDP fast-path loaders. |

## 2. Action items landed in this branch

1. `unit_test.vr` — 20 `@test`s pinning a representative slice of the
   30-variant `MapType` ADT (one constructor per documented arm
   sampled from each functional group: hash, array, perf, percpu,
   queue/stack, sockmap/devmap/xskmap, ringbuf, storage, of-maps,
   bloom-filter), `MapConfig` field round-trip including the
   `BPF_F_MMAPABLE` flag use case, and `MapDescriptor` field
   round-trip with `map_type` preservation across record
   construction.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `create_map` / `map_lookup` / `map_update` / `map_delete` / `map_iterate` round-trip | Requires Linux host with bpf() syscall + CAP_BPF. Deferred to integration suite on a Linux runner. |
| 2 | `BPF_F_*` flag constants enumeration | The `flags: UInt32` field is currently free-form. A dedicated `BpfMapFlags` namespace with `NO_PREALLOC` / `RDONLY_PROG` / `WRONLY_PROG` / `INNER_MAP` / `MMAPABLE` constants would let callers compose without bare magic numbers. |
| 3 | property_test.vr — pairwise distinctness over the 30 variants | Deferred until the broader stdlib base/protocols/Eq property suite ships a generic property runner that can be parameterised over an ADT's variants. |
| 4 | regression_test.vr | No defects yet pinned. |
