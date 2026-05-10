//! Stdlib coercion-protocol registry — temporary scaffold pending
//! `#101` protocol-based replacement.
//!

//! # Background
//!

//! `verum_types/src/CLAUDE.md` lays down a hard architectural rule:
//!

//! > **NEVER hardcode stdlib/core type knowledge in the compiler.**
//! > The compiler implementation (verum_types, verum_vbc, etc.) must
//! > have ZERO knowledge of stdlib (`core/`) types.
//!

//! Until protocol-based discovery is wired up (each stdlib type
//! declares `implement IntCoercible / TensorLike / Indexable /
//! RangeLike for X` and the compiler scans implement-blocks to drive
//! unifier registration), we centralise the hardcoded scaffolding
//! HERE so:
//!

//!  * The violation is contained in one identifiable location, not
//!  scattered through `pipeline.rs` Pass 5.5.
//!  * A grep for the protocol name (`int_coercible_stdlib_names`,
//!  etc.) shows the entire list of stdlib types the compiler
//!  "knows about" — no hidden sites to forget when ripping the
//!  list out.
//!  * The follow-up replacement (#101) only has to delete this
//!  module and replace `register_stdlib_coercions` with a
//!  protocol-walking pass.
//!

//! # Migration plan (for #101 follow-up)
//!

//! Step 1: define the four coercion protocols in `core/base/coercion.vr`:
//!

//! ```verum
//! public type IntCoercible is protocol {};
//! public type TensorLike is protocol { /* tensor-shape methods */ };
//! public type Indexable is protocol { fn index(&self, i: Int) -> _; };
//! public type RangeLike is protocol { fn start(&self) -> Int; fn end(&self) -> Int; };
//! ```
//!

//! Step 2: have stdlib types `implement` the relevant protocol(s):
//!

//! ```verum
//! implement IntCoercible for Duration {};
//! implement IntCoercible for Port {};
//! implement TensorLike for DynTensor<T> { /* ... */ };
//! ```
//!

//! Step 3: in pipeline.rs replace the call to `register_stdlib_coercions`
//! with a pass that walks `module.items` for `ItemKind::Impl(impl_decl)`
//! whose `protocol_path` matches one of the four protocols, and calls
//! the corresponding `register_*_type` method on the unifier with the
//! impl block's target type name.
//!

//! Step 4: delete this module.

/// Stdlib type names that participate in tensor-family coercions.
///
/// Per-entry status (#101 Step 2 migration — Step 4 reached):
///   * `DynTensor` — REMOVED. `core/math/tensor.vr` declares
///     `implement<T: Numeric> TensorLike for DynTensor<T> {}`;
///     `scan_protocol_implementations` registers it.
///   * `Vector` — REMOVED. `core/math/linalg.vr` declares
///     `implement<T: Numeric> TensorLike for Vector<T> {}`.
///   * `Cotangent` / `Tangent` — REMOVED (dead). No `public type
///     Cotangent` / `public type Tangent` exists in `core/`; the
///     types are spelled `CotangentVector` / `TangentVector` and
///     have their own implement blocks.
///   * `Tensor` — REMOVED. `core/math/tensor.vr` declares
///     `public type Tensor<T: Numeric> is DynTensor<T>` (alias);
///     the unifier's `is_tensor_family` is now alias-aware
///     (`Unifier::resolve_aliased_head_name`) so the alias
///     resolves to `DynTensor` and inherits TensorLike via that
///     type's implement block. No hardcoded fallback needed.
pub const TENSOR_FAMILY_STDLIB_NAMES: &[&str] = &[];

/// Stdlib type names that support integer indexing.
///
/// Per-entry status (#101 Step 2 migration):
///   * `Range` — REMOVED. `core/base/iterator.vr` declares
///     `implement<T> Indexable for Range<T> {}` (alongside
///     IntCoercible + RangeLike).
///   * `Slice` — REMOVED (dead). No `public type Slice` exists in
///     `core/`; the closest match is `SliceOrIndex` which the
///     unifier never sees as a bare `"Slice"` name.
pub const INDEXABLE_STDLIB_NAMES: &[&str] = &[];

/// Stdlib type names that match the range-like shape (start, end).
///
/// Per-entry status (#101 Step 2 migration):
///   * `Range` — REMOVED. `core/base/iterator.vr` declares
///     `implement<T> RangeLike for Range<T> {}`. The list now
///     reaches Step 4 (delete from hardcoded fallback) for this
///     coercion category — every Range-like stdlib type carries
///     its own `implement RangeLike` block.
pub const RANGE_LIKE_STDLIB_NAMES: &[&str] = &[];

