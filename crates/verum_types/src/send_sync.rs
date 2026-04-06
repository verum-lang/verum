//! Send and Sync Thread-Safety Marker Protocol Implementation
//!
//! Basic protocols with simple associated types (initial release) — 4 - Thread-Safety Protocols
//!
//! This module implements the Send and Sync marker protocols that enable
//! thread-safe programming in Verum. These protocols have no methods but
//! encode critical semantic contracts enforced at thread boundaries.
//!
//! # Key Concepts
//!
//! - **Send**: Type can be safely transferred between threads
//! - **Sync**: Type can be safely shared between threads via &T
//! - **Duality**: T: Sync ⟺ &T: Send
//! - **Composability**: Automatic derivation for compound types
//!
//! # Automatic Derivation Rules
//!
//! 1. **Primitives**: All primitives are Send + Sync
//! 2. **Tuples/Records**: Send/Sync if all fields are
//! 3. **References**: &T is Send if T: Sync
//! 4. **Functions**: !Send + !Sync (default)
//! 5. **Generic types**: Conditional on type parameters
//!
//! # Thread Boundaries
//!
//! - `spawn()` requires captured types to be Send
//! - `Shared<T>` requires T: Send + Sync
//! - `Mutex<T>` requires T: Send (provides Sync)
//! - Channels require T: Send

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Maybe};

use crate::protocol::{Protocol, ProtocolBound, ProtocolChecker, ProtocolImpl};
use crate::ty::Type;

// ==================== Send/Sync Derivation ====================

/// Automatic Send/Sync derivation engine
///
/// This module determines whether a type is Send and/or Sync based on
/// structural rules defined in the specification.
pub struct SendSyncDerivation<'a> {
    checker: &'a ProtocolChecker,
}

impl<'a> SendSyncDerivation<'a> {
    /// Create new Send/Sync derivation engine
    pub fn new(checker: &'a ProtocolChecker) -> Self {
        Self { checker }
    }

    /// Check if a type is Send
    ///
    /// Basic protocols with simple associated types (initial release) — 4.1 - Send Protocol
    ///
    /// A type is Send if ownership can be safely transferred between threads.
    pub fn is_send(&self, ty: &Type) -> bool {
        match ty {
            // 1. Primitives are Send
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text => true,

            // 2. Type variables - assume Send (checked at instantiation)
            Type::Var(_) => true,

            // 3. Named types - check for explicit implementation or auto-derive
            Type::Named { path, args } => {
                // Known !Send types (deny-list)
                let type_name = path.segments.last().map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => "",
                }).unwrap_or("");

                if matches!(type_name, "RawPtr" | "UnsafeCell" | "Cell" | "RefCell" | "Rc") {
                    return false;
                }

                let send_path = Path::single(Ident::new("Send", Span::default()));

                // Check if explicitly implements Send
                if self.checker.implements(ty, &send_path) {
                    return true;
                }

                // Auto-derive: user-defined types are Send if all type args are Send.
                // Non-generic user types (records, variants) are assumed Send
                // unless they appear in the deny-list above. This matches the
                // structural derivation principle: type X is { a: Int, b: Text }
                // is Send because Int and Text are Send.
                if args.is_empty() {
                    // Non-generic user-defined type: auto-derive Send
                    // (deny-listed types already returned false above)
                    true
                } else {
                    // Generic type: Send if all type arguments are Send
                    args.iter().all(|arg| self.is_send(arg))
                }
            }

            // 3b. Generic types (stdlib like List<T>, Maybe<T>) - check args
            Type::Generic { name: _, args } => {
                // Generic types are Send if all type arguments are Send
                args.iter().all(|arg| self.is_send(arg))
            }

            // 4. Function types are NOT Send (contain environment)
            Type::Function { .. } => false,

            // 5. Tuples are Send if all elements are Send
            Type::Tuple(elements) => elements.iter().all(|elem| self.is_send(elem)),

            // 6. Arrays are Send if element is Send
            Type::Array { element, .. } => self.is_send(element),

            // 6b. Slices are Send if element is Send
            Type::Slice { element } => self.is_send(element),

            // 7. Records are Send if all fields are Send
            Type::Record(fields) => fields.values().all(|field_ty| self.is_send(field_ty)),

