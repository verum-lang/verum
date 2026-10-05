# Platform acceptance: 2026-10-05, stage 11

Status: **not accepted**. This records the immutable ordinary CLI build and
its actual executions; later source corrections do not change those verdicts.

## Build identity

Source: `6f26795a1cb48ff2426edf32d9429f316d354252`.
Normal `cargo build --locked -p verum_cli --bin verum`, default stdlib bake,
cache and SMT, completed in 1298.09 seconds. Schema:
`v48-2026-10-05-list-storage-query`; source fingerprint:
`40fce632b245fd79fc717c29721ea2b126b7ee205babae98ffbcee61e8bdef26`.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Host CLI | 478493240 | `ea8cb960ac6f61a8106d361883a29d09e98e0c084a3671d876e662507fefc6b9` |
| runtime.vbca | 27508486 | `1e8100cdeb1126e7c02b94b834720f5ee4ab17c13b4eb62155c7bffc2d216d1e` |
| runtime.core_metadata | 67029334 | `86abfbbf8b4262e529150c44b1a7f3d534ccd6f352a1c23af5cd9c80bc36ef5a` |
| runtime.symbol_graph | 5914200 | `3341e120a5c631e44e629e6a761c9d53cb1eb0f0c0c8c70110d2650c9241a170` |

The first build attempt at `639f49e25` failed on a missing
`resource_discipline` initializer in the legacy cached-type loader. The retry
sets it to Unknown; both attempts retain separate records.

## Actual execution results

The native-control source is unchanged from stage 10, SHA-256
`918adcc5d66a70f18aed78f031c6912bd14b9f34fa1acf523e1f7e79c752efd3`.

* Interpreter: exit 0 after 92.53 seconds. Atomics, predicate ordering, field
  addresses, actual user hash body, primitive hash and mixed output ordering
  pass. Mutex lifetime fails: observed `false,false,false`, required
  `true,false,false`. Overall FAIL despite successful process termination.
* AOT: compilation produced a 162136-byte Mach-O, SHA-256
  `71431c64081383ff8b9fc321a8cad88f421a4a6d949d79a70914b9aee0d08fdd`.
  The CLI returned 255 after 1099.83 seconds with no runtime output. Direct
  replay traps in `__tls_init_REGISTRY` before user entry. Disassembly shows
  `verum_checked_malloc` returning null and the internal exit helper returning
  without exiting. Runtime helper declaration/definition ordering is T1603.
  No individual runtime oracle passed; native hash parity is still unverified.
* The combined numeric/List interpreter control fails during type checking:
  `List<Int>.new()` returns bare `List<_>` while its annotation names
  `core.collections.list.List<Int>`. No numeric or List runtime acceptance
  can be inferred from this failed run. This exposed the T1594 constructor
  metadata correction below.
* The 21 Rational controls were prepared, not executed. Three source ignores
  remain. Preparation is not a test result.

## Dependency boundaries

The exact fresh macOS CLI and a local archive containing it both pass the host
TLS dependency inspector. The CLI imports baseline macOS frameworks/libraries
and no Homebrew/OpenSSL/libgit2 dylib. Local archive SHA-256:
`819573ca725855ddf418195cc56f32443e3f4d8fe37567a8f2d2c815e28ab20b`.
This was not a published release or a clean-OS execution test, and establishes
no Linux GLIBC or Windows VC-runtime baseline.

The failed native control imports only `/usr/lib/libSystem.B.dylib`, so its
AOT dynamic boundary passes. That result does not establish static libc
provenance or successful runtime execution. The dated published-asset findings
in the [no-libc audit](no-libc-architecture.md) remain historical facts.

## Source corrections and validation after the frozen build

* Exact Byte identity and the storage-query opcode roster: `fa9202162`.
* Two ambiguous `Normal` expressions now name `Component.Normal`: `fe63933e7`.
  At `9e54ff821`, the full VBC library passes 2051 tests with one existing
  ignore; 71 selected integration tests also pass. The initial 22 failures
  were 20 local-socket sandbox failures and these two source/test defects.
* Native returned-reference projection integration `c684fb8c3`: 316 library
  and 22 selected integration tests pass. This is source/LLVM validation,
  not execution of a freshly rebuilt CLI containing that commit.
* Constructor signatures and their exact declaring parent survive inherent,
  static and qualified metadata scheme creation (`a429a7cd1`). The actual
  frozen metadata reproduces the original error and passes with the checker
  correction. Archive conversion now publishes collision-immune method keys
  and carries the parent from its exact TypeId, including a distinct function
  origin. Both preload orders, qualified sibling results and a same-named
  generic parameter are covered. The full Types run passes 3854 tests with
  three existing ignores; the final provenance restriction then passes
  28 focused neighbors and the extension/sibling boundary controls.
* Bounded value-use observations (`09898bc15`, VBC 2.20) and checked List-owned
  storage operations (`0908806ae`, VBC 2.21) are separate prerequisites.
  Neither enables packed public List allocation or closes native Drop.

Actual CLI acceptance of the integrated corrections requires another normal
build and fresh executions. Mutex/aggregate cleanup, packed element reference
carriers, and full platform/release conformance remain open.
