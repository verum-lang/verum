//! Protocol-Based Method Resolution
//!

//! This module provides abstractions for resolving methods through protocol implementations,
//! enabling a stdlib-agnostic type system architecture.
//!

//! ## Architecture
//!

//! The method resolution system is designed around two key principles:
//!

//! 1. **No Hardcoded Type Names**: Method resolution never checks for specific type names
//!  like "List", "Text", or "Maybe". All methods are resolved through protocols.
//!

//! 2. **Pluggable Resolution**: The `MethodResolver` trait allows different implementations
//!  for different compilation modes (stdlib bootstrap vs normal compilation).
//!

//! ## Resolution Order
//!

//! When resolving `receiver.method(args)`:
//!

//! 1. **Inherent Methods**: Check `implement Type { ... }` blocks for the receiver type
//! 2. **Protocol Methods**: Check all protocols the receiver type implements
//! 3. **Auto-Deref**: If receiver implements Deref, try resolving on `*receiver`
//! 4. **Auto-Ref**: Try resolving with `&receiver` or `&mut receiver`
//!

//! ## Example
//!

//! ```ignore
//! // Resolving: some_list.len()
//!

//! // 1. Check inherent methods on List<T>
//! // Found: implement List<T> { fn len(&self) -> Int { ... } }
//! // -> Returns MethodResolution
//!

//! // Or if not found in inherent:
//! // 2. Check protocol implementations
//! // implement Len for List<T> { ... }
//! // -> Returns MethodResolution with protocol source
//! ```

use verum_ast::Span;
use verum_ast::decl::ImplDecl;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Maybe, Text};

use crate::context::TypeScheme;
use crate::ty::Type;

/// Method resolution result
#[derive(Debug, Clone)]
pub struct MethodResolution {
    /// The method's type signature (as a function type)
    pub signature: TypeScheme,

    /// Source of the method
    pub source: MethodSource,

    /// The receiver type after any auto-ref/deref
    pub adjusted_receiver: Type,

    /// Whether the receiver was auto-referenced
    pub auto_referenced: bool,

    /// Whether the receiver was auto-dereferenced
    pub auto_dereferenced: bool,

    /// Number of auto-deref steps taken
    pub deref_depth: u32,

    /// Whether the method requires mutable receiver
    pub requires_mut_receiver: bool,

    /// Type substitutions from generic instantiation
    pub type_substitutions: Map<Text, Type>,
}

/// Where a method was found.
///
/// Distinct from `verum_protocol_types::MethodSource` (which
/// classifies a method's *origin within a protocol*: Explicit /
/// Default / Inherited).  This enum classifies a method's
/// *resolution path* during method-call type checking: which
/// kind of impl block matched, plus auto-deref chaining.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodSource {
    /// From `implement Type { fn method(...) }` block
    Inherent {
        /// The type that has the inherent implementation
        type_name: Text,
    },

    /// From `implement Protocol for Type { fn method(...) }` block
    Protocol {
        /// The protocol providing the method
        protocol_name: Text,
        /// The type implementing the protocol
        type_name: Text,
    },

    /// From auto-deref through a smart pointer type
    AutoDeref {
        /// The smart pointer type that was dereferenced
        through_type: Text,
        /// The original method source after deref
        inner_source: Box<MethodSource>,
    },

    /// A compiler builtin method
    Builtin {
        /// Name of the builtin (e.g., "primitive_add")
        name: Text,
    },
}

/// Discriminator for [`MethodSource`] (the
/// method-resolution-path 4-variant — distinct from
/// `verum_protocol_types::MethodSourceKind` which classifies the
/// origin-within-a-protocol 3-variant taxonomy).  Zero-sized
/// projection used by metrics / IDE consumers that need the
/// resolution-path band without cloning the per-variant Text
/// payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MethodSourceKind {
    Inherent,
    Protocol,
    AutoDeref,
    Builtin,
}

/// Per-variant projection for [`MethodSourceKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodSourceKindMeta {
    /// Lower-snake-case wire form for telemetry.
    pub name: &'static str,
    /// Whether the method body lives in a *user-written impl
    /// block* (Inherent + Protocol).  False for AutoDeref
    /// (delegated to a wrapped MethodSource) and Builtin
    /// (compiler-internal).
    pub is_user_impl: bool,
    /// Whether the resolution went through *protocol dispatch*
    /// — Protocol singleton.  Distinct from Inherent which is
    /// non-protocol direct dispatch.
    pub is_protocol_dispatch: bool,
    /// Whether the resolution chained through *auto-deref* —
    /// AutoDeref singleton.  Pinned because callers that need
    /// to inspect the chained inner source must branch on this.
    pub is_chained: bool,
    /// Whether the method is *compiler-internal* — Builtin
    /// singleton.  Method bodies for these live in
    /// `verum_vbc::intrinsics`, not in `.vr` source.
    pub is_builtin: bool,
}

impl MethodSourceKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Inherent,
        Self::Protocol,
        Self::AutoDeref,
        Self::Builtin,
    ];

    /// Static fact-pack.
    pub const fn meta(self) -> MethodSourceKindMeta {
        match self {
            MethodSourceKind::Inherent => MethodSourceKindMeta {
                name: "inherent",
                is_user_impl: true,
                is_protocol_dispatch: false,
                is_chained: false,
                is_builtin: false,
            },
            MethodSourceKind::Protocol => MethodSourceKindMeta {
                name: "protocol",
                is_user_impl: true,
                is_protocol_dispatch: true,
                is_chained: false,
                is_builtin: false,
            },
            MethodSourceKind::AutoDeref => MethodSourceKindMeta {
                name: "auto_deref",
                is_user_impl: false,
                is_protocol_dispatch: false,
                is_chained: true,
                is_builtin: false,
            },
            MethodSourceKind::Builtin => MethodSourceKindMeta {
                name: "builtin",
                is_user_impl: false,
                is_protocol_dispatch: false,
                is_chained: false,
                is_builtin: true,
            },
        }
    }
}

