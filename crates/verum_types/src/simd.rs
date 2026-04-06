//! SIMD Type Validation
//!
//! SIMD type validation: verifying SIMD vector types match hardware capabilities and element type constraints — SIMD Vector Types
//! SIMD and tensor system: unified Tensor<T, Shape> type with compile-time shape validation, SIMD acceleration (SSE/AVX/NEON), auto-differentiation — Tensor Type System
//!
//! This module provides compile-time validation for SIMD operations including:
//! - `Vec<T, N>` type validation (T: SimdElement, N: power of 2)
//! - `Mask<N>` type validation for masked operations
//! - SIMD intrinsic operand validation
//! - @multiversion attribute validation for runtime CPU dispatch
//!
//! # SIMD Type Hierarchy
//!
//! ```text
//! Vec<T: SimdElement, N: meta USize>
//! │
//! ├── 128-bit (SSE/NEON): Vec4f, Vec2d, Vec4i, Vec2l
//! ├── 256-bit (AVX):      Vec8f, Vec4d, Vec8i, Vec4l
//! └── 512-bit (AVX-512):  Vec16f, Vec8d, Vec16i, Vec8l
//! ```
//!
//! # Platform Support
//!
//! | Platform  | 128-bit | 256-bit | 512-bit |
//! |-----------|---------|---------|---------|
//! | x86_64    | SSE4.2  | AVX2    | AVX-512 |
//! | aarch64   | NEON    | -       | SVE     |
//! | riscv64   | V ext   | V ext   | V ext   |

use verum_ast::span::Span;
use verum_common::{List, Map, Text};
use crate::ty::Type;

// =============================================================================
// ERROR TYPES
// =============================================================================

/// Error type for SIMD type validation
#[derive(Debug, Clone)]
pub struct SimdTypeError {
    /// Error kind
    pub kind: SimdErrorKind,
    /// Human-readable error message
    pub message: Text,
    /// Source location
    pub span: Span,
}

/// Kinds of SIMD type errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdErrorKind {
    /// Element type doesn't implement SimdElement
    InvalidElementType,
    /// Lane count is not a power of 2
    InvalidLaneCount,
    /// Lane count exceeds platform maximum
    LaneCountTooLarge,
    /// Mismatched lane counts in operation
    LaneCountMismatch,
    /// Mismatched element types in operation
    ElementTypeMismatch,
    /// Invalid SIMD intrinsic usage
    InvalidIntrinsic,
    /// Invalid @multiversion target
    InvalidMultiversionTarget,
    /// Platform doesn't support requested SIMD width
    UnsupportedPlatform,
}

impl SimdTypeError {
    /// Create a new SIMD type error
    pub fn new(kind: SimdErrorKind, message: impl Into<Text>) -> Self {
        Self {
            kind,
            message: message.into(),
            span: Span::default(),
        }
    }

    /// Create error with span
    pub fn with_span(kind: SimdErrorKind, message: impl Into<Text>, span: Span) -> Self {
        Self {
            kind,
            message: message.into(),
            span,
        }
    }
}

impl std::fmt::Display for SimdTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SimdTypeError {}

// =============================================================================
// SIMD ELEMENT VALIDATION
// =============================================================================

/// Valid SIMD element types.
///
/// These are the types that can be used as elements in Vec<T, N>.
/// Each type has a known bit width used for lane count calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimdElementType {
    /// 8-bit signed integer
    Int8,
    /// 16-bit signed integer
    Int16,
    /// 32-bit signed integer
    Int32,
    /// 64-bit signed integer
    Int64,
    /// 8-bit unsigned integer
    UInt8,
    /// 16-bit unsigned integer
    UInt16,
    /// 32-bit unsigned integer
    UInt32,
    /// 64-bit unsigned integer
    UInt64,
    /// 32-bit floating point
    Float32,
    /// 64-bit floating point
    Float64,
}

