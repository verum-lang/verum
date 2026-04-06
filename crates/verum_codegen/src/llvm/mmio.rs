//! MMIO (Memory-Mapped I/O) code generation for volatile memory operations.
//!
//! This module provides LLVM IR generation for volatile memory operations
//! used in hardware register access and MMIO programming.
//!
//! # Overview
//!
//! Volatile operations guarantee that:
//! - Reads are never optimized away or reordered
//! - Writes are never optimized away or reordered
//! - Memory barriers are properly placed for hardware synchronization
//!
//! # Generated Code Patterns
//!
//! ```llvm
//! ; Volatile read (32-bit register)
//! %val = load volatile i32, ptr %reg_addr, align 4
//!
//! ; Volatile write (32-bit register)
//! store volatile i32 %val, ptr %reg_addr, align 4
//!
//! ; Read-modify-write with atomic ordering
//! %old = atomicrmw or i32* %addr, i32 %mask seq_cst
//!
//! ; Memory barrier (full fence)
//! fence seq_cst
//! ```
//!
//! # Architecture-Specific Notes
//!
//! - **ARM**: Uses DMB (data memory barrier) for fences
//! - **x86**: Most volatile operations are naturally ordered
//! - **RISC-V**: Uses FENCE instruction
//!
//! # MMIO Volatile Codegen
//!
//! Verum provides type-safe MMIO through `Register<T, MODE>` wrappers with
//! `*volatile T` pointer types. Access modes (ReadOnly, WriteOnly, ReadWrite,
//! WriteOneToClear, etc.) are enforced at compile time. Volatile semantics
//! guarantee reads/writes are never optimized away or reordered. Memory barriers
//! (compiler_fence, hardware_fence) with explicit Ordering (Relaxed, Acquire,
//! Release, SeqCst) control synchronization. Architecture-specific: ARM uses
//! DMB barriers, x86 has naturally ordered volatile ops, RISC-V uses FENCE.

use verum_llvm::values::{BasicValue, IntValue, PointerValue};
use verum_llvm::types::IntType;
use verum_llvm::builder::Builder;
use verum_llvm::{AtomicOrdering, AtomicRMWBinOp};

use super::types::TypeLowering;
use super::error::{LlvmLoweringError, Result};

/// Memory ordering for volatile operations.
///
/// Controls the level of synchronization for volatile memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolatileOrdering {
    /// No ordering constraints beyond volatility.
    /// Suitable for device registers where hardware handles coherency.
    Relaxed,
    /// Acquire semantics: operations after the load cannot be reordered before it.
    /// Use for reading status registers before accessing data.
    Acquire,
    /// Release semantics: operations before the store cannot be reordered after it.
    /// Use for writing control registers after preparing data.
    Release,
    /// Full sequential consistency.
    /// Most conservative; use when uncertain about hardware requirements.
    SeqCst,
}

impl VolatileOrdering {
    /// Convert to LLVM atomic ordering.
    pub fn to_llvm(&self) -> AtomicOrdering {
        match self {
            VolatileOrdering::Relaxed => AtomicOrdering::Monotonic,
            VolatileOrdering::Acquire => AtomicOrdering::Acquire,
            VolatileOrdering::Release => AtomicOrdering::Release,
            VolatileOrdering::SeqCst => AtomicOrdering::SequentiallyConsistent,
        }
    }
}

/// Register width for MMIO operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterWidth {
    /// 8-bit register (U8)
    Byte,
    /// 16-bit register (U16)
    Half,
    /// 32-bit register (U32) - most common for embedded
    Word,
    /// 64-bit register (U64)
    Double,
}

impl RegisterWidth {
    /// Get the size in bytes.
    pub fn size_bytes(&self) -> u32 {
        match self {
            RegisterWidth::Byte => 1,
            RegisterWidth::Half => 2,
            RegisterWidth::Word => 4,
            RegisterWidth::Double => 8,
        }
    }

    /// Get the alignment in bytes.
    pub fn alignment(&self) -> u32 {
        self.size_bytes()
    }

    /// Get LLVM type for this width using a TypeLowering helper.
    pub fn llvm_type<'ctx>(&self, types: &TypeLowering<'ctx>) -> IntType<'ctx> {
        match self {
            RegisterWidth::Byte => types.i8_type(),
            RegisterWidth::Half => types.context().i16_type(),
            RegisterWidth::Word => types.i32_type(),
            RegisterWidth::Double => types.i64_type(),
        }
    }
}

/// Statistics for MMIO code generation.
#[derive(Debug, Clone, Default)]
pub struct MmioStats {
    /// Number of volatile loads generated.
    pub volatile_loads: usize,
    /// Number of volatile stores generated.
    pub volatile_stores: usize,
    /// Number of atomic RMW operations generated.
    pub atomic_rmw_ops: usize,
    /// Number of memory barriers generated.
    pub memory_barriers: usize,
}

