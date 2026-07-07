# `net/mod` audit

Module: `core/net/mod.vr` (~193 LOC) — the `core.net` umbrella
manifest. Declares no types of its own; its public surface is
(a) 26 submodule declarations, (b) ~48 item re-exports across
addr / tcp / udp / unix / http / tls / dns, (c) the nested
`core.net.prelude` submodule re-exporting the 8 most common names.

The conformance suite pins the RE-EXPORT CONTRACT: every name
reaches user code through the umbrella (`mount core.net.{Name}`)
and through the prelude glob (`mount core.net.prelude.*`), and the
IMPL SURFACE survives the chain (constructors + methods callable on
values built through the umbrella path).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.xds` | `mount core.net.{SocketAddr, TcpStream, ...}` umbrella imports. |
| `core.search` / `core.storage` | HTTP surface through `core.net.http` (direct submodule path). |
| `core.redis` | TCP + addr through direct submodule paths. |
| Application code | the primary entry point for all networking. |

Both import disciplines (umbrella vs direct submodule path) are live
in-tree; the umbrella suite here is what keeps the two equivalent.

## 2. Crate-side hardcodes

* `crates/verum_compiler/src/stdlib_reachability.rs` — the #109
  lazy-monomorphisation BFS walks mount trees; umbrella re-export
  chains (`core.net` → `.addr.Ipv4Addr`) must stay resolvable or
  reachability silently prunes the target module.
* `crates/verum_compiler/src/precompile.rs::scan_module_reexports` —
  captures `module_reexports["core.net"]` and
  `module_reexports["core.net.prelude"]` into the embedded
  CoreMetadata; the consumer replay
  (`verum_types::infer::modules::import_all_from_module`) resolves
  each leaf. Canonicalisation of leaf source-module paths (landed
  this session) is what keeps Path-arm leaves (`.addr.IpAddr`)
  resolvable against the archive's declaring-module indexing.

## 3. Language-implementation gaps

### §3.1 Umbrella-reexport-impl class (Bug C family) — pinned

`mount core.X.{Type}` through an umbrella whose AST reaches the
importer as a stub used to DROP impl blocks (`core.sys.{MemProt}` →
`MemProt.NONE` : Unit; fixed 91d1c85b0 by recursing
`import_item_from_module_body` through reexport sources). Every
unit test here constructs through the umbrella AND calls impl
methods, so a regression re-manifests as a suite-wide failure, not
a silent type degradation.

### §3.2 Prelude-glob capture (task #27 lineage) — pinned

`mount core.net.prelude.*` resolves through metadata
`module_reexports` leaves. Pre-#27 the glob branch of the
precompile scanner was a TODO and every transitively re-exported
name surfaced as `unbound variable`. `regression_test.vr` keeps the
double-hop chain (prelude → umbrella → addr) compiling.

## 4. Action items landed in this branch

* `core-tests/net/mod/unit_test.vr` — 22 unit tests: addr
  construction + RFC predicates through the umbrella (7), http
  Method/StatusCode/Version/Headers surface (7), TlsVersion
  is_secure lattice + wire_version tuple (2), dns record-type
  disjointness + is_ip_address / is_valid_domain free fns (5),
  unix ShutdownKind + PeerCred (2) — every one calling impl
  methods on umbrella-imported names.
* `core-tests/net/mod/regression_test.vr` — 3 pins: prelude-glob
  impl-surface survival, double-hop re-export resolution,
  transport type names binding in type position.
* `core-tests/net/mod/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| `ToSocketAddrs` protocol re-export exercise (needs DNS fixture) | this folder | gated on mock-resolver harness (dns audit §3.1) |
| Property file (re-export identity: umbrella name ≡ direct-path name for Eq-capable values) | this folder | 1-2h; needs `Type`-identity comparison discipline |
| Umbrella `HttpClient` / `TlsConnector` construction smoke | this folder | gated on socket/TLS harness |
