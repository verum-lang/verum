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
    FunctionNotFound { module: String, function_id: FunctionId },
    /// Type not found in source module.
    TypeNotFound { module: String, type_id: TypeId },
    /// Bytecode range invalid.
    InvalidBytecodeRange { offset: u32, length: u32, module_size: usize },
    /// String table conflict.
    StringTableConflict(String),
    /// Specialization missing.
    SpecializationMissing { function_id: FunctionId, type_args: Vec<TypeRef> },
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::FunctionNotFound { module, function_id } => {
                write!(f, "Function {:?} not found in module {}", function_id, module)
            }
            MergeError::TypeNotFound { module, type_id } => {
                write!(f, "Type {:?} not found in module {}", type_id, module)
            }
            MergeError::InvalidBytecodeRange { offset, length, module_size } => {
                write!(f, "Invalid bytecode range {}..{} in module of size {}",
                       offset, offset + length, module_size)
            }
            MergeError::StringTableConflict(msg) => {
                write!(f, "String table conflict: {}", msg)
            }
            MergeError::SpecializationMissing { function_id, type_args } => {
                write!(f, "Specialization missing for {:?} with {:?}", function_id, type_args)
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
        self.user_to_output.get(&old_id)
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
    pub fn merge(mut self) -> Result<(VbcModule, MergeStats), MergeError> {
        let mut output = VbcModule::new(self.user_module.name.clone());

        // Step 1: Copy user module structure
        self.copy_user_structure(&mut output)?;

        // Step 2: Copy user bytecode and functions
        self.copy_user_functions(&mut output)?;

        // Step 3: Add stdlib specializations
        self.add_stdlib_specializations(&mut output)?;

        // Step 4: Add newly specialized functions
        self.add_new_specializations(&mut output)?;

        // Step 5: Fixup function references in bytecode
        self.fixup_references(&mut output)?;

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

            output.bytecode.extend_from_slice(&self.user_module.bytecode[start..end]);

            // Create new function descriptor with updated offset
            let mut new_func = func.clone();
            new_func.id = FunctionId(output.functions.len() as u32);
            new_func.bytecode_offset = new_offset;
            output.functions.push(new_func);

            // Record mapping
            self.mapping.add_user(old_id, FunctionId(output.functions.len() as u32 - 1));
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
            }) = self.resolver.get_resolution(request.hash) {
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

                output.bytecode.extend_from_slice(&stdlib.bytecode[start..end]);

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
                self.mapping.add_spec(request.hash, FunctionId(output.functions.len() as u32 - 1));
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

            // Create function descriptor
            let new_func = FunctionDescriptor {
                id: FunctionId(output.functions.len() as u32),
                name: StringId::EMPTY, // Could generate from generic function name + type args
                bytecode_offset: new_offset,
                bytecode_length: specialized.bytecode.len() as u32,
                register_count: specialized.register_count,
                locals_count: specialized.locals_count,
                max_stack: specialized.max_stack,
                is_generic: false,
                ..Default::default()
            };

            output.functions.push(new_func);

            // Record mapping
            self.mapping.add_spec(request.hash, FunctionId(output.functions.len() as u32 - 1));
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

                // All other opcodes: skip their operands
                _ => {
                    pc = self.skip_instruction_operands(opcode, bytecode, pc, end);
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
        if bytecode[pc] < 128 {
            pc + 1
        } else {
            pc + 2
        }
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
        let mut v = value;
        let mut pos = pc;
        let end = pc + old_len;

        // Write varint bytes
        while pos < end {
            let byte = (v & 0x7F) as u8;
            v >>= 7;

            if pos + 1 < end && v > 0 {
                // More bytes follow
                bytecode[pos] = byte | 0x80;
            } else if pos + 1 == end {
                // Last byte - no continuation
                bytecode[pos] = byte;
            } else {
                // Pad with continuation zeros if value is smaller than old encoding
                bytecode[pos] = if v > 0 || pos + 1 < end - 1 { byte | 0x80 } else { byte };
            }
            pos += 1;
        }

        // If we have leftover space, pad with continuation zeros then final zero
        // This keeps the encoding length the same
        while pos < end {
            bytecode[pos] = if pos + 1 < end { 0x80 } else { 0x00 };
            pos += 1;
        }
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

    /// Skips instruction operands for non-call opcodes.
    fn skip_instruction_operands(&self, opcode: Opcode, bytecode: &[u8], pc: usize, end: usize) -> usize {
        match opcode {
            // No operands
            Opcode::Nop | Opcode::RetV => pc,

            // Single register
            Opcode::LoadTrue | Opcode::LoadFalse | Opcode::LoadUnit | Opcode::LoadNil
            | Opcode::Ret => self.skip_register(bytecode, pc),

            // Two registers
            Opcode::Mov | Opcode::Not | Opcode::NegI | Opcode::NegF | Opcode::Bnot
            | Opcode::Clone | Opcode::Ref | Opcode::RefMut | Opcode::Deref | Opcode::DerefMut
            | Opcode::Inc | Opcode::Dec => {
                let pc = self.skip_register(bytecode, pc);
                self.skip_register(bytecode, pc)
            }

            // Three registers
            Opcode::AddI | Opcode::SubI | Opcode::MulI | Opcode::DivI | Opcode::ModI
            | Opcode::AddF | Opcode::SubF | Opcode::MulF | Opcode::DivF | Opcode::ModF
            | Opcode::AddG | Opcode::SubG | Opcode::MulG | Opcode::DivG
            | Opcode::Band | Opcode::Bor | Opcode::Bxor | Opcode::Shl | Opcode::Shr | Opcode::Ushr
            | Opcode::And | Opcode::Or | Opcode::Xor
            | Opcode::EqI | Opcode::NeI | Opcode::LtI | Opcode::LeI | Opcode::GtI | Opcode::GeI
            | Opcode::EqF | Opcode::NeF | Opcode::LtF | Opcode::LeF | Opcode::GtF | Opcode::GeF
            | Opcode::EqG | Opcode::CmpG | Opcode::EqRef => {
                let pc = self.skip_register(bytecode, pc);
                let pc = self.skip_register(bytecode, pc);
                self.skip_register(bytecode, pc)
            }

            // Register + immediate
            Opcode::LoadI | Opcode::LoadSmallI => {
                let pc = self.skip_register(bytecode, pc);
                let (_, len) = self.read_varint(bytecode, pc);
                pc + len
            }

            Opcode::LoadF => {
                let pc = self.skip_register(bytecode, pc);
                pc + 8 // 64-bit float
            }

            Opcode::LoadK => {
                let pc = self.skip_register(bytecode, pc);
                let (_, len) = self.read_varint(bytecode, pc);
                pc + len
            }

            // Jumps: offset (4 bytes)
            Opcode::Jmp => pc + 4,

            // Conditional jumps: register + offset
            Opcode::JmpIf | Opcode::JmpNot => {
                let pc = self.skip_register(bytecode, pc);
                pc + 4
            }

            // Compare-and-jump: two registers + offset
            Opcode::JmpEq | Opcode::JmpNe | Opcode::JmpLt | Opcode::JmpLe
            | Opcode::JmpGt | Opcode::JmpGe => {
                let pc = self.skip_register(bytecode, pc);
                let pc = self.skip_register(bytecode, pc);
                pc + 4
            }

            // NEW: dst + type_id
            Opcode::New => {
                let pc = self.skip_register(bytecode, pc);
                let (_, len) = self.read_varint(bytecode, pc);
                pc + len
            }

            // NEW_G: dst + type_id + type_arg_count + type_args
            Opcode::NewG => {
                let pc = self.skip_register(bytecode, pc);
                let (_, len) = self.read_varint(bytecode, pc);
                let mut pc = pc + len;
                if pc < end {
                    let type_arg_count = bytecode[pc] as usize;
                    pc += 1;
                    for _ in 0..type_arg_count {
                        pc = self.skip_type_ref(bytecode, pc, end);
                    }
                }
                pc
            }

            // GET_F/SET_F: obj + field_idx
            Opcode::GetF => {
                let pc = self.skip_register(bytecode, pc);
                let pc = self.skip_register(bytecode, pc);
                let (_, len) = self.read_varint(bytecode, pc);
                pc + len
            }

            Opcode::SetF => {
                let pc = self.skip_register(bytecode, pc);
                let (_, len) = self.read_varint(bytecode, pc);
                let pc = pc + len;
                self.skip_register(bytecode, pc)
            }

            // Default: estimate 4 bytes
            _ => std::cmp::min(pc + 4, end),
        }
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

        Self { base, mapping, stats }
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