impl MethodSource {
    /// Discriminator projection — strip the payload, keep tag.
    pub const fn kind(&self) -> MethodSourceKind {
        match self {
            MethodSource::Inherent { .. } => MethodSourceKind::Inherent,
            MethodSource::Protocol { .. } => MethodSourceKind::Protocol,
            MethodSource::AutoDeref { .. } => MethodSourceKind::AutoDeref,
            MethodSource::Builtin { .. } => MethodSourceKind::Builtin,
        }
    }

    /// Walk through any AutoDeref chain to the underlying
    /// non-deref source.  Pinned via the
    /// `meta_pin_method_source_resolved_kind` test that
    /// `resolved_kind() != AutoDeref` for every input — the
    /// chain always bottoms out at a non-deref leaf.
    pub fn resolved_kind(&self) -> MethodSourceKind {
        let mut cur = self;
        while let MethodSource::AutoDeref { inner_source, .. } = cur {
            cur = inner_source;
        }
        cur.kind()
    }
}

/// Method resolution error
#[derive(Debug, Clone)]
pub enum MethodError {
    /// No method with this name found on the type
    MethodNotFound {
        receiver_type: Type,
        method_name: Text,
        span: Maybe<Span>,
        /// Similar methods that might be what the user meant
        suggestions: List<Text>,
    },

    /// Method found but receiver mutability doesn't match
    MutabilityMismatch {
        method_name: Text,
        expected_mut: bool,
        actual_mut: bool,
        span: Maybe<Span>,
    },

    /// Ambiguous method - multiple protocols provide the same method
    AmbiguousMethod {
        receiver_type: Type,
        method_name: Text,
        candidates: List<MethodSource>,
        span: Maybe<Span>,
    },

    /// Type parameters couldn't be inferred
    TypeInferenceFailed {
        method_name: Text,
        type_params: List<Text>,
        span: Maybe<Span>,
    },

    /// Protocol bound not satisfied
    ProtocolBoundNotSatisfied {
        type_param: Text,
        required_protocol: Text,
        actual_type: Type,
        span: Maybe<Span>,
    },
}

impl std::fmt::Display for MethodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MethodError::MethodNotFound {
                receiver_type,
                method_name,
                suggestions,
                ..
            } => {
                write!(
                    f,
                    "no method named '{}' found for type '{}'",
                    method_name, receiver_type
                )?;
                if !suggestions.is_empty() {
                    write!(f, ". Did you mean: ")?;
                    for (i, s) in suggestions.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "'{}'", s)?;
                    }
                }
                Ok(())
            }
            MethodError::MutabilityMismatch {
                method_name,
                expected_mut,
                ..
            } => {
                if *expected_mut {
                    write!(f, "method '{}' requires a mutable receiver", method_name)
                } else {
                    write!(
                        f,
                        "method '{}' doesn't require a mutable receiver",
                        method_name
                    )
                }
            }
            MethodError::AmbiguousMethod {
                method_name,
                candidates,
                ..
            } => {
                write!(f, "ambiguous method '{}', could be from: ", method_name)?;
                for (i, c) in candidates.iter().enumerate() {
                    if i > 0 {
                        write!(f, " or ")?;
                    }
                    match c {
                        MethodSource::Inherent { type_name } => write!(f, "{}", type_name)?,
                        MethodSource::Protocol { protocol_name, .. } => {
                            write!(f, "{}", protocol_name)?
                        }
                        MethodSource::AutoDeref { through_type, .. } => {
                            write!(f, "*{}", through_type)?
                        }
                        MethodSource::Builtin { name } => write!(f, "builtin:{}", name)?,
                    }
                }
                Ok(())
            }
            MethodError::TypeInferenceFailed {
                method_name,
                type_params,
                ..
            } => {
                write!(
                    f,
                    "couldn't infer type parameters for '{}': {:?}",
                    method_name, type_params
                )
            }
            MethodError::ProtocolBoundNotSatisfied {
                type_param,
                required_protocol,
                actual_type,
                ..
            } => {
                write!(
                    f,
                    "type '{}' doesn't implement protocol '{}' required by type parameter '{}'",
                    actual_type, required_protocol, type_param
                )
            }
        }
    }
}

impl std::error::Error for MethodError {}

/// Discriminator for [`MethodError`] — zero-sized projection
/// classifying the *failure-mode band* of a method-resolution
/// error.  Used by metrics consumers and IDE quick-fix
/// generators that branch on the failure kind without cloning
/// the per-variant Type / Text payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MethodErrorKind {
    MethodNotFound,
    MutabilityMismatch,
    AmbiguousMethod,
    TypeInferenceFailed,
    ProtocolBoundNotSatisfied,
}

