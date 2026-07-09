//! Module merging for monomorphization.
//!

//! The ModuleMerger combines:
//! - User module VBC
//! - Stdlib precompiled specializations
//! - Newly monomorphized functions
//!

//! Into a final monomorphized VBC module ready for execution.
//!

//! Key responsibilities:
//! 1. Copy user module structure (types, strings, constants)
//! 2. Copy user bytecode with offset remapping
//! 3. Add stdlib precompiled specializations
//! 4. Add newly specialized functions
//! 5. **CRITICAL: Fixup all function references in bytecode**
//!

//! Final phase of monomorphization: produces a self-contained VBC module with all
//! generic instantiations resolved to concrete specialized functions.

use std::collections::HashMap;
use std::sync::Arc;

use crate::instruction::Opcode;
use crate::module::{FunctionDescriptor, FunctionId, SpecializationEntry, VbcModule};
use crate::types::{StringId, TypeId, TypeRef};

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
    /// Stdlib specialization mappings.
    stdlib_to_output: HashMap<FunctionId, FunctionId>,
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

    /// Records a stdlib specialization mapping.
    pub fn add_stdlib(&mut self, old_id: FunctionId, new_id: FunctionId) {
        self.stdlib_to_output.insert(old_id, new_id);
    }

    /// Records a new specialization mapping.
    pub fn add_spec(&mut self, hash: u64, new_id: FunctionId) {
        self.spec_to_output.insert(hash, new_id);
    }

    /// Looks up a function in the output module.
    pub fn get(&self, old_id: FunctionId) -> Option<FunctionId> {
        self.user_to_output
            .get(&old_id)
            .or_else(|| self.stdlib_to_output.get(&old_id))
            .copied()
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
    /// Generic-function ids that `fixup_references` actually routed to a single
    /// specialization (old generic id → its bytecode was rewritten to call the
    /// spec).  `decode_specialized_instructions` reads this to re-decode exactly
    /// the callers whose instructions went stale — recomputing the set there
    /// disagreed with the routing (duplicate seed/new specialization entries
    /// perturb the per-generic count), so the routing records it directly.
    routed_generics: Vec<u32>,
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
            routed_generics: Vec::new(),
        }
    }

    /// Merges everything into a final monomorphized module.
    pub fn merge(mut self) -> Result<(VbcModule, MergeStats), MergeError> {
        let mut output = VbcModule::new(self.user_module.name.clone());

        // Step 1: Copy user module structure
        self.copy_user_structure(&mut output)?;

        // Step 2: Copy user bytecode and functions
        self.copy_user_functions(&mut output)?;

        // Step 3: Add stdlib specializations
        self.add_stdlib_specializations(&mut output)?;

        // Step 4: Add newly specialized functions
        let first_new_spec = output.functions.len();
        self.add_new_specializations(&mut output)?;

        // Step 5: Fixup function references in bytecode
        self.fixup_references(&mut output)?;

        // Step 5.5: Decode `instructions` for the new specializations from the
        // now-FIXED-UP bytecode.  The AOT lowers function BODIES only for
        // descriptors whose `instructions` is populated (it builds its
        // VbcFunction work-list by filtering on `instructions.is_some()`); a
        // specialization left with `instructions: None` is forward-declared but
        // never defined, so every call to it lands on an undefined symbol
        // (SIGSEGV).  Decoding here — after `fixup_references` — guarantees the
        // instruction stream carries the final, remapped call targets.
        self.decode_specialized_instructions(&mut output, first_new_spec);

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

    /// Adds stdlib precompiled specializations.
    fn add_stdlib_specializations(&mut self, output: &mut VbcModule) -> Result<(), MergeError> {
        let Some(ref stdlib) = self.stdlib else {
            return Ok(());
        };

        // Get all stdlib precompiled resolutions
        for request in self.resolver.pending() {
            if let Some(ResolvedSpecialization::StdlibPrecompiled {
                bytecode_offset,
                bytecode_length,
                register_count,
            }) = self.resolver.get_resolution(request.hash)
            {
                // Copy bytecode from stdlib
                let new_offset = output.bytecode.len() as u32;
                let start = *bytecode_offset as usize;
                let end = start + *bytecode_length as usize;

                if end > stdlib.bytecode.len() {
                    return Err(MergeError::InvalidBytecodeRange {
                        offset: *bytecode_offset,
                        length: *bytecode_length,
                        module_size: stdlib.bytecode.len(),
                    });
                }

                output
                    .bytecode
                    .extend_from_slice(&stdlib.bytecode[start..end]);

                // Create function descriptor for specialization
                let new_func = FunctionDescriptor {
                    id: FunctionId(output.functions.len() as u32),
                    name: StringId::EMPTY, // Could copy from stdlib
                    bytecode_offset: new_offset,
                    bytecode_length: *bytecode_length,
                    register_count: *register_count,
                    is_generic: false, // Specialized - no longer generic
                    ..Default::default()
                };

                output.functions.push(new_func);

                // Record mapping
                self.mapping
                    .add_spec(request.hash, FunctionId(output.functions.len() as u32 - 1));
                self.stats.stdlib_specializations += 1;
            }
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
            // location- and size-fields are overridden.  `instructions` is left
            // None here and decoded from the FIXED-UP bytecode after
            // `fixup_references` (see `decode_specialized_instructions`).
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
            new_func.instructions = None;

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

    fn decode_specialized_instructions(&self, output: &mut VbcModule, first_new_spec: usize) {
        // fixup_references rewrote call func_ids IN THE BYTECODE, but the AOT
        // body-lowering consumes each descriptor's DECODED `instructions`, not
        // the raw bytecode.  So any function whose calls were rewritten (e.g.
        // `main`, whose CallG to a generic was routed to the specialization)
        // has STALE instructions still naming the old target.  Re-decode from
        // the fixed-up bytecode for (a) every function that already carried a
        // body (instructions.is_some() → refresh) and (b) the new
        // specializations at [first_new_spec, len) (populate).  Bodyless
        // FFI/forward-decl functions (None and not a new spec) are left
        // untouched so they stay declared-only.
        // The routing (fixup_references) only rewrites calls to
        // single-instantiation generics, so ONLY the new specializations and
        // the (few) functions that call a routed generic have stale
        // instructions.  Re-decoding every body would be O(module) and times
        // out on stdlib-sized inputs, so target precisely those.  Use the EXACT
        // set of generic ids the routing rewrote (recorded in
        // `self.routed_generics`) — recomputing it here disagreed because the
        // specializations table holds duplicate seed/new entries.
        let routed: std::collections::HashSet<u32> =
            self.routed_generics.iter().copied().collect();
        let calls_routed = |instrs: &[crate::instruction::Instruction]| -> bool {
            instrs.iter().any(|i| match i {
                crate::instruction::Instruction::Call { func_id, .. }
                | crate::instruction::Instruction::TailCall { func_id, .. }
                | crate::instruction::Instruction::CallG { func_id, .. } => {
                    routed.contains(func_id)
                }
                _ => false,
            })
        };
        let ranges: Vec<(usize, usize, usize)> = output
            .functions
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                f.bytecode_length > 0
                    && (*i >= first_new_spec
                        || f.instructions.as_deref().is_some_and(&calls_routed))
            })
            .map(|(i, f)| (i, f.bytecode_offset as usize, f.bytecode_length as usize))
            .collect();
        if std::env::var_os("VERUM_TRACE_MONO").is_some() {
            let with_body = output
                .functions
                .iter()
                .filter(|f| f.instructions.is_some())
                .count();
            eprintln!(
                "[mono-refresh] routed={:?} candidate_ranges={} first_new_spec={} fns_with_body={}",
                routed,
                ranges.len(),
                first_new_spec,
                with_body
            );
        }
        for (idx, off, len) in ranges {
            if len == 0 || off + len > output.bytecode.len() {
                continue;
            }
            let mut instrs = Vec::new();
            let mut positions = Vec::new();
            let mut pc = off;
            let end = off + len;
            let mut ok = true;
            while pc < end {
                positions.push(pc - off);
                match crate::bytecode::decode_instruction(&output.bytecode, &mut pc) {
                    Ok(instr) => instrs.push(instr),
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && pc == end {
                // Convert the decoder's BYTE-relative jump offsets to the
                // INSTRUCTION-relative offsets the AOT body-lowering expects
                // (vbc_lowering computes `target_index = index + offset`). The
                // codegen path builds instruction-relative offsets directly, so
                // a freshly-DECODED specialization must be converted to match —
                // otherwise a byte offset (e.g. 23) is read as an instruction
                // delta, the jump target lands out of range, and the match
                // collapses (future_poll_sync's Poll->Maybe Pending arm became a
                // reachable `unreachable`).
                Self::convert_jump_offsets_byte_to_instr(&mut instrs, &positions, len);
                if std::env::var_os("VERUM_TRACE_MONO").is_some() {
                    let callg_ids: Vec<u32> = instrs
                        .iter()
                        .filter_map(|i| match i {
                            crate::instruction::Instruction::CallG { func_id, .. }
                            | crate::instruction::Instruction::Call { func_id, .. } => {
                                Some(*func_id)
                            }
                            _ => None,
                        })
                        .filter(|id| routed.contains(id) || *id >= first_new_spec as u32)
                        .collect();
                    if !callg_ids.is_empty() {
                        let nm = output
                            .get_string(output.functions[idx].name)
                            .unwrap_or("<?>")
                            .to_string();
                        eprintln!(
                            "[mono-refresh] idx={} '{}' now-calls-routed/spec={:?}",
                            idx, nm, callg_ids
                        );
                    }
                    let nm = output
                        .get_string(output.functions[idx].name)
                        .unwrap_or("<?>")
                        .to_string();
                    if nm.contains("poll_sync") {
                        let ops: Vec<String> = instrs
                            .iter()
                            .take(12)
                            .map(|i| {
                                let s = format!("{:?}", i);
                                s.split([' ', '{', '(']).next().unwrap_or("?").to_string()
                            })
                            .collect();
                        eprintln!(
                            "[mono-decode] '{}' off={} len={} n_instr={} first_ops={:?}",
                            nm,
                            off,
                            len,
                            instrs.len(),
                            ops
                        );
                    }
                }
                output.functions[idx].instructions = Some(instrs);
            } else if std::env::var_os("VERUM_TRACE_MONO").is_some() {
                let nm = output
                    .get_string(output.functions[idx].name)
                    .unwrap_or("<?>")
                    .to_string();
                if nm.contains("poll_sync") {
                    eprintln!(
                        "[mono-decode] '{}' off={} len={} DECODE-FAILED (ok={} pc={} end={})",
                        nm, off, len, ok, pc, end
                    );
                }
            }
        }
    }

    /// Fixes up function references in bytecode.
    ///

    /// This is **CRITICAL** for correctness - rewrites all CALL, CALL_G, CALL_V,
    /// TAIL_CALL instructions to point to the correct function IDs in the merged module.
    ///

    /// The algorithm:
    /// 1. For each function's bytecode range
    /// 2. Scan for call-related opcodes
    /// 3. Read the old function ID
    /// 4. Look up the new function ID in mapping
    /// 5. Rewrite in place
    fn fixup_references(&mut self, output: &mut VbcModule) -> Result<(), MergeError> {
        // Build reverse mapping: old_function_id -> new_function_id
        // This is needed because bytecode contains old IDs
        let mut id_remap: HashMap<u32, u32> = HashMap::new();
        for (old_id, new_id) in &self.mapping.user_to_output {
            id_remap.insert(old_id.0, new_id.0);
        }
        for (old_id, new_id) in &self.mapping.stdlib_to_output {
            id_remap.insert(old_id.0, new_id.0);
        }

        // VBC-GENERIC-INSTANTIATION routing: for a generic function with
        // EXACTLY ONE specialization, route EVERY reference to it (Call/CallG,
        // in any function including non-specialized callers like `main`) to the
        // specialized body by overriding its id_remap entry.  The intra-function
        // `specialize_call_g` only rewrites CallG *inside* functions being
        // specialized; a plain caller's call would otherwise still target the
        // un-specialized generic body (whose protocol-method call on the type
        // parameter is a passthrough → wrong result / crash).  Single-
        // instantiation only — a generic used at several concrete types can't
        // collapse to one id.  `output.specializations` holds only the newly-
        // created specializations, so there is no double counting.
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
                    self.routed_generics.push(spec.generic_fn.0);
                }
            }
        }

        // Process each function's bytecode
        for func in &output.functions {
            let start = func.bytecode_offset as usize;
            let end = start + func.bytecode_length as usize;

            if end > output.bytecode.len() {
                continue; // Skip invalid ranges
            }

            // Scan and fixup this function's bytecode
            self.fixup_function_bytecode(&mut output.bytecode, start, end, &id_remap)?;
        }

        // Update specialization entries with correct function IDs
        for spec in &mut output.specializations {
            if let Some(&new_id) = id_remap.get(&spec.generic_fn.0) {
                spec.generic_fn = FunctionId(new_id);
            }
        }

        Ok(())
    }

    /// Fixes up function references in a single function's bytecode.
    fn fixup_function_bytecode(
        &self,
        bytecode: &mut [u8],
        start: usize,
        end: usize,
        id_remap: &HashMap<u32, u32>,
    ) -> Result<(), MergeError> {
        let mut pc = start;

        while pc < end {
            let opcode_byte = bytecode[pc];
            let opcode = Opcode::from_byte(opcode_byte);
            pc += 1;

            match opcode {
                // CALL dst:reg func_id:varint arg_count:u8 [args:reg...]
                Opcode::Call | Opcode::TailCall => {
                    // Skip destination register
                    pc = self.skip_register(bytecode, pc);

                    // Read and rewrite function ID (varint)
                    let (old_func_id, varint_len) = self.read_varint(bytecode, pc);
                    if let Some(&new_func_id) = id_remap.get(&(old_func_id as u32)) {
                        // Rewrite the varint in place
                        self.write_varint_in_place(bytecode, pc, varint_len, new_func_id as u64);
                    }
                    pc += varint_len;

                    // Skip arg_count and args
                    if pc < end {
                        let arg_count = bytecode[pc] as usize;
                        pc += 1;
                        for _ in 0..arg_count {
                            pc = self.skip_register(bytecode, pc);
                        }
                    }
                }

                // CALL_G dst:reg func_id:varint type_arg_count:u8 [type_args...] arg_count:u8 [args:reg...]
                Opcode::CallG => {
                    // Skip destination register
                    pc = self.skip_register(bytecode, pc);

                    // Read and rewrite function ID (varint)
                    let (old_func_id, varint_len) = self.read_varint(bytecode, pc);
                    if let Some(&new_func_id) = id_remap.get(&(old_func_id as u32)) {
                        self.write_varint_in_place(bytecode, pc, varint_len, new_func_id as u64);
                    }
                    pc += varint_len;

                    // Skip type args
                    if pc < end {
                        let type_arg_count = bytecode[pc] as usize;
                        pc += 1;
                        for _ in 0..type_arg_count {
                            pc = self.skip_type_ref(bytecode, pc, end);
                        }
                    }

                    // Skip arg_count and args
                    if pc < end {
                        let arg_count = bytecode[pc] as usize;
                        pc += 1;
                        for _ in 0..arg_count {
                            pc = self.skip_register(bytecode, pc);
                        }
                    }
                }

                // CALL_V dst:reg receiver:reg method_id:varint arg_count:u8 [args:reg...]
                Opcode::CallV => {
                    // Skip destination register
                    pc = self.skip_register(bytecode, pc);
                    // Skip receiver register
                    pc = self.skip_register(bytecode, pc);

                    // Read and potentially rewrite method ID
                    let (method_id, varint_len) = self.read_varint(bytecode, pc);
                    if let Some(&new_method_id) = id_remap.get(&(method_id as u32)) {
                        self.write_varint_in_place(bytecode, pc, varint_len, new_method_id as u64);
                    }
                    pc += varint_len;

                    // Skip arg_count and args
                    if pc < end {
                        let arg_count = bytecode[pc] as usize;
                        pc += 1;
                        for _ in 0..arg_count {
                            pc = self.skip_register(bytecode, pc);
                        }
                    }
                }

                // CALL_C dst:reg cache_slot:u32 func_id:varint arg_count:u8 [args:reg...]
                Opcode::CallC => {
                    // Skip destination register
                    pc = self.skip_register(bytecode, pc);
                    // Skip cache slot (4 bytes)
                    pc += 4;

                    // Read and rewrite function ID
                    let (old_func_id, varint_len) = self.read_varint(bytecode, pc);
                    if let Some(&new_func_id) = id_remap.get(&(old_func_id as u32)) {
                        self.write_varint_in_place(bytecode, pc, varint_len, new_func_id as u64);
                    }
                    pc += varint_len;

                    // Skip arg_count and args
                    if pc < end {
                        let arg_count = bytecode[pc] as usize;
                        pc += 1;
                        for _ in 0..arg_count {
                            pc = self.skip_register(bytecode, pc);
                        }
                    }
                }

                // NEW_CLOSURE dst:reg func_id:varint capture_count:u8 [captures:reg...]
                Opcode::NewClosure => {
                    // Skip destination register
                    pc = self.skip_register(bytecode, pc);

                    // Read and rewrite function ID
                    let (old_func_id, varint_len) = self.read_varint(bytecode, pc);
                    if let Some(&new_func_id) = id_remap.get(&(old_func_id as u32)) {
                        self.write_varint_in_place(bytecode, pc, varint_len, new_func_id as u64);
                    }
                    pc += varint_len;

                    // Skip capture_count and captures
                    if pc < end {
                        let capture_count = bytecode[pc] as usize;
                        pc += 1;
                        for _ in 0..capture_count {
                            pc = self.skip_register(bytecode, pc);
                        }
                    }
                }

                // All other opcodes carry no FunctionId to rewrite — advance
                // past the whole instruction using the CANONICAL decoder.  The
                // previous hand-rolled `skip_instruction_operands` fell back to
                // `min(pc + 4, end)` ("estimate 4 bytes") for any opcode it
                // didn't enumerate — a wrong guess that desynchronised the
                // fixup scan, after which a later operand byte could be
                // mis-read as a call opcode and have its "func_id" clobbered,
                // silently corrupting the merged module.
                _ => {
                    let instr_start = pc - 1; // opcode byte (pc was advanced past it)
                    let mut probe = instr_start;
                    match crate::bytecode::decode_instruction(bytecode, &mut probe) {
                        Ok(_) if probe > pc && probe <= end => pc = probe,
                        // Undecodable / overruns the function — stop the scan
                        // rather than risk clobbering unrelated bytes.
                        _ => pc = end,
                    }
                }
            }
        }

        Ok(())
    }

    /// Skips a register operand and returns new pc.
    fn skip_register(&self, bytecode: &[u8], pc: usize) -> usize {
        if pc >= bytecode.len() {
            return pc;
        }
        if bytecode[pc] < 128 { pc + 1 } else { pc + 2 }
    }

    /// Reads a varint and returns (value, length).
    fn read_varint(&self, bytecode: &[u8], pc: usize) -> (u64, usize) {
        let mut result: u64 = 0;
        let mut shift = 0;
        let mut len = 0;
        let mut pos = pc;

        while pos < bytecode.len() {
            let byte = bytecode[pos];
            result |= ((byte & 0x7F) as u64) << shift;
            len += 1;
            pos += 1;
            if byte < 128 {
                break;
            }
            shift += 7;
            if shift >= 64 {
                break;
            }
        }

        (result, len)
    }

    /// Writes a varint in place, padding with continuation bytes if needed.
    fn write_varint_in_place(&self, bytecode: &mut [u8], pc: usize, old_len: usize, value: u64) {
        // Re-encode `value` into EXACTLY `old_len` bytes so the instruction's
        // byte length is unchanged — an in-place func-id remap must not shift
        // the bytes that follow, because jumps and later instructions are
        // byte-addressed. A varint is a run of continuation bytes (bit 7 set)
        // terminated by one clear byte, so EVERY byte except the last must have
        // bit 7 set; the value's 7-bit groups are zero-extended into the pad.
        //
        // The previous version cleared bit 7 on an interior pad byte, so a
        // remapped id smaller than the original (contiguous index < original
        // sparse id) decoded in FEWER than old_len bytes. The leftover pad byte
        // then mis-aligned the decoder and swallowed the FOLLOWING instruction —
        // dropping the Jmp/JmpNot of future_poll_sync's `match Poll { Ready =>
        // Some, Pending => None }`, so both arms ran straight-line, the last
        // (None) won, and the AOT lowered a reachable `unreachable` → trap.
        debug_assert!(old_len >= 1);
        let mut v = value;
        for i in 0..old_len {
            let mut byte = (v & 0x7F) as u8;
            v >>= 7;
            if i + 1 < old_len {
                byte |= 0x80; // continuation on every byte but the last
            }
            bytecode[pc + i] = byte;
        }
        debug_assert!(
            v == 0,
            "write_varint_in_place: value {} does not fit in {} varint bytes",
            value,
            old_len
        );
    }

    /// Skips a TypeRef in bytecode.
    fn skip_type_ref(&self, bytecode: &[u8], pc: usize, end: usize) -> usize {
        if pc >= end {
            return pc;
        }

        let tag = bytecode[pc];
        let mut pos = pc + 1;

        match tag {
            0 => {
                // Concrete: varint type_id
                let (_, len) = self.read_varint(bytecode, pos);
                pos += len;
            }
            1 => {
                // Generic: u16 param_id
                pos += 2;
            }
            2 => {
                // Instantiated: varint base + u8 arg_count + args
                let (_, len) = self.read_varint(bytecode, pos);
                pos += len;
                if pos < end {
                    let arg_count = bytecode[pos] as usize;
                    pos += 1;
                    for _ in 0..arg_count {
                        pos = self.skip_type_ref(bytecode, pos, end);
                    }
                }
            }
            3
                // Function: u8 param_count + params + return_type
                if pos < end => {
                    let param_count = bytecode[pos] as usize;
                    pos += 1;
                    for _ in 0..param_count {
                        pos = self.skip_type_ref(bytecode, pos, end);
                    }
                    pos = self.skip_type_ref(bytecode, pos, end);
                }
            4 => {
                // Reference: inner + u8 mutability + u8 tier
                pos = self.skip_type_ref(bytecode, pos, end);
                pos += 2;
            }
            5
                // Tuple: u8 elem_count + elems
                if pos < end => {
                    let elem_count = bytecode[pos] as usize;
                    pos += 1;
                    for _ in 0..elem_count {
                        pos = self.skip_type_ref(bytecode, pos, end);
                    }
                }
            6 => {
                // Array: element + varint length
                pos = self.skip_type_ref(bytecode, pos, end);
                let (_, len) = self.read_varint(bytecode, pos);
                pos += len;
            }
            7 => {
                // Slice: element
                pos = self.skip_type_ref(bytecode, pos, end);
            }
            _ => {
                // Unknown - assume no additional data
            }
        }

        pos
    }

    /// Returns the function mapping.
    pub fn mapping(&self) -> &FunctionMapping {
        &self.mapping
    }
}

