//! Affine and Linear Type System Implementation
//!
//! Higher-kinded types: type constructors parameterized by type-level functions
//! Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
//!
//! This module implements compile-time verification for resource types:
//!
//! # Resource Kinds
//!
//! - **Copy**: Can be used any number of times (default for primitives)
//! - **Affine**: Can be used at most once (heap-allocated types)
//! - **Linear**: Must be used exactly once (resources requiring explicit cleanup)
//!
//! # Key Features
//!
//! - **At-most-once usage (Affine)**: Values can be used 0 or 1 times
//! - **Exactly-once usage (Linear)**: Values must be used exactly once
//! - **Move semantics**: First use consumes the value
//! - **Cleanup integration**: Unused affine values call cleanup() automatically
//! - **Linear checking**: Unused linear values cause compile error
//! - **CBGR bypass**: Affine references promote to &checked (0ns overhead)
//!
//! # Examples
//!
//! ```verum
//! // Affine type - at most once
//! type affine FileHandle is { fd: Int };
//!
//! // Linear type - exactly once (must be consumed)
//! type linear MustClose is { fd: Int };
//!
//! fn good() {
//!     let f = open_file("data.txt");  // Linear value
//!     close_file(f);                   // OK - consumed exactly once
//! }
//!
//! fn bad() {
//!     let f = open_file("data.txt");
//!     // ERROR: linear value `f` must be consumed exactly once
//! }
//! ```

use crate::TypeError;
use crate::ty::Type;
use verum_ast::decl::ResourceModifier;
use verum_ast::span::Span;
use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::ToText;

// ============================================================================
// Resource Kinds
// ============================================================================

/// Resource kind for type classification.
///
/// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// Can be used any number of times (Copy types).
    Copy,
    /// Can be used at most once (affine types).
    /// Unused values trigger automatic cleanup.
    Affine,
    /// Must be used exactly once (linear types).
    /// Unused values cause compile-time error.
    Linear,
}

impl ResourceKind {
    /// Check if this kind allows multiple uses.
    #[must_use]
    pub fn allows_multiple_use(&self) -> bool {
        matches!(self, ResourceKind::Copy)
    }

    /// Check if this kind requires at-most-once usage.
    #[must_use]
    pub fn is_at_most_once(&self) -> bool {
        matches!(self, ResourceKind::Affine | ResourceKind::Linear)
    }

    /// Check if this kind requires exactly-once usage.
    #[must_use]
    pub fn is_exactly_once(&self) -> bool {
        matches!(self, ResourceKind::Linear)
    }
}

impl From<ResourceModifier> for ResourceKind {
    fn from(modifier: ResourceModifier) -> Self {
        match modifier {
            ResourceModifier::Affine => ResourceKind::Affine,
            ResourceModifier::Linear => ResourceKind::Linear,
        }
    }
}

// ============================================================================
// Affine Tracker
// ============================================================================

/// Tracks usage of affine and linear values during type checking.
///
/// Higher-kinded types: type constructors parameterized by type-level functions
/// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
#[derive(Debug, Clone)]
pub struct AffineTracker {
    /// Map from variable name to binding info
    bindings: Map<Text, AffineBinding>,

    /// Set of affine types (by type name) - at most once
    affine_types: Set<Text>,

    /// Set of linear types (by type name) - exactly once
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
    linear_types: Set<Text>,

    /// Current loop nesting depth (0 = not in loop)
    loop_depth: usize,

    /// Set of variable names that were bound before entering the current loop
    /// Used to detect use of outer-scope affine values in loops
    pre_loop_bindings: Set<Text>,
}

