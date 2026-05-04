# `.vbca` format specification

> Status: STABLE for `format_version = 1.0`. Forward-compatible
> additions land at minor-version increments; breaking changes
> require a major-version bump and an explicit migration story.
> Date: 2026-05-04
> Authority: this document is the canonical contract that
> `crates/verum_vbc/src/archive.rs` (writer + reader) implements.
> Any divergence between code and this spec is a bug in the code.

## Purpose

The Verum Bytecode Compiled Archive (`.vbca`) is the single
distribution format for compiled Verum modules. One format serves
three channels:

1. **Embedded stdlib** in the compiler binary — compiled once per
   compiler release, included via `include_bytes!`. Source:
   `target/precompiled-stdlib/runtime.vbca` (Phase 4).
2. **Per-script VBC cache** in `~/.verum/script-cache/<hash>/main.vbca` —
   produced after the first successful run of an unchanged user
   script. Subsequent runs hit cache and skip every front-end phase.
3. **Registry-distributed cog artefacts** — `<name>-<version>-verum-
   <compiler-version>.vbca` next to the source tarball on the
   verum-lang/registry server. Clients fetch the archive instead of
   recompiling cog source on every install.

The same magic header, version tag, and section layout cover all
three. The same `crates/verum_vbc/src/archive.rs` reader / writer
parses every channel.

## Top-level layout

```
┌─ Header (32 bytes, fixed) ──────────────────────────────────────┐
│ magic              [u8; 4]   "VBCA" little-endian               │
│ format_major       u16                                           │
│ format_minor       u16                                           │
│ flags              u32       see ArchiveFlags                    │
│ module_count       u32       number of modules in the archive    │
│ index_offset       u64       absolute byte offset of index       │
│ index_size         u64       size of index in bytes              │
└──────────────────────────────────────────────────────────────────┘

┌─ Module Data Region ────────────────────────────────────────────┐
│ Module 0 bytes                                                   │
│ Module 1 bytes                                                   │
│ ...                                                              │
│ Module N-1 bytes                                                 │
│                                                                  │
│ Each module is a separately serialised VbcModule (bincode +      │
│ optional zstd when ArchiveFlags::COMPRESSED is set).             │
└──────────────────────────────────────────────────────────────────┘

┌─ Index ─────────────────────────────────────────────────────────┐
│ For each ModuleEntry (count = module_count):                    │
│   name_length      u32                                           │
│   name             utf8 bytes                                    │
│   data_offset      u64       absolute offset into archive         │
│   data_size        u64       bytes the serialised module occupies │
│   content_hash     u64       xxh3 of module bytes                 │
│   dependency_count u32                                           │
│   dependencies     [u32; dependency_count]                       │
└──────────────────────────────────────────────────────────────────┘
```

The index sits at the *end* of the archive so a writer can
stream-emit module bytes without knowing total file size up-front.
Readers `seek` to `index_offset`, parse the index, then random-
access individual modules via `data_offset`.

### Magic bytes

`"VBCA"` = `0x56 0x42 0x43 0x41`. Distinguishes a `.vbca` archive
from the per-module `.vbc` format (`"VBC1"` magic, used internally
by the codegen path before archives wrap them).

### Format version

```
format_major:  bumps on breaking layout change. Readers reject
               archives with a major version they don't understand.
format_minor:  bumps on backward-compatible additions (new
               ArchiveFlags bits, new optional ModuleEntry fields).
               Newer readers accept older minors; older readers
               accept newer minors silently.
```

Current version: `1.0`. Major-version bumps require an explicit
migration plan (e.g. on-disk converter, kernel-recheck against the
previous version).

### Flags

```rust
bitflags! {
    pub struct ArchiveFlags: u32 {
        // Channel
        const IS_STDLIB                = 0b0000_0000_0001;
        // Compression
        const COMPRESSED               = 0b0000_0000_0010;
        // Debug payload
        const DEBUG_INFO               = 0b0000_0000_0100;
        const SOURCE_MAPS              = 0b0000_0000_1000;
        // Stripping (release builds)
        const STRIP_FIELD_NAMES        = 0b0000_0001_0000;
        const STRIP_VARIANT_NAMES      = 0b0000_0010_0000;
        const STRIP_CONSTRAINTS        = 0b0000_0100_0000;
        const STRIP_PROTOCOL_DETAILS   = 0b0000_1000_0000;
    }
}
```

Combinations:

- **`IS_STDLIB | COMPRESSED`** — what `verum stdlib precompile`
  emits. Default for embedded stdlib distribution.
- **`COMPRESSED | DEBUG_INFO | SOURCE_MAPS`** — what registry build
  workers emit for cogs. Source-map info is preserved so
  `verum diagnose` can reconstruct line numbers from the published
  archive without the original source.
