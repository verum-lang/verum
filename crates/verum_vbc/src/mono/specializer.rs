//! Industrial-grade bytecode specialization for monomorphization.
//!

//! The VBC Specializer transforms generic bytecode into specialized bytecode by:
//! 1. Substituting type parameters with concrete types
//! 2. Rewriting CALL_G to CALL with specialized function IDs
//! 3. Rewriting NEW_G to NEW with concrete type IDs
//! 4. Specializing generic arithmetic (ADD_G → ADD_I/ADD_F)
//! 5. Specializing generic comparison (EQ_G → EQ_I/EQ_F)
//! 6. Computing concrete sizes/alignments for SIZE_OF_G/ALIGN_OF_G
//!

//! Part of the VBC monomorphization pipeline. Operates on the InstantiationGraph built
//! during type checking to produce specialized bytecode with concrete type arguments.

use std::collections::HashMap;

use crate::module::{Constant, FunctionDescriptor, FunctionId, VbcModule};
use crate::types::{ProtocolId, TypeDescriptor, TypeId, TypeParamId, TypeRef};

use super::graph::{InstantiationGraph, InstantiationKey};
use super::substitution::TypeSubstitution;

// ============================================================================
// Specialized Function
// ============================================================================

/// Result of bytecode specialization.
#[derive(Debug, Clone)]
pub struct SpecializedFunction {
    /// Specialized bytecode.
    pub bytecode: Vec<u8>,
    /// Number of registers needed.
    pub register_count: u16,
    /// Number of local variables.
    pub locals_count: u16,
    /// Maximum stack depth.
    pub max_stack: u16,
    /// New constants added during specialization.
    pub new_constants: Vec<Constant>,
    /// task #41: substituted param descriptors — the generic descriptor's params
    /// with type params resolved to concretes (Reference{Generic(T)} →
    /// Reference{Concrete(Int)}). The merger writes these onto the specialized
    /// FunctionDescriptor so the AOT param loop can mark scalar `&T` ref params
    /// (e.g. Deque<Int>.contains's `value: &Int`); without this the specialized
    /// descriptor has empty params and the AOT can't deref a lone `&scalar`.
    pub params: smallvec::SmallVec<[crate::module::ParamDescriptor; 4]>,
    /// task #39/#35: substituted return type — the generic descriptor's
    /// return_type with type params resolved (T → Float64/Float32/…). The merger
    /// writes it onto the specialized FunctionDescriptor so the AOT can
    /// float-mark the call result (mark_register_from_return_type). Without it a
    /// generic `fn fma<T>(...) -> T` monomorphized to Float64 loses the float
    /// mark, so a downstream assert_eq/CmpG compares raw bits (e.g. +0.0 vs
    /// -0.0 signed-zero) instead of via fcmp and fails at Tier-1 only.
    pub return_type: crate::types::TypeRef,
}

// ============================================================================
// Specialization Error
// ============================================================================

/// Error during specialization.
#[derive(Debug, Clone)]
pub enum SpecializationError {
    /// Function not found.
    FunctionNotFound(FunctionId),
    /// Type not found.
    TypeNotFound(TypeId),
    /// Invalid bytecode at a specific offset.
    InvalidBytecode {
        /// Byte offset in the bytecode where the error occurred.
        offset: usize,
        /// Description of the bytecode error.
        message: String,
    },
    /// Unresolved type parameter.
    UnresolvedTypeParam(TypeParamId),
    /// Specialization lookup failed.
    SpecializationNotFound {
        /// The function that failed to specialize.
        function_id: FunctionId,
        /// The type arguments that could not be resolved.
        type_args: Vec<TypeRef>,
    },
}

impl std::fmt::Display for SpecializationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpecializationError::FunctionNotFound(id) => {
                write!(f, "Function not found: {:?}", id)
            }
            SpecializationError::TypeNotFound(id) => {
                write!(f, "Type not found: {:?}", id)
            }
            SpecializationError::InvalidBytecode { offset, message } => {
                write!(f, "Invalid bytecode at offset {}: {}", offset, message)
            }
            SpecializationError::UnresolvedTypeParam(id) => {
                write!(f, "Unresolved type parameter: {:?}", id)
            }
            SpecializationError::SpecializationNotFound {
                function_id,
                type_args,
            } => {
                write!(
                    f,
                    "Specialization not found for {:?} with {:?}",
                    function_id, type_args
                )
            }
        }
    }
}

impl std::error::Error for SpecializationError {}

// ============================================================================
// Type Size/Alignment Info
// ============================================================================

/// Type layout information for specialization.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypeLayout {
    /// Size in bytes.
    pub size: u32,
    /// Alignment in bytes.
    pub alignment: u32,
}

impl TypeLayout {
    /// Layout for built-in types.
    pub fn for_builtin(type_id: TypeId) -> Self {
        match type_id.0 {
            0 => Self {
                size: 0,
                alignment: 1,
            }, // UNIT
            1 => Self {
                size: 1,
                alignment: 1,
            }, // BOOL
            2 => Self {
                size: 8,
                alignment: 8,
            }, // INT
            3 => Self {
                size: 8,
                alignment: 8,
            }, // FLOAT
            4 => Self {
                size: 24,
                alignment: 8,
            }, // TEXT (ptr + len + cap)
            5 => Self {
                size: 0,
                alignment: 1,
            }, // NEVER
            6 => Self {
                size: 1,
                alignment: 1,
            }, // U8
            7 => Self {
                size: 2,
                alignment: 2,
            }, // U16
            8 => Self {
                size: 4,
                alignment: 4,
            }, // U32
            9 => Self {
                size: 8,
                alignment: 8,
            }, // U64
            10 => Self {
                size: 1,
                alignment: 1,
            }, // I8
            11 => Self {
                size: 2,
                alignment: 2,
            }, // I16
            12 => Self {
                size: 4,
                alignment: 4,
            }, // I32
            13 => Self {
                size: 4,
                alignment: 4,
            }, // F32
            14 => Self {
                size: 8,
                alignment: 8,
            }, // PTR
            _ => Self {
                size: 8,
                alignment: 8,
            }, // Default for user types
        }
    }
}

// ============================================================================
// Specialization Statistics
// ============================================================================

