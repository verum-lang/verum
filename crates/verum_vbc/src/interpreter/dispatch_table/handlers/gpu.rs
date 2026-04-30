//! GPU extended opcode handlers for VBC interpreter dispatch.

use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::super::dispatch_loop_table_with_entry_depth;
use super::bytecode_io::*;

// ============================================================================
// GPU Extended Handler (0xF8)
// ============================================================================

/// Handler for GpuExtended opcode (0xF8).
///
/// This dispatches to GPU operations based on the GpuSubOpcode byte.
/// Most operations return stubs since the interpreter uses CPU fallbacks.
pub(in super::super) fn handle_gpu_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    use crate::instruction::GpuSubOpcode;
    use super::super::super::kernel::device::{get_registry, Vendor};

    let sub_op_byte = read_u8(state)?;
    let sub_op = GpuSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Device Enumeration (0x90-0x93)
        // ================================================================
        Some(GpuSubOpcode::EnumerateCuda) => {
            // CUDA not supported in interpreter - return empty list
            let dst = read_reg(state)?;
            // Return an empty list as Value
            let empty_list: Vec<Value> = Vec::new();
            let ptr = Box::into_raw(Box::new(empty_list));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EnumerateMetal) => {
            // Return detected Metal devices on macOS
            let dst = read_reg(state)?;
            let registry = get_registry();
            let metal_devices: Vec<Value> = registry.gpus()
                .filter(|(_, info)| info.vendor == Vendor::Apple)
                .map(|(id, _)| Value::from_i64(id.0 as i64))
                .collect();
            let ptr = Box::into_raw(Box::new(metal_devices));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EnumerateRocm) => {
            // ROCm not supported in interpreter - return empty list
            let dst = read_reg(state)?;
            let empty_list: Vec<Value> = Vec::new();
            let ptr = Box::into_raw(Box::new(empty_list));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EnumerateVulkan) => {
            // Vulkan not supported in interpreter - return empty list
            let dst = read_reg(state)?;
            let empty_list: Vec<Value> = Vec::new();
            let ptr = Box::into_raw(Box::new(empty_list));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Device Management (0x50-0x5F) - Stubs
        // ================================================================
        Some(GpuSubOpcode::GetDevice) => {
            let dst = read_reg(state)?;
            // Return current device ID from GPU context
            state.set_reg(dst, Value::from_i64(state.gpu_context.device_id as i64));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::SetDevice) => {
            let dst = read_reg(state)?;
            let device_reg = read_reg(state)?;
            // Set the active device in the GPU context
            let device_id = state.get_reg(device_reg).as_i64() as u32;
            state.gpu_context.device_id = device_id;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GetDeviceCount) => {
            let dst = read_reg(state)?;
            let registry = get_registry();
            let gpu_count = registry.gpus().count();
            state.set_reg(dst, Value::from_i64(gpu_count as i64));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GetMemoryInfo) => {
            let dst = read_reg(state)?;
            let device_reg = read_reg(state)?;
            // Report actual registry-reported memory if a real device is
            // registered; otherwise return (0, 0) so callers can detect
            // the absence of a usable GPU instead of assuming 8GB exists.
            let device_id = state.get_reg(device_reg).as_i64() as u16;
            let registry = get_registry();
            let mem_info: (u64, u64) = registry.gpus()
                .find(|(id, _)| id.0 == device_id)
                .map(|(_, info)| (info.memory_bytes as u64, info.memory_bytes as u64))
                .unwrap_or((0, 0));
            let ptr = Box::into_raw(Box::new(mem_info));
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::DeviceReset) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Synchronization (0x10-0x1F) - Stubs
        // ================================================================
        Some(GpuSubOpcode::SyncStream) | Some(GpuSubOpcode::SyncDevice) => {
            let dst = read_reg(state)?;
            let _stream_or_device = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::SyncEvent) => {
            let dst = read_reg(state)?;
            let _event = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::QueryStream) => {
            let dst = read_reg(state)?;
            let _stream = read_reg(state)?;
            // Always complete in interpreter
            state.set_reg(dst, Value::from_bool(true));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Stream/Event Management - Stubs
        // ================================================================
        Some(GpuSubOpcode::StreamCreate) | Some(GpuSubOpcode::StreamCreateNonBlocking) => {
            let dst = read_reg(state)?;
            // Allocate a new stream handle from the GPU context
            let stream_id = state.gpu_context.next_stream_id;
            state.gpu_context.next_stream_id += 1;
            state.gpu_context.streams.insert(stream_id, true);
            state.set_reg(dst, Value::from_i64(stream_id as i64));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::StreamCreateWithPriority) => {
            let dst = read_reg(state)?;
            let _priority = read_reg(state)?; // Read priority register (ignored in CPU fallback)
            let stream_id = state.gpu_context.next_stream_id;
            state.gpu_context.next_stream_id += 1;
            state.gpu_context.streams.insert(stream_id, true);
            state.set_reg(dst, Value::from_i64(stream_id as i64));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::StreamDestroy) => {
            let dst = read_reg(state)?;
            let stream_reg = read_reg(state)?;
            let stream_id = state.get_reg(stream_reg).as_i64() as u32;
            // Remove stream from GPU context
            state.gpu_context.streams.remove(&stream_id);
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::StreamWaitEvent) => {
            let dst = read_reg(state)?;
            let _stream = read_reg(state)?;
            let _event = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::StreamAddCallback) => {
            let dst = read_reg(state)?;
            let _stream = read_reg(state)?;
            let _callback_id = read_varint(state)?;
            let _user_data = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::StreamGetPriority) => {
            let dst = read_reg(state)?;
            let _stream = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // Default priority
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::StreamQuery) => {
            let dst = read_reg(state)?;
            let stream_reg = read_reg(state)?;
            let stream_id = state.get_reg(stream_reg).as_i64() as u32;
            // Check if stream exists in the GPU context
            let exists = state.gpu_context.streams.contains_key(&stream_id);
            state.set_reg(dst, Value::from_bool(exists));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EventCreate) | Some(GpuSubOpcode::EventCreateWithFlags) => {
            let dst = read_reg(state)?;
            // Allocate a real event with timestamp tracking
            let event_id = state.gpu_context.next_event_id;
            state.gpu_context.next_event_id += 1;
            state.set_reg(dst, Value::from_i64(event_id as i64));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EventDestroy) => {
            let dst = read_reg(state)?;
            let event_reg = read_reg(state)?;
            let event_id = state.get_reg(event_reg).as_i64() as u32;
            state.gpu_context.events.remove(&event_id);
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EventRecord) => {
            let dst = read_reg(state)?;
            let event_reg = read_reg(state)?;
            let _stream_reg = read_reg(state)?;
            let event_id = state.get_reg(event_reg).as_i64() as u32;
            // Record current timestamp for this event
            state.gpu_context.events.insert(event_id, std::time::Instant::now());
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EventRecordWithFlags) => {
            let dst = read_reg(state)?;
            let event_reg = read_reg(state)?;
            let _stream_reg = read_reg(state)?;
            let _flags = read_u8(state)?;
            let event_id = state.get_reg(event_reg).as_i64() as u32;
            state.gpu_context.events.insert(event_id, std::time::Instant::now());
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EventSynchronize) => {
            let dst = read_reg(state)?;
            let _event_reg = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EventQuery) => {
            let dst = read_reg(state)?;
            let _event = read_reg(state)?;
            // Always complete in interpreter
            state.set_reg(dst, Value::from_bool(true));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EventElapsed) => {
            let dst = read_reg(state)?;
            let start_reg = read_reg(state)?;
            let end_reg = read_reg(state)?;
            let start_id = state.get_reg(start_reg).as_i64() as u32;
            let end_id = state.get_reg(end_reg).as_i64() as u32;
            // Compute real elapsed time between events in milliseconds
            let elapsed_ms = match (state.gpu_context.events.get(&start_id), state.gpu_context.events.get(&end_id)) {
                (Some(start), Some(end)) => {
                    let duration = end.duration_since(*start);
                    duration.as_secs_f64() * 1000.0
                }
                _ => 0.0, // Events not recorded
            };
            state.set_reg(dst, Value::from_f64(elapsed_ms));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Memory Operations - Stubs (interpreter uses CPU memory)
        // ================================================================
        Some(GpuSubOpcode::Alloc) => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _device_reg = read_reg(state)?; // memory_space arg (CPU fallback ignores)
            let size = state.get_reg(size_reg).as_i64() as usize;
            let alloc_size = if size == 0 { 1 } else { size };
            // Layout-construction failure means the requested size
            // exceeds isize::MAX once aligned.  The previous
            // `unwrap_or(Layout::new::<u8>())` fallback silently
            // downgraded to a 1-byte allocation while leaving the
            // caller believing they got `alloc_size` bytes — a heap
            // overflow waiting to happen the first time the caller
            // wrote past byte 0.  Treat as null pointer (allocation
            // failure), matching the standard malloc-fail contract.
            let layout = match std::alloc::Layout::from_size_align(alloc_size, 8) {
                Ok(l) => l,
                Err(_) => {
                    state.set_reg(dst, Value::from_ptr::<u8>(std::ptr::null_mut()));
                    return Ok(DispatchResult::Continue);
                }
            };
            let ptr = unsafe { std::alloc::alloc(layout) };
            if !ptr.is_null() {
                state.gpu_context.allocated_buffers.insert(ptr as usize, alloc_size);
            }
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::MallocManaged) => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let size = state.get_reg(size_reg).as_i64() as usize;
            let alloc_size = if size == 0 { 1 } else { size };
            // Same heap-overflow class as GpuSubOpcode::Alloc — never
            // silently downgrade an oversize layout to 1 byte.
            let layout = match std::alloc::Layout::from_size_align(alloc_size, 8) {
                Ok(l) => l,
                Err(_) => {
                    state.set_reg(dst, Value::from_ptr::<u8>(std::ptr::null_mut()));
                    return Ok(DispatchResult::Continue);
                }
            };
            let ptr = unsafe { std::alloc::alloc(layout) };
            if !ptr.is_null() {
                state.gpu_context.allocated_buffers.insert(ptr as usize, alloc_size);
            }
            state.set_reg(dst, Value::from_ptr(ptr));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::Free) => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let ptr_addr = ptr as usize;
            // Look up the allocation size and deallocate properly.
            //
            // Layout-construction failure on the dealloc path is
            // architecturally impossible — `allocated_buffers` only
            // contains `alloc_size` values that successfully
            // constructed a layout above, so the matching call here
            // cannot fail.  If it ever does, leak the allocation
            // rather than dealloc with a wrong (1-byte) layout —
            // dealloc with mismatched layout is undefined behaviour
            // in std::alloc; the leak is strictly the safer
            // failure mode.
            if let Some(size) = state.gpu_context.allocated_buffers.remove(&ptr_addr)
                && let Ok(layout) = std::alloc::Layout::from_size_align(size, 8) {
                    unsafe { std::alloc::dealloc(ptr, layout); }
                }
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::PinMemory) | Some(GpuSubOpcode::UnpinMemory) => {
            let dst = read_reg(state)?;
            let _ptr = read_reg(state)?;
            let _size = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::Prefetch) | Some(GpuSubOpcode::PrefetchAsync) => {
            let dst = read_reg(state)?;
            let _ptr = read_reg(state)?;
            let _size = read_reg(state)?;
            let _device = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::MemAdvise) => {
            let dst = read_reg(state)?;
            let _ptr = read_reg(state)?;
            let _size = read_reg(state)?;
            let _advice = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::Memcpy) | Some(GpuSubOpcode::MemcpyAsync) |
        Some(GpuSubOpcode::Memcpy2D) | Some(GpuSubOpcode::Memcpy2DAsync) => {
            let dst = read_reg(state)?;
            let dst_ptr_reg = read_reg(state)?;
            let src_ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let dst_ptr = state.get_reg(dst_ptr_reg).as_ptr::<u8>();
            let src_ptr = state.get_reg(src_ptr_reg).as_ptr::<u8>();
            let size = state.get_reg(size_reg).as_i64() as usize;
            if !dst_ptr.is_null() && !src_ptr.is_null() && size > 0 {
                unsafe {
                    std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, size);
                }
            }
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        // Dedicated direction-specific memcpy (synchronous)
        Some(GpuSubOpcode::MemcpyH2D) | Some(GpuSubOpcode::MemcpyD2H) |
        Some(GpuSubOpcode::MemcpyD2D) => {
            // In interpreter, all memory is CPU-accessible, so these are equivalent
            let dst = read_reg(state)?;
            let dst_ptr_reg = read_reg(state)?;
            let src_ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let dst_ptr = state.get_reg(dst_ptr_reg).as_ptr::<u8>();
            let src_ptr = state.get_reg(src_ptr_reg).as_ptr::<u8>();
            let size = state.get_reg(size_reg).as_i64() as usize;
            if !dst_ptr.is_null() && !src_ptr.is_null() && size > 0 {
                unsafe {
                    std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, size);
                }
            }
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        // Dedicated direction-specific memcpy (asynchronous) - sync in interpreter
        Some(GpuSubOpcode::MemcpyAsyncH2D) | Some(GpuSubOpcode::MemcpyAsyncD2H) => {
            let dst = read_reg(state)?;
            let dst_ptr_reg = read_reg(state)?;
            let src_ptr_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let _stream_reg = read_reg(state)?; // Ignored in interpreter
            let dst_ptr = state.get_reg(dst_ptr_reg).as_ptr::<u8>();
            let src_ptr = state.get_reg(src_ptr_reg).as_ptr::<u8>();
            let size = state.get_reg(size_reg).as_i64() as usize;
            if !dst_ptr.is_null() && !src_ptr.is_null() && size > 0 {
                unsafe {
                    std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, size);
                }
            }
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::Memset) | Some(GpuSubOpcode::MemsetAsync) => {
            let dst = read_reg(state)?;
            let ptr_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let ptr = state.get_reg(ptr_reg).as_ptr::<u8>();
            let value = state.get_reg(value_reg).as_i64() as u8;
            let size = state.get_reg(size_reg).as_i64() as usize;
            if !ptr.is_null() && size > 0 {
                unsafe {
                    std::ptr::write_bytes(ptr, value, size);
                }
            }
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::MemGetAttribute) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Graph API - with capture tracking
        // ================================================================
        Some(GpuSubOpcode::GraphCreate) => {
            let dst = read_reg(state)?;
            // Return a graph handle (index into graph_ops log)
            let graph_id = state.gpu_context.graph_ops.len() as i64;
            state.set_reg(dst, Value::from_i64(graph_id));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GraphInstantiate) => {
            let dst = read_reg(state)?;
            let _graph = read_reg(state)?;
            // Return a graph exec handle
            let graph_exec_id = state.gpu_context.graph_ops.len() as i64;
            state.set_reg(dst, Value::from_i64(graph_exec_id));
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GraphBeginCapture) => {
            let dst = read_reg(state)?;
            let _stream = read_reg(state)?;
            state.gpu_context.graph_capturing = true;
            state.gpu_context.graph_ops.push((0, "begin_capture".to_string()));
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GraphEndCapture) => {
            let dst = read_reg(state)?;
            let _stream = read_reg(state)?;
            state.gpu_context.graph_capturing = false;
            state.gpu_context.graph_ops.push((0, "end_capture".to_string()));
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GraphLaunch) => {
            let dst = read_reg(state)?;
            let _graph_exec = read_reg(state)?;
            let _stream = read_reg(state)?;
            state.gpu_context.graph_ops.push((0, "graph_launch".to_string()));
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GraphDestroy) | Some(GpuSubOpcode::GraphExecDestroy) => {
            let dst = read_reg(state)?;
            let _graph = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GraphExecUpdate) => {
            let dst = read_reg(state)?;
            let _graph_exec = read_reg(state)?;
            let _graph = read_reg(state)?;
            state.gpu_context.graph_ops.push((0, "graph_exec_update".to_string()));
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Profiling - Stubs
        // ================================================================
        Some(GpuSubOpcode::ProfileRangeStart) => {
            let dst = read_reg(state)?;
            let _name = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::ProfileRangeEnd) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::ProfileMarkerPush) => {
            let dst = read_reg(state)?;
            let _name = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::ProfileMarkerPop) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Kernel Launch - CPU Fallback with Thread Model Simulation
        // ================================================================
        //
        // GPU kernel launch executes the kernel function for every thread
        // in the grid. Threads execute sequentially on the CPU, with each
        // thread having access to its identity (threadIdx, blockIdx) via
        // the gpu_thread_ctx field in InterpreterState.
        //
        // Shared memory is allocated per-block and shared across threads.
        // __syncthreads() is a no-op since threads execute in order.
        Some(GpuSubOpcode::Launch) | Some(GpuSubOpcode::LaunchCooperative) => {
            use super::super::super::gpu_simulator::{GpuThreadContext, SharedMemoryBlock, KernelLaunchParams};

            // Read kernel_id (varint)
            let kernel_id = read_varint(state)? as u32;
            // Read grid dimensions (3 registers)
            let grid_x_reg = read_reg(state)?;
            let grid_y_reg = read_reg(state)?;
            let grid_z_reg = read_reg(state)?;
            // Read block dimensions (3 registers)
            let block_x_reg = read_reg(state)?;
            let block_y_reg = read_reg(state)?;
            let block_z_reg = read_reg(state)?;
            // Read shared memory size and stream registers
            let shared_mem_reg = read_reg(state)?;
            let _stream = read_reg(state)?;
            // Read argument registers (varint count + regs)
            let arg_count = read_varint(state)? as usize;
            let mut arg_regs = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                arg_regs.push(read_reg(state)?);
            }

            let caller_base = state.reg_base();

            // Read launch parameters from registers
            let grid_dim = [
                state.registers.get(caller_base, grid_x_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, grid_y_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, grid_z_reg).as_i64().max(1) as u32,
            ];
            let block_dim = [
                state.registers.get(caller_base, block_x_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, block_y_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, block_z_reg).as_i64().max(1) as u32,
            ];
            let shared_mem_size = state.registers.get(caller_base, shared_mem_reg).as_i64().max(0) as usize;

            // Collect argument values before modifying state
            let mut arg_values = Vec::with_capacity(arg_count);
            for &reg in &arg_regs {
                arg_values.push(state.registers.get(caller_base, reg));
            }

            let func_id = FunctionId(kernel_id);
            let func = match state.module.get_function(func_id) {
                Some(f) => f,
                None => return Ok(DispatchResult::Continue),
            };
            let reg_count = func.register_count;

            let params = KernelLaunchParams {
                kernel_id,
                grid_dim,
                block_dim,
                shared_mem_size,
                args: arg_values.iter().map(|v| v.as_i64()).collect(),
            };

            // Save PC after reading all launch operands — this is the continuation point
            let return_pc = state.pc();

            // Save previous GPU thread context (for nested launches)
            let prev_gpu_ctx = state.gpu_thread_ctx.take();
            let prev_shared_mem = state.gpu_shared_memory.take();
            let prev_shared_offset = state.gpu_shared_mem_offset;

            // Iterate over all blocks in the grid
            let mut current_block: Option<[u32; 3]> = None;
            for (block_id, thread_id) in params.thread_iter() {
                // Allocate new shared memory when entering a new block
                if current_block.as_ref() != Some(&block_id) {
                    current_block = Some(block_id);
                    state.gpu_shared_memory = Some(SharedMemoryBlock::new(shared_mem_size));
                    state.gpu_shared_mem_offset = 0;
                }

                // Set thread context for this thread
                let smem = match state.gpu_shared_memory.as_ref() {
                    Some(m) => m,
                    None => return Err(InterpreterError::InvalidOperand {
                        message: "GPU shared memory not initialized for kernel launch".to_string(),
                    }),
                };
                state.gpu_thread_ctx = Some(GpuThreadContext::new(
                    thread_id, block_id, block_dim, grid_dim, smem,
                ));

                // Push a new frame for the kernel function
                let entry_depth = state.call_stack.depth();
                let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, Reg(0))?;
                state.registers.push_frame(reg_count);

                // Copy arguments to callee registers
                for (i, val) in arg_values.iter().enumerate() {
                    state.registers.set(new_base, Reg(i as u16), *val);
                }

                // Set PC to start of kernel function
                state.set_pc(0);
                state.record_call();

                // Execute kernel function to completion using nested dispatch loop
                let _result = dispatch_loop_table_with_entry_depth(state, entry_depth)?;
            }

            // Restore previous GPU context (for nested launches)
            state.gpu_thread_ctx = prev_gpu_ctx;
            state.gpu_shared_memory = prev_shared_mem;
            state.gpu_shared_mem_offset = prev_shared_offset;

            // Restore PC to continuation point after launch
            // PC is already restored by frame pop in dispatch_loop_table_with_entry_depth
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::LaunchMultiDevice) => {
            use super::super::super::gpu_simulator::{GpuThreadContext, SharedMemoryBlock, KernelLaunchParams};

            // Read kernel_id (varint)
            let kernel_id = read_varint(state)? as u32;
            // Read devices register (ignored for CPU fallback)
            let _devices = read_reg(state)?;
            // Read grid dimensions (3 registers)
            let grid_x_reg = read_reg(state)?;
            let grid_y_reg = read_reg(state)?;
            let grid_z_reg = read_reg(state)?;
            // Read block dimensions (3 registers)
            let block_x_reg = read_reg(state)?;
            let block_y_reg = read_reg(state)?;
            let block_z_reg = read_reg(state)?;
            // Read shared memory size register
            let shared_mem_reg = read_reg(state)?;
            // Read argument registers (varint count + regs)
            let arg_count = read_varint(state)? as usize;
            let mut arg_regs = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                arg_regs.push(read_reg(state)?);
            }

            let caller_base = state.reg_base();

            let grid_dim = [
                state.registers.get(caller_base, grid_x_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, grid_y_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, grid_z_reg).as_i64().max(1) as u32,
            ];
            let block_dim = [
                state.registers.get(caller_base, block_x_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, block_y_reg).as_i64().max(1) as u32,
                state.registers.get(caller_base, block_z_reg).as_i64().max(1) as u32,
            ];
            let shared_mem_size = state.registers.get(caller_base, shared_mem_reg).as_i64().max(0) as usize;

            let mut arg_values = Vec::with_capacity(arg_count);
            for &reg in &arg_regs {
                arg_values.push(state.registers.get(caller_base, reg));
            }

            let func_id = FunctionId(kernel_id);
            let func = match state.module.get_function(func_id) {
                Some(f) => f,
                None => return Ok(DispatchResult::Continue),
            };
            let reg_count = func.register_count;

            let params = KernelLaunchParams {
                kernel_id,
                grid_dim,
                block_dim,
                shared_mem_size,
                args: arg_values.iter().map(|v| v.as_i64()).collect(),
            };

            let return_pc = state.pc();
            let prev_gpu_ctx = state.gpu_thread_ctx.take();
            let prev_shared_mem = state.gpu_shared_memory.take();
            let prev_shared_offset = state.gpu_shared_mem_offset;

            let mut current_block: Option<[u32; 3]> = None;
            for (block_id, thread_id) in params.thread_iter() {
                if current_block.as_ref() != Some(&block_id) {
                    current_block = Some(block_id);
                    state.gpu_shared_memory = Some(SharedMemoryBlock::new(shared_mem_size));
                    state.gpu_shared_mem_offset = 0;
                }

                let smem = match state.gpu_shared_memory.as_ref() {
                    Some(m) => m,
                    None => return Err(InterpreterError::InvalidOperand {
                        message: "GPU shared memory not initialized for kernel launch".to_string(),
                    }),
                };
                state.gpu_thread_ctx = Some(GpuThreadContext::new(
                    thread_id, block_id, block_dim, grid_dim, smem,
                ));

                let entry_depth = state.call_stack.depth();
                let new_base = state.call_stack.push_frame(func_id, reg_count, return_pc, Reg(0))?;
                state.registers.push_frame(reg_count);

                for (i, val) in arg_values.iter().enumerate() {
                    state.registers.set(new_base, Reg(i as u16), *val);
                }

                state.set_pc(0);
                state.record_call();
                let _result = dispatch_loop_table_with_entry_depth(state, entry_depth)?;
            }

            state.gpu_thread_ctx = prev_gpu_ctx;
            state.gpu_shared_memory = prev_shared_mem;
            state.gpu_shared_mem_offset = prev_shared_offset;
            // PC is already restored by frame pop in dispatch_loop_table_with_entry_depth
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Thread Intrinsics (0xA0-0xAF)
        // ================================================================
        Some(GpuSubOpcode::ThreadIdX) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(0, |ctx| ctx.thread_id[0] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::ThreadIdY) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(0, |ctx| ctx.thread_id[1] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::ThreadIdZ) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(0, |ctx| ctx.thread_id[2] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::BlockIdX) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(0, |ctx| ctx.block_id[0] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::BlockIdY) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(0, |ctx| ctx.block_id[1] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::BlockIdZ) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(0, |ctx| ctx.block_id[2] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::BlockDimX) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(1, |ctx| ctx.block_dim[0] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::BlockDimY) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(1, |ctx| ctx.block_dim[1] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::BlockDimZ) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(1, |ctx| ctx.block_dim[2] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::GridDimX) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(1, |ctx| ctx.grid_dim[0] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::GridDimY) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(1, |ctx| ctx.grid_dim[1] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::GridDimZ) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(1, |ctx| ctx.grid_dim[2] as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SyncThreads) => {
            // No-op in CPU fallback: threads execute sequentially within a block.
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SyncWarp) => {
            // Read optional mask register
            let _mask = read_reg(state)?;
            // No-op in CPU fallback.
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::WarpSize) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(32));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::LinearThreadId) => {
            let dst = read_reg(state)?;
            let val = state.gpu_thread_ctx.as_ref().map_or(0, |ctx| ctx.linear_thread_id() as i64);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Shared Memory Operations (0xB0-0xBF)
        // ================================================================
        Some(GpuSubOpcode::SharedMemAlloc) => {
            let dst = read_reg(state)?;
            let size_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let size = state.registers.get(caller_base, size_reg).as_i64() as usize;
            let offset = state.gpu_shared_mem_offset;
            state.gpu_shared_mem_offset += size;
            state.set_reg(dst, Value::from_i64(offset as i64));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemLoadI64) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let val = state.gpu_shared_memory.as_ref()
                .and_then(|smem| smem.read_i64(offset))
                .unwrap_or(0);
            state.set_reg(dst, Value::from_i64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemStoreI64) => {
            let offset_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let value = state.registers.get(caller_base, value_reg).as_i64();
            if let Some(smem) = state.gpu_shared_memory.as_mut() {
                smem.write_i64(offset, value);
            }
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemLoadF64) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let val = state.gpu_shared_memory.as_ref()
                .and_then(|smem| smem.read_f64(offset))
                .unwrap_or(0.0);
            state.set_reg(dst, Value::from_f64(val));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemStoreF64) => {
            let offset_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let value = state.registers.get(caller_base, value_reg).as_f64();
            if let Some(smem) = state.gpu_shared_memory.as_mut() {
                smem.write_f64(offset, value);
            }
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemAtomicAddI64) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let value = state.registers.get(caller_base, value_reg).as_i64();
            let old = state.gpu_shared_memory.as_mut()
                .and_then(|smem| smem.atomic_add_i64(offset, value))
                .unwrap_or(0);
            state.set_reg(dst, Value::from_i64(old));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemAtomicAddF64) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let value = state.registers.get(caller_base, value_reg).as_f64();
            let old = state.gpu_shared_memory.as_mut()
                .and_then(|smem| smem.atomic_add_f64(offset, value))
                .unwrap_or(0.0);
            state.set_reg(dst, Value::from_f64(old));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemAtomicCasI64) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let expected_reg = read_reg(state)?;
            let desired_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let expected = state.registers.get(caller_base, expected_reg).as_i64();
            let desired = state.registers.get(caller_base, desired_reg).as_i64();
            let old = state.gpu_shared_memory.as_mut()
                .and_then(|smem| smem.atomic_cas_i64(offset, expected, desired))
                .unwrap_or(0);
            state.set_reg(dst, Value::from_i64(old));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemAtomicMaxI64) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let value = state.registers.get(caller_base, value_reg).as_i64();
            let old = state.gpu_shared_memory.as_mut()
                .and_then(|smem| smem.atomic_max_i64(offset, value))
                .unwrap_or(0);
            state.set_reg(dst, Value::from_i64(old));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemAtomicMinI64) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let value = state.registers.get(caller_base, value_reg).as_i64();
            let old = state.gpu_shared_memory.as_mut()
                .and_then(|smem| smem.atomic_min_i64(offset, value))
                .unwrap_or(0);
            state.set_reg(dst, Value::from_i64(old));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemLoadU32) => {
            let dst = read_reg(state)?;
            let offset_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let val = state.gpu_shared_memory.as_ref()
                .and_then(|smem| smem.read_u32(offset))
                .unwrap_or(0);
            state.set_reg(dst, Value::from_i64(val as i64));
            Ok(DispatchResult::Continue)
        }
        Some(GpuSubOpcode::SharedMemStoreU32) => {
            let offset_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let caller_base = state.reg_base();
            let offset = state.registers.get(caller_base, offset_reg).as_i64() as usize;
            let value = state.registers.get(caller_base, value_reg).as_i64() as u32;
            if let Some(smem) = state.gpu_shared_memory.as_mut() {
                smem.write_u32(offset, value);
            }
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Peer Access - Stubs
        // ================================================================
        Some(GpuSubOpcode::CanAccessPeer) => {
            let dst = read_reg(state)?;
            let _src_device = read_reg(state)?;
            let _peer_device = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // no peer access in interpreter
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::EnablePeerAccess) | Some(GpuSubOpcode::DisablePeerAccess) => {
            let dst = read_reg(state)?;
            let _src_device = read_reg(state)?;
            let _peer_device = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0)); // success
            Ok(DispatchResult::Continue)
        }

        Some(GpuSubOpcode::GetDeviceProperty) | Some(GpuSubOpcode::SetDeviceFlags) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        None => {
            Err(InterpreterError::NotImplemented {
                feature: "unknown GPU sub-opcode",
                opcode: Some(crate::instruction::Opcode::GpuExtended),
            })
        }
    }
}