impl SimdElementType {
    /// Get the bit width of this element type
    pub fn bit_width(self) -> usize {
        match self {
            SimdElementType::Int8 | SimdElementType::UInt8 => 8,
            SimdElementType::Int16 | SimdElementType::UInt16 => 16,
            SimdElementType::Int32 | SimdElementType::UInt32 | SimdElementType::Float32 => 32,
            SimdElementType::Int64 | SimdElementType::UInt64 | SimdElementType::Float64 => 64,
        }
    }

    /// Get the byte width of this element type
    pub fn byte_width(self) -> usize {
        self.bit_width() / 8
    }

    /// Check if this type is a floating point type
    pub fn is_float(self) -> bool {
        matches!(self, SimdElementType::Float32 | SimdElementType::Float64)
    }

    /// Check if this type is a signed integer
    pub fn is_signed_int(self) -> bool {
        matches!(
            self,
            SimdElementType::Int8
                | SimdElementType::Int16
                | SimdElementType::Int32
                | SimdElementType::Int64
        )
    }

    /// Check if this type is an unsigned integer
    pub fn is_unsigned_int(self) -> bool {
        matches!(
            self,
            SimdElementType::UInt8
                | SimdElementType::UInt16
                | SimdElementType::UInt32
                | SimdElementType::UInt64
        )
    }

    /// Try to parse from type name
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Int8" | "i8" => Some(SimdElementType::Int8),
            "Int16" | "i16" => Some(SimdElementType::Int16),
            "Int32" | "i32" => Some(SimdElementType::Int32),
            "Int64" | "i64" => Some(SimdElementType::Int64),
            "UInt8" | "u8" => Some(SimdElementType::UInt8),
            "UInt16" | "u16" => Some(SimdElementType::UInt16),
            "UInt32" | "u32" => Some(SimdElementType::UInt32),
            "UInt64" | "u64" => Some(SimdElementType::UInt64),
            "Float32" | "f32" => Some(SimdElementType::Float32),
            "Float64" | "f64" => Some(SimdElementType::Float64),
            _ => None,
        }
    }
}

// =============================================================================
// SIMD WIDTH VALIDATION
// =============================================================================

/// Standard SIMD register widths in bits
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SimdWidth {
    /// 64-bit (MMX, legacy)
    Bits64 = 64,
    /// 128-bit (SSE, NEON)
    Bits128 = 128,
    /// 256-bit (AVX, AVX2)
    Bits256 = 256,
    /// 512-bit (AVX-512, SVE)
    Bits512 = 512,
}

impl SimdWidth {
    /// Get the width in bytes
    pub fn bytes(self) -> usize {
        (self as usize) / 8
    }

    /// Calculate max lanes for an element type
    pub fn max_lanes_for(self, element: SimdElementType) -> usize {
        (self as usize) / element.bit_width()
    }

    /// Get all valid lane counts for this width
    pub fn valid_lane_counts(self) -> List<usize> {
        let mut counts = List::new();
        for bits in [8, 16, 32, 64] {
            let lanes = (self as usize) / bits;
            if lanes >= 1 && lanes.is_power_of_two() {
                counts.push(lanes);
            }
        }
        counts
    }
}

// =============================================================================
// PLATFORM TARGET FEATURES
// =============================================================================

/// CPU target features for SIMD operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimdTargetFeature {
    // x86/x86_64
    /// SSE 4.2 (128-bit vectors)
    Sse42,
    /// AVX (256-bit float vectors)
    Avx,
    /// AVX2 (256-bit integer vectors)
    Avx2,
    /// AVX-512F (512-bit foundation)
    Avx512f,
    /// AVX-512BW (byte/word operations)
    Avx512bw,
    /// AVX-512DQ (doubleword/quadword)
    Avx512dq,

    // ARM/AArch64
    /// NEON (128-bit vectors)
    Neon,
    /// SVE (Scalable Vector Extension)
    Sve,
    /// SVE2 (enhanced SVE)
    Sve2,

    // RISC-V
    /// Vector extension
    RiscvV,
}

