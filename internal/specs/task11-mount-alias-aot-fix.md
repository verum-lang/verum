# Task #11 — Fundamental fix for `mount X.{NAME as ALIAS}` AOT codegen

> **Status**: PLAN — implementation deferred to a dedicated multi-day branch.
> Workarounds for ~50+ specific call sites have landed in the 73-cycle
> whack-a-mole sweep (commits between `259f39b06` and `5ce5c77d0` in the
> 2026-05-23/24 session); they unblock specific files but every newly-added
> stdlib `mount X.{N as A}` will keep hitting the same class until this
> fix lands.

## Defect summary

Stdlib (and user) files routinely use mount-rename aliases to disambiguate
common-name imports from local symbols or from sibling stdlib modules:

```verum
mount core.database.postgres.connection.{
    PgConnection, PgConnState, PcsReady,
    connect as sync_connect,
};

// ...later in the body of a fn in this file:
sync_connect(&cfg_owned)
```

Under `verum test --interp` this works.  Under `verum test --aot` the
compile fails with `undefined function: sync_connect (in function
connect_listener_async)`.

The defect surfaces every place where a stdlib file's `mount X.{NAME as
ALIAS}` is followed by `ALIAS(args)` inside a function body that lands in
the AOT-eager-compile transitive closure of the user test.  This class
covers function-rename, variant-rename, const-rename, in-body mount-rename,
and module-level-alias forms.

## Symptom origin (traced in this session)

The error is raised at `crates/verum_vbc/src/codegen/mod.rs:12546`:

```rust
let result = self.compile_block(block)
    .map_err(|e| e.with_context(format!("in function {}", lookup_name)))?;