/// Per-variant projection for [`MethodErrorKind`].  Classifier
/// flags partition the five failure modes by:
///   * *resolution stage* — name lookup, signature check,
///     overload resolution, generics inference, bound check;
///   * *IDE-actionable* — whether the error has a structured
///     suggestion the IDE can render as a quick-fix;
///   * *carries-suggestion* — whether the variant exposes a
///     suggestion list payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodErrorKindMeta {
    /// Lower-snake-case wire form.
    pub name: &'static str,
    /// Resolution failed at the *name-lookup* stage — the
    /// method name doesn't exist on the receiver type.
    /// MethodNotFound singleton.
    pub is_name_lookup_failure: bool,
    /// Resolution found a method but its *signature* doesn't
    /// match the call site (mutability or generic-bound
    /// mismatch).  MutabilityMismatch + ProtocolBoundNotSatisfied.
    pub is_signature_mismatch: bool,
    /// Resolution found *multiple* methods and couldn't pick
    /// one.  AmbiguousMethod singleton.  Pinned because the
    /// IDE uses this to render a "pick which" picker.
    pub is_ambiguity: bool,
    /// Resolution failed at the *generic-inference* stage —
    /// the type parameters couldn't be inferred from the call
    /// shape.  TypeInferenceFailed singleton.
    pub is_inference_failure: bool,
    /// Whether the variant carries a *suggestions* payload
    /// (typo-correction list).  MethodNotFound singleton —
    /// the only failure mode that surfaces "did you mean X" UI.
    pub carries_suggestions: bool,
}

impl MethodErrorKind {
    /// All variants in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::MethodNotFound,
        Self::MutabilityMismatch,
        Self::AmbiguousMethod,
        Self::TypeInferenceFailed,
        Self::ProtocolBoundNotSatisfied,
    ];

    /// Static fact-pack.
    pub const fn meta(self) -> MethodErrorKindMeta {
        match self {
            MethodErrorKind::MethodNotFound => MethodErrorKindMeta {
                name: "method_not_found",
                is_name_lookup_failure: true,
                is_signature_mismatch: false,
                is_ambiguity: false,
                is_inference_failure: false,
                carries_suggestions: true,
            },
            MethodErrorKind::MutabilityMismatch => MethodErrorKindMeta {
                name: "mutability_mismatch",
                is_name_lookup_failure: false,
                is_signature_mismatch: true,
                is_ambiguity: false,
                is_inference_failure: false,
                carries_suggestions: false,
            },
            MethodErrorKind::AmbiguousMethod => MethodErrorKindMeta {
                name: "ambiguous_method",
                is_name_lookup_failure: false,
                is_signature_mismatch: false,
                is_ambiguity: true,
                is_inference_failure: false,
                carries_suggestions: false,
            },
            MethodErrorKind::TypeInferenceFailed => MethodErrorKindMeta {
                name: "type_inference_failed",
                is_name_lookup_failure: false,
                is_signature_mismatch: false,
                is_ambiguity: false,
                is_inference_failure: true,
                carries_suggestions: false,
            },
            MethodErrorKind::ProtocolBoundNotSatisfied => MethodErrorKindMeta {
                name: "protocol_bound_not_satisfied",
                is_name_lookup_failure: false,
                is_signature_mismatch: true,
                is_ambiguity: false,
                is_inference_failure: false,
                carries_suggestions: false,
            },
        }
    }
}

impl MethodError {
    /// Discriminator projection — strip the payload, keep tag.
    pub const fn kind(&self) -> MethodErrorKind {
        match self {
            MethodError::MethodNotFound { .. } => MethodErrorKind::MethodNotFound,
            MethodError::MutabilityMismatch { .. } => {
                MethodErrorKind::MutabilityMismatch
            }
            MethodError::AmbiguousMethod { .. } => MethodErrorKind::AmbiguousMethod,
            MethodError::TypeInferenceFailed { .. } => {
                MethodErrorKind::TypeInferenceFailed
            }
            MethodError::ProtocolBoundNotSatisfied { .. } => {
                MethodErrorKind::ProtocolBoundNotSatisfied
            }
        }
    }

    /// Returns the typo-correction suggestions for the
    /// `MethodNotFound` band.  Decoupled from per-variant
    /// matching via `meta().carries_suggestions`.
    pub fn suggestions(&self) -> Option<&List<Text>> {
        match self {
            MethodError::MethodNotFound { suggestions, .. } => Some(suggestions),
            _ => None,
        }
    }

    /// Returns the source span if the variant carries one.
    /// Every variant carries a `span: Maybe<Span>` field —
    /// pinned so consumers can read it without per-variant
    /// matching.
    pub fn span(&self) -> Maybe<Span> {
        match self {
            MethodError::MethodNotFound { span, .. } => *span,
            MethodError::MutabilityMismatch { span, .. } => *span,
            MethodError::AmbiguousMethod { span, .. } => *span,
            MethodError::TypeInferenceFailed { span, .. } => *span,
            MethodError::ProtocolBoundNotSatisfied { span, .. } => *span,
        }
    }
}

/// Trait for method resolution
///

/// This trait abstracts away method resolution, allowing different implementations
/// for different compilation contexts (stdlib bootstrap, normal build, etc.)
pub trait MethodResolver {
    /// Resolve a method call
    ///

    /// # Arguments
    ///

    /// * `receiver_type` - The type of the receiver expression
    /// * `method_name` - The name of the method being called
    /// * `type_hints` - Optional type hints from the call site
    /// * `is_mut_receiver` - Whether the receiver is a mutable reference
    ///

    /// # Returns
    ///

    /// `Ok(resolution)` if the method was found, `Err(error)` otherwise
    fn resolve_method(
        &self,
        receiver_type: &Type,
        method_name: &str,
        type_hints: &[Type],
        is_mut_receiver: bool,
    ) -> Result<MethodResolution, MethodError>;

    /// Check if a type implements a protocol
    fn implements_protocol(&self, ty: &Type, protocol_name: &str) -> bool;

    /// Get all methods available on a type
    fn available_methods(&self, ty: &Type) -> List<MethodInfo>;

    /// Find similar method names for error suggestions
    fn find_similar_methods(&self, ty: &Type, method_name: &str) -> List<Text>;

    /// Register a new inherent implementation
    fn register_inherent_impl(&mut self, type_name: &str, impl_block: &ImplDecl);