            // 8. Variants are Send if all variants are Send
            Type::Variant(variants) => variants.values().all(|variant_ty| self.is_send(variant_ty)),

            // 9. References: &T is Send if T: Sync
            // Basic protocols with simple associated types (initial release) — 4.1 lines 11903-11909
            Type::Reference { inner, .. } => self.is_sync(inner),
            Type::CheckedReference { inner, .. } => self.is_sync(inner),

            // 10. Unsafe references: User responsibility (assume Send)
            // Basic protocols with simple associated types (initial release) — 4.1 line 11908
            Type::UnsafeReference { .. } => true,

            // 11. Ownership references are Send if inner is Send
            Type::Ownership { inner, .. } => self.is_send(inner),

            // 12. Raw pointers are NOT Send (unsafe, explicit opt-in required)
            Type::Pointer { .. } => false,

            // 12b. Volatile pointers (for MMIO) are NOT Send
            Type::VolatilePointer { .. } => false,

            // 13. Refined types inherit Send from base type
            Type::Refined { base, .. } => self.is_send(base),

            // 14. Existential types - check body
            Type::Exists { body, .. } => self.is_send(body),

            // 15. Universal quantification - check body
            Type::Forall { body, .. } => self.is_send(body),

            // 16. Meta parameters are compile-time only (Send)
            Type::Meta { .. } => true,

            // 17. Future<T> is Send if T: Send
            // Spec: async/await requires Send for spawn()
            Type::Future { output } => self.is_send(output),

            // 18. Generator<Y, R> is Send if Y: Send and R: Send
            Type::Generator {
                yield_ty,
                return_ty,
            } => self.is_send(yield_ty) && self.is_send(return_ty),

            // 19. Tensor<T, Shape> is Send if T: Send
            Type::Tensor { element, .. } => self.is_send(element),

            // 20. Lifetimes are not types, but for completeness
            Type::Lifetime { .. } => true,

            // 21. GenRef<T> is Send if T: Send
            // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .2 - GenRef wraps CBGR reference
            Type::GenRef { inner } => self.is_send(inner),

            // 22. TypeConstructor - type-level function, Send by default
            Type::TypeConstructor { .. } => true,

            // 23. TypeApp - applied type constructor, check arguments
            Type::TypeApp { constructor, args } => {
                self.is_send(constructor) && args.iter().all(|arg| self.is_send(arg))
            }

            // Dependent Types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )

            // 24. Pi types - Send if both param and return types are Send
            Type::Pi {
                param_type,
                return_type,
                ..
            } => self.is_send(param_type) && self.is_send(return_type),

            // 25. Sigma types - Send if both components are Send
            Type::Sigma {
                fst_type, snd_type, ..
            } => self.is_send(fst_type) && self.is_send(snd_type),

            // 26. Equality types - Send (proofs are generally Send)
            Type::Eq { ty, .. } => self.is_send(ty),

            // 27. Universe types are Send (type-level)
            Type::Universe { .. } => true,

            // 28. Prop is Send (proof-irrelevant)
            Type::Prop => true,

            // 29. Inductive types - Send if all params are Send
            Type::Inductive { params, .. } => params.iter().all(|(_, ty)| self.is_send(ty)),

            // 30. Coinductive types - Send if all params are Send
            Type::Coinductive { params, .. } => params.iter().all(|(_, ty)| self.is_send(ty)),

            // 31. Higher Inductive Types - Send if all params are Send
            Type::HigherInductive { params, .. } => params.iter().all(|(_, ty)| self.is_send(ty)),

            // 32. Quantified types - depends on quantity
            // Linear types (Quantity::One) are NOT Send (unique ownership)
            // Affine and erased types are Send if inner is Send
            Type::Quantified { inner, quantity } => match quantity {
                crate::ty::Quantity::One => false, // Linear - not Send
                _ => self.is_send(inner),
            },

