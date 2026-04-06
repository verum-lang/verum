//! Core tensor opcode handlers for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

use crate::instruction::{TensorBinaryOp, TensorUnaryOp, TensorReduceOp};
use super::super::super::tensor::{TensorHandle, DType};

// ============================================================================
// Main Tensor Opcode Handlers (0xF0-0xF7)
// ============================================================================

/// Handler for TensorNew opcode (0xF0).
/// Creates a new tensor with specified shape and dtype.
///
/// Format: `dst:reg, shape_len:u8, shape..., dtype:u8`
pub(in super::super) fn handle_tensor_new(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let shape_len = read_u8(state)? as usize;
    let mut shape = Vec::with_capacity(shape_len);
    for _ in 0..shape_len {
        shape.push(read_varint(state)? as usize);
    }
    let dtype_byte = read_u8(state)?;
    let dtype = DType::from_type_id(dtype_byte);

    if let Some(tensor) = TensorHandle::zeros(&shape, dtype) {
        let ptr = Box::into_raw(Box::new(tensor));
        state.set_reg(dst, Value::from_ptr(ptr));
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorBinop opcode (0xF1).
/// Performs element-wise binary operation on tensors.
///
/// Format: `op:u8, dst:reg, a:reg, b:reg`
pub(in super::super) fn handle_tensor_binop(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let op_byte = read_u8(state)?;
    let dst = read_reg(state)?;
    let a_reg = read_reg(state)?;
    let b_reg = read_reg(state)?;

    let op = TensorBinaryOp::from_byte(op_byte);
    let a_val = state.get_reg(a_reg);
    let b_val = state.get_reg(b_reg);

    let a_ptr = a_val.as_ptr::<TensorHandle>();
    let b_ptr = b_val.as_ptr::<TensorHandle>();

    if !a_ptr.is_null() && !b_ptr.is_null() {
        let a = unsafe { &*a_ptr };
        let b = unsafe { &*b_ptr };

        if let Some(result) = super::super::super::tensor::tensor_binop(a, b, op) {
            let ptr = Box::into_raw(Box::new(result));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorUnop opcode (0xF2).
/// Performs element-wise unary operation on tensor.
///
/// Format: `op:u8, dst:reg, src:reg`
pub(in super::super) fn handle_tensor_unop(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let op_byte = read_u8(state)?;
    let dst = read_reg(state)?;
    let src_reg = read_reg(state)?;

    let op = TensorUnaryOp::from_byte(op_byte);
    let src_val = state.get_reg(src_reg);
    let src_ptr = src_val.as_ptr::<TensorHandle>();

    if !src_ptr.is_null() {
        let src = unsafe { &*src_ptr };

        if let Some(result) = super::super::super::tensor::tensor_unop(src, op) {
            let ptr = Box::into_raw(Box::new(result));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorMatmul opcode (0xF3).
/// Performs matrix multiplication on tensors.
///
/// Format: `dst:reg, a:reg, b:reg`
pub(in super::super) fn handle_tensor_matmul(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a_reg = read_reg(state)?;
    let b_reg = read_reg(state)?;

    let a_val = state.get_reg(a_reg);
    let b_val = state.get_reg(b_reg);

    let a_ptr = a_val.as_ptr::<TensorHandle>();
    let b_ptr = b_val.as_ptr::<TensorHandle>();

    if !a_ptr.is_null() && !b_ptr.is_null() {
        let a = unsafe { &*a_ptr };
        let b = unsafe { &*b_ptr };

        if let Some(result) = super::super::super::tensor::tensor_matmul(a, b) {
            let ptr = Box::into_raw(Box::new(result));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorReduce opcode (0xF4).
/// Performs reduction operation along specified axes.
///
/// Format: `op:u8, dst:reg, src:reg, axes_len:u8, axes..., keepdim:bool`
pub(in super::super) fn handle_tensor_reduce(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let op_byte = read_u8(state)?;
    let dst = read_reg(state)?;
    let src_reg = read_reg(state)?;
    let axes_len = read_u8(state)? as usize;
    let mut axes = Vec::with_capacity(axes_len);
    for _ in 0..axes_len {
        axes.push(read_u8(state)? as usize);
    }
    let _keepdim = read_u8(state)? != 0;

    let op = TensorReduceOp::from_byte(op_byte);
    let src_val = state.get_reg(src_reg);
    let src_ptr = src_val.as_ptr::<TensorHandle>();

    if !src_ptr.is_null() {
        let src = unsafe { &*src_ptr };

        // For single axis reduction
        let axis = if axes.is_empty() { None } else { Some(axes[0]) };

        if let Some(result) = super::super::super::tensor::tensor_reduce(src, axis, op) {
            let ptr = Box::into_raw(Box::new(result));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorReshape opcode (0xF5).
/// Reshapes tensor to new shape.
///
/// Format: `dst:reg, src:reg, shape_len:u8, shape...`
pub(in super::super) fn handle_tensor_reshape(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src_reg = read_reg(state)?;
    let shape_len = read_u8(state)? as usize;
    let mut new_shape = Vec::with_capacity(shape_len);
    for _ in 0..shape_len {
        new_shape.push(read_varint(state)? as usize);
    }

    let src_val = state.get_reg(src_reg);
    let src_ptr = src_val.as_ptr::<TensorHandle>();

    if !src_ptr.is_null() {
        let src = unsafe { &*src_ptr };

        if let Some(result) = super::super::super::tensor::tensor_reshape(src, &new_shape) {
            let ptr = Box::into_raw(Box::new(result));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorTranspose opcode (0xF6).
/// Transposes tensor (swaps last two dimensions).
///
/// Format: `dst:reg, src:reg`
pub(in super::super) fn handle_tensor_transpose(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src_reg = read_reg(state)?;

    let src_val = state.get_reg(src_reg);
    let src_ptr = src_val.as_ptr::<TensorHandle>();

    if !src_ptr.is_null() {
        let src = unsafe { &*src_ptr };

        if let Some(result) = super::super::super::tensor::tensor_transpose(src) {
            let ptr = Box::into_raw(Box::new(result));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorSlice opcode (0xF7).
/// Slices tensor along specified ranges.
///
/// Format: `dst:reg, src:reg, ranges_len:u8, (start, end)...`
pub(in super::super) fn handle_tensor_slice(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src_reg = read_reg(state)?;
    let ranges_len = read_u8(state)? as usize;
    let mut ranges = Vec::with_capacity(ranges_len);
    for _ in 0..ranges_len {
        let start = read_varint(state)? as usize;
        let end = read_varint(state)? as usize;
        ranges.push((start, end));
    }

    let src_val = state.get_reg(src_reg);
    let src_ptr = src_val.as_ptr::<TensorHandle>();

    if !src_ptr.is_null() {
        let src = unsafe { &*src_ptr };

        if let Some(result) = super::super::super::tensor::tensor_slice(src, &ranges) {
            let ptr = Box::into_raw(Box::new(result));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

// ============================================================================

/// Handler for TensorFull opcode (0xFE).
/// Creates a tensor filled with a constant value.
///
/// Format: `dst:reg, value:reg, shape_len:varint, shape_regs..., dtype:u8`
pub(in super::super) fn handle_tensor_full(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let value_reg = read_reg(state)?;
    let shape_len = read_varint(state)? as usize;
    let mut shape = Vec::with_capacity(shape_len);
    for _ in 0..shape_len {
        let reg = read_reg(state)?;
        shape.push(state.get_reg(reg).as_i64() as usize);
    }
    let dtype_byte = read_u8(state)?;
    let dtype = DType::from_type_id(dtype_byte);

    let fill_value = state.get_reg(value_reg);
    let fill_f64 = if fill_value.is_float() {
        fill_value.as_f64()
    } else {
        fill_value.as_i64() as f64
    };

    if let Some(tensor) = TensorHandle::full(&shape, dtype, fill_f64) {
        let ptr = Box::into_raw(Box::new(tensor));
        state.set_reg(dst, Value::from_ptr(ptr));
    } else {
        state.set_reg(dst, Value::nil());
    }
    Ok(DispatchResult::Continue)
}

/// Handler for TensorFromSlice opcode (0xFF).
/// Creates a tensor from a data source (list of values).
///
/// Format: `dst:reg, data:reg, shape_len:varint, shape_regs..., dtype:u8`
pub(in super::super) fn handle_tensor_from_slice(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let data_reg = read_reg(state)?;
    let shape_len = read_varint(state)? as usize;
    let mut shape = Vec::with_capacity(shape_len);
    for _ in 0..shape_len {
        let reg = read_reg(state)?;
        shape.push(state.get_reg(reg).as_i64() as usize);
    }
    let dtype_byte = read_u8(state)?;
    let dtype = DType::from_type_id(dtype_byte);

    // Get data from register - could be a pointer to Vec<Value>
    let data_val = state.get_reg(data_reg);
    let data_ptr = data_val.as_ptr::<Vec<Value>>();

    if !data_ptr.is_null() {
        let values = unsafe { &*data_ptr };
        let total_elements: usize = shape.iter().product();
        let num_values = values.len().min(total_elements);

        // Create tensor filled with zeros, then write values
        if let Some(tensor) = TensorHandle::zeros(&shape, dtype) {
            if let Some(data) = &tensor.data {
                unsafe {
                    let buf_ptr = (*data.as_ptr()).as_mut_ptr();
                    let elem_size = dtype.size_bytes();
                    for i in 0..num_values {
                        let f = if values[i].is_float() {
                            values[i].as_f64()
                        } else {
                            values[i].as_i64() as f64
                        };
                        match dtype {
                            DType::F32 => {
                                let p = buf_ptr.add(i * elem_size) as *mut f32;
                                *p = f as f32;
                            }
                            DType::F64 => {
                                let p = buf_ptr.add(i * elem_size) as *mut f64;
                                *p = f;
                            }
                            DType::I32 => {
                                let p = buf_ptr.add(i * elem_size) as *mut i32;
                                *p = f as i32;
                            }
                            DType::I64 => {
                                let p = buf_ptr.add(i * elem_size) as *mut i64;
                                *p = f as i64;
                            }
                            _ => {
                                let p = buf_ptr.add(i * elem_size) as *mut f32;
                                *p = f as f32;
                            }
                        }
                    }
                }
            }
            let ptr = Box::into_raw(Box::new(tensor));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        // If data is not a vec pointer, try to create empty tensor
        if let Some(tensor) = TensorHandle::zeros(&shape, dtype) {
            let ptr = Box::into_raw(Box::new(tensor));
            state.set_reg(dst, Value::from_ptr(ptr));
        } else {
            state.set_reg(dst, Value::nil());
        }
    }
    Ok(DispatchResult::Continue)
}