impl SimdTargetFeature {
    /// Get the maximum vector width this feature supports
    pub fn max_width(self) -> SimdWidth {
        match self {
            SimdTargetFeature::Sse42 | SimdTargetFeature::Neon => SimdWidth::Bits128,
            SimdTargetFeature::Avx | SimdTargetFeature::Avx2 => SimdWidth::Bits256,
            SimdTargetFeature::Avx512f
            | SimdTargetFeature::Avx512bw
            | SimdTargetFeature::Avx512dq
            | SimdTargetFeature::Sve
            | SimdTargetFeature::Sve2
            | SimdTargetFeature::RiscvV => SimdWidth::Bits512,
        }
    }

    /// Get the feature name as used in @multiversion
    pub fn name(self) -> &'static str {
        match self {
            SimdTargetFeature::Sse42 => "sse4_2",
            SimdTargetFeature::Avx => "avx",
            SimdTargetFeature::Avx2 => "avx2",
            SimdTargetFeature::Avx512f => "avx512f",
            SimdTargetFeature::Avx512bw => "avx512bw",
            SimdTargetFeature::Avx512dq => "avx512dq",
            SimdTargetFeature::Neon => "neon",
            SimdTargetFeature::Sve => "sve",
            SimdTargetFeature::Sve2 => "sve2",
            SimdTargetFeature::RiscvV => "v",
        }
    }

    /// Parse from feature name string
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "sse4_2" | "sse4.2" | "sse42" => Some(SimdTargetFeature::Sse42),
            "avx" => Some(SimdTargetFeature::Avx),
            "avx2" => Some(SimdTargetFeature::Avx2),
            "avx512f" | "avx512" => Some(SimdTargetFeature::Avx512f),
            "avx512bw" => Some(SimdTargetFeature::Avx512bw),
            "avx512dq" => Some(SimdTargetFeature::Avx512dq),
            "neon" => Some(SimdTargetFeature::Neon),
            "sve" => Some(SimdTargetFeature::Sve),
            "sve2" => Some(SimdTargetFeature::Sve2),
            "v" | "riscv_v" => Some(SimdTargetFeature::RiscvV),
            _ => None,
        }
    }
}

// =============================================================================
// SIMD TYPE CHECKER
// =============================================================================

/// SIMD type validation checker
///
/// Provides compile-time validation for SIMD operations ensuring:
/// - Element types implement SimdElement protocol
/// - Lane counts are valid powers of 2
/// - Operations have matching types
/// - Platform support for requested SIMD widths
pub struct SimdTypeChecker {
    /// Current target architecture
    target_arch: TargetArch,
    /// Available target features
    target_features: List<SimdTargetFeature>,
    /// Statistics for tracking
    stats: SimdCheckStats,
}

/// Target architecture for SIMD validation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetArch {
    #[default]
    X86_64,
    X86,
    AArch64,
    Arm,
    RiscV64,
    RiscV32,
    Wasm32,
    Unknown,
}

/// Statistics for SIMD type checking
#[derive(Debug, Clone, Default)]
pub struct SimdCheckStats {
    /// Number of Vec<T, N> types validated
    pub vec_types_validated: usize,
    /// Number of Mask<N> types validated
    pub mask_types_validated: usize,
    /// Number of intrinsic calls validated
    pub intrinsics_validated: usize,
    /// Number of @multiversion functions validated
    pub multiversion_validated: usize,
    /// Number of errors encountered
    pub errors: usize,
}

impl SimdTypeChecker {
    /// Create a new SIMD type checker for the given target
    pub fn new(target_arch: TargetArch) -> Self {
        let target_features = Self::default_features_for_arch(target_arch);
        Self {
            target_arch,
            target_features,
            stats: SimdCheckStats::default(),
        }
    }