            // 33. Placeholder types - conservative assumption during two-pass resolution
            //
            // Spec: Forward references during order-independent type resolution
            // (see infer.rs - two-pass resolution pattern)
            //
            // During the first pass of two-pass type resolution, types may be
            // represented as Placeholder types before their full definitions are
            // processed. At this stage, we conservatively assume not Send because:
            //
            // 1. We cannot know the actual type's Send status until resolved
            // 2. Returning false triggers a re-check after resolution completes
            // 3. This prevents incorrectly allowing unsafe cross-thread sharing
            //
            // After pass 2 completes, all Placeholder types should be resolved.
            // If a Placeholder reaches Send checking, it indicates either:
            // - Resolution is incomplete (will be caught by verify_no_placeholders)
            // - A forward reference cycle exists
            Type::Placeholder { .. } => false,

            // ExtensibleRecord is Send if all its fields are Send
            // Row polymorphism doesn't affect Send-ness
            Type::ExtensibleRecord { fields, .. } => fields.values().all(|ty| self.is_send(ty)),

            // CapabilityRestricted types - Send if base is Send
            // Capability restrictions don't affect thread safety
            Type::CapabilityRestricted { base, .. } => self.is_send(base),

            // Unknown type - conservatively NOT Send
            // We cannot know the underlying type, so we must be safe
            Type::Unknown => false,

