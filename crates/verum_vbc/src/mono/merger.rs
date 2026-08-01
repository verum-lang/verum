//! Module merging for monomorphization.
//!
//! The ModuleMerger combines the user module VBC with newly
//! monomorphized functions into a final module ready for AOT lowering.
//!
//! Key responsibilities:
//! 1. Copy user module structure (types, strings, constants)
//! 2. Copy user functions (bytecode + decoded instructions)
//! 3. Add newly specialized functions (instructions decoded eagerly)
//! 4. **Rewrite id references on the DECODED instruction streams via
//!    the ONE remap authority** (`bytecode_remap::rewrite_instruction_ids`)
//! 5. Re-encode the bytecode blob from the final instruction streams
//!
//! The stdlib raw-copy leg was REMOVED (T0277): it copied archive bytes
//! with no string/type/const/function id remap and its mapping arm had
//! zero callers — archived specializations must enter through the
//! archive-merge authority (T0313).
//!
//! Final phase of monomorphization: produces a self-contained VBC module with all
//! generic instantiations resolved to concrete specialized functions.

use std::collections::HashMap;
use std::sync::Arc;

use crate::module::{FunctionDescriptor, FunctionId, SpecializationEntry, VbcModule};
use crate::types::{TypeId, TypeRef};

use super::graph::InstantiationRequest;
use super::resolver::{MonomorphizationResolver, ResolvedSpecialization};
use super::specializer::SpecializedFunction;

// ============================================================================
// Merge Error
// ============================================================================

/// Error during module merging.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum MergeError {
    /// Function not found in source module.
    FunctionNotFound {
        module: String,
        function_id: FunctionId,
    },
    /// Type not found in source module.
    TypeNotFound { module: String, type_id: TypeId },
    /// Bytecode range invalid.
    InvalidBytecodeRange {
        offset: u32,
        length: u32,
        module_size: usize,
    },
    /// String table conflict.
    StringTableConflict(String),
    /// Specialization missing.
    SpecializationMissing {
        function_id: FunctionId,
        type_args: Vec<TypeRef>,
    },
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::FunctionNotFound {
                module,
                function_id,
            } => {
                write!(
                    f,
                    "Function {:?} not found in module {}",
                    function_id, module
                )
            }
            MergeError::TypeNotFound { module, type_id } => {
                write!(f, "Type {:?} not found in module {}", type_id, module)
            }
            MergeError::InvalidBytecodeRange {
                offset,
                length,
                module_size,
            } => {
                write!(
                    f,
                    "Invalid bytecode range {}..{} in module of size {}",
                    offset,
                    offset + length,
                    module_size
                )
            }
            MergeError::StringTableConflict(msg) => {
                write!(f, "String table conflict: {}", msg)
            }
            MergeError::SpecializationMissing {
                function_id,
                type_args,
            } => {
                write!(
                    f,
                    "Specialization missing for {:?} with {:?}",
                    function_id, type_args
                )
            }
        }
    }
}

impl std::error::Error for MergeError {}

// ============================================================================
// Merge Statistics
// ============================================================================

/// Statistics from module merging.
#[derive(Debug, Clone, Default)]
pub struct MergeStats {
    /// Number of user functions copied.
    pub user_functions: usize,
    /// Number of stdlib specializations linked.
    pub stdlib_specializations: usize,
    /// Number of newly specialized functions added.
    pub new_specializations: usize,
    /// Total bytecode size before merge.
    pub bytecode_before: usize,
    /// Total bytecode size after merge.
    pub bytecode_after: usize,
    /// Number of types merged.
    pub types_merged: usize,
    /// Number of constants merged.
    pub constants_merged: usize,
}

// ============================================================================
// Function Mapping
// ============================================================================

/// Mapping from old function IDs to new function IDs.
#[derive(Debug, Clone, Default)]
pub struct FunctionMapping {
    /// User module function mappings.
    user_to_output: HashMap<FunctionId, FunctionId>,
    /// New specialization mappings (by instantiation hash).
    spec_to_output: HashMap<u64, FunctionId>,
}

impl FunctionMapping {
    /// Creates a new empty mapping.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a user function mapping.
    pub fn add_user(&mut self, old_id: FunctionId, new_id: FunctionId) {
        self.user_to_output.insert(old_id, new_id);
    }

