//! Type Introspection Sub-Context
//!
//! Manages type definitions, protocol implementations, and type registry
//! for compile-time type reflection.
//!
//! ## Responsibility
//!
//! - Type definitions (structs, enums, protocols)
//! - Protocol implementation registry
//! - Type metadata and attributes
//! - Method resolution
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_ast::ty::Type;
use verum_common::{List, Map, Maybe, Text};

use crate::meta::{
    CodeSearchTypeInfo, FunctionInfo, MethodResolution, MethodSource, ModuleInfo,
    ProtocolImplementation, TypeDefinition, UsageInfo,
};

/// Type attribute information
#[derive(Debug, Clone)]
pub struct TypeAttribute {
    /// Attribute name
    pub name: Text,
    /// Attribute value (if any)
    pub value: Maybe<Text>,
    /// Attribute arguments
    pub args: List<Text>,
}

/// Type introspection context
///
/// Manages type definitions, protocol implementations, and provides
/// type reflection capabilities for meta functions.
#[derive(Debug, Clone, Default)]
pub struct TypeIntrospection {
    /// Type definitions (name -> definition)
    type_definitions: Map<Text, TypeDefinition>,

    /// Protocol implementations ((type_name, protocol_name) -> implementation)
    protocol_implementations: Map<(Text, Text), ProtocolImplementation>,

    /// Type registry for code search (name -> type info)
    type_registry: Map<Text, CodeSearchTypeInfo>,

    /// Function usage index (function name -> usages)
    usage_index: Map<Text, List<UsageInfo>>,

    /// Type usage index (type name -> usages)
    type_usage_index: Map<Text, List<UsageInfo>>,

    /// Constant usage index (const name -> usages)
    const_usage_index: Map<Text, List<UsageInfo>>,

    /// Module registry (module path -> module info)
    module_registry: Map<Text, ModuleInfo>,

    /// Type attributes ((type_name, attr_name) -> attribute)
    type_attributes: Map<(Text, Text), TypeAttribute>,

    /// Type documentation (type_name -> doc string)
    type_docs: Map<Text, Text>,

    /// Associated types ((type_name, assoc_name) -> type)
    associated_types: Map<(Text, Text), Type>,

    /// Super types / protocol hierarchy (protocol -> super protocols)
    super_types: Map<Text, List<Text>>,
}

impl TypeIntrospection {
    /// Create a new empty type introspection context
    pub fn new() -> Self {
        Self::default()
    }

    // ======== Type Definition Operations ========

    /// Register a struct type
    pub fn register_struct(&mut self, name: Text, fields: List<(Text, Type)>) {
        self.type_definitions.insert(
            name.clone(),
            TypeDefinition::simple_struct(name, fields),
        );
    }

    /// Register an enum type
    pub fn register_enum(&mut self, name: Text, variants: List<(Text, Type)>) {
        self.type_definitions.insert(
            name.clone(),
            TypeDefinition::simple_enum(name, variants),
        );
    }

    /// Register a protocol type
    pub fn register_protocol(&mut self, name: Text, methods: List<Text>) {
        self.type_definitions.insert(
            name.clone(),
            TypeDefinition::simple_protocol(name, methods),
        );
    }

    /// Register a full type definition
    pub fn register_type_definition(&mut self, type_def: TypeDefinition) {
        self.type_definitions.insert(
            type_def.name().clone(),
            type_def,
        );
    }

    /// Get a type definition
    #[inline]
    pub fn get_type_definition(&self, name: &Text) -> Option<&TypeDefinition> {
        self.type_definitions.get(name)
    }

    /// Get struct fields
    pub fn get_struct_fields(&self, name: &Text) -> Option<&List<(Text, Type)>> {
        match self.type_definitions.get(name) {
            Some(TypeDefinition::Struct { fields, .. }) => Some(fields),
            _ => None,
        }
    }

    /// Get enum variants
    pub fn get_enum_variants(&self, name: &Text) -> Option<&List<(Text, Type)>> {
        match self.type_definitions.get(name) {
            Some(TypeDefinition::Enum { variants, .. }) => Some(variants),
            _ => None,
        }
    }

    /// Get protocol method names
    pub fn get_protocol_methods(&self, name: &Text) -> Option<List<Text>> {
        match self.type_definitions.get(name) {
            Some(TypeDefinition::Protocol { methods, .. }) => {
                Some(methods.iter().map(|m| m.name.clone()).collect())
            },
            _ => None,
        }
    }

