//! Register file management.
//!
//! VBC uses a register-based architecture where each function has a fixed
//! number of registers allocated at compile time. The register file is
//! a growable array shared across all call frames.
//!
//! # Layout
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────────────┐
//! │                         REGISTER FILE                                   │
//! ├────────────────────────────────────────────────────────────────────────┤
//! │ Frame 0 (base=0)  │ Frame 1 (base=16) │ Frame 2 (base=48) │ ...       │
//! │ [r0..r15]         │ [r0..r31]         │ [r0..r7]          │           │
//! └────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Each frame's registers are accessed relative to a base offset.

use crate::instruction::Reg;
use crate::value::Value;

/// Default initial capacity for register file.
const INITIAL_CAPACITY: usize = 1024;

/// Maximum register file size (to prevent runaway allocation).
const MAX_SIZE: usize = 16 * 1024 * 1024; // 16M registers = 128MB

/// Maximum generation value before overflow triggers epoch advancement.
/// Matches spec: GEN_MAX = 0xFFFF_FFFE (leaves room for wraparound detection).
pub const GEN_MAX: u32 = 0xFFFF_FFFE;

/// Initial generation value for fresh allocations.
/// Matches spec: GEN_INITIAL = 1 (0 means unallocated/invalid).
pub const GEN_INITIAL: u32 = 1;

/// Sentinel generation value indicating no CBGR check needed (Tier 1/2).
/// Uses 0x7FFE to stay within NaN-boxing range while being distinct from valid generations.
pub const GEN_NO_CHECK: u32 = 0x7FFE;

/// Register file - storage for all registers across frames.
///
/// The register file is a contiguous array where each frame occupies
/// a slice starting at `base`. Registers within a frame are accessed
/// as `base + reg.0`.
///
/// # CBGR Generation and Epoch Tracking
///
/// Each register slot has associated generation and epoch counters used by
/// the CBGR (Capability-Based Generational References) system:
///
/// - **Generation (32-bit)**: Bumped when variable goes out of scope
/// - **Epoch (16-bit)**: Incremented when generation overflows, prevents ABA problem
///
/// When a variable goes out of scope, its slot's generation is bumped,
/// invalidating any references that captured the old generation.
/// If generation would overflow GEN_MAX, epoch is incremented instead.
///
/// CBGR register-level tracking: each register slot has a generation counter. On reassignment,
/// generation increments, invalidating references that captured the old generation. ThinRef<T>
/// is 16 bytes (ptr + generation + epoch_caps); FatRef<T> is 24 bytes (adds len). Tier 0 checks
/// compare reference generation vs allocation generation at ~15ns per deref.
#[derive(Debug)]
pub struct RegisterFile {
    /// Register storage.
    registers: Vec<Value>,

    /// CBGR generation counter per register slot.
    /// Parallel to `registers`: `slot_generations[i]` is the generation for `registers[i]`.
    /// Bumped when a variable goes out of scope to invalidate dangling references.
    slot_generations: Vec<u32>,

    /// CBGR epoch counter per register slot.
    /// Parallel to `registers`: `slot_epochs[i]` is the epoch for `registers[i]`.
    /// Incremented when generation would overflow, preventing ABA problem.
    slot_epochs: Vec<u16>,

    /// Current top (next free position).
    top: usize,

    /// Global epoch counter for this register file.
    /// Incremented when any slot's generation overflows.
    global_epoch: u64,
}

impl Default for RegisterFile {
    fn default() -> Self {
        Self::new()
    }
}

