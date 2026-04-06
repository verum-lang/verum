//! Tensor extended opcode handler for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use crate::instruction::{Opcode, TensorSubOpcode, TensorExtSubOpcode, TensorBinaryOp, TensorUnaryOp, TensorReduceOp};
use super::bytecode_io::*;
use super::super::alloc_list_from_values;

// ============================================================================
// Tensor Extended Handler (0xFC)
// ============================================================================

/// Handler for TensorExtended opcode (0xFC).
///
/// This dispatches to tensor operations based on the sub-opcode byte.
/// Uses the tensor.rs module for actual tensor computations.
pub(in super::super) fn handle_tensor_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    let sub_op = TensorSubOpcode::from_byte(sub_op_byte);

    use super::super::super::tensor::{
        TensorHandle, PoolOp,
        tensor_pool2d, tensor_argmin, tensor_permute, tensor_softmax, tensor_argmax,
        tensor_layer_norm, tensor_batch_norm,
        tensor_batch_matmul, tensor_topk, tensor_conv2d,
        tensor_clone,
    };
    use super::super::super::kernel::{
        dispatch_solve, dispatch_gather, dispatch_qr, dispatch_svd, dispatch_lu,
        dispatch_eig, dispatch_nansum, dispatch_nanmean, dispatch_flip, dispatch_roll,
        dispatch_lstsq, dispatch_trisolve, dispatch_eig_symmetric, dispatch_schur,
        dispatch_rank, dispatch_cond, dispatch_norm, dispatch_kron, dispatch_cross,
        dispatch_contract, dispatch_matrix_power, dispatch_expm, dispatch_logm,
        dispatch_inverse, dispatch_rfft, dispatch_irfft, dispatch_complex_mul,
        dispatch_complex_pow, dispatch_ssm_scan, dispatch_uniform, dispatch_bincount,
        dispatch_gather_nd, dispatch_arange_usize, dispatch_repeat, dispatch_tanh,
        dispatch_sum_all, dispatch_from_array, dispatch_random_float_01,
        dispatch_sample_top_p, dispatch_sample_temperature, dispatch_paged_attention,
        dispatch_parse_tool_call, dispatch_format_value, dispatch_tensor_from_slice_usize,
        dispatch_quantized_matmul, dispatch_tensor_norm, dispatch_generate_request_id,
        dispatch_json_schema_to_json, dispatch_function_schema_to_json, dispatch_parse_function_calls,
        dispatch_all_reduce, dispatch_all_gather, dispatch_broadcast, dispatch_reduce_scatter,
        dispatch_barrier, dispatch_pmap_psum, dispatch_pmap_pmean, dispatch_pmap_pmax,
        dispatch_pmap_all_gather, dispatch_vmap_transform, dispatch_pmap_transform,
        dispatch_dist_world_group, dispatch_dist_new_group, dispatch_dist_get_rank,
        dispatch_p2p_send, dispatch_p2p_recv, dispatch_collective_gather, dispatch_collective_scatter,
        dispatch_bucket_gradients, dispatch_get_grad, dispatch_set_grad, dispatch_module_backward,
        dispatch_mesh_select, dispatch_actor_new_id, dispatch_rdma_create_ref, dispatch_rdma_fetch,
        dispatch_rdma_write, dispatch_rdma_check_valid, dispatch_regex_find_all, dispatch_regex_replace_all,
        dispatch_regex_is_match, dispatch_regex_split, ProcessGroupHandle, ActorMeshHandle,
        RdmaRefHandle, ParameterHandle, ReduceOp,
        dispatch_cholesky, dispatch_einsum, dispatch_diag, dispatch_triu, dispatch_tril,
    };
    use super::super::super::tensor::DType;
    use super::super::super::kernel::tokenizer::{
        dispatch_tokenizer_load_bpe, dispatch_tokenizer_load_pretrained, dispatch_tokenizer_encode,
        dispatch_tokenizer_decode, dispatch_tokenizer_load_spm, dispatch_tokenizer_spm_encode,
        dispatch_tokenizer_spm_decode, TokenizerHandle,
    };

    match sub_op {
        Some(TensorSubOpcode::Pool) => {
            let op_byte = read_u8(state)?;
            let dst = read_reg(state)?;
            let input_reg = read_reg(state)?;
            let kernel_len = read_varint(state)? as usize;
            let mut kernel_size = Vec::with_capacity(kernel_len);
            for _ in 0..kernel_len {
                kernel_size.push(read_u8(state)? as usize);
            }
            let stride_len = read_varint(state)? as usize;
            let mut stride = Vec::with_capacity(stride_len);
            for _ in 0..stride_len {
                stride.push(read_u8(state)? as usize);
            }
            let padding_len = read_varint(state)? as usize;
            let mut padding = Vec::with_capacity(padding_len);
            for _ in 0..padding_len {
                padding.push(read_u8(state)? as usize);
            }

            let op = match op_byte {
                0x00 => PoolOp::Max,
                0x01 => PoolOp::Avg,
                0x02 => PoolOp::Sum,
                _ => PoolOp::Max,
            };

            let input_val = state.get_reg(input_reg);
            let input_ptr = input_val.as_ptr::<TensorHandle>();

            if !input_ptr.is_null() {
                let input_handle = unsafe { &*input_ptr };
                let kh = kernel_size.first().copied().unwrap_or(2);
                let kw = kernel_size.get(1).copied().unwrap_or(kh);
                let sh = stride.first().copied().unwrap_or(1);
                let sw = stride.get(1).copied().unwrap_or(sh);
                let ph = padding.first().copied().unwrap_or(0);
                let pw = padding.get(1).copied().unwrap_or(ph);

                if let Some(result) = tensor_pool2d(input_handle, op, (kh, kw), (sh, sw), (ph, pw)) {
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

        // ================================================================
        // Register-based tensor operations (from intrinsic calls)
        // All arguments come from registers, values extracted at runtime.
        // ================================================================

        Some(TensorSubOpcode::NewFromArgs) => {
            // tensor_new(shape, dtype) — create zero-filled tensor
            let dst = read_reg(state)?;
            let shape_reg = read_reg(state)?;
            let dtype_reg = read_reg(state)?;

            let shape = extract_shape_from_register(state, shape_reg)?;
            let dtype_val = state.get_reg(dtype_reg).as_i64() as u8;
            let dtype = DType::from_type_id(dtype_val);

            if let Some(tensor) = TensorHandle::zeros(&shape, dtype) {
                let ptr = Box::into_raw(Box::new(tensor));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::FillFromArgs) => {
            // tensor_fill(shape, value, dtype)
            let dst = read_reg(state)?;
            let shape_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let dtype_reg = read_reg(state)?;

            let shape = extract_shape_from_register(state, shape_reg)?;
            let fill_raw = state.get_reg(value_reg);
            let fill_val = if fill_raw.is_float() {
                fill_raw.as_f64()
            } else {
                fill_raw.as_i64() as f64
            };
            let dtype_val = state.get_reg(dtype_reg).as_i64() as u8;
            let dtype = DType::from_type_id(dtype_val);

            if let Some(mut tensor) = TensorHandle::zeros(&shape, dtype) {
                // Fill all elements with the value
                let data_ptr = tensor.data_ptr_f64_mut();
                if !data_ptr.is_null() {
                    for i in 0..tensor.numel {
                        unsafe { *data_ptr.add(i) = fill_val; }
                    }
                }
                let ptr = Box::into_raw(Box::new(tensor));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::FromSliceArgs) => {
            // tensor_from_slice(data, shape, dtype) — create tensor from data list
            let dst = read_reg(state)?;
            let data_reg = read_reg(state)?;
            let shape_reg = read_reg(state)?;
            let dtype_reg = read_reg(state)?;

            let shape = extract_shape_from_register(state, shape_reg)?;
            let dtype_val = state.get_reg(dtype_reg).as_i64() as u8;
            let dtype = DType::from_type_id(dtype_val);
            let data_values = extract_f64_list_from_register(state, data_reg)?;

            if let Some(mut tensor) = TensorHandle::zeros(&shape, dtype) {
                let data_ptr = tensor.data_ptr_f64_mut();
                if !data_ptr.is_null() {
                    let copy_len = data_values.len().min(tensor.numel);
                    for (i, &val) in data_values.iter().enumerate().take(copy_len) {
                        unsafe { *data_ptr.add(i) = val; }
                    }
                }
                let ptr = Box::into_raw(Box::new(tensor));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::BinopFromArgs) => {
            // tensor_binop(a, b, op)
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let op_reg = read_reg(state)?;

            let op_val = state.get_reg(op_reg).as_i64() as u8;
            let op = TensorBinaryOp::from_byte(op_val);
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

        Some(TensorSubOpcode::UnopFromArgs) => {
            // tensor_unop(tensor, op)
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let op_reg = read_reg(state)?;

            let op_val = state.get_reg(op_reg).as_i64() as u8;
            let op = TensorUnaryOp::from_byte(op_val);
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

        Some(TensorSubOpcode::MatmulFromArgs) => {
            // tensor_matmul(a, b)
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

        Some(TensorSubOpcode::ReduceFromArgs) => {
            // tensor_reduce(tensor, op, axis)
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let op_reg = read_reg(state)?;
            let axis_reg = read_reg(state)?;

            let op_val = state.get_reg(op_reg).as_i64() as u8;
            let op = TensorReduceOp::from_byte(op_val);
            let axis_val = state.get_reg(axis_reg).as_i64();
            // axis -1 means reduce all
            let axis = if axis_val < 0 { None } else { Some(axis_val as usize) };
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src = unsafe { &*src_ptr };
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

        Some(TensorSubOpcode::ReshapeFromArgs) => {
            // tensor_reshape(tensor, shape)
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let shape_reg = read_reg(state)?;

            let new_shape = extract_shape_from_register(state, shape_reg)?;
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

        Some(TensorSubOpcode::TransposeFromArgs) => {
            // tensor_transpose(tensor)
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

        Some(TensorSubOpcode::SliceFromArgs) => {
            // tensor_slice(tensor, ranges) — ranges is a list of [start, end] pairs
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let ranges_reg = read_reg(state)?;

            let ranges_vals = extract_f64_list_from_register(state, ranges_reg)?;
            // Interpret as pairs: [start0, end0, start1, end1, ...]
            let mut ranges = Vec::new();
            let mut i = 0;
            while i + 1 < ranges_vals.len() {
                ranges.push((ranges_vals[i] as usize, ranges_vals[i + 1] as usize));
                i += 2;
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

        Some(TensorSubOpcode::GetElementFromArgs) => {
            // tensor_get_element(tensor, index) — get element at flat index
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;

            let index = state.get_reg(index_reg).as_i64() as usize;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src = unsafe { &*src_ptr };
                if let Some(val) = src.get_element_f64(index) {
                    state.set_reg(dst, Value::from_f64(val));
                } else {
                    state.set_reg(dst, Value::from_f64(0.0));
                }
            } else {
                state.set_reg(dst, Value::from_f64(0.0));
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::SetElementFromArgs) => {
            // tensor_set_element(tensor, index, value) — set element at flat index, returns new tensor
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;

            let index = state.get_reg(index_reg).as_i64() as usize;
            let value_raw = state.get_reg(value_reg);
            let value = if value_raw.is_float() {
                value_raw.as_f64()
            } else {
                value_raw.as_i64() as f64
            };
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src = unsafe { &*src_ptr };
                if let Some(mut result) = super::super::super::tensor::tensor_clone(src) {
                    let data_ptr = result.data_ptr_f64_mut();
                    if !data_ptr.is_null() && index < result.numel {
                        unsafe { *data_ptr.add(index) = value; }
                    }
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

        Some(TensorSubOpcode::Argmin) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;
            let _keepdim = read_u8(state)? != 0;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let axis_opt = if axis < 0 { None } else { Some(axis as i32) };

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((idx, _val)) = tensor_argmin(src_handle, axis_opt) {
                    state.set_reg(dst, Value::from_i64(idx as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Solve) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = dispatch_solve(a_handle, b_handle) {
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

        Some(TensorSubOpcode::Gather) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;

            let src_val = state.get_reg(src_reg);
            let index_val = state.get_reg(index_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let index_ptr = index_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() && !index_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                let index_handle = unsafe { &*index_ptr };
                if let Some(result) = dispatch_gather(src_handle, index_handle, axis) {
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

        Some(TensorSubOpcode::Permute) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axes_len = read_varint(state)? as usize;
            let mut axes = Vec::with_capacity(axes_len);
            for _ in 0..axes_len {
                axes.push(read_u8(state)? as usize);
            }

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = tensor_permute(src_handle, &axes) {
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

        Some(TensorSubOpcode::QR) => {
            let q_reg = read_reg(state)?;
            let r_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _mode = read_u8(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((q, r)) = dispatch_qr(src_handle) {
                    let q_ptr = Box::into_raw(Box::new(q));
                    let r_ptr = Box::into_raw(Box::new(r));
                    state.set_reg(q_reg, Value::from_ptr(q_ptr));
                    state.set_reg(r_reg, Value::from_ptr(r_ptr));
                } else {
                    state.set_reg(q_reg, Value::nil());
                    state.set_reg(r_reg, Value::nil());
                }
            } else {
                state.set_reg(q_reg, Value::nil());
                state.set_reg(r_reg, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::SVD) => {
            let u_reg = read_reg(state)?;
            let s_reg = read_reg(state)?;
            let vh_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _full_matrices = read_u8(state)? != 0;
            let _compute_uv = read_u8(state)? != 0;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((u, s, vh)) = dispatch_svd(src_handle) {
                    let u_ptr = Box::into_raw(Box::new(u));
                    let s_ptr = Box::into_raw(Box::new(s));
                    let vh_ptr = Box::into_raw(Box::new(vh));
                    state.set_reg(u_reg, Value::from_ptr(u_ptr));
                    state.set_reg(s_reg, Value::from_ptr(s_ptr));
                    state.set_reg(vh_reg, Value::from_ptr(vh_ptr));
                } else {
                    state.set_reg(u_reg, Value::nil());
                    state.set_reg(s_reg, Value::nil());
                    state.set_reg(vh_reg, Value::nil());
                }
            } else {
                state.set_reg(u_reg, Value::nil());
                state.set_reg(s_reg, Value::nil());
                state.set_reg(vh_reg, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::LU) => {
            let p_reg = read_reg(state)?;
            let l_reg = read_reg(state)?;
            let u_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((p, l, u)) = dispatch_lu(src_handle) {
                    let p_ptr = Box::into_raw(Box::new(p));
                    let l_ptr = Box::into_raw(Box::new(l));
                    let u_ptr = Box::into_raw(Box::new(u));
                    state.set_reg(p_reg, Value::from_ptr(p_ptr));
                    state.set_reg(l_reg, Value::from_ptr(l_ptr));
                    state.set_reg(u_reg, Value::from_ptr(u_ptr));
                } else {
                    state.set_reg(p_reg, Value::nil());
                    state.set_reg(l_reg, Value::nil());
                    state.set_reg(u_reg, Value::nil());
                }
            } else {
                state.set_reg(p_reg, Value::nil());
                state.set_reg(l_reg, Value::nil());
                state.set_reg(u_reg, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Eig) => {
            let eigenvalues_reg = read_reg(state)?;
            let eigenvectors_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _compute_v = read_u8(state)? != 0;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((eigenvalues, eigenvectors)) = dispatch_eig(src_handle) {
                    let ev_ptr = Box::into_raw(Box::new(eigenvalues));
                    let evec_ptr = Box::into_raw(Box::new(eigenvectors));
                    state.set_reg(eigenvalues_reg, Value::from_ptr(ev_ptr));
                    state.set_reg(eigenvectors_reg, Value::from_ptr(evec_ptr));
                } else {
                    state.set_reg(eigenvalues_reg, Value::nil());
                    state.set_reg(eigenvectors_reg, Value::nil());
                }
            } else {
                state.set_reg(eigenvalues_reg, Value::nil());
                state.set_reg(eigenvectors_reg, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Softmax) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let axis_opt = if axis < 0 { None } else { Some(axis as i32) };

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = tensor_softmax(src_handle, axis_opt) {
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

        Some(TensorSubOpcode::LayerNorm) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let gamma_reg = read_reg(state)?;
            let beta_reg = read_reg(state)?;
            let eps = read_f64(state)?;
            let _axis = read_u8(state)?; // Read but ignore axis (function normalizes last axis)

            let src_val = state.get_reg(src_reg);
            let gamma_val = state.get_reg(gamma_reg);
            let beta_val = state.get_reg(beta_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let gamma_ptr = gamma_val.as_ptr::<TensorHandle>();
            let beta_ptr = beta_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                let gamma = if gamma_ptr.is_null() { None } else { Some(unsafe { &*gamma_ptr }) };
                let beta = if beta_ptr.is_null() { None } else { Some(unsafe { &*beta_ptr }) };
                if let Some(result) = tensor_layer_norm(src_handle, gamma, beta, eps) {
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

        Some(TensorSubOpcode::BatchNorm) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let gamma_reg = read_reg(state)?;
            let beta_reg = read_reg(state)?;
            let mean_reg = read_reg(state)?;
            let var_reg = read_reg(state)?;
            let eps = read_f64(state)?;

            let src_val = state.get_reg(src_reg);
            let gamma_val = state.get_reg(gamma_reg);
            let beta_val = state.get_reg(beta_reg);
            let mean_val = state.get_reg(mean_reg);
            let var_val = state.get_reg(var_reg);

            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let gamma_ptr = gamma_val.as_ptr::<TensorHandle>();
            let beta_ptr = beta_val.as_ptr::<TensorHandle>();
            let mean_ptr = mean_val.as_ptr::<TensorHandle>();
            let var_ptr = var_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                let gamma = if gamma_ptr.is_null() { None } else { Some(unsafe { &*gamma_ptr }) };
                let beta = if beta_ptr.is_null() { None } else { Some(unsafe { &*beta_ptr }) };
                let mean = if mean_ptr.is_null() { None } else { Some(unsafe { &*mean_ptr }) };
                let var = if var_ptr.is_null() { None } else { Some(unsafe { &*var_ptr }) };
                // Inference mode (training=false) - use running statistics
                if let Some(result) = tensor_batch_norm(src_handle, gamma, beta, mean, var, eps, false) {
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

        Some(TensorSubOpcode::Conv) => {
            let dst = read_reg(state)?;
            let input_reg = read_reg(state)?;
            let weight_reg = read_reg(state)?;
            let bias_reg = read_reg(state)?;
            let stride_len = read_varint(state)? as usize;
            let mut stride = Vec::with_capacity(stride_len);
            for _ in 0..stride_len {
                stride.push(read_u8(state)? as usize);
            }
            let padding_len = read_varint(state)? as usize;
            let mut padding = Vec::with_capacity(padding_len);
            for _ in 0..padding_len {
                padding.push(read_u8(state)? as usize);
            }

            let input_val = state.get_reg(input_reg);
            let weight_val = state.get_reg(weight_reg);
            let bias_val = state.get_reg(bias_reg);

            let input_ptr = input_val.as_ptr::<TensorHandle>();
            let weight_ptr = weight_val.as_ptr::<TensorHandle>();
            let bias_ptr = bias_val.as_ptr::<TensorHandle>();

            if !input_ptr.is_null() && !weight_ptr.is_null() {
                let input_handle = unsafe { &*input_ptr };
                let weight_handle = unsafe { &*weight_ptr };
                let bias = if bias_ptr.is_null() { None } else { Some(unsafe { &*bias_ptr }) };
                let sh = stride.first().copied().unwrap_or(1);
                let sw = stride.get(1).copied().unwrap_or(sh);
                let ph = padding.first().copied().unwrap_or(0);
                let pw = padding.get(1).copied().unwrap_or(ph);

                if let Some(result) = tensor_conv2d(
                    input_handle, weight_handle, bias, (sh, sw), (ph, pw), (1, 1), 1
                ) {
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

        Some(TensorSubOpcode::BatchMatmul) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = tensor_batch_matmul(a_handle, b_handle) {
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

        Some(TensorSubOpcode::Argmax) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;
            let _keepdim = read_u8(state)? != 0;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let axis_opt = if axis < 0 { None } else { Some(axis as i32) };

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((idx, _val)) = tensor_argmax(src_handle, axis_opt) {
                    state.set_reg(dst, Value::from_i64(idx as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Topk) => {
            let values_reg = read_reg(state)?;
            let indices_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let k = read_varint(state)? as usize;
            let axis = read_u8(state)? as i8;
            let largest = read_u8(state)? != 0;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let axis_opt = if axis < 0 { None } else { Some(axis as i32) };

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                // sorted=true by default for topk
                if let Some((values, indices)) = tensor_topk(src_handle, k, axis_opt, largest, true) {
                    let values_ptr = Box::into_raw(Box::new(values));
                    let indices_ptr = Box::into_raw(Box::new(indices));
                    state.set_reg(values_reg, Value::from_ptr(values_ptr));
                    state.set_reg(indices_reg, Value::from_ptr(indices_ptr));
                } else {
                    state.set_reg(values_reg, Value::nil());
                    state.set_reg(indices_reg, Value::nil());
                }
            } else {
                state.set_reg(values_reg, Value::nil());
                state.set_reg(indices_reg, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Reduction Variants (0x10-0x1F)
        // ====================================================================

        Some(TensorSubOpcode::Nansum) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;
            let keepdim = read_u8(state)? != 0;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let axis_opt = if axis < 0 { None } else { Some(axis) };

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_nansum(src_handle, axis_opt, keepdim) {
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

        Some(TensorSubOpcode::Nanmean) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;
            let keepdim = read_u8(state)? != 0;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let axis_opt = if axis < 0 { None } else { Some(axis) };

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_nanmean(src_handle, axis_opt, keepdim) {
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

        // ====================================================================
        // Advanced Indexing (0x20-0x2F)
        // ====================================================================

        Some(TensorSubOpcode::Flip) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axes_len = read_varint(state)? as usize;
            let mut axes = Vec::with_capacity(axes_len);
            for _ in 0..axes_len {
                axes.push(read_u8(state)? as usize);
            }

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_flip(src_handle, &axes) {
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

        Some(TensorSubOpcode::Roll) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let shift = read_signed_varint(state)? as i32;
            let axis = read_u8(state)? as i8;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_roll(src_handle, shift, axis) {
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

        // ====================================================================
        // Linear System Solvers (0x30-0x3F)
        // ====================================================================

        Some(TensorSubOpcode::Lstsq) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = dispatch_lstsq(a_handle, b_handle) {
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

        Some(TensorSubOpcode::TriSolve) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            // Flags packed as: bit 0 = upper, bit 1 = trans, bit 2 = unit_diag
            let flags = read_u8(state)?;

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = dispatch_trisolve(a_handle, b_handle, flags) {
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

        // ====================================================================
        // Matrix Decompositions (0x40-0x5F)
        // ====================================================================

        Some(TensorSubOpcode::EigSymmetric) => {
            let eigenvalues_reg = read_reg(state)?;
            let eigenvectors_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((eigenvalues, eigenvectors)) = dispatch_eig_symmetric(src_handle) {
                    let ev_ptr = Box::into_raw(Box::new(eigenvalues));
                    let evec_ptr = Box::into_raw(Box::new(eigenvectors));
                    state.set_reg(eigenvalues_reg, Value::from_ptr(ev_ptr));
                    state.set_reg(eigenvectors_reg, Value::from_ptr(evec_ptr));
                } else {
                    state.set_reg(eigenvalues_reg, Value::nil());
                    state.set_reg(eigenvectors_reg, Value::nil());
                }
            } else {
                state.set_reg(eigenvalues_reg, Value::nil());
                state.set_reg(eigenvectors_reg, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Schur) => {
            let t_reg = read_reg(state)?;
            let z_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((t, z)) = dispatch_schur(src_handle) {
                    let t_ptr = Box::into_raw(Box::new(t));
                    let z_ptr = Box::into_raw(Box::new(z));
                    state.set_reg(t_reg, Value::from_ptr(t_ptr));
                    state.set_reg(z_reg, Value::from_ptr(z_ptr));
                } else {
                    state.set_reg(t_reg, Value::nil());
                    state.set_reg(z_reg, Value::nil());
                }
            } else {
                state.set_reg(t_reg, Value::nil());
                state.set_reg(z_reg, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Matrix Properties (0x60-0x6F)
        // ====================================================================

        Some(TensorSubOpcode::Rank) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let tol = read_f64(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(rank) = dispatch_rank(src_handle, tol) {
                    state.set_reg(dst, Value::from_i64(rank as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Cond) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let p = read_u8(state)? as i8;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(cond) = dispatch_cond(src_handle, p) {
                    state.set_reg(dst, Value::from_f64(cond));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Norm) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let ord = read_f64(state)?;
            let axis = read_u8(state)? as i8;
            let axis_opt = if axis < 0 { None } else { Some(axis) };

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_norm(src_handle, ord, axis_opt) {
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

        // ====================================================================
        // Advanced Operations (0x70-0x7F)
        // ====================================================================

        Some(TensorSubOpcode::Kron) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = dispatch_kron(a_handle, b_handle) {
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

        Some(TensorSubOpcode::Cross) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let _axis = read_u8(state)? as i8; // Read but currently unused

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = dispatch_cross(a_handle, b_handle) {
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

        Some(TensorSubOpcode::Contract) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let axis_a = read_u8(state)? as usize;
            let axis_b = read_u8(state)? as usize;

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = dispatch_contract(a_handle, b_handle, axis_a, axis_b) {
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

        Some(TensorSubOpcode::MatrixPower) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let n = read_signed_varint(state)? as i32;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_matrix_power(src_handle, n) {
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

        Some(TensorSubOpcode::Expm) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_expm(src_handle) {
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

        Some(TensorSubOpcode::Logm) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_logm(src_handle) {
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

        Some(TensorSubOpcode::Inverse) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_inverse(src_handle) {
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

        Some(TensorSubOpcode::Rfft) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let n = read_varint(state)? as usize;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_rfft(src_handle, n) {
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

        Some(TensorSubOpcode::Irfft) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let n = read_varint(state)? as usize;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_irfft(src_handle, n) {
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

        Some(TensorSubOpcode::ComplexMul) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_handle = unsafe { &*a_ptr };
                let b_handle = unsafe { &*b_ptr };
                if let Some(result) = dispatch_complex_mul(a_handle, b_handle) {
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

        Some(TensorSubOpcode::ComplexPow) => {
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let exp_reg = read_reg(state)?;

            let base_val = state.get_reg(base_reg);
            let exp_val = state.get_reg(exp_reg);
            let base_ptr = base_val.as_ptr::<TensorHandle>();
            let exp_ptr = exp_val.as_ptr::<TensorHandle>();

            if !base_ptr.is_null() && !exp_ptr.is_null() {
                let base_handle = unsafe { &*base_ptr };
                let exp_handle = unsafe { &*exp_ptr };
                if let Some(result) = dispatch_complex_pow(base_handle, exp_handle) {
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

        Some(TensorSubOpcode::SsmScan) => {
            let dst = read_reg(state)?;
            let op = read_u8(state)?;
            let init_reg = read_reg(state)?;
            let elements_reg = read_reg(state)?;
            let dim = read_u8(state)? as i8;

            let init_val = state.get_reg(init_reg);
            let elements_val = state.get_reg(elements_reg);
            let init_ptr = init_val.as_ptr::<TensorHandle>();
            let elements_ptr = elements_val.as_ptr::<TensorHandle>();

            if !init_ptr.is_null() && !elements_ptr.is_null() {
                let init_handle = unsafe { &*init_ptr };
                let elements_handle = unsafe { &*elements_ptr };
                if let Some(result) = dispatch_ssm_scan(op, init_handle, elements_handle, dim) {
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

        Some(TensorSubOpcode::Uniform) => {
            let dst = read_reg(state)?;
            let shape_len = read_u8(state)? as usize;
            let mut shape = Vec::with_capacity(shape_len);
            for _ in 0..shape_len {
                shape.push(read_varint(state)? as usize);
            }
            let low_reg = read_reg(state)?;
            let high_reg = read_reg(state)?;

            let low_val = state.get_reg(low_reg);
            let high_val = state.get_reg(high_reg);
            let low = if low_val.is_float() { low_val.as_f64() } else { 0.0 };
            let high = if high_val.is_float() { high_val.as_f64() } else { 1.0 };

            if let Some(result) = dispatch_uniform(&shape, low, high, DType::F64) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Bincount) => {
            let dst = read_reg(state)?;
            let indices_reg = read_reg(state)?;
            let num_bins = read_varint(state)? as usize;

            let indices_val = state.get_reg(indices_reg);
            let indices_ptr = indices_val.as_ptr::<TensorHandle>();

            if !indices_ptr.is_null() {
                let indices_handle = unsafe { &*indices_ptr };
                if let Some(result) = dispatch_bincount(indices_handle, num_bins) {
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

        Some(TensorSubOpcode::GatherNd) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let indices_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let indices_val = state.get_reg(indices_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let indices_ptr = indices_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() && !indices_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                let indices_handle = unsafe { &*indices_ptr };
                if let Some(result) = dispatch_gather_nd(src_handle, indices_handle) {
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

        Some(TensorSubOpcode::ArangeUsize) => {
            let dst = read_reg(state)?;
            let start_reg = read_reg(state)?;
            let end_reg = read_reg(state)?;
            let step_reg = read_reg(state)?;

            let start_val = state.get_reg(start_reg);
            let end_val = state.get_reg(end_reg);
            let step_val = state.get_reg(step_reg);
            let start = if start_val.is_int() { start_val.as_i64() as usize } else { 0 };
            let end = if end_val.is_int() { end_val.as_i64() as usize } else { 0 };
            let step = if step_val.is_int() { step_val.as_i64() as usize } else { 1 };

            if let Some(result) = dispatch_arange_usize(start, end, step) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Extended Tensor Operations (0x80-0x8F)
        // ====================================================================

        Some(TensorSubOpcode::Repeat) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let times = read_varint(state)? as usize;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_repeat(src_handle, times) {
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

        Some(TensorSubOpcode::Tanh) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_tanh(src_handle) {
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

        Some(TensorSubOpcode::SumAll) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_sum_all(src_handle) {
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

        Some(TensorSubOpcode::FromArray) => {
            let dst = read_reg(state)?;
            let len = read_varint(state)? as usize;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                let val = read_f64(state)?;
                values.push(val);
            }

            if let Some(result) = dispatch_from_array(&values, DType::F64) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::IsTraining) => {
            let dst = read_reg(state)?;
            let is_training = state.context_stack.get_training_mode();
            state.set_reg(dst, Value::from_bool(is_training));
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::RandomFloat01) => {
            let dst = read_reg(state)?;
            let random = dispatch_random_float_01();
            state.set_reg(dst, Value::from_f64(random));
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Tokenizer Operations (0x90-0x9F)
        // ====================================================================

        Some(TensorSubOpcode::TokenizerLoadBpe) => {
            let dst = read_reg(state)?;
            let vocab_path_reg = read_reg(state)?;
            let merges_path_reg = read_reg(state)?;

            // Get paths from string table via registers
            let vocab_val = state.get_reg(vocab_path_reg);
            let merges_val = state.get_reg(merges_path_reg);
            let vocab_id = if vocab_val.is_int() { vocab_val.as_i64() as u32 } else { 0 };
            let merges_id = if merges_val.is_int() { merges_val.as_i64() as u32 } else { 0 };
            let vocab_path = state.module.get_string(crate::types::StringId(vocab_id)).unwrap_or("");
            let merges_path = state.module.get_string(crate::types::StringId(merges_id)).unwrap_or("");

            if let Some(tokenizer) = dispatch_tokenizer_load_bpe(vocab_path, merges_path) {
                let ptr = Box::into_raw(Box::new(tokenizer));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::TokenizerLoadPretrained) => {
            let dst = read_reg(state)?;
            let model_name_reg = read_reg(state)?;

            let model_name_val = state.get_reg(model_name_reg);
            let model_name_id = if model_name_val.is_int() { model_name_val.as_i64() as u32 } else { 0 };
            let model_name = state.module.get_string(crate::types::StringId(model_name_id)).unwrap_or("");

            if let Some(tokenizer) = dispatch_tokenizer_load_pretrained(model_name) {
                let ptr = Box::into_raw(Box::new(tokenizer));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::TokenizerEncode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;

            let tokenizer_val = state.get_reg(tokenizer_reg);
            let tokenizer_ptr = tokenizer_val.as_ptr::<TokenizerHandle>();
            let text_val = state.get_reg(text_reg);
            let text_id = if text_val.is_int() { text_val.as_i64() as u32 } else { 0 };
            let text = state.module.get_string(crate::types::StringId(text_id)).unwrap_or("");

            if !tokenizer_ptr.is_null() {
                let tokenizer = unsafe { &*tokenizer_ptr };
                if let Some(tokens) = dispatch_tokenizer_encode(tokenizer, text) {
                    // Store tokens as a tensor
                    let token_values: Vec<f64> = tokens.iter().map(|&t| t as f64).collect();
                    if let Some(result) = dispatch_from_array(&token_values, DType::I64) {
                        let ptr = Box::into_raw(Box::new(result));
                        state.set_reg(dst, Value::from_ptr(ptr));
                    } else {
                        state.set_reg(dst, Value::nil());
                    }
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::TokenizerDecode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let tokens_reg = read_reg(state)?;

            let tokenizer_val = state.get_reg(tokenizer_reg);
            let tokenizer_ptr = tokenizer_val.as_ptr::<TokenizerHandle>();
            let tokens_val = state.get_reg(tokens_reg);
            let tokens_ptr = tokens_val.as_ptr::<TensorHandle>();

            if !tokenizer_ptr.is_null() && !tokens_ptr.is_null() {
                let tokenizer = unsafe { &*tokenizer_ptr };
                let tokens_handle = unsafe { &*tokens_ptr };
                // Extract tokens from tensor - use data_ptr_f64 and numel
                let numel = tokens_handle.numel;
                let data_ptr = tokens_handle.data_ptr_f64();
                let tokens: Vec<u32> = if !data_ptr.is_null() {
                    unsafe { std::slice::from_raw_parts(data_ptr, numel) }
                        .iter()
                        .map(|&f| f as u32)
                        .collect()
                } else {
                    vec![]
                };
                if let Some(text) = dispatch_tokenizer_decode(tokenizer, &tokens) {
                    // Store string in string table and return ID
                    // For now, return nil (would need module mutation to add new string)
                    state.set_reg(dst, Value::nil());
                    let _ = text; // Suppress unused warning
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::TokenizerLoadSpm) => {
            let dst = read_reg(state)?;
            let model_path_reg = read_reg(state)?;

            let model_path_val = state.get_reg(model_path_reg);
            let model_path_id = if model_path_val.is_int() { model_path_val.as_i64() as u32 } else { 0 };
            let model_path = state.module.get_string(crate::types::StringId(model_path_id)).unwrap_or("");

            if let Some(tokenizer) = dispatch_tokenizer_load_spm(model_path) {
                let ptr = Box::into_raw(Box::new(tokenizer));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::TokenizerSpmEncode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;

            let tokenizer_val = state.get_reg(tokenizer_reg);
            let tokenizer_ptr = tokenizer_val.as_ptr::<TokenizerHandle>();
            let text_val = state.get_reg(text_reg);
            let text_id = if text_val.is_int() { text_val.as_i64() as u32 } else { 0 };
            let text = state.module.get_string(crate::types::StringId(text_id)).unwrap_or("");

            if !tokenizer_ptr.is_null() {
                let tokenizer = unsafe { &*tokenizer_ptr };
                if let Some(tokens) = dispatch_tokenizer_spm_encode(tokenizer, text) {
                    let token_values: Vec<f64> = tokens.iter().map(|&t| t as f64).collect();
                    if let Some(result) = dispatch_from_array(&token_values, DType::I64) {
                        let ptr = Box::into_raw(Box::new(result));
                        state.set_reg(dst, Value::from_ptr(ptr));
                    } else {
                        state.set_reg(dst, Value::nil());
                    }
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::TokenizerSpmDecode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let tokens_reg = read_reg(state)?;

            let tokenizer_val = state.get_reg(tokenizer_reg);
            let tokenizer_ptr = tokenizer_val.as_ptr::<TokenizerHandle>();
            let tokens_val = state.get_reg(tokens_reg);
            let tokens_ptr = tokens_val.as_ptr::<TensorHandle>();

            if !tokenizer_ptr.is_null() && !tokens_ptr.is_null() {
                let tokenizer = unsafe { &*tokenizer_ptr };
                let tokens_handle = unsafe { &*tokens_ptr };
                let numel = tokens_handle.numel;
                let data_ptr = tokens_handle.data_ptr_f64();
                let tokens: Vec<u32> = if !data_ptr.is_null() {
                    unsafe { std::slice::from_raw_parts(data_ptr, numel) }
                        .iter()
                        .map(|&f| f as u32)
                        .collect()
                } else {
                    vec![]
                };
                if let Some(text) = dispatch_tokenizer_spm_decode(tokenizer, &tokens) {
                    state.set_reg(dst, Value::nil());
                    let _ = text;
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Sampling Operations (0xA0-0xAF)
        // ====================================================================

        Some(TensorSubOpcode::SampleTopP) => {
            let dst = read_reg(state)?;
            let logits_reg = read_reg(state)?;
            let p_reg = read_reg(state)?;

            let logits_val = state.get_reg(logits_reg);
            let logits_ptr = logits_val.as_ptr::<TensorHandle>();
            let p_val = state.get_reg(p_reg);
            let p = if p_val.is_float() { p_val.as_f64() } else { 0.9 };

            if !logits_ptr.is_null() {
                let logits_handle = unsafe { &*logits_ptr };
                if let Some(token) = dispatch_sample_top_p(logits_handle, p) {
                    state.set_reg(dst, Value::from_i64(token as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::SampleTemperature) => {
            let dst = read_reg(state)?;
            let logits_reg = read_reg(state)?;
            let temperature_reg = read_reg(state)?;

            let logits_val = state.get_reg(logits_reg);
            let logits_ptr = logits_val.as_ptr::<TensorHandle>();
            let temperature_val = state.get_reg(temperature_reg);
            let temperature = if temperature_val.is_float() { temperature_val.as_f64() } else { 1.0 };

            if !logits_ptr.is_null() {
                let logits_handle = unsafe { &*logits_ptr };
                if let Some(token) = dispatch_sample_temperature(logits_handle, temperature) {
                    state.set_reg(dst, Value::from_i64(token as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::PagedAttention) => {
            let dst = read_reg(state)?;
            let q_reg = read_reg(state)?;
            let kv_cache_reg = read_reg(state)?;
            let block_table_reg = read_reg(state)?;
            let context_len_reg = read_reg(state)?;

            let q_val = state.get_reg(q_reg);
            let kv_val = state.get_reg(kv_cache_reg);
            let block_val = state.get_reg(block_table_reg);
            let q_ptr = q_val.as_ptr::<TensorHandle>();
            let kv_ptr = kv_val.as_ptr::<TensorHandle>();
            let block_ptr = block_val.as_ptr::<TensorHandle>();
            let context_len_val = state.get_reg(context_len_reg);
            let context_len = if context_len_val.is_int() { context_len_val.as_i64() as usize } else { 0 };

            if !q_ptr.is_null() && !kv_ptr.is_null() && !block_ptr.is_null() {
                let q_handle = unsafe { &*q_ptr };
                let kv_handle = unsafe { &*kv_ptr };
                let block_handle = unsafe { &*block_ptr };
                if let Some(result) = dispatch_paged_attention(q_handle, kv_handle, block_handle, context_len) {
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

        // ====================================================================
        // Inference Utility Operations (0xB0-0xBF)
        // ====================================================================

        Some(TensorSubOpcode::ParseToolCall) => {
            let dst = read_reg(state)?;
            let action_reg = read_reg(state)?;

            let action_val = state.get_reg(action_reg);
            let action_id = if action_val.is_int() { action_val.as_i64() as u32 } else { 0 };
            let action = state.module.get_string(crate::types::StringId(action_id)).unwrap_or("");

            if let Some((_tool, _args)) = dispatch_parse_tool_call(action) {
                // Return tuple would need allocation
                state.set_reg(dst, Value::nil());
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::FormatValue) => {
            let dst = read_reg(state)?;
            let value_reg = read_reg(state)?;

            let value = state.get_reg(value_reg);
            let _formatted = dispatch_format_value(&value);
            // Store string would need module mutation
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::TensorFromSliceUsize) => {
            let dst = read_reg(state)?;
            let values_reg = read_reg(state)?;

            let values_val = state.get_reg(values_reg);
            let values_ptr = values_val.as_ptr::<TensorHandle>();

            if !values_ptr.is_null() {
                let values_handle = unsafe { &*values_ptr };
                let numel = values_handle.numel;
                let data_ptr = values_handle.data_ptr_f64();
                let values: Vec<usize> = if !data_ptr.is_null() {
                    unsafe { std::slice::from_raw_parts(data_ptr, numel) }
                        .iter()
                        .map(|&f| f as usize)
                        .collect()
                } else {
                    vec![]
                };
                if let Some(result) = dispatch_tensor_from_slice_usize(&values) {
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

        Some(TensorSubOpcode::QuantizedMatmul) => {
            let dst = read_reg(state)?;
            let input_reg = read_reg(state)?;
            let weight_reg = read_reg(state)?;
            let scale_reg = read_reg(state)?;
            let zero_point_reg = read_reg(state)?;

            let input_val = state.get_reg(input_reg);
            let weight_val = state.get_reg(weight_reg);
            let scale_val = state.get_reg(scale_reg);
            let zp_val = state.get_reg(zero_point_reg);

            let input_ptr = input_val.as_ptr::<TensorHandle>();
            let weight_ptr = weight_val.as_ptr::<TensorHandle>();
            let scale_ptr = scale_val.as_ptr::<TensorHandle>();
            let zp_ptr = zp_val.as_ptr::<TensorHandle>();

            if !input_ptr.is_null() && !weight_ptr.is_null() && !scale_ptr.is_null() && !zp_ptr.is_null() {
                let input = unsafe { &*input_ptr };
                let weight = unsafe { &*weight_ptr };
                let scale = unsafe { &*scale_ptr };
                let zp = unsafe { &*zp_ptr };
                if let Some(result) = dispatch_quantized_matmul(input, weight, scale, zp) {
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

        Some(TensorSubOpcode::TensorNorm) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;

            let x_val = state.get_reg(x_reg);
            let x_ptr = x_val.as_ptr::<TensorHandle>();

            if !x_ptr.is_null() {
                let x_handle = unsafe { &*x_ptr };
                if let Some(norm) = dispatch_tensor_norm(x_handle) {
                    state.set_reg(dst, Value::from_f64(norm));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::GenerateRequestId) => {
            let dst = read_reg(state)?;
            let _request_id = dispatch_generate_request_id();
            // Store string would need module mutation
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::JsonSchemaToJson) => {
            let dst = read_reg(state)?;
            let schema_reg = read_reg(state)?;
            let schema = state.get_reg(schema_reg);
            let _json = dispatch_json_schema_to_json(&schema);
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::FunctionSchemaToJson) => {
            let dst = read_reg(state)?;
            let schema_reg = read_reg(state)?;
            let schema = state.get_reg(schema_reg);
            let _json = dispatch_function_schema_to_json(&schema);
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::ParseFunctionCalls) => {
            let dst = read_reg(state)?;
            let response_reg = read_reg(state)?;

            let response_val = state.get_reg(response_reg);
            let response_id = if response_val.is_int() { response_val.as_i64() as u32 } else { 0 };
            let response = state.module.get_string(crate::types::StringId(response_id)).unwrap_or("");

            if let Some(_calls) = dispatch_parse_function_calls(response) {
                state.set_reg(dst, Value::nil());
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Distributed/Collective Operations (0xC0-0xCF)
        // ====================================================================

        Some(TensorSubOpcode::AllReduce) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;
            let op_byte = read_u8(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let group_val = state.get_reg(group_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();

            // Convert u8 to ReduceOp
            let op = match op_byte {
                0 => ReduceOp::Sum,
                1 => ReduceOp::Mean,
                2 => ReduceOp::Max,
                3 => ReduceOp::Min,
                4 => ReduceOp::Prod,
                _ => ReduceOp::Sum,
            };

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let group_handle = unsafe { &*group_ptr };
                if let Some(result) = dispatch_all_reduce(tensor_handle, group_handle, op) {
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

        Some(TensorSubOpcode::AllGather) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let group_val = state.get_reg(group_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let group_handle = unsafe { &*group_ptr };
                if let Some(result) = dispatch_all_gather(tensor_handle, group_handle) {
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

        Some(TensorSubOpcode::Broadcast) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let src_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let group_val = state.get_reg(group_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();
            let src_rank_val = state.get_reg(src_rank_reg);
            let src_rank = if src_rank_val.is_int() { src_rank_val.as_i64() as usize } else { 0 };

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let group_handle = unsafe { &*group_ptr };
                if let Some(result) = dispatch_broadcast(tensor_handle, src_rank, group_handle) {
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

        Some(TensorSubOpcode::ReduceScatter) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;
            let op_byte = read_u8(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let group_val = state.get_reg(group_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();

            // Convert u8 to ReduceOp
            let op = match op_byte {
                0 => ReduceOp::Sum,
                1 => ReduceOp::Mean,
                2 => ReduceOp::Max,
                3 => ReduceOp::Min,
                4 => ReduceOp::Prod,
                _ => ReduceOp::Sum,
            };

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let group_handle = unsafe { &*group_ptr };
                if let Some(result) = dispatch_reduce_scatter(tensor_handle, group_handle, op) {
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

        Some(TensorSubOpcode::Barrier) => {
            let group_reg = read_reg(state)?;
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();

            if !group_ptr.is_null() {
                let group_handle = unsafe { &*group_ptr };
                dispatch_barrier(group_handle);
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::PmapPsum) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let axis_val = state.get_reg(axis_name_reg);
            let axis_id = if axis_val.is_int() { axis_val.as_i64() as u32 } else { 0 };
            let axis_name = state.module.get_string(crate::types::StringId(axis_id)).unwrap_or("");

            if !tensor_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                if let Some(result) = dispatch_pmap_psum(tensor_handle, axis_name) {
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

        Some(TensorSubOpcode::PmapPmean) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let axis_val = state.get_reg(axis_name_reg);
            let axis_id = if axis_val.is_int() { axis_val.as_i64() as u32 } else { 0 };
            let axis_name = state.module.get_string(crate::types::StringId(axis_id)).unwrap_or("");

            if !tensor_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                if let Some(result) = dispatch_pmap_pmean(tensor_handle, axis_name) {
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

        Some(TensorSubOpcode::PmapPmax) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let axis_val = state.get_reg(axis_name_reg);
            let axis_id = if axis_val.is_int() { axis_val.as_i64() as u32 } else { 0 };
            let axis_name = state.module.get_string(crate::types::StringId(axis_id)).unwrap_or("");

            if !tensor_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                if let Some(result) = dispatch_pmap_pmax(tensor_handle, axis_name) {
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

        Some(TensorSubOpcode::PmapAllGather) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let axis_val = state.get_reg(axis_name_reg);
            let axis_id = if axis_val.is_int() { axis_val.as_i64() as u32 } else { 0 };
            let axis_name = state.module.get_string(crate::types::StringId(axis_id)).unwrap_or("");

            if !tensor_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                if let Some(result) = dispatch_pmap_all_gather(tensor_handle, axis_name) {
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

        Some(TensorSubOpcode::VmapTransform) => {
            let dst = read_reg(state)?;
            let _func_reg = read_reg(state)?;
            let _in_axes_reg = read_reg(state)?;
            let _out_axes_reg = read_reg(state)?;

            let _ = dispatch_vmap_transform();
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::PmapTransform) => {
            let dst = read_reg(state)?;
            let _func_reg = read_reg(state)?;
            let _axis_name_reg = read_reg(state)?;
            let _in_axes_reg = read_reg(state)?;
            let _out_axes_reg = read_reg(state)?;

            let _ = dispatch_pmap_transform();
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Process Group Operations (0xCB-0xCD)
        // ====================================================================

        Some(TensorSubOpcode::DistWorldGroup) => {
            let dst = read_reg(state)?;
            let group = dispatch_dist_world_group();
            let ptr = Box::into_raw(Box::new(group));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::DistNewGroup) => {
            let dst = read_reg(state)?;
            let ranks_reg = read_reg(state)?;

            let ranks_val = state.get_reg(ranks_reg);
            let ranks_ptr = ranks_val.as_ptr::<TensorHandle>();

            if !ranks_ptr.is_null() {
                let ranks_handle = unsafe { &*ranks_ptr };
                let numel = ranks_handle.numel;
                let data_ptr = ranks_handle.data_ptr_f64();
                let ranks: Vec<usize> = if !data_ptr.is_null() {
                    unsafe { std::slice::from_raw_parts(data_ptr, numel) }
                        .iter()
                        .map(|&f| f as usize)
                        .collect()
                } else {
                    vec![]
                };
                let group = dispatch_dist_new_group(&ranks);
                let ptr = Box::into_raw(Box::new(group));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::DistGetRank) => {
            let dst = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();

            if !group_ptr.is_null() {
                let group_handle = unsafe { &*group_ptr };
                let rank = dispatch_dist_get_rank(group_handle);
                state.set_reg(dst, Value::from_i64(rank as i64));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Point-to-Point Operations (0xCE-0xCF)
        // ====================================================================

        Some(TensorSubOpcode::P2PSend) => {
            let tensor_reg = read_reg(state)?;
            let dst_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let group_val = state.get_reg(group_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();
            let dst_rank_val = state.get_reg(dst_rank_reg);
            let dst_rank = if dst_rank_val.is_int() { dst_rank_val.as_i64() as usize } else { 0 };

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let group_handle = unsafe { &*group_ptr };
                dispatch_p2p_send(tensor_handle, dst_rank, group_handle);
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::P2PRecv) => {
            let dst = read_reg(state)?;
            let src_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();
            let src_rank_val = state.get_reg(src_rank_reg);
            let src_rank = if src_rank_val.is_int() { src_rank_val.as_i64() as usize } else { 0 };

            if !group_ptr.is_null() {
                let group_handle = unsafe { &*group_ptr };
                if let Some(result) = dispatch_p2p_recv(src_rank, group_handle) {
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

        // ====================================================================
        // Additional Collective Operations (0xD0-0xD1)
        // ====================================================================

        Some(TensorSubOpcode::CollectiveGather) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let dst_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let group_val = state.get_reg(group_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();
            let dst_rank_val = state.get_reg(dst_rank_reg);
            let dst_rank = if dst_rank_val.is_int() { dst_rank_val.as_i64() as usize } else { 0 };

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let group_handle = unsafe { &*group_ptr };
                if let Some(result) = dispatch_collective_gather(tensor_handle, dst_rank, group_handle) {
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

        Some(TensorSubOpcode::CollectiveScatter) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let src_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let group_val = state.get_reg(group_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();
            let group_ptr = group_val.as_ptr::<ProcessGroupHandle>();
            let src_rank_val = state.get_reg(src_rank_reg);
            let src_rank = if src_rank_val.is_int() { src_rank_val.as_i64() as usize } else { 0 };

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let group_handle = unsafe { &*group_ptr };
                if let Some(result) = dispatch_collective_scatter(tensor_handle, src_rank, group_handle) {
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

        // ====================================================================
        // Gradient Operations (0xD2-0xD5)
        // ====================================================================

        Some(TensorSubOpcode::BucketGradients) => {
            let dst = read_reg(state)?;
            let gradients_reg = read_reg(state)?;
            let bucket_size_reg = read_reg(state)?;

            let gradients_val = state.get_reg(gradients_reg);
            let gradients_ptr = gradients_val.as_ptr::<TensorHandle>();
            let bucket_size_val = state.get_reg(bucket_size_reg);
            let bucket_size = if bucket_size_val.is_int() { bucket_size_val.as_i64() as usize } else { 25_000_000 };

            if !gradients_ptr.is_null() {
                // BucketGradients takes a list of tensors; for single tensor, wrap it
                let gradients_handle = unsafe { &*gradients_ptr };
                let gradients_list = vec![gradients_handle.clone()];
                let buckets = dispatch_bucket_gradients(&gradients_list, bucket_size);
                // Return first bucket for simplicity
                if let Some(first) = buckets.into_iter().next() {
                    let ptr = Box::into_raw(Box::new(first));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::GetGrad) => {
            let dst = read_reg(state)?;
            let param_reg = read_reg(state)?;

            let param_val = state.get_reg(param_reg);
            let param_ptr = param_val.as_ptr::<ParameterHandle>();

            if !param_ptr.is_null() {
                let param_handle = unsafe { &*param_ptr };
                if let Some(grad) = dispatch_get_grad(param_handle) {
                    let ptr = Box::into_raw(Box::new(grad));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::SetGrad) => {
            let param_reg = read_reg(state)?;
            let grad_reg = read_reg(state)?;

            let param_val = state.get_reg(param_reg);
            let grad_val = state.get_reg(grad_reg);
            let param_ptr = param_val.as_ptr::<ParameterHandle>();
            let grad_ptr = grad_val.as_ptr::<TensorHandle>();

            if !param_ptr.is_null() && !grad_ptr.is_null() {
                let param_handle = unsafe { &mut *param_ptr };
                let grad_handle = unsafe { (*grad_ptr).clone() };
                dispatch_set_grad(param_handle, grad_handle);
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::ModuleBackward) => {
            let dst = read_reg(state)?;
            let _module_reg = read_reg(state)?;
            let grad_output_reg = read_reg(state)?;

            let grad_output_val = state.get_reg(grad_output_reg);
            let grad_output_ptr = grad_output_val.as_ptr::<TensorHandle>();

            if !grad_output_ptr.is_null() {
                let grad_output_handle = unsafe { &*grad_output_ptr };
                // Module is just a unit type placeholder for now
                if let Some(result) = dispatch_module_backward(&(), grad_output_handle) {
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

        // ====================================================================
        // Actor Mesh Operations (0xD6-0xD7)
        // ====================================================================

        Some(TensorSubOpcode::MeshSelect) => {
            let dst = read_reg(state)?;
            let mesh_reg = read_reg(state)?;
            let coords_reg = read_reg(state)?;

            let mesh_val = state.get_reg(mesh_reg);
            let coords_val = state.get_reg(coords_reg);
            let mesh_ptr = mesh_val.as_ptr::<ActorMeshHandle>();
            let coords_ptr = coords_val.as_ptr::<TensorHandle>();

            if !mesh_ptr.is_null() && !coords_ptr.is_null() {
                let mesh_handle = unsafe { &*mesh_ptr };
                let coords_handle = unsafe { &*coords_ptr };
                let numel = coords_handle.numel;
                let data_ptr = coords_handle.data_ptr_f64();
                let coords: Vec<usize> = if !data_ptr.is_null() {
                    unsafe { std::slice::from_raw_parts(data_ptr, numel) }
                        .iter()
                        .map(|&f| f as usize)
                        .collect()
                } else {
                    vec![]
                };
                let result = dispatch_mesh_select(mesh_handle, &coords);
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::ActorNewId) => {
            let dst = read_reg(state)?;
            let actor_id = dispatch_actor_new_id();
            let ptr = Box::into_raw(Box::new(actor_id));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // RDMA Operations (0xD8-0xDB)
        // ====================================================================

        Some(TensorSubOpcode::RdmaCreateRef) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();

            if !tensor_ptr.is_null() {
                let tensor_handle = unsafe { &*tensor_ptr };
                let rdma_ref = dispatch_rdma_create_ref(tensor_handle);
                let ptr = Box::into_raw(Box::new(rdma_ref));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::RdmaFetch) => {
            let dst = read_reg(state)?;
            let rdma_ref_reg = read_reg(state)?;

            let rdma_ref_val = state.get_reg(rdma_ref_reg);
            let rdma_ref_ptr = rdma_ref_val.as_ptr::<RdmaRefHandle>();

            if !rdma_ref_ptr.is_null() {
                let rdma_ref_handle = unsafe { &*rdma_ref_ptr };
                if let Some(result) = dispatch_rdma_fetch(rdma_ref_handle) {
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

        Some(TensorSubOpcode::RdmaWrite) => {
            let rdma_ref_reg = read_reg(state)?;
            let tensor_reg = read_reg(state)?;

            let rdma_ref_val = state.get_reg(rdma_ref_reg);
            let tensor_val = state.get_reg(tensor_reg);
            let rdma_ref_ptr = rdma_ref_val.as_ptr::<RdmaRefHandle>();
            let tensor_ptr = tensor_val.as_ptr::<TensorHandle>();

            if !rdma_ref_ptr.is_null() && !tensor_ptr.is_null() {
                let rdma_ref_handle = unsafe { &mut *rdma_ref_ptr };
                let tensor_handle = unsafe { &*tensor_ptr };
                dispatch_rdma_write(rdma_ref_handle, tensor_handle);
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::RdmaCheckValid) => {
            let dst = read_reg(state)?;
            let rdma_ref_reg = read_reg(state)?;

            let rdma_ref_val = state.get_reg(rdma_ref_reg);
            let rdma_ref_ptr = rdma_ref_val.as_ptr::<RdmaRefHandle>();

            if !rdma_ref_ptr.is_null() {
                let rdma_ref_handle = unsafe { &*rdma_ref_ptr };
                let valid = dispatch_rdma_check_valid(rdma_ref_handle);
                state.set_reg(dst, Value::from_bool(valid));
            } else {
                state.set_reg(dst, Value::from_bool(false));
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Regex Operations (0xE0-0xE3)
        // ====================================================================

        Some(TensorSubOpcode::RegexFindAll) => {
            let dst = read_reg(state)?;
            let pattern_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;

            let pattern_val = state.get_reg(pattern_reg);
            let text_val = state.get_reg(text_reg);
            let pattern_id = if pattern_val.is_int() { pattern_val.as_i64() as u32 } else { 0 };
            let text_id = if text_val.is_int() { text_val.as_i64() as u32 } else { 0 };
            let pattern = state.module.get_string(crate::types::StringId(pattern_id)).unwrap_or("");
            let text = state.module.get_string(crate::types::StringId(text_id)).unwrap_or("");

            if let Some(_matches) = dispatch_regex_find_all(pattern, text) {
                // Returning list of strings would need allocation
                state.set_reg(dst, Value::nil());
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::RegexReplaceAll) => {
            let dst = read_reg(state)?;
            let pattern_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;
            let replacement_reg = read_reg(state)?;

            let pattern_val = state.get_reg(pattern_reg);
            let text_val = state.get_reg(text_reg);
            let replacement_val = state.get_reg(replacement_reg);
            let pattern_id = if pattern_val.is_int() { pattern_val.as_i64() as u32 } else { 0 };
            let text_id = if text_val.is_int() { text_val.as_i64() as u32 } else { 0 };
            let replacement_id = if replacement_val.is_int() { replacement_val.as_i64() as u32 } else { 0 };
            let pattern = state.module.get_string(crate::types::StringId(pattern_id)).unwrap_or("");
            let text = state.module.get_string(crate::types::StringId(text_id)).unwrap_or("");
            let replacement = state.module.get_string(crate::types::StringId(replacement_id)).unwrap_or("");

            if let Some(_result) = dispatch_regex_replace_all(pattern, text, replacement) {
                state.set_reg(dst, Value::nil());
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::RegexIsMatch) => {
            let dst = read_reg(state)?;
            let pattern_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;

            let pattern_val = state.get_reg(pattern_reg);
            let text_val = state.get_reg(text_reg);
            let pattern_id = if pattern_val.is_int() { pattern_val.as_i64() as u32 } else { 0 };
            let text_id = if text_val.is_int() { text_val.as_i64() as u32 } else { 0 };
            let pattern = state.module.get_string(crate::types::StringId(pattern_id)).unwrap_or("");
            let text = state.module.get_string(crate::types::StringId(text_id)).unwrap_or("");

            let is_match = dispatch_regex_is_match(pattern, text);
            state.set_reg(dst, Value::from_bool(is_match));
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::RegexSplit) => {
            let dst = read_reg(state)?;
            let pattern_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;

            let pattern_val = state.get_reg(pattern_reg);
            let text_val = state.get_reg(text_reg);
            let pattern_id = if pattern_val.is_int() { pattern_val.as_i64() as u32 } else { 0 };
            let text_id = if text_val.is_int() { text_val.as_i64() as u32 } else { 0 };
            let pattern = state.module.get_string(crate::types::StringId(pattern_id)).unwrap_or("");
            let text = state.module.get_string(crate::types::StringId(text_id)).unwrap_or("");

            if let Some(_parts) = dispatch_regex_split(pattern, text) {
                state.set_reg(dst, Value::nil());
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Shape Manipulation Operations
        // ====================================================================

        Some(TensorSubOpcode::Squeeze) => {
            use super::super::super::tensor::tensor_squeeze;

            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src = unsafe { &*src_ptr };
                // axis of -1 means squeeze all, otherwise squeeze specific dim
                let dim = if axis < 0 { None } else { Some(axis as usize) };
                if let Some(result) = tensor_squeeze(src, dim) {
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

        Some(TensorSubOpcode::Unsqueeze) => {
            use super::super::super::tensor::tensor_unsqueeze;

            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let dim = read_u8(state)? as i8;

            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src = unsafe { &*src_ptr };
                if let Some(result) = tensor_unsqueeze(src, dim as i32) {
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

        Some(TensorSubOpcode::Cmp) => {
            use super::super::super::tensor::{tensor_cmp, CompareOp};

            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let op_byte = read_u8(state)?;

            let op = match op_byte {
                0x00 => CompareOp::Eq,
                0x01 => CompareOp::Ne,
                0x02 => CompareOp::Lt,
                0x03 => CompareOp::Le,
                0x04 => CompareOp::Gt,
                0x05 => CompareOp::Ge,
                _ => CompareOp::Eq, // Default to Eq for unknown
            };

            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();

            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a = unsafe { &*a_ptr };
                let b = unsafe { &*b_ptr };
                if let Some(result) = tensor_cmp(a, b, op) {
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

        Some(TensorSubOpcode::Where) => {
            use super::super::super::tensor::tensor_where;

            let dst = read_reg(state)?;
            let cond_reg = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;

            let cond_val = state.get_reg(cond_reg);
            let x_val = state.get_reg(x_reg);
            let y_val = state.get_reg(y_reg);

            let cond_ptr = cond_val.as_ptr::<TensorHandle>();
            let x_ptr = x_val.as_ptr::<TensorHandle>();
            let y_ptr = y_val.as_ptr::<TensorHandle>();

            if !cond_ptr.is_null() && !x_ptr.is_null() && !y_ptr.is_null() {
                let cond = unsafe { &*cond_ptr };
                let x = unsafe { &*x_ptr };
                let y = unsafe { &*y_ptr };
                if let Some(result) = tensor_where(cond, x, y) {
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

        Some(TensorSubOpcode::Clamp) => {
            use super::super::super::tensor::tensor_clamp;

            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let min_reg = read_reg(state)?;
            let max_reg = read_reg(state)?;

            let src_val = state.get_reg(src_reg);
            let min_val = state.get_reg(min_reg);
            let max_val = state.get_reg(max_reg);

            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let min_f = if min_val.is_float() { min_val.as_f64() } else { min_val.as_i64() as f64 };
            let max_f = if max_val.is_float() { max_val.as_f64() } else { max_val.as_i64() as f64 };

            if !src_ptr.is_null() {
                let src = unsafe { &*src_ptr };
                if let Some(result) = tensor_clamp(src, min_f, max_f) {
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

        Some(TensorSubOpcode::Cast) => {
            use super::super::super::tensor::tensor_cast;

            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let dtype_byte = read_u8(state)?;

            let dtype = DType::from_type_id(dtype_byte);
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();

            if !src_ptr.is_null() {
                let src = unsafe { &*src_ptr };
                if let Some(result) = tensor_cast(src, dtype) {
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

        // ====================================================================
        // Newly wired TensorSubOpcodes (31 ops)
        // ====================================================================

        Some(TensorSubOpcode::Clone) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_clone(src_handle) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Arange) => {
            let dst = read_reg(state)?;
            let start = read_f64(state)?;
            let end = read_f64(state)?;
            let step = read_f64(state)?;
            if let Some(result) = super::super::super::tensor::tensor_arange(start, end, step, DType::F64) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Linspace) => {
            let dst = read_reg(state)?;
            let start = read_f64(state)?;
            let end = read_f64(state)?;
            let steps = read_varint(state)? as usize;
            if let Some(result) = super::super::super::tensor::tensor_linspace(start, end, steps, DType::F64) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Identity) => {
            let dst = read_reg(state)?;
            let n = read_varint(state)? as usize;
            if let Some(result) = super::super::super::tensor::tensor_identity(n, DType::F64) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Rand) => {
            let dst = read_reg(state)?;
            let ndim = read_varint(state)? as usize;
            let mut shape = Vec::with_capacity(ndim);
            for _ in 0..ndim { shape.push(read_varint(state)? as usize); }
            if let Some(result) = super::super::super::tensor::tensor_rand(&shape, DType::F64) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Dot) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();
            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_h = unsafe { &*a_ptr };
                let b_h = unsafe { &*b_ptr };
                if let Some(result) = super::super::super::tensor::tensor_dot(a_h, b_h) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Outer) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();
            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_h = unsafe { &*a_ptr };
                let b_h = unsafe { &*b_ptr };
                if let Some(result) = super::super::super::tensor::tensor_outer(a_h, b_h) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Lerp) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let t = read_f64(state)?;
            let a_val = state.get_reg(a_reg);
            let b_val = state.get_reg(b_reg);
            let a_ptr = a_val.as_ptr::<TensorHandle>();
            let b_ptr = b_val.as_ptr::<TensorHandle>();
            if !a_ptr.is_null() && !b_ptr.is_null() {
                let a_h = unsafe { &*a_ptr };
                let b_h = unsafe { &*b_ptr };
                if let Some(result) = super::super::super::tensor::tensor_lerp(a_h, b_h, t) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Det) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_det(src_handle) {
                    state.set_reg(dst, Value::from_f64(result));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Trace) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_trace(src_handle) {
                    state.set_reg(dst, Value::from_f64(result));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::BroadcastToShape) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let ndim = read_varint(state)? as usize;
            let mut shape = Vec::with_capacity(ndim);
            for _ in 0..ndim { shape.push(read_varint(state)? as usize); }
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_broadcast(src_handle, &shape) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Contiguous) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_clone(src_handle) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::ToDevice) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let _device = read_varint(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_clone(src_handle) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Concat) => {
            let dst = read_reg(state)?;
            let count = read_varint(state)? as usize;
            let mut tensor_regs = Vec::with_capacity(count);
            for _ in 0..count { tensor_regs.push(read_reg(state)?); }
            let axis = read_varint(state)? as usize;
            let mut handles: Vec<*const TensorHandle> = Vec::with_capacity(count);
            for &reg in &tensor_regs {
                let val = state.get_reg(reg);
                let ptr = val.as_ptr::<TensorHandle>();
                if ptr.is_null() { state.set_reg(dst, Value::nil()); return Ok(DispatchResult::Continue); }
                handles.push(ptr);
            }
            let refs: Vec<&TensorHandle> = handles.iter().map(|&p| unsafe { &*p }).collect();
            if let Some(result) = super::super::super::tensor::tensor_concat(&refs, axis) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Stack) => {
            let dst = read_reg(state)?;
            let count = read_varint(state)? as usize;
            let mut tensor_regs = Vec::with_capacity(count);
            for _ in 0..count { tensor_regs.push(read_reg(state)?); }
            let axis = read_varint(state)? as usize;
            let mut handles: Vec<*const TensorHandle> = Vec::with_capacity(count);
            for &reg in &tensor_regs {
                let val = state.get_reg(reg);
                let ptr = val.as_ptr::<TensorHandle>();
                if ptr.is_null() { state.set_reg(dst, Value::nil()); return Ok(DispatchResult::Continue); }
                handles.push(ptr);
            }
            let refs: Vec<&TensorHandle> = handles.iter().map(|&p| unsafe { &*p }).collect();
            if let Some(result) = super::super::super::tensor::tensor_stack(&refs, axis) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Cumulative) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let axis = read_u8(state)? as i8;
            let op_byte = read_u8(state)?;
            let op = match op_byte {
                0 => super::super::super::tensor::CumulativeOp::Sum,
                1 => super::super::super::tensor::CumulativeOp::Prod,
                2 => super::super::super::tensor::CumulativeOp::Max,
                _ => super::super::super::tensor::CumulativeOp::Min,
            };
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_cumulative(src_handle, op, Some(axis as i32)) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Split) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let count = read_varint(state)? as usize;
            let mut sizes = Vec::with_capacity(count);
            for _ in 0..count { sizes.push(read_varint(state)? as usize); }
            let axis = read_varint(state)? as usize;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(results) = super::super::super::tensor::tensor_split(src_handle, &sizes, axis) {
                    let mut values = Vec::with_capacity(results.len());
                    for r in results {
                        let ptr = Box::into_raw(Box::new(r));
                        values.push(Value::from_ptr(ptr));
                    }
                    let list = alloc_list_from_values(state, values)?;
                    state.set_reg(dst, list);
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::SplitAt) => {
            let dst1 = read_reg(state)?;
            let dst2 = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let pos = read_varint(state)? as usize;
            let axis = read_varint(state)? as usize;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some((a, b)) = super::super::super::tensor::tensor_split_at(src_handle, pos, axis) {
                    state.set_reg(dst1, Value::from_ptr(Box::into_raw(Box::new(a))));
                    state.set_reg(dst2, Value::from_ptr(Box::into_raw(Box::new(b))));
                } else { state.set_reg(dst1, Value::nil()); state.set_reg(dst2, Value::nil()); }
            } else { state.set_reg(dst1, Value::nil()); state.set_reg(dst2, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Diag) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let offset = read_u8(state)? as i8 as i32;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_diag(src_handle, offset) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Triu) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let offset = read_u8(state)? as i8 as i32;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_triu(src_handle, offset) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Tril) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let offset = read_u8(state)? as i8 as i32;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_tril(src_handle, offset) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Cholesky) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = dispatch_cholesky(src_handle, false) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Einsum) => {
            let dst = read_reg(state)?;
            let subscripts_id = read_varint(state)? as u32;
            let count = read_varint(state)? as usize;
            let mut tensor_regs = Vec::with_capacity(count);
            for _ in 0..count { tensor_regs.push(read_reg(state)?); }
            let equation = state.module.get_string(crate::types::StringId(subscripts_id))
                .unwrap_or("").to_string();
            let mut handles: Vec<*const TensorHandle> = Vec::with_capacity(count);
            for &reg in &tensor_regs {
                let val = state.get_reg(reg);
                let ptr = val.as_ptr::<TensorHandle>();
                if ptr.is_null() { state.set_reg(dst, Value::nil()); return Ok(DispatchResult::Continue); }
                handles.push(ptr);
            }
            let refs: Vec<&TensorHandle> = handles.iter().map(|&p| unsafe { &*p }).collect();
            if let Some(result) = dispatch_einsum(&equation, &refs) {
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::MaskedFill) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let mask_reg = read_reg(state)?;
            let value = read_f64(state)?;
            let src_val = state.get_reg(src_reg);
            let mask_val = state.get_reg(mask_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let mask_ptr = mask_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() && !mask_ptr.is_null() {
                let s = unsafe { &*src_ptr };
                let m = unsafe { &*mask_ptr };
                if let Some(result) = super::super::super::tensor::tensor_masked_fill(s, m, value) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::MaskedSelect) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let mask_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let mask_val = state.get_reg(mask_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let mask_ptr = mask_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() && !mask_ptr.is_null() {
                let s = unsafe { &*src_ptr };
                let m = unsafe { &*mask_ptr };
                if let Some(result) = super::super::super::tensor::tensor_masked_select(s, m) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Nonzero) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_nonzero(src_handle) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::OneHot) => {
            let dst = read_reg(state)?;
            let indices_reg = read_reg(state)?;
            let num_classes = read_varint(state)? as usize;
            let idx_val = state.get_reg(indices_reg);
            let idx_ptr = idx_val.as_ptr::<TensorHandle>();
            if !idx_ptr.is_null() {
                let idx_handle = unsafe { &*idx_ptr };
                if let Some(result) = super::super::super::tensor::tensor_one_hot(idx_handle, num_classes) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::GetScalar) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(scalar) = src_handle.get_scalar_f64() {
                    state.set_reg(dst, Value::from_f64(scalar));
                } else { state.set_reg(dst, Value::from_f64(0.0)); }
            } else { state.set_reg(dst, Value::from_f64(0.0)); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::SetScalar) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let value = read_f64(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(mut result) = super::super::super::tensor::tensor_clone(src_handle) {
                    if result.numel > 0 {
                        let data_ptr = result.data_ptr_f64_mut();
                        if !data_ptr.is_null() {
                            unsafe { *data_ptr = value; }
                        }
                    }
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::Index) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let index_reg = read_reg(state)?;
            let axis = read_varint(state)? as usize;
            let src_val = state.get_reg(src_reg);
            let idx_val = state.get_reg(index_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            let idx_ptr = idx_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() && !idx_ptr.is_null() {
                let s = unsafe { &*src_ptr };
                let i = unsafe { &*idx_ptr };
                if let Some(result) = super::super::super::tensor::tensor_index_select(s, i, axis) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        Some(TensorSubOpcode::LeakyRelu) => {
            let dst = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let alpha = read_f64(state)?;
            let src_val = state.get_reg(src_reg);
            let src_ptr = src_val.as_ptr::<TensorHandle>();
            if !src_ptr.is_null() {
                let src_handle = unsafe { &*src_ptr };
                if let Some(result) = super::super::super::tensor::tensor_leaky_relu(src_handle, alpha) {
                    let ptr = Box::into_raw(Box::new(result));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else { state.set_reg(dst, Value::nil()); }
            } else { state.set_reg(dst, Value::nil()); }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // TensorExtSubOpcode fallback — check extended tensor ops
        // ================================================================
        _ => {
            let ext_op = TensorExtSubOpcode::from_byte(sub_op_byte);
            match ext_op {
                Some(TensorExtSubOpcode::RmsNorm) => {
                    // RMS normalization: dst, input, gamma (optional), eps (f32)
                    let dst = read_reg(state)?;
                    let input_reg = read_reg(state)?;
                    let gamma_flag = read_u8(state)?;
                    let gamma_reg = if gamma_flag != 0 { Some(read_reg(state)?) } else { None };
                    let eps_bits = read_u32(state)?;
                    let eps = f32::from_bits(eps_bits) as f64;

                    let input_val = state.get_reg(input_reg);
                    let input_ptr = input_val.as_ptr::<TensorHandle>();

                    if !input_ptr.is_null() {
                        let input_handle = unsafe { &*input_ptr };
                        // Resolve optional gamma tensor
                        let gamma_handle: Option<&TensorHandle> = gamma_reg.and_then(|g_reg| {
                            let g_val = state.get_reg(g_reg);
                            let g_ptr = g_val.as_ptr::<TensorHandle>();
                            if !g_ptr.is_null() { Some(unsafe { &*g_ptr }) } else { None }
                        });
                        // Delegate to tensor_rms_norm helper
                        if let Some(result) = super::super::super::tensor::tensor_rms_norm(input_handle, gamma_handle, eps) {
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

                Some(TensorExtSubOpcode::FlashAttention) => {
                    // Flash attention: dst, q, k, v, mask (optional), scale, causal
                    // Scaled dot-product attention: softmax(Q*K^T * scale) * V
                    let dst = read_reg(state)?;
                    let q_reg = read_reg(state)?;
                    let k_reg = read_reg(state)?;
                    let v_reg = read_reg(state)?;
                    let mask_flag = read_u8(state)?;
                    let _mask_reg = if mask_flag != 0 { Some(read_reg(state)?) } else { None };
                    let scale_reg = read_reg(state)?;
                    let causal = read_u8(state)? != 0;

                    let q_val = state.get_reg(q_reg);
                    let k_val = state.get_reg(k_reg);
                    let v_val = state.get_reg(v_reg);
                    let scale_val = state.get_reg(scale_reg).as_f64();

                    let q_ptr = q_val.as_ptr::<TensorHandle>();
                    let k_ptr = k_val.as_ptr::<TensorHandle>();
                    let v_ptr = v_val.as_ptr::<TensorHandle>();

                    if !q_ptr.is_null() && !k_ptr.is_null() && !v_ptr.is_null() {
                        let q = unsafe { &*q_ptr };
                        let k = unsafe { &*k_ptr };
                        let v = unsafe { &*v_ptr };

                        let seq_len = q.shape[0];
                        let d_k = if q.ndim > 1 { q.shape[1] } else { 1 };
                        let v_dim = if v.ndim > 1 { v.shape[1] } else { 1 };

                        // Work with f64 for computation (cast from source dtype)
                        let q_data = q.data_ptr_f64();
                        let k_data = k.data_ptr_f64();
                        let v_data = v.data_ptr_f64();

                        // Compute Q * K^T (seq_len x seq_len attention scores)
                        let mut scores = vec![0.0f64; seq_len * seq_len];
                        unsafe {
                            for i in 0..seq_len {
                                for j in 0..seq_len {
                                    if causal && j > i {
                                        scores[i * seq_len + j] = f64::NEG_INFINITY;
                                        continue;
                                    }
                                    let mut dot = 0.0;
                                    for dk in 0..d_k {
                                        let qi = if i * d_k + dk < q.numel { *q_data.add(i * d_k + dk) } else { 0.0 };
                                        let kj = if j * d_k + dk < k.numel { *k_data.add(j * d_k + dk) } else { 0.0 };
                                        dot += qi * kj;
                                    }
                                    scores[i * seq_len + j] = dot * scale_val;
                                }
                            }

                            // Apply mask if provided
                            if let Some(m_reg) = _mask_reg {
                                let m_val = state.get_reg(m_reg);
                                let m_ptr = m_val.as_ptr::<TensorHandle>();
                                if !m_ptr.is_null() {
                                    let mask = &*m_ptr;
                                    let mask_data = mask.data_ptr_f64();
                                    for (i, s) in scores.iter_mut().enumerate() {
                                        if i < mask.numel && *mask_data.add(i) == 0.0 {
                                            *s = f64::NEG_INFINITY;
                                        }
                                    }
                                }
                            }
                        }

                        // Softmax per row
                        for i in 0..seq_len {
                            let row_start = i * seq_len;
                            let row_end = row_start + seq_len;
                            let row = &mut scores[row_start..row_end];
                            let max_val = row.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                            let sum: f64 = row.iter().map(|&x| (x - max_val).exp()).sum();
                            if sum > 0.0 {
                                for x in row.iter_mut() {
                                    *x = (*x - max_val).exp() / sum;
                                }
                            }
                        }

                        // Multiply by V: result[i] = sum_j scores[i][j] * V[j]
                        if let Some(result) = TensorHandle::zeros(&[seq_len, v_dim], q.dtype) {
                            if let Some(ref dst_data) = result.data {
                                unsafe {
                                    let out_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;
                                    for i in 0..seq_len {
                                        for j in 0..seq_len {
                                            let s = scores[i * seq_len + j];
                                            for d in 0..v_dim {
                                                let vj = if j * v_dim + d < v.numel { *v_data.add(j * v_dim + d) } else { 0.0 };
                                                *out_ptr.add(i * v_dim + d) += s * vj;
                                            }
                                        }
                                    }
                                }
                            }
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

                Some(TensorExtSubOpcode::Fft) => {
                    // FFT: dst, src, dim (i8), inverse (bool)
                    let dst = read_reg(state)?;
                    let src_reg = read_reg(state)?;
                    let _dim = read_u8(state)? as i8;
                    let inverse = read_u8(state)? != 0;

                    let src_val = state.get_reg(src_reg);
                    let src_ptr = src_val.as_ptr::<TensorHandle>();

                    if !src_ptr.is_null() {
                        let src = unsafe { &*src_ptr };
                        let n = src.numel;
                        if n == 0 {
                            state.set_reg(dst, Value::nil());
                        } else if inverse {
                            // Use existing dispatch_irfft helper
                            if let Some(result) = dispatch_irfft(src, n) {
                                let ptr = Box::into_raw(Box::new(result));
                                state.set_reg(dst, Value::from_ptr(ptr));
                            } else {
                                state.set_reg(dst, Value::nil());
                            }
                        } else {
                            // Use existing dispatch_rfft helper
                            if let Some(result) = dispatch_rfft(src, n) {
                                let ptr = Box::into_raw(Box::new(result));
                                state.set_reg(dst, Value::from_ptr(ptr));
                            } else {
                                state.set_reg(dst, Value::nil());
                            }
                        }
                    } else {
                        state.set_reg(dst, Value::nil());
                    }
                    Ok(DispatchResult::Continue)
                }

                Some(TensorExtSubOpcode::Scatter) => {
                    // Scatter: dst, src, index, values, axis (i8), mode (u8)
                    let dst = read_reg(state)?;
                    let src_reg = read_reg(state)?;
                    let index_reg = read_reg(state)?;
                    let values_reg = read_reg(state)?;
                    let axis = read_u8(state)?;
                    let _mode = read_u8(state)?; // 0=assign, 1=add, 2=mul

                    let src_val = state.get_reg(src_reg);
                    let index_val = state.get_reg(index_reg);
                    let values_val = state.get_reg(values_reg);
                    let src_ptr = src_val.as_ptr::<TensorHandle>();
                    let index_ptr = index_val.as_ptr::<TensorHandle>();
                    let values_ptr = values_val.as_ptr::<TensorHandle>();

                    if !src_ptr.is_null() && !index_ptr.is_null() && !values_ptr.is_null() {
                        let src = unsafe { &*src_ptr };
                        let index = unsafe { &*index_ptr };
                        let values = unsafe { &*values_ptr };

                        // Clone src into mutable result, then scatter values into it
                        if let Some(mut result) = tensor_clone(src) {
                            let _ = super::super::super::tensor::tensor_scatter(&mut result, values, index, axis as usize);
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

                Some(TensorExtSubOpcode::ContiguousView) => {
                    // Contiguous view: dst, src — return a contiguous copy
                    let dst = read_reg(state)?;
                    let src_reg = read_reg(state)?;

                    let src_val = state.get_reg(src_reg);
                    let src_ptr = src_val.as_ptr::<TensorHandle>();

                    if !src_ptr.is_null() {
                        let src = unsafe { &*src_ptr };
                        if let Some(result) = tensor_clone(src) {
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

                Some(TensorExtSubOpcode::RandomU64) => {
                    // Random u64: dst
                    let dst = read_reg(state)?;
                    // Use a simple xorshift64 PRNG
                    use std::time::SystemTime;
                    let seed = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(42);
                    // xorshift64
                    let mut x = seed;
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    state.set_reg(dst, Value::from_i64(x as i64));
                    Ok(DispatchResult::Continue)
                }

                Some(TensorExtSubOpcode::RandomFloat) => {
                    // Random float: dst, low, high
                    let dst = read_reg(state)?;
                    let low_reg = read_reg(state)?;
                    let high_reg = read_reg(state)?;
                    let low = state.get_reg(low_reg).as_f64();
                    let high = state.get_reg(high_reg).as_f64();
                    // Simple PRNG for random float in [low, high)
                    use std::time::SystemTime;
                    let seed = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(42);
                    let mut x = seed;
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    let normalized = (x as f64) / (u64::MAX as f64); // [0, 1)
                    let result = low + normalized * (high - low);
                    state.set_reg(dst, Value::from_f64(result));
                    Ok(DispatchResult::Continue)
                }

                Some(TensorExtSubOpcode::GlobalAllocator) => {
                    // Return a global allocator handle (stub: integer ID)
                    let dst = read_reg(state)?;
                    state.set_reg(dst, Value::from_i64(1)); // Allocator ID 1 (global)
                    Ok(DispatchResult::Continue)
                }

                Some(TensorExtSubOpcode::MemNewId) => {
                    // Allocate new memory ID (incrementing counter)
                    let dst = read_reg(state)?;
                    // Use a simple atomic counter
                    static NEXT_MEM_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
                    let id = NEXT_MEM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    state.set_reg(dst, Value::from_i64(id));
                    Ok(DispatchResult::Continue)
                }

                Some(TensorExtSubOpcode::MemAllocTensor) => {
                    // Allocate tensor with shape/dtype: dst, shape_reg, dtype
                    let dst = read_reg(state)?;
                    let shape_reg = read_reg(state)?;
                    let dtype_byte = read_u8(state)?;

                    let shape_val = state.get_reg(shape_reg);
                    let shape_ptr = shape_val.as_ptr::<TensorHandle>();

                    let shape: Vec<usize> = if !shape_ptr.is_null() {
                        let sh = unsafe { &*shape_ptr };
                        let n = sh.shape[0];
                        let dp = sh.data_ptr_f64();
                        (0..n).map(|i| unsafe { *dp.add(i) } as usize).collect()
                    } else {
                        vec![1]
                    };

                    let dtype = DType::from_type_id(dtype_byte);
                    if let Some(result) = TensorHandle::zeros(&shape, dtype) {
                        let ptr = Box::into_raw(Box::new(result));
                        state.set_reg(dst, Value::from_ptr(ptr));
                    } else {
                        state.set_reg(dst, Value::nil());
                    }
                    Ok(DispatchResult::Continue)
                }

                None => {
                    Err(InterpreterError::NotImplemented {
                        feature: "tensor sub-opcode",
                        opcode: Some(Opcode::TensorExtended),
                    })
                }
            }
        }
    }
}