/// Statistics from specialization.
#[derive(Debug, Clone, Default)]
pub struct SpecializerStats {
    /// Number of CALL_G instructions specialized.
    pub call_g_specialized: usize,
    /// Number of NEW_G instructions specialized.
    pub new_g_specialized: usize,
    /// Number of ADD_G/SUB_G/MUL_G/DIV_G instructions specialized.
    pub arith_g_specialized: usize,
    /// Number of EQ_G/CMP_G instructions specialized.
    pub cmp_g_specialized: usize,
    /// Number of SIZE_OF_G/ALIGN_OF_G instructions specialized.
    pub sizeof_g_specialized: usize,
    /// Number of CALL_V devirtualized.
    pub call_v_devirtualized: usize,
    /// Number of builtin `ToString` sites expanded into the concrete
    /// type's Display chain (L2, task #44).
    pub to_string_displayed: usize,
    /// Total instructions processed.
    pub total_instructions: usize,
    /// Bytes input.
    pub bytes_input: usize,
    /// Bytes output.
    pub bytes_output: usize,
}

// ============================================================================
// VBC Specializer
// ============================================================================

/// Industrial-grade bytecode specializer for generic functions.
///

/// Transforms generic VBC bytecode to specialized bytecode by:
/// - Substituting type parameters with concrete types
/// - Rewriting generic calls to direct calls
/// - Specializing generic operations to typed variants
/// - Devirtualizing CALL_V when receiver type is monomorphic
pub struct BytecodeSpecializer<'a> {
    /// Source module containing generic functions.
    module: &'a VbcModule,
    /// Type substitution to apply.
    substitution: &'a TypeSubstitution,
    /// Instantiation graph for looking up specialized functions.
    graph: &'a InstantiationGraph,
    /// Type layouts for SIZE_OF_G/ALIGN_OF_G.
    type_layouts: HashMap<TypeId, TypeLayout>,
    /// Instantiated type cache: (base_type, type_args) -> concrete TypeId.
    instantiated_types: HashMap<(TypeId, Vec<TypeRef>), TypeId>,
    /// Next type ID for new instantiated types.
    next_type_id: u32,
    /// Register type tracking for devirtualization.
    /// Maps register index -> known type at that register.
    register_types: HashMap<u16, TypeRef>,
    /// New constants generated during specialization.
    new_constants: Vec<Constant>,
    /// New type descriptors generated during specialization.
    new_type_descriptors: Vec<TypeDescriptor>,
    /// Statistics.
    stats: SpecializerStats,
}

impl<'a> BytecodeSpecializer<'a> {
    /// Creates a new bytecode specializer.
    pub fn new(
        module: &'a VbcModule,
        substitution: &'a TypeSubstitution,
        graph: &'a InstantiationGraph,
    ) -> Self {
        // Pre-populate type layouts from module
        let mut type_layouts = HashMap::new();
        let mut max_type_id = 0u32;
        for type_desc in &module.types {
            type_layouts.insert(
                type_desc.id,
                TypeLayout {
                    size: type_desc.size,
                    alignment: type_desc.alignment,
                },
            );
            max_type_id = max_type_id.max(type_desc.id.0);
        }

        Self {
            module,
            substitution,
            graph,
            type_layouts,
            instantiated_types: HashMap::new(),
            next_type_id: max_type_id + 1,
            register_types: HashMap::new(),
            new_constants: Vec::new(),
            new_type_descriptors: Vec::new(),
            stats: SpecializerStats::default(),
        }
    }

    /// Returns new type descriptors generated during specialization.
    pub fn take_new_type_descriptors(&mut self) -> Vec<TypeDescriptor> {
        std::mem::take(&mut self.new_type_descriptors)
    }

    /// Specializes a function's bytecode.
    ///

    /// Returns the specialized function or an error.
    pub fn specialize(
        &mut self,
        func: &FunctionDescriptor,
        _type_args: &[TypeRef],
    ) -> Result<SpecializedFunction, SpecializationError> {
        let start = func.bytecode_offset as usize;
        let end = start + func.bytecode_length as usize;

        if end > self.module.bytecode.len() {
            return Err(SpecializationError::InvalidBytecode {
                offset: start,
                message: format!(
                    "Bytecode range {}..{} exceeds module size {}",
                    start,
                    end,
                    self.module.bytecode.len()
                ),
            });
        }

        let bytecode = &self.module.bytecode[start..end];
        self.stats.bytes_input = bytecode.len();

        let trace_this = std::env::var_os("VERUM_TRACE_MONO").is_some()
            && self
                .module
                .get_string(func.name)
                .is_some_and(|n| n.contains("future_poll_sync"));
        if trace_this {
            eprintln!(
                "[mono-spec-body] specializing id={} '{}' bytecode_len={} first_in_bytes={:02x?}",
                func.id.0,
                self.module.get_string(func.name).unwrap_or("?"),
                bytecode.len(),
                &bytecode[..bytecode.len().min(20)]
            );
        }

        // **L2/L3 STRUCTURAL DISCIPLINE (task #44)** — the previous
        // per-opcode BYTE walkers had their own wire grammar, which had
        // drifted from the canonical codec on every call-family
        // instruction (CallG read `type_arg_count` as u8 vs varint and
        // args as count+regs vs RegRange; CallV read a protocol/method
        // pair the wire does not carry; NewG read static TypeRefs where
        // the wire carries REGISTERS).  Worse, rewrites that changed
        // byte length (CallG→Call) never recomputed jump offsets — any
        // jump across a specialized site landed mid-instruction.  That
        // was the gate-ON crash constellation.
        //
        // Canonical pipeline instead: decode the WHOLE body with the
        // one true codec → convert jump offsets to instruction-index
        // form → transform instruction VALUES → recompute byte offsets
        // against the new widths → encode.  Undecodable bodies fail
        // typed (same contract as before).
        let mut instrs = crate::bytecode::decode_instructions(bytecode).map_err(|e| {
            SpecializationError::InvalidBytecode {
                offset: start,
                message: format!("canonical decode failed: {e:?}"),
            }
        })?;
        crate::bytecode::jump_offsets_to_instr_indices(&mut instrs);
        for instr in instrs.iter_mut() {
            self.stats.total_instructions += 1;
            self.specialize_instr_value(instr);
        }
        // **L2 (task #44): ToString → Display expansion.**  With T
        // substituted concrete, a `f"{x}"` in the generic body should
        // route through the type's own `fmt` exactly like a
        // non-generic body would — the codegen made the builtin
        // `ToString` decision BEFORE mono existed to overturn it.
        // Write-once register typing from the SUBSTITUTED param
        // descriptors (sound without flow analysis) finds the sites;
        // the structural pipeline makes the 1→7 instruction expansion
        // legal (jump index offsets are remapped around insertions).
        let extra_regs = self.expand_display_tostrings(&mut instrs, func);
        crate::bytecode::fixup_jump_offsets(&mut instrs);
        let mut output: Vec<u8> = Vec::with_capacity(bytecode.len() + 16);
        for instr in instrs.iter() {
            crate::bytecode::encode_instruction(instr, &mut output);
        }

        self.stats.bytes_output = output.len();

        if trace_this {
            eprintln!(
                "[mono-spec-body] id={} OUTPUT len={} first_out_bytes={:02x?}",
                func.id.0,
                output.len(),
                &output[..output.len().min(20)]
            );
        }

        // task #41: carry the substituted param descriptors so the merged
        // specialized FunctionDescriptor exposes them to the AOT param loop
        // (Reference{Generic(T)} → Reference{Concrete(Int)} via the substitution).
        let params = func
            .params
            .iter()
            .map(|p| {
                let mut np = p.clone();
                np.type_ref = self.substitution.apply(&p.type_ref);
                np
            })
            .collect();
        // task #39/#35: substitute the return type so the merged descriptor lets
        // the AOT float-mark the call result of a generic fn returning T.
        let return_type = self.substitution.apply(&func.return_type);

        Ok(SpecializedFunction {
            bytecode: output,
            register_count: func.register_count.saturating_add(extra_regs),
            locals_count: func.locals_count,
            max_stack: func.max_stack,
            new_constants: std::mem::take(&mut self.new_constants),
            params,
            return_type,
        })
    }