    /// Records a new specialization mapping.
    pub fn add_spec(&mut self, hash: u64, new_id: FunctionId) {
        self.spec_to_output.insert(hash, new_id);
    }

    /// Looks up a function in the output module.
    pub fn get(&self, old_id: FunctionId) -> Option<FunctionId> {
        self.user_to_output.get(&old_id).copied()
    }

    /// Looks up a specialization by hash.
    pub fn get_by_hash(&self, hash: u64) -> Option<FunctionId> {
        self.spec_to_output.get(&hash).copied()
    }
}

// ============================================================================
// Module Merger
// ============================================================================

/// Merges user module, stdlib specializations, and new specializations.
pub struct ModuleMerger {
    /// User module VBC.
    user_module: VbcModule,
    /// Optional stdlib module.
    stdlib: Option<Arc<VbcModule>>,
    /// Newly specialized functions.
    specialized: Vec<(InstantiationRequest, SpecializedFunction)>,
    /// Resolver with resolution information.
    resolver: MonomorphizationResolver,
    /// Function mapping.
    mapping: FunctionMapping,
    /// Statistics.
    stats: MergeStats,
}

impl ModuleMerger {
    /// Creates a new module merger.
    pub fn new(
        user_module: VbcModule,
        stdlib: Option<Arc<VbcModule>>,
        specialized: Vec<(InstantiationRequest, SpecializedFunction)>,
        resolver: MonomorphizationResolver,
    ) -> Self {
        Self {
            user_module,
            stdlib,
            specialized,
            resolver,
            mapping: FunctionMapping::new(),
            stats: MergeStats::default(),
        }
    }

    /// Merges everything into a final monomorphized module.
    ///
    /// **Instruction-level merge (T0277).** The rewrite operates on each
    /// function's DECODED `instructions` — the artifact the AOT body
    /// lowering actually consumes (its work-list filters on
    /// `instructions.is_some()`; nothing on the mono path executes the
    /// raw byte stream). The byte-surgery twin this replaces re-derived
    /// wire layouts for four call opcodes and drifted from the canonical
    /// codec three separate ways (missing Spawn/GenCreate/FfiExtended-
    /// callback coverage, a varint-width in-place rewrite limitation,
    /// and a stale-`instructions` re-decode pass); the id rewrite now
    /// delegates to `bytecode_remap::rewrite_instruction_ids` — the ONE
    /// per-instruction id authority shared with the linker and the
    /// archive body merge. The bytecode blob is re-encoded from the
    /// final instruction streams afterwards so descriptors and bytes
    /// stay coherent for any residual consumer.
    pub fn merge(mut self) -> Result<(VbcModule, MergeStats), MergeError> {
        let mut output = VbcModule::new(self.user_module.name.clone());

        // Step 1: Copy user module structure
        self.copy_user_structure(&mut output)?;

        // Step 2: Copy user bytecode and functions
        self.copy_user_functions(&mut output)?;

        // Tombstone for the deleted raw-copy stdlib leg: a
        // `StdlibPrecompiled` resolution has no materializer here any
        // more — the raw byte copy it used performed NO string/type/
        // const/function id remap (and `FunctionMapping::add_stdlib`
        // had zero callers, so calls inside those bodies were never
        // routed), i.e. it could only ever inject silently-corrupt
        // bodies. Archived specializations must enter through the
        // archive-merge authority (`merge_archive_function_bodies`) —
        // T0313's leg. No production caller installs a stdlib module
        // today (`with_core` is unwired), so this warn is a tripwire,
        // not a behaviour change.
        if self.stdlib.is_some() {
            for request in self.resolver.pending() {
                if matches!(
                    self.resolver.get_resolution(request.hash),
                    Some(ResolvedSpecialization::StdlibPrecompiled { .. })
                ) {
                    tracing::warn!(
                        "mono merge: StdlibPrecompiled resolution for fn {} has no \
                         materializer (raw-copy leg removed under T0277); the call \
                         sites stay on the generic body. Route archived \
                         specializations through the archive-merge authority (T0313).",
                        request.function_id.0,
                    );
                }
            }
        }

        // Step 3: Add newly specialized functions (instructions decoded
        // eagerly — the rewrite below operates on them directly).
        self.add_new_specializations(&mut output)?;

        // Step 4: Rewrite id references on the DECODED instruction
        // streams via the ONE remap authority.
        self.rewrite_references(&mut output);

        // Step 5: Re-encode the bytecode blob from the final instruction
        // streams (canonical encoder; instruction-index jump offsets are
        // converted back to byte form by its fixup pass).
        Self::reencode_bytecode(&mut output);

        // Step 6: Update module flags
        output.update_flags();

        // Step 7: Compute final statistics
        self.stats.bytecode_after = output.bytecode.len();

        Ok((output, self.stats))
    }