- **`COMPRESSED | RELEASE_STRIP`** — minimal cog distribution.
  Reflection / debug-printing of stdlib types is broken in
  exchange for ~30% smaller archive. Rarely useful in practice;
  `RELEASE_STRIP` is opt-in via `verum cog precompile --strip-all`.

### Module data region

Each module is one independently-serialised `VbcModule`. The
serialisation format is `bincode` (varint-encoded structural
fields) optionally wrapped in zstd when `COMPRESSED` is set. The
on-the-wire layout of a single `VbcModule` is governed by its
`#[derive(Serialize, Deserialize)]` shape — the canonical reader
is `bincode::deserialize(&bytes)`. Field defaulting via
`#[serde(default)]` provides forward-compat: an older reader sees
a newer-archive `VbcModule` with extra fields and silently ignores
them; a newer reader sees an older archive's missing fields and
fills with `Default::default()`.

Phase 3 of the precompile-stdlib epic added five new fields to
`VbcModule`, all `#[serde(default)]`:

- `cfg_keys: Vec<CfgKey>` — multi-variant cfg-key table.
- `function_variants: Vec<FunctionVariantSet>` — per-function
  variant tables for cfg-conditional bodies.
- `theorems: Vec<TheoremEntry>` — theorem / lemma / corollary /
  axiom / tactic table.
- `framework_provenance: FrameworkProvenance` — `@framework` and
  `@framework_translate` provenance edges.
- `discharge_receipts: Vec<DischargeReceipt>` — content-hash
  pointers into `~/.verum/cert-store/`.

The bincode round-trip is deterministic (HashMap / HashSet are NOT
allowed in any field whose ordering must be stable; sorted
iteration is enforced upstream in code that constructs the module).

### Module index

The index is a flat sequence of `ModuleEntry` records. Sequential
read; no implicit hash table. Lookup-by-name is `Vec::iter().find`
(O(n)) — adequate for stdlib (~600 modules) and cogs (typically
1-50 modules per cog). For sub-millisecond lookup, deserialisers
build a `HashMap<&str, usize>` after parsing the index.

`content_hash` is xxh3 of the module data bytes. Used by:

- `~/.verum/script-cache/` cache invalidation.
- Reproducibility checker (`verum cog reproduce`) to diff
  registry-emitted archive bytes against locally-rebuilt bytes.
- Registry build worker as the artefact-content fingerprint.

`dependencies: Vec<u32>` is a list of module indices into the
archive's own index — strictly intra-archive references. Inter-
archive dependencies (one cog mounting another) are NOT encoded
here; the cog manifest expresses them and the linker resolves at
merge time.

## Determinism guarantees

Two independent `verum stdlib precompile` invocations against the
same source tree, same compiler version, and same target triple
emit byte-identical archives. This is the foundation of
reproducibility checking (Phase 15).

Specifically:

1. **Module enumeration** is path-sorted at the
   `core_compiler::StdlibModuleResolver` level — file enumeration
   order is independent of filesystem inode order.
2. **`StringId` allocation** is path-sorted and intern-order
   stable. The first string interned in any module is always the
   module's own qualified name.
3. **`TypeId` allocation** runs in the global registration phase
   (`compile_core` Step 3) which iterates modules in the same
   path-sorted order. Within a module, types are allocated in
   declaration order.
4. **`FunctionId` allocation** mirrors `TypeId` — global phase,
   path-sorted modules, declaration-order within.
5. **`ConstId` allocation** within a function follows bytecode-
   emission order, which is AST-walk-order, which is parser-order,
   which is byte-order in the source file.
6. **bincode** itself emits deterministic bytes for every shape it
   handles when called against deterministically-ordered input.
7. **zstd** is deterministic given the same level + dictionary
   (none).

Failures of byte-identical reproducibility are bugs and are
caught by `verum cog reproduce` (Phase 15). The reproducibility
checker reads both archives, compares headers (excluding
`content_hash` if set differently), then compares module-by-module.

## Security surface

A `.vbca` is unsigned bytes. The trust model layers signatures on
top:

