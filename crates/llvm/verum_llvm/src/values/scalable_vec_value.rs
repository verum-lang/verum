#[allow(deprecated)]
use verum_llvm_sys::core::LLVMGetElementAsConstant;
use verum_llvm_sys::core::{LLVMConstExtractElement, LLVMConstInsertElement, LLVMConstShuffleVector};
use verum_llvm_sys::prelude::LLVMValueRef;

use std::ffi::CStr;
use std::fmt::{self, Display};

use crate::types::ScalableVectorType;
use crate::values::traits::AsValueRef;
use crate::values::{BasicValue, BasicValueEnum, InstructionValue, IntValue, Value};

use super::AnyValue;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct ScalableVectorValue<'ctx> {
    scalable_vec_value: Value<'ctx>,
}

impl<'ctx> ScalableVectorValue<'ctx> {
    /// Get a value from an [LLVMValueRef].
    ///
    /// # Safety
    ///
    /// The ref must be valid and of type scalable vector.
    pub unsafe fn new(scalable_vector_value: LLVMValueRef) -> Self {
        assert!(!scalable_vector_value.is_null());

        ScalableVectorValue {
            scalable_vec_value: unsafe { Value::new(scalable_vector_value) },
        }
    }

    /// Determines whether or not a `ScalableVectorValue` is a constant.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_llvm::context::Context;
    ///
    /// let context = Context::create();
    /// let i8_type = context.i8_type();
    /// let i8_scalable_vec_type = i8_type.scalable_vec_type(3);
    /// let i8_scalable_vec_zero = i8_scalable_vec_type.const_zero();
    ///
    /// assert!(i8_scalable_vec_zero.is_const());
    /// ```
    pub fn is_const(self) -> bool {
        self.scalable_vec_value.is_const()
    }

    pub fn print_to_stderr(self) {
        self.scalable_vec_value.print_to_stderr()
    }

    /// Gets the name of a `ScalableVectorValue`. If the value is a constant, this will
    /// return an empty string.
    pub fn get_name(&self) -> &CStr {
        self.scalable_vec_value.get_name()
    }

    /// Set name of the `ScalableVectorValue`.
    pub fn set_name(&self, name: &str) {
        self.scalable_vec_value.set_name(name)
    }

    pub fn get_type(self) -> ScalableVectorType<'ctx> {
        unsafe { ScalableVectorType::new(self.scalable_vec_value.get_type()) }
    }

    pub fn is_null(self) -> bool {
        self.scalable_vec_value.is_null()
    }

    pub fn is_undef(self) -> bool {
        self.scalable_vec_value.is_undef()
    }

    pub fn as_instruction(self) -> Option<InstructionValue<'ctx>> {
        self.scalable_vec_value.as_instruction()
    }

    pub fn const_extract_element(self, index: IntValue<'ctx>) -> BasicValueEnum<'ctx> {
        unsafe { BasicValueEnum::new(LLVMConstExtractElement(self.as_value_ref(), index.as_value_ref())) }
    }

    // SubTypes: value should really be T in self: ScalableVectorValue<T> I think
    pub fn const_insert_element<BV: BasicValue<'ctx>>(self, index: IntValue<'ctx>, value: BV) -> BasicValueEnum<'ctx> {
        unsafe {
            BasicValueEnum::new(LLVMConstInsertElement(
                self.as_value_ref(),
                value.as_value_ref(),
                index.as_value_ref(),
            ))
        }
    }

    pub fn replace_all_uses_with(self, other: ScalableVectorValue<'ctx>) {
        self.scalable_vec_value.replace_all_uses_with(other.as_value_ref())
    }

    /// Returns the element at `index` as a constant. LLVM returns a zero-initialized
    /// value if the index is out of bounds (no error is raised).
    #[allow(deprecated)]
    pub fn get_element_as_constant(self, index: u32) -> BasicValueEnum<'ctx> {
        unsafe { BasicValueEnum::new(LLVMGetElementAsConstant(self.as_value_ref(), index)) }
    }

    // Note: const_select was removed because LLVMConstSelect was removed in LLVM 17+.
    // Use Builder::build_select instead.

    // SubTypes: <V: ScalableVectorValue<T, Const>> self: V, right: V, mask: V -> V
    pub fn const_shuffle_vector(
        self,
        right: ScalableVectorValue<'ctx>,
        mask: ScalableVectorValue<'ctx>,
    ) -> ScalableVectorValue<'ctx> {
        unsafe {
            ScalableVectorValue::new(LLVMConstShuffleVector(
                self.as_value_ref(),
                right.as_value_ref(),
                mask.as_value_ref(),
            ))
        }
    }
}

unsafe impl AsValueRef for ScalableVectorValue<'_> {
    fn as_value_ref(&self) -> LLVMValueRef {
        self.scalable_vec_value.value
    }
}

impl Display for ScalableVectorValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.print_to_string())
    }
}