impl RegisterFile {
    /// Creates a new register file with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(INITIAL_CAPACITY)
    }

    /// Creates a new register file with the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            registers: vec![Value::unit(); capacity],
            slot_generations: vec![GEN_INITIAL; capacity],
            slot_epochs: vec![0u16; capacity],
            top: 0,
            global_epoch: 0,
        }
    }

    /// Allocates registers for a new frame.
    ///
    /// Returns the base offset for the frame's registers.
    ///
    /// # Panics
    ///
    /// Panics if the register file exceeds [`MAX_SIZE`].
    pub fn push_frame(&mut self, count: u16) -> u32 {
        let base = self.top as u32;
        let new_top = self.top + count as usize;

        // Grow if needed
        if new_top > self.registers.len() {
            let new_size = (new_top * 2).min(MAX_SIZE);
            if new_top > MAX_SIZE {
                panic!(
                    "Register file overflow: requested {} registers, max {}",
                    new_top, MAX_SIZE
                );
            }
            self.registers.resize(new_size, Value::unit());
            self.slot_generations.resize(new_size, GEN_INITIAL);
            self.slot_epochs.resize(new_size, 0);
        }

        // Initialize registers to unit and bump generations.
        // Bumping (rather than resetting) ensures that CBGR register refs
        // captured from a previous frame occupying these slots are invalidated.
        for r in &mut self.registers[self.top..new_top] {
            *r = Value::unit();
        }
        for g in &mut self.slot_generations[self.top..new_top] {
            *g = g.wrapping_add(1);
        }

        self.top = new_top;
        base
    }

    /// Releases registers from the current frame.
    ///
    /// Bumps generations of all popped slots so that any CBGR register refs
    /// pointing to them become stale (generation mismatch). Then sets top back.
    pub fn pop_frame(&mut self, base: u32) {
        debug_assert!(
            (base as usize) <= self.top,
            "Invalid frame base {} (top: {})",
            base,
            self.top
        );
        // Bump generations of popped slots to invalidate captured CBGR refs
        for g in &mut self.slot_generations[base as usize..self.top] {
            *g = g.wrapping_add(1);
        }
        self.top = base as usize;
    }

    /// Gets a register value.
    ///
    /// The register is accessed as `base + reg.0`.
    #[inline(always)]
    pub fn get(&self, base: u32, reg: Reg) -> Value {
        let idx = (base + reg.0 as u32) as usize;
        if idx >= self.registers.len() {
            return Value::unit(); // Return unit for out-of-bounds register reads from bad bytecode
        }
        self.registers[idx]
    }

    /// Sets a register value.
    ///
    /// The register is accessed as `base + reg.0`.
    #[inline(always)]
    pub fn set(&mut self, base: u32, reg: Reg, value: Value) {
        let idx = (base + reg.0 as u32) as usize;
        if idx >= self.registers.len() {
            return; // Silently ignore out-of-bounds register writes from bad bytecode
        }
        self.registers[idx] = value;
    }

    /// Gets a register value without bounds checking.
    ///
    /// # Safety
    ///
    /// The caller must ensure `base + reg.0 < top`.
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, base: u32, reg: Reg) -> Value {
        let idx = (base + reg.0 as u32) as usize;
        // SAFETY: debug_assert validates bounds in debug builds (compiles away in release).
        // This catches register index bugs early without impacting production performance.
        debug_assert!(
            idx < self.top,
            "get_unchecked: register r{} (absolute idx {}) out of bounds (top={})",
            reg.0, idx, self.top
        );
        debug_assert!(
            idx < self.registers.len(),
            "get_unchecked: register idx {} exceeds register file size {}",
            idx, self.registers.len()
        );
        // SAFETY: Caller guarantees idx is in bounds; debug_assert verifies in debug.
        unsafe { *self.registers.get_unchecked(idx) }
    }

    /// Sets a register value without bounds checking.
    ///
    /// # Safety
    ///
    /// The caller must ensure `base + reg.0 < top`.
    #[inline(always)]
    pub unsafe fn set_unchecked(&mut self, base: u32, reg: Reg, value: Value) {
        let idx = (base + reg.0 as u32) as usize;
        // SAFETY: debug_assert validates bounds in debug builds (compiles away in release).
        debug_assert!(
            idx < self.top,
            "set_unchecked: register r{} (absolute idx {}) out of bounds (top={})",
            reg.0, idx, self.top
        );
        debug_assert!(
            idx < self.registers.len(),
            "set_unchecked: register idx {} exceeds register file size {}",
            idx, self.registers.len()
        );
        // SAFETY: Caller guarantees idx is in bounds; debug_assert verifies in debug.
        unsafe { *self.registers.get_unchecked_mut(idx) = value; }
    }

    /// Gets a register value by absolute index (no base offset).
    ///
    /// Used for mutable reference dereferencing where the absolute register
    /// index is stored as the reference value.
    #[inline(always)]
    pub fn get_absolute(&self, abs_index: u32) -> Value {
        let idx = abs_index as usize;
        debug_assert!(idx < self.top, "Absolute register index {} out of bounds", abs_index);
        self.registers[idx]
    }

    /// Sets a register value by absolute index (no base offset).
    ///
    /// Used for mutable reference write-through where the absolute register
    /// index is stored as the reference value.
    #[inline(always)]
    pub fn set_absolute(&mut self, abs_index: u32, value: Value) {
        let idx = abs_index as usize;
        debug_assert!(idx < self.top, "Absolute register index {} out of bounds", abs_index);
        self.registers[idx] = value;
    }

    /// Gets the CBGR generation for a register slot by absolute index.
    ///
    /// Used when creating Tier 0 references: the generation at creation time
    /// is embedded in the reference value so it can be validated on dereference.
    #[inline(always)]
    pub fn get_generation(&self, abs_index: u32) -> u32 {
        let idx = abs_index as usize;
        debug_assert!(idx < self.top, "Generation index {} out of bounds", abs_index);
        self.slot_generations[idx]
    }

    /// Bumps the CBGR generation for a register slot by absolute index.
    ///
    /// Called when a variable goes out of scope. Incrementing the generation
    /// invalidates any references that captured the old generation value,
    /// enabling use-after-free detection on subsequent dereferences.
    ///
    /// If generation would exceed GEN_MAX, epoch is incremented instead to prevent
    /// the ABA problem where generation wraps to a previously-used value.
    #[inline(always)]
    pub fn bump_generation(&mut self, abs_index: u32) {
        let idx = abs_index as usize;
        debug_assert!(idx < self.top, "Generation index {} out of bounds", abs_index);

        let current_gen = self.slot_generations[idx];
        if current_gen >= GEN_MAX {
            // Generation overflow: increment epoch instead of wrapping generation
            self.slot_epochs[idx] = self.slot_epochs[idx].wrapping_add(1);
            self.slot_generations[idx] = GEN_INITIAL;
            self.global_epoch = self.global_epoch.wrapping_add(1);
        } else {
            self.slot_generations[idx] = current_gen + 1;
        }
    }

    /// Gets the CBGR epoch for a register slot by absolute index.
    ///
    /// Used when creating Tier 0 references: the epoch at creation time
    /// is embedded in the reference value for wraparound protection.
    #[inline(always)]
    pub fn get_epoch(&self, abs_index: u32) -> u16 {
        let idx = abs_index as usize;
        debug_assert!(idx < self.top, "Epoch index {} out of bounds", abs_index);
        self.slot_epochs[idx]
    }

    /// Gets both generation and epoch for a register slot atomically.
    ///
    /// This is the preferred method for reference creation as it ensures
    /// consistent generation/epoch pairs.
    #[inline(always)]
    pub fn get_generation_epoch(&self, abs_index: u32) -> (u32, u16) {
        let idx = abs_index as usize;
        debug_assert!(idx < self.top, "Generation/epoch index {} out of bounds", abs_index);
        (self.slot_generations[idx], self.slot_epochs[idx])
    }

    /// Validates a reference's generation and epoch against current values.
    ///
    /// Returns true if the reference is still valid (generation and epoch match).
    #[inline(always)]
    pub fn validate_ref(&self, abs_index: u32, ref_gen: u32, ref_epoch: u16) -> bool {
        // Special sentinel for Tier 1/2 references (skip validation)
        if ref_gen == GEN_NO_CHECK {
            return true;
        }

        let idx = abs_index as usize;
        if idx >= self.top {
            return false;
        }

        self.slot_generations[idx] == ref_gen && self.slot_epochs[idx] == ref_epoch
    }

    /// Returns the global epoch counter.
    pub fn global_epoch(&self) -> u64 {
        self.global_epoch
    }

    /// Returns the current top offset.
    pub fn top(&self) -> usize {
        self.top
    }

    /// Returns the total capacity.
    pub fn capacity(&self) -> usize {
        self.registers.len()
    }

    /// Clears all registers (resets top to 0).
    pub fn clear(&mut self) {
        self.top = 0;
    }

    /// Copies registers from one range to another.
    ///
    /// Used for argument passing during function calls.
    pub fn copy_range(&mut self, src_base: u32, src_start: Reg, dst_base: u32, dst_start: Reg, count: u16) {
        for i in 0..count {
            let src_idx = (src_base + src_start.0 as u32 + i as u32) as usize;
            let dst_idx = (dst_base + dst_start.0 as u32 + i as u32) as usize;
            debug_assert!(src_idx < self.top && dst_idx < self.top);
            self.registers[dst_idx] = self.registers[src_idx];
        }
    }

    /// Swaps two registers.
    #[inline]
    pub fn swap(&mut self, base: u32, r1: Reg, r2: Reg) {
        let idx1 = (base + r1.0 as u32) as usize;
        let idx2 = (base + r2.0 as u32) as usize;
        debug_assert!(idx1 < self.top && idx2 < self.top);
        self.registers.swap(idx1, idx2);
    }

    /// Returns an iterator over registers in a frame.
    pub fn frame_iter(&self, base: u32, count: u16) -> impl Iterator<Item = Value> + '_ {
        let start = base as usize;
        let end = (start + count as usize).min(self.top);
        self.registers[start..end].iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_file_creation() {
        let rf = RegisterFile::new();
        assert_eq!(rf.top(), 0);
        assert!(rf.capacity() >= INITIAL_CAPACITY);
    }

    #[test]
    fn test_push_pop_frame() {
        let mut rf = RegisterFile::new();

        // Push first frame
        let base1 = rf.push_frame(10);
        assert_eq!(base1, 0);
        assert_eq!(rf.top(), 10);

        // Push second frame
        let base2 = rf.push_frame(20);
        assert_eq!(base2, 10);
        assert_eq!(rf.top(), 30);

        // Pop second frame
        rf.pop_frame(base2);
        assert_eq!(rf.top(), 10);

        // Pop first frame
        rf.pop_frame(base1);
        assert_eq!(rf.top(), 0);
    }

    #[test]
    fn test_get_set() {
        let mut rf = RegisterFile::new();
        let base = rf.push_frame(16);

        // Set and get
        rf.set(base, Reg(0), Value::from_i64(42));
        rf.set(base, Reg(5), Value::from_bool(true));
        rf.set(base, Reg(15), Value::from_f64(3.14));

        assert_eq!(rf.get(base, Reg(0)).as_i64(), 42);
        assert!(rf.get(base, Reg(5)).as_bool());
        assert_eq!(rf.get(base, Reg(15)).as_f64(), 3.14);

        // Unset registers should be unit
        assert!(rf.get(base, Reg(1)).is_unit());
    }

    #[test]
    fn test_copy_range() {
        let mut rf = RegisterFile::new();
        let base1 = rf.push_frame(10);
        let base2 = rf.push_frame(10);

        // Set up source
        rf.set(base1, Reg(0), Value::from_i64(1));
        rf.set(base1, Reg(1), Value::from_i64(2));
        rf.set(base1, Reg(2), Value::from_i64(3));

        // Copy to destination
        rf.copy_range(base1, Reg(0), base2, Reg(5), 3);

        // Verify
        assert_eq!(rf.get(base2, Reg(5)).as_i64(), 1);
        assert_eq!(rf.get(base2, Reg(6)).as_i64(), 2);
        assert_eq!(rf.get(base2, Reg(7)).as_i64(), 3);
    }

    #[test]
    fn test_swap() {
        let mut rf = RegisterFile::new();
        let base = rf.push_frame(4);

        rf.set(base, Reg(0), Value::from_i64(10));
        rf.set(base, Reg(1), Value::from_i64(20));

        rf.swap(base, Reg(0), Reg(1));

        assert_eq!(rf.get(base, Reg(0)).as_i64(), 20);
        assert_eq!(rf.get(base, Reg(1)).as_i64(), 10);
    }

    #[test]
    fn test_frame_iter() {
        let mut rf = RegisterFile::new();
        let base = rf.push_frame(5);

        for i in 0..5 {
            rf.set(base, Reg(i), Value::from_i64(i as i64));
        }

        let values: Vec<_> = rf.frame_iter(base, 5).collect();
        assert_eq!(values.len(), 5);
        for (i, v) in values.iter().enumerate() {
            assert_eq!(v.as_i64(), i as i64);
        }
    }

    #[test]
    fn test_clear() {
        let mut rf = RegisterFile::new();
        rf.push_frame(100);
        assert_eq!(rf.top(), 100);

        rf.clear();
        assert_eq!(rf.top(), 0);
    }

    #[test]
    fn test_multiple_frames() {
        let mut rf = RegisterFile::new();

        // Simulate a call stack
        let bases: Vec<u32> = (0..100).map(|_| rf.push_frame(16)).collect();

        assert_eq!(rf.top(), 1600);

        // Set a value in each frame
        for (i, &base) in bases.iter().enumerate() {
            rf.set(base, Reg(0), Value::from_i64(i as i64));
        }

        // Verify values are separate
        for (i, &base) in bases.iter().enumerate() {
            assert_eq!(rf.get(base, Reg(0)).as_i64(), i as i64);
        }
    }

    #[test]
    fn test_growth() {
        let mut rf = RegisterFile::with_capacity(16);
        assert_eq!(rf.capacity(), 16);

        // Force growth
        let base = rf.push_frame(100);
        assert!(rf.capacity() >= 100);
        assert_eq!(rf.top(), 100);

        // Access should work
        rf.set(base, Reg(99), Value::from_i64(99));
        assert_eq!(rf.get(base, Reg(99)).as_i64(), 99);
    }

    #[test]
    fn test_unsafe_access() {
        let mut rf = RegisterFile::new();
        let base = rf.push_frame(16);

        rf.set(base, Reg(5), Value::from_i64(123));

        unsafe {
            assert_eq!(rf.get_unchecked(base, Reg(5)).as_i64(), 123);

            rf.set_unchecked(base, Reg(5), Value::from_i64(456));
            assert_eq!(rf.get_unchecked(base, Reg(5)).as_i64(), 456);
        }
    }
}
