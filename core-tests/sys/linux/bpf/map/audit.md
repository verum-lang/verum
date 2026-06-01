# `core.sys.linux.bpf.map` — implementation audit

## Status: **partial** (full unit + property + integration + regression under `--interp`; AOT pending D1; CRUD deferred to Linux runner)

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

1. `unit_test.vr` — 22 `@test`s pinning a representative slice of the
   30-variant `MapType` ADT, `MapConfig` field round-trip including the
   `BPF_F_MMAPABLE` flag use case, and `MapDescriptor` field round-trip
   with `map_type` preservation across record construction.
2. `property_test.vr` — exhaustive discriminant-injectivity law over all
   30 `MapType` variants (a total `match → Int` mapping is dense over
   `[0, 30)` and matches `enum bpf_map_type` ordinal order — closes
   deferred #3 without needing a generic property runner), `MapConfig`
   field round-trip over UInt32 boundary widths, and `map_type`
   preservation through `MapDescriptor` for every variant.
3. `integration_test.vr` — config→`Result<MapDescriptor, BpfError>` round
   trip (pure `fake_create_map`), the `value_out.len() == value_size`
   geometry contract `map_lookup` documents, `List<MapType>` per-class
   aggregation, and descriptor flow through `Maybe`.
4. `regression_test.vr` — pins **D6** (`FileDesc.as_raw()`, not the
   nonexistent `raw()` the original unit tests called — see below),
   `map_type` no-field-drift through record construction, `MapConfig`
   field independence, and high-ordinal (`BloomFilter`, ordinal 29)
   variant dispatch.

### D6 — FileDesc field-value readback collides with NEWTYPE-UNBOX-1

FileDesc is a `(Int)` newtype: reading a record field of type FileDesc
(`d.fd`) UNBOXES it to a bare `Int`, losing the static type at the call
site. Method dispatch on the unboxed value is then broken on BOTH tiers,
just with different symptoms — the **NEWTYPE-UNBOX-1** class (defect
catalogue §12):

* `d.fd.raw()` (the pre-existing unit-test idiom): the interp int-receiver
  intercept (`"raw" => Value::from_i64(v)` at
  `method_dispatch.rs:3673`) FABRICATES the value (works by accident),
  while AOT correctly rejects it (`no method named 'raw' for FileDesc` —
  `raw` exists only on `windows.ntdll.Handle`, not FileDesc).
* `d.fd.as_raw()` (the real accessor): AOT resolves it, but interp PANICS
  with `as_raw not found on receiver of runtime kind Int` over **6
  ambiguous `.as_raw` candidates** (FileDesc / Fd / ValidFd / DarwinInstant
  / SharedFd) — the unboxed Int carries no type to disambiguate.

**Resolution (this branch):** assert FileDesc identity through the
type-aware `==` OPERATOR — `d.fd == FileDesc(N)` / `a.fd != b.fd`.
`compile_binary` keeps both operands typed as FileDesc and lowers to a
primitive Int comparison, sidestepping the unboxed method-dispatch
entirely (the same idiom `core.sys.common` already uses for `fd >= 0` /
`fd == STDIN`). Confirmed working under `--interp`. The fundamental fix
for the underlying NEWTYPE-UNBOX-1 (preserve a newtype's type through
field-access for method dispatch) is deep VBC codegen work tracked at
catalogue §12.

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `create_map` / `map_lookup` / `map_update` / `map_delete` / `map_iterate` round-trip | Requires Linux host with bpf() syscall + CAP_BPF. Deferred to integration suite on a Linux runner. |
| 2 | `BPF_F_*` flag constants enumeration | The `flags: UInt32` field is currently free-form. A dedicated `BpfMapFlags` namespace with `NO_PREALLOC` / `RDONLY_PROG` / `WRONLY_PROG` / `INNER_MAP` / `MMAPABLE` constants would let callers compose without bare magic numbers. |
