//! Subtyping with refinement types.
//!
//! This module implements subtyping rules including:
//! - Structural subtyping for records and variants
//! - Refinement subtyping (stronger predicates are subtypes)
//! - Function subtyping (contravariant in arguments, covariant in return)
//! - Array/List subtyping (covariant in element type)
//! - Reference variance rules (covariant for shared, invariant for mutable)
//!
//! Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 - Subtyping Algorithm

use crate::ty::{Type, TypeVar};
use crate::variance::Variance;

use std::cell::Cell;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Text};

thread_local! {
    static SUBTYPE_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Subtyping checker.
///
/// Implements the complete subtyping algorithm per subtyping specification: structural subtyping for records, refinement subtyping, protocol-based nominal subtyping.
/// The subtyping relation S <: T means "a value of type S can be used where T is expected".
pub struct Subtyping {
    /// Cache for subtyping queries (for performance)
    cache: dashmap::DashMap<(Text, Text), bool>,
    /// Data-driven set of collection type names that support array coercion.
    /// Array types `[T; N]` are subtypes of any `C<T>` where `C` is in this set.
    /// Defaults to `{"List"}`. Will be replaced with `FromArray` protocol once available.
    array_coercible_types: std::collections::HashSet<Text>,
}

impl Subtyping {
    pub fn new() -> Self {
        let mut array_coercible_types = std::collections::HashSet::new();
        array_coercible_types.insert(Text::from(WKT::List.as_str()));
        Self {
            cache: dashmap::DashMap::new(),
            array_coercible_types,
        }
    }

    /// Register a collection type name that supports array literal coercion.
    pub fn register_array_coercible_type(&mut self, type_name: Text) {
        self.array_coercible_types.insert(type_name);
    }

    /// Check whether a type name supports array coercion (data-driven).
    fn is_array_coercible(&self, name: &str) -> bool {
        self.array_coercible_types.contains(name)
    }

    /// Helper to check if a path represents an array-coercible collection type.
    /// Uses the data-driven `array_coercible_types` set instead of hardcoding "List".
    /// Get variance for each type parameter of a Named type.
    /// Uses well-known type names for standard library types,
    /// defaults to covariant for unknown types (safe for read-only containers).
    fn get_named_type_variances(&self, path: &verum_ast::ty::Path, arg_count: usize) -> List<Variance> {
        use verum_common::well_known_types::type_names as wkt;
        let type_name = path.segments.last().map(|seg| {
            match seg {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                _ => "",
            }
        }).unwrap_or("");
        Self::variances_for_type_name(type_name, arg_count)
    }

    /// Get variance for each type parameter of a Generic type.
    fn get_generic_type_variances(&self, name: &str, arg_count: usize) -> List<Variance> {
        Self::variances_for_type_name(name, arg_count)
    }

    /// Determine variance for type parameters by type name.
    /// Well-known types get their correct variance; unknown types default to covariant.
    fn variances_for_type_name(name: &str, arg_count: usize) -> List<Variance> {
        use verum_common::well_known_types::type_names as wkt;
        match name {
            // Invariant types (mutable interior)
            "Cell" | "RefCell" | "Atomic" | "Shared" | "Mutex" | "RwLock" =>
                vec![Variance::Invariant; arg_count].into(),
            // Map: keys invariant, values covariant
            n if n == wkt::MAP || n == "HashMap" || n == "TreeMap" => {
                if arg_count >= 2 {
                    vec![Variance::Invariant, Variance::Covariant].into()
                } else {
                    vec![Variance::Invariant; arg_count].into()
                }
            }
            // Everything else: covariant (List, Set, Maybe, Result, Heap, etc.)
            _ => vec![Variance::Covariant; arg_count].into(),
        }
    }

    fn path_is_array_coercible(&self, path: &verum_ast::ty::Path) -> bool {
        if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
            self.is_array_coercible(ident.name.as_str())
        } else {
            false
        }
    }

    /// Check if t1 is a subtype of t2: t1 <: t2
    ///
    /// This means a value of type t1 can be used where t2 is expected.
    ///
    /// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 - Subtyping Algorithm
    pub fn is_subtype(&self, t1: &Type, t2: &Type) -> bool {
        // Depth guard to prevent stack overflow on deeply nested or cyclic types
        const MAX_SUBTYPE_DEPTH: u32 = 128;
        let depth = SUBTYPE_DEPTH.with(|d| { let v = d.get(); d.set(v + 1); v + 1 });
        struct SubtypeDepthGuard;
        impl Drop for SubtypeDepthGuard {
            fn drop(&mut self) {
                SUBTYPE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
            }
        }
        let _guard = SubtypeDepthGuard;
        if depth > MAX_SUBTYPE_DEPTH {
            return false; // Conservative: assume not a subtype
        }

        use Type::*;

        match (t1, t2) {
            // Named types: compare by segments only (ignore span)
            // This must come before the reflexivity check to handle spans correctly
            (Named { path: p1, args: a1 }, Named { path: p2, args: a2 }) => {
                if p1.segments != p2.segments || a1.len() != a2.len() {
                    return false;
                }
                // Check type arguments with proper variance:
                // Covariant: a1_i <: a2_i
                // Contravariant: a2_i <: a1_i
                // Invariant: a1_i == a2_i (both directions)
                let variances = self.get_named_type_variances(p1, a1.len());
                a1.iter()
                    .zip(a2.iter())
                    .enumerate()
                    .all(|(i, (arg1, arg2))| {
                        match variances.get(i).copied().unwrap_or(Variance::Covariant) {
                            Variance::Covariant => self.is_subtype(arg1, arg2),
                            Variance::Contravariant => self.is_subtype(arg2, arg1),
                            Variance::Invariant => self.is_subtype(arg1, arg2) && self.is_subtype(arg2, arg1),
                        }
                    })
            }

            // Generic types (stdlib types like List<T>, Map<K,V>, etc.)
            // Core semantics: value semantics by default, explicit reference/heap allocation, no implicit copying — Semantic types
            //
            // Generic types are covariant in their type arguments.
            // List<Int{> 0}> <: List<Int> because Int{> 0} <: Int
            //
            // CRITICAL FIX: Generic types need explicit handling for subtyping
            // This enables type inference to work when comparing synthesized types
            // (with type variables) against expected types (with concrete types).
            (Generic { name: n1, args: a1 }, Generic { name: n2, args: a2 }) => {
                if n1 != n2 || a1.len() != a2.len() {
                    return false;
                }
                // Check type arguments with proper variance
                let variances = self.get_generic_type_variances(n1, a1.len());
                a1.iter()
                    .zip(a2.iter())
                    .enumerate()
                    .all(|(i, (arg1, arg2))| {
                        match variances.get(i).copied().unwrap_or(Variance::Covariant) {
                            Variance::Covariant => self.is_subtype(arg1, arg2),
                            Variance::Contravariant => self.is_subtype(arg2, arg1),
                            Variance::Invariant => self.is_subtype(arg1, arg2) && self.is_subtype(arg2, arg1),
                        }
                    })
            }

            // Cross-comparison: Generic <-> Named
            // Handles cases where the same type is represented differently
            (Generic { name, args: a1 }, Named { path, args: a2 })
            | (Named { path, args: a2 }, Generic { name, args: a1 }) => {
                let path_name = path.segments.last().map(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    }
                }).unwrap_or("");

                if name.as_str() != path_name || a1.len() != a2.len() {
                    return false;
                }
                a1.iter()
                    .zip(a2.iter())
                    .all(|(t1, t2)| self.is_subtype(t1, t2))
            }

            // Reflexivity: T <: T
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9551
            (t1, t2) if t1 == t2 => true,

            // Never type (bottom type): Never <: T for all T
            // Never is the type of expressions that never produce a value (panic, return, etc.)
            // As the bottom type, it is a subtype of every type.
            // Type lattice: Never is bottom (subtype of all), Unknown is top (supertype of all)
            (Never, _) => true,

            // Unit and empty tuple are equivalent
            // () is sometimes parsed as empty Tuple but semantically equals Unit
            (Unit, Tuple(elements)) | (Tuple(elements), Unit) if elements.is_empty() => true,

            // Refinement subtyping: T{p1} <: T{p2} if p1 => p2
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9554-9560
            (
                Refined {
                    base: b1,
                    predicate: p1,
                },
                Refined {
                    base: b2,
                    predicate: p2,
                },
            ) => {
                // Base types must match
                if !self.is_subtype(b1, b2) {
                    return false;
                }

                // Three-tier subsumption checking strategy:
                // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3.1 lines 9696-9707

                // Tier 1: Syntactic fast path (<1ms, 85% hit rate)
                if let Some(result) = check_syntactic_subsumption(&p1.predicate, &p2.predicate) {
                    return result;
                }

                // Tier 2: SMT solver with timeout (10-100ms)
                //
                // Previously delegated to `crate::smt_backend::check_subsumption_smt`,
                // which has been moved to `verum_smt::refinement_backend` to
                // break the `verum_types ↔ verum_smt` cycle. `Subtyping` no
                // longer has direct access to an SMT backend, so we
                // conservative-reject when the syntactic path could not
                // decide the relation. Callers that want SMT subsumption
                // should go through `RefinementChecker` (which owns an
                // injected `SmtBackend`).
                //
                // Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — .1 line 439 requires conservative rejection
                false
            }

            // Refinement erases to base: T{p} <: T
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9562-9564
            (Refined { base, .. }, t) => self.is_subtype(base, t),

            // Base does not erase to refinement: T <: T{p} only if p always holds
            // Conservative: reject unless we can prove it
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9566-9568
            (t, Refined { base, .. }) => self.is_subtype(t, base),

            // Function subtyping (contravariant in params, covariant in return)
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9574-9576
            (
                Function {
                    params: p1,
                    return_type: r1,
                    type_params: _,
                    contexts: c1,
                    properties: _,
                },
                Function {
                    params: p2,
                    return_type: r2,
                    type_params: _,
                    contexts: c2,
                    properties: _,
                },
            ) => self.check_function_subtype(p1, r1, c1, p2, r2, c2),

            // Tuple subtyping (covariant in components)
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9578-9585
            (Tuple(t1s), Tuple(t2s)) => {
                t1s.len() == t2s.len()
                    && t1s
                        .iter()
                        .zip(t2s.iter())
                        .all(|(t1, t2)| self.is_subtype(t1, t2))
            }

            // Record subtyping (width and depth)
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9570-9572
            (Record(f1), Record(f2)) => self.check_record_subtype(f1, f2),

            // Variant subtyping (contravariant in tags)
            // Spec: Sum types are dual to products
            (Variant(v1), Variant(v2)) => self.check_variant_subtype(v1, v2),

            // Array subtyping (covariant in element, invariant in size)
            // Spec: Arrays are covariant in element type when sizes match
            (
                Array {
                    element: e1,
                    size: s1,
                },
                Array {
                    element: e2,
                    size: s2,
                },
            ) => self.check_array_subtype(e1, s1, e2, s2),

            // Array -> List coercion
            // Array literal to List coercion: [1, 2, 3] infers as List<Int>, array literals coerce to List<T> - Array literals infer/coerce to List<T>
            // [T; N] <: List<T> - arrays can be coerced to lists
            (
                Array { element: e1, size: _ },
                Generic { name, args },
            ) if self.is_array_coercible(name.as_str()) && args.len() == 1 => {
                self.is_subtype(e1, &args[0])
            }
            // Array -> Named collection coercion (when collection type is parsed as Named)
            (
                Array { element: e1, size: _ },
                Named { path, args },
            ) if self.path_is_array_coercible(path) && args.len() == 1 => {
                self.is_subtype(e1, &args[0])
            }

            // Array -> Slice coercion
            // [T; N] <: [T] - arrays can be coerced to slices
            // This enables &[T; N] -> &[T] through reference subtyping
            // Array/slice coercion: fixed-size arrays [T; N] coerce to slices &[T] automatically
            (Array { element: e1, size: _ }, Slice { element: e2 }) => self.is_subtype(e1, e2),

            // Slice subtyping (covariant in element type)
            // [S] <: [T] if S <: T
            // This is safe because slices are read-only views
            // Collection covariance: List<Derived> <: List<Base> when Derived <: Base for immutable access
            (Slice { element: e1 }, Slice { element: e2 }) => self.is_subtype(e1, e2),

            // Reference subtyping with variance rules
            // Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9593-9602
            // Three-tier reference model: &T (managed, CBGR ~15ns), &checked T (statically verified, 0ns), &unsafe T (unchecked, 0ns). Memory layouts: ThinRef 16 bytes (ptr+generation+epoch), FatRef 24 bytes (+len) — Reference Coercion Rules
            (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => self.check_reference_subtype(*m1, i1, *m2, i2),

            // CheckedReference subtyping
            // Three-tier reference model: &T (managed, CBGR ~15ns), &checked T (statically verified, 0ns), &unsafe T (unchecked, 0ns). Memory layouts: ThinRef 16 bytes (ptr+generation+epoch), FatRef 24 bytes (+len) — line 636
            // &checked T <: &T (can forget compile-time proof, add runtime checks)
            (
                CheckedReference {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Checked -> Managed: Implicit upcast (forgetful)
                m1 == m2 && self.is_subtype(i1, i2)
            }

            (
                CheckedReference {
                    mutable: m1,
                    inner: i1,
                },
                CheckedReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Same tier: standard variance rules
                if *m1 && *m2 {
                    // Mutable: invariant
                    i1 == i2
                } else if !m1 && !m2 {
                    // Shared: covariant
                    self.is_subtype(i1, i2)
                } else {
                    false
                }
            }

            // UnsafeReference subtyping
            // Three-tier reference model: &T (managed, CBGR ~15ns), &checked T (statically verified, 0ns), &unsafe T (unchecked, 0ns). Memory layouts: ThinRef 16 bytes (ptr+generation+epoch), FatRef 24 bytes (+len) — line 636
            // &unsafe T <: &checked T <: &T (forgetful upcasts)
            (
                UnsafeReference {
                    mutable: m1,
                    inner: i1,
                },
                CheckedReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Unsafe -> Checked: Implicit upcast
                m1 == m2 && self.is_subtype(i1, i2)
            }

            (
                UnsafeReference {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Unsafe -> Managed: Implicit upcast (via Checked)
                m1 == m2 && self.is_subtype(i1, i2)
            }

            (
                UnsafeReference {
                    mutable: m1,
                    inner: i1,
                },
                UnsafeReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Same tier: standard variance rules
                if *m1 && *m2 {
                    // Mutable: invariant
                    i1 == i2
                } else if !m1 && !m2 {
                    // Shared: covariant
                    self.is_subtype(i1, i2)
                } else {
                    false
                }
            }

            // Ownership reference subtyping (same as Reference)
            (
                Ownership {
                    mutable: m1,
                    inner: i1,
                },
                Ownership {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                if *m1 && *m2 {
                    // Mutable: invariant
                    i1 == i2
                } else if !m1 && !m2 {
                    // Shared: covariant
                    self.is_subtype(i1, i2)
                } else {
                    false
                }
            }

            // Pointer subtyping: *mut T <: *const T (mutable can be used where const expected)
            // Three-tier reference model: &T (managed, CBGR ~15ns), &checked T (statically verified, 0ns), &unsafe T (unchecked, 0ns). Memory layouts: ThinRef 16 bytes (ptr+generation+epoch), FatRef 24 bytes (+len) — Pointer Coercion Rules
            //
            // This is a standard and safe coercion found in C, C++, and Rust:
            // - *mut T → *const T is safe (reading is a subset of read/write)
            // - *const T → *mut T is UNSAFE (would allow writing to read-only)
            //
            // The inner type must match exactly (no covariance/contravariance)
            // because pointers can be written through and read from.
            //
            // Examples:
            // ```verum
            // let p: *mut Int = alloc(1);
            // let q: *const Int = p;  // OK: *mut Int <: *const Int
            //
            // let r: *const Int = ...;
            // let s: *mut Int = r;    // ERROR: *const Int </: *mut Int
            // ```
            (
                Pointer {
                    mutable: m1,
                    inner: i1,
                },
                Pointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Inner types must match exactly (invariant in pointed-to type)
                if i1 != i2 {
                    return false;
                }

                match (m1, m2) {
                    // Same mutability: allowed
                    (true, true) | (false, false) => true,
                    // *mut T → *const T: safe coercion (forgetting write capability)
                    (true, false) => true,
                    // *const T → *mut T: UNSAFE, not allowed as implicit subtyping
                    (false, true) => false,
                }
            }

            // Volatile pointer subtyping (same rules as raw pointer)
            (
                VolatilePointer {
                    mutable: m1,
                    inner: i1,
                },
                VolatilePointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                if i1 != i2 {
                    return false;
                }
                match (m1, m2) {
                    (true, true) | (false, false) => true,
                    (true, false) => true,
                    (false, true) => false,
                }
            }

            // Meta parameter subtyping
            // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Meta parameters are compile-time values
            // Meta parameters are subtypes if:
            // 1. Names match (same compile-time parameter)
            // 2. Base types are subtypes
            // 3. Refinements are compatible (if present)
            (
                Meta {
                    name: n1,
                    ty: t1,
                    refinement: r1,
                    value: v1,
                },
                Meta {
                    name: n2,
                    ty: t2,
                    refinement: r2,
                    value: v2,
                },
            ) => {
                // Concrete values take precedence over names for subtyping checks,
                // matching the unification behavior (see unify.rs). Two Metas with
                // concrete compile-time values are in a subtype relation iff the
                // values are equal.
                match (v1, v2) {
                    (Some(a), Some(b)) => {
                        if a != b {
                            return false;
                        }
                    }
                    (Some(_), None) | (None, Some(_)) => {
                        // Mixed: fall through to base-type + refinement check;
                        // a concrete value always inhabits the variable meta slot.
                    }
                    (None, None) => {
                        // Names must match (same meta parameter)
                        if n1 != n2 {
                            return false;
                        }
                    }
                }

                // Base types must be subtypes
                if !self.is_subtype(t1, t2) {
                    return false;
                }

                // Refinements must be compatible
                match (r1, r2) {
                    (Some(_pred1), Some(_pred2)) => {
                        // Previously delegated to `crate::smt_backend::check_subsumption_smt`,
                        // which has been moved to `verum_smt::refinement_backend`
                        // to break the `verum_types ↔ verum_smt` cycle.
                        // `Subtyping` conservative-rejects without an
                        // injected SMT backend; callers that want SMT
                        // subsumption should use `RefinementChecker`.
                        false
                    }
                    (None, None) => true,
                    (Some(_), None) => true, // Stronger predicate is subtype
                    (None, Some(_)) => false, // Weaker is not subtype
                }
            }

            // Tensor subtyping
            // Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays
            //
            // Tensors are subtypes if:
            // 1. Element types are covariant (T1 <: T2)
            // 2. Shapes are invariant (must match exactly)
            //
            // Example:
            // Tensor<Int{> 0}, [2, 3]> <: Tensor<Int, [2, 3]>  // OK: covariant element
            // Tensor<Int, [2, 3]> </: Tensor<Int, [3, 2]>      // ERROR: shapes differ
            (
                Tensor {
                    element: e1,
                    shape: s1,
                    ..
                },
                Tensor {
                    element: e2,
                    shape: s2,
                    ..
                },
            ) => {
                // Shapes must match exactly (invariant)
                if s1 != s2 {
                    return false;
                }

                // Element type is covariant
                self.is_subtype(e1, e2)
            }

            // Universe subtyping (Cumulativity)
            // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe Hierarchy
            //
            // Universe cumulativity: Type_n <: Type_{n+1}
            // This means any type at level n can be used where a type at level n+1 is expected.
            //
            // Examples:
            // - Type₀ <: Type₁ <: Type₂ ...
            // - Prop <: Type₁
            (Universe { level: l1 }, Universe { level: l2 }) => self.check_universe_subtype(l1, l2),

            // Prop is a subtype of Type₁
            // Inductive types: recursive type definitions with structural recursion, termination checking — .1 - Proof Irrelevance
            //
            // Prop : Type₁, so Prop <: Type₁
            // But Type₀ ≢ Prop (different universes for data vs proofs)
            (Prop, Universe { level }) => {
                // Prop is in Type₁, so Prop <: Type_n for n >= 1
                use crate::ty::UniverseLevel;
                match level {
                    UniverseLevel::Concrete(n) => *n >= 1,
                    UniverseLevel::Succ(_) => true, // succ(k) >= 1 for any k
                    UniverseLevel::Max(_, _) => true, // max(a,b) >= 1 if either >= 1
                    UniverseLevel::Variable(_) => true, // conservative: allow (resolved at use-site)
                }
            }

            // Pi type subtyping
            // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi Types
            //
            // (x: A) -> B(x) <: (x: A') -> B'(x) if:
            // - A' <: A (contravariant in parameter type)
            // - B(x) <: B'(x) (covariant in return type, under x : A')
            (
                Pi {
                    param_name: n1,
                    param_type: p1,
                    return_type: r1,
                },
                Pi {
                    param_name: n2,
                    param_type: p2,
                    return_type: r2,
                },
            ) => {
                // Contravariant in parameter: p2 <: p1
                if !self.is_subtype(p2, p1) {
                    return false;
                }
                // Covariant in return: r1 <: r2
                // Note: For a full implementation, we should substitute p2 for n1 in r1
                // and check under the context where the parameter has type p2.
                // For now, we use a simpler approximation.
                self.is_subtype(r1, r2)
            }

            // Sigma type subtyping
            // Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma — Sigma Types
            //
            // (x: A, B(x)) <: (x: A', B'(x)) if:
            // - A <: A' (covariant in first component)
            // - B(x) <: B'(x) (covariant in second component)
            (
                Sigma {
                    fst_name: n1,
                    fst_type: f1,
                    snd_type: s1,
                },
                Sigma {
                    fst_name: n2,
                    fst_type: f2,
                    snd_type: s2,
                },
            ) => {
                // Covariant in first: f1 <: f2
                if !self.is_subtype(f1, f2) {
                    return false;
                }
                // Covariant in second: s1 <: s2
                self.is_subtype(s1, s2)
            }

            // Equality type subtyping
            // Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — Equality Types
            //
            // Eq<A, x, y> <: Eq<A', x', y'> if:
            // - A <: A' (covariant in carrier type)
            // - x = x' and y = y' (terms must be identical)
            (
                Eq {
                    ty: t1,
                    lhs: l1,
                    rhs: r1,
                },
                Eq {
                    ty: t2,
                    lhs: l2,
                    rhs: r2,
                },
            ) => {
                // Covariant in type
                if !self.is_subtype(t1, t2) {
                    return false;
                }
                // Terms must be identical
                l1 == l2 && r1 == r2
            }

            // Inductive type subtyping
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1 - Inductive Types
            //
            // Inductive types with the same name are subtypes if their indices are subtypes.
            (
                Inductive {
                    name: n1,
                    params: p1,
                    indices: i1,
                    universe: u1,
                    ..
                },
                Inductive {
                    name: n2,
                    params: p2,
                    indices: i2,
                    universe: u2,
                    ..
                },
            ) => {
                // Names must match
                if n1 != n2 {
                    return false;
                }
                // Universe levels must be compatible
                if !self.check_universe_subtype(u1, u2) {
                    return false;
                }
                // Parameters and indices must be subtypes
                // params and indices are List<(Text, Box<Type>)>
                p1.len() == p2.len()
                    && i1.len() == i2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|((_, t1), (_, t2))| self.is_subtype(t1.as_ref(), t2.as_ref()))
                    && i1
                        .iter()
                        .zip(i2.iter())
                        .all(|((_, t1), (_, t2))| self.is_subtype(t1.as_ref(), t2.as_ref()))
            }

            // Auto-reference coercion: Text can be used where &Text is expected
            // This enables passing string literals directly to functions expecting references
            // Three-tier reference model: &T (managed, CBGR ~15ns), &checked T (statically verified, 0ns), &unsafe T (unchecked, 0ns). Memory layouts: ThinRef 16 bytes (ptr+generation+epoch), FatRef 24 bytes (+len) — Reference Coercion Rules
            //
            // An owned value can always be borrowed immutably:
            // - The lifetime is managed by the caller
            // - This is consistent with Rust's auto-ref behavior
            // - Semantically correct: Text <: &Text for immutable references
            (
                Type::Text,
                Type::Reference {
                    mutable: false,
                    inner,
                },
            ) if matches!(inner.as_ref(), Type::Text) => true,

            // Similarly for Char
            (
                Type::Char,
                Type::Reference {
                    mutable: false,
                    inner,
                },
            ) if matches!(inner.as_ref(), Type::Char) => true,

            // Existential type subtyping
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .4 - Existential Subtyping
            //
            // Two existentials: (some a. S) <: (some b. T) if S[a/witness] <: T[b/witness]
            // for a fresh witness type variable.
            (
                Exists {
                    var: var1,
                    body: body1,
                },
                Exists {
                    var: var2,
                    body: body2,
                },
            ) => {
                // Create a fresh type variable to serve as the witness
                let witness_var = TypeVar::fresh();
                let witness = Type::Var(witness_var);

                // Substitute the witness for both existential variables
                let mut subst1 = crate::ty::Substitution::new();
                subst1.insert(*var1, witness.clone());
                let body1_subst = body1.apply_subst(&subst1);

                let mut subst2 = crate::ty::Substitution::new();
                subst2.insert(*var2, witness);
                let body2_subst = body2.apply_subst(&subst2);

                // Check if the substituted bodies are subtypes
                self.is_subtype(&body1_subst, &body2_subst)
            }

            // Concrete type packing into existential:
            // T <: (some a. S) if T <: S[a/T]
            (concrete, Exists { var, body }) if !matches!(concrete, Exists { .. }) => {
                let mut subst = crate::ty::Substitution::new();
                subst.insert(*var, concrete.clone());
                let body_subst = body.apply_subst(&subst);
                self.is_subtype(concrete, &body_subst)
            }

            // Existential unpacking to concrete (conservative)
            (Exists { var, body }, concrete) if !matches!(concrete, Exists { .. }) => {
                let fresh_var = TypeVar::fresh();
                let mut subst = crate::ty::Substitution::new();
                subst.insert(*var, Type::Var(fresh_var));
                let body_subst = body.apply_subst(&subst);
                self.is_subtype(&body_subst, concrete)
            }

            // Universal type subtyping
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — Universal Types
            //
            // (forall a. S) <: (forall b. T) if S[a/fresh] <: T[b/fresh]
            (
                Forall {
                    vars: vars1,
                    body: body1,
                },
                Forall {
                    vars: vars2,
                    body: body2,
                },
            ) => {
                // Must have same arity
                if vars1.len() != vars2.len() {
                    return false;
                }

                // Create fresh type variables for the bound vars
                let fresh_vars: List<Type> = vars1
                    .iter()
                    .map(|_| Type::Var(TypeVar::fresh()))
                    .collect();

                // Substitute the fresh vars for both bodies
                let mut subst1 = crate::ty::Substitution::new();
                for (v, fresh) in vars1.iter().zip(fresh_vars.iter()) {
                    subst1.insert(*v, fresh.clone());
                }
                let body1_subst = body1.apply_subst(&subst1);

                let mut subst2 = crate::ty::Substitution::new();
                for (v, fresh) in vars2.iter().zip(fresh_vars.iter()) {
                    subst2.insert(*v, fresh.clone());
                }
                let body2_subst = body2.apply_subst(&subst2);

                // Check body subtyping
                self.is_subtype(&body1_subst, &body2_subst)
            }

            // Rank-2: Forall is subtype of non-Forall by instantiation
            (Forall { vars, body }, other) if !matches!(other, Forall { .. }) => {
                let mut subst = crate::ty::Substitution::new();
                for v in vars.iter() {
                    subst.insert(*v, Type::Var(TypeVar::fresh()));
                }
                let instantiated = body.apply_subst(&subst);
                self.is_subtype(&instantiated, other)
            }

            // Non-Forall is subtype of Forall (must work for all instantiations)
            (other, Forall { vars, body }) if !matches!(other, Forall { .. }) => {
                let mut subst = crate::ty::Substitution::new();
                for v in vars.iter() {
                    subst.insert(*v, Type::Var(TypeVar::fresh()));
                }
                let instantiated = body.apply_subst(&subst);
                self.is_subtype(other, &instantiated)
            }

            // Capability-restricted type subtyping
            // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
            //
            // T with [A, B, C] <: T with [A, B] when:
            // 1. Base types are subtypes: t1.base <: t2.base
            // 2. t1's capabilities are a SUPERSET of t2's capabilities
            //
            // "More capabilities" is a subtype of "fewer capabilities" because:
            // - A value with [Read, Write] can be used where [Read] is expected
            // - The extra capabilities (Write) are simply not used
            // - This enables automatic capability attenuation at call sites
            //
            // This is contravariant in capabilities (more caps -> subtype)
            //
            // Example:
            // ```verum
            // fn analyze(db: Database with [Read]) -> Stats { ... }
            // fn process(db: Database with [Read, Write]) {
            //     analyze(db);  // OK: [Read, Write] ⊇ [Read]
            // }
            // ```
            (
                CapabilityRestricted {
                    base: b1,
                    capabilities: c1,
                },
                CapabilityRestricted {
                    base: b2,
                    capabilities: c2,
                },
            ) => {
                // Base types must be subtypes
                if !self.is_subtype(b1, b2) {
                    return false;
                }

                // t1's capabilities must be a superset of t2's capabilities
                // (more capabilities = subtype, contravariant in capability sets)
                // Uses TypeCapabilitySet::is_subset_of for proper set comparison
                c2.is_subset_of(c1)
            }

            // Capability-restricted to base type coercion
            // T with [Caps] <: T (forgetful upcast)
            // A capability-restricted value can be used where the unrestricted base is expected
            (
                CapabilityRestricted { base, .. },
                t2,
            ) => self.is_subtype(base, t2),

            // Base type to capability-restricted: NOT allowed automatically
            // T </: T with [Caps] (would need to prove capabilities are available)
            // This must be done explicitly via `provide` or explicit casting

            // =============================================================================
            // Unknown type subtyping (top type)
            // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 13.2 - Unknown Type
            // =============================================================================
            //
            // Unknown is the TOP type (dual of Never):
            // - Any type T <: unknown (any value can be assigned to unknown)
            // - unknown <: T only if T == unknown (nothing can be done without narrowing)
            //
            // This enables safe FFI, deserialization, and rapid prototyping by
            // forcing explicit type narrowing (via `x is T` or pattern matching).

            // T <: unknown for any T (unknown is the top type)
            // Any value can be assigned to unknown
            (_, Unknown) => true,

            // unknown <: T only if T == unknown
            // This case is already handled by reflexivity (t1 == t2 => true)
            // So we don't need an explicit case here

            _ => false,
        }
    }

    /// Check record subtyping with width and depth subtyping.
    ///
    /// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9607-9619
    ///
    /// Width subtyping: S can have MORE fields than T
    /// Depth subtyping: Matching fields must be subtypes
    ///
    /// Example:
    /// ```verum
    /// type Point2D is { x: Float, y: Float }
    /// type Point3D is { x: Float, y: Float, z: Float }
    /// // Point3D <: Point2D (extra field z ignored)
    /// ```
    fn check_record_subtype(
        &self,
        fields1: &indexmap::IndexMap<Text, Type>,
        fields2: &indexmap::IndexMap<Text, Type>,
    ) -> bool {
        // All fields in fields2 must be in fields1 (width subtyping)
        // And their types must be subtypes (depth subtyping)
        for (field_name, field_type2) in fields2 {
            match fields1.get(field_name) {
                Some(field_type1) => {
                    // Depth subtyping: field types must be subtypes
                    if !self.is_subtype(field_type1, field_type2) {
                        return false;
                    }
                }
                None => {
                    // fields1 missing required field from fields2
                    return false;
                }
            }
        }

        // fields1 can have extra fields (width subtyping)
        true
    }

    /// Check variant (sum type) subtyping.
    ///
    /// Variants are dual to records:
    /// - Contravariant in tags: fewer tags = subtype
    /// - Covariant in variant types
    ///
    /// Example:
    /// ```verum
    /// type Shape2D is Circle(Float) | Square(Float)
    /// type Shape is Circle(Float) | Square(Float) | Triangle(Float)
    /// // Shape2D <: Shape? NO! Shape <: Shape2D? NO!
    /// // For sum types: S <: T if S's tags ⊆ T's tags
    /// ```
    ///
    /// Actually, for safe variance:
    /// S <: T if for each tag in S, T also has that tag with a subtype
    fn check_variant_subtype(
        &self,
        variants1: &indexmap::IndexMap<Text, Type>,
        variants2: &indexmap::IndexMap<Text, Type>,
    ) -> bool {
        // For each variant tag in variants1, variants2 must also have that tag
        // And the type for each matching tag in variants1 must be subtype of type in variants2
        for (tag, type1) in variants1 {
            match variants2.get(tag) {
                Some(type2) => {
                    // Covariance in variant types
                    if !self.is_subtype(type1, type2) {
                        return false;
                    }
                }
                None => {
                    // variants1 has tag not in variants2 - not a subtype
                    return false;
                }
            }
        }

        // variants2 can have extra tags (safe for exhaustiveness)
        true
    }

    /// Check array subtyping: covariant in element type, invariant in size.
    ///
    /// Arrays are fixed-size, so sizes must match exactly.
    /// Element types are covariant (read-only access).
    ///
    /// Example:
    /// ```verum
    /// type PositiveArray is [Int{> 0}; 10]
    /// type IntArray is [Int; 10]
    /// // PositiveArray <: IntArray
    /// // [Int; 10] </: [Int; 20] (different sizes)
    /// ```
    fn check_array_subtype(
        &self,
        elem1: &Type,
        size1: &Option<usize>,
        elem2: &Type,
        size2: &Option<usize>,
    ) -> bool {
        // Sizes must match exactly (invariant)
        if size1 != size2 {
            return false;
        }

        // Element types are covariant
        self.is_subtype(elem1, elem2)
    }

    /// Check reference subtyping with variance rules.
    ///
    /// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9593-9602
    ///
    /// Variance rules:
    /// - Shared references (&T): covariant in T
    /// - Mutable references (&mut T): invariant in T
    ///
    /// Example:
    /// ```verum
    /// let x: Int{> 0} = 10;
    /// let r: &Int = &x;  // OK: &Int{> 0} <: &Int (shared refs covariant)
    ///
    /// let mut y: Int = 5;
    /// let r_mut: &mut Int{> 0} = &mut y;  // ERROR: &mut refs invariant
    /// ```
    fn check_reference_subtype(&self, m1: bool, i1: &Type, m2: bool, i2: &Type) -> bool {
        if m1 && m2 {
            // Both mutable: invariant
            // &mut S <: &mut T iff S == T
            i1 == i2
        } else if !m1 && !m2 {
            // Both shared: covariant
            // &S <: &T if S <: T
            self.is_subtype(i1, i2)
        } else if m1 && !m2 {
            // &mut T <: &T (mutable reference can be used as shared)
            // This is safe because shared references only allow reading
            // Reference coercion: &mut T coerces to &T, &T coerces to &dyn Protocol when Protocol is implemented
            self.is_subtype(i1, i2)
        } else {
            // &T cannot become &mut T (unsafe)
            false
        }
    }

    /// Check function subtyping: contravariant in parameters, covariant in return.
    ///
    /// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9621-9638
    /// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Context requirement subtyping
    ///
    /// Function subtyping rules:
    /// - Parameters: contravariant (accept more inputs)
    /// - Return type: covariant (produce more specific outputs)
    /// - Contexts: S can have FEWER contexts than T
    ///
    /// Example:
    /// ```verum
    /// type IntToBool is Int -> Bool
    /// type PosToBool is Int{> 0} -> Bool
    /// // IntToBool <: PosToBool (accepts more inputs)
    /// ```
    fn check_function_subtype(
        &self,
        params1: &[Type],
        return1: &Type,
        contexts1: &Option<crate::di::requirement::ContextExpr>,
        params2: &[Type],
        return2: &Type,
        contexts2: &Option<crate::di::requirement::ContextExpr>,
    ) -> bool {
        // Parameter counts must match
        if params1.len() != params2.len() {
            return false;
        }

        // Params are contravariant (reversed subtyping)
        for (pt1, pt2) in params1.iter().zip(params2.iter()) {
            if !self.is_subtype(pt2, pt1) {
                // NOTE: reversed!
                return false;
            }
        }

        // Return type is covariant
        if !self.is_subtype(return1, return2) {
            return false;
        }

        // Context subtyping: S can have FEWER contexts than T
        // This allows passing pure functions (None) where contextful functions expected
        // Context group expansion: resolving context group names to their constituent contexts recursively — Context requirement subtyping
        self.check_context_subtype(contexts1, contexts2)
    }

    /// Check context subtyping: S can have FEWER contexts than T.
    ///
    /// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3 line 9663-9666
    /// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Context requirement subtyping
    ///
    /// This allows passing pure functions where effectful functions are expected.
    ///
    /// Rules:
    /// - None (no contexts) is a subtype of any context requirement (pure functions can be used anywhere)
    /// - Some(Concrete(req1)) <: Some(Concrete(req2)) if req1.is_subset_of(req2)
    /// - Variable contexts are handled conservatively (assume compatible during inference)
    ///
    /// Example:
    /// ```verum
    /// fn pure_fn(x: Int) -> Int { x + 1 }
    /// fn takes_io_fn(f: Int -> Int using [IO]) { ... }
    /// takes_io_fn(pure_fn)  // OK: None <: Some(IO)
    /// ```
    fn check_context_subtype(
        &self,
        contexts1: &Option<crate::di::requirement::ContextExpr>,
        contexts2: &Option<crate::di::requirement::ContextExpr>,
    ) -> bool {
        use crate::di::requirement::ContextExpr;

        match (contexts1, contexts2) {
            // No contexts (pure) is a subtype of any context requirement
            (None, _) => true,
            // Some contexts cannot be a subtype of no contexts (unless variable)
            (Some(ContextExpr::Variable(_)), None) => true, // Variable might be empty
            (Some(ContextExpr::Concrete(_)), None) => false,
            // Both concrete: check subset relationship
            (Some(ContextExpr::Concrete(req1)), Some(ContextExpr::Concrete(req2))) => {
                req1.is_subset_of(req2)
            }
            // Variables: assume compatible during inference (will be checked at unification)
            (Some(ContextExpr::Variable(_)), Some(_)) => true,
            (Some(_), Some(ContextExpr::Variable(_))) => true,
        }
    }

    /// Check universe subtyping (cumulativity).
    ///
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe Hierarchy
    ///
    /// Universe cumulativity: Type_n <: Type_m if n <= m
    /// This allows types at lower levels to be used where higher levels are expected.
    ///
    /// # Rules:
    /// - Concrete(n) <: Concrete(m) if n <= m
    /// - Variable(i) <: Variable(j) is constraint-checked later
    /// - Succ(n) <: Succ(m) if n <= m
    /// - Max(a, b) <: Concrete(n) if max(a, b) <= n
    ///
    /// # Examples:
    /// ```verum
    /// // Type₀ <: Type₁ <: Type₂
    /// let ty: Type₁ = Type₀;  // OK: cumulativity
    /// ```
    fn check_universe_subtype(
        &self,
        l1: &crate::ty::UniverseLevel,
        l2: &crate::ty::UniverseLevel,
    ) -> bool {
        use crate::ty::UniverseLevel;

        match (l1, l2) {
            // Concrete levels: n <= m for cumulativity
            (UniverseLevel::Concrete(n1), UniverseLevel::Concrete(n2)) => n1 <= n2,

            // Level variables: constraint-checked later, conservatively accept
            (UniverseLevel::Variable(_), _) | (_, UniverseLevel::Variable(_)) => true,

            // Successor levels: Succ(n) <: Succ(m) if n <= m
            (UniverseLevel::Succ(n1), UniverseLevel::Succ(n2)) => n1 <= n2,

            // Succ(n) <: Concrete(m) if n + 1 <= m
            (UniverseLevel::Succ(n), UniverseLevel::Concrete(m)) => (*n + 1) <= *m,

            // Concrete(n) <: Succ(m) if n <= m + 1
            (UniverseLevel::Concrete(n), UniverseLevel::Succ(m)) => *n <= (*m + 1),

            // Max(a, b) <: Concrete(n) if both a <= n and b <= n
            (UniverseLevel::Max(a, b), UniverseLevel::Concrete(n)) => *a <= *n && *b <= *n,

            // Concrete(n) <: Max(a, b) if n <= min(a, b) - conservative
            (UniverseLevel::Concrete(n), UniverseLevel::Max(a, b)) => {
                // Conservative: n must be <= max(a, b) = min(a, b) for soundness
                *n <= std::cmp::min(*a, *b)
            }

            // Max(a1, b1) <: Max(a2, b2) if both components are <=
            (UniverseLevel::Max(a1, b1), UniverseLevel::Max(a2, b2)) => {
                // max(a1, b1) <= max(a2, b2) if a1 <= max(a2, b2) && b1 <= max(a2, b2)
                let max_rhs = std::cmp::max(*a2, *b2);
                *a1 <= max_rhs && *b1 <= max_rhs
            }

            // Max to Succ: conservative
            (UniverseLevel::Max(a, b), UniverseLevel::Succ(m)) => *a <= (*m + 1) && *b <= (*m + 1),

            (UniverseLevel::Succ(n), UniverseLevel::Max(a, b)) => (*n + 1) <= std::cmp::max(*a, *b),
        }
    }
}

impl Default for Subtyping {
    fn default() -> Self {
        Self::new()
    }
}

// Tests moved to tests/subtype_tests.rs

// ==================== Syntactic Subsumption Fast Path ====================
// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3.1 lines 9696-9707
// Syntactic subsumption patterns - complete reference for type subsumption via syntactic checks

/// Check syntactic subsumption for common refinement patterns.
///
/// This provides a fast path that resolves >80% of refinement subsumption checks
/// in <1ms without invoking the SMT solver.
///
/// Returns:
/// - Some(true): Predicates syntactically subsume
/// - Some(false): Predicates syntactically don't subsume
/// - None: Cannot determine syntactically, need SMT solver
///
/// Subtyping: structural subtyping for records, refinement subtyping (T{P} <: T when P holds), protocol-based nominal subtyping — .3.1 lines 9696-9707
/// Syntactic subsumption: fast-path type checking via pattern matching before full unification
fn check_syntactic_subsumption(phi1: &verum_ast::Expr, phi2: &verum_ast::Expr) -> Option<bool> {
    use verum_ast::expr::{BinOp, ExprKind};
    use verum_ast::literal::{Literal, LiteralKind};

    // Reflexivity: φ => φ (always valid)
    if phi1 == phi2 {
        return Some(true);
    }

    // Boolean tautologies
    match (&phi1.kind, &phi2.kind) {
        // true => φ (always valid - true is strongest predicate)
        (
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(true),
                ..
            }),
            _,
        ) => return Some(true),
        // φ => true (always valid - true is weakest constraint)
        (
            _,
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(true),
                ..
            }),
        ) => return Some(true),
        // false => φ (always valid - vacuous truth)
        (
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(false),
                ..
            }),
            _,
        ) => return Some(true),
        _ => {}
    }

    // Pattern: Conjunction weakening (φ1 && φ2) => φ
    if let ExprKind::Binary {
        op: BinOp::And,
        left: l1,
        right: r1,
    } = &phi1.kind
    {
        // (a && b) => a OR (a && b) => b
        if check_syntactic_subsumption(l1, phi2) == Some(true) {
            return Some(true);
        }
        if check_syntactic_subsumption(r1, phi2) == Some(true) {
            return Some(true);
        }
    }

    // Pattern: Disjunction strengthening φ => (φ1 || φ2)
    if let ExprKind::Binary {
        op: BinOp::Or,
        left: l2,
        right: r2,
    } = &phi2.kind
    {
        // a => (a || b) OR b => (a || b)
        if check_syntactic_subsumption(phi1, l2) == Some(true) {
            return Some(true);
        }
        if check_syntactic_subsumption(phi1, r2) == Some(true) {
            return Some(true);
        }
    }

    // Numeric comparison patterns for refinement type subsumption
    //
    // This uses syntactic pattern matching to decide common cases efficiently,
    // without invoking an SMT solver. The patterns cover:
    //
    // 1. x > N1 => x > N2 when N1 >= N2 (stronger lower bound implies weaker)
    // 2. x >= N1 => x >= N2 when N1 >= N2
    // 3. x < N1 => x < N2 when N1 <= N2 (stronger upper bound implies weaker)
    // 4. x <= N1 => x <= N2 when N1 <= N2
    // 5. x != N => x != N (equality preservation)
    //
    // For cases not covered by these patterns, return None to indicate
    // that SMT verification is needed (handled by caller).
    //
    // Type inference: Hindley-Milner algorithm W with extensions for refinement types, bidirectional type checking, and constraint-based inference — .3 - Refinement Type Subsumption
    match (&phi1.kind, &phi2.kind) {
        // Pattern: x > N1 => x > N2 if N1 >= N2
        (
            ExprKind::Binary {
                op: BinOp::Gt,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Gt,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                return Some(n1 >= n2);
            }
        }

        // Pattern: x >= N1 => x >= N2 if N1 >= N2
        (
            ExprKind::Binary {
                op: BinOp::Ge,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Ge,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                return Some(n1 >= n2);
            }
        }

        // Pattern: x < N1 => x < N2 if N1 <= N2
        (
            ExprKind::Binary {
                op: BinOp::Lt,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Lt,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                return Some(n1 <= n2);
            }
        }

        // Pattern: x <= N1 => x <= N2 if N1 <= N2
        (
            ExprKind::Binary {
                op: BinOp::Le,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Le,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                return Some(n1 <= n2);
            }
        }

        // Mixed operator patterns for cross-comparison subsumption
        // Pattern: x > N1 => x >= N2 if N1 >= N2 (gt implies ge with same bound)
        (
            ExprKind::Binary {
                op: BinOp::Gt,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Ge,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                // x > N1 implies x >= N1+1, so x > N1 => x >= N2 if N1+1 >= N2
                return Some(n1 + 1 >= n2);
            }
        }

        // Pattern: x >= N1 => x > N2 if N1 > N2 (ge implies gt with smaller bound)
        (
            ExprKind::Binary {
                op: BinOp::Ge,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Gt,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                // x >= N1 implies x > N1-1, so x >= N1 => x > N2 if N1-1 >= N2
                return Some(n1 > n2);
            }
        }

        // Pattern: x < N1 => x <= N2 if N1 <= N2+1 (lt implies le)
        (
            ExprKind::Binary {
                op: BinOp::Lt,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Le,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                // x < N1 implies x <= N1-1, so x < N1 => x <= N2 if N1-1 <= N2
                return Some(n1 - 1 <= n2);
            }
        }

        // Pattern: x <= N1 => x < N2 if N1 < N2 (le implies lt with larger bound)
        (
            ExprKind::Binary {
                op: BinOp::Le,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Lt,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                // x <= N1 implies x < N1+1, so x <= N1 => x < N2 if N1+1 <= N2
                return Some(n1 < n2);
            }
        }

        // Pattern: x == N => x >= N (equality implies ge)
        (
            ExprKind::Binary {
                op: BinOp::Eq,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Ge,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                // x == N1 => x >= N2 if N1 >= N2
                return Some(n1 >= n2);
            }
        }

        // Pattern: x == N => x <= N (equality implies le)
        (
            ExprKind::Binary {
                op: BinOp::Eq,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Le,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                // x == N1 => x <= N2 if N1 <= N2
                return Some(n1 <= n2);
            }
        }

        // Pattern: x == N => x > N2 only if N > N2
        (
            ExprKind::Binary {
                op: BinOp::Eq,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Gt,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                return Some(n1 > n2);
            }
        }

        // Pattern: x == N => x < N2 only if N < N2
        (
            ExprKind::Binary {
                op: BinOp::Eq,
                left: var1,
                right: val1,
            },
            ExprKind::Binary {
                op: BinOp::Lt,
                left: var2,
                right: val2,
            },
        ) if var1 == var2 => {
            if let (Some(n1), Some(n2)) = (extract_int_literal(val1), extract_int_literal(val2)) {
                return Some(n1 < n2);
            }
        }

        // Pattern: Combined bounds (x >= A && x <= B) => x >= C or x <= C
        (
            ExprKind::Binary {
                op: BinOp::And,
                left: conj1,
                right: conj2,
            },
            ExprKind::Binary { op: op2, .. },
        ) if matches!(op2, BinOp::Ge | BinOp::Le | BinOp::Gt | BinOp::Lt) => {
            // Check if either conjunct implies the target
            if check_syntactic_subsumption(conj1, phi2) == Some(true)
                || check_syntactic_subsumption(conj2, phi2) == Some(true)
            {
                return Some(true);
            }
        }

        // Pattern: Negation of comparison (!(x < N)) => x >= N
        (
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Not,
                expr: inner,
            },
            ExprKind::Binary {
                op: op2,
                left: var2,
                right: val2,
            },
        ) => {
            // !(x < N) is equivalent to x >= N
            if let ExprKind::Binary {
                op: BinOp::Lt,
                left: var1,
                right: val1,
            } = &inner.kind
            {
                if var1 == var2 {
                    if let (Some(n1), Some(n2)) =
                        (extract_int_literal(val1), extract_int_literal(val2))
                    {
                        match op2 {
                            BinOp::Ge => return Some(n1 >= n2),
                            BinOp::Gt => return Some(n1 > n2),
                            _ => {}
                        }
                    }
                }
            }
            // !(x > N) is equivalent to x <= N
            if let ExprKind::Binary {
                op: BinOp::Gt,
                left: var1,
                right: val1,
            } = &inner.kind
            {
                if var1 == var2 {
                    if let (Some(n1), Some(n2)) =
                        (extract_int_literal(val1), extract_int_literal(val2))
                    {
                        match op2 {
                            BinOp::Le => return Some(n1 <= n2),
                            BinOp::Lt => return Some(n1 < n2),
                            _ => {}
                        }
                    }
                }
            }
            // !(x <= N) is equivalent to x > N
            if let ExprKind::Binary {
                op: BinOp::Le,
                left: var1,
                right: val1,
            } = &inner.kind
            {
                if var1 == var2 {
                    if let (Some(n1), Some(n2)) =
                        (extract_int_literal(val1), extract_int_literal(val2))
                    {
                        match op2 {
                            BinOp::Gt => return Some(n1 >= n2),
                            BinOp::Ge => return Some(n1 > n2),
                            _ => {}
                        }
                    }
                }
            }
            // !(x >= N) is equivalent to x < N
            if let ExprKind::Binary {
                op: BinOp::Ge,
                left: var1,
                right: val1,
            } = &inner.kind
            {
                if var1 == var2 {
                    if let (Some(n1), Some(n2)) =
                        (extract_int_literal(val1), extract_int_literal(val2))
                    {
                        match op2 {
                            BinOp::Lt => return Some(n1 <= n2),
                            BinOp::Le => return Some(n1 < n2),
                            _ => {}
                        }
                    }
                }
            }
        }

        // Pattern: Arithmetic expressions: (x + k) > N1 => x > N2
        (
            ExprKind::Binary {
                op: op1,
                left: left1,
                right: val1,
            },
            ExprKind::Binary {
                op: op2,
                left: left2,
                right: val2,
            },
        ) if matches!(op1, BinOp::Gt | BinOp::Ge | BinOp::Lt | BinOp::Le)
            && matches!(op2, BinOp::Gt | BinOp::Ge | BinOp::Lt | BinOp::Le) =>
        {
            // Check if left sides have arithmetic addition relationship
            if let (
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: base1,
                    right: offset1,
                },
                ExprKind::Path(_),
            ) = (&left1.kind, &left2.kind)
            {
                // (x + k) op1 N1 => x op2 N2
                if base1 == left2 {
                    if let (Some(k), Some(n1), Some(n2)) = (
                        extract_int_literal(offset1),
                        extract_int_literal(val1),
                        extract_int_literal(val2),
                    ) {
                        // Adjust n1 by subtracting k and compare
                        let adjusted_n1 = n1 - k;
                        match (op1, op2) {
                            (BinOp::Gt, BinOp::Gt) | (BinOp::Ge, BinOp::Ge) => {
                                return Some(adjusted_n1 >= n2);
                            }
                            (BinOp::Lt, BinOp::Lt) | (BinOp::Le, BinOp::Le) => {
                                return Some(adjusted_n1 <= n2);
                            }
                            (BinOp::Gt, BinOp::Ge) => return Some(adjusted_n1 + 1 >= n2),
                            (BinOp::Ge, BinOp::Gt) => return Some(adjusted_n1 > n2),
                            (BinOp::Lt, BinOp::Le) => return Some(adjusted_n1 - 1 <= n2),
                            (BinOp::Le, BinOp::Lt) => return Some(adjusted_n1 < n2),
                            _ => {}
                        }
                    }
                }
            }

            // Check if left sides have subtraction relationship
            if let (
                ExprKind::Binary {
                    op: BinOp::Sub,
                    left: base1,
                    right: offset1,
                },
                ExprKind::Path(_),
            ) = (&left1.kind, &left2.kind)
            {
                // (x - k) op1 N1 => x op2 N2
                if base1 == left2 {
                    if let (Some(k), Some(n1), Some(n2)) = (
                        extract_int_literal(offset1),
                        extract_int_literal(val1),
                        extract_int_literal(val2),
                    ) {
                        // Adjust n1 by adding k and compare
                        let adjusted_n1 = n1 + k;
                        match (op1, op2) {
                            (BinOp::Gt, BinOp::Gt) | (BinOp::Ge, BinOp::Ge) => {
                                return Some(adjusted_n1 >= n2);
                            }
                            (BinOp::Lt, BinOp::Lt) | (BinOp::Le, BinOp::Le) => {
                                return Some(adjusted_n1 <= n2);
                            }
                            (BinOp::Gt, BinOp::Ge) => return Some(adjusted_n1 + 1 >= n2),
                            (BinOp::Ge, BinOp::Gt) => return Some(adjusted_n1 > n2),
                            (BinOp::Lt, BinOp::Le) => return Some(adjusted_n1 - 1 <= n2),
                            (BinOp::Le, BinOp::Lt) => return Some(adjusted_n1 < n2),
                            _ => {}
                        }
                    }
                }
            }
        }

        _ => {}
    }

    // Cannot determine syntactically - need SMT solver
    None
}

/// Extract integer literal from expression
fn extract_int_literal(expr: &verum_ast::Expr) -> Option<i64> {
    use verum_ast::expr::{ExprKind, UnOp};
    use verum_ast::literal::{Literal, LiteralKind};

    match &expr.kind {
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(int_lit),
            ..
        }) => {
            // Convert i128 to i64 (safe for reasonable refinement bounds)
            i64::try_from(int_lit.value).ok()
        }
        // Handle negative literals: -(N)
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: operand,
        } => {
            if let ExprKind::Literal(Literal {
                kind: LiteralKind::Int(int_lit),
                ..
            }) = &operand.kind
            {
                // Negate and convert
                i64::try_from(int_lit.value)
                    .ok()
                    .and_then(|n| n.checked_neg())
            } else {
                None
            }
        }
        _ => None,
    }
}
