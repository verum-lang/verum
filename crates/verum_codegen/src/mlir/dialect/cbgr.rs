//! Comprehensive CBGR (Compile-time Borrow and Generation-based Reference) dialect operations.
//!
//! This module implements the full CBGR memory safety system for Verum, providing:
//! - Three-tier reference system (Managed/Checked/Unsafe)
//! - Generation-based validity tracking
//! - Epoch-based lifetime tracking
//! - Escape analysis annotations
//! - Borrow scope management
//!
//! # CBGR Reference Layout
//!
//! ```text
//! ThinRef<T>: { ptr: *mut T, generation: u32, epoch_caps: u32 } = 16 bytes
//! FatRef<T>:  { ptr: *mut T, generation: u32, epoch_caps: u32, metadata: u64 } = 24 bytes
//! ```
//!
//! # Reference Tiers
//!
//! | Tier | Name    | Overhead | Validation |
//! |------|---------|----------|------------|
//! | 0    | Managed | ~15ns    | Full CBGR checks |
//! | 1    | Checked | 0ns      | Compile-time proven safe |
//! | 2    | Unsafe  | 0ns      | Manual safety proof required |

use verum_mlir::{
    Context,
    ir::{
        Attribute, Block, Identifier, Location, Module, Operation, Region, Type, Value,
        attribute::{
            ArrayAttribute, DenseI32ArrayAttribute, DenseI64ArrayAttribute,
            IntegerAttribute, StringAttribute, TypeAttribute,
        },
        operation::{OperationBuilder, OperationLike},
        r#type::IntegerType,
    },
    dialect::llvm,
};
use verum_common::Text;
use crate::mlir::error::{MlirError, Result};
use crate::mlir::dialect::{op_names, attr_names};
use super::types::RefTier;

// ============================================================================
// CBGR Type Structures
// ============================================================================

/// CBGR reference structure layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CbgrRefLayout {
    /// Thin reference: { ptr, generation, epoch_caps }
    Thin,
    /// Fat reference: { ptr, generation, epoch_caps, metadata }
    Fat,
}

impl CbgrRefLayout {
    /// Get the size in bytes for this layout.
    pub const fn size_bytes(&self) -> usize {
        match self {
            Self::Thin => 16,
            Self::Fat => 24,
        }
    }

    /// Get the number of fields in this layout.
    pub const fn field_count(&self) -> usize {
        match self {
            Self::Thin => 3,
            Self::Fat => 4,
        }
    }
}

/// CBGR capability flags stored in epoch_caps field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CbgrCapability {
    /// Reference can be read.
    Read = 0b0001,
    /// Reference can be written.
    Write = 0b0010,
    /// Reference can be shared.
    Share = 0b0100,
    /// Reference owns the data.
    Own = 0b1000,
}

impl CbgrCapability {
    /// Combine multiple capabilities.
    pub fn combine(caps: &[Self]) -> u32 {
        caps.iter().fold(0u32, |acc, cap| acc | (*cap as u32))
    }

    /// Check if a capability set includes a specific capability.
    pub fn has(caps: u32, cap: Self) -> bool {
        (caps & (cap as u32)) != 0
    }
}

/// Escape analysis category for CBGR optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeCategory {
    /// Value never escapes the current function.
    NoEscape,
    /// Value escapes within the function but not beyond.
    LocalEscape,
    /// Value may escape to caller or other functions.
    MayEscape,
    /// Escape behavior is unknown or complex.
    Unknown,
}

impl EscapeCategory {
    /// Check if this category allows CBGR check elimination.
    pub fn can_eliminate_check(&self) -> bool {
        matches!(self, Self::NoEscape | Self::LocalEscape)
    }

    /// Check if this category allows promotion to Checked tier.
    pub fn can_promote_to_checked(&self) -> bool {
        matches!(self, Self::NoEscape | Self::LocalEscape | Self::MayEscape)
    }

    /// Get the attribute value for this category.
    pub fn to_attr_value(&self) -> i64 {
        match self {
            Self::NoEscape => 0,
            Self::LocalEscape => 1,
            Self::MayEscape => 2,
            Self::Unknown => 3,
        }
    }

