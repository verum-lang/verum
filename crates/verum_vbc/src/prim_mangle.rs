//! **ONE authority for primitive width-semantic method mangling** (T0695).
//!
//! # The contract
//!
//! The uniform-i64 register model erases integer widths at runtime, so a
//! method whose SEMANTICS depend on the receiver's declared width
//! (`Byte.saturating_add` must ceil at 255, not i64::MAX) cannot dispatch
//! through the ordinary `Type.method` surface: the runtime CallM
//! dispatcher strips the type prefix and routes by the receiver's runtime
//! kind (`Int`), landing on the 64-bit implementation. For exactly that
//! set of methods, codegen mangles the call to `<prefix>$<method>`
//! (`byte$saturating_add`) and the interpreter's primitive intercept
//! carries the width-correct implementation.
//!
//! # The defect class this module closes
//!
//! Pre-T0695 the codegen mangled **every** method on a width-typed
//! receiver while the interpreter intercepted only the width-semantic
//! subset — two hand-maintained lists drifting by construction. Anything
//! outside the subset (`Byte.to_hex`, `escape_debug`, …) became an
//! unresolvable mangled name and died at dispatch with
//! `method 'byte$to_hex' not found` (live: the CBGR by-example page).
//!
//! Now:
//! * codegen consults [`mangled`] — a member mangles, a NON-member
//!   compiles as the alias-canonical qualified name
//!   (`UInt8.to_hex`), which is how the method is actually registered;
//! * the interpreter consults [`demangle`] on a mangled miss and retries
//!   the canonical qualified name — old bytecode (baked archives carry
//!   pre-T0695 mangles) stays dispatchable;
//! * the `paired_lists_cover_interp_arms` test scans the interpreter
//!   source so an intercept arm can never exist outside these tables
//!   (the drift that silently loses width semantics), and set entries
//!   without arms fail the same test.
//!
//! Adding a width-semantic method = add the arm in
//! `method_dispatch.rs` AND the name here; the test holds the pair
//! together.

/// Primitive widths with mangled dispatch surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimWidth {
    /// `Byte` / `UInt8` / `U8` / `u8` receivers.
    Byte,
    /// `Int32` / `I32` / `i32` receivers.
    Int32,
    /// `UInt64` / `U64` / `u64` / `USize` / `usize` receivers.
    UInt64,
}

impl PrimWidth {
    /// The mangle prefix (`byte` in `byte$to_int`).
    pub fn prefix(self) -> &'static str {
        match self {
            PrimWidth::Byte => "byte",
            PrimWidth::Int32 => "int32",
            PrimWidth::UInt64 => "uint64",
        }
    }

    /// The canonical STDLIB type name whose qualified methods carry the
    /// non-width-semantic surface (`UInt8.to_hex`). This is the alias
    /// target — `Byte` is `public type Byte is UInt8`, and the bake
    /// registers the impl-block methods under the target's name.
    pub fn canonical_type_name(self) -> &'static str {
        match self {
            PrimWidth::Byte => "UInt8",
            PrimWidth::Int32 => "Int32",
            PrimWidth::UInt64 => "UInt64",
        }
    }

    /// The width-semantic method set — the ONLY names codegen may
    /// mangle and the ONLY `<prefix>$` arms the interpreter may carry.
    pub fn methods(self) -> &'static [&'static str] {
        match self {
            PrimWidth::Byte => BYTE_METHODS,
            PrimWidth::Int32 => INT32_METHODS,
            PrimWidth::UInt64 => UINT64_METHODS,
        }
    }
}

/// `Byte` methods whose semantics depend on u8 width. ASCII predicates
/// live here because their `Char` twins interpret the receiver as a
/// Unicode codepoint — same bare name, different reading of the bits.
pub const BYTE_METHODS: &[&str] = &[
    "checked_add",
    "checked_mul",
    "checked_sub",
    "is_ascii",
    "is_ascii_alphabetic",
    "is_ascii_alphanumeric",
    "is_ascii_control",
    "is_ascii_digit",
    "is_ascii_graphic",
    "is_ascii_hexdigit",
    "is_ascii_lowercase",
    "is_ascii_punctuation",
    "is_ascii_uppercase",
    "is_ascii_whitespace",
    "saturating_add",
    "saturating_sub",
    "to_ascii_lowercase",
    "to_ascii_uppercase",
    "to_int",
    "wrapping_add",
    "wrapping_mul",
    "wrapping_sub",
];

/// `Int32` width-semantic methods (i32 two's-complement behaviour).
pub const INT32_METHODS: &[&str] = &[
    "MAX",
    "MIN",
    "abs",
    "checked_add",
    "checked_mul",
    "checked_sub",
    "count_ones",
    "from_be_bytes",
    "from_le_bytes",
    "leading_zeros",
    "rotate_left",
    "rotate_right",
    "saturating_add",
    "saturating_sub",
    "signum",
    "swap_bytes",
    "to_be_bytes",
    "to_int",
    "to_le_bytes",
    "trailing_zeros",
    "wrapping_add",
    "wrapping_mul",
    "wrapping_sub",
];

