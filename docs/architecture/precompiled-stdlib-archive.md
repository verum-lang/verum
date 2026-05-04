# Precompiled stdlib VBC archive — design

> Status: DESIGN, awaiting approval
> Date: 2026-05-04
> Author: design session, addresses #281 + cold-start-perf epic
> Constraints from user:
>   (a) cross-compile must work for all supported triples without
>       per-target archive redundancy — single universal archive
>   (b) theorems / proof-code / meta must keep their optimised
>       processed representation through serialise/deserialise
>   (c) maximum runtime performance + minimum binary size + full
>       cross-compile coverage

## Problem statement

Cold start of `verum script.vr` on a debug binary takes **~25 minutes** for
trivial scripts; 99.9 % of that time is spent re-running parser →
typecheck → monomorphise → VBC codegen over **stdlib** (~83 K functions).
Node.js / Bun start in milliseconds because their stdlib is already
compiled into the VM at build time. Verum currently treats stdlib as
ordinary user code and re-derives the whole pipeline on every invocation.

## Stdlib inventory (measured 2026-05-04)

| Metric | Count |
|--------|-------|
| stdlib `.vr` files | **2 413** |
| Total LOC | 512 154 |
| Source size on disk | 38 MB |
| Files containing `@cfg` / `target_os` / `target_arch` | **96** (4.0 %) |
| Per-platform `core/sys/{darwin,linux,windows}/` files | 31 (1.3 %) |
| Files with `theorem` | 142 (5.9 %) |
| Files with `axiom` | 69 (2.9 %) |
| Files with `@verify(formal)` | 54 (2.2 %) |
| Files with `@meta` | 4 (0.17 %) |

Distribution by top-level subtree:

| Subtree | Files | Note |
|---------|-------|------|
| `core/database/` | **1 539** | sqlite + postgres + mysql; ~64 % of stdlib |
| `core/net/` | 191 | TCP/UDP/Unix/DNS/TLS/QUIC |
| `core/math/` | 110 | tensors, frameworks, refinement primitives |
| `core/security/` | 72 | crypto, x509 |
| `core/term/` | 68 | TUI |
| `core/sys/` | 51 | platform-specific |
| `core/verify/` | 43 | verify ladder, kernel hooks |
| All other | ~340 | base, mem, sync, async, collections, text, io, time, shell, runtime, intrinsics, action, encoding, … |