    /// Specializes a CALL_G instruction.
    ///

    /// Specializes a CALL_V instruction.
    ///

    /// CALL_V format: dst:reg receiver:reg protocol:varint method_idx:u8 arg_count:u8 [args:reg...]
    ///

    /// Resolve a `dyn:Protocol.method` method-id string to the concrete
    /// `ConcreteType.method` string id via the substitution's primary type
    /// parameter.  Returns None if the token is not a dyn-dispatch, the primary
    /// type isn't concrete, or the concrete method function is absent.
    /// Value-form specialization of one canonical instruction
    /// (task #44 L2/L3 structural discipline — see `specialize`).
    /// Mutates in place; instructions with no generic content pass
    /// through untouched.
    fn specialize_instr_value(&mut self, instr: &mut crate::instruction::Instruction) {
        use crate::instruction::Instruction as I;
        match instr {
            I::CallG {
                dst,
                func_id,
                type_args,
                args,
            } => {
                let substituted: Vec<TypeRef> = type_args
                    .iter()
                    .map(|t| self.substitution.apply(t))
                    .collect();
                let key = InstantiationKey::new(FunctionId(*func_id), substituted.clone());
                match self.graph.get_specialization_by_key(&key) {
                    Some(spec_fn) => {
                        // Direct call to the specialization.  The
                        // RegRange arg block carries over verbatim —
                        // receiver/argument contiguity is preserved by
                        // construction.
                        *instr = I::Call {
                            dst: *dst,
                            func_id: spec_fn.0,
                            args: *args,
                        };
                        self.stats.call_g_specialized += 1;
                    }
                    None => {
                        // No exact instantiation — keep the CallG but
                        // carry the SUBSTITUTED type args so a later
                        // merge round (or the runtime witness path)
                        // sees concrete types instead of stale
                        // Generic(i) placeholders.
                        *type_args = substituted;
                    }
                }
            }
            I::CallM { method_id, .. } => {
                if let Some(devirt) = self.devirt_dyn_method_id(*method_id) {
                    *method_id = devirt;
                }
            }
            I::BinaryG {
                op,
                dst,
                a,
                b,
                protocol_id: _,
            } => {
                use crate::instruction::{BinaryFloatOp, BinaryGenericOp, BinaryIntOp};
                let operand_type = self.infer_operand_type_from_context();
                let concrete = match &operand_type {
                    Some(TypeRef::Concrete(id)) if id.is_integer() || id.0 == TypeId::INT.0 => {
                        Some(false)
                    }
                    Some(TypeRef::Concrete(id)) if id.is_float() || id.0 == TypeId::FLOAT.0 => {
                        Some(true)
                    }
                    _ => None,
                };
                if let Some(is_float) = concrete {
                    let (iop, fop) = match op {
                        BinaryGenericOp::Add => (BinaryIntOp::Add, BinaryFloatOp::Add),
                        BinaryGenericOp::Sub => (BinaryIntOp::Sub, BinaryFloatOp::Sub),
                        BinaryGenericOp::Mul => (BinaryIntOp::Mul, BinaryFloatOp::Mul),
                        BinaryGenericOp::Div => (BinaryIntOp::Div, BinaryFloatOp::Div),
                    };
                    *instr = if is_float {
                        I::BinaryF {
                            op: fop,
                            dst: *dst,
                            a: *a,
                            b: *b,
                        }
                    } else {
                        I::BinaryI {
                            op: iop,
                            dst: *dst,
                            a: *a,
                            b: *b,
                        }
                    };
                    self.stats.arith_g_specialized += 1;
                }
            }
            I::CmpG {
                eq: true,
                dst,
                a,
                b,
                protocol_id: _,
            } => {
                use crate::instruction::CompareOp;
                let operand_type = self.infer_operand_type_from_context();
                match &operand_type {
                    Some(TypeRef::Concrete(id))
                        if id.is_integer() || id.0 == TypeId::INT.0 || id.0 == TypeId::BOOL.0 =>
                    {
                        *instr = I::CmpI {
                            op: CompareOp::Eq,
                            dst: *dst,
                            a: *a,
                            b: *b,
                        };
                        self.stats.cmp_g_specialized += 1;
                    }
                    Some(TypeRef::Concrete(id)) if id.is_float() => {
                        *instr = I::CmpF {
                            op: CompareOp::Eq,
                            dst: *dst,
                            a: *a,
                            b: *b,
                        };
                        self.stats.cmp_g_specialized += 1;
                    }
                    _ => {}
                }
            }
            I::LoadT { type_ref, .. } => {
                *type_ref = self.substitution.apply(type_ref);
            }
            // NewG carries RUNTIME type-argument REGISTERS on the wire
            // (the old byte arm misread them as static TypeRefs and
            // rewrote the instruction from garbage).  A static rewrite
            // needs a witness-aware design — passthrough until then.
            // CallV carries a vtable slot, not the (protocol, method)
            // pair the old arm assumed — devirtualization by slot needs
            // the vtable layout carried into the specializer;
            // passthrough keeps runtime dispatch correct.
            _ => {}
        }
    }