/// Stdlib type names whose in-memory layout is a packed C-struct
/// byte mirror of a kernel / libsystem ABI — the unifier accepts
/// `[Byte]` / `[UInt8]` against any name in this set as an
/// FFI-shape coercion.
///
/// Replaces the previously hardcoded
/// `matches!(name, "Sockaddr" | "SocketAddr" | "SockaddrIn")`
/// short-circuit inline in `verum_types::unify::Unifier`'s
/// Named↔Named arm.
///
/// Per-entry status (Step 4 reached):
///   * `Sockaddr` — REMOVED. `core/sys/darwin/libsystem.vr`
///     declares `implement BytewiseFfi for Sockaddr {}`;
///     `scan_protocol_implementations` registers the name.
///   * `SocketAddr` — REMOVED (architecturally wrong).
///     `core/net/addr.vr` declares `SocketAddr is V4(...) |
///     V6(...)` — a sum type, not a packed-C-struct byte
///     mirror.  The unifier was over-permissive; including
///     this name accepted a high-level enum where only the
///     low-level FFI mirror should pass.
///   * `SockaddrIn` — REMOVED.  Linux declares
///     `public type SockaddrIn is { ... }` with an explicit
///     `implement BytewiseFfi for SockaddrIn {}`;
///     Darwin declares `public type SockaddrIn =
///     DarwinSockaddrIn;` and the unifier's `is_bytewise_ffi`
///     is now alias-aware (`Unifier::resolve_aliased_head_name`)
///     so the Darwin alias resolves to `DarwinSockaddrIn` and
///     inherits the marker via that type's implement block.
///     No hardcoded fallback needed.
pub const BYTEWISE_FFI_STDLIB_NAMES: &[&str] = &[];

/// Stdlib type names that cross-coerce with `Int` in unification.
///
/// **Categories** (entries here are types whose `.vr` source has
/// not yet been retrofitted with `implement IntCoercible for X
/// {}`; once retrofitted they're discoverable via
/// `scan_protocol_implementations` and the entry can be deleted):
///
///   * **FFI integer typedefs** — `Timespec` / `TimeSpec` are
///     POSIX `struct timespec` mirrors used in syscall args.
///   * **Bitflags / sockaddr** — `MemProt` / `MapFlags` are
///     packed-bit i64s for `mmap`; `Sockaddr` is the libsystem
///     socket-address typedef.
///   * **Path types** — `Path` / `PathBuf` coerce because
///     indexing and slicing produce `Int`, not because they're
///     integer-shaped themselves.
///   * **Resource handles** — `GPUBuffer` / `DeviceRegistry` /
///     `ProcessGroup` are opaque-i64 handles managed by the GPU
///     runtime / process supervisor.
///
/// **Removed** (already retrofitted with `implement IntCoercible`):
///   * `Port` (sys/io_engine.vr), `FileDesc` (sys/common.vr),
///     `MachPort` (sys/darwin/libsystem.vr), `VmAddress` /
///     `VmSize` (sys/darwin/mach.vr) — FFI integer typedefs.
///   * `Duration` (time/duration.vr), `Instant` (time/instant.vr)
///     — time scalars.
///   * `DynTensor` (math/tensor.vr), `Vector` (math/linalg.vr) —
///     tensor family.
///   * `List` (collections/list.vr), `Range` (base/iterator.vr),
///     `Maybe` (base/maybe.vr) — container family.
///
/// **Removed** (architectural — type doesn't fit `Int` coercion
/// or doesn't exist as a public stdlib type):
///   * `ClockId` — only `CClockId is (Int32)` in `core/sys/cabi.vr`;
///     the bare name `ClockId` was dead in the unifier.
///   * `Slice` — no `public type Slice` exists; the closest match
///     is `SliceOrIndex` which the unifier never sees as `"Slice"`.
///   * `Epoch` — `core/mem/epoch.vr` declares `Epoch is ()` (unit
///     type, not a scalar); `Int` coercion is meaningless for unit.
///   * `Once`, `Lazy` — sync / lazy-value wrappers; coercing them
///     to `Int` is a type-error the unifier should surface, not
///     accept.
///   * `Tensor` — type alias `Tensor<T> is DynTensor<T>`; the
///     unifier resolves aliases before consulting tensor_family,
///     and DynTensor's `implement TensorLike` carries through.
///
/// Becomes obsolete entirely once every entry below has its
/// `implement IntCoercible` block and is dropped from this list
/// (#101 step 3 close-out).
pub const INT_COERCIBLE_STDLIB_NAMES: &[&str] = &[
    // **Step 4 reached for IntCoercible.**  Every stdlib type
    // that needs `Int`-context coercion now carries its own
    // `implement IntCoercible for X {}` block in the
    // source-of-truth `.vr` file:
    //
    //   * `Timespec` — `core/sys/linux/syscall.vr` (Linux-side);
    //     `TimeSpec` (Darwin-side) — `core/sys/io_engine.vr`.
    //   * `Path`, `PathBuf` — `core/io/path.vr`.
    //   * `GPUBuffer`, `DeviceRegistry` — `core/math/gpu.vr`.
    //   * `ProcessGroup` — `core/math/distributed.vr`.
    //
    // `scan_protocol_implementations` registers them via the
    // AST walk; this hardcoded list is now empty.  The whole
    // `register_stdlib_coercions` function still gets called as
    // a no-op safety net so a future re-introduction of a
    // hardcoded entry surfaces structurally at the
    // `migration_progress_pinned` test.
];

