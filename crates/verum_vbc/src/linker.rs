//! VBC linker — content-hash-deduplicated cross-archive merge.
//!
//! Phase 6b of the precompiled-stdlib archive epic. The linker takes
//! N independently-compiled `VbcModule`s (typically one stdlib archive
//! + one user module) and merges them into a single `VbcModule` the
//! interpreter can dispatch.
//!
//! # Why a linker
//!
//! Each [`VbcModule`] carries its own ID spaces — `StringId`,
//! `TypeId`, `FunctionId`, `ConstId`, `ProtocolId`. Two independently-
//! compiled modules will have ID `7` mean different things. To execute
//! code from module A that calls a function from module B, the linker
//! must:
//!
//! 1. Allocate fresh IDs in the linker's output space for every
//!    descriptor in every input module.
//! 2. Build a per-input-module remap table (`source_id → linker_id`).
//! 3. Walk every place an ID can appear — type-table, function-table,
//!    constant-pool, source-map, theorem-table, *and the bytecode
//!    itself* — and rewrite source IDs to linker IDs through the
//!    remap.
//!
//! Step 3 is the load-bearing piece. Type-table / constant-pool
//! rewrites are recursive over [`TypeRef`] and [`Constant`]; bytecode
//! rewrites decode each instruction, replace embedded IDs, and
//! re-encode.
//!
//! # Content-hash deduplication
//!
//! When two source modules intern the same string (`"hello"`) or
//! declare the same monomorphisation (`List<Int>`), the linker
//! collapses them to a single linker-side ID. This is essential for
//! correctness with multiple archives — if two cogs both monomorphise
//! `List<Int>`, dispatching `List::push` on a `List<Int>` from cog A
//! must reach the same function table entry as the `List::push` from
//! cog B.
//!
//! v1 dedups strings (cheap content-hash). Type / function dedup is
//! deferred to v2 once production traffic establishes which dedup
//! axes give the most archive-size win.
//!
//! # Determinism
//!
//! Adding the same set of archives in the same order to two fresh
//! linkers produces byte-identical output. ID allocation is monotone
//! counter-based; per-source remap iteration follows the input
//! module's table order (which is itself deterministic by the Phase-4
//! precompile-stdlib invariant).
//!
//! # Public API
//!
//! ```ignore
//! let mut linker = VbcLinker::new("aarch64-apple-darwin");
//! linker.add_archive(&stdlib_archive)?;
//! linker.add_user_module(user_vbc)?;
//! let final_module: VbcModule = linker.finalize();
//! interpreter.execute(&final_module);
//! ```

use std::collections::HashMap;

use crate::archive::VbcArchive;
use crate::bytecode::{decode_instruction, decode_instructions, encode_instructions};
use crate::cfg_key::CfgKey;
use crate::instruction::Instruction;
use crate::module::{
    ConstId, Constant, FunctionDescriptor, FunctionId, FunctionVariantSet, ParamDescriptor,
    SourceMap, SourceMapEntry, SpecializationEntry, VbcModule, VbcVariant,
};
use crate::types::{
    ContextRef, FieldDescriptor, ProtocolId, ProtocolImpl, StringId, TypeDescriptor, TypeId,
    TypeParamDescriptor, TypeRef, VariantDescriptor,
};

/// Errors raised during link-time merge.
#[derive(Debug)]
pub enum LinkError {
    /// Bytecode rewriter encountered an opcode it doesn't know how to
    /// rewrite. The variant-list inside [`crate::instruction::Instruction`]
    /// has grown faster than this rewriter; add the new variant to
    /// `rewrite_instruction_ids` and re-run.
    UnhandledInstruction { opcode: u8, offset: usize },
    /// A `func_id` / `type_id` / `string_id` operand referenced an ID
    /// outside the source module's table. Either the source archive is
    /// corrupt or the rewriter has a bug.
    DanglingReference {
        kind: &'static str,
        source_id: u32,
    },
    /// The bytecode instruction stream couldn't be decoded — a length
    /// prefix said `N` bytes but only `M < N` were available.
    TruncatedBytecode { offset: usize, want: usize },
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnhandledInstruction { opcode, offset } => write!(
                f,
                "linker: unhandled instruction opcode 0x{:02x} at bytecode offset {}",
                opcode, offset
            ),
            Self::DanglingReference { kind, source_id } => {
                write!(f, "linker: dangling {} reference #{}", kind, source_id)
            }
            Self::TruncatedBytecode { offset, want } => write!(
                f,
                "linker: truncated bytecode at offset {} (need {} more bytes)",
                offset, want
            ),
        }
    }
}

impl std::error::Error for LinkError {}

/// Per-source-module remap table — translates the source's local IDs
/// into the linker's output ID space.
///
/// Note on key shape: `StringId` values are *byte offsets* into the
/// source's serialised string region (not sequential indices), so the
/// remap is keyed `HashMap<u32, StringId>` rather than `Vec`. `TypeId`
/// values include the `FIRST_USER` offset and may be sparse, again
/// HashMap-keyed. `FunctionId` and `ConstId` are sequential vec
/// indices (`functions.len()` / `constants.len()` allocator), so a
/// `Vec` works for them.
#[derive(Debug, Default)]
struct RemapTable {
    string: HashMap<u32, StringId>,
    type_: HashMap<u32, TypeId>,
    function: Vec<FunctionId>,
    constant: Vec<ConstId>,
    protocol: HashMap<u32, ProtocolId>,
    /// Flat index into `VbcModule.context_names`; the linker extends
    /// that table and remaps refs through this vec.
    context: Vec<ContextRef>,
}

impl RemapTable {
    fn map_string(&self, src: StringId) -> Result<StringId, LinkError> {
        // StringId::EMPTY is the sentinel for "no string"; identity-map.
        if src == StringId::EMPTY {
            return Ok(src);
        }
        self.string
            .get(&src.0)
            .copied()
            .ok_or(LinkError::DanglingReference {
                kind: "string",
                source_id: src.0,
            })
    }

    /// Lenient variant of [`map_string`]: when the source ID isn't in
    /// the remap, fall back to `StringId::EMPTY` rather than failing.
    /// Used for cross-module references where the string text lives in
    /// a sibling module not yet merged. The linker output preserves
    /// the reference but loses the string's text — debug printing
    /// shows `<extern>` instead of the original name. This is
    /// observably correct for code paths that don't dereference the
    /// string (most don't); a future Phase 4b post-pass can fix
    /// compile_core to emit self-contained per-module string tables
    /// and remove this fallback.
    fn map_string_lenient(&self, src: StringId) -> StringId {
        self.map_string(src).unwrap_or(StringId::EMPTY)
    }

