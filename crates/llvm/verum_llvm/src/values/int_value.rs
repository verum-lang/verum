//! Integer values in LLVM.
//!
//! Note: In LLVM 17+, many const_* operations were removed from the C API.
//! Constant folding is now done automatically by instruction builders.
//! Use Builder::build_int_* methods instead of const_* methods.

use verum_llvm_sys::core::{
    LLVMConstAdd, LLVMConstBitCast, LLVMConstIntGetSExtValue, LLVMConstIntGetZExtValue,
    LLVMConstIntToPtr, LLVMConstMul, LLVMConstNSWAdd, LLVMConstNSWMul, LLVMConstNSWNeg,
    LLVMConstNSWSub, LLVMConstNUWAdd, LLVMConstNUWMul, LLVMConstNUWSub, LLVMConstNeg,
    LLVMConstNot, LLVMConstSub, LLVMConstTrunc, LLVMConstTruncOrBitCast, LLVMConstXor,
    LLVMIsAConstantInt,
};

use verum_llvm_sys::prelude::LLVMValueRef;

use std::convert::TryFrom;
use std::ffi::CStr;
use std::fmt::{self, Display};

use crate::types::{AsTypeRef, IntType, PointerType};
use crate::values::traits::AsValueRef;
use crate::values::{InstructionValue, PointerValue, Value};

use super::AnyValue;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct IntValue<'ctx> {
    int_value: Value<'ctx>,
}

impl<'ctx> IntValue<'ctx> {
    /// Get a value from an [LLVMValueRef].
    ///
    /// # Safety
    ///
    /// The ref must be valid and of type int.
    pub unsafe fn new(value: LLVMValueRef) -> Self {
        assert!(!value.is_null());

        IntValue {
            int_value: unsafe { Value::new(value) },
        }
    }

    /// Gets the name of an `IntValue`. If the value is a constant, this will
    /// return an empty string.
    pub fn get_name(&self) -> &CStr {
        self.int_value.get_name()
    }

    /// Set name of the `IntValue`.
    pub fn set_name(&self, name: &str) {
        self.int_value.set_name(name)
    }