/// Stdlib type names that cross-coerce with each other in
/// Named<->Named unification (e.g. Duration ↔ Instant via i64 backing
/// representation). Language-level numeric primitives (Int8 / U64 /
/// Float64) live in `unify.rs::Unifier::new` and are NOT included
/// here — those are part of the language definition, not stdlib.
///

/// Becomes obsolete once `Numeric` protocol query lands (separate
/// follow-up — `Numeric` exists at `core/base/protocols.vr` but isn't
/// queryable from the unifier yet).
pub const SIZED_NUMERIC_STDLIB_NAMES: &[&str] = &["Duration", "Instant", "Epoch"];

/// Register every stdlib type in the four lists above with the
/// unifier. Single entry point so callers in `pipeline.rs` Pass 5.5
/// don't see any hardcoded names.
///

/// When #101 protocol-based discovery lands, this function's body
/// gets replaced by a `walk_implement_blocks_and_register` pass —
/// the call sites stay the same.
pub fn register_stdlib_coercions(unifier: &mut verum_types::unify::Unifier) {
    for name in TENSOR_FAMILY_STDLIB_NAMES {
        unifier.register_tensor_family_type(verum_common::Text::from(*name));
    }
    for name in INDEXABLE_STDLIB_NAMES {
        unifier.register_indexable_type(verum_common::Text::from(*name));
    }
    for name in RANGE_LIKE_STDLIB_NAMES {
        unifier.register_range_like_type(verum_common::Text::from(*name));
    }
    for name in SIZED_NUMERIC_STDLIB_NAMES {
        unifier.register_sized_numeric_type(verum_common::Text::from(*name));
    }
    for name in INT_COERCIBLE_STDLIB_NAMES {
        unifier.register_int_coercible_type(verum_common::Text::from(*name));
    }
    for name in BYTEWISE_FFI_STDLIB_NAMES {
        unifier.register_bytewise_ffi_type(verum_common::Text::from(*name));
    }
}

// ============================================================================
// Step 2 of #101 — protocol-based discovery
// ============================================================================
//

// Walks AST `ItemKind::Impl(ImplKind::Protocol { protocol, for_type, ..})`
// blocks and registers the target type with the unifier when the
// protocol path's tail matches one of the four coercion markers
// declared in `core/base/coercion.vr`. Combined with the hardcoded
// fallback above, this gives:
//

//  * Stdlib types that already `implement <Coercion>` get registered
//  by the protocol scan (ZERO architectural violation for those
//  types).
//  * Stdlib types that haven't yet been retrofitted with implement
//  blocks still get registered via the hardcoded fallback,
//  keeping behaviour stable.
//  * Each retrofit (adding `implement IntCoercible for X` to one
//  stdlib type) lets us delete X from the hardcoded list and
//  verify nothing regresses — incremental migration with safe
//  rollback at every step.

/// Match the four coercion-marker protocol names against the LAST
/// segment of an impl-block's protocol path. Returns the name as a
/// stable `&'static str` we can dispatch on, or `None` for any other
/// protocol.
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
        _ => None,
    }
}

/// Extract the head type name from an impl-block's `for_type`. We only
/// look at the OUTER name; generic args don't matter for unifier
/// registration since the unifier treats e.g. `Vector<T>` the same
/// way regardless of T.
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

/// Scan a list of AST modules for `implement <Coercion> for X` blocks
/// and register the target types with the unifier. Idempotent — calling
/// it more than once is harmless because the unifier's register_*
/// methods de-duplicate via HashSet.
///

/// Public so `pipeline.rs` Pass 5.5 can call it with the loaded
/// stdlib + user modules.
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
                _ => unreachable!("match_coercion_protocol guards this set"),
            }
            registered += 1;
        }
    }
    registered
}