/// Binding information for an affine or linear value.
#[derive(Debug, Clone)]
struct AffineBinding {
    /// The type of the binding
    ty: Type,
    /// Resource kind (Affine or Linear)
    resource_kind: ResourceKind,
    /// Where this binding was introduced
    binding_span: Span,
    /// Where it was first used (if used)
    first_use: Option<Span>,
    /// Whether the value has been consumed
    is_consumed: bool,
    /// Fields that have been moved out (for partial move tracking)
    /// When a field is moved, the struct is partially moved and cannot be used as a whole
    moved_fields: Set<Text>,
    /// Tuple indices that have been moved out (for tuple partial move tracking)
    /// When a tuple element is moved, the tuple is partially moved and cannot be used as a whole
    moved_indices: Set<usize>,
}

impl AffineTracker {
    /// Create a new affine tracker
    pub fn new() -> Self {
        Self {
            bindings: Map::new(),
            affine_types: Set::new(),
            linear_types: Set::new(),
            loop_depth: 0,
            pre_loop_bindings: Set::new(),
        }
    }

    /// Create a new affine tracker with stdlib types pre-registered.
    ///
    /// This is the recommended constructor for production use. It automatically
    /// registers heap-allocated stdlib types (Text, List, Map, etc.) as affine,
    /// eliminating the need for users to know which types are move-only.
    ///
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 2 (Implicit Affine для stdlib)
    ///
    /// # Example
    /// ```rust,ignore
    /// let tracker = AffineTracker::with_core();
    /// assert!(tracker.is_affine_type("Text"));
    /// assert!(tracker.is_affine_type("List"));
    /// ```
    pub fn with_core() -> Self {
        // No hardcoded stdlib types. All affine types are discovered from
        // `type affine` declarations in source code (user or core/ stdlib).
        Self::new()
    }

    /// Create a new scope-local affine tracker
    ///
    /// This creates a fresh tracker with empty bindings but preserves
    /// the set of registered affine and linear types. Used when entering a new
    /// function scope where outer variable bindings are not accessible.
    pub fn new_scope(&self) -> Self {
        Self {
            bindings: Map::new(),
            affine_types: self.affine_types.clone(),
            linear_types: self.linear_types.clone(),
            loop_depth: 0,
            pre_loop_bindings: Set::new(),
        }
    }

    /// Register a type as affine (at most once)
    pub fn register_affine_type(&mut self, type_name: impl Into<Text>) {
        self.affine_types.insert(type_name.into());
    }

    /// Register a type as linear (exactly once)
    ///
    /// Linear types must be consumed exactly once. Unused linear values
    /// cause a compile-time error.
    ///
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
    pub fn register_linear_type(&mut self, type_name: impl Into<Text>) {
        self.linear_types.insert(type_name.into());
    }

    /// Check if a type is linear
    pub fn is_linear_type(&self, type_name: &str) -> bool {
        self.linear_types.contains(&Text::from(type_name))
    }

    /// Get resource kind for a type by name
    pub fn get_resource_kind(&self, type_name: &str) -> ResourceKind {
        if self.linear_types.contains(&Text::from(type_name)) {
            ResourceKind::Linear
        } else if self.affine_types.contains(&Text::from(type_name)) {
            ResourceKind::Affine
        } else {
            ResourceKind::Copy
        }
    }

    /// Register all standard library types that are implicitly affine.
    ///
    /// Heap-allocated types are affine by default because they require cleanup.
    /// Register stdlib affine types.
    ///
    // REMOVED: register_stdlib_affine_types()
    // ARCHITECTURAL PRINCIPLE: The compiler must NEVER hardcode knowledge of stdlib types.
    // All affine types are discovered through `type affine` declarations in source code.
    // See project development guidelines for details.

    /// Check if a type is affine (at most once)
    pub fn is_affine_type(&self, type_name: &str) -> bool {
        self.affine_types.contains(&Text::from(type_name))
    }

    /// Check if a variable binding is tracked as affine (at-most-once usage)
    pub fn is_affine_binding(&self, name: &str) -> bool {
        self.bindings
            .get(&Text::from(name))
            .map(|b| b.resource_kind.is_at_most_once())
            .unwrap_or(false)
    }

    /// Check if a type is affine or linear (at-most-once semantics)
    pub fn is_type_affine(&self, ty: &Type) -> bool {
        self.get_type_resource_kind(ty).is_at_most_once()
    }

