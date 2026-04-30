//! Call operation handlers for VBC interpreter dispatch.
//!
//! Handles: Call (0x5B), CallR (0x5F), CallG (0x80), CallV (0x81),
//! CallC (0x82), CallClosure (0x5E), TailCall (0x5C), NewClosure (0x8A)

use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::types::{StringId, TypeId};
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

// ============================================================================
// Call Operations
// ============================================================================

/// Call function: `dst = fn(args...)`
///
/// Format: `[0x5B] [dst:reg] [func_id:varint] [args:reg_range]`
pub(in super::super) fn handle_call(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let func_id = FunctionId(read_varint(state)? as u32);
    let args = read_reg_range(state)?;

    // Get function descriptor — extract needed fields to release borrow
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;
    let bytecode_length = func.bytecode_length;
    let func_name_id = func.name;
    let reg_count = func.register_count;

    // Check for external/intrinsic functions with no bytecode body.
    // These are functions declared with @intrinsic("llvm.xxx") that have no
    // Verum implementation body. When the codegen can't resolve the intrinsic
    // to a typed opcode, it emits a plain Call to the function descriptor,
    // which has bytecode_length == 0. We intercept these here and compute
    // the result directly in Rust.
    if bytecode_length == 0 {
        let caller_base = state.reg_base();
        if let Some(result) = try_dispatch_intrinsic_by_name(
            state, func_name_id, dst, args.start, args.count, caller_base,
        )? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }
        // Fall through to normal call path — function may have register_count > 0
        // but empty bytecode (e.g., placeholder stubs that immediately return).
    }

    let return_pc = state.pc();
    let caller_base = state.reg_base();

    // Push new frame
    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;

    // Allocate registers for new frame
    state.registers.push_frame(reg_count);

    // Copy arguments from caller to callee
    for i in 0..args.count {
        let arg_value = state.registers.get(caller_base, Reg(args.start.0 + i as u16));
        state.registers.set(new_base, Reg(i as u16), arg_value);
    }

    // Reset PC to start of function
    state.set_pc(0);

    state.record_call();
    Ok(DispatchResult::Continue)
}

/// CallR (0x5F) - Indirect call via register.
///
/// The function address is stored in a register rather than being a constant.
pub(in super::super) fn handle_call_indirect(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let fn_reg = read_reg(state)?;
    let args = read_reg_range(state)?;

    // Get function reference from register
    let fn_val = state.get_reg(fn_reg);

    // Check if it's a function reference
    if fn_val.is_func_ref() {
        let func_id = fn_val.as_func_id();

        // Get function descriptor
        let func = state
            .module
            .get_function(func_id)
            .ok_or(InterpreterError::FunctionNotFound(func_id))?;

        let reg_count = func.register_count;
        let return_pc = state.pc();
        let caller_base = state.reg_base();

        // Push new frame
        let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;

        // Allocate registers for new frame
        state.registers.push_frame(reg_count);

        // Copy arguments from caller to callee
        for i in 0..args.count {
            let arg_value = state.registers.get(caller_base, Reg(args.start.0 + i as u16));
            state.registers.set(new_base, Reg(i as u16), arg_value);
        }

        // Reset PC to start of function
        state.set_pc(0);

        state.record_call();
        Ok(DispatchResult::Continue)
    } else {
        Err(InterpreterError::TypeMismatch {
            expected: "function reference",
            got: "other",
            operation: "indirect call",
        })
    }
}

/// CallG (0x80) - Generic function call with type parameters.
///
/// Encoding: opcode + dst:reg + func_id:varint + type_args:reg_vec + args:reg_range
/// type_args is encoded as varint(count) + reg * count (must consume all type arg registers).
pub(in super::super) fn handle_call_generic(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let func_id = FunctionId(read_varint(state)? as u32);
    // Read type_args as reg_vec: varint(count) + reg * count
    // Type args are not used at runtime (monomorphization) but must be consumed from bytecode.
    let type_args_count = read_varint(state)? as usize;
    for _ in 0..type_args_count {
        let _type_arg = read_reg(state)?;
    }
    let args = read_reg_range(state)?;

    // Get function descriptor — extract needed fields to release borrow
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;
    let bytecode_length = func.bytecode_length;
    let func_name_id = func.name;
    let reg_count = func.register_count;

    // Intercept external/intrinsic functions with no bytecode body
    if bytecode_length == 0 {
        let caller_base = state.reg_base();
        if let Some(result) = try_dispatch_intrinsic_by_name(
            state, func_name_id, dst, args.start, args.count, caller_base,
        )? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }
    }

    let return_pc = state.pc();
    let caller_base = state.reg_base();

    // Push new frame
    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;

    // Allocate registers for new frame
    state.registers.push_frame(reg_count);

    // Copy arguments from caller to callee
    for i in 0..args.count {
        let arg_value = state.registers.get(caller_base, Reg(args.start.0 + i as u16));
        state.registers.set(new_base, Reg(i as u16), arg_value);
    }

    // Reset PC to start of function
    state.set_pc(0);

    state.record_call();
    Ok(DispatchResult::Continue)
}

/// CallV (0x81) - Virtual dispatch call.
///
/// Performs vtable-based method dispatch on the receiver's runtime type.
/// The vtable_slot encodes protocol index (upper 16 bits) and method index (lower 16 bits).
pub(in super::super) fn handle_call_virtual(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let receiver = read_reg(state)?;
    let vtable_slot = read_varint(state)? as u32;
    let args = read_reg_range(state)?;

    // Get receiver value
    let recv_val = state.get_reg(receiver);

    // Resolve the actual function via vtable dispatch
    let resolved_func_id = resolve_vtable_method(state, recv_val, vtable_slot)?;

    // Get function descriptor
    let func = state
        .module
        .get_function(resolved_func_id)
        .ok_or(InterpreterError::FunctionNotFound(resolved_func_id))?;

    let reg_count = func.register_count;
    let return_pc = state.pc();
    let caller_base = state.reg_base();

    // Push new frame
    let new_base = state.call_stack.push_frame(resolved_func_id, reg_count, return_pc, dst)?;

    // Allocate registers for new frame
    state.registers.push_frame(reg_count);

    // First argument is receiver
    state.registers.set(new_base, Reg(0), recv_val);

    // Copy remaining arguments from caller to callee
    for i in 0..args.count {
        let arg_value = state.registers.get(caller_base, Reg(args.start.0 + i as u16));
        state.registers.set(new_base, Reg(1 + i as u16), arg_value);
    }

    // Reset PC to start of function
    state.set_pc(0);

    state.record_call();
    Ok(DispatchResult::Continue)
}

