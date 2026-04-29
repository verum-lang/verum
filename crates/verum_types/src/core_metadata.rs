//! Stdlib type metadata definitions
//!
//! This module defines data structures for representing stdlib type information
//! that can be loaded from pre-compiled stdlib.vbc archives.
//!
//! Stdlib type metadata definitions extracted from core .vr files during compilation pipeline.

use verum_common::{List, Maybe, OrderedMap, Text};

/// Metadata extracted from stdlib.vbc for type checking
///
/// This struct contains all the type information needed to compile user code
/// without having to parse stdlib source files.
///
/// Uses `OrderedMap` (BTreeMap) instead of `Map` (HashMap) to ensure
/// deterministic iteration order. HashMap iteration order depends on the
/// per-process random hash seed, causing non-deterministic type registration
/// order and intermittent type resolution failures.
#[derive(Debug, Clone, Default)]
pub struct CoreMetadata {
    /// Type definitions (List<T>, Map<K,V>, Maybe<T>, etc.)
    ///
    /// Storage is `OrderedMap` (BTreeMap-backed) for O(log n) lookup by name.
    /// Iteration order from this map is alphabetical and MUST NOT be used for
    /// any operation where source declaration order matters — see
    /// `type_declaration_order` for that.
    pub types: OrderedMap<Text, TypeDescriptor>,

    /// Type names in source declaration order (load order across the archive).
    ///
    /// Recorded as types are inserted into `types`. Provides a stable iteration
    /// order that reflects stdlib layer ordering (Core → Text → Collections → …)
    /// and per-module .vr file declaration order.
    ///
    /// Critical for: variant-name registration (Maybe must register `None|Some`
    /// before any sibling cog's variant aliases), inductive-constructor
    /// registration (Result must beat CheckedResult to the `Ok|Err` signature),
    /// and any other first-registered-wins resolution. Without this list, sites
    /// would hardcode stdlib type names (Result/Maybe/Ordering/Bool) to force
    /// priority — a violation of the no-stdlib-knowledge-in-compiler rule.
    pub type_declaration_order: List<Text>,

    /// Protocol definitions (Eq, Ord, Clone, Iterator, etc.)
    pub protocols: OrderedMap<Text, ProtocolDescriptor>,

    /// Function signatures for method resolution
    pub functions: OrderedMap<Text, FunctionDescriptor>,

    /// Protocol implementations (impl Eq for Int, etc.)
    pub implementations: List<ImplementationDescriptor>,

    /// Pre-computed monomorphizations (List<Int>, Map<Text, Int>, etc.)
    pub monomorphizations: OrderedMap<Text, MonomorphizedType>,

    /// Version of stdlib.vbc format
    pub version: u32,

    /// Content hash for cache validation
    pub content_hash: [u8; 32],

    /// Context protocol names declared in the stdlib.
    /// These are registered as available contexts during
    /// NormalBuild type-checking so that `using [ComputeDevice]`
    /// etc. resolve without requiring the declaring module to
    /// be loaded first. Populated during stdlib bootstrap
    /// from `context Name { ... }` and `context protocol Name { ... }`
    /// declarations.
    pub context_declarations: List<Text>,
}

/// Descriptor for a type definition in stdlib
#[derive(Debug, Clone)]
pub struct TypeDescriptor {
    /// Type name (e.g., "List", "Map", "Maybe")
    pub name: Text,

    /// Module path (e.g., "std.collections")
    pub module_path: Text,

    /// Generic parameters (e.g., ["T"] for List<T>)
    pub generic_params: List<GenericParam>,

    /// Type kind (struct, variant, protocol, alias)
    pub kind: TypeDescriptorKind,

    /// Size in bytes (None if generic)
    pub size: Maybe<usize>,

    /// Alignment in bytes (None if generic)
    pub alignment: Maybe<usize>,

    /// Associated methods
    pub methods: List<Text>,

    /// Protocol implementations
    pub implements: List<Text>,
}

/// Generic parameter descriptor
#[derive(Debug, Clone)]
pub struct GenericParam {
    /// Parameter name (e.g., "T", "K", "V")
    pub name: Text,

    /// Protocol bounds (e.g., ["Eq", "Hash"])
    pub bounds: List<Text>,

    /// Default type (if any)
    pub default: Maybe<Text>,
}

/// Kind of type descriptor
#[derive(Debug, Clone)]
pub enum TypeDescriptorKind {
    /// Record type (struct)
    Record { fields: List<FieldDescriptor> },

    /// Variant type (enum)
    Variant { cases: List<VariantCase> },

    /// Protocol type (trait)
    Protocol {
        super_protocols: List<Text>,
        associated_types: List<AssociatedTypeDescriptor>,
        required_methods: List<MethodSignature>,
        default_methods: List<Text>,
    },

    /// Type alias
    Alias { target: Text },

    /// Opaque type (FFI or builtin)
    Opaque,
}

/// Field descriptor for record types
#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub name: Text,
    pub ty: Text,
    pub is_public: bool,
}

/// Variant case descriptor
#[derive(Debug, Clone)]
pub struct VariantCase {
    pub name: Text,
    pub payload: Maybe<VariantPayload>,
}

/// Variant payload type
#[derive(Debug, Clone)]
pub enum VariantPayload {
    /// Tuple payload: Some(T)
    Tuple(List<Text>),
    /// Record payload: Node { left: T, right: T }
    Record(List<FieldDescriptor>),
}