    /// **L2 (task #44)**: expand builtin `ToString` sites whose source
    /// register provably (write-once) holds a substituted-concrete
    /// value of a type with its own `fmt` into the canonical Display
    /// chain the non-generic codegen would have emitted:
    ///
    /// ```text
    /// buf       = Call Text.new()
    /// r_buf     = RefObj buf
    /// formatter = Call Formatter.new(r_buf)
    /// a0        = Mov src
    /// a1        = RefObj formatter
    /// _         = Call <T>.fmt(a0, a1)
    /// dst       = Mov buf
    /// ```
    ///
    /// Returns the number of EXTRA registers the expansions use (the
    /// caller bumps the specialized descriptor's register_count).
    /// Sites that don't resolve (no write-once type, no `<T>.fmt`
    /// arity-2 body, missing Text.new/Formatter.new) keep the builtin
    /// `ToString` — today's behaviour, honest degradation.
    fn expand_display_tostrings(
        &mut self,
        instrs: &mut Vec<crate::instruction::Instruction>,
        func: &FunctionDescriptor,
    ) -> u16 {
        use crate::instruction::Instruction as I;
        use crate::instruction::Reg;

        // ---- write-once register typing from substituted params ----
        let mut write_counts: std::collections::HashMap<u16, u32> =
            std::collections::HashMap::new();
        for instr in instrs.iter() {
            if let Some(d) = Self::written_reg(instr) {
                *write_counts.entry(d.0).or_insert(0) += 1;
            }
        }
        let mut reg_types: std::collections::HashMap<u16, TypeRef> =
            std::collections::HashMap::new();
        for (i, p) in func.params.iter().enumerate() {
            let substituted = self.substitution.apply(&p.type_ref);
            // Param registers are written by the CALL frame, not the
            // body; a body write means reuse — drop the fact.
            if write_counts.get(&(i as u16)).copied().unwrap_or(0) == 0 {
                reg_types.insert(i as u16, substituted);
            }
        }
        // One forward propagation over write-once Movs (chains resolve
        // because sources settle before their single-write consumers
        // in a second sweep).
        for _ in 0..2 {
            for instr in instrs.iter() {
                if let I::Mov { dst, src } = instr
                    && write_counts.get(&dst.0).copied().unwrap_or(0) == 1
                    && let Some(t) = reg_types.get(&src.0).cloned()
                {
                    reg_types.insert(dst.0, t);
                }
            }
        }

        // ---- collect expandable sites ----
        let mut sites: Vec<(usize, u16, u32)> = Vec::new(); // (idx, src_reg, fmt_fid)
        for (idx, instr) in instrs.iter().enumerate() {
            let I::ToString { src, .. } = instr else { continue };
            let Some(TypeRef::Concrete(tid)) = reg_types.get(&src.0) else {
                continue;
            };
            // Primitives keep the builtin formatter (identical text,
            // one opcode instead of seven).
            if tid.is_builtin() {
                continue;
            }
            let Some(type_name) = self.type_name_of(*tid) else { continue };
            let fmt_name = format!("{}.fmt", type_name);
            let Some(fmt_fid) = self.module.find_function_by_name(&fmt_name) else {
                continue;
            };
            sites.push((idx, src.0, fmt_fid.0));
        }
        if sites.is_empty() {
            return 0;
        }
        let (Some(text_new), Some(formatter_new)) = (
            self.module.find_function_by_name("Text.new"),
            self.module.find_function_by_name("Formatter.new"),
        ) else {
            return 0;
        };

        // ---- jump bookkeeping: absolute targets before insertion ----
        let mut abs_targets: Vec<Option<i64>> = Vec::with_capacity(instrs.len());
        for (idx, instr) in instrs.iter().enumerate() {
            abs_targets.push(Self::jump_offset_of(instr).map(|o| idx as i64 + o as i64));
        }

        // ---- expand, back to front (indices stay valid) ----
        let base = func.register_count;
        let mut expansions = 0u16;
        // position map: old index -> shift accumulated AFTER insertions
        let mut inserted_at: Vec<(usize, usize)> = Vec::new(); // (old_idx, count_inserted)
        for &(idx, src_reg, fmt_fid) in sites.iter().rev() {
            let I::ToString { dst, .. } = instrs[idx] else { continue };
            let r = |k: u16| Reg(base + expansions * 6 + k);
            let buf = r(0);
            let buf_ref = r(1);
            let formatter = r(2);
            let a0 = r(3);
            let a1 = r(4);
            let res = r(5);
            let seq = vec![
                I::Call {
                    dst: buf,
                    func_id: text_new.0,
                    args: crate::instruction::RegRange { start: Reg(0), count: 0 },
                },
                I::RefObj { dst: buf_ref, src: buf },
                I::Call {
                    dst: formatter,
                    func_id: formatter_new.0,
                    args: crate::instruction::RegRange { start: buf_ref, count: 1 },
                },
                I::Mov { dst: a0, src: Reg(src_reg) },
                I::RefObj { dst: a1, src: formatter },
                I::Call {
                    dst: res,
                    func_id: fmt_fid,
                    args: crate::instruction::RegRange { start: a0, count: 2 },
                },
                I::Mov { dst, src: buf },
            ];
            let n = seq.len();
            instrs.splice(idx..idx + 1, seq);
            inserted_at.push((idx, n - 1)); // net growth
            expansions += 1;
            self.stats.to_string_displayed += 1;
        }

        // ---- remap jump index offsets across insertions ----
        let shift_for = |old_idx: usize| -> i64 {
            inserted_at
                .iter()
                .map(|&(at, n)| if old_idx > at { n as i64 } else { 0 })
                .sum()
        };
        for (old_idx, tgt) in abs_targets.iter().enumerate() {
            let Some(abs) = tgt else { continue };
            let new_idx = old_idx as i64 + shift_for(old_idx);
            // Target shifts when it lies strictly beyond an insertion
            // point; a target AT the site (jump to the expanded
            // instruction) lands on the expansion's first instruction.
            let new_abs = *abs + shift_for((*abs).max(0) as usize);
            let new_off = (new_abs - new_idx) as i32;
            if let Some(slot) = Self::jump_offset_slot(&mut instrs[new_idx as usize]) {
                *slot = new_off;
            }
        }

        expansions * 6
    }