    fn map_type_id(&self, src: TypeId) -> Result<TypeId, LinkError> {
        // Built-in IDs (TypeId::UNIT, BOOL, INT, FLOAT, …) are
        // identity-mapped — they're stable across every VBC module
        // by the Verum core type contract.
        if src.is_builtin() {
            return Ok(src);
        }
        // User-allocated IDs that appear in remap → use the remap.
        // IDs that *don't* appear in remap are cross-module references
        // to types declared in modules outside the current merge set
        // (typically: the kernel-side global type registry, or a
        // sibling cog merged in a later add_archive call). Identity-
        // map them; downstream lookups via `VbcModule::get_type` will
        // surface the missing descriptor as a typed runtime error
        // rather than a silent miscompile.
        Ok(self.type_.get(&src.0).copied().unwrap_or(src))
    }

    fn map_function(&self, src: FunctionId) -> Result<FunctionId, LinkError> {
        // u32::MAX sentinel + identity fallback for cross-archive
        // references (kernel intrinsics dispatched via the global
        // function table, runtime stubs registered outside the
        // current merge set). The runtime catches a truly-undefined
        // function call as a typed `FunctionNotFound` panic.
        if src.0 == u32::MAX {
            return Ok(src);
        }
        Ok(self
            .function
            .get(src.0 as usize)
            .copied()
            .filter(|fid| fid.0 != u32::MAX) // skip placeholder slots
            .unwrap_or(src))
    }

    fn map_const(&self, src: ConstId) -> Result<ConstId, LinkError> {
        Ok(self
            .constant
            .get(src.0 as usize)
            .copied()
            .filter(|cid| cid.0 != u32::MAX)
            .unwrap_or(src))
    }

    fn map_protocol(&self, src: ProtocolId) -> Result<ProtocolId, LinkError> {
        // Protocols live in a kernel-level global registry (verum_kernel
        // ProtocolId space); the linker treats unknown IDs as identity
        // rather than dangling, so a TypeDescriptor.protocols entry
        // referencing a kernel-protocol that the source module didn't
        // explicitly declare still merges cleanly.
        Ok(self.protocol.get(&src.0).copied().unwrap_or(src))
    }

    fn map_context(&self, src: ContextRef) -> Result<ContextRef, LinkError> {
        // ContextRef indexes into VbcModule.context_names. Cross-module
        // references identity-map: the linker output's context_names
        // table inherits every input's entries through Step 2c.5
        // append, so non-overlapping ContextRefs from different
        // modules will collide unless the source modules came from
        // the same compile_core run (which they do, today). Identity
        // fallback handles the cross-module case until Phase 4b emits
        // self-contained per-module context tables.
        Ok(self.context.get(src.0 as usize).copied().unwrap_or(src))
    }
}

/// Linker output assembler. See module-level docs for the API
/// contract.
pub struct VbcLinker {
    /// Active target's resolved cfg-key. Multi-variant function
    /// bodies pick the matching slot through this key.
    cfg: CfgKey,

    /// Linker output state — populated incrementally by `add_*`,
    /// finalised by `finalize`.
    out: VbcModule,

    /// Content-addressed string-pool dedup. Two modules interning
    /// `"hello"` collapse to one linker StringId.
    string_dedup: HashMap<String, StringId>,

    /// Bytecode region offset for the next function appended. Each
    /// per-source bytecode block is concatenated into `out.bytecode`;
    /// per-function offsets land here.
    bytecode_cursor: u32,
}

impl VbcLinker {
    /// Construct a fresh linker for `target_triple`. The cfg-key is
    /// resolved once and consulted on every multi-variant function
    /// addition.
    pub fn new(target_triple: &str) -> Self {
        let mut out = VbcModule::new("__linker_output".to_string());
        // Rebuild the dedup map's StringId(0) entry for the module
        // name so subsequent string lookups stay consistent.
        let mut string_dedup: HashMap<String, StringId> = HashMap::new();
        string_dedup.insert("__linker_output".to_string(), StringId(0));

        let cfg = {
            // Use a temporary closure to satisfy `for_triple`'s
            // intern signature; ID assignment for unknown OS / arch
            // / feature tokens routes through the linker's string
            // table so the resulting CfgKey is comparable to keys
            // stored in archives.
            let mut intern = |s: &str| {
                if let Some(id) = string_dedup.get(s) {
                    return *id;
                }
                let id = out.intern_string(s);
                string_dedup.insert(s.to_string(), id);
                id
            };
            CfgKey::for_triple(target_triple, &mut intern)
        };

        Self {
            cfg,
            out,
            string_dedup,
            bytecode_cursor: 0,
        }
    }

    /// Active target cfg-key. Public so tests / tools can introspect.
    pub fn target_cfg(&self) -> &CfgKey {
        &self.cfg
    }

    /// Number of modules merged so far.
    pub fn module_count(&self) -> usize {
        // The linker doesn't track this independently — derive from
        // the function table population. Approximate but useful for
        // diagnostics.
        self.out.functions.len()
    }

    /// Add an entire VBC archive. Each contained module is loaded,
    /// then merged through a single archive-wide remap pool: type /
    /// function / constant IDs that one module references but another
    /// declares (the pattern produced by `compile_core` global
    /// registration) resolve correctly within the same `add_archive`
    /// call.
    ///
    /// On `Err`, the linker is left in a partial state — the caller
    /// should drop it and start over with a fresh `VbcLinker::new`.
    pub fn add_archive(&mut self, archive: &VbcArchive) -> Result<usize, LinkError> {
        // Two-pass:
        //   Pass 1: load every archive module, populate one shared
        //           RemapTable with every (string|type|function|
        //           constant|context|protocol) ID across all modules.
        //   Pass 2: walk modules in archive order, apply the shared
        //           remap to descriptors / bodies, append to the
        //           linker output.
        //
        // This is what makes cross-module type references inside one
        // archive resolve cleanly: a TypeRef::Concrete(TypeId(512))
        // inside module A points at a TypeDescriptor whose id=512
        // lives in module B. After Pass 1, the shared remap has both
        // covered; Pass 2's `remap_type_ref` finds the linker-side
        // TypeId without falling into the dangling-reference branch.

        let module_count = archive.module_count();
        let mut modules: Vec<VbcModule> = Vec::with_capacity(module_count);
        for idx in 0..module_count {
            let entry = &archive.index[idx];
            let module = archive
                .load_module(&entry.name)
                .map_err(|_| LinkError::DanglingReference {
                    kind: "archive_module_load",
                    source_id: idx as u32,
                })?;
            modules.push(module);
        }

        // Pass 1: pre-allocate output IDs across the union, populate
        // one shared `RemapTable`.
        let mut shared = RemapTable::default();
        self.populate_archive_wide_remap(&modules, &mut shared)?;

        // Pass 2: append per-module descriptors + bytecode using the
        // shared remap.
        for module in modules {
            self.append_with_shared_remap(module, &shared)?;
        }

        Ok(module_count)
    }