/// Resolves a vtable slot to a concrete FunctionId based on the receiver's runtime type.
fn resolve_vtable_method(
    state: &mut InterpreterState,
    recv_val: Value,
    vtable_slot: u32,
) -> InterpreterResult<FunctionId> {
    use super::super::super::heap::OBJECT_HEADER_SIZE;

    // Extract protocol and method indices from vtable_slot
    let protocol_index = (vtable_slot >> 16) as usize;
    let method_index = (vtable_slot & 0xFFFF) as usize;

    // Get receiver's runtime type
    let type_id = if recv_val.is_ptr() && !recv_val.is_nil() {
        let data_ptr = recv_val.as_ptr::<u8>();
        unsafe {
            let header_ptr = data_ptr.sub(OBJECT_HEADER_SIZE)
                as *const super::super::super::heap::ObjectHeader;
            if header_ptr.is_null() {
                return Err(InterpreterError::NullPointer);
            }
            (*header_ptr).type_id
        }
    } else if recv_val.is_type_ref() {
        recv_val.as_type_id()
    } else if recv_val.is_thin_ref() {
        let thin_ref = recv_val.as_thin_ref();
        let data_ptr = thin_ref.ptr;
        if data_ptr.is_null() {
            return Err(InterpreterError::NullPointer);
        }
        unsafe {
            let header_ptr = data_ptr.sub(OBJECT_HEADER_SIZE)
                as *const super::super::super::heap::ObjectHeader;
            (*header_ptr).type_id
        }
    } else if recv_val.is_fat_ref() {
        let fat_ref = recv_val.as_fat_ref();
        let data_ptr = fat_ref.thin.ptr;
        if data_ptr.is_null() {
            return Err(InterpreterError::NullPointer);
        }
        unsafe {
            let header_ptr = data_ptr.sub(OBJECT_HEADER_SIZE)
                as *const super::super::super::heap::ObjectHeader;
            (*header_ptr).type_id
        }
    } else {
        get_builtin_type_id(recv_val)
    };

    // Look up type descriptor
    let type_desc = state
        .module
        .get_type(type_id)
        .ok_or(InterpreterError::InvalidType(type_id))?;

    // Find the protocol implementation
    let protocol_impl = type_desc
        .protocols
        .get(protocol_index)
        .ok_or_else(|| InterpreterError::InvalidFieldIndex {
            type_id,
            field: protocol_index as u16,
            num_fields: type_desc.protocols.len() as u16,
        })?;

    // Get the method from the protocol
    let func_id_raw = *protocol_impl
        .methods
        .get(method_index)
        .ok_or(InterpreterError::InvalidFieldIndex {
            type_id,
            field: method_index as u16,
            num_fields: protocol_impl.methods.len() as u16,
        })?;

    Ok(FunctionId(func_id_raw))
}

/// Returns the builtin TypeId for a primitive Value.
fn get_builtin_type_id(val: Value) -> TypeId {
    if val.is_int() {
        TypeId::INT
    } else if val.is_float() {
        TypeId::FLOAT
    } else if val.is_bool() {
        TypeId::BOOL
    } else if val.is_unit() {
        TypeId::UNIT
    } else if val.is_small_string() {
        TypeId::TEXT
    } else {
        TypeId::PTR
    }
}

/// CallC (0x82) - Inline cached call.
///
/// Uses a cache slot to speed up repeated calls to the same target.
pub(in super::super) fn handle_call_cached(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let func_id = FunctionId(read_varint(state)? as u32);
    let _cache_slot = read_u8(state)?; // Cache slot for PIC (optimization, ignored in interpreter)
    let args = read_reg_range(state)?;

    // Get function descriptor — extract needed fields to release borrow
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;
    let bytecode_length = func.bytecode_length;
    let func_name_id = func.name;
    let reg_count = func.register_count;

    // Intercept external/intrinsic functions with no bytecode body
    if bytecode_length == 0 {
        let caller_base = state.reg_base();
        if let Some(result) = try_dispatch_intrinsic_by_name(
            state, func_name_id, dst, args.start, args.count, caller_base,
        )? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }
    }

    let return_pc = state.pc();
    let caller_base = state.reg_base();

    // Push new frame
    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;

    // Allocate registers for new frame
    state.registers.push_frame(reg_count);

    // Copy arguments from caller to callee
    for i in 0..args.count {
        let arg_value = state.registers.get(caller_base, Reg(args.start.0 + i as u16));
        state.registers.set(new_base, Reg(i as u16), arg_value);
    }

    // Reset PC to start of function
    state.set_pc(0);

    state.record_call();
    Ok(DispatchResult::Continue)
}

/// Call closure: `dst = closure(args...)`
pub(in super::super) fn handle_call_closure(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let closure_reg = read_reg(state)?;
    let args_start = read_reg(state)?;
    let arg_count = read_u8(state)?;

    let closure_val = state.get_reg(closure_reg);

    // Extract function ID and captures from closure object
    // Closure layout: [ObjectHeader][func_id:u32][capture_count:u32][captures:Value...]
    if !closure_val.is_ptr() || closure_val.is_nil() {
        return Err(InterpreterError::TypeMismatch {
            expected: "closure",
            got: "non-pointer",
            operation: "call_closure",
        });
    }

    let base_ptr = closure_val.as_ptr::<u8>();
    let header_offset = super::super::super::heap::OBJECT_HEADER_SIZE;

    let (func_id, capture_count) = unsafe {
        let func_id = *(base_ptr.add(header_offset) as *const u32);
        let capture_count = *(base_ptr.add(header_offset + 4) as *const u32);
        (func_id, capture_count as usize)
    };

    let func_id = FunctionId(func_id);
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;

    let reg_count = func.register_count;
    let return_pc = state.pc();
    let caller_base = state.reg_base();

    // Collect argument values before pushing frame
    let mut arg_values = Vec::with_capacity(arg_count as usize);
    for i in 0..arg_count as u16 {
        arg_values.push(state.registers.get(caller_base, Reg(args_start.0 + i)));
    }

    // Collect captured values
    let mut capture_values = Vec::with_capacity(capture_count);
    unsafe {
        let captures_offset = header_offset + 8; // after func_id + capture_count
        for i in 0..capture_count {
            let cap_ptr = base_ptr.add(captures_offset + i * std::mem::size_of::<Value>()) as *const Value;
            capture_values.push(std::ptr::read(cap_ptr));
        }
    }

    // Push new frame
    let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;
    state.registers.push_frame(reg_count);

    // Copy captured values first (they go before parameters in the closure's register layout)
    for (i, val) in capture_values.into_iter().enumerate() {
        state.registers.set(new_base, Reg(i as u16), val);
    }

    // Copy arguments after captures
    for (i, val) in arg_values.into_iter().enumerate() {
        state.registers.set(new_base, Reg((capture_count + i) as u16), val);
    }

    // Jump to function start
    state.set_pc(0);
    Ok(DispatchResult::Continue)
}