// ============================================================================

/// Handler for GpuSync opcode (0xF9).
/// Synchronizes a GPU stream. No-op in interpreter (everything is synchronous).
///
/// Format: `stream:reg`
pub(in super::super) fn handle_gpu_sync(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let _stream = read_reg(state)?;
    // No-op in interpreter - all operations are synchronous
    Ok(DispatchResult::Continue)
}

/// Handler for GpuMemcpy opcode (0xFA).
/// GPU memory copy with direction. Falls back to CPU memcpy in interpreter.
///
/// Format: `dst:reg, src:reg, direction:u8`
pub(in super::super) fn handle_gpu_memcpy(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst_reg = read_reg(state)?;
    let src_reg = read_reg(state)?;
    let _direction = read_u8(state)?; // 0=H2D, 1=D2H, 2=D2D - ignored, all CPU memory

    // In interpreter, dst and src are tensor handles - just copy reference
    let src_val = state.get_reg(src_reg);
    state.set_reg(dst_reg, src_val);
    Ok(DispatchResult::Continue)
}

/// Handler for GpuAlloc opcode (0xFB).
/// GPU memory allocation. Falls back to CPU heap allocation in interpreter.
///
/// Format: `dst:reg, size:reg, device:reg`
pub(in super::super) fn handle_gpu_alloc(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let size_reg = read_reg(state)?;
    let _device_reg = read_reg(state)?; // Ignored in CPU fallback

    let size = state.get_reg(size_reg).as_i64() as usize;
    let alloc_size = if size == 0 { 1 } else { size };
    // Layout-construction failure means the requested size exceeds
    // isize::MAX once aligned.  Treat as null pointer (allocation
    // failure) — never silently downgrade to a 1-byte layout, since
    // that would lie about the size to the caller and produce a
    // heap overflow on the first write.
    let layout = match std::alloc::Layout::from_size_align(alloc_size, 8) {
        Ok(l) => l,
        Err(_) => {
            state.set_reg(dst, Value::from_ptr::<u8>(std::ptr::null_mut()));
            return Ok(DispatchResult::Continue);
        }
    };
    let ptr = unsafe { std::alloc::alloc(layout) };
    if !ptr.is_null() {
        state.gpu_context.allocated_buffers.insert(ptr as usize, alloc_size);
    }
    state.set_reg(dst, Value::from_ptr(ptr));
    Ok(DispatchResult::Continue)
}
