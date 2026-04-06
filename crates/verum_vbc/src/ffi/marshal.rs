//! Type marshalling between Verum values and C types.
//!
//! This module handles the conversion of VBC `Value` types to/from
//! raw C types for FFI calls.
//!
//! # Marshalling Rules
//!
//! | Verum Type | C Type | Notes |
//! |------------|--------|-------|
//! | Int | i8/i16/i32/i64 | Sign extension for smaller types |
//! | Int | u8/u16/u32/u64 | Zero extension for smaller types |
//! | Float | f32/f64 | IEEE 754 conversion |
//! | Bool | _Bool | 0 or 1 |
//! | Ptr | void* | Raw pointer passthrough |
//! | Text | const char* | Null-terminated UTF-8 |

use std::ffi::CString;
use std::fmt;

use super::CTypeRuntime;
use crate::value::Value;

/// Error type for marshalling operations.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum MarshalError {
    /// Type conversion not supported.
    UnsupportedConversion {
        from: &'static str,
        to: CTypeRuntime,
    },
    /// Value out of range for target type.
    ValueOutOfRange {
        value: String,
        target: CTypeRuntime,
    },
    /// Invalid string (contains null bytes).
    InvalidString(String),
    /// Invalid pointer.
    InvalidPointer,
    /// Null pointer where non-null required.
    NullPointer,
}

impl fmt::Display for MarshalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarshalError::UnsupportedConversion { from, to } => {
                write!(f, "unsupported conversion from {} to {:?}", from, to)
            }
            MarshalError::ValueOutOfRange { value, target } => {
                write!(f, "value {} out of range for {:?}", value, target)
            }
            MarshalError::InvalidString(s) => {
                write!(f, "invalid string for C conversion: {}", s)
            }
            MarshalError::InvalidPointer => write!(f, "invalid pointer"),
            MarshalError::NullPointer => write!(f, "null pointer where non-null required"),
        }
    }
}

impl std::error::Error for MarshalError {}

/// Storage for a reference argument passed to FFI.
///
/// When a Verum value (like an Int) needs to be passed by reference to FFI,
/// we allocate temporary storage, store the value there, and pass the address.
/// After the FFI call, for mutable references, we read back the value.
#[derive(Debug)]
pub struct RefArgStorage {
    /// The allocated storage for the value.
    /// Using Box to get a stable address on the heap.
    storage: Box<u64>,
    /// The register index for write-back (if mutable).
    /// None for immutable references.
    pub write_back_reg: Option<u16>,
}

impl RefArgStorage {
    /// Creates new ref arg storage with an initial value.
    pub fn new(value: u64, write_back_reg: Option<u16>) -> Self {
        Self {
            storage: Box::new(value),
            write_back_reg,
        }
    }

    /// Gets the address of the storage.
    pub fn as_ptr(&self) -> *const u64 {
        &*self.storage as *const u64
    }

    /// Gets a mutable address of the storage.
    pub fn as_mut_ptr(&mut self) -> *mut u64 {
        &mut *self.storage as *mut u64
    }

    /// Reads the current value from storage.
    pub fn read(&self) -> u64 {
        *self.storage
    }
}

/// Information about a marshalled array buffer.
///
/// When arrays are marshalled from VBC Values to C data, we track the buffer
/// for cleanup and potential write-back of mutable references.
#[derive(Debug, Clone)]
pub struct ArrayBufferInfo {
    /// Pointer to the marshalled C data buffer.
    pub buffer: *mut u8,
    /// Size of the buffer in bytes.
    pub buffer_size: usize,
    /// Pointer to the original VBC array.
    pub array_ptr: *const u8,
    /// Number of elements in the array.
    pub array_len: usize,
    /// Element type tag (0x01=i8, 0x02=i16, 0x03=i32, 0x04=i64, 0x05=f32, 0x06=f64, 0x07=ptr).
    pub element_type: u8,
    /// If true, write back changes after FFI call.
    pub is_mutable: bool,
}

/// Marshaller for converting between Verum values and C types.
///
/// The marshaller handles all type conversions for FFI calls,
/// including string allocation and lifetime management.
pub struct Marshaller {
    /// Cached CStrings for string arguments (kept alive during calls).
    string_cache: Vec<CString>,
    /// Temporary storage for reference arguments.
    /// These are kept alive during the FFI call and can be read back afterward.
    ref_arg_storage: Vec<RefArgStorage>,
    /// Tracked array buffers for cleanup and write-back.
    array_buffers: Vec<ArrayBufferInfo>,
}

