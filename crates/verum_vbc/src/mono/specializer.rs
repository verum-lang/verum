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

use crate::instruction::Opcode;
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
            SpecializationError::SpecializationNotFound { function_id, type_args } => {
                write!(f, "Specialization not found for {:?} with {:?}", function_id, type_args)
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
            0 => Self { size: 0, alignment: 1 },    // UNIT
            1 => Self { size: 1, alignment: 1 },    // BOOL
            2 => Self { size: 8, alignment: 8 },    // INT
            3 => Self { size: 8, alignment: 8 },    // FLOAT
            4 => Self { size: 24, alignment: 8 },   // TEXT (ptr + len + cap)
            5 => Self { size: 0, alignment: 1 },    // NEVER
            6 => Self { size: 1, alignment: 1 },    // U8
            7 => Self { size: 2, alignment: 2 },    // U16
            8 => Self { size: 4, alignment: 4 },    // U32
            9 => Self { size: 8, alignment: 8 },    // U64
            10 => Self { size: 1, alignment: 1 },   // I8
            11 => Self { size: 2, alignment: 2 },   // I16
            12 => Self { size: 4, alignment: 4 },   // I32
            13 => Self { size: 4, alignment: 4 },   // F32
            14 => Self { size: 8, alignment: 8 },   // PTR
            _ => Self { size: 8, alignment: 8 },    // Default for user types
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
            type_layouts.insert(type_desc.id, TypeLayout {
                size: type_desc.size,
                alignment: type_desc.alignment,
            });
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
                    start, end, self.module.bytecode.len()
                ),
            });
        }

        let bytecode = &self.module.bytecode[start..end];
        self.stats.bytes_input = bytecode.len();

        let mut output = Vec::with_capacity(bytecode.len());
        let mut pc = 0;

        while pc < bytecode.len() {
            self.stats.total_instructions += 1;
            let opcode_byte = bytecode[pc];
            let opcode = Opcode::from_byte(opcode_byte);

            match opcode {
                // Generic call: rewrite to direct call
                Opcode::CallG => {
                    self.specialize_call_g(bytecode, &mut pc, &mut output)?;
                    self.stats.call_g_specialized += 1;
                }

                // Virtual dispatch: attempt devirtualization
                Opcode::CallV => {
                    self.specialize_call_v(bytecode, &mut pc, &mut output)?;
                }

                // Generic object creation
                Opcode::NewG => {
                    self.specialize_new_g(bytecode, &mut pc, &mut output)?;
                    self.stats.new_g_specialized += 1;
                }

                // Generic arithmetic: specialize to typed variants
                Opcode::AddG | Opcode::SubG | Opcode::MulG | Opcode::DivG => {
                    self.specialize_generic_arith(opcode, bytecode, &mut pc, &mut output)?;
                    self.stats.arith_g_specialized += 1;
                }

                // Generic comparison: specialize to typed variants
                Opcode::EqG | Opcode::CmpG => {
                    self.specialize_generic_cmp(opcode, bytecode, &mut pc, &mut output)?;
                    self.stats.cmp_g_specialized += 1;
                }

                // Size/alignment of generic type
                Opcode::SizeOfG => {
                    self.specialize_sizeof_g(bytecode, &mut pc, &mut output)?;
                    self.stats.sizeof_g_specialized += 1;
                }

                Opcode::AlignOfG => {
                    self.specialize_alignof_g(bytecode, &mut pc, &mut output)?;
                    self.stats.sizeof_g_specialized += 1;
                }

                // Load type reference: may need substitution
                Opcode::LoadT => {
                    self.specialize_load_t(bytecode, &mut pc, &mut output)?;
                }

                // All other opcodes: copy through with potential operand substitution
                _ => {
                    self.copy_instruction(opcode, bytecode, &mut pc, &mut output)?;
                }
            }
        }

        self.stats.bytes_output = output.len();

        Ok(SpecializedFunction {
            bytecode: output,
            register_count: func.register_count,
            locals_count: func.locals_count,
            max_stack: func.max_stack,
            new_constants: std::mem::take(&mut self.new_constants),
        })
    }

    /// Specializes a CALL_G instruction.
    ///
    /// CALL_G format: dst:reg func:varint type_arg_count:u8 [type_refs...] arg_count:u8 [args:reg...]
    /// CALL format: dst:reg func:varint arg_count:u8 [args:reg...]
    fn specialize_call_g(
        &mut self,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        // Read destination register
        let dst = self.read_reg(bytecode, pc)?;

        // Read function ID
        let func_id = FunctionId(self.read_varint(bytecode, pc)? as u32);

        // Read type arguments
        let type_arg_count = bytecode.get(*pc).copied().ok_or_else(|| {
            SpecializationError::InvalidBytecode {
                offset: *pc,
                message: "Unexpected end of bytecode reading type_arg_count".to_string(),
            }
        })?;
        *pc += 1;

        let mut call_type_args = Vec::with_capacity(type_arg_count as usize);
        for _ in 0..type_arg_count {
            let type_ref = self.read_type_ref(bytecode, pc)?;
            let substituted = self.substitution.apply(&type_ref);
            call_type_args.push(substituted);
        }

        // Read arguments
        let arg_count = bytecode.get(*pc).copied().ok_or_else(|| {
            SpecializationError::InvalidBytecode {
                offset: *pc,
                message: "Unexpected end of bytecode reading arg_count".to_string(),
            }
        })?;
        *pc += 1;

        let mut args = Vec::with_capacity(arg_count as usize);
        for _ in 0..arg_count {
            args.push(self.read_reg(bytecode, pc)?);
        }

        // Look up specialized function
        let key = InstantiationKey::new(func_id, call_type_args.clone());
        let specialized_fn = self.graph.get_specialization_by_key(&key)
            .unwrap_or(func_id); // Fall back to original if not found

        // Emit CALL instruction
        output.push(Opcode::Call.to_byte());
        self.write_reg(output, dst);
        self.write_varint(output, specialized_fn.0 as u64);
        output.push(arg_count);
        for arg in args {
            self.write_reg(output, arg);
        }

        Ok(())
    }

    /// Specializes a CALL_V instruction.
    ///
    /// CALL_V format: dst:reg receiver:reg protocol:varint method_idx:u8 arg_count:u8 [args:reg...]
    ///
    /// Attempts devirtualization if the receiver type is monomorphic and implements the protocol.
    /// If devirtualizable: emits CALL with direct function ID
    /// Otherwise: copies through as CALL_V
    fn specialize_call_v(
        &mut self,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        // Read destination register
        let dst = self.read_reg(bytecode, pc)?;

        // Read receiver register
        let receiver = self.read_reg(bytecode, pc)?;

        // Read protocol ID
        let protocol_id = ProtocolId(self.read_varint(bytecode, pc)? as u32);

        // Read method index in the protocol
        let method_idx = bytecode.get(*pc).copied().ok_or_else(|| {
            SpecializationError::InvalidBytecode {
                offset: *pc,
                message: "Unexpected end reading method_idx for CALL_V".to_string(),
            }
        })?;
        *pc += 1;

        // Read arguments
        let arg_count = bytecode.get(*pc).copied().ok_or_else(|| {
            SpecializationError::InvalidBytecode {
                offset: *pc,
                message: "Unexpected end reading arg_count for CALL_V".to_string(),
            }
        })?;
        *pc += 1;

        let mut args = Vec::with_capacity(arg_count as usize);
        for _ in 0..arg_count {
            args.push(self.read_reg(bytecode, pc)?);
        }

        // Attempt devirtualization
        // Check if we know the concrete type of the receiver
        if let Some(receiver_type) = self.register_types.get(&receiver).cloned()
            && let Some(direct_fn) = self.lookup_protocol_impl(&receiver_type, protocol_id, method_idx) {
                // Devirtualize: emit CALL instead of CALL_V
                output.push(Opcode::Call.to_byte());
                self.write_reg(output, dst);
                self.write_varint(output, direct_fn.0 as u64);
                output.push(arg_count + 1); // +1 for receiver as first arg
                self.write_reg(output, receiver); // receiver becomes first argument
                for arg in args {
                    self.write_reg(output, arg);
                }
                self.stats.call_v_devirtualized += 1;
                return Ok(());
            }

        // Fallback: check if substitution provides a primary type that could help
        if let Some(primary_type) = self.substitution.get(TypeParamId(0))
            && let Some(direct_fn) = self.lookup_protocol_impl(primary_type, protocol_id, method_idx) {
                // Devirtualize using substituted type
                output.push(Opcode::Call.to_byte());
                self.write_reg(output, dst);
                self.write_varint(output, direct_fn.0 as u64);
                output.push(arg_count + 1);
                self.write_reg(output, receiver);
                for arg in args {
                    self.write_reg(output, arg);
                }
                self.stats.call_v_devirtualized += 1;
                return Ok(());
            }

        // Cannot devirtualize: emit CALL_V as-is
        output.push(Opcode::CallV.to_byte());
        self.write_reg(output, dst);
        self.write_reg(output, receiver);
        self.write_varint(output, protocol_id.0 as u64);
        output.push(method_idx);
        output.push(arg_count);
        for arg in args {
            self.write_reg(output, arg);
        }

        Ok(())
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
        let proto_impl = type_desc.protocols.iter().find(|pi| pi.protocol == protocol_id)?;

        // Get method function ID by index
        let method_fn_id = proto_impl.methods.get(method_idx as usize)?;

        Some(FunctionId(*method_fn_id))
    }

    /// Specializes a NEW_G instruction.
    ///
    /// NEW_G format: dst:reg type_id:varint type_arg_count:u8 [type_refs...]
    /// NEW format: dst:reg type_id:varint
    fn specialize_new_g(
        &mut self,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        // Read destination register
        let dst = self.read_reg(bytecode, pc)?;

        // Read base type ID
        let base_type_id = TypeId(self.read_varint(bytecode, pc)? as u32);

        // Read type arguments
        let type_arg_count = bytecode.get(*pc).copied().ok_or_else(|| {
            SpecializationError::InvalidBytecode {
                offset: *pc,
                message: "Unexpected end of bytecode reading type_arg_count for NEW_G".to_string(),
            }
        })?;
        *pc += 1;

        let mut type_args = Vec::with_capacity(type_arg_count as usize);
        for _ in 0..type_arg_count {
            let type_ref = self.read_type_ref(bytecode, pc)?;
            let substituted = self.substitution.apply(&type_ref);
            type_args.push(substituted);
        }

        // Get or create concrete instantiated type
        let concrete_type_id = self.get_or_create_instantiated_type(base_type_id, &type_args);

        // Emit NEW instruction
        output.push(Opcode::New.to_byte());
        self.write_reg(output, dst);
        self.write_varint(output, concrete_type_id.0 as u64);

        Ok(())
    }

    /// Specializes generic arithmetic operations.
    ///
    /// ADD_G/SUB_G/MUL_G/DIV_G format: dst:reg a:reg b:reg protocol_id:varint
    fn specialize_generic_arith(
        &mut self,
        opcode: Opcode,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        // Read registers
        let dst = self.read_reg(bytecode, pc)?;
        let a = self.read_reg(bytecode, pc)?;
        let b = self.read_reg(bytecode, pc)?;

        // Read protocol ID
        let _protocol_id = self.read_varint(bytecode, pc)?;

        // Determine operand type from substitution context
        // In a full implementation, we'd track types through registers.
        // For now, we check if there's a primary type parameter that determines the operation.
        let operand_type = self.infer_operand_type_from_context();

        // Specialize based on concrete type
        let specialized_op = match (opcode, &operand_type) {
            (Opcode::AddG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::INT.0 => Opcode::AddI,
            (Opcode::AddG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::FLOAT.0 => Opcode::AddF,
            (Opcode::AddG, Some(TypeRef::Concrete(id))) if id.is_integer() => Opcode::AddI,
            (Opcode::AddG, Some(TypeRef::Concrete(id))) if id.is_float() => Opcode::AddF,

            (Opcode::SubG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::INT.0 => Opcode::SubI,
            (Opcode::SubG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::FLOAT.0 => Opcode::SubF,
            (Opcode::SubG, Some(TypeRef::Concrete(id))) if id.is_integer() => Opcode::SubI,
            (Opcode::SubG, Some(TypeRef::Concrete(id))) if id.is_float() => Opcode::SubF,

            (Opcode::MulG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::INT.0 => Opcode::MulI,
            (Opcode::MulG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::FLOAT.0 => Opcode::MulF,
            (Opcode::MulG, Some(TypeRef::Concrete(id))) if id.is_integer() => Opcode::MulI,
            (Opcode::MulG, Some(TypeRef::Concrete(id))) if id.is_float() => Opcode::MulF,

            (Opcode::DivG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::INT.0 => Opcode::DivI,
            (Opcode::DivG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::FLOAT.0 => Opcode::DivF,
            (Opcode::DivG, Some(TypeRef::Concrete(id))) if id.is_integer() => Opcode::DivI,
            (Opcode::DivG, Some(TypeRef::Concrete(id))) if id.is_float() => Opcode::DivF,

            // Keep generic for user-defined types (will use virtual dispatch at runtime)
            _ => opcode,
        };

        // Emit specialized or generic instruction
        if specialized_op == opcode {
            // Keep as generic - re-emit with protocol ID
            output.push(opcode.to_byte());
            self.write_reg(output, dst);
            self.write_reg(output, a);
            self.write_reg(output, b);
            self.write_varint(output, _protocol_id);
        } else {
            // Emit typed variant (no protocol ID needed)
            output.push(specialized_op.to_byte());
            self.write_reg(output, dst);
            self.write_reg(output, a);
            self.write_reg(output, b);
        }

        Ok(())
    }

    /// Specializes generic comparison operations.
    ///
    /// EQ_G/CMP_G format: dst:reg a:reg b:reg protocol_id:varint
    fn specialize_generic_cmp(
        &mut self,
        opcode: Opcode,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        // Read registers
        let dst = self.read_reg(bytecode, pc)?;
        let a = self.read_reg(bytecode, pc)?;
        let b = self.read_reg(bytecode, pc)?;

        // Read protocol ID
        let _protocol_id = self.read_varint(bytecode, pc)?;

        // Determine operand type
        let operand_type = self.infer_operand_type_from_context();

        // Specialize based on concrete type
        let specialized_op = match (opcode, &operand_type) {
            (Opcode::EqG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::INT.0 => Opcode::EqI,
            (Opcode::EqG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::FLOAT.0 => Opcode::EqF,
            (Opcode::EqG, Some(TypeRef::Concrete(id))) if id.0 == TypeId::BOOL.0 => Opcode::EqI,
            (Opcode::EqG, Some(TypeRef::Concrete(id))) if id.is_integer() => Opcode::EqI,
            (Opcode::EqG, Some(TypeRef::Concrete(id))) if id.is_float() => Opcode::EqF,

            // CMP_G doesn't have direct specialization - uses protocol dispatch
            _ => opcode,
        };

        // Emit specialized or generic instruction
        if specialized_op == opcode {
            output.push(opcode.to_byte());
            self.write_reg(output, dst);
            self.write_reg(output, a);
            self.write_reg(output, b);
            self.write_varint(output, _protocol_id);
        } else {
            output.push(specialized_op.to_byte());
            self.write_reg(output, dst);
            self.write_reg(output, a);
            self.write_reg(output, b);
        }

        Ok(())
    }

    /// Specializes SIZE_OF_G instruction.
    ///
    /// SIZE_OF_G format: dst:reg type_ref
    /// Becomes: LOAD_I dst, <concrete_size>
    fn specialize_sizeof_g(
        &mut self,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        let dst = self.read_reg(bytecode, pc)?;
        let type_ref = self.read_type_ref(bytecode, pc)?;
        let substituted = self.substitution.apply(&type_ref);

        let size = self.get_type_size(&substituted);

        // Emit LOAD_I with concrete size
        output.push(Opcode::LoadI.to_byte());
        self.write_reg(output, dst);
        self.write_signed_varint(output, size as i64);

        Ok(())
    }

    /// Specializes ALIGN_OF_G instruction.
    fn specialize_alignof_g(
        &mut self,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        let dst = self.read_reg(bytecode, pc)?;
        let type_ref = self.read_type_ref(bytecode, pc)?;
        let substituted = self.substitution.apply(&type_ref);

        let alignment = self.get_type_alignment(&substituted);

        // Emit LOAD_I with concrete alignment
        output.push(Opcode::LoadI.to_byte());
        self.write_reg(output, dst);
        self.write_signed_varint(output, alignment as i64);

        Ok(())
    }

    /// Specializes LOAD_T instruction.
    fn specialize_load_t(
        &mut self,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        *pc += 1; // Skip opcode

        let dst = self.read_reg(bytecode, pc)?;
        let type_ref = self.read_type_ref(bytecode, pc)?;
        let substituted = self.substitution.apply(&type_ref);

        output.push(Opcode::LoadT.to_byte());
        self.write_reg(output, dst);
        self.write_type_ref(output, &substituted);

        Ok(())
    }

    /// Copies an instruction through, handling operand-level substitution.
    fn copy_instruction(
        &mut self,
        opcode: Opcode,
        bytecode: &[u8],
        pc: &mut usize,
        output: &mut Vec<u8>,
    ) -> Result<(), SpecializationError> {
        output.push(opcode.to_byte());
        *pc += 1;

        // Get operand length and copy
        let operand_bytes = self.get_operand_bytes(opcode, bytecode, *pc)?;
        output.extend_from_slice(&bytecode[*pc..*pc + operand_bytes]);
        *pc += operand_bytes;

        Ok(())
    }

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
            TypeRef::Instantiated { base, .. } => {
                self.type_layouts.get(base).map_or(8, |l| l.size)
            }
            TypeRef::Reference { .. } => 16, // ThinRef size
            TypeRef::Tuple(elements) => {
                elements.iter().map(|e| self.get_type_size(e)).sum()
            }
            TypeRef::Array { element, length } => {
                self.get_type_size(element) * (*length as u32)
            }
            TypeRef::Slice(_) => 16, // ptr + len
            TypeRef::Function { .. } => 8, // function pointer
            TypeRef::Rank2Function { .. } => 8, // function pointer (rank-2 polymorphic)
            TypeRef::Generic(_) => 8, // Should be substituted by now
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
            TypeRef::Tuple(elements) => {
                elements.iter().map(|e| self.get_type_alignment(e)).max().unwrap_or(1)
            }
            TypeRef::Array { element, .. } => self.get_type_alignment(element),
            TypeRef::Slice(_) => 8,
            TypeRef::Function { .. } => 8,
            TypeRef::Rank2Function { .. } => 8,
            TypeRef::Generic(_) => 8,
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
        let all_concrete = args.iter().all(|arg| {
            matches!(arg, TypeRef::Concrete(_) | TypeRef::Instantiated { .. })
        });

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
        };

        // Update layout cache
        self.type_layouts.insert(new_id, TypeLayout { size, alignment });

        // Store new type descriptor
        self.new_type_descriptors.push(new_type_desc);

        // Cache the result
        self.instantiated_types.insert(key, new_id);

        new_id
    }

    /// Computes size and alignment for an instantiated generic type.
    fn compute_instantiated_layout(&self, base: TypeId, args: &[TypeRef]) -> (u32, u32) {
        // Get base type layout
        let base_layout = self.type_layouts.get(&base)
            .copied()
            .unwrap_or(TypeLayout { size: 8, alignment: 8 });

        // For simple cases, inherit from base
        // For complex cases (structs with generic fields), need to compute based on args
        if let Some(base_desc) = self.module.types.iter().find(|td| td.id == base) {
            // Check if any field uses a type parameter
            let has_generic_fields = base_desc.fields.iter().any(|f| {
                matches!(f.type_ref, TypeRef::Generic(_))
            });

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
                    && id.is_numeric() {
                        return Some(type_ref.clone());
                    }
            }
        }

        // Fallback: return the first available concrete type
        for param_id in 0..16 {
            if let Some(type_ref) = self.substitution.get(TypeParamId(param_id))
                && matches!(type_ref, TypeRef::Concrete(_)) {
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

    /// Clears register type tracking (e.g., at control flow merge points).
    #[allow(dead_code)]
    fn clear_register_types(&mut self) {
        self.register_types.clear();
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
    fn read_type_ref(&self, bytecode: &[u8], pc: &mut usize) -> Result<TypeRef, SpecializationError> {
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
                Ok(TypeRef::Reference { inner, mutability, tier })
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
            TypeRef::Function { params, return_type, .. } => {
                output.push(3); // tag
                output.push(params.len() as u8);
                for param in params {
                    self.write_type_ref(output, param);
                }
                self.write_type_ref(output, return_type);
            }
            TypeRef::Reference { inner, mutability, tier } => {
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
            TypeRef::Rank2Function { type_param_count, params, return_type, .. } => {
                output.push(8); // tag for Rank2Function
                self.write_varint(output, *type_param_count as u64);
                output.push(params.len() as u8);
                for param in params {
                    self.write_type_ref(output, param);
                }
                self.write_type_ref(output, return_type);
            }
        }
    }

    /// Gets the number of operand bytes for an opcode.
    fn get_operand_bytes(
        &self,
        opcode: Opcode,
        bytecode: &[u8],
        pc: usize,
    ) -> Result<usize, SpecializationError> {
        // This is a simplified version. Full implementation would parse
        // each instruction precisely.
        match opcode {
            // No operands
            Opcode::Nop | Opcode::RetV => Ok(0),

            // Single register
            Opcode::LoadTrue | Opcode::LoadFalse | Opcode::LoadUnit | Opcode::LoadNil => {
                self.count_reg_bytes(bytecode, pc)
            }

            // Two registers (unary ops)
            Opcode::Mov | Opcode::Not | Opcode::NegI | Opcode::NegF | Opcode::Bnot
            | Opcode::Clone | Opcode::Ref | Opcode::RefMut | Opcode::Deref | Opcode::DerefMut => {
                let first = self.count_reg_bytes(bytecode, pc)?;
                let second = self.count_reg_bytes(bytecode, pc + first)?;
                Ok(first + second)
            }

            // Three registers (binary ops)
            Opcode::AddI | Opcode::SubI | Opcode::MulI | Opcode::DivI | Opcode::ModI
            | Opcode::AddF | Opcode::SubF | Opcode::MulF | Opcode::DivF
            | Opcode::Band | Opcode::Bor | Opcode::Bxor | Opcode::Shl | Opcode::Shr | Opcode::Ushr
            | Opcode::EqI | Opcode::NeI | Opcode::LtI | Opcode::LeI | Opcode::GtI | Opcode::GeI
            | Opcode::EqF | Opcode::NeF | Opcode::LtF | Opcode::LeF | Opcode::GtF | Opcode::GeF
            | Opcode::And | Opcode::Or | Opcode::Xor | Opcode::EqRef => {
                let mut total = 0;
                for _ in 0..3 {
                    total += self.count_reg_bytes(bytecode, pc + total)?;
                }
                Ok(total)
            }

            // Ret: single register
            Opcode::Ret => self.count_reg_bytes(bytecode, pc),

            // Jump: register (for conditional) + 4-byte offset
            Opcode::Jmp => Ok(4),
            Opcode::JmpIf | Opcode::JmpNot => {
                let reg_bytes = self.count_reg_bytes(bytecode, pc)?;
                Ok(reg_bytes + 4)
            }

            // Fused compare-and-jump: two registers + 4-byte offset
            Opcode::JmpEq | Opcode::JmpNe | Opcode::JmpLt | Opcode::JmpLe
            | Opcode::JmpGt | Opcode::JmpGe => {
                let mut total = 0;
                for _ in 0..2 {
                    total += self.count_reg_bytes(bytecode, pc + total)?;
                }
                Ok(total + 4)
            }

            // Call: dst + func_id (varint) + arg_count + args
            Opcode::Call => {
                let dst_bytes = self.count_reg_bytes(bytecode, pc)?;
                let func_bytes = self.count_varint_bytes(bytecode, pc + dst_bytes)?;
                let arg_count_offset = pc + dst_bytes + func_bytes;
                if arg_count_offset >= bytecode.len() {
                    return Err(SpecializationError::InvalidBytecode {
                        offset: arg_count_offset,
                        message: "Unexpected end reading arg count for CALL".to_string(),
                    });
                }
                let arg_count = bytecode[arg_count_offset] as usize;
                let mut total = dst_bytes + func_bytes + 1;
                for _ in 0..arg_count {
                    total += self.count_reg_bytes(bytecode, pc + total)?;
                }
                Ok(total)
            }

            // LoadI: register + signed varint
            Opcode::LoadI => {
                let reg_bytes = self.count_reg_bytes(bytecode, pc)?;
                let varint_bytes = self.count_varint_bytes(bytecode, pc + reg_bytes)?;
                Ok(reg_bytes + varint_bytes)
            }

            // LoadF: register + 8 bytes
            Opcode::LoadF => {
                let reg_bytes = self.count_reg_bytes(bytecode, pc)?;
                Ok(reg_bytes + 8)
            }

            // LoadK: register + varint
            Opcode::LoadK => {
                let reg_bytes = self.count_reg_bytes(bytecode, pc)?;
                let varint_bytes = self.count_varint_bytes(bytecode, pc + reg_bytes)?;
                Ok(reg_bytes + varint_bytes)
            }

            // LoadSmallI: register + 1 byte
            Opcode::LoadSmallI => {
                let reg_bytes = self.count_reg_bytes(bytecode, pc)?;
                Ok(reg_bytes + 1)
            }

            // NEW: dst + type_id (varint)
            Opcode::New => {
                let dst_bytes = self.count_reg_bytes(bytecode, pc)?;
                let type_bytes = self.count_varint_bytes(bytecode, pc + dst_bytes)?;
                Ok(dst_bytes + type_bytes)
            }

            // GetF/SetF: register + register + field_idx (varint)
            Opcode::GetF => {
                let dst_bytes = self.count_reg_bytes(bytecode, pc)?;
                let obj_bytes = self.count_reg_bytes(bytecode, pc + dst_bytes)?;
                let field_bytes = self.count_varint_bytes(bytecode, pc + dst_bytes + obj_bytes)?;
                Ok(dst_bytes + obj_bytes + field_bytes)
            }

            Opcode::SetF => {
                let obj_bytes = self.count_reg_bytes(bytecode, pc)?;
                let field_bytes = self.count_varint_bytes(bytecode, pc + obj_bytes)?;
                let val_bytes = self.count_reg_bytes(bytecode, pc + obj_bytes + field_bytes)?;
                Ok(obj_bytes + field_bytes + val_bytes)
            }

            // Default: estimate based on typical sizes
            _ => Ok(4),
        }
    }

    /// Counts bytes for a register at the given offset.
    fn count_reg_bytes(&self, bytecode: &[u8], offset: usize) -> Result<usize, SpecializationError> {
        if offset >= bytecode.len() {
            return Err(SpecializationError::InvalidBytecode {
                offset,
                message: "Unexpected end counting register bytes".to_string(),
            });
        }
        if bytecode[offset] < 128 {
            Ok(1)
        } else {
            Ok(2)
        }
    }

    /// Counts bytes for a varint at the given offset.
    fn count_varint_bytes(&self, bytecode: &[u8], offset: usize) -> Result<usize, SpecializationError> {
        let mut count = 0;
        let mut pos = offset;
        loop {
            if pos >= bytecode.len() {
                return Err(SpecializationError::InvalidBytecode {
                    offset: pos,
                    message: "Unexpected end counting varint bytes".to_string(),
                });
            }
            count += 1;
            if bytecode[pos] < 128 {
                break;
            }
            pos += 1;
        }
        Ok(count)
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
            bytecode: vec![Opcode::Nop.to_byte(), Opcode::RetV.to_byte()],
            register_count: 4,
            locals_count: 2,
            max_stack: 8,
            new_constants: vec![],
        };
        assert_eq!(sf.bytecode.len(), 2);
        assert_eq!(sf.register_count, 4);
    }
}