```

This is the body-compile step inside `compile_function` (line 12126).  The
function name `connect_listener_async` is from
`core/database/postgres/async_listener.vr`, NOT the user test file.  The
error wrap "VBC codegen error (user bodies)" at
`crates/verum_compiler/src/pipeline/vbc_codegen.rs:444` runs through
`compile_module_items` (line 3478), which iterates `module.items`.

**Open trace question**: how does `connect_listener_async`'s AST land in the
user test module's `items`?  Two candidate paths to verify:

1. Precompile-side lenient-skip emits a stub body marker into the archive;
   user AOT path re-parses the source to retry the body compile.
2. The eager-compile path in `apply_lazy_with_types` (archive_ctx_loader.rs:1543)
   triggers AST-mode re-compile for archive entries with missing/stub bodies.

The next session should verify by running with `VERUM_TRACE_CODEGEN_PATH=1
VERUM_TRACE_DECL=1 RUST_LOG=trace`.

## Architectural diagnosis (high confidence)

The `process_import_tree` machinery at `mod.rs:7323` DOES correctly handle
aliases when called — it calls `register_function_authoritative(alias_name,
func_info)` on successful lookup.  Verified by direct inspection: see lines
7390-7438.

The gap is **WHEN this is called**.  It only fires for items in the user
module's `module.items` (`mod.rs:6588` inside `collect_all_declarations` and
`mod.rs:3803` inside the lenient variant).  Stdlib files' own mount
declarations are processed during PRECOMPILE, not at user-test compile
time.  When the user-side AOT re-compiles a stdlib body, the alias scope of
the containing stdlib file is NOT re-established.

## Three candidate fixes (ranked by surgical-ness)

### Fix A — re-process source module's mounts before AOT-recompiling its body

**Scope**: `crates/verum_vbc/src/codegen/mod.rs::compile_function`
(line ~12126), `crates/verum_compiler/src/archive_ctx_loader.rs`.

When `compile_function` is invoked on a function whose `current_source_module`
differs from the user module being compiled, run that source module's `mount`
declarations through `process_import_tree` BEFORE entering the body.

**Pros**: minimal-invariant change; preserves archive format.

**Cons**: requires the source AST of each stdlib module to be loadable on
demand.  May add cold-start cost (re-parse stdlib files for body recompile).

### Fix B — store alias mappings in the precompiled archive

**Scope**: archive format change + apply_lazy_with_types.

Each stdlib module's `mount X.{N as A}` mappings get serialised into the
`VbcModule` archive entry alongside types/functions.  When
`apply_lazy_with_types` loads a module's function info, also load its alias
table and register entries in `ctx.functions` via
`register_function_authoritative(alias, info)`.

**Pros**: zero per-body cost; alias resolution amortised across all callers.

**Cons**: archive format change (versioned migration); precompile side
needs to emit the table.  Requires touching `archive.rs`, `module.rs` (add
`mount_aliases` field to `VbcModule`), `archive_metadata.rs`, and
`apply_lazy_with_types`.

### Fix C — eager-resolve aliases at precompile time

**Scope**: `crates/verum_stdlib_precompiler` + dependency ordering.

Currently the precompile may compile module A before module B even though
A imports from B.  When A's body references `B.f` via alias, the lookup
fails, the alias is dropped, A's body falls into lenient-skip.

Fix: topologically sort precompile order by mount dependencies, then
re-run `resolve_pending_imports` after all modules are loaded.

**Pros**: smallest code change; one-time fix at precompile time.

**Cons**: doesn't help if the precompile pipeline doesn't allow re-walking
bodies after first-pass.  May still leave gaps for cyclic stdlib mount
graphs.

## Recommended sequencing

1. **Verify trace**: run a failing test with full tracing to confirm
   whether the body recompile is fed from archive bytecode or re-parsed
   source.  Determines Fix A vs Fix B.
2. **Implement Fix B** (archive-side alias plumbing) — most-fundamental,
   eliminates the entire defect class for all consumers regardless of
   compile path.
3. **Add regression test**: pin `core/database/postgres/async_listener.vr`-
   shape repro under `vcs/specs/L0-critical/codegen/mount_alias_aot.vr`
   so future archive-format changes can't regress.

## Workaround inventory (landed in this session)

50+ commits across 40+ files. Pattern: drop the rename in mount block +
fully-qualify the call site.  Sample affected modules (full list in
`git log --grep='task #11'`):

- postgres: async_listener / adapter / async_connection / async_pool /
  async_replication / async_base_backup / extended / wire/typed / row /
  auth/scram
- mysql: async_connection / async_pool
- security: sigstore/{verify, rekor, fulcio} / tuf/{canonical, client,
  role_verify} / webauthn/{authenticate, register, attestation, cose_key,
  authenticator_data} / oidc / jwt / token / password_hash / provenance /
  transparency_log / kdf/hkdf / aead/{aes_gcm + chacha20_poly1305 free-
  function shims added} / ecc/{x25519, ed25519} / util/{constant_time, rng}
- sys: common (mmap/munmap/gettid) / linux/time (rdtsc/rdtscp) /
  process_native (sys_* → platform_syscall.*) / windows/winsock2
  (winsock_read/write/peek/recv_nonblock/send_nonblock → winsock_recv/send)
- encoding: cbor / pem / jcs / json_patch / json_extractor
- runtime: spawn (spawn_with_env_intrinsic / spawn_supervised_intrinsic)
- io: time/system_time (added `epoch_seconds()` alias)
- net: tls13/handshake/{psk, resume_verify} / quic/connection_sm/{rx, aead} /
  weft/{tracing, metrics, handler, health, rpc, router} / h3/qpack/{decoder,
  encoder, instructions, session}
- math: simplicial / giry / nn / tactics
- base: log (LogLevel variants) / panic (Result/Ok/Err explicit mount) /
  primitives (intrinsic_clamp) / memory/cbgr
- mem: segment (popcnt_u64)
- redis: client (cursor_new) / commands (closure capture rebind)
- storage: s3/{client, signing} (Text.from_byte / parse_rfc3339 /
  b64_encode/decode)
- shell: glob / permissions / builtins / stream (max_memory closure capture)
- cli: parser / testing (Text.empty)
- theory_interop: core (compute_obstruction)
- configuration: convert (type_name vs intrinsic)
- verify/kernel_soundness: theorem / rules (List.empty)
- sqlite/native: 8+ rename + variant-rename sites (l1_pager/wal_writer +
  wal_actor_bridge / l7_api/{pragma_api, backup} / l6_session/{connection,
  statement_lifecycle} / l4_vdbe/{cursor_btree_bridge, ssa/from_register_vm} /
  integrity/engine / sqlite_version_fmt/version / l0_vfs/posix_vfs /
  expr_affinity_resolver/node / incrblob/engine)

Also fixed one-off non-rename defects: math/giry let-mut, websocket
payload_len let-mut, ArithOp::AddOp → ArithOp.AddOp Rust-vs-Verum path
separator, wal_writer PagerErrorKind variant qualification.

## Cost / benefit

- Whack-a-mole continuation: ~30-45 min per cycle × N more files = many
  days, brittle (next stdlib commit hits same class).
- Fix B (recommended): 1-2 days of focused work; closes the entire class
  in one rebuild.  Eliminates the need for the per-call-site workarounds
  long-term (workarounds can stay or be opportunistically reverted as
  cosmetic cleanup once Fix B is in place).
