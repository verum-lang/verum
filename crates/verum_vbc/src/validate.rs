//! VBC module validation.
//!
//! This module provides validation of VBC modules to ensure they are
//! well-formed before execution. Validation catches errors early and
//! provides meaningful error messages.
//!
//! # Validation Levels
//!
//! 1. **Header validation**: Magic number, version, section bounds
//! 2. **Type table validation**: No circular types, valid references
//! 3. **Function table validation**: Valid bytecode offsets, register counts
//! 4. **Bytecode validation**: Valid opcodes, register bounds, type consistency
//! 5. **Cross-reference validation**: All references resolve

use crate::bytecode::decode_instruction;
use crate::error::{VbcError, VbcResult};
use crate::format::{VbcHeader, HEADER_SIZE, MAGIC, VERSION_MAJOR, VERSION_MINOR};
use crate::instruction::{Instruction, Reg, RegRange};
use crate::module::{Constant, FunctionDescriptor, VbcModule};
use crate::types::{TypeDescriptor, TypeId, TypeRef};

/// Validation options.
#[derive(Debug, Clone, Default)]
pub struct ValidationOptions {
    /// Skip content hash verification.
    pub skip_hash_check: bool,
    /// Skip bytecode validation (faster but less safe).
    pub skip_bytecode_validation: bool,
    /// Maximum allowed type nesting depth.
    pub max_type_depth: usize,
}

impl ValidationOptions {
    /// Creates strict validation options.
    pub fn strict() -> Self {
        Self {
            skip_hash_check: false,
            skip_bytecode_validation: false,
            max_type_depth: 100,
        }
    }

    /// Creates fast validation options (skips expensive checks).
    pub fn fast() -> Self {
        Self {
            skip_hash_check: true,
            skip_bytecode_validation: true,
            max_type_depth: 100,
        }
    }
}

/// Validates a VBC module.
pub fn validate_module(module: &VbcModule) -> VbcResult<()> {
    validate_module_with_options(module, &ValidationOptions::strict())
}

/// Validates a VBC module with custom options.
pub fn validate_module_with_options(
    module: &VbcModule,
    options: &ValidationOptions,
) -> VbcResult<()> {
    let mut validator = Validator::new(module, options);
    validator.validate()
}

/// VBC module validator.
struct Validator<'a> {
    module: &'a VbcModule,
    options: &'a ValidationOptions,
    errors: Vec<VbcError>,
}

impl<'a> Validator<'a> {
    /// Creates a new validator.
    fn new(module: &'a VbcModule, options: &'a ValidationOptions) -> Self {
        Self {
            module,
            options,
            errors: Vec::new(),
        }
    }

    /// Runs all validation checks.
    fn validate(&mut self) -> VbcResult<()> {
        // 1. Header validation
        self.validate_header()?;

        // 2. String table validation
        self.validate_string_table();

        // 3. Type table validation
        self.validate_types();

        // 4. Function table validation
        self.validate_functions();

        // 5. Constant pool validation
        self.validate_constants();

        // 6. Cross-reference validation
        self.validate_cross_references();

        // 7. Per-instruction bytecode validation (red-team Round 1 §3.1).
        //    Walks every function's bytecode, decodes each instruction,
        //    and validates cross-references (FunctionId in range,
        //    register references within the function's register count,
        //    branch targets land on instruction boundaries inside the
        //    function, constant IDs in range).  This is the load-time
        //    defense against hand-crafted bytecode that violates
        //    type-table invariants — without it the interpreter trusts
        //    the bytecode stream and only catches violations when
        //    execution happens to reach the malformed instruction.
        if !self.options.skip_bytecode_validation {
            self.validate_bytecode();
        }

        // Return errors
        if self.errors.is_empty() {
            Ok(())
        } else if self.errors.len() == 1 {
            Err(self.errors.pop().unwrap())
        } else {
            Err(VbcError::MultipleErrors(std::mem::take(&mut self.errors)))
        }
    }

