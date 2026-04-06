//! # Intrinsic Type Signatures
//!
//! This module provides type signature validation for intrinsic functions,
//! including support for generic intrinsics with protocol bounds.
//!
//! ## Protocol Bounds
//!
//! Protocol bounds constrain generic type parameters:
//!
//! | Protocol | Types | Description |
//! |----------|-------|-------------|
//! | `Atomic` | U8, U16, U32, U64, Int, Bool, *const T, *mut T | Atomic operations |
//! | `Integer` | I8-I64, U8-U64, Int, ISize, USize | Integer arithmetic |
//! | `SignedInteger` | I8-I64, Int, ISize | Signed arithmetic |
//! | `UnsignedInteger` | U8-U64, USize | Unsigned arithmetic |
//! | `FloatingPoint` | F32, F64 | Floating-point math |
//!
//! ## Example
//!
//! ```ignore
//! // Generic atomic intrinsic with protocol bound
//! @intrinsic("atomic_load")
//! pub fn atomic_load<T: Atomic>(ptr: *const T, ordering: U8) -> T;
//! ```

use std::collections::HashMap;
use std::sync::LazyLock;

/// Protocol bounds for generic type parameters.
///
/// These constrain which types can be used with generic intrinsics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolBound {
    /// Types supporting atomic operations (1, 2, 4, or 8 bytes).
    /// Includes: U8, U16, U32, U64, Int, Bool, *const T, *mut T
    Atomic,

    /// All integer types.
    /// Includes: I8-I64, U8-U64, Int, ISize, USize
    Integer,

    /// Signed integer types only.
    /// Includes: I8-I64, Int, ISize
    SignedInteger,

    /// Unsigned integer types only.
    /// Includes: U8-U64, USize
    UnsignedInteger,

    /// Floating-point types.
    /// Includes: F32, F64
    FloatingPoint,

    /// Types with known size at compile time.
    Sized,

    /// Types that can be copied bitwise.
    Copy,

    /// No constraints.
    None,
}

impl ProtocolBound {
    /// Get the name of this protocol for diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            ProtocolBound::Atomic => "Atomic",
            ProtocolBound::Integer => "Integer",
            ProtocolBound::SignedInteger => "SignedInteger",
            ProtocolBound::UnsignedInteger => "UnsignedInteger",
            ProtocolBound::FloatingPoint => "FloatingPoint",
            ProtocolBound::Sized => "Sized",
            ProtocolBound::Copy => "Copy",
            ProtocolBound::None => "Any",
        }
    }

    /// Check if a type name satisfies this protocol bound.
    pub fn is_satisfied_by(self, type_name: &str) -> bool {
        match self {
            ProtocolBound::Atomic => matches!(
                type_name,
                "U8" | "UInt8" | "u8"
                    | "U16" | "UInt16" | "u16"
                    | "U32" | "UInt32" | "u32"
                    | "U64" | "UInt64" | "u64"
                    | "Int" | "i64"
                    | "Bool" | "bool"
            ) || type_name.starts_with("*const ")
                || type_name.starts_with("*mut "),

            ProtocolBound::Integer => matches!(
                type_name,
                "I8" | "Int8" | "i8"
                    | "I16" | "Int16" | "i16"
                    | "I32" | "Int32" | "i32"
                    | "I64" | "Int64" | "i64"
                    | "U8" | "UInt8" | "u8"
                    | "U16" | "UInt16" | "u16"
                    | "U32" | "UInt32" | "u32"
                    | "U64" | "UInt64" | "u64"
                    | "Int"
                    | "ISize"
                    | "USize"
                    | "isize"
                    | "usize"
            ),

            ProtocolBound::SignedInteger => matches!(
                type_name,
                "I8" | "Int8" | "i8"
                    | "I16" | "Int16" | "i16"
                    | "I32" | "Int32" | "i32"
                    | "I64" | "Int64" | "i64"
                    | "Int"
                    | "ISize"
                    | "isize"
            ),

            ProtocolBound::UnsignedInteger => matches!(
                type_name,
                "U8" | "UInt8" | "u8"
                    | "U16" | "UInt16" | "u16"
                    | "U32" | "UInt32" | "u32"
                    | "U64" | "UInt64" | "u64"
                    | "USize"
                    | "usize"
            ),

            ProtocolBound::FloatingPoint => {
                matches!(type_name, "F32" | "Float32" | "f32" | "F64" | "Float64" | "f64")
            }

            ProtocolBound::Sized | ProtocolBound::Copy | ProtocolBound::None => true,
        }
    }
}