    /// Register a new protocol implementation
    fn register_protocol_impl(
        &mut self,
        protocol_name: &str,
        target_type: &str,
        impl_block: &ImplDecl,
    );
}

/// Information about a method
#[derive(Debug, Clone)]
pub struct MethodInfo {
    /// Method name
    pub name: Text,
    /// Method signature
    pub signature: TypeScheme,
    /// Where the method comes from
    pub source: MethodSource,
    /// Brief documentation
    pub doc: Maybe<Text>,
    /// Whether it takes &self, &mut self, self, or is static
    pub receiver_kind: ReceiverKind,
}

/// Kind of receiver for a method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    /// No receiver (static method)
    None,
    /// Takes self by value
    ByValue,
    /// Takes &self
    ByRef,
    /// Takes &mut self
    ByMutRef,
}

/// A method's type signature together with its receiver information.
#[derive(Debug, Clone)]
struct StoredMethod {
    /// The method's type signature (as a function type)
    signature: TypeScheme,
    /// What kind of receiver the method takes (none, by-value, &self, &mut self)
    receiver_kind: ReceiverKind,
    /// Whether the method requires a mutable receiver
    requires_mut: bool,
}

/// Default method resolver using the existing ProtocolChecker infrastructure
///

/// This implementation wraps the current hardcoded approach but can be replaced
/// with a stdlib-agnostic implementation during stdlib bootstrap.
#[derive(Debug, Default)]
pub struct DefaultMethodResolver {
    /// Inherent method implementations: type_name -> method_name -> stored method
    inherent_impls: Map<Text, Map<Text, StoredMethod>>,

    /// Protocol implementations: (protocol_name, type_name) -> methods
    protocol_impls: Map<(Text, Text), Map<Text, StoredMethod>>,

    /// Protocol definitions: protocol_name -> list of method names
    protocols: Map<Text, List<Text>>,

    /// Cache of resolved methods for performance
    resolution_cache: Map<(Text, Text), MethodResolution>,
}

impl DefaultMethodResolver {
    /// Create a new empty resolver
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a resolver with builtin methods for primitives
    pub fn with_primitives() -> Self {
        let mut resolver = Self::new();

        // Register builtin methods for primitive types
        // These are the only "hardcoded" methods - they're truly primitive
        resolver.register_primitive_methods();

        resolver
    }

    /// Register methods for true primitive types (Int, Float, Bool, Char)
    fn register_primitive_methods(&mut self) {
        // For Int, Float, Bool - these are the core arithmetic/comparison methods
        // that don't come from protocols but are built into the VM

        // Note: In the final architecture, even these might come from
        // core/base/primitives.vr for consistency
    }

    /// Clear the resolution cache
    pub fn clear_cache(&mut self) {
        self.resolution_cache.clear();
    }

    /// Get statistics about registered methods
    pub fn stats(&self) -> ResolverStats {
        let inherent_methods = self
            .inherent_impls
            .iter()
            .map(|(_, methods)| methods.len())
            .sum();

        let protocol_methods = self
            .protocol_impls
            .iter()
            .map(|(_, methods)| methods.len())
            .sum();

        ResolverStats {
            inherent_impl_count: self.inherent_impls.len(),
            protocol_impl_count: self.protocol_impls.len(),
            protocol_count: self.protocols.len(),
            inherent_method_count: inherent_methods,
            protocol_method_count: protocol_methods,
            cache_entries: self.resolution_cache.len(),
        }
    }
}

/// Statistics about the method resolver
#[derive(Debug, Clone)]
pub struct ResolverStats {
    /// Number of types with inherent implementations
    pub inherent_impl_count: usize,
    /// Number of protocol implementations
    pub protocol_impl_count: usize,
    /// Number of registered protocols
    pub protocol_count: usize,
    /// Total number of inherent methods
    pub inherent_method_count: usize,
    /// Total number of protocol methods
    pub protocol_method_count: usize,
    /// Number of cached resolution results
    pub cache_entries: usize,
}

impl MethodResolver for DefaultMethodResolver {
    fn resolve_method(
        &self,
        receiver_type: &Type,
        method_name: &str,
        type_hints: &[Type],
        is_mut_receiver: bool,
    ) -> Result<MethodResolution, MethodError> {
        let type_name = type_name_from_type(receiver_type);

        // 1. Check inherent implementations first
        if let Maybe::Some(methods) = self.inherent_impls.get(&type_name) {
            if let Maybe::Some(stored) = methods.get(&Text::from(method_name)) {
                return Ok(MethodResolution {
                    signature: stored.signature.clone(),
                    source: MethodSource::Inherent {
                        type_name: type_name.clone(),
                    },
                    adjusted_receiver: receiver_type.clone(),
                    auto_referenced: false,
                    auto_dereferenced: false,
                    deref_depth: 0,
                    requires_mut_receiver: stored.requires_mut,
                    type_substitutions: Map::new(),
                });
            }
        }

        // 2. Check protocol implementations
        for ((protocol, impl_type), methods) in self.protocol_impls.iter() {
            if impl_type == &type_name {
                if let Maybe::Some(stored) = methods.get(&Text::from(method_name)) {
                    return Ok(MethodResolution {
                        signature: stored.signature.clone(),
                        source: MethodSource::Protocol {
                            protocol_name: protocol.clone(),
                            type_name: type_name.clone(),
                        },
                        adjusted_receiver: receiver_type.clone(),
                        auto_referenced: false,
                        auto_dereferenced: false,
                        deref_depth: 0,
                        requires_mut_receiver: stored.requires_mut,
                        type_substitutions: Map::new(),
                    });
                }
            }
        }

        // 3. Method not found - provide suggestions
        let suggestions = self.find_similar_methods(receiver_type, method_name);
        Err(MethodError::MethodNotFound {
            receiver_type: receiver_type.clone(),
            method_name: Text::from(method_name),
            span: Maybe::None,
            suggestions,
        })
    }