    /// Register written by an instruction, for the write-once census.
    /// Conservative: instructions with multi-reg or indirect writes
    /// return None (their dsts simply never qualify as typed).
    fn written_reg(instr: &crate::instruction::Instruction) -> Option<crate::instruction::Reg> {
        use crate::instruction::Instruction as I;
        match instr {
            I::Mov { dst, .. }
            | I::LoadI { dst, .. }
            | I::LoadK { dst, .. }
            | I::Call { dst, .. }
            | I::CallM { dst, .. }
            | I::CallG { dst, .. }
            | I::ToString { dst, .. }
            | I::Concat { dst, .. }
            | I::GetF { dst, .. }
            | I::RefObj { dst, .. } => Some(*dst),
            _ => None,
        }
    }

    fn jump_offset_of(instr: &crate::instruction::Instruction) -> Option<i32> {
        use crate::instruction::Instruction as I;
        match instr {
            I::Jmp { offset }
            | I::JmpIf { offset, .. }
            | I::JmpNot { offset, .. }
            | I::JmpCmp { offset, .. } => Some(*offset),
            I::CtxProvide { body_offset, .. } => Some(*body_offset),
            I::TryBegin { handler_offset } => Some(*handler_offset),
            _ => None,
        }
    }

    fn jump_offset_slot(instr: &mut crate::instruction::Instruction) -> Option<&mut i32> {
        use crate::instruction::Instruction as I;
        match instr {
            I::Jmp { offset }
            | I::JmpIf { offset, .. }
            | I::JmpNot { offset, .. }
            | I::JmpCmp { offset, .. } => Some(offset),
            I::CtxProvide { body_offset, .. } => Some(body_offset),
            I::TryBegin { handler_offset } => Some(handler_offset),
            _ => None,
        }
    }

    /// Resolve a TypeId to its declared name via the module type table.
    fn type_name_of(&self, tid: TypeId) -> Option<String> {
        self.module
            .types
            .iter()
            .find(|t| t.id == tid)
            .and_then(|t| self.module.get_string(t.name))
            .map(|s| {
                // Strip module qualification: `core.time.Duration` → `Duration`
                s.rsplit('.').next().unwrap_or(s).to_string()
            })
    }

    fn devirt_dyn_method_id(&self, method_id: u32) -> Option<u32> {
        let name = self
            .module
            .get_string(crate::types::StringId(method_id))?
            .to_string();
        // Extract the method name from either a `dyn:Protocol.method` token or
        // a BARE `method` (a protocol-method call on a type parameter whose
        // receiver's concrete type is only known after substitution — e.g.
        // `future.poll()` in `future_poll_sync<F: Future>` compiles to a
        // CALL_M with the bare method name "poll").  An already-concrete
        // `Type.method` is left untouched.
        let method: &str = if let Some(rest) = name.strip_prefix("dyn:") {
            rest.rsplit('.').next()?
        } else if !name.contains('.') {
            name.as_str()
        } else {
            return None;
        };
        // Devirtualize on the receiver's BASE type. A monomorphized receiver
        // like `ReadyFuture<Text>` is carried as `Instantiated { base, args }`
        // (the args preserve the payload type for associated-type resolution);
        // the concrete method lives on the base `ReadyFuture.poll`.
        let tid = match self.substitution.get(TypeParamId(0))? {
            TypeRef::Concrete(id) => id,
            TypeRef::Instantiated { base, .. } => base,
            _ => return None,
        };
        let type_name = self.module.get_type_name(*tid)?;
        let concrete = format!("{}.{}", type_name, method);
        if std::env::var_os("VERUM_TRACE_MONO").is_some() {
            let hit = self
                .module
                .functions
                .iter()
                .any(|f| self.module.get_string(f.name).is_some_and(|s| s == concrete));
            eprintln!(
                "[mono-callm] dyn='{}' -> concrete='{}' found={}",
                name, concrete, hit
            );
        }
        self.module
            .functions
            .iter()
            .find(|f| self.module.get_string(f.name).is_some_and(|s| s == concrete))
            .map(|f| f.name.0)
    }

    /// Looks up a protocol implementation for a concrete type.
    ///

    /// Returns the function ID for the method if the type implements the protocol.
    fn lookup_protocol_impl(
        &self,
        type_ref: &TypeRef,
        protocol_id: ProtocolId,
        method_idx: u8,
    ) -> Option<FunctionId> {
        // Get concrete type ID
        let type_id = match type_ref {
            TypeRef::Concrete(id) => *id,
            TypeRef::Instantiated { base, .. } => *base,
            _ => return None,
        };

        // Look up type descriptor in module
        let type_desc = self.module.types.iter().find(|td| td.id == type_id)?;

        // Find protocol implementation
        let proto_impl = type_desc
            .protocols
            .iter()
            .find(|pi| pi.protocol == protocol_id)?;

        // Get method function ID by index
        let method_fn_id = proto_impl.methods.get(method_idx as usize)?;

        Some(FunctionId(*method_fn_id))
    }

    /// Specializes a NEW_G instruction.
    ///

    /// Specializes generic arithmetic operations.
    ///

    /// Specializes generic comparison operations.
    ///

    /// Specializes SIZE_OF_G instruction.
    ///