/// Type parameter for generic intrinsics.
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// The type parameter name (e.g., "T").
    pub name: String,
    /// Protocol bounds for this parameter.
    pub bounds: Vec<ProtocolBound>,
}

impl TypeParam {
    /// Create a new type parameter with bounds.
    pub fn new(name: impl Into<String>, bounds: Vec<ProtocolBound>) -> Self {
        Self {
            name: name.into(),
            bounds,
        }
    }

    /// Create an unconstrained type parameter.
    pub fn unconstrained(name: impl Into<String>) -> Self {
        Self::new(name, vec![ProtocolBound::None])
    }

    /// Check if a type satisfies all bounds.
    pub fn is_satisfied_by(&self, type_name: &str) -> bool {
        self.bounds.iter().all(|bound| bound.is_satisfied_by(type_name))
    }
}

/// Type in an intrinsic signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntrinsicType {
    /// Concrete type by name (e.g., "U64", "Bool").
    Concrete(String),

    /// Type parameter (e.g., "T").
    TypeParam(String),

    /// Raw pointer to type (*const T).
    Ptr(Box<IntrinsicType>),

    /// Mutable raw pointer (*mut T).
    MutPtr(Box<IntrinsicType>),

    /// Tuple of types (T, U, V).
    Tuple(Vec<IntrinsicType>),

    /// Unit type ().
    Unit,
}

impl IntrinsicType {
    /// Create a concrete type.
    pub fn concrete(name: impl Into<String>) -> Self {
        IntrinsicType::Concrete(name.into())
    }

    /// Create a type parameter reference.
    pub fn param(name: impl Into<String>) -> Self {
        IntrinsicType::TypeParam(name.into())
    }

    /// Create a pointer type.
    pub fn ptr(inner: IntrinsicType) -> Self {
        IntrinsicType::Ptr(Box::new(inner))
    }

    /// Create a mutable pointer type.
    pub fn mut_ptr(inner: IntrinsicType) -> Self {
        IntrinsicType::MutPtr(Box::new(inner))
    }

    /// Create a tuple type.
    pub fn tuple(types: Vec<IntrinsicType>) -> Self {
        IntrinsicType::Tuple(types)
    }

    /// Format this type for display.
    pub fn display(&self) -> String {
        match self {
            IntrinsicType::Concrete(name) => name.clone(),
            IntrinsicType::TypeParam(name) => name.clone(),
            IntrinsicType::Ptr(inner) => format!("*const {}", inner.display()),
            IntrinsicType::MutPtr(inner) => format!("*mut {}", inner.display()),
            IntrinsicType::Tuple(types) => {
                let inner: Vec<_> = types.iter().map(|t| t.display()).collect();
                format!("({})", inner.join(", "))
            }
            IntrinsicType::Unit => "()".to_string(),
        }
    }

    /// Check if this type matches a concrete type name, given type parameter substitutions.
    pub fn matches(&self, type_name: &str, type_args: &HashMap<String, String>) -> bool {
        match self {
            IntrinsicType::Concrete(expected) => {
                normalize_type_name(expected) == normalize_type_name(type_name)
            }
            IntrinsicType::TypeParam(param) => {
                if let Some(actual) = type_args.get(param) {
                    normalize_type_name(actual) == normalize_type_name(type_name)
                } else {
                    // Type parameter not bound yet - allow anything
                    true
                }
            }
            IntrinsicType::Ptr(inner) => {
                if let Some(inner_type) = type_name.strip_prefix("*const ") {
                    inner.matches(inner_type, type_args)
                } else {
                    false
                }
            }
            IntrinsicType::MutPtr(inner) => {
                if let Some(inner_type) = type_name.strip_prefix("*mut ") {
                    inner.matches(inner_type, type_args)
                } else {
                    false
                }
            }
            IntrinsicType::Tuple(types) => {
                // Parse tuple type from string
                if type_name.starts_with('(') && type_name.ends_with(')') {
                    let inner = &type_name[1..type_name.len() - 1];
                    let parts: Vec<_> = split_tuple_types(inner);
                    if parts.len() != types.len() {
                        return false;
                    }
                    types
                        .iter()
                        .zip(parts.iter())
                        .all(|(t, p)| t.matches(p.trim(), type_args))
                } else {
                    false
                }
            }
            IntrinsicType::Unit => type_name == "()",
        }
    }
}

