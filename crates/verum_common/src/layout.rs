//! Single source of truth for Verum type-layout constants.
//!
//! All compiler crates that need to answer the question "how many bytes
//! does a value of type `T` occupy at runtime?" consult this module —
//! the typechecker (`@sizeof` / stack-allocation budgets), the MIR
//! lowering (TypeProperty construction), the `@const` / `@property`
//! meta evaluators, and downstream codegen passes.
//!
//! Why a single module? CBGR's memory model has three reference tiers
//! with different on-stack footprints. Each tier's choice — **ThinRef
//! is 16 bytes**, **raw `&unsafe` pointers are 8 bytes** — is load-
//! bearing for stack-allocation analysis, escape analysis, and the
//! `@sizeof` reflection surface. Pre-this-module, those constants were
//! duplicated across four files with at least one site silently
//! disagreeing (the type-checker's `calculate_type_size` returned 8
//! for ALL reference tiers, including Tier-0/Tier-1, contradicting
//! the CBGR design and `type_props`'s 16). A misclassification flowed
//! into `@sizeof` and stack-budget violations.
//!
//! Every constant here is a mirror of an architectural decision
//! documented in `docs/detailed/26-cbgr-implementation.md` and
//! `docs/architecture/no-libc-architecture.md`. Editing a value here
//! is a load-bearing change requiring matching updates to the runtime
//! header layout in `verum_common::cbgr` and the LLVM lowering in
//! `verum_codegen::llvm::types`.

// ============================================================================
// CBGR reference layout
// ============================================================================

/// Default raw-pointer width (Verum currently targets 64-bit only).
///
/// Used for: function pointers, `*const T` / `*mut T`, untyped void
/// pointers in FFI lowering, plus the per-cell building block of the
/// fat pointer / ThinRef compounds below.
pub const POINTER_SIZE: u64 = 8;

/// Size of a CBGR Tier-0 / Tier-1 reference (`&T` and `&checked T`).
///
/// ```text
///     ThinRef<T> = { ptr: *T, generation: u32, epoch_caps: u32 }
/// ```
///
/// 16 bytes total. Both Tier 0 (default, runtime-validated) and Tier 1
/// (compiler-proven safe via escape analysis) share the same on-stack
/// representation — Tier 1 simply skips the validation read at runtime.
/// **Drift contract:** the runtime header layout in
/// `verum_common::cbgr::ThinRef` MUST agree with this constant; the
/// `cbgr_layout_invariants` test in this file pins the equality.
pub const THIN_REF_SIZE: u64 = 16;

/// Size of a CBGR FatRef (slice / dynamic-bound reference).
///
/// ```text
///     FatRef<T> = ThinRef<T> + { metadata: 8, offset: 4, reserved: 4 }
/// ```
///
/// 32 bytes total. Used for slice references where additional length /
/// stride / vtable metadata is required alongside the base pointer.
pub const FAT_REF_SIZE: u64 = 32;

/// Size of a Tier-0 reference (`&T`). Alias of [`THIN_REF_SIZE`].
pub const REF_TIER0_SIZE: u64 = THIN_REF_SIZE;

/// Size of a Tier-1 reference (`&checked T`). Alias of [`THIN_REF_SIZE`]
/// — Tier 1 retains the same on-stack layout as Tier 0 to allow
/// transparent reuse of CBGR-validated callees; only the runtime
/// validation read is elided.
pub const REF_TIER1_SIZE: u64 = THIN_REF_SIZE;

/// Size of a Tier-2 reference (`&unsafe T`).
///
/// Tier 2 references opt out of CBGR validation entirely and lower to
/// a raw pointer — 8 bytes, no generation / epoch metadata. The unsafe
/// boundary requires the user to discharge soundness manually.
pub const REF_TIER2_SIZE: u64 = POINTER_SIZE;

/// Slice fat-pointer width (`{ ptr, len }`).
pub const SLICE_FAT_PTR_SIZE: u64 = POINTER_SIZE * 2;

// ============================================================================
// Built-in scalar layouts
// ============================================================================

/// `Bool` width (1 byte).
pub const BOOL_SIZE: u64 = 1;

/// `Char` width (4 bytes — Unicode scalar value, UTF-32 storage).
pub const CHAR_SIZE: u64 = 4;

/// Default `Int` width on the 64-bit target (8 bytes).
///
/// Width-tagged variants (`Int8`/`Int16`/...) override this — see
/// [`primitive_size_by_name`].
pub const INT_SIZE: u64 = 8;

/// Default `Float` width (8 bytes — IEEE 754 binary64).
pub const FLOAT_SIZE: u64 = 8;