    /// First pass of [`add_archive`]: walk every module in the
    /// archive, allocate output IDs in deterministic order
    /// (path-sorted by module name; per-module-table position within
    /// a single module), and populate `shared` with the full union
    /// of cross-module references.
    ///
    /// Functions and constants are also archive-wide pooled here, by
    /// `(module_index, source_id)` keys, so a `Call { func_id: 372 }`
    /// inside module A pointing at function 372 of module B resolves
    /// to the linker-side FunctionId allocated for B's slot 372.
    fn populate_archive_wide_remap(
        &mut self,
        modules: &[VbcModule],
        shared: &mut RemapTable,
    ) -> Result<(), LinkError> {
        // Strings, types, protocols: archive-wide.
        for src in modules {
            for (text, src_id) in src.strings.iter() {
                if shared.string.contains_key(&src_id.0) {
                    continue;
                }
                let linker_id = if let Some(existing) = self.string_dedup.get(text) {
                    *existing
                } else {
                    let new_id = self.out.intern_string(text);
                    self.string_dedup.insert(text.to_string(), new_id);
                    new_id
                };
                shared.string.insert(src_id.0, linker_id);
            }
            for src_desc in &src.types {
                if shared.type_.contains_key(&src_desc.id.0) {
                    continue;
                }
                let new_id = TypeId(self.next_user_type_id_value(shared));
                shared.type_.insert(src_desc.id.0, new_id);
            }
            for src_desc in &src.types {
                for proto_impl in &src_desc.protocols {
                    shared
                        .protocol
                        .entry(proto_impl.protocol.0)
                        .or_insert(proto_impl.protocol);
                }
            }
        }

        // Functions and constants are pooled differently. Archive-
        // wide bytecode references like `Call { func_id: 372 }`
        // inside module A point at function 372 *globally across the
        // archive*, not at module A's local table. So the pool maps
        // each source FunctionId / ConstId to a linker-allocated ID;
        // the same source ID coming from different modules of the
        // same archive collapse to one linker entry.
        let mut next_function = self.out.functions.len() as u32;
        let mut next_constant = self.out.constants.len() as u32;
        for src in modules {
            for desc in &src.functions {
                let src_id = desc.id.0;
                let needed_size = (src_id as usize).saturating_add(1);
                if shared.function.len() < needed_size {
                    shared
                        .function
                        .resize(needed_size, FunctionId(u32::MAX));
                }
                if shared.function[src_id as usize].0 == u32::MAX {
                    shared.function[src_id as usize] = FunctionId(next_function);
                    next_function = next_function.saturating_add(1);
                }
            }
            // Constants are allocated by sequential index within the
            // module — but since modules of the same archive may
            // share constants by index (they're emitted from the same
            // global pool), we treat them archive-wide too.
            for (idx, _) in src.constants.iter().enumerate() {
                let src_id = idx as u32;
                let needed_size = (src_id as usize).saturating_add(1);
                if shared.constant.len() < needed_size {
                    shared
                        .constant
                        .resize(needed_size, ConstId(u32::MAX));
                }
                if shared.constant[src_id as usize].0 == u32::MAX {
                    shared.constant[src_id as usize] = ConstId(next_constant);
                    next_constant = next_constant.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    /// Returns the next unused TypeId.0 value larger than every
    /// linker-side type already allocated and every type already
    /// reserved in the shared remap.
    fn next_user_type_id_value(&self, shared: &RemapTable) -> u32 {
        let from_out = self.out.types.len() as u32 + TypeId::FIRST_USER;
        let from_shared = shared
            .type_
            .values()
            .map(|t| t.0)
            .max()
            .map(|v| v + 1)
            .unwrap_or(TypeId::FIRST_USER);
        from_out.max(from_shared)
    }

    /// Second pass of [`add_archive`]: append a single module's
    /// descriptors / bodies through the pre-built shared remap.
    /// Per-module function/constant IDs are still allocated here so
    /// the linker output's sequential FunctionId space stays
    /// contiguous in module-emission order.
    fn append_with_shared_remap(
        &mut self,
        src: VbcModule,
        shared: &RemapTable,
    ) -> Result<(), LinkError> {
        // Archive-wide remap: strings, types, protocols, functions,
        // constants are all pre-populated. Per-module remap is just
        // a clone — no per-module ID allocation needed.
        let mut remap = RemapTable {
            string: shared.string.clone(),
            type_: shared.type_.clone(),
            protocol: shared.protocol.clone(),
            function: shared.function.clone(),
            constant: shared.constant.clone(),
            ..RemapTable::default()
        };

        // Context remap: extend output's context_names, build a
        // remap.context vec.
        let context_base = self.out.context_names.len() as u32;
        for &sid in &src.context_names {
            self.out.context_names.push(remap.map_string_lenient(sid));
        }
        remap.context = (0..src.context_names.len() as u32)
            .map(|i| ContextRef(context_base + i))
            .collect();

        // Append type descriptors — only the FIRST sighting across
        // the archive becomes the canonical descriptor in linker
        // output. Subsequent module sightings of the same source
        // TypeId are duplicates and must be skipped to keep the
        // output type table monotone-allocated.
        for src_desc in &src.types {
            // Only emit the descriptor when its (just-shared-remapped)
            // linker-side id sits at the next sequential slot in
            // `out.types`. Otherwise the type was already emitted by
            // an earlier module sharing the same source TypeId.
            let mapped_id = remap.map_type_id(src_desc.id)?;
            let expected_idx = self.out.types.len();
            if mapped_id.0 as usize >= expected_idx + TypeId::FIRST_USER as usize {
                let mut new_desc = self.remap_type_descriptor(src_desc, &remap)?;
                new_desc.id = mapped_id;
                // Pad with default descriptors if remap allocated a
                // sparse id (shouldn't happen in v1 but guards the
                // invariant `out.types[i].id == FIRST_USER + i`).
                while self.out.types.len() < (mapped_id.0 - TypeId::FIRST_USER) as usize {
                    self.out.types.push(TypeDescriptor::default());
                }
                self.out.types.push(new_desc);
            }
        }

        // Constants, functions, specializations, multi-variant cfg
        // tables, source-map, ffi, ctors/dtors — same as the original
        // single-module path. We delegate to a shared helper to avoid
        // drift.
        self.append_module_bodies(&src, &remap)
    }

    /// Body-append helper shared between the archive path
    /// ([`append_with_shared_remap`]) and the user-module path
    /// ([`add_user_module`]). Handles constants, functions, source-
    /// maps, specializations, cfg-keys, multi-variants, ctors/dtors,
    /// and bytecode. The caller has already populated `remap` and
    /// extended `out.types` / `out.context_names`.
    fn append_module_bodies(
        &mut self,
        src: &VbcModule,
        remap: &RemapTable,
    ) -> Result<(), LinkError> {
        // Constants — archive-wide pool. Skip already-appended
        // constants (the SAME source-ConstId appearing in multiple
        // modules of the archive).
        for (idx, src_const) in src.constants.iter().enumerate() {
            let linker_id = remap.map_const(ConstId(idx as u32))?;
            if (linker_id.0 as usize) < self.out.constants.len() {
                continue; // already appended by an earlier module
            }
            // Pad with placeholder constants if the linker ID is
            // ahead of the table (sparse allocation guard).
            while self.out.constants.len() < linker_id.0 as usize {
                self.out.constants.push(Constant::Int(0));
            }
            let mapped = self.remap_constant(src_const, remap)?;
            self.out.constants.push(mapped);
        }

        // Functions — per-function bytecode rewrite, archive-wide ID
        // pool. Same dedup-skip pattern as constants. Track per-func
        // offsets via map keyed on source-function-id so the
        // specialisation / variant fix-up can find them without
        // assuming module-local indexing.
        let mut per_func_offsets: HashMap<u32, (u32, u32)> = HashMap::new();
        for src_desc in &src.functions {
            let linker_id = remap.map_function(src_desc.id)?;
            if (linker_id.0 as usize) < self.out.functions.len() {
                // Already appended by an earlier module — but record
                // its offset/length for spec/variant fix-up.
                let existing = &self.out.functions[linker_id.0 as usize];
                per_func_offsets
                    .insert(src_desc.id.0, (existing.bytecode_offset, existing.bytecode_length));
                continue;
            }
            // Pad with placeholder descriptors if the linker ID is
            // ahead of the table.
            while self.out.functions.len() < linker_id.0 as usize {
                self.out.functions.push(FunctionDescriptor::default());
            }

            let src_start = src_desc.bytecode_offset as usize;
            let src_end = src_start.saturating_add(src_desc.bytecode_length as usize);
            let region: &[u8] = if src_start < src.bytecode.len() && src_end <= src.bytecode.len() {
                &src.bytecode[src_start..src_end]
            } else {
                &[]
            };
            let rewritten = self.rewrite_function_bytecode(region, remap)?;
            let linker_offset = self.out.bytecode.len() as u32;
            let linker_length = rewritten.len() as u32;
            self.out.bytecode.extend_from_slice(&rewritten);
            per_func_offsets.insert(src_desc.id.0, (linker_offset, linker_length));

            let mut new_desc = self.remap_function_descriptor(src_desc, remap, &src.bytecode)?;
            new_desc.bytecode_offset = linker_offset;
            new_desc.bytecode_length = linker_length;
            self.out.functions.push(new_desc);
        }

        // Specializations — bytecode_offset shifts the same way as
        // function descriptors. We don't re-decode/re-rewrite the
        // specialisation bytecode separately because per-spec bodies
        // live inside one of the input function regions which we
        // already rewrote above; instead we approximate by sliding
        // the offset by the cumulative cursor difference. The
        // specialisation table is observability-only at runtime, so
        // a slightly-off offset doesn't break dispatch — but full
        // correctness lands when Phase 6c-2 emits per-spec rewrites.
        for spec in &src.specializations {
            let remapped_fn = remap.map_function(spec.generic_fn)?;
            let mut remapped_args = Vec::with_capacity(spec.type_args.len());
            for arg in &spec.type_args {
                remapped_args.push(self.remap_type_ref(arg, remap)?);
            }
            // Best-effort: reuse the corresponding function's linker
            // offset when the spec offset matches.
            let mut new_offset = spec.bytecode_offset;
            for src_desc in &src.functions {
                if spec.bytecode_offset >= src_desc.bytecode_offset
                    && spec.bytecode_offset
                        < src_desc.bytecode_offset + src_desc.bytecode_length
                {
                    if let Some(&(linker_off, _)) = per_func_offsets.get(&src_desc.id.0) {
                        new_offset =
                            linker_off + (spec.bytecode_offset - src_desc.bytecode_offset);
                    }
                    break;
                }
            }
            self.out.specializations.push(SpecializationEntry {
                generic_fn: remapped_fn,
                type_args: remapped_args,
                hash: spec.hash,
                bytecode_offset: new_offset,
                bytecode_length: spec.bytecode_length,
                register_count: spec.register_count,
            });
        }

        // cfg_keys — append, remap StringId-bearing fields.
        let cfg_key_base = self.out.cfg_keys.len() as u16;
        for src_key in &src.cfg_keys {
            let mut remapped = src_key.clone();
            remapped.os = match remapped.os {
                Some(crate::cfg_key::TargetOs::Other(sid)) => {
                    Some(crate::cfg_key::TargetOs::Other(remap.map_string_lenient(sid)))
                }
                other => other,
            };
            remapped.arch = match remapped.arch {
                Some(crate::cfg_key::TargetArch::Other(sid)) => {
                    Some(crate::cfg_key::TargetArch::Other(remap.map_string_lenient(sid)))
                }
                other => other,
            };
            for f in remapped.features.iter_mut() {
                *f = remap.map_string_lenient(*f);
            }
            self.out.cfg_keys.push(remapped);
        }

        // function_variants — same per-function offset shift as the
        // primary function descriptors.
        for set in &src.function_variants {
            let new_function_id = remap.map_function(set.function_id)?;
            let mut new_variants = smallvec::SmallVec::<[VbcVariant; 4]>::new();
            for v in set.variants.iter() {
                // Shift via the source-function table when the variant
                // body sits inside one of the input functions; default
                // to the source offset when no match.
                let mut new_offset = v.bytecode_offset;
                for src_desc in &src.functions {
                    if v.bytecode_offset >= src_desc.bytecode_offset
                        && v.bytecode_offset
                            < src_desc.bytecode_offset + src_desc.bytecode_length
                    {
                        if let Some(&(linker_off, _)) = per_func_offsets.get(&src_desc.id.0) {
                            new_offset =
                                linker_off + (v.bytecode_offset - src_desc.bytecode_offset);
                        }
                        break;
                    }
                }
                new_variants.push(VbcVariant {
                    cfg_key_id: cfg_key_base + v.cfg_key_id,
                    flags: v.flags,
                    bytecode_offset: new_offset,
                    bytecode_length: v.bytecode_length,
                });
            }
            self.out.function_variants.push(FunctionVariantSet {
                function_id: new_function_id,
                variants: new_variants,
            });
        }

        // FFI passthrough.
        self.out.ffi_libraries.extend(src.ffi_libraries.iter().cloned());
        self.out.ffi_symbols.extend(src.ffi_symbols.iter().cloned());
        self.out.ffi_layouts.extend(src.ffi_layouts.iter().cloned());

        // Global ctors / dtors.
        for (fid, prio) in &src.global_ctors {
            self.out.global_ctors.push((remap.map_function(*fid)?, *prio));
        }
        for (fid, prio) in &src.global_dtors {
            self.out.global_dtors.push((remap.map_function(*fid)?, *prio));
        }

        Ok(())
    }

    /// Add a single user-side VBC module — typically the just-codegen'd
    /// output of `compile_ast_to_vbc` for the user's own source files.
    /// Internally builds a single-module remap pool and delegates to
    /// the shared body-append helper.
    pub fn add_user_module(&mut self, module: VbcModule) -> Result<(), LinkError> {
        let mut shared = RemapTable::default();
        self.populate_archive_wide_remap(std::slice::from_ref(&module), &mut shared)?;
        self.append_with_shared_remap(module, &shared)?;
        Ok(())
    }

    /// Consume the linker, return the merged output module ready for
    /// the interpreter or AOT lowering.
    pub fn finalize(self) -> VbcModule {
        self.out
    }


    // ========================================================================
    // Internal: descriptor rewriters
    // ========================================================================

    fn remap_type_descriptor(
        &self,
        src: &TypeDescriptor,
        remap: &RemapTable,
    ) -> Result<TypeDescriptor, LinkError> {
        let mut out = src.clone();
        out.id = remap.map_type_id(src.id)?;
        out.name = remap.map_string_lenient(src.name);
        // type_params: each carries name (StringId), id (TypeParamId
        // — function-local, not remapped), bounds (ProtocolId), default
        // (TypeRef).
        for param in out.type_params.iter_mut() {
            param.name = remap.map_string_lenient(param.name);
            for bound in param.bounds.iter_mut() {
                *bound = remap.map_protocol(*bound)?;
            }
            if let Some(def) = &param.default {
                param.default = Some(self.remap_type_ref(def, remap)?);
            }
        }
        for field in out.fields.iter_mut() {
            field.name = remap.map_string_lenient(field.name);
            field.type_ref = self.remap_type_ref(&field.type_ref, remap)?;
        }
        for variant in out.variants.iter_mut() {
            variant.name = remap.map_string_lenient(variant.name);
            for vf in variant.fields.iter_mut() {
                vf.name = remap.map_string_lenient(vf.name);
                vf.type_ref = self.remap_type_ref(&vf.type_ref, remap)?;
            }
        }
        for proto_impl in out.protocols.iter_mut() {
            proto_impl.protocol = remap.map_protocol(proto_impl.protocol)?;
            // methods: stored as Vec<u32> of FunctionId.0 values.
            for mid in proto_impl.methods.iter_mut() {
                *mid = remap.map_function(FunctionId(*mid))?.0;
            }
        }
        // drop_fn / clone_fn: stored as Option<u32> (FunctionId.0).
        if let Some(f) = out.drop_fn {
            out.drop_fn = Some(remap.map_function(FunctionId(f))?.0);
        }
        if let Some(f) = out.clone_fn {
            out.clone_fn = Some(remap.map_function(FunctionId(f))?.0);
        }
        Ok(out)
    }

    fn remap_constant(&self, src: &Constant, remap: &RemapTable) -> Result<Constant, LinkError> {
        Ok(match src {
            Constant::Int(v) => Constant::Int(*v),
            Constant::Float(v) => Constant::Float(*v),
            Constant::String(sid) => Constant::String(remap.map_string_lenient(*sid)),
            Constant::Type(tref) => Constant::Type(self.remap_type_ref(tref, remap)?),
            Constant::Function(fid) => Constant::Function(remap.map_function(*fid)?),
            Constant::Protocol(pid) => Constant::Protocol(remap.map_protocol(*pid)?),
            Constant::Array(ids) => {
                let mut out = Vec::with_capacity(ids.len());
                for cid in ids {
                    out.push(remap.map_const(*cid)?);
                }
                Constant::Array(out)
            }
            Constant::Bytes(b) => Constant::Bytes(b.clone()),
        })
    }

    fn remap_type_ref(&self, src: &TypeRef, remap: &RemapTable) -> Result<TypeRef, LinkError> {
        Ok(match src {
            TypeRef::Concrete(tid) => TypeRef::Concrete(remap.map_type_id(*tid)?),
            TypeRef::Generic(pid) => TypeRef::Generic(*pid),
            TypeRef::Instantiated { base, args } => {
                let mut out_args = Vec::with_capacity(args.len());
                for a in args {
                    out_args.push(self.remap_type_ref(a, remap)?);
                }
                TypeRef::Instantiated {
                    base: remap.map_type_id(*base)?,
                    args: out_args,
                }
            }
            TypeRef::Function {
                params,
                return_type,
                contexts,
            } => {
                let mut out_params = Vec::with_capacity(params.len());
                for p in params {
                    out_params.push(self.remap_type_ref(p, remap)?);
                }
                let out_ret = Box::new(self.remap_type_ref(return_type, remap)?);
                let mut out_ctx = smallvec::SmallVec::<[ContextRef; 2]>::new();
                for c in contexts {
                    out_ctx.push(remap.map_context(*c)?);
                }
                TypeRef::Function {
                    params: out_params,
                    return_type: out_ret,
                    contexts: out_ctx,
                }
            }
            TypeRef::Rank2Function {
                type_param_count,
                params,
                return_type,
                contexts,
            } => {
                let mut out_params = Vec::with_capacity(params.len());
                for p in params {
                    out_params.push(self.remap_type_ref(p, remap)?);
                }
                let out_ret = Box::new(self.remap_type_ref(return_type, remap)?);
                let mut out_ctx = smallvec::SmallVec::<[ContextRef; 2]>::new();
                for c in contexts {
                    out_ctx.push(remap.map_context(*c)?);
                }
                TypeRef::Rank2Function {
                    type_param_count: *type_param_count,
                    params: out_params,
                    return_type: out_ret,
                    contexts: out_ctx,
                }
            }
            TypeRef::Reference {
                inner,
                mutability,
                tier,
            } => TypeRef::Reference {
                inner: Box::new(self.remap_type_ref(inner, remap)?),
                mutability: *mutability,
                tier: *tier,
            },
            TypeRef::Tuple(elems) => {
                let mut out_elems = Vec::with_capacity(elems.len());
                for e in elems {
                    out_elems.push(self.remap_type_ref(e, remap)?);
                }
                TypeRef::Tuple(out_elems)
            }
            TypeRef::Array { element, length } => TypeRef::Array {
                element: Box::new(self.remap_type_ref(element, remap)?),
                length: *length,
            },
            TypeRef::Slice(inner) => {
                TypeRef::Slice(Box::new(self.remap_type_ref(inner, remap)?))
            }
        })
    }

    fn remap_function_descriptor(
        &self,
        src: &FunctionDescriptor,
        remap: &RemapTable,
        _src_bytecode: &[u8],
    ) -> Result<FunctionDescriptor, LinkError> {
        let mut out = src.clone();
        out.id = remap.map_function(src.id)?;
        out.name = remap.map_string_lenient(src.name);
        if let Some(pt) = src.parent_type {
            out.parent_type = Some(remap.map_type_id(pt)?);
        }
        for tp in out.type_params.iter_mut() {
            tp.name = remap.map_string_lenient(tp.name);
            for b in tp.bounds.iter_mut() {
                *b = remap.map_protocol(*b)?;
            }
            if let Some(d) = &tp.default {
                tp.default = Some(self.remap_type_ref(d, remap)?);
            }
        }
        for p in out.params.iter_mut() {
            p.name = remap.map_string_lenient(p.name);
            p.type_ref = self.remap_type_ref(&p.type_ref, remap)?;
        }
        out.return_type = self.remap_type_ref(&src.return_type, remap)?;
        for ctx in out.contexts.iter_mut() {
            *ctx = remap.map_context(*ctx)?;
        }
        if let Some(yt) = &src.yield_type {
            out.yield_type = Some(self.remap_type_ref(yt, remap)?);
        }
        // bytecode_offset / bytecode_length are placeholders here —
        // the per-function rewriter in `append_module_bodies`
        // overwrites them with the linker-side values after
        // re-encoding the rewritten body.
        Ok(out)
    }

    // ========================================================================
    // Internal: bytecode rewriter (the load-bearing piece)
    // ========================================================================

    /// Decode the function's bytecode region, rewrite every
    /// id-bearing instruction operand through `remap`, re-encode.
    ///
    /// Returns the rewritten byte buffer. The output length may differ
    /// from the input — VBC encodes IDs as varints, so a 1-byte
    /// source ID can grow to 2-3 bytes after remap (and vice versa).
    /// Caller must update `FunctionDescriptor.bytecode_offset` /
    /// `bytecode_length` to point at the linker-side location.
    ///
    /// Sixteen id-bearing instruction variants are handled here, the
    /// canonical list per `crates/verum_vbc/src/instruction.rs`:
    /// `LoadK / MetaQuote` (const_id), `Call / CallG / TailCall /
    /// CallM / NewClosure / Spawn / GenCreate` (func_id / method_id),
    /// `New / NewG / MetaReflect / MakeVariantTyped / MakePi`
    /// (type_id / return_type_id), `BinaryG / CmpG` (protocol_id).
    /// Every other variant (~200 of them) carries no module-local ID
    /// and passes through untouched.
    fn rewrite_function_bytecode(
        &self,
        src_bytes: &[u8],
        remap: &RemapTable,
    ) -> Result<Vec<u8>, LinkError> {
        // Per-instruction decode loop so that a `TruncatedBytecode`
        // reports the actual fault offset rather than the entire
        // function length.  The previous wrapper around
        // `decode_instructions` discarded the offset, making decoder
        // gaps practically untraceable through the linker boundary.
        let mut instructions = Vec::new();
        let mut offset = 0;
        while offset < src_bytes.len() {
            let start = offset;
            match decode_instruction(src_bytes, &mut offset) {
                Ok(instr) => instructions.push(instr),
                Err(_) => {
                    if std::env::var("VBC_LINKER_TRACE").is_ok() {
                        let opcode = src_bytes.get(start).copied().unwrap_or(0xFF);
                        let dump: String = src_bytes
                            .iter()
                            .enumerate()
                            .map(|(i, b)| {
                                if i == start {
                                    format!("[{:02x}]", b)
                                } else {
                                    format!("{:02x}", b)
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        eprintln!(
                            "[linker-trace] decode failure at offset={} opcode=0x{:02X}: {}",
                            start, opcode, dump
                        );
                    }
                    return Err(LinkError::TruncatedBytecode {
                        offset: start,
                        want: src_bytes.len(),
                    });
                }
            }
        }
        for instr in instructions.iter_mut() {
            rewrite_instruction_ids(instr, remap)?;
        }
        let mut out = Vec::with_capacity(src_bytes.len());
        encode_instructions(&instructions, &mut out);
        Ok(out)
    }
}

/// In-place rewrite of any id-bearing operand in `instr`. No-op on
/// id-free variants. Called per-instruction by
/// [`VbcLinker::rewrite_function_bytecode`].
///
/// **Maintenance contract**: every new id-bearing instruction variant
/// added to `instruction.rs` MUST be added here. Missing a variant
/// surfaces as a runtime `FunctionNotFound` / `TypeNotFound` panic
/// when user code transitively calls into the unrewritten body.
fn rewrite_instruction_ids(
    instr: &mut Instruction,
    remap: &RemapTable,
) -> Result<(), LinkError> {
    match instr {
        // --- Constant pool index ---
        Instruction::LoadK { const_id, .. } => {
            *const_id = remap.map_const(ConstId(*const_id))?.0;
        }
        Instruction::MetaQuote { bytes_const_id, .. } => {
            *bytes_const_id = remap.map_const(ConstId(*bytes_const_id))?.0;
        }
        // --- Function table index ---
        Instruction::Call { func_id, .. }
        | Instruction::CallG { func_id, .. }
        | Instruction::TailCall { func_id, .. }
        | Instruction::NewClosure { func_id, .. }
        | Instruction::Spawn { func_id, .. }
        | Instruction::GenCreate { func_id, .. } => {
            *func_id = remap.map_function(FunctionId(*func_id))?.0;
        }
        Instruction::CallM { method_id, .. } => {
            // Method table is a slice of the function table; method_id
            // is a flat function-id under the hood.
            *method_id = remap.map_function(FunctionId(*method_id))?.0;
        }
        // --- Type table index ---
        Instruction::New { type_id, .. }
        | Instruction::NewG { type_id, .. }
        | Instruction::MetaReflect { type_id, .. }
        | Instruction::MakeVariantTyped { type_id, .. } => {
            *type_id = remap.map_type_id(TypeId(*type_id))?.0;
        }
        Instruction::MakePi { return_type_id, .. } => {
            *return_type_id = remap.map_type_id(TypeId(*return_type_id))?.0;
        }
        // --- Protocol table index ---
        Instruction::BinaryG { protocol_id, .. }
        | Instruction::CmpG { protocol_id, .. } => {
            *protocol_id = remap.map_protocol(ProtocolId(*protocol_id))?.0;
        }
        // Everything else has no id operand.
        _ => {}
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_linker_finalizes() {
        let linker = VbcLinker::new("aarch64-apple-darwin");
        let m = linker.finalize();
        assert_eq!(m.functions.len(), 0);
        assert_eq!(m.types.len(), 0);
        assert_eq!(m.constants.len(), 0);
    }

    #[test]
    fn target_cfg_resolves() {
        let linker = VbcLinker::new("x86_64-unknown-linux-gnu");
        let cfg = linker.target_cfg();
        assert_eq!(cfg.os, Some(crate::cfg_key::TargetOs::Linux));
        assert_eq!(cfg.arch, Some(crate::cfg_key::TargetArch::X86_64));
    }

    /// Phase 7 of the precompiled-stdlib epic: cross-compile via
    /// archive variant pick. Confirms that every triple in the
    /// supported matrix resolves to a stable, well-formed
    /// `CfgKey` via `VbcLinker::new`. Matrix:
    ///   darwin / linux / windows × x86_64 / aarch64
    ///   plus riscv64 / wasm32 / bpfel single-OS combos
    /// One archive serves every triple via `CfgKey::matches`.
    #[test]
    fn cross_compile_matrix_resolves() {
        use crate::cfg_key::{Endian, PtrWidth, TargetArch, TargetOs};

        // (triple, expected_os, expected_arch, expected_ptr_width, expected_endian)
        let matrix: &[(&str, TargetOs, TargetArch, PtrWidth, Endian)] = &[
            // darwin × { x86_64, aarch64 }
            (
                "x86_64-apple-darwin",
                TargetOs::Darwin,
                TargetArch::X86_64,
                PtrWidth::Bits64,
                Endian::Little,
            ),
            (
                "aarch64-apple-darwin",
                TargetOs::Darwin,
                TargetArch::Aarch64,
                PtrWidth::Bits64,
                Endian::Little,
            ),
            // linux × { x86_64, aarch64 }
            (
                "x86_64-unknown-linux-gnu",
                TargetOs::Linux,
                TargetArch::X86_64,
                PtrWidth::Bits64,
                Endian::Little,
            ),
            (
                "aarch64-unknown-linux-gnu",
                TargetOs::Linux,
                TargetArch::Aarch64,
                PtrWidth::Bits64,
                Endian::Little,
            ),
            // windows × { x86_64, aarch64 }
            (
                "x86_64-pc-windows-msvc",
                TargetOs::Windows,
                TargetArch::X86_64,
                PtrWidth::Bits64,
                Endian::Little,
            ),
            (
                "aarch64-pc-windows-msvc",
                TargetOs::Windows,
                TargetArch::Aarch64,
                PtrWidth::Bits64,
                Endian::Little,
            ),
            // exotic ISAs — single canonical triple each
            (
                "riscv64gc-unknown-linux-gnu",
                TargetOs::Linux,
                TargetArch::Riscv64,
                PtrWidth::Bits64,
                Endian::Little,
            ),
            (
                "wasm32-unknown-unknown",
                TargetOs::None,
                TargetArch::Wasm32,
                PtrWidth::Bits32,
                Endian::Little,
            ),
            (
                "bpfel-unknown-none",
                TargetOs::None,
                TargetArch::Bpfel,
                PtrWidth::Bits32,
                Endian::Little,
            ),
        ];

        for (triple, want_os, want_arch, want_ptr, want_endian) in matrix {
            let linker = VbcLinker::new(triple);
            let cfg = linker.target_cfg();
            assert_eq!(cfg.os, Some(*want_os), "os mismatch for {}", triple);
            assert_eq!(cfg.arch, Some(*want_arch), "arch mismatch for {}", triple);
            assert_eq!(
                cfg.ptr_width,
                Some(*want_ptr),
                "ptr_width mismatch for {}",
                triple
            );
            assert_eq!(
                cfg.endian,
                Some(*want_endian),
                "endian mismatch for {}",
                triple
            );
        }
    }

    /// Phase 7: a "darwin-only" stored variant must NOT match a
    /// linux active triple (cross-compile target mismatch
    /// rejection). This is the cfg_key invariant that lets one
    /// archive carry multiple per-OS variants without ambiguity.
    #[test]
    fn cross_compile_variant_rejection() {
        use crate::cfg_key::{CfgKey, TargetOs};

        let darwin_linker = VbcLinker::new("aarch64-apple-darwin");
        let linux_linker = VbcLinker::new("x86_64-unknown-linux-gnu");

        // Stored constraint: "this variant is darwin-only"
        let darwin_only = CfgKey {
            os: Some(TargetOs::Darwin),
            ..CfgKey::default()
        };

        assert!(
            darwin_only.matches(darwin_linker.target_cfg()),
            "darwin-only variant should match darwin active triple"
        );
        assert!(
            !darwin_only.matches(linux_linker.target_cfg()),
            "darwin-only variant must NOT match linux active triple"
        );
    }

    #[test]
    fn add_empty_user_module_succeeds() {
        let mut linker = VbcLinker::new("aarch64-apple-darwin");
        let user = VbcModule::new("user".to_string());
        linker.add_user_module(user).expect("add_user_module");
        let _ = linker.finalize();
    }

    #[test]
    fn string_dedup_collapses_identical_interns() {
        let mut linker = VbcLinker::new("aarch64-apple-darwin");

        // Two source modules each interning "hello" should produce
        // exactly one "hello" entry in the linker output.
        let mut a = VbcModule::new("a".to_string());
        let _ = a.intern_string("hello");
        let _ = a.intern_string("world");

        let mut b = VbcModule::new("b".to_string());
        let _ = b.intern_string("hello");
        let _ = b.intern_string("verum");

        linker.add_user_module(a).expect("add a");
        linker.add_user_module(b).expect("add b");

        let m = linker.finalize();
        // StringId values are byte offsets, not sequential indices —
        // iterate the table via `iter()` to enumerate every string.
        let strings: Vec<String> = m
            .strings
            .iter()
            .map(|(s, _)| s.to_string())
            .collect();
        assert!(strings.contains(&"hello".to_string()));
        assert!(strings.contains(&"world".to_string()));
        assert!(strings.contains(&"verum".to_string()));
        // hello appears exactly once even though both modules interned it.
        let hellos = strings.iter().filter(|s| s.as_str() == "hello").count();
        assert_eq!(hellos, 1, "string-pool dedup violated");
    }

    #[test]
    fn bytecode_rewriter_remaps_call_func_id() {
        use crate::bytecode::{decode_instructions, encode_instructions};
        use crate::instruction::Instruction;
        use crate::instruction::{Reg, RegRange};

        // Synthesise a Call { func_id: 5 } at the source level, encode
        // it, hand the bytes to the rewriter with a remap that says
        // "5 → 99", verify the decoded output instruction reads
        // func_id: 99.
        let source_call = Instruction::Call {
            dst: Reg(0),
            func_id: 5,
            args: RegRange { start: Reg(1), count: 0 },
        };
        let mut src_bytes = Vec::new();
        encode_instructions(&[source_call.clone()], &mut src_bytes);

        // Construct a remap that maps source FunctionId(5) →
        // linker FunctionId(99).
        let mut remap = RemapTable::default();
        // Pad function vec so index 5 holds 99.
        remap.function = vec![FunctionId(u32::MAX); 6];
        remap.function[5] = FunctionId(99);

        let linker = VbcLinker::new("aarch64-apple-darwin");
        let rewritten = linker
            .rewrite_function_bytecode(&src_bytes, &remap)
            .expect("rewrite");

        let decoded = decode_instructions(&rewritten).expect("decode");
        assert_eq!(decoded.len(), 1);
        match &decoded[0] {
            Instruction::Call { func_id, .. } => {
                assert_eq!(*func_id, 99, "rewriter did not remap func_id");
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn bytecode_rewriter_remaps_loadk_const_id() {
        use crate::bytecode::{decode_instructions, encode_instructions};
        use crate::instruction::Instruction;
        use crate::instruction::Reg;

        let src = Instruction::LoadK { dst: Reg(0), const_id: 42 };
        let mut bytes = Vec::new();
        encode_instructions(&[src], &mut bytes);

        let mut remap = RemapTable::default();
        remap.constant = vec![ConstId(u32::MAX); 43];
        remap.constant[42] = ConstId(7);

        let linker = VbcLinker::new("aarch64-apple-darwin");
        let rewritten = linker.rewrite_function_bytecode(&bytes, &remap).expect("rewrite");
        let decoded = decode_instructions(&rewritten).expect("decode");
        assert_eq!(decoded.len(), 1);
        match &decoded[0] {
            Instruction::LoadK { const_id, .. } => {
                assert_eq!(*const_id, 7, "const_id rewrite failed");
            }
            other => panic!("expected LoadK, got {other:?}"),
        }
    }

    #[test]
    fn bytecode_rewriter_remaps_new_type_id() {
        use crate::bytecode::{decode_instructions, encode_instructions};
        use crate::instruction::Instruction;
        use crate::instruction::Reg;

        let src = Instruction::New {
            dst: Reg(0),
            type_id: 200,
            field_count: 4,
        };
        let mut bytes = Vec::new();
        encode_instructions(&[src], &mut bytes);

        let mut remap = RemapTable::default();
        remap.type_.insert(200, TypeId(456));

        let linker = VbcLinker::new("aarch64-apple-darwin");
        let rewritten = linker.rewrite_function_bytecode(&bytes, &remap).expect("rewrite");
        let decoded = decode_instructions(&rewritten).expect("decode");
        assert_eq!(decoded.len(), 1);
        match &decoded[0] {
            Instruction::New { type_id, .. } => {
                assert_eq!(*type_id, 456, "type_id rewrite failed");
            }
            other => panic!("expected New, got {other:?}"),
        }
    }

    #[test]
    fn function_id_allocation_is_monotone() {
        let mut linker = VbcLinker::new("aarch64-apple-darwin");

        let mut a = VbcModule::new("a".to_string());
        let mut b = VbcModule::new("b".to_string());

        // Synthesise two functions each.
        let mk_fn = |id: u32, name_id: StringId| FunctionDescriptor {
            id: FunctionId(id),
            name: name_id,
            parent_type: None,
            type_params: smallvec::SmallVec::new(),
            params: smallvec::SmallVec::new(),
            return_type: TypeRef::Concrete(TypeId::UNIT),
            contexts: smallvec::SmallVec::new(),
            properties: crate::types::PropertySet::default(),
            bytecode_offset: 0,
            bytecode_length: 0,
            locals_count: 0,
            register_count: 0,
            max_stack: 0,
            is_inline_candidate: false,
            is_generic: false,
            visibility: crate::types::Visibility::Public,
            is_generator: false,
            yield_type: None,
            suspend_point_count: 0,
            calling_convention: Default::default(),
            optimization_hints: Default::default(),
            instructions: None,
            func_id_base: 0,
            debug_variables: Vec::new(),
            is_test: false,
            is_gpu_only: false,
        };
        let n0 = a.intern_string("alpha");
        let n1 = a.intern_string("beta");
        a.functions.push(mk_fn(0, n0));
        a.functions.push(mk_fn(1, n1));

        let n2 = b.intern_string("gamma");
        let n3 = b.intern_string("delta");
        b.functions.push(mk_fn(0, n2));
        b.functions.push(mk_fn(1, n3));

        linker.add_user_module(a).expect("add a");
        linker.add_user_module(b).expect("add b");

        let m = linker.finalize();
        assert_eq!(m.functions.len(), 4, "all four functions present");
        // Each function's `id` field matches its index in the table.
        for (i, f) in m.functions.iter().enumerate() {
            assert_eq!(
                f.id.0, i as u32,
                "function {} has non-monotone id {}",
                i, f.id.0
            );
        }
    }
}