    fn implements_protocol(&self, ty: &Type, protocol_name: &str) -> bool {
        let type_name = type_name_from_type(ty);
        let key = (Text::from(protocol_name), type_name);
        self.protocol_impls.contains_key(&key)
    }

    fn available_methods(&self, ty: &Type) -> List<MethodInfo> {
        let type_name = type_name_from_type(ty);
        let mut methods = List::new();

        // Collect inherent methods
        if let Maybe::Some(inherent) = self.inherent_impls.get(&type_name) {
            for (name, stored) in inherent.iter() {
                methods.push(MethodInfo {
                    name: name.clone(),
                    signature: stored.signature.clone(),
                    source: MethodSource::Inherent {
                        type_name: type_name.clone(),
                    },
                    doc: Maybe::None,
                    receiver_kind: stored.receiver_kind,
                });
            }
        }

        // Collect protocol methods
        for ((protocol, impl_type), proto_methods) in self.protocol_impls.iter() {
            if impl_type == &type_name {
                for (name, stored) in proto_methods.iter() {
                    methods.push(MethodInfo {
                        name: name.clone(),
                        signature: stored.signature.clone(),
                        source: MethodSource::Protocol {
                            protocol_name: protocol.clone(),
                            type_name: type_name.clone(),
                        },
                        doc: Maybe::None,
                        receiver_kind: stored.receiver_kind,
                    });
                }
            }
        }

        methods
    }

    fn find_similar_methods(&self, ty: &Type, method_name: &str) -> List<Text> {
        let type_name = type_name_from_type(ty);
        let mut suggestions = List::new();

        // Simple edit distance check for similar method names
        let available = self.available_methods(ty);
        for method in available.iter() {
            if is_similar(&method.name, method_name) {
                suggestions.push(method.name.clone());
            }
        }

        suggestions
    }

    fn register_inherent_impl(&mut self, type_name: &str, impl_block: &ImplDecl) {
        let type_text = Text::from(type_name);
        let methods = self.inherent_impls.entry(type_text).or_default();

        for item in &impl_block.items {
            if let verum_ast::decl::ImplItemKind::Function(func) = &item.kind {
                let method_name = Text::from(func.name.name.as_str());
                let method_type = function_decl_to_type(func);
                let (receiver_kind, requires_mut) = receiver_info_from_decl(func);
                methods.insert(
                    method_name,
                    StoredMethod {
                        signature: TypeScheme::mono(method_type),
                        receiver_kind,
                        requires_mut,
                    },
                );
            }
        }
    }

    fn register_protocol_impl(
        &mut self,
        protocol_name: &str,
        target_type: &str,
        impl_block: &ImplDecl,
    ) {
        let key = (Text::from(protocol_name), Text::from(target_type));
        let methods = self.protocol_impls.entry(key).or_default();

        for item in &impl_block.items {
            if let verum_ast::decl::ImplItemKind::Function(func) = &item.kind {
                let method_name = Text::from(func.name.name.as_str());
                let method_type = function_decl_to_type(func);
                let (receiver_kind, requires_mut) = receiver_info_from_decl(func);
                methods.insert(
                    method_name,
                    StoredMethod {
                        signature: TypeScheme::mono(method_type),
                        receiver_kind,
                        requires_mut,
                    },
                );
            }
        }
    }
}

/// Determine the receiver kind and mutability from a function declaration's parameters.
///

/// Inspects the first parameter to see if it's a self parameter and what kind.
/// Returns `(ReceiverKind, requires_mut_receiver)`.
fn receiver_info_from_decl(func: &verum_ast::decl::FunctionDecl) -> (ReceiverKind, bool) {
    use verum_ast::decl::FunctionParamKind;

    match func.params.first() {
        Some(param) => match &param.kind {
            // By-value self (not mutable)
            FunctionParamKind::SelfValue => (ReceiverKind::ByValue, false),
            // By-value self (mutable binding)
            FunctionParamKind::SelfValueMut => (ReceiverKind::ByValue, true),
            // Immutable reference receivers: &self, &checked self, &unsafe self
            FunctionParamKind::SelfRef
            | FunctionParamKind::SelfRefChecked
            | FunctionParamKind::SelfRefUnsafe => (ReceiverKind::ByRef, false),
            // Mutable reference receivers: &mut self, &checked mut self, &unsafe mut self
            FunctionParamKind::SelfRefMut
            | FunctionParamKind::SelfRefCheckedMut
            | FunctionParamKind::SelfRefUnsafeMut => (ReceiverKind::ByMutRef, true),
            // Ownership receivers: %self, %mut self
            FunctionParamKind::SelfOwn => (ReceiverKind::ByValue, false),
            FunctionParamKind::SelfOwnMut => (ReceiverKind::ByValue, true),
            // Regular parameter (no self) — static method
            FunctionParamKind::Regular { .. } => (ReceiverKind::None, false),
        },
        // No parameters — static method
        None => (ReceiverKind::None, false),
    }
}

/// Convert a function declaration to a Type::Function
fn function_decl_to_type(func: &verum_ast::decl::FunctionDecl) -> Type {
    let params: List<Type> = func
        .params
        .iter()
        .filter_map(|p| {
            if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                Some(convert_simple_ast_type(ty))
            } else {
                None // Skip self params
            }
        })
        .collect();

    let return_type = match &func.return_type {
        verum_common::Maybe::Some(ty) => convert_simple_ast_type(ty),
        verum_common::Maybe::None => Type::Unit,
    };

    Type::Function {
        params,
        return_type: Box::new(return_type),
        contexts: None,
        type_params: List::new(),
        properties: None,
    }
}