    /// Create from attribute value.
    pub fn from_attr_value(value: i64) -> Self {
        match value {
            0 => Self::NoEscape,
            1 => Self::LocalEscape,
            2 => Self::MayEscape,
            _ => Self::Unknown,
        }
    }
}

// ============================================================================
// CBGR Operations - Allocation
// ============================================================================

/// CBGR allocation operation.
///
/// Allocates memory with CBGR tracking (generation + epoch).
///
/// ```mlir
/// %ref = verum.cbgr_alloc %value {
///     tier = 0 : i32,
///     initial_gen = 1 : i32,
///     caps = 3 : i32,  // Read | Write
///     layout = "thin"
/// } : T -> !llvm.struct<(ptr, i32, i32)>
/// ```
pub struct CbgrAllocOp;

impl CbgrAllocOp {
    /// Build a CBGR allocation operation with full options.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        tier: RefTier,
        capabilities: u32,
        layout: CbgrRefLayout,
    ) -> Result<Operation<'c>> {
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();

        OperationBuilder::new(op_names::CBGR_ALLOC, location)
            .add_operands(&[value])
            .add_results(&[result_type])
            .add_attributes(&[
                (
                    Identifier::new(context, attr_names::CBGR_TIER),
                    IntegerAttribute::new(i32_type, tier as i64).into(),
                ),
                (
                    Identifier::new(context, "initial_gen"),
                    IntegerAttribute::new(i32_type, 1).into(),
                ),
                (
                    Identifier::new(context, "caps"),
                    IntegerAttribute::new(i32_type, capabilities as i64).into(),
                ),
                (
                    Identifier::new(context, "layout"),
                    StringAttribute::new(
                        context,
                        match layout {
                            CbgrRefLayout::Thin => "thin",
                            CbgrRefLayout::Fat => "fat",
                        },
                    )
                    .into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_ALLOC, format!("{:?}", e)))
    }

    /// Build a simple CBGR allocation with defaults.
    pub fn build_simple<'c>(
        context: &'c Context,
        location: Location<'c>,
        value: Value<'c, '_>,
        result_type: Type<'c>,
        tier: RefTier,
    ) -> Result<Operation<'c>> {
        let caps = CbgrCapability::combine(&[CbgrCapability::Read, CbgrCapability::Write]);
        Self::build(context, location, value, result_type, tier, caps, CbgrRefLayout::Thin)
    }
}

/// CBGR reallocation operation.
///
/// Reallocates a CBGR reference with a new value, preserving generation.
///
/// ```mlir
/// %new_ref = verum.cbgr_realloc %ref, %new_value : !llvm.struct<(ptr, i32, i32)>
/// ```
pub struct CbgrReallocOp;

impl CbgrReallocOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        new_value: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.cbgr_realloc", location)
            .add_operands(&[reference, new_value])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_realloc", format!("{:?}", e)))
    }
}

// ============================================================================
// CBGR Operations - Validation
// ============================================================================

/// CBGR check operation - validates reference validity.
///
/// ```mlir
/// %valid = verum.cbgr_check %ref, %expected_gen {
///     check_caps = true,
///     expected_caps = 3 : i32
/// } : i1
/// ```
pub struct CbgrCheckOp;

impl CbgrCheckOp {
    /// Build a full CBGR check with capability validation.
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        expected_generation: Value<'c, '_>,
        check_capabilities: bool,
        expected_caps: Option<u32>,
    ) -> Result<Operation<'c>> {
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();

        let mut attrs = vec![(
            Identifier::new(context, "check_caps"),
            IntegerAttribute::new(i1_type, if check_capabilities { 1 } else { 0 }).into(),
        )];

        if let Some(caps) = expected_caps {
            attrs.push((
                Identifier::new(context, "expected_caps"),
                IntegerAttribute::new(i32_type, caps as i64).into(),
            ));
        }

        OperationBuilder::new(op_names::CBGR_CHECK, location)
            .add_operands(&[reference, expected_generation])
            .add_results(&[i1_type])
            .add_attributes(&attrs)
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_CHECK, format!("{:?}", e)))
    }

    /// Build a simple generation-only check.
    pub fn build_simple<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        expected_generation: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        Self::build(context, location, reference, expected_generation, false, None)
    }
}