impl Marshaller {
    /// Creates a new marshaller.
    pub fn new() -> Self {
        Self {
            string_cache: Vec::new(),
            ref_arg_storage: Vec::new(),
            array_buffers: Vec::new(),
        }
    }

    /// Clears all cached data.
    ///
    /// Call this after an FFI call completes to free cached strings and ref storage.
    pub fn clear_cache(&mut self) {
        self.string_cache.clear();
        self.ref_arg_storage.clear();
        // Note: array_buffers are NOT cleared here - they need explicit cleanup
        // to handle write-back for mutable references
    }

    /// Tracks an array buffer for FFI marshalling.
    ///
    /// The buffer will be cleaned up when `cleanup_array_buffers` is called.
    pub fn track_array_buffer(&mut self, info: ArrayBufferInfo) {
        self.array_buffers.push(info);
    }

    /// Cleans up array buffers, optionally writing back mutable ones.
    ///
    /// For mutable array references, this converts the C data back to VBC Values
    /// and writes them to the original array. All buffers are then freed.
    ///
    /// # Safety
    ///
    /// The array_ptr in each ArrayBufferInfo must still be valid, and the buffer
    /// must not have been freed previously.
    pub unsafe fn cleanup_array_buffers(&mut self) {
        // SAFETY: Caller guarantees all array_ptr values are valid
        unsafe {
            for info in self.array_buffers.drain(..) {
                // For mutable references, write back the modified data
                if info.is_mutable && !info.array_ptr.is_null() {
                    // Calculate header size and data pointer
                    const OBJECT_HEADER_SIZE: usize = 24; // from heap.rs
                    let data_start = info.array_ptr.add(OBJECT_HEADER_SIZE) as *mut Value;

                    // Determine element size in C buffer
                    let c_elem_size = match info.element_type {
                        0x01 => 1,  // i8
                        0x02 => 2,  // i16
                        0x03 => 4,  // i32
                        0x04 => 8,  // i64
                        0x05 => 4,  // f32
                        0x06 => 8,  // f64
                        0x07 => 8,  // ptr
                        _ => 8,     // default to i64 size
                    };

                    // Convert each C element back to a Value
                    for i in 0..info.array_len {
                        let src_ptr = info.buffer.add(i * c_elem_size);
                        let val = match info.element_type {
                            0x01 => Value::from_i64(*(src_ptr as *const i8) as i64),
                            0x02 => Value::from_i64(*(src_ptr as *const i16) as i64),
                            0x03 => Value::from_i64(*(src_ptr as *const i32) as i64),
                            0x04 => Value::from_i64(*(src_ptr as *const i64)),
                            0x05 => Value::from_f64(*(src_ptr as *const f32) as f64),
                            0x06 => Value::from_f64(*(src_ptr as *const f64)),
                            0x07 => Value::from_ptr(*(src_ptr as *const *mut u8)),
                            _ => Value::from_i64(*(src_ptr as *const i64)),
                        };
                        *data_start.add(i) = val;
                    }
                }

                // Free the buffer
                if !info.buffer.is_null() && info.buffer_size > 0 {
                    let layout = std::alloc::Layout::from_size_align(info.buffer_size, 8)
                        .expect("invalid buffer layout");
                    std::alloc::dealloc(info.buffer, layout);
                }
            }
        }
    }

    /// Returns an iterator over ref arg storage for write-back.
    pub fn ref_arg_storage(&self) -> impl Iterator<Item = &RefArgStorage> {
        self.ref_arg_storage.iter()
    }

    /// Converts a Verum Value to a raw C value, allocating temporary storage for references.
    ///
    /// For pointer arguments, this creates temporary storage for the value and returns
    /// the address of that storage. Use `write_back_reg` to indicate the register
    /// to write back to after the FFI call (for mutable references).
    pub fn value_to_c_ref(
        &mut self,
        value: Value,
        target: CTypeRuntime,
        write_back_reg: Option<u16>,
    ) -> Result<u64, MarshalError> {
        // For pointer types, allocate temporary storage for non-pointer values
        if matches!(target, CTypeRuntime::Ptr | CTypeRuntime::StructPtr(_) | CTypeRuntime::ArrayPtr) {
            // If it's already a pointer, just pass it through
            if value.is_ptr() {
                return Ok(value.as_ptr::<u8>() as u64);
            }

            // For integers (the common case), allocate storage
            if value.is_int() {
                let i = value.as_i64();
                let storage = RefArgStorage::new(i as u64, write_back_reg);
                let ptr = storage.as_ptr() as u64;
                self.ref_arg_storage.push(storage);
                return Ok(ptr);
            }

            // For floats passed by reference
            if value.is_float() {
                let f = value.as_f64();
                let storage = RefArgStorage::new(f.to_bits(), write_back_reg);
                let ptr = storage.as_ptr() as u64;
                self.ref_arg_storage.push(storage);
                return Ok(ptr);
            }

            // For bools passed by reference
            if value.is_bool() {
                let b = if value.as_bool() { 1u64 } else { 0u64 };
                let storage = RefArgStorage::new(b, write_back_reg);
                let ptr = storage.as_ptr() as u64;
                self.ref_arg_storage.push(storage);
                return Ok(ptr);
            }

            // Null/unit becomes null pointer
            if value.is_nil() || value.is_unit() {
                return Ok(0);
            }

            return Err(MarshalError::UnsupportedConversion {
                from: value_type_name(&value),
                to: target,
            });
        }

        // For non-pointer types, use the regular conversion
        self.value_to_c(value, target)
    }