/// `Text` value layout (`{ ptr, len, capacity }`).
///
/// Text is a value-typed UTF-8 string buffer with capacity tracking;
/// the on-stack footprint mirrors a triple of pointer-sized words.
pub const TEXT_SIZE: u64 = POINTER_SIZE * 3;

// ============================================================================
// Primitive lookup
// ============================================================================

/// Resolve a primitive type's runtime size by its canonical Verum name.
///
/// Returns `Some(size_in_bytes)` for any primitive type Verum models —
/// scalars (`Int` / `Float` / `Bool` / `Char` / `Unit` / `Never`),
/// width-tagged numerics (`Int8` … `Int128`, `UInt8` … `UInt128`,
/// `IntSize`, `USize`, `Float32`, `Float64`), the legacy uppercase-
/// short forms (`I8` … `I128`, `U8` … `U128`, `F32`, `F64`, `Usize`,
/// `Isize`), the Rust-style lowercase aliases (`i8` … `i128`, `u8` …
/// `u128`, `usize`, `isize`, `f32`, `f64`), and `Text`.
///
/// Returns `None` for compound types (records, sum types, generic
/// applications, function types, references) — those go through
/// shape-aware size computation in the typechecker / MIR lowering.
///
/// **Single source of truth:** every primitive width that appears in
/// the compiler reads through this function.
pub fn primitive_size_by_name(name: &str) -> Option<u64> {
    match name {
        // Unit-like — no payload
        "Unit" | "()" | "Never" => Some(0),

        // Boolean
        "Bool" | "bool" => Some(BOOL_SIZE),

        // Character
        "Char" | "char" => Some(CHAR_SIZE),

        // Default-width numerics (target-pointer-width)
        "Int" | "Float" => Some(INT_SIZE),

        // Width-tagged 1-byte
        "Int8" | "UInt8" | "Byte" | "I8" | "U8" | "i8" | "u8" => Some(1),

        // Width-tagged 2-byte
        "Int16" | "UInt16" | "I16" | "U16" | "i16" | "u16" => Some(2),

        // Width-tagged 4-byte
        "Int32" | "UInt32" | "Float32" | "I32" | "U32" | "F32" | "i32" | "u32" | "f32" => Some(4),

        // Width-tagged 8-byte (incl. canonical Int64/UInt64/Float64,
        // pointer-sized Int/UInt aliases, and lowercase forms)
        "Int64" | "UInt64" | "Float64" | "IntSize" | "USize" | "UIntSize"
        | "I64" | "U64" | "F64" | "Usize" | "Isize"
        | "i64" | "u64" | "f64" | "isize" | "usize" => Some(POINTER_SIZE),

        // Width-tagged 16-byte
        "Int128" | "UInt128" | "I128" | "U128" | "i128" | "u128" => Some(16),

        // Text — value-typed string buffer
        "Text" => Some(TEXT_SIZE),

        // Compound or unknown
        _ => None,
    }
}