    // ========================================================================
    // Type Layout Helpers
    // ========================================================================

    /// Gets type size for a TypeRef.
    fn get_type_size(&self, type_ref: &TypeRef) -> u32 {
        match type_ref {
            TypeRef::Concrete(id) => {
                if id.is_builtin() {
                    TypeLayout::for_builtin(*id).size
                } else {
                    self.type_layouts.get(id).map_or(8, |l| l.size)
                }
            }
            TypeRef::Instantiated { base, .. } => self.type_layouts.get(base).map_or(8, |l| l.size),
            TypeRef::Reference { .. } => 16, // ThinRef size
            TypeRef::Tuple(elements) => elements.iter().map(|e| self.get_type_size(e)).sum(),
            TypeRef::Array { element, length } => self.get_type_size(element) * (*length as u32),
            TypeRef::Slice(_) => 16,            // ptr + len
            TypeRef::Function { .. } => 8,      // function pointer
            TypeRef::Rank2Function { .. } => 8, // function pointer (rank-2 polymorphic)
            TypeRef::Generic(_) => 8,           // Should be substituted by now
            TypeRef::AssociatedProjection { .. } => 8, // resolved before mono
        }
    }

    /// Gets type alignment for a TypeRef.
    fn get_type_alignment(&self, type_ref: &TypeRef) -> u32 {
        match type_ref {
            TypeRef::Concrete(id) => {
                if id.is_builtin() {
                    TypeLayout::for_builtin(*id).alignment
                } else {
                    self.type_layouts.get(id).map_or(8, |l| l.alignment)
                }
            }
            TypeRef::Instantiated { base, .. } => {
                self.type_layouts.get(base).map_or(8, |l| l.alignment)
            }
            TypeRef::Reference { .. } => 8,
            TypeRef::Tuple(elements) => elements
                .iter()
                .map(|e| self.get_type_alignment(e))
                .max()
                .unwrap_or(1),
            TypeRef::Array { element, .. } => self.get_type_alignment(element),
            TypeRef::Slice(_) => 8,
            TypeRef::Function { .. } => 8,
            TypeRef::Rank2Function { .. } => 8,
            TypeRef::Generic(_) => 8,
            TypeRef::AssociatedProjection { .. } => 8, // resolved before mono
        }
    }

    /// Gets or creates an instantiated type.
    ///

    /// This is a CRITICAL operation for monomorphization correctness.
    /// Creates a new concrete type for a generic type instantiated with specific type arguments.
    fn get_or_create_instantiated_type(&mut self, base: TypeId, args: &[TypeRef]) -> TypeId {
        // Check cache first
        let key = (base, args.to_vec());
        if let Some(&id) = self.instantiated_types.get(&key) {
            return id;
        }

        // If no type args, just return base type
        if args.is_empty() {
            self.instantiated_types.insert(key, base);
            return base;
        }

        // Check if all args are concrete - if any are generic, we can't fully instantiate
        let all_concrete = args
            .iter()
            .all(|arg| matches!(arg, TypeRef::Concrete(_) | TypeRef::Instantiated { .. }));

        if !all_concrete {
            // Can't fully instantiate - return base type
            self.instantiated_types.insert(key, base);
            return base;
        }

        // Create new instantiated type descriptor
        let new_id = TypeId(self.next_type_id);
        self.next_type_id += 1;

        // Get base type descriptor to inherit properties
        let base_desc = self.module.types.iter().find(|td| td.id == base);

        // Compute size and alignment based on type args
        let (size, alignment) = self.compute_instantiated_layout(base, args);

        // Create new type descriptor (note: TypeDescriptor doesn't have type_args/generic_base)
        // The instantiation relationship is tracked in our instantiated_types cache instead
        let new_type_desc = TypeDescriptor {
            id: new_id,
            name: base_desc.map(|d| d.name).unwrap_or_default(),
            kind: base_desc.map(|d| d.kind).unwrap_or_default(),
            size,
            alignment,
            fields: base_desc.map(|d| d.fields.clone()).unwrap_or_default(),
            variants: base_desc.map(|d| d.variants.clone()).unwrap_or_default(),
            type_params: smallvec::smallvec![], // No params - fully instantiated
            drop_fn: base_desc.and_then(|d| d.drop_fn),
            clone_fn: base_desc.and_then(|d| d.clone_fn),
            protocols: base_desc.map(|d| d.protocols.clone()).unwrap_or_default(),
            visibility: base_desc.map(|d| d.visibility).unwrap_or_default(),
            alias_target: base_desc.and_then(|d| d.alias_target.clone()),
            // Specialised types inherit the wrapper policy of the base.
            // `List<Int>` mono'd from `List<T>` stays a record;
            // `Meters<Int>` (hypothetical) from a transparent base
            // stays transparent.  See
            // `TypeDescriptor::is_transparent_wrapper`.
            is_transparent_wrapper: base_desc
                .map(|d| d.is_transparent_wrapper)
                .unwrap_or(false),
        };

        // Update layout cache
        self.type_layouts
            .insert(new_id, TypeLayout { size, alignment });

        // Store new type descriptor
        self.new_type_descriptors.push(new_type_desc);

        // Cache the result
        self.instantiated_types.insert(key, new_id);

        new_id
    }

    /// Computes size and alignment for an instantiated generic type.
    fn compute_instantiated_layout(&self, base: TypeId, args: &[TypeRef]) -> (u32, u32) {
        // Get base type layout
        let base_layout = self.type_layouts.get(&base).copied().unwrap_or(TypeLayout {
            size: 8,
            alignment: 8,
        });

        // For simple cases, inherit from base
        // For complex cases (structs with generic fields), need to compute based on args
        if let Some(base_desc) = self.module.types.iter().find(|td| td.id == base) {
            // Check if any field uses a type parameter
            let has_generic_fields = base_desc
                .fields
                .iter()
                .any(|f| matches!(f.type_ref, TypeRef::Generic(_)));

            if has_generic_fields && !args.is_empty() {
                // Compute actual layout based on substituted field types
                let mut total_size: u32 = 0;
                let mut max_alignment: u32 = 1;

                for field in &base_desc.fields {
                    // Substitute type params
                    let field_type = self.substitution.apply(&field.type_ref);
                    let field_size = self.get_type_size(&field_type);
                    let field_align = self.get_type_alignment(&field_type);

                    // Align current offset
                    total_size = (total_size + field_align - 1) & !(field_align - 1);
                    total_size += field_size;
                    max_alignment = max_alignment.max(field_align);
                }

                // Final alignment padding
                total_size = (total_size + max_alignment - 1) & !(max_alignment - 1);

                return (total_size, max_alignment);
            }
        }

        // Fallback: use base type's layout
        (base_layout.size, base_layout.alignment)
    }

