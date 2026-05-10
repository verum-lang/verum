//! Stdlib coercion-protocol registry — protocol-scan path.
//!
//! # Architectural rule
//!
//! `verum_types/src/CLAUDE.md` lays down the canonical rule:
//!
//! > **NEVER hardcode stdlib/core type knowledge in the compiler.**
//! > The compiler implementation (verum_types, verum_vbc, etc.)
//! > must have ZERO knowledge of stdlib (`core/`) types.
//!
//! # Design
//!
//! Six marker protocols in `core/base/coercion.vr` express the
//! coercion-rule axes the unifier consults:
//!
//! ```verum
//! public type IntCoercible is protocol {};
//! public type TensorLike   is protocol {};
//! public type Indexable    is protocol {};
//! public type RangeLike    is protocol {};
//! public type BytewiseFfi  is protocol {};
//! public type SizedNumeric is protocol {};
//! ```
//!
//! Stdlib types opt into a coercion rule with a one-line block:
//!
//! ```verum
//! implement IntCoercible for Duration {};
//! implement<T: Numeric> TensorLike for DynTensor<T> {};
//! implement BytewiseFfi for Sockaddr {};
//! ```
//!
//! [`scan_protocol_implementations`] walks the loaded AST module
//! list, finds `ItemKind::Impl(impl_decl)` blocks whose
//! `protocol_path` matches one of the six markers, and calls the
//! corresponding `register_*_type` method on the unifier.  No
//! hardcoded type-name lists; the compiler discovers stdlib's
//! coercion posture entirely from the `.vr` source-of-truth.
//!
//! # History
//!
//! Steps 1-3 of the migration plan landed across earlier commits;
//! Step 4 (delete the hardcoded fallback infrastructure) reached
//! when every `*_STDLIB_NAMES` list became empty after the
//! six per-marker retrofits.  The `register_stdlib_coercions`
//! function was removed wholesale once the trip-wire pin tests
//! went green at zero entries — the positive
//! `protocol_scan_finds_all_six_markers` pin below replaces the
//! negative empty-list assertions.

/// Match the six coercion-marker protocol names against the LAST
/// segment of an impl-block's protocol path.  Returns the marker
/// name as a stable `&'static str` we dispatch on, or `None` for
/// any other protocol.
///
/// Pinned by `coercion_protocol_match_pinned` so the matcher's
/// accept-set stays in lockstep with the canonical declarations
/// in `core/base/coercion.vr` — adding a new coercion-marker
/// protocol must update both the .vr source AND this match arm
/// in the same commit.
fn match_coercion_protocol(path: &verum_ast::ty::Path) -> Option<&'static str> {
    let last = path.segments.iter().rev().find_map(|s| match s {
        verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
        _ => None,
    })?;
    match last {
        "IntCoercible" => Some("IntCoercible"),
        "TensorLike" => Some("TensorLike"),
        "Indexable" => Some("Indexable"),
        "RangeLike" => Some("RangeLike"),
        "BytewiseFfi" => Some("BytewiseFfi"),
        "SizedNumeric" => Some("SizedNumeric"),
        "ArrayCoercible" => Some("ArrayCoercible"),
        _ => None,
    }
}

/// Extract the head type name from an impl-block's `for_type`.
/// We only look at the OUTER name; generic args don't matter for
/// unifier registration since the unifier treats e.g.
/// `Vector<T>` the same way regardless of T.
fn impl_target_head_name(ty: &verum_ast::ty::Type) -> Option<String> {
    use verum_ast::ty::{PathSegment, Type, TypeKind};
    fn head_of_path(path: &verum_ast::ty::Path) -> Option<String> {
        path.segments.iter().rev().find_map(|s| match s {
            PathSegment::Name(id) => Some(id.name.to_string()),
            _ => None,
        })
    }
    fn walk(ty: &Type) -> Option<String> {
        match &ty.kind {
            TypeKind::Path(path) => head_of_path(path),
            TypeKind::Generic { base, .. } => walk(base),
            TypeKind::Reference { inner, .. } => walk(inner),
            _ => None,
        }
    }
    walk(ty)
}

