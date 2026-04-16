//! Stdlib Integration Module
//!
//! This module provides the bridge between the stdlib-agnostic type system architecture
//! and the existing `ProtocolChecker` infrastructure.
//!
//! ## Key Components
//!
//! - `StdlibAgnosticChecker`: A wrapper around `ProtocolChecker` that supports
//!   dynamic method registration from parsed stdlib files or preloaded metadata.
//!
//! - `DynamicMethodRegistry`: Registers methods dynamically instead of hardcoding
//!   them in Rust code.
//!
//! ## Usage
//!
//! ```ignore
//! // Create with preloaded metadata (normal usage)
//! let checker = StdlibAgnosticChecker::with_metadata(core_metadata);
//!
//! // Create empty and register methods dynamically
//! let mut checker = StdlibAgnosticChecker::new();
//! checker.register_inherent_method("List", "len", signature);
//!
//! // Legacy mode - for backward compatibility with hardcoded methods
//! let checker = StdlibAgnosticChecker::legacy();
//! ```

use verum_common::{List, Text};

use crate::method_resolution::{DefaultMethodResolver, MethodInfo, MethodResolver};
use crate::protocol::{MethodSignature, ProtocolChecker};
use crate::core_metadata::{ImplementationDescriptor, CoreMetadata};
use crate::ty::Type;

/// A stdlib-agnostic wrapper around ProtocolChecker
///
/// This wrapper enables dynamic method registration from parsed stdlib files
/// or preloaded metadata, removing the need for hardcoded method definitions.
///
/// Note: Protocol resolution is fully protocol-based. All special operators
/// (IntoIterator, Future, Try, Index, Maybe) resolve via registered implementations
/// rather than hardcoded type name matching.
pub struct StdlibAgnosticChecker {
    /// The underlying protocol checker
    inner: ProtocolChecker,

    /// Dynamic method resolver for additional lookups
    dynamic_resolver: DefaultMethodResolver,
}

impl std::fmt::Debug for StdlibAgnosticChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdlibAgnosticChecker")
            .finish_non_exhaustive()
    }
}

impl StdlibAgnosticChecker {
    /// Create a new empty checker (methods registered dynamically)
    ///
    /// In this mode:
    /// - Standard protocols are registered (Eq, Ord, etc.) as they're fundamental
    /// - Methods must be registered via `register_inherent_method()` etc.
    pub fn new() -> Self {
        Self {
            inner: ProtocolChecker::new_empty(),
            dynamic_resolver: DefaultMethodResolver::new(),
        }
    }

    /// Create a checker with preloaded metadata
    ///
    /// In this mode:
    /// - All types, protocols, and methods come from CoreMetadata
    /// - No hardcoded methods are used
    pub fn with_metadata(metadata: CoreMetadata) -> Self {
        let mut checker = Self::new();
        checker.load_from_metadata(&metadata);
        checker
    }

    /// Create a checker with standard protocols and method registry
    ///
    /// This mode uses the standard method registry from ProtocolChecker::new().
    /// Use this for existing tests and gradual migration.
    pub fn legacy() -> Self {
        Self {
            inner: ProtocolChecker::new(),
            dynamic_resolver: DefaultMethodResolver::new(),
        }
    }

    /// Register an inherent method for a type
    ///
    /// # Arguments
    ///
    /// * `type_name` - The type name (e.g., "List", "Map", "Text")
    /// * `method_name` - The method name (e.g., "len", "get", "push")
    /// * `signature` - The method signature
    pub fn register_inherent_method(
        &mut self,
        type_name: &str,
        _method_name: &str,
        signature: MethodSignature,
    ) {
        // Register with inner ProtocolChecker's method registry
        self.inner.register_method_public(type_name, signature);
    }

    /// Register a protocol method for a type
    ///
    /// # Arguments
    ///
    /// * `protocol_name` - The protocol name (e.g., "Eq", "Iterator")
    /// * `type_name` - The implementing type (e.g., "Int", "List")
    /// * `method_name` - The method name (e.g., "eq", "next")
    /// * `signature` - The method signature
    pub fn register_protocol_method(
        &mut self,
        protocol_name: &str,
        type_name: &str,
        method_name: &str,
        signature: Type,
    ) {
        self.inner
            .register_protocol_method_public(protocol_name, type_name, method_name, signature);
    }