/// Tail call: reuses current stack frame
///
/// Format: `[0x5C] [func_id:varint] [args:reg_range]`
pub(in super::super) fn handle_tail_call_op(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let func_id = FunctionId(read_varint(state)? as u32);
    let args = read_reg_range(state)?;

    // Get function descriptor
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;

    let reg_count = func.register_count;
    let current_base = state.reg_base();

    // Collect arguments first (before we modify registers)
    let arg_values: Vec<Value> = (0..args.count as u16)
        .map(|i| state.registers.get(current_base, Reg(args.start.0 + i)))
        .collect();

    // Update current frame to new function
    if let Some(frame) = state.call_stack.current_mut() {
        frame.function = func_id;
        frame.pc = 0;
        frame.reg_count = reg_count;
    }

    // Copy arguments to start of frame
    for (i, value) in arg_values.into_iter().enumerate() {
        state.registers.set(current_base, Reg(i as u16), value);
    }

    // Reset PC to start of new function (also in frame)
    state.set_pc(0);

    Ok(DispatchResult::Continue)
}

/// Create closure: `dst = closure(fn_id, captures...)`
/// Encoding: opcode + dst:reg + func_id:varint + captures:reg_vec
pub(in super::super) fn handle_new_closure(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let func_id = read_varint(state)? as u32;
    let capture_count = read_varint(state)? as usize;

    let mut capture_regs = Vec::with_capacity(capture_count);
    for _ in 0..capture_count {
        capture_regs.push(read_reg(state)?);
    }

    // Allocate closure object on heap
    // Layout: [ObjectHeader][func_id:u32][capture_count:u32][captures:Value...]
    let data_size = 8 + capture_count * std::mem::size_of::<Value>();
    let type_id = TypeId(0xC000); // Closure type ID

    let obj = state.heap.alloc_with_init(
        type_id,
        data_size,
        |data| {
            let ptr = data.as_mut_ptr();
            unsafe {
                // Write func_id
                *(ptr as *mut u32) = func_id;
                // Write capture count
                *(ptr.add(4) as *mut u32) = capture_count as u32;
            }
        },
    )?;
    state.record_allocation();

    // Write captured values
    let base_ptr = obj.as_ptr() as *mut u8;
    let captures_offset = super::super::super::heap::OBJECT_HEADER_SIZE + 8;
    for (i, cap_reg) in capture_regs.iter().enumerate() {
        let val = state.get_reg(*cap_reg);
        unsafe {
            let cap_ptr = base_ptr.add(captures_offset + i * std::mem::size_of::<Value>()) as *mut Value;
            std::ptr::write(cap_ptr, val);
        }
    }

    let closure_val = Value::from_ptr(base_ptr);
    state.set_reg(dst, closure_val);
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Intrinsic Function Interception for External/Library Calls
// ============================================================================

/// Attempts to dispatch a function call as an intrinsic by looking up the
/// function's name in the module string table. This handles external functions
/// declared with @intrinsic("llvm.xxx") that have no bytecode body
/// (bytecode_length == 0).
///
/// Returns `Ok(Some(result))` if the intrinsic was handled, `Ok(None)` if not
/// recognized (caller should fall through to normal call path).
///
/// Covers:
/// - F64/F32 math functions (sqrt, sin, cos, tan, exp, log, pow, etc.)
/// - Rounding functions (floor, ceil, round, trunc)
/// - Special float ops (abs, copysign, fma, hypot, cbrt)
/// - Float classification (is_nan, is_inf, is_finite)
/// - Saturating arithmetic (saturating_add/sub/mul)
/// - Platform intrinsics (num_cpus, abort, cbgr_advance_epoch)
/// - Tier/async stubs (tier_promote, get_tier, future_poll_sync)
fn try_dispatch_intrinsic_by_name(
    state: &mut InterpreterState,
    func_name_id: StringId,
    _dst: Reg,
    args_start: Reg,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    // Snapshot func name + args eagerly so we don't carry an immutable
    // borrow of `state` through the body — TCP/UDP intrinsics need
    // `&mut state` to allocate Text return values.
    let func_name: String = match state.module.strings.get(func_name_id) {
        Some(name) => name.to_string(),
        None => return Ok(None),
    };

    // Helper closures to read argument values from the caller's frame
    let get_arg = |state: &InterpreterState, idx: u8| -> Value {
        if idx < arg_count {
            state.registers.get(caller_base, Reg(args_start.0 + idx as u16))
        } else {
            Value::from_i64(0)
        }
    };

    let get_f64_arg = |state: &InterpreterState, idx: u8| -> f64 {
        get_arg(state, idx).as_f64()
    };

    let get_i64_arg = |state: &InterpreterState, idx: u8| -> i64 {
        get_arg(state, idx).as_i64()
    };

    // Normalize function name: strip common prefixes and qualifications
    // e.g., "math.sqrt" -> "sqrt", "elementary.sqrt" -> "sqrt",
    //        "llvm.sqrt.f64" -> handled directly
    let name = func_name.as_str();

    // Match against known intrinsic function names.
    // We check both qualified names (e.g., "llvm.sqrt.f64") and bare names
    // (e.g., "sqrt", "sqrt_f64") to handle all codegen paths.
    match name {
        // ================================================================
        // F64 Square Root
        // ================================================================
        "sqrt" | "sqrt_f64" | "llvm.sqrt.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.sqrt())))
        }
        "sqrt_f32" | "llvm.sqrt.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.sqrt() as f64)))
        }

        // ================================================================
        // F64 Trigonometric
        // ================================================================
        "sin" | "sin_f64" | "llvm.sin.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.sin())))
        }
        "cos" | "cos_f64" | "llvm.cos.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.cos())))
        }
        "tan" | "tan_f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.tan())))
        }
        "asin" | "asin_f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.asin())))
        }
        "acos" | "acos_f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.acos())))
        }
        "atan" | "atan_f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.atan())))
        }
        "atan2" | "atan2_f64" => {
            let y = get_f64_arg(state, 0);
            let x = get_f64_arg(state, 1);
            Ok(Some(Value::from_f64(y.atan2(x))))
        }

        // ================================================================
        // F32 Trigonometric
        // ================================================================
        "sin_f32" | "llvm.sin.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.sin() as f64)))
        }
        "cos_f32" | "llvm.cos.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.cos() as f64)))
        }
        "tan_f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.tan() as f64)))
        }
        "asin_f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.asin() as f64)))
        }
        "acos_f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.acos() as f64)))
        }
        "atan_f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.atan() as f64)))
        }
        "atan2_f32" => {
            let y = get_f64_arg(state, 0) as f32;
            let x = get_f64_arg(state, 1) as f32;
            Ok(Some(Value::from_f64(y.atan2(x) as f64)))
        }

        // ================================================================
        // F64 Hyperbolic
        // ================================================================
        "sinh" | "sinh_f64" | "llvm.sinh.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.sinh())))
        }
        "cosh" | "cosh_f64" | "llvm.cosh.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.cosh())))
        }
        "tanh" | "tanh_f64" | "llvm.tanh.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.tanh())))
        }
        "asinh" | "asinh_f64" | "llvm.asinh.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.asinh())))
        }
        "acosh" | "acosh_f64" | "llvm.acosh.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.acosh())))
        }
        "atanh" | "atanh_f64" | "llvm.atanh.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.atanh())))
        }

        // ================================================================
        // F32 Hyperbolic
        // ================================================================
        "sinh_f32" | "llvm.sinh.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.sinh() as f64)))
        }
        "cosh_f32" | "llvm.cosh.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.cosh() as f64)))
        }
        "tanh_f32" | "llvm.tanh.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.tanh() as f64)))
        }

        // ================================================================
        // F64 Exponential / Logarithmic
        // ================================================================
        "exp" | "exp_f64" | "llvm.exp.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.exp())))
        }
        "exp2" | "exp2_f64" | "llvm.exp2.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.exp2())))
        }
        "expm1" | "expm1_f64" | "llvm.expm1.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.exp_m1())))
        }
        "log" | "log_f64" | "llvm.log.f64" | "ln" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.ln())))
        }
        "log2" | "log2_f64" | "llvm.log2.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.log2())))
        }
        "log10" | "log10_f64" | "llvm.log10.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.log10())))
        }
        "log1p" | "log1p_f64" | "llvm.log1p.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.ln_1p())))
        }
        "pow" | "pow_f64" | "llvm.pow.f64" => {
            let base = get_f64_arg(state, 0);
            let exp = get_f64_arg(state, 1);
            Ok(Some(Value::from_f64(base.powf(exp))))
        }
        "powi" | "powi_f64" | "llvm.powi.f64" | "llvm.powi.f64.i32" => {
            let base = get_f64_arg(state, 0);
            let exp = get_i64_arg(state, 1) as i32;
            Ok(Some(Value::from_f64(base.powi(exp))))
        }

        // ================================================================
        // F32 Exponential / Logarithmic
        // ================================================================
        "exp_f32" | "llvm.exp.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.exp() as f64)))
        }
        "exp2_f32" | "llvm.exp2.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.exp2() as f64)))
        }
        "expm1_f32" | "llvm.expm1.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.exp_m1() as f64)))
        }
        "log_f32" | "llvm.log.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.ln() as f64)))
        }
        "log2_f32" | "llvm.log2.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.log2() as f64)))
        }
        "log10_f32" | "llvm.log10.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.log10() as f64)))
        }
        "log1p_f32" | "llvm.log1p.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.ln_1p() as f64)))
        }
        "pow_f32" | "llvm.pow.f32" => {
            let base = get_f64_arg(state, 0) as f32;
            let exp = get_f64_arg(state, 1) as f32;
            Ok(Some(Value::from_f64(base.powf(exp) as f64)))
        }
        "powi_f32" | "llvm.powi.f32" | "llvm.powi.f32.i32" => {
            let base = get_f64_arg(state, 0) as f32;
            let exp = get_i64_arg(state, 1) as i32;
            Ok(Some(Value::from_f64(base.powi(exp) as f64)))
        }

        // ================================================================
        // F64 Root / Power
        // ================================================================
        "cbrt" | "cbrt_f64" | "llvm.cbrt.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.cbrt())))
        }
        "hypot" | "hypot_f64" | "llvm.hypot.f64" => {
            let x = get_f64_arg(state, 0);
            let y = get_f64_arg(state, 1);
            Ok(Some(Value::from_f64(x.hypot(y))))
        }

        // ================================================================
        // F32 Root / Power
        // ================================================================
        "cbrt_f32" | "llvm.cbrt.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.cbrt() as f64)))
        }
        "hypot_f32" | "llvm.hypot.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            let y = get_f64_arg(state, 1) as f32;
            Ok(Some(Value::from_f64(x.hypot(y) as f64)))
        }

        // ================================================================
        // F64 Rounding
        // ================================================================
        "floor" | "floor_f64" | "llvm.floor.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.floor())))
        }
        "ceil" | "ceil_f64" | "llvm.ceil.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.ceil())))
        }
        "round" | "round_f64" | "llvm.round.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.round())))
        }
        "trunc" | "trunc_f64" | "llvm.trunc.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.trunc())))
        }
        "rint" | "rint_f64" | "llvm.rint.f64" => {
            let x = get_f64_arg(state, 0);
            // IEEE 754 round-to-nearest-even
            Ok(Some(Value::from_f64(x.round_ties_even())))
        }

        // ================================================================
        // F32 Rounding
        // ================================================================
        "floor_f32" | "llvm.floor.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.floor() as f64)))
        }
        "ceil_f32" | "llvm.ceil.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.ceil() as f64)))
        }
        "round_f32" | "llvm.round.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.round() as f64)))
        }
        "trunc_f32" | "llvm.trunc.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.trunc() as f64)))
        }

        // ================================================================
        // F64 Special
        // ================================================================
        "abs" | "abs_f64" | "fabs" | "llvm.fabs.f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_f64(x.abs())))
        }
        "copysign" | "copysign_f64" | "llvm.copysign.f64" => {
            let mag = get_f64_arg(state, 0);
            let sign = get_f64_arg(state, 1);
            Ok(Some(Value::from_f64(mag.copysign(sign))))
        }
        "fma" | "fma_f64" | "llvm.fma.f64" => {
            let a = get_f64_arg(state, 0);
            let b = get_f64_arg(state, 1);
            let c = get_f64_arg(state, 2);
            Ok(Some(Value::from_f64(a.mul_add(b, c))))
        }
        "minnum" | "minnum_f64" | "llvm.minnum.f64" => {
            let x = get_f64_arg(state, 0);
            let y = get_f64_arg(state, 1);
            Ok(Some(Value::from_f64(x.min(y))))
        }
        "maxnum" | "maxnum_f64" | "llvm.maxnum.f64" => {
            let x = get_f64_arg(state, 0);
            let y = get_f64_arg(state, 1);
            Ok(Some(Value::from_f64(x.max(y))))
        }

        // ================================================================
        // F32 Special
        // ================================================================
        "abs_f32" | "llvm.fabs.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_f64(x.abs() as f64)))
        }
        "copysign_f32" | "llvm.copysign.f32" => {
            let mag = get_f64_arg(state, 0) as f32;
            let sign = get_f64_arg(state, 1) as f32;
            Ok(Some(Value::from_f64(mag.copysign(sign) as f64)))
        }
        "fma_f32" | "llvm.fma.f32" => {
            let a = get_f64_arg(state, 0) as f32;
            let b = get_f64_arg(state, 1) as f32;
            let c = get_f64_arg(state, 2) as f32;
            Ok(Some(Value::from_f64(a.mul_add(b, c) as f64)))
        }
        "minnum_f32" | "llvm.minnum.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            let y = get_f64_arg(state, 1) as f32;
            Ok(Some(Value::from_f64(x.min(y) as f64)))
        }
        "maxnum_f32" | "llvm.maxnum.f32" => {
            let x = get_f64_arg(state, 0) as f32;
            let y = get_f64_arg(state, 1) as f32;
            Ok(Some(Value::from_f64(x.max(y) as f64)))
        }

        // ================================================================
        // Float Classification
        // ================================================================
        "is_nan" | "is_nan_f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_bool(x.is_nan())))
        }
        "is_inf" | "is_infinite" | "is_infinite_f64" | "is_inf_f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_bool(x.is_infinite())))
        }
        "is_finite" | "is_finite_f64" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_bool(x.is_finite())))
        }
        "is_nan_f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_bool(x.is_nan())))
        }
        "is_infinite_f32" | "is_inf_f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_bool(x.is_infinite())))
        }
        "is_finite_f32" => {
            let x = get_f64_arg(state, 0) as f32;
            Ok(Some(Value::from_bool(x.is_finite())))
        }

        // ================================================================
        // Saturating Arithmetic
        // ================================================================
        "verum_saturating_add" | "verum_saturating_add_i64"
        | "verum_saturating_add_i128" | "saturating_add" => {
            let a = get_i64_arg(state, 0);
            let b = get_i64_arg(state, 1);
            Ok(Some(Value::from_i64(a.saturating_add(b))))
        }
        "verum_saturating_sub" | "verum_saturating_sub_i64"
        | "verum_saturating_sub_i128" | "saturating_sub" => {
            let a = get_i64_arg(state, 0);
            let b = get_i64_arg(state, 1);
            Ok(Some(Value::from_i64(a.saturating_sub(b))))
        }
        "verum_saturating_mul" | "verum_saturating_mul_i64"
        | "verum_saturating_mul_i128" | "saturating_mul" => {
            let a = get_i64_arg(state, 0);
            let b = get_i64_arg(state, 1);
            Ok(Some(Value::from_i64(a.saturating_mul(b))))
        }

        // ================================================================
        // Platform Intrinsics
        // ================================================================
        "verum_num_cpus" | "num_cpus" => {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get() as i64)
                .unwrap_or(1);
            Ok(Some(Value::from_i64(cpus)))
        }
        "abort" | "verum_abort" => {
            Err(InterpreterError::Panic { message: "abort() called".to_string() })
        }

        // ================================================================
        // CBGR Intrinsics (no-ops in interpreter)
        // ================================================================
        "verum_cbgr_advance_epoch" | "cbgr_advance_epoch"
        | "verum_cbgr_new_generation" | "verum_cbgr_invalidate"
        | "verum_cbgr_get_generation" | "verum_cbgr_advance_generation"
        | "verum_cbgr_get_epoch_caps" | "verum_cbgr_get_stats" => {
            Ok(Some(Value::from_i64(0)))
        }

        // ================================================================
        // Tier / Async Stubs (no-ops in interpreter)
        // ================================================================
        "verum_tier_promote" | "tier_promote" => {
            // No-op: JIT promotion not available in interpreter
            Ok(Some(Value::unit()))
        }
        "verum_get_tier" | "get_tier" => {
            // Interpreter is always tier 0
            Ok(Some(Value::from_i64(0)))
        }
        "is_interpreted" => {
            Ok(Some(Value::from_bool(true)))
        }
        "verum_future_poll_sync" | "future_poll_sync" => {
            // Return false/nil: async not supported in tier 0
            Ok(Some(Value::from_bool(false)))
        }
        "verum_supervisor_set_parent" | "supervisor_set_parent" => {
            // No-op: supervisor hierarchy not available
            Ok(Some(Value::unit()))
        }
        "verum_exec_with_recovery" | "exec_with_recovery" => {
            // Execute without recovery: just call the factory directly
            // Return unit as placeholder
            Ok(Some(Value::unit()))
        }
        "verum_shared_registry_global" | "shared_registry_global" => {
            // Return nil: no shared registry in interpreter
            Ok(Some(Value::nil()))
        }
        "verum_middleware_chain_empty" | "middleware_chain_empty" => {
            // Return unit as empty chain
            Ok(Some(Value::unit()))
        }

        // ================================================================
        // Type conversion intrinsics
        // ================================================================
        "int_to_float" | "sitofp" | "uitofp" => {
            let x = get_i64_arg(state, 0);
            Ok(Some(Value::from_f64(x as f64)))
        }
        "float_to_int" | "fptosi" | "fptoui" => {
            let x = get_f64_arg(state, 0);
            Ok(Some(Value::from_i64(x as i64)))
        }

        // ================================================================
        // Runtime Intrinsics — Real implementations for interpreter
        // These mirror the AOT C runtime functions, using Rust std.
        // ================================================================

        // --- File I/O (interpreter: basic support via Rust std) ---
        "__file_read_to_string_raw" => {
            // In interpreter, file paths are NaN-boxed values — extract would need
            // VBC string infrastructure. Return nil for now; use AOT for file I/O.
            Ok(Some(Value::nil()))
        }
        "__file_write_string_raw" => Ok(Some(Value::from_i64(-1))),
        "__file_open_raw" | "__file_close_raw" | "__file_size_raw" |
        "__file_seek_raw" | "__file_delete_raw" | "__mkdir_raw" => {
            Ok(Some(Value::from_i64(0)))
        }

        // --- Command-line Arguments ---
        "__args_count_raw" => {
            Ok(Some(Value::from_i64(std::env::args().count() as i64)))
        }
        "__arg_raw" => {
            let idx = get_i64_arg(state, 0) as usize;
            match std::env::args().nth(idx) {
                Some(arg) => {
                    // Use small string if fits, otherwise nil
                    Ok(Some(Value::from_small_string(&arg).unwrap_or(Value::nil())))
                }
                None => Ok(Some(Value::nil())),
            }
        }

        // --- Time ---
        "__time_monotonic_nanos_raw" => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as i64;
            Ok(Some(Value::from_i64(nanos)))
        }
        "__time_sleep_nanos_raw" => {
            let ns = get_i64_arg(state, 0);
            if ns > 0 {
                std::thread::sleep(std::time::Duration::from_nanos(ns as u64));
            }
            Ok(Some(Value::from_i64(0)))
        }
        "__time_now_ms_raw" => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            Ok(Some(Value::from_i64(ms)))
        }

        // --- Synchronization (single-threaded interpreter stubs) ---
        "__mutex_new_raw" => Ok(Some(Value::from_i64(1))),  // fake handle
        "__mutex_lock_raw" | "__mutex_unlock_raw" | "__mutex_trylock_raw" => {
            Ok(Some(Value::from_i64(0)))  // always succeed
        }
        "__cond_new_raw" => Ok(Some(Value::from_i64(1))),
        "__cond_wait_raw" | "__cond_timedwait_raw" => Ok(Some(Value::from_i64(0))),
        "__cond_signal_raw" | "__cond_broadcast_raw" => Ok(Some(Value::from_i64(0))),
        "__waitgroup_new_raw" => Ok(Some(Value::from_i64(1))),
        "__waitgroup_add_raw" | "__waitgroup_done_raw" | "__waitgroup_wait_raw" |
        "__waitgroup_destroy_raw" => Ok(Some(Value::from_i64(0))),
        "__gen_close_raw" => Ok(Some(Value::from_i64(0))),

        // --- IO Engine (interpreter: no-op, returns fake handles) ---
        "__io_engine_new_raw" => Ok(Some(Value::from_i64(1))),
        "__io_engine_destroy_raw" | "__io_submit_raw" | "__io_remove_raw" |
        "__io_modify_raw" => Ok(Some(Value::from_i64(0))),
        "__io_poll_raw" => Ok(Some(Value::from_i64(0))),  // no events

        // --- Thread Pool (interpreter: single-threaded execution) ---
        "__pool_create_raw" => Ok(Some(Value::from_i64(1))),
        "__pool_submit_raw" | "__pool_await_raw" | "__pool_destroy_raw" |
        "__pool_global_submit_raw" => Ok(Some(Value::from_i64(0))),

        // --- Socket Options ---
        "__socket_set_nonblocking_raw" | "__socket_set_blocking_raw" |
        "__socket_set_reuseaddr_raw" | "__socket_set_nodelay_raw" |
        "__socket_set_keepalive_raw" | "__socket_get_error_raw" => {
            Ok(Some(Value::from_i64(0)))
        }
        "__async_accept_raw" | "__async_read_raw" | "__async_write_raw" => {
            Ok(Some(Value::from_i64(-1)))  // not available in interpreter
        }

        // --- TCP Networking (Tier-0 std-net backed; see handlers/net_runtime.rs) ---
        "__tcp_listen_raw" | "tcp_listen" => {
            let port = get_i64_arg(state, 0);
            Ok(Some(Value::from_i64(super::net_runtime::tcp_listen(port))))
        }
        "__tcp_accept_raw" | "tcp_accept" => {
            let listen_fd = get_i64_arg(state, 0);
            Ok(Some(Value::from_i64(super::net_runtime::tcp_accept(listen_fd))))
        }
        "__tcp_connect_raw" | "tcp_connect" => {
            let host = super::string_helpers::resolve_string_value(&get_arg(state, 0), state);
            let port = get_i64_arg(state, 1);
            Ok(Some(Value::from_i64(super::net_runtime::tcp_connect(&host, port))))
        }
        "__tcp_send_raw" | "tcp_send" => {
            let fd = get_i64_arg(state, 0);
            let data = super::string_helpers::resolve_string_value(&get_arg(state, 1), state);
            Ok(Some(Value::from_i64(
                super::net_runtime::tcp_send(fd, data.as_bytes()),
            )))
        }
        "__tcp_recv_raw" | "tcp_recv" => {
            let fd = get_i64_arg(state, 0);
            let max_len = get_i64_arg(state, 1);
            let body = super::net_runtime::tcp_recv(fd, max_len).unwrap_or_default();
            let v = super::string_helpers::alloc_string_value(state, &body)?;
            Ok(Some(v))
        }
        "__tcp_close_raw" | "tcp_close" => {
            let fd = get_i64_arg(state, 0);
            Ok(Some(Value::from_i64(super::net_runtime::tcp_close(fd))))
        }
        // Rich-signature listen — backs `core.net.tcp.TcpListener.bind`
        // by way of the unified intrinsic (Route A of the weft TCP-bind
        // architectural-fork closure). Failures return `-errno` so the
        // Verum-side `IoError.from_raw_os_error` mapping is lossless.
        "__tcp_listen_v2_raw" | "tcp_listen_v2" => {
            let host = super::string_helpers::resolve_string_value(&get_arg(state, 0), state);
            let port = get_i64_arg(state, 1);
            let backlog = get_i64_arg(state, 2);
            let flags = get_i64_arg(state, 3);
            Ok(Some(Value::from_i64(
                super::net_runtime::tcp_listen_v2(&host, port, backlog, flags),
            )))
        }
        // Companion to `__tcp_listen_v2_raw`: retrieves the OS-assigned
        // local port after `port=0` binds. Also works for connected
        // streams and UDP sockets.
        "__tcp_local_port_raw" | "tcp_local_port" => {
            let fd = get_i64_arg(state, 0);
            Ok(Some(Value::from_i64(super::net_runtime::tcp_local_port(fd))))
        }
        "__udp_bind_raw" | "udp_bind" => {
            let port = get_i64_arg(state, 0);
            Ok(Some(Value::from_i64(super::net_runtime::udp_bind(port))))
        }
        "__udp_send_raw" | "udp_send" => {
            let fd = get_i64_arg(state, 0);
            let data = super::string_helpers::resolve_string_value(&get_arg(state, 1), state);
            let host = super::string_helpers::resolve_string_value(&get_arg(state, 2), state);
            let port = get_i64_arg(state, 3);
            Ok(Some(Value::from_i64(
                super::net_runtime::udp_send(fd, data.as_bytes(), &host, port),
            )))
        }
        "__udp_recv_raw" | "udp_recv" => {
            let fd = get_i64_arg(state, 0);
            let max_len = get_i64_arg(state, 1);
            let body = super::net_runtime::udp_recv(fd, max_len).unwrap_or_default();
            let v = super::string_helpers::alloc_string_value(state, &body)?;
            Ok(Some(v))
        }
        "__udp_close_raw" | "udp_close" => {
            let fd = get_i64_arg(state, 0);
            Ok(Some(Value::from_i64(super::net_runtime::udp_close(fd))))
        }

        // --- Context System ---
        "__ctx_get_raw" => Ok(Some(Value::nil())),
        "__ctx_provide_raw" | "__ctx_end_raw" => Ok(Some(Value::from_i64(0))),

        // --- Defer Cleanup ---
        "__defer_push_raw" | "__defer_pop_raw" | "__defer_run_to_raw" => {
            Ok(Some(Value::from_i64(0)))
        }
        "__defer_depth_raw" => Ok(Some(Value::from_i64(0))),

        // --- Memory Allocation ---
        "__alloc_raw" => {
            let size = get_i64_arg(state, 0) as usize;
            let layout = std::alloc::Layout::from_size_align(size.max(8), 8)
                .unwrap_or(std::alloc::Layout::new::<u64>());
            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            Ok(Some(Value::from_i64(ptr as i64)))
        }
        "__alloc_zeroed_raw" => {
            let size = get_i64_arg(state, 0) as usize;
            let layout = std::alloc::Layout::from_size_align(size.max(8), 8)
                .unwrap_or(std::alloc::Layout::new::<u64>());
            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            Ok(Some(Value::from_i64(ptr as i64)))
        }
        "__dealloc_raw" => {
            // Skip deallocation in interpreter to avoid double-free
            Ok(Some(Value::from_i64(0)))
        }

        // mmap-shaped allocation for the stdlib CBGR page allocator
        // (`core/mem/allocator.rs::os_mmap` → FFI `mmap`). In the Tier 0
        // interpreter we don't go through libffi/dlopen; route straight
        // to host allocation instead so `Shared::new`, `Map`, `List`
        // allocations work without requiring FFI.
        //
        // Semantics we implement:
        //   mmap(addr=0, len, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANON,
        //        fd=-1, off=0)  returns  ptr   (NOT -1/MAP_FAILED).
        //   mmap with fd != -1 or unexpected flags is passed through as -1
        //   (mapped-file case is out of scope for the interpreter — the
        //    AOT path owns that).
        "mmap" | "mmap_ffi" => {
            let fd = get_i64_arg(state, 4);
            let len = get_i64_arg(state, 1) as usize;
            if fd != -1 || len == 0 {
                return Ok(Some(Value::from_i64(-1)));
            }
            let layout = std::alloc::Layout::from_size_align(len.max(8), 16)
                .unwrap_or(std::alloc::Layout::new::<u64>());
            let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
            if ptr.is_null() {
                Ok(Some(Value::from_i64(-1)))
            } else {
                Ok(Some(Value::from_i64(ptr as i64)))
            }
        }

        // munmap: best-effort release. The interpreter doesn't track page
        // lifetimes (Shared/Heap refcounts are the stdlib's job); deallocating
        // here risks double-free when objects escape their original page.
        // Return 0 ("success") without freeing, matching the __dealloc_raw
        // semantics.
        "munmap" => Ok(Some(Value::from_i64(0))),

        // Memory-protection and advice syscalls: in the interpreter all
        // pages are host-allocated rwx by default, so these are no-ops
        // that must return 0 so the stdlib doesn't error.
        "mprotect" | "madvise" | "mlock" | "munlock" | "msync"
            | "madvise_willneed" | "madvise_dontneed" | "madvise_free"
            | "madvise_free_reusable" | "madvise_free_reuse" => {
            Ok(Some(Value::from_i64(0)))
        }

        // --- Process Management ---
        // Process bridge via core.sys.process_native — these legacy stubs
        // are kept for backwards-compat with bytecode that still references
        // the old C-runtime entrypoint names. Once all callers have been
        // moved to the native path they can be removed entirely.
        "__process_spawn_raw" | "__process_exec_raw"
            | "__process_spawn_full_raw"
            | "__process_wait_raw" | "__process_kill_raw" => {
            Ok(Some(Value::from_i64(-1)))
        }
        "__fd_read_all_raw" | "__fd_read_chunk_raw" => Ok(Some(Value::from_i64(0))),
        "__fd_write_all_raw" => Ok(Some(Value::from_i64(-1))),
        "__fd_close_raw" | "__fd_close_raw_buf" => Ok(Some(Value::from_i64(0))),
        "__ptr_read_i64" | "__ptr_free" => Ok(Some(Value::from_i64(0))),

        // --- Echo control (interactive password prompt) ---
        "__termios_save_and_disable_echo" | "__windows_save_and_disable_echo" => {
            Ok(Some(Value::from_i64(0)))
        }
        "__termios_restore_echo" | "__windows_restore_echo" => Ok(Some(Value::from_i64(0))),

        // --- Metal GPU (interpreter: not available) ---
        "__metal_get_device" => Ok(Some(Value::from_i64(0))),  // no GPU
        "__metal_device_name" | "__metal_max_memory" |
        "__metal_max_threads_per_threadgroup" | "__metal_gpu_core_count" => {
            Ok(Some(Value::from_i64(0)))
        }
        "__metal_alloc" | "__metal_alloc_with_data" | "__metal_buffer_contents" |
        "__metal_buffer_length" | "__metal_compile_shader" | "__metal_get_pipeline" => {
            Ok(Some(Value::from_i64(0)))
        }
        "__metal_free" | "__metal_wait" | "__metal_dispatch_1d" |
        "__metal_dispatch_2d" | "__metal_dispatch_async" => {
            Ok(Some(Value::from_i64(0)))
        }
        "__metal_execution_time_ns" | "__metal_vector_add_f32" |
        "__metal_sgemm" | "__metal_benchmark" => {
            Ok(Some(Value::from_i64(0)))
        }

        // --- LLVM Math Intrinsics ---
        "__llvm_sqrt" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.sqrt()))) }
        "__llvm_sin" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.sin()))) }
        "__llvm_cos" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.cos()))) }
        "__llvm_exp" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.exp()))) }
        "__llvm_log" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.ln()))) }
        "__llvm_pow" => { let x = get_f64_arg(state, 0); let y = get_f64_arg(state, 1); Ok(Some(Value::from_f64(x.powf(y)))) }
        "__llvm_fabs" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.abs()))) }
        "__llvm_floor" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.floor()))) }
        "__llvm_ceil" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.ceil()))) }
        "__llvm_round" => { let x = get_f64_arg(state, 0); Ok(Some(Value::from_f64(x.round()))) }
        "__llvm_copysign" => { let x = get_f64_arg(state, 0); let y = get_f64_arg(state, 1); Ok(Some(Value::from_f64(x.copysign(y)))) }
        "__llvm_minnum" => { let x = get_f64_arg(state, 0); let y = get_f64_arg(state, 1); Ok(Some(Value::from_f64(x.min(y)))) }
        "__llvm_maxnum" => { let x = get_f64_arg(state, 0); let y = get_f64_arg(state, 1); Ok(Some(Value::from_f64(x.max(y)))) }
        "__llvm_fma" => { let a = get_f64_arg(state, 0); let b = get_f64_arg(state, 1); let c = get_f64_arg(state, 2); Ok(Some(Value::from_f64(a.mul_add(b, c)))) }

        // --- Memory Operations ---
        "__llvm_memcpy" | "__llvm_memmove" => {
            let dst = get_i64_arg(state, 0);
            let src = get_i64_arg(state, 1);
            let n = get_i64_arg(state, 2) as usize;
            if dst != 0 && src != 0 && n > 0 {
                unsafe {
                    std::ptr::copy(src as *const u8, dst as *mut u8, n);
                }
            }
            Ok(Some(Value::from_i64(dst)))
        }
        "__llvm_memset" => {
            let dst = get_i64_arg(state, 0);
            let val = get_i64_arg(state, 1) as u8;
            let n = get_i64_arg(state, 2) as usize;
            if dst != 0 && n > 0 {
                unsafe {
                    std::ptr::write_bytes(dst as *mut u8, val, n);
                }
            }
            Ok(Some(Value::from_i64(dst)))
        }

        // --- Raw byte/word load/store ---
        "__load_byte" => {
            let addr = get_i64_arg(state, 0);
            if addr != 0 {
                let val = unsafe { *(addr as *const u8) };
                Ok(Some(Value::from_i64(val as i64)))
            } else {
                Ok(Some(Value::from_i64(0)))
            }
        }
        "__store_byte" => {
            let addr = get_i64_arg(state, 0);
            let val = get_i64_arg(state, 1) as u8;
            if addr != 0 {
                unsafe { *(addr as *mut u8) = val; }
            }
            Ok(Some(Value::from_i64(0)))
        }
        "__load_i64" => {
            let addr = get_i64_arg(state, 0);
            if addr != 0 {
                let val = unsafe { *(addr as *const i64) };
                Ok(Some(Value::from_i64(val)))
            } else {
                Ok(Some(Value::from_i64(0)))
            }
        }
        "__store_i64" => {
            let addr = get_i64_arg(state, 0);
            let val = get_i64_arg(state, 1);
            if addr != 0 {
                unsafe { *(addr as *mut i64) = val; }
            }
            Ok(Some(Value::from_i64(0)))
        }
        "__load_i32" => {
            let addr = get_i64_arg(state, 0);
            if addr != 0 {
                let val = unsafe { *(addr as *const i32) };
                Ok(Some(Value::from_i64(val as i64)))
            } else {
                Ok(Some(Value::from_i64(0)))
            }
        }
        "__store_i32" => {
            let addr = get_i64_arg(state, 0);
            let val = get_i64_arg(state, 1) as i32;
            if addr != 0 {
                unsafe { *(addr as *mut i32) = val; }
            }
            Ok(Some(Value::from_i64(0)))
        }

        // ================================================================
        // Not recognized — return None to fall through
        // ================================================================
        _ => {
            // Try suffix matching for qualified names like "math.sqrt", "elementary.sin"
            let suffix = name.rsplit('.').next().unwrap_or(name);
            if suffix != name {
                // Recurse with just the suffix
                return try_dispatch_intrinsic_by_suffix(state, suffix, args_start, arg_count, caller_base);
            }
            Ok(None)
        }
    }
}

