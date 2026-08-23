//! `Opcode::SysExtended` — the sys family's honest home (T0852).
//! These arms lived in `ffi_extended.rs` as `SystemSubOpcode` squatters;
//! the operations have nothing to do with FFI.
//!
//! Wire: standard extended envelope via `dispatch_enveloped`.

use super::super::DispatchResult;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::bytecode_io::read_reg;
use super::envelope::dispatch_enveloped;
use crate::instruction::{Opcode, SysSubOpcode};
use super::ffi_extended::{extract_filedesc, extract_mapflags, extract_memprot_flags, get_platform_errno, make_oserror_variant, make_oserror_variant_with_msg, make_result_ok_ptr, make_result_ok_unit};
use crate::interpreter::error::InterpreterError;
use crate::value::Value;

pub(in super::super) fn handle_sys_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, sys_extended_body)
}

fn sys_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    match SysSubOpcode::from_byte(sub_op_byte) {
        Some(SysSubOpcode::GetPid) => {
            let dst = read_reg(state)?;
            let pid = std::process::id();
            state.set_reg(dst, Value::from_i64(pid as i64));
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::GetTid) => {
            let dst = read_reg(state)?;
            #[cfg(unix)]
            let tid: u64 = {
                // On macOS the 0 init is the out-param seed; elsewhere the
                // binding is assigned exactly once below (the seed would be
                // 'never read' under -D warnings on Linux builders).
                #[cfg(target_os = "macos")]
                let mut tid: u64 = 0;
                #[cfg(not(target_os = "macos"))]
                let tid: u64;
                // SAFETY: `tid` is a live stack u64. `pthread_threadid_np` writes
                // exactly one u64 via the provided pointer when the first arg is
                // 0 (self). The Apple libc contract is well-defined.
                #[cfg(target_os = "macos")]
                unsafe {
                    libc::pthread_threadid_np(0, &mut tid);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // On other Unix, use the thread id as a hash of the thread handle
                    let id = std::thread::current().id();
                    tid = format!("{:?}", id)
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0);
                }
                tid
            };
            #[cfg(windows)]
            let tid: u64 = {
                // SAFETY: GetCurrentThreadId is always safe and takes no pointer arguments.
                unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() as u64 }
            };
            #[cfg(not(any(unix, windows)))]
            let tid: u64 = 0;
            state.set_reg(dst, Value::from_i64(tid as i64));
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::Mmap) => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;
            let prot_reg = read_reg(state)?;
            let flags_reg = read_reg(state)?;
            let fd_reg = read_reg(state)?;
            let offset_reg = read_reg(state)?;

            let addr = state.get_reg(addr_reg).as_i64();
            let len = state.get_reg(len_reg).as_i64();
            let _offset = state.get_reg(offset_reg).as_i64();

            // Extract prot flags from MemProt struct object
            // MemProt { read: Bool, write: Bool, exec: Bool }
            let prot_val = state.get_reg(prot_reg);
            let prot_flags = extract_memprot_flags(state, prot_val);

            // Extract map flags from MapFlags struct object
            // MapFlags { shared: Bool, is_private: Bool, anonymous: Bool, fixed: Bool }
            let flags_val = state.get_reg(flags_reg);
            let map_flags = extract_mapflags(state, flags_val);

            // Extract fd from FileDesc newtype (Int)
            let fd_val = state.get_reg(fd_reg);
            let fd = extract_filedesc(state, fd_val);

            #[cfg(unix)]
            {
                let offset = _offset;
                // SAFETY: `mmap` is a well-defined kernel syscall. The caller
                // supplies the same arguments the AOT path would; invalid inputs
                // return `MAP_FAILED` without corrupting our process state. No
                // Rust references are dereferenced here.
                let result = unsafe {
                    libc::mmap(
                        addr as *mut libc::c_void,
                        len as libc::size_t,
                        prot_flags,
                        map_flags,
                        fd,
                        offset as libc::off_t,
                    )
                };

                if result == libc::MAP_FAILED {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_ptr(state, result as i64)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(windows)]
            {
                let _ = (fd, map_flags);
                // Translate MemProt flags to Windows page protection constants
                let win_prot = memprot_to_win_protect(prot_flags);
                let alloc_type = 0x00001000u32 | 0x00002000u32; // MEM_COMMIT | MEM_RESERVE
                // SAFETY: VirtualAlloc is a well-defined Win32 API. Invalid inputs
                // return NULL without corrupting process state.
                let result = unsafe {
                    windows_sys::Win32::System::Memory::VirtualAlloc(
                        if addr == 0 {
                            std::ptr::null()
                        } else {
                            addr as *const core::ffi::c_void
                        },
                        len as usize,
                        alloc_type,
                        win_prot,
                    )
                };
                if result.is_null() {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_ptr(state, result as i64)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(not(any(unix, windows)))]
            {
                let err_obj = make_oserror_variant_with_msg(
                    state,
                    38,
                    "mmap not supported on this platform",
                )?;
                state.set_reg(dst, err_obj);
            }

            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::Munmap) => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;

            let addr = state.get_reg(addr_reg).as_i64();
            let len = state.get_reg(len_reg).as_i64();

            #[cfg(unix)]
            {
                // SAFETY: `munmap` is a well-defined kernel syscall that fails
                // with a negative result on invalid inputs. No Rust references
                // are dereferenced; correctness is the caller's responsibility.
                let result =
                    unsafe { libc::munmap(addr as *mut libc::c_void, len as libc::size_t) };

                if result < 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(windows)]
            {
                let _ = len;
                // SAFETY: VirtualFree with MEM_RELEASE (0x00008000) is well-defined.
                // The size parameter must be 0 when using MEM_RELEASE.
                let result = unsafe {
                    windows_sys::Win32::System::Memory::VirtualFree(
                        addr as *mut core::ffi::c_void,
                        0,
                        0x00008000u32, // MEM_RELEASE
                    )
                };
                if result == 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(not(any(unix, windows)))]
            {
                let _ = (addr, len);
                let err_obj = make_oserror_variant_with_msg(
                    state,
                    38,
                    "munmap not supported on this platform",
                )?;
                state.set_reg(dst, err_obj);
            }

            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::Madvise) => {
            let dst = read_reg(state)?;
            let addr_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;
            let advice_reg = read_reg(state)?;

            let _addr = state.get_reg(addr_reg).as_i64();
            let _len = state.get_reg(len_reg).as_i64();
            let _advice = state.get_reg(advice_reg).as_i64();

            #[cfg(unix)]
            {
                // SAFETY: `madvise` is a kernel syscall that validates the
                // supplied address range and returns `-1` on invalid input.
                let result = unsafe {
                    libc::madvise(
                        _addr as *mut libc::c_void,
                        _len as libc::size_t,
                        _advice as i32,
                    )
                };

                if result < 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }

            #[cfg(not(unix))]
            {
                // madvise is advisory-only; no-op on Windows and other platforms
                let ok_obj = make_result_ok_unit(state)?;
                state.set_reg(dst, ok_obj);
            }

            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::GetEntropy) => {
            let dst = read_reg(state)?;
            let buf_reg = read_reg(state)?;
            let len_reg = read_reg(state)?;

            // `buf` arrives as a Verum `&unsafe Byte` reference whose
            // runtime representation is a Pointer-tagged Value (the
            // common case — `tail.as_mut_ptr() as &unsafe Byte`). A
            // small minority of callers fabricate a raw address as
            // an Int and cast — handle that too. `as_i64()` on a
            // Pointer-tagged value extracts the sign-extended payload
            // bits, which silently misreads high-address pointers as
            // negative offsets, making the draw fault with EFAULT and
            // the whole CSPRNG chain return Err.
            let buf_val = state.get_reg(buf_reg);
            let buf = if buf_val.is_ptr() {
                buf_val.as_ptr::<u8>() as usize as u64
            } else {
                buf_val.as_i64() as u64
            };
            let len = state.get_reg(len_reg).as_i64();

            if len > 256 {
                // getentropy has a 256-byte limit
                let err_obj = make_oserror_variant_with_msg(state, 5, "getentropy: max 256 bytes")?;
                state.set_reg(dst, err_obj);
            } else {
                // ONE entropy implementation (crate::entropy). This
                // shim keeps the REPORTING contract — a syscall shim
                // relays the kernel's answer — while
                // `entropy::fill_secure` carries the abort contract
                // the language needs.
                //
                // SAFETY: `len` was bounded to <= 256 above and `buf`
                // is the caller's address; building a slice over it is
                // the same trust boundary the direct call had, and the
                // kernel still validates the pointer (EFAULT).
                let result: i32 = {
                    let slice =
                        unsafe { std::slice::from_raw_parts_mut(buf as *mut u8, len as usize) };
                    match crate::entropy::try_fill_secure(slice) {
                        Ok(()) => 0,
                        Err(_) => -1,
                    }
                };

                if result < 0 {
                    let errno = get_platform_errno();
                    let err_obj = make_oserror_variant(state, errno)?;
                    state.set_reg(dst, err_obj);
                } else {
                    let ok_obj = make_result_ok_unit(state)?;
                    state.set_reg(dst, ok_obj);
                }
            }
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::ExecutionTier) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(0));
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::IsInterpreted) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_bool(true));
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::EnvGet) => {
            let dst = read_reg(state)?;
            let name_reg = read_reg(state)?;
            let base = state.reg_base();
            let name = super::file_runtime::extract_byte_list_arg(state, name_reg.0, base);
            let key = String::from_utf8_lossy(&name).into_owned();
            let value = super::env_runtime::env_get_maybe(state, &key)?
                .expect("env_get_maybe always yields a Maybe value");
            state.set_reg(dst, value);
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::EnvSet) => {
            let dst = read_reg(state)?;
            let name_reg = read_reg(state)?;
            let value_reg = read_reg(state)?;
            let base = state.reg_base();
            let name = super::file_runtime::extract_byte_list_arg(state, name_reg.0, base);
            let val = super::file_runtime::extract_byte_list_arg(state, value_reg.0, base);
            let key = String::from_utf8_lossy(&name).into_owned();
            let value = String::from_utf8_lossy(&val).into_owned();
            super::env_runtime::env_set_raw(state, &key, &value);
            let ok = super::env_runtime::env_unit_ok(state)?;
            state.set_reg(dst, ok);
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::EnvUnset) => {
            let dst = read_reg(state)?;
            let name_reg = read_reg(state)?;
            let base = state.reg_base();
            let name = super::file_runtime::extract_byte_list_arg(state, name_reg.0, base);
            let key = String::from_utf8_lossy(&name).into_owned();
            super::env_runtime::env_unset_raw(state, &key);
            let ok = super::env_runtime::env_unit_ok(state)?;
            state.set_reg(dst, ok);
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::RandomU64) => {
            // Format: dst:reg
            let dst = read_reg(state)?;
            // ONE entropy authority (crate::entropy): every return
            // code is checked, and it aborts rather than yielding
            // predictable bytes. The per-platform draw used to be
            // inline here AND in RandomFloat, discarding the syscall
            // result in both.
            let random_value = crate::entropy::secure_random_u64();
            state.set_reg(dst, Value::from_i64(random_value as i64));
            Ok(DispatchResult::Continue)
        }

        Some(SysSubOpcode::RandomFloat) => {
            // Format: dst:reg
            let dst = read_reg(state)?;
            let random_u64 = crate::entropy::secure_random_u64();
            // IEEE 754 conversion: (bits >> 11) * (1.0 / 2^53) — the
            // 53 bits a double represents exactly, so every
            // representable value in [0, 1) is equally likely.
            let float_value = (random_u64 >> 11) as f64 * (1.0 / ((1u64 << 53) as f64));
            state.set_reg(dst, Value::from_f64(float_value));
            Ok(DispatchResult::Continue)
        }

        _ => Err(InterpreterError::NotImplemented {
            feature: "sys_extended sub-opcode",
            opcode: Some(Opcode::SysExtended),
        }),
    }
}