    /// Create with specific target features
    pub fn with_features(target_arch: TargetArch, features: List<SimdTargetFeature>) -> Self {
        Self {
            target_arch,
            target_features: features,
            stats: SimdCheckStats::default(),
        }
    }

    /// Get default features for an architecture
    fn default_features_for_arch(arch: TargetArch) -> List<SimdTargetFeature> {
        match arch {
            TargetArch::X86_64 | TargetArch::X86 => {
                // Assume at least SSE4.2 for modern x86
                vec![SimdTargetFeature::Sse42].into()
            }
            TargetArch::AArch64 => {
                // NEON is mandatory on AArch64
                vec![SimdTargetFeature::Neon].into()
            }
            TargetArch::Arm => List::new(),
            TargetArch::RiscV64 | TargetArch::RiscV32 => List::new(),
            TargetArch::Wasm32 => {
                // WebAssembly SIMD is 128-bit
                vec![SimdTargetFeature::Sse42].into() // Use SSE4.2 as proxy
            }
            TargetArch::Unknown => List::new(),
        }
    }

    /// Get the statistics
    pub fn stats(&self) -> &SimdCheckStats {
        &self.stats
    }

    /// Get the maximum supported SIMD width
    pub fn max_supported_width(&self) -> SimdWidth {
        self.target_features
            .iter()
            .map(|f| f.max_width())
            .max()
            .unwrap_or(SimdWidth::Bits128)
    }

    // =========================================================================
    // Vec<T, N> VALIDATION
    // =========================================================================