    /// Copies user module structure (types, strings, constants, dependencies).
    fn copy_user_structure(&mut self, output: &mut VbcModule) -> Result<(), MergeError> {
        // Copy header
        output.header = self.user_module.header.clone();

        // Copy string table
        output.strings = self.user_module.strings.clone();

        // Copy type table
        output.types = self.user_module.types.clone();
        self.stats.types_merged = output.types.len();

        // Copy constant pool
        output.constants = self.user_module.constants.clone();
        self.stats.constants_merged = output.constants.len();

        // Copy source map
        output.source_map = self.user_module.source_map.clone();

        // Copy dependencies
        output.dependencies = self.user_module.dependencies.clone();

        // XMOD-BAND-RESOLVE (#38): carry the cross-module external-symbol name
        // table (XMOD band-id → qualified name). XMOD band ids (0x2000_0000+)
        // are placeholders, NOT module-local function ids, so they are not
        // remapped by copy_user_functions and this table stays consistent after
        // the merge. Without this copy, a mono-merged module reaches AOT codegen
        // with an empty table and every cross-module call (mmap/munmap/close/…)
        // degrades to a wrong-result const-zero stub.
        output.external_function_names = self.user_module.external_function_names.clone();

        Ok(())
    }

    /// Copies user module functions and bytecode.
    fn copy_user_functions(&mut self, output: &mut VbcModule) -> Result<(), MergeError> {
        self.stats.bytecode_before = self.user_module.bytecode.len();

        // Copy all user functions
        for func in &self.user_module.functions {
            let old_id = func.id;
            let new_offset = output.bytecode.len() as u32;

            // Copy bytecode
            let start = func.bytecode_offset as usize;
            let end = start + func.bytecode_length as usize;

            if end > self.user_module.bytecode.len() {
                return Err(MergeError::InvalidBytecodeRange {
                    offset: func.bytecode_offset,
                    length: func.bytecode_length,
                    module_size: self.user_module.bytecode.len(),
                });
            }

            output
                .bytecode
                .extend_from_slice(&self.user_module.bytecode[start..end]);

            // Create new function descriptor with updated offset
            let mut new_func = func.clone();
            new_func.id = FunctionId(output.functions.len() as u32);
            new_func.bytecode_offset = new_offset;
            output.functions.push(new_func);

            // Record mapping
            self.mapping
                .add_user(old_id, FunctionId(output.functions.len() as u32 - 1));
            self.stats.user_functions += 1;
        }

        Ok(())
    }