impl MmioStats {
    /// Merge statistics from another instance.
    pub fn merge(&mut self, other: &MmioStats) {
        self.volatile_loads += other.volatile_loads;
        self.volatile_stores += other.volatile_stores;
        self.atomic_rmw_ops += other.atomic_rmw_ops;
        self.memory_barriers += other.memory_barriers;
    }

    /// Get total number of MMIO operations.
    pub fn total(&self) -> usize {
        self.volatile_loads + self.volatile_stores + self.atomic_rmw_ops + self.memory_barriers
    }
}

/// MMIO code generation context.
///
/// Holds state and statistics for MMIO operations in a function.
pub struct MmioLowering<'ctx> {
    /// Reference to the builder for generating instructions.
    builder: &'ctx Builder<'ctx>,
    /// Type lowering helper.
    types: &'ctx TypeLowering<'ctx>,
    /// Statistics.
    stats: MmioStats,
}

impl<'ctx> MmioLowering<'ctx> {
    /// Create a new MMIO lowering context.
    pub fn new(builder: &'ctx Builder<'ctx>, types: &'ctx TypeLowering<'ctx>) -> Self {
        Self {
            builder,
            types,
            stats: MmioStats::default(),
        }
    }

    /// Get accumulated statistics.
    pub fn stats(&self) -> &MmioStats {
        &self.stats
    }

    /// Generate a volatile load instruction.
    ///
    /// # Parameters
    /// - `ptr`: Pointer to the register/memory location
    /// - `width`: Width of the register
    /// - `name`: Name for the result value
    ///
    /// # Returns
    /// The loaded value as an integer.
    ///
    /// # Example
    ///
    /// ```verum
    /// // Reading a 32-bit status register
    /// let status = volatile_read(*volatile UART_STATUS);
    /// ```
    pub fn volatile_load(
        &mut self,
        ptr: PointerValue<'ctx>,
        width: RegisterWidth,
        name: &str,
    ) -> Result<IntValue<'ctx>> {
        let int_type = width.llvm_type(self.types);

        // Build volatile load instruction
        let load_instr = self.builder
            .build_load(int_type, ptr, name)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Mark as volatile
        if let Some(instr) = load_instr.as_instruction_value() {
            // Set volatile flag - in LLVM this makes the load volatile
            // The actual volatile flag is set via the instruction's metadata
            let _ = instr.set_volatile(true);
        }

        // Track statistics
        self.stats.volatile_loads += 1;