    /// Infers operand type from substitution context.
    ///

    /// Checks all type parameters in the substitution to find a concrete numeric type
    /// that can be used for arithmetic specialization.
    fn infer_operand_type_from_context(&self) -> Option<TypeRef> {
        // Check all type parameters, not just the first one
        // Priority: look for common patterns like T, Element, Item, Value
        for param_id in 0..16 {
            if let Some(type_ref) = self.substitution.get(TypeParamId(param_id)) {
                // Only return if it's a concrete numeric type
                if let TypeRef::Concrete(id) = type_ref
                    && id.is_numeric()
                {
                    return Some(type_ref.clone());
                }
            }
        }

        // Fallback: return the first available concrete type
        for param_id in 0..16 {
            if let Some(type_ref) = self.substitution.get(TypeParamId(param_id))
                && matches!(type_ref, TypeRef::Concrete(_))
            {
                return Some(type_ref.clone());
            }
        }

        None
    }

    /// Updates register type tracking after an instruction.
    ///

    /// This is used for devirtualization - knowing the concrete type of a register
    /// allows us to resolve virtual calls statically.
    fn _track_register_type(&mut self, dst: u16, type_ref: TypeRef) {
        self.register_types.insert(dst, type_ref);
    }

    // ========================================================================
    // Bytecode Reading Helpers
    // ========================================================================

    /// Reads a register from bytecode.
    fn read_reg(&self, bytecode: &[u8], pc: &mut usize) -> Result<u16, SpecializationError> {
        if *pc >= bytecode.len() {
            return Err(SpecializationError::InvalidBytecode {
                offset: *pc,
                message: "Unexpected end of bytecode reading register".to_string(),
            });
        }

        let byte = bytecode[*pc];
        *pc += 1;

        if byte < 128 {
            Ok(byte as u16)
        } else {
            if *pc >= bytecode.len() {
                return Err(SpecializationError::InvalidBytecode {
                    offset: *pc,
                    message: "Unexpected end of bytecode reading extended register".to_string(),
                });
            }
            let high = (byte & 0x7F) as u16;
            let low = bytecode[*pc] as u16;
            *pc += 1;
            Ok((high << 8) | low)
        }
    }

    /// Reads a varint from bytecode.
    fn read_varint(&self, bytecode: &[u8], pc: &mut usize) -> Result<u64, SpecializationError> {
        let mut result: u64 = 0;
        let mut shift = 0;

        loop {
            if *pc >= bytecode.len() {
                return Err(SpecializationError::InvalidBytecode {
                    offset: *pc,
                    message: "Unexpected end of bytecode reading varint".to_string(),
                });
            }

            let byte = bytecode[*pc];
            *pc += 1;

            result |= ((byte & 0x7F) as u64) << shift;
            if byte < 128 {
                break;
            }
            shift += 7;
            if shift >= 64 {
                return Err(SpecializationError::InvalidBytecode {
                    offset: *pc,
                    message: "Varint overflow".to_string(),
                });
            }
        }

        Ok(result)
    }

    /// Reads a TypeRef from bytecode.
    fn read_type_ref(
        &self,
        bytecode: &[u8],
        pc: &mut usize,
    ) -> Result<TypeRef, SpecializationError> {
        if *pc >= bytecode.len() {
            return Err(SpecializationError::InvalidBytecode {
                offset: *pc,
                message: "Unexpected end of bytecode reading type tag".to_string(),
            });
        }

        let tag = bytecode[*pc];
        *pc += 1;

        match tag {
            0 => {
                // Concrete
                let type_id = self.read_varint(bytecode, pc)? as u32;
                Ok(TypeRef::Concrete(TypeId(type_id)))
            }
            1 => {
                // Generic
                if *pc + 1 >= bytecode.len() {
                    return Err(SpecializationError::InvalidBytecode {
                        offset: *pc,
                        message: "Unexpected end reading generic type param".to_string(),
                    });
                }
                let param_id = bytecode[*pc] as u16 | ((bytecode[*pc + 1] as u16) << 8);
                *pc += 2;
                Ok(TypeRef::Generic(TypeParamId(param_id)))
            }
            2 => {
                // Instantiated
                let base = self.read_varint(bytecode, pc)? as u32;
                if *pc >= bytecode.len() {
                    return Err(SpecializationError::InvalidBytecode {
                        offset: *pc,
                        message: "Unexpected end reading instantiated arg count".to_string(),
                    });
                }
                let arg_count = bytecode[*pc] as usize;
                *pc += 1;
                let mut args = Vec::with_capacity(arg_count);
                for _ in 0..arg_count {
                    args.push(self.read_type_ref(bytecode, pc)?);
                }
                Ok(TypeRef::Instantiated {
                    base: TypeId(base),
                    args,
                })
            }
            3 => {
                // Function
                if *pc >= bytecode.len() {
                    return Err(SpecializationError::InvalidBytecode {
                        offset: *pc,
                        message: "Unexpected end reading function param count".to_string(),
                    });
                }
                let param_count = bytecode[*pc] as usize;
                *pc += 1;
                let mut params = Vec::with_capacity(param_count);
                for _ in 0..param_count {
                    params.push(self.read_type_ref(bytecode, pc)?);
                }
                let return_type = Box::new(self.read_type_ref(bytecode, pc)?);
                Ok(TypeRef::Function {
                    params,
                    return_type,
                    contexts: smallvec::smallvec![],
                })
            }
            4 => {
                // Reference
                let inner = Box::new(self.read_type_ref(bytecode, pc)?);
                if *pc + 1 >= bytecode.len() {
                    return Err(SpecializationError::InvalidBytecode {
                        offset: *pc,
                        message: "Unexpected end reading reference flags".to_string(),
                    });
                }
                let mutability = if bytecode[*pc] == 0 {
                    crate::types::Mutability::Immutable
                } else {
                    crate::types::Mutability::Mutable
                };
                *pc += 1;
                let tier = match bytecode[*pc] {
                    0 => crate::types::CbgrTier::Tier0,
                    1 => crate::types::CbgrTier::Tier1,
                    _ => crate::types::CbgrTier::Tier2,
                };
                *pc += 1;
                Ok(TypeRef::Reference {
                    inner,
                    mutability,
                    tier,
                })
            }
            5 => {
                // Tuple
                if *pc >= bytecode.len() {
                    return Err(SpecializationError::InvalidBytecode {
                        offset: *pc,
                        message: "Unexpected end reading tuple element count".to_string(),
                    });
                }
                let elem_count = bytecode[*pc] as usize;
                *pc += 1;
                let mut elements = Vec::with_capacity(elem_count);
                for _ in 0..elem_count {
                    elements.push(self.read_type_ref(bytecode, pc)?);
                }
                Ok(TypeRef::Tuple(elements))
            }
            6 => {
                // Array
                let element = Box::new(self.read_type_ref(bytecode, pc)?);
                let length = self.read_varint(bytecode, pc)?;
                Ok(TypeRef::Array { element, length })
            }
            7 => {
                // Slice
                let element = Box::new(self.read_type_ref(bytecode, pc)?);
                Ok(TypeRef::Slice(element))
            }
            _ => {
                // Unknown tag - default to unit
                Ok(TypeRef::Concrete(TypeId::UNIT))
            }
        }
    }