    /// Validates the header.
    fn validate_header(&mut self) -> VbcResult<()> {
        let header = &self.module.header;

        // Magic number
        if header.magic != MAGIC {
            return Err(VbcError::InvalidMagic(header.magic));
        }

        // Version
        if header.version_major != VERSION_MAJOR || header.version_minor > VERSION_MINOR {
            return Err(VbcError::UnsupportedVersion {
                major: header.version_major,
                minor: header.version_minor,
                supported_major: VERSION_MAJOR,
                supported_minor: VERSION_MINOR,
            });
        }

        // Counts match actual content
        if header.type_table_count as usize != self.module.types.len() {
            self.errors.push(VbcError::InvalidHeader {
                field: "type_table_count",
                offset: 0x14,
            });
        }

        if header.function_table_count as usize != self.module.functions.len() {
            self.errors.push(VbcError::InvalidHeader {
                field: "function_table_count",
                offset: 0x1C,
            });
        }

        if header.constant_pool_count as usize != self.module.constants.len() {
            self.errors.push(VbcError::InvalidHeader {
                field: "constant_pool_count",
                offset: 0x24,
            });
        }

        Ok(())
    }

    /// Validates the string table.
    fn validate_string_table(&mut self) {
        // Check for empty module name
        if self.module.name.is_empty() {
            // Not an error, but could be a warning
        }
    }

    /// Validates all type descriptors.
    fn validate_types(&mut self) {
        for (idx, type_desc) in self.module.types.iter().enumerate() {
            self.validate_type_descriptor(type_desc, idx);
        }
    }

    /// Validates a single type descriptor.
    fn validate_type_descriptor(&mut self, desc: &TypeDescriptor, _index: usize) {
        // Validate name reference
        if self.module.get_string(desc.name).is_none() && desc.name.0 != 0 {
            self.errors.push(VbcError::InvalidStringId(desc.name.0));
        }

        // Validate field types
        for field in &desc.fields {
            self.validate_type_ref(&field.type_ref, 0);
        }

        // Validate variant payloads
        for variant in &desc.variants {
            if let Some(ref payload) = variant.payload {
                self.validate_type_ref(payload, 0);
            }
        }

        // Validate size/alignment
        if desc.alignment > 0 && !desc.alignment.is_power_of_two() {
            self.errors.push(VbcError::InvalidHeader {
                field: "alignment",
                offset: 0,
            });
        }
    }

