//! ML extended opcode handler for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;
use super::string_helpers::{extract_string, alloc_string_value};
use super::super::super::autodiff::GradMode as AutodiffGradMode;

/// Handler for MlExtended opcode (0xFD).
///
/// This dispatches to ML operations based on the sub-opcode byte.
/// Supports tokenizer, sampling, distributed training, and gradient operations.
pub(in super::super) fn handle_ml_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    use crate::instruction::MlSubOpcode;

    let sub_op_byte = read_u8(state)?;
    let sub_op = MlSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ====================================================================
        // Forward-Mode Autodiff (JVP) Operations (0x66-0x67)
        // ====================================================================

        Some(MlSubOpcode::JvpBegin) => {
            let dst = read_reg(state)?;
            let _primals_reg = read_reg(state)?;
            let _tangents_reg = read_reg(state)?;

            // Begin forward-mode autodiff scope
            let scope_id = state.grad_tape.begin_scope(AutodiffGradMode::Forward);
            let id_val = scope_id.map(|s| s.0 as i64).unwrap_or(0);
            state.set_reg(dst, Value::from_i64(id_val));
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::JvpEnd) => {
            let dst = read_reg(state)?;
            let _scope_reg = read_reg(state)?;

            // Run forward pass (tangent propagation) and end scope
            state.grad_tape.backward();
            state.grad_tape.end_scope();
            // Return 0 on success
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Custom Gradient Operations (0x68-0x6A)
        // ====================================================================

        Some(MlSubOpcode::GradCustom) => {
            let dst = read_reg(state)?;
            let forward_fn_reg = read_reg(state)?;
            let backward_fn_reg = read_reg(state)?;

            // Register custom gradient function pair (forward_fn, vjp_fn)
            let forward_fn = state.get_reg(forward_fn_reg).as_i64() as u32;
            let backward_fn = state.get_reg(backward_fn_reg).as_i64() as u32;
            let rule_id = state.grad_tape.register_custom_vjp(forward_fn, backward_fn);
            state.set_reg(dst, Value::from_i64(rule_id.0 as i64));
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::GradZeroTangent) => {
            let _tangents_reg = read_reg(state)?;

            // Zero out all tangent vectors in the current scope
            state.grad_tape.zero_tangents();
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::GradRecompute) => {
            let dst = read_reg(state)?;
            let checkpoint_reg = read_reg(state)?;
            let _grad_output_reg = read_reg(state)?;

            // Recompute forward pass from checkpoint
            let cp_id = state.get_reg(checkpoint_reg).as_i64() as u32;
            let checkpoint_id = super::super::super::autodiff::CheckpointId(cp_id);
            state.grad_tape.recompute(checkpoint_id);
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Existing ML Operations (delegate to kernel dispatchers)
        // ====================================================================

        Some(MlSubOpcode::ZeroGrad) => {
            let _params_reg = read_reg(state)?;

            // Zero all gradients in current scope
            state.grad_tape.zero_grad();
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::ClipGradNorm) => {
            let params_reg = read_reg(state)?;
            let max_norm_reg = read_reg(state)?;

            // Clip gradient norms to max_norm using L2 norm
            // For scalar mode: if |grad| > max_norm, scale to max_norm
            let grad_val = state.get_reg(params_reg).as_f64();
            let max_norm = state.get_reg(max_norm_reg).as_f64();

            let grad_norm = if grad_val < 0.0 { -grad_val } else { grad_val };
            if grad_norm > max_norm && grad_norm > 0.0 {
                let scale = max_norm / grad_norm;
                state.set_reg(params_reg, Value::from_f64(grad_val * scale));
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Tokenizer Operations (0x00-0x06)
        // ====================================================================

        Some(MlSubOpcode::TokenizerLoadBpe) => {
            let dst = read_reg(state)?;
            let vocab_path_reg = read_reg(state)?;
            let merges_path_reg = read_reg(state)?;

            let vocab_path = extract_string(&state.get_reg(vocab_path_reg), state);
            let merges_path = extract_string(&state.get_reg(merges_path_reg), state);

            if let Some(handle) = super::super::super::kernel::dispatch_tokenizer_load_bpe(&vocab_path, &merges_path) {
                let ptr = Box::into_raw(Box::new(handle));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::TokenizerLoadPretrained) => {
            let dst = read_reg(state)?;
            let model_name_reg = read_reg(state)?;

            let model_name = extract_string(&state.get_reg(model_name_reg), state);

            if let Some(handle) = super::super::super::kernel::dispatch_tokenizer_load_pretrained(&model_name) {
                let ptr = Box::into_raw(Box::new(handle));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::TokenizerEncode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;

            let tok_val = state.get_reg(tokenizer_reg);
            let tok_ptr = tok_val.as_ptr::<super::super::super::kernel::tokenizer::TokenizerHandle>();
            let text = extract_string(&state.get_reg(text_reg), state);

            if !tok_ptr.is_null() {
                let tok = unsafe { &*tok_ptr };
                if let Some(tokens) = super::super::super::kernel::dispatch_tokenizer_encode(tok, &text) {
                    // Store token array as a tensor of u32 values
                    let floats: Vec<f64> = tokens.iter().map(|&t| t as f64).collect();
                    if let Some(tensor) = super::super::super::tensor::tensor_from_slice(&floats, &[floats.len()], super::super::super::tensor::DType::F64) {
                        let ptr = Box::into_raw(Box::new(tensor));
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

        Some(MlSubOpcode::TokenizerDecode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let tokens_reg = read_reg(state)?;

            let tok_val = state.get_reg(tokenizer_reg);
            let tok_ptr = tok_val.as_ptr::<super::super::super::kernel::tokenizer::TokenizerHandle>();
            let tokens_val = state.get_reg(tokens_reg);
            let tokens_tensor_ptr = tokens_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !tok_ptr.is_null() && !tokens_tensor_ptr.is_null() {
                let tok = unsafe { &*tok_ptr };
                let tokens_tensor = unsafe { &*tokens_tensor_ptr };
                // Extract u32 tokens from tensor
                let n = tokens_tensor.shape[0];
                let data_ptr = tokens_tensor.data_ptr_f64();
                let mut tokens = Vec::with_capacity(n);
                for i in 0..n {
                    tokens.push(unsafe { *data_ptr.add(i) } as u32);
                }
                if let Some(text) = super::super::super::kernel::dispatch_tokenizer_decode(tok, &tokens) {
                    let val = alloc_string_value(state, &text)?;
                    state.set_reg(dst, val);
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::TokenizerLoadSpm) => {
            let dst = read_reg(state)?;
            let model_path_reg = read_reg(state)?;

            let model_path = extract_string(&state.get_reg(model_path_reg), state);

            if let Some(handle) = super::super::super::kernel::dispatch_tokenizer_load_spm(&model_path) {
                let ptr = Box::into_raw(Box::new(handle));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::TokenizerSpmEncode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let text_reg = read_reg(state)?;

            let tok_val = state.get_reg(tokenizer_reg);
            let tok_ptr = tok_val.as_ptr::<super::super::super::kernel::tokenizer::TokenizerHandle>();
            let text = extract_string(&state.get_reg(text_reg), state);

            if !tok_ptr.is_null() {
                let tok = unsafe { &*tok_ptr };
                if let Some(tokens) = super::super::super::kernel::dispatch_tokenizer_spm_encode(tok, &text) {
                    let floats: Vec<f64> = tokens.iter().map(|&t| t as f64).collect();
                    if let Some(tensor) = super::super::super::tensor::tensor_from_slice(&floats, &[floats.len()], super::super::super::tensor::DType::F64) {
                        let ptr = Box::into_raw(Box::new(tensor));
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

        Some(MlSubOpcode::TokenizerSpmDecode) => {
            let dst = read_reg(state)?;
            let tokenizer_reg = read_reg(state)?;
            let tokens_reg = read_reg(state)?;

            let tok_val = state.get_reg(tokenizer_reg);
            let tok_ptr = tok_val.as_ptr::<super::super::super::kernel::tokenizer::TokenizerHandle>();
            let tokens_val = state.get_reg(tokens_reg);
            let tokens_tensor_ptr = tokens_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !tok_ptr.is_null() && !tokens_tensor_ptr.is_null() {
                let tok = unsafe { &*tok_ptr };
                let tokens_tensor = unsafe { &*tokens_tensor_ptr };
                let n = tokens_tensor.shape[0];
                let data_ptr = tokens_tensor.data_ptr_f64();
                let mut tokens = Vec::with_capacity(n);
                for i in 0..n {
                    tokens.push(unsafe { *data_ptr.add(i) } as u32);
                }
                if let Some(text) = super::super::super::kernel::dispatch_tokenizer_spm_decode(tok, &tokens) {
                    let val = alloc_string_value(state, &text)?;
                    state.set_reg(dst, val);
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Sampling Operations (0x10-0x15)
        // ====================================================================

        Some(MlSubOpcode::SampleTopP) => {
            let dst = read_reg(state)?;
            let logits_reg = read_reg(state)?;
            let p_reg = read_reg(state)?;

            let logits_val = state.get_reg(logits_reg);
            let logits_ptr = logits_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let p = state.get_reg(p_reg).as_f64();

            if !logits_ptr.is_null() {
                let logits = unsafe { &*logits_ptr };
                if let Some(token) = super::super::super::kernel::dispatch_sample_top_p(logits, p) {
                    state.set_reg(dst, Value::from_i64(token as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::SampleTemperature) => {
            let dst = read_reg(state)?;
            let logits_reg = read_reg(state)?;
            let temp_reg = read_reg(state)?;

            let logits_val = state.get_reg(logits_reg);
            let logits_ptr = logits_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let temperature = state.get_reg(temp_reg).as_f64();

            if !logits_ptr.is_null() {
                let logits = unsafe { &*logits_ptr };
                if let Some(token) = super::super::super::kernel::dispatch_sample_temperature(logits, temperature) {
                    state.set_reg(dst, Value::from_i64(token as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::PagedAttention) => {
            let dst = read_reg(state)?;
            let q_reg = read_reg(state)?;
            let kv_cache_reg = read_reg(state)?;
            let block_table_reg = read_reg(state)?;
            let context_len_reg = read_reg(state)?;

            let q_val = state.get_reg(q_reg);
            let q_ptr = q_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let kv_val = state.get_reg(kv_cache_reg);
            let kv_ptr = kv_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let bt_val = state.get_reg(block_table_reg);
            let bt_ptr = bt_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let context_len = state.get_reg(context_len_reg).as_i64() as usize;

            if !q_ptr.is_null() && !kv_ptr.is_null() && !bt_ptr.is_null() {
                let q = unsafe { &*q_ptr };
                let kv = unsafe { &*kv_ptr };
                let bt = unsafe { &*bt_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_paged_attention(q, kv, bt, context_len) {
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

        Some(MlSubOpcode::SampleTopK) => {
            let dst = read_reg(state)?;
            let logits_reg = read_reg(state)?;
            let k_reg = read_reg(state)?;

            let logits_val = state.get_reg(logits_reg);
            let logits_ptr = logits_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let k = state.get_reg(k_reg).as_i64() as usize;

            if !logits_ptr.is_null() {
                let logits = unsafe { &*logits_ptr };
                if let Some(token) = super::super::super::kernel::dispatch_sample_top_k(logits, k) {
                    state.set_reg(dst, Value::from_i64(token as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::SampleTopKTopP) => {
            let dst = read_reg(state)?;
            let logits_reg = read_reg(state)?;
            let k_reg = read_reg(state)?;
            let p_reg = read_reg(state)?;

            let logits_val = state.get_reg(logits_reg);
            let logits_ptr = logits_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let k = state.get_reg(k_reg).as_i64() as usize;
            let p = state.get_reg(p_reg).as_f64();

            if !logits_ptr.is_null() {
                let logits = unsafe { &*logits_ptr };
                if let Some(token) = super::super::super::kernel::dispatch_sample_top_k_top_p(logits, k, p) {
                    state.set_reg(dst, Value::from_i64(token as i64));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::RepetitionPenalty) => {
            let dst = read_reg(state)?;
            let logits_reg = read_reg(state)?;
            let past_tokens_reg = read_reg(state)?;
            let penalty_reg = read_reg(state)?;

            let logits_val = state.get_reg(logits_reg);
            let logits_ptr = logits_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let past_val = state.get_reg(past_tokens_reg);
            let past_ptr = past_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let penalty = state.get_reg(penalty_reg).as_f64();

            if !logits_ptr.is_null() && !past_ptr.is_null() {
                let logits = unsafe { &*logits_ptr };
                let past = unsafe { &*past_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_repetition_penalty(logits, past, penalty) {
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
        // Inference Utility Operations (0x20-0x28)
        // ====================================================================

        Some(MlSubOpcode::ParseToolCall) => {
            let dst = read_reg(state)?;
            let action_reg = read_reg(state)?;

            let action = extract_string(&state.get_reg(action_reg), state);
            if let Some((name, args)) = super::super::super::kernel::dispatch_parse_tool_call(&action) {
                // Return as a string "{name}:{args}"
                let result = format!("{}:{}", name, args);
                let val = alloc_string_value(state, &result)?;
                state.set_reg(dst, val);
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::FormatValue) => {
            let dst = read_reg(state)?;
            let value_reg = read_reg(state)?;

            let val = state.get_reg(value_reg);
            let formatted = super::super::super::kernel::dispatch_format_value(&val);
            let str_val = alloc_string_value(state, &formatted)?;
            state.set_reg(dst, str_val);
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::QuantizedMatmul) => {
            let dst = read_reg(state)?;
            let input_reg = read_reg(state)?;
            let weight_reg = read_reg(state)?;
            let scale_reg = read_reg(state)?;
            let zero_point_reg = read_reg(state)?;

            let input_val = state.get_reg(input_reg);
            let input_ptr = input_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let weight_val = state.get_reg(weight_reg);
            let weight_ptr = weight_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let scale_val = state.get_reg(scale_reg);
            let scale_ptr = scale_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let zp_val = state.get_reg(zero_point_reg);
            let zp_ptr = zp_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !input_ptr.is_null() && !weight_ptr.is_null() && !scale_ptr.is_null() && !zp_ptr.is_null() {
                let input = unsafe { &*input_ptr };
                let weight = unsafe { &*weight_ptr };
                let scale = unsafe { &*scale_ptr };
                let zp = unsafe { &*zp_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_quantized_matmul(input, weight, scale, zp) {
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

        Some(MlSubOpcode::GenerateRequestId) => {
            let dst = read_reg(state)?;

            let id = super::super::super::kernel::dispatch_generate_request_id();
            let val = alloc_string_value(state, &id)?;
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::JsonSchemaToJson) => {
            let dst = read_reg(state)?;
            let schema_reg = read_reg(state)?;

            let schema = state.get_reg(schema_reg);
            let json = super::super::super::kernel::dispatch_json_schema_to_json(&schema);
            let val = alloc_string_value(state, &json)?;
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::FunctionSchemaToJson) => {
            let dst = read_reg(state)?;
            let schema_reg = read_reg(state)?;

            let schema = state.get_reg(schema_reg);
            let json = super::super::super::kernel::dispatch_function_schema_to_json(&schema);
            let val = alloc_string_value(state, &json)?;
            state.set_reg(dst, val);
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::ParseFunctionCalls) => {
            let dst = read_reg(state)?;
            let response_reg = read_reg(state)?;

            let response = extract_string(&state.get_reg(response_reg), state);
            if let Some(calls) = super::super::super::kernel::dispatch_parse_function_calls(&response) {
                // Return count of parsed calls as int
                state.set_reg(dst, Value::from_i64(calls.len() as i64));
            } else {
                state.set_reg(dst, Value::from_i64(0));
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::KvCacheOp) => {
            let dst = read_reg(state)?;
            let op = read_u8(state)?;
            let cache_reg = read_reg(state)?;

            let cache_val = state.get_reg(cache_reg);
            let result = super::super::super::kernel::dispatch_kv_cache_op(op, &cache_val);
            state.set_reg(dst, result);
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::SpeculativeVerify) => {
            let dst = read_reg(state)?;
            let draft_tokens_reg = read_reg(state)?;
            let target_probs_reg = read_reg(state)?;

            let draft_val = state.get_reg(draft_tokens_reg);
            let draft_ptr = draft_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let target_val = state.get_reg(target_probs_reg);
            let target_ptr = target_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !draft_ptr.is_null() && !target_ptr.is_null() {
                let draft = unsafe { &*draft_ptr };
                let target = unsafe { &*target_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_speculative_verify(draft, target) {
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
        // Distributed/Collective Operations (0x30-0x3A)
        // ====================================================================

        Some(MlSubOpcode::AllReduce) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;
            let op_byte = read_u8(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                let group = unsafe { &*group_ptr };
                let op = super::super::super::kernel::ReduceOp::from_u8(op_byte).unwrap_or(super::super::super::kernel::ReduceOp::Sum);
                if let Some(result) = super::super::super::kernel::dispatch_all_reduce(tensor, group, op) {
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

        Some(MlSubOpcode::AllGather) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                let group = unsafe { &*group_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_all_gather(tensor, group) {
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

        Some(MlSubOpcode::Broadcast) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let src_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let src_rank = state.get_reg(src_reg).as_i64() as usize;
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                let group = unsafe { &*group_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_broadcast(tensor, src_rank, group) {
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

        Some(MlSubOpcode::ReduceScatter) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;
            let op_byte = read_u8(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                let group = unsafe { &*group_ptr };
                let op = super::super::super::kernel::ReduceOp::from_u8(op_byte).unwrap_or(super::super::super::kernel::ReduceOp::Sum);
                if let Some(result) = super::super::super::kernel::dispatch_reduce_scatter(tensor, group, op) {
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

        Some(MlSubOpcode::Barrier) => {
            let group_reg = read_reg(state)?;

            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !group_ptr.is_null() {
                let group = unsafe { &*group_ptr };
                super::super::super::kernel::dispatch_barrier(group);
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::PmapPsum) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let axis_name = extract_string(&state.get_reg(axis_name_reg), state);

            if !tensor_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_pmap_psum(tensor, &axis_name) {
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

        Some(MlSubOpcode::PmapPmean) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let axis_name = extract_string(&state.get_reg(axis_name_reg), state);

            if !tensor_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_pmap_pmean(tensor, &axis_name) {
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

        Some(MlSubOpcode::PmapPmax) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let axis_name = extract_string(&state.get_reg(axis_name_reg), state);

            if !tensor_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_pmap_pmax(tensor, &axis_name) {
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

        Some(MlSubOpcode::PmapAllGather) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let axis_name_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let axis_name = extract_string(&state.get_reg(axis_name_reg), state);

            if !tensor_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_pmap_all_gather(tensor, &axis_name) {
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

        Some(MlSubOpcode::VmapTransform) => {
            let dst = read_reg(state)?;
            let _func_reg = read_reg(state)?;
            let _in_axes_reg = read_reg(state)?;
            let _out_axes_reg = read_reg(state)?;

            // Vmap transform stub: returns nil (requires JIT tracing)
            let _ = super::super::super::kernel::dispatch_vmap_transform();
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::PmapTransform) => {
            let dst = read_reg(state)?;
            let _func_reg = read_reg(state)?;
            let _axis_name_reg = read_reg(state)?;
            let _in_axes_reg = read_reg(state)?;
            let _out_axes_reg = read_reg(state)?;

            // Pmap transform stub: returns nil (requires distributed runtime)
            let _ = super::super::super::kernel::dispatch_pmap_transform();
            state.set_reg(dst, Value::nil());
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Process Group Operations (0x40-0x44)
        // ====================================================================

        Some(MlSubOpcode::DistWorldGroup) => {
            let dst = read_reg(state)?;

            let group = super::super::super::kernel::dispatch_dist_world_group();
            let ptr = Box::into_raw(Box::new(group));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::DistNewGroup) => {
            let dst = read_reg(state)?;
            let ranks_reg = read_reg(state)?;

            // Ranks register contains a tensor of rank indices
            let ranks_val = state.get_reg(ranks_reg);
            let ranks_ptr = ranks_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !ranks_ptr.is_null() {
                let ranks_tensor = unsafe { &*ranks_ptr };
                let n = ranks_tensor.shape[0];
                let data_ptr = ranks_tensor.data_ptr_f64();
                let mut ranks = Vec::with_capacity(n);
                for i in 0..n {
                    ranks.push(unsafe { *data_ptr.add(i) } as usize);
                }
                let group = super::super::super::kernel::dispatch_dist_new_group(&ranks);
                let ptr = Box::into_raw(Box::new(group));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                // Default: create a single-rank group
                let group = super::super::super::kernel::dispatch_dist_new_group(&[0]);
                let ptr = Box::into_raw(Box::new(group));
                state.set_reg(dst, Value::from_ptr(ptr));
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::DistGetRank) => {
            let dst = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !group_ptr.is_null() {
                let group = unsafe { &*group_ptr };
                let rank = super::super::super::kernel::dispatch_dist_get_rank(group);
                state.set_reg(dst, Value::from_i64(rank as i64));
            } else {
                state.set_reg(dst, Value::from_i64(0));
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::DistWorldSize) => {
            let dst = read_reg(state)?;

            let size = super::super::super::kernel::dispatch_dist_world_size();
            state.set_reg(dst, Value::from_i64(size as i64));
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::DistLocalRank) => {
            let dst = read_reg(state)?;

            let rank = super::super::super::kernel::dispatch_dist_local_rank();
            state.set_reg(dst, Value::from_i64(rank as i64));
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Point-to-Point Operations (0x50-0x54)
        // ====================================================================

        Some(MlSubOpcode::P2PSend) => {
            let tensor_reg = read_reg(state)?;
            let dst_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let dst_rank = state.get_reg(dst_rank_reg).as_i64() as usize;
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                let group = unsafe { &*group_ptr };
                super::super::super::kernel::dispatch_p2p_send(tensor, dst_rank, group);
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::P2PRecv) => {
            let dst = read_reg(state)?;
            let src_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let src_rank = state.get_reg(src_rank_reg).as_i64() as usize;
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !group_ptr.is_null() {
                let group = unsafe { &*group_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_p2p_recv(src_rank, group) {
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

        Some(MlSubOpcode::P2PIsend) => {
            let handle_dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;
            let dst_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();
            let dst_rank = state.get_reg(dst_rank_reg).as_i64() as usize;
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !tensor_ptr.is_null() && !group_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                let group = unsafe { &*group_ptr };
                let handle_id = super::super::super::kernel::dispatch_p2p_isend(tensor, dst_rank, group);
                state.set_reg(handle_dst, Value::from_i64(handle_id as i64));
            } else {
                state.set_reg(handle_dst, Value::from_i64(0));
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::P2PIrecv) => {
            let handle_dst = read_reg(state)?;
            let dst = read_reg(state)?;
            let src_rank_reg = read_reg(state)?;
            let group_reg = read_reg(state)?;

            let src_rank = state.get_reg(src_rank_reg).as_i64() as usize;
            let group_val = state.get_reg(group_reg);
            let group_ptr = group_val.as_ptr::<super::super::super::kernel::ProcessGroupHandle>();

            if !group_ptr.is_null() {
                let group = unsafe { &*group_ptr };
                let (handle_id, maybe_tensor) = super::super::super::kernel::dispatch_p2p_irecv(src_rank, group);
                state.set_reg(handle_dst, Value::from_i64(handle_id as i64));
                if let Some(tensor) = maybe_tensor {
                    let ptr = Box::into_raw(Box::new(tensor));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(handle_dst, Value::from_i64(0));
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::P2PWait) => {
            let handle_reg = read_reg(state)?;

            let handle_id = state.get_reg(handle_reg).as_i64() as u64;
            super::super::super::kernel::dispatch_p2p_wait(handle_id);
            Ok(DispatchResult::Continue)
        }

        // ====================================================================
        // Gradient Operations (0x60-0x63) — remaining ones
        // ====================================================================

        Some(MlSubOpcode::BucketGradients) => {
            let dst = read_reg(state)?;
            let gradients_reg = read_reg(state)?;
            let bucket_size_reg = read_reg(state)?;

            let _grads_val = state.get_reg(gradients_reg);
            let _bucket_size = state.get_reg(bucket_size_reg).as_i64() as usize;

            // Stub: gradient bucketing is a no-op in single-process mode
            // Just return the same gradients register value
            state.set_reg(dst, state.get_reg(gradients_reg));
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::GetGrad) => {
            let dst = read_reg(state)?;
            let param_reg = read_reg(state)?;

            let param_val = state.get_reg(param_reg);
            let param_ptr = param_val.as_ptr::<super::super::super::kernel::ParameterHandle>();

            if !param_ptr.is_null() {
                let param = unsafe { &*param_ptr };
                if let Some(grad) = super::super::super::kernel::dispatch_get_grad(param) {
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

        Some(MlSubOpcode::SetGrad) => {
            let param_reg = read_reg(state)?;
            let grad_reg = read_reg(state)?;

            let param_val = state.get_reg(param_reg);
            let param_ptr = param_val.as_ptr::<super::super::super::kernel::ParameterHandle>() as *mut super::super::super::kernel::ParameterHandle;
            let grad_val = state.get_reg(grad_reg);
            let grad_ptr = grad_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !param_ptr.is_null() && !grad_ptr.is_null() {
                let param = unsafe { &mut *param_ptr };
                let grad = unsafe { &*grad_ptr };
                super::super::super::kernel::dispatch_set_grad(param, grad.clone());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::ModuleBackward) => {
            let dst = read_reg(state)?;
            let _module_reg = read_reg(state)?;
            let grad_output_reg = read_reg(state)?;

            let grad_val = state.get_reg(grad_output_reg);
            let grad_ptr = grad_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !grad_ptr.is_null() {
                let grad = unsafe { &*grad_ptr };
                if let Some(result) = super::super::super::kernel::dispatch_module_backward(&(), grad) {
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
        // Actor/Mesh Operations (0x70-0x73)
        // ====================================================================

        Some(MlSubOpcode::MeshSelect) => {
            let dst = read_reg(state)?;
            let mesh_reg = read_reg(state)?;
            let coords_reg = read_reg(state)?;

            let mesh_val = state.get_reg(mesh_reg);
            let mesh_ptr = mesh_val.as_ptr::<super::super::super::kernel::ActorMeshHandle>();
            let coords_val = state.get_reg(coords_reg);
            let coords_ptr = coords_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !mesh_ptr.is_null() {
                let mesh = unsafe { &*mesh_ptr };
                let coords = if !coords_ptr.is_null() {
                    let ct = unsafe { &*coords_ptr };
                    let n = ct.shape[0];
                    let dp = ct.data_ptr_f64();
                    (0..n).map(|i| unsafe { *dp.add(i) } as usize).collect::<Vec<_>>()
                } else {
                    vec![0]
                };
                let result = super::super::super::kernel::dispatch_mesh_select(mesh, &coords);
                let ptr = Box::into_raw(Box::new(result));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::ActorNewId) => {
            let dst = read_reg(state)?;

            let id = super::super::super::kernel::dispatch_actor_new_id();
            state.set_reg(dst, Value::from_i64(id.id as i64));
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::MeshCreate) => {
            let dst = read_reg(state)?;
            let shape_reg = read_reg(state)?;

            let shape_val = state.get_reg(shape_reg);
            let shape_ptr = shape_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !shape_ptr.is_null() {
                let shape_tensor = unsafe { &*shape_ptr };
                let n = shape_tensor.shape[0];
                let dp = shape_tensor.data_ptr_f64();
                let shape: Vec<usize> = (0..n).map(|i| unsafe { *dp.add(i) } as usize).collect();
                let mesh = super::super::super::kernel::dispatch_mesh_create(&shape);
                let ptr = Box::into_raw(Box::new(mesh));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                let mesh = super::super::super::kernel::dispatch_mesh_create(&[1]);
                let ptr = Box::into_raw(Box::new(mesh));
                state.set_reg(dst, Value::from_ptr(ptr));
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::MeshShape) => {
            let dst = read_reg(state)?;
            let mesh_reg = read_reg(state)?;

            let mesh_val = state.get_reg(mesh_reg);
            let mesh_ptr = mesh_val.as_ptr::<super::super::super::kernel::ActorMeshHandle>();

            if !mesh_ptr.is_null() {
                let mesh = unsafe { &*mesh_ptr };
                let shape = super::super::super::kernel::dispatch_mesh_shape(mesh);
                let floats: Vec<f64> = shape.iter().map(|&s| s as f64).collect();
                if let Some(tensor) = super::super::super::tensor::tensor_from_slice(&floats, &[floats.len()], super::super::super::tensor::DType::F64) {
                    let ptr = Box::into_raw(Box::new(tensor));
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
        // RDMA Operations (0x80-0x83)
        // ====================================================================

        Some(MlSubOpcode::RdmaCreateRef) => {
            let dst = read_reg(state)?;
            let tensor_reg = read_reg(state)?;

            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !tensor_ptr.is_null() {
                let tensor = unsafe { &*tensor_ptr };
                let rdma_ref = super::super::super::kernel::dispatch_rdma_create_ref(tensor);
                let ptr = Box::into_raw(Box::new(rdma_ref));
                state.set_reg(dst, Value::from_ptr(ptr));
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::RdmaFetch) => {
            let dst = read_reg(state)?;
            let rdma_ref_reg = read_reg(state)?;

            let ref_val = state.get_reg(rdma_ref_reg);
            let ref_ptr = ref_val.as_ptr::<super::super::super::kernel::RdmaRefHandle>();

            if !ref_ptr.is_null() {
                let rdma_ref = unsafe { &*ref_ptr };
                if let Some(tensor) = super::super::super::kernel::dispatch_rdma_fetch(rdma_ref) {
                    let ptr = Box::into_raw(Box::new(tensor));
                    state.set_reg(dst, Value::from_ptr(ptr));
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::RdmaWrite) => {
            let rdma_ref_reg = read_reg(state)?;
            let tensor_reg = read_reg(state)?;

            let ref_val = state.get_reg(rdma_ref_reg);
            let ref_ptr = ref_val.as_ptr::<super::super::super::kernel::RdmaRefHandle>() as *mut super::super::super::kernel::RdmaRefHandle;
            let tensor_val = state.get_reg(tensor_reg);
            let tensor_ptr = tensor_val.as_ptr::<super::super::super::tensor::TensorHandle>();

            if !ref_ptr.is_null() && !tensor_ptr.is_null() {
                let rdma_ref = unsafe { &mut *ref_ptr };
                let tensor = unsafe { &*tensor_ptr };
                super::super::super::kernel::dispatch_rdma_write(rdma_ref, tensor);
            }
            Ok(DispatchResult::Continue)
        }

        Some(MlSubOpcode::RdmaCheckValid) => {
            let dst = read_reg(state)?;
            let rdma_ref_reg = read_reg(state)?;

            let ref_val = state.get_reg(rdma_ref_reg);
            let ref_ptr = ref_val.as_ptr::<super::super::super::kernel::RdmaRefHandle>();

            if !ref_ptr.is_null() {
                let rdma_ref = unsafe { &*ref_ptr };
                let valid = super::super::super::kernel::dispatch_rdma_check_valid(rdma_ref);
                state.set_reg(dst, Value::from_bool(valid));
            } else {
                state.set_reg(dst, Value::from_bool(false));
            }
            Ok(DispatchResult::Continue)
        }

        // Fallback for unknown sub-opcode bytes (from_byte returned None)
        None => {
            use crate::instruction::Opcode;
            Err(InterpreterError::NotImplemented {
                feature: "ML sub-opcode",
                opcode: Some(Opcode::MlExtended),
            })
        }
    }
}