/// Normalize type names for comparison (handles Verum vs Rust naming).
fn normalize_type_name(name: &str) -> &str {
    match name {
        "u8" => "U8",
        "u16" => "U16",
        "u32" => "U32",
        "u64" => "U64",
        "i8" => "I8",
        "i16" => "I16",
        "i32" => "I32",
        "i64" => "I64",
        "f32" => "F32",
        "f64" => "F64",
        "bool" => "Bool",
        "isize" => "ISize",
        "usize" => "USize",
        "UInt8" => "U8",
        "UInt16" => "U16",
        "UInt32" => "U32",
        "UInt64" => "U64",
        "Int8" => "I8",
        "Int16" => "I16",
        "Int32" => "I32",
        "Int64" => "I64",
        "Float32" => "F32",
        "Float64" => "F64",
        _ => name,
    }
}

/// Split tuple types, handling nested tuples.
fn split_tuple_types(inner: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;

    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    if start < inner.len() {
        parts.push(&inner[start..]);
    }

    parts
}

/// Intrinsic function signature.
#[derive(Debug, Clone)]
pub struct IntrinsicSignature {
    /// Intrinsic name.
    pub name: String,
    /// Generic type parameters.
    pub type_params: Vec<TypeParam>,
    /// Parameter types.
    pub param_types: Vec<IntrinsicType>,
    /// Return type.
    pub return_type: IntrinsicType,
}

/// Error from signature validation.
#[derive(Debug, Clone)]
pub enum SignatureError {
    /// Wrong number of arguments.
    WrongArgCount {
        /// Expected number of arguments.
        expected: usize,
        /// Actual number of arguments provided.
        actual: usize,
    },
    /// Wrong argument type.
    WrongArgType {
        /// Argument index (0-based).
        index: usize,
        /// Expected type.
        expected: String,
        /// Actual type provided.
        actual: String,
    },
    /// Protocol bound not satisfied.
    ProtocolBoundNotSatisfied {
        /// Type parameter name.
        type_param: String,
        /// Actual type that was provided.
        actual_type: String,
        /// Protocol that was not satisfied.
        protocol: String,
    },
}

impl IntrinsicSignature {
    /// Create a new intrinsic signature.
    pub fn new(
        name: impl Into<String>,
        type_params: Vec<TypeParam>,
        param_types: Vec<IntrinsicType>,
        return_type: IntrinsicType,
    ) -> Self {
        Self {
            name: name.into(),
            type_params,
            param_types,
            return_type,
        }
    }

    /// Validate arguments against this signature.
    ///
    /// `arg_types` are the actual argument types as strings.
    /// `type_args` are the explicit type arguments provided at the call site.
    pub fn validate_args(
        &self,
        arg_types: &[String],
        type_args: &HashMap<String, String>,
    ) -> Result<(), SignatureError> {
        // Check argument count
        if arg_types.len() != self.param_types.len() {
            return Err(SignatureError::WrongArgCount {
                expected: self.param_types.len(),
                actual: arg_types.len(),
            });
        }

        // Build type parameter bindings from explicit type args and inferred from arguments
        let mut bindings = type_args.clone();

        // First pass: infer type parameters from arguments
        for (param_type, arg_type) in self.param_types.iter().zip(arg_types.iter()) {
            if let IntrinsicType::TypeParam(param_name) = param_type
                && !bindings.contains_key(param_name)
            {
                bindings.insert(param_name.clone(), arg_type.clone());
            }
        }

        // Check protocol bounds for type parameters
        for type_param in &self.type_params {
            if let Some(actual_type) = bindings.get(&type_param.name)
                && !type_param.is_satisfied_by(actual_type)
            {
                // Find the first unsatisfied bound
                for bound in &type_param.bounds {
                    if !bound.is_satisfied_by(actual_type) {
                        return Err(SignatureError::ProtocolBoundNotSatisfied {
                            type_param: type_param.name.clone(),
                            actual_type: actual_type.clone(),
                            protocol: bound.name().to_string(),
                        });
                    }
                }
            }
        }

        // Second pass: validate argument types
        for (i, (param_type, arg_type)) in
            self.param_types.iter().zip(arg_types.iter()).enumerate()
        {
            if !param_type.matches(arg_type, &bindings) {
                return Err(SignatureError::WrongArgType {
                    index: i,
                    expected: param_type.display(),
                    actual: arg_type.clone(),
                });
            }
        }

        Ok(())
    }
}