/// `UInt64` width-semantic methods (u64 unsigned behaviour — including
/// the comparison family, which must compare UNSIGNED while the i64
/// register model would compare signed).
pub const UINT64_METHODS: &[&str] = &[
    "MAX",
    "MIN",
    "checked_add",
    "checked_mul",
    "checked_sub",
    "count_ones",
    "eq",
    "from_be_bytes",
    "from_le_bytes",
    "ge",
    "gt",
    "le",
    "leading_zeros",
    "lt",
    "ne",
    "rotate_left",
    "rotate_right",
    "saturating_add",
    "saturating_sub",
    "swap_bytes",
    "to_be_bytes",
    "to_int",
    "to_le_bytes",
    "trailing_zeros",
    "wrapping_add",
    "wrapping_sub",
];

/// The dispatch name for `method` on a `width`-typed receiver:
/// `Some("<prefix>$<method>")` when width semantics apply, `None` when
/// the caller must use [`qualified`] (the real registered method).
pub fn mangled(width: PrimWidth, method: &str) -> Option<String> {
    if width.methods().contains(&method) {
        Some(format!("{}${}", width.prefix(), method))
    } else {
        None
    }
}

/// The alias-canonical qualified dispatch name (`UInt8.to_hex`) for a
/// non-width-semantic method.
pub fn qualified(width: PrimWidth, method: &str) -> String {
    format!("{}.{}", width.canonical_type_name(), method)
}

/// The ONE composition every codegen mangle site uses: width-semantic
/// members mangle, everything else dispatches to the real method.
pub fn dispatch_name(width: PrimWidth, method: &str) -> String {
    mangled(width, method).unwrap_or_else(|| qualified(width, method))
}

/// Split `<prefix>$<method>` back into its parts. `None` for names that
/// are not prim-mangles (no `$`, or an unknown prefix — `$` also appears
/// in lexical-fn mangles `parent$child`, which use arbitrary parents).
pub fn demangle(name: &str) -> Option<(PrimWidth, &str)> {
    let (prefix, method) = name.split_once('$')?;
    let width = match prefix {
        "byte" => PrimWidth::Byte,
        "int32" => PrimWidth::Int32,
        "uint64" => PrimWidth::UInt64,
        _ => return None,
    };
    Some((width, method))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangle_members_and_qualify_others() {
        assert_eq!(
            mangled(PrimWidth::Byte, "saturating_add").as_deref(),
            Some("byte$saturating_add")
        );
        assert_eq!(mangled(PrimWidth::Byte, "to_hex"), None);
        assert_eq!(dispatch_name(PrimWidth::Byte, "to_hex"), "UInt8.to_hex");
        assert_eq!(
            dispatch_name(PrimWidth::UInt64, "wrapping_add"),
            "uint64$wrapping_add"
        );
        assert_eq!(dispatch_name(PrimWidth::Int32, "to_hex"), "Int32.to_hex");
    }

    #[test]
    fn demangle_round_trips_and_rejects_foreign_dollars() {
        for w in [PrimWidth::Byte, PrimWidth::Int32, PrimWidth::UInt64] {
            for m in w.methods() {
                let mangled = mangled(w, m).expect("member mangles");
                let (w2, m2) = demangle(&mangled).expect("demangles");
                assert_eq!(w2, w);
                assert_eq!(&m2, m);
            }
        }
        // Lexical nested-fn mangles must NOT read as prim mangles.
        assert_eq!(demangle("outer$inner"), None);
        assert_eq!(demangle("no_dollar"), None);
    }

    /// PAIRED-LIST GATE: the interpreter's `<prefix>$` intercept arms
    /// and these tables are ONE set. An arm outside the table silently
    /// loses width semantics the day codegen stops mangling its name;
    /// a table entry without an arm dispatches a mangled name nothing
    /// intercepts. Both directions fail here, naming the drifted
    /// method.
    #[test]
    fn paired_lists_cover_interp_arms() {
        let src = include_str!("interpreter/dispatch_table/handlers/method_dispatch.rs");
        for (width, re_prefix) in [
            (PrimWidth::Byte, "byte"),
            (PrimWidth::Int32, "int32"),
            (PrimWidth::UInt64, "uint64"),
        ] {
            let needle = format!("\"{}$", re_prefix);
            let mut arm_methods: Vec<&str> = src
                .match_indices(&needle)
                .filter_map(|(i, _)| {
                    let rest = &src[i + needle.len()..];
                    let end = rest.find('"')?;
                    Some(&rest[..end])
                })
                .collect();
            arm_methods.sort_unstable();
            arm_methods.dedup();
            let mut table: Vec<&str> = width.methods().to_vec();
            table.sort_unstable();
            let missing_from_table: Vec<&&str> = arm_methods
                .iter()
                .filter(|m| !table.contains(*m))
                .collect();
            let missing_arms: Vec<&&str> = table
                .iter()
                .filter(|m| !arm_methods.contains(*m))
                .collect();
            assert!(
                missing_from_table.is_empty() && missing_arms.is_empty(),
                "prim-mangle drift for `{}`: arms outside table {:?}; table entries without arms {:?}",
                re_prefix,
                missing_from_table,
                missing_arms,
            );
        }
    }
}