            // DynProtocol (dyn Display + Debug) - Send if all bounds are Send-compatible
            // Dynamic dispatch through vtables is generally NOT Send safe
            // because we cannot statically verify the concrete type is Send
            Type::DynProtocol { .. } => false,
        }
    }

    /// Check if a type is Sync
    ///
    /// Basic protocols with simple associated types (initial release) — 4.2 - Sync Protocol
    ///
    /// A type is Sync if &T can be safely shared between threads.
    /// Equivalently: T: Sync ⟺ &T: Send
    pub fn is_sync(&self, ty: &Type) -> bool {
        match ty {
            // 1. Primitives are Sync
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text => true,

            // 2. Type variables - assume Sync (checked at instantiation)
            Type::Var(_) => true,

            // 3. Named types - check for explicit implementation or auto-derive
            Type::Named { path, args } => {
                // Known !Sync types (deny-list) — types with interior mutability
                let type_name = path.segments.last().map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => "",
                }).unwrap_or("");

                if matches!(type_name, "Cell" | "RefCell" | "UnsafeCell" | "Rc") {
                    return false;
                }

                let sync_path = Path::single(Ident::new("Sync", Span::default()));

                // Check if explicitly implements Sync
                if self.checker.implements(ty, &sync_path) {
                    return true;
                }

                // Auto-derive: user-defined types are Sync if all type args are Sync.
                // Non-generic user types assumed Sync unless deny-listed.
                if args.is_empty() {
                    true
                } else {
                    args.iter().all(|arg| self.is_sync(arg))
                }
            }

            // 3b. Generic types (stdlib like List<T>, Maybe<T>) - check args
            Type::Generic { name: _, args } => {
                args.iter().all(|arg| self.is_sync(arg))
            }

            // 4. Function types are NOT Sync (contain environment)
            Type::Function { .. } => false,

            // 5. Tuples are Sync if all elements are Sync
            Type::Tuple(elements) => elements.iter().all(|elem| self.is_sync(elem)),

            // 6. Arrays are Sync if element is Sync
            Type::Array { element, .. } => self.is_sync(element),

            // 6b. Slices are Sync if element is Sync
            Type::Slice { element } => self.is_sync(element),

            // 7. Records are Sync if all fields are Sync
            Type::Record(fields) => fields.values().all(|field_ty| self.is_sync(field_ty)),

            // 8. Variants are Sync if all variants are Sync
            Type::Variant(variants) => variants.values().all(|variant_ty| self.is_sync(variant_ty)),

            // 9. References: &T is Sync if T: Sync
            // Basic protocols with simple associated types (initial release) — 4.2 line 12028
            Type::Reference { inner, .. } => self.is_sync(inner),
            Type::CheckedReference { inner, .. } => self.is_sync(inner),

            // 10. Unsafe references: User responsibility (assume Sync)
            // Note: &unsafe T is actually assumed Sync but this is unsafe
            Type::UnsafeReference { .. } => true,

            // 11. Ownership references are Sync if inner is Sync
            Type::Ownership { inner, .. } => self.is_sync(inner),

            // 12. Raw pointers are NOT Sync
            Type::Pointer { .. } => false,

            // 12b. Volatile pointers (for MMIO) are NOT Sync
            Type::VolatilePointer { .. } => false,

            // 13. Refined types inherit Sync from base type
            Type::Refined { base, .. } => self.is_sync(base),

            // 14. Existential types - check body
            Type::Exists { body, .. } => self.is_sync(body),

            // 15. Universal quantification - check body
            Type::Forall { body, .. } => self.is_sync(body),

            // 16. Meta parameters are compile-time only (Sync)
            Type::Meta { .. } => true,

            // 17. Future<T> is NOT Sync (interior mutability)
            // Futures contain state that shouldn't be shared
            Type::Future { .. } => false,

            // 18. Generator<Y, R> is NOT Sync (interior mutability)
            Type::Generator { .. } => false,

            // 19. Tensor<T, Shape> is Sync if T: Sync
            Type::Tensor { element, .. } => self.is_sync(element),

            // 20. Lifetimes are not types, but for completeness
            Type::Lifetime { .. } => true,

            // 21. GenRef<T> is NOT Sync (contains mutable generation tracking)
            // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .2 - GenRef enables lending iterators
            Type::GenRef { .. } => false,

            // 22. TypeConstructor - type-level function, Sync by default
            Type::TypeConstructor { .. } => true,

            // 23. TypeApp - applied type constructor, check arguments
            Type::TypeApp { constructor, args } => {
                self.is_sync(constructor) && args.iter().all(|arg| self.is_sync(arg))
            }

            // Dependent Types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )

            // 24. Pi types - Sync if both param and return types are Sync
            Type::Pi {
                param_type,
                return_type,
                ..
            } => self.is_sync(param_type) && self.is_sync(return_type),

            // 25. Sigma types - Sync if both components are Sync
            Type::Sigma {
                fst_type, snd_type, ..
            } => self.is_sync(fst_type) && self.is_sync(snd_type),

            // 26. Equality types - Sync (proofs are generally Sync)
            Type::Eq { ty, .. } => self.is_sync(ty),

            // 27. Universe types are Sync (type-level)
            Type::Universe { .. } => true,

            // 28. Prop is Sync (proof-irrelevant)
            Type::Prop => true,

            // 29. Inductive types - Sync if all params are Sync
            Type::Inductive { params, .. } => params.iter().all(|(_, ty)| self.is_sync(ty)),

            // 30. Coinductive types - Sync if all params are Sync
            Type::Coinductive { params, .. } => params.iter().all(|(_, ty)| self.is_sync(ty)),

            // 31. Higher Inductive Types - Sync if all params are Sync
            Type::HigherInductive { params, .. } => params.iter().all(|(_, ty)| self.is_sync(ty)),

            // 32. Quantified types - Linear types are NOT Sync
            Type::Quantified { inner, quantity } => match quantity {
                crate::ty::Quantity::One => false, // Linear - not Sync
                _ => self.is_sync(inner),
            },

            // 33. Placeholder types - conservative assumption during two-pass resolution
            //
            // Spec: Forward references during order-independent type resolution
            // (see infer.rs - two-pass resolution pattern)
            //
            // During the first pass of two-pass type resolution, types may be
            // represented as Placeholder types before their full definitions are
            // processed. At this stage, we conservatively assume not Sync because:
            //
            // 1. We cannot know the actual type's Sync status until resolved
            // 2. Returning false triggers a re-check after resolution completes
            // 3. This prevents incorrectly allowing unsafe shared references
            //
            // After pass 2 completes, all Placeholder types should be resolved.
            // If a Placeholder reaches Sync checking, it indicates either:
            // - Resolution is incomplete (will be caught by verify_no_placeholders)
            // - A forward reference cycle exists
            Type::Placeholder { .. } => false,

            // ExtensibleRecord is Sync if all its fields are Sync
            // Row polymorphism doesn't affect Sync-ness
            Type::ExtensibleRecord { fields, .. } => fields.values().all(|ty| self.is_sync(ty)),

            // CapabilityRestricted types - Sync if base is Sync
            // Capability restrictions don't affect thread safety
            Type::CapabilityRestricted { base, .. } => self.is_sync(base),

            // Unknown type - conservatively NOT Sync
            // We cannot know the underlying type, so we must be safe
            Type::Unknown => false,

            // DynProtocol (dyn Display + Debug) - NOT Sync
            // Dynamic dispatch through vtables is generally NOT Sync safe
            // because we cannot statically verify the concrete type is Sync
            Type::DynProtocol { .. } => false,
        }
    }

    /// Derive Send implementation for a type
    ///
    /// Returns a ProtocolImpl if the type can be Send, or None if it cannot.
    pub fn derive_send(&self, ty: &Type) -> Maybe<ProtocolImpl> {
        if !self.is_send(ty) {
            return Maybe::None;
        }

        Maybe::Some(ProtocolImpl {
            protocol: Path::single(Ident::new("Send", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: self.generate_send_where_clauses(ty),
            methods: Map::new(), // Marker protocol - no methods
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Derive Sync implementation for a type
    ///
    /// Returns a ProtocolImpl if the type can be Sync, or None if it cannot.
    pub fn derive_sync(&self, ty: &Type) -> Maybe<ProtocolImpl> {
        if !self.is_sync(ty) {
            return Maybe::None;
        }

        Maybe::Some(ProtocolImpl {
            protocol: Path::single(Ident::new("Sync", Span::default())),
            protocol_args: List::new(),
            for_type: ty.clone(),
            where_clauses: self.generate_sync_where_clauses(ty),
            methods: Map::new(), // Marker protocol - no methods
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        })
    }

    /// Generate where clauses for Send implementation
    ///
    /// For generic types like List<T>, generate: where T: Send
    fn generate_send_where_clauses(&self, ty: &Type) -> List<crate::protocol::WhereClause> {
        let mut clauses = List::new();

        if let Type::Named { args, .. } = ty {
            for arg in args {
                if let Type::Var(_tv) = arg {
                    let mut bounds = List::new();
                    bounds.push(ProtocolBound {
                        protocol: Path::single(Ident::new("Send", Span::default())),
                        args: List::new(),
                        is_negative: false,
                    });
                    clauses.push(crate::protocol::WhereClause {
                        ty: arg.clone(),
                        bounds,
                    });
                }
            }
        }

        clauses
    }

    /// Generate where clauses for Sync implementation
    ///
    /// For generic types like List<T>, generate: where T: Sync
    fn generate_sync_where_clauses(&self, ty: &Type) -> List<crate::protocol::WhereClause> {
        let mut clauses = List::new();

        if let Type::Named { args, .. } = ty {
            for arg in args {
                if let Type::Var(_tv) = arg {
                    let mut bounds = List::new();
                    bounds.push(ProtocolBound {
                        protocol: Path::single(Ident::new("Sync", Span::default())),
                        args: List::new(),
                        is_negative: false,
                    });
                    clauses.push(crate::protocol::WhereClause {
                        ty: arg.clone(),
                        bounds,
                    });
                }
            }
        }

        clauses
    }
}

// ==================== Protocol Registration ====================

/// Register Send and Sync marker protocols
///
/// This should be called during protocol checker initialization to register
/// the standard Send and Sync protocols.
pub fn register_send_sync_protocols(checker: &mut ProtocolChecker) {
    // Register Send protocol
    let send = Protocol {
        name: "Send".into(),
        kind: crate::protocol::ProtocolKind::Constraint,
        type_params: List::new(),
        methods: Map::new(), // Marker protocol - no methods
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
    };
    checker.register_protocol(send);

    // Register Sync protocol
    let sync = Protocol {
        name: "Sync".into(),
        kind: crate::protocol::ProtocolKind::Constraint,
        type_params: List::new(),
        methods: Map::new(), // Marker protocol - no methods
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
    };
    checker.register_protocol(sync);
}

/// Register Send/Sync implementations for standard types
///
/// This registers implementations for:
/// - Primitive types (Int, Bool, Float, etc.)
/// - Standard library types (List, Map, Set, etc.)
/// - Synchronization primitives (Shared, Mutex)
pub fn register_standard_send_sync_impls(checker: &mut ProtocolChecker) {
    use verum_ast::{Ident, Path};

    // Helper to create implementation
    let make_impl = |for_type: Type, protocol: &str| -> ProtocolImpl {
        ProtocolImpl {
            for_type,
            protocol: Path::single(Ident::new(protocol, Span::default())),
            protocol_args: List::new(),
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: Map::new(),
            associated_consts: Map::new(),
            specialization: Maybe::None,
            impl_crate: Maybe::Some("stdlib".into()),
            span: Span::default(),
            type_param_fn_bounds: Map::new(),
        }
    };

    // 1. Register Send for primitive types
    // Basic protocols with simple associated types (initial release) — 4.1 lines 11886-11892
    for ty in &[
        Type::Unit,
        Type::Int,
        Type::Float,
        Type::Bool,
        Type::Char,
        Type::Text,
    ] {
        let _ = checker.register_impl(make_impl(ty.clone(), "Send"));
    }

    // 2. Register Sync for primitive types
    // Basic protocols with simple associated types (initial release) — 4.2 lines 12013-12019
    for ty in &[
        Type::Unit,
        Type::Int,
        Type::Float,
        Type::Bool,
        Type::Char,
        Type::Text,
    ] {
        let _ = checker.register_impl(make_impl(ty.clone(), "Sync"));
    }

    // 3. Register for standard library types (as Named types)
    let std_types = vec![WKT::List.as_str(), WKT::Map.as_str(), WKT::Set.as_str(), WKT::Maybe.as_str(), WKT::Result.as_str(), WKT::Heap.as_str()];

    for type_name in std_types {
        let ty = Type::Named {
            path: Path::single(Ident::new(type_name, Span::default())),
            args: vec![].into(),
        };

        // These are generic types - actual Send/Sync depends on type parameters
        // Register base implementations (will be checked with bounds)
        let _ = checker.register_impl(make_impl(ty.clone(), "Send"));
        let _ = checker.register_impl(make_impl(ty, "Sync"));
    }

    // 4. Register Shared<T>: Send + Sync where T: Send + Sync
    // Basic protocols with simple associated types (initial release) — 4.4 lines 12193-12211
    let shared_ty = Type::Named {
        path: Path::single(Ident::new(WKT::Shared.as_str(), Span::default())),
        args: vec![Type::Var(crate::ty::TypeVar::with_id(0))].into(),
    };

    let mut shared_send_where = List::new();
    let t_var = Type::Var(crate::ty::TypeVar::with_id(0));
    let mut bounds = List::new();
    bounds.push(ProtocolBound {
        protocol: Path::single(Ident::new("Send", Span::default())),
        args: List::new(),
        is_negative: false,
    });
    bounds.push(ProtocolBound {
        protocol: Path::single(Ident::new("Sync", Span::default())),
        args: List::new(),
        is_negative: false,
    });
    shared_send_where.push(crate::protocol::WhereClause {
        ty: t_var.clone(),
        bounds: bounds.clone(),
    });

    let _ = checker.register_impl(ProtocolImpl {
        for_type: shared_ty.clone(),
        protocol: Path::single(Ident::new("Send", Span::default())),
        protocol_args: List::new(),
        where_clauses: shared_send_where.clone(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    });

    let _ = checker.register_impl(ProtocolImpl {
        for_type: shared_ty,
        protocol: Path::single(Ident::new("Sync", Span::default())),
        protocol_args: List::new(),
        where_clauses: shared_send_where,
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    });

    // 5. Register Mutex<T>: Send + Sync where T: Send
    // Basic protocols with simple associated types (initial release) — 4.5 lines 12428-12446
    let mutex_ty = Type::Named {
        path: Path::single(Ident::new("Mutex", Span::default())),
        args: vec![Type::Var(crate::ty::TypeVar::with_id(0))].into(),
    };

    let mut mutex_where = List::new();
    let mut mutex_bounds = List::new();
    mutex_bounds.push(ProtocolBound {
        protocol: Path::single(Ident::new("Send", Span::default())),
        args: List::new(),
        is_negative: false,
    });
    mutex_where.push(crate::protocol::WhereClause {
        ty: t_var.clone(),
        bounds: mutex_bounds,
    });

    let _ = checker.register_impl(ProtocolImpl {
        for_type: mutex_ty.clone(),
        protocol: Path::single(Ident::new("Send", Span::default())),
        protocol_args: List::new(),
        where_clauses: mutex_where.clone(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    });

    let _ = checker.register_impl(ProtocolImpl {
        for_type: mutex_ty,
        protocol: Path::single(Ident::new("Sync", Span::default())),
        protocol_args: List::new(),
        where_clauses: mutex_where,
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    });
}