/// CBGR generation extraction operation.
///
/// ```mlir
/// %gen = verum.cbgr_get_gen %ref : i32
/// ```
pub struct CbgrGetGenOp;

impl CbgrGetGenOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();

        OperationBuilder::new("verum.cbgr_get_gen", location)
            .add_operands(&[reference])
            .add_results(&[i32_type])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_get_gen", format!("{:?}", e)))
    }
}

/// CBGR increment generation operation.
///
/// ```mlir
/// %new_ref = verum.cbgr_inc_gen %ref : !llvm.struct<(ptr, i32, i32)>
/// ```
pub struct CbgrIncGenOp;

impl CbgrIncGenOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.cbgr_inc_gen", location)
            .add_operands(&[reference])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_inc_gen", format!("{:?}", e)))
    }
}

// ============================================================================
// CBGR Operations - Dereference
// ============================================================================

/// CBGR dereference operation with validation.
///
/// ```mlir
/// %value = verum.cbgr_deref %ref {
///     validate = true,
///     tier = 0 : i32
/// } : !llvm.struct<(ptr, i32, i32)> -> T
/// ```
pub struct CbgrDerefOp;

impl CbgrDerefOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        result_type: Type<'c>,
        validate: bool,
        tier: RefTier,
    ) -> Result<Operation<'c>> {
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();

        OperationBuilder::new(op_names::CBGR_DEREF, location)
            .add_operands(&[reference])
            .add_results(&[result_type])
            .add_attributes(&[
                (
                    Identifier::new(context, "validate"),
                    IntegerAttribute::new(i1_type, if validate { 1 } else { 0 }).into(),
                ),
                (
                    Identifier::new(context, attr_names::CBGR_TIER),
                    IntegerAttribute::new(i32_type, tier as i64).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_DEREF, format!("{:?}", e)))
    }

    /// Build a simple dereference with validation based on tier.
    pub fn build_simple<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        result_type: Type<'c>,
        tier: RefTier,
    ) -> Result<Operation<'c>> {
        let validate = tier.requires_runtime_check();
        Self::build(context, location, reference, result_type, validate, tier)
    }
}

/// CBGR unchecked dereference - for Checked/Unsafe tiers.
///
/// ```mlir
/// %value = verum.cbgr_deref_unchecked %ref : !llvm.ptr -> T
/// ```
pub struct CbgrDerefUncheckedOp;

impl CbgrDerefUncheckedOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.cbgr_deref_unchecked", location)
            .add_operands(&[reference])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_deref_unchecked", format!("{:?}", e)))
    }
}

// ============================================================================
// CBGR Operations - Store
// ============================================================================

/// CBGR store operation with mutation tracking.
///
/// ```mlir
/// verum.cbgr_store %ref, %value {
///     validate = true,
///     inc_gen = true
/// } : !llvm.struct<(ptr, i32, i32)>, T
/// ```
pub struct CbgrStoreOp;

impl CbgrStoreOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        value: Value<'c, '_>,
        validate: bool,
        increment_generation: bool,
    ) -> Result<Operation<'c>> {
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();

        OperationBuilder::new("verum.cbgr_store", location)
            .add_operands(&[reference, value])
            .add_attributes(&[
                (
                    Identifier::new(context, "validate"),
                    IntegerAttribute::new(i1_type, if validate { 1 } else { 0 }).into(),
                ),
                (
                    Identifier::new(context, "inc_gen"),
                    IntegerAttribute::new(i1_type, if increment_generation { 1 } else { 0 }).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_store", format!("{:?}", e)))
    }
}

// ============================================================================
// CBGR Operations - Drop and Cleanup
// ============================================================================

/// CBGR drop operation with optional invalidation.
///
/// ```mlir
/// verum.cbgr_drop %ref {
///     invalidate = true,
///     run_destructor = true
/// }
/// ```
pub struct CbgrDropOp;

impl CbgrDropOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        invalidate: bool,
        run_destructor: bool,
    ) -> Result<Operation<'c>> {
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();

        OperationBuilder::new(op_names::CBGR_DROP, location)
            .add_operands(&[reference])
            .add_attributes(&[
                (
                    Identifier::new(context, "invalidate"),
                    IntegerAttribute::new(i1_type, if invalidate { 1 } else { 0 }).into(),
                ),
                (
                    Identifier::new(context, "run_destructor"),
                    IntegerAttribute::new(i1_type, if run_destructor { 1 } else { 0 }).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation(op_names::CBGR_DROP, format!("{:?}", e)))
    }

    /// Build a simple drop with full cleanup.
    pub fn build_simple<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        Self::build(context, location, reference, true, true)
    }
}

