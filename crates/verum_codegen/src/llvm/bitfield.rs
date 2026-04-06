//! Bitfield accessor code generation.
//!
//! This module provides LLVM IR generation for bitfield accessor methods,
//! implementing type-safe bit-level data manipulation.
//!
//! # Overview
//!
//! For each bitfield type, the compiler generates:
//! - `get_<field>() -> T`: Extract field value with proper masking/shifting
//! - `set_<field>(value: T) -> &mut Self`: Update field with bounds checking
//! - `with_<field>(value: T) -> Self`: Builder pattern for immutable updates
//!
//! # Generated Code Patterns
//!
//! ```llvm
//! ; Extract 4-bit field at offset 4 from 8-bit container
//! ; Verum: let version = header.get_version();
//! %shifted = lshr i8 %container, 4
//! %masked = and i8 %shifted, 15  ; 0b1111 = 15
//!
//! ; Set 4-bit field at offset 4 in 8-bit container
//! ; Verum: header.set_version(7);
//! %cleared = and i8 %container, -241  ; ~(15 << 4) = -241
//! %value_masked = and i8 %new_value, 15
//! %value_shifted = shl i8 %value_masked, 4
//! %result = or i8 %cleared, %value_shifted
//! ```
//!
//! # Byte Order Handling
//!
//! For multi-byte containers with specified byte order:
//! - Big-endian: Use `@llvm.bswap` intrinsic for load/store
//! - Little-endian: Direct access (native for most platforms)
//! - Native: Platform-dependent, may emit bswap
//!
//! # Bitfield System
//!
//! Verum provides first-class bitfield support via `@bitfield` type attribute and
//! `@bits(N)` field attributes. Key rules:
//! - `@bits(N)` specifies the bit width of a field within a bitfield type
//! - `@bitfield` enables bitfield semantics for the containing type
//! - `@endian(little|big|native)` controls byte order for multi-byte containers
//! - Compiler generates get/set/with accessor methods at compile time
//! - C ABI compatibility is maintained via `@repr(C)` for FFI interop
//! - Enum variants must fit within specified bit widths (compile-time verified)
//! - Total bits must align to byte boundaries (or require explicit padding)
//! - Non-bitfield fields in a `@bitfield` type allow pointer access

use verum_llvm::values::{BasicValueEnum, IntValue, PointerValue};
use verum_llvm::types::IntType;
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::module::Module;

use verum_ast::bitfield::{ByteOrder, ResolvedBitField, ResolvedBitLayout};

use super::error::{LlvmLoweringError, Result};

// =============================================================================
// STATISTICS
// =============================================================================

/// Statistics for bitfield code generation.
#[derive(Debug, Clone, Default)]
pub struct BitfieldStats {
    /// Number of getter methods generated
    pub getters_generated: usize,
    /// Number of setter methods generated
    pub setters_generated: usize,
    /// Number of with-builder methods generated
    pub with_builders_generated: usize,
    /// Number of byte-swap operations inserted
    pub byte_swaps: usize,
}

// =============================================================================
// BITFIELD LOWERING
// =============================================================================

/// Bitfield code generation context.
///
/// Provides LLVM IR generation for bitfield accessor methods.
pub struct BitfieldLowering<'ctx> {
    /// LLVM context
    context: &'ctx Context,
    /// LLVM IR builder
    builder: &'ctx Builder<'ctx>,
    /// Module for adding intrinsic declarations
    #[allow(dead_code)]
    module: &'ctx Module<'ctx>,
    /// Whether we're on a little-endian platform
    target_is_little_endian: bool,
    /// Statistics
    stats: BitfieldStats,
}

impl<'ctx> BitfieldLowering<'ctx> {
    /// Create a new bitfield lowering context.
    pub fn new(
        context: &'ctx Context,
        builder: &'ctx Builder<'ctx>,
        module: &'ctx Module<'ctx>,
        target_is_little_endian: bool,
    ) -> Self {
        Self {
            context,
            builder,
            module,
            target_is_little_endian,
            stats: BitfieldStats::default(),
        }
    }

    /// Get the accumulated statistics.
    pub fn stats(&self) -> &BitfieldStats {
        &self.stats
    }

    // =========================================================================
    // TYPE HELPERS
    // =========================================================================