Key observation: **database alone is ~64 % of stdlib by file count**. A
script like `build-paper.vr` doesn't touch any of it. The real
"essential" core that *every* script needs is ~150-200 files (the
non-database, non-net, non-security, non-term, non-database-derived
~340 files in "all other" + the host platform's slice of `sys/`).

Therefore the reachability filter (already embedded as `stdlib_dep_graph`)
is the right primary lever: most scripts touch only ~10 % of stdlib.

## Goal

Reduce stdlib pipeline cost from ~25 min to **<50 ms** at runtime, while:

- supporting cross-compile across **darwin/linux/windows × x86_64/aarch64
  + linux/riscv64 + bpfel/bpfeb + powerpc + s390x + wasm** (10+ triples)
  from a *single* universal archive;
- preserving the proof-code / theorem / certificate representation
  through serialise → embed → deserialise;
- preserving meta-evaluation (`@meta`, `@const`, `@derive`) results;
- shipping a release binary whose stdlib overhead is bounded by ~5–10 MB
  (compressed VBC) regardless of platform count.

## Solution architecture — four orthogonal axes

Four independent design axes combine into one coherent system:

1. **Layer separation** — runtime / proof / meta. Three distinct purposes,
   distinct trust contracts, distinct hot-vs-cold-path treatment.
2. **Multi-variant target encoding** — single archive holds all platform
   variants; loader picks one at runtime. No per-target archive
   duplication.
3. **Lazy reachability** — even within the runtime layer, only modules
   reachable from the user's mount tree are deserialised into the active
   `VbcModule`; the dep-graph and module index are lookup tables, not
   prefetch lists.
4. **Tiered stdlib partitioning** — "essential" core is mandatorily
   embedded; "extra" stdlib (database, net, security, term) is in
   separate archive sections that can be embedded together (default
   release) or shipped as on-demand cog packages (`--features minimal`
   binary).

These compose multiplicatively: a `--help` print on a non-database script
loads ~5 ms of essential-runtime archive ⊓ user-mount reachable subset,
ignores extra-stdlib sections, ignores proof archive, ignores meta
archive. Total work: ~5 ms regardless of the 38 MB of stdlib source on
disk.

## Solution detail: three-layer VBC archive, single universal binary

`stdlib` is split into three explicit layers, each with its own archive,
classification rule, and trust contract:

| Layer | Content | Cross-target story | Embed default | Cold-load cost |
|------|---------|--------------------|---------------|----------------|
| **runtime** | `core.{mem, sync, async_, collections, text, io, net, time, base, sys}` — anything reached during normal program execution. | **Multi-variant**: target-conditional functions stored as N variants in one archive; loader selects on triple at run/AOT time. | mandatory | ~5–20 ms (zstd + bincode decode + variant select) |
| **proofs** | `theorem` / `axiom` / `lemma`, refinement obligations, framework citations, ATS-V annotations, discharge certificates (Z3 / Lean / Coq stamps). | **Target-agnostic** (no platform conditionals). | feature `proofs` (default on in release) | lazy — first `--verify formal` or `verum audit` |
| **meta** | `@meta` / `@const` / `@derive` evaluators, macro definitions, expansion templates. | **Host-only** at evaluator run time (meta runs on host arch); evaluator outputs feed into runtime archive variant selection. | feature `meta` (default on in release) | lazy — first user-side meta invocation |

This split answers the user's two constraints:

**(a) Cross-compile without redundancy.** *Single* universal archive
embedded once per binary. The runtime layer holds **multi-variant
function entries** for the ~5 % of stdlib that has `#[cfg(target_os/arch/…)]`
branches (mostly FFI declarations: `core.sys.darwin.libsystem`,
`core.sys.linux.syscall`, `core.sys.windows.kernel32`, sockaddr layouts,
syscall numbers). Each such function ships once per cfg-arm; everything
else (≈95 % — collections, text, math, async, channels, mem allocator,
sync primitives) ships as **one** target-agnostic copy. Variant selection
is a HashMap lookup at load time keyed on the active target triple, no
re-codegen, no per-target archive duplication.

Quantified: stdlib has ~5 K monomorphised functions, ~250 of them
target-conditional with ≤4 platform variants each. Naive per-target
shipping for 5 supported triples = 5 × 5000 = 25 000 entries
(redundant). Multi-variant single-archive = 5000 + 4 × 250 = 6000
entries. Saving: **76 %** before zstd, more after (zstd compresses near-
identical variants to a few bytes each via dictionary effect).

The same archive serves `verum run script.vr` (host triple), `verum build
--target X` (cross-compile), and `verum check` (target-agnostic) without
filesystem-side per-target caches. Cross-compile is a pure variant pick
at AOT phase 0.

**(b) Proof / meta representation preservation.** Theorem statements,
parameter bindings, generic bindings, per-backend translations, discharge
status, and certificate hashes are all captured in a structured
`TheoremTable` extension to the VBC archive (see *Archive format
extension* below). The optimised processed representation — the form that
type-checking, MSFS-cross-format export, and verify-ladder dispatch
consume — round-trips losslessly through serialise / deserialise. Meta
results (the actual values produced by `@const`/`@meta`) are stored in
the constant pool, indexed by call-site hash, so the same call elides
re-evaluation.

## Read-optimised wire format — deserialization >> serialization

Production reads `.vbca` on every script run; serialisation happens once
per `precompile-stdlib` build or once per `precompile-cog` registry
publish. Therefore the format is designed for **asymmetric speed**: read
path optimised aggressively; write path simple and correct, no read-side
overhead allowed in the name of write convenience.

Concrete techniques:

1. **mmap-friendly fixed-position header.** The first 4 KB contain
   absolute byte offsets of every later section, so the loader can
   `pread` the section it needs without scanning. No varint header sizes,
   no chained TLV that requires sequential walk.
2. **Sectioned compression.** Body regions are zstd-compressed
   per-section (not whole-archive). Eager sections (string table,
   function index, cfg-key table — together ~90 KB for stdlib) are
   *uncompressed* in archive so their cost is one memcpy, not a
   decompress. Lazy sections (function bodies, theorem table,
   discharge receipts, meta cache) are zstd-compressed and decoded
   only on demand.
3. **Index-based O(1) function lookup.** The function index is a flat
   array `Box<[FunctionEntry]>` aligned to 32 bytes — direct array
   indexing by `FunctionId(u32)`, no HashMap lookup. Variant selection
   is a 4-byte slot in `FunctionEntry` pointing at a small variant-table
   region.
4. **Position-independent body encoding.** Bytecode references function /
   string / type / constant IDs through flat index tables. No absolute
   offsets baked in. This means a body region can be mmap'd directly
   into the interpreter's address space without rewriting any pointers.
5. **Asymmetric serde split.** `serialize_archive(&Module) -> Vec<u8>` is
   straightforward and may sort / hash / dedup as much as it likes (it
   runs once, server-side). `deserialize_archive(&[u8]) -> Archive` does
   *zero allocations* for eager sections beyond the per-section box;
   string-pool, function-index, and cfg-key table are returned as
   `&'archive [...]` slices into the mmapped region.
6. **Variant pre-decode caching.** First interpreter call to function F
   materialises the active variant into the live `VbcModule` and caches.
   Subsequent calls on same target hit the cache. Cross-target compile
   paths may evict and re-materialise — cost amortised over the AOT run.
7. **Forward-compat sections.** Every section starts with
   `(name: StringId, format_version: u16, length: u64)`. Unknown sections
   are skipped on read, preserved on resave. Older loaders survive
   newer archive features they don't understand.
8. **Streaming validation, no full parse on load.** Header + section
   table read implies xxh3 checksum verify. Body sections checksummed
   per-region on first decode, lazily. Bad section is isolated — corrupt
   `meta` archive doesn't kill `runtime` archive.

### Concrete on-disk layout

```
┌─ Fixed Header (256 bytes, page-aligned) ──────────────────────────┐
│ magic         u32   "VBCA" little-endian                           │
│ format_major  u16                                                   │
│ format_minor  u16                                                   │
│ flags         u32   bits: signed, multi-variant, layered, …       │
│ archive_size  u64                                                   │
│ checksum      u128  xxh3 of body (offset 256..archive_size)        │
│ string_table  Region (offset, length, compressed?)                  │
│ function_idx  Region                                                │
│ type_idx      Region                                                │
│ cfg_key_table Region                                                │
│ section_dir   Region                                                │
│ signature     Region (optional, present iff flags & SIGNED)         │
│ reserved      [u8; 96]                                              │
└────────────────────────────────────────────────────────────────────┘

┌─ String Table (uncompressed, eager) ───────────── ~50 KB stdlib ──┐
│ count: u32                                                         │
│ offsets: [u32; count]                                              │
│ utf8_bytes: [u8; …]                                                │
└────────────────────────────────────────────────────────────────────┘

┌─ Function Index (uncompressed, eager) ────────── ~40 KB stdlib ──┐
│ count: u32                                                         │
│ entries: [FunctionEntry; count]                                    │
│   { name: StringId(u32), signature_type: TypeId(u32),              │
│     flags: u32, body_kind: u8, body_ref: u64,                      │
│     section_id: SectionId(u16) }                                   │
└────────────────────────────────────────────────────────────────────┘
                  body_kind = Universal → body_ref is offset into Section[section_id]
                  body_kind = MultiVariant → body_ref is offset into Variant Table

┌─ Variant Table (uncompressed, eager) ──────────── ~5 KB stdlib ──┐
│ for each multi-variant function: [VariantSlot; n]                  │
│   { cfg_key_id: u16, section_id: u16, body_offset: u64,            │
│     body_length: u32 }                                             │
└────────────────────────────────────────────────────────────────────┘

┌─ CfgKey Table (uncompressed, eager) ──────────── ~1 KB stdlib ───┐
│ count: u16                                                         │
│ entries: [CfgKey; count]                                           │
└────────────────────────────────────────────────────────────────────┘

┌─ Section Directory (uncompressed, eager) ─────── ~2 KB stdlib ───┐
│ count: u16                                                         │
│ entries: [Section; count]                                          │
│   { name: StringId, layer: u8 (Runtime/Proof/Meta), kind: u8,      │
│     compression: u8, format_version: u16,                          │
│     offset: u64, size_compressed: u64, size_uncompressed: u64,     │
│     checksum: u128 (xxh3) }                                        │
└────────────────────────────────────────────────────────────────────┘

┌─ Body Regions (zstd-compressed, lazy) ────────────────────────────┐
│ Section "runtime.bodies"      ~500 KB compressed                   │
│ Section "proof.theorems"      ~200 KB compressed (default lazy)    │
│ Section "proof.discharge"     ~50 KB compressed (default lazy)     │
│ Section "framework.provenance" ~30 KB compressed (default lazy)    │
│ Section "meta.evaluators"     ~50 KB compressed (default lazy)     │
│ Section "meta.cache"          ~20 KB compressed (default lazy)     │
└────────────────────────────────────────────────────────────────────┘
```

### Eager vs lazy split — what gets read at archive open

**Eager (always read at archive open, ~90 KB total memcpy)**

- Header
- String Table (every section refs strings by ID, can't avoid)
- Function Index (need full table for ID-based lookup)
- Variant Table (loader picks variant per function up-front)
- CfgKey Table (loader resolves active variant)
- Section Directory (need section locations)

**Lazy (decoded on first reference)**

- Function bodies (decoded when interpreter dispatches the function for
  the first time; cached afterwards)
- Theorem table (decoded only when verify-ladder runs)
- Discharge receipts (decoded with theorem table)
- Framework provenance (decoded only by audit / theory-interop tools)
- Meta cache (decoded when meta evaluator hits a cached call site)

Under typical script execution (no `@verify(formal)`, no `verum audit`,
no meta from stdlib), only function bodies are touched — and only the
~50-200 functions main() actually transitively calls. Total cold-start
deserialisation work: **~5-15 ms wall-clock**.

### Parallelism guarantees

The sectioned format is designed so every step parallelises along
independent units of work, both server-side and client-side.

**Serialisation (server-side `precompile-stdlib` / `precompile-cog`)**

- `rayon::par_iter` over modules during typecheck + monomorphization
  (already in place via the parallel post-typecheck fan-out).
- `rayon::par_iter` over functions during VBC codegen — each function
  emits its bytecode independently into a thread-local buffer.
- Per-cfg-arm variant emission: when a function has 4 platform variants,
  4 worker threads emit them in parallel.
- Per-section zstd compression: each section compressed in its own
  worker; with `--long` mode, each large section uses multi-block
  parallelism internally too.
- IDs assigned in deterministic post-merge pass (single-threaded, fast)
  so parallelism doesn't hurt reproducibility.

Target: 8-core machine compresses full stdlib in ~1.5 s wall-clock
(vs ~10 s single-threaded).

**Deserialisation (client-side `compile_ast_to_vbc`)**

- Header / eager sections: ~90 KB single-threaded memcpy. Parallelism
  overhead would dominate; kept sequential.
- Lazy section decode: each section is independent. When user code
  triggers proof archive + meta archive load simultaneously (e.g.
  `verum audit --with-meta-traces`), both decompress on parallel rayon
  workers — ~3 ms each if interleaved, ~1.5 ms total wall-clock with
  parallelism.
- Function-body decode: typical hot path is sequential (interpreter
  dispatches one function at a time), but `phase_dependency_analysis`
  + `phase_cbgr_analysis` walking many functions in parallel decode
  bodies on demand from their respective threads — each body's zstd
  decode is independent.
- Variant materialisation per `compile_ast_to_vbc` call is single-pass
  over function index — ~1 ms; parallelism unnecessary.

Target: client cold-load completes in ~5 ms on single core, no benefit
from going parallel for the eager path; lazy paths parallelise
automatically when triggered concurrently.

### Asymmetric perf measurement (target)

| Metric | Serialisation (server / build-time) | Deserialisation (client / runtime) |
|--------|-------------------------------------|------------------------------------|
| Wall clock for 5 K-function stdlib | 5–10 s | **5–15 ms** |
| Allocations | unbounded (rich AST → flat tables) | ~30 (one Box per eager section) |
| zstd compression | ~3 GB/s output | n/a |
| zstd decompression (lazy) | n/a | ~12 GB/s |
| Memory peak | ~500 MB (full module + intermediate) | ~5 MB (eager sections + decoded bodies) |
| CPU | ~3 cores (parallel codegen) | <0.1 core (single read, no codegen) |

Asymmetry ratio: **~600× faster deserialise than serialise**. This is the
explicit production goal — write-side cost lives on the registry build
worker (paid once per publish, amortised over thousands of installs);
read-side cost lives on every script invocation (must be sub-frame).

## Multi-variant target encoding — the key trick

The mechanism that lets one archive serve all 10+ supported triples
without duplicating common code:

```rust
/// Single VBC function entry. Most stdlib functions ship as one
/// target-agnostic body. Functions with `#[cfg(...)]` branches ship
/// as a `Variants` body with one slot per active cfg-arm.
pub enum FunctionBody {
    /// Target-agnostic — single bytecode body, one FFI symbol set,
    /// works for every triple. ~95 % of stdlib functions.
    Universal {
        bytecode_offset: u32,
        bytecode_length: u32,
        ffi_symbols: SmallVec<[FfiSymbolId; 1]>,
    },

    /// Target-conditional — the precompiler emitted N variants, one
    /// per `#[cfg]` arm in the source. Loader picks the first variant
    /// whose `cfg` matches the active triple.
    Variants(SmallVec<[Variant; 4]>),
}

pub struct Variant {
    /// Structured cfg expression — not a string, so matching is O(1)
    /// HashMap lookup, not regex eval.
    pub cfg: CfgKey,
    pub bytecode_offset: u32,
    pub bytecode_length: u32,
    pub ffi_symbols: SmallVec<[FfiSymbolId; 1]>,
}

/// Compact, hashable cfg expression. Build-time precompiler converts
/// AST `#[cfg(all(target_os = "macos", target_arch = "aarch64"))]` into
/// CfgKey { os: Some(Darwin), arch: Some(Aarch64), ptr_width: None, … }.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct CfgKey {
    pub os:        Option<TargetOs>,    // Darwin | Linux | Windows | FreeBsd | Wasm | None
    pub arch:      Option<TargetArch>,  // X86_64 | Aarch64 | Riscv64 | … | None
    pub ptr_width: Option<u8>,          // 32 | 64
    pub endian:    Option<Endian>,      // Little | Big
    pub features:  SmallVec<[FeatureFlag; 2]>, // sse2, neon, avx2, …
}
```

**Selection at load time**: build a `CfgKey::for_triple(triple)`,
then walk each `Variants(_)` entry and pick the matching arm. ~250
target-conditional functions × ≤4 variants each = ~1000 cfg-key
comparisons, each O(1). Total: <1 ms.

**Size analysis** (estimate, to be measured in Phase 4):

```
stdlib monomorphised functions ............... ~5 000
  target-agnostic ............................ ~4 750  (95%)
  target-conditional ......................... ~250    (5%)
    avg variants per function ................ ~3      (darwin/linux/windows)

per-archive function entries:
  multi-variant single archive: 4 750 + 250 × 3 = 5 500
  naive 5-archive ship-all-targets: 5 × 5 000  = 25 000   (4.5× redundant)
  per-target ship-only-active: 5 000           (need 5 archives stored separately)
  multi-variant + zstd: ~5 500 entries, dictionary-compressed across
    near-identical variants — empirically ~1.05× a single-target archive.
```

Multi-variant single archive **strictly dominates** the alternatives:

- vs naive multi-archive: 4.5× smaller, no per-target file management.
- vs per-target on-disk: same effective size, no filesystem dependency,
  cross-compile zero-overhead.

This is the answer to user constraint (a): **no redundancy, all
platforms in one binary, runtime selects.**

## Multi-archive merge — VBC linker (interpreter + AOT)

Production projects pull stdlib + N cog `.vbca` artifacts + their own
user code into one execution unit. AOT and interpreter resolve this
fundamentally differently, so the design must answer both:

### AOT path — LLVM linker handles it

For AOT compile (`verum build`), each `.vbca` is lowered to one `.o`
through `verum_codegen::llvm::VbcToLlvmLowering`. The system linker
(`ld` / `lld`) then merges the `.o` files into the final binary. Type
deduplication, string-pool merging, and symbol-resolution are LLVM
linker territory — Verum doesn't need a custom layer.

The only Verum-side work is **per-cfg variant selection** at lowering
time. For a function with `Variants(...)` body, the lowerer picks the
arm matching `module.get_triple()` and emits LLVM IR for that arm only.
Other arms are dropped. End result is identical to today's source-based
AOT, just without the parse/typecheck/monomorphise client-side cost.

### Interpreter path — explicit `VbcLinker`

The interpreter takes a single `VbcModule` (function table, bytecode
pool, string table, type table). Today `compile_ast_to_vbc` builds it
from scratch by walking the user AST and re-codegening every reachable
stdlib module. With pre-compiled archives, this becomes a **linker
pass over already-compiled artefacts**:

```rust
// crates/verum_vbc/src/linker.rs — new module inside existing crate
pub struct VbcLinker {
    target_cfg: CfgKey,

    // Linker-local ID space — fresh, monotonically increasing.
    string_pool: StringInterner,
    type_table:     Vec<TypeDescriptor>,
    function_table: Vec<FunctionDescriptor>,
    constant_pool:  Vec<Constant>,
    bytecode:       Vec<u8>,
    theorem_table:  Vec<TheoremEntry>,

    // For each input archive: archive-local ID → linker ID. Built once
    // per archive, used to remap bytecode-embedded IDs.
    remap_tables:   Vec<RemapTable>,
}

pub struct RemapTable {
    string_remap:   Vec<StringId>,        // archive_string_id → linker_string_id
    type_remap:     Vec<TypeId>,
    function_remap: Vec<FunctionId>,
    constant_remap: Vec<ConstId>,
}

impl VbcLinker {
    /// Build a fresh linker for `triple`. The cfg key is computed once
    /// and cached — every variant selection during merge consults it.
    pub fn new(triple: &str) -> Self;

    /// Add an archive (stdlib or cog .vbca). Fast path:
    ///   1. Read eager sections (string-pool, function-index, cfg-key
    ///      table, variant tables) — ~1 ms.
    ///   2. For each function with `Variants(_)`, pick the slot whose
    ///      `cfg` matches `target_cfg`. Drop others.
    ///   3. Allocate fresh linker IDs for active functions / types /
    ///      strings; populate the archive's RemapTable.
    ///   4. Append (renumbered) FunctionDescriptors / TypeDescriptors
    ///      / Constants to the linker tables.
    ///   5. Body bytes: defer until finalisation. The decoded /
    ///      lazy-decompressed body region is appended at the end of
    ///      linker.bytecode with a per-function offset; bytecode
    ///      embedded IDs are rewritten through the RemapTable on
    ///      first dispatch (or eagerly during finalize, configurable).
    pub fn add_archive(&mut self, archive: &VbcArchive) -> Result<()>;

    /// Add a freshly-codegen'd user module (`compile_ast_to_vbc` output
    /// for the user's source files only — no stdlib).
    pub fn add_user_module(&mut self, module: VbcModule) -> Result<()>;

    /// Consume the linker, produce the consolidated runtime VbcModule.
    /// The interpreter uses this directly. AOT also uses this when the
    /// `--cross-target-via-vbc` mode is selected (default = direct
    /// per-archive LLVM lowering).
    pub fn finalize(self) -> VbcModule;
}
```

**Performance contract for the linker hot path:**

- Per-archive add: O(eager-section size) + O(variant table) + O(name
  collision dedup). For the stdlib runtime archive (~5 K functions):
  ~5 ms wall-clock.
- N cog archives: each ~1-50 functions → ~0.5 ms each. 20 cogs ≈ 10 ms.
- User module add: source codegen still happens, but only for user
  functions (~10-100 typical). ~5 ms.
- Finalize: O(total functions) for descriptor renumbering, O(total
  bytecode) for ID rewriting. Both ~10 ms for medium project.

**Total cold-start linker work**: ~30 ms for medium project (stdlib +
20 cogs + user). vs ~25 min today for stdlib-from-source + user.
**~50 000× speedup**.

**Parallelism:**
- Multiple archives can be **deserialised in parallel** (Rayon par_iter
  over `archives` slice during `decode_eager_sections`).
- ID allocation must be sequential (counter-based — single-threaded for
  determinism).
- Bytecode rewriting is **per-function parallel** at finalize time
  (each function rewrite is independent given the RemapTable).
- Net: 8-core machine completes the whole link in ~5-10 ms.

### Deduplication policy

Across N archives, the linker dedups by **content hash**:

- **Strings**: identical UTF-8 byte sequences map to same linker StringId.
- **Types**: identical `TypeDescriptor` (after recursive remap of
  contained TypeIds) map to same linker TypeId. Critical for
  `List<Int>` appearing in 10 cogs to consume one TypeId, not 10.
- **Constants**: identical bit pattern + same TypeId → same ConstId.
- **Functions**: NEVER deduped by name across archives. `cog_a.parse`
  and `cog_b.parse` are distinct symbols at distinct qualified paths;
  collision only on user-redeclared stdlib symbols, which is a hard
  error.

Dedup tables built per-archive add via `HashMap<ContentHash, LinkerID>`
(amortised O(1) lookup). Dedup pays for itself when stdlib types
appear across cogs — typical 30-50 % type-table reduction in real
projects.

### Backwards-compatibility with current `compile_ast_to_vbc`

Phase 6 replaces today's flow:

```
old:  user_ast → typecheck → monomorphise stdlib + user → VbcModule
new:  load embedded stdlib archive
     + user_ast → typecheck (against stdlib archive's exports table)
              → monomorphise user only (no stdlib touched)
              → user_ast → VbcModule  // user-only
     VbcLinker.new(triple)
       .add_archive(stdlib)
       .add_archive(cog_1) .. .add_archive(cog_N)
       .add_user_module(user_module)
       .finalize()  → final VbcModule for interpreter
```

The user's `compile_ast_to_vbc` API stays the same shape; the
implementation changes from "re-codegen everything" to "link archives
+ codegen user-only". External callers (CLI, REPL, test runner) don't
notice the swap.

## Phase 1 finding — item-level layers, not file-level split

The Phase 1 classifier (`verum audit --stdlib-layers`) ran against the
embedded archive (2 413 modules) and produced this baseline:

| | Count | % |
|--|--|--|
| pure runtime | 1 731 | 71.7 % |
| pure proof   | 42 | 1.7 % |
| pure meta    | 0 | 0 % |
| **mixed (proof+runtime)** | **109** | **4.5 %** |
| parse errors | 3 | 0.1 % |
| empty (re-export-only `mod.vr`) | 528 | 21.9 % |

The 109 mixed-layer modules cluster in `core/math/*` (70 of them),
`core/action/*` (12), `core/verify/*` (16) and a handful elsewhere.
Files like `core/math/giry.vr` (31 runtime + 12 proof items),
`core/math/algebra.vr` (27 runtime + 37 proof), `core/math/tactics.vr`
(10 runtime + 75 proof) naturally interleave executable definitions
with theorems *about* those definitions. Splitting them into separate
files would (a) duplicate the type-context plumbing per file, (b) break
the natural co-location idiom where `theorem foo_correct` sits next to
the `fn foo` it proves, (c) inflate the directory-refactor blast radius
to the point where Phase 2 becomes a multi-week mechanical sweep.

**Decision (informed by data):** layers are encoded at the **item level**
in the VBC archive, not the file level. The classifier's per-item tally
is exactly what the Phase 4 precompile pipeline consumes: each emitted
`FunctionEntry` / `TheoremEntry` / `MetaEntry` carries its own
`section_id`, and the archive groups items into runtime / proof / meta
sections regardless of which source file they came from.

This means **Phase 2 (directory refactor) is no longer required**.
`core/` keeps its current shape; the precompile pipeline reads source
file authority, classifies per-item, and writes layered output. The
classifier stays as the audit tool that surfaces stylistic
inconsistencies (e.g. a module with one stray `theorem` may want to
move that to a sibling file for readability), but it's no longer a
pre-condition for Phase 4.

Revised phase plan: skip Phase 2, fold any directory hygiene into a
post-Phase-9 cleanup pass.

## Layer classification

Three classification mechanisms, applied in priority order:

1. **Explicit module-level**: `@layer(runtime|proof|meta)` at the top
   of the `.vr` file — overrides everything else.
2. **Cog manifest**: `[layer]` key in `verum.toml` applies to the cog
   as a whole.
3. **Auto-detect** (default for stdlib): a build-time scanner walks
   each module's AST. Modules whose every public item is `theorem`,
   `axiom`, `lemma`, or annotated with `@verify(formal)` and contains
   no executable body → **proof**. Modules whose public items are
   `@meta` / `@const` / macro definitions → **meta**. Everything else
   → **runtime**.

Mixed-layer modules raise a build-time error pointing at the offending
items and suggesting an explicit `@layer(...)`. This forces architectural
hygiene: a runtime module can't quietly accumulate proof code that
inflates the runtime archive.

## Archive format extension

`VbcModule` gains four new fields, all `#[serde(default)]` so the format
stays backward-compatible with existing on-disk caches:

```rust
pub struct VbcModule {
    // ... existing 30 fields ...

    /// Theorem / axiom / lemma table — populated for proof-layer
    /// modules and for runtime modules carrying refinement contracts.
    /// Empty for plain runtime modules without contracts.
    #[serde(default)]
    pub theorems: Vec<TheoremEntry>,

    /// Discharge certificates for refinement obligations and
    /// theorems. Indexed by (theorem_id, backend). Backend bodies
    /// are content-addressed — only the hash is stored here, the
    /// full proof body lives in a sibling certificate-store
    /// (`~/.verum/cert-store/<hash>`).
    #[serde(default)]
    pub discharge_receipts: Vec<DischargeReceipt>,

    /// `@framework(name, citation)` and
    /// `@framework_translate(src, tgt, citation)` provenance edges.
    /// Loaded eagerly (cheap), used by audit + theory-interop tools.
    #[serde(default)]
    pub framework_provenance: FrameworkTable,

    /// Meta-evaluation cache — per-call-site `@meta`/`@const` results
    /// keyed by the canonical call-site hash. Hit means the embedded
    /// VBC already contains the evaluated value; miss means re-evaluate.
    #[serde(default)]
    pub meta_results: Vec<MetaCacheEntry>,
}
```

`TheoremEntry` mirrors `verum_kernel::soundness::corpus_export::TheoremSpec`
extended with execution metadata:

```rust
pub struct TheoremEntry {
    pub name: StringId,
    pub module_path: StringId,
    pub proposition: PropositionRef,         // VBC-typed predicate body
    pub params: Vec<TheoremParam>,
    pub generics: Vec<TheoremGeneric>,
    pub per_backend_propositions: BTreeMap<BackendId, StringId>,
    pub lifecycle: Lifecycle,                // [T] / [H] / [D] / [C] / [P] / [I] / [✗]
    pub stratum: MsfsStratum,                // ATS-V stratum
    pub foundation: Foundation,              // ATS-V foundation
    pub discharged_by: Vec<DischargeRef>,    // edges into discharge_receipts
}
```

`PropositionRef` is a typed AST pointer into the function table — it
points at a VBC predicate function the verify-ladder runs (the same
shape `requires`/`ensures` predicates compile to today). This is the
**optimised processed representation** the user mentioned in (b): the
predicate is already typed, name-resolved, monomorphised, and lowered
to VBC. Lean / Coq / Agda / Isabelle / Dedukti translations are stored
as already-rendered text in `per_backend_propositions` so cross-format
export is a HashMap lookup, not a re-render.

## Build-time pipeline

New entry: `cargo xtask precompile-stdlib [--target TRIPLE]`.

```
cargo xtask precompile-stdlib --target aarch64-apple-darwin

    Phase 1  scan core/ → classify each module
                  (auto + explicit + manifest, fail on mixed)
    Phase 2  parse all modules
    Phase 3  typecheck against bootstrap stdlib metadata
    Phase 4  monomorphise per layer
                  - runtime: full mono
                  - proof:   predicate bodies + theorem statements only
                  - meta:    evaluator bodies; pre-compute @const cache
    Phase 5  VBC codegen per layer
    Phase 6  attach proof certificates from ~/.verum/cert-store/
    Phase 7  serialise VbcModule per layer + zstd compress
    Phase 8  emit:
        target/precompiled-stdlib/<triple>/runtime.vbc.zst
        target/precompiled-stdlib/proofs.vbc.zst        # target-agnostic
        target/precompiled-stdlib/meta.vbc.zst          # target-agnostic
        target/precompiled-stdlib/<triple>/manifest.toml
            content_hash, compiler_version, build_timestamp,
            triple, layer_sizes
```

Hooked into `build.rs` of `verum_compiler`:

```rust
// build.rs
fn main() {
    let host_triple = env::var("TARGET").unwrap();
    if !precompiled_stdlib_present(&host_triple)? {
        run_xtask("precompile-stdlib", &["--target", &host_triple])?;
    }
    // Tell the source archive embedding code where to find the VBC files
    println!("cargo:rustc-env=STDLIB_RUNTIME_VBC={}", runtime_vbc_path);
    if cfg!(feature = "proofs") {
        println!("cargo:rustc-env=STDLIB_PROOFS_VBC={}", proofs_vbc_path);
    }
    if cfg!(feature = "meta") {
        println!("cargo:rustc-env=STDLIB_META_VBC={}", meta_vbc_path);
    }
}
```

For cross-compile, `verum build --target X` triggers the same xtask with
`--target X` if the cache misses; the emitted archive lives in
`target/.verum-cache/stdlib-precompiled/<X>/runtime.vbc.zst` and is
*looked up by the compiler at runtime*, not embedded.

## Runtime path

`compile_ast_to_vbc` (the hot path that today walks
`collect_imported_stdlib_modules` — 83 K functions) becomes:

```rust
pub fn compile_ast_to_vbc(&self, user: &Module) -> Result<Arc<VbcModule>> {
    // 1. Deserialise embedded runtime stdlib (~5-20 ms).
    let mut module = embedded_stdlib_vbc::runtime_for_target(target_triple)?;

    // 2. Compile user module on top.
    let user_vbc = vbc_codegen::compile_module(user, &module)?;

    // 3. Merge into single module (dedup ID space).
    module.merge(user_vbc)?;

    Ok(Arc::new(module))
}
```

Old path (full re-codegen of stdlib) is retained behind
`VERUM_NO_PRECOMPILED_STDLIB=1` as the dev-mode escape hatch — used when
stdlib source is modified mid-session and the embedded archive is stale.
The compiler detects staleness automatically by hashing `core/` and
comparing to the manifest.

## Cross-compile — no separate cache, no per-target files

The multi-variant archive **eliminates** the need for a per-target
filesystem cache. Cross-compile is a pure variant pick at AOT phase 0:

```
verum build --target aarch64-unknown-linux-gnu

  Phase 0:  let archive = embedded_stdlib_vbc::runtime();   // already in binary
            let cfg_key = CfgKey::for_triple("aarch64-unknown-linux-gnu");
            let module  = archive.materialize(&cfg_key);    // ~5 ms
  Phase 1+: continue with user-source codegen
```

No `target/.verum-cache/stdlib-precompiled/<triple>/` directory, no
`(stdlib_hash, compiler_version, target)` cache invalidation, no
network/filesystem dependency for cross-compile. All supported targets
cross-compile out of the box from the same binary.

**Tiered stdlib partitioning** (axis 4) addresses binary-size budget:

```
binary feature flags:
  --features minimal      runtime-essential only (~150 modules)
                          → ~1.5 MB embedded VBC after zstd
                          → no proofs, no meta, no extra stdlib
                          → for embedded / WASM builds
  --features default      runtime-essential + runtime-extra (database,
                          net, security, term, math, shell)
                          + proofs + meta
                          → ~10 MB embedded VBC after zstd
                          → standard desktop / server build
  --features full         everything default ships +
                          experimental cog archives
                          → ~15 MB embedded VBC after zstd
```

The "extra" runtime sub-archives (database, net, …) are *separate
sections* inside the same VBC archive file. The embedding harness
chooses which sections to `include_bytes!` based on cargo features.
Sections not embedded become external cog dependencies (downloadable
via `verum cog install core-database@<compiler-version>`).

## Refactor — three top-level core/ subdirectories

Source layout cleanup, justified by user's "вынести весь meta код в
отдельные модули как и все proof":

```
core/
├── runtime/          # current core/ minus theorems, axioms, lemmas, @meta
│   ├── mem/
│   ├── sync/
│   ├── async_/
│   ├── collections/
│   ├── text/
│   ├── io/
│   ├── net/
│   ├── time/
│   ├── base/
│   └── sys/
├── proofs/           # all theorem/axiom/lemma + frameworks
│   ├── refinement/
│   ├── theorems/     # bare statements (formerly inline in giry.vr etc.)
│   ├── frameworks/   # owl2_fs, lurie_htt, msfs, ...
│   └── interop/      # bridges
└── meta/             # @meta + @const + @derive + macros
    ├── derives/
    ├── constexpr/
    └── macros/
```

Migration is mechanical (a follow-up PR after the archive format lands):
move files, update mount paths, run the classifier to verify hygiene.
Backward-compat aliases: `mount core.mem.X` resolves to
`core.runtime.mem.X` for one release cycle, then warning, then removal.

This makes (b) free: classifier doesn't need to inspect modules at all
once the directory layout is the authority. `core/proofs/**` →
proof layer. `core/meta/**` → meta layer. `core/runtime/**` → runtime.

## Trust contract for proofs

A discharged theorem in `stdlib_proofs.vbc` is **not** trusted just
because it's embedded. The VBC archive ships only the *certificate
hash*; the verify-ladder still re-runs the kernel against the
hash → `~/.verum/cert-store/<hash>` proof body the first time the
theorem is referenced after the binary is upgraded. The kernel-recheck
takes ~50 ms per theorem (one Z3 unsat replay or one Lean kernel
re-elaborate). Result is cached per-binary-version in
`~/.verum/replay-cache/<compiler_version>/<theorem_id>` so subsequent
runs of the same compiler are O(1).

This preserves the "audit by anyone, anytime" property — embedded
certs are never the trust root, the kernel is.

## Phase plan

| Phase | Output | Risk | Estimate |
|-------|--------|------|----------|
| **0** | This design doc reviewed + approved | — | done |
| **1** | Layer classifier scanner (`cargo xtask classify-stdlib`) — emits report only, no enforcement | Low — read-only audit | 1d |
| **2** | `core/` directory refactor: move proof/meta into `core/proofs/` and `core/meta/`, keep mount-path aliases | Mechanical but wide-radius | 2-3d |
| **3** | `VbcModule` format extension fields + multi-variant `FunctionBody` + serde-default backward-compat + `.vbca` magic header for stand-alone archives | Low | 2d |
| **4** | `cargo xtask precompile-stdlib` — pipeline implementation, multi-variant emission | Medium — needs deterministic ID assignment | 4-5d |
| **5** | `embedded_stdlib_vbc.rs` + build.rs hook | Low | 1d |
| **6** | Runtime path switch in `compile_ast_to_vbc` (host build) — `archive.materialize(&CfgKey::for_triple(triple))` | Medium — cold start measurement | 2d |
| **7** | Cross-compile path: zero-cache, archive variant pick at AOT phase 0 | Low (multi-variant collapses this) | 1d |
| **8** | Verify-ladder integration — proof archive lazy-load | Medium — kernel re-check | 2-3d |
| **9** | Meta archive lazy-load | Low | 1d |
| **10** | Mount-path alias removal (after warning cycle) | Low | 0.5d |
| **11** | `.vbca` format spec freeze + standalone archive doc | Low | 1d |
| **12** | `cargo xtask precompile-cog` — same pipeline as stdlib but scoped to single cog | Low (reuses Phase 4 work) | 1-2d |
| **13** | Registry server-side: build worker, `.vbca` publish, signing, manifest | Medium — operational | 3-5d (registry-side) |
| **14** | Client `cog_resolver` enhancements: prefer `.vbca`, signature verify, fallback to source | Low | 2d |
| **15** | Reproducibility checker (`verum cog reproduce <name>@<ver>` rebuilds from source, byte-compares to registry `.vbca`) | Low | 1d |

Total compiler-side: ~17-22 days (Phases 1-12, 14-15).
Registry-side: ~3-5 days (Phase 13).

Phases 1-9 are the **stdlib cold-start fix** (the original perf goal).
Phases 11-15 generalise the format to cogs and unlock registry
distribution. Phases 11-12 are pure compiler-side; Phase 13 is
registry-server work and lands independently.

## Open questions

1. **Determinism of monomorphisation IDs across host/cross builds?** The
   current path-sorted iteration (#143) gives a deterministic order for a
   single target but cross-target archives must agree on TypeId layout.
   Probably OK because TypeId is only meaningful inside a single VBC
   module, and runtime/user merge is per-binary-target.
2. **Format-version bump**: existing `~/.verum/script-cache/` entries
   serialised under old format must be invalidated. Use the existing
   `compiler_version` axis in the cache key — bumping the version number
   in `Cargo.toml` automatically invalidates.
3. **Embedded-archive size budget**. Current source archive is 4.7 MB
   compressed. Runtime VBC archive is denser (no comments, no
   whitespace, no doc strings), expected ~500 KB compressed. Total
   binary growth from this work is **net negative** because the source
   archive can be downgraded to a lazy-load fallback only.

## Registry-distributed `.vbca` artifacts — same format, same loader

The VBC archive format defined here is **not stdlib-specific**. It is the
canonical compiled-cog format — the "Verum Bytecode Compiled Archive"
(`.vbca`) — and serves *three* distribution channels under one trust
model and one loader:

| Channel | What | Producer | Consumer |
|---------|------|----------|----------|
| **Embedded stdlib** | `core` cog precompiled into VBC archive | `cargo xtask precompile-stdlib` (build-time) | `embedded_stdlib_vbc.rs`, included in binary |
| **Script VBC cache** | User-script VBC after first cold compile | Compiler `phase_interpret_with_args` | `~/.verum/script-cache/<hash>/main.vbca` |
| **Registry cog artifacts** | Third-party cogs precompiled by the registry | Registry build worker (server-side) | Client `verum cog install foo@1.2.3` |

Symmetry of these three closes the user's question about
**fastest-possible builds**: when a user runs `verum build` on a project
with 20 dependencies, each dependency arrives over the wire as a
`.vbca` ready to merge into the user's `VbcModule`. No client-side
parse, typecheck, or codegen of cog sources. Per-cog cost goes from
*seconds-to-minutes* (parse + typecheck + monomorphise + codegen the
cog's source) to *milliseconds* (zstd decode + variant pick + ID
remap during merge).

### Registry build pipeline

The registry (verum-lang/registry) gains a build worker that runs after
each cog upload:

```
publish flow
─────────────
1. publisher  →  POST /cogs/<name>/<version>  (uploads source.tar.gz)
2. registry   →  store source.tar.gz, kick off build worker
3. worker     →  cargo xtask precompile-cog <name>@<version>
                   ─ classify modules (runtime / proof / meta)
                   ─ parse + typecheck against pinned compiler version
                   ─ monomorphise + emit multi-variant VBC
                   ─ collect proof certs from cert-store
                   ─ serialise + zstd → <name>-<version>-<compiler-version>.vbca
                   ─ sign with registry private key
4. registry   →  publish .vbca + sig + manifest (lists available
                  triples, layers, compiler versions, content hash)

install flow
────────────
client:  verum cog install foo@^1.2

  resolve  →  registry.get_metadata("foo", "^1.2")
              → returns version 1.2.7, available .vbca for compiler
                version 0.X with all 10 supported triples in one archive
  fetch    →  GET foo-1.2.7-verum-0.X.vbca       (~500 KB compressed)
              GET foo-1.2.7-verum-0.X.vbca.sig
  verify   →  signature check against registry pubkey
  cache    →  ~/.verum/cogs/foo/1.2.7/foo.vbca
  link     →  on next compile, embed .vbca into project's VbcModule
              via the same archive merge code that ingests stdlib
```

### Why this is the optimal distribution model

For a project with N dependencies:

| Step | Source-tarball flow (today) | `.vbca` flow |
|------|----------------------------|--------------|
| Network | ~1 MB tarball per cog | ~500 KB .vbca per cog |
| Decode | tar+gzip extract (~100 ms) | zstd decompress (~10 ms) |
| Parse + typecheck + monomorphise + codegen | **per-cog, every user, every build** (1-30 s each on debug builds) | **once on registry, never on client** |
| Variant select for current target | n/a (had to be re-codegened) | O(1) per function (~1 ms total) |
| Merge into user VbcModule | wide-radius re-codegen | linear remap of FunctionId / TypeId space (~10 ms) |
| **Per-cog client cost** | ~1-30 s | **~50 ms** |
| **20-dependency project** | 20 s – 10 min cold | **~1 s cold** |

**Bonus**: registry can pre-build `.vbca` for *every* supported compiler
version → user's compiler version match is a metadata lookup, not a
"sorry, recompile" failure.

### Trust model

```
publisher signs source                    → publisher key
registry rebuilds .vbca, signs            → registry build-worker key
client verifies registry signature        → registry pubkey embedded in binary
optional: client also verifies publisher  → publisher key from .vbca manifest
optional: client re-runs kernel-recheck   → reproducible local rebuild from source
                                            should yield byte-identical .vbca
```

The byte-identical-rebuild property is a function of the deterministic-ID
work in the precompile pipeline. As long as path-sorted iteration (#143)
holds and the compiler version is pinned, anyone can verify the registry
hasn't tampered with the artifact.

### Cog-side multi-variant — same encoding as stdlib

A cog's `core.foo.unix` module has the same `#[cfg(target_os = "macos")]`
/ `#[cfg(target_os = "linux")]` branches as stdlib. The `.vbca` ships
both variants in `Variants(_)` entries. One `.vbca` per cog version
covers every triple — registry doesn't store per-target tarballs.

### Client fallback path

When `.vbca` is unavailable or registry is offline:

```
1. .vbca download fails
2. Try .vbca from local cache (~/.verum/cogs/cache/)
3. Try source.tar.gz, compile locally (slow path, current behaviour)
4. Try git URL (registry-form `{ git = "..." }`)
5. Hard failure with diagnostic
```

Local-path cogs (`{ path = "./local" }`) always compile from source —
no .vbca on the registry. This is the dev workflow.

### Registry build cost vs current model

The registry **trades server CPU for client cold-start time**. Per cog
publish:

- Build worker: ~30 s – 5 min (parse + typecheck + monomorphise + codegen
  for one compiler version).
- Storage: ~500 KB per (cog × compiler-version) — small relative to
  source tarball.

This cost is paid **once per publish**, amortised over every install of
that cog version — typically 100s-10000s of installs in registry-scale
ecosystems. Win is enormous.

## Performance / size summary

| Metric | Today | After this design | Change |
|--------|-------|-------------------|--------|
| Cold start (`verum --help` style) | ~25 min debug, ~1-2 min release | <50 ms | **~30 000×** |
| Embedded stdlib in binary | 4.7 MB source (zstd) | 5–10 MB VBC (zstd) | +5 MB |
| Source archive in binary | 4.7 MB (zstd) | 0 (lazy fallback only) | −4.7 MB |
| Net binary size delta | — | **+0.3 to +5 MB** | within budget |
| Cross-compile per-target storage | 170 MB `registry.bin` per workspace | 0 | −170 MB |
| Memory at runtime (host) | ~7 GB peak | ~50–200 MB | **35–140×** |
| Function table at codegen | 83 K (all stdlib) | ~5 K (mount-reachable) | **17×** |
| `--features minimal` binary | n/a | ~1.5 MB stdlib overhead | — |

## Trade-off decisions taken in this design

1. **Single universal archive vs per-target archives** → universal,
   multi-variant. Justified by 4.5× size economy and zero filesystem
   per-target cache.
2. **Runtime/proof/meta split vs monolithic archive** → split. Lazy load
   for proof/meta means zero cost when not used; clean separation of
   trust contracts.
3. **Embed sources as fallback or remove?** → remove from default
   release; keep only behind `--features dev-stdlib-source` for
   self-modifying-stdlib workflows.
4. **Cross-compile cache on disk or in-binary?** → in-binary, no
   filesystem dependency, no cache-invalidation logic needed.
5. **Tiered partitioning vs everything-or-nothing?** → tiered. `minimal`
   for embedded targets, `default` for desktop, `full` for audit servers.

## Decision points the user must approve

Before I start writing code:

1. **Layer separation + directory refactor** to `core/runtime/`,
   `core/proofs/`, `core/meta/` — agree, or keep current layout and use
   only `@layer(...)` annotation? (Refactor is mechanical but wide-radius.)
2. **Tier partition** — runtime-essential vs runtime-extra (database,
   net, security, term as on-demand cogs) — agree this is the right
   split, or do you want database to be in the default-embedded tier?
3. **Phase ordering** — Phase 4 (precompile pipeline) is the critical
   path for the stdlib perf win. Phase 1-3 (classifier + format extension +
   refactor) are necessary preconditions. Approve "stdlib phases 1-9 first,
   then registry phases 11-15"?
4. **Backward-compat duration** — mount-path aliases for the directory
   refactor: 1 release cycle (warning), then removal? Or evergreen?
5. **Embedded-sources fallback** — `--features dev-stdlib-source`
   default off, only used by people developing stdlib itself?
6. **Registry build-worker hosting** — registry self-hosts the build
   worker (server pays CPU for every publish), or community-CI publishes
   `.vbca` next to source (publisher pays via GitHub Actions / similar)?
7. **Registry signing model** — single registry key (simple, requires
   trusting registry), or publisher key + registry counter-signature
   (more secure, more UX friction at publish time)?

Once these are answered I begin implementation.