/// Resolve the alignment of a primitive type by its canonical Verum
/// name. For width-tagged scalars the alignment equals the width;
/// `Text` and other heap-backed values align to the pointer width.
pub fn primitive_alignment_by_name(name: &str) -> Option<u64> {
    match name {
        "Unit" | "()" | "Never" => Some(1),
        // Heap-backed value types align to pointer width (the first
        // field of the layout is a pointer).
        "Text" => Some(POINTER_SIZE),
        // Everything else: alignment == width.
        _ => primitive_size_by_name(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CBGR layout invariants — these constants are load-bearing for
    /// the reference-tier semantics and the runtime header layout.
    /// Drifting any of them is a non-trivial design change requiring
    /// updates to `verum_common::cbgr`, the LLVM lowering, and the
    /// reference-tier docs.
    #[test]
    fn cbgr_layout_invariants() {
        assert_eq!(POINTER_SIZE, 8, "Verum targets 64-bit only");
        assert_eq!(THIN_REF_SIZE, 16, "ThinRef = ptr + gen + epoch_caps");
        assert_eq!(FAT_REF_SIZE, 32, "FatRef = ThinRef + 16 bytes metadata");
        assert_eq!(REF_TIER0_SIZE, THIN_REF_SIZE);
        assert_eq!(REF_TIER1_SIZE, THIN_REF_SIZE);
        assert_eq!(REF_TIER2_SIZE, POINTER_SIZE);
        assert_eq!(SLICE_FAT_PTR_SIZE, 16);
        assert_eq!(TEXT_SIZE, 24);
    }

    /// All primitive scalars resolve to non-None sizes via the
    /// canonical names recognized by `well_known_types::type_names`.
    #[test]
    fn canonical_primitives_have_size() {
        for n in [
            "Bool", "Char", "Int", "Float", "Text", "Unit", "Never",
            "Int8", "Int16", "Int32", "Int64", "Int128", "IntSize",
            "UInt8", "UInt16", "UInt32", "UInt64", "UInt128", "USize",
            "Float32", "Float64", "Byte",
        ] {
            assert!(
                primitive_size_by_name(n).is_some(),
                "primitive '{}' must have a known size",
                n
            );
        }
    }

    /// Width-tagged numerics align with their declared widths.
    #[test]
    fn width_tagged_numeric_widths() {
        assert_eq!(primitive_size_by_name("Int8"), Some(1));
        assert_eq!(primitive_size_by_name("Int16"), Some(2));
        assert_eq!(primitive_size_by_name("Int32"), Some(4));
        assert_eq!(primitive_size_by_name("Int64"), Some(8));
        assert_eq!(primitive_size_by_name("Int128"), Some(16));
        assert_eq!(primitive_size_by_name("UInt8"), Some(1));
        assert_eq!(primitive_size_by_name("UInt16"), Some(2));
        assert_eq!(primitive_size_by_name("UInt32"), Some(4));
        assert_eq!(primitive_size_by_name("UInt64"), Some(8));
        assert_eq!(primitive_size_by_name("UInt128"), Some(16));
        assert_eq!(primitive_size_by_name("Float32"), Some(4));
        assert_eq!(primitive_size_by_name("Float64"), Some(8));
        assert_eq!(primitive_size_by_name("Byte"), Some(1));
        assert_eq!(primitive_size_by_name("USize"), Some(8));
        assert_eq!(primitive_size_by_name("IntSize"), Some(8));
    }

    /// Legacy uppercase-short and Rust-lowercase aliases agree with
    /// their canonical counterparts. Drift here would silently change
    /// `@sizeof` answers between source spelling forms.
    #[test]
    fn primitive_alias_consistency() {
        // Canonical ↔ Verum-uppercase-short ↔ Rust-lowercase matrix.
        let table: &[(&str, &str, &str)] = &[
            ("Int8",   "I8",   "i8"),
            ("Int16",  "I16",  "i16"),
            ("Int32",  "I32",  "i32"),
            ("Int64",  "I64",  "i64"),
            ("UInt8",  "U8",   "u8"),
            ("UInt16", "U16",  "u16"),
            ("UInt32", "U32",  "u32"),
            ("UInt64", "U64",  "u64"),
            ("Float32","F32",  "f32"),
            ("Float64","F64",  "f64"),
            ("USize",  "Usize","usize"),
            // IntSize -> Isize -> isize
            ("IntSize","Isize","isize"),
        ];
        for &(canon, short, lower) in table {
            let c = primitive_size_by_name(canon);
            let s = primitive_size_by_name(short);
            let l = primitive_size_by_name(lower);
            assert!(c.is_some(), "canonical '{}' missing", canon);
            assert_eq!(c, s, "short alias '{}' disagrees with '{}'", short, canon);
            assert_eq!(c, l, "lower alias '{}' disagrees with '{}'", lower, canon);
        }
    }

    /// Compound / non-primitive names return None.
    #[test]
    fn compound_types_return_none() {
        for n in ["List", "Map", "Set", "Maybe", "Result", "MyType", "T"] {
            assert_eq!(
                primitive_size_by_name(n),
                None,
                "'{}' should not be classified as a primitive",
                n
            );
        }
    }

    /// Alignment matches width for fixed scalars; pointer-sized for
    /// heap-backed values.
    #[test]
    fn primitive_alignment_rules() {
        assert_eq!(primitive_alignment_by_name("Int8"), Some(1));
        assert_eq!(primitive_alignment_by_name("Int16"), Some(2));
        assert_eq!(primitive_alignment_by_name("Int32"), Some(4));
        assert_eq!(primitive_alignment_by_name("Int64"), Some(8));
        assert_eq!(primitive_alignment_by_name("Int"), Some(8));
        assert_eq!(primitive_alignment_by_name("Float"), Some(8));
        assert_eq!(primitive_alignment_by_name("Bool"), Some(1));
        assert_eq!(primitive_alignment_by_name("Char"), Some(4));
        assert_eq!(primitive_alignment_by_name("Unit"), Some(1));
        // Text aligns to pointer, not to its 24-byte total width.
        assert_eq!(primitive_alignment_by_name("Text"), Some(POINTER_SIZE));
    }
}