    /// Converts a Verum Value to a raw C value.
    ///
    /// Returns the raw bytes that can be passed to libffi.
    pub fn value_to_c(&mut self, value: Value, target: CTypeRuntime) -> Result<u64, MarshalError> {
        // Handle void first
        if target == CTypeRuntime::Void {
            return Ok(0);
        }

        // Integer conversions
        if value.is_int() {
            let i = value.as_i64();
            return match target {
                CTypeRuntime::I8 => {
                    if i < i8::MIN as i64 || i > i8::MAX as i64 {
                        return Err(MarshalError::ValueOutOfRange {
                            value: i.to_string(),
                            target,
                        });
                    }
                    Ok(i as i8 as u64)
                }
                CTypeRuntime::I16 => {
                    if i < i16::MIN as i64 || i > i16::MAX as i64 {
                        return Err(MarshalError::ValueOutOfRange {
                            value: i.to_string(),
                            target,
                        });
                    }
                    Ok(i as i16 as u64)
                }
                CTypeRuntime::I32 => {
                    if i < i32::MIN as i64 || i > i32::MAX as i64 {
                        return Err(MarshalError::ValueOutOfRange {
                            value: i.to_string(),
                            target,
                        });
                    }
                    Ok(i as i32 as u64)
                }
                CTypeRuntime::I64 | CTypeRuntime::Ssize => Ok(i as u64),
                CTypeRuntime::U8 => {
                    if i < 0 || i > u8::MAX as i64 {
                        return Err(MarshalError::ValueOutOfRange {
                            value: i.to_string(),
                            target,
                        });
                    }
                    Ok(i as u8 as u64)
                }
                CTypeRuntime::U16 => {
                    if i < 0 || i > u16::MAX as i64 {
                        return Err(MarshalError::ValueOutOfRange {
                            value: i.to_string(),
                            target,
                        });
                    }
                    Ok(i as u16 as u64)
                }
                CTypeRuntime::U32 => {
                    if i < 0 || i > u32::MAX as i64 {
                        return Err(MarshalError::ValueOutOfRange {
                            value: i.to_string(),
                            target,
                        });
                    }
                    Ok(i as u32 as u64)
                }
                CTypeRuntime::U64 | CTypeRuntime::Size => {
                    if i < 0 {
                        return Err(MarshalError::ValueOutOfRange {
                            value: i.to_string(),
                            target,
                        });
                    }
                    Ok(i as u64)
                }
                CTypeRuntime::F32 => Ok((i as f32).to_bits() as u64),
                CTypeRuntime::F64 => Ok((i as f64).to_bits()),
                CTypeRuntime::Bool => Ok(if i != 0 { 1 } else { 0 }),
                CTypeRuntime::Ptr => {
                    if i == 0 {
                        Ok(0) // null pointer
                    } else {
                        Err(MarshalError::UnsupportedConversion {
                            from: "Int",
                            to: target,
                        })
                    }
                }
                _ => Err(MarshalError::UnsupportedConversion {
                    from: "Int",
                    to: target,
                }),
            };
        }

        // Float conversions (including NaN values which are stored with TAG_NAN)
        // Use try_as_f64() which handles both regular floats and tagged NaN values
        if let Some(f) = value.try_as_f64() {
            return match target {
                CTypeRuntime::F32 => Ok((f as f32).to_bits() as u64),
                CTypeRuntime::F64 => Ok(f.to_bits()),
                _ => Err(MarshalError::UnsupportedConversion {
                    from: "Float",
                    to: target,
                }),
            };
        }

        // Boolean conversion
        if value.is_bool() {
            let b = value.as_bool();
            return match target {
                CTypeRuntime::Bool => Ok(if b { 1 } else { 0 }),
                CTypeRuntime::I8 | CTypeRuntime::U8 => Ok(if b { 1 } else { 0 }),
                CTypeRuntime::I32 | CTypeRuntime::U32 => Ok(if b { 1 } else { 0 }),
                _ => Err(MarshalError::UnsupportedConversion {
                    from: "Bool",
                    to: target,
                }),
            };
        }

        // Pointer conversions
        if value.is_ptr() {
            let p = value.as_ptr::<u8>() as u64;
            return match target {
                CTypeRuntime::Ptr
                | CTypeRuntime::StructPtr(_)
                | CTypeRuntime::ArrayPtr
                | CTypeRuntime::FnPtr
                | CTypeRuntime::CStr => Ok(p),
                _ => Err(MarshalError::UnsupportedConversion {
                    from: "Ptr",
                    to: target,
                }),
            };
        }

        // Unit/nil conversion
        if value.is_unit() || value.is_nil() {
            return match target {
                CTypeRuntime::Ptr | CTypeRuntime::CStr => Ok(0), // null pointer
                CTypeRuntime::Void => Ok(0),
                _ => Err(MarshalError::UnsupportedConversion {
                    from: if value.is_unit() { "Unit" } else { "Nil" },
                    to: target,
                }),
            };
        }

        // Small string conversion
        // Supports CStr (null-terminated) and Ptr/ArrayPtr (raw bytes)
        if value.is_small_string() {
            let s = value.as_small_string();
            return match target {
                CTypeRuntime::CStr => {
                    // Null-terminated C string
                    let cstring = CString::new(s.as_str()).map_err(|_| {
                        MarshalError::InvalidString(s.as_str().to_string())
                    })?;
                    let ptr = cstring.as_ptr() as u64;
                    self.string_cache.push(cstring);
                    Ok(ptr)
                }
                CTypeRuntime::Ptr | CTypeRuntime::ArrayPtr => {
                    // Raw byte pointer (for write() etc.)
                    // String data is NOT null-terminated in this case
                    // We use CString internally to ensure stable memory
                    let cstring = CString::new(s.as_str()).map_err(|_| {
                        MarshalError::InvalidString(s.as_str().to_string())
                    })?;
                    let ptr = cstring.as_ptr() as u64;
                    self.string_cache.push(cstring);
                    Ok(ptr)
                }
                _ => Err(MarshalError::UnsupportedConversion {
                    from: "SmallString",
                    to: target,
                }),
            };
        }

        // Unsupported type
        Err(MarshalError::UnsupportedConversion {
            from: value_type_name(&value),
            to: target,
        })
    }