/// Convert an AST type to a simple internal Type.
/// Handles common cases; complex types fall back to Type::Unknown.
fn convert_simple_ast_type(ast_ty: &verum_ast::ty::Type) -> Type {
    use verum_ast::ty::PathSegment;
    use verum_ast::ty::TypeKind;

    match &ast_ty.kind {
        TypeKind::Path(path) => {
            if let Some(seg) = path.segments.last() {
                match seg {
                    PathSegment::Name(ident) => {
                        let s = ident.as_str();
                        match s {
                            s if WKT::Int.matches(s) => Type::Int,
                            s if WKT::Float.matches(s) => Type::Float,
                            s if WKT::Bool.matches(s) => Type::Bool,
                            s if WKT::Char.matches(s) => Type::Char,
                            s if WKT::Text.matches(s) || s == "String" => Type::Text,
                            verum_common::well_known_types::type_names::BYTE => Type::Named {
                                path: path.clone(),
                                args: List::new(),
                            },
                            verum_common::well_known_types::type_names::UNIT => Type::Unit,
                            _ => Type::Named {
                                path: path.clone(),
                                args: List::new(),
                            },
                        }
                    }
                    // Non-name path segments (e.g., generic args) can't be resolved here;
                    // fall back to Unknown (gradual typing top type) for method resolution
                    _ => Type::Unknown,
                }
            } else {
                // Empty path segments — degenerate AST node; Unknown is safe fallback
                Type::Unknown
            }
        }
        TypeKind::Generic { base, args } => {
            // Generic type like List<T>, Map<K,V>, etc.
            let base_type = convert_simple_ast_type(base);
            let converted_args: List<Type> = args
                .iter()
                .filter_map(|a| {
                    if let verum_ast::ty::GenericArg::Type(t) = a {
                        Some(convert_simple_ast_type(t))
                    } else {
                        None
                    }
                })
                .collect();
            match base_type {
                Type::Named { path, .. } => Type::Named {
                    path,
                    args: converted_args,
                },
                // Base type resolved to non-Named (e.g., primitive); can't attach generic args
                _ => Type::Unknown,
            }
        }
        TypeKind::Reference { mutable, inner } => Type::Reference {
            mutable: *mutable,
            inner: Box::new(convert_simple_ast_type(inner)),
        },
        TypeKind::Tuple(elems) => Type::Tuple(elems.iter().map(convert_simple_ast_type).collect()),
        TypeKind::Unit => Type::Unit,
        TypeKind::Never => Type::Never,
        TypeKind::Int => Type::Int,
        TypeKind::Float => Type::Float,
        TypeKind::Bool => Type::Bool,
        TypeKind::Char => Type::Char,
        TypeKind::Text => Type::Text,
        // Unhandled AST TypeKind variants (e.g., function types, closures, impl types);
        // Unknown is the safe gradual-typing fallback for method resolution
        _ => Type::Unknown,
    }
}

