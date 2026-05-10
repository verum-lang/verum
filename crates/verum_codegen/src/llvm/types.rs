//! VBC Type → LLVM Type lowering.
//!

//! This module handles the translation of VBC types to their LLVM IR
//! representations.

use verum_llvm::context::Context;
use verum_llvm::types::{
    ArrayType, BasicMetadataTypeEnum, BasicTypeEnum, FloatType, FunctionType, IntType, PointerType,
    ScalableVectorType, StructType, VectorType, VoidType,
};
use verum_vbc::types::{TypeId, TypeRef};

use super::error::{LlvmLoweringError, Result};

/// Type lowering context for VBC → LLVM type translation.
pub struct TypeLowering<'ctx> {
    /// LLVM context reference.
    context: &'ctx Context,

    /// Cached LLVM types for primitives.
    i1_type: IntType<'ctx>,
    i8_type: IntType<'ctx>,
    i16_type: IntType<'ctx>,
    i32_type: IntType<'ctx>,
    i64_type: IntType<'ctx>,
    f32_type: FloatType<'ctx>,
    f64_type: FloatType<'ctx>,
    void_type: VoidType<'ctx>,
    ptr_type: PointerType<'ctx>,
}

impl<'ctx> TypeLowering<'ctx> {
    /// Create a new type lowering context.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            i1_type: context.bool_type(),
            i8_type: context.i8_type(),
            i16_type: context.i16_type(),
            i32_type: context.i32_type(),
            i64_type: context.i64_type(),
            f32_type: context.f32_type(),
            f64_type: context.f64_type(),
            void_type: context.void_type(),
            ptr_type: context.ptr_type(Default::default()),
        }
    }

    /// Lower a VBC TypeRef to LLVM BasicTypeEnum.
    pub fn lower_type_ref(&self, type_ref: &TypeRef) -> Result<BasicTypeEnum<'ctx>> {
        match type_ref {
            TypeRef::Concrete(type_id) => self.lower_type_id(*type_id),
            TypeRef::Generic(_) => Err(LlvmLoweringError::type_lowering(
                "Generic types should be monomorphized before lowering",
            )),
            TypeRef::Instantiated { base, args: _ } => {
                // Instantiated generic - for now just lower the base type
                // In practice this needs full generic resolution
                self.lower_type_id(*base)
            }
            TypeRef::Function { .. } | TypeRef::Rank2Function { .. } => {
                // Function types are represented as opaque pointers
                Ok(self.ptr_type.into())
            }
            TypeRef::Tuple(types) => self.lower_tuple(types),
            TypeRef::Reference { .. } => {
                // All references are opaque pointers in LLVM
                Ok(self.ptr_type.into())
            }
            TypeRef::Array { element, length } => {
                let elem_type = self.lower_type_ref(element)?;
                // Create array type from the element type
                let array_ty = self.make_array_type(elem_type, *length as u32)?;
                Ok(array_ty.into())
            }
            TypeRef::Slice(_) => {
                // Slices are fat pointers: { ptr, len }
                let slice_type = self
                    .context
                    .struct_type(&[self.ptr_type.into(), self.i64_type.into()], false);
                Ok(slice_type.into())
            }
        }
    }

    /// Lower a VBC TypeId to LLVM BasicTypeEnum.
    pub fn lower_type_id(&self, type_id: TypeId) -> Result<BasicTypeEnum<'ctx>> {
        // Handle built-in types
        Ok(match type_id {
            TypeId::UNIT => self.context.struct_type(&[], false).into(),
            TypeId::BOOL => self.i1_type.into(),
            // Note: INT and I64 are aliases (both TypeId(2)), as are FLOAT and F64 (both TypeId(3))
            TypeId::INT => self.i64_type.into(),
            TypeId::FLOAT => self.f64_type.into(),
            TypeId::TEXT => self.ptr_type.into(), // Text is a pointer to string data
            TypeId::NEVER => self.i8_type.into(), // Placeholder for never type
            TypeId::U8 | TypeId::I8 => self.i8_type.into(),
            TypeId::U16 | TypeId::I16 => self.i16_type.into(),
            TypeId::U32 | TypeId::I32 => self.i32_type.into(),
            TypeId::U64 => self.i64_type.into(),
            TypeId::F32 => self.f32_type.into(),
            // PTR and user-defined types are heap-allocated objects represented as
            // opaque pointers. Using LLVM `ptr` preserves pointer provenance for
            // optimization passes (GVN, SROA, inlining). The coerce_value() function
            // in instruction.rs handles bidirectional i64↔ptr conversions at call
            // sites transparently (ptrtoint for ptr→i64, inttoptr for i64→ptr).
            TypeId::PTR => self.ptr_type.into(),
            _ => {
                // User-defined types are heap pointers — use opaque ptr.
                // This enables LLVM to optimize through function boundaries.
                self.ptr_type.into()
            }
        })
    }

    /// Lower a tuple type to LLVM struct.
    pub fn lower_tuple(&self, types: &[TypeRef]) -> Result<BasicTypeEnum<'ctx>> {
        let llvm_types: Vec<BasicTypeEnum<'ctx>> = types
            .iter()
            .map(|t| self.lower_type_ref(t))
            .collect::<Result<_>>()?;

        Ok(self.context.struct_type(&llvm_types, false).into())
    }

    /// Create an array type from a BasicTypeEnum element type.
    fn make_array_type(&self, elem: BasicTypeEnum<'ctx>, size: u32) -> Result<ArrayType<'ctx>> {
        use BasicTypeEnum::*;
        Ok(match elem {
            ArrayType(t) => t.array_type(size),
            FloatType(t) => t.array_type(size),
            IntType(t) => t.array_type(size),
            PointerType(t) => t.array_type(size),
            StructType(t) => t.array_type(size),
            VectorType(t) => t.array_type(size),
            ScalableVectorType(t) => t.array_type(size),
        })
    }

    /// Create a function type from a BasicTypeEnum return type.
    fn make_fn_type(
        &self,
        ret: BasicTypeEnum<'ctx>,
        params: &[BasicMetadataTypeEnum<'ctx>],
        is_var_args: bool,
    ) -> FunctionType<'ctx> {
        use BasicTypeEnum::*;
        match ret {
            ArrayType(t) => t.fn_type(params, is_var_args),
            FloatType(t) => t.fn_type(params, is_var_args),
            IntType(t) => t.fn_type(params, is_var_args),
            PointerType(t) => t.fn_type(params, is_var_args),
            StructType(t) => t.fn_type(params, is_var_args),
            VectorType(t) => t.fn_type(params, is_var_args),
            ScalableVectorType(t) => t.fn_type(params, is_var_args),
        }
    }

    /// Lower function parameter types for declaration.
    pub fn lower_param_types(
        &self,
        params: &[verum_vbc::module::ParamDescriptor],
    ) -> Result<Vec<BasicMetadataTypeEnum<'ctx>>> {
        params
            .iter()
            .map(|p| self.lower_type_ref(&p.type_ref).map(|t| t.into()))
            .collect()
    }

    /// Lower a function type for declaration.
    pub fn lower_function_type(
        &self,
        params: &[verum_vbc::module::ParamDescriptor],
        ret: &TypeRef,
    ) -> Result<FunctionType<'ctx>> {
        let param_types = self.lower_param_types(params)?;

        // Check if return type is Unit
        let is_unit = matches!(ret, TypeRef::Concrete(TypeId::UNIT));

        if is_unit {
            Ok(self.void_type.fn_type(&param_types, false))
        } else {
            let ret_type = self.lower_type_ref(ret)?;
            Ok(self.make_fn_type(ret_type, &param_types, false))
        }
    }

    /// Get the LLVM i1 (bool) type.
    pub fn bool_type(&self) -> IntType<'ctx> {
        self.i1_type
    }

    /// Get the LLVM i8 type.
    pub fn i8_type(&self) -> IntType<'ctx> {
        self.i8_type
    }

    /// Get the LLVM i32 type.
    pub fn i32_type(&self) -> IntType<'ctx> {
        self.i32_type
    }

    /// Get the LLVM i64 type.
    pub fn i64_type(&self) -> IntType<'ctx> {
        self.i64_type
    }

    /// Get the LLVM f32 type.
    pub fn f32_type(&self) -> FloatType<'ctx> {
        self.f32_type
    }

    /// Get the LLVM f64 type.
    pub fn f64_type(&self) -> FloatType<'ctx> {
        self.f64_type
    }

    /// Get the LLVM void type.
    pub fn void_type(&self) -> VoidType<'ctx> {
        self.void_type
    }

    /// Get the LLVM opaque pointer type.
    pub fn ptr_type(&self) -> PointerType<'ctx> {
        self.ptr_type
    }

    /// Get the underlying LLVM context.
    pub fn context(&self) -> &'ctx Context {
        self.context
    }
}