    /// Get type functions (methods defined in impl blocks)
    pub fn get_type_functions(&self, type_name: &Text) -> List<FunctionInfo> {
        let mut functions = List::new();
        for ((ty, _protocol), impl_def) in &self.protocol_implementations {
            if ty == type_name {
                for method in &impl_def.implemented_methods {
                    functions.push(FunctionInfo::new(method.clone(), Text::from("()")));
                }
            }
        }
        functions
    }

    /// Clear all type definitions
    #[inline]
    pub fn clear_type_definitions(&mut self) {
        self.type_definitions.clear();
    }

    // ======== Protocol Implementation Operations ========

    /// Register a protocol implementation
    pub fn register_protocol_implementation(
        &mut self,
        type_name: Text,
        protocol_name: Text,
        methods: List<Text>,
    ) {
        self.protocol_implementations.insert(
            (type_name.clone(), protocol_name.clone()),
            ProtocolImplementation {
                implementing_type: type_name,
                protocol_name,
                implemented_methods: methods,
            },
        );
    }

    /// Get protocols implemented by a type
    pub fn get_implemented_protocols(&self, type_name: &Text) -> List<Text> {
        self.protocol_implementations
            .keys()
            .filter(|(ty, _)| ty == type_name)
            .map(|(_, proto)| proto.clone())
            .collect()
    }

    /// Get types that implement a protocol
    pub fn get_implementors(&self, protocol_name: &Text) -> List<Text> {
        self.protocol_implementations
            .keys()
            .filter(|(_, proto)| proto == protocol_name)
            .map(|(ty, _)| ty.clone())
            .collect()
    }

    /// Get protocol implementation details
    pub fn get_protocol_implementation(
        &self,
        type_name: &Text,
        protocol_name: &Text,
    ) -> Option<&ProtocolImplementation> {
        self.protocol_implementations
            .get(&(type_name.clone(), protocol_name.clone()))
    }

    /// Check if type implements protocol
    pub fn type_implements_protocol(&self, type_name: &Text, protocol_name: &Text) -> bool {
        self.protocol_implementations
            .contains_key(&(type_name.clone(), protocol_name.clone()))
    }

    /// Clear all protocol implementations
    #[inline]
    pub fn clear_protocol_implementations(&mut self) {
        self.protocol_implementations.clear();
    }

    // ======== Method Resolution ========

    /// Resolve a method on a type
    pub fn resolve_method(&self, type_name: &Text, method_name: &Text) -> Option<MethodResolution> {
        // First check inherent methods
        for ((ty, protocol), impl_def) in &self.protocol_implementations {
            if ty == type_name {
                for method in &impl_def.implemented_methods {
                    if method == method_name {
                        return Some(MethodResolution {
                            function: FunctionInfo::new(method.clone(), Text::from("()")),
                            source: MethodSource::Inherent,
                            providing_protocol: if protocol.is_empty() {
                                Maybe::None
                            } else {
                                Maybe::Some(protocol.clone())
                            },
                            is_default_impl: false,
                        });
                    }
                }
            }
        }
        None
    }

    // ======== Type Attributes ========

    /// Register a type attribute
    pub fn register_type_attribute(&mut self, type_name: Text, attr: TypeAttribute) {
        self.type_attributes
            .insert((type_name, attr.name.clone()), attr);
    }

    /// Get all attributes for a type
    pub fn get_type_attributes(&self, type_name: &Text) -> List<Text> {
        self.type_attributes
            .iter()
            .filter(|((ty, _), _)| ty == type_name)
            .map(|((_, name), _)| name.clone())
            .collect()
    }

    /// Check if type has attribute
    pub fn type_has_attribute(&self, type_name: &Text, attr_name: &Text) -> bool {
        self.type_attributes
            .contains_key(&(type_name.clone(), attr_name.clone()))
    }

    /// Get type attribute
    pub fn get_type_attribute(&self, type_name: &Text, attr_name: &Text) -> Option<&TypeAttribute> {
        self.type_attributes
            .get(&(type_name.clone(), attr_name.clone()))
    }

    // ======== Type Documentation ========

    /// Set type documentation
    pub fn set_type_doc(&mut self, type_name: Text, doc: Text) {
        self.type_docs.insert(type_name, doc);
    }

    /// Get type documentation
    pub fn get_type_doc(&self, type_name: &Text) -> Maybe<&Text> {
        self.type_docs.get(type_name).into()
    }

    // ======== Associated Types ========

    /// Register an associated type
    pub fn register_associated_type(&mut self, type_name: Text, assoc_name: Text, assoc_type: Type) {
        self.associated_types
            .insert((type_name, assoc_name), assoc_type);
    }