/// Associated type descriptor
#[derive(Debug, Clone)]
pub struct AssociatedTypeDescriptor {
    pub name: Text,
    pub bounds: List<Text>,
    pub default: Maybe<Text>,
}

/// Method signature descriptor
#[derive(Debug, Clone)]
pub struct MethodSignature {
    pub name: Text,
    pub receiver: ReceiverKind,
    pub params: List<ParamDescriptor>,
    pub return_type: Text,
    pub contexts: List<Text>,
    pub is_async: bool,
}

/// Receiver kind for methods
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    None,
    SelfValue,
    SelfRef,
    SelfMut,
}

/// Parameter descriptor
#[derive(Debug, Clone)]
pub struct ParamDescriptor {
    pub name: Text,
    pub ty: Text,
}

/// Protocol definition descriptor
#[derive(Debug, Clone)]
pub struct ProtocolDescriptor {
    /// Protocol name
    pub name: Text,

    /// Module path
    pub module_path: Text,

    /// Generic parameters
    pub generic_params: List<GenericParam>,

    /// Super protocols (inheritance)
    pub super_protocols: List<Text>,

    /// Associated types
    pub associated_types: List<AssociatedTypeDescriptor>,

    /// Required methods (must be implemented)
    pub required_methods: List<MethodSignature>,

    /// Default methods (have implementations)
    pub default_methods: List<MethodSignature>,
}

/// Function definition descriptor
#[derive(Debug, Clone)]
pub struct FunctionDescriptor {
    /// Function name
    pub name: Text,

    /// Module path
    pub module_path: Text,

    /// Generic parameters
    pub generic_params: List<GenericParam>,

    /// Parameters
    pub params: List<ParamDescriptor>,

    /// Return type
    pub return_type: Text,

    /// Context requirements
    pub contexts: List<Text>,

    /// Is async function
    pub is_async: bool,

    /// Is unsafe function
    pub is_unsafe: bool,

    /// Intrinsic ID (if compiler intrinsic)
    pub intrinsic_id: Maybe<u32>,
}

/// Implementation descriptor (impl Protocol for Type)
#[derive(Debug, Clone)]
pub struct ImplementationDescriptor {
    /// Protocol being implemented
    pub protocol: Text,

    /// Type implementing the protocol
    pub target_type: Text,

    /// Generic parameters
    pub generic_params: List<GenericParam>,

    /// Where clause constraints
    pub where_clause: List<Text>,

    /// Associated type bindings
    pub associated_types: OrderedMap<Text, Text>,

    /// Method implementations
    pub methods: List<Text>,
}

/// Pre-monomorphized type
#[derive(Debug, Clone)]
pub struct MonomorphizedType {
    /// Generic type name (e.g., "List")
    pub generic_name: Text,

    /// Type arguments (e.g., ["Int"])
    pub type_args: List<Text>,

    /// Computed size
    pub size: usize,

    /// Computed alignment
    pub alignment: usize,

    /// VBC code offset for specialized methods
    pub code_offset: u64,
}

/// Layer ordering for stdlib modules (from stdlib VBC layer specification)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StdlibLayer {
    /// Layer 0: Core (primitives, memory, maybe, result, ordering, protocols)
    Core = 0,
    /// Layer 1: Text (text, char, format)
    Text = 1,
    /// Layer 2: Collections (list, map, set, deque, heap, tree)
    Collections = 2,
    /// Layer 3: I/O (protocols, path, file, buffer, stdio, fs)
    Io = 3,
    /// Layer 4: Async (future, task, executor, channel, mutex, select)
    Async = 4,
    /// Layer 5: Network (socket, tcp, udp, dns, tls, http)
    Network = 5,
    /// Layer 6: Cognitive (tensor, device, autodiff, module, optimizer, agent)
    Cognitive = 6,
}

impl StdlibLayer {
    /// Get the prefix used for modules in this layer
    pub fn prefix(&self) -> &'static str {
        match self {
            StdlibLayer::Core => "core/",
            StdlibLayer::Text => "text/",
            StdlibLayer::Collections => "collections/",
            StdlibLayer::Io => "io/",
            StdlibLayer::Async => "async/",
            StdlibLayer::Network => "net/",
            StdlibLayer::Cognitive => "cognitive/",
        }
    }

    /// Get the module order for this layer.
    ///
    /// Derived from `ModuleOrder::default_order()` in `core_pipeline.rs`,
    /// which is the single source of truth for stdlib module ordering.
    pub fn modules(&self) -> Vec<&'static str> {
        use crate::core_pipeline::ModuleOrder;
        let prefix = self.prefix();
        ModuleOrder::default_order()
            .iter()
            .copied()
            .filter(|m| m.starts_with(prefix))
            .collect()
    }
}

/// Get full stdlib module compilation order.
///
/// Delegates to `ModuleOrder::default_order()` in `core_pipeline.rs`,
/// which is the single source of truth for stdlib module ordering.
pub fn stdlib_module_order() -> Vec<&'static str> {
    use crate::core_pipeline::ModuleOrder;
    ModuleOrder::default_order().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdlib_layer_order() {
        let order = stdlib_module_order();
        assert!(order.len() > 30, "Should have many modules");
        assert_eq!(order[0], "core/primitives", "Primitives must be first");
        assert_eq!(order.last(), Some(&"mod"), "Root module must be last");
    }
}