/// Extract a canonical type name from a Type
fn type_name_from_type(ty: &Type) -> Text {
    use verum_ast::ty::PathSegment;

    match ty {
        Type::Int => WKT::Int.as_str().into(),
        Type::Float => WKT::Float.as_str().into(),
        Type::Bool => WKT::Bool.as_str().into(),
        Type::Char => WKT::Char.as_str().into(),
        Type::Text => WKT::Text.as_str().into(),
        Type::Unit => "Unit".into(),
        Type::Never => "Never".into(),
        Type::Named { path, .. } => {
            // Use the last segment of the path as the type name
            // PathSegment is an enum, so we need to match on Name(Ident)
            path.segments
                .last()
                .and_then(|seg| {
                    if let PathSegment::Name(ident) = seg {
                        Some(ident.name.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "Unknown".into())
        }
        Type::Generic { name, .. } => name.clone(),
        // Reference types use struct variants with `mutable` and `inner` fields
        Type::Reference { inner, .. } => type_name_from_type(inner),
        Type::CheckedReference { inner, .. } => type_name_from_type(inner),
        Type::UnsafeReference { inner, .. } => type_name_from_type(inner),
        Type::Ownership { inner, .. } => type_name_from_type(inner),
        // Handle other structural types
        Type::Tuple(elements) => {
            if elements.is_empty() {
                "Unit".into()
            } else {
                "Tuple".into()
            }
        }
        Type::Array { element, .. } => format!("Array<{}>", type_name_from_type(element)).into(),
        Type::Slice { element } => format!("Slice<{}>", type_name_from_type(element)).into(),
        Type::Future { output } => format!("Future<{}>", type_name_from_type(output)).into(),
        // Refined types delegate to their base type for method resolution
        Type::Refined { base, .. } => type_name_from_type(base),
        // Sigma types delegate to their base type
        Type::Sigma { fst_type, .. } => type_name_from_type(fst_type),
        _ => "Unknown".into(),
    }
}

/// Check if two strings are similar (simple Levenshtein-like check)
fn is_similar(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }

    let a = a.to_lowercase();
    let b = b.to_lowercase();

    // Check prefix match
    if a.starts_with(&b) || b.starts_with(&a) {
        return true;
    }

    // Simple character difference check
    if (a.len() as i32 - b.len() as i32).abs() <= 2 {
        let mut diffs = 0;
        for (ca, cb) in a.chars().zip(b.chars()) {
            if ca != cb {
                diffs += 1;
            }
        }
        return diffs <= 2;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift-pin: `MethodSourceKind` discriminator projection
    /// (the method-resolution-path 4-variant — distinct from
    /// `verum_protocol_types::MethodSourceKind`).  Pins variant
    /// count, four single-flag classifiers, the AutoDeref chain
    /// resolution invariant, and the live-payload kind()
    /// projection.
    #[test]
    fn meta_pin_method_source_kind_round_trip_and_partitions() {
        // 1. Variant count + names + uniqueness.
        assert_eq!(MethodSourceKind::ALL.len(), 4);
        let mut seen = std::collections::HashSet::new();
        for k in MethodSourceKind::ALL {
            let m = k.meta();
            assert!(
                m.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{:?}: name not snake_case",
                k
            );
            assert!(seen.insert(m.name), "{:?}: duplicate name", k);
        }

        // 2. is_user_impl — Inherent + Protocol.  AutoDeref
        //    delegates to wrapped source; Builtin is compiler-
        //    internal.
        let user: Vec<_> = MethodSourceKind::ALL
            .iter()
            .filter(|k| k.meta().is_user_impl)
            .copied()
            .collect();
        assert_eq!(
            user,
            vec![MethodSourceKind::Inherent, MethodSourceKind::Protocol],
        );

        // 3. is_protocol_dispatch — Protocol singleton.
        let pd: Vec<_> = MethodSourceKind::ALL
            .iter()
            .filter(|k| k.meta().is_protocol_dispatch)
            .copied()
            .collect();
        assert_eq!(pd, vec![MethodSourceKind::Protocol]);

        // 4. is_chained — AutoDeref singleton.
        let ch: Vec<_> = MethodSourceKind::ALL
            .iter()
            .filter(|k| k.meta().is_chained)
            .copied()
            .collect();
        assert_eq!(ch, vec![MethodSourceKind::AutoDeref]);

        // 5. is_builtin — Builtin singleton.
        let bi: Vec<_> = MethodSourceKind::ALL
            .iter()
            .filter(|k| k.meta().is_builtin)
            .copied()
            .collect();
        assert_eq!(bi, vec![MethodSourceKind::Builtin]);

        // 6. is_protocol_dispatch ⇒ is_user_impl (protocol
        //    dispatch always lands in a user-written impl
        //    block, never in builtin or auto-deref).
        for k in MethodSourceKind::ALL {
            let m = k.meta();
            assert!(
                !m.is_protocol_dispatch || m.is_user_impl,
                "{:?}: protocol-dispatch ⇒ user-impl",
                k
            );
        }

        // 7. is_chained ⊕ is_builtin (chaining and built-in are
        //    disjoint — auto-deref delegates to a wrapped
        //    source which itself can never be built-in
        //    in the chain head).  More importantly, no variant
        //    flips both.
        for k in MethodSourceKind::ALL {
            let m = k.meta();
            assert!(!(m.is_chained && m.is_builtin), "{:?}: chained ⊕ builtin", k);
        }

        // 8. Live-payload kind() projection.
        let inh = MethodSource::Inherent { type_name: Text::from("List") };
        assert_eq!(inh.kind(), MethodSourceKind::Inherent);

        let prot = MethodSource::Protocol {
            protocol_name: Text::from("Eq"),
            type_name: Text::from("Int"),
        };
        assert_eq!(prot.kind(), MethodSourceKind::Protocol);

        let bi = MethodSource::Builtin { name: Text::from("primitive_add") };
        assert_eq!(bi.kind(), MethodSourceKind::Builtin);
    }

    /// Drift-pin: `MethodSource::resolved_kind()` walks through
    /// the AutoDeref chain to a non-AutoDeref leaf.  Pinned so
    /// that no caller of `resolved_kind` ever observes
    /// `MethodSourceKind::AutoDeref` — the chain always bottoms
    /// out.
    #[test]
    fn meta_pin_method_source_resolved_kind() {
        // Single layer: AutoDeref → Inherent.
        let chain1 = MethodSource::AutoDeref {
            through_type: Text::from("Heap"),
            inner_source: Box::new(MethodSource::Inherent {
                type_name: Text::from("List"),
            }),
        };
        assert_eq!(chain1.resolved_kind(), MethodSourceKind::Inherent);
        // The unresolved kind is still AutoDeref.
        assert_eq!(chain1.kind(), MethodSourceKind::AutoDeref);

        // Two layers: AutoDeref → AutoDeref → Builtin.
        let chain2 = MethodSource::AutoDeref {
            through_type: Text::from("Shared"),
            inner_source: Box::new(MethodSource::AutoDeref {
                through_type: Text::from("Heap"),
                inner_source: Box::new(MethodSource::Builtin {
                    name: Text::from("primitive_clone"),
                }),
            }),
        };
        assert_eq!(chain2.resolved_kind(), MethodSourceKind::Builtin);

        // Non-deref leaf: resolved == kind.
        for k in MethodSourceKind::ALL {
            if *k == MethodSourceKind::AutoDeref {
                continue;
            }
            let leaf = match k {
                MethodSourceKind::Inherent => MethodSource::Inherent {
                    type_name: Text::from("X"),
                },
                MethodSourceKind::Protocol => MethodSource::Protocol {
                    protocol_name: Text::from("P"),
                    type_name: Text::from("X"),
                },
                MethodSourceKind::Builtin => MethodSource::Builtin {
                    name: Text::from("b"),
                },
                MethodSourceKind::AutoDeref => unreachable!(),
            };
            assert_eq!(leaf.resolved_kind(), leaf.kind());
        }
    }

    /// Drift-pin: `MethodErrorKind` discriminator projection.
    /// Pins the five resolution-stage classifiers, the
    /// suggestion-bearing partition, and cross-cutting
    /// invariants.
    #[test]
    fn meta_pin_method_error_kind_round_trip_and_partitions() {
        // 1. Variant count + names.
        assert_eq!(MethodErrorKind::ALL.len(), 5);
        let mut seen = std::collections::HashSet::new();
        for k in MethodErrorKind::ALL {
            let m = k.meta();
            assert!(
                m.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{:?}: name not snake_case",
                k
            );
            assert!(seen.insert(m.name), "{:?}: duplicate name", k);
        }

        // 2. is_name_lookup_failure — MethodNotFound singleton.
        let nl: Vec<_> = MethodErrorKind::ALL
            .iter()
            .filter(|k| k.meta().is_name_lookup_failure)
            .copied()
            .collect();
        assert_eq!(nl, vec![MethodErrorKind::MethodNotFound]);

        // 3. is_signature_mismatch — MutabilityMismatch +
        //    ProtocolBoundNotSatisfied (both found a method
        //    but the signature didn't match).
        let sm: Vec<_> = MethodErrorKind::ALL
            .iter()
            .filter(|k| k.meta().is_signature_mismatch)
            .copied()
            .collect();
        assert_eq!(
            sm,
            vec![
                MethodErrorKind::MutabilityMismatch,
                MethodErrorKind::ProtocolBoundNotSatisfied,
            ],
        );

        // 4. is_ambiguity — AmbiguousMethod singleton.
        let am: Vec<_> = MethodErrorKind::ALL
            .iter()
            .filter(|k| k.meta().is_ambiguity)
            .copied()
            .collect();
        assert_eq!(am, vec![MethodErrorKind::AmbiguousMethod]);

        // 5. is_inference_failure — TypeInferenceFailed singleton.
        let inf: Vec<_> = MethodErrorKind::ALL
            .iter()
            .filter(|k| k.meta().is_inference_failure)
            .copied()
            .collect();
        assert_eq!(inf, vec![MethodErrorKind::TypeInferenceFailed]);

        // 6. carries_suggestions — MethodNotFound singleton
        //    (the only failure mode that surfaces "did you mean
        //    X" UI).
        let sug: Vec<_> = MethodErrorKind::ALL
            .iter()
            .filter(|k| k.meta().carries_suggestions)
            .copied()
            .collect();
        assert_eq!(sug, vec![MethodErrorKind::MethodNotFound]);

        // 7. carries_suggestions ⇒ is_name_lookup_failure
        //    (suggestions only meaningful when name lookup
        //    failed — at other stages we already found a
        //    method, just couldn't use it).
        for k in MethodErrorKind::ALL {
            let m = k.meta();
            assert!(
                !m.carries_suggestions || m.is_name_lookup_failure,
                "{:?}: suggestions ⇒ name-lookup-failure",
                k
            );
        }

        // 8. Every variant flips exactly one of the four
        //    primary failure-mode classifiers
        //    (name-lookup / signature / ambiguity / inference).
        //    Perfect partition pin.  Note signature mismatch
        //    has 2 variants, the others are singletons.
        for k in MethodErrorKind::ALL {
            let m = k.meta();
            let count = (m.is_name_lookup_failure as u32)
                + (m.is_signature_mismatch as u32)
                + (m.is_ambiguity as u32)
                + (m.is_inference_failure as u32);
            assert_eq!(
                count, 1,
                "{:?}: must flip exactly one failure-mode classifier",
                k
            );
        }

        // 9. Live-payload kind() + suggestions() + span()
        //    projection.
        let nf = MethodError::MethodNotFound {
            receiver_type: Type::Int,
            method_name: Text::from("frobnicate"),
            span: Maybe::None,
            suggestions: List::from(vec![Text::from("frobenius")]),
        };
        assert_eq!(nf.kind(), MethodErrorKind::MethodNotFound);
        assert_eq!(nf.suggestions().unwrap().len(), 1);
        assert!(matches!(nf.span(), Maybe::None));

        let mm = MethodError::MutabilityMismatch {
            method_name: Text::from("push"),
            expected_mut: true,
            actual_mut: false,
            span: Maybe::None,
        };
        assert_eq!(mm.kind(), MethodErrorKind::MutabilityMismatch);
        assert!(mm.suggestions().is_none());
    }

    #[test]
    fn test_type_name_extraction() {
        assert_eq!(type_name_from_type(&Type::Int).as_str(), "Int");
        assert_eq!(type_name_from_type(&Type::Text).as_str(), "Text");
        // Use Type::Generic for simple generic types like List<T>
        assert_eq!(
            type_name_from_type(&Type::Generic {
                name: "List".into(),
                args: List::new()
            })
            .as_str(),
            "List"
        );
        // Test reference unwrapping
        assert_eq!(
            type_name_from_type(&Type::Reference {
                mutable: false,
                inner: Box::new(Type::Int)
            })
            .as_str(),
            "Int"
        );
    }

    #[test]
    fn test_similarity_check() {
        assert!(is_similar("length", "length"));
        assert!(is_similar("length", "len"));
        assert!(is_similar("contains", "contain"));
        assert!(!is_similar("foo", "bar"));
    }

    #[test]
    fn test_default_resolver_stats() {
        let resolver = DefaultMethodResolver::new();
        let stats = resolver.stats();
        assert_eq!(stats.inherent_impl_count, 0);
        assert_eq!(stats.protocol_impl_count, 0);
    }

    #[test]
    fn test_method_source_display() {
        let inherent = MethodSource::Inherent {
            type_name: "List".into(),
        };
        let protocol = MethodSource::Protocol {
            protocol_name: "Iterator".into(),
            type_name: "List".into(),
        };
        // Just ensure these don't panic
        assert!(matches!(inherent, MethodSource::Inherent { .. }));
        assert!(matches!(protocol, MethodSource::Protocol { .. }));
    }
}