#[cfg(test)]
mod migration_pins {
    //! Drift-detection pins for the #101 Step 2 migration.
    //!
    //! Each hardcoded `*_STDLIB_NAMES` list shrinks monotonically —
    //! every retrofitted stdlib type (one `implement <Marker> for X
    //! {}` block in its `.vr` file) must drop the corresponding
    //! entry from the hardcoded fallback in the same commit. The
    //! pins below assert the residual hardcoded set is well-formed
    //! and disjoint from itself, so accidental duplicates can't
    //! survive the type-checker's HashSet de-duplication silently.
    use super::*;

    /// Each hardcoded list is an internally consistent set — no
    /// duplicate names within a single category. (Cross-category
    /// duplication is fine and expected: `Range` legitimately
    /// belongs in both Indexable and RangeLike, for example,
    /// before retrofit.)
    #[test]
    fn hardcoded_lists_have_no_intra_list_duplicates() {
        for (name, list) in [
            ("TENSOR_FAMILY_STDLIB_NAMES", TENSOR_FAMILY_STDLIB_NAMES),
            ("INDEXABLE_STDLIB_NAMES", INDEXABLE_STDLIB_NAMES),
            ("RANGE_LIKE_STDLIB_NAMES", RANGE_LIKE_STDLIB_NAMES),
            ("INT_COERCIBLE_STDLIB_NAMES", INT_COERCIBLE_STDLIB_NAMES),
            ("SIZED_NUMERIC_STDLIB_NAMES", SIZED_NUMERIC_STDLIB_NAMES),
            ("BYTEWISE_FFI_STDLIB_NAMES", BYTEWISE_FFI_STDLIB_NAMES),
        ] {
            let unique: std::collections::HashSet<&str> =
                list.iter().copied().collect();
            assert_eq!(
                unique.len(),
                list.len(),
                "{} has intra-list duplicates",
                name,
            );
        }
    }

    /// Migration progress pin: the hardcoded fallback shrinks each
    /// time a stdlib retrofit lands. Bumping this pin **down**
    /// requires the matching `implement <Marker> for X {}` block
    /// to land in the .vr source-of-truth in the same commit.
    /// Going **up** is forbidden — a future caller adding a new
    /// hardcoded entry must justify why the protocol-scan path
    /// won't work for that type. The closure of #101 step 3 is
    /// reached when every count below is 0.
    ///
    /// **Status: 4 of 5 coercion-marker categories are at
    /// Step 4.** Only `SIZED_NUMERIC_STDLIB_NAMES` retains a
    /// non-zero baseline because that category follows a
    /// separate timeline (the `Numeric` protocol query
    /// in `core/base/protocols.vr` is not yet wired into the
    /// unifier).
    #[test]
    fn migration_progress_pinned() {
        // Tensor family: Step 4 reached. The lone `"Tensor"`
        // alias-coverage entry was eliminated by adding
        // alias-aware lookup to `Unifier::is_tensor_family`
        // (`resolve_aliased_head_name`), so `Tensor<T>`
        // (alias for `DynTensor<T>`) now inherits TensorLike
        // through DynTensor's implement block transparently.
        assert_eq!(TENSOR_FAMILY_STDLIB_NAMES.len(), 0);
        // Indexable + RangeLike: every reachable Range-like /
        // indexable stdlib type carries its own implement block.
        assert_eq!(INDEXABLE_STDLIB_NAMES.len(), 0);
        assert_eq!(RANGE_LIKE_STDLIB_NAMES.len(), 0);
        // IntCoercible: Step 4 reached. Every stdlib type that
        // needs Int-context coercion now carries its own
        // `implement IntCoercible for X {}` block in the .vr
        // source. List is empty; protocol-scan is the sole
        // registration path.
        assert_eq!(INT_COERCIBLE_STDLIB_NAMES.len(), 0);
        // SIZED_NUMERIC follows a different timeline (`Numeric`
        // protocol query landing) — keep its 3-entry baseline.
        assert_eq!(SIZED_NUMERIC_STDLIB_NAMES.len(), 3);
        // BytewiseFfi: Step 4 reached. The `"SockaddrIn"`
        // alias-coverage entry was eliminated by the same
        // alias-aware-lookup change — Darwin's
        // `type SockaddrIn = DarwinSockaddrIn` alias now
        // resolves through `is_bytewise_ffi` to the underlying
        // `DarwinSockaddrIn` (which carries the marker block).
        assert_eq!(BYTEWISE_FFI_STDLIB_NAMES.len(), 0);
    }

    /// `match_coercion_protocol` accepts exactly the four
    /// canonical marker names declared in
    /// `core/base/coercion.vr`.  Adding a new coercion-marker
    /// protocol would require updating both the .vr source and
    /// this match arm in the same commit.
    #[test]
    fn coercion_protocol_match_pinned() {
        for marker in [
            "IntCoercible",
            "TensorLike",
            "Indexable",
            "RangeLike",
            "BytewiseFfi",
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
}