/// Fallback dispatch for qualified function names: extracts the last segment
/// and matches against known intrinsic names. E.g., "math.elementary.sqrt" -> "sqrt".
fn try_dispatch_intrinsic_by_suffix(
    state: &InterpreterState,
    suffix: &str,
    args_start: Reg,
    arg_count: u8,
    caller_base: u32,
) -> InterpreterResult<Option<Value>> {
    let get_f64_arg = |state: &InterpreterState, idx: u8| -> f64 {
        if idx < arg_count {
            state.registers.get(caller_base, Reg(args_start.0 + idx as u16)).as_f64()
        } else {
            0.0
        }
    };

    let get_i64_arg = |state: &InterpreterState, idx: u8| -> i64 {
        if idx < arg_count {
            state.registers.get(caller_base, Reg(args_start.0 + idx as u16)).as_i64()
        } else {
            0
        }
    };

    match suffix {
        // Unary F64 math
        "sqrt" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).sqrt()))),
        "sin" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).sin()))),
        "cos" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).cos()))),
        "tan" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).tan()))),
        "asin" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).asin()))),
        "acos" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).acos()))),
        "atan" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).atan()))),
        "sinh" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).sinh()))),
        "cosh" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).cosh()))),
        "tanh" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).tanh()))),
        "asinh" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).asinh()))),
        "acosh" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).acosh()))),
        "atanh" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).atanh()))),
        "exp" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).exp()))),
        "exp2" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).exp2()))),
        "expm1" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).exp_m1()))),
        "ln" | "log" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).ln()))),
        "log2" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).log2()))),
        "log10" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).log10()))),
        "log1p" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).ln_1p()))),
        "cbrt" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).cbrt()))),
        "floor" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).floor()))),
        "ceil" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).ceil()))),
        "round" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).round()))),
        "trunc" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).trunc()))),
        "fabs" | "abs" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).abs()))),
        // Binary F64 math
        "atan2" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).atan2(get_f64_arg(state, 1))))),
        "pow" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).powf(get_f64_arg(state, 1))))),
        "hypot" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).hypot(get_f64_arg(state, 1))))),
        "copysign" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).copysign(get_f64_arg(state, 1))))),
        "minnum" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).min(get_f64_arg(state, 1))))),
        "maxnum" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).max(get_f64_arg(state, 1))))),
        // Ternary
        "fma" => Ok(Some(Value::from_f64(get_f64_arg(state, 0).mul_add(get_f64_arg(state, 1), get_f64_arg(state, 2))))),
        // Classification
        "is_nan" => Ok(Some(Value::from_bool(get_f64_arg(state, 0).is_nan()))),
        "is_infinite" | "is_inf" => Ok(Some(Value::from_bool(get_f64_arg(state, 0).is_infinite()))),
        "is_finite" => Ok(Some(Value::from_bool(get_f64_arg(state, 0).is_finite()))),
        // Saturating
        "saturating_add" => Ok(Some(Value::from_i64(get_i64_arg(state, 0).saturating_add(get_i64_arg(state, 1))))),
        "saturating_sub" => Ok(Some(Value::from_i64(get_i64_arg(state, 0).saturating_sub(get_i64_arg(state, 1))))),
        "saturating_mul" => Ok(Some(Value::from_i64(get_i64_arg(state, 0).saturating_mul(get_i64_arg(state, 1))))),
        // Not recognized
        _ => Ok(None),
    }
}