    // ========================================================================
    // Bytecode Writing Helpers
    // ========================================================================

    /// Writes a register to bytecode.
    fn write_reg(&self, output: &mut Vec<u8>, reg: u16) {
        if reg < 128 {
            output.push(reg as u8);
        } else {
            output.push(0x80 | ((reg >> 8) as u8));
            output.push((reg & 0xFF) as u8);
        }
    }

    /// Writes a varint to bytecode.
    fn write_varint(&self, output: &mut Vec<u8>, mut value: u64) {
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                output.push(byte);
                break;
            } else {
                output.push(byte | 0x80);
            }
        }
    }

    /// Writes a signed varint (ZigZag encoded).
    fn write_signed_varint(&self, output: &mut Vec<u8>, value: i64) {
        let encoded = ((value << 1) ^ (value >> 63)) as u64;
        self.write_varint(output, encoded);
    }

    /// Writes a TypeRef to bytecode.
    fn write_type_ref(&self, output: &mut Vec<u8>, type_ref: &TypeRef) {
        match type_ref {
            TypeRef::Concrete(id) => {
                output.push(0); // tag
                self.write_varint(output, id.0 as u64);
            }
            TypeRef::Generic(param) => {
                output.push(1); // tag
                output.push((param.0 & 0xFF) as u8);
                output.push(((param.0 >> 8) & 0xFF) as u8);
            }
            TypeRef::Instantiated { base, args } => {
                output.push(2); // tag
                self.write_varint(output, base.0 as u64);
                output.push(args.len() as u8);
                for arg in args {
                    self.write_type_ref(output, arg);
                }
            }
            TypeRef::Function {
                params,
                return_type,
                ..
            } => {
                output.push(3); // tag
                output.push(params.len() as u8);
                for param in params {
                    self.write_type_ref(output, param);
                }
                self.write_type_ref(output, return_type);
            }
            TypeRef::Reference {
                inner,
                mutability,
                tier,
            } => {
                output.push(4); // tag
                self.write_type_ref(output, inner);
                output.push(*mutability as u8);
                output.push(*tier as u8);
            }
            TypeRef::Tuple(elements) => {
                output.push(5); // tag
                output.push(elements.len() as u8);
                for elem in elements {
                    self.write_type_ref(output, elem);
                }
            }
            TypeRef::Array { element, length } => {
                output.push(6); // tag
                self.write_type_ref(output, element);
                self.write_varint(output, *length);
            }
            TypeRef::Slice(element) => {
                output.push(7); // tag
                self.write_type_ref(output, element);
            }
            TypeRef::Rank2Function {
                type_param_count,
                params,
                return_type,
                ..
            } => {
                output.push(8); // tag for Rank2Function
                self.write_varint(output, *type_param_count as u64);
                output.push(params.len() as u8);
                for param in params {
                    self.write_type_ref(output, param);
                }
                self.write_type_ref(output, return_type);
            }
            TypeRef::AssociatedProjection { base, assoc } => {
                output.push(9); // tag for AssociatedProjection
                self.write_type_ref(output, base);
                let bytes = assoc.as_bytes();
                self.write_varint(output, bytes.len() as u64);
                output.extend_from_slice(bytes);
            }
        }
    }

    /// Returns statistics from specialization.
    pub fn stats(&self) -> &SpecializerStats {
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
    fn test_type_layout_builtin() {
        assert_eq!(TypeLayout::for_builtin(TypeId::UNIT).size, 0);
        assert_eq!(TypeLayout::for_builtin(TypeId::BOOL).size, 1);
        assert_eq!(TypeLayout::for_builtin(TypeId::INT).size, 8);
        assert_eq!(TypeLayout::for_builtin(TypeId::FLOAT).size, 8);
        assert_eq!(TypeLayout::for_builtin(TypeId::TEXT).size, 24);
    }

    #[test]
    fn test_specializer_stats_default() {
        let stats = SpecializerStats::default();
        assert_eq!(stats.call_g_specialized, 0);
        assert_eq!(stats.total_instructions, 0);
    }

    #[test]
    fn test_specialized_function_layout() {
        let sf = SpecializedFunction {
            bytecode: vec![crate::instruction::Opcode::Nop.to_byte(), crate::instruction::Opcode::RetV.to_byte()],
            register_count: 4,
            locals_count: 2,
            max_stack: 8,
            new_constants: vec![],
            params: Default::default(),
            return_type: crate::types::TypeRef::Concrete(crate::types::TypeId::UNIT),
        };
        assert_eq!(sf.bytecode.len(), 2);
        assert_eq!(sf.register_count, 4);
    }
}
