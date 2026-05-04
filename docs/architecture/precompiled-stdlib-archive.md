# Precompiled stdlib VBC archive — design

> Status: DESIGN, awaiting approval
> Date: 2026-05-04
> Author: design session, addresses #281 + cold-start-perf epic
> Constraints from user:
>   (a) cross-compile ladder must avoid redundancy
>   (b) theorems / proof-code / meta must keep their optimised
>       processed representation through serialise/deserialise

## Problem statement

Cold start of `verum script.vr` on a debug binary takes **~25 minutes** for
trivial scripts; the entire 99.9 % of that time is spent re-running parser →
typecheck → monomorphise → VBC codegen over **stdlib** (~83 K functions).
Node.js / Bun start in milliseconds because their stdlib is already
compiled into the VM at build time. Verum currently treats stdlib as
ordinary user code and re-derives the whole pipeline on every invocation.

The infrastructure for a fix is already partly in place: an embedded
zstd-compressed *source* archive (`embedded_stdlib.rs`), a precomputed
dependency graph (`stdlib_dep_graph.rs`), and a path-based reachability
filter (#109). What is missing is the **last mile** — embedding a
serialised, ready-to-execute VBC archive instead of the compressed
source.

## Goal

Reduce stdlib pipeline cost from ~25 min to **<50 ms** at runtime, while:

- preserving correctness for cross-compile (host build ≠ target triple);
- preserving the proof-code / theorem / certificate representation
  through serialise → embed → deserialise;
- preserving meta-evaluation (`@meta`, `@const`, `@derive`) results;
- shipping a *small* default binary (`--no-default-features` <1 MB stdlib
  overhead) and a *complete* release binary (all three layers ~750 KB).

## Solution: three-layer VBC archive

`stdlib` is split into three explicit layers, each with its own archive,
classification rule, and trust contract:

| Layer | Content | Target-dependence | Embed default | Cold-load cost |
|------|---------|-------------------|---------------|----------------|
| **runtime** | `core.{mem, sync, async_, collections, text, io, net, time, base, sys}` — anything reached during normal program execution. | **Per-target** (FFI decls, syscall numbers) | mandatory | ~5–20 ms (zstd + bincode decode) |
| **proofs** | `theorem` / `axiom` / `lemma`, refinement obligations, framework citations, ATS-V annotations, discharge certificates (Z3 / Lean / Coq stamps). | **Target-agnostic** | feature `proofs` (default on in release) | lazy — first `--verify formal` or `verum audit` |
| **meta** | `@meta` / `@const` / `@derive` evaluators, macro definitions, expansion templates. | **Compiler-version-specific** | feature `meta` (default on in release) | lazy — first user-side meta invocation |

This split answers the user's two constraints:

**(a) Cross-compile redundancy.** Only the *runtime* layer is per-triple.
Proof and meta archives are produced once and reused across every target.
At binary build time we precompile `stdlib_runtime_<host-triple>` and embed
it. For cross-compile builds, `cargo xtask precompile-stdlib --target X`
generates an additional `<target>` archive into
`target/.verum-cache/stdlib-precompiled/<triple>/runtime.vbc.zst` once and
caches it on disk by `(stdlib_content_hash, compiler_version, target)`.
Sub-sequent cross-compiles for the same target reuse the cache. Default
host build embeds *only* the host runtime archive — no 8× target bloat.

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

## Cross-compile cache layout

```
target/.verum-cache/stdlib-precompiled/
├── manifest.toml                          # global cross-target manifest
├── x86_64-apple-darwin/
│   └── runtime.vbc.zst
├── aarch64-apple-darwin/
│   └── runtime.vbc.zst
├── x86_64-unknown-linux-gnu/
│   └── runtime.vbc.zst
├── aarch64-unknown-linux-gnu/
│   └── runtime.vbc.zst
├── x86_64-pc-windows-msvc/
│   └── runtime.vbc.zst
├── proofs.vbc.zst                         # target-agnostic, shared
└── meta.vbc.zst                           # target-agnostic, shared
```

Cache invalidation key: `(stdlib_content_hash, compiler_version, target)`.
First cross-compile to a target generates the cache; subsequent builds
reuse. Cargo never re-precompiles when the inputs are unchanged.

`cargo xtask precompile-stdlib --target X` is also a stand-alone command
for CI — pre-warm the cache for every supported target before publishing
a release.

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
| **3** | `VbcModule` format extension fields + serde-default backward-compat | Low | 1d |
| **4** | `cargo xtask precompile-stdlib` — pipeline implementation | Medium — needs deterministic ID assignment | 3-4d |
| **5** | `embedded_stdlib_vbc.rs` + build.rs hook | Low | 1d |
| **6** | Runtime path switch in `compile_ast_to_vbc` (host build) | Medium — cold start measurement | 2d |
| **7** | Cross-compile cache (`target/.verum-cache/stdlib-precompiled`) | Medium — cache invalidation | 2d |
| **8** | Verify-ladder integration — proof archive lazy-load | Medium — kernel re-check | 2-3d |
| **9** | Meta archive lazy-load | Low | 1d |
| **10** | Mount-path alias removal (after warning cycle) | Low | 0.5d |

Total: ~15-20 days. Phase 1-3 are unblocked by Phase 0 approval; Phase 4-7
form the critical path for cold-start fix; Phase 8-9 are deferable to a
second milestone.

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

## Decision points the user must approve

Before I start writing code:

1. **Three-layer split** (runtime / proof / meta) — agree?
2. **Directory refactor** (`core/runtime/`, `core/proofs/`, `core/meta/`) —
   agree, or keep current layout and use only `@layer(...)` annotation?
3. **Default features in release**: all three layers on, or runtime-only
   with proofs/meta as opt-in `cargo install --features proofs,meta`?
4. **Cross-compile cache**: in `target/.verum-cache/` (per-project) or
   `~/.verum/stdlib-precompiled/` (global)?
5. **Phase 1-3 first?** — quickest path to measuring real-world wins is
   getting the format extension + classifier scanner landed before the
   pipeline work; or do the user prefer pipeline-first because that's
   the actual perf payoff?

Once these five are answered I begin implementation in the order above.