    /// Converts a raw C value to a Verum Value.
    pub fn c_to_value(&self, raw: u64, source: CTypeRuntime) -> Result<Value, MarshalError> {
        match source {
            CTypeRuntime::Void => Ok(Value::unit()),

            // Signed integers
            CTypeRuntime::I8 => Ok(Value::from_i64(raw as i8 as i64)),
            CTypeRuntime::I16 => Ok(Value::from_i64(raw as i16 as i64)),
            CTypeRuntime::I32 => Ok(Value::from_i64(raw as i32 as i64)),
            CTypeRuntime::I64 => Ok(Value::from_i64(raw as i64)),
            CTypeRuntime::Ssize => Ok(Value::from_i64(raw as isize as i64)),

            // Unsigned integers
            CTypeRuntime::U8 => Ok(Value::from_i64(raw as u8 as i64)),
            CTypeRuntime::U16 => Ok(Value::from_i64(raw as u16 as i64)),
            CTypeRuntime::U32 => Ok(Value::from_i64(raw as u32 as i64)),
            CTypeRuntime::U64 => {
                // Note: u64 may overflow i64
                Ok(Value::from_i64(raw as i64))
            }
            CTypeRuntime::Size => Ok(Value::from_i64(raw as usize as i64)),

            // Floats
            CTypeRuntime::F32 => Ok(Value::from_f64(f32::from_bits(raw as u32) as f64)),
            CTypeRuntime::F64 => Ok(Value::from_f64(f64::from_bits(raw))),

            // Boolean
            CTypeRuntime::Bool => Ok(Value::from_bool(raw != 0)),

            // Pointers
            CTypeRuntime::Ptr | CTypeRuntime::StructPtr(_) | CTypeRuntime::ArrayPtr | CTypeRuntime::FnPtr => {
                Ok(Value::from_ptr(raw as *mut u8))
            }

            // C string - return as pointer, caller can convert to string
            CTypeRuntime::CStr => {
                if raw == 0 {
                    Ok(Value::nil())
                } else {
                    Ok(Value::from_ptr(raw as *mut u8))
                }
            }

            // Struct by value - raw contains the data or pointer to return buffer
            // The caller (FfiRuntime) handles struct unmarshalling using the layout
            CTypeRuntime::StructValue(_layout_idx) => {
                // For struct-by-value returns, the raw value is a pointer to the
                // struct data buffer allocated by libffi. The actual conversion
                // to Verum struct values is handled by FfiRuntime which has access
                // to the FfiStructLayout.
                Ok(Value::from_ptr(raw as *mut u8))
            }
        }
    }

}