    /// Check if a type is linear (exactly-once semantics)
    pub fn is_type_linear(&self, ty: &Type) -> bool {
        self.get_type_resource_kind(ty).is_exactly_once()
    }

    /// Get the resource kind for a Type
    pub fn get_type_resource_kind(&self, ty: &Type) -> ResourceKind {
        match ty {
            Type::Named { path, .. } => {
                // Get the last segment of the path as the type name
                let type_name = path
                    .segments
                    .last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("");
                self.get_resource_kind(type_name)
            }
            _ => ResourceKind::Copy,
        }
    }

    /// Bind an affine or linear value
    ///
    /// Automatically determines resource kind from the type.
    pub fn bind(&mut self, name: impl Into<Text>, ty: Type, span: Span) {
        let name = name.into();
        let resource_kind = self.get_type_resource_kind(&ty);

        // Only track affine or linear types
        if resource_kind.is_at_most_once() {
            self.bindings.insert(
                name,
                AffineBinding {
                    ty,
                    resource_kind,
                    binding_span: span,
                    first_use: None,
                    is_consumed: false,
                    moved_fields: Set::new(),
                    moved_indices: Set::new(),
                },
            );
        }
    }

    /// Bind a value with explicit resource kind
    ///
    /// Used for containers or when resource kind is known.
    pub fn bind_with_kind(
        &mut self,
        name: impl Into<Text>,
        ty: Type,
        resource_kind: ResourceKind,
        span: Span,
    ) {
        let name = name.into();
        if resource_kind.is_at_most_once() {
            self.bindings.insert(
                name,
                AffineBinding {
                    ty,
                    resource_kind,
                    binding_span: span,
                    first_use: None,
                    is_consumed: false,
                    moved_fields: Set::new(),
                    moved_indices: Set::new(),
                },
            );
        }
    }

    /// Bind a value that contains affine fields (for partial move tracking)
    /// This is called by the type checker which handles cycle detection
    pub fn bind_container(&mut self, name: impl Into<Text>, ty: Type, span: Span) {
        let name = name.into();
        // Trust the caller (type checker) to only call this for types containing affine fields
        // Default to Affine for containers
        self.bindings.insert(
            name,
            AffineBinding {
                ty,
                resource_kind: ResourceKind::Affine,
                binding_span: span,
                first_use: None,
                is_consumed: false,
                moved_fields: Set::new(),
                moved_indices: Set::new(),
            },
        );
    }

    /// Enter a loop context
    ///
    /// Records all current bindings as "pre-loop" so we can detect
    /// when affine values from outer scope are used inside the loop.
    pub fn enter_loop(&mut self) {
        if self.loop_depth == 0 {
            // First loop level - record current bindings
            self.pre_loop_bindings = self.bindings.keys().cloned().collect();
        }
        self.loop_depth += 1;
    }

    /// Exit a loop context
    pub fn exit_loop(&mut self) {
        if self.loop_depth > 0 {
            self.loop_depth -= 1;
            if self.loop_depth == 0 {
                self.pre_loop_bindings.clear();
            }
        }
    }

    /// Check if we're currently inside a loop
    pub fn in_loop(&self) -> bool {
        self.loop_depth > 0
    }