    /// Adds newly specialized functions.
    fn add_new_specializations(&mut self, output: &mut VbcModule) -> Result<(), MergeError> {
        for (request, specialized) in std::mem::take(&mut self.specialized) {
            let new_offset = output.bytecode.len() as u32;

            // Add bytecode
            output.bytecode.extend_from_slice(&specialized.bytecode);

            // Add new constants
            for constant in specialized.new_constants {
                output.constants.push(constant);
            }

            // Generate a UNIQUE, non-empty name mangled from the generic
            // function name + concrete type args.  The AOT backend lowers and
            // CALLs functions BY NAME; an empty name (or one colliding with the
            // still-present generic body) makes every caller resolve back to
            // the un-specialized generic — so the whole specialization is inert
            // and a protocol-method call on the type parameter stays a
            // passthrough (the async-AOT SIGSEGV).  Look the generic name up in
            // the user module, falling back to the stdlib.
            let generic_name = self
                .user_module
                .get_function(request.function_id)
                .and_then(|f| self.user_module.get_string(f.name))
                .map(|s| s.to_string())
                .or_else(|| {
                    self.stdlib.as_ref().and_then(|s| {
                        s.get_function(request.function_id)
                            .and_then(|f| s.get_string(f.name))
                            .map(|n| n.to_string())
                    })
                })
                .unwrap_or_else(|| format!("mono_fn_{}", request.function_id.0));
            fn mangle_tr(t: &TypeRef) -> String {
                match t {
                    TypeRef::Concrete(id) => id.0.to_string(),
                    TypeRef::Instantiated { base, args } => {
                        let inner: Vec<String> = args.iter().map(mangle_tr).collect();
                        if inner.is_empty() {
                            base.0.to_string()
                        } else {
                            format!("{}i{}", base.0, inner.join("_"))
                        }
                    }
                    TypeRef::Generic(tp) => format!("g{}", tp.0),
                    _ => "x".to_string(),
                }
            }
            let mangle: String = request
                .type_args
                .iter()
                .map(mangle_tr)
                .collect::<Vec<_>>()
                .join("_");
            let spec_name = format!("{}$mono${}", generic_name, mangle);
            let name_id = output.intern_string(&spec_name);
            if std::env::var_os("VERUM_TRACE_MONO").is_some()
                && (spec_name.contains("poll_sync") || spec_name.contains("ready"))
            {
                eprintln!(
                    "[mono-spec-name] specialized fn id={} name='{}'",
                    output.functions.len(),
                    spec_name
                );
            }

            // Base the specialized descriptor on the GENERIC descriptor so it
            // inherits the parameter list, return type and context/property
            // metadata — the AOT declares each function's LLVM signature from
            // `params`/`return_type`, and an empty `params` (the old
            // `..Default::default()`) declares a zero-arg `()` signature the
            // real body can't satisfy (the callee reads argument registers that
            // were never passed → garbage/crash).  Only the identity-, name-,
            // location- and size-fields are overridden; `instructions` is
            // decoded below (`decode_spec_body`) and rewritten in place by
            // `rewrite_references`.
            let base_desc = self
                .user_module
                .get_function(request.function_id)
                .cloned()
                .or_else(|| {
                    self.stdlib
                        .as_ref()
                        .and_then(|s| s.get_function(request.function_id).cloned())
                })
                .unwrap_or_default();
            let mut new_func = base_desc;
            new_func.id = FunctionId(output.functions.len() as u32);
            new_func.name = name_id;
            new_func.bytecode_offset = new_offset;
            new_func.bytecode_length = specialized.bytecode.len() as u32;
            new_func.register_count = specialized.register_count;
            new_func.locals_count = specialized.locals_count;
            new_func.max_stack = specialized.max_stack;
            new_func.is_generic = false;
            // Decode the specialized body NOW, from its own coherent byte
            // stream (jump offsets converted BYTE→INSTRUCTION form — the
            // representation the AOT body-lowering consumes). The id
            // rewrite pass operates on these decoded instructions
            // directly; an undecodable body stays forward-declared,
            // exactly as before.
            new_func.instructions = Self::decode_spec_body(&specialized.bytecode);
            // Layer main's #39/#35 + #41 onto the branch's inherit-from-generic
            // base: float-marking needs the SUBSTITUTED concrete return type
            // (not the generic `T`, which can't be float-classified), and AOT
            // ref-marking wants the substituted `&T` param descriptors. Guard
            // params so an empty specialized list never clobbers the generic's
            // non-empty params (the zero-arg-signature crash `base_desc` fixed).
            if !specialized.params.is_empty() {
                new_func.params = specialized.params;
            }
            new_func.return_type = specialized.return_type;

            // Resolve associated-type projections in the inherited return type:
            // `Maybe<F.Output>` with F = ReadyFuture<Text> → `Maybe<Text>`. The
            // AOT text-marks a call's result from the callee's return type
            // (mark_register_from_return_type); an unresolved `F.Output` leaves
            // the payload unmarked so `print` formats it as the raw pointer-int
            // instead of the string. Future::Output ≡ poll's `Poll<Output>`
            // inner, Iterator::Item ≡ next's inner — read from the concrete
            // impl's method signature.
            new_func.return_type = Self::resolve_ret_projections(
                &new_func.return_type,
                &request.type_args,
                output,
                &self.user_module,
                self.stdlib.as_deref(),
            );

            output.functions.push(new_func);

            // Record mapping
            self.mapping
                .add_spec(request.hash, FunctionId(output.functions.len() as u32 - 1));
            self.stats.new_specializations += 1;

            // Add to specialization table
            output.specializations.push(SpecializationEntry {
                generic_fn: request.function_id,
                type_args: request.type_args.clone(),
                hash: request.hash,
                bytecode_offset: new_offset,
                bytecode_length: specialized.bytecode.len() as u32,
                register_count: specialized.register_count,
            });
        }

        Ok(())
    }