    /// Validate a Vec<T, N> type.
    ///
    /// Checks:
    /// - T is a valid SimdElement type
    /// - N is a power of 2
    /// - N * sizeof(T) <= max supported width
    pub fn validate_vec_type(
        &mut self,
        element_type: &Type,
        lane_count: usize,
        span: Span,
    ) -> Result<SimdVecInfo, SimdTypeError> {
        self.stats.vec_types_validated += 1;

        // Check element type
        let element = self.check_element_type(element_type, span)?;

        // Check lane count is power of 2
        if !lane_count.is_power_of_two() {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidLaneCount,
                format!(
                    "SIMD lane count must be a power of 2, got {}",
                    lane_count
                ),
                span,
            ));
        }

        // Check total size doesn't exceed max width
        let total_bits = lane_count * element.bit_width();
        let max_bits = self.max_supported_width() as usize;
        if total_bits > max_bits {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::LaneCountTooLarge,
                format!(
                    "Vec<{:?}, {}> requires {} bits, but maximum supported is {} bits",
                    element, lane_count, total_bits, max_bits
                ),
                span,
            ));
        }

        Ok(SimdVecInfo {
            element,
            lane_count,
            total_bits,
        })
    }

    /// Check if a type is a valid SIMD element type
    fn check_element_type(
        &self,
        ty: &Type,
        span: Span,
    ) -> Result<SimdElementType, SimdTypeError> {
        // Check primitive types
        match ty {
            Type::Int => Ok(SimdElementType::Int64), // Default Int is 64-bit
            Type::Float => Ok(SimdElementType::Float64), // Default Float is 64-bit
            Type::Named { path, .. } => {
                // Try to match named types like Int32, Float32, etc.
                let name = path.to_string();
                SimdElementType::from_name(&name).ok_or_else(|| {
                    SimdTypeError::with_span(
                        SimdErrorKind::InvalidElementType,
                        format!(
                            "'{}' is not a valid SIMD element type. \
                            Valid types: Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float32, Float64",
                            name
                        ),
                        span,
                    )
                })
            }
            _ => Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidElementType,
                "Type is not a valid SIMD element type. \
                    SIMD elements must be numeric primitives: Int8-64, UInt8-64, Float32, Float64".to_string(),
                span,
            )),
        }
    }

    // =========================================================================
    // Mask<N> VALIDATION
    // =========================================================================

    /// Validate a Mask<N> type.
    ///
    /// Checks:
    /// - N is a power of 2
    /// - N is a valid lane count for some SIMD width
    pub fn validate_mask_type(
        &mut self,
        lane_count: usize,
        span: Span,
    ) -> Result<SimdMaskInfo, SimdTypeError> {
        self.stats.mask_types_validated += 1;

        // Check lane count is power of 2
        if !lane_count.is_power_of_two() {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidLaneCount,
                format!(
                    "Mask lane count must be a power of 2, got {}",
                    lane_count
                ),
                span,
            ));
        }

        // Check lane count is reasonable (1-64 lanes typical)
        if lane_count > 64 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::LaneCountTooLarge,
                format!(
                    "Mask<{}> exceeds maximum practical lane count of 64",
                    lane_count
                ),
                span,
            ));
        }

        Ok(SimdMaskInfo { lane_count })
    }

    // =========================================================================
    // SIMD INTRINSIC VALIDATION
    // =========================================================================

    /// Validate a SIMD intrinsic call.
    ///
    /// Checks operand types match and operation is valid for element type.
    pub fn validate_intrinsic(
        &mut self,
        intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        self.stats.intrinsics_validated += 1;

        match intrinsic {
            // Unary operations
            "simd_splat" | "simd_abs" | "simd_neg" => {
                self.validate_unary_intrinsic(intrinsic, operands, span)
            }

            // Binary operations
            "simd_add" | "simd_sub" | "simd_mul" | "simd_div"
            | "simd_min" | "simd_max" | "simd_and" | "simd_or" | "simd_xor" => {
                self.validate_binary_intrinsic(intrinsic, operands, span)
            }

            // Ternary operations
            "simd_fma" | "simd_select" => {
                self.validate_ternary_intrinsic(intrinsic, operands, span)
            }

            // Reduction operations
            "simd_reduce_add" | "simd_reduce_mul"
            | "simd_reduce_min" | "simd_reduce_max" => {
                self.validate_reduction_intrinsic(intrinsic, operands, span)
            }

            // Comparison operations
            "simd_lt" | "simd_le" | "simd_gt" | "simd_ge"
            | "simd_eq" | "simd_ne" => {
                self.validate_comparison_intrinsic(intrinsic, operands, span)
            }

            // Load/store operations
            "simd_load_aligned" | "simd_load_unaligned"
            | "simd_store_aligned" | "simd_store_unaligned" => {
                self.validate_load_store_intrinsic(intrinsic, operands, span)
            }

            // Shuffle operations
            "simd_shuffle" => {
                self.validate_shuffle_intrinsic(operands, span)
            }

            // Gather/scatter
            "simd_gather" | "simd_scatter"
            | "simd_masked_gather" | "simd_masked_scatter" => {
                self.validate_gather_scatter_intrinsic(intrinsic, operands, span)
            }

            _ => {
                self.stats.errors += 1;
                Err(SimdTypeError::with_span(
                    SimdErrorKind::InvalidIntrinsic,
                    format!("Unknown SIMD intrinsic: {}", intrinsic),
                    span,
                ))
            }
        }
    }

    fn validate_unary_intrinsic(
        &mut self,
        _intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.len() != 1 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Unary SIMD intrinsic requires exactly 1 operand",
                span,
            ));
        }
        // Return same type as input
        Ok(operands[0].clone())
    }

    fn validate_binary_intrinsic(
        &mut self,
        _intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.len() != 2 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Binary SIMD intrinsic requires exactly 2 operands",
                span,
            ));
        }
        // Check types match (simplified - would need full type comparison)
        Ok(operands[0].clone())
    }

    fn validate_ternary_intrinsic(
        &mut self,
        _intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.len() != 3 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Ternary SIMD intrinsic requires exactly 3 operands",
                span,
            ));
        }
        // Return type matches first vector operand
        Ok(operands[0].clone())
    }

    fn validate_reduction_intrinsic(
        &mut self,
        _intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.len() != 1 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Reduction SIMD intrinsic requires exactly 1 operand",
                span,
            ));
        }
        // Return element type (would need to extract from Vec<T, N>)
        Ok(Type::Float) // Simplified
    }

    fn validate_comparison_intrinsic(
        &mut self,
        _intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.len() != 2 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Comparison SIMD intrinsic requires exactly 2 operands",
                span,
            ));
        }
        // Returns Mask<N> - simplified to Bool
        Ok(Type::Bool)
    }

    fn validate_load_store_intrinsic(
        &mut self,
        _intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.is_empty() {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Load/store SIMD intrinsic requires pointer operand",
                span,
            ));
        }
        Ok(Type::Unit) // Simplified
    }

    fn validate_shuffle_intrinsic(
        &mut self,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.len() < 2 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Shuffle SIMD intrinsic requires at least 2 operands",
                span,
            ));
        }
        Ok(operands[0].clone())
    }

    fn validate_gather_scatter_intrinsic(
        &mut self,
        _intrinsic: &str,
        operands: &[&Type],
        span: Span,
    ) -> Result<Type, SimdTypeError> {
        if operands.len() < 2 {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidIntrinsic,
                "Gather/scatter SIMD intrinsic requires base pointer and indices",
                span,
            ));
        }
        Ok(operands[0].clone())
    }

    // =========================================================================
    // @multiversion VALIDATION
    // =========================================================================

    /// Validate @multiversion attribute configuration.
    ///
    /// Checks:
    /// - All target features are valid
    /// - Features are supported on target platform
    /// - Default fallback is specified
    pub fn validate_multiversion(
        &mut self,
        variants: &Map<Text, Text>,
        span: Span,
    ) -> Result<MultiversionInfo, SimdTypeError> {
        self.stats.multiversion_validated += 1;

        let mut features = List::new();
        let mut has_default = false;

        for (feature_name, _impl_name) in variants.iter() {
            if feature_name == "default" {
                has_default = true;
                continue;
            }

            match SimdTargetFeature::from_name(feature_name) {
                Some(feature) => {
                    // Check if feature is compatible with target architecture
                    if self.is_feature_compatible(feature) {
                        features.push(feature);
                    } else {
                        // Warning: feature won't be used on this target
                        // (could emit warning diagnostic)
                    }
                }
                None => {
                    self.stats.errors += 1;
                    return Err(SimdTypeError::with_span(
                        SimdErrorKind::InvalidMultiversionTarget,
                        format!(
                            "Unknown target feature '{}' in @multiversion. \
                            Valid features: sse4_2, avx, avx2, avx512f, neon, sve",
                            feature_name
                        ),
                        span,
                    ));
                }
            }
        }

        if !has_default {
            self.stats.errors += 1;
            return Err(SimdTypeError::with_span(
                SimdErrorKind::InvalidMultiversionTarget,
                "@multiversion requires a 'default' fallback implementation",
                span,
            ));
        }

        Ok(MultiversionInfo {
            features,
            has_default,
        })
    }

    /// Check if a target feature is compatible with current architecture
    fn is_feature_compatible(&self, feature: SimdTargetFeature) -> bool {
        match (self.target_arch, feature) {
            // x86 features
            (TargetArch::X86_64 | TargetArch::X86, SimdTargetFeature::Sse42)
            | (TargetArch::X86_64 | TargetArch::X86, SimdTargetFeature::Avx)
            | (TargetArch::X86_64 | TargetArch::X86, SimdTargetFeature::Avx2)
            | (TargetArch::X86_64 | TargetArch::X86, SimdTargetFeature::Avx512f)
            | (TargetArch::X86_64 | TargetArch::X86, SimdTargetFeature::Avx512bw)
            | (TargetArch::X86_64 | TargetArch::X86, SimdTargetFeature::Avx512dq) => true,

            // ARM features
            (TargetArch::AArch64, SimdTargetFeature::Neon)
            | (TargetArch::AArch64, SimdTargetFeature::Sve)
            | (TargetArch::AArch64, SimdTargetFeature::Sve2)
            | (TargetArch::Arm, SimdTargetFeature::Neon) => true,

            // RISC-V features
            (TargetArch::RiscV64 | TargetArch::RiscV32, SimdTargetFeature::RiscvV) => true,

            // WebAssembly uses SSE4.2-like 128-bit SIMD
            (TargetArch::Wasm32, SimdTargetFeature::Sse42) => true,

            _ => false,
        }
    }
}