// ============================================================================
// CBGR Operations - Borrow Scopes
// ============================================================================

/// CBGR borrow scope - tracks exclusive/shared borrows.
///
/// ```mlir
/// %result = verum.cbgr_borrow_scope {
///     borrow_kind = "exclusive"
/// } {
///     // scope body
///     verum.cbgr_borrow_yield %value : T
/// } : T
/// ```
pub struct CbgrBorrowScopeOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    /// Shared borrow - multiple readers allowed.
    Shared,
    /// Exclusive borrow - single writer.
    Exclusive,
}

impl CbgrBorrowScopeOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        borrow_kind: BorrowKind,
        result_types: &[Type<'c>],
        body: Region<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.cbgr_borrow_scope", location)
            .add_attributes(&[(
                Identifier::new(context, "borrow_kind"),
                StringAttribute::new(
                    context,
                    match borrow_kind {
                        BorrowKind::Shared => "shared",
                        BorrowKind::Exclusive => "exclusive",
                    },
                )
                .into(),
            )])
            .add_results(result_types)
            .add_regions([body])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_borrow_scope", format!("{:?}", e)))
    }
}

/// CBGR borrow yield - terminates a borrow scope.
pub struct CbgrBorrowYieldOp;

impl CbgrBorrowYieldOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        values: &[Value<'c, '_>],
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.cbgr_borrow_yield", location)
            .add_operands(values)
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_borrow_yield", format!("{:?}", e)))
    }
}

// ============================================================================
// CBGR Operations - Tier Promotion/Demotion
// ============================================================================

/// CBGR promote operation - promotes to a higher tier.
///
/// ```mlir
/// %checked_ref = verum.cbgr_promote %managed_ref {
///     from_tier = 0 : i32,
///     to_tier = 1 : i32,
///     escape_category = 0 : i32  // NoEscape
/// } : !llvm.struct<(ptr, i32, i32)> -> !llvm.ptr
/// ```
pub struct CbgrPromoteOp;

impl CbgrPromoteOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        from_tier: RefTier,
        to_tier: RefTier,
        escape_category: EscapeCategory,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();

        OperationBuilder::new("verum.cbgr_promote", location)
            .add_operands(&[reference])
            .add_results(&[result_type])
            .add_attributes(&[
                (
                    Identifier::new(context, "from_tier"),
                    IntegerAttribute::new(i32_type, from_tier as i64).into(),
                ),
                (
                    Identifier::new(context, "to_tier"),
                    IntegerAttribute::new(i32_type, to_tier as i64).into(),
                ),
                (
                    Identifier::new(context, attr_names::ESCAPE_CATEGORY),
                    IntegerAttribute::new(i32_type, escape_category.to_attr_value()).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_promote", format!("{:?}", e)))
    }
}

/// CBGR demote operation - demotes to a lower tier.
pub struct CbgrDemoteOp;

impl CbgrDemoteOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        reference: Value<'c, '_>,
        from_tier: RefTier,
        to_tier: RefTier,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();

        OperationBuilder::new("verum.cbgr_demote", location)
            .add_operands(&[reference])
            .add_results(&[result_type])
            .add_attributes(&[
                (
                    Identifier::new(context, "from_tier"),
                    IntegerAttribute::new(i32_type, from_tier as i64).into(),
                ),
                (
                    Identifier::new(context, "to_tier"),
                    IntegerAttribute::new(i32_type, to_tier as i64).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_demote", format!("{:?}", e)))
    }
}

// ============================================================================
// CBGR Operations - Escape Analysis Annotations
// ============================================================================

/// CBGR escape annotation - marks escape analysis results.
///
/// ```mlir
/// %annotated = verum.cbgr_escape_annotate %ref {
///     escape_category = 0 : i32,  // NoEscape
///     proven = true
/// } : T -> T
/// ```
pub struct CbgrEscapeAnnotateOp;

impl CbgrEscapeAnnotateOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        value: Value<'c, '_>,
        escape_category: EscapeCategory,
        proven: bool,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let i32_type: Type<'c> = IntegerType::new(context, 32).into();
        let i1_type: Type<'c> = IntegerType::new(context, 1).into();

        OperationBuilder::new("verum.cbgr_escape_annotate", location)
            .add_operands(&[value])
            .add_results(&[result_type])
            .add_attributes(&[
                (
                    Identifier::new(context, attr_names::ESCAPE_CATEGORY),
                    IntegerAttribute::new(i32_type, escape_category.to_attr_value()).into(),
                ),
                (
                    Identifier::new(context, "proven"),
                    IntegerAttribute::new(i1_type, if proven { 1 } else { 0 }).into(),
                ),
            ])
            .build()
            .map_err(|e| MlirError::operation("verum.cbgr_escape_annotate", format!("{:?}", e)))
    }
}