    /// Get associated types for a type
    pub fn get_associated_types(&self, type_name: &Text) -> List<(Text, Type)> {
        self.associated_types
            .iter()
            .filter(|((ty, _), _)| ty == type_name)
            .map(|((_, name), ty)| (name.clone(), ty.clone()))
            .collect()
    }

    // ======== Super Types (Protocol Hierarchy) ========

    /// Register super types for a protocol
    pub fn register_super_types(&mut self, protocol_name: Text, supers: List<Text>) {
        self.super_types.insert(protocol_name, supers);
    }

    /// Get super types for a type/protocol
    pub fn get_super_types(&self, type_name: &Text) -> List<Text> {
        self.super_types
            .get(type_name)
            .cloned()
            .unwrap_or_default()
    }

    // ======== Code Search Operations ========

    /// Register type in code search registry
    pub fn register_type_info(&mut self, name: Text, info: CodeSearchTypeInfo) {
        self.type_registry.insert(name, info);
    }

    /// Get type info from registry
    pub fn get_type_info(&self, name: &Text) -> Option<&CodeSearchTypeInfo> {
        self.type_registry.get(name)
    }

    /// Get the type registry
    pub fn type_registry(&self) -> &Map<Text, CodeSearchTypeInfo> {
        &self.type_registry
    }

    /// Get the usage index
    pub fn usage_index(&self) -> &Map<Text, List<UsageInfo>> {
        &self.usage_index
    }

    /// Get mutable usage index
    pub fn usage_index_mut(&mut self) -> &mut Map<Text, List<UsageInfo>> {
        &mut self.usage_index
    }

    /// Get type usage index
    pub fn type_usage_index(&self) -> &Map<Text, List<UsageInfo>> {
        &self.type_usage_index
    }

    /// Get mutable type usage index
    pub fn type_usage_index_mut(&mut self) -> &mut Map<Text, List<UsageInfo>> {
        &mut self.type_usage_index
    }

    /// Get const usage index
    pub fn const_usage_index(&self) -> &Map<Text, List<UsageInfo>> {
        &self.const_usage_index
    }

    /// Get mutable const usage index
    pub fn const_usage_index_mut(&mut self) -> &mut Map<Text, List<UsageInfo>> {
        &mut self.const_usage_index
    }

    /// Get module registry
    pub fn module_registry(&self) -> &Map<Text, ModuleInfo> {
        &self.module_registry
    }

    /// Get mutable module registry
    pub fn module_registry_mut(&mut self) -> &mut Map<Text, ModuleInfo> {
        &mut self.module_registry
    }

    // ======== Clear All ========

    /// Clear all type introspection state
    pub fn clear_all(&mut self) {
        self.type_definitions.clear();
        self.protocol_implementations.clear();
        self.type_registry.clear();
        self.usage_index.clear();
        self.type_usage_index.clear();
        self.const_usage_index.clear();
        self.module_registry.clear();
        self.type_attributes.clear();
        self.type_docs.clear();
        self.associated_types.clear();
        self.super_types.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::Span;

    #[test]
    fn test_register_struct() {
        let mut ti = TypeIntrospection::new();
        ti.register_struct(
            Text::from("Point"),
            List::from(vec![
                (Text::from("x"), Type::int(Span::dummy())),
                (Text::from("y"), Type::int(Span::dummy())),
            ]),
        );
        assert!(ti.get_type_definition(&Text::from("Point")).is_some());
        assert!(ti.get_struct_fields(&Text::from("Point")).is_some());
    }

    #[test]
    fn test_protocol_implementation() {
        let mut ti = TypeIntrospection::new();
        ti.register_protocol_implementation(
            Text::from("Point"),
            Text::from("Debug"),
            List::from(vec![Text::from("debug")]),
        );
        assert!(ti.type_implements_protocol(&Text::from("Point"), &Text::from("Debug")));
        assert!(!ti.type_implements_protocol(&Text::from("Point"), &Text::from("Clone")));
    }

    #[test]
    fn test_type_attributes() {
        let mut ti = TypeIntrospection::new();
        ti.register_type_attribute(
            Text::from("Point"),
            TypeAttribute {
                name: Text::from("derive"),
                value: Maybe::None,
                args: List::from(vec![Text::from("Debug"), Text::from("Clone")]),
            },
        );
        assert!(ti.type_has_attribute(&Text::from("Point"), &Text::from("derive")));
        assert!(!ti.type_has_attribute(&Text::from("Point"), &Text::from("repr")));
    }
}