    pub fn get_type(self) -> IntType<'ctx> {
        unsafe { IntType::new(self.int_value.get_type()) }
    }

    pub fn is_null(self) -> bool {
        self.int_value.is_null()
    }

    pub fn is_undef(self) -> bool {
        self.int_value.is_undef()
    }

    pub fn print_to_stderr(self) {
        self.int_value.print_to_stderr()
    }

    pub fn as_instruction(self) -> Option<InstructionValue<'ctx>> {
        self.int_value.as_instruction()
    }

    pub fn const_not(self) -> Self {
        unsafe { IntValue::new(LLVMConstNot(self.as_value_ref())) }
    }

    pub fn const_neg(self) -> Self {
        unsafe { IntValue::new(LLVMConstNeg(self.as_value_ref())) }
    }

    pub fn const_nsw_neg(self) -> Self {
        unsafe { IntValue::new(LLVMConstNSWNeg(self.as_value_ref())) }
    }

    /// Computes the NUW negation of this value (0 - self, with no unsigned wrap).
    pub fn const_nuw_neg(self) -> Self {
        let zero = self.get_type().const_zero();
        zero.const_nuw_sub(self)
    }

    pub fn const_add(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstAdd(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_nsw_add(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstNSWAdd(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_nuw_add(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstNUWAdd(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_sub(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstSub(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_nsw_sub(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstNSWSub(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_nuw_sub(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstNUWSub(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_mul(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstMul(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_nsw_mul(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstNSWMul(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_nuw_mul(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstNUWMul(self.as_value_ref(), rhs.as_value_ref())) }
    }

    // Note: const_and, const_or, const_shl, const_rshr, const_ashr, const_cast,
    // const_s_extend, const_z_ext, const_s_extend_or_bit_cast, const_z_ext_or_bit_cast,
    // const_unsigned_to_float, const_signed_to_float, const_int_compare, and const_select
    // were removed because the underlying LLVM C API functions were removed in LLVM 17+.
    //
    // Use Builder::build_and, build_or, build_left_shift, build_right_shift, etc. instead.
    // These will automatically constant-fold when given constant operands.

    pub fn const_xor(self, rhs: IntValue<'ctx>) -> Self {
        unsafe { IntValue::new(LLVMConstXor(self.as_value_ref(), rhs.as_value_ref())) }
    }

    pub fn const_to_pointer(self, ptr_type: PointerType<'ctx>) -> PointerValue<'ctx> {
        unsafe { PointerValue::new(LLVMConstIntToPtr(self.as_value_ref(), ptr_type.as_type_ref())) }
    }

    pub fn const_truncate(self, int_type: IntType<'ctx>) -> IntValue<'ctx> {
        unsafe { IntValue::new(LLVMConstTrunc(self.as_value_ref(), int_type.as_type_ref())) }
    }

    pub fn const_truncate_or_bit_cast(self, int_type: IntType<'ctx>) -> IntValue<'ctx> {
        unsafe { IntValue::new(LLVMConstTruncOrBitCast(self.as_value_ref(), int_type.as_type_ref())) }
    }

    pub fn const_bit_cast(self, int_type: IntType) -> IntValue<'ctx> {
        unsafe { IntValue::new(LLVMConstBitCast(self.as_value_ref(), int_type.as_type_ref())) }
    }

    /// Determines whether or not an `IntValue` is an `llvm::Constant`.
    ///
    /// Constants includes values that are not known at compile time, for
    /// example the address of a function casted to an integer.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::context::Context;
    ///
    /// let context = Context::create();
    /// let i64_type = context.i64_type();
    /// let i64_val = i64_type.const_int(12, false);
    ///
    /// assert!(i64_val.is_const());
    /// ```
    pub fn is_const(self) -> bool {
        self.int_value.is_const()
    }

    /// Determines whether or not an `IntValue` is an `llvm::ConstantInt`.
    ///
    /// ConstantInt only includes values that are known at compile time.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::context::Context;
    ///
    /// let context = Context::create();
    /// let i64_type = context.i64_type();
    /// let i64_val = i64_type.const_int(12, false);
    ///
    /// assert!(i64_val.is_constant_int());
    /// ```
    pub fn is_constant_int(self) -> bool {
        !unsafe { LLVMIsAConstantInt(self.as_value_ref()) }.is_null()
    }

    /// Obtains a constant `IntValue`'s zero extended value.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::context::Context;
    ///
    /// let context = Context::create();
    /// let i8_type = context.i8_type();
    /// let i8_all_ones = i8_type.const_all_ones();
    ///
    /// assert_eq!(i8_all_ones.get_zero_extended_constant(), Some(255));
    /// ```
    pub fn get_zero_extended_constant(self) -> Option<u64> {
        // Garbage values are produced on non constant values
        if !self.is_constant_int() {
            return None;
        }
        if self.get_type().get_bit_width() > 64 {
            return None;
        }

        unsafe { Some(LLVMConstIntGetZExtValue(self.as_value_ref())) }
    }

    /// Obtains a constant `IntValue`'s sign extended value.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_llvm::context::Context;
    ///
    /// let context = Context::create();
    /// let i8_type = context.i8_type();
    /// let i8_all_ones = i8_type.const_all_ones();
    ///
    /// assert_eq!(i8_all_ones.get_sign_extended_constant(), Some(-1));
    /// ```
    pub fn get_sign_extended_constant(self) -> Option<i64> {
        // Garbage values are produced on non constant values
        if !self.is_constant_int() {
            return None;
        }
        if self.get_type().get_bit_width() > 64 {
            return None;
        }

        unsafe { Some(LLVMConstIntGetSExtValue(self.as_value_ref())) }
    }

    pub fn replace_all_uses_with(self, other: IntValue<'ctx>) {
        self.int_value.replace_all_uses_with(other.as_value_ref())
    }
}

unsafe impl AsValueRef for IntValue<'_> {
    fn as_value_ref(&self) -> LLVMValueRef {
        self.int_value.value
    }
}

impl Display for IntValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.print_to_string())
    }
}

impl<'ctx> TryFrom<InstructionValue<'ctx>> for IntValue<'ctx> {
    type Error = ();

    fn try_from(value: InstructionValue) -> Result<Self, Self::Error> {
        if value.get_type().is_int_type() {
            unsafe { Ok(IntValue::new(value.as_value_ref())) }
        } else {
            Err(())
        }
    }
}