    /// Resolve a method call on a type
    pub fn lookup_method(
        &self,
        ty: &Type,
        method_name: &str,
    ) -> Option<crate::protocol::MethodLookupResult> {
        self.inner.lookup_method(ty, method_name)
    }

    /// Check if a type implements a protocol
    pub fn implements_protocol(&self, ty: &Type, protocol_name: &str) -> bool {
        if self.dynamic_resolver.implements_protocol(ty, protocol_name) {
            return true;
        }
        self.inner.implements_by_name(ty, protocol_name)
    }

    /// Get all available methods on a type
    pub fn available_methods(&self, ty: &Type) -> List<MethodInfo> {
        self.dynamic_resolver.available_methods(ty)
    }

    /// Get the underlying ProtocolChecker (for compatibility)
    pub fn inner(&self) -> &ProtocolChecker {
        &self.inner
    }

    /// Get mutable access to the underlying ProtocolChecker
    pub fn inner_mut(&mut self) -> &mut ProtocolChecker {
        &mut self.inner
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    /// Load types, protocols, and methods from CoreMetadata
    fn load_from_metadata(&mut self, metadata: &CoreMetadata) {
        for impl_desc in metadata.implementations.iter() {
            self.register_impl_from_descriptor(impl_desc);
        }
    }

    /// Register an implementation from metadata descriptor
    fn register_impl_from_descriptor(&mut self, desc: &ImplementationDescriptor) {
        for method_name in desc.methods.iter() {
            let sig = MethodSignature::immutable(
                method_name.as_str(),
                List::new(),
                Type::Unknown, // Placeholder - would be parsed from descriptor
            );
            self.inner
                .register_method_public(&desc.target_type, sig.clone());
        }
    }
}

impl Default for StdlibAgnosticChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension methods for ProtocolChecker to support stdlib-agnostic mode
pub trait ProtocolCheckerExt {
    /// Register a method (made public for stdlib integration)
    fn register_method_public(&mut self, type_name: &str, signature: MethodSignature);

    /// Register a protocol method (made public for stdlib integration)
    fn register_protocol_method_public(
        &mut self,
        protocol: &str,
        target_type: &str,
        method: &str,
        signature: Type,
    );
}

impl ProtocolCheckerExt for ProtocolChecker {
    fn register_method_public(&mut self, type_name: &str, signature: MethodSignature) {
        let key = (Text::from(type_name), signature.name.clone());
        self.method_registry_mut().insert(key, signature);
    }

    fn register_protocol_method_public(
        &mut self,
        _protocol: &str,
        target_type: &str,
        method: &str,
        signature: Type,
    ) {
        // Record the method in the flat method registry so lookup_method
        // can find it by (type_name, method_name). Prior implementation
        // was a complete no-op — this at least enables method resolution
        // in stdlib-agnostic mode. Full protocol impl wiring requires
        // access to ProtocolImpl::methods which is not exposed here.
        use crate::protocol::{MethodSignature as Sig, ReceiverKind};
        let (params, return_type) = if let Type::Function { params, return_type, .. } = &signature {
            (params.clone(), (**return_type).clone())
        } else {
            (verum_common::List::new(), signature.clone())
        };
        let sig = Sig {
            name: Text::from(method),
            type_params: verum_common::List::new(),
            receiver: ReceiverKind::Ref,
            params,
            return_type,
            is_mutating: false,
        };
        self.method_registry_mut()
            .insert((Text::from(target_type), Text::from(method)), sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_checker() {
        let _checker = StdlibAgnosticChecker::new();
    }

    #[test]
    fn test_legacy_mode() {
        let _checker = StdlibAgnosticChecker::legacy();
    }

    #[test]
    fn test_metadata_mode() {
        let metadata = CoreMetadata::default();
        let _checker = StdlibAgnosticChecker::with_metadata(metadata);
    }

    #[test]
    fn test_register_inherent_method() {
        let mut checker = StdlibAgnosticChecker::new();

        let sig = MethodSignature::immutable("len", List::new(), Type::Int);
        checker.register_inherent_method("List", "len", sig);

        let list_type = Type::Generic {
            name: "List".into(),
            args: List::from(vec![Type::Int]),
        };
        let result = checker.lookup_method(&list_type, "len");
        assert!(result.is_some());
    }
}