// ============================================================================
// CBGR Type Builder
// ============================================================================

/// Builder for CBGR reference types.
pub struct CbgrTypeBuilder<'c> {
    context: &'c Context,
}

impl<'c> CbgrTypeBuilder<'c> {
    /// Create a new CBGR type builder.
    pub fn new(context: &'c Context) -> Self {
        Self { context }
    }

    /// Create a thin reference type (managed tier).
    pub fn thin_ref_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.struct<(ptr, i32, i32)>")
            .ok_or_else(|| MlirError::type_translation("thin_ref", "failed to parse"))
    }

    /// Create a fat reference type (with metadata).
    pub fn fat_ref_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.struct<(ptr, i32, i32, i64)>")
            .ok_or_else(|| MlirError::type_translation("fat_ref", "failed to parse"))
    }

    /// Create a checked/unsafe pointer type.
    pub fn raw_ptr_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.ptr")
            .ok_or_else(|| MlirError::type_translation("raw_ptr", "failed to parse"))
    }

    /// Create a generation counter type.
    pub fn generation_type(&self) -> Type<'c> {
        IntegerType::new(self.context, 32).into()
    }

    /// Create an epoch/caps type.
    pub fn epoch_caps_type(&self) -> Type<'c> {
        IntegerType::new(self.context, 32).into()
    }

    /// Get reference type for a specific tier.
    pub fn ref_type_for_tier(&self, tier: RefTier, layout: CbgrRefLayout) -> Result<Type<'c>> {
        match tier {
            RefTier::Managed => match layout {
                CbgrRefLayout::Thin => self.thin_ref_type(),
                CbgrRefLayout::Fat => self.fat_ref_type(),
            },
            RefTier::Checked | RefTier::Unsafe => self.raw_ptr_type(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cbgr_capability() {
        let caps = CbgrCapability::combine(&[CbgrCapability::Read, CbgrCapability::Write]);
        assert_eq!(caps, 0b0011);
        assert!(CbgrCapability::has(caps, CbgrCapability::Read));
        assert!(CbgrCapability::has(caps, CbgrCapability::Write));
        assert!(!CbgrCapability::has(caps, CbgrCapability::Share));
    }

    #[test]
    fn test_escape_category() {
        assert!(EscapeCategory::NoEscape.can_eliminate_check());
        assert!(EscapeCategory::LocalEscape.can_eliminate_check());
        assert!(!EscapeCategory::MayEscape.can_eliminate_check());
        assert!(!EscapeCategory::Unknown.can_eliminate_check());
    }

    #[test]
    fn test_ref_tier_overhead() {
        assert_eq!(RefTier::Managed.overhead_ns(), 15);
        assert_eq!(RefTier::Checked.overhead_ns(), 0);
        assert_eq!(RefTier::Unsafe.overhead_ns(), 0);
    }

    #[test]
    fn test_cbgr_ref_layout() {
        assert_eq!(CbgrRefLayout::Thin.size_bytes(), 16);
        assert_eq!(CbgrRefLayout::Fat.size_bytes(), 24);
        assert_eq!(CbgrRefLayout::Thin.field_count(), 3);
        assert_eq!(CbgrRefLayout::Fat.field_count(), 4);
    }
}