/// CBGR reference tier for lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefTier {
    /// Tier 0: Full runtime checks (~15ns overhead).
    Tier0,
    /// Tier 1: Compiler-proven safe (zero overhead).
    Tier1,
    /// Tier 2: Manually marked unsafe (zero overhead).
    Tier2,
}

/// Per-variant projection for [`RefTier`].
///
/// `name` is the canonical short identifier used at the IR-comment /
/// audit-report wire form; `level` is the dense u8 0..=2 (matches
/// the documentation's "Tier 0/1/2" naming). `requires_runtime_check`
/// flags Tier0 — the only tier that emits the ~15ns CBGR check at
/// runtime; both Tier1 and Tier2 are zero-overhead, but only Tier1
/// is *proven* safe (Tier2 is unsafe-asserted).
#[derive(Debug, Clone, Copy)]
pub struct RefTierMeta {
    pub name: &'static str,
    pub level: u8,
    pub requires_runtime_check: bool,
    pub is_proven_safe: bool,
}

impl RefTier {
    pub const ALL: &'static [Self] = &[Self::Tier0, Self::Tier1, Self::Tier2];

    pub const fn meta(self) -> RefTierMeta {
        match self {
            Self::Tier0 => RefTierMeta {
                name: "tier0",
                level: 0,
                requires_runtime_check: true,
                is_proven_safe: false,
            },
            Self::Tier1 => RefTierMeta {
                name: "tier1",
                level: 1,
                requires_runtime_check: false,
                is_proven_safe: true,
            },
            Self::Tier2 => RefTierMeta {
                name: "tier2",
                level: 2,
                requires_runtime_check: false,
                is_proven_safe: false,
            },
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    /// Canonical short name (`"tier0"` / `"tier1"` / `"tier2"`).
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Dense u8 level 0..=2.
    #[inline]
    pub const fn level(&self) -> u8 {
        self.meta().level
    }

    /// True for Tier0 — the only tier that emits a runtime CBGR
    /// check.
    #[inline]
    pub const fn requires_runtime_check(&self) -> bool {
        self.meta().requires_runtime_check
    }

    /// True for Tier1 (compiler-proven safe). Tier2 is also
    /// zero-overhead but is unsafe-asserted, not proven.
    #[inline]
    pub const fn is_proven_safe(&self) -> bool {
        self.meta().is_proven_safe
    }
}

impl Default for RefTier {
    fn default() -> Self {
        RefTier::Tier0
    }
}

/// ThinRef layout: `{ ptr: *T, generation: u32, epoch_caps: u32 }`.
///
/// Re-exports the canonical [`verum_common::layout::THIN_REF_SIZE`] —
/// the single source of truth for CBGR reference layout, shared with
/// the typechecker, MIR lowering, and `@const` evaluator.
pub const THIN_REF_SIZE: usize = verum_common::layout::THIN_REF_SIZE as usize;

/// FatRef layout (canonical, per `core/mem/fat_ref.vr`):
/// `{ ptr: *T, generation: u32, epoch_and_caps: u32,
///    metadata: i64, offset_from_base: u32, reserved: u32 }`
/// — 32 bytes total.
///
/// Re-exports the canonical [`verum_common::layout::FAT_REF_SIZE`] —
/// the single source of truth. `verum_codegen::llvm::cbgr` constructs
/// the struct with all 6 fields, matching the stdlib declaration.
pub const FAT_REF_SIZE: usize = verum_common::layout::FAT_REF_SIZE as usize;

#[cfg(test)]
mod meta_consolidation_pins {
    use super::RefTier;

    #[test]
    fn ref_tier_round_trip_unique_dense_level_and_classification() {
        assert_eq!(RefTier::ALL.len(), 3);
        for v in RefTier::ALL {
            let s = v.as_str();
            assert_eq!(
                RefTier::from_str(s),
                Some(*v),
                "RefTier::{:?}: '{}' must round-trip",
                v,
                s
            );
        }
        // Dense u8 level 0..=2 in declaration order (matches the
        // documentation's "Tier 0/1/2" naming).
        for (i, v) in RefTier::ALL.iter().enumerate() {
            assert_eq!(v.level() as usize, i);
        }
        // Classification — pinned cross-cutting invariants:
        //   * Tier0 is the only tier requiring a runtime check.
        //   * Tier1 is the only tier proven safe.
        //   * Tier2 is zero-overhead but unsafe-asserted (neither
        //     proven safe nor runtime-checked).
        for v in RefTier::ALL {
            let expected_runtime = matches!(v, RefTier::Tier0);
            let expected_proven = matches!(v, RefTier::Tier1);
            assert_eq!(v.requires_runtime_check(), expected_runtime);
            assert_eq!(v.is_proven_safe(), expected_proven);
            // Cross-pin: requires_runtime_check ⇒ ¬is_proven_safe;
            // any zero-overhead tier (Tier1 / Tier2) is
            // ¬requires_runtime_check.
            if v.requires_runtime_check() {
                assert!(!v.is_proven_safe(),
                    "RefTier::{:?}: runtime-checked tier cannot be proven safe", v);
            }
        }
        // Default is Tier0 (the safe-but-slow conservative tier).
        assert_eq!(RefTier::default(), RefTier::Tier0);
        // Wire-form spot pins.
        assert_eq!(RefTier::Tier0.as_str(), "tier0");
        assert_eq!(RefTier::Tier1.as_str(), "tier1");
        assert_eq!(RefTier::Tier2.as_str(), "tier2");
    }
}