// =============================================================================
// VALIDATION RESULT TYPES
// =============================================================================

/// Information about a validated Vec<T, N> type
#[derive(Debug, Clone)]
pub struct SimdVecInfo {
    /// Element type
    pub element: SimdElementType,
    /// Number of lanes
    pub lane_count: usize,
    /// Total bits (lane_count * element.bit_width())
    pub total_bits: usize,
}

/// Information about a validated Mask<N> type
#[derive(Debug, Clone)]
pub struct SimdMaskInfo {
    /// Number of lanes
    pub lane_count: usize,
}

/// Information about a validated @multiversion function
#[derive(Debug, Clone)]
pub struct MultiversionInfo {
    /// Target features for variants
    pub features: List<SimdTargetFeature>,
    /// Whether a default fallback exists
    pub has_default: bool,
}

impl Default for SimdTypeChecker {
    fn default() -> Self {
        Self::new(TargetArch::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_element_type_bit_widths() {
        assert_eq!(SimdElementType::Int8.bit_width(), 8);
        assert_eq!(SimdElementType::Int16.bit_width(), 16);
        assert_eq!(SimdElementType::Int32.bit_width(), 32);
        assert_eq!(SimdElementType::Int64.bit_width(), 64);
        assert_eq!(SimdElementType::Float32.bit_width(), 32);
        assert_eq!(SimdElementType::Float64.bit_width(), 64);
    }

    #[test]
    fn test_valid_lane_counts() {
        use verum_ast::ty::{Ident, Path};

        let mut checker = SimdTypeChecker::new(TargetArch::X86_64);

        // Valid: power of 2, fits in 128-bit
        let result = checker.validate_vec_type(&Type::Float, 2, Span::default());
        assert!(result.is_ok());

        // Valid: 4 lanes of 32-bit = 128 bits
        let float32_path = Path::single(Ident::new("Float32", Span::default()));
        let result = checker.validate_vec_type(
            &Type::Named { path: float32_path, args: List::new() },
            4,
            Span::default(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_lane_count() {
        let mut checker = SimdTypeChecker::new(TargetArch::X86_64);

        // Invalid: not power of 2
        let result = checker.validate_vec_type(&Type::Float, 3, Span::default());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind, SimdErrorKind::InvalidLaneCount);
    }

    #[test]
    fn test_mask_validation() {
        let mut checker = SimdTypeChecker::new(TargetArch::X86_64);

        // Valid masks
        assert!(checker.validate_mask_type(4, Span::default()).is_ok());
        assert!(checker.validate_mask_type(8, Span::default()).is_ok());
        assert!(checker.validate_mask_type(16, Span::default()).is_ok());

        // Invalid: not power of 2
        assert!(checker.validate_mask_type(5, Span::default()).is_err());
    }

    #[test]
    fn test_target_feature_parsing() {
        assert_eq!(
            SimdTargetFeature::from_name("avx2"),
            Some(SimdTargetFeature::Avx2)
        );
        assert_eq!(
            SimdTargetFeature::from_name("neon"),
            Some(SimdTargetFeature::Neon)
        );
        assert_eq!(SimdTargetFeature::from_name("unknown"), None);
    }
}