// ============================================================================
// Incremental Merger
// ============================================================================

/// Incremental module merger for hot-reload scenarios.
///

/// Supports adding new specializations without rebuilding the entire module.
pub struct IncrementalMerger {
    /// Base merged module.
    base: VbcModule,
    /// Accumulated function mapping.
    mapping: FunctionMapping,
    /// Statistics.
    stats: MergeStats,
}

impl IncrementalMerger {
    /// Creates a new incremental merger from a base module.
    pub fn new(base: VbcModule) -> Self {
        let stats = MergeStats {
            user_functions: base.functions.len(),
            bytecode_before: base.bytecode.len(),
            bytecode_after: base.bytecode.len(),
            types_merged: base.types.len(),
            constants_merged: base.constants.len(),
            ..Default::default()
        };

        // Initialize mapping with existing functions
        let mut mapping = FunctionMapping::new();
        for (i, func) in base.functions.iter().enumerate() {
            mapping.add_user(func.id, FunctionId(i as u32));
        }

        Self {
            base,
            mapping,
            stats,
        }
    }

    /// Adds a new specialization to the module.
    pub fn add_specialization(
        &mut self,
        request: &InstantiationRequest,
        specialized: SpecializedFunction,
    ) -> FunctionId {
        let new_offset = self.base.bytecode.len() as u32;

        // Add bytecode
        self.base.bytecode.extend_from_slice(&specialized.bytecode);

        // Add constants
        for constant in specialized.new_constants {
            self.base.constants.push(constant);
        }

        // Create function descriptor
        let new_id = FunctionId(self.base.functions.len() as u32);
        let new_func = FunctionDescriptor {
            id: new_id,
            name: StringId::EMPTY,
            bytecode_offset: new_offset,
            bytecode_length: specialized.bytecode.len() as u32,
            register_count: specialized.register_count,
            locals_count: specialized.locals_count,
            max_stack: specialized.max_stack,
            is_generic: false,
            ..Default::default()
        };

        self.base.functions.push(new_func);

        // Add to specialization table
        self.base.specializations.push(SpecializationEntry {
            generic_fn: request.function_id,
            type_args: request.type_args.clone(),
            hash: request.hash,
            bytecode_offset: new_offset,
            bytecode_length: specialized.bytecode.len() as u32,
            register_count: specialized.register_count,
        });

        // Update mapping and stats
        self.mapping.add_spec(request.hash, new_id);
        self.stats.new_specializations += 1;
        self.stats.bytecode_after = self.base.bytecode.len();

        new_id
    }

    /// Returns the current module.
    pub fn module(&self) -> &VbcModule {
        &self.base
    }

    /// Consumes the merger and returns the module.
    pub fn into_module(mut self) -> VbcModule {
        self.base.update_flags();
        self.base
    }

    /// Returns the function mapping.
    pub fn mapping(&self) -> &FunctionMapping {
        &self.mapping
    }

    /// Returns current statistics.
    pub fn stats(&self) -> &MergeStats {
        &self.stats
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

    #[test]
    fn test_incremental_merger() {
        let module = VbcModule::new("test".to_string());
        let merger = IncrementalMerger::new(module);

        assert_eq!(merger.stats().user_functions, 0);
        assert!(merger.module().bytecode.is_empty());
    }
}