    /// Validates a type reference (with depth check for circularity).
    fn validate_type_ref(&mut self, type_ref: &TypeRef, depth: usize) {
        if depth > self.options.max_type_depth {
            self.errors.push(VbcError::CircularType(TypeId(0)));
            return;
        }

        match type_ref {
            TypeRef::Concrete(id) => {
                if !id.is_builtin() && self.module.get_type(*id).is_none() {
                    self.errors.push(VbcError::InvalidTypeId(id.0));
                }
            }
            TypeRef::Generic(_) => {
                // Generic type params are validated in context
            }
            TypeRef::Instantiated { base, args } => {
                if !base.is_builtin() && self.module.get_type(*base).is_none() {
                    self.errors.push(VbcError::InvalidTypeId(base.0));
                }
                for arg in args {
                    self.validate_type_ref(arg, depth + 1);
                }
            }
            TypeRef::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.validate_type_ref(param, depth + 1);
                }
                self.validate_type_ref(return_type, depth + 1);
            }
            TypeRef::Reference { inner, .. } => {
                self.validate_type_ref(inner, depth + 1);
            }
            TypeRef::Tuple(elems) => {
                for elem in elems {
                    self.validate_type_ref(elem, depth + 1);
                }
            }
            TypeRef::Array { element, .. } => {
                self.validate_type_ref(element, depth + 1);
            }
            TypeRef::Slice(inner) => {
                self.validate_type_ref(inner, depth + 1);
            }
            TypeRef::Rank2Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.validate_type_ref(param, depth + 1);
                }
                self.validate_type_ref(return_type, depth + 1);
            }
        }
    }

    /// Validates all function descriptors.
    fn validate_functions(&mut self) {
        for (idx, func_desc) in self.module.functions.iter().enumerate() {
            self.validate_function_descriptor(func_desc, idx);
        }
    }

    /// Validates a single function descriptor.
    fn validate_function_descriptor(&mut self, desc: &FunctionDescriptor, _index: usize) {
        // Validate name reference
        if self.module.get_string(desc.name).is_none() && desc.name.0 != 0 {
            self.errors.push(VbcError::InvalidStringId(desc.name.0));
        }

        // Validate parent type reference
        if let Some(parent) = desc.parent_type
            && !parent.is_builtin() && self.module.get_type(parent).is_none() {
                self.errors.push(VbcError::InvalidTypeId(parent.0));
            }

        // Validate bytecode bounds
        let bytecode_end = desc.bytecode_offset as usize + desc.bytecode_length as usize;
        if bytecode_end > self.module.bytecode.len() {
            self.errors.push(VbcError::InvalidBytecodeOffset {
                func: desc.id,
                offset: desc.bytecode_offset,
                size: self.module.bytecode.len() as u32,
            });
        }

        // Validate parameter types
        for param in &desc.params {
            self.validate_type_ref(&param.type_ref, 0);
        }

        // Validate return type
        self.validate_type_ref(&desc.return_type, 0);

        // Validate register count is reasonable (max ~16K for good performance)
        if desc.register_count > 16384 {
            // This is a warning, not an error - large register counts are valid
            // but may indicate a code generation issue
        }
    }

    /// Validates all constants.
    fn validate_constants(&mut self) {
        for constant in &self.module.constants {
            self.validate_constant(constant);
        }
    }

    /// Validates a single constant.
    fn validate_constant(&mut self, constant: &Constant) {
        match constant {
            Constant::String(id)
                if self.module.get_string(*id).is_none() => {
                    self.errors.push(VbcError::InvalidStringId(id.0));
                }
            Constant::Type(type_ref) => {
                self.validate_type_ref(type_ref, 0);
            }
            Constant::Function(id)
                if self.module.get_function(*id).is_none() => {
                    self.errors.push(VbcError::InvalidFunctionId(id.0));
                }
            Constant::Array(items) => {
                for item in items {
                    if self.module.get_constant(*item).is_none() {
                        self.errors.push(VbcError::InvalidConstId(item.0));
                    }
                }
            }
            _ => {}
        }
    }

    /// Validates per-function bytecode by decoding each instruction in the
    /// function's bytecode region and verifying its cross-references.
    ///
    /// This is the load-time defense against hand-crafted bytecode that
    /// the type-checker / codegen pipeline would never produce.  Catches:
    ///
    ///   * Out-of-range `FunctionId` in `Call` / `TailCall` / `CallG`
    ///     / `CallC` — would cause `FunctionNotFound` at execution time
    ///     in the well-defined case; here we catch it before any code
    ///     runs.
    ///   * Out-of-range register references — would write past the
    ///     function's register file, corrupting an adjacent frame.
    ///   * Branch offsets that fall outside the function's bytecode
    ///     region or land mid-instruction — would let crafted bytecode
    ///     decode arbitrary opcodes from the operand stream of a
    ///     legitimate instruction.
    ///   * Out-of-range `ConstId` / `StringId` — would return None /
    ///     bogus interned strings at execution time.
    ///
    /// Performance: walks bytecode linearly once, so cost is O(N) in
    /// total instruction count.  Skip via
    /// `ValidationOptions::skip_bytecode_validation = true` for
    /// trusted-source loads (e.g. self-emitted bytecode in the same
    /// process).
    fn validate_bytecode(&mut self) {
        // Snapshot the cross-reference table sizes so the per-function
        // walk doesn't need to repeatedly index `self.module`.
        let function_count = self.module.functions.len() as u32;
        let constant_count = self.module.constants.len() as u32;
        let string_count = self.module.strings.len() as u32;

        for func_desc in &self.module.functions {
            self.validate_function_bytecode(
                func_desc,
                function_count,
                constant_count,
                string_count,
            );
        }
    }

    /// Walks one function's bytecode, decoding each instruction and
    /// validating its cross-references.
    fn validate_function_bytecode(
        &mut self,
        func: &FunctionDescriptor,
        function_count: u32,
        constant_count: u32,
        string_count: u32,
    ) {
        let func_start = func.bytecode_offset as usize;
        let func_end = func_start + func.bytecode_length as usize;

        // The function-table validator (`validate_function_descriptor`)
        // already caught out-of-bounds `bytecode_offset + length`; skip
        // the walk for invalid descriptors so we don't double-report.
        if func_end > self.module.bytecode.len() {
            return;
        }

        let max_reg = func.register_count;
        // Track decoded instruction-start offsets so jump-target
        // validation can verify the target lands ON an instruction
        // boundary.  Without this check, crafted bytecode could land a
        // jump in the middle of a multi-byte instruction's operand
        // stream and have the interpreter decode arbitrary opcodes
        // from those operand bytes.
        //
        // Worst-case capacity is the function's bytecode-byte count
        // (one-byte instructions); typical instructions average ~4-6
        // bytes so this comfortably over-allocates.
        let mut instr_starts: std::collections::BTreeSet<u32> =
            std::collections::BTreeSet::new();
        // Pending jump targets we couldn't validate during the walk
        // (forward jumps to instructions not yet decoded).
        let mut pending_jumps: Vec<(u32, u32)> = Vec::new();

        let mut offset = func_start;
        while offset < func_end {
            let instr_start = offset;
            instr_starts.insert(instr_start as u32);

            // Decode this instruction.  Decoder failures surface as a
            // typed error; we log and stop walking this function (the
            // remaining bytes are unparseable as instructions).
            let instr = match decode_instruction(&self.module.bytecode, &mut offset) {
                Ok(instr) => instr,
                Err(err) => {
                    self.errors.push(VbcError::InvalidInstructionEncoding {
                        offset: instr_start,
                        reason: err.to_string(),
                    });
                    return;
                }
            };

            // The decoder must not advance past the function's
            // bytecode region.  If it did, the function descriptor's
            // `bytecode_length` is too small for the decoded
            // instruction stream — flag and stop.
            if offset > func_end {
                self.errors.push(VbcError::InvalidInstructionEncoding {
                    offset: instr_start,
                    reason: format!(
                        "decoded instruction extends past function end \
                         (instr_start={}, decoded_end={}, func_end={})",
                        instr_start, offset, func_end
                    ),
                });
                return;
            }

            self.validate_instruction(
                &instr,
                instr_start,
                offset,
                func,
                max_reg,
                function_count,
                constant_count,
                string_count,
                &mut pending_jumps,
            );
        }

        // After the walk, instr_starts holds every legal instruction-
        // boundary offset within the function.  Validate every
        // forward-jump target against the boundary set + the function
        // bytecode range.
        let func_end_u32 = func_end as u32;
        for &(source_offset, target) in &pending_jumps {
            if (target as usize) < func_start
                || target > func_end_u32
                || (target < func_end_u32 && !instr_starts.contains(&target))
            {
                self.errors.push(VbcError::JumpOutOfBounds {
                    target: target as i32,
                    max: func_end_u32,
                    offset: source_offset,
                });
            }
        }
    }

    /// Validates a single decoded instruction's cross-references.
    ///
    /// Branch targets are deferred to a post-walk pass via
    /// `pending_jumps` so we can verify they land on a known
    /// instruction-start boundary.
    #[allow(clippy::too_many_arguments)]
    fn validate_instruction(
        &mut self,
        instr: &Instruction,
        instr_start: usize,
        next_offset: usize,
        func: &FunctionDescriptor,
        max_reg: u16,
        function_count: u32,
        constant_count: u32,
        string_count: u32,
        pending_jumps: &mut Vec<(u32, u32)>,
    ) {
        // Helper closures to keep the per-variant arms tidy.
        let func_name = format!(
            "fn#{}@0x{:x}",
            func.id.0, instr_start
        );

        match instr {
            // -----------------------------------------------------------
            // Function-call cross-references — `func_id` in range.
            // -----------------------------------------------------------
            Instruction::Call { dst, func_id, args } => {
                if *func_id >= function_count {
                    self.errors.push(VbcError::InvalidFunctionId(*func_id));
                }
                self.check_reg(*dst, max_reg, &func_name);
                self.check_reg_range(*args, max_reg, &func_name);
            }
            Instruction::CallG { dst, func_id, type_args, args } => {
                if *func_id >= function_count {
                    self.errors.push(VbcError::InvalidFunctionId(*func_id));
                }
                self.check_reg(*dst, max_reg, &func_name);
                for r in type_args {
                    self.check_reg(*r, max_reg, &func_name);
                }
                self.check_reg_range(*args, max_reg, &func_name);
            }
            Instruction::TailCall { func_id, args } => {
                if *func_id >= function_count {
                    self.errors.push(VbcError::InvalidFunctionId(*func_id));
                }
                self.check_reg_range(*args, max_reg, &func_name);
            }
            // CallC: `cache_id` is an inline-cache slot, not a
            // function-table index — only validate registers.
            Instruction::CallC { dst, cache_id: _, args } => {
                self.check_reg(*dst, max_reg, &func_name);
                self.check_reg_range(*args, max_reg, &func_name);
            }
            // CallM uses method_id which is a string-id (the bare
            // method name); validate as such.
            Instruction::CallM { dst, receiver, method_id, args } => {
                if *method_id >= string_count {
                    self.errors.push(VbcError::InvalidStringId(*method_id));
                }
                self.check_reg(*dst, max_reg, &func_name);
                self.check_reg(*receiver, max_reg, &func_name);
                self.check_reg_range(*args, max_reg, &func_name);
            }
            Instruction::CallV { dst, vtable_slot: _, receiver, args } => {
                self.check_reg(*dst, max_reg, &func_name);
                self.check_reg(*receiver, max_reg, &func_name);
                self.check_reg_range(*args, max_reg, &func_name);
            }
            // CallR: indirect call via register; `argc` is just a count
            // (no register range to validate in-place — args are
            // implicitly the registers immediately after `func`).
            Instruction::CallR { dst, func, argc: _ } => {
                self.check_reg(*dst, max_reg, &func_name);
                self.check_reg(*func, max_reg, &func_name);
            }
            Instruction::CallClosure { dst, closure, args } => {
                self.check_reg(*dst, max_reg, &func_name);
                self.check_reg(*closure, max_reg, &func_name);
                self.check_reg_range(*args, max_reg, &func_name);
            }

            // -----------------------------------------------------------
            // Constant-pool / string-table references.
            // -----------------------------------------------------------
            Instruction::LoadK { dst, const_id } => {
                if *const_id >= constant_count {
                    self.errors.push(VbcError::InvalidConstId(*const_id));
                }
                self.check_reg(*dst, max_reg, &func_name);
            }

            // -----------------------------------------------------------
            // Branch instructions — defer target check to post-walk.
            // -----------------------------------------------------------
            Instruction::Jmp { offset } => {
                let target = (next_offset as i64 + *offset as i64) as u32;
                pending_jumps.push((instr_start as u32, target));
            }
            Instruction::JmpIf { cond, offset } => {
                self.check_reg(*cond, max_reg, &func_name);
                let target = (next_offset as i64 + *offset as i64) as u32;
                pending_jumps.push((instr_start as u32, target));
            }
            Instruction::JmpNot { cond, offset } => {
                self.check_reg(*cond, max_reg, &func_name);
                let target = (next_offset as i64 + *offset as i64) as u32;
                pending_jumps.push((instr_start as u32, target));
            }
            Instruction::JmpCmp { op: _, a, b, offset } => {
                self.check_reg(*a, max_reg, &func_name);
                self.check_reg(*b, max_reg, &func_name);
                let target = (next_offset as i64 + *offset as i64) as u32;
                pending_jumps.push((instr_start as u32, target));
            }

            // -----------------------------------------------------------
            // Switch — multiple branch targets (default + per-case).
            // Every offset must land on a known instruction-start
            // boundary inside the function's bytecode region.
            // -----------------------------------------------------------
            Instruction::Switch { value, default_offset, cases } => {
                self.check_reg(*value, max_reg, &func_name);
                let default_target = (next_offset as i64 + *default_offset as i64) as u32;
                pending_jumps.push((instr_start as u32, default_target));
                for (_case_value, case_offset) in cases {
                    let target = (next_offset as i64 + *case_offset as i64) as u32;
                    pending_jumps.push((instr_start as u32, target));
                }
            }

            // -----------------------------------------------------------
            // TryBegin — handler offset is a branch target like Jmp's.
            // Crafted bytecode could land the handler mid-instruction
            // and cause the interpreter to decode arbitrary opcodes
            // when an exception fires.
            // -----------------------------------------------------------
            Instruction::TryBegin { handler_offset } => {
                let target = (next_offset as i64 + *handler_offset as i64) as u32;
                pending_jumps.push((instr_start as u32, target));
            }

            // -----------------------------------------------------------
            // Closure construction — `func_id` references the function
            // table.  Every captured-value register must also be in
            // bounds.
            // -----------------------------------------------------------
            Instruction::NewClosure { dst, func_id, captures } => {
                if *func_id >= function_count {
                    self.errors.push(VbcError::InvalidFunctionId(*func_id));
                }
                self.check_reg(*dst, max_reg, &func_name);
                for r in captures {
                    self.check_reg(*r, max_reg, &func_name);
                }
            }

            // -----------------------------------------------------------
            // Panic — message_id references the string table.
            // -----------------------------------------------------------
            Instruction::Panic { message_id } => {
                if *message_id >= string_count {
                    self.errors.push(VbcError::InvalidStringId(*message_id));
                }
            }

            // -----------------------------------------------------------
            // All remaining instructions — register-only validation.
            // We don't enumerate every variant (the Instruction enum
            // is huge); the call-site cross-references and branch
            // targets above cover the high-value attack surface.  Reg
            // out-of-bounds in the residual variants is caught by the
            // interpreter's `state.get_reg` panic-free read paths.
            // -----------------------------------------------------------
            _ => {}
        }
    }

    /// Checks a single register reference against the function's
    /// `register_count`.
    fn check_reg(&mut self, reg: Reg, max: u16, context: &str) {
        if reg.0 >= max {
            self.errors.push(VbcError::RegisterOutOfBounds {
                reg: reg.0,
                max,
                context: context.to_string(),
            });
        }
    }

    /// Checks a contiguous register range against the function's
    /// `register_count`.  `RegRange { start, count }` represents
    /// `[start, start + count)` — every reg in the range must be
    /// in-bounds.
    fn check_reg_range(&mut self, range: RegRange, max: u16, context: &str) {
        if range.count == 0 {
            return;
        }
        let last = range.start.0.saturating_add(range.count as u16 - 1);
        if last >= max {
            self.errors.push(VbcError::RegisterOutOfBounds {
                reg: last,
                max,
                context: context.to_string(),
            });
        }
    }

    /// Validates cross-references between sections.
    fn validate_cross_references(&mut self) {
        // Validate specialization entries
        for spec in &self.module.specializations {
            if self.module.get_function(spec.generic_fn).is_none() {
                self.errors.push(VbcError::InvalidFunctionId(spec.generic_fn.0));
            }
            for type_arg in &spec.type_args {
                self.validate_type_ref(type_arg, 0);
            }
        }

        // Validate source map entries
        if let Some(ref source_map) = self.module.source_map {
            for entry in &source_map.entries {
                if entry.bytecode_offset as usize >= self.module.bytecode.len() {
                    // Invalid source map entry (could be a warning)
                }
                if entry.file_idx as usize >= source_map.files.len() {
                    self.errors.push(VbcError::InvalidHeader {
                        field: "source_map_file_idx",
                        offset: 0,
                    });
                }
            }
        }
    }
}