    /// Record a use of an affine value
    ///
    /// Returns an error if the value was already consumed.
    /// Also returns an error if the value is from outer scope and we're in a loop.
    /// Also returns an error if any field has been moved out (partial move).
    pub fn use_value(&mut self, name: &str, span: Span) -> Result<(), TypeError> {
        if let Some(binding) = self.bindings.get_mut(&Text::from(name)) {
            if binding.is_consumed {
                // Value already consumed
                return Err(TypeError::MovedValueUsed {
                    name: name.to_text(),
                    moved_at: binding.first_use.unwrap_or(binding.binding_span),
                    used_at: span,
                });
            }

            // Check for partial move - if any field has been moved out, the whole struct
            // cannot be used as a value anymore
            if let Some(moved_field) = binding.moved_fields.iter().next().cloned() {
                return Err(TypeError::PartiallyMovedValue {
                    name: name.to_text(),
                    moved_field,
                    moved_at: binding.first_use.unwrap_or(binding.binding_span),
                    used_at: span,
                });
            }

            // Check for tuple partial move - if any tuple index has been moved out
            if let Some(&moved_index) = binding.moved_indices.iter().next() {
                return Err(TypeError::PartiallyMovedValue {
                    name: name.to_text(),
                    moved_field: Text::from(format!(".{}", moved_index)),
                    moved_at: binding.first_use.unwrap_or(binding.binding_span),
                    used_at: span,
                });
            }

            // Only apply move semantics to affine/linear types.
            // Copy types can be used any number of times.
            // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #copy-vs-affine-semantics
            if binding.resource_kind.allows_multiple_use() {
                // Copy type — allow unlimited use, just track first use
                if binding.first_use.is_none() {
                    binding.first_use = Some(span);
                }
                return Ok(());
            }

            // Check for use of outer-scope affine value in loop
            // This is an error because the loop might execute multiple times
            if self.loop_depth > 0 && self.pre_loop_bindings.contains(&Text::from(name)) {
                return Err(TypeError::AffineValueInLoop {
                    name: name.to_text(),
                    binding_span: binding.binding_span,
                    use_span: span,
                });
            }

            // Mark as consumed on first use (move semantics for affine/linear)
            binding.first_use = Some(span);
            binding.is_consumed = true;
        }
        Ok(())
    }

    /// Record a use of a field from a struct containing affine values
    ///
    /// This is called when accessing `container.field` where `field` is an affine type.
    /// The field is marked as moved, preventing subsequent use of the whole struct.
    pub fn use_field_value(&mut self, var_name: &str, field_name: &str, span: Span) -> Result<(), TypeError> {
        if let Some(binding) = self.bindings.get_mut(&Text::from(var_name)) {
            // Check if the whole struct has been consumed
            if binding.is_consumed {
                return Err(TypeError::MovedValueUsed {
                    name: var_name.to_text(),
                    moved_at: binding.first_use.unwrap_or(binding.binding_span),
                    used_at: span,
                });
            }

            // Check if this specific field has already been moved
            if binding.moved_fields.contains(&Text::from(field_name)) {
                return Err(TypeError::MovedValueUsed {
                    name: format!("{}.{}", var_name, field_name).to_text(),
                    moved_at: binding.first_use.unwrap_or(binding.binding_span),
                    used_at: span,
                });
            }

            // Check for use in loop
            if self.loop_depth > 0 && self.pre_loop_bindings.contains(&Text::from(var_name)) {
                return Err(TypeError::AffineValueInLoop {
                    name: format!("{}.{}", var_name, field_name).to_text(),
                    binding_span: binding.binding_span,
                    use_span: span,
                });
            }

            // Mark the field as moved and record the span
            binding.moved_fields.insert(Text::from(field_name));
            if binding.first_use.is_none() {
                binding.first_use = Some(span);
            }
        }
        Ok(())
    }

    /// Check if using a non-affine field from a partially moved struct is allowed
    ///
    /// Returns true if the access is to a non-moved, non-affine field.
    pub fn can_access_field(&self, var_name: &str, field_name: &str) -> bool {
        if let Some(binding) = self.bindings.get(&Text::from(var_name)) {
            // If whole struct is consumed, cannot access any field
            if binding.is_consumed {
                return false;
            }
            // Can access if this specific field wasn't moved
            !binding.moved_fields.contains(&Text::from(field_name))
        } else {
            // Not tracked - assume allowed
            true
        }
    }

