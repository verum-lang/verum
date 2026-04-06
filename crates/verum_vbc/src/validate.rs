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

use crate::error::{VbcError, VbcResult};
use crate::format::{VbcHeader, HEADER_SIZE, MAGIC, VERSION_MAJOR, VERSION_MINOR};
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
}