impl Default for Marshaller {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the type name of a Value for error messages.
fn value_type_name(value: &Value) -> &'static str {
    if value.is_nil() {
        "Nil"
    } else if value.is_int() {
        "Int"
    } else if value.is_float() {
        "Float"
    } else if value.is_bool() {
        "Bool"
    } else if value.is_unit() {
        "Unit"
    } else if value.is_ptr() {
        "Ptr"
    } else if value.is_small_string() {
        "SmallString"
    } else if value.is_type_ref() {
        "Type"
    } else if value.is_func_ref() {
        "Function"
    } else if value.is_generator() {
        "Generator"
    } else {
        "Unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int_to_c() {
        let mut marshaller = Marshaller::new();

        // i32 conversion
        let raw = marshaller.value_to_c(Value::from_i64(42), CTypeRuntime::I32).unwrap();
        assert_eq!(raw as i32, 42);

        // Negative i32
        let raw = marshaller.value_to_c(Value::from_i64(-42), CTypeRuntime::I32).unwrap();
        assert_eq!(raw as i32, -42);

        // u32 conversion
        let raw = marshaller.value_to_c(Value::from_i64(42), CTypeRuntime::U32).unwrap();
        assert_eq!(raw as u32, 42);

        // u32 overflow check
        let result = marshaller.value_to_c(Value::from_i64(-1), CTypeRuntime::U32);
        assert!(result.is_err());
    }

    #[test]
    fn test_float_to_c() {
        let mut marshaller = Marshaller::new();

        // f64 conversion
        let raw = marshaller.value_to_c(Value::from_f64(3.14), CTypeRuntime::F64).unwrap();
        assert_eq!(f64::from_bits(raw), 3.14);

        // f32 conversion (with precision loss)
        let raw = marshaller.value_to_c(Value::from_f64(3.14), CTypeRuntime::F32).unwrap();
        let result = f32::from_bits(raw as u32);
        assert!((result - 3.14_f32).abs() < 0.0001);
    }

    #[test]
    fn test_c_to_value() {
        let marshaller = Marshaller::new();

        // i32 to Value
        let value = marshaller.c_to_value(42u64, CTypeRuntime::I32).unwrap();
        assert!(value.is_int());
        assert_eq!(value.as_i64(), 42);

        // Negative i32 to Value
        let value = marshaller.c_to_value((-42i32) as u64, CTypeRuntime::I32).unwrap();
        assert!(value.is_int());
        assert_eq!(value.as_i64(), -42);

        // f64 to Value
        let raw = 3.14_f64.to_bits();
        let value = marshaller.c_to_value(raw, CTypeRuntime::F64).unwrap();
        assert!(value.is_float());
        assert_eq!(value.as_f64(), 3.14);

        // bool to Value
        let value = marshaller.c_to_value(1, CTypeRuntime::Bool).unwrap();
        assert!(value.is_bool());
        assert!(value.as_bool());
        let value = marshaller.c_to_value(0, CTypeRuntime::Bool).unwrap();
        assert!(value.is_bool());
        assert!(!value.as_bool());
    }

    #[test]
    fn test_small_string_to_c() {
        let mut marshaller = Marshaller::new();

        // Convert small string
        let value = Value::from_small_string("hello").unwrap();
        let raw = marshaller.value_to_c(value, CTypeRuntime::CStr).unwrap();
        assert_ne!(raw, 0);

        // String should be in cache
        assert_eq!(marshaller.string_cache.len(), 1);

        // Clear cache
        marshaller.clear_cache();
        assert!(marshaller.string_cache.is_empty());
    }

    #[test]
    fn test_nil_to_c() {
        let mut marshaller = Marshaller::new();

        // Nil to pointer
        let raw = marshaller.value_to_c(Value::nil(), CTypeRuntime::Ptr).unwrap();
        assert_eq!(raw, 0);

        // Unit to void
        let raw = marshaller.value_to_c(Value::unit(), CTypeRuntime::Void).unwrap();
        assert_eq!(raw, 0);
    }
}