    /// Resolve `AssociatedProjection` nodes in a specialized function's return
    /// type using the concrete `type_args`: substitute generic params, then
    /// recover e.g. `F.Output` from the concrete `F`'s protocol method
    /// signature. Leaves any projection it can't resolve untouched.
    fn resolve_ret_projections(
        ty: &TypeRef,
        type_args: &[TypeRef],
        output: &VbcModule,
        user_module: &VbcModule,
        stdlib: Option<&VbcModule>,
    ) -> TypeRef {
        match ty {
            TypeRef::Generic(tp) => type_args
                .get(tp.0 as usize)
                .cloned()
                .unwrap_or_else(|| ty.clone()),
            TypeRef::AssociatedProjection { base, assoc } => {
                let rbase =
                    Self::resolve_ret_projections(base, type_args, output, user_module, stdlib);
                Self::resolve_assoc_via_method(&rbase, assoc, output, user_module, stdlib).unwrap_or(
                    TypeRef::AssociatedProjection {
                        base: Box::new(rbase),
                        assoc: assoc.clone(),
                    },
                )
            }
            TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
                base: *base,
                args: args
                    .iter()
                    .map(|a| {
                        Self::resolve_ret_projections(a, type_args, output, user_module, stdlib)
                    })
                    .collect(),
            },
            TypeRef::Reference {
                inner,
                mutability,
                tier,
            } => TypeRef::Reference {
                inner: Box::new(Self::resolve_ret_projections(
                    inner, type_args, output, user_module, stdlib,
                )),
                mutability: *mutability,
                tier: *tier,
            },
            _ => ty.clone(),
        }
    }

    /// Recover `<base>.<assoc>` from `base`'s concrete protocol-method return
    /// types: Future::Output is the sole arg of `poll(...) -> Poll<Output>`,
    /// Iterator::Item the sole arg of `next(...) -> Maybe<Item>`. The impl's
    /// method return carries `Output` as an impl generic (e.g. `Poll<T>`), so
    /// substitute the concrete base's type args into it. Returns None when the
    /// projection can't be recovered (caller keeps it unresolved).
    fn resolve_assoc_via_method(
        base: &TypeRef,
        assoc: &str,
        output: &VbcModule,
        user_module: &VbcModule,
        stdlib: Option<&VbcModule>,
    ) -> Option<TypeRef> {
        let trace = std::env::var_os("VERUM_TRACE_MONO").is_some();
        let (tid, targs): (TypeId, &[TypeRef]) = match base {
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            TypeRef::Concrete(id) => (*id, &[]),
            _ => return None,
        };
        // The type_arg id is a MERGED-module id; resolve it against `output`
        // (the module being built and lowered) first, source modules as fallback.
        let td = match output
            .get_type(tid)
            .or_else(|| user_module.get_type(tid))
            .or_else(|| stdlib.and_then(|s| s.get_type(tid)))
        {
            Some(td) => td,
            None => {
                if trace {
                    eprintln!("[mono-assoc] type#{} NOT FOUND", tid.0);
                }
                return None;
            }
        };
        let want_method = match assoc {
            "Output" => "poll",
            "Item" => "next",
            _ => return None,
        };
        for pi in &td.protocols {
            for &m in &pi.methods {
                if m == u32::MAX {
                    continue;
                }
                let (fd, home): (&FunctionDescriptor, &VbcModule) =
                    if let Some(f) = output.get_function(FunctionId(m)) {
                        (f, output)
                    } else if let Some(f) = user_module.get_function(FunctionId(m)) {
                        (f, user_module)
                    } else if let Some(f) = stdlib.and_then(|s| s.get_function(FunctionId(m))) {
                        (f, stdlib.unwrap())
                    } else {
                        continue;
                    };
                let mname = home.get_string(fd.name).unwrap_or("");
                let short = mname.rsplit('.').next().unwrap_or(mname);
                if short != want_method {
                    continue;
                }
                if let TypeRef::Instantiated { args, .. } = &fd.return_type {
                    if let Some(inner) = args.first() {
                        let resolved = Self::subst_type_params(inner, targs);
                        if std::env::var_os("VERUM_TRACE_MONO").is_some() {
                            eprintln!(
                                "[mono-assoc] type#{}.{} → {:?} (via {} return {:?})",
                                tid.0, assoc, resolved, short, fd.return_type
                            );
                        }
                        return Some(resolved);
                    }
                }
            }
        }
        // Fallback: the protocol-impl method list may not carry `poll`/`next`
        // (it can record a coincidental method id after import remapping).
        // Find the method by NAME — `<Type>.<method>` — across the merged and
        // source modules. `<Type>` is matched exactly (as a `.`-delimited
        // segment) so `SelectReadyFuture.poll` doesn't match `ReadyFuture`.
        let tn = output
            .get_type_name(tid)
            .or_else(|| user_module.get_type_name(tid))
            .or_else(|| stdlib.and_then(|s| s.get_type_name(tid)))?;
        let exact = format!("{tn}.{want_method}");
        let suffix = format!(".{tn}.{want_method}");
        let modules: Vec<&VbcModule> = std::iter::once(output)
            .chain(std::iter::once(user_module))
            .chain(stdlib)
            .collect();
        for home in modules {
            for f in &home.functions {
                let fname = home.get_string(f.name).unwrap_or("");
                if fname == exact || fname.ends_with(&suffix) {
                    if let TypeRef::Instantiated { args, .. } = &f.return_type {
                        if let Some(inner) = args.first() {
                            let resolved = Self::subst_type_params(inner, targs);
                            if trace {
                                eprintln!(
                                    "[mono-assoc-byname] {} -> {:?} (from {} return {:?})",
                                    fname, resolved, tn, f.return_type
                                );
                            }
                            return Some(resolved);
                        }
                    }
                }
            }
        }
        None
    }

    /// Substitute `Generic(n)` with `args[n]` — maps an impl method's generic
    /// (`Poll<T>`'s `T`) to the concrete base type's Nth type argument.
    fn subst_type_params(ty: &TypeRef, args: &[TypeRef]) -> TypeRef {
        match ty {
            TypeRef::Generic(tp) => args
                .get(tp.0 as usize)
                .cloned()
                .unwrap_or_else(|| ty.clone()),
            TypeRef::Instantiated { base, args: iargs } => TypeRef::Instantiated {
                base: *base,
                args: iargs
                    .iter()
                    .map(|a| Self::subst_type_params(a, args))
                    .collect(),
            },
            _ => ty.clone(),
        }
    }

    /// Decode `instructions` for the functions in `[first_new_spec, len)` from
    /// the (already fixed-up) module bytecode.  Required so the AOT lowers
    /// their bodies (its body work-list filters on `instructions.is_some()`).
    /// A function whose byte range fails to decode cleanly is left as-is (it
    /// will simply be forward-declared, as before) rather than aborting merge.
    /// Convert BYTE-relative jump offsets (as produced by the bytecode decoder)
    /// to INSTRUCTION-relative offsets (`target_index - this_index`), which is
    /// what the AOT body-lowering consumes. `positions[i]` is the start byte of
    /// instruction `i` (relative to the function); the target byte of a jump is
    /// `(end of this instruction) + byte_offset`.
    fn convert_jump_offsets_byte_to_instr(
        instrs: &mut [crate::instruction::Instruction],
        positions: &[usize],
        total_len: usize,
    ) {
        use crate::instruction::Instruction;
        use std::collections::HashMap;
        let byte_to_idx: HashMap<usize, i32> = positions
            .iter()
            .enumerate()
            .map(|(i, &p)| (p, i as i32))
            .collect();
        for i in 0..instrs.len() {
            let end = if i + 1 < positions.len() {
                positions[i + 1]
            } else {
                total_len
            };
            let to_instr = |off: i32| -> Option<i32> {
                let target_byte = end as i64 + off as i64;
                if target_byte < 0 {
                    return None;
                }
                byte_to_idx.get(&(target_byte as usize)).map(|&j| j - i as i32)
            };
            match &mut instrs[i] {
                Instruction::Jmp { offset }
                | Instruction::JmpNot { offset, .. }
                | Instruction::JmpIf { offset, .. }
                | Instruction::JmpCmp { offset, .. } => {
                    if let Some(o) = to_instr(*offset) {
                        *offset = o;
                    }
                }
                Instruction::CtxProvide { body_offset, .. } => {
                    if let Some(o) = to_instr(*body_offset) {
                        *body_offset = o;
                    }
                }
                Instruction::TryBegin { handler_offset } => {
                    if let Some(o) = to_instr(*handler_offset) {
                        *handler_offset = o;
                    }
                }
                _ => {}
            }
        }
    }

    /// Decode one specialized function body from its own byte stream,
    /// converting the decoder's BYTE-relative jump offsets to the
    /// INSTRUCTION-relative offsets the AOT body-lowering expects.
    fn decode_spec_body(bytes: &[u8]) -> Option<Vec<crate::instruction::Instruction>> {
        let mut instrs = Vec::new();
        let mut positions = Vec::new();
        let mut pc = 0usize;
        while pc < bytes.len() {
            positions.push(pc);
            match crate::bytecode::decode_instruction(bytes, &mut pc) {
                Ok(i) => instrs.push(i),
                Err(_) => return None,
            }
        }
        Self::convert_jump_offsets_byte_to_instr(&mut instrs, &positions, bytes.len());
        Some(instrs)
    }

    /// Rewrites id references on every function's DECODED instruction
    /// stream — the ONE per-instruction id authority
    /// ([`crate::bytecode_remap::rewrite_instruction_ids`]) covers every
    /// id-bearing variant (incl. Spawn / GenCreate / FfiExtended
    /// CreateCallback, which the byte-surgery twin silently missed), and
    /// rewriting the DECODED form removes both prior limitation classes:
    /// no varint-width in-place constraint ("site left generic"), and no
    /// stale-`instructions` re-decode pass (the instructions ARE the
    /// artifact being rewritten).
    ///
    /// Routing semantics preserved from the byte twin:
    ///  * VBC-GENERIC-INSTANTIATION single-instantiation routing — a
    ///    generic with EXACTLY ONE specialization routes EVERY reference
    ///    (any opcode) to the specialized body via the id map.
    ///  * Per-site `CallG` routing — the site's static type args select
    ///    the matching specialization; site targets are freshly-appended
    ///    output ids, disjoint from the old-id keyspace of the blanket
    ///    map, so the passes compose without double-remap.
    fn rewrite_references(&mut self, output: &mut VbcModule) {
        use crate::instruction::Instruction;

        // Old function id → output id (user compaction).
        let mut id_remap: HashMap<u32, u32> = HashMap::new();
        for (old_id, new_id) in &self.mapping.user_to_output {
            id_remap.insert(old_id.0, new_id.0);
        }

        // Single-instantiation routing (see doc above).
        {
            let mut spec_count: HashMap<u32, usize> = HashMap::new();
            for spec in &output.specializations {
                if self.mapping.get_by_hash(spec.hash).is_some() {
                    *spec_count.entry(spec.generic_fn.0).or_insert(0) += 1;
                }
            }
            let trace = std::env::var_os("VERUM_TRACE_MONO").is_some();
            for spec in &output.specializations {
                if spec_count.get(&spec.generic_fn.0) == Some(&1)
                    && let Some(spec_id) = self.mapping.get_by_hash(spec.hash)
                {
                    if trace {
                        eprintln!(
                            "[mono-route] generic_fn={} -> specialized_fn={}",
                            spec.generic_fn.0, spec_id.0
                        );
                    }
                    id_remap.insert(spec.generic_fn.0, spec_id.0);
                }
            }
        }

        // Per-site CallG routing table.
        let mut site_route: HashMap<(u32, Vec<TypeRef>), u32> = HashMap::new();
        for spec in &output.specializations {
            if let Some(spec_id) = self.mapping.get_by_hash(spec.hash) {
                site_route.insert((spec.generic_fn.0, spec.type_args.clone()), spec_id.0);
            }
        }

        struct MonoIdRemap<'a> {
            map: &'a HashMap<u32, u32>,
        }
        impl crate::bytecode_remap::IdRemap for MonoIdRemap<'_> {
            fn map_function(&self, src: FunctionId) -> FunctionId {
                FunctionId(*self.map.get(&src.0).unwrap_or(&src.0))
            }
        }
        let remap = MonoIdRemap { map: &id_remap };

        for func in output.functions.iter_mut() {
            let Some(instrs) = func.instructions.as_mut() else {
                continue;
            };
            for instr in instrs.iter_mut() {
                if let Instruction::CallG {
                    func_id, type_args, ..
                } = instr
                {
                    if let Some(&spec_id) =
                        site_route.get(&(*func_id, type_args.clone()))
                    {
                        // Site target is a final output id — no further
                        // remap applies (and none matches: the blanket
                        // map is keyed by OLD-space ids).
                        *func_id = spec_id;
                        continue;
                    }
                }
                crate::bytecode_remap::rewrite_instruction_ids(instr, &remap);
            }
        }

        // Update specialization entries with the routed generic ids.
        for spec in &mut output.specializations {
            if let Some(&new_id) = id_remap.get(&spec.generic_fn.0) {
                spec.generic_fn = FunctionId(new_id);
            }
        }
    }

    /// Rebuilds the bytecode blob from the final (rewritten) instruction
    /// streams via the canonical encoder, updating each descriptor's
    /// offset/length. Bodiless descriptors that still reference a raw
    /// byte range (FFI / forward declarations) have their bytes carried
    /// over verbatim so descriptors and bytes stay coherent for every
    /// consumer of the merged module.
    fn reencode_bytecode(output: &mut VbcModule) {
        let old_blob = std::mem::take(&mut output.bytecode);
        let mut blob: Vec<u8> = Vec::with_capacity(old_blob.len());
        for f in output.functions.iter_mut() {
            let new_off = blob.len() as u32;
            if let Some(instrs) = f.instructions.as_deref() {
                let n = crate::bytecode::encode_instructions_with_fixup(instrs, &mut blob);
                f.bytecode_offset = new_off;
                f.bytecode_length = n as u32;
            } else if f.bytecode_length > 0 {
                let s = f.bytecode_offset as usize;
                let e = s + f.bytecode_length as usize;
                if e <= old_blob.len() {
                    blob.extend_from_slice(&old_blob[s..e]);
                    f.bytecode_offset = new_off;
                } else {
                    // Invalid stale range (skipped by every prior pass
                    // too) — make the descriptor honestly bodiless.
                    f.bytecode_offset = new_off;
                    f.bytecode_length = 0;
                }
            }
        }
        output.bytecode = blob;
    }

    /// Returns the function mapping.
    pub fn mapping(&self) -> &FunctionMapping {
        &self.mapping
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_mapping() {
        let mut mapping = FunctionMapping::new();

        mapping.add_user(FunctionId(0), FunctionId(10));
        mapping.add_user(FunctionId(1), FunctionId(11));
        mapping.add_spec(0x123456, FunctionId(20));

        assert_eq!(mapping.get(FunctionId(0)), Some(FunctionId(10)));
        assert_eq!(mapping.get(FunctionId(1)), Some(FunctionId(11)));
        assert_eq!(mapping.get_by_hash(0x123456), Some(FunctionId(20)));
        assert_eq!(mapping.get(FunctionId(99)), None);
    }

    #[test]
    fn test_merge_stats_default() {
        let stats = MergeStats::default();
        assert_eq!(stats.user_functions, 0);
        assert_eq!(stats.stdlib_specializations, 0);
        assert_eq!(stats.new_specializations, 0);
    }

}