    /// Reinitialize a field that was previously moved out
    ///
    /// This is called when a moved field is reassigned, making the struct "whole" again.
    /// After reinitialization, the field can be accessed again and the struct can be
    /// used as a whole (if no other fields are still moved).
    ///
    /// # Example
    ///
    /// ```verum
    /// let mut container = Container { resource: Resource { id: 42 }, name: "test" };
    /// let old_res = container.resource;  // Move out affine field
    /// container.resource = Resource { id: 99 };  // Reinitialize - struct is whole again
    /// let whole = container;  // Now valid!
    /// ```
    pub fn reinitialize_field(&mut self, var_name: &str, field_name: &str) {
        if let Some(binding) = self.bindings.get_mut(&Text::from(var_name)) {
            // Remove the field from moved_fields set
            binding.moved_fields.remove(&Text::from(field_name));

            // If no more moved fields, clear the first_use span since struct is whole again
            // (but only if the struct wasn't fully consumed)
            if binding.moved_fields.is_empty() && !binding.is_consumed {
                // Note: We don't clear first_use because the struct was still partially moved
                // at some point. This is for tracking purposes only.
            }
        }
    }

    /// Check if a struct has any moved fields (is partially moved)
    pub fn has_moved_fields(&self, var_name: &str) -> bool {
        if let Some(binding) = self.bindings.get(&Text::from(var_name)) {
            !binding.moved_fields.is_empty()
        } else {
            false
        }
    }

    // ==================== TUPLE INDEX TRACKING ====================

    /// Record a use of a tuple element from a tuple containing affine values
    ///
    /// This is called when accessing `tuple.N` where the element at index N is an affine type.
    /// The index is marked as moved, preventing subsequent use of the whole tuple.
    pub fn use_index_value(&mut self, var_name: &str, index: usize, span: Span) -> Result<(), TypeError> {
        if let Some(binding) = self.bindings.get_mut(&Text::from(var_name)) {
            // Check if the whole tuple has been consumed
            if binding.is_consumed {
                return Err(TypeError::MovedValueUsed {
                    name: var_name.to_text(),
                    moved_at: binding.first_use.unwrap_or(binding.binding_span),
                    used_at: span,
                });
            }

            // Check if this specific index has already been moved
            if binding.moved_indices.contains(&index) {
                return Err(TypeError::MovedValueUsed {
                    name: format!("{}.{}", var_name, index).to_text(),
                    moved_at: binding.first_use.unwrap_or(binding.binding_span),
                    used_at: span,
                });
            }

            // Check for use in loop
            if self.loop_depth > 0 && self.pre_loop_bindings.contains(&Text::from(var_name)) {
                return Err(TypeError::AffineValueInLoop {
                    name: format!("{}.{}", var_name, index).to_text(),
                    binding_span: binding.binding_span,
                    use_span: span,
                });
            }

            // Mark the index as moved and record the span
            binding.moved_indices.insert(index);
            if binding.first_use.is_none() {
                binding.first_use = Some(span);
            }
        }
        Ok(())
    }

    /// Check if using a non-affine tuple element from a partially moved tuple is allowed
    ///
    /// Returns true if the access is to a non-moved, non-affine element.
    pub fn can_access_index(&self, var_name: &str, index: usize) -> bool {
        if let Some(binding) = self.bindings.get(&Text::from(var_name)) {
            // If whole tuple is consumed, cannot access any element
            if binding.is_consumed {
                return false;
            }
            // Can access if this specific index wasn't moved
            !binding.moved_indices.contains(&index)
        } else {
            // Not tracked - assume allowed
            true
        }
    }

    /// Reinitialize a tuple element that was previously moved out
    ///
    /// This is called when a moved tuple element is reassigned.
    pub fn reinitialize_index(&mut self, var_name: &str, index: usize) {
        if let Some(binding) = self.bindings.get_mut(&Text::from(var_name)) {
            binding.moved_indices.remove(&index);
        }
    }

    /// Check if a tuple has any moved indices (is partially moved)
    pub fn has_moved_indices(&self, var_name: &str) -> bool {
        if let Some(binding) = self.bindings.get(&Text::from(var_name)) {
            !binding.moved_indices.is_empty()
        } else {
            false
        }
    }