/// Global registry of intrinsic signatures.
pub static INTRINSIC_SIGNATURES: LazyLock<HashMap<&'static str, IntrinsicSignature>> =
    LazyLock::new(build_signature_registry);

/// Get the signature for an intrinsic by name.
pub fn get_signature(name: &str) -> Option<&'static IntrinsicSignature> {
    INTRINSIC_SIGNATURES.get(name)
}

/// Build the signature registry.
fn build_signature_registry() -> HashMap<&'static str, IntrinsicSignature> {
    let mut registry = HashMap::new();

    // === Atomic Operations (generic) ===
    registry.insert(
        "atomic_load",
        IntrinsicSignature::new(
            "atomic_load",
            vec![TypeParam::new("T", vec![ProtocolBound::Atomic])],
            vec![
                IntrinsicType::ptr(IntrinsicType::param("T")),
                IntrinsicType::concrete("U8"), // ordering
            ],
            IntrinsicType::param("T"),
        ),
    );

    registry.insert(
        "atomic_store",
        IntrinsicSignature::new(
            "atomic_store",
            vec![TypeParam::new("T", vec![ProtocolBound::Atomic])],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::param("T")),
                IntrinsicType::param("T"),
                IntrinsicType::concrete("U8"), // ordering
            ],
            IntrinsicType::Unit,
        ),
    );

    registry.insert(
        "atomic_cas",
        IntrinsicSignature::new(
            "atomic_cas",
            vec![TypeParam::new("T", vec![ProtocolBound::Atomic])],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::param("T")),
                IntrinsicType::param("T"), // expected
                IntrinsicType::param("T"), // desired
                IntrinsicType::concrete("U8"), // success ordering
                IntrinsicType::concrete("U8"), // failure ordering
            ],
            IntrinsicType::tuple(vec![
                IntrinsicType::param("T"),
                IntrinsicType::concrete("Bool"),
            ]),
        ),
    );

    registry.insert(
        "atomic_exchange",
        IntrinsicSignature::new(
            "atomic_exchange",
            vec![TypeParam::new("T", vec![ProtocolBound::Atomic])],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::param("T")),
                IntrinsicType::param("T"),
                IntrinsicType::concrete("U8"), // ordering
            ],
            IntrinsicType::param("T"),
        ),
    );

    registry.insert(
        "atomic_fetch_add",
        IntrinsicSignature::new(
            "atomic_fetch_add",
            vec![TypeParam::new("T", vec![ProtocolBound::Atomic, ProtocolBound::Integer])],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::param("T")),
                IntrinsicType::param("T"),
                IntrinsicType::concrete("U8"), // ordering
            ],
            IntrinsicType::param("T"),
        ),
    );

    registry.insert(
        "atomic_fetch_sub",
        IntrinsicSignature::new(
            "atomic_fetch_sub",
            vec![TypeParam::new("T", vec![ProtocolBound::Atomic, ProtocolBound::Integer])],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::param("T")),
                IntrinsicType::param("T"),
                IntrinsicType::concrete("U8"), // ordering
            ],
            IntrinsicType::param("T"),
        ),
    );

    // === Bit Operations (generic) ===
    registry.insert(
        "clz",
        IntrinsicSignature::new(
            "clz",
            vec![TypeParam::new("T", vec![ProtocolBound::Integer])],
            vec![IntrinsicType::param("T")],
            IntrinsicType::concrete("U32"),
        ),
    );

    registry.insert(
        "ctz",
        IntrinsicSignature::new(
            "ctz",
            vec![TypeParam::new("T", vec![ProtocolBound::Integer])],
            vec![IntrinsicType::param("T")],
            IntrinsicType::concrete("U32"),
        ),
    );

    registry.insert(
        "popcnt",
        IntrinsicSignature::new(
            "popcnt",
            vec![TypeParam::new("T", vec![ProtocolBound::Integer])],
            vec![IntrinsicType::param("T")],
            IntrinsicType::concrete("U32"),
        ),
    );

    registry.insert(
        "bswap",
        IntrinsicSignature::new(
            "bswap",
            vec![TypeParam::new("T", vec![ProtocolBound::Integer])],
            vec![IntrinsicType::param("T")],
            IntrinsicType::param("T"),
        ),
    );

    // === Float Math (F64) ===
    for name in [
        "sqrt_f64", "sin_f64", "cos_f64", "tan_f64", "asin_f64", "acos_f64", "atan_f64",
        "exp_f64", "log_f64", "log10_f64", "log2_f64", "floor_f64", "ceil_f64",
        "round_f64", "trunc_f64", "abs_f64", "cbrt_f64",
    ] {
        registry.insert(
            name,
            IntrinsicSignature::new(
                name,
                vec![],
                vec![IntrinsicType::concrete("F64")],
                IntrinsicType::concrete("F64"),
            ),
        );
    }

    // Two-argument F64 functions
    for name in ["pow_f64", "atan2_f64", "copysign_f64", "hypot_f64"] {
        registry.insert(
            name,
            IntrinsicSignature::new(
                name,
                vec![],
                vec![IntrinsicType::concrete("F64"), IntrinsicType::concrete("F64")],
                IntrinsicType::concrete("F64"),
            ),
        );
    }

    // FMA (three arguments)
    registry.insert(
        "fma_f64",
        IntrinsicSignature::new(
            "fma_f64",
            vec![],
            vec![
                IntrinsicType::concrete("F64"),
                IntrinsicType::concrete("F64"),
                IntrinsicType::concrete("F64"),
            ],
            IntrinsicType::concrete("F64"),
        ),
    );

    // === Float Math (F32) ===
    for name in [
        "sqrt_f32", "floor_f32", "ceil_f32", "round_f32", "trunc_f32",
    ] {
        registry.insert(
            name,
            IntrinsicSignature::new(
                name,
                vec![],
                vec![IntrinsicType::concrete("F32")],
                IntrinsicType::concrete("F32"),
            ),
        );
    }

    // === Memory Operations ===
    registry.insert(
        "memcpy",
        IntrinsicSignature::new(
            "memcpy",
            vec![],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::concrete("U8")),
                IntrinsicType::ptr(IntrinsicType::concrete("U8")),
                IntrinsicType::concrete("USize"),
            ],
            IntrinsicType::Unit,
        ),
    );

    registry.insert(
        "memset",
        IntrinsicSignature::new(
            "memset",
            vec![],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::concrete("U8")),
                IntrinsicType::concrete("U8"),
                IntrinsicType::concrete("USize"),
            ],
            IntrinsicType::Unit,
        ),
    );

    registry.insert(
        "memmove",
        IntrinsicSignature::new(
            "memmove",
            vec![],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::concrete("U8")),
                IntrinsicType::ptr(IntrinsicType::concrete("U8")),
                IntrinsicType::concrete("USize"),
            ],
            IntrinsicType::Unit,
        ),
    );

    registry.insert(
        "memcmp",
        IntrinsicSignature::new(
            "memcmp",
            vec![],
            vec![
                IntrinsicType::ptr(IntrinsicType::concrete("U8")),
                IntrinsicType::ptr(IntrinsicType::concrete("U8")),
                IntrinsicType::concrete("USize"),
            ],
            IntrinsicType::concrete("I32"),
        ),
    );

    // === Type Info ===
    registry.insert(
        "size_of",
        IntrinsicSignature::new(
            "size_of",
            vec![TypeParam::unconstrained("T")],
            vec![],
            IntrinsicType::concrete("USize"),
        ),
    );

    registry.insert(
        "align_of",
        IntrinsicSignature::new(
            "align_of",
            vec![TypeParam::unconstrained("T")],
            vec![],
            IntrinsicType::concrete("USize"),
        ),
    );

    // === Control Flow ===
    registry.insert(
        "panic",
        IntrinsicSignature::new(
            "panic",
            vec![],
            vec![IntrinsicType::ptr(IntrinsicType::concrete("Text"))],
            IntrinsicType::Unit, // Actually ! but we use Unit for simplicity
        ),
    );

    registry.insert(
        "unreachable",
        IntrinsicSignature::new(
            "unreachable",
            vec![],
            vec![],
            IntrinsicType::Unit, // Actually !
        ),
    );

    // === CBGR (Counter-Based Generational References) ===
    // These intrinsics support Verum's memory safety model.

    // Validate a CBGR reference against expected generation and epoch
    registry.insert(
        "cbgr_validate",
        IntrinsicSignature::new(
            "cbgr_validate",
            vec![TypeParam::unconstrained("T")],
            vec![
                IntrinsicType::ptr(IntrinsicType::param("T")), // reference
                IntrinsicType::concrete("U32"),                // expected_generation
                IntrinsicType::concrete("U16"),                // expected_epoch
            ],
            IntrinsicType::concrete("Bool"),
        ),
    );

    // Get the current global CBGR epoch
    registry.insert(
        "cbgr_current_epoch",
        IntrinsicSignature::new(
            "cbgr_current_epoch",
            vec![],
            vec![],
            IntrinsicType::concrete("U16"),
        ),
    );

    // Advance the global CBGR epoch (synchronization point)
    registry.insert(
        "cbgr_advance_epoch",
        IntrinsicSignature::new(
            "cbgr_advance_epoch",
            vec![],
            vec![],
            IntrinsicType::Unit,
        ),
    );

    // Get the generation counter from an allocation header
    registry.insert(
        "cbgr_get_generation",
        IntrinsicSignature::new(
            "cbgr_get_generation",
            vec![TypeParam::unconstrained("T")],
            vec![IntrinsicType::ptr(IntrinsicType::param("T"))],
            IntrinsicType::concrete("U32"),
        ),
    );

    // Get the epoch from an allocation header
    registry.insert(
        "cbgr_get_epoch",
        IntrinsicSignature::new(
            "cbgr_get_epoch",
            vec![TypeParam::unconstrained("T")],
            vec![IntrinsicType::ptr(IntrinsicType::param("T"))],
            IntrinsicType::concrete("U16"),
        ),
    );

    // Combined get of generation and epoch (optimized single load)
    registry.insert(
        "cbgr_get_gen_epoch",
        IntrinsicSignature::new(
            "cbgr_get_gen_epoch",
            vec![TypeParam::unconstrained("T")],
            vec![IntrinsicType::ptr(IntrinsicType::param("T"))],
            IntrinsicType::tuple(vec![
                IntrinsicType::concrete("U32"), // generation
                IntrinsicType::concrete("U16"), // epoch
            ]),
        ),
    );

    // CBGR-aware allocation returning (ptr, generation, epoch)
    registry.insert(
        "cbgr_alloc",
        IntrinsicSignature::new(
            "cbgr_alloc",
            vec![],
            vec![
                IntrinsicType::concrete("USize"), // size
                IntrinsicType::concrete("USize"), // alignment
            ],
            IntrinsicType::tuple(vec![
                IntrinsicType::mut_ptr(IntrinsicType::concrete("U8")), // ptr
                IntrinsicType::concrete("U32"),                        // generation
                IntrinsicType::concrete("U16"),                        // epoch
            ]),
        ),
    );

    // CBGR-aware deallocation with validation
    registry.insert(
        "cbgr_dealloc",
        IntrinsicSignature::new(
            "cbgr_dealloc",
            vec![TypeParam::unconstrained("T")],
            vec![
                IntrinsicType::mut_ptr(IntrinsicType::param("T")), // ptr
                IntrinsicType::concrete("U32"),                    // expected_generation
                IntrinsicType::concrete("U16"),                    // expected_epoch
            ],
            IntrinsicType::concrete("Bool"), // success
        ),
    );

    // Increment generation counter on deallocation (internal use)
    registry.insert(
        "cbgr_increment_generation",
        IntrinsicSignature::new(
            "cbgr_increment_generation",
            vec![TypeParam::unconstrained("T")],
            vec![IntrinsicType::mut_ptr(IntrinsicType::param("T"))],
            IntrinsicType::concrete("U32"), // new generation
        ),
    );

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_bound_atomic() {
        assert!(ProtocolBound::Atomic.is_satisfied_by("U32"));
        assert!(ProtocolBound::Atomic.is_satisfied_by("U64"));
        assert!(ProtocolBound::Atomic.is_satisfied_by("Bool"));
        assert!(ProtocolBound::Atomic.is_satisfied_by("*const U8"));
        assert!(ProtocolBound::Atomic.is_satisfied_by("*mut T"));
        assert!(!ProtocolBound::Atomic.is_satisfied_by("F64"));
    }

    #[test]
    fn test_protocol_bound_integer() {
        assert!(ProtocolBound::Integer.is_satisfied_by("I32"));
        assert!(ProtocolBound::Integer.is_satisfied_by("U64"));
        assert!(ProtocolBound::Integer.is_satisfied_by("Int"));
        assert!(!ProtocolBound::Integer.is_satisfied_by("F64"));
    }

    #[test]
    fn test_signature_validation() {
        let sig = get_signature("atomic_load").unwrap();

        // Valid call
        let mut type_args = HashMap::new();
        type_args.insert("T".to_string(), "U64".to_string());

        let args = vec!["*const U64".to_string(), "U8".to_string()];
        assert!(sig.validate_args(&args, &type_args).is_ok());
    }

    #[test]
    fn test_wrong_arg_count() {
        let sig = get_signature("atomic_load").unwrap();
        let args = vec!["*const U64".to_string()]; // Missing ordering

        match sig.validate_args(&args, &HashMap::new()) {
            Err(SignatureError::WrongArgCount { expected: 2, actual: 1 }) => {}
            other => panic!("Expected WrongArgCount, got {:?}", other),
        }
    }

    #[test]
    fn test_protocol_bound_violation() {
        let sig = get_signature("atomic_fetch_add").unwrap();

        let mut type_args = HashMap::new();
        type_args.insert("T".to_string(), "F64".to_string()); // F64 doesn't satisfy Atomic

        let args = vec!["*mut F64".to_string(), "F64".to_string(), "U8".to_string()];

        match sig.validate_args(&args, &type_args) {
            Err(SignatureError::ProtocolBoundNotSatisfied { protocol, .. }) => {
                assert!(protocol == "Atomic" || protocol == "Integer");
            }
            other => panic!("Expected ProtocolBoundNotSatisfied, got {:?}", other),
        }
    }

    #[test]
    fn test_cbgr_validate_signature() {
        let sig = get_signature("cbgr_validate").unwrap();
        assert_eq!(sig.param_types.len(), 3);
        assert_eq!(sig.return_type, IntrinsicType::concrete("Bool"));
    }

    #[test]
    fn test_cbgr_current_epoch_signature() {
        let sig = get_signature("cbgr_current_epoch").unwrap();
        assert_eq!(sig.param_types.len(), 0);
        assert_eq!(sig.return_type, IntrinsicType::concrete("U16"));
    }

    #[test]
    fn test_cbgr_alloc_signature() {
        let sig = get_signature("cbgr_alloc").unwrap();
        assert_eq!(sig.param_types.len(), 2);
        // Returns tuple (ptr, generation, epoch)
        match &sig.return_type {
            IntrinsicType::Tuple(types) => assert_eq!(types.len(), 3),
            _ => panic!("Expected tuple return type"),
        }
    }

    #[test]
    fn test_cbgr_dealloc_signature() {
        let sig = get_signature("cbgr_dealloc").unwrap();
        assert_eq!(sig.param_types.len(), 3);
        assert_eq!(sig.return_type, IntrinsicType::concrete("Bool"));
    }
}