- **Embedded stdlib** in a compiler binary inherits the binary's
  trust path (publisher's signing key, signed cargo crate, etc.).
  No per-archive signature inside.
- **Per-script VBC cache** is in the user's home directory; trust
  is "this byte sequence reproduces the script's source hash and
  compiler version". The cache key includes the source hash, so
  a bytewise-corrupted cache cannot pass off as a different
  script.
- **Registry-distributed cog `.vbca`** carries a sibling `.sig`
  file in the registry filesystem. The signature covers
  `(magic, version, flags, module_count, index_offset,
  index_size, content_hashes...)`. Clients verify against the
  registry's well-known public key before linking. Phase 13
  registry server-side build worker handles signing; Phase 14
  client-side cog resolver verifies.

The format itself does NOT mandate signatures — embedded use
doesn't need them, per-script cache doesn't need them. Only the
registry channel does, and it carries the signature out-of-band.

## Backward / forward compatibility rules

| Change kind | Allowed within `format_major = 1`? |
|-------------|-------------------------------------|
| Add a new `ArchiveFlags` bit | yes (older readers see "unknown flag", treat as default-off) |
| Add a new field to `ModuleEntry` | yes — must be at the END of the record + bincode tolerates trailing bytes via `serde(default)` |
| Add a new `#[serde(default)]` field to `VbcModule` | yes — Phase 3 demonstrated this with five fields |
| Change a field's serialised type | NO — major bump required |
| Change `MAGIC` bytes | NO — major bump required |
| Change `index_offset` / `index_size` width | NO — major bump required |
| Move sections around (header → index swap) | NO — major bump required |
| Drop a field that older archives serialised | YES if + only if the field is `#[serde(default)]`; older archives reading newer-emitter output get `Default::default()` for the absent field |

When a major bump is necessary, the canonical migration is:

1. Bump `format_major` in code.
2. Reader checks: `format_major <= READER_MAX_MAJOR` else
   `VbcError::UnsupportedFormatVersion`.
3. Add a `verum cog migrate <old.vbca> <new.vbca>` CLI command
   that rewrites old archives to the new layout. Used by the
   registry to bulk-migrate stored artefacts.

## Reader / writer authority

The canonical implementations live in
`crates/verum_vbc/src/archive.rs`:

- `pub fn write_archive(archive: &VbcArchive, writer: impl Write) -> io::Result<()>`
- `pub fn read_archive(reader: impl Read + Seek) -> io::Result<VbcArchive>`
- `pub fn write_archive_to_file(archive: &VbcArchive, path: impl AsRef<Path>) -> io::Result<()>`
- `pub fn read_archive_from_file(path: impl AsRef<Path>) -> io::Result<VbcArchive>`

Tests in `crates/verum_vbc/src/archive.rs::tests` validate the
round-trip property byte-for-byte. The
`test_read_archive_rejects_huge_module_count` test specifically
guards against the adversarial-archive class of vulnerabilities
(where a hostile `module_count = u32::MAX` would cause the reader
to allocate ~70 GB before discovering the file is too short).

## What this format is NOT

- It is NOT a streaming format. The whole archive must be
  available before a reader can locate the index and parse
  individual modules. For streaming, use the per-module `.vbc`
  format directly (bincode-encoded `VbcModule` with no archive
  wrapper).
- It is NOT mmap-friendly. Future Phase 3b work introduces a
  read-optimised v2 format that *is* mmap-friendly with fixed
  header offsets, eager-vs-lazy section split, and zero-copy
  string-table access. The current v1 format trades raw read
  perf for write simplicity.
- It is NOT a content-addressed store. `.vbca` files are named
  by `(name, version, compiler-version, target)` 4-tuple, not
  by their content hash. Content hashes appear in `ModuleEntry`
  for cache-invalidation purposes, not as primary keys.
- It does NOT preserve the source AST. Once compiled, the AST
  is gone. Source-level diagnostics in archived modules require
  the `SOURCE_MAPS` flag to have been set at write time.

## Versioning policy

- Patch (`1.0.X`): no on-disk layout change. Documentation,
  reader/writer micro-optimisations, test additions.
- Minor (`1.X.0`): backward-compat addition. New `ArchiveFlags`
  bits, new `#[serde(default)]` fields on `VbcModule` /
  `ModuleEntry`. Older readers tolerate.
- Major (`X.0.0`): breaking layout change. Migration CLI
  required. Coordinated bump across compiler binary,
  registry server, and `verum cog reproduce` infrastructure.

Major bumps are rare — current trajectory has the v2 read-
optimised format as the next major bump (mmap-friendly,
fixed-position header, zero-copy string-table access). Until
that lands, every change happens at minor.

## Cross-references

- Format implementation: `crates/verum_vbc/src/archive.rs`
- Phase 3 module extensions: `crates/verum_vbc/src/module.rs` (new
  fields), `crates/verum_vbc/src/cfg_key.rs` (CfgKey type)
- Phase 4 producer: `crates/verum_compiler/src/precompile.rs`
- Phase 5 consumer: `crates/verum_compiler/src/embedded_stdlib_vbc.rs`
- Phase 6 linker (in progress): `crates/verum_vbc/src/linker.rs`
- Cog resolver (Phase 14): `crates/verum_modules/src/cog_resolver.rs`