    /// Borrow an affine value (does not consume)
    ///
    /// Allowed for immutable borrows that don't move the value.
    pub fn borrow_value(&mut self, name: &str, _span: Span) -> Result<(), TypeError> {
        // Borrowing is allowed as long as the value wasn't consumed
        if let Some(binding) = self.bindings.get(&Text::from(name))
            && binding.is_consumed
        {
            return Err(TypeError::MovedValueUsed {
                name: name.to_text(),
                moved_at: binding.first_use.unwrap_or(binding.binding_span),
                used_at: _span,
            });
        }
        Ok(())
    }

    /// Remove a binding (e.g., when leaving scope)
    ///
    /// This triggers cleanup for unconsumed affine values.
    pub fn unbind(&mut self, name: &str) -> Maybe<bool> {
        self.bindings
            .remove(&Text::from(name))
            .map(|b| b.is_consumed)
    }

    /// Check for unconsumed linear values at scope end.
    ///
    /// Linear values must be consumed exactly once. This method returns errors
    /// for any linear values that were not consumed when leaving scope.
    ///
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
    ///
    /// # Arguments
    /// * `scope_end` - The span representing the end of the scope
    ///
    /// # Returns
    /// A list of errors for each unconsumed linear value
    pub fn check_linear_consumed(&self, scope_end: Span) -> List<TypeError> {
        self.bindings
            .iter()
            .filter(|(_, b)| b.resource_kind == ResourceKind::Linear && !b.is_consumed)
            .map(|(name, b)| TypeError::LinearNotConsumed {
                name: name.clone(),
                binding_span: b.binding_span,
                scope_end,
            })
            .collect()
    }

    /// Check if a binding is linear
    pub fn is_binding_linear(&self, name: &str) -> bool {
        self.bindings
            .get(&Text::from(name))
            .map(|b| b.resource_kind == ResourceKind::Linear)
            .unwrap_or(false)
    }

    /// Get the resource kind of a binding
    pub fn get_binding_resource_kind(&self, name: &str) -> Option<ResourceKind> {
        self.bindings
            .get(&Text::from(name))
            .map(|b| b.resource_kind)
    }

    /// Check if a value exists and is still available (not consumed)
    pub fn is_available(&self, name: &str) -> bool {
        self.bindings
            .get(&Text::from(name))
            .map(|b| !b.is_consumed)
            .unwrap_or(false)
    }

    /// Create a new scope (copy current bindings)
    pub fn enter_scope(&self) -> Self {
        self.clone()
    }

    /// Merge bindings from a branch (for control flow)
    ///
    /// After an if-expression or match, we need to merge the affine states
    /// from all branches. A value is consumed if it's consumed in ALL branches.
    pub fn merge_branch(&mut self, other: &AffineTracker) {
        // For each binding in self, check if it's also consumed in other
        for (name, binding) in self.bindings.iter_mut() {
            if let Some(other_binding) = other.bindings.get(name) {
                // Only keep as consumed if consumed in BOTH branches
                binding.is_consumed = binding.is_consumed && other_binding.is_consumed;
            } else {
                // If not in other branch, it's not consistently consumed
                binding.is_consumed = false;
            }
        }
    }
}

impl Default for AffineTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to check if a type declaration has a resource modifier (affine or linear).
///
/// Returns true if the modifier indicates at-most-once or exactly-once semantics.
pub fn check_resource_modifier(modifier: &Option<ResourceModifier>) -> bool {
    matches!(
        modifier,
        Some(ResourceModifier::Affine) | Some(ResourceModifier::Linear)
    )
}

/// Helper to check if a type declaration has a linear resource modifier.
///
/// Returns true only for exactly-once (linear) types.
pub fn check_linear_modifier(modifier: &Option<ResourceModifier>) -> bool {
    matches!(modifier, Some(ResourceModifier::Linear))
}

// Tests moved to tests/affine_module_tests.rs per project testing guidelines.
