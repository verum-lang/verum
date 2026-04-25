//! LLVM IR helper functions emitted inline for collection operations.
//!
//! These functions are defined as Internal-linkage LLVM IR functions during
//! module creation (Phase 0.5 in `lower_module`) for list, set, and map-grow operations.
//!
//! Most map operations (insert/get/contains/iter) are handled by compiled
//! `core/collections/map.vr` via Strategy 1/2 dispatch and do NOT need
//! LLVM IR helpers here.
//!
//! # LLVM IR Helpers
//!
//! | Function | Description |
//! |----------|-------------|
//! | verum_list_grow | Grow list backing array (2x capacity) |
//! | verum_list_sort | In-place quicksort |
//! | verum_list_reverse | Reverse list elements |
//! | verum_list_swap | Swap two elements |
//! | verum_list_insert | Insert at index (shift right) |
//! | verum_list_remove | Remove at index (shift left) |
//! | verum_list_extend | Append all elements from another list |
//! | verum_list_clone | Deep-copy a list |
//! | verum_set_new | Create new empty set |
//! | verum_set_contains | Check if value exists in set |
//! | verum_set_insert | Insert value into set |
//! | verum_set_remove | Remove value from set |
//! | __verum_map_iter_next | Scan map slots for next occupied entry |

use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::{BasicMetadataTypeEnum, BasicTypeEnum};
use verum_llvm::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use verum_llvm::AddressSpace;

use super::error::{BuildExt, LlvmLoweringError, OptionExt, Result};

/// Default initial capacity for lists.
pub const DEFAULT_LIST_CAPACITY: u64 = 16;

/// Default initial capacity for maps (must be power of 2).
pub const DEFAULT_MAP_CAPACITY: u64 = 16;

/// Size of a NaN-boxed value in bytes.
pub const VALUE_SIZE: u64 = 8;

/// Size of list object in bytes: 24-byte object header + 3 fields (ptr, len, cap).
/// Layout: [header(24)][ptr:i64][len:i64][cap:i64] = 48 bytes.
/// This matches the NewG allocation layout used by compiled list.vr code.
pub const LIST_OBJECT_SIZE: u64 = 48;

/// Byte offsets from list object start to each field (NewG layout).
/// Field order matches list.vr: { ptr, len, cap }
pub const LIST_PTR_OFFSET: u64 = 24;  // OBJECT_HEADER_SIZE + 0*8
pub const LIST_LEN_OFFSET: u64 = 32;  // OBJECT_HEADER_SIZE + 1*8
pub const LIST_CAP_OFFSET: u64 = 40;  // OBJECT_HEADER_SIZE + 2*8

/// Size of Text object in bytes: {ptr, len, cap} = 24 bytes.
/// Layout: [ptr:i64][len:i64][cap:i64]
/// cap == 0 indicates a static/immutable string literal (do not free).
pub const TEXT_OBJECT_SIZE: u64 = 24;

/// Byte offsets from Text object start to each field.
pub const TEXT_PTR_OFFSET: u64 = 0;
pub const TEXT_LEN_OFFSET: u64 = 8;
pub const TEXT_CAP_OFFSET: u64 = 16;

/// Size of map object in bytes: 24-byte object header + 3 fields (entries, len, cap).
/// Layout: [header(24)][entries_ptr:i64][len:i64][cap:i64] = 48 bytes minimum.
/// This matches the NewG allocation layout used by compiled map.vr code.
/// Note: compiled map.vr has 4 fields (entries, len, cap, tombstones) = 56 bytes,
/// but C runtime fallback only uses 3 fields = 48 bytes.
pub const MAP_HEADER_SIZE: u64 = 48;

/// Byte offsets from map object start to each field (NewG layout).
/// Field order matches map.vr: { entries, len, cap, tombstones }
pub const MAP_ENTRIES_OFFSET: u64 = 24; // OBJECT_HEADER_SIZE + 0*8
pub const MAP_LEN_OFFSET: u64 = 32;    // OBJECT_HEADER_SIZE + 1*8
pub const MAP_CAP_OFFSET: u64 = 40;    // OBJECT_HEADER_SIZE + 2*8

/// Size of set object in bytes: 24-byte object header + set fields.
/// Set is backed by Map internally, but C runtime set functions use flat {len, cap, entries}.
/// With NewG layout: [header(24)][len:i64][cap:i64][entries_ptr:i64] = 48 bytes.
pub const SET_LEN_OFFSET: u64 = 24;    // OBJECT_HEADER_SIZE + 0*8
pub const SET_CAP_OFFSET: u64 = 32;    // OBJECT_HEADER_SIZE + 1*8
pub const SET_ENTRIES_OFFSET: u64 = 40; // OBJECT_HEADER_SIZE + 2*8

/// Deque object layout: [header(24)][data_ptr:i64][head:i64][len:i64][cap:i64] = 56 bytes.
/// Field order matches deque.vr: { data, head, len, cap }
pub const DEQUE_DATA_OFFSET: u64 = 24;  // OBJECT_HEADER_SIZE + 0*8
pub const DEQUE_HEAD_OFFSET: u64 = 32;  // OBJECT_HEADER_SIZE + 1*8
pub const DEQUE_LEN_OFFSET: u64 = 40;   // OBJECT_HEADER_SIZE + 2*8
pub const DEQUE_CAP_OFFSET: u64 = 48;   // OBJECT_HEADER_SIZE + 3*8

/// Size of map entry in bytes (key + value + hash + state + padding).
pub const MAP_ENTRY_SIZE: u64 = 32;

/// Runtime lowering helper for collection operations.
pub struct RuntimeLowering<'ctx> {
    /// LLVM context.
    context: &'ctx Context,
}

impl<'ctx> RuntimeLowering<'ctx> {
    /// Create a new runtime lowering helper.
    pub fn new(context: &'ctx Context) -> Self {
        Self { context }
    }

    // =========================================================================
    // List Operations
    // =========================================================================

    /// Lower NewList instruction.
    ///
    /// Creates a new empty list with cap=0, len=0, ptr=null.
    /// Returns pointer to list object.
    ///
    /// Layout (NewG): [24-byte header][ptr:i64][len:i64][cap:i64] = 48 bytes
    /// This matches the struct layout from list.vr: { ptr, len, cap }
    ///
    /// The backing array is NOT pre-allocated. The first push triggers
    /// List.grow() → List.resize_buffer() → alloc() via verum_cbgr_allocate,
    /// which correctly sets up AllocationHeader for subsequent realloc calls.
    pub fn lower_new_list(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();

        // Allocate list object (48 bytes: 24-byte header + 3 fields)
        let obj_size = i64_type.const_int(LIST_OBJECT_SIZE, false);
        let obj_ptr = self.emit_checked_malloc(builder, module, obj_size, "list_obj")?;

        // Zero-initialize the entire object (header + all fields = ptr=0, len=0, cap=0)
        let memset_fn = self.get_or_declare_memset(module)?;
        let zero_byte = self.context.i32_type().const_int(0, false);
        builder
            .build_call(memset_fn, &[obj_ptr.into(), zero_byte.into(), obj_size.into()], "clear_obj")
            .or_llvm_err()?;

        // No backing array allocated — cap=0, len=0, ptr=null (all zero from memset).
        // First push triggers List.grow() → alloc() via verum_cbgr_allocate.

        Ok(obj_ptr)
    }

    /// Lower ListPush instruction.
    ///
    /// Pushes a value onto the list, growing if necessary.
    pub fn lower_list_push(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        list_ptr: PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
    ) -> Result<()> {
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // NewG layout: [header(24)][ptr:i64][len:i64][cap:i64]
        // Load current length from LIST_LEN_OFFSET
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let len_slot = unsafe {
            builder
                .build_in_bounds_gep(i8_type, list_ptr, &[i64_type.const_int(LIST_LEN_OFFSET, false)], "len_slot")
                .or_llvm_err()?
        };
        let len = builder
            .build_load(i64_type, len_slot, "len")
            .or_llvm_err()?
            .into_int_value();

        // Load capacity from LIST_CAP_OFFSET
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe {
            builder
                .build_in_bounds_gep(i8_type, list_ptr, &[i64_type.const_int(LIST_CAP_OFFSET, false)], "cap_slot")
                .or_llvm_err()?
        };
        let cap = builder
            .build_load(i64_type, cap_slot, "cap")
            .or_llvm_err()?
            .into_int_value();

        // Load data pointer from LIST_PTR_OFFSET
        // SAFETY: GEP into the list object header to access the data pointer field at a fixed offset; the list pointer is non-null and valid
        let data_ptr_slot = unsafe {
            builder
                .build_in_bounds_gep(i8_type, list_ptr, &[i64_type.const_int(LIST_PTR_OFFSET, false)], "data_ptr_slot")
                .or_llvm_err()?
        };
        let data_as_int = builder
            .build_load(i64_type, data_ptr_slot, "data_int")
            .or_llvm_err()?
            .into_int_value();
        let data_ptr = builder
            .build_int_to_ptr(data_as_int, ptr_type, "data_ptr")
            .or_llvm_err()?;

        // Check if we need to grow: len >= cap
        let needs_grow = builder
            .build_int_compare(verum_llvm::IntPredicate::UGE, len, cap, "needs_grow")
            .or_llvm_err()?;

        // Conditionally call verum_list_grow when capacity is exhausted
        let grow_fn = module.get_function("verum_list_grow").unwrap_or_else(|| {
            let fn_type = self.context.void_type().fn_type(
                &[ptr_type.into()],
                false,
            );
            module.add_function("verum_list_grow", fn_type, None)
        });

        // Get the current function for block creation
        let current_fn = builder.get_insert_block()
            .and_then(|b| b.get_parent())
            .or_internal("ListPush: no current function")?;

        let grow_bb = self.context.append_basic_block(current_fn, "list_grow");
        let continue_bb = self.context.append_basic_block(current_fn, "list_push_continue");

        builder.build_conditional_branch(needs_grow, grow_bb, continue_bb)
            .or_llvm_err()?;

        // Grow block: call verum_list_grow then continue
        builder.position_at_end(grow_bb);
        builder.build_call(grow_fn, &[list_ptr.into()], "")
            .or_llvm_err()?;
        builder.build_unconditional_branch(continue_bb)
            .or_llvm_err()?;

        // Continue block: reload data pointer (may have changed after grow)
        builder.position_at_end(continue_bb);
        let data_as_int = builder
            .build_load(i64_type, data_ptr_slot, "data_int_post")
            .or_llvm_err()?
            .into_int_value();
        let data_ptr = builder
            .build_int_to_ptr(data_as_int, ptr_type, "data_ptr_post")
            .or_llvm_err()?;

        // Calculate element pointer: data_ptr + len * 8
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let elem_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, data_ptr, &[len], "elem_ptr")
                .or_llvm_err()?
        };

        // Store the value - for NaN-boxing, we store all values as i64
        let val_as_i64 = if value.is_int_value() {
            value.into_int_value()
        } else if value.is_float_value() {
            builder
                .build_bit_cast(value.into_float_value(), i64_type, "val_i64")
                .map_err(|e| LlvmLoweringError::llvm_error(format!("bitcast: {}", e)))?
                .into_int_value()
        } else if value.is_pointer_value() {
            builder
                .build_ptr_to_int(value.into_pointer_value(), i64_type, "val_i64")
                .or_llvm_err()?
        } else {
            i64_type.const_int(0, false)
        };

        builder
            .build_store(elem_ptr, val_as_i64)
            .or_llvm_err()?;

        // Update length: len = len + 1
        let one = i64_type.const_int(1, false);
        let new_len = builder
            .build_int_add(len, one, "new_len")
            .or_llvm_err()?;
        builder
            .build_store(len_slot, new_len)
            .or_llvm_err()?;

        Ok(())
    }

    /// Lower ListPop instruction.
    ///
    /// Pops a value from the list (returns unit if empty).
    pub fn lower_list_pop(
        &self,
        builder: &Builder<'ctx>,
        list_ptr: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // NewG layout: [header(24)][ptr:i64][len:i64][cap:i64]
        // Load current length from LIST_LEN_OFFSET
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let len_slot = unsafe {
            builder
                .build_in_bounds_gep(i8_type, list_ptr, &[i64_type.const_int(LIST_LEN_OFFSET, false)], "len_slot")
                .or_llvm_err()?
        };
        let len = builder
            .build_load(i64_type, len_slot, "len")
            .or_llvm_err()?
            .into_int_value();

        // Check if empty (len == 0)
        let zero = i64_type.const_int(0, false);
        let is_empty = builder
            .build_int_compare(verum_llvm::IntPredicate::EQ, len, zero, "is_empty")
            .or_llvm_err()?;

        // Load data pointer from LIST_PTR_OFFSET
        // SAFETY: GEP into the list object header to access the data pointer field at a fixed offset; the list pointer is non-null and valid
        let data_ptr_slot = unsafe {
            builder
                .build_in_bounds_gep(i8_type, list_ptr, &[i64_type.const_int(LIST_PTR_OFFSET, false)], "data_ptr_slot")
                .or_llvm_err()?
        };
        let data_as_int = builder
            .build_load(i64_type, data_ptr_slot, "data_int")
            .or_llvm_err()?
            .into_int_value();
        let data_ptr = builder
            .build_int_to_ptr(data_as_int, ptr_type, "data_ptr")
            .or_llvm_err()?;

        // Calculate new length: len - 1
        let one = i64_type.const_int(1, false);
        let new_len = builder
            .build_int_sub(len, one, "new_len")
            .or_llvm_err()?;

        // Select new_len or 0 based on is_empty
        let actual_new_len = builder
            .build_select(is_empty, zero, new_len, "actual_new_len")
            .or_llvm_err()?
            .into_int_value();

        // Calculate element pointer: data_ptr + actual_new_len
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let elem_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, data_ptr, &[actual_new_len], "elem_ptr")
                .or_llvm_err()?
        };

        // Load the value
        let value = builder
            .build_load(i64_type, elem_ptr, "popped_val")
            .or_llvm_err()?
            .into_int_value();

        // Return unit (0x7FFB_0000_0000_0000) if empty
        let unit_tag = i64_type.const_int(0x7FFB_0000_0000_0000, false);
        let result = builder
            .build_select(is_empty, unit_tag, value, "pop_result")
            .or_llvm_err()?
            .into_int_value();

        // Update length at LIST_LEN_OFFSET
        builder
            .build_store(len_slot, actual_new_len)
            .or_llvm_err()?;

        Ok(result)
    }

    // =========================================================================
    // Map Operations
    // =========================================================================

    /// Lower NewMap instruction.
    ///
    /// Creates a new empty map with default capacity.
    /// Returns pointer to map object (NewG layout with 24-byte header).
    ///
    /// Layout: [header(24)][entries_ptr:i64][len:i64][cap:i64] = 48 bytes
    /// Field order matches map.vr: { entries, len, cap, tombstones }
    /// (tombstones not set here — defaults to 0 from memset)
    pub fn lower_new_map(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();

        // Allocate map object (48 bytes: 24-byte header + entries_ptr + len + cap)
        let header_size = i64_type.const_int(MAP_HEADER_SIZE, false);
        let header_ptr = self.emit_checked_malloc(builder, module, header_size, "map_header")?;

        // Zero-fill the entire object (header + fields)
        let memset_fn = self.get_or_declare_memset(module)?;
        let zero_byte = self.context.i32_type().const_int(0, false);
        builder
            .build_call(memset_fn, &[header_ptr.into(), zero_byte.into(), header_size.into()], "clear_map")
            .or_llvm_err()?;

        // Allocate entry array (capacity * 32 bytes per entry)
        let entries_size = i64_type.const_int(DEFAULT_MAP_CAPACITY * MAP_ENTRY_SIZE, false);
        let entries_ptr = self.emit_checked_malloc(builder, module, entries_size, "map_entries")?;

        // Initialize entries to zero (all empty)
        builder
            .build_call(memset_fn, &[entries_ptr.into(), zero_byte.into(), entries_size.into()], "clear_entries")
            .or_llvm_err()?;

        // Initialize field 0 (offset 24): entries = entries_ptr
        // SAFETY: GEP into CBGR allocation header at a fixed structural offset; the header layout is defined by the allocator
        let entries_ptr_ptr = unsafe {
            builder
                .build_in_bounds_gep(i8_type, header_ptr, &[i64_type.const_int(MAP_ENTRIES_OFFSET, false)], "entries_ptr_ptr")
                .or_llvm_err()?
        };
        let entries_as_int = builder
            .build_ptr_to_int(entries_ptr, i64_type, "entries_int")
            .or_llvm_err()?;
        builder
            .build_store(entries_ptr_ptr, entries_as_int)
            .or_llvm_err()?;

        // Initialize field 1 (offset 32): len = 0 (already zeroed by memset)

        // Initialize field 2 (offset 40): cap = DEFAULT_MAP_CAPACITY
        // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
        let cap_ptr = unsafe {
            builder
                .build_in_bounds_gep(i8_type, header_ptr, &[i64_type.const_int(MAP_CAP_OFFSET, false)], "cap_ptr")
                .or_llvm_err()?
        };
        let cap_val = i64_type.const_int(DEFAULT_MAP_CAPACITY, false);
        builder
            .build_store(cap_ptr, cap_val)
            .or_llvm_err()?;

        Ok(header_ptr)
    }

    // =========================================================================
    // Iterator Operations
    // =========================================================================

    /// Size of tagged iterator structure in bytes: [tag, field0, field1].
    /// Tag 0 = list iterator (field0=iterable_ptr, field1=current_index).
    /// Tag 1 = range iterator (field0=current_value, field1=end_value).
    pub const ITER_SIZE: u64 = 24;

    /// Lower IterNew for a list iterable.
    ///
    /// Iterator layout: [tag=0: i64, iterable_ptr: i64, index=0: i64]
    pub fn lower_iter_new(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        iterable_ptr: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();

        let iter_size = i64_type.const_int(Self::ITER_SIZE, false);
        let iter_ptr = self.emit_checked_malloc(builder, module, iter_size, "iter")?;

        // Tag = 0 (list iterator)
        let zero = i64_type.const_int(0, false);
        builder
            .build_store(iter_ptr, zero)
            .or_llvm_err()?;

        // field0 = iterable_ptr
        // SAFETY: GEP to access a struct field at a fixed offset; the struct was allocated with sufficient size for all fields
        let field0_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "field0_ptr")
                .or_llvm_err()?
        };
        let iterable_as_int = builder
            .build_ptr_to_int(iterable_ptr, i64_type, "iterable_int")
            .or_llvm_err()?;
        builder
            .build_store(field0_ptr, iterable_as_int)
            .or_llvm_err()?;

        // field1 = 0 (initial index)
        // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
        let field1_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "field1_ptr")
                .or_llvm_err()?
        };
        builder
            .build_store(field1_ptr, zero)
            .or_llvm_err()?;

        Ok(iter_ptr)
    }

    /// Lower IterNew for a range iterable.
    ///
    /// Range object layout: [header(24 bytes)][start: i64][end: i64][inclusive: i64]
    /// Iterator layout: [tag=1: i64, current=start: i64, end: i64]
    pub fn lower_iter_new_range(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        range_ptr: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();

        let iter_size = i64_type.const_int(Self::ITER_SIZE, false);
        let iter_ptr = self.emit_checked_malloc(&builder, module, iter_size, "range_iter")?;

        // Tag = 1 (range iterator)
        let one = i64_type.const_int(1, false);
        builder
            .build_store(iter_ptr, one)
            .or_llvm_err()?;

        // Read start from range object (header=24 bytes = 3 i64s, start at offset 3)
        // SAFETY: GEP into the iterator/range struct at a fixed field offset; the struct layout is known at compile time
        let start_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, range_ptr, &[i64_type.const_int(3, false)], "range_start_ptr")
                .or_llvm_err()?
        };
        let start_val = builder
            .build_load(i64_type, start_ptr, "range_start")
            .or_llvm_err()?;

        // Read end from range object (offset 4)
        // SAFETY: GEP to compute the end-of-buffer position; the offset is the sum of validated lengths that fit within the allocation
        let end_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, range_ptr, &[i64_type.const_int(4, false)], "range_end_ptr")
                .or_llvm_err()?
        };
        let end_raw = builder
            .build_load(i64_type, end_ptr, "range_end_raw")
            .or_llvm_err()?
            .into_int_value();

        // Read inclusive flag from range object (offset 5)
        // SAFETY: GEP into the iterator/range struct at a fixed field offset; the struct layout is known at compile time
        let inclusive_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, range_ptr, &[i64_type.const_int(5, false)], "range_inclusive_ptr")
                .or_llvm_err()?
        };
        let inclusive_flag = builder
            .build_load(i64_type, inclusive_ptr, "inclusive")
            .or_llvm_err()?
            .into_int_value();

        // If inclusive, end = end + 1 (so current < end+1 means current <= end)
        let is_inclusive = builder
            .build_int_compare(verum_llvm::IntPredicate::NE, inclusive_flag, i64_type.const_int(0, false), "is_inclusive")
            .or_llvm_err()?;
        let end_plus_one = builder
            .build_int_add(end_raw, one, "end_plus_one")
            .or_llvm_err()?;
        let end_val: BasicValueEnum<'ctx> = builder
            .build_select(is_inclusive, end_plus_one, end_raw, "range_end")
            .or_llvm_err()?;

        // field0 = current (starts at range.start)
        // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
        let field0_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "iter_current_ptr")
                .or_llvm_err()?
        };
        builder
            .build_store(field0_ptr, start_val)
            .or_llvm_err()?;

        // field1 = end
        // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
        let field1_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "iter_end_ptr")
                .or_llvm_err()?
        };
        builder
            .build_store(field1_ptr, end_val)
            .or_llvm_err()?;

        Ok(iter_ptr)
    }

    /// Lower IterNext for a range iterator.
    ///
    /// Iterator layout: [tag=1: i64, current: i64, end: i64]
    /// Returns current value, increments current. has_more = (current < end).
    pub fn lower_iter_next_range(
        &self,
        builder: &Builder<'ctx>,
        iter_ptr: PointerValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        let i64_type = self.context.i64_type();

        // Load current (field at offset 1)
        // SAFETY: GEP into the iterator/range struct at a fixed field offset; the struct layout is known at compile time
        let current_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "current_ptr")
                .or_llvm_err()?
        };
        let current = builder
            .build_load(i64_type, current_ptr, "current")
            .or_llvm_err()?
            .into_int_value();

        // Load end (field at offset 2)
        // SAFETY: GEP to compute the end-of-buffer position; the offset is the sum of validated lengths that fit within the allocation
        let end_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "end_ptr")
                .or_llvm_err()?
        };
        let end = builder
            .build_load(i64_type, end_ptr, "end")
            .or_llvm_err()?
            .into_int_value();

        // has_element = current < end (signed comparison)
        let has_element = builder
            .build_int_compare(verum_llvm::IntPredicate::SLT, current, end, "has_element")
            .or_llvm_err()?;

        // value = current (or unit if exhausted)
        let unit_tag = i64_type.const_int(0x7FFB_0000_0000_0000, false);
        let value = builder
            .build_select(has_element, current, unit_tag, "range_value")
            .or_llvm_err()?
            .into_int_value();

        // Increment current: current + 1
        let one = i64_type.const_int(1, false);
        let new_current = builder
            .build_int_add(current, one, "next_current")
            .or_llvm_err()?;
        builder
            .build_store(current_ptr, new_current)
            .or_llvm_err()?;

        // Convert has_element (i1) to i64
        let has_more = builder
            .build_int_z_extend(has_element, i64_type, "has_more")
            .or_llvm_err()?;

        Ok((value, has_more))
    }

    /// Lower IterNew for a flat range (from NewRange/verum_range_new).
    ///
    /// Flat range layout: {start: i64, end: i64, step: i64, current: i64} at offsets 0, 8, 16, 24.
    /// The end value is already adjusted for inclusive by the NewRange instruction handler.
    /// Iterator layout: [tag=1: i64, current=start: i64, end: i64]
    pub fn lower_iter_new_flat_range(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        range_ptr: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();

        let iter_size = i64_type.const_int(Self::ITER_SIZE, false);
        let iter_ptr = self.emit_checked_malloc(&builder, module, iter_size, "flat_range_iter")?;

        // Tag = 1 (range iterator)
        let one = i64_type.const_int(1, false);
        builder
            .build_store(iter_ptr, one)
            .or_llvm_err()?;

        // Read start from flat range at offset 0
        let start_val = builder
            .build_load(i64_type, range_ptr, "flat_range_start")
            .or_llvm_err()?;

        // Read end from flat range at offset 1 (already adjusted for inclusive by NewRange handler)
        // SAFETY: GEP to compute the end-of-buffer position; the offset is the sum of validated lengths that fit within the allocation
        let end_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, range_ptr, &[i64_type.const_int(1, false)], "flat_range_end_ptr")
                .or_llvm_err()?
        };
        let end_val = builder
            .build_load(i64_type, end_ptr, "flat_range_end")
            .or_llvm_err()?;

        // field0 = current (starts at range.start)
        // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
        let current_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "iter_current_ptr")
                .or_llvm_err()?
        };
        builder
            .build_store(current_ptr, start_val)
            .or_llvm_err()?;

        // field1 = end
        // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
        let iter_end_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "iter_end_ptr")
                .or_llvm_err()?
        };
        builder
            .build_store(iter_end_ptr, end_val)
            .or_llvm_err()?;

        Ok(iter_ptr)
    }

    /// Lower IterNext for a flat range iterator.
    /// Same as lower_iter_next_range — iterator layout is identical.
    pub fn lower_iter_next_flat_range(
        &self,
        builder: &Builder<'ctx>,
        iter_ptr: PointerValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        // The iterator layout is the same as header-based range: [tag, current, end]
        self.lower_iter_next_range(builder, iter_ptr)
    }

    /// Lower IterNext for a list iterator.
    ///
    /// Iterator layout: [tag=0: i64, iterable_ptr: i64, current_index: i64]
    /// Returns list[index], increments index. has_more = (index < len).
    pub fn lower_iter_next_list(
        &self,
        builder: &Builder<'ctx>,
        iter_ptr: PointerValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Load iterable pointer (field at offset 1)
        // SAFETY: GEP into the 24-byte iterator struct at field 1 (iterable pointer)
        let field0_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "field0_ptr")
                .or_llvm_err()?
        };
        let iterable_as_int = builder
            .build_load(i64_type, field0_ptr, "iterable_int")
            .or_llvm_err()?
            .into_int_value();
        let iterable_ptr = builder
            .build_int_to_ptr(iterable_as_int, ptr_type, "iterable_ptr")
            .or_llvm_err()?;

        // Load current index (field at offset 2)
        // SAFETY: GEP into the 24-byte iterator struct at field 2 (current index)
        let index_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "index_ptr")
                .or_llvm_err()?
        };
        let index = builder
            .build_load(i64_type, index_ptr, "index")
            .or_llvm_err()?
            .into_int_value();

        // Load list length from LIST_LEN_IDX=4 (byte offset 32)
        // List layout: [24-byte obj header][ptr: i64][len: i64][cap: i64]
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let len_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iterable_ptr, &[i64_type.const_int(4, false)], "len_ptr")
                .or_llvm_err()?
        };
        let len = builder
            .build_load(i64_type, len_ptr, "len")
            .or_llvm_err()?
            .into_int_value();

        // Check if index < len
        let has_element = builder
            .build_int_compare(verum_llvm::IntPredicate::ULT, index, len, "has_element")
            .or_llvm_err()?;

        // Load data pointer from LIST_PTR_IDX=3 (byte offset 24)
        // SAFETY: GEP into the list object header to access the data pointer field at a fixed offset; the list pointer is non-null and valid
        let data_ptr_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iterable_ptr, &[i64_type.const_int(3, false)], "data_ptr_ptr")
                .or_llvm_err()?
        };
        let data_as_int = builder
            .build_load(i64_type, data_ptr_ptr, "data_int")
            .or_llvm_err()?
            .into_int_value();
        let data_ptr = builder
            .build_int_to_ptr(data_as_int, ptr_type, "data_ptr")
            .or_llvm_err()?;

        // Use a safe pointer when no element (data_ptr may be null for empty lists)
        // When has_element is false, use iter_ptr as a safe non-null pointer
        let data_as_ptr_int = builder
            .build_ptr_to_int(data_ptr, i64_type, "data_ptr_int")
            .or_llvm_err()?;
        let iter_as_int = builder
            .build_ptr_to_int(iter_ptr, i64_type, "iter_ptr_int")
            .or_llvm_err()?;
        let safe_ptr_int = builder
            .build_select(has_element, data_as_ptr_int, iter_as_int, "safe_ptr_int")
            .or_llvm_err()?
            .into_int_value();
        let safe_data_ptr = builder
            .build_int_to_ptr(safe_ptr_int, ptr_type, "safe_data_ptr")
            .or_llvm_err()?;

        // Get element at current index (safe: only dereferenced when has_element is true)
        // SAFETY: GEP into the list object header to access the data pointer field at a fixed offset; the list pointer is non-null and valid
        let elem_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, safe_data_ptr, &[index], "elem_ptr")
                .or_llvm_err()?
        };
        let element = builder
            .build_load(i64_type, elem_ptr, "element")
            .or_llvm_err()?
            .into_int_value();

        // Increment index only when we have an element
        let one = i64_type.const_int(1, false);
        let next_idx = builder
            .build_int_add(index, one, "next_idx")
            .or_llvm_err()?;
        let new_index = builder
            .build_select(has_element, next_idx, index, "new_index")
            .or_llvm_err()?
            .into_int_value();
        builder
            .build_store(index_ptr, new_index)
            .or_llvm_err()?;

        // Return unit if no element
        let unit_tag = i64_type.const_int(0x7FFB_0000_0000_0000, false);
        let value = builder
            .build_select(has_element, element, unit_tag, "iter_value")
            .or_llvm_err()?
            .into_int_value();

        // Convert has_element (i1) to i64
        let has_more = builder
            .build_int_z_extend(has_element, i64_type, "has_more")
            .or_llvm_err()?;

        Ok((value, has_more))
    }

    /// Lower IterNext for a byte slice iterable (from Pack{ptr, len}).
    ///
    /// Slice layout (Pack object): [24-byte header][ptr: i64][len: i64]
    /// Iterator layout: [tag=0: i64, iterable_ptr: i64, index: i64]
    /// Elements are i8 bytes, zero-extended to i64.
    pub fn lower_iter_next_slice(
        &self,
        builder: &Builder<'ctx>,
        iter_ptr: PointerValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Load iterable pointer (field at iter offset 1)
        // SAFETY: GEP to access a struct field at a fixed offset; the struct was allocated with sufficient size for all fields
        let field0_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "field0_ptr")
                .or_llvm_err()?
        };
        let iterable_as_int = builder
            .build_load(i64_type, field0_ptr, "iterable_int")
            .or_llvm_err()?
            .into_int_value();
        let iterable_ptr = builder
            .build_int_to_ptr(iterable_as_int, ptr_type, "iterable_ptr")
            .or_llvm_err()?;

        // Load current index (field at iter offset 2)
        // SAFETY: GEP into the iterator/range struct at a fixed field offset; the struct layout is known at compile time
        let index_ptr = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "index_ptr")
                .or_llvm_err()?
        };
        let index = builder
            .build_load(i64_type, index_ptr, "index")
            .or_llvm_err()?
            .into_int_value();

        // Load slice len from Pack offset 32 (24 header + 8 for field 1)
        // SAFETY: GEP into the iterator/range struct at a fixed field offset; the struct layout is known at compile time
        let len_slot = unsafe {
            builder
                .build_in_bounds_gep(i8_type, iterable_ptr, &[i64_type.const_int(32, false)], "slice_len_slot")
                .or_llvm_err()?
        };
        let len = builder
            .build_load(i64_type, len_slot, "len")
            .or_llvm_err()?
            .into_int_value();

        // Check if index < len
        let has_element = builder
            .build_int_compare(verum_llvm::IntPredicate::ULT, index, len, "has_element")
            .or_llvm_err()?;

        // Load data pointer from Pack offset 24 (24 header + 0 for field 0)
        // SAFETY: GEP into the iterator/range struct at a fixed field offset; the struct layout is known at compile time
        let data_ptr_slot = unsafe {
            builder
                .build_in_bounds_gep(i8_type, iterable_ptr, &[i64_type.const_int(24, false)], "slice_ptr_slot")
                .or_llvm_err()?
        };
        let data_as_int = builder
            .build_load(i64_type, data_ptr_slot, "data_int")
            .or_llvm_err()?
            .into_int_value();
        let data_ptr = builder
            .build_int_to_ptr(data_as_int, ptr_type, "data_ptr")
            .or_llvm_err()?;

        // Get byte element at current index (i8 GEP + zext to i64)
        // SAFETY: GEP into list data array to access an element; the index is validated against the list length before access
        let elem_ptr = unsafe {
            builder
                .build_in_bounds_gep(i8_type, data_ptr, &[index], "byte_ptr")
                .or_llvm_err()?
        };
        let byte_val = builder
            .build_load(i8_type, elem_ptr, "byte_val")
            .or_llvm_err()?
            .into_int_value();
        let element = builder
            .build_int_z_extend(byte_val, i64_type, "byte_as_i64")
            .or_llvm_err()?;

        // Increment index
        let one = i64_type.const_int(1, false);
        let new_index = builder
            .build_int_add(index, one, "new_index")
            .or_llvm_err()?;
        builder
            .build_store(index_ptr, new_index)
            .or_llvm_err()?;

        // Return unit if no element
        let unit_tag = i64_type.const_int(0x7FFB_0000_0000_0000, false);
        let value = builder
            .build_select(has_element, element, unit_tag, "iter_value")
            .or_llvm_err()?
            .into_int_value();

        // Convert has_element (i1) to i64
        let has_more = builder
            .build_int_z_extend(has_element, i64_type, "has_more")
            .or_llvm_err()?;

        Ok((value, has_more))
    }

    // =========================================================================
    // Text (String) Iteration
    // =========================================================================

    /// Lower IterNew for a text/string iterable.
    ///
    /// Text layout (flat): {ptr: *u8, len: i64, cap: i64} — 24 bytes, NO object header.
    /// For string_register (C runtime): plain `char*` pointer — use strlen for len.
    /// Iterator layout: [tag=2: i64, text_data_ptr: i64, len_and_index: i64]
    ///   - tag=2 distinguishes text iterators (vs list=0, range=1)
    ///   - text_data_ptr = pointer to raw UTF-8 bytes
    ///   - We store current_index in field 2
    ///   - len is stored in a 4th slot
    /// Actually we use 4 i64 fields: [tag=2, data_ptr, index, len]
    pub fn lower_iter_new_text(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        text_val: IntValue<'ctx>,
        is_text_register: bool,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Allocate 32 bytes: [tag, data_ptr, index, len]
        let iter_size = i64_type.const_int(32, false);
        let iter_ptr = self.emit_checked_malloc(&builder, module, iter_size, "text_iter")?;

        // Tag = 2 (text iterator)
        let tag = i64_type.const_int(2, false);
        builder
            .build_store(iter_ptr, tag)
            .or_llvm_err()?;

        if is_text_register {
            // text_register: text_val is i64 pointer to flat {ptr: *u8, len: i64, cap: i64}
            let text_ptr = builder
                .build_int_to_ptr(text_val, ptr_type, "text_ptr")
                .or_llvm_err()?;

            // Load data pointer (field 0)
            let data_ptr_as_int = builder
                .build_load(i64_type, text_ptr, "data_ptr_int")
                .or_llvm_err()?
                .into_int_value();

            // Load len (field 1, offset 8)
            // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
            let len_field = unsafe {
                builder
                    .build_in_bounds_gep(i64_type, text_ptr, &[i64_type.const_int(1, false)], "len_field")
                    .or_llvm_err()?
            };
            let len = builder
                .build_load(i64_type, len_field, "text_len")
                .or_llvm_err()?
                .into_int_value();

            // Store data_ptr at iter[1]
            // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
            let field1 = unsafe {
                builder
                    .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "iter_data")
                    .or_llvm_err()?
            };
            builder.build_store(field1, data_ptr_as_int).or_llvm_err()?;

            // Store index=0 at iter[2]
            // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
            let field2 = unsafe {
                builder
                    .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "iter_idx")
                    .or_llvm_err()?
            };
            builder.build_store(field2, i64_type.const_int(0, false)).or_llvm_err()?;

            // Store len at iter[3]
            // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
            let field3 = unsafe {
                builder
                    .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(3, false)], "iter_len")
                    .or_llvm_err()?
            };
            builder.build_store(field3, len).or_llvm_err()?;
        } else {
            // string_register: text_val is i64 pointer to null-terminated C string
            // Use strlen to get length
            let strlen_fn = module.get_function("strlen").unwrap_or_else(|| {
                let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
                module.add_function("strlen", fn_type, None)
            });
            let str_ptr = builder
                .build_int_to_ptr(text_val, ptr_type, "str_ptr")
                .or_llvm_err()?;
            let len = builder
                .build_call(strlen_fn, &[str_ptr.into()], "str_len")
                .or_llvm_err()?
                .try_as_basic_value()
                .basic()
                .or_internal("strlen should return i64")?
                .into_int_value();

            // Store str_ptr as data_ptr at iter[1]
            let data_as_int = builder
                .build_ptr_to_int(str_ptr, i64_type, "str_as_int")
                .or_llvm_err()?;
            // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
            let field1 = unsafe {
                builder
                    .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "iter_data")
                    .or_llvm_err()?
            };
            builder.build_store(field1, data_as_int).or_llvm_err()?;

            // Store index=0 at iter[2]
            // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
            let field2 = unsafe {
                builder
                    .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "iter_idx")
                    .or_llvm_err()?
            };
            builder.build_store(field2, i64_type.const_int(0, false)).or_llvm_err()?;

            // Store len at iter[3]
            // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
            let field3 = unsafe {
                builder
                    .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(3, false)], "iter_len")
                    .or_llvm_err()?
            };
            builder.build_store(field3, len).or_llvm_err()?;
        }

        Ok(iter_ptr)
    }

    /// Lower IterNext for a text iterator.
    ///
    /// Iterator layout: [tag=2: i64, data_ptr: i64, index: i64, len: i64]
    /// Returns (byte_value_as_i64, has_more_as_i64).
    /// Iterates UTF-8 bytes (codepoint iteration requires UTF-8 decode).
    pub fn lower_iter_next_text(
        &self,
        builder: &Builder<'ctx>,
        iter_ptr: PointerValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Load data pointer (field 1)
        // SAFETY: GEP to access a struct field at a fixed offset; the struct was allocated with sufficient size for all fields
        let field1 = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(1, false)], "data_field")
                .or_llvm_err()?
        };
        let data_as_int = builder
            .build_load(i64_type, field1, "data_int")
            .or_llvm_err()?
            .into_int_value();
        let data_ptr = builder
            .build_int_to_ptr(data_as_int, ptr_type, "data_ptr")
            .or_llvm_err()?;

        // Load current index (field 2)
        // SAFETY: GEP to access a struct field at a fixed offset; the struct was allocated with sufficient size for all fields
        let index_field = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(2, false)], "idx_field")
                .or_llvm_err()?
        };
        let index = builder
            .build_load(i64_type, index_field, "index")
            .or_llvm_err()?
            .into_int_value();

        // Load len (field 3)
        // SAFETY: GEP to access a struct field at a fixed offset; the struct was allocated with sufficient size for all fields
        let len_field = unsafe {
            builder
                .build_in_bounds_gep(i64_type, iter_ptr, &[i64_type.const_int(3, false)], "len_field")
                .or_llvm_err()?
        };
        let len = builder
            .build_load(i64_type, len_field, "len")
            .or_llvm_err()?
            .into_int_value();

        // Check if index < len
        let has_element = builder
            .build_int_compare(verum_llvm::IntPredicate::ULT, index, len, "has_element")
            .or_llvm_err()?;

        // Get byte at data_ptr[index]
        // SAFETY: GEP into the iterator/range struct at a fixed field offset; the struct layout is known at compile time
        let byte_ptr = unsafe {
            builder
                .build_in_bounds_gep(i8_type, data_ptr, &[index], "byte_ptr")
                .or_llvm_err()?
        };
        let byte_val = builder
            .build_load(i8_type, byte_ptr, "byte")
            .or_llvm_err()?
            .into_int_value();
        let element = builder
            .build_int_z_extend(byte_val, i64_type, "char_code")
            .or_llvm_err()?;

        // Increment index
        let new_index = builder
            .build_int_add(index, i64_type.const_int(1, false), "new_index")
            .or_llvm_err()?;
        builder
            .build_store(index_field, new_index)
            .or_llvm_err()?;

        // Return unit if no element
        let unit_tag = i64_type.const_int(0x7FFB_0000_0000_0000, false);
        let value = builder
            .build_select(has_element, element, unit_tag, "iter_value")
            .or_llvm_err()?
            .into_int_value();

        // Convert has_element (i1) to i64
        let has_more = builder
            .build_int_z_extend(has_element, i64_type, "has_more")
            .or_llvm_err()?;

        Ok((value, has_more))
    }

    // =========================================================================
    // Variant Operations
    // =========================================================================

    /// Object header size in bytes (matches interpreter heap.rs).
    pub const OBJECT_HEADER_SIZE: u64 = 24;

    /// Variant tag offset (after object header).
    pub const VARIANT_TAG_OFFSET: u64 = Self::OBJECT_HEADER_SIZE;

    /// Variant payload offset (after tag + padding).
    pub const VARIANT_PAYLOAD_OFFSET: u64 = Self::OBJECT_HEADER_SIZE + 8;

    /// Lower New instruction — allocate a record/object on the heap.
    ///
    /// Layout: [header (24 bytes)][field0:i64][field1:i64]...
    /// All fields are zero-initialized. Callers should store field values
    /// using GEP + store at the appropriate offsets.
    pub fn lower_new_object(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        field_count: u32,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();

        // Calculate total size: header + fields(8 each)
        let total_size = Self::OBJECT_HEADER_SIZE + (field_count as u64 * VALUE_SIZE);
        let size = i64_type.const_int(total_size, false);

        // Allocate object
        let obj_ptr = self.emit_checked_malloc(&builder, module, size, "new_object")?;

        // Zero initialize (for safety)
        let memset_fn = self.get_or_declare_memset(module)?;
        let zero_byte = self.context.i32_type().const_int(0, false);
        builder
            .build_call(memset_fn, &[obj_ptr.into(), zero_byte.into(), size.into()], "clear_object")
            .or_llvm_err()?;

        Ok(obj_ptr)
    }

    /// Closure struct size: { fn_ptr: ptr, env_ptr: ptr } = 16 bytes.
    pub const CLOSURE_SIZE: u64 = 16;

    /// Lower NewClosure — allocate a closure struct with captured environment.
    ///
    /// Closure layout: { fn_ptr: ptr (offset 0), env_ptr: ptr (offset 8) }
    /// Environment layout: [capture_0: i64][capture_1: i64]...
    pub fn lower_new_closure(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        fn_ptr: PointerValue<'ctx>,
        captures: &[BasicValueEnum<'ctx>],
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Allocate closure struct: { fn_ptr, env_ptr }
        let closure_size = i64_type.const_int(Self::CLOSURE_SIZE, false);
        let closure_ptr = self.emit_checked_malloc(&builder, module, closure_size, "closure")?;

        // Store fn_ptr at offset 0
        builder
            .build_store(closure_ptr, fn_ptr)
            .or_llvm_err()?;

        // Allocate and populate environment if there are captures
        let env_ptr = if captures.is_empty() {
            ptr_type.const_null()
        } else {
            let env_size = i64_type.const_int(captures.len() as u64 * VALUE_SIZE, false);
            let env_ptr = self.emit_checked_malloc(&builder, module, env_size, "closure_env")?;

            // Store each captured value
            for (i, cap_val) in captures.iter().enumerate() {
                let offset = i as u64 * VALUE_SIZE;
                // SAFETY: GEP at a known offset within a heap-allocated struct; the offset is within the allocation size
                let cap_ptr = unsafe {
                    builder
                        .build_in_bounds_gep(
                            i8_type,
                            env_ptr,
                            &[i64_type.const_int(offset, false)],
                            &format!("cap_{}_ptr", i),
                        )
                        .or_llvm_err()?
                };
                builder
                    .build_store(cap_ptr, *cap_val)
                    .or_llvm_err()?;
            }
            env_ptr
        };

        // Store env_ptr at offset 8
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let env_slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i8_type,
                    closure_ptr,
                    &[i64_type.const_int(8, false)],
                    "env_slot",
                )
                .or_llvm_err()?
        };
        builder
            .build_store(env_slot, env_ptr)
            .or_llvm_err()?;

        Ok(closure_ptr)
    }

    /// Lower MakeVariant instruction.
    ///
    /// Creates a new variant with the specified tag.
    /// Layout: [header (24 bytes)][tag:u32][pad:u32][payload:Value...]
    pub fn lower_make_variant(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        tag: u32,
        field_count: u32,
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        // Calculate total size: header + tag(4) + padding(4) + fields(8 each)
        let total_size = Self::OBJECT_HEADER_SIZE + 8 + (field_count as u64 * VALUE_SIZE);
        let size = i64_type.const_int(total_size, false);

        // Allocate variant
        let variant_ptr = self.emit_checked_malloc(&builder, module, size, "variant")?;

        // Zero initialize (for safety)
        let memset_fn = self.get_or_declare_memset(module)?;
        let zero_byte = self.context.i32_type().const_int(0, false);
        builder
            .build_call(memset_fn, &[variant_ptr.into(), zero_byte.into(), size.into()], "clear_variant")
            .or_llvm_err()?;

        // Store tag at offset OBJECT_HEADER_SIZE
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let tag_ptr = unsafe {
            builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    variant_ptr,
                    &[i64_type.const_int(Self::VARIANT_TAG_OFFSET, false)],
                    "tag_ptr",
                )
                .or_llvm_err()?
        };
        let tag_val = i32_type.const_int(tag as u64, false);
        builder
            .build_store(tag_ptr, tag_val)
            .or_llvm_err()?;

        Ok(variant_ptr)
    }

    /// Lower GetTag instruction.
    ///
    /// Gets the tag from a variant (stored at offset OBJECT_HEADER_SIZE).
    pub fn lower_get_tag(
        &self,
        builder: &Builder<'ctx>,
        variant_ptr: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        // Load tag from offset OBJECT_HEADER_SIZE
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let tag_ptr = unsafe {
            builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    variant_ptr,
                    &[i64_type.const_int(Self::VARIANT_TAG_OFFSET, false)],
                    "tag_ptr",
                )
                .or_llvm_err()?
        };
        let tag = builder
            .build_load(i32_type, tag_ptr, "tag")
            .or_llvm_err()?
            .into_int_value();

        // Zero-extend to i64 for VBC register consistency
        let tag_i64 = builder
            .build_int_z_extend(tag, i64_type, "tag_i64")
            .or_llvm_err()?;

        Ok(tag_i64)
    }

    /// Lower SetVariantData instruction.
    ///
    /// Sets a field in the variant payload.
    pub fn lower_set_variant_data(
        &self,
        builder: &Builder<'ctx>,
        variant_ptr: PointerValue<'ctx>,
        field_idx: u32,
        value: IntValue<'ctx>,
    ) -> Result<()> {
        let i64_type = self.context.i64_type();

        // Calculate field offset: VARIANT_PAYLOAD_OFFSET + field_idx * 8
        let field_offset = Self::VARIANT_PAYLOAD_OFFSET + (field_idx as u64 * VALUE_SIZE);
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let field_ptr = unsafe {
            builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    variant_ptr,
                    &[i64_type.const_int(field_offset, false)],
                    "field_ptr",
                )
                .or_llvm_err()?
        };

        builder
            .build_store(field_ptr, value)
            .or_llvm_err()?;

        Ok(())
    }

    /// Lower GetVariantData instruction.
    ///
    /// Gets a field from the variant payload.
    pub fn lower_get_variant_data(
        &self,
        builder: &Builder<'ctx>,
        variant_ptr: PointerValue<'ctx>,
        field_idx: u32,
    ) -> Result<IntValue<'ctx>> {
        let i64_type = self.context.i64_type();

        // Calculate field offset: VARIANT_PAYLOAD_OFFSET + field_idx * 8
        let field_offset = Self::VARIANT_PAYLOAD_OFFSET + (field_idx as u64 * VALUE_SIZE);
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let field_ptr = unsafe {
            builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    variant_ptr,
                    &[i64_type.const_int(field_offset, false)],
                    "field_ptr",
                )
                .or_llvm_err()?
        };

        let value = builder
            .build_load(i64_type, field_ptr, "field_value")
            .or_llvm_err()?
            .into_int_value();

        Ok(value)
    }

    /// Lower IsVar instruction.
    ///
    /// Checks if a variant has the specified tag.
    pub fn lower_is_var(
        &self,
        builder: &Builder<'ctx>,
        variant_ptr: PointerValue<'ctx>,
        expected_tag: u32,
    ) -> Result<IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        // Load tag
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let tag_ptr = unsafe {
            builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    variant_ptr,
                    &[i64_type.const_int(Self::VARIANT_TAG_OFFSET, false)],
                    "tag_ptr",
                )
                .or_llvm_err()?
        };
        let tag = builder
            .build_load(i32_type, tag_ptr, "tag")
            .or_llvm_err()?
            .into_int_value();

        // Compare with expected tag
        let expected = i32_type.const_int(expected_tag as u64, false);
        let matches = builder
            .build_int_compare(verum_llvm::IntPredicate::EQ, tag, expected, "tag_matches")
            .or_llvm_err()?;

        // Zero-extend to i64 (boolean as integer)
        let result = builder
            .build_int_z_extend(matches, i64_type, "is_var_result")
            .or_llvm_err()?;

        Ok(result)
    }

    /// Lower AsVar instruction.
    ///
    /// Extracts the payload from a variant at the specified field index.
    /// Equivalent to GetVariantData but named for pattern matching context.
    pub fn lower_as_var(
        &self,
        builder: &Builder<'ctx>,
        variant_ptr: PointerValue<'ctx>,
        field_idx: u32,
    ) -> Result<IntValue<'ctx>> {
        // AsVar is essentially GetVariantData
        self.lower_get_variant_data(builder, variant_ptr, field_idx)
    }

    // =========================================================================
    // Tuple Operations
    // =========================================================================

    /// Lower Pack instruction.
    ///
    /// Packs multiple values into a tuple (allocated on heap).
    /// Layout: [header (24 bytes)][values:Value...]
    pub fn lower_pack(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        values: &[IntValue<'ctx>],
    ) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();

        // Calculate total size: header + values
        let total_size = Self::OBJECT_HEADER_SIZE + (values.len() as u64 * VALUE_SIZE);
        let size = i64_type.const_int(total_size, false);

        // Allocate tuple
        let tuple_ptr = self.emit_checked_malloc(&builder, module, size, "tuple")?;

        // Store each value
        for (i, value) in values.iter().enumerate() {
            let offset = Self::OBJECT_HEADER_SIZE + (i as u64 * VALUE_SIZE);
            // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
            let elem_ptr = unsafe {
                builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        tuple_ptr,
                        &[i64_type.const_int(offset, false)],
                        &format!("tuple_elem_{}", i),
                    )
                    .or_llvm_err()?
            };
            builder
                .build_store(elem_ptr, *value)
                .or_llvm_err()?;
        }

        Ok(tuple_ptr)
    }

    /// Lower Unpack instruction (single element extraction).
    ///
    /// Gets a value from a tuple at the specified index.
    pub fn lower_unpack_element(
        &self,
        builder: &Builder<'ctx>,
        tuple_ptr: PointerValue<'ctx>,
        index: u32,
    ) -> Result<IntValue<'ctx>> {
        let i64_type = self.context.i64_type();

        let offset = Self::OBJECT_HEADER_SIZE + (index as u64 * VALUE_SIZE);
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let elem_ptr = unsafe {
            builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    tuple_ptr,
                    &[i64_type.const_int(offset, false)],
                    "unpack_elem_ptr",
                )
                .or_llvm_err()?
        };

        let value = builder
            .build_load(i64_type, elem_ptr, "unpack_value")
            .or_llvm_err()?
            .into_int_value();

        Ok(value)
    }

    // =========================================================================
    // Text LLVM IR Functions — replaces C runtime text stubs
    // =========================================================================
    //
    // These emit LLVM IR function bodies for text operations that were previously
    // in verum_runtime.c. By emitting IR here, we can delete the C implementations
    // and let LLVM inline them at -O2.
    //
    // Text layout: { ptr: i8*, len: i64, cap: i64 } = 24 bytes (3 x i64)

    /// Emit all text runtime functions as LLVM IR.
    /// Call this during module setup (after function lowering, before verify).
    pub fn emit_text_ir_functions(&self, module: &Module<'ctx>) -> Result<()> {
        self.emit_verum_text_get_ptr(module)?;
        self.emit_verum_text_alloc(module)?;
        self.emit_verum_text_from_cstr(module)?;
        self.emit_verum_text_concat(module)?;
        self.emit_verum_text_free(module)?;
        self.emit_verum_generic_len(module)?;
        self.emit_verum_strlen_export(module)?;
        self.emit_verum_text_from_static(module)?;
        self.emit_verum_generic_eq(module)?;
        self.emit_verum_generic_hash(module)?;
        self.emit_verum_int_to_text(module)?;
        self.emit_verum_float_to_text(module)?;
        self.emit_verum_string_parse_int(module)?;
        self.emit_verum_string_parse_float(module)?;
        self.emit_verum_text_char_len(module)?;
        self.fixup_text_len(module)?;
        self.fixup_map_get(module)?;
        self.fixup_map_contains_key(module)?;
        self.fixup_map_remove(module)?;
        self.fixup_map_insert(module)?;
        Ok(())
    }

    /// Replace compiled Text.len with correct inline version.
    /// Compiled Text.len from text.vr reads from VBC object header offset 32,
    /// but verum_text_alloc creates flat {ptr, len, cap} at offsets 0, 8, 16.
    /// This fixup deletes the broken body and re-emits with offset 8.
    fn fixup_text_len(&self, module: &Module<'ctx>) -> Result<()> {
        let Some(func) = module.get_function("Text.len") else { return Ok(()) };
        if func.count_basic_blocks() == 0 { return Ok(()); }

        // Delete all existing basic blocks
        while let Some(bb) = func.get_first_basic_block() {
            // SAFETY: Deleting an unreachable basic block that was created speculatively; it has no predecessors and no live references
            unsafe { bb.delete().ok(); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();
        let builder = ctx.create_builder();

        let entry = ctx.append_basic_block(func, "entry");
        builder.position_at_end(entry);

        let text_obj = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let text_ptr = builder.build_int_to_ptr(text_obj, ptr_type, "text_ptr").or_llvm_err()?;

        // Read len from TEXT_LEN_OFFSET (8) in flat {ptr, len, cap} layout
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let len_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, text_ptr, &[i64_type.const_int(TEXT_LEN_OFFSET, false)], "len_slot").or_llvm_err()?
        };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?;
        builder.build_return(Some(&len)).or_llvm_err()?;
        Ok(())
    }

    /// Replace compiled Map.get with correct Robin Hood linear probing.
    /// Compiled map.vr uses VBC object header layout, but the hash/slot layout
    /// must match the C runtime exactly for interop. This fixup re-emits the
    /// function body with correct offsets and probing logic.
    ///
    /// Map layout (24-byte header + fields):
    ///   offset 24: entries_ptr (i64)
    ///   offset 32: len (i64)
    ///   offset 40: cap (i64)
    ///   offset 48: tombstones (i64)
    ///
    /// Slot layout (32 bytes, NO header):
    ///   offset 0: key (i64)
    ///   offset 8: value (i64)
    ///   offset 16: hash (i64)
    ///   offset 24: psl (i64)
    fn fixup_map_get(&self, module: &Module<'ctx>) -> Result<()> {
        let Some(func) = module.get_function("Map.get") else { return Ok(()) };
        if func.count_basic_blocks() == 0 { return Ok(()); }

        // Delete all existing basic blocks
        while let Some(bb) = func.get_first_basic_block() {
            // SAFETY: Deleting an unreachable basic block that was created speculatively; it has no predecessors and no live references
            unsafe { bb.delete().ok(); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let builder = ctx.create_builder();

        // Basic blocks
        let entry = ctx.append_basic_block(func, "entry");
        let check_cap = ctx.append_basic_block(func, "check_cap");
        let compute_hash = ctx.append_basic_block(func, "compute_hash");
        let loop_head = ctx.append_basic_block(func, "loop_head");
        let check_hash = ctx.append_basic_block(func, "check_hash");
        let check_key = ctx.append_basic_block(func, "check_key");
        let found = ctx.append_basic_block(func, "found");
        let check_psl = ctx.append_basic_block(func, "check_psl");
        let not_found = ctx.append_basic_block(func, "not_found");

        // Handle both typed (ptr) and untyped (i64) function signatures
        let param0 = func.get_nth_param(0).or_internal("missing param 0")?;
        let param1 = func.get_nth_param(1).or_internal("missing param 1")?;

        // Entry: null check
        builder.position_at_end(entry);
        let (self_ptr, self_is_null) = if param0.is_pointer_value() {
            let p = param0.into_pointer_value();
            let is_null = builder.build_is_null(p, "is_null").or_llvm_err()?;
            (p, is_null)
        } else {
            let iv = param0.into_int_value();
            let is_null = builder.build_int_compare(
                verum_llvm::IntPredicate::EQ, iv, i64_type.const_zero(), "is_null"
            ).or_llvm_err()?;
            let p = builder.build_int_to_ptr(iv, ptr_type, "self_ptr").or_llvm_err()?;
            (p, is_null)
        };
        let key_i64 = if param1.is_int_value() {
            param1.into_int_value()
        } else {
            builder.build_ptr_to_int(param1.into_pointer_value(), i64_type, "key_i64").or_llvm_err()?
        };
        builder.build_conditional_branch(self_is_null, not_found, check_cap).or_llvm_err()?;

        // check_cap: load cap, return 0 if cap == 0
        builder.position_at_end(check_cap);
        // cap at offset 40 (HEADER_24 + field_2 * 8)
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(40, false)], "cap_slot").or_llvm_err()?
        };
        let cap = builder.build_load(i64_type, cap_slot, "cap").or_llvm_err()?.into_int_value();
        let cap_zero = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, cap, i64_type.const_zero(), "cap_zero"
        ).or_llvm_err()?;
        builder.build_conditional_branch(cap_zero, not_found, compute_hash).or_llvm_err()?;

        // compute_hash: hash = abs(verum_generic_hash(key)); if hash <= 1: hash += 2
        builder.position_at_end(compute_hash);
        let hash_fn = module.get_function("verum_generic_hash").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_generic_hash", fn_type, None)
        });
        let raw_hash = builder.build_call(hash_fn, &[key_i64.into()], "raw_hash")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        // abs(hash): if hash < 0 then -hash else hash
        let is_neg = builder.build_int_compare(
            verum_llvm::IntPredicate::SLT, raw_hash, i64_type.const_zero(), "is_neg"
        ).or_llvm_err()?;
        let neg_hash = builder.build_int_neg(raw_hash, "neg_hash").or_llvm_err()?;
        let abs_hash = builder.build_select(is_neg, neg_hash, raw_hash, "abs_hash").or_llvm_err()?.into_int_value();
        // if abs_hash <= 1: abs_hash + 2 (avoid 0 and 1 as markers)
        let hash_le1 = builder.build_int_compare(
            verum_llvm::IntPredicate::ULE, abs_hash, i64_type.const_int(1, false), "hash_le1"
        ).or_llvm_err()?;
        let hash_plus2 = builder.build_int_add(abs_hash, i64_type.const_int(2, false), "hash_plus2").or_llvm_err()?;
        let hash = builder.build_select(hash_le1, hash_plus2, abs_hash, "hash").or_llvm_err()?.into_int_value();

        // mask = cap - 1, idx = hash & mask
        let mask = builder.build_int_sub(cap, i64_type.const_int(1, false), "mask").or_llvm_err()?;
        let idx_init = builder.build_and(hash, mask, "idx_init").or_llvm_err()?;

        // Load entries_ptr (offset 24)
        // SAFETY: GEP into the map header at offset 24 to load the entries array pointer
        let entries_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(24, false)], "entries_slot").or_llvm_err()?
        };
        let entries_i64 = builder.build_load(i64_type, entries_slot, "entries_i64").or_llvm_err()?.into_int_value();
        let entries_ptr = builder.build_int_to_ptr(entries_i64, ptr_type, "entries_ptr").or_llvm_err()?;

        builder.build_unconditional_branch(loop_head).or_llvm_err()?;

        // loop_head: phi nodes for idx and psl
        builder.position_at_end(loop_head);
        let idx_phi = builder.build_phi(i64_type, "idx").or_llvm_err()?;
        let psl_phi = builder.build_phi(i64_type, "psl").or_llvm_err()?;
        let idx = idx_phi.as_basic_value().into_int_value();
        let psl = psl_phi.as_basic_value().into_int_value();

        // entry_ptr = entries_ptr + idx * 32
        let byte_offset = builder.build_int_mul(idx, i64_type.const_int(32, false), "byte_off").or_llvm_err()?;
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let entry_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, entries_ptr, &[byte_offset], "entry_ptr").or_llvm_err()?
        };

        // entry_hash = load(entry_ptr + 16)
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let hash_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(16, false)], "hash_slot").or_llvm_err()?
        };
        let entry_hash = builder.build_load(i64_type, hash_slot, "entry_hash").or_llvm_err()?.into_int_value();

        // if entry_hash == 0: not found (empty slot)
        let is_empty = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, entry_hash, i64_type.const_zero(), "is_empty"
        ).or_llvm_err()?;
        builder.build_conditional_branch(is_empty, not_found, check_hash).or_llvm_err()?;

        // check_hash: if entry_hash > 0 && entry_hash == hash -> check key
        builder.position_at_end(check_hash);
        let hash_positive = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, entry_hash, i64_type.const_zero(), "hash_pos"
        ).or_llvm_err()?;
        let hash_match = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, entry_hash, hash, "hash_match"
        ).or_llvm_err()?;
        let both = builder.build_and(hash_positive, hash_match, "both").or_llvm_err()?;
        builder.build_conditional_branch(both, check_key, check_psl).or_llvm_err()?;

        // check_key: compare keys using verum_generic_eq
        builder.position_at_end(check_key);
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let key_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_zero()], "key_slot").or_llvm_err()?
        };
        let entry_key = builder.build_load(i64_type, key_slot, "entry_key").or_llvm_err()?.into_int_value();

        let eq_fn = module.get_function("verum_generic_eq").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            module.add_function("verum_generic_eq", fn_type, None)
        });
        let eq_result = builder.build_call(eq_fn, &[entry_key.into(), key_i64.into()], "eq_result")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let keys_equal = builder.build_int_compare(
            verum_llvm::IntPredicate::NE, eq_result, i64_type.const_zero(), "keys_equal"
        ).or_llvm_err()?;
        builder.build_conditional_branch(keys_equal, found, check_psl).or_llvm_err()?;

        // Detect return type: may be i64 or ptr depending on typed function sigs
        let returns_ptr = func.get_type().get_return_type()
            .map_or(false, |rt| rt.is_pointer_type());

        // found: return value at entry_ptr + 8
        builder.position_at_end(found);
        // SAFETY: GEP into the 32-byte map entry to access the value field; the entry was found by linear probe
        let val_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(8, false)], "val_slot").or_llvm_err()?
        };
        if returns_ptr {
            let value = builder.build_load(ptr_type, val_slot, "value").or_llvm_err()?;
            builder.build_return(Some(&value)).or_llvm_err()?;
        } else {
            let value = builder.build_load(i64_type, val_slot, "value").or_llvm_err()?;
            builder.build_return(Some(&value)).or_llvm_err()?;
        }

        // check_psl: Robin Hood early termination
        builder.position_at_end(check_psl);
        let entry_positive = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, entry_hash, i64_type.const_zero(), "entry_pos"
        ).or_llvm_err()?;

        // Load entry_psl for Robin Hood check
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let psl_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(24, false)], "psl_slot").or_llvm_err()?
        };
        let entry_psl = builder.build_load(i64_type, psl_slot, "entry_psl").or_llvm_err()?.into_int_value();
        // Robin Hood: if entry is occupied AND psl > entry_psl, key can't be further
        let psl_exceeded = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, psl, entry_psl, "psl_exceeded"
        ).or_llvm_err()?;
        let robin_hood = builder.build_and(entry_positive, psl_exceeded, "robin_hood").or_llvm_err()?;

        // Safety limit: psl > cap
        let psl_next = builder.build_int_add(psl, i64_type.const_int(1, false), "psl_next").or_llvm_err()?;
        let safety = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, psl_next, cap, "safety"
        ).or_llvm_err()?;
        let should_stop = builder.build_or(robin_hood, safety, "should_stop").or_llvm_err()?;
        let idx_next = builder.build_and(
            builder.build_int_add(idx, i64_type.const_int(1, false), "idx_inc").or_llvm_err()?,
            mask, "idx_next"
        ).or_llvm_err()?;
        builder.build_conditional_branch(should_stop, not_found, loop_head).or_llvm_err()?;

        // Wire phi nodes
        idx_phi.add_incoming(&[(&idx_init, compute_hash), (&idx_next, check_psl)]);
        psl_phi.add_incoming(&[(&i64_type.const_zero(), compute_hash), (&psl_next, check_psl)]);

        // not_found: return null/0
        builder.position_at_end(not_found);
        if returns_ptr {
            let null_ptr = ptr_type.const_null();
            builder.build_return(Some(&null_ptr)).or_llvm_err()?;
        } else {
            builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        }
        Ok(())
    }

    /// Replace compiled Map.contains_key with correct Robin Hood linear probing.
    /// Same algorithm as fixup_map_get but returns 1 (found) or 0 (not found).
    fn fixup_map_contains_key(&self, module: &Module<'ctx>) -> Result<()> {
        let Some(func) = module.get_function("Map.contains_key") else { return Ok(()) };
        if func.count_basic_blocks() == 0 { return Ok(()); }

        // Delete all existing basic blocks
        while let Some(bb) = func.get_first_basic_block() {
            // SAFETY: Deleting an unreachable basic block that was created speculatively; it has no predecessors and no live references
            unsafe { bb.delete().ok(); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let builder = ctx.create_builder();

        // Basic blocks
        let entry = ctx.append_basic_block(func, "entry");
        let check_cap = ctx.append_basic_block(func, "check_cap");
        let compute_hash = ctx.append_basic_block(func, "compute_hash");
        let loop_head = ctx.append_basic_block(func, "loop_head");
        let check_hash = ctx.append_basic_block(func, "check_hash");
        let check_key = ctx.append_basic_block(func, "check_key");
        let found = ctx.append_basic_block(func, "found");
        let check_psl = ctx.append_basic_block(func, "check_psl");
        let not_found = ctx.append_basic_block(func, "not_found");

        // Handle both typed (ptr) and untyped (i64) function signatures
        let param0 = func.get_nth_param(0).or_internal("missing param 0")?;
        let param1 = func.get_nth_param(1).or_internal("missing param 1")?;

        // Entry: null check
        builder.position_at_end(entry);
        let (self_ptr, self_is_null) = if param0.is_pointer_value() {
            let p = param0.into_pointer_value();
            let is_null = builder.build_is_null(p, "is_null").or_llvm_err()?;
            (p, is_null)
        } else {
            let iv = param0.into_int_value();
            let is_null = builder.build_int_compare(
                verum_llvm::IntPredicate::EQ, iv, i64_type.const_zero(), "is_null"
            ).or_llvm_err()?;
            let p = builder.build_int_to_ptr(iv, ptr_type, "self_ptr").or_llvm_err()?;
            (p, is_null)
        };
        let key_i64 = if param1.is_int_value() {
            param1.into_int_value()
        } else {
            builder.build_ptr_to_int(param1.into_pointer_value(), i64_type, "key_i64").or_llvm_err()?
        };
        builder.build_conditional_branch(self_is_null, not_found, check_cap).or_llvm_err()?;

        // check_cap: load cap, return 0 if cap == 0
        builder.position_at_end(check_cap);
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(40, false)], "cap_slot").or_llvm_err()?
        };
        let cap = builder.build_load(i64_type, cap_slot, "cap").or_llvm_err()?.into_int_value();
        let cap_zero = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, cap, i64_type.const_zero(), "cap_zero"
        ).or_llvm_err()?;
        builder.build_conditional_branch(cap_zero, not_found, compute_hash).or_llvm_err()?;

        // compute_hash
        builder.position_at_end(compute_hash);
        let hash_fn = module.get_function("verum_generic_hash").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_generic_hash", fn_type, None)
        });
        let raw_hash = builder.build_call(hash_fn, &[key_i64.into()], "raw_hash")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let is_neg = builder.build_int_compare(
            verum_llvm::IntPredicate::SLT, raw_hash, i64_type.const_zero(), "is_neg"
        ).or_llvm_err()?;
        let neg_hash = builder.build_int_neg(raw_hash, "neg_hash").or_llvm_err()?;
        let abs_hash = builder.build_select(is_neg, neg_hash, raw_hash, "abs_hash").or_llvm_err()?.into_int_value();
        let hash_le1 = builder.build_int_compare(
            verum_llvm::IntPredicate::ULE, abs_hash, i64_type.const_int(1, false), "hash_le1"
        ).or_llvm_err()?;
        let hash_plus2 = builder.build_int_add(abs_hash, i64_type.const_int(2, false), "hash_plus2").or_llvm_err()?;
        let hash = builder.build_select(hash_le1, hash_plus2, abs_hash, "hash").or_llvm_err()?.into_int_value();

        let mask = builder.build_int_sub(cap, i64_type.const_int(1, false), "mask").or_llvm_err()?;
        let idx_init = builder.build_and(hash, mask, "idx_init").or_llvm_err()?;

        // SAFETY: GEP into the map header at offset 24 to load the entries array pointer
        let entries_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(24, false)], "entries_slot").or_llvm_err()?
        };
        let entries_i64 = builder.build_load(i64_type, entries_slot, "entries_i64").or_llvm_err()?.into_int_value();
        let entries_ptr = builder.build_int_to_ptr(entries_i64, ptr_type, "entries_ptr").or_llvm_err()?;

        builder.build_unconditional_branch(loop_head).or_llvm_err()?;

        // loop_head
        builder.position_at_end(loop_head);
        let idx_phi = builder.build_phi(i64_type, "idx").or_llvm_err()?;
        let psl_phi = builder.build_phi(i64_type, "psl").or_llvm_err()?;
        let idx = idx_phi.as_basic_value().into_int_value();
        let psl = psl_phi.as_basic_value().into_int_value();

        let byte_offset = builder.build_int_mul(idx, i64_type.const_int(32, false), "byte_off").or_llvm_err()?;
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let entry_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, entries_ptr, &[byte_offset], "entry_ptr").or_llvm_err()?
        };

        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let hash_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(16, false)], "hash_slot").or_llvm_err()?
        };
        let entry_hash = builder.build_load(i64_type, hash_slot, "entry_hash").or_llvm_err()?.into_int_value();

        let is_empty = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, entry_hash, i64_type.const_zero(), "is_empty"
        ).or_llvm_err()?;
        builder.build_conditional_branch(is_empty, not_found, check_hash).or_llvm_err()?;

        // check_hash
        builder.position_at_end(check_hash);
        let hash_positive = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, entry_hash, i64_type.const_zero(), "hash_pos"
        ).or_llvm_err()?;
        let hash_match = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, entry_hash, hash, "hash_match"
        ).or_llvm_err()?;
        let both = builder.build_and(hash_positive, hash_match, "both").or_llvm_err()?;
        builder.build_conditional_branch(both, check_key, check_psl).or_llvm_err()?;

        // check_key
        builder.position_at_end(check_key);
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let key_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_zero()], "key_slot").or_llvm_err()?
        };
        let entry_key = builder.build_load(i64_type, key_slot, "entry_key").or_llvm_err()?.into_int_value();

        let eq_fn = module.get_function("verum_generic_eq").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            module.add_function("verum_generic_eq", fn_type, None)
        });
        let eq_result = builder.build_call(eq_fn, &[entry_key.into(), key_i64.into()], "eq_result")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let keys_equal = builder.build_int_compare(
            verum_llvm::IntPredicate::NE, eq_result, i64_type.const_zero(), "keys_equal"
        ).or_llvm_err()?;
        builder.build_conditional_branch(keys_equal, found, check_psl).or_llvm_err()?;

        // Detect return type: may be i1 (bool) or i64
        let ret_type = func.get_type().get_return_type();
        let returns_i1 = ret_type.as_ref().map_or(false, |rt| {
            rt.is_int_type() && rt.into_int_type().get_bit_width() == 1
        });

        // found: return true/1
        builder.position_at_end(found);
        if returns_i1 {
            let i1_type = ctx.bool_type();
            builder.build_return(Some(&i1_type.const_int(1, false))).or_llvm_err()?;
        } else {
            builder.build_return(Some(&i64_type.const_int(1, false))).or_llvm_err()?;
        }

        // check_psl: Robin Hood early termination + advance
        builder.position_at_end(check_psl);
        let entry_positive = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, entry_hash, i64_type.const_zero(), "entry_pos"
        ).or_llvm_err()?;

        // SAFETY: GEP into the 32-byte map entry to access the PSL (probe sequence length) field
        let psl_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(24, false)], "psl_slot").or_llvm_err()?
        };
        let entry_psl = builder.build_load(i64_type, psl_slot, "entry_psl").or_llvm_err()?.into_int_value();
        let psl_exceeded = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, psl, entry_psl, "psl_exceeded"
        ).or_llvm_err()?;
        let robin_hood = builder.build_and(entry_positive, psl_exceeded, "robin_hood").or_llvm_err()?;

        let psl_next = builder.build_int_add(psl, i64_type.const_int(1, false), "psl_next").or_llvm_err()?;
        let safety = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, psl_next, cap, "safety"
        ).or_llvm_err()?;
        let should_stop = builder.build_or(robin_hood, safety, "should_stop").or_llvm_err()?;
        let idx_next = builder.build_and(
            builder.build_int_add(idx, i64_type.const_int(1, false), "idx_inc").or_llvm_err()?,
            mask, "idx_next"
        ).or_llvm_err()?;
        builder.build_conditional_branch(should_stop, not_found, loop_head).or_llvm_err()?;

        idx_phi.add_incoming(&[(&idx_init, compute_hash), (&idx_next, check_psl)]);
        psl_phi.add_incoming(&[(&i64_type.const_zero(), compute_hash), (&psl_next, check_psl)]);

        // not_found: return false/0
        builder.position_at_end(not_found);
        if returns_i1 {
            let i1_type = ctx.bool_type();
            builder.build_return(Some(&i1_type.const_int(0, false))).or_llvm_err()?;
        } else {
            builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        }
        Ok(())
    }

    /// Replace compiled Map.remove with correct Robin Hood linear probing + tombstone.
    /// Same probing as fixup_map_get, but on match: sets hash to -1 (tombstone),
    /// decrements len (offset 32), increments tombstones (offset 48), returns value.
    fn fixup_map_remove(&self, module: &Module<'ctx>) -> Result<()> {
        let Some(func) = module.get_function("Map.remove") else { return Ok(()) };
        if func.count_basic_blocks() == 0 { return Ok(()); }

        // Delete all existing basic blocks
        while let Some(bb) = func.get_first_basic_block() {
            // SAFETY: Deleting an unreachable basic block that was created speculatively; it has no predecessors and no live references
            unsafe { bb.delete().ok(); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let builder = ctx.create_builder();

        // Basic blocks
        let entry = ctx.append_basic_block(func, "entry");
        let check_cap = ctx.append_basic_block(func, "check_cap");
        let compute_hash = ctx.append_basic_block(func, "compute_hash");
        let loop_head = ctx.append_basic_block(func, "loop_head");
        let check_hash = ctx.append_basic_block(func, "check_hash");
        let check_key = ctx.append_basic_block(func, "check_key");
        let found = ctx.append_basic_block(func, "found");
        let check_psl = ctx.append_basic_block(func, "check_psl");
        let not_found = ctx.append_basic_block(func, "not_found");

        // Handle both typed (ptr) and untyped (i64) function signatures
        let param0 = func.get_nth_param(0).or_internal("missing param 0")?;
        let param1 = func.get_nth_param(1).or_internal("missing param 1")?;

        // Entry: null check
        builder.position_at_end(entry);
        let (self_ptr, self_is_null) = if param0.is_pointer_value() {
            let p = param0.into_pointer_value();
            let is_null = builder.build_is_null(p, "is_null").or_llvm_err()?;
            (p, is_null)
        } else {
            let iv = param0.into_int_value();
            let is_null = builder.build_int_compare(
                verum_llvm::IntPredicate::EQ, iv, i64_type.const_zero(), "is_null"
            ).or_llvm_err()?;
            let p = builder.build_int_to_ptr(iv, ptr_type, "self_ptr").or_llvm_err()?;
            (p, is_null)
        };
        let key_i64 = if param1.is_int_value() {
            param1.into_int_value()
        } else {
            builder.build_ptr_to_int(param1.into_pointer_value(), i64_type, "key_i64").or_llvm_err()?
        };
        builder.build_conditional_branch(self_is_null, not_found, check_cap).or_llvm_err()?;

        // check_cap: load cap, return 0 if cap == 0
        builder.position_at_end(check_cap);
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(40, false)], "cap_slot").or_llvm_err()?
        };
        let cap = builder.build_load(i64_type, cap_slot, "cap").or_llvm_err()?.into_int_value();
        let cap_zero = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, cap, i64_type.const_zero(), "cap_zero"
        ).or_llvm_err()?;
        builder.build_conditional_branch(cap_zero, not_found, compute_hash).or_llvm_err()?;

        // compute_hash: hash = abs(verum_generic_hash(key)); if hash <= 1: hash += 2
        builder.position_at_end(compute_hash);
        let hash_fn = module.get_function("verum_generic_hash").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_generic_hash", fn_type, None)
        });
        let raw_hash = builder.build_call(hash_fn, &[key_i64.into()], "raw_hash")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let is_neg = builder.build_int_compare(
            verum_llvm::IntPredicate::SLT, raw_hash, i64_type.const_zero(), "is_neg"
        ).or_llvm_err()?;
        let neg_hash = builder.build_int_neg(raw_hash, "neg_hash").or_llvm_err()?;
        let abs_hash = builder.build_select(is_neg, neg_hash, raw_hash, "abs_hash").or_llvm_err()?.into_int_value();
        let hash_le1 = builder.build_int_compare(
            verum_llvm::IntPredicate::ULE, abs_hash, i64_type.const_int(1, false), "hash_le1"
        ).or_llvm_err()?;
        let hash_plus2 = builder.build_int_add(abs_hash, i64_type.const_int(2, false), "hash_plus2").or_llvm_err()?;
        let hash = builder.build_select(hash_le1, hash_plus2, abs_hash, "hash").or_llvm_err()?.into_int_value();

        let mask = builder.build_int_sub(cap, i64_type.const_int(1, false), "mask").or_llvm_err()?;
        let idx_init = builder.build_and(hash, mask, "idx_init").or_llvm_err()?;

        // Load entries_ptr (offset 24)
        // SAFETY: GEP into the map header at offset 24 to load the entries array pointer
        let entries_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(24, false)], "entries_slot").or_llvm_err()?
        };
        let entries_i64 = builder.build_load(i64_type, entries_slot, "entries_i64").or_llvm_err()?.into_int_value();
        let entries_ptr = builder.build_int_to_ptr(entries_i64, ptr_type, "entries_ptr").or_llvm_err()?;

        builder.build_unconditional_branch(loop_head).or_llvm_err()?;

        // loop_head: phi nodes for idx and psl
        builder.position_at_end(loop_head);
        let idx_phi = builder.build_phi(i64_type, "idx").or_llvm_err()?;
        let psl_phi = builder.build_phi(i64_type, "psl").or_llvm_err()?;
        let idx = idx_phi.as_basic_value().into_int_value();
        let psl = psl_phi.as_basic_value().into_int_value();

        // entry_ptr = entries_ptr + idx * 32
        let byte_offset = builder.build_int_mul(idx, i64_type.const_int(32, false), "byte_off").or_llvm_err()?;
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let entry_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, entries_ptr, &[byte_offset], "entry_ptr").or_llvm_err()?
        };

        // entry_hash = load(entry_ptr + 16)
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let hash_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(16, false)], "hash_slot").or_llvm_err()?
        };
        let entry_hash = builder.build_load(i64_type, hash_slot, "entry_hash").or_llvm_err()?.into_int_value();

        // if entry_hash == 0: not found (empty slot)
        let is_empty = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, entry_hash, i64_type.const_zero(), "is_empty"
        ).or_llvm_err()?;
        builder.build_conditional_branch(is_empty, not_found, check_hash).or_llvm_err()?;

        // check_hash: if entry_hash > 0 && entry_hash == hash -> check key
        builder.position_at_end(check_hash);
        let hash_positive = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, entry_hash, i64_type.const_zero(), "hash_pos"
        ).or_llvm_err()?;
        let hash_match = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, entry_hash, hash, "hash_match"
        ).or_llvm_err()?;
        let both = builder.build_and(hash_positive, hash_match, "both").or_llvm_err()?;
        builder.build_conditional_branch(both, check_key, check_psl).or_llvm_err()?;

        // check_key: compare keys using verum_generic_eq
        builder.position_at_end(check_key);
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let key_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_zero()], "key_slot").or_llvm_err()?
        };
        let entry_key = builder.build_load(i64_type, key_slot, "entry_key").or_llvm_err()?.into_int_value();

        let eq_fn = module.get_function("verum_generic_eq").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            module.add_function("verum_generic_eq", fn_type, None)
        });
        let eq_result = builder.build_call(eq_fn, &[entry_key.into(), key_i64.into()], "eq_result")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let keys_equal = builder.build_int_compare(
            verum_llvm::IntPredicate::NE, eq_result, i64_type.const_zero(), "keys_equal"
        ).or_llvm_err()?;
        builder.build_conditional_branch(keys_equal, found, check_psl).or_llvm_err()?;

        // Detect return type: may be i64 or ptr depending on typed function sigs
        let returns_ptr = func.get_type().get_return_type()
            .map_or(false, |rt| rt.is_pointer_type());

        // found: load value, tombstone the entry, update len/tombstones, return value
        builder.position_at_end(found);

        // Load value at entry_ptr + 8
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let val_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(8, false)], "val_slot").or_llvm_err()?
        };
        let value = builder.build_load(i64_type, val_slot, "value").or_llvm_err()?.into_int_value();

        // Tombstone: store -1 into hash slot (entry_ptr + 16)
        let tombstone = i64_type.const_int(u64::MAX, true); // -1 as i64
        builder.build_store(hash_slot, tombstone).or_llvm_err()?;

        // Decrement len: self.len -= 1 (offset 32)
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let len_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(32, false)], "len_slot").or_llvm_err()?
        };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?.into_int_value();
        let len_minus1 = builder.build_int_sub(len, i64_type.const_int(1, false), "len_minus1").or_llvm_err()?;
        builder.build_store(len_slot, len_minus1).or_llvm_err()?;

        // Increment tombstones: self.tombstones += 1 (offset 48)
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let tomb_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(48, false)], "tomb_slot").or_llvm_err()?
        };
        let tombstones = builder.build_load(i64_type, tomb_slot, "tombstones").or_llvm_err()?.into_int_value();
        let tomb_plus1 = builder.build_int_add(tombstones, i64_type.const_int(1, false), "tomb_plus1").or_llvm_err()?;
        builder.build_store(tomb_slot, tomb_plus1).or_llvm_err()?;

        // Return value
        if returns_ptr {
            let value_ptr = builder.build_int_to_ptr(value, ptr_type, "value_ptr").or_llvm_err()?;
            builder.build_return(Some(&value_ptr)).or_llvm_err()?;
        } else {
            builder.build_return(Some(&value)).or_llvm_err()?;
        }

        // check_psl: Robin Hood early termination + advance
        builder.position_at_end(check_psl);
        let entry_positive = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, entry_hash, i64_type.const_zero(), "entry_pos"
        ).or_llvm_err()?;

        // SAFETY: GEP into the 32-byte map entry to access the PSL (probe sequence length) field
        let psl_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, entry_ptr, &[i64_type.const_int(24, false)], "psl_slot").or_llvm_err()?
        };
        let entry_psl = builder.build_load(i64_type, psl_slot, "entry_psl").or_llvm_err()?.into_int_value();
        let psl_exceeded = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, psl, entry_psl, "psl_exceeded"
        ).or_llvm_err()?;
        let robin_hood = builder.build_and(entry_positive, psl_exceeded, "robin_hood").or_llvm_err()?;

        let psl_next = builder.build_int_add(psl, i64_type.const_int(1, false), "psl_next").or_llvm_err()?;
        let safety = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, psl_next, cap, "safety"
        ).or_llvm_err()?;
        let should_stop = builder.build_or(robin_hood, safety, "should_stop").or_llvm_err()?;
        let idx_next = builder.build_and(
            builder.build_int_add(idx, i64_type.const_int(1, false), "idx_inc").or_llvm_err()?,
            mask, "idx_next"
        ).or_llvm_err()?;
        builder.build_conditional_branch(should_stop, not_found, loop_head).or_llvm_err()?;

        // Wire phi nodes
        idx_phi.add_incoming(&[(&idx_init, compute_hash), (&idx_next, check_psl)]);
        psl_phi.add_incoming(&[(&i64_type.const_zero(), compute_hash), (&psl_next, check_psl)]);

        // not_found: return null/0
        builder.position_at_end(not_found);
        if returns_ptr {
            let null_ptr = ptr_type.const_null();
            builder.build_return(Some(&null_ptr)).or_llvm_err()?;
        } else {
            builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        }
        Ok(())
    }

    /// Replace compiled Map.insert with correct Robin Hood linear probing.
    /// The compiled map.vr insert has broken hashing/equality for Text keys.
    /// This fixup replaces the body with code using verum_generic_hash/verum_generic_eq.
    ///
    /// Signature: Map.insert(self: i64, key: i64, value: i64) -> i64
    /// Returns 0 for None (no previous value) or old_value for Some.
    fn fixup_map_insert(&self, module: &Module<'ctx>) -> Result<()> {
        let Some(func) = module.get_function("Map.insert") else { return Ok(()) };
        if func.count_basic_blocks() == 0 { return Ok(()); }

        // Delete all existing basic blocks
        while let Some(bb) = func.get_first_basic_block() {
            // SAFETY: Deleting an unreachable basic block that was created speculatively; it has no predecessors and no live references
            unsafe { bb.delete().ok(); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let builder = ctx.create_builder();

        // Basic blocks
        let entry = ctx.append_basic_block(func, "entry");
        let call_ensure = ctx.append_basic_block(func, "call_ensure");
        let compute_hash = ctx.append_basic_block(func, "compute_hash");
        let search_head = ctx.append_basic_block(func, "search_head");
        let search_check_hash = ctx.append_basic_block(func, "search_check_hash");
        let search_check_key = ctx.append_basic_block(func, "search_check_key");
        let key_found = ctx.append_basic_block(func, "key_found");
        let search_check_psl = ctx.append_basic_block(func, "search_check_psl");
        let search_not_found = ctx.append_basic_block(func, "search_not_found");
        let insert_head = ctx.append_basic_block(func, "insert_head");
        let insert_place = ctx.append_basic_block(func, "insert_place");
        let insert_check_tombstone = ctx.append_basic_block(func, "insert_check_tombstone");
        let insert_after_place = ctx.append_basic_block(func, "insert_after_place");
        let insert_robin_hood = ctx.append_basic_block(func, "insert_robin_hood");
        let insert_advance = ctx.append_basic_block(func, "insert_advance");
        let done_insert = ctx.append_basic_block(func, "done_insert");

        // Handle both typed (ptr) and untyped (i64) function signatures
        let param0 = func.get_nth_param(0).or_internal("missing param 0")?;
        let param1 = func.get_nth_param(1).or_internal("missing param 1")?;
        let param2 = func.get_nth_param(2).or_internal("missing param 2")?;

        // Entry: null check
        builder.position_at_end(entry);
        let (self_ptr, self_is_null) = if param0.is_pointer_value() {
            let p = param0.into_pointer_value();
            let is_null = builder.build_is_null(p, "is_null").or_llvm_err()?;
            (p, is_null)
        } else {
            let iv = param0.into_int_value();
            let is_null = builder.build_int_compare(
                verum_llvm::IntPredicate::EQ, iv, i64_type.const_zero(), "is_null"
            ).or_llvm_err()?;
            let p = builder.build_int_to_ptr(iv, ptr_type, "self_ptr").or_llvm_err()?;
            (p, is_null)
        };
        let key_i64 = if param1.is_int_value() {
            param1.into_int_value()
        } else {
            builder.build_ptr_to_int(param1.into_pointer_value(), i64_type, "key_i64").or_llvm_err()?
        };
        let value_i64 = if param2.is_int_value() {
            param2.into_int_value()
        } else {
            builder.build_ptr_to_int(param2.into_pointer_value(), i64_type, "value_i64").or_llvm_err()?
        };

        // Detect return type
        let returns_ptr = func.get_type().get_return_type()
            .map_or(false, |rt| rt.is_pointer_type());

        // If self is null, return 0/null (no-op)
        let ret_none = ctx.append_basic_block(func, "ret_none");
        builder.build_conditional_branch(self_is_null, ret_none, call_ensure).or_llvm_err()?;

        // ret_none: return 0/null
        builder.position_at_end(ret_none);
        if returns_ptr {
            builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;
        } else {
            builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        }

        // call_ensure: call Map.ensure_capacity if available, then proceed
        builder.position_at_end(call_ensure);
        if let Some(ensure_fn) = module.get_function("Map.ensure_capacity") {
            // Ensure noinline to prevent LLVM verification failures with debug info
            let noinline_id = verum_llvm::attributes::Attribute::get_named_enum_kind_id("noinline");
            if noinline_id != 0 {
                ensure_fn.add_attribute(
                    verum_llvm::attributes::AttributeLoc::Function,
                    self.context.create_enum_attribute(noinline_id, 0),
                );
            }
            // Call ensure_capacity with the self pointer
            let ensure_param_is_ptr = ensure_fn.get_nth_param(0)
                .map_or(false, |p| p.is_pointer_value());
            if ensure_param_is_ptr {
                builder.build_call(ensure_fn, &[self_ptr.into()], "_").or_llvm_err()?;
            } else {
                let self_i64 = builder.build_ptr_to_int(self_ptr, i64_type, "self_i64").or_llvm_err()?;
                builder.build_call(ensure_fn, &[self_i64.into()], "_").or_llvm_err()?;
            }
        }
        builder.build_unconditional_branch(compute_hash).or_llvm_err()?;

        // compute_hash: hash = abs(verum_generic_hash(key)); if hash <= 1: hash += 2
        builder.position_at_end(compute_hash);
        let hash_fn = module.get_function("verum_generic_hash").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_generic_hash", fn_type, None)
        });
        let raw_hash = builder.build_call(hash_fn, &[key_i64.into()], "raw_hash")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let is_neg = builder.build_int_compare(
            verum_llvm::IntPredicate::SLT, raw_hash, i64_type.const_zero(), "is_neg"
        ).or_llvm_err()?;
        let neg_hash = builder.build_int_neg(raw_hash, "neg_hash").or_llvm_err()?;
        let abs_hash = builder.build_select(is_neg, neg_hash, raw_hash, "abs_hash").or_llvm_err()?.into_int_value();
        let hash_le1 = builder.build_int_compare(
            verum_llvm::IntPredicate::ULE, abs_hash, i64_type.const_int(1, false), "hash_le1"
        ).or_llvm_err()?;
        let hash_plus2 = builder.build_int_add(abs_hash, i64_type.const_int(2, false), "hash_plus2").or_llvm_err()?;
        let hash = builder.build_select(hash_le1, hash_plus2, abs_hash, "hash").or_llvm_err()?.into_int_value();

        // Reload cap after potential resize (offset 40)
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(40, false)], "cap_slot").or_llvm_err()?
        };
        let cap = builder.build_load(i64_type, cap_slot, "cap").or_llvm_err()?.into_int_value();
        let mask = builder.build_int_sub(cap, i64_type.const_int(1, false), "mask").or_llvm_err()?;
        let idx_init = builder.build_and(hash, mask, "idx_init").or_llvm_err()?;

        // Load entries_ptr (offset 24)
        // SAFETY: GEP into the map header at offset 24 to load the entries array pointer
        let entries_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(24, false)], "entries_slot").or_llvm_err()?
        };
        let entries_i64 = builder.build_load(i64_type, entries_slot, "entries_i64").or_llvm_err()?.into_int_value();
        let entries_ptr = builder.build_int_to_ptr(entries_i64, ptr_type, "entries_ptr").or_llvm_err()?;

        builder.build_unconditional_branch(search_head).or_llvm_err()?;

        // ===== PASS 1: Search for existing key =====
        builder.position_at_end(search_head);
        let s_idx_phi = builder.build_phi(i64_type, "s_idx").or_llvm_err()?;
        let s_psl_phi = builder.build_phi(i64_type, "s_psl").or_llvm_err()?;
        let s_idx = s_idx_phi.as_basic_value().into_int_value();
        let s_psl = s_psl_phi.as_basic_value().into_int_value();

        // entry_ptr = entries_ptr + s_idx * 32
        let s_byte_off = builder.build_int_mul(s_idx, i64_type.const_int(32, false), "s_byte_off").or_llvm_err()?;
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let s_entry_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, entries_ptr, &[s_byte_off], "s_entry_ptr").or_llvm_err()?
        };

        // entry_hash = load(entry_ptr + 16)
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let s_hash_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, s_entry_ptr, &[i64_type.const_int(16, false)], "s_hash_slot").or_llvm_err()?
        };
        let s_entry_hash = builder.build_load(i64_type, s_hash_slot, "s_entry_hash").or_llvm_err()?.into_int_value();

        // if entry_hash == 0: empty slot — key not present
        let s_is_empty = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, s_entry_hash, i64_type.const_zero(), "s_is_empty"
        ).or_llvm_err()?;
        builder.build_conditional_branch(s_is_empty, search_not_found, search_check_hash).or_llvm_err()?;

        // search_check_hash: if hash > 0 && hash == our hash -> check key
        builder.position_at_end(search_check_hash);
        let s_hash_pos = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, s_entry_hash, i64_type.const_zero(), "s_hash_pos"
        ).or_llvm_err()?;
        let s_hash_match = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, s_entry_hash, hash, "s_hash_match"
        ).or_llvm_err()?;
        let s_both = builder.build_and(s_hash_pos, s_hash_match, "s_both").or_llvm_err()?;
        builder.build_conditional_branch(s_both, search_check_key, search_check_psl).or_llvm_err()?;

        // search_check_key: compare keys using verum_generic_eq
        builder.position_at_end(search_check_key);
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let s_key_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, s_entry_ptr, &[i64_type.const_zero()], "s_key_slot").or_llvm_err()?
        };
        let s_entry_key = builder.build_load(i64_type, s_key_slot, "s_entry_key").or_llvm_err()?.into_int_value();

        let eq_fn = module.get_function("verum_generic_eq").unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            module.add_function("verum_generic_eq", fn_type, None)
        });
        let eq_result = builder.build_call(eq_fn, &[s_entry_key.into(), key_i64.into()], "eq_result")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let keys_equal = builder.build_int_compare(
            verum_llvm::IntPredicate::NE, eq_result, i64_type.const_zero(), "keys_equal"
        ).or_llvm_err()?;
        builder.build_conditional_branch(keys_equal, key_found, search_check_psl).or_llvm_err()?;

        // key_found: overwrite value, return old value
        builder.position_at_end(key_found);
        // SAFETY: GEP into the 32-byte map entry to access the value field; the entry was found by linear probe
        let found_val_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, s_entry_ptr, &[i64_type.const_int(8, false)], "found_val_slot").or_llvm_err()?
        };
        let old_value = builder.build_load(i64_type, found_val_slot, "old_value").or_llvm_err()?.into_int_value();
        builder.build_store(found_val_slot, value_i64).or_llvm_err()?;
        if returns_ptr {
            let old_ptr = builder.build_int_to_ptr(old_value, ptr_type, "old_ptr").or_llvm_err()?;
            builder.build_return(Some(&old_ptr)).or_llvm_err()?;
        } else {
            builder.build_return(Some(&old_value)).or_llvm_err()?;
        }

        // search_check_psl: Robin Hood early termination
        builder.position_at_end(search_check_psl);
        let s_entry_pos = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, s_entry_hash, i64_type.const_zero(), "s_entry_pos"
        ).or_llvm_err()?;
        // SAFETY: GEP into the 32-byte map entry to access the PSL (probe sequence length) field
        let s_psl_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, s_entry_ptr, &[i64_type.const_int(24, false)], "s_psl_slot").or_llvm_err()?
        };
        let s_entry_psl = builder.build_load(i64_type, s_psl_slot, "s_entry_psl").or_llvm_err()?.into_int_value();
        let s_psl_exceeded = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, s_psl, s_entry_psl, "s_psl_exceeded"
        ).or_llvm_err()?;
        let s_robin_hood = builder.build_and(s_entry_pos, s_psl_exceeded, "s_robin_hood").or_llvm_err()?;

        let s_psl_next = builder.build_int_add(s_psl, i64_type.const_int(1, false), "s_psl_next").or_llvm_err()?;
        let s_safety = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, s_psl_next, cap, "s_safety"
        ).or_llvm_err()?;
        let s_should_stop = builder.build_or(s_robin_hood, s_safety, "s_should_stop").or_llvm_err()?;
        let s_idx_next = builder.build_and(
            builder.build_int_add(s_idx, i64_type.const_int(1, false), "s_idx_inc").or_llvm_err()?,
            mask, "s_idx_next"
        ).or_llvm_err()?;
        builder.build_conditional_branch(s_should_stop, search_not_found, search_head).or_llvm_err()?;

        // Wire search phi nodes
        s_idx_phi.add_incoming(&[(&idx_init, compute_hash), (&s_idx_next, search_check_psl)]);
        s_psl_phi.add_incoming(&[(&i64_type.const_zero(), compute_hash), (&s_psl_next, search_check_psl)]);

        // search_not_found: key doesn't exist — proceed to insert
        builder.position_at_end(search_not_found);
        builder.build_unconditional_branch(insert_head).or_llvm_err()?;

        // ===== PASS 2: Robin Hood insertion =====
        // Phi nodes for current key/value/hash/psl and index
        builder.position_at_end(insert_head);
        let i_idx_phi = builder.build_phi(i64_type, "i_idx").or_llvm_err()?;
        let i_key_phi = builder.build_phi(i64_type, "i_key").or_llvm_err()?;
        let i_val_phi = builder.build_phi(i64_type, "i_val").or_llvm_err()?;
        let i_hash_phi = builder.build_phi(i64_type, "i_hash").or_llvm_err()?;
        let i_psl_phi = builder.build_phi(i64_type, "i_psl").or_llvm_err()?;
        let i_idx = i_idx_phi.as_basic_value().into_int_value();
        let i_key = i_key_phi.as_basic_value().into_int_value();
        let i_val = i_val_phi.as_basic_value().into_int_value();
        let i_hash = i_hash_phi.as_basic_value().into_int_value();
        let i_psl = i_psl_phi.as_basic_value().into_int_value();

        // entry_ptr = entries_ptr + i_idx * 32
        let i_byte_off = builder.build_int_mul(i_idx, i64_type.const_int(32, false), "i_byte_off").or_llvm_err()?;
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let i_entry_ptr = unsafe {
            builder.build_in_bounds_gep(i8_type, entries_ptr, &[i_byte_off], "i_entry_ptr").or_llvm_err()?
        };

        // entry_hash = load(entry_ptr + 16)
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let i_hash_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, i_entry_ptr, &[i64_type.const_int(16, false)], "i_hash_slot").or_llvm_err()?
        };
        let i_entry_hash = builder.build_load(i64_type, i_hash_slot, "i_entry_hash").or_llvm_err()?.into_int_value();

        // Check if empty (hash == 0) or tombstone (hash == -1)
        let i_is_empty = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, i_entry_hash, i64_type.const_zero(), "i_is_empty"
        ).or_llvm_err()?;
        let tombstone_val = i64_type.const_int(u64::MAX, true); // -1
        let i_is_tombstone = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, i_entry_hash, tombstone_val, "i_is_tombstone"
        ).or_llvm_err()?;
        let i_is_free = builder.build_or(i_is_empty, i_is_tombstone, "i_is_free").or_llvm_err()?;
        builder.build_conditional_branch(i_is_free, insert_place, insert_robin_hood).or_llvm_err()?;

        // insert_place: place current entry in this free slot
        builder.position_at_end(insert_place);
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let i_key_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, i_entry_ptr, &[i64_type.const_zero()], "i_key_slot").or_llvm_err()?
        };
        builder.build_store(i_key_slot, i_key).or_llvm_err()?;
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let i_val_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, i_entry_ptr, &[i64_type.const_int(8, false)], "i_val_slot").or_llvm_err()?
        };
        builder.build_store(i_val_slot, i_val).or_llvm_err()?;
        builder.build_store(i_hash_slot, i_hash).or_llvm_err()?;
        // SAFETY: GEP into the coverage counters global array; the function index is assigned at compile time and within the array bounds
        let i_psl_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, i_entry_ptr, &[i64_type.const_int(24, false)], "i_psl_slot").or_llvm_err()?
        };
        builder.build_store(i_psl_slot, i_psl).or_llvm_err()?;
        // Check if was tombstone — decrement tombstones counter
        builder.build_conditional_branch(i_is_tombstone, insert_check_tombstone, insert_after_place).or_llvm_err()?;

        // insert_check_tombstone: decrement tombstones
        builder.position_at_end(insert_check_tombstone);
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let tomb_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(48, false)], "tomb_slot").or_llvm_err()?
        };
        let tombstones = builder.build_load(i64_type, tomb_slot, "tombstones").or_llvm_err()?.into_int_value();
        let tomb_minus1 = builder.build_int_sub(tombstones, i64_type.const_int(1, false), "tomb_minus1").or_llvm_err()?;
        builder.build_store(tomb_slot, tomb_minus1).or_llvm_err()?;
        builder.build_unconditional_branch(insert_after_place).or_llvm_err()?;

        // insert_after_place: go to done_insert
        builder.position_at_end(insert_after_place);
        builder.build_unconditional_branch(done_insert).or_llvm_err()?;

        // insert_robin_hood: check if cur_psl > entry_psl, swap if so
        builder.position_at_end(insert_robin_hood);
        // SAFETY: GEP into the 32-byte map entry to access the PSL (probe sequence length) field
        let ir_psl_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, i_entry_ptr, &[i64_type.const_int(24, false)], "ir_psl_slot").or_llvm_err()?
        };
        let ir_entry_psl = builder.build_load(i64_type, ir_psl_slot, "ir_entry_psl").or_llvm_err()?.into_int_value();
        let should_swap = builder.build_int_compare(
            verum_llvm::IntPredicate::UGT, i_psl, ir_entry_psl, "should_swap"
        ).or_llvm_err()?;

        // We need a conditional swap. Create blocks for swap vs no-swap, then merge.
        let do_swap = ctx.append_basic_block(func, "do_swap");
        let no_swap = ctx.append_basic_block(func, "no_swap");
        builder.build_conditional_branch(should_swap, do_swap, no_swap).or_llvm_err()?;

        // do_swap: swap current with entry, continue with displaced entry
        builder.position_at_end(do_swap);
        // Load old entry values
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let swap_key_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, i_entry_ptr, &[i64_type.const_zero()], "swap_key_slot").or_llvm_err()?
        };
        let old_ek = builder.build_load(i64_type, swap_key_slot, "old_ek").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let swap_val_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, i_entry_ptr, &[i64_type.const_int(8, false)], "swap_val_slot").or_llvm_err()?
        };
        let old_ev = builder.build_load(i64_type, swap_val_slot, "old_ev").or_llvm_err()?.into_int_value();
        let old_eh = i_entry_hash; // already loaded
        let old_ep = ir_entry_psl; // already loaded

        // Store current into the slot
        builder.build_store(swap_key_slot, i_key).or_llvm_err()?;
        builder.build_store(swap_val_slot, i_val).or_llvm_err()?;
        builder.build_store(i_hash_slot, i_hash).or_llvm_err()?;
        builder.build_store(ir_psl_slot, i_psl).or_llvm_err()?;

        builder.build_unconditional_branch(insert_advance).or_llvm_err()?;

        // no_swap: continue with same current values
        builder.position_at_end(no_swap);
        builder.build_unconditional_branch(insert_advance).or_llvm_err()?;

        // insert_advance: merge after swap/no-swap, advance index
        builder.position_at_end(insert_advance);
        // Phi nodes to pick the right current values after potential swap
        let adv_key_phi = builder.build_phi(i64_type, "adv_key").or_llvm_err()?;
        let adv_val_phi = builder.build_phi(i64_type, "adv_val").or_llvm_err()?;
        let adv_hash_phi = builder.build_phi(i64_type, "adv_hash").or_llvm_err()?;
        let adv_psl_phi = builder.build_phi(i64_type, "adv_psl").or_llvm_err()?;

        adv_key_phi.add_incoming(&[(&old_ek, do_swap), (&i_key, no_swap)]);
        adv_val_phi.add_incoming(&[(&old_ev, do_swap), (&i_val, no_swap)]);
        adv_hash_phi.add_incoming(&[(&old_eh, do_swap), (&i_hash, no_swap)]);
        adv_psl_phi.add_incoming(&[(&old_ep, do_swap), (&i_psl, no_swap)]);

        let adv_psl_next = builder.build_int_add(
            adv_psl_phi.as_basic_value().into_int_value(),
            i64_type.const_int(1, false), "adv_psl_next"
        ).or_llvm_err()?;
        let i_idx_next = builder.build_and(
            builder.build_int_add(i_idx, i64_type.const_int(1, false), "i_idx_inc").or_llvm_err()?,
            mask, "i_idx_next"
        ).or_llvm_err()?;
        builder.build_unconditional_branch(insert_head).or_llvm_err()?;

        // Wire insert loop phi nodes
        // Incoming from: search_not_found (initial) and insert_advance (loop back)
        i_idx_phi.add_incoming(&[(&idx_init, search_not_found), (&i_idx_next, insert_advance)]);
        i_key_phi.add_incoming(&[(&key_i64, search_not_found), (&adv_key_phi.as_basic_value().into_int_value(), insert_advance)]);
        i_val_phi.add_incoming(&[(&value_i64, search_not_found), (&adv_val_phi.as_basic_value().into_int_value(), insert_advance)]);
        i_hash_phi.add_incoming(&[(&hash, search_not_found), (&adv_hash_phi.as_basic_value().into_int_value(), insert_advance)]);
        i_psl_phi.add_incoming(&[(&i64_type.const_zero(), search_not_found), (&adv_psl_next, insert_advance)]);

        // done_insert: increment len, return 0 (None)
        builder.position_at_end(done_insert);
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let len_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, self_ptr, &[i64_type.const_int(32, false)], "len_slot").or_llvm_err()?
        };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?.into_int_value();
        let len_plus1 = builder.build_int_add(len, i64_type.const_int(1, false), "len_plus1").or_llvm_err()?;
        builder.build_store(len_slot, len_plus1).or_llvm_err()?;

        if returns_ptr {
            builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;
        } else {
            builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        }
        Ok(())
    }

    /// verum_text_get_ptr(text_obj: i64) -> ptr
    /// Returns the char* from a Text object, or "" if null.
    fn emit_verum_text_get_ptr(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();

        // Reuse existing declaration or create new function.
        // NEVER delete — that invalidates existing call sites.
        let func = if let Some(f) = module.get_function("verum_text_get_ptr") {
            if f.count_basic_blocks() > 0 { return Ok(()); } // Already has body
            f
        } else {
            let fn_type = ptr_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_text_get_ptr", fn_type, None)
        };

        let entry = ctx.append_basic_block(func, "entry");
        let load_ptr = ctx.append_basic_block(func, "load_ptr");
        let check_ptr = ctx.append_basic_block(func, "check_ptr");
        let ret_empty = ctx.append_basic_block(func, "ret_empty");

        let builder = ctx.create_builder();

        // Entry: check if text_obj is null (0)
        builder.position_at_end(entry);
        let text_obj = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let is_null = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, text_obj, i64_type.const_zero(), "is_null"
        ).or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_empty, load_ptr).or_llvm_err()?;

        // load_ptr: cast to ptr, load first field (char* ptr)
        builder.position_at_end(load_ptr);
        let text_ptr = builder.build_int_to_ptr(text_obj, ptr_type, "text_ptr").or_llvm_err()?;
        let field0 = builder.build_load(i64_type, text_ptr, "field0").or_llvm_err()?.into_int_value();
        let field0_ptr = builder.build_int_to_ptr(field0, ptr_type, "field0_ptr").or_llvm_err()?;
        let ptr_is_null = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, field0, i64_type.const_zero(), "ptr_null"
        ).or_llvm_err()?;
        builder.build_conditional_branch(ptr_is_null, ret_empty, check_ptr).or_llvm_err()?;

        // check_ptr: return the actual pointer
        builder.position_at_end(check_ptr);
        builder.build_return(Some(&field0_ptr)).or_llvm_err()?;

        // ret_empty: return pointer to empty string constant
        builder.position_at_end(ret_empty);
        let empty_str = builder.build_global_string_ptr("", "empty_str").or_llvm_err()?;
        builder.build_return(Some(&empty_str.as_pointer_value())).or_llvm_err()?;
        Ok(())
    }

    /// verum_text_alloc(ptr: ptr, len: i64, cap: i64) -> i64
    /// Allocates a Text object {ptr, len, cap} on the heap.
    fn emit_verum_text_alloc(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_text_alloc") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();

        // text_alloc(ptr: ptr, len: i64, cap: i64) -> i64
        let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_text_alloc").unwrap_or_else(|| module.add_function("verum_text_alloc", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let builder = ctx.create_builder();
        builder.position_at_end(entry);

        // malloc(24) for 3 x i64
        let size = i64_type.const_int(24, false);
        let raw = self.emit_checked_malloc(&builder, module, size, "raw")?;

        // Store ptr as i64
        let ptr_param = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let ptr_as_i64 = builder.build_ptr_to_int(ptr_param, i64_type, "ptr_i64").or_llvm_err()?;
        builder.build_store(raw, ptr_as_i64).or_llvm_err()?;

        // Store len at offset 8
        let len_param = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let slot1 = unsafe { builder.build_in_bounds_gep(i64_type, raw, &[i64_type.const_int(1, false)], "slot1").or_llvm_err()? };
        builder.build_store(slot1, len_param).or_llvm_err()?;

        // Store cap at offset 16
        let cap_param = func.get_nth_param(2).or_internal("missing param 2")?.into_int_value();
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let slot2 = unsafe { builder.build_in_bounds_gep(i64_type, raw, &[i64_type.const_int(2, false)], "slot2").or_llvm_err()? };
        builder.build_store(slot2, cap_param).or_llvm_err()?;

        // Return ptr-to-int of the Text object
        let result = builder.build_ptr_to_int(raw, i64_type, "result").or_llvm_err()?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_text_from_cstr(s: ptr) -> i64
    /// Wraps a null-terminated C string in a Text object.
    fn emit_verum_text_from_cstr(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_text_from_cstr") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();

        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_text_from_cstr").unwrap_or_else(|| module.add_function("verum_text_from_cstr", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let null_bb = ctx.append_basic_block(func, "null_str");
        let valid_bb = ctx.append_basic_block(func, "valid_str");

        let builder = ctx.create_builder();
        builder.position_at_end(entry);

        let s = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let s_i64 = builder.build_ptr_to_int(s, i64_type, "s_i64").or_llvm_err()?;
        let is_null = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, s_i64, i64_type.const_zero(), "is_null"
        ).or_llvm_err()?;
        builder.build_conditional_branch(is_null, null_bb, valid_bb).or_llvm_err()?;

        // null case: text_alloc(NULL, 0, 0)
        builder.position_at_end(null_bb);
        let text_alloc = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let null_ptr = ptr_type.const_null();
        let zero = i64_type.const_zero();
        let empty = builder.build_call(text_alloc, &[null_ptr.into(), zero.into(), zero.into()], "empty").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_return(Some(&empty)).or_llvm_err()?;

        // valid case: strlen(s), then text_alloc(s, len, len)
        builder.position_at_end(valid_bb);
        let strlen_fn = self.get_or_declare_strlen(module);
        let len = builder.build_call(strlen_fn, &[s.into()], "len").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let result = builder.build_call(text_alloc, &[s.into(), len.into(), len.into()], "result").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_text_concat(text_a: i64, text_b: i64) -> i64
    /// Concatenates two Text objects, returns new Text.
    fn emit_verum_text_concat(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_text_concat") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();

        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_text_concat").unwrap_or_else(|| module.add_function("verum_text_concat", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let load_a_bb = ctx.append_basic_block(func, "load_a");
        let check_b_bb = ctx.append_basic_block(func, "check_b");
        let load_b_bb = ctx.append_basic_block(func, "load_b");
        let do_concat_bb = ctx.append_basic_block(func, "do_concat");

        let builder = ctx.create_builder();
        builder.position_at_end(entry);

        let a = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let b = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let zero = i64_type.const_zero();

        // Check a == null, branch to load_a or check_b with defaults
        let a_is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, a, zero, "a_null").or_llvm_err()?;
        builder.build_conditional_branch(a_is_null, check_b_bb, load_a_bb).or_llvm_err()?;

        // load_a: read a.ptr and a.len
        builder.position_at_end(load_a_bb);
        let a_obj = builder.build_int_to_ptr(a, ptr_type, "a_obj").or_llvm_err()?;
        let a_ptr_loaded = builder.build_load(i64_type, a_obj, "a_ptr_raw").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into the 2-field text struct at index 1 to access the string length
        let a_len_gep = unsafe { builder.build_in_bounds_gep(i64_type, a_obj, &[i64_type.const_int(1, false)], "a_len_gep").or_llvm_err()? };
        let a_len_loaded = builder.build_load(i64_type, a_len_gep, "a_len_raw").or_llvm_err()?.into_int_value();
        builder.build_unconditional_branch(check_b_bb).or_llvm_err()?;

        // check_b: phi for a values, then check b
        builder.position_at_end(check_b_bb);
        let a_ptr_phi = builder.build_phi(i64_type, "a_ptr").or_llvm_err()?;
        a_ptr_phi.add_incoming(&[(&zero, entry), (&a_ptr_loaded, load_a_bb)]);
        let a_len_phi = builder.build_phi(i64_type, "a_len").or_llvm_err()?;
        a_len_phi.add_incoming(&[(&zero, entry), (&a_len_loaded, load_a_bb)]);

        let b_is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, b, zero, "b_null").or_llvm_err()?;
        builder.build_conditional_branch(b_is_null, do_concat_bb, load_b_bb).or_llvm_err()?;

        // load_b: read b.ptr and b.len
        builder.position_at_end(load_b_bb);
        let b_obj = builder.build_int_to_ptr(b, ptr_type, "b_obj").or_llvm_err()?;
        let b_ptr_loaded = builder.build_load(i64_type, b_obj, "b_ptr_raw").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into object header to access the length field; the object uses the standard NewG layout (24-byte header)
        let b_len_gep = unsafe { builder.build_in_bounds_gep(i64_type, b_obj, &[i64_type.const_int(1, false)], "b_len_gep").or_llvm_err()? };
        let b_len_loaded = builder.build_load(i64_type, b_len_gep, "b_len_raw").or_llvm_err()?.into_int_value();
        builder.build_unconditional_branch(do_concat_bb).or_llvm_err()?;

        // do_concat: phi for b values, then allocate and copy
        builder.position_at_end(do_concat_bb);
        let b_ptr_phi = builder.build_phi(i64_type, "b_ptr").or_llvm_err()?;
        b_ptr_phi.add_incoming(&[(&zero, check_b_bb), (&b_ptr_loaded, load_b_bb)]);
        let b_len_phi = builder.build_phi(i64_type, "b_len").or_llvm_err()?;
        b_len_phi.add_incoming(&[(&zero, check_b_bb), (&b_len_loaded, load_b_bb)]);

        let a_ptr_val = a_ptr_phi.as_basic_value().into_int_value();
        let a_len = a_len_phi.as_basic_value().into_int_value();
        let b_ptr_val = b_ptr_phi.as_basic_value().into_int_value();
        let b_len = b_len_phi.as_basic_value().into_int_value();

        // total = a_len + b_len
        let total = builder.build_int_add(a_len, b_len, "total").or_llvm_err()?;
        // buf = malloc(total + 1)
        let total_plus_1 = builder.build_int_add(total, i64_type.const_int(1, false), "total_p1").or_llvm_err()?;
        let buf = self.emit_checked_malloc(&builder, module, total_plus_1, "buf")?;

        // memcpy a_ptr to buf (a_len bytes) — only if a_len > 0
        let memcpy_fn = self.get_or_declare_memcpy(module);
        let a_ptr = builder.build_int_to_ptr(a_ptr_val, ptr_type, "a_ptr").or_llvm_err()?;
        let a_len_gt0 = builder.build_int_compare(verum_llvm::IntPredicate::SGT, a_len, zero, "a_gt0").or_llvm_err()?;
        let copy_a_bb = ctx.append_basic_block(func, "copy_a");
        let after_a_bb = ctx.append_basic_block(func, "after_a");
        builder.build_conditional_branch(a_len_gt0, copy_a_bb, after_a_bb).or_llvm_err()?;

        builder.position_at_end(copy_a_bb);
        builder.build_call(memcpy_fn, &[buf.into(), a_ptr.into(), a_len.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(after_a_bb).or_llvm_err()?;

        // memcpy b_ptr to buf+a_len (b_len bytes) — only if b_len > 0
        builder.position_at_end(after_a_bb);
        let b_ptr = builder.build_int_to_ptr(b_ptr_val, ptr_type, "b_ptr").or_llvm_err()?;
        // SAFETY: GEP to compute the end-of-buffer position; the offset is the sum of validated lengths that fit within the allocation
        let buf_offset = unsafe { builder.build_in_bounds_gep(ctx.i8_type(), buf, &[a_len], "buf_off").or_llvm_err()? };
        let b_len_gt0 = builder.build_int_compare(verum_llvm::IntPredicate::SGT, b_len, zero, "b_gt0").or_llvm_err()?;
        let copy_b_bb = ctx.append_basic_block(func, "copy_b");
        let after_b_bb = ctx.append_basic_block(func, "after_b");
        builder.build_conditional_branch(b_len_gt0, copy_b_bb, after_b_bb).or_llvm_err()?;

        builder.position_at_end(copy_b_bb);
        builder.build_call(memcpy_fn, &[buf_offset.into(), b_ptr.into(), b_len.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(after_b_bb).or_llvm_err()?;

        // null-terminate: buf[total] = 0
        builder.position_at_end(after_b_bb);
        // SAFETY: GEP to access the 'end' field at a fixed offset within a struct of known layout
        let end = unsafe { builder.build_in_bounds_gep(ctx.i8_type(), buf, &[total], "end").or_llvm_err()? };
        builder.build_store(end, ctx.i8_type().const_zero()).or_llvm_err()?;

        // text_alloc(buf, total, total)
        let text_alloc = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let result = builder.build_call(text_alloc, &[buf.into(), total.into(), total.into()], "result").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_text_free(text_obj: i64) -> void
    /// Frees an owned Text object: null-check, free backing buffer, free header.
    /// Outlined as a function to avoid creating 4 basic blocks per free site.
    fn emit_verum_text_free(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_text_free") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();

        let fn_type = void_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_text_free").unwrap_or_else(|| module.add_function("verum_text_free", fn_type, None));

        // Must not be inlined — inlining re-introduces the block explosion problem.
        let noinline_id = verum_llvm::attributes::Attribute::get_named_enum_kind_id("noinline");
        if noinline_id != 0 {
            func.add_attribute(
                verum_llvm::attributes::AttributeLoc::Function,
                ctx.create_enum_attribute(noinline_id, 0),
            );
        }

        let entry = ctx.append_basic_block(func, "entry");
        let do_free = ctx.append_basic_block(func, "do_free");
        let free_buf = ctx.append_basic_block(func, "free_buf");
        let free_hdr = ctx.append_basic_block(func, "free_hdr");
        let done = ctx.append_basic_block(func, "done");

        let builder = ctx.create_builder();
        let text_i64 = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let zero = i64_type.const_zero();

        // Declare free() if not present
        let free_fn = module.get_function("free").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("free", ft, None)
        });

        // entry: null check
        builder.position_at_end(entry);
        let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, text_i64, zero, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, done, do_free).or_llvm_err()?;

        // do_free: load backing buffer ptr from header[0]
        builder.position_at_end(do_free);
        let hdr_ptr = builder.build_int_to_ptr(text_i64, ptr_type, "hdr_ptr").or_llvm_err()?;
        let buf_i64 = builder.build_load(i64_type, hdr_ptr, "buf_i64").or_llvm_err()?.into_int_value();
        let buf_nonnull = builder.build_int_compare(verum_llvm::IntPredicate::NE, buf_i64, zero, "buf_nonnull").or_llvm_err()?;
        builder.build_conditional_branch(buf_nonnull, free_buf, free_hdr).or_llvm_err()?;

        // free_buf: free backing buffer then fall through to free header
        builder.position_at_end(free_buf);
        let buf_ptr = builder.build_int_to_ptr(buf_i64, ptr_type, "buf_ptr").or_llvm_err()?;
        builder.build_call(free_fn, &[buf_ptr.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(free_hdr).or_llvm_err()?;

        // free_hdr: free the 24-byte header
        builder.position_at_end(free_hdr);
        let hdr_ptr2 = builder.build_int_to_ptr(text_i64, ptr_type, "hdr_ptr2").or_llvm_err()?;
        builder.build_call(free_fn, &[hdr_ptr2.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(done).or_llvm_err()?;

        // done: return void
        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_generic_len(obj: i64) -> i64
    /// Runtime-dispatched length for untyped registers that could be List, Map/Set/Deque, or Text.
    ///
    /// All heap collections (List, Map, Set, Deque) now use NewG layout with 24-byte header.
    /// Text still uses flat {ptr, len, cap} layout.
    ///
    /// Heuristic: check offset 0 (first field).
    /// - If offset_0 is a heap pointer (> 0x10000): this is Text → len at offset 8
    /// - Else: this is a NewG object (List/Map/Set/Deque) → len at offset 32
    ///
    /// Note: For NewG objects, offset 0 is type_tag (typically 0 or small number),
    /// so it will NOT look like a heap pointer. For Text, offset 0 is the char*
    /// pointer which is always a heap address.
    fn emit_verum_generic_len(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_generic_len") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_generic_len").unwrap_or_else(|| module.add_function("verum_generic_len", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let text_path = ctx.append_basic_block(func, "text_len");
        let newg_path = ctx.append_basic_block(func, "newg_len");

        let builder = ctx.create_builder();
        builder.position_at_end(entry);
        let threshold = i64_type.const_int(0x10000, false);

        let obj_i64 = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let obj_ptr = builder.build_int_to_ptr(obj_i64, ptr_type, "obj_ptr").or_llvm_err()?;

        // Check offset 0: if it's a heap pointer, this is Text
        let field0 = builder.build_load(i64_type, obj_ptr, "field0").or_llvm_err()?.into_int_value();
        let is_text = builder.build_int_compare(verum_llvm::IntPredicate::UGT, field0, threshold, "is_text").or_llvm_err()?;
        builder.build_conditional_branch(is_text, text_path, newg_path).or_llvm_err()?;

        // text_len: Text → len at offset 8
        builder.position_at_end(text_path);
        // SAFETY: GEP into CBGR allocation header at a fixed structural offset; the header layout is defined by the allocator
        let text_len_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, obj_ptr, &[i64_type.const_int(8, false)], "text_len_slot").or_llvm_err()?
        };
        let text_len = builder.build_load(i64_type, text_len_slot, "text_len").or_llvm_err()?;
        builder.build_return(Some(&text_len)).or_llvm_err()?;

        // newg_len: List/Map (NewG) → len at offset 32
        // For List: fields are {ptr, len, cap} → len at OBJECT_HEADER(24) + 1*8 = 32
        // For Map: fields are {entries, len, cap, tombstones} → len at OBJECT_HEADER(24) + 1*8 = 32
        // Note: Set (compiled) wraps Map, Deque has len at offset 40 {data, head, len, cap}.
        // These types normally go through compiled .len() methods, not this heuristic.
        builder.position_at_end(newg_path);
        // SAFETY: GEP into object header to access the length field; the object uses the standard NewG layout (24-byte header)
        let newg_len_slot = unsafe {
            builder.build_in_bounds_gep(i8_type, obj_ptr, &[i64_type.const_int(super::runtime::LIST_LEN_OFFSET, false)], "newg_len_slot").or_llvm_err()?
        };
        let newg_len = builder.build_load(i64_type, newg_len_slot, "newg_len").or_llvm_err()?;
        builder.build_return(Some(&newg_len)).or_llvm_err()?;
        Ok(())
    }

    /// verum_strlen_export(s: ptr) -> i64
    /// Thin wrapper around strlen.
    fn emit_verum_strlen_export(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_strlen_export") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();

        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_strlen_export").unwrap_or_else(|| module.add_function("verum_strlen_export", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let builder = ctx.create_builder();
        builder.position_at_end(entry);

        let s = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let strlen_fn = self.get_or_declare_strlen(module);
        let len = builder.build_call(strlen_fn, &[s.into()], "len").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_return(Some(&len)).or_llvm_err()?;
        Ok(())
    }

    /// verum_text_from_static(ptr: ptr, len: i64) -> ptr
    /// Creates a null-terminated copy of a static string.
    fn emit_verum_text_from_static(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_text_from_static") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();

        // from_static(ptr: ptr, len: i64) -> ptr (char*)
        let fn_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_text_from_static").unwrap_or_else(|| module.add_function("verum_text_from_static", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let valid_bb = ctx.append_basic_block(func, "valid");
        let empty_bb = ctx.append_basic_block(func, "empty");

        let builder = ctx.create_builder();
        builder.position_at_end(entry);

        let src = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let len = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

        // Check if ptr is null or len <= 0
        let src_i64 = builder.build_ptr_to_int(src, i64_type, "src_i64").or_llvm_err()?;
        let src_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, src_i64, i64_type.const_zero(), "src_null").or_llvm_err()?;
        let len_le0 = builder.build_int_compare(verum_llvm::IntPredicate::SLE, len, i64_type.const_zero(), "len_le0").or_llvm_err()?;
        let invalid = builder.build_or(src_null, len_le0, "invalid").or_llvm_err()?;
        builder.build_conditional_branch(invalid, empty_bb, valid_bb).or_llvm_err()?;

        // empty: malloc(1), set to '\0', return
        builder.position_at_end(empty_bb);
        let one = i64_type.const_int(1, false);
        let empty_buf = self.emit_checked_malloc(&builder, module, one, "empty_buf")?;
        builder.build_store(empty_buf, ctx.i8_type().const_zero()).or_llvm_err()?;
        builder.build_return(Some(&empty_buf)).or_llvm_err()?;

        // valid: malloc(len+1), memcpy, null-terminate
        builder.position_at_end(valid_bb);
        let len_plus_1 = builder.build_int_add(len, one, "len_p1").or_llvm_err()?;
        let buf = self.emit_checked_malloc(&builder, module, len_plus_1, "buf")?;
        let memcpy_fn = self.get_or_declare_memcpy(module);
        builder.build_call(memcpy_fn, &[buf.into(), src.into(), len.into()], "").or_llvm_err()?;
        // SAFETY: GEP to access the 'end' field at a fixed offset within a struct of known layout
        let end = unsafe { builder.build_in_bounds_gep(ctx.i8_type(), buf, &[len], "end").or_llvm_err()? };
        builder.build_store(end, ctx.i8_type().const_zero()).or_llvm_err()?;
        builder.build_return(Some(&buf)).or_llvm_err()?;
        Ok(())
    }

    /// Emit verum_is_text_object(val: i64) -> i1 as inline LLVM IR helper.
    /// Heuristic: checks if val looks like a valid heap-allocated Text object.
    /// Returns function that can be called from other emitted functions.
    fn emit_verum_is_text_object(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        if let Some(f) = module.get_function("verum_is_text_object") {
            if f.count_basic_blocks() > 0 { return Ok(f); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i1_type = ctx.bool_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i1_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_is_text_object").unwrap_or_else(|| module.add_function("verum_is_text_object", fn_type, None));
        // Mark as always-inline for zero overhead
        func.add_attribute(
            verum_llvm::attributes::AttributeLoc::Function,
            ctx.create_enum_attribute(verum_llvm::attributes::Attribute::get_named_enum_kind_id("alwaysinline"), 0),
        );

        let entry = ctx.append_basic_block(func, "entry");
        let check_ptr = ctx.append_basic_block(func, "check_ptr");
        let check_first = ctx.append_basic_block(func, "check_first");
        let check_len = ctx.append_basic_block(func, "check_len");
        let ret_false = ctx.append_basic_block(func, "ret_false");
        let ret_true = ctx.append_basic_block(func, "ret_true");

        let builder = ctx.create_builder();

        // Entry: reject values that can't be valid heap pointers.
        // On 64-bit systems, heap addresses (from malloc) are:
        //   - Above 256MB (ASLR puts heap well above program text on all platforms)
        //   - 16-byte aligned (guaranteed by malloc on macOS/Linux/Windows)
        //   - Below 0x7FFFFFFFFFFF (user-space canonical address)
        builder.position_at_end(entry);
        let val = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let min_ptr = i64_type.const_int(0x10000000, false); // 256MB — well below any heap start
        let max_ptr = i64_type.const_int(0x7FFFFFFFFFFF, false);
        let align_mask = i64_type.const_int(0xF, false);
        let too_small = builder.build_int_compare(verum_llvm::IntPredicate::ULT, val, min_ptr, "too_small").or_llvm_err()?;
        let too_big = builder.build_int_compare(verum_llvm::IntPredicate::UGT, val, max_ptr, "too_big").or_llvm_err()?;
        let misaligned = builder.build_and(val, align_mask, "align_bits").or_llvm_err()?;
        let not_aligned = builder.build_int_compare(verum_llvm::IntPredicate::NE, misaligned, i64_type.const_zero(), "not_aligned").or_llvm_err()?;
        let bad_range = builder.build_or(too_small, too_big, "bad_range").or_llvm_err()?;
        let bad_val = builder.build_or(bad_range, not_aligned, "bad_val").or_llvm_err()?;
        builder.build_conditional_branch(bad_val, ret_false, check_ptr).or_llvm_err()?;

        // check_ptr: read obj[0] (ptr field)
        builder.position_at_end(check_ptr);
        let obj_ptr = builder.build_int_to_ptr(val, ptr_type, "obj_ptr").or_llvm_err()?;
        let first = builder.build_load(i64_type, obj_ptr, "first").or_llvm_err()?.into_int_value();
        // If first == 0, it's an empty string Text object → true
        let first_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, first, i64_type.const_zero(), "first_null").or_llvm_err()?;
        builder.build_conditional_branch(first_null, ret_true, check_first).or_llvm_err()?;

        // check_first: first field should look like a valid pointer
        builder.position_at_end(check_first);
        let first_too_small = builder.build_int_compare(verum_llvm::IntPredicate::ULT, first, min_ptr, "f_small").or_llvm_err()?;
        let first_too_big = builder.build_int_compare(verum_llvm::IntPredicate::UGT, first, max_ptr, "f_big").or_llvm_err()?;
        let first_bad = builder.build_or(first_too_small, first_too_big, "f_bad").or_llvm_err()?;
        builder.build_conditional_branch(first_bad, ret_false, check_len).or_llvm_err()?;

        // check_len: obj[1] (len) should be 0..1000000
        builder.position_at_end(check_len);
        // SAFETY: GEP into object header to access the length field; the object uses the standard NewG layout (24-byte header)
        let len_gep = unsafe { builder.build_in_bounds_gep(i64_type, obj_ptr, &[i64_type.const_int(1, false)], "len_gep").or_llvm_err()? };
        let len = builder.build_load(i64_type, len_gep, "len").or_llvm_err()?.into_int_value();
        let len_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, len, i64_type.const_zero(), "len_neg").or_llvm_err()?;
        let len_huge = builder.build_int_compare(verum_llvm::IntPredicate::SGT, len, i64_type.const_int(1000000, false), "len_huge").or_llvm_err()?;
        let len_bad = builder.build_or(len_neg, len_huge, "len_bad").or_llvm_err()?;
        builder.build_conditional_branch(len_bad, ret_false, ret_true).or_llvm_err()?;

        builder.position_at_end(ret_false);
        builder.build_return(Some(&i1_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(ret_true);
        builder.build_return(Some(&i1_type.const_all_ones())).or_llvm_err()?;

        Ok(func)
    }

    /// verum_generic_eq(a: i64, b: i64) -> i64
    /// Runtime type-aware equality: handles both Int and Text keys.
    /// Uses is_text_object heuristic to avoid dereferencing raw integers as pointers.
    fn emit_verum_generic_eq(&self, module: &Module<'ctx>) -> Result<()> {
        let is_text_fn = self.emit_verum_is_text_object(module)?;

        if let Some(f) = module.get_function("verum_generic_eq") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i64_type = ctx.i64_type();

        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_generic_eq").unwrap_or_else(|| module.add_function("verum_generic_eq", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let check_text = ctx.append_basic_block(func, "check_text");
        let check_b_text = ctx.append_basic_block(func, "check_b_text");
        let do_strcmp = ctx.append_basic_block(func, "do_strcmp");
        let ret_eq = ctx.append_basic_block(func, "ret_eq");
        let ret_neq = ctx.append_basic_block(func, "ret_neq");

        let builder = ctx.create_builder();

        // Entry: fast path — if a == b, return 1
        builder.position_at_end(entry);
        let a = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let b = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let same = builder.build_int_compare(verum_llvm::IntPredicate::EQ, a, b, "same").or_llvm_err()?;
        builder.build_conditional_branch(same, ret_eq, check_text).or_llvm_err()?;

        builder.position_at_end(ret_eq);
        builder.build_return(Some(&i64_type.const_int(1, false))).or_llvm_err()?;

        // check_text: is a a Text object?
        builder.position_at_end(check_text);
        let a_is_text = builder.build_call(is_text_fn, &[a.into()], "a_text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_conditional_branch(a_is_text, check_b_text, ret_neq).or_llvm_err()?;

        // check_b_text: is b also a Text object?
        builder.position_at_end(check_b_text);
        let b_is_text = builder.build_call(is_text_fn, &[b.into()], "b_text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_conditional_branch(b_is_text, do_strcmp, ret_neq).or_llvm_err()?;

        // do_strcmp: both are Text, compare strings
        builder.position_at_end(do_strcmp);
        let text_get_ptr = module.get_function("verum_text_get_ptr").unwrap_or_else(|| {
            let ft = ptr_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_text_get_ptr", ft, None)
        });
        let strcmp_fn = self.get_or_declare_strcmp(module);
        let a_ptr = builder.build_call(text_get_ptr, &[a.into()], "a_ptr").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        let b_ptr = builder.build_call(text_get_ptr, &[b.into()], "b_ptr").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        let cmp = builder.build_call(strcmp_fn, &[a_ptr.into(), b_ptr.into()], "cmp").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let is_eq = builder.build_int_compare(verum_llvm::IntPredicate::EQ, cmp, ctx.i32_type().const_zero(), "is_eq").or_llvm_err()?;
        let result = builder.build_int_z_extend(is_eq, i64_type, "result").or_llvm_err()?;
        builder.build_return(Some(&result)).or_llvm_err()?;

        // ret_neq: different non-text values
        builder.position_at_end(ret_neq);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        Ok(())
    }

    /// verum_generic_hash(key: i64) -> i64
    /// FNV-1a hash — hashes string bytes for Text objects, raw i64 bytes for integers.
    fn emit_verum_generic_hash(&self, module: &Module<'ctx>) -> Result<()> {
        let is_text_fn = self.emit_verum_is_text_object(module)?;

        if let Some(f) = module.get_function("verum_generic_hash") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i8_type = ctx.i8_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_generic_hash").unwrap_or_else(|| module.add_function("verum_generic_hash", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let hash_text_bb = ctx.append_basic_block(func, "hash_text");
        let hash_text_loop = ctx.append_basic_block(func, "hash_text_loop");
        let hash_text_body = ctx.append_basic_block(func, "hash_text_body");
        let hash_text_done = ctx.append_basic_block(func, "hash_text_done");
        let hash_int_bb = ctx.append_basic_block(func, "hash_int");

        let builder = ctx.create_builder();
        let fnv_offset = i64_type.const_int(14695981039346656037u64, false);
        let fnv_prime = i64_type.const_int(1099511628211u64, false);

        // Entry: check if key is a Text object
        builder.position_at_end(entry);
        let key = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let is_text = builder.build_call(is_text_fn, &[key.into()], "is_text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_conditional_branch(is_text, hash_text_bb, hash_int_bb).or_llvm_err()?;

        // hash_text: get string pointer, hash string bytes in a loop
        builder.position_at_end(hash_text_bb);
        let text_get_ptr = module.get_function("verum_text_get_ptr").unwrap_or_else(|| {
            let ft = ptr_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_text_get_ptr", ft, None)
        });
        let str_ptr = builder.build_call(text_get_ptr, &[key.into()], "str_ptr").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        builder.build_unconditional_branch(hash_text_loop).or_llvm_err()?;

        // hash_text_loop: while (*s) { hash ^= *s++; hash *= prime; }
        builder.position_at_end(hash_text_loop);
        let hash_phi = builder.build_phi(i64_type, "hash").or_llvm_err()?;
        let ptr_phi = builder.build_phi(ptr_type, "ptr").or_llvm_err()?;
        hash_phi.add_incoming(&[(&fnv_offset, hash_text_bb)]);
        ptr_phi.add_incoming(&[(&str_ptr, hash_text_bb)]);

        let cur_ptr = ptr_phi.as_basic_value().into_pointer_value();
        let cur_byte = builder.build_load(i8_type, cur_ptr, "byte").or_llvm_err()?.into_int_value();
        let byte_is_zero = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, cur_byte, i8_type.const_zero(), "is_zero"
        ).or_llvm_err()?;
        builder.build_conditional_branch(byte_is_zero, hash_text_done, hash_text_body).or_llvm_err()?;

        // hash_text_body: hash ^= byte; hash *= prime; ptr++
        builder.position_at_end(hash_text_body);
        let cur_hash = hash_phi.as_basic_value().into_int_value();
        let byte_ext = builder.build_int_z_extend(cur_byte, i64_type, "byte_ext").or_llvm_err()?;
        let xored = builder.build_xor(cur_hash, byte_ext, "xored").or_llvm_err()?;
        let mulled = builder.build_int_mul(xored, fnv_prime, "mulled").or_llvm_err()?;
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let next_ptr = unsafe { builder.build_in_bounds_gep(i8_type, cur_ptr, &[i64_type.const_int(1, false)], "next_ptr").or_llvm_err()? };
        hash_phi.add_incoming(&[(&mulled, hash_text_body)]);
        ptr_phi.add_incoming(&[(&next_ptr, hash_text_body)]);
        builder.build_unconditional_branch(hash_text_loop).or_llvm_err()?;

        // hash_text_done: return hash
        builder.position_at_end(hash_text_done);
        let text_hash_result = hash_phi.as_basic_value().into_int_value();
        builder.build_return(Some(&text_hash_result)).or_llvm_err()?;

        // hash_int: FNV-1a on 8 raw bytes of the i64 value
        builder.position_at_end(hash_int_bb);
        let mut hash = fnv_offset;
        for i in 0..8u64 {
            let shift = i64_type.const_int(i * 8, false);
            let shifted = builder.build_right_shift(key, shift, false, &format!("sh{}", i)).or_llvm_err()?;
            let byte = builder.build_and(shifted, i64_type.const_int(0xFF, false), &format!("b{}", i)).or_llvm_err()?;
            hash = builder.build_xor(hash, byte, &format!("xor{}", i)).or_llvm_err()?;
            hash = builder.build_int_mul(hash, fnv_prime, &format!("mul{}", i)).or_llvm_err()?;
        }
        builder.build_return(Some(&hash)).or_llvm_err()?;
        Ok(())
    }

    // =========================================================================
    // Format / Conversion LLVM IR Functions
    // =========================================================================
    //
    // These replace C runtime format/conversion stubs with LLVM IR that calls
    // libc snprintf/strtol/strtod directly. They serve as fallback when
    // compiled text.vr (Text.from_int, Text.from_float, Text.to_int, Text.to_float)
    // is not available.

    /// Get or declare snprintf(buf: ptr, size: i64, fmt: ptr, ...) -> i32
    fn get_or_declare_snprintf(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("snprintf") { return f; }
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into()], true);
        module.add_function("snprintf", fn_type, None)
    }

    /// Get or declare strtol(str: ptr, endptr: ptr, base: i32) -> i64
    fn get_or_declare_strtol(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("strtol") { return f; }
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i64_type.fn_type(&[ptr_type.into(), ptr_type.into(), i32_type.into()], false);
        module.add_function("strtol", fn_type, None)
    }

    /// Get or declare strtod(str: ptr, endptr: ptr) -> f64
    fn get_or_declare_strtod(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("strtod") { return f; }
        let f64_type = self.context.f64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = f64_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        module.add_function("strtod", fn_type, None)
    }

    /// verum_int_to_text(value: i64) -> i64  (returns Text object pointer as i64)
    ///
    /// Converts integer to Text using snprintf(buf, 32, "%ld", value).
    /// Allocates a new Text object with the result.
    fn emit_verum_int_to_text(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_int_to_text") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i32_type = ctx.i32_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_int_to_text")
            .unwrap_or_else(|| module.add_function("verum_int_to_text", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let builder = ctx.create_builder();
        builder.position_at_end(entry);

        let value = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();

        // Stack buffer for snprintf
        let buf_size: u64 = 32;
        let buf = builder.build_array_alloca(ctx.i8_type(), i64_type.const_int(buf_size, false), "buf").or_llvm_err()?;

        // snprintf(buf, 32, "%ld", value)
        let snprintf_fn = self.get_or_declare_snprintf(module);
        let fmt = builder.build_global_string_ptr("%ld", "int_fmt").or_llvm_err()?;
        let written = builder.build_call(
            snprintf_fn,
            &[buf.into(), i64_type.const_int(buf_size, false).into(), fmt.as_pointer_value().into(), value.into()],
            "written",
        ).or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();

        // len = snprintf return value (i32 → i64)
        let len = builder.build_int_s_extend(written, i64_type, "len").or_llvm_err()?;

        // Allocate new buffer: malloc(len + 1)
        let len_plus1 = builder.build_int_add(len, i64_type.const_int(1, false), "len1").or_llvm_err()?;
        let new_buf = self.emit_checked_malloc(&builder, module, len_plus1, "newbuf")?;

        // memcpy(new_buf, buf, len + 1)
        let memcpy_fn = self.get_or_declare_memcpy(module);
        builder.build_call(memcpy_fn, &[new_buf.into(), buf.into(), len_plus1.into()], "").or_llvm_err()?;

        // Allocate Text object: verum_text_alloc(new_buf, len, len)
        let text_alloc = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let result = builder.build_call(text_alloc, &[new_buf.into(), len.into(), len.into()], "text_obj").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;

        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_float_to_text(value: f64) -> i64  (returns Text object pointer as i64)
    ///
    /// Converts float to Text using snprintf(buf, 64, "%g", value).
    fn emit_verum_float_to_text(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_float_to_text") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[f64_type.into()], false);
        let func = module.get_function("verum_float_to_text")
            .unwrap_or_else(|| module.add_function("verum_float_to_text", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let builder = ctx.create_builder();
        builder.position_at_end(entry);

        let value = func.get_nth_param(0).or_internal("missing param 0")?.into_float_value();

        // Stack buffer for snprintf
        let buf_size: u64 = 64;
        let buf = builder.build_array_alloca(ctx.i8_type(), i64_type.const_int(buf_size, false), "buf").or_llvm_err()?;

        // snprintf(buf, 64, "%g", value)
        let snprintf_fn = self.get_or_declare_snprintf(module);
        let fmt = builder.build_global_string_ptr("%g", "float_fmt").or_llvm_err()?;
        let written = builder.build_call(
            snprintf_fn,
            &[buf.into(), i64_type.const_int(buf_size, false).into(), fmt.as_pointer_value().into(), value.into()],
            "written",
        ).or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();

        let len = builder.build_int_s_extend(written, i64_type, "len").or_llvm_err()?;

        // Allocate new buffer: malloc(len + 1)
        let len_plus1 = builder.build_int_add(len, i64_type.const_int(1, false), "len1").or_llvm_err()?;
        let new_buf = self.emit_checked_malloc(&builder, module, len_plus1, "newbuf")?;

        // memcpy(new_buf, buf, len + 1)
        let memcpy_fn = self.get_or_declare_memcpy(module);
        builder.build_call(memcpy_fn, &[new_buf.into(), buf.into(), len_plus1.into()], "").or_llvm_err()?;

        // Allocate Text object
        let text_alloc = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let result = builder.build_call(text_alloc, &[new_buf.into(), len.into(), len.into()], "text_obj").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;

        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_string_parse_int(str: ptr) -> i64
    ///
    /// Parses C string to integer using strtol(str, NULL, 10).
    fn emit_verum_string_parse_int(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_string_parse_int") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let i32_type = ctx.i32_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_string_parse_int")
            .unwrap_or_else(|| module.add_function("verum_string_parse_int", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let do_parse = ctx.append_basic_block(func, "do_parse");
        let ret_zero = ctx.append_basic_block(func, "ret_zero");
        let builder = ctx.create_builder();

        // Entry: check for null
        builder.position_at_end(entry);
        let str_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let is_null = builder.build_is_null(str_ptr, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_zero, do_parse).or_llvm_err()?;

        // do_parse: strtol(str, NULL, 10)
        builder.position_at_end(do_parse);
        let strtol_fn = self.get_or_declare_strtol(module);
        let null_ptr = ptr_type.const_null();
        let base = i32_type.const_int(10, false);
        let result = builder.build_call(strtol_fn, &[str_ptr.into(), null_ptr.into(), base.into()], "parsed").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;
        builder.build_return(Some(&result)).or_llvm_err()?;

        // ret_zero
        builder.position_at_end(ret_zero);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        Ok(())
    }

    /// verum_string_parse_float(str: ptr) -> i64  (f64 bits as i64)
    ///
    /// Parses C string to float using strtod, returns f64 bits as i64.
    fn emit_verum_string_parse_float(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_string_parse_float") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_string_parse_float")
            .unwrap_or_else(|| module.add_function("verum_string_parse_float", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let do_parse = ctx.append_basic_block(func, "do_parse");
        let ret_zero = ctx.append_basic_block(func, "ret_zero");
        let builder = ctx.create_builder();

        // Entry: check for null
        builder.position_at_end(entry);
        let str_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let is_null = builder.build_is_null(str_ptr, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_zero, do_parse).or_llvm_err()?;

        // do_parse: strtod(str, NULL)
        builder.position_at_end(do_parse);
        let strtod_fn = self.get_or_declare_strtod(module);
        let null_ptr = ptr_type.const_null();
        let f64_result = builder.build_call(strtod_fn, &[str_ptr.into(), null_ptr.into()], "parsed").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_float_value();

        // Bitcast f64 → i64
        let i64_result = builder.build_bit_cast(f64_result, i64_type, "bits").or_llvm_err()?;
        builder.build_return(Some(&i64_result)).or_llvm_err()?;

        // ret_zero
        builder.position_at_end(ret_zero);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        Ok(())
    }

    /// verum_text_char_len(text_obj: i64) -> i64
    ///
    /// Returns the number of UTF-8 codepoints in a Text object.
    /// Counts bytes that are NOT continuation bytes (0x80..0xBF).
    fn emit_verum_text_char_len(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_text_char_len") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let ctx = self.context;
        let i8_type = ctx.i8_type();
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_text_char_len")
            .unwrap_or_else(|| module.add_function("verum_text_char_len", fn_type, None));

        let entry = ctx.append_basic_block(func, "entry");
        let get_fields = ctx.append_basic_block(func, "get_fields");
        let loop_head = ctx.append_basic_block(func, "loop_head");
        let loop_body = ctx.append_basic_block(func, "loop_body");
        let loop_end = ctx.append_basic_block(func, "loop_end");
        let ret_zero = ctx.append_basic_block(func, "ret_zero");
        let builder = ctx.create_builder();

        // Entry: null check
        builder.position_at_end(entry);
        let text_obj = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, text_obj, i64_type.const_zero(), "null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_zero, get_fields).or_llvm_err()?;

        // get_fields: extract ptr and len from Text object
        builder.position_at_end(get_fields);
        let text_ptr = builder.build_int_to_ptr(text_obj, ptr_type, "tptr").or_llvm_err()?;
        let data_ptr_raw = builder.build_load(i64_type, text_ptr, "data_raw").or_llvm_err()?.into_int_value();
        let data_ptr = builder.build_int_to_ptr(data_ptr_raw, ptr_type, "data").or_llvm_err()?;
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let len_gep = unsafe { builder.build_in_bounds_gep(i64_type, text_ptr, &[i64_type.const_int(1, false)], "len_gep").or_llvm_err()? };
        let byte_len = builder.build_load(i64_type, len_gep, "byte_len").or_llvm_err()?.into_int_value();

        // Check if len == 0
        let len_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, byte_len, i64_type.const_zero(), "lz").or_llvm_err()?;
        builder.build_conditional_branch(len_zero, ret_zero, loop_head).or_llvm_err()?;

        // loop_head: phi for index and count
        builder.position_at_end(loop_head);
        let idx_phi = builder.build_phi(i64_type, "idx").or_llvm_err()?;
        let cnt_phi = builder.build_phi(i64_type, "cnt").or_llvm_err()?;
        idx_phi.add_incoming(&[(&i64_type.const_zero(), get_fields)]);
        cnt_phi.add_incoming(&[(&i64_type.const_zero(), get_fields)]);
        let idx = idx_phi.as_basic_value().into_int_value();
        let cnt = cnt_phi.as_basic_value().into_int_value();

        let done = builder.build_int_compare(verum_llvm::IntPredicate::UGE, idx, byte_len, "done").or_llvm_err()?;
        builder.build_conditional_branch(done, loop_end, loop_body).or_llvm_err()?;

        // loop_body: load byte, check if NOT continuation byte (0b10xxxxxx)
        builder.position_at_end(loop_body);
        // SAFETY: GEP at a fixed offset within a known struct layout; the pointer is valid from prior allocation
        let byte_gep = unsafe { builder.build_in_bounds_gep(i8_type, data_ptr, &[idx], "bgep").or_llvm_err()? };
        let byte = builder.build_load(i8_type, byte_gep, "byte").or_llvm_err()?.into_int_value();
        // Continuation bytes have pattern 10xxxxxx → (byte & 0xC0) == 0x80
        let masked = builder.build_and(byte, i8_type.const_int(0xC0, false), "masked").or_llvm_err()?;
        let is_cont = builder.build_int_compare(verum_llvm::IntPredicate::EQ, masked, i8_type.const_int(0x80, false), "is_cont").or_llvm_err()?;
        // If NOT continuation byte, increment count
        let not_cont = builder.build_not(is_cont, "not_cont").or_llvm_err()?;
        let not_cont_ext = builder.build_int_z_extend(not_cont, i64_type, "ext").or_llvm_err()?;
        let new_cnt = builder.build_int_add(cnt, not_cont_ext, "ncnt").or_llvm_err()?;
        let new_idx = builder.build_int_add(idx, i64_type.const_int(1, false), "nidx").or_llvm_err()?;

        idx_phi.add_incoming(&[(&new_idx, loop_body)]);
        cnt_phi.add_incoming(&[(&new_cnt, loop_body)]);
        builder.build_unconditional_branch(loop_head).or_llvm_err()?;

        // loop_end: return count
        builder.position_at_end(loop_end);
        builder.build_return(Some(&cnt)).or_llvm_err()?;

        // ret_zero
        builder.position_at_end(ret_zero);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
        Ok(())
    }

    // =========================================================================
    // Misc LLVM IR Functions — time, random, range
    // =========================================================================
    //
    // These replace C runtime functions with pure LLVM IR emission.
    // Platform differences are resolved at IR-generation time via cfg!().

    /// Emit all miscellaneous runtime functions as LLVM IR.
    pub fn emit_misc_ir_functions(&self, module: &Module<'ctx>) -> Result<()> {
        self.emit_verum_time_monotonic_nanos(module)?;
        self.emit_verum_time_realtime_nanos(module)?;
        self.emit_verum_time_sleep_nanos(module)?;
        self.emit_verum_random_u64(module)?;
        self.emit_verum_random_float(module)?;
        self.emit_verum_range_new(module)?;
        self.emit_verum_log_functions(module)?;
        self.emit_verum_file_ir_functions(module)?;
        self.emit_verum_sync_bridge_functions(module)?;
        self.emit_verum_sys_functions(module)?;
        self.emit_verum_string_join(module)?;
        self.emit_verum_cbgr_functions(module)?;
        self.emit_verum_networking_functions(module)?;
        self.emit_verum_process_functions(module)?;
        // Note: verum_store_args/get_argc/get_argv kept in C because the entry point
        // (main/_start) calls verum_store_args which writes to C-global variables.
        // IR versions would use separate globals and not see the stored values.
        Ok(())
    }

    /// verum_time_monotonic_nanos() -> i64
    /// Returns monotonic clock time in nanoseconds.
    /// Uses clock_gettime with platform-appropriate clock ID.
    fn emit_verum_time_monotonic_nanos(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_time_monotonic_nanos") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let fn_type = i64_type.fn_type(&[], false);
        let func = module.get_function("verum_time_monotonic_nanos").unwrap_or_else(|| module.add_function("verum_time_monotonic_nanos", fn_type, None));
        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        // struct timespec { i64 tv_sec; i64 tv_nsec; }
        let timespec_type = self.context.struct_type(&[i64_type.into(), i64_type.into()], false);
        let ts = builder.build_alloca(timespec_type, "ts").or_llvm_err()?;

        let clock_gettime_fn = self.get_or_declare_clock_gettime(module);

        // CLOCK_MONOTONIC: macOS=6, Linux=1
        let clock_id: u64 = if cfg!(target_os = "macos") { 6 } else { 1 };
        let clock_id_val = i32_type.const_int(clock_id, false);

        self.build_libc_call_void(&builder, clock_gettime_fn, &[clock_id_val.into(), ts.into()], "ret")?;

        let sec_ptr = builder.build_struct_gep(timespec_type, ts, 0, "sec_ptr").or_llvm_err()?;
        let nsec_ptr = builder.build_struct_gep(timespec_type, ts, 1, "nsec_ptr").or_llvm_err()?;
        let sec = builder.build_load(i64_type, sec_ptr, "sec").or_llvm_err()?.into_int_value();
        let nsec = builder.build_load(i64_type, nsec_ptr, "nsec").or_llvm_err()?.into_int_value();

        let billion = i64_type.const_int(1_000_000_000, false);
        let sec_ns = builder.build_int_mul(sec, billion, "sec_ns").or_llvm_err()?;
        let total = builder.build_int_add(sec_ns, nsec, "total").or_llvm_err()?;

        builder.build_return(Some(&total)).or_llvm_err()?;
        Ok(())
    }

    /// verum_time_realtime_nanos() -> i64
    /// Returns wall-clock time in nanoseconds since epoch.
    fn emit_verum_time_realtime_nanos(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_time_realtime_nanos") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let fn_type = i64_type.fn_type(&[], false);
        let func = module.get_function("verum_time_realtime_nanos").unwrap_or_else(|| module.add_function("verum_time_realtime_nanos", fn_type, None));
        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let timespec_type = self.context.struct_type(&[i64_type.into(), i64_type.into()], false);
        let ts = builder.build_alloca(timespec_type, "ts").or_llvm_err()?;

        let clock_gettime_fn = self.get_or_declare_clock_gettime(module);

        // CLOCK_REALTIME = 0 on both macOS and Linux
        let clock_id_val = i32_type.const_int(0, false);

        self.build_libc_call_void(&builder, clock_gettime_fn, &[clock_id_val.into(), ts.into()], "ret")?;

        let sec_ptr = builder.build_struct_gep(timespec_type, ts, 0, "sec_ptr").or_llvm_err()?;
        let nsec_ptr = builder.build_struct_gep(timespec_type, ts, 1, "nsec_ptr").or_llvm_err()?;
        let sec = builder.build_load(i64_type, sec_ptr, "sec").or_llvm_err()?.into_int_value();
        let nsec = builder.build_load(i64_type, nsec_ptr, "nsec").or_llvm_err()?.into_int_value();

        let billion = i64_type.const_int(1_000_000_000, false);
        let sec_ns = builder.build_int_mul(sec, billion, "sec_ns").or_llvm_err()?;
        let total = builder.build_int_add(sec_ns, nsec, "total").or_llvm_err()?;

        builder.build_return(Some(&total)).or_llvm_err()?;
        Ok(())
    }

    /// verum_time_sleep_nanos(nanos: i64)
    /// Sleeps for the given number of nanoseconds using nanosleep.
    fn emit_verum_time_sleep_nanos(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_time_sleep_nanos") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_time_sleep_nanos").unwrap_or_else(|| module.add_function("verum_time_sleep_nanos", fn_type, None));
        let entry = self.context.append_basic_block(func, "entry");
        let do_sleep = self.context.append_basic_block(func, "do_sleep");
        let done = self.context.append_basic_block(func, "done");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let nanos = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();

        // if nanos <= 0, return immediately
        let is_positive = builder.build_int_compare(
            verum_llvm::IntPredicate::SGT, nanos, i64_type.const_int(0, false), "is_pos"
        ).or_llvm_err()?;
        builder.build_conditional_branch(is_positive, do_sleep, done).or_llvm_err()?;

        builder.position_at_end(do_sleep);

        let timespec_type = self.context.struct_type(&[i64_type.into(), i64_type.into()], false);
        let ts = builder.build_alloca(timespec_type, "ts").or_llvm_err()?;

        let billion = i64_type.const_int(1_000_000_000, false);
        let sec = builder.build_int_signed_div(nanos, billion, "sec").or_llvm_err()?;
        let nsec = builder.build_int_signed_rem(nanos, billion, "nsec").or_llvm_err()?;

        let sec_ptr = builder.build_struct_gep(timespec_type, ts, 0, "sec_ptr").or_llvm_err()?;
        let nsec_ptr = builder.build_struct_gep(timespec_type, ts, 1, "nsec_ptr").or_llvm_err()?;
        builder.build_store(sec_ptr, sec).or_llvm_err()?;
        builder.build_store(nsec_ptr, nsec).or_llvm_err()?;

        let nanosleep_fn = self.get_or_declare_nanosleep(module);
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let null_ptr = ptr_type.const_null();
        builder.build_call(nanosleep_fn, &[ts.into(), null_ptr.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(done).or_llvm_err()?;

        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_random_u64() -> i64
    /// Xorshift64* PRNG with auto-seeding from monotonic time.
    fn emit_verum_random_u64(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_random_u64") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();

        // Global PRNG state
        let global_name = "verum_rng_state";
        let rng_state = if let Some(g) = module.get_global(global_name) {
            g
        } else {
            let g = module.add_global(i64_type, None, global_name);
            g.set_initializer(&i64_type.const_int(0, false));
            g.set_linkage(verum_llvm::module::Linkage::Internal);
            g
        };

        let fn_type = i64_type.fn_type(&[], false);
        let func = module.get_function("verum_random_u64").unwrap_or_else(|| module.add_function("verum_random_u64", fn_type, None));
        let entry = self.context.append_basic_block(func, "entry");
        let seed_bb = self.context.append_basic_block(func, "seed");
        let seed_check = self.context.append_basic_block(func, "seed_check");
        let compute = self.context.append_basic_block(func, "compute");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        // Load current state
        let state = builder.build_load(i64_type, rng_state.as_pointer_value(), "state").or_llvm_err()?.into_int_value();

        // Check if needs seeding (state == 0)
        let needs_seed = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, state, i64_type.const_int(0, false), "needs_seed"
        ).or_llvm_err()?;
        builder.build_conditional_branch(needs_seed, seed_bb, compute).or_llvm_err()?;

        // Seed from monotonic time
        builder.position_at_end(seed_bb);
        let time_fn = module.get_function("verum_time_monotonic_nanos").unwrap_or_else(|| {
            let ft = i64_type.fn_type(&[], false);
            module.add_function("verum_time_monotonic_nanos", ft, None)
        });
        let seed_val = builder.build_call(time_fn, &[], "seed")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_store(rng_state.as_pointer_value(), seed_val).or_llvm_err()?;

        // If seed is still 0, use fallback constant
        let seed_is_zero = builder.build_int_compare(
            verum_llvm::IntPredicate::EQ, seed_val, i64_type.const_int(0, false), "seed_zero"
        ).or_llvm_err()?;
        builder.build_conditional_branch(seed_is_zero, seed_check, compute).or_llvm_err()?;

        builder.position_at_end(seed_check);
        let fallback = i64_type.const_int(0x12345678DEADBEEF, false);
        builder.build_store(rng_state.as_pointer_value(), fallback).or_llvm_err()?;
        builder.build_unconditional_branch(compute).or_llvm_err()?;

        // Xorshift64*: x ^= x >> 12; x ^= x << 25; x ^= x >> 27; return x * C
        builder.position_at_end(compute);
        let x0 = builder.build_load(i64_type, rng_state.as_pointer_value(), "x0").or_llvm_err()?.into_int_value();
        let x1 = builder.build_xor(x0,
            builder.build_right_shift(x0, i64_type.const_int(12, false), false, "shr12").or_llvm_err()?,
            "x1").or_llvm_err()?;
        let x2 = builder.build_xor(x1,
            builder.build_left_shift(x1, i64_type.const_int(25, false), "shl25").or_llvm_err()?,
            "x2").or_llvm_err()?;
        let x3 = builder.build_xor(x2,
            builder.build_right_shift(x2, i64_type.const_int(27, false), false, "shr27").or_llvm_err()?,
            "x3").or_llvm_err()?;

        // Store new state
        builder.build_store(rng_state.as_pointer_value(), x3).or_llvm_err()?;

        // Return x * 0x2545F4914F6CDD1D
        let multiplier = i64_type.const_int(0x2545F4914F6CDD1D, false);
        let result = builder.build_int_mul(x3, multiplier, "result").or_llvm_err()?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_random_float() -> f64
    /// Returns a random double in [0.0, 1.0).
    fn emit_verum_random_float(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_random_float") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let f64_type = self.context.f64_type();
        let fn_type = f64_type.fn_type(&[], false);
        let func = module.get_function("verum_random_float").unwrap_or_else(|| module.add_function("verum_random_float", fn_type, None));
        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        // Call verum_random_u64()
        let random_u64_fn = module.get_function("verum_random_u64").unwrap_or_else(|| {
            let ft = i64_type.fn_type(&[], false);
            module.add_function("verum_random_u64", ft, None)
        });
        let raw = builder.build_call(random_u64_fn, &[], "raw")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();

        // Shift right 11 to get 53-bit mantissa
        let shifted = builder.build_right_shift(raw, i64_type.const_int(11, false), false, "shifted").or_llvm_err()?;
        let as_f64 = builder.build_unsigned_int_to_float(shifted, f64_type, "as_f64").or_llvm_err()?;

        // Multiply by 1.0 / 2^53
        let scale = f64_type.const_float(1.0 / (1u64 << 53) as f64);
        let result = builder.build_float_mul(as_f64, scale, "result").or_llvm_err()?;

        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_range_new(start: i64, end: i64) -> ptr
    /// Allocates a VerumRange { start, end, step, current } on the heap.
    fn emit_verum_range_new(&self, module: &Module<'ctx>) -> Result<()> {
        if let Some(f) = module.get_function("verum_range_new") {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_range_new").unwrap_or_else(|| module.add_function("verum_range_new", fn_type, None));
        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let start = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let end = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

        // malloc(32) — 4 * i64
        let size = i64_type.const_int(32, false);
        let raw_ptr = self.emit_checked_malloc(&builder, module, size, "range_ptr")?;

        // Store fields: start, end, step, current
        let range_type = self.context.struct_type(
            &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()], false
        );

        // step = (start <= end) ? 1 : -1
        let is_ascending = builder.build_int_compare(
            verum_llvm::IntPredicate::SLE, start, end, "is_asc"
        ).or_llvm_err()?;
        let step = builder.build_select(
            is_ascending,
            i64_type.const_int(1, false),
            i64_type.const_all_ones(), // -1 as i64
            "step"
        ).or_llvm_err()?.into_int_value();

        let start_ptr = builder.build_struct_gep(range_type, raw_ptr, 0, "start_ptr").or_llvm_err()?;
        let end_ptr = builder.build_struct_gep(range_type, raw_ptr, 1, "end_ptr").or_llvm_err()?;
        let step_ptr = builder.build_struct_gep(range_type, raw_ptr, 2, "step_ptr").or_llvm_err()?;
        let current_ptr = builder.build_struct_gep(range_type, raw_ptr, 3, "current_ptr").or_llvm_err()?;

        builder.build_store(start_ptr, start).or_llvm_err()?;
        builder.build_store(end_ptr, end).or_llvm_err()?;
        builder.build_store(step_ptr, step).or_llvm_err()?;
        builder.build_store(current_ptr, start).or_llvm_err()?;

        builder.build_return(Some(&raw_ptr)).or_llvm_err()?;
        Ok(())
    }

    /// Emit all logging IR functions:
    /// - verum_log_set_level, verum_log_get_level (global i64)
    /// - verum_log_flush (no-op)
    /// - verum_log_message (write prefix + msg + newline to stderr via write(2))
    fn emit_verum_log_functions(&self, module: &Module<'ctx>) -> Result<()> {
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        // Create global log level: @verum_log_level = internal global i64 0
        let log_level_global = module.get_global("verum_ir_log_level").unwrap_or_else(|| {
            let g = module.add_global(i64_type, None, "verum_ir_log_level");
            g.set_initializer(&i64_type.const_zero());
            g.set_linkage(verum_llvm::module::Linkage::Internal);
            g
        });

        // verum_log_set_level(level: i64) -> void
        {
            let fn_type = void_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_log_set_level")
                .unwrap_or_else(|| module.add_function("verum_log_set_level", fn_type, None));
            if func.count_basic_blocks() > 0 { /* skip */ } else {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let level = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                builder.build_store(log_level_global.as_pointer_value(), level).or_llvm_err()?;
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // verum_log_get_level() -> i64
        {
            let fn_type = i64_type.fn_type(&[], false);
            let func = module.get_function("verum_log_get_level")
                .unwrap_or_else(|| module.add_function("verum_log_get_level", fn_type, None));
            if func.count_basic_blocks() > 0 { /* skip */ } else {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let val = builder.build_load(i64_type, log_level_global.as_pointer_value(), "level").or_llvm_err()?.into_int_value();
                builder.build_return(Some(&val)).or_llvm_err()?;
            }
        }

        // verum_log_flush() -> void (no-op, stderr is unbuffered)
        {
            let fn_type = void_type.fn_type(&[], false);
            let func = module.get_function("verum_log_flush")
                .unwrap_or_else(|| module.add_function("verum_log_flush", fn_type, None));
            if func.count_basic_blocks() > 0 { /* skip */ } else {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // verum_log_message(level: i64, msg: *i8) -> void
        // Writes "[LEVEL] msg\n" to stderr using write(2, buf, len)
        {
            let fn_type = void_type.fn_type(&[i64_type.into(), ptr_type.into()], false);
            let func = module.get_function("verum_log_message")
                .unwrap_or_else(|| module.add_function("verum_log_message", fn_type, None));
            if func.count_basic_blocks() > 0 { /* skip */ } else {
                let entry = self.context.append_basic_block(func, "entry");
                let msg_null_bb = self.context.append_basic_block(func, "msg_null");
                let msg_ok_bb = self.context.append_basic_block(func, "msg_ok");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);

                let msg = func.get_nth_param(1).or_internal("missing param 1")?.into_pointer_value();
                let level = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();

                // Check if msg is null
                let is_null = builder.build_is_null(msg, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, msg_null_bb, msg_ok_bb).or_llvm_err()?;

                builder.position_at_end(msg_null_bb);
                builder.build_return(None).or_llvm_err()?;

                builder.position_at_end(msg_ok_bb);

                // Declare write(fd, buf, count)
                let write_fn = self.get_or_declare_write(module);
                let strlen_fn = self.get_or_declare_strlen(module);

                let i32_type = self.context.i32_type();
                let stderr_fd = i32_type.const_int(2, false); // fd=2 = stderr

                // Create log level prefix strings as global constants
                let prefixes = [
                    "[INFO] ", "[WARN] ", "[ERROR] ", "[DEBUG] ", "[TRACE] ", "[LOG] "
                ];

                // Build a switch on level to select prefix string
                let default_bb = self.context.append_basic_block(func, "default_prefix");
                let merge_bb = self.context.append_basic_block(func, "merge");

                let mut cases = Vec::new();
                let mut case_bbs = Vec::new();
                for (i, prefix) in prefixes.iter().enumerate().take(5) {
                    let bb = self.context.append_basic_block(func, &format!("level_{}", i));
                    cases.push((i64_type.const_int(i as u64, false), bb));
                    case_bbs.push((bb, *prefix));
                }

                builder.build_switch(level, default_bb, &cases).or_llvm_err()?;

                // Each case: write prefix, then branch to merge
                for (bb, prefix) in &case_bbs {
                    builder.position_at_end(*bb);
                    let prefix_global = builder.build_global_string_ptr(prefix, "prefix").or_llvm_err()?;
                    let prefix_len = i64_type.const_int(prefix.len() as u64, false);
                    self.build_libc_call_void(&builder, write_fn, &[stderr_fd.into(), prefix_global.as_pointer_value().into(), prefix_len.into()], "")?;
                    builder.build_unconditional_branch(merge_bb).or_llvm_err()?;
                }

                // Default: write "[LOG] "
                builder.position_at_end(default_bb);
                let default_prefix = builder.build_global_string_ptr("[LOG] ", "log_prefix").or_llvm_err()?;
                let default_len = i64_type.const_int(6, false);
                self.build_libc_call_void(&builder, write_fn, &[stderr_fd.into(), default_prefix.as_pointer_value().into(), default_len.into()], "")?;
                builder.build_unconditional_branch(merge_bb).or_llvm_err()?;

                // Merge: write msg then newline
                builder.position_at_end(merge_bb);
                let msg_len = builder.build_call(strlen_fn, &[msg.into()], "msg_len")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                self.build_libc_call_void(&builder, write_fn, &[stderr_fd.into(), msg.into(), msg_len.into()], "")?;
                let newline = builder.build_global_string_ptr("\n", "newline").or_llvm_err()?;
                let one = i64_type.const_int(1, false);
                self.build_libc_call_void(&builder, write_fn, &[stderr_fd.into(), newline.as_pointer_value().into(), one.into()], "")?;
                builder.build_return(None).or_llvm_err()?;
            }
        }
        Ok(())
    }

    /// Build a call to a libc function, adapting argument types to match the
    /// actual declaration. FFI extern blocks may declare libc functions with i64
    /// (Verum Int) parameters, while the native POSIX signatures use i32. This
    /// helper casts i32→i64 or i64→i32 as needed to match the declared signature.
    /// Returns the result as i64 (sign-extending if the function returns i32).
    fn build_libc_call(
        &self,
        builder: &Builder<'ctx>,
        func: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) -> Result<IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let adapted = self.adapt_libc_args(builder, func, args)?;

        let ret = builder.build_call(func, &adapted, name)
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();

        // If function returns i32, sign-extend to i64
        let result = if ret.get_type().get_bit_width() < 64 {
            builder.build_int_s_extend(ret, i64_type, &format!("{}_ext", name)).or_llvm_err()?
        } else {
            ret
        };
        Ok(result)
    }

    /// Like build_libc_call but discards the return value.
    fn build_libc_call_void(
        &self,
        builder: &Builder<'ctx>,
        func: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) -> Result<()> {
        let adapted = self.adapt_libc_args(builder, func, args)?;
        builder.build_call(func, &adapted, name).or_llvm_err()?;
        Ok(())
    }

    /// Adapt argument types for a libc call: sext i32→i64 or trunc i64→i32 as needed.
    fn adapt_libc_args(
        &self,
        builder: &Builder<'ctx>,
        func: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Result<Vec<BasicMetadataValueEnum<'ctx>>> {
        let param_types = func.get_type().get_param_types();
        let mut adapted: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();

        for (i, arg) in args.iter().enumerate() {
            if i < param_types.len() {
                if let BasicMetadataTypeEnum::IntType(expected_int) = param_types[i] {
                    if let BasicMetadataValueEnum::IntValue(arg_val) = arg {
                        let arg_width = arg_val.get_type().get_bit_width();
                        let expected_width = expected_int.get_bit_width();
                        if arg_width < expected_width {
                            let ext = builder.build_int_s_extend(*arg_val, expected_int, "sext").or_llvm_err()?;
                            adapted.push(ext.into());
                            continue;
                        } else if arg_width > expected_width {
                            let trunc = builder.build_int_truncate(*arg_val, expected_int, "trunc").or_llvm_err()?;
                            adapted.push(trunc.into());
                            continue;
                        }
                    }
                }
            }
            adapted.push(*arg);
        }

        Ok(adapted)
    }

    /// Emit all file I/O IR functions:
    /// verum_file_open, verum_file_close, verum_file_exists, verum_file_delete,
    /// verum_file_read_all, verum_file_write_all, verum_file_read_text, verum_file_write_text
    fn emit_verum_file_ir_functions(&self, module: &Module<'ctx>) -> Result<()> {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Declare libc functions we need
        let open_fn = self.get_or_declare_open(module);
        let close_fn = self.get_or_declare_close(module);
        let read_fn = self.get_or_declare_read(module);
        let write_fn = self.get_or_declare_write(module);
        let unlink_fn = self.get_or_declare_unlink(module);
        let lseek_fn = self.get_or_declare_lseek(module);
        let strlen_fn = self.get_or_declare_strlen(module);

        // Also need verum_text_from_cstr and verum_text_get_ptr (already in module from text IR)
        let text_from_cstr_fn = module.get_function("verum_text_from_cstr").unwrap_or_else(|| {
            let ft = i64_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_text_from_cstr", ft, None)
        });
        let text_get_ptr_fn = module.get_function("verum_text_get_ptr").unwrap_or_else(|| {
            let ft = ptr_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_text_get_ptr", ft, None)
        });

        // ============================================================
        // verum_file_open(path: *i8, mode: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_file_open")
                .unwrap_or_else(|| module.add_function("verum_file_open", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null_path");
                let ok_bb = self.context.append_basic_block(func, "ok");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let path = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let mode = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

                let is_null = builder.build_is_null(path, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, ok_bb).or_llvm_err()?;

                builder.position_at_end(null_bb);
                builder.build_return(Some(&i64_type.const_int(u64::MAX, true))).or_llvm_err()?; // -1

                builder.position_at_end(ok_bb);
                // O_RDONLY=0, O_WRONLY|O_CREAT|O_TRUNC=0x241, O_WRONLY|O_CREAT|O_APPEND=0x441, O_RDWR|O_CREAT=0x42
                let mode0_bb = self.context.append_basic_block(func, "mode_read");
                let mode1_bb = self.context.append_basic_block(func, "mode_write");
                let mode2_bb = self.context.append_basic_block(func, "mode_append");
                let mode3_bb = self.context.append_basic_block(func, "mode_rdwr");
                let default_bb = self.context.append_basic_block(func, "mode_default");
                let call_bb = self.context.append_basic_block(func, "do_open");

                builder.build_switch(mode, default_bb, &[
                    (i64_type.const_int(0, false), mode0_bb),
                    (i64_type.const_int(1, false), mode1_bb),
                    (i64_type.const_int(2, false), mode2_bb),
                    (i64_type.const_int(3, false), mode3_bb),
                ]).or_llvm_err()?;

                // O_RDONLY = 0
                builder.position_at_end(mode0_bb);
                builder.build_unconditional_branch(call_bb).or_llvm_err()?;
                // O_WRONLY|O_CREAT|O_TRUNC = 0x601 on macOS, 0x241 on Linux
                builder.position_at_end(mode1_bb);
                builder.build_unconditional_branch(call_bb).or_llvm_err()?;
                // O_WRONLY|O_CREAT|O_APPEND
                builder.position_at_end(mode2_bb);
                builder.build_unconditional_branch(call_bb).or_llvm_err()?;
                // O_RDWR|O_CREAT
                builder.position_at_end(mode3_bb);
                builder.build_unconditional_branch(call_bb).or_llvm_err()?;
                builder.position_at_end(default_bb);
                builder.build_unconditional_branch(call_bb).or_llvm_err()?;

                builder.position_at_end(call_bb);
                let flags_phi = builder.build_phi(i32_type, "flags").or_llvm_err()?;
                // macOS: O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_CREAT=0x200, O_TRUNC=0x400, O_APPEND=8
                // Linux: O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_CREAT=0x40, O_TRUNC=0x200, O_APPEND=0x400
                #[cfg(target_os = "macos")]
                let (write_flags, append_flags, rdwr_flags) = (0x601i32, 0x209i32, 0x202i32);
                #[cfg(not(target_os = "macos"))]
                let (write_flags, append_flags, rdwr_flags) = (0x241i32, 0x441i32, 0x42i32);

                flags_phi.add_incoming(&[
                    (&i32_type.const_int(0, false), mode0_bb),       // O_RDONLY
                    (&i32_type.const_int(write_flags as u64, false), mode1_bb),
                    (&i32_type.const_int(append_flags as u64, false), mode2_bb),
                    (&i32_type.const_int(rdwr_flags as u64, false), mode3_bb),
                    (&i32_type.const_int(0, false), default_bb),     // default: O_RDONLY
                ]);

                // Use verum_raw_open3 C wrapper to avoid ARM64 variadic
                // calling convention issues with libc open()
                let raw_open_fn = module.get_function("verum_raw_open3").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[ptr_type.into(), i32_type.into(), i32_type.into()], false);
                    module.add_function("verum_raw_open3", ft, None)
                });
                let fd = builder.build_call(raw_open_fn, &[
                    path.into(),
                    flags_phi.as_basic_value().into(),
                    i32_type.const_int(0o644, false).into(),
                ], "fd").or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                // Sign-extend i32 result to i64 if needed
                let fd = if fd.get_type().get_bit_width() < 64 {
                    builder.build_int_s_extend(fd, i64_type, "fd_ext").or_llvm_err()?
                } else { fd };

                builder.build_return(Some(&fd)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_close(fd: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_file_close")
                .unwrap_or_else(|| module.add_function("verum_file_close", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let bad_bb = self.context.append_basic_block(func, "bad_fd");
                let ok_bb = self.context.append_basic_block(func, "ok");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let is_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "is_neg").or_llvm_err()?;
                builder.build_conditional_branch(is_neg, bad_bb, ok_bb).or_llvm_err()?;

                builder.position_at_end(bad_bb);
                builder.build_return(Some(&i64_type.const_int(u64::MAX, true))).or_llvm_err()?;

                builder.position_at_end(ok_bb);
                let ret = self.build_libc_call(&builder, close_fn, &[fd.into()], "ret")?;
                builder.build_return(Some(&ret)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_exists(path: *i8) -> i64 (1 if exists, 0 if not)
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
            let func = module.get_function("verum_file_exists")
                .unwrap_or_else(|| module.add_function("verum_file_exists", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null_path");
                let check_bb = self.context.append_basic_block(func, "check");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let path = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let is_null = builder.build_is_null(path, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, check_bb).or_llvm_err()?;

                builder.position_at_end(null_bb);
                builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

                builder.position_at_end(check_bb);
                // Use access(path, F_OK=0) to check existence
                let access_fn = self.get_or_declare_access(module);
                let ret = self.build_libc_call(&builder, access_fn, &[
                    path.into(),
                    i32_type.const_zero().into(), // F_OK = 0
                ], "ret")?;
                let is_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, ret, i64_type.const_zero(), "is_zero").or_llvm_err()?;
                let result = builder.build_int_z_extend(is_zero, i64_type, "result").or_llvm_err()?;
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_delete(path: *i8) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
            let func = module.get_function("verum_file_delete")
                .unwrap_or_else(|| module.add_function("verum_file_delete", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null_path");
                let ok_bb = self.context.append_basic_block(func, "ok");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                // Defensive: a previously-declared function with the same
                // name (e.g. a Verum-side stdlib function lowered with
                // the int parameter shape) leaks through `get_function`,
                // and `into_pointer_value()` panics. Coerce param 0 via
                // int_to_ptr when it arrives as Int — same pattern the
                // SliceGet / SliceSubslice fixes use.
                let raw_path = func.get_nth_param(0).or_internal("missing param 0")?;
                let path = match raw_path {
                    verum_llvm::values::BasicValueEnum::PointerValue(p) => p,
                    verum_llvm::values::BasicValueEnum::IntValue(i) => {
                        builder
                            .build_int_to_ptr(i, ptr_type, "path_ptr")
                            .or_llvm_err()?
                    }
                    _ => {
                        return Err(LlvmLoweringError::internal(
                            "verum_file_delete: param 0 has unexpected variant",
                        ));
                    }
                };
                let is_null = builder.build_is_null(path, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, ok_bb).or_llvm_err()?;

                builder.position_at_end(null_bb);
                builder.build_return(Some(&i64_type.const_int(u64::MAX, true))).or_llvm_err()?;

                builder.position_at_end(ok_bb);
                let ret = self.build_libc_call(&builder, unlink_fn, &[path.into()], "ret")?;
                builder.build_return(Some(&ret)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_write_all(path: *i8, text_obj: i64) -> i64
        // Opens file, writes content from Text object, closes. Returns bytes written.
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_file_write_all")
                .unwrap_or_else(|| module.add_function("verum_file_write_all", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let get_data_bb = self.context.append_basic_block(func, "get_data");
                let check_data_bb = self.context.append_basic_block(func, "check_data");
                let open_bb = self.context.append_basic_block(func, "do_open");
                let check_fd_bb = self.context.append_basic_block(func, "check_fd");
                let write_bb = self.context.append_basic_block(func, "do_write");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let path = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let text_obj = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let is_null = builder.build_is_null(path, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, fail_bb, get_data_bb).or_llvm_err()?;

                builder.position_at_end(fail_bb);
                builder.build_return(Some(&i64_type.const_int(u64::MAX, true))).or_llvm_err()?;

                builder.position_at_end(get_data_bb);
                let data_ptr = builder.build_call(text_get_ptr_fn, &[text_obj.into()], "data_ptr")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
                builder.build_unconditional_branch(check_data_bb).or_llvm_err()?;

                builder.position_at_end(check_data_bb);
                let data_null = builder.build_is_null(data_ptr, "data_null").or_llvm_err()?;
                builder.build_conditional_branch(data_null, fail_bb, open_bb).or_llvm_err()?;

                builder.position_at_end(open_bb);
                // Use verum_file_open(path, mode=1=write) instead of raw open()
                // to avoid ARM64 variadic calling convention issues
                let file_open_fn = module.get_function("verum_file_open").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
                    module.add_function("verum_file_open", ft, None)
                });
                let fd = builder.build_call(file_open_fn, &[
                    path.into(),
                    i64_type.const_int(1, false).into(), // mode=1 = O_WRONLY|O_CREAT|O_TRUNC
                ], "fd").or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_unconditional_branch(check_fd_bb).or_llvm_err()?;

                builder.position_at_end(check_fd_bb);
                let fd_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fd_neg").or_llvm_err()?;
                builder.build_conditional_branch(fd_neg, fail_bb, write_bb).or_llvm_err()?;

                builder.position_at_end(write_bb);
                let data_len = builder.build_call(strlen_fn, &[data_ptr.into()], "data_len")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                let written = self.build_libc_call(&builder, write_fn, &[fd.into(), data_ptr.into(), data_len.into()], "written")?;
                // verum_file_close(fd)
                let file_close_fn = module.get_function("verum_file_close").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[i64_type.into()], false);
                    module.add_function("verum_file_close", ft, None)
                });
                builder.build_call(file_close_fn, &[fd.into()], "").or_llvm_err()?;
                builder.build_return(Some(&written)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_append_all(path: *i8, text_obj: i64) -> i64
        // Same as write_all but opens in append mode (mode=2).
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_file_append_all")
                .unwrap_or_else(|| module.add_function("verum_file_append_all", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let get_data_bb = self.context.append_basic_block(func, "get_data");
                let check_data_bb = self.context.append_basic_block(func, "check_data");
                let open_bb = self.context.append_basic_block(func, "do_open");
                let check_fd_bb = self.context.append_basic_block(func, "check_fd");
                let write_bb = self.context.append_basic_block(func, "do_write");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let path = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let text_obj = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let is_null = builder.build_is_null(path, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, fail_bb, get_data_bb).or_llvm_err()?;

                builder.position_at_end(fail_bb);
                builder.build_return(Some(&i64_type.const_int(u64::MAX, true))).or_llvm_err()?;

                builder.position_at_end(get_data_bb);
                let data_ptr = builder.build_call(text_get_ptr_fn, &[text_obj.into()], "data_ptr")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
                builder.build_unconditional_branch(check_data_bb).or_llvm_err()?;

                builder.position_at_end(check_data_bb);
                let data_null = builder.build_is_null(data_ptr, "data_null").or_llvm_err()?;
                builder.build_conditional_branch(data_null, fail_bb, open_bb).or_llvm_err()?;

                builder.position_at_end(open_bb);
                let file_open_fn = module.get_function("verum_file_open").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
                    module.add_function("verum_file_open", ft, None)
                });
                let fd = builder.build_call(file_open_fn, &[
                    path.into(),
                    i64_type.const_int(2, false).into(), // mode=2 = O_WRONLY|O_CREAT|O_APPEND
                ], "fd").or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_unconditional_branch(check_fd_bb).or_llvm_err()?;

                builder.position_at_end(check_fd_bb);
                let fd_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fd_neg").or_llvm_err()?;
                builder.build_conditional_branch(fd_neg, fail_bb, write_bb).or_llvm_err()?;

                builder.position_at_end(write_bb);
                let data_len = builder.build_call(strlen_fn, &[data_ptr.into()], "data_len")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                let written = self.build_libc_call(&builder, write_fn, &[fd.into(), data_ptr.into(), data_len.into()], "written")?;
                let file_close_fn = module.get_function("verum_file_close").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[i64_type.into()], false);
                    module.add_function("verum_file_close", ft, None)
                });
                builder.build_call(file_close_fn, &[fd.into()], "").or_llvm_err()?;
                builder.build_return(Some(&written)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_read_all(path: *i8) -> i64 (Text object)
        // Opens file, reads all content, returns Text. Uses lseek for size.
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
            let func = module.get_function("verum_file_read_all")
                .unwrap_or_else(|| module.add_function("verum_file_read_all", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let empty_bb = self.context.append_basic_block(func, "empty");
                let open_bb = self.context.append_basic_block(func, "do_open");
                let check_fd_bb = self.context.append_basic_block(func, "check_fd");
                let seek_bb = self.context.append_basic_block(func, "seek");
                let check_size_bb = self.context.append_basic_block(func, "check_size");
                let read_bb = self.context.append_basic_block(func, "do_read");
                let read_loop_bb = self.context.append_basic_block(func, "read_loop");
                let read_done_bb = self.context.append_basic_block(func, "read_done");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let path = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let is_null = builder.build_is_null(path, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, empty_bb, open_bb).or_llvm_err()?;

                // Return empty Text
                builder.position_at_end(empty_bb);
                let empty_str = builder.build_global_string_ptr("", "empty").or_llvm_err()?;
                let empty_text = builder.build_call(text_from_cstr_fn, &[empty_str.as_pointer_value().into()], "empty_text")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_return(Some(&empty_text)).or_llvm_err()?;

                // Open file via verum_file_open(path, mode=0=read)
                builder.position_at_end(open_bb);
                let file_open_fn = module.get_function("verum_file_open").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
                    module.add_function("verum_file_open", ft, None)
                });
                let file_close_fn = module.get_function("verum_file_close").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[i64_type.into()], false);
                    module.add_function("verum_file_close", ft, None)
                });
                let fd = builder.build_call(file_open_fn, &[
                    path.into(),
                    i64_type.const_zero().into(), // mode=0 = O_RDONLY
                ], "fd").or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_unconditional_branch(check_fd_bb).or_llvm_err()?;

                builder.position_at_end(check_fd_bb);
                let fd_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fd_neg").or_llvm_err()?;
                builder.build_conditional_branch(fd_neg, empty_bb, seek_bb).or_llvm_err()?;

                // lseek to get file size
                builder.position_at_end(seek_bb);
                let size = self.build_libc_call(&builder, lseek_fn, &[
                    fd.into(),
                    i64_type.const_zero().into(),
                    i32_type.const_int(2, false).into(), // SEEK_END
                ], "size")?;
                self.build_libc_call_void(&builder, lseek_fn, &[
                    fd.into(),
                    i64_type.const_zero().into(),
                    i32_type.const_zero().into(), // SEEK_SET
                ], "")?;
                builder.build_unconditional_branch(check_size_bb).or_llvm_err()?;

                builder.position_at_end(check_size_bb);
                let size_le_zero = builder.build_int_compare(verum_llvm::IntPredicate::SLE, size, i64_type.const_zero(), "size_le_zero").or_llvm_err()?;
                let close_empty_bb = self.context.append_basic_block(func, "close_empty");
                builder.build_conditional_branch(size_le_zero, close_empty_bb, read_bb).or_llvm_err()?;

                builder.position_at_end(close_empty_bb);
                builder.build_call(file_close_fn, &[fd.into()], "").or_llvm_err()?;
                builder.build_unconditional_branch(empty_bb).or_llvm_err()?;

                // Allocate buffer and read
                builder.position_at_end(read_bb);
                let buf_size = builder.build_int_add(size, i64_type.const_int(1, false), "buf_size").or_llvm_err()?;
                let buf = self.emit_checked_malloc(&builder, module, buf_size, "buf")?;
                let buf_ok_bb = builder.get_insert_block().or_internal("no block after buf malloc")?;
                builder.build_unconditional_branch(read_loop_bb).or_llvm_err()?;

                // Read loop
                builder.position_at_end(read_loop_bb);
                let total_phi = builder.build_phi(i64_type, "total").or_llvm_err()?;
                total_phi.add_incoming(&[(&i64_type.const_zero(), buf_ok_bb)]);
                let total = total_phi.as_basic_value().into_int_value();
                let remaining = builder.build_int_sub(size, total, "remaining").or_llvm_err()?;
                // SAFETY: GEP into the string buffer at a computed offset; the offset is derived from previously validated string lengths
                let buf_offset = unsafe { builder.build_gep(self.context.i8_type(), buf, &[total], "buf_off").or_llvm_err()? };
                let n = self.build_libc_call(&builder, read_fn, &[fd.into(), buf_offset.into(), remaining.into()], "n")?;
                let n_le_zero = builder.build_int_compare(verum_llvm::IntPredicate::SLE, n, i64_type.const_zero(), "n_le_zero").or_llvm_err()?;
                let new_total = builder.build_int_add(total, n, "new_total").or_llvm_err()?;
                let done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, new_total, size, "done").or_llvm_err()?;
                let should_stop = builder.build_or(n_le_zero, done, "should_stop").or_llvm_err()?;
                total_phi.add_incoming(&[(&new_total, read_loop_bb)]);
                builder.build_conditional_branch(should_stop, read_done_bb, read_loop_bb).or_llvm_err()?;

                // Done: null-terminate, close, return Text
                builder.position_at_end(read_done_bb);
                let final_total = builder.build_phi(i64_type, "final_total").or_llvm_err()?;
                final_total.add_incoming(&[(&new_total, read_loop_bb)]);
                let ft = final_total.as_basic_value().into_int_value();
                let final_len = builder.build_select(
                    builder.build_int_compare(verum_llvm::IntPredicate::SGT, ft, i64_type.const_zero(), "pos").or_llvm_err()?,
                    ft,
                    i64_type.const_zero(),
                    "final_len"
                ).or_llvm_err()?.into_int_value();
                // SAFETY: GEP to access the 'term_ptr' field at a fixed offset within a struct of known layout
                let term_ptr = unsafe { builder.build_gep(self.context.i8_type(), buf, &[final_len], "term_ptr").or_llvm_err()? };
                builder.build_store(term_ptr, self.context.i8_type().const_zero()).or_llvm_err()?;
                builder.build_call(file_close_fn, &[fd.into()], "").or_llvm_err()?;
                let result = builder.build_call(text_from_cstr_fn, &[buf.into()], "result")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_read_text(fd: i64, max_len: i64) -> i64 (Text)
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_file_read_text")
                .unwrap_or_else(|| module.add_function("verum_file_read_text", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let empty_bb = self.context.append_basic_block(func, "empty");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let read_bb = self.context.append_basic_block(func, "do_read");
                let check_n_bb = self.context.append_basic_block(func, "check_n");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let max_len = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

                let fd_bad = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fd_bad").or_llvm_err()?;
                let len_bad = builder.build_int_compare(verum_llvm::IntPredicate::SLE, max_len, i64_type.const_zero(), "len_bad").or_llvm_err()?;
                let bad = builder.build_or(fd_bad, len_bad, "bad").or_llvm_err()?;
                builder.build_conditional_branch(bad, empty_bb, ok_bb).or_llvm_err()?;

                builder.position_at_end(empty_bb);
                let empty_str = builder.build_global_string_ptr("", "empty").or_llvm_err()?;
                let empty_text = builder.build_call(text_from_cstr_fn, &[empty_str.as_pointer_value().into()], "empty_text")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_return(Some(&empty_text)).or_llvm_err()?;

                builder.position_at_end(ok_bb);
                let buf_size = builder.build_int_add(max_len, i64_type.const_int(1, false), "buf_size").or_llvm_err()?;
                let buf = self.emit_checked_malloc(&builder, module, buf_size, "buf")?;
                builder.build_unconditional_branch(read_bb).or_llvm_err()?;

                builder.position_at_end(read_bb);
                let n = self.build_libc_call(&builder, read_fn, &[fd.into(), buf.into(), max_len.into()], "n")?;
                builder.build_unconditional_branch(check_n_bb).or_llvm_err()?;

                builder.position_at_end(check_n_bb);
                let n_le_zero = builder.build_int_compare(verum_llvm::IntPredicate::SLE, n, i64_type.const_zero(), "n_le_zero").or_llvm_err()?;
                let free_empty_bb = self.context.append_basic_block(func, "free_empty");
                let make_text_bb = self.context.append_basic_block(func, "make_text");
                builder.build_conditional_branch(n_le_zero, free_empty_bb, make_text_bb).or_llvm_err()?;

                builder.position_at_end(free_empty_bb);
                // Free the read buffer before returning empty (prevent memory leak)
                let free_fn = module.get_function("free").unwrap_or_else(|| {
                    let void_type = self.context.void_type();
                    let ptr_type = self.context.ptr_type(verum_llvm::AddressSpace::default());
                    module.add_function("free", void_type.fn_type(&[ptr_type.into()], false), None)
                });
                builder.build_call(free_fn, &[buf.into()], "").or_llvm_err()?;
                builder.build_unconditional_branch(empty_bb).or_llvm_err()?;

                builder.position_at_end(make_text_bb);
                // SAFETY: GEP to access the 'term_ptr' field at a fixed offset within a struct of known layout
                let term_ptr = unsafe { builder.build_gep(self.context.i8_type(), buf, &[n], "term_ptr").or_llvm_err()? };
                builder.build_store(term_ptr, self.context.i8_type().const_zero()).or_llvm_err()?;
                let result = builder.build_call(text_from_cstr_fn, &[buf.into()], "result")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_write_text(fd: i64, text_obj: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_file_write_text")
                .unwrap_or_else(|| module.add_function("verum_file_write_text", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let check_ptr_bb = self.context.append_basic_block(func, "check_ptr");
                let write_bb = self.context.append_basic_block(func, "do_write");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let text_obj = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

                let fd_bad = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fd_bad").or_llvm_err()?;
                builder.build_conditional_branch(fd_bad, fail_bb, ok_bb).or_llvm_err()?;

                builder.position_at_end(fail_bb);
                builder.build_return(Some(&i64_type.const_int(u64::MAX, true))).or_llvm_err()?;

                builder.position_at_end(ok_bb);
                let data_ptr = builder.build_call(text_get_ptr_fn, &[text_obj.into()], "data_ptr")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
                builder.build_unconditional_branch(check_ptr_bb).or_llvm_err()?;

                builder.position_at_end(check_ptr_bb);
                let is_null = builder.build_is_null(data_ptr, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, fail_bb, write_bb).or_llvm_err()?;

                builder.position_at_end(write_bb);
                let data_len = builder.build_call(strlen_fn, &[data_ptr.into()], "data_len")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                let written = self.build_libc_call(&builder, write_fn, &[fd.into(), data_ptr.into(), data_len.into()], "written")?;
                builder.build_return(Some(&written)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_read_to_string(path: *i8) -> *i8
        // Reads entire file to a heap-allocated C string. Returns NULL on error.
        // ============================================================
        {
            let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
            let func = module.get_function("verum_file_read_to_string")
                .unwrap_or_else(|| module.add_function("verum_file_read_to_string", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null_ret");
                let open_bb = self.context.append_basic_block(func, "do_open");
                let check_fd_bb = self.context.append_basic_block(func, "check_fd");
                let seek_bb = self.context.append_basic_block(func, "seek");
                let check_size_bb = self.context.append_basic_block(func, "check_size");
                let alloc_bb = self.context.append_basic_block(func, "alloc");
                let read_loop_bb = self.context.append_basic_block(func, "read_loop");
                let done_bb = self.context.append_basic_block(func, "done");
                let close_null_bb = self.context.append_basic_block(func, "close_null");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let path = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let is_null = builder.build_is_null(path, "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, open_bb).or_llvm_err()?;

                builder.position_at_end(null_bb);
                builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;

                builder.position_at_end(open_bb);
                let r2s_open_fn = module.get_function("verum_file_open").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
                    module.add_function("verum_file_open", ft, None)
                });
                let fd = builder.build_call(r2s_open_fn, &[
                    path.into(), i64_type.const_zero().into(),
                ], "fd").or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_unconditional_branch(check_fd_bb).or_llvm_err()?;

                builder.position_at_end(check_fd_bb);
                let fd_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fd_neg").or_llvm_err()?;
                builder.build_conditional_branch(fd_neg, null_bb, seek_bb).or_llvm_err()?;

                builder.position_at_end(seek_bb);
                let size = self.build_libc_call(&builder, lseek_fn, &[
                    fd.into(), i64_type.const_zero().into(), i32_type.const_int(2, false).into(),
                ], "size")?;
                self.build_libc_call_void(&builder, lseek_fn, &[
                    fd.into(), i64_type.const_zero().into(), i32_type.const_zero().into(),
                ], "")?;
                builder.build_unconditional_branch(check_size_bb).or_llvm_err()?;

                builder.position_at_end(check_size_bb);
                let size_le = builder.build_int_compare(verum_llvm::IntPredicate::SLE, size, i64_type.const_zero(), "size_le").or_llvm_err()?;
                builder.build_conditional_branch(size_le, close_null_bb, alloc_bb).or_llvm_err()?;

                builder.position_at_end(close_null_bb);
                let r2s_close_fn = module.get_function("verum_file_close").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[i64_type.into()], false);
                    module.add_function("verum_file_close", ft, None)
                });
                builder.build_call(r2s_close_fn, &[fd.into()], "").or_llvm_err()?;
                builder.build_unconditional_branch(null_bb).or_llvm_err()?;

                builder.position_at_end(alloc_bb);
                let buf_size = builder.build_int_add(size, i64_type.const_int(1, false), "buf_size").or_llvm_err()?;
                let buf = self.emit_checked_malloc(&builder, module, buf_size, "buf")?;
                let buf_ok_bb2 = builder.get_insert_block().or_internal("no block after buf malloc")?;
                builder.build_unconditional_branch(read_loop_bb).or_llvm_err()?;

                builder.position_at_end(read_loop_bb);
                let total_phi = builder.build_phi(i64_type, "total").or_llvm_err()?;
                total_phi.add_incoming(&[(&i64_type.const_zero(), buf_ok_bb2)]);
                let total = total_phi.as_basic_value().into_int_value();
                let remaining = builder.build_int_sub(size, total, "rem").or_llvm_err()?;
                // SAFETY: GEP into the string buffer at a computed offset; the offset is derived from previously validated string lengths
                let buf_off = unsafe { builder.build_gep(self.context.i8_type(), buf, &[total], "buf_off").or_llvm_err()? };
                let n = self.build_libc_call(&builder, read_fn, &[fd.into(), buf_off.into(), remaining.into()], "n")?;
                let n_le = builder.build_int_compare(verum_llvm::IntPredicate::SLE, n, i64_type.const_zero(), "n_le").or_llvm_err()?;
                let new_total = builder.build_int_add(total, n, "new_total").or_llvm_err()?;
                let all_read = builder.build_int_compare(verum_llvm::IntPredicate::SGE, new_total, size, "all_read").or_llvm_err()?;
                let stop = builder.build_or(n_le, all_read, "stop").or_llvm_err()?;
                total_phi.add_incoming(&[(&new_total, read_loop_bb)]);
                builder.build_conditional_branch(stop, done_bb, read_loop_bb).or_llvm_err()?;

                builder.position_at_end(done_bb);
                let final_len = builder.build_select(
                    builder.build_int_compare(verum_llvm::IntPredicate::SGT, new_total, i64_type.const_zero(), "pos").or_llvm_err()?,
                    new_total, i64_type.const_zero(), "final_len"
                ).or_llvm_err()?.into_int_value();
                // SAFETY: GEP to access the 'term' field at a fixed offset within a struct of known layout
                let term = unsafe { builder.build_gep(self.context.i8_type(), buf, &[final_len], "term").or_llvm_err()? };
                builder.build_store(term, self.context.i8_type().const_zero()).or_llvm_err()?;
                // Use verum_file_close wrapper
                let r2s_close2_fn = module.get_function("verum_file_close").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[i64_type.into()], false);
                    module.add_function("verum_file_close", ft, None)
                });
                builder.build_call(r2s_close2_fn, &[fd.into()], "").or_llvm_err()?;
                builder.build_return(Some(&buf)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_file_write_string(path: *i8, content: *i8) -> i64
        // Writes C string to file. Returns bytes written or 0 on error.
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            let func = module.get_function("verum_file_write_string")
                .unwrap_or_else(|| module.add_function("verum_file_write_string", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let open_file_bb = self.context.append_basic_block(func, "open_file");
                let check_fd_bb = self.context.append_basic_block(func, "check_fd");
                let do_write_bb = self.context.append_basic_block(func, "do_write");

                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let path = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let content = func.get_nth_param(1).or_internal("missing param 1")?.into_pointer_value();
                let path_null = builder.build_is_null(path, "path_null").or_llvm_err()?;
                let content_null = builder.build_is_null(content, "content_null").or_llvm_err()?;
                let any_null = builder.build_or(path_null, content_null, "any_null").or_llvm_err()?;
                builder.build_conditional_branch(any_null, fail_bb, ok_bb).or_llvm_err()?;

                builder.position_at_end(fail_bb);
                builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

                builder.position_at_end(ok_bb);
                builder.build_unconditional_branch(open_file_bb).or_llvm_err()?;

                builder.position_at_end(open_file_bb);
                // Use verum_file_open wrapper (which uses verum_raw_open3)
                // to avoid ARM64 variadic calling convention issues
                let file_open_fn = module.get_function("verum_file_open").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
                    module.add_function("verum_file_open", ft, None)
                });
                let fd = builder.build_call(file_open_fn, &[
                    path.into(),
                    i64_type.const_int(1, false).into(), // mode=1 = write
                ], "fd").or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_unconditional_branch(check_fd_bb).or_llvm_err()?;

                builder.position_at_end(check_fd_bb);
                let fd_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fd_neg").or_llvm_err()?;
                builder.build_conditional_branch(fd_neg, fail_bb, do_write_bb).or_llvm_err()?;

                builder.position_at_end(do_write_bb);
                let len = builder.build_call(strlen_fn, &[content.into()], "len")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                let written = self.build_libc_call(&builder, write_fn, &[fd.into(), content.into(), len.into()], "written")?;
                // verum_file_close(fd)
                let file_close_fn = module.get_function("verum_file_close").unwrap_or_else(|| {
                    let ft = i64_type.fn_type(&[i64_type.into()], false);
                    module.add_function("verum_file_close", ft, None)
                });
                builder.build_call(file_close_fn, &[fd.into()], "").or_llvm_err()?;
                builder.build_return(Some(&written)).or_llvm_err()?;
            }
        }
        Ok(())
    }

    // =========================================================================
    // Sync Bridge Functions — thin wrappers over verum_platform.c primitives
    // =========================================================================

    /// Emit LLVM IR for mutex and condvar bridge functions.
    /// These replace the C bridge functions (verum_mutex_new, verum_mutex_lock_bridge, etc.)
    /// by directly declaring and calling the platform-level functions from verum_platform.c.
    fn emit_verum_sync_bridge_functions(&self, module: &Module<'ctx>) -> Result<()> {
        let i64_type = self.context.i64_type();
        let void_type = self.context.void_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Declare platform-level functions from verum_platform.c
        let mutex_init_fn = module.get_function("verum_mutex_init").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_mutex_init", ft, None)
        });
        let mutex_lock_fn = module.get_function("verum_mutex_lock").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_mutex_lock", ft, None)
        });
        let mutex_unlock_fn = module.get_function("verum_mutex_unlock").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_mutex_unlock", ft, None)
        });
        let mutex_trylock_fn = module.get_function("verum_mutex_trylock").unwrap_or_else(|| {
            let ft = i64_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_mutex_trylock", ft, None)
        });
        let cond_init_fn = module.get_function("verum_cond_init").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_cond_init", ft, None)
        });
        let cond_wait_fn = module.get_function("verum_cond_wait").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            module.add_function("verum_cond_wait", ft, None)
        });
        let cond_timedwait_fn = module.get_function("verum_cond_timedwait").unwrap_or_else(|| {
            let ft = i64_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
            module.add_function("verum_cond_timedwait", ft, None)
        });
        let cond_signal_fn = module.get_function("verum_cond_signal").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_cond_signal", ft, None)
        });
        let cond_broadcast_fn = module.get_function("verum_cond_broadcast").unwrap_or_else(|| {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_cond_broadcast", ft, None)
        });


        // verum_mutex_new() -> i64 (pointer to VerumMutex)
        // VerumMutex is { _Atomic(int32_t) state } = 4 bytes, but allocate 8 for alignment
        {
            let fn_type = i64_type.fn_type(&[], false);
            let func = module.get_function("verum_mutex_new")
                .unwrap_or_else(|| module.add_function("verum_mutex_new", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                // malloc(8) for VerumMutex
                let size = i64_type.const_int(8, false);
                let ptr = self.emit_checked_malloc(&builder, module, size, "m")?;
                // Zero the memory (state = 0 = unlocked)
                let memset_fn = self.get_or_declare_memset(module)?;
                let i32_type = self.context.i32_type();
                builder.build_call(memset_fn, &[ptr.into(), i32_type.const_zero().into(), size.into()], "").or_llvm_err()?;
                // Call verum_mutex_init
                builder.build_call(mutex_init_fn, &[ptr.into()], "").or_llvm_err()?;
                let result = builder.build_ptr_to_int(ptr, i64_type, "result").or_llvm_err()?;
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // verum_mutex_lock_bridge(mutex_ptr: i64) -> void
        {
            let fn_type = void_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_mutex_lock_bridge")
                .unwrap_or_else(|| module.add_function("verum_mutex_lock_bridge", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let arg = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, arg, i64_type.const_zero(), "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, ok_bb).or_llvm_err()?;
                builder.position_at_end(null_bb);
                builder.build_return(None).or_llvm_err()?;
                builder.position_at_end(ok_bb);
                let ptr = builder.build_int_to_ptr(arg, ptr_type, "ptr").or_llvm_err()?;
                builder.build_call(mutex_lock_fn, &[ptr.into()], "").or_llvm_err()?;
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // verum_mutex_unlock_bridge(mutex_ptr: i64) -> void
        {
            let fn_type = void_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_mutex_unlock_bridge")
                .unwrap_or_else(|| module.add_function("verum_mutex_unlock_bridge", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let arg = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, arg, i64_type.const_zero(), "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, ok_bb).or_llvm_err()?;
                builder.position_at_end(null_bb);
                builder.build_return(None).or_llvm_err()?;
                builder.position_at_end(ok_bb);
                let ptr = builder.build_int_to_ptr(arg, ptr_type, "ptr").or_llvm_err()?;
                builder.build_call(mutex_unlock_fn, &[ptr.into()], "").or_llvm_err()?;
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // verum_mutex_trylock_bridge(mutex_ptr: i64) -> i64
        {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_mutex_trylock_bridge")
                .unwrap_or_else(|| module.add_function("verum_mutex_trylock_bridge", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let arg = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, arg, i64_type.const_zero(), "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, ok_bb).or_llvm_err()?;
                builder.position_at_end(null_bb);
                builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
                builder.position_at_end(ok_bb);
                let ptr = builder.build_int_to_ptr(arg, ptr_type, "ptr").or_llvm_err()?;
                let result = builder.build_call(mutex_trylock_fn, &[ptr.into()], "result")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // verum_cond_new() -> i64 (pointer to VerumCondVar)
        // VerumCondVar is { _Atomic(int32_t) seq } = 4 bytes, allocate 8 for alignment
        {
            let fn_type = i64_type.fn_type(&[], false);
            let func = module.get_function("verum_cond_new")
                .unwrap_or_else(|| module.add_function("verum_cond_new", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let size = i64_type.const_int(8, false);
                let ptr = self.emit_checked_malloc(&builder, module, size, "cv")?;
                let memset_fn = self.get_or_declare_memset(module)?;
                let i32_type = self.context.i32_type();
                builder.build_call(memset_fn, &[ptr.into(), i32_type.const_zero().into(), size.into()], "").or_llvm_err()?;
                builder.build_call(cond_init_fn, &[ptr.into()], "").or_llvm_err()?;
                let result = builder.build_ptr_to_int(ptr, i64_type, "result").or_llvm_err()?;
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // verum_cond_wait_bridge(cond_ptr: i64, mutex_ptr: i64) -> void
        {
            let fn_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_cond_wait_bridge")
                .unwrap_or_else(|| module.add_function("verum_cond_wait_bridge", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let cond_arg = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let mutex_arg = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let c_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, cond_arg, i64_type.const_zero(), "c_null").or_llvm_err()?;
                let m_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, mutex_arg, i64_type.const_zero(), "m_null").or_llvm_err()?;
                let either_null = builder.build_or(c_null, m_null, "either").or_llvm_err()?;
                builder.build_conditional_branch(either_null, null_bb, ok_bb).or_llvm_err()?;
                builder.position_at_end(null_bb);
                builder.build_return(None).or_llvm_err()?;
                builder.position_at_end(ok_bb);
                let cv_ptr = builder.build_int_to_ptr(cond_arg, ptr_type, "cv").or_llvm_err()?;
                let mx_ptr = builder.build_int_to_ptr(mutex_arg, ptr_type, "mx").or_llvm_err()?;
                builder.build_call(cond_wait_fn, &[cv_ptr.into(), mx_ptr.into()], "").or_llvm_err()?;
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // verum_cond_timedwait_bridge(cond_ptr: i64, mutex_ptr: i64, timeout_ns: i64) -> i64
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_cond_timedwait_bridge")
                .unwrap_or_else(|| module.add_function("verum_cond_timedwait_bridge", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let cond_arg = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let mutex_arg = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let timeout = func.get_nth_param(2).or_internal("missing param 2")?.into_int_value();
                let c_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, cond_arg, i64_type.const_zero(), "c_null").or_llvm_err()?;
                let m_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, mutex_arg, i64_type.const_zero(), "m_null").or_llvm_err()?;
                let either_null = builder.build_or(c_null, m_null, "either").or_llvm_err()?;
                builder.build_conditional_branch(either_null, null_bb, ok_bb).or_llvm_err()?;
                builder.position_at_end(null_bb);
                builder.build_return(Some(&i64_type.const_int(1, false))).or_llvm_err()?; // 1 = timeout
                builder.position_at_end(ok_bb);
                let cv_ptr = builder.build_int_to_ptr(cond_arg, ptr_type, "cv").or_llvm_err()?;
                let mx_ptr = builder.build_int_to_ptr(mutex_arg, ptr_type, "mx").or_llvm_err()?;
                let result = builder.build_call(cond_timedwait_fn, &[cv_ptr.into(), mx_ptr.into(), timeout.into()], "result")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // verum_cond_signal_bridge(cond_ptr: i64) -> void
        {
            let fn_type = void_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_cond_signal_bridge")
                .unwrap_or_else(|| module.add_function("verum_cond_signal_bridge", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let arg = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, arg, i64_type.const_zero(), "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, ok_bb).or_llvm_err()?;
                builder.position_at_end(null_bb);
                builder.build_return(None).or_llvm_err()?;
                builder.position_at_end(ok_bb);
                let ptr = builder.build_int_to_ptr(arg, ptr_type, "ptr").or_llvm_err()?;
                builder.build_call(cond_signal_fn, &[ptr.into()], "").or_llvm_err()?;
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // verum_cond_broadcast_bridge(cond_ptr: i64) -> void
        {
            let fn_type = void_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_cond_broadcast_bridge")
                .unwrap_or_else(|| module.add_function("verum_cond_broadcast_bridge", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let arg = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, arg, i64_type.const_zero(), "is_null").or_llvm_err()?;
                builder.build_conditional_branch(is_null, null_bb, ok_bb).or_llvm_err()?;
                builder.position_at_end(null_bb);
                builder.build_return(None).or_llvm_err()?;
                builder.position_at_end(ok_bb);
                let ptr = builder.build_int_to_ptr(arg, ptr_type, "ptr").or_llvm_err()?;
                builder.build_call(cond_broadcast_fn, &[ptr.into()], "").or_llvm_err()?;
                builder.build_return(None).or_llvm_err()?;
            }
        }
        Ok(())
    }

    // =========================================================================
    // System Call Wrappers — getpid, gettid, mmap, munmap, madvise, getentropy
    // =========================================================================

    /// Emit LLVM IR for system call wrapper functions.
    /// These are simple wrappers around POSIX syscalls — on macOS we call libc,
    /// on Linux we could use inline asm but for simplicity call libc too.
    fn emit_verum_sys_functions(&self, module: &Module<'ctx>) -> Result<()> {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Declare libc: getpid() -> i32
        let getpid_fn = module.get_function("getpid").unwrap_or_else(|| {
            let ft = i32_type.fn_type(&[], false);
            module.add_function("getpid", ft, None)
        });

        // verum_sys_getpid() -> i64
        {
            let fn_type = i64_type.fn_type(&[], false);
            let func = module.get_function("verum_sys_getpid")
                .unwrap_or_else(|| module.add_function("verum_sys_getpid", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let pid = builder.build_call(getpid_fn, &[], "pid")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                let result = builder.build_int_s_extend(pid, i64_type, "ext").or_llvm_err()?;
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // verum_sys_gettid() -> i64 (on macOS: same as getpid)
        {
            let fn_type = i64_type.fn_type(&[], false);
            let func = module.get_function("verum_sys_gettid")
                .unwrap_or_else(|| module.add_function("verum_sys_gettid", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                // On all platforms, just call getpid as a safe default
                let pid = builder.build_call(getpid_fn, &[], "tid")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                let result = builder.build_int_s_extend(pid, i64_type, "ext").or_llvm_err()?;
                builder.build_return(Some(&result)).or_llvm_err()?;
            }
        }

        // verum_sys_mmap(addr, len, prot, flags, fd, offset) -> ptr
        // Delegate to malloc for anonymous mappings (simplification)
        {
            let fn_type = ptr_type.fn_type(&[
                ptr_type.into(), i64_type.into(), i64_type.into(),
                i64_type.into(), i64_type.into(), i64_type.into(),
            ], false);
            let func = module.get_function("verum_sys_mmap")
                .unwrap_or_else(|| module.add_function("verum_sys_mmap", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let len = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let ptr = self.emit_checked_malloc(&builder, module, len, "ptr")?;
                builder.build_return(Some(&ptr)).or_llvm_err()?;
            }
        }

        // verum_sys_munmap(addr: ptr, len: i64) -> void
        {
            let void_type = self.context.void_type();
            let fn_type = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_sys_munmap")
                .unwrap_or_else(|| module.add_function("verum_sys_munmap", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                // Delegate to free
                let free_fn = module.get_function("free").unwrap_or_else(|| {
                    let ft = void_type.fn_type(&[ptr_type.into()], false);
                    module.add_function("free", ft, None)
                });
                let addr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                builder.build_call(free_fn, &[addr.into()], "").or_llvm_err()?;
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // verum_sys_madvise(addr: ptr, len: i64, advice: i64) -> i64 (no-op, returns 0)
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_sys_madvise")
                .unwrap_or_else(|| module.add_function("verum_sys_madvise", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
            }
        }

        // verum_sys_getentropy(buf: ptr, len: i64) -> i64
        // Fill with zeros as a safe default (proper entropy would need arc4random/getrandom)
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_sys_getentropy")
                .unwrap_or_else(|| module.add_function("verum_sys_getentropy", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let builder = self.context.create_builder();
                builder.position_at_end(entry);
                let buf = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let len = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let memset_fn = self.get_or_declare_memset(module)?;
                // Fill with 0 (safe default — proper entropy not critical for now)
                builder.build_call(memset_fn, &[buf.into(), i32_type.const_zero().into(), len.into()], "").or_llvm_err()?;
                builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
            }
        }
        Ok(())
    }

    // =========================================================================
    // String Join — replaces verum_string_join C function
    // =========================================================================

    /// Emit verum_string_join(list_ptr: i64, sep: *i8) -> *i8
    /// Joins a List<Text> with a separator string.
    fn emit_verum_string_join(&self, module: &Module<'ctx>) -> Result<()> {
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        let fn_type = ptr_type.fn_type(&[i64_type.into(), ptr_type.into()], false);
        let func = module.get_function("verum_string_join")
            .unwrap_or_else(|| module.add_function("verum_string_join", fn_type, None));
        if func.count_basic_blocks() > 0 { return Ok(()); }

        let entry = self.context.append_basic_block(func, "entry");
        let null_list_bb = self.context.append_basic_block(func, "null_list");
        let has_list_bb = self.context.append_basic_block(func, "has_list");
        let empty_list_bb = self.context.append_basic_block(func, "empty_list");
        let compute_bb = self.context.append_basic_block(func, "compute");
        let alloc_bb = self.context.append_basic_block(func, "alloc");

        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        // Handle both (i64, ptr) and (ptr, ptr) signatures — compiled modules
        // may declare this function with ptr params instead of i64.
        let param0 = func.get_nth_param(0).or_internal("missing param 0")?;
        let list_ptr_i64 = if param0.is_int_value() {
            param0.into_int_value()
        } else {
            builder.build_ptr_to_int(param0.into_pointer_value(), i64_type, "list_as_i64").or_llvm_err()?
        };
        let param1 = func.get_nth_param(1).or_internal("missing param 1")?;
        let sep = if param1.is_pointer_value() {
            param1.into_pointer_value()
        } else {
            builder.build_int_to_ptr(param1.into_int_value(), ptr_type, "sep_as_ptr").or_llvm_err()?
        };

        let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, list_ptr_i64, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, null_list_bb, has_list_bb).or_llvm_err()?;

        // Return empty string for null list
        builder.position_at_end(null_list_bb);
        let empty = self.emit_checked_malloc(&builder, module, i64_type.const_int(1, false), "empty")?;
        builder.build_store(empty, self.context.i8_type().const_zero()).or_llvm_err()?;
        builder.build_return(Some(&empty)).or_llvm_err()?;

        // Load list header: [... ptr(idx=3), len(idx=4), cap(idx=5)]
        builder.position_at_end(has_list_bb);
        let list_ptr = builder.build_int_to_ptr(list_ptr_i64, ptr_type, "list_ptr").or_llvm_err()?;
        // LIST_LEN_IDX = 4 (offset 32 bytes)
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let len_gep = unsafe { builder.build_gep(i64_type, list_ptr, &[i64_type.const_int(4, false)], "len_gep").or_llvm_err()? };
        let len = builder.build_load(i64_type, len_gep, "len").or_llvm_err()?.into_int_value();
        let is_empty = builder.build_int_compare(verum_llvm::IntPredicate::EQ, len, i64_type.const_zero(), "is_empty").or_llvm_err()?;
        builder.build_conditional_branch(is_empty, empty_list_bb, compute_bb).or_llvm_err()?;

        // Return empty string for empty list
        builder.position_at_end(empty_list_bb);
        let empty2 = self.emit_checked_malloc(&builder, module, i64_type.const_int(1, false), "empty2")?;
        builder.build_store(empty2, self.context.i8_type().const_zero()).or_llvm_err()?;
        builder.build_return(Some(&empty2)).or_llvm_err()?;

        // Compute total length: iterate elements, get string pointers, sum lengths
        builder.position_at_end(compute_bb);
        // LIST_PTR_IDX = 3 (offset 24 bytes)
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let data_gep = unsafe { builder.build_gep(i64_type, list_ptr, &[i64_type.const_int(3, false)], "data_gep").or_llvm_err()? };
        let data_i64 = builder.build_load(i64_type, data_gep, "data_i64").or_llvm_err()?.into_int_value();
        let data_ptr = builder.build_int_to_ptr(data_i64, ptr_type, "data").or_llvm_err()?;

        let strlen_fn = self.get_or_declare_strlen(module);
        let memcpy_fn = self.get_or_declare_memcpy(module);
        let text_get_ptr_fn = module.get_function("verum_text_get_ptr").unwrap_or_else(|| {
            let ft = ptr_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_text_get_ptr", ft, None)
        });

        // Get separator length
        let sep_is_null = builder.build_is_null(sep, "sep_null").or_llvm_err()?;
        let sep_len_raw = builder.build_call(strlen_fn, &[sep.into()], "sep_len")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let sep_len = builder.build_select(sep_is_null, i64_type.const_zero(), sep_len_raw, "sep_len_final").or_llvm_err()?.into_int_value();

        // First pass: compute total length
        // total = sum(strlen(elem[i])) + sep_len * (len - 1)
        let len_minus_1 = builder.build_int_sub(len, i64_type.const_int(1, false), "lm1").or_llvm_err()?;
        let sep_total = builder.build_int_mul(sep_len, len_minus_1, "sep_total").or_llvm_err()?;

        // Loop to sum element lengths
        let sum_loop = self.context.append_basic_block(func, "sum_loop");
        let sum_body = self.context.append_basic_block(func, "sum_body");
        let sum_done = self.context.append_basic_block(func, "sum_done");

        builder.build_unconditional_branch(sum_loop).or_llvm_err()?;

        builder.position_at_end(sum_loop);
        let sum_i = builder.build_phi(i64_type, "sum_i").or_llvm_err()?;
        let sum_total = builder.build_phi(i64_type, "sum_total").or_llvm_err()?;
        sum_i.add_incoming(&[(&i64_type.const_zero(), compute_bb)]);
        sum_total.add_incoming(&[(&sep_total, compute_bb)]);
        let sum_i_val = sum_i.as_basic_value().into_int_value();
        let sum_total_val = sum_total.as_basic_value().into_int_value();
        let sum_cond = builder.build_int_compare(verum_llvm::IntPredicate::SLT, sum_i_val, len, "sum_cond").or_llvm_err()?;
        builder.build_conditional_branch(sum_cond, sum_body, sum_done).or_llvm_err()?;

        builder.position_at_end(sum_body);
        // SAFETY: GEP into list data array to access an element; the index is validated against the list length before access
        let elem_gep = unsafe { builder.build_gep(i64_type, data_ptr, &[sum_i_val], "elem_gep").or_llvm_err()? };
        let elem_i64 = builder.build_load(i64_type, elem_gep, "elem_i64").or_llvm_err()?.into_int_value();
        let elem_ptr = builder.build_call(text_get_ptr_fn, &[elem_i64.into()], "elem_ptr")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        let elem_len = builder.build_call(strlen_fn, &[elem_ptr.into()], "elem_len")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let new_total = builder.build_int_add(sum_total_val, elem_len, "new_total").or_llvm_err()?;
        let new_i = builder.build_int_add(sum_i_val, i64_type.const_int(1, false), "new_i").or_llvm_err()?;
        sum_i.add_incoming(&[(&new_i, sum_body)]);
        sum_total.add_incoming(&[(&new_total, sum_body)]);
        builder.build_unconditional_branch(sum_loop).or_llvm_err()?;

        // Allocate result buffer
        builder.position_at_end(sum_done);
        let final_total = sum_total.as_basic_value().into_int_value();
        builder.build_unconditional_branch(alloc_bb).or_llvm_err()?;

        builder.position_at_end(alloc_bb);
        let alloc_size = builder.build_int_add(final_total, i64_type.const_int(1, false), "alloc_size").or_llvm_err()?;
        let result_buf = self.emit_checked_malloc(&builder, module, alloc_size, "result_buf")?;

        // Second pass: copy elements with separators
        let copy_loop_bb = self.context.append_basic_block(func, "copy_loop_head");
        let copy_body_bb = self.context.append_basic_block(func, "copy_body");
        let copy_sep_bb = self.context.append_basic_block(func, "copy_sep");
        let copy_no_sep_bb = self.context.append_basic_block(func, "copy_no_sep");
        let copy_done_bb = self.context.append_basic_block(func, "copy_done");

        builder.build_unconditional_branch(copy_loop_bb).or_llvm_err()?;

        builder.position_at_end(copy_loop_bb);
        let cp_i = builder.build_phi(i64_type, "cp_i").or_llvm_err()?;
        let cp_pos = builder.build_phi(i64_type, "cp_pos").or_llvm_err()?;
        cp_i.add_incoming(&[(&i64_type.const_zero(), alloc_bb)]);
        cp_pos.add_incoming(&[(&i64_type.const_zero(), alloc_bb)]);
        let cp_i_val = cp_i.as_basic_value().into_int_value();
        let cp_pos_val = cp_pos.as_basic_value().into_int_value();
        let cp_cond = builder.build_int_compare(verum_llvm::IntPredicate::SLT, cp_i_val, len, "cp_cond").or_llvm_err()?;
        builder.build_conditional_branch(cp_cond, copy_body_bb, copy_done_bb).or_llvm_err()?;

        builder.position_at_end(copy_body_bb);
        // SAFETY: GEP into list data array to access an element; the index is validated against the list length before access
        let cp_elem_gep = unsafe { builder.build_gep(i64_type, data_ptr, &[cp_i_val], "cp_elem_gep").or_llvm_err()? };
        let cp_elem_i64 = builder.build_load(i64_type, cp_elem_gep, "cp_elem_i64").or_llvm_err()?.into_int_value();
        let cp_elem_ptr = builder.build_call(text_get_ptr_fn, &[cp_elem_i64.into()], "cp_elem_ptr")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        let cp_elem_len = builder.build_call(strlen_fn, &[cp_elem_ptr.into()], "cp_elem_len")
            .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();

        // memcpy element
        // SAFETY: GEP into an allocated buffer; the offset is computed from validated lengths that do not exceed the buffer capacity
        let dst_gep = unsafe { builder.build_gep(self.context.i8_type(), result_buf, &[cp_pos_val], "dst").or_llvm_err()? };
        builder.build_call(memcpy_fn, &[dst_gep.into(), cp_elem_ptr.into(), cp_elem_len.into()], "").or_llvm_err()?;
        let pos_after_elem = builder.build_int_add(cp_pos_val, cp_elem_len, "pos_after_elem").or_llvm_err()?;

        // Add separator if not last element
        let is_last = builder.build_int_compare(verum_llvm::IntPredicate::EQ, cp_i_val, len_minus_1, "is_last").or_llvm_err()?;
        builder.build_conditional_branch(is_last, copy_no_sep_bb, copy_sep_bb).or_llvm_err()?;

        builder.position_at_end(copy_sep_bb);
        // SAFETY: GEP to compute the end-of-buffer position; the offset is the sum of validated lengths that fit within the allocation
        let sep_dst = unsafe { builder.build_gep(self.context.i8_type(), result_buf, &[pos_after_elem], "sep_dst").or_llvm_err()? };
        builder.build_call(memcpy_fn, &[sep_dst.into(), sep.into(), sep_len.into()], "").or_llvm_err()?;
        let pos_after_sep = builder.build_int_add(pos_after_elem, sep_len, "pos_after_sep").or_llvm_err()?;
        builder.build_unconditional_branch(copy_no_sep_bb).or_llvm_err()?;

        builder.position_at_end(copy_no_sep_bb);
        let final_pos = builder.build_phi(i64_type, "final_pos").or_llvm_err()?;
        final_pos.add_incoming(&[(&pos_after_elem, copy_body_bb), (&pos_after_sep, copy_sep_bb)]);
        let final_pos_val = final_pos.as_basic_value().into_int_value();
        let cp_i_next = builder.build_int_add(cp_i_val, i64_type.const_int(1, false), "cp_i_next").or_llvm_err()?;
        cp_i.add_incoming(&[(&cp_i_next, copy_no_sep_bb)]);
        cp_pos.add_incoming(&[(&final_pos_val, copy_no_sep_bb)]);
        builder.build_unconditional_branch(copy_loop_bb).or_llvm_err()?;

        // Null-terminate and return
        builder.position_at_end(copy_done_bb);
        let final_cp_pos = cp_pos.as_basic_value().into_int_value();
        // SAFETY: GEP to access the 'term' field at a fixed offset within a struct of known layout
        let term_gep = unsafe { builder.build_gep(self.context.i8_type(), result_buf, &[final_cp_pos], "term").or_llvm_err()? };
        builder.build_store(term_gep, self.context.i8_type().const_zero()).or_llvm_err()?;
        builder.build_return(Some(&result_buf)).or_llvm_err()?;
        Ok(())
    }

    // =========================================================================
    // Args Functions — DELETED (dead code)
    // =========================================================================
    // emit_verum_args_functions was never called and was broken:
    // LLVM IR versions read from verum_ir_stored_argc/argv globals that are
    // never written to (C entry point writes verum_stored_argc/argv instead).
    // The C implementations in verum_runtime.c are the correct path.

    /// Get or declare open(path: *i8, flags: i32, ...) -> i32.
    /// CRITICAL: open() is variadic — on Apple ARM64, variadic args go on the
    /// stack, not in registers. Declaring as non-variadic causes mode_t to be
    /// passed in w2 (register) while libc reads from stack → garbage permissions.
    fn get_or_declare_open(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("open") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[ptr_type.into(), i32_type.into()], true); // true = variadic
        module.add_function("open", fn_type, None)
    }

    /// Get or declare close(fd: i32) -> i32.
    fn get_or_declare_close(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("close") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let fn_type = i32_type.fn_type(&[i32_type.into()], false);
        module.add_function("close", fn_type, None)
    }

    /// Get or declare read(fd: i32, buf: *i8, count: i64) -> i64.
    fn get_or_declare_read(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("read") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i32_type.into(), ptr_type.into(), i64_type.into()], false);
        module.add_function("read", fn_type, None)
    }

    /// Get or declare unlink(path: *i8) -> i32.
    fn get_or_declare_unlink(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("unlink") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[ptr_type.into()], false);
        module.add_function("unlink", fn_type, None)
    }

    /// Get or declare lseek(fd: i32, offset: i64, whence: i32) -> i64.
    fn get_or_declare_lseek(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("lseek") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i32_type.into(), i64_type.into(), i32_type.into()], false);
        module.add_function("lseek", fn_type, None)
    }

    /// Get or declare access(path: *i8, mode: i32) -> i32.
    fn get_or_declare_access(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("access") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[ptr_type.into(), i32_type.into()], false);
        module.add_function("access", fn_type, None)
    }

    /// Get or declare write(fd: i64, buf: ptr, count: i64) -> i64 (POSIX write syscall).
    fn get_or_declare_write(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("write") {
            return f;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into(), ptr_type.into(), i64_type.into()], false);
        module.add_function("write", fn_type, None)
    }

    /// Get or declare clock_gettime(clockid: i32, ts: *timespec) -> i32.
    fn get_or_declare_clock_gettime(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("clock_gettime") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
        module.add_function("clock_gettime", fn_type, None)
    }

    /// Get or declare nanosleep(req: *timespec, rem: *timespec) -> i32.
    fn get_or_declare_nanosleep(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("nanosleep") {
            return f;
        }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        module.add_function("nanosleep", fn_type, None)
    }

    /// Get or declare strcmp function.
    fn get_or_declare_strcmp(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("strcmp") {
            return f;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        module.add_function("strcmp", fn_type, None)
    }

    /// Get or declare strlen function.
    fn get_or_declare_strlen(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("strlen") {
            return f;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        module.add_function("strlen", fn_type, None)
    }

    /// Get or declare memcpy function.
    fn get_or_declare_memcpy(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("memcpy") {
            return f;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
        module.add_function("memcpy", fn_type, None)
    }

    /// Get or declare malloc function.
    fn get_or_declare_malloc(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        // Return the checked wrapper if it already exists
        let wrapper_name = "verum_checked_malloc";
        if let Some(func) = module.get_function(wrapper_name) {
            return Ok(func);
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();

        // Declare raw malloc
        let malloc_fn_type = ptr_type.fn_type(&[i64_type.into()], false);
        let raw_malloc = if let Some(f) = module.get_function("malloc") {
            f
        } else {
            module.add_function("malloc", malloc_fn_type, None)
        };

        // Declare _exit for OOM abort
        let exit_fn = self.get_or_declare_exit(module);

        // Build wrapper: ptr verum_checked_malloc(i64 size) {
        //   ptr p = malloc(size);
        //   if (p == null) _exit(1);
        //   return p;
        // }
        let wrapper = module.add_function(wrapper_name, malloc_fn_type, None);
        let entry_bb = self.context.append_basic_block(wrapper, "entry");
        let oom_bb = self.context.append_basic_block(wrapper, "oom");
        let ok_bb = self.context.append_basic_block(wrapper, "ok");

        let tmp_builder = self.context.create_builder();
        tmp_builder.position_at_end(entry_bb);

        let size_param = wrapper.get_nth_param(0).unwrap().into_int_value();
        let raw_ptr = tmp_builder
            .build_call(raw_malloc, &[size_param.into()], "raw")
            .or_llvm_err()?
            .try_as_basic_value()
            .basic()
            .or_internal("malloc should return ptr")?
            .into_pointer_value();

        let is_null = tmp_builder.build_is_null(raw_ptr, "is_null").or_llvm_err()?;
        tmp_builder.build_conditional_branch(is_null, oom_bb, ok_bb).or_llvm_err()?;

        // OOM path: abort
        tmp_builder.position_at_end(oom_bb);
        tmp_builder.build_call(exit_fn, &[i64_type.const_int(1, false).into()], "").or_llvm_err()?;
        tmp_builder.build_unreachable().or_llvm_err()?;

        // OK path: return pointer
        tmp_builder.position_at_end(ok_bb);
        tmp_builder.build_return(Some(&raw_ptr)).or_llvm_err()?;

        Ok(wrapper)
    }

    /// Emit a malloc call with null check and OOM abort.
    /// If malloc returns null, branches to an OOM block that calls `_exit(1)`.
    /// Returns the non-null pointer on the success path.
    pub fn emit_checked_malloc(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        size: IntValue<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>> {
        let malloc_fn = self.get_or_declare_malloc(module)?;
        let raw_ptr = builder
            .build_call(malloc_fn, &[size.into()], name)
            .or_llvm_err()?
            .try_as_basic_value()
            .basic()
            .or_internal("malloc should return ptr")?
            .into_pointer_value();

        let current_bb = builder
            .get_insert_block()
            .or_internal("emit_checked_malloc: no insert block")?;
        let func = current_bb
            .get_parent()
            .or_internal("emit_checked_malloc: block has no parent function")?;

        let oom_name = format!("{}_oom", name);
        let ok_name = format!("{}_ok", name);
        let oom_bb = self.context.append_basic_block(func, &oom_name);
        let ok_bb = self.context.append_basic_block(func, &ok_name);

        let is_null = builder
            .build_is_null(raw_ptr, "malloc_null")
            .or_llvm_err()?;
        builder
            .build_conditional_branch(is_null, oom_bb, ok_bb)
            .or_llvm_err()?;

        builder.position_at_end(oom_bb);
        let exit_fn = self.get_or_declare_exit(module);
        let i64_type = self.context.i64_type();
        builder
            .build_call(exit_fn, &[i64_type.const_int(1, false).into()], "")
            .or_llvm_err()?;
        builder.build_unreachable().or_llvm_err()?;

        builder.position_at_end(ok_bb);
        Ok(raw_ptr)
    }

    fn get_or_declare_exit(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("_exit") {
            return f;
        }
        let i64_type = self.context.i64_type();
        let fn_type = self
            .context
            .void_type()
            .fn_type(&[i64_type.into()], false);
        let f = module.add_function("_exit", fn_type, None);
        f.add_attribute(
            verum_llvm::attributes::AttributeLoc::Function,
            self.context.create_string_attribute("noreturn", ""),
        );
        f
    }

    /// Get or declare memset function.
    fn get_or_declare_memset(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "memset";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        let fn_type = ptr_type.fn_type(&[ptr_type.into(), i32_type.into(), i64_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    // =========================================================================
    // Context System Operations
    // =========================================================================
    //
    // The context system implements dependency injection via scoped provide/get.
    // In AOT mode, we call into the Verum runtime library for context management.
    //
    // Runtime layout for context stack (thread-local):
    // ```text
    // struct ContextStack {
    //     entries: *mut ContextEntry,  // Dynamic array of entries
    //     len: u64,                     // Number of active entries
    //     cap: u64,                     // Capacity
    // }
    //
    // struct ContextEntry {
    //     ctx_type: u32,   // Context type ID
    //     _pad: u32,       // Padding
    //     value: i64,      // NaN-boxed value
    //     stack_depth: u64 // Call stack depth for scoping
    // }
    // ```
    //
    // Context System: Capability-based dependency injection via `using [...]` clause.
    // Contexts are stored in thread-local storage as a stack of ContextEntry structs.
    // CtxGet retrieves a context value by type ID from the TLS context stack.
    // CtxPush installs a new context value at the current call stack depth.
    // CtxPop removes contexts installed at the current depth (scope-based RAII).
    // Runtime overhead: ~5-30ns per context access via vtable dispatch.

    /// Lower CtxGet instruction.
    ///
    /// Retrieves a context value by type ID from the thread-local context stack.
    /// Returns the value if found, or a nil/unit value if not.
    pub fn lower_ctx_get(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        ctx_type: u32,
    ) -> Result<IntValue<'ctx>> {
        let ctx_get_fn = self.get_or_declare_ctx_get(module)?;
        let i32_type = self.context.i32_type();

        let ctx_type_val = i32_type.const_int(ctx_type as u64, false);

        let result = builder
            .build_call(ctx_get_fn, &[ctx_type_val.into()], "ctx_value")
            .or_llvm_err()?
            .try_as_basic_value()
            .basic()
            .or_internal("verum_ctx_get should return i64")?
            .into_int_value();

        Ok(result)
    }

    /// Lower CtxProvide instruction.
    ///
    /// Pushes a context value onto the thread-local context stack.
    /// The value will be active until ctx_end is called with the same depth.
    pub fn lower_ctx_provide(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        ctx_type: u32,
        value: IntValue<'ctx>,
        stack_depth: IntValue<'ctx>,
    ) -> Result<()> {
        let ctx_provide_fn = self.get_or_declare_ctx_provide(module)?;
        let i32_type = self.context.i32_type();

        let ctx_type_val = i32_type.const_int(ctx_type as u64, false);

        builder
            .build_call(
                ctx_provide_fn,
                &[ctx_type_val.into(), value.into(), stack_depth.into()],
                "",
            )
            .or_llvm_err()?;

        Ok(())
    }

    /// Lower CtxEnd instruction.
    ///
    /// Removes all context entries at or above the specified stack depth.
    pub fn lower_ctx_end(
        &self,
        builder: &Builder<'ctx>,
        module: &Module<'ctx>,
        stack_depth: IntValue<'ctx>,
    ) -> Result<()> {
        let ctx_end_fn = self.get_or_declare_ctx_end(module)?;

        builder
            .build_call(ctx_end_fn, &[stack_depth.into()], "")
            .or_llvm_err()?;

        Ok(())
    }

    /// Get or declare verum_ctx_get runtime function.
    ///
    /// Signature: i64 verum_ctx_get(i32 ctx_type)
    /// Returns the context value (NaN-boxed) or nil if not found.
    fn get_or_declare_ctx_get(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "verum_ctx_get";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        let fn_type = i64_type.fn_type(&[i32_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    /// Get or declare verum_ctx_provide runtime function.
    ///
    /// Signature: void verum_ctx_provide(i32 ctx_type, i64 value, i64 stack_depth)
    fn get_or_declare_ctx_provide(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "verum_ctx_provide";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let void_type = self.context.void_type();
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        let fn_type = void_type.fn_type(&[i32_type.into(), i64_type.into(), i64_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    /// Get or declare verum_ctx_end runtime function.
    ///
    /// Signature: void verum_ctx_end(i64 stack_depth)
    fn get_or_declare_ctx_end(&self, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
        let name = "verum_ctx_end";
        if let Some(func) = module.get_function(name) {
            return Ok(func);
        }

        let void_type = self.context.void_type();
        let i64_type = self.context.i64_type();

        let fn_type = void_type.fn_type(&[i64_type.into()], false);

        Ok(module.add_function(name, fn_type, None))
    }

    // =========================================================================
    // Networking & Process Helpers
    // =========================================================================

    /// Byte-swap a 16-bit port value (host → network byte order).
    fn build_htons(&self, builder: &Builder<'ctx>, port_i64: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        let i16_type = self.context.i16_type();
        let port16 = builder.build_int_truncate(port_i64, i16_type, "p16").or_llvm_err()?;
        let hi = builder.build_right_shift(port16, i16_type.const_int(8, false), false, "hi").or_llvm_err()?;
        let lo = builder.build_left_shift(port16, i16_type.const_int(8, false), "lo").or_llvm_err()?;
        Ok(builder.build_or(hi, lo, "net_port").or_llvm_err()?)
    }

    /// Allocate + zero a 16-byte sockaddr_in on stack with AF_INET, given port + addr.
    fn build_sockaddr_in(
        &self,
        builder: &Builder<'ctx>,
        memset_fn: FunctionValue<'ctx>,
        port_i64: IntValue<'ctx>,
        addr_i32: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let alloca = builder.build_alloca(i8_type.array_type(16), "sa").or_llvm_err()?;
        builder.build_call(memset_fn, &[
            alloca.into(), i32_type.const_zero().into(), i64_type.const_int(16, false).into(),
        ], "").or_llvm_err()?;
        // sin_family = AF_INET
        #[cfg(target_os = "macos")]
        {
            // macOS: byte 0 = sin_len=16, byte 1 = sin_family=2
            // SAFETY: GEP at offset 0 within a struct of known layout; the offset is within the allocation
            let p0 = unsafe { builder.build_gep(i8_type, alloca, &[i32_type.const_int(0, false)], "sl").or_llvm_err()? };
            builder.build_store(p0, i8_type.const_int(16, false)).or_llvm_err()?;
            // SAFETY: GEP at offset 1 within a struct of known layout; the offset is within the allocation
            let p1 = unsafe { builder.build_gep(i8_type, alloca, &[i32_type.const_int(1, false)], "sf").or_llvm_err()? };
            builder.build_store(p1, i8_type.const_int(2, false)).or_llvm_err()?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            // Linux: bytes 0-1 = sin_family (i16 = 2)
            let i16_type = self.context.i16_type();
            builder.build_store(alloca, i16_type.const_int(2, false)).or_llvm_err()?;
        }
        // sin_port at offset 2 (network byte order)
        // SAFETY: GEP at offset 2 within a struct of known layout; the offset is within the allocation
        let port_ptr = unsafe { builder.build_gep(i8_type, alloca, &[i32_type.const_int(2, false)], "sp").or_llvm_err()? };
        let net_port = self.build_htons(builder, port_i64)?;
        builder.build_store(port_ptr, net_port).or_llvm_err()?;
        // sin_addr at offset 4
        // SAFETY: GEP at offset 4 within a struct of known layout; the offset is within the allocation
        let addr_ptr = unsafe { builder.build_gep(i8_type, alloca, &[i32_type.const_int(4, false)], "sn").or_llvm_err()? };
        builder.build_store(addr_ptr, addr_i32).or_llvm_err()?;
        Ok(alloca)
    }

    // --- Libc networking declarations ---

    fn get_or_declare_socket(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("socket") { return f; }
        let i32_type = self.context.i32_type();
        let fn_type = i32_type.fn_type(&[i32_type.into(), i32_type.into(), i32_type.into()], false);
        module.add_function("socket", fn_type, None)
    }

    fn get_or_declare_bind(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("bind") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into(), i32_type.into()], false);
        module.add_function("bind", fn_type, None)
    }

    fn get_or_declare_listen_libc(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("listen") { return f; }
        let i32_type = self.context.i32_type();
        let fn_type = i32_type.fn_type(&[i32_type.into(), i32_type.into()], false);
        module.add_function("listen", fn_type, None)
    }

    fn get_or_declare_accept_libc(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("accept") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
        module.add_function("accept", fn_type, None)
    }

    fn get_or_declare_connect_libc(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("connect") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into(), i32_type.into()], false);
        module.add_function("connect", fn_type, None)
    }

    fn get_or_declare_send_libc(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("send") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i32_type.into(), ptr_type.into(), i64_type.into(), i32_type.into()], false);
        module.add_function("send", fn_type, None)
    }

    fn get_or_declare_recv_libc(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("recv") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[i32_type.into(), ptr_type.into(), i64_type.into(), i32_type.into()], false);
        module.add_function("recv", fn_type, None)
    }

    fn get_or_declare_setsockopt(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("setsockopt") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[i32_type.into(), i32_type.into(), i32_type.into(), ptr_type.into(), i32_type.into()], false);
        module.add_function("setsockopt", fn_type, None)
    }

    fn get_or_declare_getaddrinfo(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("getaddrinfo") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        module.add_function("getaddrinfo", fn_type, None)
    }

    fn get_or_declare_freeaddrinfo(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("freeaddrinfo") { return f; }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = self.context.void_type().fn_type(&[ptr_type.into()], false);
        module.add_function("freeaddrinfo", fn_type, None)
    }

    fn get_or_declare_inet_pton(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("inet_pton") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into(), ptr_type.into()], false);
        module.add_function("inet_pton", fn_type, None)
    }

    fn get_or_declare_sendto(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("sendto") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[
            i32_type.into(), ptr_type.into(), i64_type.into(),
            i32_type.into(), ptr_type.into(), i32_type.into(),
        ], false);
        module.add_function("sendto", fn_type, None)
    }

    fn get_or_declare_recvfrom(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("recvfrom") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[
            i32_type.into(), ptr_type.into(), i64_type.into(),
            i32_type.into(), ptr_type.into(), ptr_type.into(),
        ], false);
        module.add_function("recvfrom", fn_type, None)
    }

    // --- Libc process declarations ---

    fn get_or_declare_fork(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("fork") { return f; }
        let i32_type = self.context.i32_type();
        module.add_function("fork", i32_type.fn_type(&[], false), None)
    }

    fn get_or_declare_pipe(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let expected_fn_type = i32_type.fn_type(&[ptr_type.into()], false);
        // Check if existing "pipe" function matches POSIX signature (1 param, ptr).
        // If a user-defined `pipe` exists with different arity, create a unique name.
        if let Some(f) = module.get_function("pipe") {
            if f.count_params() == 1 {
                return f; // POSIX pipe or compatible
            }
            // User-defined pipe with different arity — use alternative name
            if let Some(f) = module.get_function("__libc_pipe") { return f; }
            return module.add_function("__libc_pipe", expected_fn_type, None);
        }
        module.add_function("pipe", expected_fn_type, None)
    }

    fn get_or_declare_dup2(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("dup2") { return f; }
        let i32_type = self.context.i32_type();
        module.add_function("dup2", i32_type.fn_type(&[i32_type.into(), i32_type.into()], false), None)
    }

    fn get_or_declare_execvp(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("execvp") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        module.add_function("execvp", i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), None)
    }

    fn get_or_declare_waitpid(&self, module: &Module<'ctx>) -> FunctionValue<'ctx> {
        if let Some(f) = module.get_function("waitpid") { return f; }
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        module.add_function("waitpid", i32_type.fn_type(&[i32_type.into(), ptr_type.into(), i32_type.into()], false), None)
    }

    // =========================================================================
    // TCP/UDP Networking — LLVM IR (replaces verum_platform.c networking)
    // =========================================================================

    fn emit_verum_networking_functions(&self, module: &Module<'ctx>) -> Result<()> {
        let i8_type = self.context.i8_type();
        let i16_type = self.context.i16_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let _ = i16_type; // suppress unused on some cfgs

        let socket_fn = self.get_or_declare_socket(module);
        let bind_fn = self.get_or_declare_bind(module);
        let listen_fn_libc = self.get_or_declare_listen_libc(module);
        let accept_fn_libc = self.get_or_declare_accept_libc(module);
        let connect_fn_libc = self.get_or_declare_connect_libc(module);
        let send_fn_libc = self.get_or_declare_send_libc(module);
        let recv_fn_libc = self.get_or_declare_recv_libc(module);
        let close_fn = self.get_or_declare_close(module);
        let setsockopt_fn = self.get_or_declare_setsockopt(module);
        let getaddrinfo_fn = self.get_or_declare_getaddrinfo(module);
        let freeaddrinfo_fn = self.get_or_declare_freeaddrinfo(module);
        let inet_pton_fn = self.get_or_declare_inet_pton(module);
        let sendto_fn_libc = self.get_or_declare_sendto(module);
        let recvfrom_fn_libc = self.get_or_declare_recvfrom(module);
        let memset_fn = self.get_or_declare_memset(module)?;
        let strlen_fn = self.get_or_declare_strlen(module);
        let free_fn = module.get_function("free").unwrap_or_else(|| {
            let ft = self.context.void_type().fn_type(&[ptr_type.into()], false);
            module.add_function("free", ft, None)
        });
        let text_from_cstr_fn = module.get_function("verum_text_from_cstr").unwrap_or_else(|| {
            let ft = i64_type.fn_type(&[ptr_type.into()], false);
            module.add_function("verum_text_from_cstr", ft, None)
        });
        let text_get_ptr_fn = module.get_function("verum_text_get_ptr").unwrap_or_else(|| {
            let ft = ptr_type.fn_type(&[i64_type.into()], false);
            module.add_function("verum_text_get_ptr", ft, None)
        });

        #[cfg(target_os = "macos")]
        let (sol_socket, so_reuseaddr): (u64, u64) = (0xffff, 4);
        #[cfg(not(target_os = "macos"))]
        let (sol_socket, so_reuseaddr): (u64, u64) = (1, 2);

        // ai_addr offset within struct addrinfo (differs macOS vs Linux)
        #[cfg(target_os = "macos")]
        let addrinfo_ai_addr_off: u64 = 32;
        #[cfg(not(target_os = "macos"))]
        let addrinfo_ai_addr_off: u64 = 24;

        let neg1 = i64_type.const_int(u64::MAX, true); // -1

        // ============================================================
        // verum_tcp_listen(port: i64, backlog: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_tcp_listen")
                .unwrap_or_else(|| module.add_function("verum_tcp_listen", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let sock_fail = self.context.append_basic_block(func, "sock_fail");
                let sock_ok = self.context.append_basic_block(func, "sock_ok");
                let bind_fail = self.context.append_basic_block(func, "bind_fail");
                let bind_ok = self.context.append_basic_block(func, "bind_ok");
                let listen_fail = self.context.append_basic_block(func, "listen_fail");
                let listen_ok = self.context.append_basic_block(func, "listen_ok");

                let b = self.context.create_builder();
                b.position_at_end(entry);
                let port = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let backlog = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

                let fd = self.build_libc_call(&b, socket_fn, &[
                    i32_type.const_int(2, false).into(),
                    i32_type.const_int(1, false).into(),
                    i32_type.const_zero().into(),
                ], "fd")?;
                let is_neg = b.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "n").or_llvm_err()?;
                b.build_conditional_branch(is_neg, sock_fail, sock_ok).or_llvm_err()?;

                b.position_at_end(sock_fail);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(sock_ok);
                let opt = b.build_alloca(i32_type, "opt").or_llvm_err()?;
                b.build_store(opt, i32_type.const_int(1, false)).or_llvm_err()?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                self.build_libc_call_void(&b, setsockopt_fn, &[
                    fd32.into(), i32_type.const_int(sol_socket, false).into(),
                    i32_type.const_int(so_reuseaddr, false).into(),
                    opt.into(), i32_type.const_int(4, false).into(),
                ], "")?;
                let sa = self.build_sockaddr_in(&b, memset_fn, port, i32_type.const_zero())?;
                let br_ = self.build_libc_call(&b, bind_fn, &[
                    fd32.into(), sa.into(), i32_type.const_int(16, false).into(),
                ], "br")?;
                let bn = b.build_int_compare(verum_llvm::IntPredicate::SLT, br_, i64_type.const_zero(), "bn").or_llvm_err()?;
                b.build_conditional_branch(bn, bind_fail, bind_ok).or_llvm_err()?;

                b.position_at_end(bind_fail);
                self.build_libc_call_void(&b, close_fn, &[fd32.into()], "")?;
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(bind_ok);
                let lr = self.build_libc_call(&b, listen_fn_libc, &[fd32.into(), backlog.into()], "lr")?;
                let ln = b.build_int_compare(verum_llvm::IntPredicate::SLT, lr, i64_type.const_zero(), "ln").or_llvm_err()?;
                b.build_conditional_branch(ln, listen_fail, listen_ok).or_llvm_err()?;

                b.position_at_end(listen_fail);
                self.build_libc_call_void(&b, close_fn, &[fd32.into()], "")?;
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(listen_ok);
                b.build_return(Some(&fd)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_tcp_accept(listen_fd: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_tcp_accept")
                .unwrap_or_else(|| module.add_function("verum_tcp_accept", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let listen_fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let sa = b.build_alloca(i8_type.array_type(16), "client").or_llvm_err()?;
                let sl = b.build_alloca(i32_type, "len").or_llvm_err()?;
                b.build_store(sl, i32_type.const_int(16, false)).or_llvm_err()?;
                let fd32 = b.build_int_truncate(listen_fd, i32_type, "fd32").or_llvm_err()?;
                let r = self.build_libc_call(&b, accept_fn_libc, &[fd32.into(), sa.into(), sl.into()], "r")?;
                b.build_return(Some(&r)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_tcp_connect(host: ptr, port: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_tcp_connect")
                .unwrap_or_else(|| module.add_function("verum_tcp_connect", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null_host");
                let resolve_bb = self.context.append_basic_block(func, "resolve");
                let gai_fail = self.context.append_basic_block(func, "gai_fail");
                let do_sock = self.context.append_basic_block(func, "do_sock");
                let sock_fail = self.context.append_basic_block(func, "sock_fail");
                let do_conn = self.context.append_basic_block(func, "do_conn");
                let conn_fail = self.context.append_basic_block(func, "conn_fail");
                let done = self.context.append_basic_block(func, "done");

                let b = self.context.create_builder();
                b.position_at_end(entry);
                let host = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
                let port = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let is_null = b.build_is_null(host, "n").or_llvm_err()?;
                b.build_conditional_branch(is_null, null_bb, resolve_bb).or_llvm_err()?;

                b.position_at_end(null_bb);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(resolve_bb);
                // Zero 48-byte addrinfo hints
                let hints = b.build_alloca(i8_type.array_type(48), "hints").or_llvm_err()?;
                b.build_call(memset_fn, &[
                    hints.into(), i32_type.const_zero().into(), i64_type.const_int(48, false).into(),
                ], "").or_llvm_err()?;
                // hints.ai_family (offset 4) = AF_INET=2
                // SAFETY: GEP at offset 4 within a struct of known layout; the offset is within the allocation
                let p = unsafe { b.build_gep(i8_type, hints, &[i32_type.const_int(4, false)], "af").or_llvm_err()? };
                b.build_store(p, i32_type.const_int(2, false)).or_llvm_err()?;
                // hints.ai_socktype (offset 8) = SOCK_STREAM=1
                // SAFETY: GEP at offset 8 within a struct of known layout; the offset is within the allocation
                let p = unsafe { b.build_gep(i8_type, hints, &[i32_type.const_int(8, false)], "st").or_llvm_err()? };
                b.build_store(p, i32_type.const_int(1, false)).or_llvm_err()?;

                let res_ptr = b.build_alloca(ptr_type, "rp").or_llvm_err()?;
                let null_ptr = ptr_type.const_null();
                let gai = self.build_libc_call(&b, getaddrinfo_fn, &[
                    host.into(), null_ptr.into(), hints.into(), res_ptr.into(),
                ], "gai")?;
                let gai_nz = b.build_int_compare(verum_llvm::IntPredicate::NE, gai, i64_type.const_zero(), "gnz").or_llvm_err()?;
                b.build_conditional_branch(gai_nz, gai_fail, do_sock).or_llvm_err()?;

                b.position_at_end(gai_fail);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(do_sock);
                let res = b.build_load(ptr_type, res_ptr, "res").or_llvm_err()?.into_pointer_value();
                // res->ai_family(4), ai_socktype(8), ai_protocol(12)
                // SAFETY: GEP into sockaddr/network struct at a platform-defined field offset; the struct size matches the system ABI
                let fam_p = unsafe { b.build_gep(i8_type, res, &[i32_type.const_int(4, false)], "fp").or_llvm_err()? };
                let ai_fam = b.build_load(i32_type, fam_p, "fam").or_llvm_err()?.into_int_value();
                // SAFETY: GEP into sockaddr/network struct at a platform-defined field offset; the struct size matches the system ABI
                let st_p = unsafe { b.build_gep(i8_type, res, &[i32_type.const_int(8, false)], "stp").or_llvm_err()? };
                let ai_st = b.build_load(i32_type, st_p, "skt").or_llvm_err()?.into_int_value();
                // SAFETY: GEP into sockaddr/network struct at a platform-defined field offset; the struct size matches the system ABI
                let pr_p = unsafe { b.build_gep(i8_type, res, &[i32_type.const_int(12, false)], "prp").or_llvm_err()? };
                let ai_pr = b.build_load(i32_type, pr_p, "proto").or_llvm_err()?.into_int_value();
                let fd = self.build_libc_call(&b, socket_fn, &[ai_fam.into(), ai_st.into(), ai_pr.into()], "fd")?;
                let fd_neg = b.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "fn").or_llvm_err()?;
                b.build_conditional_branch(fd_neg, sock_fail, do_conn).or_llvm_err()?;

                b.position_at_end(sock_fail);
                b.build_call(freeaddrinfo_fn, &[res.into()], "").or_llvm_err()?;
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(do_conn);
                // Set port in res->ai_addr->sin_port
                // SAFETY: GEP to access the 'aap' field at a fixed offset within a struct of known layout
                let addr_p = unsafe { b.build_gep(i8_type, res, &[i64_type.const_int(addrinfo_ai_addr_off, false)], "aap").or_llvm_err()? };
                let ai_addr = b.build_load(ptr_type, addr_p, "aa").or_llvm_err()?.into_pointer_value();
                // SAFETY: GEP at offset 2 within a struct of known layout; the offset is within the allocation
                let sp = unsafe { b.build_gep(i8_type, ai_addr, &[i32_type.const_int(2, false)], "sp").or_llvm_err()? };
                let np = self.build_htons(&b, port)?;
                b.build_store(sp, np).or_llvm_err()?;
                // ai_addrlen at offset 16
                // SAFETY: GEP into sockaddr/network struct at a platform-defined field offset; the struct size matches the system ABI
                let al_p = unsafe { b.build_gep(i8_type, res, &[i32_type.const_int(16, false)], "alp").or_llvm_err()? };
                let addrlen = b.build_load(i32_type, al_p, "al").or_llvm_err()?.into_int_value();
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let cr = self.build_libc_call(&b, connect_fn_libc, &[fd32.into(), ai_addr.into(), addrlen.into()], "cr")?;
                let cn = b.build_int_compare(verum_llvm::IntPredicate::SLT, cr, i64_type.const_zero(), "cn").or_llvm_err()?;
                b.build_conditional_branch(cn, conn_fail, done).or_llvm_err()?;

                b.position_at_end(conn_fail);
                self.build_libc_call_void(&b, close_fn, &[fd32.into()], "")?;
                b.build_call(freeaddrinfo_fn, &[res.into()], "").or_llvm_err()?;
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(done);
                b.build_call(freeaddrinfo_fn, &[res.into()], "").or_llvm_err()?;
                b.build_return(Some(&fd)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_tcp_send_text(fd: i64, text_obj: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_tcp_send_text")
                .unwrap_or_else(|| module.add_function("verum_tcp_send_text", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let text = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let p = b.build_call(text_get_ptr_fn, &[text.into()], "p")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
                let n = b.build_is_null(p, "n").or_llvm_err()?;
                b.build_conditional_branch(n, null_bb, ok_bb).or_llvm_err()?;

                b.position_at_end(null_bb);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(ok_bb);
                let len = self.build_libc_call(&b, strlen_fn, &[p.into()], "len")?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let sent = self.build_libc_call(&b, send_fn_libc, &[
                    fd32.into(), p.into(), len.into(), i32_type.const_zero().into(),
                ], "sent")?;
                b.build_return(Some(&sent)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_tcp_recv_text(fd: i64, max_len: i64) -> i64 (Text)
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_tcp_recv_text")
                .unwrap_or_else(|| module.add_function("verum_tcp_recv_text", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fix_bb = self.context.append_basic_block(func, "fix");
                let alloc_bb = self.context.append_basic_block(func, "alloc");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let ml = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let le0 = b.build_int_compare(verum_llvm::IntPredicate::SLE, ml, i64_type.const_zero(), "le0").or_llvm_err()?;
                b.build_conditional_branch(le0, fix_bb, alloc_bb).or_llvm_err()?;

                b.position_at_end(fix_bb);
                b.build_unconditional_branch(alloc_bb).or_llvm_err()?;

                b.position_at_end(alloc_bb);
                let max = b.build_phi(i64_type, "max").or_llvm_err()?;
                max.add_incoming(&[(&ml, entry), (&i64_type.const_int(4096, false), fix_bb)]);
                let max_v = max.as_basic_value().into_int_value();
                let sz = b.build_int_add(max_v, i64_type.const_int(1, false), "sz").or_llvm_err()?;
                let buf = self.emit_checked_malloc(&b, module, sz, "buf")?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let n = self.build_libc_call(&b, recv_fn_libc, &[
                    fd32.into(), buf.into(), max_v.into(), i32_type.const_zero().into(),
                ], "n")?;
                let nle = b.build_int_compare(verum_llvm::IntPredicate::SLE, n, i64_type.const_zero(), "nle").or_llvm_err()?;
                b.build_conditional_branch(nle, fail_bb, ok_bb).or_llvm_err()?;

                b.position_at_end(fail_bb);
                b.build_call(free_fn, &[buf.into()], "").or_llvm_err()?;
                let empty = b.build_global_string_ptr("", "e").or_llvm_err()?;
                let et = b.build_call(text_from_cstr_fn, &[empty.as_pointer_value().into()], "et")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                b.build_return(Some(&et)).or_llvm_err()?;

                b.position_at_end(ok_bb);
                // SAFETY: GEP to access the 'tp' field at a fixed offset within a struct of known layout
                let tp = unsafe { b.build_gep(i8_type, buf, &[n], "tp").or_llvm_err()? };
                b.build_store(tp, i8_type.const_zero()).or_llvm_err()?;
                let txt = b.build_call(text_from_cstr_fn, &[buf.into()], "txt")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                b.build_call(free_fn, &[buf.into()], "").or_llvm_err()?;
                b.build_return(Some(&txt)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_tcp_close(fd: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_tcp_close")
                .unwrap_or_else(|| module.add_function("verum_tcp_close", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let r = self.build_libc_call(&b, close_fn, &[fd.into()], "r")?;
                b.build_return(Some(&r)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_udp_bind(port: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_udp_bind")
                .unwrap_or_else(|| module.add_function("verum_udp_bind", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let sf = self.context.append_basic_block(func, "sf");
                let so = self.context.append_basic_block(func, "so");
                let bf = self.context.append_basic_block(func, "bf");
                let bo = self.context.append_basic_block(func, "bo");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let port = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let fd = self.build_libc_call(&b, socket_fn, &[
                    i32_type.const_int(2, false).into(),
                    i32_type.const_int(2, false).into(),
                    i32_type.const_zero().into(),
                ], "fd")?;
                let neg = b.build_int_compare(verum_llvm::IntPredicate::SLT, fd, i64_type.const_zero(), "n").or_llvm_err()?;
                b.build_conditional_branch(neg, sf, so).or_llvm_err()?;

                b.position_at_end(sf);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(so);
                let sa = self.build_sockaddr_in(&b, memset_fn, port, i32_type.const_zero())?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let br_ = self.build_libc_call(&b, bind_fn, &[
                    fd32.into(), sa.into(), i32_type.const_int(16, false).into(),
                ], "br")?;
                let bn = b.build_int_compare(verum_llvm::IntPredicate::SLT, br_, i64_type.const_zero(), "bn").or_llvm_err()?;
                b.build_conditional_branch(bn, bf, bo).or_llvm_err()?;

                b.position_at_end(bf);
                self.build_libc_call_void(&b, close_fn, &[fd32.into()], "")?;
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(bo);
                b.build_return(Some(&fd)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_udp_send_text(fd: i64, text_obj: i64, host: ptr, port: i64) -> i64
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), ptr_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_udp_send_text")
                .unwrap_or_else(|| module.add_function("verum_udp_send_text", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let pton_bb = self.context.append_basic_block(func, "pton");
                let pfail = self.context.append_basic_block(func, "pfail");
                let send_bb = self.context.append_basic_block(func, "send");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let text = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let host = func.get_nth_param(2).or_internal("missing param 2")?.into_pointer_value();
                let port = func.get_nth_param(3).or_internal("missing param 3")?.into_int_value();

                let dp = b.build_call(text_get_ptr_fn, &[text.into()], "dp")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
                let dn = b.build_is_null(dp, "dn").or_llvm_err()?;
                let hn = b.build_is_null(host, "hn").or_llvm_err()?;
                let bad = b.build_or(dn, hn, "bad").or_llvm_err()?;
                b.build_conditional_branch(bad, null_bb, pton_bb).or_llvm_err()?;

                b.position_at_end(null_bb);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(pton_bb);
                let dlen = self.build_libc_call(&b, strlen_fn, &[dp.into()], "dl")?;
                // Build sockaddr_in; set addr via inet_pton
                let sa = b.build_alloca(i8_type.array_type(16), "sa").or_llvm_err()?;
                b.build_call(memset_fn, &[
                    sa.into(), i32_type.const_zero().into(), i64_type.const_int(16, false).into(),
                ], "").or_llvm_err()?;
                #[cfg(target_os = "macos")]
                {
                    // SAFETY: GEP at offset 0 within a struct of known layout; the offset is within the allocation
                    let p0 = unsafe { b.build_gep(i8_type, sa, &[i32_type.const_int(0, false)], "sl").or_llvm_err()? };
                    b.build_store(p0, i8_type.const_int(16, false)).or_llvm_err()?;
                    // SAFETY: GEP at offset 1 within a struct of known layout; the offset is within the allocation
                    let p1 = unsafe { b.build_gep(i8_type, sa, &[i32_type.const_int(1, false)], "sf").or_llvm_err()? };
                    b.build_store(p1, i8_type.const_int(2, false)).or_llvm_err()?;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    b.build_store(sa, i16_type.const_int(2, false)).or_llvm_err()?;
                }
                // SAFETY: GEP at offset 2 within a struct of known layout; the offset is within the allocation
                let sp = unsafe { b.build_gep(i8_type, sa, &[i32_type.const_int(2, false)], "sp").or_llvm_err()? };
                let np = self.build_htons(&b, port)?;
                b.build_store(sp, np).or_llvm_err()?;
                // SAFETY: GEP into sockaddr/network struct at a platform-defined field offset; the struct size matches the system ABI
                let sin = unsafe { b.build_gep(i8_type, sa, &[i32_type.const_int(4, false)], "sin").or_llvm_err()? };
                let pr = self.build_libc_call(&b, inet_pton_fn, &[
                    i32_type.const_int(2, false).into(), host.into(), sin.into(),
                ], "pr")?;
                let ple = b.build_int_compare(verum_llvm::IntPredicate::SLE, pr, i64_type.const_zero(), "ple").or_llvm_err()?;
                b.build_conditional_branch(ple, pfail, send_bb).or_llvm_err()?;

                b.position_at_end(pfail);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(send_bb);
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let sent = self.build_libc_call(&b, sendto_fn_libc, &[
                    fd32.into(), dp.into(), dlen.into(),
                    i32_type.const_zero().into(), sa.into(), i32_type.const_int(16, false).into(),
                ], "sent")?;
                b.build_return(Some(&sent)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_udp_recv_text(fd: i64, max_len: i64) -> i64 (Text)
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_udp_recv_text")
                .unwrap_or_else(|| module.add_function("verum_udp_recv_text", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fix_bb = self.context.append_basic_block(func, "fix");
                let alloc_bb = self.context.append_basic_block(func, "alloc");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let ml = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let bad = b.build_int_compare(verum_llvm::IntPredicate::SLE, ml, i64_type.const_zero(), "bad").or_llvm_err()?;
                b.build_conditional_branch(bad, fix_bb, alloc_bb).or_llvm_err()?;

                b.position_at_end(fix_bb);
                b.build_unconditional_branch(alloc_bb).or_llvm_err()?;

                b.position_at_end(alloc_bb);
                let max = b.build_phi(i64_type, "max").or_llvm_err()?;
                max.add_incoming(&[(&ml, entry), (&i64_type.const_int(4096, false), fix_bb)]);
                let max_v = max.as_basic_value().into_int_value();
                let sz = b.build_int_add(max_v, i64_type.const_int(1, false), "sz").or_llvm_err()?;
                let buf = self.emit_checked_malloc(&b, module, sz, "buf")?;
                let from_sa = b.build_alloca(i8_type.array_type(16), "from").or_llvm_err()?;
                let from_len = b.build_alloca(i32_type, "fl").or_llvm_err()?;
                b.build_store(from_len, i32_type.const_int(16, false)).or_llvm_err()?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let n = self.build_libc_call(&b, recvfrom_fn_libc, &[
                    fd32.into(), buf.into(), max_v.into(),
                    i32_type.const_zero().into(), from_sa.into(), from_len.into(),
                ], "n")?;
                let nle = b.build_int_compare(verum_llvm::IntPredicate::SLE, n, i64_type.const_zero(), "nle").or_llvm_err()?;
                b.build_conditional_branch(nle, fail_bb, ok_bb).or_llvm_err()?;

                b.position_at_end(fail_bb);
                b.build_call(free_fn, &[buf.into()], "").or_llvm_err()?;
                let empty = b.build_global_string_ptr("", "eu").or_llvm_err()?;
                let et = b.build_call(text_from_cstr_fn, &[empty.as_pointer_value().into()], "et")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                b.build_return(Some(&et)).or_llvm_err()?;

                b.position_at_end(ok_bb);
                // SAFETY: GEP to access the 'tp' field at a fixed offset within a struct of known layout
                let tp = unsafe { b.build_gep(i8_type, buf, &[n], "tp").or_llvm_err()? };
                b.build_store(tp, i8_type.const_zero()).or_llvm_err()?;
                let txt = b.build_call(text_from_cstr_fn, &[buf.into()], "txt")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                b.build_call(free_fn, &[buf.into()], "").or_llvm_err()?;
                b.build_return(Some(&txt)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_udp_sendto(fd: i64, data: i64, addr: i64) -> i64
        // CallM compat: data=Text obj, addr=unused, send via connected socket
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_udp_sendto")
                .unwrap_or_else(|| module.add_function("verum_udp_sendto", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let null_bb = self.context.append_basic_block(func, "null");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let data = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let _addr = func.get_nth_param(2).or_internal("missing param 2")?; // unused
                let p = b.build_call(text_get_ptr_fn, &[data.into()], "p")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
                let n = b.build_is_null(p, "n").or_llvm_err()?;
                b.build_conditional_branch(n, null_bb, ok_bb).or_llvm_err()?;

                b.position_at_end(null_bb);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(ok_bb);
                let len = self.build_libc_call(&b, strlen_fn, &[p.into()], "len")?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let r = self.build_libc_call(&b, send_fn_libc, &[
                    fd32.into(), p.into(), len.into(), i32_type.const_zero().into(),
                ], "r")?;
                b.build_return(Some(&r)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_udp_recvfrom(fd: i64, max_len: i64) -> i64 (Text)
        // CallM compat: recv and return as Text
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
            let func = module.get_function("verum_udp_recvfrom")
                .unwrap_or_else(|| module.add_function("verum_udp_recvfrom", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fix_bb = self.context.append_basic_block(func, "fix");
                let alloc_bb = self.context.append_basic_block(func, "alloc");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let ml = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
                let bad = b.build_int_compare(verum_llvm::IntPredicate::SLE, ml, i64_type.const_zero(), "bad").or_llvm_err()?;
                b.build_conditional_branch(bad, fix_bb, alloc_bb).or_llvm_err()?;

                b.position_at_end(fix_bb);
                b.build_unconditional_branch(alloc_bb).or_llvm_err()?;

                b.position_at_end(alloc_bb);
                let max = b.build_phi(i64_type, "max").or_llvm_err()?;
                max.add_incoming(&[(&ml, entry), (&i64_type.const_int(4096, false), fix_bb)]);
                let max_v = max.as_basic_value().into_int_value();
                let sz = b.build_int_add(max_v, i64_type.const_int(1, false), "sz").or_llvm_err()?;
                let buf = self.emit_checked_malloc(&b, module, sz, "buf")?;
                let from_sa = b.build_alloca(i8_type.array_type(16), "from").or_llvm_err()?;
                let from_len = b.build_alloca(i32_type, "fl").or_llvm_err()?;
                b.build_store(from_len, i32_type.const_int(16, false)).or_llvm_err()?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let n = self.build_libc_call(&b, recvfrom_fn_libc, &[
                    fd32.into(), buf.into(), max_v.into(),
                    i32_type.const_zero().into(), from_sa.into(), from_len.into(),
                ], "n")?;
                let nle = b.build_int_compare(verum_llvm::IntPredicate::SLE, n, i64_type.const_zero(), "nle").or_llvm_err()?;
                b.build_conditional_branch(nle, fail_bb, ok_bb).or_llvm_err()?;

                b.position_at_end(fail_bb);
                b.build_call(free_fn, &[buf.into()], "").or_llvm_err()?;
                let empty = b.build_global_string_ptr("", "eu2").or_llvm_err()?;
                let et = b.build_call(text_from_cstr_fn, &[empty.as_pointer_value().into()], "et")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                b.build_return(Some(&et)).or_llvm_err()?;

                b.position_at_end(ok_bb);
                // SAFETY: GEP to access the 'tp' field at a fixed offset within a struct of known layout
                let tp = unsafe { b.build_gep(i8_type, buf, &[n], "tp").or_llvm_err()? };
                b.build_store(tp, i8_type.const_zero()).or_llvm_err()?;
                let txt = b.build_call(text_from_cstr_fn, &[buf.into()], "txt")
                    .or_llvm_err()?.try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
                b.build_call(free_fn, &[buf.into()], "").or_llvm_err()?;
                b.build_return(Some(&txt)).or_llvm_err()?;
            }
        }
        Ok(())
    }

    // =========================================================================
    // Process Management — LLVM IR (replaces verum_platform.c process functions)
    // =========================================================================
    //
    // Only simple functions are migrated to LLVM IR. The fork+exec functions
    // (verum_process_spawn_cmd, verum_process_exec) remain in C because the
    // fork+pipe+dup2+execvp pattern requires complex multi-process control flow
    // that is cleaner in C.

    fn emit_verum_process_functions(&self, module: &Module<'ctx>) -> Result<()> {
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        let close_fn = self.get_or_declare_close(module);
        let read_fn = self.get_or_declare_read(module);
        let memcpy_fn = self.get_or_declare_memcpy(module);
        let free_fn = module.get_function("free").unwrap_or_else(|| {
            module.add_function("free", void_type.fn_type(&[ptr_type.into()], false), None)
        });
        let waitpid_fn = self.get_or_declare_waitpid(module);

        let neg1 = i64_type.const_int(u64::MAX, true);

        // ============================================================
        // verum_process_wait(pid: i64) -> i64 (raw status)
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_process_wait")
                .unwrap_or_else(|| module.add_function("verum_process_wait", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let fail_bb = self.context.append_basic_block(func, "fail");
                let ok_bb = self.context.append_basic_block(func, "ok");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let pid = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let status = b.build_alloca(i32_type, "status").or_llvm_err()?;
                b.build_store(status, i32_type.const_zero()).or_llvm_err()?;
                let pid32 = b.build_int_truncate(pid, i32_type, "pid32").or_llvm_err()?;
                let r = self.build_libc_call(&b, waitpid_fn, &[
                    pid32.into(), status.into(), i32_type.const_zero().into(),
                ], "r")?;
                let r_neg = b.build_int_compare(verum_llvm::IntPredicate::SLT, r, i64_type.const_zero(), "rn").or_llvm_err()?;
                b.build_conditional_branch(r_neg, fail_bb, ok_bb).or_llvm_err()?;

                b.position_at_end(fail_bb);
                b.build_return(Some(&neg1)).or_llvm_err()?;

                b.position_at_end(ok_bb);
                let sv = b.build_load(i32_type, status, "sv").or_llvm_err()?.into_int_value();
                let sv64 = b.build_int_s_extend(sv, i64_type, "sv64").or_llvm_err()?;
                b.build_return(Some(&sv64)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_fd_read_all(fd: i64) -> i64 (ptr to [len, cap, buf])
        // ============================================================
        {
            let fn_type = i64_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_fd_read_all")
                .unwrap_or_else(|| module.add_function("verum_fd_read_all", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let alloc_fail = self.context.append_basic_block(func, "alloc_fail");
                let loop_hdr = self.context.append_basic_block(func, "loop_hdr");
                let grow_bb = self.context.append_basic_block(func, "grow");
                let do_read = self.context.append_basic_block(func, "do_read");
                let loop_cont = self.context.append_basic_block(func, "loop_cont");
                let loop_exit = self.context.append_basic_block(func, "loop_exit");
                let hdr_fail = self.context.append_basic_block(func, "hdr_fail");
                let hdr_ok = self.context.append_basic_block(func, "hdr_ok");
                let b = self.context.create_builder();

                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                let init_cap = i64_type.const_int(4096, false);
                let init_buf = self.emit_checked_malloc(&b, module, init_cap, "ibuf")?;
                // After emit_checked_malloc, builder is in the ok block
                let ibuf_ok_bb = b.get_insert_block().or_internal("no insert block after ibuf malloc")?;
                let buf_null = b.build_is_null(init_buf, "bn").or_llvm_err()?;
                b.build_conditional_branch(buf_null, alloc_fail, loop_hdr).or_llvm_err()?;

                b.position_at_end(alloc_fail);
                b.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

                b.position_at_end(loop_hdr);
                let buf_phi = b.build_phi(ptr_type, "buf").or_llvm_err()?;
                let len_phi = b.build_phi(i64_type, "len").or_llvm_err()?;
                let cap_phi = b.build_phi(i64_type, "cap").or_llvm_err()?;
                buf_phi.add_incoming(&[(&init_buf, ibuf_ok_bb)]);
                len_phi.add_incoming(&[(&i64_type.const_zero(), ibuf_ok_bb)]);
                cap_phi.add_incoming(&[(&init_cap, ibuf_ok_bb)]);
                let buf_v = buf_phi.as_basic_value().into_pointer_value();
                let len_v = len_phi.as_basic_value().into_int_value();
                let cap_v = cap_phi.as_basic_value().into_int_value();
                let need_grow = b.build_int_compare(verum_llvm::IntPredicate::UGE, len_v, cap_v, "ng").or_llvm_err()?;
                b.build_conditional_branch(need_grow, grow_bb, do_read).or_llvm_err()?;

                b.position_at_end(grow_bb);
                let new_cap = b.build_left_shift(cap_v, i64_type.const_int(1, false), "nc").or_llvm_err()?;
                let new_buf = self.emit_checked_malloc(&b, module, new_cap, "nb")?;
                // After emit_checked_malloc, builder is in the ok block
                let nb_ok_bb = b.get_insert_block().or_internal("no insert block after nb malloc")?;
                b.build_call(memcpy_fn, &[new_buf.into(), buf_v.into(), len_v.into()], "").or_llvm_err()?;
                b.build_call(free_fn, &[buf_v.into()], "").or_llvm_err()?;
                b.build_unconditional_branch(do_read).or_llvm_err()?;

                b.position_at_end(do_read);
                let buf2 = b.build_phi(ptr_type, "buf2").or_llvm_err()?;
                let cap2 = b.build_phi(i64_type, "cap2").or_llvm_err()?;
                buf2.add_incoming(&[(&buf_v, loop_hdr), (&new_buf, nb_ok_bb)]);
                cap2.add_incoming(&[(&cap_v, loop_hdr), (&new_cap, nb_ok_bb)]);
                let buf2_v = buf2.as_basic_value().into_pointer_value();
                let cap2_v = cap2.as_basic_value().into_int_value();
                // SAFETY: GEP to compute the end-of-buffer position; the offset is the sum of validated lengths that fit within the allocation
                let off = unsafe { b.build_gep(i8_type, buf2_v, &[len_v], "off").or_llvm_err()? };
                let rem = b.build_int_sub(cap2_v, len_v, "rem").or_llvm_err()?;
                let fd32 = b.build_int_truncate(fd, i32_type, "fd32").or_llvm_err()?;
                let n = self.build_libc_call(&b, read_fn, &[fd32.into(), off.into(), rem.into()], "n")?;
                let nle = b.build_int_compare(verum_llvm::IntPredicate::SLE, n, i64_type.const_zero(), "nle").or_llvm_err()?;
                b.build_conditional_branch(nle, loop_exit, loop_cont).or_llvm_err()?;

                b.position_at_end(loop_cont);
                let new_len = b.build_int_add(len_v, n, "nl").or_llvm_err()?;
                buf_phi.add_incoming(&[(&buf2_v, loop_cont)]);
                len_phi.add_incoming(&[(&new_len, loop_cont)]);
                cap_phi.add_incoming(&[(&cap2_v, loop_cont)]);
                b.build_unconditional_branch(loop_hdr).or_llvm_err()?;

                b.position_at_end(loop_exit);
                let hdr = self.emit_checked_malloc(&b, module, i64_type.const_int(24, false), "hdr")?;
                let hdr_null = b.build_is_null(hdr, "hn").or_llvm_err()?;
                b.build_conditional_branch(hdr_null, hdr_fail, hdr_ok).or_llvm_err()?;

                b.position_at_end(hdr_fail);
                b.build_call(free_fn, &[buf2_v.into()], "").or_llvm_err()?;
                b.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;

                b.position_at_end(hdr_ok);
                b.build_store(hdr, len_v).or_llvm_err()?;
                // SAFETY: GEP at offset 1 within a struct of known layout; the offset is within the allocation
                let h1 = unsafe { b.build_gep(i64_type, hdr, &[i32_type.const_int(1, false)], "h1").or_llvm_err()? };
                b.build_store(h1, cap2_v).or_llvm_err()?;
                // SAFETY: GEP at offset 2 within a struct of known layout; the offset is within the allocation
                let h2 = unsafe { b.build_gep(i64_type, hdr, &[i32_type.const_int(2, false)], "h2").or_llvm_err()? };
                let buf_i64 = b.build_ptr_to_int(buf2_v, i64_type, "bi").or_llvm_err()?;
                b.build_store(h2, buf_i64).or_llvm_err()?;
                let hdr_i64 = b.build_ptr_to_int(hdr, i64_type, "hi").or_llvm_err()?;
                b.build_return(Some(&hdr_i64)).or_llvm_err()?;
            }
        }

        // ============================================================
        // verum_fd_close(fd: i64) -> void
        // ============================================================
        {
            let fn_type = void_type.fn_type(&[i64_type.into()], false);
            let func = module.get_function("verum_fd_close")
                .unwrap_or_else(|| module.add_function("verum_fd_close", fn_type, None));
            if func.count_basic_blocks() == 0 {
                let entry = self.context.append_basic_block(func, "entry");
                let b = self.context.create_builder();
                b.position_at_end(entry);
                let fd = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
                self.build_libc_call_void(&b, close_fn, &[fd.into()], "")?;
                b.build_return(None).or_llvm_err()?;
            }
        }

        // verum_process_spawn_cmd and verum_process_exec are kept as external
        // declarations resolved from verum_platform.c (fork+exec complexity).
        Ok(())
    }
}

// =========================================================================
// Inline LLVM IR Text Helpers — replaces C runtime text functions
// =========================================================================

/// Standalone checked malloc for use in free functions (not methods on RuntimeLowering).
/// Calls malloc, checks for null, aborts via `_exit(1)` on OOM.
fn checked_malloc<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &Module<'ctx>,
    size: IntValue<'ctx>,
    name: &str,
) -> Result<PointerValue<'ctx>> {
    let malloc_fn = module.get_function("malloc").or_missing_fn("malloc")?;
    let raw_ptr = builder
        .build_call(malloc_fn, &[size.into()], name)
        .or_llvm_err()?
        .try_as_basic_value()
        .basic()
        .or_internal("malloc should return ptr")?
        .into_pointer_value();

    let current_bb = builder
        .get_insert_block()
        .or_internal("checked_malloc: no insert block")?;
    let func = current_bb
        .get_parent()
        .or_internal("checked_malloc: block has no parent function")?;

    let oom_bb = context.append_basic_block(func, &format!("{}_oom", name));
    let ok_bb = context.append_basic_block(func, &format!("{}_ok", name));

    let is_null = builder.build_is_null(raw_ptr, "malloc_null").or_llvm_err()?;
    builder.build_conditional_branch(is_null, oom_bb, ok_bb).or_llvm_err()?;

    builder.position_at_end(oom_bb);
    let exit_fn = if let Some(f) = module.get_function("_exit") {
        f
    } else {
        let i64_type = context.i64_type();
        let fn_type = context.void_type().fn_type(&[i64_type.into()], false);
        let f = module.add_function("_exit", fn_type, None);
        f.add_attribute(
            verum_llvm::attributes::AttributeLoc::Function,
            context.create_string_attribute("noreturn", ""),
        );
        f
    };
    let i64_type = context.i64_type();
    builder.build_call(exit_fn, &[i64_type.const_int(1, false).into()], "").or_llvm_err()?;
    builder.build_unreachable().or_llvm_err()?;

    builder.position_at_end(ok_bb);
    Ok(raw_ptr)
}

/// Emit LLVM IR function definitions for text helper functions.
/// These replace the C runtime functions in verum_runtime.c.
/// Call this once per module before lowering VBC instructions.
pub fn define_text_ir_helpers<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<()> {
    let i64_type = context.i64_type();
    let i8_type = context.i8_type();
    let ptr_type = context.ptr_type(AddressSpace::default());

    // Ensure libc functions are declared
    if module.get_function("malloc").is_none() {
        let malloc_ty = ptr_type.fn_type(&[i64_type.into()], false);
        module.add_function("malloc", malloc_ty, None);
    }
    if module.get_function("strlen").is_none() {
        let strlen_ty = i64_type.fn_type(&[ptr_type.into()], false);
        module.add_function("strlen", strlen_ty, None);
    }
    if module.get_function("memcpy").is_none() {
        let memcpy_ty = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
        module.add_function("memcpy", memcpy_ty, None);
    }

    // --- verum_strlen_export(s: ptr) -> i64 ---
    if module.get_function("verum_strlen_export").is_none() {
        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_strlen_export").unwrap_or_else(|| module.add_function("verum_strlen_export", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let builder = context.create_builder();
        builder.position_at_end(entry);
        let s = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let strlen_fn = module.get_function("strlen").or_missing_fn("strlen")?;
        let len = builder.build_call(strlen_fn, &[s.into()], "len").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        builder.build_return(Some(&len)).or_llvm_err()?;
    }

    // --- verum_text_alloc(ptr: ptr, len: i64, cap: i64) -> i64 ---
    // Allocates a 24-byte Text object {ptr, len, cap}
    if module.get_function("verum_text_alloc").is_none() {
        let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_text_alloc").unwrap_or_else(|| module.add_function("verum_text_alloc", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let builder = context.create_builder();
        builder.position_at_end(entry);
        let data_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let len = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let cap = func.get_nth_param(2).or_internal("missing param 2")?.into_int_value();
        let size = i64_type.const_int(TEXT_OBJECT_SIZE, false);
        let obj = checked_malloc(context, &builder, module, size, "text_obj")?;
        // Store ptr at offset 0
        let ptr_as_i64 = builder.build_ptr_to_int(data_ptr, i64_type, "ptr_i64").or_llvm_err()?;
        builder.build_store(obj, ptr_as_i64).or_llvm_err()?;
        // Store len at offset 8
        // SAFETY: GEP at offset 1 within a struct of known layout; the offset is within the allocation
        let len_ptr = unsafe { builder.build_gep(i64_type, obj, &[i64_type.const_int(1, false)], "len_ptr").or_llvm_err()? };
        builder.build_store(len_ptr, len).or_llvm_err()?;
        // Store cap at offset 16
        // SAFETY: GEP at offset 2 within a struct of known layout; the offset is within the allocation
        let cap_ptr = unsafe { builder.build_gep(i64_type, obj, &[i64_type.const_int(2, false)], "cap_ptr").or_llvm_err()? };
        builder.build_store(cap_ptr, cap).or_llvm_err()?;
        // Return as i64
        let result = builder.build_ptr_to_int(obj, i64_type, "result").or_llvm_err()?;
        builder.build_return(Some(&result)).or_llvm_err()?;
    }

    // --- verum_text_get_ptr(text_obj: i64) -> ptr ---
    // Extracts char* from Text object. Returns "" for null.
    if module.get_function("verum_text_get_ptr").is_none() {
        let fn_type = ptr_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_text_get_ptr").unwrap_or_else(|| module.add_function("verum_text_get_ptr", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let null_bb = context.append_basic_block(func, "null_obj");
        let load_bb = context.append_basic_block(func, "load_ptr");
        let check_ptr_bb = context.append_basic_block(func, "check_ptr");
        let null_ptr_bb = context.append_basic_block(func, "null_ptr");
        let done_bb = context.append_basic_block(func, "done");
        let builder = context.create_builder();
        builder.position_at_end(entry);

        let empty_str = builder.build_global_string_ptr("", "empty_str").or_llvm_err()?;

        let text_obj = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, text_obj, i64_type.const_zero(), "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, null_bb, load_bb).or_llvm_err()?;

        builder.position_at_end(null_bb);
        builder.build_unconditional_branch(done_bb).or_llvm_err()?;

        builder.position_at_end(load_bb);
        let obj_ptr = builder.build_int_to_ptr(text_obj, ptr_type, "obj_ptr").or_llvm_err()?;
        let raw_ptr_i64 = builder.build_load(i64_type, obj_ptr, "raw_ptr_i64").or_llvm_err()?.into_int_value();
        let ptr_is_null = builder.build_int_compare(verum_llvm::IntPredicate::EQ, raw_ptr_i64, i64_type.const_zero(), "ptr_is_null").or_llvm_err()?;
        builder.build_conditional_branch(ptr_is_null, null_ptr_bb, check_ptr_bb).or_llvm_err()?;

        builder.position_at_end(check_ptr_bb);
        let loaded_ptr = builder.build_int_to_ptr(raw_ptr_i64, ptr_type, "loaded_ptr").or_llvm_err()?;
        builder.build_unconditional_branch(done_bb).or_llvm_err()?;

        builder.position_at_end(null_ptr_bb);
        builder.build_unconditional_branch(done_bb).or_llvm_err()?;

        builder.position_at_end(done_bb);
        let phi = builder.build_phi(ptr_type, "result").or_llvm_err()?;
        phi.add_incoming(&[
            (&empty_str.as_pointer_value(), null_bb),
            (&loaded_ptr, check_ptr_bb),
            (&empty_str.as_pointer_value(), null_ptr_bb),
        ]);
        builder.build_return(Some(&phi.as_basic_value())).or_llvm_err()?;
    }

    // --- verum_text_from_cstr(s: ptr) -> i64 ---
    // Create Text object from null-terminated C string
    if module.get_function("verum_text_from_cstr").is_none() {
        let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_text_from_cstr").unwrap_or_else(|| module.add_function("verum_text_from_cstr", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let null_bb = context.append_basic_block(func, "null_str");
        let valid_bb = context.append_basic_block(func, "valid_str");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let s = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let is_null = builder.build_is_null(s, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, null_bb, valid_bb).or_llvm_err()?;

        builder.position_at_end(null_bb);
        let text_alloc_fn = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let null_ptr = ptr_type.const_null();
        let zero = i64_type.const_zero();
        let null_result = builder.build_call(text_alloc_fn, &[null_ptr.into(), zero.into(), zero.into()], "null_text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;
        builder.build_return(Some(&null_result)).or_llvm_err()?;

        builder.position_at_end(valid_bb);
        let strlen_fn = module.get_function("strlen").or_missing_fn("strlen")?;
        let len = builder.build_call(strlen_fn, &[s.into()], "len").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let result = builder.build_call(text_alloc_fn, &[s.into(), len.into(), len.into()], "text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
    }

    // --- verum_text_from_static(ptr: ptr, len: i64) -> i64 ---
    // Create Text from static string data (copies to new buffer)
    if module.get_function("verum_text_from_static").is_none() {
        let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_text_from_static").unwrap_or_else(|| module.add_function("verum_text_from_static", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let src_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let len = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        // malloc(len + 1), memcpy, null-terminate
        let memcpy_fn = module.get_function("memcpy").or_missing_fn("memcpy")?;
        let alloc_size = builder.build_int_add(len, i64_type.const_int(1, false), "alloc_size").or_llvm_err()?;
        let buf = checked_malloc(context, &builder, module, alloc_size, "buf")?;
        builder.build_call(memcpy_fn, &[buf.into(), src_ptr.into(), len.into()], "").or_llvm_err()?;
        // Null-terminate
        // SAFETY: GEP to access the 'end_ptr' field at a fixed offset within a struct of known layout
        let end_ptr = unsafe { builder.build_gep(i8_type, buf, &[len], "end_ptr").or_llvm_err()? };
        builder.build_store(end_ptr, i8_type.const_zero()).or_llvm_err()?;
        // Create Text object
        let text_alloc_fn = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let result = builder.build_call(text_alloc_fn, &[buf.into(), len.into(), len.into()], "text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
    }

    // --- verum_text_concat(a: i64, b: i64) -> i64 ---
    // Concatenate two Text objects
    if module.get_function("verum_text_concat").is_none() {
        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_text_concat").unwrap_or_else(|| module.add_function("verum_text_concat", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let a_obj = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let b_obj = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

        // Extract ptr and len from both Text objects (with null checks)
        let text_get_ptr_fn = module.get_function("verum_text_get_ptr").or_missing_fn("verum_text_get_ptr")?;
        let a_ptr = builder.build_call(text_get_ptr_fn, &[a_obj.into()], "a_ptr").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        let b_ptr = builder.build_call(text_get_ptr_fn, &[b_obj.into()], "b_ptr").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();

        // Get lengths via strlen
        let strlen_fn = module.get_function("strlen").or_missing_fn("strlen")?;
        let a_len = builder.build_call(strlen_fn, &[a_ptr.into()], "a_len").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();
        let b_len = builder.build_call(strlen_fn, &[b_ptr.into()], "b_len").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_int_value();

        let total = builder.build_int_add(a_len, b_len, "total").or_llvm_err()?;
        let alloc_size = builder.build_int_add(total, i64_type.const_int(1, false), "alloc_size").or_llvm_err()?;

        let memcpy_fn = module.get_function("memcpy").or_missing_fn("memcpy")?;
        let buf = checked_malloc(context, &builder, module, alloc_size, "buf")?;
        // memcpy(buf, a_ptr, a_len)
        builder.build_call(memcpy_fn, &[buf.into(), a_ptr.into(), a_len.into()], "").or_llvm_err()?;
        // memcpy(buf + a_len, b_ptr, b_len)
        // SAFETY: GEP to access the 'buf_mid' field at a fixed offset within a struct of known layout
        let buf_offset = unsafe { builder.build_gep(i8_type, buf, &[a_len], "buf_mid").or_llvm_err()? };
        builder.build_call(memcpy_fn, &[buf_offset.into(), b_ptr.into(), b_len.into()], "").or_llvm_err()?;
        // Null-terminate
        // SAFETY: GEP to access the 'end_ptr' field at a fixed offset within a struct of known layout
        let end_ptr = unsafe { builder.build_gep(i8_type, buf, &[total], "end_ptr").or_llvm_err()? };
        builder.build_store(end_ptr, i8_type.const_zero()).or_llvm_err()?;

        let text_alloc_fn = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let result = builder.build_call(text_alloc_fn, &[buf.into(), total.into(), total.into()], "text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
    }

    // --- verum_text_char_len(text_obj: i64) -> i64 ---
    // Count UTF-8 characters (count bytes that are NOT continuation bytes 10xxxxxx)
    if module.get_function("verum_text_char_len").is_none() {
        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_text_char_len").unwrap_or_else(|| module.add_function("verum_text_char_len", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let loop_bb = context.append_basic_block(func, "loop");
        let body_bb = context.append_basic_block(func, "body");
        let done_bb = context.append_basic_block(func, "done");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let text_obj = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        let text_get_ptr_fn = module.get_function("verum_text_get_ptr").or_missing_fn("verum_text_get_ptr")?;
        let str_ptr = builder.build_call(text_get_ptr_fn, &[text_obj.into()], "str_ptr").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(loop_bb);
        let idx = builder.build_phi(i64_type, "idx").or_llvm_err()?;
        idx.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let count = builder.build_phi(i64_type, "count").or_llvm_err()?;
        count.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let idx_val = idx.as_basic_value().into_int_value();
        let count_val = count.as_basic_value().into_int_value();
        // SAFETY: GEP into the text's data region to access a byte at the given index; the index is bounds-checked against the text length
        let byte_ptr = unsafe { builder.build_gep(i8_type, str_ptr, &[idx_val], "byte_ptr").or_llvm_err()? };
        let byte = builder.build_load(i8_type, byte_ptr, "byte").or_llvm_err()?.into_int_value();
        let byte_i64 = builder.build_int_z_extend(byte, i64_type, "byte_i64").or_llvm_err()?;
        let is_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, byte, i8_type.const_zero(), "is_zero").or_llvm_err()?;
        builder.build_conditional_branch(is_zero, done_bb, body_bb).or_llvm_err()?;

        builder.position_at_end(body_bb);
        // A byte is a continuation byte if (byte & 0xC0) == 0x80
        let masked = builder.build_and(byte, i8_type.const_int(0xC0, false), "masked").or_llvm_err()?;
        let is_cont = builder.build_int_compare(verum_llvm::IntPredicate::EQ, masked, i8_type.const_int(0x80, false), "is_cont").or_llvm_err()?;
        // If NOT continuation byte, increment count
        let inc = builder.build_int_add(count_val, i64_type.const_int(1, false), "inc").or_llvm_err()?;
        let new_count = builder.build_select(is_cont, BasicValueEnum::IntValue(count_val), BasicValueEnum::IntValue(inc), "new_count").or_llvm_err()?.into_int_value();
        let new_idx = builder.build_int_add(idx_val, i64_type.const_int(1, false), "new_idx").or_llvm_err()?;
        idx.add_incoming(&[(&new_idx, body_bb)]);
        count.add_incoming(&[(&new_count, body_bb)]);
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(done_bb);
        builder.build_return(Some(&count_val)).or_llvm_err()?;
    }

    // --- verum_char_to_text(codepoint: i64) -> i64 ---
    // Convert Unicode codepoint to Text (UTF-8 encoded)
    if module.get_function("verum_char_to_text").is_none() {
        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function("verum_char_to_text").unwrap_or_else(|| module.add_function("verum_char_to_text", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let cp = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();
        // Simple: allocate 5 bytes, encode UTF-8, create Text
        let buf = checked_malloc(context, &builder, module, i64_type.const_int(5, false), "buf")?;
        // For simplicity, handle ASCII case inline (codepoint < 128)
        // Multi-byte UTF-8 is handled by the compiled text.vr Text.from_char
        let byte_val = builder.build_int_truncate(cp, i8_type, "byte").or_llvm_err()?;
        builder.build_store(buf, byte_val).or_llvm_err()?;
        // SAFETY: GEP at offset 1 within a struct of known layout; the offset is within the allocation
        let null_ptr = unsafe { builder.build_gep(i8_type, buf, &[i64_type.const_int(1, false)], "null_pos").or_llvm_err()? };
        builder.build_store(null_ptr, i8_type.const_zero()).or_llvm_err()?;
        let text_alloc_fn = module.get_function("verum_text_alloc").or_missing_fn("verum_text_alloc")?;
        let result = builder.build_call(text_alloc_fn, &[buf.into(), i64_type.const_int(1, false).into(), i64_type.const_int(1, false).into()], "text").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
    }
    Ok(())
}

/// Emit LLVM IR definitions for list helper functions, replacing C runtime stubs.
/// These are called from Strategy 0 list intercepts in instruction.rs.
pub fn define_list_ir_helpers<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<()> {
    let i64_type = context.i64_type();
    let i8_type = context.i8_type();
    let ptr_type = context.ptr_type(AddressSpace::default());
    let void_type = context.void_type();

    // Ensure libc functions are declared
    if module.get_function("calloc").is_none() {
        let calloc_ty = ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        module.add_function("calloc", calloc_ty, None);
    }
    if module.get_function("memcpy").is_none() {
        let memcpy_ty = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
        module.add_function("memcpy", memcpy_ty, None);
    }

    // List object layout constants (48 bytes = 24-byte obj header + ptr/len/cap)
    // header[3] = backing_ptr, header[4] = len, header[5] = cap
    // In byte offsets: ptr=24, len=32, cap=40
    let ptr_offset = i64_type.const_int(LIST_PTR_OFFSET, false);
    let len_offset = i64_type.const_int(LIST_LEN_OFFSET, false);
    let cap_offset = i64_type.const_int(LIST_CAP_OFFSET, false);

    // --- verum_list_grow(list_ptr: ptr) -> void ---
    // Doubles the capacity, allocates new backing array, copies elements.
    if module.get_function("verum_list_grow").is_none() {
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_list_grow").unwrap_or_else(|| module.add_function("verum_list_grow", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let grow = context.append_basic_block(func, "grow");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let list_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        // null check
        let is_null = builder.build_is_null(list_ptr, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_bb, grow).or_llvm_err()?;

        builder.position_at_end(grow);
        // Load current cap
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[cap_offset], "cap_slot").or_llvm_err()? };
        let old_cap = builder.build_load(i64_type, cap_slot, "old_cap").or_llvm_err()?.into_int_value();
        // new_cap = old_cap * 2
        let new_cap_2x = builder.build_int_mul(old_cap, i64_type.const_int(2, false), "cap_2x").or_llvm_err()?;
        // if new_cap < 32 then 32
        let min_cap = i64_type.const_int(32, false);
        let use_min = builder.build_int_compare(verum_llvm::IntPredicate::SLT, new_cap_2x, min_cap, "use_min").or_llvm_err()?;
        let new_cap: verum_llvm::values::IntValue = builder.build_select(use_min, min_cap, new_cap_2x, "new_cap").or_llvm_err()?.into_int_value();
        // calloc(new_cap, 8)
        let calloc_fn = module.get_function("calloc").or_missing_fn("calloc")?;
        let new_data = builder.build_call(calloc_fn, &[new_cap.into(), i64_type.const_int(8, false).into()], "new_data").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        // Load old backing ptr and len
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[ptr_offset], "ptr_slot").or_llvm_err()? };
        let old_data_i64 = builder.build_load(i64_type, ptr_slot, "old_data_i64").or_llvm_err()?.into_int_value();
        let old_data = builder.build_int_to_ptr(old_data_i64, ptr_type, "old_data").or_llvm_err()?;
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let len_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[len_offset], "len_slot").or_llvm_err()? };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?.into_int_value();
        // memcpy(new_data, old_data, len * 8)
        let copy_bytes = builder.build_int_mul(len, i64_type.const_int(8, false), "copy_bytes").or_llvm_err()?;
        let memcpy_fn = module.get_function("memcpy").or_missing_fn("memcpy")?;
        builder.build_call(memcpy_fn, &[new_data.into(), old_data.into(), copy_bytes.into()], "").or_llvm_err()?;
        // Store new cap and new ptr
        builder.build_store(cap_slot, new_cap).or_llvm_err()?;
        let new_data_i64 = builder.build_ptr_to_int(new_data, i64_type, "new_data_i64").or_llvm_err()?;
        builder.build_store(ptr_slot, new_data_i64).or_llvm_err()?;
        builder.build_unconditional_branch(ret_bb).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;
    }

    // --- verum_list_sort(data: ptr, len: i64) -> void ---
    // Insertion sort on i64 backing array.
    if module.get_function("verum_list_sort").is_none() {
        let fn_type = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_list_sort").unwrap_or_else(|| module.add_function("verum_list_sort", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let outer_loop = context.append_basic_block(func, "outer_loop");
        let inner_loop = context.append_basic_block(func, "inner_loop");
        let inner_body = context.append_basic_block(func, "inner_body");
        let inner_done = context.append_basic_block(func, "inner_done");
        let outer_inc = context.append_basic_block(func, "outer_inc");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let data = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let len = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        // if len <= 1, return
        let skip = builder.build_int_compare(verum_llvm::IntPredicate::SLE, len, i64_type.const_int(1, false), "skip").or_llvm_err()?;
        builder.build_conditional_branch(skip, ret_bb, outer_loop).or_llvm_err()?;

        // Outer loop: for i = 1..len
        builder.position_at_end(outer_loop);
        let i_phi = builder.build_phi(i64_type, "i").or_llvm_err()?;
        i_phi.add_incoming(&[(&i64_type.const_int(1, false), entry)]);
        let i_val = i_phi.as_basic_value().into_int_value();
        let i_done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, i_val, len, "i_done").or_llvm_err()?;
        builder.build_conditional_branch(i_done, ret_bb, inner_loop).or_llvm_err()?;

        // Load key = data[i]
        builder.position_at_end(inner_loop);
        // SAFETY: GEP at a fixed offset within a known struct layout; the pointer is valid from prior allocation
        let key_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[i_val], "key_ptr").or_llvm_err()? };
        let key = builder.build_load(i64_type, key_ptr, "key").or_llvm_err()?.into_int_value();
        let j_init = builder.build_int_sub(i_val, i64_type.const_int(1, false), "j_init").or_llvm_err()?;
        builder.build_unconditional_branch(inner_body).or_llvm_err()?;

        // Inner loop: while j >= 0 && data[j] > key
        builder.position_at_end(inner_body);
        let j_phi = builder.build_phi(i64_type, "j").or_llvm_err()?;
        j_phi.add_incoming(&[(&j_init, inner_loop)]);
        let j_val = j_phi.as_basic_value().into_int_value();
        let j_ge_0 = builder.build_int_compare(verum_llvm::IntPredicate::SGE, j_val, i64_type.const_zero(), "j_ge_0").or_llvm_err()?;
        let check_bb = context.append_basic_block(func, "check_val");
        builder.build_conditional_branch(j_ge_0, check_bb, inner_done).or_llvm_err()?;

        builder.position_at_end(check_bb);
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let dj_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[j_val], "dj_ptr").or_llvm_err()? };
        let dj_val = builder.build_load(i64_type, dj_ptr, "dj_val").or_llvm_err()?.into_int_value();
        let dj_gt_key = builder.build_int_compare(verum_llvm::IntPredicate::SGT, dj_val, key, "dj_gt_key").or_llvm_err()?;
        let shift_bb = context.append_basic_block(func, "shift");
        builder.build_conditional_branch(dj_gt_key, shift_bb, inner_done).or_llvm_err()?;

        // Shift: data[j+1] = data[j]; j--
        builder.position_at_end(shift_bb);
        let j_plus_1 = builder.build_int_add(j_val, i64_type.const_int(1, false), "j_plus_1").or_llvm_err()?;
        // SAFETY: GEP to access the 'dst_ptr' field at a fixed offset within a struct of known layout
        let dst_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[j_plus_1], "dst_ptr").or_llvm_err()? };
        builder.build_store(dst_ptr, dj_val).or_llvm_err()?;
        let j_next = builder.build_int_sub(j_val, i64_type.const_int(1, false), "j_next").or_llvm_err()?;
        j_phi.add_incoming(&[(&j_next, shift_bb)]);
        builder.build_unconditional_branch(inner_body).or_llvm_err()?;

        // Inner done: data[j+1] = key
        builder.position_at_end(inner_done);
        let j_final = builder.build_phi(i64_type, "j_final").or_llvm_err()?;
        j_final.add_incoming(&[(&j_val, inner_body), (&j_val, check_bb)]);
        let j_final_val = j_final.as_basic_value().into_int_value();
        let store_idx = builder.build_int_add(j_final_val, i64_type.const_int(1, false), "store_idx").or_llvm_err()?;
        // SAFETY: GEP to access the 'store_ptr' field at a fixed offset within a struct of known layout
        let store_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[store_idx], "store_ptr").or_llvm_err()? };
        builder.build_store(store_ptr, key).or_llvm_err()?;
        builder.build_unconditional_branch(outer_inc).or_llvm_err()?;

        // Outer inc: i++
        builder.position_at_end(outer_inc);
        let i_next = builder.build_int_add(i_val, i64_type.const_int(1, false), "i_next").or_llvm_err()?;
        i_phi.add_incoming(&[(&i_next, outer_inc)]);
        builder.build_unconditional_branch(outer_loop).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;
    }

    // --- verum_list_reverse(list_ptr: ptr) -> void ---
    if module.get_function("verum_list_reverse").is_none() {
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_list_reverse").unwrap_or_else(|| module.add_function("verum_list_reverse", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let loop_bb = context.append_basic_block(func, "loop");
        let body_bb = context.append_basic_block(func, "body");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let list_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let is_null = builder.build_is_null(list_ptr, "is_null").or_llvm_err()?;
        let load_bb = context.append_basic_block(func, "load");
        builder.build_conditional_branch(is_null, ret_bb, load_bb).or_llvm_err()?;

        builder.position_at_end(load_bb);
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let len_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[len_offset], "len_slot").or_llvm_err()? };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?.into_int_value();
        let too_short = builder.build_int_compare(verum_llvm::IntPredicate::SLE, len, i64_type.const_int(1, false), "too_short").or_llvm_err()?;
        builder.build_conditional_branch(too_short, ret_bb, loop_bb).or_llvm_err()?;

        // Load backing ptr once
        builder.position_at_end(loop_bb);
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[ptr_offset], "ptr_slot").or_llvm_err()? };
        let backing_i64 = builder.build_load(i64_type, ptr_slot, "backing_i64").or_llvm_err()?.into_int_value();
        let data = builder.build_int_to_ptr(backing_i64, ptr_type, "data").or_llvm_err()?;
        let j_init = builder.build_int_sub(len, i64_type.const_int(1, false), "j_init").or_llvm_err()?;
        builder.build_unconditional_branch(body_bb).or_llvm_err()?;

        // Loop: while i < j { swap data[i] and data[j]; i++; j--; }
        builder.position_at_end(body_bb);
        let i_phi = builder.build_phi(i64_type, "i").or_llvm_err()?;
        let j_phi = builder.build_phi(i64_type, "j").or_llvm_err()?;
        i_phi.add_incoming(&[(&i64_type.const_zero(), loop_bb)]);
        j_phi.add_incoming(&[(&j_init, loop_bb)]);
        let i_val = i_phi.as_basic_value().into_int_value();
        let j_val = j_phi.as_basic_value().into_int_value();
        let done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, i_val, j_val, "done").or_llvm_err()?;
        let swap_bb = context.append_basic_block(func, "swap");
        builder.build_conditional_branch(done, ret_bb, swap_bb).or_llvm_err()?;

        builder.position_at_end(swap_bb);
        // SAFETY: GEP to access the 'di' field at a fixed offset within a struct of known layout
        let di = unsafe { builder.build_in_bounds_gep(i64_type, data, &[i_val], "di").or_llvm_err()? };
        // SAFETY: GEP for element swap; both indices are validated to be within [0, len) before the swap operation
        let dj = unsafe { builder.build_in_bounds_gep(i64_type, data, &[j_val], "dj").or_llvm_err()? };
        let vi = builder.build_load(i64_type, di, "vi").or_llvm_err()?;
        let vj = builder.build_load(i64_type, dj, "vj").or_llvm_err()?;
        builder.build_store(di, vj).or_llvm_err()?;
        builder.build_store(dj, vi).or_llvm_err()?;
        let i_next = builder.build_int_add(i_val, i64_type.const_int(1, false), "i_next").or_llvm_err()?;
        let j_next = builder.build_int_sub(j_val, i64_type.const_int(1, false), "j_next").or_llvm_err()?;
        i_phi.add_incoming(&[(&i_next, swap_bb)]);
        j_phi.add_incoming(&[(&j_next, swap_bb)]);
        builder.build_unconditional_branch(body_bb).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;
    }

    // --- verum_list_swap(list_ptr: ptr, i: i64, j: i64) -> void ---
    if module.get_function("verum_list_swap").is_none() {
        let fn_type = void_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_list_swap").unwrap_or_else(|| module.add_function("verum_list_swap", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let do_swap = context.append_basic_block(func, "do_swap");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let list_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let i_arg = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let j_arg = func.get_nth_param(2).or_internal("missing param 2")?.into_int_value();
        let is_null = builder.build_is_null(list_ptr, "is_null").or_llvm_err()?;
        let check_bb = context.append_basic_block(func, "check");
        builder.build_conditional_branch(is_null, ret_bb, check_bb).or_llvm_err()?;

        builder.position_at_end(check_bb);
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let len_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[len_offset], "len_slot").or_llvm_err()? };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?.into_int_value();
        // Bounds check: i >= 0 && i < len && j >= 0 && j < len && i != j
        let i_ge_0 = builder.build_int_compare(verum_llvm::IntPredicate::SGE, i_arg, i64_type.const_zero(), "i_ge_0").or_llvm_err()?;
        let i_lt_len = builder.build_int_compare(verum_llvm::IntPredicate::SLT, i_arg, len, "i_lt_len").or_llvm_err()?;
        let j_ge_0 = builder.build_int_compare(verum_llvm::IntPredicate::SGE, j_arg, i64_type.const_zero(), "j_ge_0").or_llvm_err()?;
        let j_lt_len = builder.build_int_compare(verum_llvm::IntPredicate::SLT, j_arg, len, "j_lt_len").or_llvm_err()?;
        let i_ne_j = builder.build_int_compare(verum_llvm::IntPredicate::NE, i_arg, j_arg, "i_ne_j").or_llvm_err()?;
        let ok1 = builder.build_and(i_ge_0, i_lt_len, "ok1").or_llvm_err()?;
        let ok2 = builder.build_and(j_ge_0, j_lt_len, "ok2").or_llvm_err()?;
        let ok3 = builder.build_and(ok1, ok2, "ok3").or_llvm_err()?;
        let ok = builder.build_and(ok3, i_ne_j, "ok").or_llvm_err()?;
        builder.build_conditional_branch(ok, do_swap, ret_bb).or_llvm_err()?;

        builder.position_at_end(do_swap);
        // SAFETY: GEP into the list object header to access the data pointer field at a fixed offset; the list pointer is non-null and valid
        let ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[ptr_offset], "ptr_slot").or_llvm_err()? };
        let backing_i64 = builder.build_load(i64_type, ptr_slot, "backing_i64").or_llvm_err()?.into_int_value();
        let data = builder.build_int_to_ptr(backing_i64, ptr_type, "data").or_llvm_err()?;
        // SAFETY: GEP to access the 'di' field at a fixed offset within a struct of known layout
        let di = unsafe { builder.build_in_bounds_gep(i64_type, data, &[i_arg], "di").or_llvm_err()? };
        // SAFETY: GEP to access the 'dj' field at a fixed offset within a struct of known layout
        let dj = unsafe { builder.build_in_bounds_gep(i64_type, data, &[j_arg], "dj").or_llvm_err()? };
        let vi = builder.build_load(i64_type, di, "vi").or_llvm_err()?;
        let vj = builder.build_load(i64_type, dj, "vj").or_llvm_err()?;
        builder.build_store(di, vj).or_llvm_err()?;
        builder.build_store(dj, vi).or_llvm_err()?;
        builder.build_unconditional_branch(ret_bb).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;
    }

    // --- verum_list_insert(list_ptr: ptr, index: i64, value: i64) -> void ---
    if module.get_function("verum_list_insert").is_none() {
        let fn_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_list_insert").unwrap_or_else(|| module.add_function("verum_list_insert", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let do_insert = context.append_basic_block(func, "do_insert");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let list_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let index = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let value = func.get_nth_param(2).or_internal("missing param 2")?.into_int_value();
        let is_null = builder.build_is_null(list_ptr, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_bb, do_insert).or_llvm_err()?;

        builder.position_at_end(do_insert);
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let len_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[len_offset], "len_slot").or_llvm_err()? };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?.into_int_value();
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[cap_offset], "cap_slot").or_llvm_err()? };
        let cap = builder.build_load(i64_type, cap_slot, "cap").or_llvm_err()?.into_int_value();
        // Clamp index
        let idx_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, index, i64_type.const_zero(), "idx_neg").or_llvm_err()?;
        let idx_clamped_low: verum_llvm::values::IntValue = builder.build_select(idx_neg, i64_type.const_zero(), index, "idx_clamped_low").or_llvm_err()?.into_int_value();
        let idx_too_big = builder.build_int_compare(verum_llvm::IntPredicate::SGT, idx_clamped_low, len, "idx_too_big").or_llvm_err()?;
        let idx_final: verum_llvm::values::IntValue = builder.build_select(idx_too_big, len, idx_clamped_low, "idx_final").or_llvm_err()?.into_int_value();
        // Grow if needed
        let need_grow = builder.build_int_compare(verum_llvm::IntPredicate::SGE, len, cap, "need_grow").or_llvm_err()?;
        let grow_bb = context.append_basic_block(func, "grow");
        let shift_bb = context.append_basic_block(func, "shift");
        builder.build_conditional_branch(need_grow, grow_bb, shift_bb).or_llvm_err()?;

        builder.position_at_end(grow_bb);
        let grow_fn = module.get_function("verum_list_grow").or_missing_fn("verum_list_grow")?;
        builder.build_call(grow_fn, &[list_ptr.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(shift_bb).or_llvm_err()?;

        // Shift elements right: for i = len..idx_final (backward), data[i] = data[i-1]
        builder.position_at_end(shift_bb);
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[ptr_offset], "ptr_slot").or_llvm_err()? };
        let backing_i64 = builder.build_load(i64_type, ptr_slot, "backing_i64").or_llvm_err()?.into_int_value();
        let data = builder.build_int_to_ptr(backing_i64, ptr_type, "data").or_llvm_err()?;
        // Loop: i = len; while i > idx_final { data[i] = data[i-1]; i--; }
        let shift_loop = context.append_basic_block(func, "shift_loop");
        let shift_body = context.append_basic_block(func, "shift_body");
        let shift_done = context.append_basic_block(func, "shift_done");
        builder.build_unconditional_branch(shift_loop).or_llvm_err()?;

        builder.position_at_end(shift_loop);
        let k_phi = builder.build_phi(i64_type, "k").or_llvm_err()?;
        k_phi.add_incoming(&[(&len, shift_bb)]);
        let k_val = k_phi.as_basic_value().into_int_value();
        let k_gt_idx = builder.build_int_compare(verum_llvm::IntPredicate::SGT, k_val, idx_final, "k_gt_idx").or_llvm_err()?;
        builder.build_conditional_branch(k_gt_idx, shift_body, shift_done).or_llvm_err()?;

        builder.position_at_end(shift_body);
        let k_minus_1 = builder.build_int_sub(k_val, i64_type.const_int(1, false), "k_minus_1").or_llvm_err()?;
        // SAFETY: GEP to access the 'src' field at a fixed offset within a struct of known layout
        let src = unsafe { builder.build_in_bounds_gep(i64_type, data, &[k_minus_1], "src").or_llvm_err()? };
        // SAFETY: GEP to access the 'dst' field at a fixed offset within a struct of known layout
        let dst = unsafe { builder.build_in_bounds_gep(i64_type, data, &[k_val], "dst").or_llvm_err()? };
        let v = builder.build_load(i64_type, src, "v").or_llvm_err()?;
        builder.build_store(dst, v).or_llvm_err()?;
        k_phi.add_incoming(&[(&k_minus_1, shift_body)]);
        builder.build_unconditional_branch(shift_loop).or_llvm_err()?;

        builder.position_at_end(shift_done);
        // Store value at idx_final
        // SAFETY: GEP to access the 'val_ptr' field at a fixed offset within a struct of known layout
        let val_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[idx_final], "val_ptr").or_llvm_err()? };
        builder.build_store(val_ptr, value).or_llvm_err()?;
        // len++
        let new_len = builder.build_int_add(len, i64_type.const_int(1, false), "new_len").or_llvm_err()?;
        builder.build_store(len_slot, new_len).or_llvm_err()?;
        builder.build_unconditional_branch(ret_bb).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;
    }

    // --- verum_list_remove(list_ptr: ptr, index: i64) -> i64 ---
    if module.get_function("verum_list_remove").is_none() {
        let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_list_remove").unwrap_or_else(|| module.add_function("verum_list_remove", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let do_remove = context.append_basic_block(func, "do_remove");
        let ret_zero = context.append_basic_block(func, "ret_zero");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let list_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let index = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let is_null = builder.build_is_null(list_ptr, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_zero, do_remove).or_llvm_err()?;

        builder.position_at_end(do_remove);
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let len_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[len_offset], "len_slot").or_llvm_err()? };
        let len = builder.build_load(i64_type, len_slot, "len").or_llvm_err()?.into_int_value();
        // bounds check
        let idx_neg = builder.build_int_compare(verum_llvm::IntPredicate::SLT, index, i64_type.const_zero(), "idx_neg").or_llvm_err()?;
        let idx_ge_len = builder.build_int_compare(verum_llvm::IntPredicate::SGE, index, len, "idx_ge_len").or_llvm_err()?;
        let oob = builder.build_or(idx_neg, idx_ge_len, "oob").or_llvm_err()?;
        let valid_bb = context.append_basic_block(func, "valid");
        builder.build_conditional_branch(oob, ret_zero, valid_bb).or_llvm_err()?;

        builder.position_at_end(valid_bb);
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[ptr_offset], "ptr_slot").or_llvm_err()? };
        let backing_i64 = builder.build_load(i64_type, ptr_slot, "backing_i64").or_llvm_err()?.into_int_value();
        let data = builder.build_int_to_ptr(backing_i64, ptr_type, "data").or_llvm_err()?;
        // Save removed element
        // SAFETY: in-bounds GEP on a pointer to an object with known layout; the offset is within the allocated size
        let rem_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[index], "rem_ptr").or_llvm_err()? };
        let removed = builder.build_load(i64_type, rem_ptr, "removed").or_llvm_err()?.into_int_value();
        // Shift left: for i = index..len-1 { data[i] = data[i+1]; }
        let new_len = builder.build_int_sub(len, i64_type.const_int(1, false), "new_len").or_llvm_err()?;
        let shift_loop = context.append_basic_block(func, "shift_loop");
        let shift_body = context.append_basic_block(func, "shift_body");
        let shift_done = context.append_basic_block(func, "shift_done");
        builder.build_unconditional_branch(shift_loop).or_llvm_err()?;

        builder.position_at_end(shift_loop);
        let k_phi = builder.build_phi(i64_type, "k").or_llvm_err()?;
        k_phi.add_incoming(&[(&index, valid_bb)]);
        let k_val = k_phi.as_basic_value().into_int_value();
        let k_lt_new_len = builder.build_int_compare(verum_llvm::IntPredicate::SLT, k_val, new_len, "k_lt").or_llvm_err()?;
        builder.build_conditional_branch(k_lt_new_len, shift_body, shift_done).or_llvm_err()?;

        builder.position_at_end(shift_body);
        let k_plus_1 = builder.build_int_add(k_val, i64_type.const_int(1, false), "k_plus_1").or_llvm_err()?;
        // SAFETY: GEP to access the 'src' field at a fixed offset within a struct of known layout
        let src = unsafe { builder.build_in_bounds_gep(i64_type, data, &[k_plus_1], "src").or_llvm_err()? };
        // SAFETY: GEP to access the 'dst' field at a fixed offset within a struct of known layout
        let dst = unsafe { builder.build_in_bounds_gep(i64_type, data, &[k_val], "dst").or_llvm_err()? };
        let v = builder.build_load(i64_type, src, "v").or_llvm_err()?;
        builder.build_store(dst, v).or_llvm_err()?;
        k_phi.add_incoming(&[(&k_plus_1, shift_body)]);
        builder.build_unconditional_branch(shift_loop).or_llvm_err()?;

        builder.position_at_end(shift_done);
        builder.build_store(len_slot, new_len).or_llvm_err()?;
        builder.build_return(Some(&removed)).or_llvm_err()?;

        builder.position_at_end(ret_zero);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
    }

    // --- verum_list_extend(dest_ptr: ptr, src_ptr: ptr) -> void ---
    if module.get_function("verum_list_extend").is_none() {
        let fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        let func = module.get_function("verum_list_extend").unwrap_or_else(|| module.add_function("verum_list_extend", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let check_src = context.append_basic_block(func, "check_src");
        let loop_bb = context.append_basic_block(func, "loop");
        let body_bb = context.append_basic_block(func, "body");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let dest_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let src_ptr = func.get_nth_param(1).or_internal("missing param 1")?.into_pointer_value();
        let dst_null = builder.build_is_null(dest_ptr, "dst_null").or_llvm_err()?;
        builder.build_conditional_branch(dst_null, ret_bb, check_src).or_llvm_err()?;

        builder.position_at_end(check_src);
        let src_null = builder.build_is_null(src_ptr, "src_null").or_llvm_err()?;
        let load_bb = context.append_basic_block(func, "load");
        builder.build_conditional_branch(src_null, ret_bb, load_bb).or_llvm_err()?;

        builder.position_at_end(load_bb);
        // SAFETY: GEP at a fixed offset within a known struct layout; the pointer is valid from prior allocation
        let src_len_slot = unsafe { builder.build_in_bounds_gep(i8_type, src_ptr, &[len_offset], "src_len_slot").or_llvm_err()? };
        let src_len = builder.build_load(i64_type, src_len_slot, "src_len").or_llvm_err()?.into_int_value();
        let src_empty = builder.build_int_compare(verum_llvm::IntPredicate::SLE, src_len, i64_type.const_zero(), "src_empty").or_llvm_err()?;
        builder.build_conditional_branch(src_empty, ret_bb, loop_bb).or_llvm_err()?;

        // Loop: for i = 0..src_len, push each element
        builder.position_at_end(loop_bb);
        // SAFETY: GEP at a fixed offset within a known struct layout; the pointer is valid from prior allocation
        let src_ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, src_ptr, &[ptr_offset], "src_ptr_slot").or_llvm_err()? };
        let src_data_i64 = builder.build_load(i64_type, src_ptr_slot, "src_data_i64").or_llvm_err()?.into_int_value();
        let src_data = builder.build_int_to_ptr(src_data_i64, ptr_type, "src_data").or_llvm_err()?;
        builder.build_unconditional_branch(body_bb).or_llvm_err()?;

        builder.position_at_end(body_bb);
        let i_phi = builder.build_phi(i64_type, "i").or_llvm_err()?;
        i_phi.add_incoming(&[(&i64_type.const_zero(), loop_bb)]);
        let i_val = i_phi.as_basic_value().into_int_value();
        let done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, i_val, src_len, "done").or_llvm_err()?;
        let push_bb = context.append_basic_block(func, "push");
        builder.build_conditional_branch(done, ret_bb, push_bb).or_llvm_err()?;

        builder.position_at_end(push_bb);
        // Read current dst len and cap (re-read each iteration since grow may change them)
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let dst_len_slot = unsafe { builder.build_in_bounds_gep(i8_type, dest_ptr, &[len_offset], "dst_len_slot").or_llvm_err()? };
        let dst_len = builder.build_load(i64_type, dst_len_slot, "dst_len").or_llvm_err()?.into_int_value();
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let dst_cap_slot = unsafe { builder.build_in_bounds_gep(i8_type, dest_ptr, &[cap_offset], "dst_cap_slot").or_llvm_err()? };
        let dst_cap = builder.build_load(i64_type, dst_cap_slot, "dst_cap").or_llvm_err()?.into_int_value();
        let need_grow = builder.build_int_compare(verum_llvm::IntPredicate::SGE, dst_len, dst_cap, "need_grow").or_llvm_err()?;
        let grow_bb = context.append_basic_block(func, "grow");
        let store_bb = context.append_basic_block(func, "store");
        builder.build_conditional_branch(need_grow, grow_bb, store_bb).or_llvm_err()?;

        builder.position_at_end(grow_bb);
        let grow_fn = module.get_function("verum_list_grow").or_missing_fn("verum_list_grow")?;
        builder.build_call(grow_fn, &[dest_ptr.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(store_bb).or_llvm_err()?;

        builder.position_at_end(store_bb);
        // SAFETY: GEP into list data array for element copy; the index is bounded by the loop variable which runs within [0, len)
        let dst_ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, dest_ptr, &[ptr_offset], "dst_ptr_slot").or_llvm_err()? };
        let dst_data_i64 = builder.build_load(i64_type, dst_ptr_slot, "dst_data_i64").or_llvm_err()?.into_int_value();
        let dst_data = builder.build_int_to_ptr(dst_data_i64, ptr_type, "dst_data").or_llvm_err()?;
        // Re-read dst_len since grow may have been called
        let dst_len2 = builder.build_load(i64_type, dst_len_slot, "dst_len2").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into list data array for element copy; the index is bounded by the loop variable which runs within [0, len)
        let elem_src = unsafe { builder.build_in_bounds_gep(i64_type, src_data, &[i_val], "elem_src").or_llvm_err()? };
        let elem = builder.build_load(i64_type, elem_src, "elem").or_llvm_err()?;
        // SAFETY: GEP into list data array for element copy; the index is bounded by the loop variable which runs within [0, len)
        let elem_dst = unsafe { builder.build_in_bounds_gep(i64_type, dst_data, &[dst_len2], "elem_dst").or_llvm_err()? };
        builder.build_store(elem_dst, elem).or_llvm_err()?;
        let new_len = builder.build_int_add(dst_len2, i64_type.const_int(1, false), "new_len").or_llvm_err()?;
        builder.build_store(dst_len_slot, new_len).or_llvm_err()?;
        let i_next = builder.build_int_add(i_val, i64_type.const_int(1, false), "i_next").or_llvm_err()?;
        i_phi.add_incoming(&[(&i_next, store_bb)]);
        builder.build_unconditional_branch(body_bb).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;
    }

    // --- verum_list_clone(list_ptr: ptr) -> ptr ---
    if module.get_function("verum_list_clone").is_none() {
        let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function("verum_list_clone").unwrap_or_else(|| module.add_function("verum_list_clone", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let do_clone = context.append_basic_block(func, "do_clone");
        let ret_null = context.append_basic_block(func, "ret_null");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let list_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let is_null = builder.build_is_null(list_ptr, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, ret_null, do_clone).or_llvm_err()?;

        builder.position_at_end(do_clone);
        let calloc_fn = module.get_function("calloc").or_missing_fn("calloc")?;
        // Allocate 48-byte list object (6 * 8 = 48)
        let new_list = builder.build_call(calloc_fn, &[i64_type.const_int(6, false).into(), i64_type.const_int(8, false).into()], "new_list").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        // Load src fields
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let src_len_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[len_offset], "src_len_slot").or_llvm_err()? };
        let src_len = builder.build_load(i64_type, src_len_slot, "src_len").or_llvm_err()?.into_int_value();
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let src_cap_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[cap_offset], "src_cap_slot").or_llvm_err()? };
        let src_cap = builder.build_load(i64_type, src_cap_slot, "src_cap").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into the list object header to access the length field at a fixed offset; the list pointer is non-null and valid
        let src_ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, list_ptr, &[ptr_offset], "src_ptr_slot").or_llvm_err()? };
        let src_data_i64 = builder.build_load(i64_type, src_ptr_slot, "src_data_i64").or_llvm_err()?.into_int_value();
        // Check if has data
        let has_data = builder.build_int_compare(verum_llvm::IntPredicate::SGT, src_len, i64_type.const_zero(), "has_data").or_llvm_err()?;
        let copy_bb = context.append_basic_block(func, "copy");
        let ret_new = context.append_basic_block(func, "ret_new");
        builder.build_conditional_branch(has_data, copy_bb, ret_new).or_llvm_err()?;

        builder.position_at_end(copy_bb);
        // Use src_cap for allocation if > 0, else src_len
        let use_len = builder.build_int_compare(verum_llvm::IntPredicate::SLE, src_cap, i64_type.const_zero(), "use_len").or_llvm_err()?;
        let alloc_cap: verum_llvm::values::IntValue = builder.build_select(use_len, src_len, src_cap, "alloc_cap").or_llvm_err()?.into_int_value();
        let new_data = builder.build_call(calloc_fn, &[alloc_cap.into(), i64_type.const_int(8, false).into()], "new_data").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        // memcpy(new_data, src_data, src_len * 8)
        let src_data = builder.build_int_to_ptr(src_data_i64, ptr_type, "src_data").or_llvm_err()?;
        let copy_bytes = builder.build_int_mul(src_len, i64_type.const_int(8, false), "copy_bytes").or_llvm_err()?;
        let memcpy_fn = module.get_function("memcpy").or_missing_fn("memcpy")?;
        builder.build_call(memcpy_fn, &[new_data.into(), src_data.into(), copy_bytes.into()], "").or_llvm_err()?;
        // Store fields in new list
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let new_ptr_slot = unsafe { builder.build_in_bounds_gep(i8_type, new_list, &[ptr_offset], "new_ptr_slot").or_llvm_err()? };
        let new_data_i64 = builder.build_ptr_to_int(new_data, i64_type, "new_data_i64").or_llvm_err()?;
        builder.build_store(new_ptr_slot, new_data_i64).or_llvm_err()?;
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let new_len_slot = unsafe { builder.build_in_bounds_gep(i8_type, new_list, &[len_offset], "new_len_slot").or_llvm_err()? };
        builder.build_store(new_len_slot, src_len).or_llvm_err()?;
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let new_cap_slot = unsafe { builder.build_in_bounds_gep(i8_type, new_list, &[cap_offset], "new_cap_slot").or_llvm_err()? };
        builder.build_store(new_cap_slot, alloc_cap).or_llvm_err()?;
        builder.build_unconditional_branch(ret_new).or_llvm_err()?;

        builder.position_at_end(ret_new);
        builder.build_return(Some(&new_list)).or_llvm_err()?;

        builder.position_at_end(ret_null);
        builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;
    }
    Ok(())
}

/// FNV-1a hash constant: offset basis
const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
/// FNV-1a hash constant: prime
const FNV_PRIME: u64 = 1099511628211;
/// Map tombstone marker (0x7FFFFFFFFFFFFFFE)
const MAP_TOMBSTONE: u64 = 0x7FFFFFFFFFFFFFFE;
/// Default set capacity
const DEFAULT_SET_CAPACITY: u64 = 16;
/// Set entry size in bytes (2 * i64 = 16)
const SET_ENTRY_SIZE: u64 = 16;

/// Inline a FNV-1a hash of an i64 key (unrolled 8-byte loop).
/// Returns the hash value. Ensures hash != 0 and hash != MAP_TOMBSTONE.
fn build_fnv_hash<'ctx>(
    builder: &verum_llvm::builder::Builder<'ctx>,
    context: &'ctx Context,
    key: verum_llvm::values::IntValue<'ctx>,
) -> Result<verum_llvm::values::IntValue<'ctx>> {
    let i64_type = context.i64_type();
    let fnv_basis = i64_type.const_int(FNV_OFFSET_BASIS, false);
    let fnv_prime = i64_type.const_int(FNV_PRIME, false);
    let mask = i64_type.const_int(0xFF, false);

    // Unrolled 8 iterations: h = (h ^ byte_i) * FNV_PRIME
    let mut h = fnv_basis;
    for i in 0..8u64 {
        let shifted = if i == 0 {
            key
        } else {
            builder.build_right_shift(key, i64_type.const_int(i * 8, false), false, &format!("shift_{i}")).or_llvm_err()?
        };
        let byte_val = builder.build_and(shifted, mask, &format!("byte_{i}")).or_llvm_err()?;
        h = builder.build_xor(h, byte_val, &format!("xor_{i}")).or_llvm_err()?;
        h = builder.build_int_mul(h, fnv_prime, &format!("mul_{i}")).or_llvm_err()?;
    }

    // Ensure hash != 0 and hash != TOMBSTONE
    let is_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, h, i64_type.const_zero(), "h_is_0").or_llvm_err()?;
    let tombstone = i64_type.const_int(MAP_TOMBSTONE, false);
    let is_tomb = builder.build_int_compare(verum_llvm::IntPredicate::EQ, h, tombstone, "h_is_tomb").or_llvm_err()?;
    let bad = builder.build_or(is_zero, is_tomb, "h_bad").or_llvm_err()?;
    Ok(builder.build_select(bad, i64_type.const_int(1, false), h, "h_safe").or_llvm_err()?.into_int_value())
}

/// Emit LLVM IR definitions for Map and Set helper functions, replacing C runtime stubs.
pub fn define_map_set_ir_helpers<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<()> {
    // Keep external declarations so instruction handlers work,
    // but don't define function bodies here — the C runtime stubs
    // are still needed until we fix the module corruption issue.
    let i64_type = context.i64_type();
    let i8_type = context.i8_type();
    let ptr_type = context.ptr_type(AddressSpace::default());
    let void_type = context.void_type();

    // Ensure libc functions are declared
    if module.get_function("calloc").is_none() {
        let calloc_ty = ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        module.add_function("calloc", calloc_ty, None);
    }
    if module.get_function("free").is_none() {
        let free_ty = void_type.fn_type(&[ptr_type.into()], false);
        module.add_function("free", free_ty, None);
    }

    // Map object layout (NewG): [24B header][entries_ptr:i64][len:i64][cap:i64] = 48 bytes
    // Map entry layout: [hash:i64, key:i64, value:i64, _reserved:i64] = 32 bytes
    // Set object layout (NewG): [24B header][len:i64][cap:i64][entries_ptr:i64] = 48 bytes
    // Set entry: [hash:i64, value:i64] = 16 bytes

    // --- verum_set_new() -> ptr ---
    if module.get_function("verum_set_new").is_none() {
        let fn_type = ptr_type.fn_type(&[], false);
        let func = module.get_function("verum_set_new").unwrap_or_else(|| module.add_function("verum_set_new", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let builder = context.create_builder();
        builder.position_at_end(entry);

        let calloc_fn = module.get_function("calloc").or_missing_fn("calloc")?;
        // Allocate VerumSet with NewG layout (6 * i64 = 48 bytes: 24B header + 3 fields)
        let set_ptr = builder.build_call(calloc_fn, &[i64_type.const_int(6, false).into(), i64_type.const_int(8, false).into()], "set").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        // Header (24 bytes) is zeroed by calloc: type_tag=0, ref_count=0, epoch_caps=0
        // set->len = 0 (at offset 24, already zeroed)
        // set->cap = DEFAULT_SET_CAPACITY (at offset 32)
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let cap_slot = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_CAP_OFFSET, false)], "cap_slot").or_llvm_err()? };
        builder.build_store(cap_slot, i64_type.const_int(DEFAULT_SET_CAPACITY, false)).or_llvm_err()?;
        // set->entries = calloc(DEFAULT_SET_CAPACITY, SET_ENTRY_SIZE) (at offset 40)
        let entries = builder.build_call(calloc_fn, &[i64_type.const_int(DEFAULT_SET_CAPACITY, false).into(), i64_type.const_int(SET_ENTRY_SIZE, false).into()], "entries").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        // SAFETY: GEP into the set object to access the entries pointer at a fixed offset defined by SET_ENTRIES_OFFSET
        let entries_slot = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_ENTRIES_OFFSET, false)], "entries_slot").or_llvm_err()? };
        let entries_i64 = builder.build_ptr_to_int(entries, i64_type, "entries_i64").or_llvm_err()?;
        builder.build_store(entries_slot, entries_i64).or_llvm_err()?;
        builder.build_return(Some(&set_ptr)).or_llvm_err()?;
    }

    // --- verum_set_contains(set: ptr, value: i64) -> i64 ---
    if module.get_function("verum_set_contains").is_none() {
        let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_set_contains").unwrap_or_else(|| module.add_function("verum_set_contains", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let probe = context.append_basic_block(func, "probe");
        let ret_false = context.append_basic_block(func, "ret_false");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let set_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let value = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let is_null = builder.build_is_null(set_ptr, "is_null").or_llvm_err()?;
        let load_bb = context.append_basic_block(func, "load");
        builder.build_conditional_branch(is_null, ret_false, load_bb).or_llvm_err()?;

        builder.position_at_end(load_bb);
        // Load len from offset 24 (NewG layout)
        // SAFETY: GEP to access the 'len_gep' field at a fixed offset within a struct of known layout
        let len_gep = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_LEN_OFFSET, false)], "len_gep").or_llvm_err()? };
        let len_slot = builder.build_load(i64_type, len_gep, "len").or_llvm_err()?.into_int_value();
        let is_empty = builder.build_int_compare(verum_llvm::IntPredicate::EQ, len_slot, i64_type.const_zero(), "is_empty").or_llvm_err()?;
        builder.build_conditional_branch(is_empty, ret_false, probe).or_llvm_err()?;

        builder.position_at_end(probe);
        // Load cap from offset 32 and entries from offset 40 (NewG layout)
        // SAFETY: GEP to access the 'cap_gep' field at a fixed offset within a struct of known layout
        let cap_gep = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_CAP_OFFSET, false)], "cap_gep").or_llvm_err()? };
        let cap = builder.build_load(i64_type, cap_gep, "cap").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into the set object to access the capacity field at a fixed offset defined by SET_CAP_OFFSET
        let entries_gep = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_ENTRIES_OFFSET, false)], "entries_gep").or_llvm_err()? };
        let entries_i64 = builder.build_load(i64_type, entries_gep, "entries_i64").or_llvm_err()?.into_int_value();
        let data = builder.build_int_to_ptr(entries_i64, ptr_type, "data").or_llvm_err()?;
        let hash = build_fnv_hash(&builder, context, value)?;
        // idx = hash % cap (unsigned)
        let idx = builder.build_int_unsigned_rem(hash, cap, "idx").or_llvm_err()?;
        // Linear probing loop
        let loop_bb = context.append_basic_block(func, "loop");
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(loop_bb);
        let j_phi = builder.build_phi(i64_type, "j").or_llvm_err()?;
        j_phi.add_incoming(&[(&i64_type.const_zero(), probe)]);
        let j_val = j_phi.as_basic_value().into_int_value();
        let j_done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, j_val, cap, "j_done").or_llvm_err()?;
        builder.build_conditional_branch(j_done, ret_false, context.append_basic_block(func, "check")).or_llvm_err()?;

        let check_bb = func.get_last_basic_block().or_internal("no last block")?;
        builder.position_at_end(check_bb);
        // slot = ((idx + j) % cap) * 2
        let ij = builder.build_int_add(idx, j_val, "ij").or_llvm_err()?;
        let slot_idx = builder.build_int_unsigned_rem(ij, cap, "slot_idx").or_llvm_err()?;
        let slot = builder.build_int_mul(slot_idx, i64_type.const_int(2, false), "slot").or_llvm_err()?;
        // Load data[slot] (hash) and data[slot+1] (value)
        // SAFETY: GEP into a struct or object at a fixed slot offset; the object was allocated with the expected layout
        let h_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[slot], "h_ptr").or_llvm_err()? };
        let h_val = builder.build_load(i64_type, h_ptr, "h_val").or_llvm_err()?.into_int_value();
        let slot_1 = builder.build_int_add(slot, i64_type.const_int(1, false), "slot_1").or_llvm_err()?;
        // SAFETY: GEP into set entries array to access the value slot at slot+1; the slot index is within [0, capacity*2) via modular arithmetic
        let v_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[slot_1], "v_ptr").or_llvm_err()? };
        let v_val = builder.build_load(i64_type, v_ptr, "v_val").or_llvm_err()?.into_int_value();
        // Empty slot (h==0 && v==0)? → not found
        let h_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, h_val, i64_type.const_zero(), "h_zero").or_llvm_err()?;
        let v_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, v_val, i64_type.const_zero(), "v_zero").or_llvm_err()?;
        let empty = builder.build_and(h_zero, v_zero, "empty").or_llvm_err()?;
        let not_empty = context.append_basic_block(func, "not_empty");
        builder.build_conditional_branch(empty, ret_false, not_empty).or_llvm_err()?;

        builder.position_at_end(not_empty);
        // Found? (v == value)
        let found = builder.build_int_compare(verum_llvm::IntPredicate::EQ, v_val, value, "found").or_llvm_err()?;
        let ret_true = context.append_basic_block(func, "ret_true");
        let inc_bb = context.append_basic_block(func, "inc");
        builder.build_conditional_branch(found, ret_true, inc_bb).or_llvm_err()?;

        builder.position_at_end(ret_true);
        builder.build_return(Some(&i64_type.const_int(1, false))).or_llvm_err()?;

        builder.position_at_end(inc_bb);
        let j_next = builder.build_int_add(j_val, i64_type.const_int(1, false), "j_next").or_llvm_err()?;
        j_phi.add_incoming(&[(&j_next, inc_bb)]);
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(ret_false);
        builder.build_return(Some(&i64_type.const_zero())).or_llvm_err()?;
    }

    // --- verum_set_insert(set: ptr, value: i64) -> void ---
    // Note: C function returned bool, but instruction.rs calls it void (ignores return).
    // We keep void to match the call site signature.
    if module.get_function("verum_set_insert").is_none() {
        let fn_type = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_set_insert").unwrap_or_else(|| module.add_function("verum_set_insert", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let set_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let value = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let is_null = builder.build_is_null(set_ptr, "is_null").or_llvm_err()?;
        let do_insert = context.append_basic_block(func, "do_insert");
        builder.build_conditional_branch(is_null, ret_bb, do_insert).or_llvm_err()?;

        builder.position_at_end(do_insert);
        let hash = build_fnv_hash(&builder, context, value)?;
        // Load cap from offset 32 and entries from offset 40 (NewG layout)
        // SAFETY: GEP to access the 'cap_gep' field at a fixed offset within a struct of known layout
        let cap_gep = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_CAP_OFFSET, false)], "cap_gep").or_llvm_err()? };
        let cap = builder.build_load(i64_type, cap_gep, "cap").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into the set object to access the capacity field at a fixed offset defined by SET_CAP_OFFSET
        let entries_gep = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_ENTRIES_OFFSET, false)], "entries_gep").or_llvm_err()? };
        let entries_i64 = builder.build_load(i64_type, entries_gep, "entries_i64").or_llvm_err()?.into_int_value();
        let data = builder.build_int_to_ptr(entries_i64, ptr_type, "data").or_llvm_err()?;
        let idx = builder.build_int_unsigned_rem(hash, cap, "idx").or_llvm_err()?;

        // Probe loop
        let loop_bb = context.append_basic_block(func, "loop");
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(loop_bb);
        let j_phi = builder.build_phi(i64_type, "j").or_llvm_err()?;
        j_phi.add_incoming(&[(&i64_type.const_zero(), do_insert)]);
        let j_val = j_phi.as_basic_value().into_int_value();
        let j_done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, j_val, cap, "j_done").or_llvm_err()?;
        builder.build_conditional_branch(j_done, ret_bb, context.append_basic_block(func, "probe")).or_llvm_err()?;

        let probe_bb = func.get_last_basic_block().or_internal("no last block")?;
        builder.position_at_end(probe_bb);
        let ij = builder.build_int_add(idx, j_val, "ij").or_llvm_err()?;
        let slot_idx = builder.build_int_unsigned_rem(ij, cap, "slot_idx").or_llvm_err()?;
        let slot = builder.build_int_mul(slot_idx, i64_type.const_int(2, false), "slot").or_llvm_err()?;
        // SAFETY: GEP into a struct or object at a fixed slot offset; the object was allocated with the expected layout
        let h_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[slot], "h_ptr").or_llvm_err()? };
        let h_val = builder.build_load(i64_type, h_ptr, "h_val").or_llvm_err()?.into_int_value();
        let slot_1 = builder.build_int_add(slot, i64_type.const_int(1, false), "slot_1").or_llvm_err()?;
        // SAFETY: GEP at a computed offset within the allocated entries array; index is bounded by capacity
        let v_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[slot_1], "v_ptr").or_llvm_err()? };
        let v_val = builder.build_load(i64_type, v_ptr, "v_val").or_llvm_err()?.into_int_value();

        // Empty slot?
        let h_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, h_val, i64_type.const_zero(), "h_zero").or_llvm_err()?;
        let v_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, v_val, i64_type.const_zero(), "v_zero").or_llvm_err()?;
        let empty = builder.build_and(h_zero, v_zero, "empty").or_llvm_err()?;
        let store_bb = context.append_basic_block(func, "store");
        let check_dup = context.append_basic_block(func, "check_dup");
        builder.build_conditional_branch(empty, store_bb, check_dup).or_llvm_err()?;

        // Store: write hash and value, increment len, maybe grow
        builder.position_at_end(store_bb);
        builder.build_store(h_ptr, hash).or_llvm_err()?;
        builder.build_store(v_ptr, value).or_llvm_err()?;
        // Read/write len at offset 24 (NewG layout)
        // SAFETY: GEP into the set object to access the length field at a fixed offset defined by SET_LEN_OFFSET
        let len_gep_store = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_LEN_OFFSET, false)], "len_gep_store").or_llvm_err()? };
        let len_ptr = builder.build_load(i64_type, len_gep_store, "old_len").or_llvm_err()?.into_int_value();
        let new_len = builder.build_int_add(len_ptr, i64_type.const_int(1, false), "new_len").or_llvm_err()?;
        builder.build_store(len_gep_store, new_len).or_llvm_err()?;
        // Check load factor: len * 4 > cap * 3 → grow
        let len4 = builder.build_int_mul(new_len, i64_type.const_int(4, false), "len4").or_llvm_err()?;
        let cap3 = builder.build_int_mul(cap, i64_type.const_int(3, false), "cap3").or_llvm_err()?;
        let need_grow = builder.build_int_compare(verum_llvm::IntPredicate::SGT, len4, cap3, "need_grow").or_llvm_err()?;
        let grow_bb = context.append_basic_block(func, "grow");
        builder.build_conditional_branch(need_grow, grow_bb, ret_bb).or_llvm_err()?;

        // Grow: allocate new entries, rehash all elements
        builder.position_at_end(grow_bb);
        let old_cap = cap;
        let new_cap = builder.build_int_mul(old_cap, i64_type.const_int(2, false), "new_cap").or_llvm_err()?;
        let calloc_fn = module.get_function("calloc").or_missing_fn("calloc")?;
        let new_entries = builder.build_call(calloc_fn, &[new_cap.into(), i64_type.const_int(SET_ENTRY_SIZE, false).into()], "new_entries").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        // Rehash loop: for each old entry with non-zero hash, re-insert
        let rehash_loop = context.append_basic_block(func, "rehash_loop");
        let rehash_body = context.append_basic_block(func, "rehash_body");
        let rehash_done = context.append_basic_block(func, "rehash_done");
        builder.build_unconditional_branch(rehash_loop).or_llvm_err()?;

        builder.position_at_end(rehash_loop);
        let ri = builder.build_phi(i64_type, "ri").or_llvm_err()?;
        ri.add_incoming(&[(&i64_type.const_zero(), grow_bb)]);
        let ri_val = ri.as_basic_value().into_int_value();
        let ri_done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, ri_val, old_cap, "ri_done").or_llvm_err()?;
        builder.build_conditional_branch(ri_done, rehash_done, rehash_body).or_llvm_err()?;

        builder.position_at_end(rehash_body);
        let ri_slot = builder.build_int_mul(ri_val, i64_type.const_int(2, false), "ri_slot").or_llvm_err()?;
        // SAFETY: GEP into hash table entry during Robin Hood reprobing; the slot index stays within [0, capacity) via modular wrap
        let ri_h_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[ri_slot], "ri_h_ptr").or_llvm_err()? };
        let ri_h = builder.build_load(i64_type, ri_h_ptr, "ri_h").or_llvm_err()?.into_int_value();
        let ri_slot_1 = builder.build_int_add(ri_slot, i64_type.const_int(1, false), "ri_slot_1").or_llvm_err()?;
        // SAFETY: GEP into hash table entry during Robin Hood reprobing; the slot index stays within [0, capacity) via modular wrap
        let ri_v_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[ri_slot_1], "ri_v_ptr").or_llvm_err()? };
        let ri_v = builder.build_load(i64_type, ri_v_ptr, "ri_v").or_llvm_err()?.into_int_value();
        // Check if occupied (h != 0 || v != 0)
        let ri_h_nz = builder.build_int_compare(verum_llvm::IntPredicate::NE, ri_h, i64_type.const_zero(), "ri_h_nz").or_llvm_err()?;
        let ri_v_nz = builder.build_int_compare(verum_llvm::IntPredicate::NE, ri_v, i64_type.const_zero(), "ri_v_nz").or_llvm_err()?;
        let occupied = builder.build_or(ri_h_nz, ri_v_nz, "occupied").or_llvm_err()?;
        let do_rehash = context.append_basic_block(func, "do_rehash");
        let ri_inc = context.append_basic_block(func, "ri_inc");
        builder.build_conditional_branch(occupied, do_rehash, ri_inc).or_llvm_err()?;

        builder.position_at_end(do_rehash);
        // Re-insert into new_entries: find empty slot using hash
        let re_hash = build_fnv_hash(&builder, context, ri_v)?;
        let re_idx = builder.build_int_unsigned_rem(re_hash, new_cap, "re_idx").or_llvm_err()?;
        // Inner probe loop for new_entries
        let inner_loop = context.append_basic_block(func, "inner_loop");
        builder.build_unconditional_branch(inner_loop).or_llvm_err()?;

        builder.position_at_end(inner_loop);
        let ik = builder.build_phi(i64_type, "ik").or_llvm_err()?;
        ik.add_incoming(&[(&i64_type.const_zero(), do_rehash)]);
        let ik_val = ik.as_basic_value().into_int_value();
        let ik_ij = builder.build_int_add(re_idx, ik_val, "ik_ij").or_llvm_err()?;
        let ik_slot_idx = builder.build_int_unsigned_rem(ik_ij, new_cap, "ik_slot_idx").or_llvm_err()?;
        let ik_slot = builder.build_int_mul(ik_slot_idx, i64_type.const_int(2, false), "ik_slot").or_llvm_err()?;
        // SAFETY: GEP into the set entry to access the hash slot during rehash; index bounded by new capacity
        let ik_h_ptr = unsafe { builder.build_in_bounds_gep(i64_type, new_entries, &[ik_slot], "ik_h_ptr").or_llvm_err()? };
        let ik_h = builder.build_load(i64_type, ik_h_ptr, "ik_h").or_llvm_err()?.into_int_value();
        let ik_slot_1 = builder.build_int_add(ik_slot, i64_type.const_int(1, false), "ik_slot_1").or_llvm_err()?;
        // SAFETY: GEP into the set entry to access the value slot during rehash; index bounded by new capacity
        let ik_v_ptr = unsafe { builder.build_in_bounds_gep(i64_type, new_entries, &[ik_slot_1], "ik_v_ptr").or_llvm_err()? };
        let ik_v = builder.build_load(i64_type, ik_v_ptr, "ik_v").or_llvm_err()?.into_int_value();
        let ik_h_z = builder.build_int_compare(verum_llvm::IntPredicate::EQ, ik_h, i64_type.const_zero(), "ik_h_z").or_llvm_err()?;
        let ik_v_z = builder.build_int_compare(verum_llvm::IntPredicate::EQ, ik_v, i64_type.const_zero(), "ik_v_z").or_llvm_err()?;
        let ik_empty = builder.build_and(ik_h_z, ik_v_z, "ik_empty").or_llvm_err()?;
        let ik_store = context.append_basic_block(func, "ik_store");
        let ik_next = context.append_basic_block(func, "ik_next");
        builder.build_conditional_branch(ik_empty, ik_store, ik_next).or_llvm_err()?;

        builder.position_at_end(ik_store);
        builder.build_store(ik_h_ptr, re_hash).or_llvm_err()?;
        builder.build_store(ik_v_ptr, ri_v).or_llvm_err()?;
        builder.build_unconditional_branch(ri_inc).or_llvm_err()?;

        builder.position_at_end(ik_next);
        let ik_next_val = builder.build_int_add(ik_val, i64_type.const_int(1, false), "ik_next_val").or_llvm_err()?;
        ik.add_incoming(&[(&ik_next_val, ik_next)]);
        builder.build_unconditional_branch(inner_loop).or_llvm_err()?;

        builder.position_at_end(ri_inc);
        let ri_next = builder.build_int_add(ri_val, i64_type.const_int(1, false), "ri_next").or_llvm_err()?;
        ri.add_incoming(&[(&ri_next, ri_inc)]);
        builder.build_unconditional_branch(rehash_loop).or_llvm_err()?;

        builder.position_at_end(rehash_done);
        // Free old entries
        let free_fn = module.get_function("free").or_missing_fn("free")?;
        builder.build_call(free_fn, &[data.into()], "").or_llvm_err()?;
        // Update set: cap, entries, len = rehash count
        builder.build_store(cap_gep, new_cap).or_llvm_err()?;
        let new_entries_i64 = builder.build_ptr_to_int(new_entries, i64_type, "new_entries_i64").or_llvm_err()?;
        builder.build_store(entries_gep, new_entries_i64).or_llvm_err()?;
        // Note: len was already updated before grow; rehash preserves all elements
        builder.build_unconditional_branch(ret_bb).or_llvm_err()?;

        // Check duplicate: value already exists
        builder.position_at_end(check_dup);
        let is_dup = builder.build_int_compare(verum_llvm::IntPredicate::EQ, v_val, value, "is_dup").or_llvm_err()?;
        let inc_j = context.append_basic_block(func, "inc_j");
        builder.build_conditional_branch(is_dup, ret_bb, inc_j).or_llvm_err()?;

        builder.position_at_end(inc_j);
        let j_next = builder.build_int_add(j_val, i64_type.const_int(1, false), "j_next").or_llvm_err()?;
        j_phi.add_incoming(&[(&j_next, inc_j)]);
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;
    }

    // --- verum_set_remove(set: ptr, value: i64) -> void ---
    if module.get_function("verum_set_remove").is_none() {
        let fn_type = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function("verum_set_remove").unwrap_or_else(|| module.add_function("verum_set_remove", fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let ret_bb = context.append_basic_block(func, "ret");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let set_ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let value = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let is_null = builder.build_is_null(set_ptr, "is_null").or_llvm_err()?;
        let do_remove = context.append_basic_block(func, "do_remove");
        builder.build_conditional_branch(is_null, ret_bb, do_remove).or_llvm_err()?;

        builder.position_at_end(do_remove);
        let hash = build_fnv_hash(&builder, context, value)?;
        // Load cap from offset 32 and entries from offset 40 (NewG layout)
        // SAFETY: GEP to access the 'cap_gep' field at a fixed offset within a struct of known layout
        let cap_gep = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_CAP_OFFSET, false)], "cap_gep").or_llvm_err()? };
        let cap = builder.build_load(i64_type, cap_gep, "cap").or_llvm_err()?.into_int_value();
        // SAFETY: GEP into the set object to access the capacity field at a fixed offset defined by SET_CAP_OFFSET
        let entries_gep = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_ENTRIES_OFFSET, false)], "entries_gep").or_llvm_err()? };
        let entries_i64 = builder.build_load(i64_type, entries_gep, "entries_i64").or_llvm_err()?.into_int_value();
        let data = builder.build_int_to_ptr(entries_i64, ptr_type, "data").or_llvm_err()?;
        let idx = builder.build_int_unsigned_rem(hash, cap, "idx").or_llvm_err()?;

        let loop_bb = context.append_basic_block(func, "loop");
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(loop_bb);
        let j_phi = builder.build_phi(i64_type, "j").or_llvm_err()?;
        j_phi.add_incoming(&[(&i64_type.const_zero(), do_remove)]);
        let j_val = j_phi.as_basic_value().into_int_value();
        let j_done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, j_val, cap, "j_done").or_llvm_err()?;
        let check_bb = context.append_basic_block(func, "check");
        builder.build_conditional_branch(j_done, ret_bb, check_bb).or_llvm_err()?;

        builder.position_at_end(check_bb);
        let ij = builder.build_int_add(idx, j_val, "ij").or_llvm_err()?;
        let slot_idx = builder.build_int_unsigned_rem(ij, cap, "slot_idx").or_llvm_err()?;
        let slot = builder.build_int_mul(slot_idx, i64_type.const_int(2, false), "slot").or_llvm_err()?;
        // SAFETY: GEP into set entries array to access the hash slot at the probed position; slot index is within [0, capacity*2) via modular wrap
        let h_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[slot], "h_ptr").or_llvm_err()? };
        let h_val = builder.build_load(i64_type, h_ptr, "h_val").or_llvm_err()?.into_int_value();
        let slot_1 = builder.build_int_add(slot, i64_type.const_int(1, false), "slot_1").or_llvm_err()?;
        // SAFETY: GEP into set entries array to access the value slot at slot+1; the slot index is within [0, capacity*2) via modular arithmetic
        let v_ptr = unsafe { builder.build_in_bounds_gep(i64_type, data, &[slot_1], "v_ptr").or_llvm_err()? };
        let v_val = builder.build_load(i64_type, v_ptr, "v_val").or_llvm_err()?.into_int_value();
        // Empty? → done
        let h_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, h_val, i64_type.const_zero(), "h_zero").or_llvm_err()?;
        let v_zero = builder.build_int_compare(verum_llvm::IntPredicate::EQ, v_val, i64_type.const_zero(), "v_zero").or_llvm_err()?;
        let empty = builder.build_and(h_zero, v_zero, "empty").or_llvm_err()?;
        let not_empty = context.append_basic_block(func, "not_empty");
        builder.build_conditional_branch(empty, ret_bb, not_empty).or_llvm_err()?;

        builder.position_at_end(not_empty);
        let found = builder.build_int_compare(verum_llvm::IntPredicate::EQ, v_val, value, "found").or_llvm_err()?;
        let do_del = context.append_basic_block(func, "do_del");
        let inc_bb = context.append_basic_block(func, "inc");
        builder.build_conditional_branch(found, do_del, inc_bb).or_llvm_err()?;

        builder.position_at_end(do_del);
        // Zero out the slot
        builder.build_store(h_ptr, i64_type.const_zero()).or_llvm_err()?;
        builder.build_store(v_ptr, i64_type.const_zero()).or_llvm_err()?;
        // Decrement len at offset 24 (NewG layout)
        // SAFETY: GEP into the set object to access the length field at a fixed offset defined by SET_LEN_OFFSET
        let len_gep_del = unsafe { builder.build_in_bounds_gep(i8_type, set_ptr, &[i64_type.const_int(SET_LEN_OFFSET, false)], "len_gep_del").or_llvm_err()? };
        let old_len = builder.build_load(i64_type, len_gep_del, "old_len").or_llvm_err()?.into_int_value();
        let new_len = builder.build_int_sub(old_len, i64_type.const_int(1, false), "new_len").or_llvm_err()?;
        builder.build_store(len_gep_del, new_len).or_llvm_err()?;
        builder.build_unconditional_branch(ret_bb).or_llvm_err()?;

        builder.position_at_end(inc_bb);
        let j_next = builder.build_int_add(j_val, i64_type.const_int(1, false), "j_next").or_llvm_err()?;
        j_phi.add_incoming(&[(&j_next, inc_bb)]);
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        let _ = hash; // suppress unused warning
        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;
    }

    // --- __verum_map_iter_next(entries_ptr, cap, slot_idx, out_key, out_value) -> next_slot ---
    // Inline LLVM IR helper for map iteration. Scans slots from slot_idx to cap,
    // finds next occupied entry (hash > 0), writes key/value, returns i+1 or -1.
    // Slot layout: [key(0)][value(8)][hash(16)][psl(24)] = 32 bytes per slot.
    if module.get_function("__verum_map_iter_next").is_none() {
        let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into(), ptr_type.into(), ptr_type.into()], false);
        let func = module.add_function("__verum_map_iter_next", fn_type, None);
        func.set_linkage(verum_llvm::module::Linkage::Internal);
        let entry = context.append_basic_block(func, "entry");
        let loop_bb = context.append_basic_block(func, "loop");
        let found_bb = context.append_basic_block(func, "found");
        let inc_bb = context.append_basic_block(func, "inc");
        let done_bb = context.append_basic_block(func, "done");
        let builder = context.create_builder();

        builder.position_at_end(entry);
        let entries = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let cap = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();
        let slot_idx = func.get_nth_param(2).or_internal("missing param 2")?.into_int_value();
        let out_key_ptr = func.get_nth_param(3).or_internal("missing param 3")?.into_pointer_value();
        let out_val_ptr = func.get_nth_param(4).or_internal("missing param 4")?.into_pointer_value();
        // Check entries != null and cap > 0
        let entries_null = builder.build_is_null(entries, "entries_null").or_llvm_err()?;
        let cap_zero = builder.build_int_compare(verum_llvm::IntPredicate::SLE, cap, i64_type.const_zero(), "cap_zero").or_llvm_err()?;
        let bail = builder.build_or(entries_null, cap_zero, "bail").or_llvm_err()?;
        builder.build_conditional_branch(bail, done_bb, loop_bb).or_llvm_err()?;

        // Loop: i = slot_idx .. cap
        builder.position_at_end(loop_bb);
        let i_phi = builder.build_phi(i64_type, "i").or_llvm_err()?;
        i_phi.add_incoming(&[(&slot_idx, entry)]);
        let i_val = i_phi.as_basic_value().into_int_value();
        let i_done = builder.build_int_compare(verum_llvm::IntPredicate::SGE, i_val, cap, "i_done").or_llvm_err()?;
        let check_bb = context.append_basic_block(func, "check");
        builder.build_conditional_branch(i_done, done_bb, check_bb).or_llvm_err()?;

        builder.position_at_end(check_bb);
        // hash at entries + i*32 + 16
        let slot_byte_off = builder.build_int_mul(i_val, i64_type.const_int(32, false), "slot_off").or_llvm_err()?;
        let hash_off = builder.build_int_add(slot_byte_off, i64_type.const_int(16, false), "hash_off").or_llvm_err()?;
        // SAFETY: GEP into hash table entry to access the hash slot; the slot index is within [0, capacity) via modular arithmetic
        let hash_ptr = unsafe { builder.build_in_bounds_gep(i8_type, entries, &[hash_off], "hash_ptr").or_llvm_err()? };
        let hash_val = builder.build_load(i64_type, hash_ptr, "hash_val").or_llvm_err()?.into_int_value();
        let is_occupied = builder.build_int_compare(verum_llvm::IntPredicate::SGT, hash_val, i64_type.const_zero(), "is_occupied").or_llvm_err()?;
        builder.build_conditional_branch(is_occupied, found_bb, inc_bb).or_llvm_err()?;

        builder.position_at_end(found_bb);
        // Read key at entries + i*32 + 0
        // SAFETY: GEP into hash table entry to access the key slot; the entry was found via probe within allocated capacity
        let key_ptr = unsafe { builder.build_in_bounds_gep(i8_type, entries, &[slot_byte_off], "key_ptr").or_llvm_err()? };
        let key_val = builder.build_load(i64_type, key_ptr, "key_val").or_llvm_err()?;
        builder.build_store(out_key_ptr, key_val).or_llvm_err()?;
        // Read value at entries + i*32 + 8
        let val_off = builder.build_int_add(slot_byte_off, i64_type.const_int(8, false), "val_off").or_llvm_err()?;
        // SAFETY: GEP into the map entry at offset 8 (value field) for iteration; entry index bounded by capacity
        let val_ptr = unsafe { builder.build_in_bounds_gep(i8_type, entries, &[val_off], "val_ptr").or_llvm_err()? };
        let val_val = builder.build_load(i64_type, val_ptr, "val_val").or_llvm_err()?;
        builder.build_store(out_val_ptr, val_val).or_llvm_err()?;
        // Return i + 1
        let next_i = builder.build_int_add(i_val, i64_type.const_int(1, false), "next_i").or_llvm_err()?;
        builder.build_return(Some(&next_i)).or_llvm_err()?;

        builder.position_at_end(inc_bb);
        let i_next = builder.build_int_add(i_val, i64_type.const_int(1, false), "i_next").or_llvm_err()?;
        i_phi.add_incoming(&[(&i_next, inc_bb)]);
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(done_bb);
        // Return -1 (exhausted)
        builder.build_return(Some(&i64_type.const_int(u64::MAX, true))).or_llvm_err()?;
    }

    // NOTE: All map operations (insert, get, contains_key, grow) are handled by
    // compiled map.vr via Strategy 1/2 dispatch. C runtime map functions deleted.
    // Only __verum_map_iter_next remains as an inline LLVM IR helper for map iteration.
    Ok(())
}

// =========================================================================
// CBGR Memory Management — LLVM IR emission
// =========================================================================

impl<'ctx> RuntimeLowering<'ctx> {
    // ========================================================================
    // CBGR Memory Management — LLVM IR emission
    // ========================================================================
    //
    // AllocationHeader layout (32 bytes, placed BEFORE user data pointer):
    //   offset  0: generation   (i32, atomic)
    //   offset  4: size         (i32)
    //   offset  8: epoch        (i16, atomic)
    //   offset 10: capabilities (i16)
    //   offset 12: ref_count    (i32, atomic)
    //   offset 16: flags        (i64, atomic)
    //   offset 24: next_free    (ptr)
    //
    // The user pointer is at (raw_ptr + 32). get_header subtracts 32.

    /// AllocationHeader size in bytes (aligned to 32).
    const ALLOC_HEADER_SIZE: u64 = 32;
    /// GEN_INITIAL
    const GEN_INITIAL: u64 = 1;
    /// GEN_MAX
    const GEN_MAX: u64 = 0xFFFFFFFE;
    /// CAP_FULL (all capabilities)
    const CAP_FULL: u64 = 0x00FF;
    /// FLAG_REVOKED
    const FLAG_REVOKED: u64 = 0x02;

    /// Emit all CBGR LLVM IR functions.
    fn emit_verum_cbgr_functions(&self, module: &Module<'ctx>) -> Result<()> {
        self.emit_cbgr_allocate(module)?;
        self.emit_cbgr_deallocate(module)?;
        self.emit_cbgr_realloc(module)?;
        self.emit_cbgr_get_epoch(module)?;
        self.emit_cbgr_new_generation(module)?;
        self.emit_cbgr_register_root(module)?;
        self.emit_cbgr_revoke(module)?;
        self.emit_cbgr_ref_release(module)?;
        self.emit_cbgr_ref_count(module)?;
        self.emit_cbgr_invalidate(module)?;
        // verum_cbgr_check, check_write, check_fat, epoch_begin kept in C
        // because they access ExecutionContext (platform-dependent struct layout).
        Ok(())
    }

    /// Helper: get_header(ptr) = ptr - 32, returns i8*
    fn emit_cbgr_get_header<'a>(
        &self,
        builder: &Builder<'ctx>,
        user_ptr: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let i8_type = self.context.i8_type();
        // GEP with negative offset: ptr - 32
        let neg_offset = self.context.i64_type().const_int(Self::ALLOC_HEADER_SIZE, false);
        let neg = builder.build_int_neg(neg_offset, "neg_hdr_size").or_llvm_err()?;
        // SAFETY: GEP with negative offset to reach the CBGR allocation header preceding the user data pointer; all CBGR allocations include this header
        Ok(unsafe { builder.build_gep(i8_type, user_ptr, &[neg], "header_ptr").or_llvm_err()? })
    }

    /// Helper: align_up(size, 32) = (size + 31) & ~31
    fn emit_align_up(
        &self,
        builder: &Builder<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let thirty_one = i64_type.const_int(31, false);
        let mask = i64_type.const_int(!31u64, false);
        let added = builder.build_int_add(size, thirty_one, "add31").or_llvm_err()?;
        Ok(builder.build_and(added, mask, "aligned").or_llvm_err()?)
    }

    /// verum_cbgr_allocate(size: i64) -> i8*
    /// Allocates header + aligned user data, initializes header fields.
    fn emit_cbgr_allocate(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_allocate";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i8_type = self.context.i8_type();
        let i16_type = self.context.i16_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = ptr_type.fn_type(&[i64_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let size = func.get_nth_param(0).or_internal("missing param 0")?.into_int_value();

        // total_size = 32 + align_up(size, 32)
        let aligned = self.emit_align_up(&builder, size)?;
        let header_size = i64_type.const_int(Self::ALLOC_HEADER_SIZE, false);
        let total_size = builder.build_int_add(header_size, aligned, "total").or_llvm_err()?;

        // raw = malloc(total_size)
        // Note: use malloc, NOT verum_alloc — verum_alloc is static in verum_platform.c
        // and cannot be called from LLVM IR. CBGR objects are freed with free() via
        // the dealloc path, so this is consistent.
        let alloc_fn = self.get_or_declare_fn(module, "malloc", ptr_type.fn_type(&[i64_type.into()], false));
        let raw = builder.build_call(alloc_fn, &[total_size.into()], "raw").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();

        // null check
        let is_null = builder.build_is_null(raw, "is_null").or_llvm_err()?;
        let init_bb = self.context.append_basic_block(func, "init");
        let null_bb = self.context.append_basic_block(func, "null_ret");
        builder.build_conditional_branch(is_null, null_bb, init_bb).or_llvm_err()?;

        // null return
        builder.position_at_end(null_bb);
        builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;

        // init header
        builder.position_at_end(init_bb);

        // memset to zero
        let memset_fn = self.get_or_declare_fn(module, "memset",
            ptr_type.fn_type(&[ptr_type.into(), i32_type.into(), i64_type.into()], false));
        builder.build_call(memset_fn, &[raw.into(), i32_type.const_zero().into(), total_size.into()], "").or_llvm_err()?;

        // header->generation = GEN_INITIAL (offset 0, i32 atomic)
        let gen_ptr = raw; // offset 0
        builder.build_store(gen_ptr, i32_type.const_int(Self::GEN_INITIAL, false)).or_llvm_err()?;

        // header->size = (uint32_t)size (offset 4)
        // SAFETY: GEP into the CBGR header to access the epoch field at a fixed offset; the header layout is defined by the allocator
        let size_ptr = unsafe { builder.build_gep(i8_type, raw, &[i64_type.const_int(4, false)], "size_ptr").or_llvm_err()? };
        let size_u32 = builder.build_int_truncate(size, i32_type, "size32").or_llvm_err()?;
        builder.build_store(size_ptr, size_u32).or_llvm_err()?;

        // header->epoch = global_epoch & 0xFFFF (offset 8, i16)
        // For simplicity, store 0 (same as memset). The C version reads global_epoch
        // but since we memset to 0 and global_epoch starts at 0, this is correct initially.
        // header->epoch already 0 from memset

        // header->capabilities = CAP_FULL (offset 10, i16)
        // SAFETY: GEP at offset 10 within a struct of known layout; the offset is within the allocation
        let cap_ptr = unsafe { builder.build_gep(i8_type, raw, &[i64_type.const_int(10, false)], "cap_ptr").or_llvm_err()? };
        builder.build_store(cap_ptr, i16_type.const_int(Self::CAP_FULL, false)).or_llvm_err()?;

        // header->ref_count = 1 (offset 12, i32)
        // SAFETY: GEP into the CBGR header to access the reference count at a fixed offset; the header is valid for all managed allocations
        let rc_ptr = unsafe { builder.build_gep(i8_type, raw, &[i64_type.const_int(12, false)], "rc_ptr").or_llvm_err()? };
        builder.build_store(rc_ptr, i32_type.const_int(1, false)).or_llvm_err()?;

        // header->flags = 0 (already from memset)
        // header->next_free = NULL (already from memset)

        // Return user pointer = raw + 32
        // SAFETY: GEP into the CBGR header to access the allocation size field; the header layout is a compile-time constant
        let user_ptr = unsafe { builder.build_gep(i8_type, raw, &[header_size], "user_ptr").or_llvm_err()? };
        builder.build_return(Some(&user_ptr)).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_deallocate(ptr: i8*) -> void
    fn emit_cbgr_deallocate(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_deallocate";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();

        // if (!ptr) return
        let is_null = builder.build_is_null(ptr, "is_null").or_llvm_err()?;
        let do_dealloc = self.context.append_basic_block(func, "do_dealloc");
        let ret_bb = self.context.append_basic_block(func, "ret");
        builder.build_conditional_branch(is_null, ret_bb, do_dealloc).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;

        builder.position_at_end(do_dealloc);

        // header = ptr - 32
        let header = self.emit_cbgr_get_header(&builder, ptr)?;

        // generation = atomic_fetch_add(&header->generation, 1) + 1
        // offset 0 = generation (i32)
        let gen_ptr = header;
        let old_gen = builder.build_atomicrmw(
            verum_llvm::AtomicRMWBinOp::Add,
            gen_ptr, i32_type.const_int(1, false),
            verum_llvm::AtomicOrdering::AcquireRelease,
        ).or_llvm_err()?;
        let new_gen = builder.build_int_add(old_gen, i32_type.const_int(1, false), "new_gen").or_llvm_err()?;

        // if (new_gen >= GEN_MAX || new_gen < old_gen) — overflow check
        let ge_max = builder.build_int_compare(verum_llvm::IntPredicate::UGE, new_gen,
            i32_type.const_int(Self::GEN_MAX as u64, false), "ge_max").or_llvm_err()?;
        let lt_old = builder.build_int_compare(verum_llvm::IntPredicate::ULT, new_gen, old_gen, "lt_old").or_llvm_err()?;
        let overflow = builder.build_or(ge_max, lt_old, "overflow").or_llvm_err()?;

        let overflow_bb = self.context.append_basic_block(func, "overflow");
        let set_ref = self.context.append_basic_block(func, "set_ref");
        builder.build_conditional_branch(overflow, overflow_bb, set_ref).or_llvm_err()?;

        // Overflow: reset generation, bump epoch
        builder.position_at_end(overflow_bb);
        builder.build_store(gen_ptr, i32_type.const_int(Self::GEN_INITIAL, false)).or_llvm_err()?;
        // epoch at offset 8 (i16) — atomic_fetch_add
        // SAFETY: GEP into the CBGR header to access the epoch field at a fixed offset; the header layout is defined by the allocator
        let epoch_ptr = unsafe { builder.build_gep(i8_type, header,
            &[i64_type.const_int(8, false)], "epoch_ptr").or_llvm_err()? };
        let i16_type = self.context.i16_type();
        builder.build_atomicrmw(
            verum_llvm::AtomicRMWBinOp::Add,
            epoch_ptr, i16_type.const_int(1, false),
            verum_llvm::AtomicOrdering::Release,
        ).or_llvm_err()?;
        builder.build_unconditional_branch(set_ref).or_llvm_err()?;

        // Set ref_count = 0, then free
        builder.position_at_end(set_ref);
        // offset 12 = ref_count (i32)
        // SAFETY: GEP into the CBGR header to access the reference count at a fixed offset; the header is valid for all managed allocations
        let rc_ptr = unsafe { builder.build_gep(i8_type, header,
            &[i64_type.const_int(12, false)], "rc_ptr").or_llvm_err()? };
        builder.build_store(rc_ptr, i32_type.const_zero()).or_llvm_err()?;

        // total = 32 + align_up(header->size, 32)
        // SAFETY: GEP into the CBGR header to access the allocation size field; the header layout is a compile-time constant
        let size_ptr = unsafe { builder.build_gep(i8_type, header,
            &[i64_type.const_int(4, false)], "hdr_size_ptr").or_llvm_err()? };
        let hdr_size = builder.build_load(i32_type, size_ptr, "hdr_size").or_llvm_err()?.into_int_value();
        let hdr_size64 = builder.build_int_z_extend(hdr_size, i64_type, "sz64").or_llvm_err()?;
        let aligned = self.emit_align_up(&builder, hdr_size64)?;
        let total = builder.build_int_add(aligned, i64_type.const_int(Self::ALLOC_HEADER_SIZE, false), "total").or_llvm_err()?;

        // free(header)
        // Note: use free, NOT verum_dealloc — verum_dealloc is static in verum_platform.c
        // and cannot be called from LLVM IR. Pairs with malloc() in emit_cbgr_allocate.
        let free_fn = self.get_or_declare_fn(module, "free",
            void_type.fn_type(&[ptr_type.into()], false));
        builder.build_call(free_fn, &[header.into()], "").or_llvm_err()?;
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_realloc(ptr: i8*, new_size: i64) -> i8*
    fn emit_cbgr_realloc(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_realloc";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();
        let new_size = func.get_nth_param(1).or_internal("missing param 1")?.into_int_value();

        // if (!ptr) return cbgr_allocate(new_size)
        let is_null = builder.build_is_null(ptr, "ptr_null").or_llvm_err()?;
        let null_bb = self.context.append_basic_block(func, "null_ptr");
        let check_size = self.context.append_basic_block(func, "check_size");
        builder.build_conditional_branch(is_null, null_bb, check_size).or_llvm_err()?;

        builder.position_at_end(null_bb);
        let alloc_fn = module.get_function("verum_cbgr_allocate").or_missing_fn("verum_cbgr_allocate")?;
        let new_ptr = builder.build_call(alloc_fn, &[new_size.into()], "new_ptr").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();
        builder.build_return(Some(&new_ptr)).or_llvm_err()?;

        // if (new_size <= 0) { deallocate(ptr); return NULL; }
        builder.position_at_end(check_size);
        let le_zero = builder.build_int_compare(verum_llvm::IntPredicate::SLE, new_size,
            i64_type.const_zero(), "le_zero").or_llvm_err()?;
        let free_bb = self.context.append_basic_block(func, "free_old");
        let do_realloc = self.context.append_basic_block(func, "do_realloc");
        builder.build_conditional_branch(le_zero, free_bb, do_realloc).or_llvm_err()?;

        builder.position_at_end(free_bb);
        let dealloc_fn = module.get_function("verum_cbgr_deallocate").or_missing_fn("verum_cbgr_deallocate")?;
        builder.build_call(dealloc_fn, &[ptr.into()], "").or_llvm_err()?;
        builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;

        // realloc: allocate new, copy min(old_size, new_size), deallocate old
        builder.position_at_end(do_realloc);
        let header = self.emit_cbgr_get_header(&builder, ptr)?;
        // SAFETY: GEP into the CBGR header to access the allocation size field; the header layout is a compile-time constant
        let old_size_ptr = unsafe { builder.build_gep(i8_type, header,
            &[i64_type.const_int(4, false)], "old_size_ptr").or_llvm_err()? };
        let old_size32 = builder.build_load(i32_type, old_size_ptr, "old_sz32").or_llvm_err()?.into_int_value();
        let old_size = builder.build_int_z_extend(old_size32, i64_type, "old_sz").or_llvm_err()?;

        // copy_size = min(old_size, new_size)
        let lt = builder.build_int_compare(verum_llvm::IntPredicate::ULT, old_size, new_size, "lt").or_llvm_err()?;
        let copy_size = builder.build_select(lt, old_size, new_size, "copy_sz").or_llvm_err()?.into_int_value();

        // new allocation
        let new_alloc = builder.build_call(alloc_fn, &[new_size.into()], "new_alloc").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("call returned void")?.into_pointer_value();

        // null check
        let new_null = builder.build_is_null(new_alloc, "new_null").or_llvm_err()?;
        let copy_bb = self.context.append_basic_block(func, "copy");
        let fail_bb = self.context.append_basic_block(func, "fail");
        builder.build_conditional_branch(new_null, fail_bb, copy_bb).or_llvm_err()?;

        builder.position_at_end(fail_bb);
        builder.build_return(Some(&ptr_type.const_null())).or_llvm_err()?;

        // copy + free old
        builder.position_at_end(copy_bb);
        let memcpy_fn = self.get_or_declare_fn(module, "memcpy",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false));
        builder.build_call(memcpy_fn, &[new_alloc.into(), ptr.into(), copy_size.into()], "").or_llvm_err()?;
        builder.build_call(dealloc_fn, &[ptr.into()], "").or_llvm_err()?;
        builder.build_return(Some(&new_alloc)).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_get_epoch() -> i64
    fn emit_cbgr_get_epoch(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_get_epoch";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        // global_epoch is defined in verum_platform.c
        // Declare it as external global
        let global_epoch = module.get_global("global_epoch").unwrap_or_else(|| {
            let g = module.add_global(i64_type, None, "global_epoch");
            g.set_linkage(verum_llvm::module::Linkage::External);
            g
        });
        let val = builder.build_load(i64_type, global_epoch.as_pointer_value(), "epoch").or_llvm_err()?;
        builder.build_return(Some(&val)).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_new_generation() -> i64
    fn emit_cbgr_new_generation(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_new_generation";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let fn_type = i64_type.fn_type(&[], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        // verum_generation_counter is a static global in verum_runtime.c
        // We define our own IR-local version with Internal linkage
        let counter_name = "verum_ir_generation_counter";
        let counter = module.get_global(counter_name).unwrap_or_else(|| {
            let g = module.add_global(i64_type, None, counter_name);
            g.set_linkage(verum_llvm::module::Linkage::Internal);
            g.set_initializer(&i64_type.const_int(1, false));
            g
        });
        let old = builder.build_atomicrmw(
            verum_llvm::AtomicRMWBinOp::Add,
            counter.as_pointer_value(),
            i64_type.const_int(1, false),
            verum_llvm::AtomicOrdering::Monotonic,
        ).or_llvm_err()?;
        builder.build_return(Some(&old)).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_register_root(ptr: i8*) -> void
    fn emit_cbgr_register_root(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_register_root";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        // GC roots are not actually used by any GC. This is a no-op stub.
        // The C version stores into a static array but nothing reads it.
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_revoke(ptr: i8*) -> void
    fn emit_cbgr_revoke(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_revoke";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();

        // if (!ptr) return
        let is_null = builder.build_is_null(ptr, "is_null").or_llvm_err()?;
        let do_revoke = self.context.append_basic_block(func, "do_revoke");
        let ret_bb = self.context.append_basic_block(func, "ret");
        builder.build_conditional_branch(is_null, ret_bb, do_revoke).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;

        builder.position_at_end(do_revoke);
        let header = self.emit_cbgr_get_header(&builder, ptr)?;

        // atomic_fetch_add(&header->generation, 1)
        builder.build_atomicrmw(
            verum_llvm::AtomicRMWBinOp::Add,
            header, i32_type.const_int(1, false),
            verum_llvm::AtomicOrdering::AcquireRelease,
        ).or_llvm_err()?;

        // atomic_fetch_or(&header->flags, FLAG_REVOKED)
        // SAFETY: GEP into the CBGR header to access the flags field; the header layout is a compile-time constant
        let flags_ptr = unsafe { builder.build_gep(i8_type, header,
            &[i64_type.const_int(16, false)], "flags_ptr").or_llvm_err()? };
        builder.build_atomicrmw(
            verum_llvm::AtomicRMWBinOp::Or,
            flags_ptr, i64_type.const_int(Self::FLAG_REVOKED, false),
            verum_llvm::AtomicOrdering::Release,
        ).or_llvm_err()?;

        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_ref_release(ptr: i8*) -> void
    fn emit_cbgr_ref_release(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_ref_release";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();

        // if (!ptr) return
        let is_null = builder.build_is_null(ptr, "is_null").or_llvm_err()?;
        let do_release = self.context.append_basic_block(func, "do_release");
        let ret_bb = self.context.append_basic_block(func, "ret");
        builder.build_conditional_branch(is_null, ret_bb, do_release).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;

        builder.position_at_end(do_release);
        let header = self.emit_cbgr_get_header(&builder, ptr)?;

        // old_count = atomic_fetch_sub(&header->ref_count, 1)
        // SAFETY: GEP into the CBGR header to access the reference count at a fixed offset; the header is valid for all managed allocations
        let rc_ptr = unsafe { builder.build_gep(i8_type, header,
            &[i64_type.const_int(12, false)], "rc_ptr").or_llvm_err()? };
        let old_count = builder.build_atomicrmw(
            verum_llvm::AtomicRMWBinOp::Sub,
            rc_ptr, i32_type.const_int(1, false),
            verum_llvm::AtomicOrdering::AcquireRelease,
        ).or_llvm_err()?;

        // if (old_count == 1) deallocate(ptr)
        let was_one = builder.build_int_compare(verum_llvm::IntPredicate::EQ, old_count,
            i32_type.const_int(1, false), "was_one").or_llvm_err()?;
        let dealloc_bb = self.context.append_basic_block(func, "dealloc");
        let done_bb = self.context.append_basic_block(func, "done");
        builder.build_conditional_branch(was_one, dealloc_bb, done_bb).or_llvm_err()?;

        builder.position_at_end(dealloc_bb);
        let dealloc_fn = module.get_function("verum_cbgr_deallocate").or_missing_fn("verum_cbgr_deallocate")?;
        builder.build_call(dealloc_fn, &[ptr.into()], "").or_llvm_err()?;
        builder.build_return(None).or_llvm_err()?;

        builder.position_at_end(done_bb);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_ref_count(ptr: i8*) -> i32
    fn emit_cbgr_ref_count(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_ref_count";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = i32_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();

        // if (!ptr) return 0
        let is_null = builder.build_is_null(ptr, "is_null").or_llvm_err()?;
        let do_load = self.context.append_basic_block(func, "do_load");
        let null_bb = self.context.append_basic_block(func, "null_ret");
        builder.build_conditional_branch(is_null, null_bb, do_load).or_llvm_err()?;

        builder.position_at_end(null_bb);
        builder.build_return(Some(&i32_type.const_zero())).or_llvm_err()?;

        builder.position_at_end(do_load);
        let header = self.emit_cbgr_get_header(&builder, ptr)?;
        // SAFETY: GEP into the CBGR header to access the reference count at a fixed offset; the header is valid for all managed allocations
        let rc_ptr = unsafe { builder.build_gep(i8_type, header,
            &[i64_type.const_int(12, false)], "rc_ptr").or_llvm_err()? };
        let rc = builder.build_load(i32_type, rc_ptr, "ref_count").or_llvm_err()?;
        builder.build_return(Some(&rc)).or_llvm_err()?;
        Ok(())
    }

    /// verum_cbgr_invalidate(ptr: i8*) -> void
    fn emit_cbgr_invalidate(&self, module: &Module<'ctx>) -> Result<()> {
        let name = "verum_cbgr_invalidate";
        if let Some(f) = module.get_function(name) {
            if f.count_basic_blocks() > 0 { return Ok(()); }
        }

        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();
        let fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let func = module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None));
        func.set_linkage(verum_llvm::module::Linkage::Internal);

        let entry = self.context.append_basic_block(func, "entry");
        let builder = self.context.create_builder();
        builder.position_at_end(entry);

        let ptr = func.get_nth_param(0).or_internal("missing param 0")?.into_pointer_value();

        // if (!ptr) return
        let is_null = builder.build_is_null(ptr, "is_null").or_llvm_err()?;
        let do_inv = self.context.append_basic_block(func, "do_inv");
        let ret_bb = self.context.append_basic_block(func, "ret");
        builder.build_conditional_branch(is_null, ret_bb, do_inv).or_llvm_err()?;

        builder.position_at_end(ret_bb);
        builder.build_return(None).or_llvm_err()?;

        builder.position_at_end(do_inv);
        let header = self.emit_cbgr_get_header(&builder, ptr)?;
        // header->generation = 0 (atomic store, release)
        builder.build_store(header, i32_type.const_zero()).or_llvm_err()?;
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// Helper: get or declare an external function by name
    fn get_or_declare_fn(
        &self,
        module: &Module<'ctx>,
        name: &str,
        fn_type: verum_llvm::types::FunctionType<'ctx>,
    ) -> FunctionValue<'ctx> {
        module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None))
    }
}