        Ok(load_instr.into_int_value())
    }

    /// Generate a volatile store instruction.
    ///
    /// # Parameters
    /// - `ptr`: Pointer to the register/memory location
    /// - `value`: Value to store
    ///
    /// # Example
    ///
    /// ```verum
    /// // Writing to a control register
    /// volatile_write(*volatile mut GPIO_CTRL, 0x0F);
    /// ```
    pub fn volatile_store(
        &mut self,
        ptr: PointerValue<'ctx>,
        value: IntValue<'ctx>,
    ) -> Result<()> {
        // Build volatile store instruction
        let store_instr = self.builder
            .build_store(ptr, value)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Mark as volatile
        let _ = store_instr.set_volatile(true);

        // Track statistics
        self.stats.volatile_stores += 1;

        Ok(())
    }

    /// Generate an atomic read-modify-write operation.
    ///
    /// This is used for operations like setting/clearing specific bits in a register.
    ///
    /// # Parameters
    /// - `ptr`: Pointer to the register
    /// - `value`: Value to combine with the register
    /// - `op`: The RMW operation (OR for set_bits, AND for clear_bits, etc.)
    /// - `ordering`: Memory ordering
    ///
    /// # Returns
    /// The previous value of the register.
    ///
    /// # Example
    ///
    /// ```verum
    /// // Set bits 0-3 atomically
    /// let old = atomic_or(*volatile mut STATUS, 0x0F);
    /// ```
    pub fn atomic_rmw(
        &mut self,
        ptr: PointerValue<'ctx>,
        value: IntValue<'ctx>,
        op: AtomicRMWBinOp,
        ordering: VolatileOrdering,
    ) -> Result<IntValue<'ctx>> {
        let result = self.builder
            .build_atomicrmw(op, ptr, value, ordering.to_llvm())
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Track statistics
        self.stats.atomic_rmw_ops += 1;

        Ok(result)
    }

    /// Generate a memory barrier (fence instruction).
    ///
    /// This ensures all memory operations before the fence complete
    /// before any operations after it begin.
    ///
    /// # Parameters
    /// - `ordering`: Memory ordering for the fence
    ///
    /// # Example
    ///
    /// ```verum
    /// // Ensure all prior writes are visible before continuing
    /// memory_barrier();
    /// ```
    pub fn memory_barrier(&mut self, ordering: VolatileOrdering) -> Result<()> {
        self.builder
            .build_fence(ordering.to_llvm(), false, "fence")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Track statistics
        self.stats.memory_barriers += 1;

        Ok(())
    }

    /// Generate a set_bits operation for a register.
    ///
    /// Atomically sets the specified bits in the register.
    ///
    /// # Parameters
    /// - `ptr`: Pointer to the register
    /// - `mask`: Bitmask of bits to set
    ///
    /// # Returns
    /// The previous value of the register.
    pub fn set_bits(
        &mut self,
        ptr: PointerValue<'ctx>,
        mask: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        self.atomic_rmw(ptr, mask, AtomicRMWBinOp::Or, VolatileOrdering::SeqCst)
    }

    /// Generate a clear_bits operation for a register.
    ///
    /// Atomically clears the specified bits in the register.
    ///
    /// # Parameters
    /// - `ptr`: Pointer to the register
    /// - `mask`: Bitmask of bits to clear (bits set to 1 will be cleared)
    ///
    /// # Returns
    /// The previous value of the register.
    pub fn clear_bits(
        &mut self,
        ptr: PointerValue<'ctx>,
        mask: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        // AND with inverted mask to clear bits
        let inverted = self.builder
            .build_not(mask, "inv_mask")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        self.atomic_rmw(ptr, inverted, AtomicRMWBinOp::And, VolatileOrdering::SeqCst)
    }

    /// Generate a toggle_bits operation for a register.
    ///
    /// Atomically toggles (XORs) the specified bits in the register.
    ///
    /// # Parameters
    /// - `ptr`: Pointer to the register
    /// - `mask`: Bitmask of bits to toggle
    ///
    /// # Returns
    /// The previous value of the register.
    pub fn toggle_bits(
        &mut self,
        ptr: PointerValue<'ctx>,
        mask: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        self.atomic_rmw(ptr, mask, AtomicRMWBinOp::Xor, VolatileOrdering::SeqCst)
    }

    /// Generate a modify_bits operation for a register.
    ///
    /// Reads, modifies, and writes back a register with a mask.
    /// Equivalent to: `(reg & ~clear_mask) | set_mask`
    ///
    /// # Parameters
    /// - `ptr`: Pointer to the register
    /// - `clear_mask`: Bits to clear (1 = clear)
    /// - `set_mask`: Bits to set (1 = set)
    /// - `width`: Width of the register
    ///
    /// # Returns
    /// The previous value of the register.
    ///
    /// # Note
    ///
    /// This is NOT atomic as a single operation. If atomicity is required,
    /// use a critical section.
    pub fn modify_bits(
        &mut self,
        ptr: PointerValue<'ctx>,
        clear_mask: IntValue<'ctx>,
        set_mask: IntValue<'ctx>,
        width: RegisterWidth,
    ) -> Result<IntValue<'ctx>> {
        // Read current value
        let current = self.volatile_load(ptr, width, "current")?;

        // Clear specified bits
        let inverted = self.builder
            .build_not(clear_mask, "inv_clear")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        let cleared = self.builder
            .build_and(current, inverted, "cleared")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Set specified bits
        let modified = self.builder
            .build_or(cleared, set_mask, "modified")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Write back
        self.volatile_store(ptr, modified)?;

        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_width_sizes() {
        assert_eq!(RegisterWidth::Byte.size_bytes(), 1);
        assert_eq!(RegisterWidth::Half.size_bytes(), 2);
        assert_eq!(RegisterWidth::Word.size_bytes(), 4);
        assert_eq!(RegisterWidth::Double.size_bytes(), 8);
    }

    #[test]
    fn test_volatile_ordering() {
        assert_eq!(
            VolatileOrdering::Relaxed.to_llvm(),
            AtomicOrdering::Monotonic
        );
        assert_eq!(
            VolatileOrdering::Acquire.to_llvm(),
            AtomicOrdering::Acquire
        );
        assert_eq!(
            VolatileOrdering::Release.to_llvm(),
            AtomicOrdering::Release
        );
        assert_eq!(
            VolatileOrdering::SeqCst.to_llvm(),
            AtomicOrdering::SequentiallyConsistent
        );
    }

    #[test]
    fn test_mmio_stats() {
        let mut stats = MmioStats::default();
        stats.volatile_loads = 5;
        stats.volatile_stores = 3;
        stats.atomic_rmw_ops = 2;
        stats.memory_barriers = 1;
        assert_eq!(stats.total(), 11);

        let mut other = MmioStats::default();
        other.volatile_loads = 2;
        stats.merge(&other);
        assert_eq!(stats.volatile_loads, 7);
    }
}