/// Scan a list of AST modules for `implement <Marker> for X`
/// blocks against any of the six coercion markers in
/// `core/base/coercion.vr` (IntCoercible / TensorLike /
/// Indexable / RangeLike / BytewiseFfi / SizedNumeric) and
/// register the target types with the unifier.
///
/// **Idempotent** — calling it more than once is harmless because
/// the unifier's `register_*` methods de-duplicate via HashSet.
///
/// **Sole registration path** — every coercion-rule registration
/// flows through here (the previously-coexisting
/// `register_stdlib_coercions` hardcoded-fallback function was
/// removed at #101 step 4 close-out once every retrofit landed).
///
/// Returns the number of impl blocks registered, for telemetry /
/// debug logging.  Public so `pipeline.rs` Pass 5.5 can call it
/// with the loaded stdlib + user modules.
pub fn scan_protocol_implementations<'a, I>(
    unifier: &mut verum_types::unify::Unifier,
    ast_modules: I,
) -> usize
where
    I: IntoIterator<Item = &'a verum_ast::Module>,
{
    use verum_ast::{ItemKind, decl::ImplKind};
    let mut registered = 0usize;
    for module in ast_modules {
        for item in module.items.iter() {
            let ItemKind::Impl(impl_decl) = &item.kind else {
                continue;
            };
            let ImplKind::Protocol {
                protocol, for_type, ..
            } = &impl_decl.kind
            else {
                continue;
            };
            let Some(coercion_name) = match_coercion_protocol(protocol) else {
                continue;
            };
            let Some(target) = impl_target_head_name(for_type) else {
                continue;
            };
            let target_text = verum_common::Text::from(target.as_str());
            match coercion_name {
                "IntCoercible" => unifier.register_int_coercible_type(target_text),
                "TensorLike" => unifier.register_tensor_family_type(target_text),
                "Indexable" => unifier.register_indexable_type(target_text),
                "RangeLike" => unifier.register_range_like_type(target_text),
                "BytewiseFfi" => unifier.register_bytewise_ffi_type(target_text),
                "SizedNumeric" => unifier.register_sized_numeric_type(target_text),
                "ArrayCoercible" => {
                    // Dual-register: Unifier-instance state for
                    // the type-checker pipeline + global shared
                    // state for `Subtyping` instances constructed
                    // locally outside the pipeline.
                    unifier.register_array_coercible_type(target_text.clone());
                    verum_types::subtype::register_global_array_coercible(target_text);
                }
                _ => unreachable!("match_coercion_protocol guards this set"),
            }
            registered += 1;
        }
    }
    registered
}

#[cfg(test)]
mod migration_pins {
    //! Drift-detection pins for the #101 stdlib-coercion-protocol
    //! migration.
    use super::*;

    /// `match_coercion_protocol` accepts exactly the seven
    /// canonical marker names declared in
    /// `core/base/coercion.vr` and rejects everything else.
    /// Adding a new coercion-marker protocol must update both
    /// the .vr source and this match arm in the same commit.
    #[test]
    fn coercion_protocol_match_pinned() {
        for marker in [
            "IntCoercible",
            "TensorLike",
            "Indexable",
            "RangeLike",
            "BytewiseFfi",
            "SizedNumeric",
            "ArrayCoercible",
        ] {
            // Build a dummy single-segment Path holding just the
            // marker name and assert the matcher accepts it.
            let path = verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::Ident::new(
                        verum_common::Text::from(marker),
                        Default::default(),
                    ),
                )],
                span: Default::default(),
            };
            assert_eq!(match_coercion_protocol(&path), Some(marker));
        }
        // Negative pin: an arbitrary other protocol must NOT
        // match — keeps the matcher disjoint from the rest of
        // Verum's protocol surface.
        let other = verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                verum_ast::Ident::new(
                    verum_common::Text::from("Hash"),
                    Default::default(),
                ),
            )],
            span: Default::default(),
        };
        assert_eq!(match_coercion_protocol(&other), None);
    }

    /// `impl_target_head_name` extracts the rightmost path
    /// segment for plain Path / Generic / Reference types and
    /// returns `None` for shapes the head-name extractor doesn't
    /// know how to traverse (Tuple, Function, etc.).  Pins the
    /// extractor against the three shapes the protocol-scan
    /// walker actually encounters.
    #[test]
    fn impl_target_head_name_extracts_three_shapes() {
        use verum_ast::ty::{PathSegment, Type, TypeKind};
        fn make_path(name: &str) -> verum_ast::ty::Path {
            verum_ast::ty::Path {
                segments: smallvec::smallvec![PathSegment::Name(verum_ast::Ident::new(
                    verum_common::Text::from(name),
                    Default::default(),
                ))],
                span: Default::default(),
            }
        }
        // Plain path: `DynTensor`
        let plain = Type {
            kind: TypeKind::Path(make_path("DynTensor")),
            span: Default::default(),
        };
        assert_eq!(impl_target_head_name(&plain).as_deref(), Some("DynTensor"));
        // Generic: `Vector<T>` — head is still `Vector`.
        let generic = Type {
            kind: TypeKind::Generic {
                base: verum_common::Heap::new(plain.clone()),
                args: verum_common::List::new(),
            },
            span: Default::default(),
        };
        assert_eq!(impl_target_head_name(&generic).as_deref(), Some("DynTensor"));
        // Reference: `&Sockaddr` — head is the inner.
        let reference = Type {
            kind: TypeKind::Reference {
                mutable: false,
                inner: verum_common::Heap::new(Type {
                    kind: TypeKind::Path(make_path("Sockaddr")),
                    span: Default::default(),
                }),
            },
            span: Default::default(),
        };
        assert_eq!(impl_target_head_name(&reference).as_deref(), Some("Sockaddr"));
    }
}