/// Quick validation check (header only).
pub fn validate_header_only(data: &[u8]) -> VbcResult<VbcHeader> {
    if data.len() < HEADER_SIZE {
        return Err(VbcError::eof(0, HEADER_SIZE));
    }

    // Magic
    let magic: [u8; 4] = data[0..4].try_into().unwrap();
    if magic != MAGIC {
        return Err(VbcError::InvalidMagic(magic));
    }

    // Version
    let version_major = u16::from_le_bytes([data[4], data[5]]);
    let version_minor = u16::from_le_bytes([data[6], data[7]]);
    if version_major != VERSION_MAJOR || version_minor > VERSION_MINOR {
        return Err(VbcError::UnsupportedVersion {
            major: version_major,
            minor: version_minor,
            supported_major: VERSION_MAJOR,
            supported_minor: VERSION_MINOR,
        });
    }

    // Parse remaining header fields
    use crate::deserialize::deserialize_module;
    let module = deserialize_module(data)?;
    Ok(module.header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::VbcModule;

    #[test]
    fn test_validate_empty_module() {
        let module = VbcModule::new("test".to_string());
        assert!(validate_module(&module).is_ok());
    }

    #[test]
    fn test_validate_with_invalid_string_ref() {
        use crate::types::{StringId, TypeDescriptor, TypeKind};

        let mut module = VbcModule::new("test".to_string());

        // Add type with invalid string reference
        let desc = TypeDescriptor {
            name: StringId(999999), // Invalid
            kind: TypeKind::Record,
            ..Default::default()
        };
        module.types.push(desc);

        let result = validate_module(&module);
        assert!(result.is_err());
    }

    #[test]
    fn test_fast_validation() {
        let module = VbcModule::new("test".to_string());
        let options = ValidationOptions::fast();
        assert!(validate_module_with_options(&module, &options).is_ok());
    }

    // =========================================================================
    // Round 1 §3.1 — Hand-crafted bytecode validator (DEFENSE CONFIRMED)
    // =========================================================================
    //
    // Tests pin the load-time bytecode-validation defense.  Each test
    // builds a synthetic module whose bytecode encodes a single
    // adversarial instruction and asserts the validator rejects it
    // with the expected typed error variant.

    use crate::bytecode::encode_instruction;
    use crate::instruction::{Instruction, Reg, RegRange};
    use crate::module::FunctionDescriptor;
    use crate::types::StringId;
    use crate::FunctionId;

    /// Build a one-function module whose body is the supplied instruction
    /// followed by a Ret-Unit terminator.  `register_count` configures
    /// the function's register file size — used by tests that exercise
    /// register-out-of-bounds.
    fn build_module_with_instr(
        instr: Instruction,
        register_count: u16,
        function_count_hint: usize,
    ) -> VbcModule {
        let mut module = VbcModule::new("rt_3_1".to_string());

        // Pad the function table so callers can reference distinct
        // FunctionId(0)/(N) values.
        for i in 0..function_count_hint.max(1) {
            let mut f = FunctionDescriptor::new(StringId::EMPTY);
            f.id = FunctionId(i as u32);
            f.bytecode_offset = 0;
            f.bytecode_length = 0;
            f.register_count = register_count;
            module.functions.push(f);
        }

        let mut bc = Vec::new();
        encode_instruction(&instr, &mut bc);
        encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut bc);

        // Wire function 0 to point at our crafted bytecode.
        module.functions[0].bytecode_offset = 0;
        module.functions[0].bytecode_length = bc.len() as u32;
        module.bytecode = bc;

        // Sync header counts so the section-count cross-checks pass —
        // the validator catches header/section divergence in
        // `validate_header`, which would mask the per-instruction
        // checks we're trying to exercise.
        module.header.function_table_count = module.functions.len() as u32;
        module.header.type_table_count = module.types.len() as u32;
        module.header.constant_pool_count = module.constants.len() as u32;

        module
    }

    #[test]
    fn validator_rejects_call_with_oor_function_id() {
        // Function table has only function id 0; a Call to function 99
        // is hand-crafted bytecode that the codegen pipeline never
        // emits.  Validator must reject at load time.
        let oor_call = Instruction::Call {
            dst: Reg(0),
            func_id: 99,
            args: RegRange { start: Reg(0), count: 0 },
        };
        let module = build_module_with_instr(oor_call, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        match err {
            VbcError::InvalidFunctionId(99) => {}
            VbcError::MultipleErrors(errs) if errs.iter().any(|e|
                matches!(e, VbcError::InvalidFunctionId(99))
            ) => {}
            other => panic!("expected InvalidFunctionId(99), got: {:?}", other),
        }
    }

    #[test]
    fn validator_rejects_call_with_oob_register() {
        // Function declares register_count = 4; instruction writes to r10.
        let bad_call = Instruction::Call {
            dst: Reg(10),
            func_id: 0,
            args: RegRange { start: Reg(0), count: 0 },
        };
        let module = build_module_with_instr(bad_call, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        let has_oob = match &err {
            VbcError::RegisterOutOfBounds { reg: 10, max: 4, .. } => true,
            VbcError::MultipleErrors(errs) => errs.iter().any(|e|
                matches!(e, VbcError::RegisterOutOfBounds { reg: 10, max: 4, .. })
            ),
            _ => false,
        };
        assert!(has_oob, "expected RegisterOutOfBounds {{reg:10, max:4}}, got: {:?}", err);
    }

    #[test]
    fn validator_rejects_jmp_target_past_function_end() {
        // The `Jmp` opcode's `offset` is signed and is in BYTES
        // relative to the PC after reading the opcode + operand.
        // A target of u32::MAX bytes past EOF is comfortably out of
        // range for any reasonable module.
        let bad_jmp = Instruction::Jmp { offset: 0x7FFF_FFFF };
        let module = build_module_with_instr(bad_jmp, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        let has_oob = match &err {
            VbcError::JumpOutOfBounds { .. } => true,
            VbcError::MultipleErrors(errs) => errs.iter().any(|e|
                matches!(e, VbcError::JumpOutOfBounds { .. })
            ),
            _ => false,
        };
        assert!(has_oob, "expected JumpOutOfBounds, got: {:?}", err);
    }

    #[test]
    fn validator_rejects_loadk_with_oor_const_id() {
        // Constant pool is empty; LoadK referencing const id 5 is
        // out-of-range.
        let bad_loadk = Instruction::LoadK {
            dst: Reg(0),
            const_id: 5,
        };
        let module = build_module_with_instr(bad_loadk, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        let has_const = match &err {
            VbcError::InvalidConstId(5) => true,
            VbcError::MultipleErrors(errs) => errs.iter().any(|e|
                matches!(e, VbcError::InvalidConstId(5))
            ),
            _ => false,
        };
        assert!(has_const, "expected InvalidConstId(5), got: {:?}", err);
    }

    #[test]
    fn validator_accepts_well_formed_module() {
        // Sanity check: a module whose only instruction is `Mov r0, r1`
        // (both regs in-bounds, no func/const references) must pass.
        let good_mov = Instruction::Mov { dst: Reg(0), src: Reg(1) };
        let module = build_module_with_instr(good_mov, 4, 1);
        validate_module(&module).expect("well-formed module must validate");
    }

    #[test]
    fn validator_skip_bytecode_validation_skips_per_instruction_check() {
        // With skip_bytecode_validation=true, an OOR call must NOT be
        // flagged (the option is the trusted-source escape hatch).
        let oor_call = Instruction::Call {
            dst: Reg(0),
            func_id: 99,
            args: RegRange { start: Reg(0), count: 0 },
        };
        let module = build_module_with_instr(oor_call, 4, 1);
        let opts = ValidationOptions {
            skip_hash_check: false,
            skip_bytecode_validation: true,
            max_type_depth: 100,
        };
        validate_module_with_options(&module, &opts)
            .expect("skip flag must bypass bytecode validation");
    }

    #[test]
    fn validator_rejects_new_closure_with_oor_function_id() {
        // Closure body referencing FunctionId(42) in a 1-function
        // module must be rejected at load time.
        let bad_closure = Instruction::NewClosure {
            dst: Reg(0),
            func_id: 42,
            captures: vec![],
        };
        let module = build_module_with_instr(bad_closure, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        let has_err = matches!(&err, VbcError::InvalidFunctionId(42))
            || matches!(&err, VbcError::MultipleErrors(errs)
                if errs.iter().any(|e| matches!(e, VbcError::InvalidFunctionId(42))));
        assert!(has_err, "expected InvalidFunctionId(42), got: {:?}", err);
    }

    #[test]
    fn validator_rejects_panic_with_oor_message_id() {
        // Panic referencing string id 999 in a module whose string
        // table only has the module name (id 0) must be rejected.
        let bad_panic = Instruction::Panic { message_id: 999 };
        let module = build_module_with_instr(bad_panic, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        let has_err = matches!(&err, VbcError::InvalidStringId(999))
            || matches!(&err, VbcError::MultipleErrors(errs)
                if errs.iter().any(|e| matches!(e, VbcError::InvalidStringId(999))));
        assert!(has_err, "expected InvalidStringId(999), got: {:?}", err);
    }

    #[test]
    fn validator_rejects_try_begin_with_handler_past_function_end() {
        // TryBegin's handler_offset is a branch target; landing it
        // far past the function's bytecode region is rejected.  The
        // offset of 0x7FFF_FFFF can surface as either:
        //   * `JumpOutOfBounds` — our post-walk boundary check fires
        //     after the byte walk completes, OR
        //   * `InvalidInstructionEncoding` — the trailing Ret can't
        //     decode because the giant signed-varint offset consumed
        //     more bytes than the descriptor's `bytecode_length`
        //     budgeted for.
        // Both rejections satisfy the load-time-defense invariant.
        let bad_try = Instruction::TryBegin {
            handler_offset: 0x7FFF_FFFF,
        };
        let module = build_module_with_instr(bad_try, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        let any_acceptable = |e: &VbcError| matches!(
            e,
            VbcError::JumpOutOfBounds { .. } | VbcError::InvalidInstructionEncoding { .. }
        );
        let has_err = any_acceptable(&err)
            || matches!(&err, VbcError::MultipleErrors(errs)
                if errs.iter().any(any_acceptable));
        assert!(has_err, "expected JumpOutOfBounds or InvalidInstructionEncoding for TryBegin, got: {:?}", err);
    }

    #[test]
    fn validator_rejects_switch_with_case_target_past_function_end() {
        // Switch with one case-offset jumping far past EOF must be
        // rejected — every case offset is validated identically to
        // Jmp's.
        let bad_switch = Instruction::Switch {
            value: Reg(0),
            default_offset: 0,
            cases: vec![(7_u32, 0x7FFF_FFFF_i32)],
        };
        let module = build_module_with_instr(bad_switch, 4, 1);
        let err = validate_module(&module).expect_err("must reject");
        let has_err = matches!(&err, VbcError::JumpOutOfBounds { .. })
            || matches!(&err, VbcError::MultipleErrors(errs)
                if errs.iter().any(|e| matches!(e, VbcError::JumpOutOfBounds { .. })));
        assert!(has_err, "expected JumpOutOfBounds for Switch case, got: {:?}", err);
    }
}