    /// Get the integer type for a container of the given byte size.
    fn container_type(&self, bytes: u32) -> IntType<'ctx> {
        match bytes {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            3..=4 => self.context.i32_type(),
            5..=8 => self.context.i64_type(),
            _ => self.context.i128_type(),
        }
    }

    /// Check if byte swapping is needed for the given byte order.
    fn needs_byte_swap(&self, byte_order: ByteOrder) -> bool {
        match byte_order {
            ByteOrder::Big => self.target_is_little_endian,
            ByteOrder::Little => !self.target_is_little_endian,
            ByteOrder::Native => false,
        }
    }

    // =========================================================================
    // GETTER GENERATION
    // =========================================================================

    /// Generate code to extract a bitfield value.
    ///
    /// # Parameters
    /// - `container`: The loaded container value
    /// - `field`: The field specification
    /// - `layout`: The overall bitfield layout
    /// - `name`: Name for the resulting value
    ///
    /// # Returns
    /// The extracted field value, zero-extended to the field's natural type.
    pub fn build_field_get(
        &mut self,
        container: IntValue<'ctx>,
        field: &ResolvedBitField,
        layout: &ResolvedBitLayout,
        name: &str,
    ) -> Result<IntValue<'ctx>> {
        self.stats.getters_generated += 1;

        // Apply byte swap if needed
        let swapped = if self.needs_byte_swap(layout.byte_order) {
            self.stats.byte_swaps += 1;
            self.build_byte_swap(container, &format!("{}_bswap", name))?
        } else {
            container
        };

        // Shift right to position field at bit 0
        let shifted = if field.offset > 0 {
            let shift_amount = container.get_type().const_int(field.offset as u64, false);
            self.builder
                .build_right_shift(swapped, shift_amount, false, &format!("{}_shr", name))
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
        } else {
            swapped
        };

        // Mask to extract only the field bits
        let mask_value = if field.width >= 64 {
            u64::MAX
        } else {
            (1u64 << field.width) - 1
        };
        let mask = container.get_type().const_int(mask_value, false);
        let result = self
            .builder
            .build_and(shifted, mask, name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        Ok(result)
    }

    /// Generate code to set a bitfield value.
    ///
    /// # Parameters
    /// - `container`: The loaded container value
    /// - `new_value`: The new field value to set
    /// - `field`: The field specification
    /// - `layout`: The overall bitfield layout
    /// - `name`: Name for the resulting value
    ///
    /// # Returns
    /// The updated container value with the field modified.
    pub fn build_field_set(
        &mut self,
        container: IntValue<'ctx>,
        new_value: IntValue<'ctx>,
        field: &ResolvedBitField,
        layout: &ResolvedBitLayout,
        name: &str,
    ) -> Result<IntValue<'ctx>> {
        self.stats.setters_generated += 1;

        // Apply byte swap to container if needed
        let swapped_container = if self.needs_byte_swap(layout.byte_order) {
            self.stats.byte_swaps += 1;
            self.build_byte_swap(container, &format!("{}_cont_bswap", name))?
        } else {
            container
        };

        // Compute masks
        let mask_value = if field.width >= 64 {
            u64::MAX
        } else {
            (1u64 << field.width) - 1
        };
        let clear_mask_value = !(mask_value << field.offset);

        let container_type = container.get_type();

        // Clear the field bits in the container
        let clear_mask = container_type.const_int(clear_mask_value, false);
        let cleared = self
            .builder
            .build_and(swapped_container, clear_mask, &format!("{}_clear", name))
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        // Mask the new value to ensure it fits
        let value_mask = container_type.const_int(mask_value, false);

        // Extend new_value to container type if needed
        let extended_value = if new_value.get_type().get_bit_width() < container_type.get_bit_width() {
            self.builder
                .build_int_z_extend(new_value, container_type, &format!("{}_zext", name))
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
        } else if new_value.get_type().get_bit_width() > container_type.get_bit_width() {
            self.builder
                .build_int_truncate(new_value, container_type, &format!("{}_trunc", name))
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
        } else {
            new_value
        };

        let masked_value = self
            .builder
            .build_and(extended_value, value_mask, &format!("{}_mask", name))
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        // Shift the value to its position
        let shifted_value = if field.offset > 0 {
            let shift_amount = container_type.const_int(field.offset as u64, false);
            self.builder
                .build_left_shift(masked_value, shift_amount, &format!("{}_shl", name))
                .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
        } else {
            masked_value
        };

        // OR the shifted value into the cleared container
        let updated = self
            .builder
            .build_or(cleared, shifted_value, &format!("{}_or", name))
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        // Apply byte swap back if needed
        let result = if self.needs_byte_swap(layout.byte_order) {
            self.build_byte_swap(updated, &format!("{}_result_bswap", name))?
        } else {
            updated
        };

        Ok(result)
    }

    // =========================================================================
    // LOAD/STORE WITH BYTE ORDER
    // =========================================================================

    /// Load a bitfield container from memory with byte order handling.
    pub fn build_container_load(
        &mut self,
        ptr: PointerValue<'ctx>,
        layout: &ResolvedBitLayout,
        name: &str,
    ) -> Result<IntValue<'ctx>> {
        let int_type = self.container_type(layout.total_bytes);

        let loaded = self
            .builder
            .build_load(int_type, ptr, name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
            .into_int_value();

        Ok(loaded)
    }

    /// Store a bitfield container to memory.
    pub fn build_container_store(
        &mut self,
        value: IntValue<'ctx>,
        ptr: PointerValue<'ctx>,
    ) -> Result<()> {
        self.builder
            .build_store(ptr, value)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?;

        Ok(())
    }

    // =========================================================================
    // BYTE SWAP INTRINSIC
    // =========================================================================

    /// Build a byte swap operation using LLVM intrinsic.
    fn build_byte_swap(
        &self,
        value: IntValue<'ctx>,
        name: &str,
    ) -> Result<IntValue<'ctx>> {
        let bit_width = value.get_type().get_bit_width();

        // bswap only works for 16, 32, 64, 128 bit values
        if bit_width < 16 {
            // For 8-bit values, no swap needed
            return Ok(value);
        }

        let intrinsic_name = match bit_width {
            16 => "llvm.bswap.i16",
            32 => "llvm.bswap.i32",
            64 => "llvm.bswap.i64",
            128 => "llvm.bswap.i128",
            _ => {
                return Err(LlvmLoweringError::InvalidType(
                    format!("Cannot byte-swap {}-bit value", bit_width).into(),
                ))
            }
        };

        // Get or declare the intrinsic
        let fn_type = value.get_type().fn_type(&[value.get_type().into()], false);
        let intrinsic = self.module.get_function(intrinsic_name).unwrap_or_else(|| {
            self.module.add_function(intrinsic_name, fn_type, None)
        });

        let result = self
            .builder
            .build_call(intrinsic, &[value.into()], name)
            .map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| {
                LlvmLoweringError::InvalidType("bswap should return a value".into())
            })?
            .into_int_value();

        Ok(result)
    }
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Compute the minimum container size (in bytes) for a bitfield.
pub fn min_container_bytes(total_bits: u32) -> u32 {
    (total_bits + 7) / 8
}

/// Compute the optimal container type bit width.
pub fn optimal_container_bits(total_bits: u32) -> u32 {
    if total_bits <= 8 {
        8
    } else if total_bits <= 16 {
        16
    } else if total_bits <= 32 {
        32
    } else if total_bits <= 64 {
        64
    } else {
        128
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_container_bytes() {
        assert_eq!(min_container_bytes(1), 1);
        assert_eq!(min_container_bytes(8), 1);
        assert_eq!(min_container_bytes(9), 2);
        assert_eq!(min_container_bytes(16), 2);
        assert_eq!(min_container_bytes(17), 3);
        assert_eq!(min_container_bytes(32), 4);
    }

    #[test]
    fn test_optimal_container_bits() {
        assert_eq!(optimal_container_bits(1), 8);
        assert_eq!(optimal_container_bits(8), 8);
        assert_eq!(optimal_container_bits(9), 16);
        assert_eq!(optimal_container_bits(16), 16);
        assert_eq!(optimal_container_bits(17), 32);
        assert_eq!(optimal_container_bits(32), 32);
        assert_eq!(optimal_container_bits(33), 64);
    }
}
