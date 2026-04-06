//! Float values in LLVM.
//!
//! Note: In LLVM 17+, many const_* operations were removed from the C API.
//! Constant folding is now done automatically by instruction builders.
//! Use Builder::build_float_* methods instead of const_* methods.

use verum_llvm_sys::core::LLVMConstRealGetDouble;
use verum_llvm_sys::prelude::LLVMValueRef;

use std::convert::TryFrom;
use std::ffi::CStr;
use std::fmt::{self, Display};

use crate::types::FloatType;
use crate::values::traits::AsValueRef;
use crate::values::{InstructionValue, Value};

use super::AnyValue;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct FloatValue<'ctx> {
    float_value: Value<'ctx>,
}

impl<'ctx> FloatValue<'ctx> {
    /// Get a value from an [LLVMValueRef].
    ///
    /// # Safety
    ///
    /// The ref must be valid and of type float.
    pub unsafe fn new(value: LLVMValueRef) -> Self {
        assert!(!value.is_null());

        FloatValue {
            float_value: unsafe { Value::new(value) },
        }
    }

    /// Gets name of the `FloatValue`. If the value is a constant, this will
    /// return an empty string.
    pub fn get_name(&self) -> &CStr {
        self.float_value.get_name()
    }

    /// Set name of the `FloatValue`.
    pub fn set_name(&self, name: &str) {
        self.float_value.set_name(name)
    }

    pub fn get_type(self) -> FloatType<'ctx> {
        unsafe { FloatType::new(self.float_value.get_type()) }
    }

    pub fn is_null(self) -> bool {
        self.float_value.is_null()
    }

    pub fn is_undef(self) -> bool {
        self.float_value.is_undef()
    }

    pub fn print_to_stderr(self) {
        self.float_value.print_to_stderr()
    }

    pub fn as_instruction(self) -> Option<InstructionValue<'ctx>> {
        self.float_value.as_instruction()
    }

    // Note: const_neg, const_add, const_sub, const_mul, const_div, const_remainder,
    // const_cast, const_to_unsigned_int, const_to_signed_int, const_truncate,
    // const_extend, and const_compare were removed because the underlying LLVM C API
    // functions (LLVMConstF*) were removed in LLVM 17+.
    //
    // Use Builder::build_float_neg, build_float_add, etc. instead.
    // These will automatically constant-fold when given constant operands.

    /// Determines whether or not a `FloatValue` is a constant.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::context::Context;
    ///
    /// let context = Context::create();
    /// let f64_type = context.f64_type();
    /// let f64_val = f64_type.const_float(1.2);
    ///
    /// assert!(f64_val.is_const());
    /// ```
    pub fn is_const(self) -> bool {
        self.float_value.is_const()
    }

    /// Obtains a constant `FloatValue`'s value and whether or not it lost info.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::context::Context;
    ///
    /// let context = Context::create();
    /// let f64_type = context.f64_type();
    /// let f64_1_2 = f64_type.const_float(1.2);
    ///
    /// assert_eq!(f64_1_2.get_constant(), Some((1.2, false)));
    /// ```
    pub fn get_constant(self) -> Option<(f64, bool)> {
        // Nothing bad happens as far as I can tell if we don't check if const
        // unlike the int versions, but just doing this just in case and for consistency
        if !self.is_const() {
            return None;
        }

        let mut lossy = 0;
        let constant = unsafe { LLVMConstRealGetDouble(self.as_value_ref(), &mut lossy) };

        Some((constant, lossy == 1))
    }

    pub fn replace_all_uses_with(self, other: FloatValue<'ctx>) {
        self.float_value.replace_all_uses_with(other.as_value_ref())
    }
}

unsafe impl AsValueRef for FloatValue<'_> {
    fn as_value_ref(&self) -> LLVMValueRef {
        self.float_value.value
    }
}

impl Display for FloatValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.print_to_string())
    }
}

impl<'ctx> TryFrom<InstructionValue<'ctx>> for FloatValue<'ctx> {
    type Error = ();

    fn try_from(value: InstructionValue) -> Result<Self, Self::Error> {
        if value.get_type().is_float_type() {
            unsafe { Ok(FloatValue::new(value.as_value_ref())) }
        } else {
            Err(())
        }
    }
}
