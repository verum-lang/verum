//! Type unification algorithm.
//!
//! This module implements Robinson's unification algorithm with extensions for:
//! - Refinement types (structural unification)
//! - Occurs check (prevent infinite types)
//! - Substitution composition
//! - Dependent types (Pi, Sigma, Eq, Universe)
//! - Inductive and coinductive types
//! - Higher inductive types (HITs)
//! - Quantitative type theory
//!
//! Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking

use crate::ty::{EqTerm, Quantity, Substitution, SubstitutionExt, Type, TypeVar, UniverseLevel};
use crate::{Result, TypeError};
use indexmap::IndexMap;
use verum_ast::span::Span;
#[allow(unused_imports)]
use verum_common::Text;
use verum_common::ToText;
use verum_common::{List, Map};

/// Unifier performs type unification.
///
/// CRITICAL: The Unifier now maintains an accumulated substitution that
/// grows with each unification. This is essential for proper Hindley-Milner
/// type inference - type variables resolved in earlier unifications must
/// be reflected in later type comparisons.
///
/// Hindley-Milner type inference: algorithm W with let-polymorphism, constraint collection, and unification
pub struct Unifier {
    /// Count of unifications performed (for metrics)
    pub unify_count: usize,
    /// Current recursion depth for unify_inner (prevents stack overflow)
    unify_depth: u32,
    /// Accumulated substitution from all unifications
    /// This is composed with each new unification result
    substitution: Substitution,
    /// Registry mapping variant signatures to type names for stdlib-agnostic unification.
    /// Key is a stable string signature of the variant structure (sorted variant names),
    /// Value is the nominal type name (e.g., "Maybe", "Result").
    /// This enables Generic<->Variant unification without hardcoding stdlib types.
    variant_type_names: Map<Text, Text>,
    /// Original (unsubstituted) variant type definitions.
    /// Key is the type name, value is the original Type::Variant with TypeVars.
    /// Used for stdlib-agnostic Generic<->Variant unification to properly map
    /// type arguments to their corresponding positions in the variant structure.
    original_variant_types: Map<Text, Type>,
    /// Type variable orders for each variant type.
    /// Key is the type name, value is the list of TypeVars in declaration order.
    /// For `type Validated<E, A>`, this stores [E_typevar, A_typevar].
    type_var_orders: Map<Text, List<TypeVar>>,
    /// Type alias registry: maps alias names to their target types.
    /// E.g., "Byte" -> Named("UInt8"), enabling transparent unification
    /// through aliases like `type Byte is UInt8;`.
    type_aliases: Map<Text, Type>,
    /// Type parameter names for generic type aliases, in declaration order.
    /// E.g., "IoResult" -> ["T"] for `type IoResult<T> is Result<T, StreamError>`.
    /// Used by try_expand_alias to substitute type arguments into generic aliases.
    type_alias_params: Map<Text, List<Text>>,
    /// Data-driven set of collection type names that support array coercion.
    /// Array literals `[1, 2, 3]` can unify with any `C<T>` where `C` is in this set.
    /// Defaults to `{"List"}`. Register additional types via `register_array_coercible_type`.
    /// This replaces hardcoded `name == "List"` checks and will be superseded by a
    /// `FromArray` protocol check once protocols are available.
    array_coercible_types: std::collections::HashSet<Text>,
    /// Data-driven set of type names in the "tensor family" that coerce freely with each
    /// other and with scalar primitives (Float, Int, Bool).
    /// Defaults to `{"DynTensor", "Tensor", "Vector", "Cotangent", "Tangent"}`.
    /// Will be superseded by a `TensorLike` or `Numeric` protocol check once protocols
    /// are available in unification.
    tensor_family_types: std::collections::HashSet<Text>,
    /// Data-driven set of Named type names that coerce bidirectionally with `Int`.
    /// Includes sized integer aliases, FFI integer newtypes, tensor types, and
    /// collection types that participate in index/length coercions.
    /// Will be superseded by structural checks (e.g., `IntCoercible` protocol or
    /// querying type declarations for integer newtype wrappers).
    int_coercible_named_types: std::collections::HashSet<Text>,
    /// Data-driven set of collection type names whose Generic form coerces with `Int`
    /// (e.g., `List<USize>` vs `Int` from indexing operations).
    /// Will be superseded by an `Indexable` protocol check.
    indexable_collection_types: std::collections::HashSet<Text>,
    /// Data-driven set of range-like type names whose Generic form coerces with `Tuple`
    /// (e.g., `Range<Int>` vs `(Maybe<USize>, Maybe<USize>)` from slicing).
    /// Will be superseded by a `RangeLike` protocol check.
    range_like_types: std::collections::HashSet<Text>,
    /// Data-driven set of sized numeric type names that coerce with each other
    /// in Named<->Named cross-coercion (e.g., USize ↔ UInt64, Int32 ↔ I32).
    /// These are arguably part of the language definition (not stdlib), but are
    /// still centralized here to avoid scattered string literals.
    sized_numeric_types: std::collections::HashSet<Text>,
    /// Current Self type for implement blocks.
    /// When set, Type::Named("Self") is treated as equivalent to this type during unification.
    /// This prevents "expected 'Foo', found 'Self'" errors when Self leaks into type comparisons.
    self_type: Option<Type>,
    /// Context variable bindings from context unification.
    /// Maps context type variables to their resolved ContextExpr values.
    context_bindings: IndexMap<TypeVar, crate::di::requirement::ContextExpr>,
}

impl Unifier {
    pub fn new() -> Self {
        let mut array_coercible_types = std::collections::HashSet::new();
        // "List" is the default array-coercible collection type.
        // Additional types can be registered via register_array_coercible_type().
        array_coercible_types.insert(Text::from("List"));

        // Tensor family: types that coerce freely with each other and with scalars.
        // TODO: Replace with TensorLike protocol check once protocol-based unification
        // is available. These types should implement `TensorLike` in the stdlib.
        let tensor_family_types: std::collections::HashSet<Text> = [
            "DynTensor", "Tensor", "Vector", "Cotangent", "Tangent",
        ].iter().map(|s| Text::from(*s)).collect();

        // Named types that coerce bidirectionally with Int.
        // Categorized: sized integers, FFI newtypes, tensor types, collection types.
        // TODO: Replace with structural checks — query type declarations for integer
        // newtype wrappers or an IntCoercible protocol.
        let int_coercible_named_types: std::collections::HashSet<Text> = [
            // Sized integer types (core language, arguably not stdlib)
            "UInt", "UInt8", "UInt16", "UInt32", "UInt64", "UInt128",
            "Int8", "Int16", "Int32", "Int64", "Int128",
            "U8", "U16", "U32", "U64", "I8", "I16", "I32", "I64",
            "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64",
            "Byte", "usize", "isize", "UIntSize", "USize", "IntSize", "ISize",
            // FFI / OS newtypes
            "Port", "FileDesc", "MachPort", "VmAddress", "VmSize",
            "Timespec", "TimeSpec", "ClockId",
            "MemProt", "MapFlags", "Sockaddr", "Path", "PathBuf",
            "GPUBuffer", "DeviceRegistry", "ProcessGroup",
            "Duration", "Instant", "Epoch",
            // Tensor family
            "DynTensor", "Tensor", "Vector",
            // Collection / wrapper types used in index coercions
            "List", "Range", "Slice", "Maybe", "Lazy", "Once",
        ].iter().map(|s| Text::from(*s)).collect();

        // Collection types whose Generic form coerces with Int (indexing).
        // TODO: Replace with Indexable protocol check.
        let indexable_collection_types: std::collections::HashSet<Text> = [
            "List", "Range", "Slice",
        ].iter().map(|s| Text::from(*s)).collect();

        // Range-like types whose Generic form coerces with Tuple (slicing).
        // TODO: Replace with RangeLike protocol check.
        let range_like_types: std::collections::HashSet<Text> = [
            "Range",
        ].iter().map(|s| Text::from(*s)).collect();

        // Sized numeric types that cross-coerce with each other (Named<->Named).
        // These are arguably language-level (not stdlib), but centralized here.
        let sized_numeric_types: std::collections::HashSet<Text> = [
            "UInt", "UInt8", "UInt16", "UInt32", "UInt64", "UInt128",
            "Int8", "Int16", "Int32", "Int64", "Int128",
            "U8", "U16", "U32", "U64", "I8", "I16", "I32", "I64",
            "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64",
            "Byte", "usize", "isize", "UIntSize", "USize", "IntSize", "ISize",
            "Float32", "Float64", "f32", "f64",
            "Duration", "Instant", "Epoch",
        ].iter().map(|s| Text::from(*s)).collect();

        Self {
            unify_count: 0,
            unify_depth: 0,
            substitution: Substitution::new(),
            variant_type_names: Map::new(),
            original_variant_types: Map::new(),
            type_var_orders: Map::new(),
            type_aliases: Map::new(),
            type_alias_params: Map::new(),
            array_coercible_types,
            tensor_family_types,
            int_coercible_named_types,
            indexable_collection_types,
            range_like_types,
            sized_numeric_types,
            self_type: None,
            context_bindings: IndexMap::new(),
        }
    }

    /// Set the current Self type for implement block unification.
    /// When set, Type::Named("Self") or Type::Generic("Self") will be resolved
    /// to this concrete type during unification.
    pub fn set_self_type(&mut self, self_type: Option<Type>) {
        self.self_type = self_type;
    }

    /// If the given type is the symbolic "Self" type, resolve it to the concrete self_type.
    /// Returns the type unchanged if it's not Self or no self_type is set.
    fn resolve_self(&self, ty: &Type) -> Option<Type> {
        self.self_type.as_ref()?;
        match ty {
            Type::Named { path, args } if args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == "Self" {
                        return self.self_type.clone();
                    }
                }
                None
            }
            Type::Generic { name, args } if args.is_empty() && name.as_str() == "Self" => {
                self.self_type.clone()
            }
            _ => None,
        }
    }

    /// Register a collection type name that supports array literal coercion.
    /// Array literals `[1, 2, 3]` will unify with `TypeName<T>` for any registered name.
    /// "List" is registered by default.
    pub fn register_array_coercible_type(&mut self, type_name: Text) {
        self.array_coercible_types.insert(type_name);
    }

    /// Check whether a type name supports array coercion (data-driven).
    fn is_array_coercible(&self, name: &str) -> bool {
        self.array_coercible_types.contains(name)
    }

    /// Register a type name as part of the tensor family (coerces with other tensor
    /// types and with scalar primitives). Default set includes DynTensor, Tensor, etc.
    pub fn register_tensor_family_type(&mut self, type_name: Text) {
        self.tensor_family_types.insert(type_name);
    }

    /// Check whether a type name is in the tensor family (data-driven).
    /// Replaces hardcoded `name.contains("Tensor") || name.contains("Vector")` checks.
    fn is_tensor_family(&self, name: &str) -> bool {
        self.tensor_family_types.contains(name)
    }

    /// Register a Named type name that coerces bidirectionally with `Int`.
    pub fn register_int_coercible_type(&mut self, type_name: Text) {
        self.int_coercible_named_types.insert(type_name);
    }

    /// Check whether a Named type name coerces bidirectionally with `Int` (data-driven).
    fn is_int_coercible_named(&self, name: &str) -> bool {
        self.int_coercible_named_types.contains(name)
    }

    /// Check whether a Generic collection type coerces with `Int` (indexing).
    fn is_indexable_collection(&self, name: &str) -> bool {
        self.indexable_collection_types.contains(name)
    }

    /// Check whether a Generic type coerces with `Tuple` (range/slice pattern).
    fn is_range_like(&self, name: &str) -> bool {
        self.range_like_types.contains(name)
    }

    /// Check whether a Named type is a sized numeric that cross-coerces with other sized numerics.
    fn is_sized_numeric(&self, name: &str) -> bool {
        self.sized_numeric_types.contains(name)
    }

    /// Set the variant type names registry for stdlib-agnostic unification.
    /// This should be called when initializing the unifier with type context data.
    pub fn set_variant_type_names(&mut self, registry: Map<Text, Text>) {
        self.variant_type_names = registry;
    }

    /// Register a variant type name mapping.
    pub fn register_variant_type_name(&mut self, signature: Text, type_name: Text) {
        // Use entry().or_insert to keep FIRST registration (stdlib types registered first
        // should take precedence). This ensures deterministic variant resolution
        // regardless of module registration order.
        self.variant_type_names.entry(signature).or_insert(type_name);
    }

    /// Register the original (unsubstituted) variant type definition.
    /// This is needed for Generic<->Variant unification to properly map type arguments.
    pub fn register_original_variant_type(&mut self, type_name: Text, original_type: Type) {
        self.original_variant_types.insert(type_name, original_type);
    }

    /// Register the type variable order for a variant type.
    /// This maps type name to the list of TypeVars in declaration order.
    pub fn register_type_var_order(&mut self, type_name: Text, order: List<TypeVar>) {
        self.type_var_orders.insert(type_name, order);
    }

    /// Register a type alias mapping (e.g., "Byte" -> UInt8 Named type).
    pub fn register_type_alias(&mut self, alias_name: Text, target: Type) {
        self.type_aliases.insert(alias_name, target);
    }

    /// Register type parameter names for a generic type alias.
    /// E.g., for `type IoResult<T> is Result<T, StreamError>`, register ["T"].
    /// This enables try_expand_alias to substitute type arguments for generic aliases.
    pub fn register_type_alias_params(&mut self, alias_name: Text, params: List<Text>) {
        self.type_alias_params.insert(alias_name, params);
    }

    /// Remove a type alias (used when user types override stdlib aliases)
    pub fn remove_type_alias(&mut self, alias_name: &str) {
        let key: Text = alias_name.into();
        self.type_aliases.remove(&key);
    }

    /// Recursively expand type aliases in a type.
    /// Handles direct aliases (Byte -> UInt8), generic types (List<Byte> -> List<UInt8>),
    /// and reference types (&mut List<Byte> -> &mut List<UInt8>).
    pub fn try_expand_alias(&self, ty: &Type) -> Option<Type> {
        self.try_expand_alias_impl(ty, 0)
    }

    fn try_expand_alias_impl(&self, ty: &Type, depth: usize) -> Option<Type> {
        const MAX_ALIAS_EXPANSION_DEPTH: usize = 30;
        if depth > MAX_ALIAS_EXPANSION_DEPTH {
            return None;
        }
        match ty {
            // Direct Named type alias (e.g., Byte -> UInt8)
            Type::Named { path, args } => {
                let type_name = path.segments.last().and_then(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                        _ => None,
                    }
                });
                if let Some(name) = type_name {
                    let key = Text::from(name);
                    if let Some(target) = self.type_aliases.get(&key) {
                        if args.is_empty() {
                            // Simple alias: Byte -> UInt8
                            return Some(target.clone());
                        }
                        // Generic alias: IoResult<Text> -> Result<Text, StreamError>
                        // Substitute positional type arguments into the alias target.
                        if let Some(param_names) = self.type_alias_params.get(&key) {
                            if !param_names.is_empty() && args.len() <= param_names.len() {
                                let substituted = self.substitute_alias_params(
                                    target, param_names, args,
                                );
                                return Some(substituted);
                            }
                        }
                    }
                }
                // Try expanding aliases in type arguments (e.g., List<Byte> -> List<UInt8>)
                let mut changed = false;
                let d = depth + 1;
                let new_args: List<Type> = args.iter().map(|arg| {
                    if let Some(expanded) = self.try_expand_alias_impl(arg, d) {
                        changed = true;
                        expanded
                    } else {
                        arg.clone()
                    }
                }).collect();
                if changed {
                    Some(Type::Named { path: path.clone(), args: new_args })
                } else {
                    None
                }
            }
            // Expand through references: &T, &mut T, &checked T, etc.
            Type::Reference { inner, mutable } => {
                self.try_expand_alias_impl(inner, depth + 1).map(|expanded| {
                    Type::Reference { inner: Box::new(expanded), mutable: *mutable }
                })
            }
            Type::CheckedReference { inner, mutable } => {
                self.try_expand_alias_impl(inner, depth + 1).map(|expanded| {
                    Type::CheckedReference { inner: Box::new(expanded), mutable: *mutable }
                })
            }
            Type::Ownership { inner, mutable } => {
                self.try_expand_alias_impl(inner, depth + 1).map(|expanded| {
                    Type::Ownership { inner: Box::new(expanded), mutable: *mutable }
                })
            }
            // Handle Generic type aliases (e.g., IoResult<Text> as Generic { name: "IoResult", args: [Text] })
            Type::Generic { name, args } => {
                let key = name.clone();
                if let Some(target) = self.type_aliases.get(&key) {
                    if args.is_empty() {
                        return Some(target.clone());
                    }
                    if let Some(param_names) = self.type_alias_params.get(&key) {
                        if !param_names.is_empty() && args.len() <= param_names.len() {
                            let substituted = self.substitute_alias_params(
                                target, param_names, args,
                            );
                            return Some(substituted);
                        }
                    }
                }
                // Try expanding aliases in type arguments
                let mut changed = false;
                let d = depth + 1;
                let new_args: List<Type> = args.iter().map(|arg| {
                    if let Some(expanded) = self.try_expand_alias_impl(arg, d) {
                        changed = true;
                        expanded
                    } else {
                        arg.clone()
                    }
                }).collect();
                if changed {
                    Some(Type::Generic { name: name.clone(), args: new_args })
                } else {
                    None
                }
            }
            // Expand through tuples
            Type::Tuple(elements) => {
                let mut changed = false;
                let d = depth + 1;
                let new_elements: List<Type> = elements.iter().map(|el| {
                    if let Some(expanded) = self.try_expand_alias_impl(el, d) {
                        changed = true;
                        expanded
                    } else {
                        el.clone()
                    }
                }).collect();
                if changed { Some(Type::Tuple(new_elements)) } else { None }
            }
            _ => None,
        }
    }

    /// Substitute type parameter names in an alias target with concrete type arguments.
    /// For `type IoResult<T> is Result<T, StreamError>`, given args=[Text],
    /// replaces all occurrences of type variable "T" with Text in the target type.
    fn substitute_alias_params(&self, target: &Type, param_names: &[Text], args: &[Type]) -> Type {
        // Build a name->type substitution map
        let subst_map: Map<Text, Type> = param_names.iter()
            .zip(args.iter())
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect();

        self.apply_alias_subst(target, &subst_map)
    }

    /// Recursively apply a name-based substitution to a type.
    fn apply_alias_subst(&self, ty: &Type, subst: &Map<Text, Type>) -> Type {
        self.apply_alias_subst_impl(ty, subst, 0)
    }

    fn apply_alias_subst_impl(&self, ty: &Type, subst: &Map<Text, Type>, depth: usize) -> Type {
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return ty.clone();
        }
        let d = depth + 1;
        match ty {
            // Type variables with matching names get substituted
            Type::Var(tv) => {
                let name = Text::from(format!("{}", tv));
                if let Some(replacement) = subst.get(&name) {
                    return replacement.clone();
                }
                ty.clone()
            }
            // Generic types: check if the name matches a parameter
            Type::Generic { name, args } => {
                if args.is_empty() {
                    if let Some(replacement) = subst.get(name) {
                        return replacement.clone();
                    }
                }
                let new_args: List<Type> = args.iter()
                    .map(|a| self.apply_alias_subst_impl(a, subst, d))
                    .collect();
                Type::Generic { name: name.clone(), args: new_args }
            }
            // Named types: recurse into args
            Type::Named { path, args } => {
                // Check if this is a single-segment path that matches a param name
                if args.is_empty() {
                    if let Some(ident) = path.as_ident() {
                        if let Some(replacement) = subst.get(&ident.name) {
                            return replacement.clone();
                        }
                    }
                }
                let new_args: List<Type> = args.iter()
                    .map(|a| self.apply_alias_subst_impl(a, subst, d))
                    .collect();
                Type::Named { path: path.clone(), args: new_args }
            }
            // Variant types: recurse into payloads
            Type::Variant(variants) => {
                let new_variants: indexmap::IndexMap<Text, Type> = variants.iter()
                    .map(|(k, v)| (k.clone(), self.apply_alias_subst_impl(v, subst, d)))
                    .collect();
                Type::Variant(new_variants)
            }
            // Record types: recurse into fields
            Type::Record(fields) => {
                let new_fields: indexmap::IndexMap<Text, Type> = fields.iter()
                    .map(|(k, v)| (k.clone(), self.apply_alias_subst_impl(v, subst, d)))
                    .collect();
                Type::Record(new_fields)
            }
            // Function types: recurse into params and return
            Type::Function { params, return_type, contexts, properties, type_params } => {
                let new_params: List<Type> = params.iter()
                    .map(|p| self.apply_alias_subst_impl(p, subst, d))
                    .collect();
                let new_ret = Box::new(self.apply_alias_subst_impl(return_type, subst, d));
                Type::Function {
                    params: new_params,
                    return_type: new_ret,
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                    type_params: type_params.clone(),
                }
            }
            // Reference types: recurse into inner
            Type::Reference { inner, mutable } => {
                Type::Reference {
                    inner: Box::new(self.apply_alias_subst_impl(inner, subst, d)),
                    mutable: *mutable,
                }
            }
            Type::CheckedReference { inner, mutable } => {
                Type::CheckedReference {
                    inner: Box::new(self.apply_alias_subst_impl(inner, subst, d)),
                    mutable: *mutable,
                }
            }
            Type::Ownership { inner, mutable } => {
                Type::Ownership {
                    inner: Box::new(self.apply_alias_subst_impl(inner, subst, d)),
                    mutable: *mutable,
                }
            }
            // Tuple types: recurse
            Type::Tuple(elements) => {
                let new_elements: List<Type> = elements.iter()
                    .map(|e| self.apply_alias_subst_impl(e, subst, d))
                    .collect();
                Type::Tuple(new_elements)
            }
            // Array types: recurse into element type
            Type::Array { element, size } => {
                Type::Array {
                    element: Box::new(self.apply_alias_subst_impl(element, subst, d)),
                    size: *size,
                }
            }
            // Future types: recurse into output
            Type::Future { output } => {
                Type::Future {
                    output: Box::new(self.apply_alias_subst_impl(output, subst, d)),
                }
            }
            // All other types: return as-is
            _ => ty.clone(),
        }
    }

    /// Generate a stable signature for a variant type (sorted variant names with payload types).
    ///
    /// IMPORTANT: Must produce identical signatures to `variant_type_signature()` in infer.rs
    /// and `variant_type_signature_static()` in protocol.rs.
    fn variant_type_signature(variants: &IndexMap<Text, Type>) -> Text {
        let mut entries: Vec<String> = variants
            .iter()
            .map(|(name, payload)| {
                let payload_name = match payload {
                    Type::Named { path, .. } => {
                        path.as_ident()
                            .map(|id| id.name.as_str().to_string())
                            .unwrap_or_default()
                    }
                    Type::Generic { name: n, .. } => n.as_str().to_string(),
                    // Unit, primitives, and TypeVars are not distinctive for
                    // disambiguation — only Named/Generic payload types matter.
                    _ => String::new(),
                };
                if payload_name.is_empty() {
                    name.as_str().to_string()
                } else {
                    format!("{}({})", name.as_str(), payload_name)
                }
            })
            .collect();
        entries.sort();
        format!("Variant({})", entries.join("|")).into()
    }

    /// Generate a relaxed variant type signature using only variant names (ignoring payload types).
    /// Used as fallback when the full signature doesn't match due to concrete type arguments.
    fn variant_type_signature_relaxed(variants: &IndexMap<Text, Type>) -> Text {
        let mut names: Vec<&str> = variants.keys().map(|k| k.as_str()).collect();
        names.sort();
        format!("Variant({})", names.join("|")).into()
    }

    /// Recursively extract TypeVar -> Type mappings by matching original and substituted types.
    /// Static version for use in unification (doesn't need self).
    fn extract_type_var_mapping_static(
        original: &Type,
        substituted: &Type,
        mapping: &mut indexmap::IndexMap<TypeVar, Type>,
    ) {
        match (original, substituted) {
            (Type::Var(tv), concrete) => {
                mapping.entry(*tv).or_insert_with(|| concrete.clone());
            }
            (Type::Generic { args: orig_args, .. }, Type::Generic { args: subst_args, .. })
            | (Type::Named { args: orig_args, .. }, Type::Named { args: subst_args, .. }) => {
                for (orig_arg, subst_arg) in orig_args.iter().zip(subst_args.iter()) {
                    Self::extract_type_var_mapping_static(orig_arg, subst_arg, mapping);
                }
            }
            (Type::Tuple(orig_elems), Type::Tuple(subst_elems)) => {
                for (orig_elem, subst_elem) in orig_elems.iter().zip(subst_elems.iter()) {
                    Self::extract_type_var_mapping_static(orig_elem, subst_elem, mapping);
                }
            }
            (Type::Record(orig_fields), Type::Record(subst_fields)) => {
                for (field_name, orig_ty) in orig_fields.iter() {
                    if let Some(subst_ty) = subst_fields.get(field_name) {
                        Self::extract_type_var_mapping_static(orig_ty, subst_ty, mapping);
                    }
                }
            }
            (
                Type::Function { params: orig_params, return_type: orig_ret, .. },
                Type::Function { params: subst_params, return_type: subst_ret, .. },
            ) => {
                for (orig_param, subst_param) in orig_params.iter().zip(subst_params.iter()) {
                    Self::extract_type_var_mapping_static(orig_param, subst_param, mapping);
                }
                Self::extract_type_var_mapping_static(orig_ret, subst_ret, mapping);
            }
            (Type::Reference { inner: orig_inner, .. }, Type::Reference { inner: subst_inner, .. })
            | (Type::CheckedReference { inner: orig_inner, .. }, Type::CheckedReference { inner: subst_inner, .. })
            | (Type::UnsafeReference { inner: orig_inner, .. }, Type::UnsafeReference { inner: subst_inner, .. }) => {
                Self::extract_type_var_mapping_static(orig_inner, subst_inner, mapping);
            }
            (Type::Variant(orig_variants), Type::Variant(subst_variants)) => {
                for (variant_name, orig_payload) in orig_variants.iter() {
                    if let Some(subst_payload) = subst_variants.get(variant_name) {
                        Self::extract_type_var_mapping_static(orig_payload, subst_payload, mapping);
                    }
                }
            }
            _ => {}
        }
    }

    /// Fallback for Generic<->Variant unification when type_var_order is not available.
    /// Uses the old payload-based approach (which may have incorrect ordering for some types).
    fn unify_generic_variant_fallback(
        &mut self,
        args: &List<Type>,
        variants: &IndexMap<Text, Type>,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let non_unit_payloads: Vec<&Type> = variants
            .values()
            .filter(|payload| **payload != Type::Unit)
            .collect();

                // #[cfg(debug_assertions)]
        // eprintln!("[DEBUG unify_generic_variant_fallback] non_unit_payloads={:?}", non_unit_payloads);

        // Number of non-Unit payloads should match number of type args
        if non_unit_payloads.len() != args.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        // Unify each type argument with corresponding payload
        let mut subst = Substitution::new();
        for (arg, payload) in args.iter().zip(non_unit_payloads.iter()) {
            let s = self.unify_inner(
                &arg.apply_subst(&subst),
                &payload.apply_subst(&subst),
                span,
            )?;
            subst = subst.compose(&s);
        }
        Ok(subst)
    }

    /// Get the current accumulated substitution
    pub fn get_substitution(&self) -> &Substitution {
        &self.substitution
    }

    /// Get context variable bindings accumulated during unification
    pub fn get_context_bindings(&self) -> &IndexMap<TypeVar, crate::di::requirement::ContextExpr> {
        &self.context_bindings
    }

    /// Apply the current accumulated substitution to a type,
    /// also resolving context variable bindings.
    pub fn apply(&self, ty: &Type) -> Type {
        let resolved = ty.apply_subst(&self.substitution);
        // Also resolve context variables in function types
        if !self.context_bindings.is_empty() {
            self.resolve_context_vars(&resolved)
        } else {
            resolved
        }
    }

    /// Save the current substitution state for tentative unification.
    /// Returns a snapshot that can be restored via `restore_substitution`.
    pub fn save_substitution(&self) -> Substitution {
        self.substitution.clone()
    }

    /// Restore a previously saved substitution state.
    /// Used to roll back tentative unification attempts.
    pub fn restore_substitution(&mut self, saved: Substitution) {
        self.substitution = saved;
    }

    /// Resolve context variables in a type using context_bindings
    fn resolve_context_vars(&self, ty: &Type) -> Type {
        match ty {
            Type::Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => {
                let resolved_contexts = contexts.as_ref().map(|ctx| {
                    ctx.apply_context_subst(&self.context_bindings)
                });
                Type::Function {
                    params: params.iter().map(|p| self.resolve_context_vars(p)).collect(),
                    return_type: Box::new(self.resolve_context_vars(return_type)),
                    type_params: type_params.clone(),
                    contexts: resolved_contexts,
                    properties: properties.clone(),
                }
            }
            _ => ty.clone(),
        }
    }

    /// Reset the accumulated substitution (e.g., for new function scope)
    pub fn reset_substitution(&mut self) {
        self.substitution = Substitution::new();
    }

    /// Helper to check if a path represents an array-coercible collection type.
    /// Uses the data-driven `array_coercible_types` set instead of hardcoding "List".
    /// Will be replaced with a `FromArray` protocol check once protocols are available.
    fn path_is_array_coercible(&self, path: &verum_ast::ty::Path) -> bool {
        if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
            self.is_array_coercible(ident.name.as_str())
        } else {
            false
        }
    }

    /// Create a constructor kind for N parameters: * -> * -> ... -> *
    /// For arity 0: Type (*)
    /// For arity 1: * -> * (unary constructor like List)
    /// For arity 2: * -> * -> * (binary constructor like Map)
    fn kind_for_arity(arity: usize) -> crate::advanced_protocols::Kind {
        use crate::advanced_protocols::Kind;
        let mut kind = Kind::Type;
        for _ in 0..arity {
            kind = Kind::arrow(Kind::Type, kind);
        }
        kind
    }

    /// Helper to collect all type variables from a type.
    /// Used for building substitutions when unifying Named types with Variant types.
    fn collect_type_vars_from_type(ty: &Type, vars: &mut Vec<TypeVar>) {
        use Type::*;
        match ty {
            Var(tv) => {
                if !vars.contains(tv) {
                    vars.push(*tv);
                }
            }
            Named { args, .. } | Generic { args, .. } => {
                for arg in args {
                    Self::collect_type_vars_from_type(arg, vars);
                }
            }
            Tuple(elems) => {
                for elem in elems {
                    Self::collect_type_vars_from_type(elem, vars);
                }
            }
            Function { params, return_type, .. } => {
                for param in params {
                    Self::collect_type_vars_from_type(param, vars);
                }
                Self::collect_type_vars_from_type(return_type, vars);
            }
            Reference { inner, .. } | CheckedReference { inner, .. } | UnsafeReference { inner, .. } => {
                Self::collect_type_vars_from_type(inner, vars);
            }
            Array { element, .. } | Slice { element } => {
                Self::collect_type_vars_from_type(element, vars);
            }
            Variant(variant_map) | Record(variant_map) => {
                for (_, payload) in variant_map {
                    Self::collect_type_vars_from_type(payload, vars);
                }
            }
            _ => {}
        }
    }

    /// Check if a type contains any unresolved type variables.
    ///
    /// This is used during associated type projection unification to determine
    /// whether a projection can be resolved or must be deferred.
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Associated type resolution
    fn has_type_vars(ty: &Type) -> bool {
        use Type::*;
        match ty {
            Var(_) => true,
            Named { args, .. } | Generic { args, .. } => args.iter().any(Self::has_type_vars),
            Tuple(elems) => elems.iter().any(Self::has_type_vars),
            Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(Self::has_type_vars) || Self::has_type_vars(return_type)
            }
            Reference { inner, .. }
            | CheckedReference { inner, .. }
            | UnsafeReference { inner, .. } => Self::has_type_vars(inner),
            Array { element, .. } | Slice { element } => Self::has_type_vars(element),
            Variant(map) | Record(map) => map.values().any(Self::has_type_vars),
            Future { output } => Self::has_type_vars(output),
            Generator {
                yield_ty,
                return_ty,
            } => Self::has_type_vars(yield_ty) || Self::has_type_vars(return_ty),
            Refined { base, .. } => Self::has_type_vars(base),
            Forall { body, .. } | Exists { body, .. } => Self::has_type_vars(body),
            // Primitive types have no type variables
            Unit | Never | Bool | Int | Float | Char | Text => false,
            // Others - conservative: assume no vars
            _ => false,
        }
    }

    /// Unify two context expressions directly (public API for context polymorphism).
    ///
    /// This is the entry point for callers (e.g., type inference) that need to unify
    /// context expressions from callback types without going through full function type
    /// unification. For example, when inferring `C` in:
    ///
    /// ```verum
    /// fn map<T, U, using C>(iter: I, f: fn(T) -> U using C) -> MapIter<T,U> using C
    /// ```
    ///
    /// The inference engine can call `unify_context_exprs(C_var, concrete_ctx)` to bind
    /// the context variable `C` to the concrete context requirement of the callback `f`.
    ///
    /// Context polymorphism rules:
    /// - Variable vs Concrete: binds variable to the concrete requirement
    /// - Variable vs Variable: binds one to the other (if distinct)
    /// - Concrete vs Concrete: must be equal
    pub fn unify_context_exprs(
        &mut self,
        a: &crate::di::requirement::ContextExpr,
        b: &crate::di::requirement::ContextExpr,
        span: Span,
    ) -> crate::Result<()> {
        use crate::di::requirement::ContextExpr;

        match (a, b) {
            // Variable binds to concrete
            (ContextExpr::Variable(var), concrete @ ContextExpr::Concrete(_)) |
            (concrete @ ContextExpr::Concrete(_), ContextExpr::Variable(var)) => {
                // Check for conflicting binding
                if let Some(existing) = self.context_bindings.get(var) {
                    if existing != concrete {
                        return Err(TypeError::ContextMismatch {
                            expected: Text::from(format!("{}", existing)),
                            actual: Text::from(format!("{}", concrete)),
                            span,
                        });
                    }
                }
                self.context_bindings.insert(*var, concrete.clone());
                Ok(())
            }

            // Both variables: bind first to second (if distinct)
            (ContextExpr::Variable(v1), ContextExpr::Variable(v2)) => {
                if v1 != v2 {
                    self.context_bindings.insert(*v1, ContextExpr::Variable(*v2));
                }
                Ok(())
            }

            // Both concrete: must be equal
            (ContextExpr::Concrete(req1), ContextExpr::Concrete(req2)) => {
                if req1 == req2 {
                    Ok(())
                } else {
                    Err(TypeError::ContextMismatch {
                        expected: Text::from(format!("using {}", req2)),
                        actual: Text::from(format!("using {}", req1)),
                        span,
                    })
                }
            }
        }
    }

    /// Unify optional context expressions for context polymorphism (internal).
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.5 - Context Polymorphism
    ///
    /// Rules:
    /// - None/None: compatible (both pure)
    /// - Concrete/Concrete: must be equal
    /// - Variable/Concrete: bind variable to the concrete requirement
    /// - Concrete/Variable: bind variable to the concrete requirement
    /// - Variable/Variable: unify the type variables
    fn unify_contexts(
        &mut self,
        ctx1: &Option<crate::di::requirement::ContextExpr>,
        ctx2: &Option<crate::di::requirement::ContextExpr>,
        span: verum_ast::span::Span,
    ) -> crate::Result<Substitution> {
        use crate::di::requirement::ContextExpr;

        match (ctx1, ctx2) {
            // Both None (pure): compatible
            (None, None) => Ok(Substitution::new()),

            // One None, one non-empty concrete: not compatible
            (None, Some(ContextExpr::Concrete(req))) if !req.is_empty() => {
                Err(TypeError::ContextMismatch {
                    expected: Text::from(format!("using {}", req)),
                    actual: Text::from("pure (no contexts)"),
                    span,
                })
            }
            (Some(ContextExpr::Concrete(req)), None) if !req.is_empty() => {
                Err(TypeError::ContextMismatch {
                    expected: Text::from("pure (no contexts)"),
                    actual: Text::from(format!("using {}", req)),
                    span,
                })
            }

            // None with empty concrete: compatible
            (None, Some(ContextExpr::Concrete(req))) if req.is_empty() => Ok(Substitution::new()),
            (Some(ContextExpr::Concrete(req)), None) if req.is_empty() => Ok(Substitution::new()),

            // None with variable: variable binds to empty (pure)
            (None, Some(ContextExpr::Variable(var))) => {
                use crate::di::requirement::ContextRequirement;
                self.context_bindings.insert(*var, ContextExpr::Concrete(ContextRequirement::empty()));
                Ok(Substitution::new())
            }
            (Some(ContextExpr::Variable(var)), None) => {
                use crate::di::requirement::ContextRequirement;
                self.context_bindings.insert(*var, ContextExpr::Concrete(ContextRequirement::empty()));
                Ok(Substitution::new())
            }

            // Both Some: delegate to unify_context_exprs
            (Some(a), Some(b)) => {
                self.unify_context_exprs(a, b, span)?;
                Ok(Substitution::new())
            }

            // Catch-all for any remaining cases
            _ => Ok(Substitution::new()),
        }
    }

    /// Unify two types, returning a substitution that makes them equal.
    ///
    /// This is the core of Hindley-Milner type inference.
    /// The algorithm finds the most general unifier (MGU) if one exists.
    ///
    /// CRITICAL FIX: The unifier now:
    /// 1. Applies the accumulated substitution to both types before unifying
    /// 2. Composes the new substitution with the accumulated one
    /// 3. Stores the composed substitution for future unifications
    ///
    /// This ensures that type variables resolved in earlier unifications are
    /// properly reflected in later type comparisons.
    pub fn unify(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        self.unify_count += 1;

        // Guard against stack overflow from recursive unification
        self.unify_depth += 1;
        if self.unify_depth > 200 {
            self.unify_depth -= 1;
            return Err(TypeError::Other(
                verum_common::Text::from(format!("unification depth exceeded ({})", self.unify_depth)),
            ));
        }
        let result = self.unify_impl(t1, t2, span);
        self.unify_depth -= 1;
        result
    }

    fn unify_impl(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        // Guard against infinite unification loops
        const MAX_UNIFY_CALLS: usize = 50_000;
        if self.unify_count > MAX_UNIFY_CALLS {
            return Err(TypeError::Other(
                verum_common::Text::from("type inference iteration limit exceeded (possible infinite loop)"),
            ));
        }

        // Apply current substitution to both types before unifying
        let t1_resolved = t1.apply_subst(&self.substitution);
        let t2_resolved = t2.apply_subst(&self.substitution);

        // Unify the resolved types, with type alias fallback.
        // If direct unification fails, try expanding type aliases on either
        // side and retry. This handles cases like `Byte` (alias for `UInt8`)
        // being used interchangeably with `UInt8`.
        let result = self.unify_inner(&t1_resolved, &t2_resolved, span);

        let new_subst = match result {
            Ok(s) => s,
            Err(_) if !self.type_aliases.is_empty() => {
                let t1_expanded = self.try_expand_alias(&t1_resolved);
                let t2_expanded = self.try_expand_alias(&t2_resolved);
                let t1_use = t1_expanded.as_ref().unwrap_or(&t1_resolved);
                let t2_use = t2_expanded.as_ref().unwrap_or(&t2_resolved);
                // Only retry if at least one side was expanded
                if t1_expanded.is_some() || t2_expanded.is_some() {
                    self.unify_inner(t1_use, t2_use, span)?
                } else {
                    return result;
                }
            }
            Err(_) => return result,
        };

        // Compose with accumulated substitution
        self.substitution = self.substitution.compose(&new_subst);

        Ok(new_subst)
    }

    fn unify_inner(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        use Type::*;

        // Recursion depth guard to prevent stack overflow (RAII for safety on early returns)
        // Each recursive unify_inner_impl frame is ~2500 lines of match arms,
        // consuming significant stack. With 64MB thread stacks, limit to 50.
        const MAX_UNIFY_DEPTH: u32 = 50;
        self.unify_depth += 1;
        if self.unify_depth > MAX_UNIFY_DEPTH {
            self.unify_depth -= 1;
            return Err(TypeError::Other(
                verum_common::Text::from("type inference recursion limit exceeded"),
            ));
        }
        let result = self.unify_inner_impl(t1, t2, span);
        self.unify_depth = self.unify_depth.saturating_sub(1);
        result
    }

    fn unify_inner_impl(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        use Type::*;

        // CRITICAL FIX: Resolve symbolic "Self" types before unification.
        // In implement blocks, Self may leak into types that reach the unifier.
        // Replace Self with the concrete implementing type to prevent
        // "expected 'Foo', found 'Self'" mismatches.
        // Route through unify_inner to maintain depth tracking.
        if let Some(resolved) = self.resolve_self(t1) {
            return self.unify_inner(&resolved, t2, span);
        }
        if let Some(resolved) = self.resolve_self(t2) {
            return self.unify_inner(t1, &resolved, span);
        }

        // PROOF IRRELEVANCE (Inductive types: recursive type definitions with structural recursion, termination checking — .1)
        // If both types have type Prop, they unify regardless of their structure.
        // All proofs of a proposition are equal.
        if self.is_type_in_prop(t1) && self.is_type_in_prop(t2) {
            return Ok(Substitution::new());
        }

        match (t1, t2) {
            // Never type (bottom type) unifies with any type
            // This allows diverging control flow (return, break, continue) to work in any context
            (Never, _) | (_, Never) => Ok(Substitution::new()),

            // Unknown type (top type) - any value can be assigned to unknown
            // Unknown is a supertype of all types, so T -> Unknown is valid
            // But Unknown -> T requires explicit narrowing (is-check)
            // In unification context, we allow it to avoid false positives
            // since the type checker handles narrowing elsewhere
            (Unknown, _) | (_, Unknown) => Ok(Substitution::new()),

            // Same types
            (Unit, Unit)
            | (Bool, Bool)
            | (Int, Int)
            | (Float, Float)
            | (Char, Char)
            | (Text, Text) => Ok(Substitution::new()),

            // Numeric widening: Int → Float (safe, no precision loss for small ints)
            // This allows `fn f(x: Float) { ... }; f(42)` without explicit cast.
            (Int, Float) | (Float, Int) => Ok(Substitution::new()),

            // Char → Text widening: a single character can be used where text is expected
            // This allows `let s: Text = 'a';` without explicit conversion.
            (Char, Text) | (Text, Char) => Ok(Substitution::new()),

            // Text and Int are distinct types in Verum. No implicit coercion.
            // Use explicit conversion: `text.parse_int()` or `int.to_text()`.
            // (Text, Int) | (Int, Text) => intentionally NOT unified

            // Sized integer coercion: Int ↔ UInt32/UInt16/Int32/Int16/Int8/UInt8/UInt64
            // Verum's `Int` is the universal integer type (64-bit signed).
            // Sized integers (UInt32, UInt16, etc.) coerce bidirectionally with Int
            // to allow natural usage: `let epoch: UInt32 = 42;` or `assert(epoch == 0);`
            // Uses data-driven int_coercible_named_types set instead of hardcoded name list.
            // TODO: Replace with structural checks (query type declarations for integer
            // newtype wrappers) or an IntCoercible protocol.
            (Int, Named { path, .. }) | (Named { path, .. }, Int) => {
                let name = path.segments.last().map(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => "",
                }).unwrap_or("");
                if self.is_int_coercible_named(name) {
                    Ok(Substitution::new())
                } else {
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }

            // Unit and empty tuple are equivalent
            // () is parsed as empty Tuple but semantically equals Unit
            (Unit, Tuple(elements)) | (Tuple(elements), Unit) if elements.is_empty() => {
                Ok(Substitution::new())
            }

            // Unit newtype coercion: `type Foo is ()` makes Foo structurally Unit.
            // When a function returns `()` but the declared return type is a named
            // unit type (like `Database` which is `type Database is ()`), allow the
            // coercion. This is safe because unit newtypes carry no data.
            (Unit, Named { path, args, .. }) | (Named { path, args, .. }, Unit) if args.is_empty() => {
                let name = path.segments.last().map(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => "",
                }).unwrap_or("");
                let name_text = verum_common::Text::from(name);
                // Check type alias registry for unit newtypes
                if let Some(resolved) = self.type_aliases.get(&name_text) {
                    if *resolved == Type::Unit || matches!(resolved, Type::Tuple(elems) if elems.is_empty()) {
                        return Ok(Substitution::new());
                    }
                }
                // Check variant_type_names for registered unit types
                if self.variant_type_names.get(&verum_common::Text::from("()")).is_some() {
                    // If "()" is registered as a variant type name, check if it matches
                }
                Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                })
            }

            // Unresolved generic type parameters: Named types with single-letter names
            // like "T", "U", "E", "K", "V" etc. that haven't been substituted with
            // type variables. Allow them to unify with any concrete type.
            (Named { path, args, .. }, other) | (other, Named { path, args, .. })
                if args.is_empty() && {
                    let name = path.segments.last().map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    }).unwrap_or("");
                    name.len() <= 2
                        && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                        && !matches!(name, "Ok" | "No" | "Eq" | "Fn" | "IO")
                        && !matches!(other, Named { .. } if {
                            let oname = match other {
                                Named { path: op, .. } => op.segments.last().map(|s| match s {
                                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                                    _ => "",
                                }).unwrap_or(""),
                                _ => "",
                            };
                            oname.len() <= 2
                                && oname.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                        })
                } =>
            {
                Ok(Substitution::new())
            }

            // Type variables
            (Var(v1), Var(v2)) if v1 == v2 => Ok(Substitution::new()),

            (Var(v), ty) | (ty, Var(v)) => self.bind_var(*v, ty, span),

            // Function types
            (
                Function {
                    params: p1,
                    return_type: r1,
                    type_params: tp1,
                    contexts: c1,
                    properties: _,
                },
                Function {
                    params: p2,
                    return_type: r2,
                    type_params: tp2,
                    contexts: c2,
                    properties: _,
                },
            ) => {
                if p1.len() != p2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // Type parameters must match structurally (same count and names)
                // For unification, we require exact structural match of type parameters.
                // Subtyping of bounds is handled separately in the subtype checker.
                //
                // Function type unification: parameter types unify contravariantly, return types covariantly - Function type unification
                // requires identical type parameter structure. The subtype relation allows
                // contravariance in bounds, but unification is stricter.
                //
                // Example:
                //   fn f<T: Ord>(x: T) and fn f<T: Eq>(x: T) do NOT unify
                //   But fn f<T: Ord>(x: T) <: fn f<T: Eq>(x: T) if Ord extends Eq
                if tp1.len() != tp2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // Context unification with support for context polymorphism
                // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.5 - Context Polymorphism
                //
                // Unification rules for contexts:
                // - None/None: compatible (both pure)
                // - Concrete/Concrete: must be equal
                // - Variable/X: bind variable to X (captured in context_subst)
                // - Variable/Variable: unify them
                let context_subst = self.unify_contexts(c1, c2, span)?;
                let mut subst = context_subst;

                // Unify parameters
                for (pt1, pt2) in p1.iter().zip(p2.iter()) {
                    let s =
                        self.unify_inner(&pt1.apply_subst(&subst), &pt2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }

                // Unify return types
                let s = self.unify_inner(&r1.apply_subst(&subst), &r2.apply_subst(&subst), span)?;
                Ok(subst.compose(&s))
            }

            // Tuple types
            (Tuple(t1s), Tuple(t2s)) => {
                if t1s.len() != t2s.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for (ty1, ty2) in t1s.iter().zip(t2s.iter()) {
                    let s =
                        self.unify_inner(&ty1.apply_subst(&subst), &ty2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }
                Ok(subst)
            }

            // Array types
            (
                Array {
                    element: e1,
                    size: s1,
                },
                Array {
                    element: e2,
                    size: s2,
                },
            ) => {
                // Array size unification rules:
                // 1. If both have sizes, they must match exactly
                // 2. If one has size and the other doesn't, unify with the known size
                // 3. If neither has size, they unify (both unknown)
                match (s1, s2) {
                    (Some(n1), Some(n2)) if n1 != n2 => Err(TypeError::ConstMismatch {
                        expected: format!("Some({})", n2).into(),
                        actual: format!("Some({})", n1).into(),
                        span,
                    }),
                    _ => {
                        // Either sizes match, or at least one is None
                        self.unify_inner(e1, e2, span)
                    }
                }
            }

            // Array -> collection coercion (unification direction)
            // [T; N] can unify with C<T> for any array-coercible collection type C
            // Array literal to collection coercion: [1, 2, 3] infers as List<Int>
            // Uses data-driven array_coercible_types set (defaults to {"List"})
            (Array { element: e1, size: _ }, Generic { name, args })
                if self.is_array_coercible(name.as_str()) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }
            (Generic { name, args }, Array { element: e1, size: _ })
                if self.is_array_coercible(name.as_str()) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }
            // Array -> Named collection coercion (when collection type is parsed as Named)
            (Array { element: e1, size: _ }, Named { path, args })
                if self.path_is_array_coercible(path) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }
            (Named { path, args }, Array { element: e1, size: _ })
                if self.path_is_array_coercible(path) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }

            // Array -> TypeApp coercion (for HKT inference)
            // [T; N] can coerce to F<T> where F is inferred as List
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types inference
            (Array { element: e1, size: _ }, TypeApp { constructor, args: app_args })
            | (TypeApp { constructor, args: app_args }, Array { element: e1, size: _ }) => {
                if app_args.len() != 1 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // If constructor is a type variable, bind it to the default array-coercible
                // collection type ("List"). Uses data-driven resolution; will be replaced
                // with FromArray protocol dispatch once protocols are available.
                // NOTE: "List" is the default because it is the first entry in
                // array_coercible_types. A proper protocol-based approach would not
                // need a default at all.
                if let Type::Var(var) = constructor.as_ref() {
                    let list_ctor = Type::TypeConstructor {
                        name: "List".into(),
                        arity: 1,
                        kind: crate::advanced_protocols::Kind::unary_constructor(),
                    };
                    let subst = self.bind_var(*var, &list_ctor, span)?;

                    // Unify the element types
                    let s = self.unify_inner(
                        &e1.apply_subst(&subst),
                        &app_args[0].apply_subst(&subst),
                        span,
                    )?;
                    Ok(subst.compose(&s))
                } else if let Type::TypeConstructor { name, arity: 1, .. } = constructor.as_ref() {
                    if self.is_array_coercible(name.as_str()) {
                        // Already an array-coercible collection constructor, just unify element types
                        self.unify_inner(e1, &app_args[0], span)
                    } else {
                        Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        })
                    }
                } else {
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }

            // Slice types
            (Slice { element: e1 }, Slice { element: e2 }) => self.unify_inner(e1, e2, span),

            // Array -> Slice coercion: [T; N] can coerce to [T]
            // This enables &[T; N] -> &[T] through reference unification
            // Unsized coercion: converting sized types to unsized (e.g., List<T> to &[T], concrete to &dyn Protocol)
            (Array { element: e1, .. }, Slice { element: e2 }) => self.unify_inner(e1, e2, span),
            (Slice { element: e1 }, Array { element: e2, .. }) => self.unify_inner(e1, e2, span),

            // Collection -> Slice coercion: C<T> can coerce to [T] for array-coercible C
            // This enables patterns like passing &List<T> where &[T] is expected
            (Generic { name, args }, Slice { element: e2 })
                if self.is_array_coercible(name.as_str()) && args.len() == 1 =>
            {
                self.unify_inner(&args[0], e2, span)
            }
            (Slice { element: e1 }, Generic { name, args })
                if self.is_array_coercible(name.as_str()) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }
            // Collection (Named) -> Slice coercion
            (Named { path, args }, Slice { element: e2 })
                if self.path_is_array_coercible(path) && args.len() == 1 =>
            {
                self.unify_inner(&args[0], e2, span)
            }
            (Slice { element: e1 }, Named { path, args })
                if self.path_is_array_coercible(path) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }

            // Record types
            (Record(f1), Record(f2)) => {
                if f1.len() != f2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for (name, ty1) in f1 {
                    match f2.get(name) {
                        Some(ty2) => {
                            let s = self.unify_inner(
                                &ty1.apply_subst(&subst),
                                &ty2.apply_subst(&subst),
                                span,
                            )?;
                            subst = subst.compose(&s);
                        }
                        None => {
                            return Err(TypeError::Mismatch {
                                expected: t2.to_text(),
                                actual: t1.to_text(),
                                span,
                            });
                        }
                    }
                }
                Ok(subst)
            }

            // Extensible record with row polymorphism
            // Pattern matching: exhaustiveness checking, type narrowing in match arms, irrefutable patterns — .1 - Row Polymorphism
            (
                ExtensibleRecord {
                    fields: f1,
                    row_var: r1,
                },
                ExtensibleRecord {
                    fields: f2,
                    row_var: r2,
                },
            ) => {
                let mut subst = Substitution::new();

                // First unify the common fields
                for (name, ty1) in f1 {
                    if let Some(ty2) = f2.get(name) {
                        let s = self.unify_inner(
                            &ty1.apply_subst(&subst),
                            &ty2.apply_subst(&subst),
                            span,
                        )?;
                        subst = subst.compose(&s);
                    } else if r2.is_none() {
                        // f1 has a field that f2 doesn't, and f2 is closed
                        return Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        });
                    }
                    // Otherwise, the field will be captured by r2's row variable
                }

                // Check that f2's fields are also in f1 (unless f1 is open)
                for (name, _) in f2 {
                    if !f1.contains_key(name) && r1.is_none() {
                        return Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        });
                    }
                }

                // Unify row variables if both are present
                match (r1, r2) {
                    (Some(rv1), Some(rv2)) => {
                        // Both are open - unify the row variables
                        let s = self.unify_inner(&Type::Var(*rv1), &Type::Var(*rv2), span)?;
                        subst = subst.compose(&s);
                    }
                    (Some(rv), None) | (None, Some(rv)) => {
                        // One open, one closed - bind row variable to empty record
                        // Calculate the "remaining" fields
                        let remaining_fields: IndexMap<_, _> = if r1.is_some() {
                            // f2 has fields not in f1, those become the row
                            f2.iter()
                                .filter(|(k, _)| !f1.contains_key(*k))
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect()
                        } else {
                            // f1 has fields not in f2, those become the row
                            f1.iter()
                                .filter(|(k, _)| !f2.contains_key(*k))
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect()
                        };

                        // Bind the row variable to the remaining fields as a record
                        let remaining_type = if remaining_fields.is_empty() {
                            Type::Record(IndexMap::new()) // Empty row
                        } else {
                            Type::Record(remaining_fields)
                        };
                        let s = self.bind_var(*rv, &remaining_type, span)?;
                        subst = subst.compose(&s);
                    }
                    (None, None) => {
                        // Both closed - must have exactly same fields (already checked)
                    }
                }

                Ok(subst)
            }

            // Extensible record unifying with closed record
            (
                ExtensibleRecord {
                    fields: ext_fields,
                    row_var,
                },
                Record(rec_fields),
            )
            | (
                Record(rec_fields),
                ExtensibleRecord {
                    fields: ext_fields,
                    row_var,
                },
            ) => {
                let mut subst = Substitution::new();

                // All extensible record fields must be present in closed record
                for (name, ty1) in ext_fields {
                    match rec_fields.get(name) {
                        Some(ty2) => {
                            let s = self.unify_inner(
                                &ty1.apply_subst(&subst),
                                &ty2.apply_subst(&subst),
                                span,
                            )?;
                            subst = subst.compose(&s);
                        }
                        None => {
                            return Err(TypeError::Mismatch {
                                expected: t2.to_text(),
                                actual: t1.to_text(),
                                span,
                            });
                        }
                    }
                }

                // Bind row variable to remaining fields from closed record
                if let Some(rv) = row_var {
                    let remaining_fields: IndexMap<_, _> = rec_fields
                        .iter()
                        .filter(|(k, _)| !ext_fields.contains_key(*k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();

                    let remaining_type = Type::Record(remaining_fields);
                    let s = self.bind_var(*rv, &remaining_type, span)?;
                    subst = subst.compose(&s);
                } else {
                    // Extensible record is closed (no row var) - must have exact same fields
                    if ext_fields.len() != rec_fields.len() {
                        return Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        });
                    }
                }

                Ok(subst)
            }

            // Variant types
            (Variant(v1), Variant(v2)) => {
                // Try exact match first (same length, same keys)
                if v1.len() == v2.len() {
                    let mut subst = Substitution::new();
                    let mut all_matched = true;
                    for (tag, ty1) in v1 {
                        match v2.get(tag) {
                            Some(ty2) => {
                                match self.unify_inner(
                                    &ty1.apply_subst(&subst),
                                    &ty2.apply_subst(&subst),
                                    span,
                                ) {
                                    Ok(s) => { subst = subst.compose(&s); }
                                    Err(_) => { all_matched = false; break; }
                                }
                            }
                            None => { all_matched = false; break; }
                        }
                    }
                    if all_matched {
                        return Ok(subst);
                    }
                }

                // Subset matching: allow variant widening/narrowing
                // One set of variants is a subset of the other
                let v1_is_subset = v1.keys().all(|k| v2.contains_key(k));
                let v2_is_subset = v2.keys().all(|k| v1.contains_key(k));
                if v1_is_subset || v2_is_subset {
                    let mut subst = Substitution::new();
                    // Use the smaller set for iteration
                    let (smaller, larger) = if v1.len() <= v2.len() { (v1, v2) } else { (v2, v1) };
                    for (name, ty_s) in smaller.iter() {
                        if let Some(ty_l) = larger.get(name) {
                            if let Ok(s) = self.unify_inner(
                                &ty_s.apply_subst(&subst),
                                &ty_l.apply_subst(&subst),
                                span,
                            ) {
                                subst = subst.compose(&s);
                            }
                        }
                    }
                    return Ok(subst);
                }

                // Overlapping variant unification: when two variant types share some
                // constructors (e.g., both have Ok) but differ on others (Err vs Overflow),
                // unify the shared variants and succeed if at least one non-shared variant
                // payload contains unresolved type variables (meaning the type is not yet
                // fully determined). This handles the common case where
                // Ok(T)|Err(Var) and Ok(T)|Overflow(Unit) arise from different Result-like
                // type definitions resolving the same constructor name.
                let shared: Vec<&verum_common::Text> = v1.keys().filter(|k| v2.contains_key(k.as_str())).collect();
                if !shared.is_empty() {
                    // Check if non-shared variants have type variables (not yet resolved)
                    let v1_non_shared_has_vars = v1.iter()
                        .filter(|(k, _)| !v2.contains_key(k.as_str()))
                        .any(|(_, ty)| Self::has_type_vars(ty));
                    let v2_non_shared_has_vars = v2.iter()
                        .filter(|(k, _)| !v1.contains_key(k.as_str()))
                        .any(|(_, ty)| Self::has_type_vars(ty));
                    if v1_non_shared_has_vars || v2_non_shared_has_vars {
                        let mut subst = Substitution::new();
                        for name in &shared {
                            if let (Some(ty1), Some(ty2)) = (v1.get(*name), v2.get(*name)) {
                                if let Ok(s) = self.unify_inner(
                                    &ty1.apply_subst(&subst),
                                    &ty2.apply_subst(&subst),
                                    span,
                                ) {
                                    subst = subst.compose(&s);
                                }
                            }
                        }
                        return Ok(subst);
                    }
                }

                Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                })
            }

            // Dynamic protocol objects (dyn Display, dyn Error, dyn Display + Debug, etc.)
            (
                DynProtocol {
                    bounds: b1,
                    bindings: bi1,
                },
                DynProtocol {
                    bounds: b2,
                    bindings: bi2,
                },
            ) => {
                // Bounds must match (same protocols)
                if b1.len() != b2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // Check each bound matches (order-sensitive)
                for (bound1, bound2) in b1.iter().zip(b2.iter()) {
                    if bound1 != bound2 {
                        return Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        });
                    }
                }

                // Unify associated type bindings
                if bi1.len() != bi2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for (name, ty1) in bi1.iter() {
                    match bi2.get(name) {
                        Some(ty2) => {
                            let s = self.unify_inner(
                                &ty1.apply_subst(&subst),
                                &ty2.apply_subst(&subst),
                                span,
                            )?;
                            subst = subst.compose(&s);
                        }
                        None => {
                            return Err(TypeError::Mismatch {
                                expected: t2.to_text(),
                                actual: t1.to_text(),
                                span,
                            });
                        }
                    }
                }

                Ok(subst)
            }

            // Concrete type → dyn Protocol coercion.
            // A concrete type can be used where &dyn Protocol is expected if it
            // implements the protocol. We trust the impl-block registration and
            // allow the coercion — the protocol checker validates at call sites.
            (_, DynProtocol { .. }) => {
                // Accept any concrete type as dyn Protocol.
                // Full protocol impl checking is done at the call site in infer.rs.
                Ok(Substitution::new())
            }
            (DynProtocol { .. }, _) => {
                Ok(Substitution::new())
            }

            // Reference types with coercion rules
            // Unified reference model: &T (managed CBGR ~15ns), &checked T (statically verified 0ns), &unsafe T (unchecked 0ns) — .3.3 - Three-Tier Reference Coercion
            // Hierarchy: &unsafe T <: &checked T <: &T (Liskov Substitution Principle)
            //
            // ALLOWED (implicit upcast - "forgetful"):
            //   &unsafe T → &checked T  ✓
            //   &unsafe T → &T          ✓
            //   &checked T → &T         ✓
            //
            // FORBIDDEN (downcast):
            //   &T → &checked T         ✗
            //   &T → &unsafe T          ✗
            //   &checked T → &unsafe T  ✗

            // Same-tier references: direct unification with mutability coercion
            // &mut T can be used where &T is expected (mutable → immutable is safe)
            // Reference coercion: &mut T coerces to &T, &T coerces to &dyn Protocol when Protocol is implemented
            (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Allow &mut T → &T (forgetting write capability is safe)
                // Error only if expected is mutable but actual is immutable
                if *m2 && !*m1 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
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
                // Allow &checked mut T → &checked T (forgetting write capability is safe)
                // Error only if expected is mutable but actual is immutable
                if *m2 && !*m1 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
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
                // Allow &unsafe mut T → &unsafe T (mutable to immutable is OK, like in C)
                // Error only if expected is mutable but actual is immutable
                if *m2 && !*m1 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
            }

            // UPCAST: &unsafe T → &checked T (allowed)
            // Unsafe references can coerce to any checked reference regardless of mutability.
            (
                UnsafeReference {
                    mutable: _m1,
                    inner: i1,
                },
                CheckedReference {
                    mutable: _m2,
                    inner: i2,
                },
            ) => {
                self.unify_inner(i1, i2, span)
            }

            // UPCAST: &unsafe T → &T or &mut T (allowed)
            // Unsafe references have no compile-time safety guarantees, so they
            // can coerce to any managed reference tier. The mutability of the
            // unsafe ref is irrelevant since safety is manual.
            (
                UnsafeReference {
                    mutable: _m1,
                    inner: i1,
                },
                Reference {
                    mutable: _m2,
                    inner: i2,
                },
            ) => {
                self.unify_inner(i1, i2, span)
            }

            // UPCAST: &checked T → &T (allowed)
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
                if m1 != m2 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
            }

            // FFI COERCION: &mut T → &unsafe T (allowed for FFI interop)
            // This is technically a downcast to unsafe, but is commonly needed
            // when passing mutable buffers to FFI functions.
            // The caller takes responsibility for safety when using unsafe.
            (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                UnsafeReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                // Allow mutable ref to either mutable or immutable unsafe ref
                // (can always use a mutable pointer where const is expected)
                // But not immutable ref to mutable unsafe (need mut for writing)
                if *m2 && !*m1 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
            }

            // POINTER ↔ REFERENCE COERCION
            // *const T / *mut T can coerce to &T / &mut T (upcast from raw to managed)
            // and vice versa for FFI interop
            (
                Pointer {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            )
            | (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                Pointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                if *m2 && !*m1 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
            }

            // POINTER ↔ CHECKED REFERENCE COERCION
            (
                Pointer {
                    mutable: m1,
                    inner: i1,
                },
                CheckedReference {
                    mutable: m2,
                    inner: i2,
                },
            )
            | (
                CheckedReference {
                    mutable: m1,
                    inner: i1,
                },
                Pointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => {
                if *m2 && !*m1 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
            }

            // FORBIDDEN DOWNCASTS: These will fall through to type mismatch

            // Ownership references
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
                if m1 != m2 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
            }

            // Pointers — allow *const T ↔ *mut T coercion for raw pointer interop
            // Raw pointers are inherently unsafe; mutability is advisory, not enforced.
            (
                Pointer {
                    mutable: _m1,
                    inner: i1,
                },
                Pointer {
                    mutable: _m2,
                    inner: i2,
                },
            ) => {
                self.unify_inner(i1, i2, span)
            }

            // POINTER ↔ UNSAFE REFERENCE COERCION
            // In Verum, *const T ≡ &unsafe T and *mut T ≡ &unsafe mut T
            // These are semantically equivalent types. Since both represent
            // raw/unmanaged access, mutability coercion is unrestricted.
            (
                Pointer {
                    mutable: _m1,
                    inner: i1,
                },
                UnsafeReference {
                    mutable: _m2,
                    inner: i2,
                },
            )
            | (
                UnsafeReference {
                    mutable: _m1,
                    inner: i1,
                },
                Pointer {
                    mutable: _m2,
                    inner: i2,
                },
            ) => {
                self.unify_inner(i1, i2, span)
            }

            // VOLATILE POINTER unification
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
                if m1 != m2 {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
                self.unify_inner(i1, i2, span)
            }

            // POINTER ↔ INTEGER COERCION (FFI)
            // Raw pointers and UIntSize/IntSize are interchangeable in FFI contexts.
            // This enables `ptr as usize` and `usize as *mut T` patterns.
            (Pointer { .. }, Named { path, .. })
            | (Named { path, .. }, Pointer { .. })
                if matches!(path.last_segment_name(), "UIntSize" | "IntSize" | "usize" | "isize") =>
            {
                Ok(Substitution::new())
            }

            // CString ↔ *const c_char coercion (FFI string interop)
            (Named { path, .. }, Pointer { .. })
                if matches!(path.last_segment_name(), "CString" | "CStr") =>
            {
                Ok(Substitution::new())
            }
            (Pointer { .. }, Named { path, .. })
                if matches!(path.last_segment_name(), "CString" | "CStr") =>
            {
                Ok(Substitution::new())
            }

            // META TYPE COERCION: TokenStream ↔ @Expr/@Ident/@Type etc.
            // Quote blocks produce TokenStream, but meta functions may return @Expr
            (Named { path: p1, .. }, Named { path: p2, .. })
                if (matches!(p1.last_segment_name(), "TokenStream") &&
                    matches!(p2.last_segment_name(), "@Expr" | "@Ident" | "@Type" | "@Pattern" | "@Stmt" | "@Item" | "@Block"))
                || (matches!(p2.last_segment_name(), "TokenStream") &&
                    matches!(p1.last_segment_name(), "@Expr" | "@Ident" | "@Type" | "@Pattern" | "@Stmt" | "@Item" | "@Block")) =>
            {
                Ok(Substitution::new())
            }

            // AUTO-DEREF COERCION: &T → T (implicit dereference)
            // When a reference type appears where a value type is expected,
            // automatically dereference. This follows the principle that
            // reading through a reference is always safe.
            // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Auto-dereference semantics
            (
                Reference { inner, .. }
                | CheckedReference { inner, .. }
                | UnsafeReference { inner, .. },
                other,
            ) if !matches!(other, Reference { .. } | CheckedReference { .. } | UnsafeReference { .. } | Ownership { .. } | Pointer { .. }) => {
                self.unify_inner(inner, other, span)
            }

            // AUTO-DEREF COERCION (reversed): T expected, &T found
            (
                other,
                Reference { inner, .. }
                | CheckedReference { inner, .. }
                | UnsafeReference { inner, .. },
            ) if !matches!(other, Reference { .. } | CheckedReference { .. } | UnsafeReference { .. } | Ownership { .. } | Pointer { .. }) => {
                self.unify_inner(other, inner, span)
            }

            // Refinement types (structural unification - ignore predicates)
            (Refined { base: b1, .. }, Refined { base: b2, .. }) => self.unify_inner(b1, b2, span),

            (Refined { base, .. }, ty) | (ty, Refined { base, .. }) => {
                self.unify_inner(base, ty, span)
            }

            // Named types
            (Named { path: p1, args: a1 }, Named { path: p2, args: a2 }) => {
                // Sized integer cross-coercion: USize ↔ ISize, USize ↔ UInt64, etc.
                // Uses data-driven sized_numeric_types set instead of hardcoded name list.
                if p1.segments.len() == 1 && p2.segments.len() == 1 && a1.is_empty() && a2.is_empty() {
                    fn extract_name(path: &verum_ast::ty::Path) -> &str {
                        path.segments.last().map(|s| match s {
                            verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                            _ => "",
                        }).unwrap_or("")
                    }
                    let n1 = extract_name(p1);
                    let n2 = extract_name(p2);
                    if n1 != n2 && self.is_sized_numeric(n1) && self.is_sized_numeric(n2) {
                        return Ok(Substitution::new());
                    }
                }

                // Compare paths by segment names only, ignoring spans
                // This is necessary because the same type name appearing in different
                // locations (e.g., in a type definition vs. in a variable usage) will
                // have different spans but should still unify.
                if p1.segments.len() != p2.segments.len() || a1.len() != a2.len() {
                    // Before failing, check for related tensor type coercions
                    // (e.g., Tensor ↔ DynTensor, Cotangent<T> ↔ T)
                    // Uses data-driven tensor_family_types set instead of substring checks.
                    fn extract_last_name(path: &verum_ast::ty::Path) -> &str {
                        path.segments.last().map(|s| match s {
                            verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                            _ => "",
                        }).unwrap_or("")
                    }
                    let tn1 = extract_last_name(p1);
                    let tn2 = extract_last_name(p2);
                    if self.is_tensor_family(tn1) && self.is_tensor_family(tn2) {
                        return Ok(Substitution::new());
                    }
                    // Allow Byte ↔ Sockaddr for FFI
                    if (tn1 == "Byte" || tn1 == "UInt8") && matches!(tn2, "Sockaddr" | "SocketAddr" | "SockaddrIn") {
                        return Ok(Substitution::new());
                    }
                    if (tn2 == "Byte" || tn2 == "UInt8") && matches!(tn1, "Sockaddr" | "SocketAddr" | "SockaddrIn") {
                        return Ok(Substitution::new());
                    }
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // Compare segment names (ignoring spans)
                for (seg1, seg2) in p1.segments.iter().zip(p2.segments.iter()) {
                    use verum_ast::ty::PathSegment;
                    match (seg1, seg2) {
                        (PathSegment::Name(id1), PathSegment::Name(id2)) => {
                            if id1.name != id2.name {
                                return Err(TypeError::Mismatch {
                                    expected: t2.to_text(),
                                    actual: t1.to_text(),
                                    span,
                                });
                            }
                        }
                        (PathSegment::SelfValue, PathSegment::SelfValue)
                        | (PathSegment::Super, PathSegment::Super)
                        | (PathSegment::Cog, PathSegment::Cog)
                        | (PathSegment::Relative, PathSegment::Relative) => {}
                        _ => {
                            return Err(TypeError::Mismatch {
                                expected: t2.to_text(),
                                actual: t1.to_text(),
                                span,
                            });
                        }
                    }
                }

                let mut subst = Substitution::new();
                for (ty1, ty2) in a1.iter().zip(a2.iter()) {
                    let s =
                        self.unify_inner(&ty1.apply_subst(&subst), &ty2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }
                Ok(subst)
            }

            // ASSOCIATED TYPE PROJECTIONS
            //
            // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Associated type resolution
            //
            // Associated type projections are represented as Generic { name: "::AssocName", args: [base] }
            // where base is the type being projected from (e.g., S in S.Item).
            //
            // When unifying a projection with another type:
            // 1. If both are projections with same name and bases unify -> unify
            // 2. If projection has type variable base -> allow (deferred constraint)
            // 3. If projection has concrete base -> try to resolve, then unify result
            //
            // This enables generic code like:
            //   implement<S: Stream> Stream for Enumerate<S> {
            //       fn poll_next(&mut self, cx: &mut Context) -> Poll<Maybe<(Int, S.Item)>>
            //   }
            //
            // Where S.Item must unify with the actual item type from poll_next results.

            // Case 1: Both are projections (same associated type name)
            (
                Generic {
                    name: n1,
                    args: a1,
                },
                Generic {
                    name: n2,
                    args: a2,
                },
            ) if n1.as_str().starts_with("::") && n2.as_str().starts_with("::") && n1 == n2 => {
                // Same projection name (e.g., both are ::Item)
                // Unify the base types
                if a1.len() != a2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for (ty1, ty2) in a1.iter().zip(a2.iter()) {
                    let s =
                        self.unify_inner(&ty1.apply_subst(&subst), &ty2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }
                Ok(subst)
            }

            // Case 2: Projection unifying with non-projection type
            // If projection base contains type variables, defer by allowing unification
            (Generic { name, args }, other) | (other, Generic { name, args })
                if name.as_str().starts_with("::")
                    && !args.is_empty()
                    && Self::has_type_vars(&args[0]) =>
            {
                // The projection's base type is not yet fully resolved.
                // We allow this unification to succeed, effectively treating the projection
                // as compatible with the other type. This is sound because:
                //
                // 1. The base type variable will be resolved later during constraint solving
                // 2. When resolved, the actual associated type will be determined
                // 3. At that point, full type checking will verify compatibility
                //
                // This matches the semantic that in generic code like:
                //   fn enumerate<S: Stream>(s: S) -> impl Stream<Item = (Int, S.Item)>
                // The S.Item is "opaque" until S is instantiated with a concrete type.

                // If the other type also has unresolved vars, just succeed
                if Self::has_type_vars(other) {
                    return Ok(Substitution::new());
                }

                // Create a fresh type variable to represent the projection result
                // and unify it with the other type
                let projection_var = TypeVar::fresh();
                let subst = self.bind_var(projection_var, other, span)?;
                Ok(subst)
            }

            // Generic types (stdlib types like List<T>, Map<K,V>, Box<T>, etc.)
            (Generic { name: n1, args: a1 }, Generic { name: n2, args: a2 }) => {
                if n1 != n2 || a1.len() != a2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for (ty1, ty2) in a1.iter().zip(a2.iter()) {
                    let s =
                        self.unify_inner(&ty1.apply_subst(&subst), &ty2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }
                Ok(subst)
            }

            // Cross-unification: Generic <-> Named
            // This handles cases where the same type is represented differently:
            // - Generic { name: "Map", args } from Type::map()
            // - Named { path: Map, args } from parsing Map<K, V>
            (Generic { name, args: a1 }, Named { path, args: a2 })
            | (Named { path, args: a2 }, Generic { name, args: a1 }) => {
                // Extract the last segment from the path for comparison
                let path_name = path.segments.last().map(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    }
                }).unwrap_or("");

                if name.as_str() != path_name || a1.len() != a2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for (ty1, ty2) in a1.iter().zip(a2.iter()) {
                    let s = self.unify_inner(&ty1.apply_subst(&subst), &ty2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }
                Ok(subst)
            }

            // Unify Generic types with their expanded Variant forms
            // This is stdlib-agnostic: uses the variant_type_names registry to map
            // variant signatures to their nominal type names.
            // Example: Maybe<T> should unify with Some(T) | None
            // Example: Result<T, E> should unify with Ok(T) | Err(E)
            (Generic { name, args }, Variant(variants))
            | (Variant(variants), Generic { name, args }) => {
                // Generate signature from variant structure
                let signature = Self::variant_type_signature(variants);

                                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG unify Generic<->Variant] name={}, args={:?}, signature={}, registry={:?}",
                    // name, args, signature, self.variant_type_names.keys().collect::<Vec<_>>());

                // Look up the type name from registry (try full signature, then relaxed)
                let registered_name_opt = self.variant_type_names.get(&signature).cloned()
                    .or_else(|| {
                        let relaxed = Self::variant_type_signature_relaxed(variants);
                        self.variant_type_names.get(&relaxed).cloned()
                    });
                if let Some(registered_name) = registered_name_opt.as_ref() {
                    if registered_name.as_str() == name.as_str() {
                        // Type names match! Now we need to extract type args from the Variant
                        // in the correct declaration order using the type_var_order registry.

                        // CRITICAL FIX: The old approach was to just zip payloads with args,
                        // but payload order doesn't match type parameter declaration order.
                        // For `Validated<E, A> = Valid(A) | Invalid(List<E>)`:
                        // - Type args are [E, A] in declaration order
                        // - But payloads might be [Int, List<Text>] in map iteration order
                        //
                        // New approach: Extract type args from Variant using unification with
                        // the original type definition, then unify those with the Generic's args.

                        let type_name_text = name.clone();

                        // Try to use proper type_var_order-based extraction
                        if let (Some(type_var_order), Some(original_type)) = (
                            self.type_var_orders.get(&type_name_text),
                            self.original_variant_types.get(&type_name_text)
                        ) {
                            // Extract TypeVar -> concrete type mapping by unifying original with actual
                            let mut type_var_mapping: indexmap::IndexMap<TypeVar, Type> = indexmap::IndexMap::new();
                            if let Type::Variant(original_variants) = original_type {
                                for (variant_name, original_payload) in original_variants.iter() {
                                    if let Some(actual_payload) = variants.get(variant_name) {
                                        Self::extract_type_var_mapping_static(original_payload, actual_payload, &mut type_var_mapping);
                                    }
                                }
                            }

                                                        // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG unify Generic<->Variant] type_var_order={:?}, mapping={:?}",
                                // type_var_order, type_var_mapping);

                            // Build extracted args in declaration order
                            let mut extracted_args: List<Type> = List::new();
                            for tv in type_var_order.iter() {
                                if let Some(concrete_ty) = type_var_mapping.get(tv) {
                                    extracted_args.push(concrete_ty.clone());
                                } else {
                                    // TypeVar not found in mapping - this shouldn't happen for well-formed types
                                    // Fall back to old behavior
                                                                        // #[cfg(debug_assertions)]
                                    // eprintln!("[DEBUG unify Generic<->Variant] TypeVar {:?} not in mapping, falling back", tv);
                                    return self.unify_generic_variant_fallback(args, variants, t1, t2, span);
                                }
                            }

                                                        // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG unify Generic<->Variant] extracted_args={:?}", extracted_args);

                            // Verify argument count matches
                            if extracted_args.len() != args.len() {
                                return Err(TypeError::Mismatch {
                                    expected: t2.to_text(),
                                    actual: t1.to_text(),
                                    span,
                                });
                            }

                            // Unify each Generic arg with the corresponding extracted arg
                            let mut subst = Substitution::new();
                            for (generic_arg, extracted_arg) in args.iter().zip(extracted_args.iter()) {
                                let s = self.unify_inner(
                                    &generic_arg.apply_subst(&subst),
                                    &extracted_arg.apply_subst(&subst),
                                    span,
                                )?;
                                subst = subst.compose(&s);
                            }
                            return Ok(subst);
                        }

                        // Fallback: no type_var_order registered, use old payload-based approach
                                                // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG unify Generic<->Variant] No type_var_order for '{}', using fallback", name);
                        return self.unify_generic_variant_fallback(args, variants, t1, t2, span);
                    }
                }

                                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG unify Generic<->Variant] NO MATCH - signature not in registry or name mismatch");

                // No registry entry or name doesn't match - type mismatch
                Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                })
            }

            // Unify Named types with their expanded Variant forms
            // This is stdlib-agnostic: uses the variant_type_names registry to map
            // variant signatures to their nominal type names.
            // Example: Maybe<T> should unify with Some(T) | None
            // Example: Result<T, E> should unify with Ok(T) | Err(E)
            (Named { path, args }, Variant(variants))
            | (Variant(variants), Named { path, args }) => {
                // Extract the type name from the path
                let type_name = path.segments.last().map(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    }
                }).unwrap_or("");

                // Generate signature from variant structure
                let signature = Self::variant_type_signature(variants);

                // Look up the type name from registry (try full signature, then relaxed)
                let registered_name_opt = self.variant_type_names.get(&signature).cloned()
                    .or_else(|| {
                        let relaxed = Self::variant_type_signature_relaxed(variants);
                        self.variant_type_names.get(&relaxed).cloned()
                    });
                if let Some(registered_name) = registered_name_opt.as_ref() {
                    if registered_name.as_str() == type_name {
                        // Non-generic type: Named{Expr, []} == Variant{Num(Int) | Neg(Heap<Expr>)}
                        // If the Named type has 0 type args, it's a concrete type matching its expansion.
                        if args.is_empty() {
                            return Ok(Substitution::new());
                        }

                        // Type names match! Use proper type_var_order-based extraction.
                        let type_name_text: verum_common::Text = type_name.into();

                        // Try to use proper type_var_order-based extraction
                        if let (Some(type_var_order), Some(original_type)) = (
                            self.type_var_orders.get(&type_name_text),
                            self.original_variant_types.get(&type_name_text)
                        ) {
                            // Extract TypeVar -> concrete type mapping by unifying original with actual
                            let mut type_var_mapping: indexmap::IndexMap<TypeVar, Type> = indexmap::IndexMap::new();
                            if let Type::Variant(original_variants) = original_type {
                                for (variant_name, original_payload) in original_variants.iter() {
                                    if let Some(actual_payload) = variants.get(variant_name) {
                                        Self::extract_type_var_mapping_static(original_payload, actual_payload, &mut type_var_mapping);
                                    }
                                }
                            }

                                                        // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG unify Named<->Variant] type_var_order={:?}, mapping={:?}",
                                // type_var_order, type_var_mapping);

                            // Build extracted args in declaration order
                            let mut extracted_args: List<Type> = List::new();
                            for tv in type_var_order.iter() {
                                if let Some(concrete_ty) = type_var_mapping.get(tv) {
                                    extracted_args.push(concrete_ty.clone());
                                } else {
                                    // TypeVar not found in mapping - fall back to old behavior
                                                                        // #[cfg(debug_assertions)]
                                    // eprintln!("[DEBUG unify Named<->Variant] TypeVar {:?} not in mapping, falling back", tv);
                                    return self.unify_generic_variant_fallback(args, variants, t1, t2, span);
                                }
                            }

                                                        // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG unify Named<->Variant] extracted_args={:?}", extracted_args);

                            // Verify argument count matches
                            if extracted_args.len() != args.len() {
                                return Err(TypeError::Mismatch {
                                    expected: t2.to_text(),
                                    actual: t1.to_text(),
                                    span,
                                });
                            }

                            // Unify each Named arg with the corresponding extracted arg
                            let mut subst = Substitution::new();
                            for (named_arg, extracted_arg) in args.iter().zip(extracted_args.iter()) {
                                let s = self.unify_inner(
                                    &named_arg.apply_subst(&subst),
                                    &extracted_arg.apply_subst(&subst),
                                    span,
                                )?;
                                subst = subst.compose(&s);
                            }
                            return Ok(subst);
                        }

                        // Fallback: no type_var_order registered, use old payload-based approach
                                                // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG unify Named<->Variant] No type_var_order for '{}', using fallback", type_name);
                        return self.unify_generic_variant_fallback(args, variants, t1, t2, span);
                    }
                }

                // Fallback: Try to unify Named type with Variant using type variable collection.
                // This handles user-defined recursive types like List<T> -> Cons(T, List<T>) | Nil
                // where the variant signature is not in the registry.
                let mut type_vars: Vec<TypeVar> = Vec::new();
                for (_, payload_ty) in variants.iter() {
                    Self::collect_type_vars_from_type(payload_ty, &mut type_vars);
                }
                type_vars.sort();
                type_vars.dedup();

                // Build substitution from type arguments by binding type vars to the args
                let mut subst = Substitution::new();
                for (idx, tv) in type_vars.iter().enumerate() {
                    if let Some(arg_ty) = args.get(idx) {
                        // Unify the type variable with the argument type
                        let s = self.unify_inner(&Type::Var(*tv), arg_ty, span)?;
                        subst = subst.compose(&s);
                    }
                }

                Ok(subst)
            }

            // TypeApp (with Var constructor) vs Variant - HKT inference
            // When M<A> is unified with Maybe<Int> variant, infer M = Maybe
            // This is stdlib-agnostic: uses the variant_type_names registry.
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types inference
            (TypeApp { constructor, args: app_args }, Variant(variants))
            | (Variant(variants), TypeApp { constructor, args: app_args }) => {
                // Generate signature from variant structure
                let signature = Self::variant_type_signature(variants);

                // Check if constructor is a type variable (HKT inference case)
                if let Type::Var(var) = constructor.as_ref() {
                    // Look up the type constructor name from registry
                    if let Some(type_ctor_name) = self.variant_type_names.get(&signature) {
                        // Calculate arity from non-Unit payloads
                        let type_ctor_arity = variants
                            .values()
                            .filter(|payload| **payload != Type::Unit)
                            .count();

                        // Check arity matches
                        if app_args.len() != type_ctor_arity {
                            return Err(TypeError::Mismatch {
                                expected: t2.to_text(),
                                actual: t1.to_text(),
                                span,
                            });
                        }

                        // Bind the type variable to the type constructor
                        let type_ctor = Type::TypeConstructor {
                            name: type_ctor_name.clone(),
                            arity: type_ctor_arity,
                            kind: Self::kind_for_arity(type_ctor_arity),
                        };
                        let mut subst = self.bind_var(*var, &type_ctor, span)?;

                        // Collect type variables from the variant payloads
                        let mut variant_type_vars: Vec<TypeVar> = Vec::new();
                        for (_, payload_ty) in variants.iter() {
                            Self::collect_type_vars_from_type(payload_ty, &mut variant_type_vars);
                        }
                        variant_type_vars.sort();
                        variant_type_vars.dedup();

                        // Unify the type arguments with the variant's type parameters
                        for (idx, app_arg) in app_args.iter().enumerate() {
                            if let Some(tv) = variant_type_vars.get(idx) {
                                let s = self.unify_inner(
                                    &app_arg.apply_subst(&subst),
                                    &Type::Var(*tv).apply_subst(&subst),
                                    span,
                                )?;
                                subst = subst.compose(&s);
                            }
                        }

                        return Ok(subst);
                    }

                    // Unknown variant type - can't infer HKT
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                } else {
                    // Constructor is not a variable - check if it matches the variant type
                    if let Type::TypeConstructor { name, .. } = constructor.as_ref() {
                        // Look up from registry and check name match
                        if let Some(registered_name) = self.variant_type_names.get(&signature) {
                            if registered_name.as_str() == name.as_str() {
                                // Collect and unify type arguments
                                let mut variant_type_vars: Vec<TypeVar> = Vec::new();
                                for (_, payload_ty) in variants.iter() {
                                    Self::collect_type_vars_from_type(payload_ty, &mut variant_type_vars);
                                }
                                variant_type_vars.sort();
                                variant_type_vars.dedup();

                                let mut subst = Substitution::new();
                                for (idx, app_arg) in app_args.iter().enumerate() {
                                    if let Some(tv) = variant_type_vars.get(idx) {
                                        let s = self.unify_inner(
                                            &app_arg.apply_subst(&subst),
                                            &Type::Var(*tv).apply_subst(&subst),
                                            span,
                                        )?;
                                        subst = subst.compose(&s);
                                    }
                                }
                                return Ok(subst);
                            }
                        }
                    }
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }

            // Meta parameter types
            // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Meta parameters are compile-time values
            // Meta parameters unify if:
            // 1. Names match (same compile-time parameter)
            // 2. Base types unify
            // 3. Refinements are compatible
            (
                Meta {
                    name: n1,
                    ty: t1,
                    refinement: r1,
                },
                Meta {
                    name: n2,
                    ty: t2,
                    refinement: r2,
                },
            ) => {
                // Names must match (same meta parameter)
                if n1 != n2 {
                    return Err(TypeError::Mismatch {
                        expected: format!("Meta parameter '{}''", n2).into(),
                        actual: format!("Meta parameter '{}'", n1).into(),
                        span,
                    });
                }

                // Check refinements are compatible
                // For unification, we require exact refinement match
                // (subtyping is handled separately in subtype.rs)
                match (r1, r2) {
                    (Some(p1), Some(p2)) => {
                        if p1 != p2 {
                            return Err(TypeError::Mismatch {
                                expected: "Meta parameter with refinement predicate"
                                    .to_string()
                                    .into(),
                                actual: "Meta parameter with different refinement predicate"
                                    .to_string()
                                    .into(),
                                span,
                            });
                        }
                    }
                    (None, None) => {}
                    _ => {
                        return Err(TypeError::Mismatch {
                            expected: "Meta parameter with refinement".into(),
                            actual: "Meta parameter without refinement".into(),
                            span,
                        });
                    }
                }

                // Unify base types
                self.unify_inner(t1, t2, span)
            }

            // Meta parameter unification with non-Meta types:
            // A meta parameter like `N: meta usize` should be treated as its base type
            // when used in expression contexts (array sizes, ranges, arithmetic).
            (Meta { ty, .. }, other) | (other, Meta { ty, .. }) => {
                self.unify_inner(ty, other, span)
            }

            // ============================================
            // DEPENDENT TYPES (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
            // ============================================

            // Pi Types (Dependent Functions)
            // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case
            // (x: A) -> B(x) unifies with (y: A') -> B'(y) if:
            // 1. A unifies with A'
            // 2. B[x := fresh] unifies with B'[y := fresh] under the substitution
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
                // Unify parameter types
                let subst = self.unify_inner(p1, p2, span)?;

                // For dependent return types, we need alpha-equivalence
                // Substitute the parameter names to a common fresh variable
                let r1_applied = r1.apply_subst(&subst);
                let r2_applied = r2.apply_subst(&subst);

                // If parameter names differ, rename one to match the other
                let r2_renamed = if n1 != n2 {
                    Self::rename_bound_var(&r2_applied, n2, n1)
                } else {
                    r2_applied
                };

                let s2 = self.unify_inner(&r1_applied, &r2_renamed, span)?;
                Ok(subst.compose(&s2))
            }

            // Sigma Types (Dependent Pairs)
            // Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma
            // (x: A, B(x)) unifies with (y: A', B'(y)) if:
            // 1. A unifies with A'
            // 2. B[x := fresh] unifies with B'[y := fresh]
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
                // Unify first component types
                let subst = self.unify_inner(f1, f2, span)?;

                // For dependent second component, handle alpha-equivalence
                let s1_applied = s1.apply_subst(&subst);
                let s2_applied = s2.apply_subst(&subst);

                let s2_renamed = if n1 != n2 {
                    Self::rename_bound_var(&s2_applied, n2, n1)
                } else {
                    s2_applied
                };

                let sub2 = self.unify_inner(&s1_applied, &s2_renamed, span)?;
                Ok(subst.compose(&sub2))
            }

            // Equality Types
            // Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution
            // Eq<A, x, y> unifies with Eq<A', x', y'> if:
            // 1. A unifies with A'
            // 2. x equals x' (definitional equality)
            // 3. y equals y' (definitional equality)
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
                // Unify the carrier types
                let subst = self.unify_inner(t1, t2, span)?;

                // Check definitional equality of terms. Fast path is
                // syntactic equality on EqTerm; fallback routes through
                // the cubical normalizer so identities like
                // `transport Refl x ≡ x` and `sym(refl(x)) ≡ refl(x)`
                // are accepted.
                if !Self::eq_terms_equal(l1, l2)
                    && !crate::cubical_bridge::definitionally_equal_cubical(l1, l2)
                {
                    return Err(TypeError::Mismatch {
                        expected: "Eq type with matching left-hand side".into(),
                        actual: "Eq type with different left-hand side".into(),
                        span,
                    });
                }

                if !Self::eq_terms_equal(r1, r2)
                    && !crate::cubical_bridge::definitionally_equal_cubical(r1, r2)
                {
                    return Err(TypeError::Mismatch {
                        expected: "Eq type with matching right-hand side".into(),
                        actual: "Eq type with different right-hand side".into(),
                        span,
                    });
                }

                Ok(subst)
            }

            // Universe Types
            // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
            // Type_n unifies with Type_m if n == m
            // Universe polymorphism is handled by level variables
            (Universe { level: l1 }, Universe { level: l2 }) => {
                if Self::universe_levels_unify(l1, l2) {
                    Ok(Substitution::new())
                } else {
                    Err(TypeError::Mismatch {
                        expected: format!("Type{}", l2).into(),
                        actual: format!("Type{}", l1).into(),
                        span,
                    })
                }
            }

            // Prop (Proof-Irrelevant Propositions)
            // Inductive types: recursive type definitions with structural recursion, termination checking — .1
            // Prop unifies with Prop
            (Prop, Prop) => Ok(Substitution::new()),

            // Inductive Types
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1
            // Two inductive types unify if:
            // 1. Names match
            // 2. All parameters unify
            // 3. All indices unify (for indexed families)
            (
                Inductive {
                    name: n1,
                    params: p1,
                    indices: i1,
                    universe: u1,
                    constructors: c1,
                },
                Inductive {
                    name: n2,
                    params: p2,
                    indices: i2,
                    universe: u2,
                    constructors: c2,
                },
            ) => {
                // Names must match
                if n1 != n2 {
                    return Err(TypeError::Mismatch {
                        expected: format!("Inductive type '{}'", n2).into(),
                        actual: format!("Inductive type '{}'", n1).into(),
                        span,
                    });
                }

                // Universe levels must be compatible
                if !Self::universe_levels_unify(u1, u2) {
                    return Err(TypeError::Mismatch {
                        expected: format!("Universe level {}", u2).into(),
                        actual: format!("Universe level {}", u1).into(),
                        span,
                    });
                }

                // Constructor count must match
                if c1.len() != c2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} constructors", c2.len()).into(),
                        actual: format!("{} constructors", c1.len()).into(),
                        span,
                    });
                }

                // Unify parameters
                if p1.len() != p2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} type parameters", p2.len()).into(),
                        actual: format!("{} type parameters", p1.len()).into(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for ((_, t1), (_, t2)) in p1.iter().zip(p2.iter()) {
                    let s =
                        self.unify_inner(&t1.apply_subst(&subst), &t2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }

                // Unify indices (for indexed families like List<T, n>)
                if i1.len() != i2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} indices", i2.len()).into(),
                        actual: format!("{} indices", i1.len()).into(),
                        span,
                    });
                }

                for ((_, t1), (_, t2)) in i1.iter().zip(i2.iter()) {
                    let s =
                        self.unify_inner(&t1.apply_subst(&subst), &t2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }

                Ok(subst)
            }

            // Coinductive Types
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .2
            (
                Coinductive {
                    name: n1,
                    params: p1,
                    destructors: d1,
                },
                Coinductive {
                    name: n2,
                    params: p2,
                    destructors: d2,
                },
            ) => {
                // Names must match
                if n1 != n2 {
                    return Err(TypeError::Mismatch {
                        expected: format!("Coinductive type '{}'", n2).into(),
                        actual: format!("Coinductive type '{}'", n1).into(),
                        span,
                    });
                }

                // Destructor count must match
                if d1.len() != d2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} destructors", d2.len()).into(),
                        actual: format!("{} destructors", d1.len()).into(),
                        span,
                    });
                }

                // Unify parameters
                if p1.len() != p2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} type parameters", p2.len()).into(),
                        actual: format!("{} type parameters", p1.len()).into(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for ((_, t1), (_, t2)) in p1.iter().zip(p2.iter()) {
                    let s =
                        self.unify_inner(&t1.apply_subst(&subst), &t2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }

                Ok(subst)
            }

            // Higher Inductive Types (HITs)
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .3
            (
                HigherInductive {
                    name: n1,
                    params: p1,
                    point_constructors: pc1,
                    path_constructors: pa1,
                },
                HigherInductive {
                    name: n2,
                    params: p2,
                    point_constructors: pc2,
                    path_constructors: pa2,
                },
            ) => {
                // Names must match
                if n1 != n2 {
                    return Err(TypeError::Mismatch {
                        expected: format!("HIT '{}'", n2).into(),
                        actual: format!("HIT '{}'", n1).into(),
                        span,
                    });
                }

                // Constructor counts must match
                if pc1.len() != pc2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} point constructors", pc2.len()).into(),
                        actual: format!("{} point constructors", pc1.len()).into(),
                        span,
                    });
                }

                if pa1.len() != pa2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} path constructors", pa2.len()).into(),
                        actual: format!("{} path constructors", pa1.len()).into(),
                        span,
                    });
                }

                // Unify parameters
                if p1.len() != p2.len() {
                    return Err(TypeError::Mismatch {
                        expected: format!("{} type parameters", p2.len()).into(),
                        actual: format!("{} type parameters", p1.len()).into(),
                        span,
                    });
                }

                let mut subst = Substitution::new();
                for ((_, t1), (_, t2)) in p1.iter().zip(p2.iter()) {
                    let s =
                        self.unify_inner(&t1.apply_subst(&subst), &t2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }

                Ok(subst)
            }

            // Quantified Types (Quantitative Type Theory)
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .4
            // Track usage: linear (1), affine (0..1), unrestricted (ω)
            (
                Quantified {
                    inner: i1,
                    quantity: q1,
                },
                Quantified {
                    inner: i2,
                    quantity: q2,
                },
            ) => {
                // Quantities must be compatible
                if !Self::quantities_compatible(q1, q2) {
                    return Err(TypeError::Mismatch {
                        expected: "Compatible quantity annotation".into(),
                        actual: "Incompatible quantity annotation".into(),
                        span,
                    });
                }

                // Unify inner types
                self.unify_inner(i1, i2, span)
            }

            // Quantified type can unify with unquantified if quantity is unrestricted
            (Quantified { inner, quantity }, ty) | (ty, Quantified { inner, quantity }) => {
                if *quantity == Quantity::Omega {
                    // Unrestricted quantity can be treated as regular type
                    self.unify_inner(inner, ty, span)
                } else {
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }

            // Existential Types
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .5 - Existential Unification
            //
            // Two existential types unify if their bodies unify after alpha-renaming
            // to use the same bound variable.
            (Exists { var: v1, body: b1 }, Exists { var: v2, body: b2 }) => {
                // Rename v2 to v1 in b2 for alpha equivalence
                let mut rename_subst = Substitution::new();
                rename_subst.insert(*v2, Type::Var(*v1));
                let b2_renamed = b2.apply_subst(&rename_subst);

                // Unify the bodies
                self.unify_inner(b1, &b2_renamed, span)
            }

            // Existential with concrete type: the concrete type becomes the witness
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .5
            (Exists { var, body }, ty) | (ty, Exists { var, body }) => {
                // The concrete type becomes the witness for the existential
                let mut subst = Substitution::new();
                subst.insert(*var, ty.clone());
                let body_subst = body.apply_subst(&subst);

                // Unify the instantiated body with the concrete type
                self.unify_inner(&body_subst, ty, span)
            }

            // Universal Types (Forall)
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — Universal Types
            //
            // Two universal types unify if their bodies unify after alpha-renaming
            (Forall { vars: vs1, body: b1 }, Forall { vars: vs2, body: b2 }) => {
                // Must have same arity
                if vs1.len() != vs2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // Rename vs2 to vs1 in b2 for alpha equivalence
                let mut rename_subst = Substitution::new();
                for (v1, v2) in vs1.iter().zip(vs2.iter()) {
                    rename_subst.insert(*v2, Type::Var(*v1));
                }
                let b2_renamed = b2.apply_subst(&rename_subst);

                // Unify the bodies
                self.unify_inner(b1, &b2_renamed, span)
            }

            // TypeApp - higher-kinded type applications (e.g., F<Int> where F is a type constructor)
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
            (
                Type::TypeApp { constructor: c1, args: a1 },
                Type::TypeApp { constructor: c2, args: a2 },
            ) => {
                // First unify the constructors
                let mut subst = self.unify_inner(c1, c2, span)?;

                // Args must have the same length
                if a1.len() != a2.len() {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // Unify each argument
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    let s = self.unify_inner(&arg1.apply_subst(&subst), &arg2.apply_subst(&subst), span)?;
                    subst = subst.compose(&s);
                }

                Ok(subst)
            }

            // TypeConstructor unification - two type constructors unify if they have the same name and arity
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
            (
                Type::TypeConstructor { name: n1, arity: a1, .. },
                Type::TypeConstructor { name: n2, arity: a2, .. },
            ) => {
                if n1 == n2 && a1 == a2 {
                    Ok(Substitution::new())
                } else {
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }

            // TypeApp with type variable constructor vs Generic - HKT inference
            // When F<A> (TypeApp with Var constructor) is unified with List<Int> (Generic),
            // we infer F = List (TypeConstructor) and A = Int
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types inference
            (
                Type::TypeApp { constructor, args: app_args },
                Type::Generic { name, args: gen_args },
            ) | (
                Type::Generic { name, args: gen_args },
                Type::TypeApp { constructor, args: app_args },
            ) => {
                // Check if constructor is a type variable (HKT inference case)
                if let Type::Var(var) = constructor.as_ref() {
                    if app_args.len() != gen_args.len() {
                        return Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        });
                    }

                    // Bind the type variable to a TypeConstructor
                    let arity = gen_args.len();
                    let kind = Self::kind_for_arity(arity);
                    let type_ctor = Type::TypeConstructor {
                        name: name.clone(),
                        arity,
                        kind,
                    };
                    let mut subst = self.bind_var(*var, &type_ctor, span)?;

                    // Unify the arguments
                    for (app_arg, gen_arg) in app_args.iter().zip(gen_args.iter()) {
                        let s = self.unify_inner(
                            &app_arg.apply_subst(&subst),
                            &gen_arg.apply_subst(&subst),
                            span,
                        )?;
                        subst = subst.compose(&s);
                    }

                    Ok(subst)
                } else {
                    // Constructor is not a variable - check if it matches the Generic name
                    if let Type::TypeConstructor { name: ctor_name, .. } = constructor.as_ref() {
                        if ctor_name == name && app_args.len() == gen_args.len() {
                            let mut subst = Substitution::new();
                            for (app_arg, gen_arg) in app_args.iter().zip(gen_args.iter()) {
                                let s = self.unify_inner(
                                    &app_arg.apply_subst(&subst),
                                    &gen_arg.apply_subst(&subst),
                                    span,
                                )?;
                                subst = subst.compose(&s);
                            }
                            return Ok(subst);
                        }
                    }
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }

            // TypeApp with type variable constructor vs Named - HKT inference
            // Similar to Generic case but for Named types
            (
                Type::TypeApp { constructor, args: app_args },
                Type::Named { path, args: named_args },
            ) | (
                Type::Named { path, args: named_args },
                Type::TypeApp { constructor, args: app_args },
            ) => {
                // Check if constructor is a type variable (HKT inference case)
                if let Type::Var(var) = constructor.as_ref() {
                    if app_args.len() != named_args.len() {
                        return Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        });
                    }

                    // Extract name from path
                    // Note: use verum_common::Text explicitly to avoid conflict with Type::Text
                    let name = path.segments.last().map(|seg| {
                        match seg {
                            verum_ast::ty::PathSegment::Name(ident) => {
                                let s: verum_common::Text = ident.name.clone();
                                s
                            }
                            _ => verum_common::Text::from(""),
                        }
                    }).unwrap_or_else(|| verum_common::Text::from(""));

                    // Bind the type variable to a TypeConstructor
                    let arity = named_args.len();
                    let kind = Self::kind_for_arity(arity);
                    let type_ctor = Type::TypeConstructor {
                        name,
                        arity,
                        kind,
                    };
                    let mut subst = self.bind_var(*var, &type_ctor, span)?;

                    // Unify the arguments
                    for (app_arg, named_arg) in app_args.iter().zip(named_args.iter()) {
                        let s = self.unify_inner(
                            &app_arg.apply_subst(&subst),
                            &named_arg.apply_subst(&subst),
                            span,
                        )?;
                        subst = subst.compose(&s);
                    }

                    Ok(subst)
                } else {
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }

            // ================================================================
            // Bool ↔ Int and Char ↔ Int coercions — REMOVED (semantic honesty)
            // ================================================================
            //
            // Previously this unifier silently accepted `Bool` and `Char` as
            // subtypes of `Int`, meaning `let x: Int = true`, `fn f() -> Int {
            // true }`, and `a: Int + b: Bool` all type-checked cleanly — an
            // enormous blind spot directly contradicting CLAUDE.md's "semantic
            // honesty" principle ("types describe meaning, not implementation")
            // and causing `integration_test::test_type_error_detection` to
            // fail because `fn bad_add(a: Int, b: Bool) -> Int { a + b }` was
            // accepted.
            //
            // The original justification was to enable patterns like
            // `assert_eq(x > 0, 1)` and arithmetic on characters. Those
            // patterns are themselves code smells — they paper over missing
            // explicit conversions. The correct forms are:
            //
            //   - `assert_eq(x > 0, true)`  (compare booleans with booleans)
            //   - `(ch as Int) == 65`       (explicit cast for char)
            //
            // Removing the coercions:
            //   - makes `test_type_error_detection` pass correctly
            //   - restores type safety for Bool and Char at assignment,
            //     return, argument passing, and binary operator sites
            //   - aligns with CLAUDE.md Rule: "Semantic Honesty"
            //   - keeps the Verum refinement system sound (Int refinements
            //     like `Int{>= 0}` can no longer be "satisfied" by a Bool)
            //
            // If a user really needs Bool-as-Int semantics they must write
            // an explicit cast `as Int` or use `if cond { 1 } else { 0 }`.
            //
            // Related: CLAUDE.md section "Semantic Types", the test
            // `crates/verum_compiler/tests/type_error_detection_debug.rs`
            // that documents the full matrix of cases (Int+Bool, Bool+Bool,
            // let x: Int = true, fn f() -> Int { true }, ...), and
            // `crates/verum_compiler/tests/integration_test.rs:113-138`.
            //
            // Note: `(Char, Char)` still unifies (same-type reflexivity) via
            // the general primitive case below; only the cross-type
            // coercions are removed here.

            // CapabilityRestricted types: `T with [C1, C2]`
            // Two capability-restricted types unify if their base types unify
            (CapabilityRestricted { base: b1, .. }, CapabilityRestricted { base: b2, .. }) => {
                self.unify_inner(b1, b2, span)
            }
            // Capability-restricted type unifies with its base type (forgetful upcast)
            (CapabilityRestricted { base, .. }, other) | (other, CapabilityRestricted { base, .. }) => {
                self.unify_inner(base, other, span)
            }

            // Placeholder types: forward-declared recursive types have placeholder inner types
            // that should unify with any concrete type until fully resolved.
            // Example: SegmentError { inner: <placeholder:SegmentError> } unifies with
            //          SegmentError { inner: MmapFailed(...) | MunmapFailed(...) | ... }
            (Placeholder { .. }, _) | (_, Placeholder { .. }) => {
                Ok(Substitution::new())
            }

            // Scalar ↔ Tensor coercion for math interoperability
            // In tensor libraries, scalar types (Float, Int, Bool) and their tensor
            // wrappers (DynTensor<Float>, etc.) are used interchangeably.
            // This bidirectional coercion allows natural math code without explicit wraps.
            // Uses data-driven tensor_family_types set instead of hardcoded name lists.
            // TODO: Replace with TensorLike protocol check once available.
            (Float, Generic { name, args })
            | (Generic { name, args }, Float)
                if args.len() == 1
                    && self.is_tensor_family(name.as_str())
                    && matches!(&args[0], Type::Float | Type::Var(_) | Type::Unknown) =>
            {
                Ok(Substitution::new())
            }
            (Float, Named { path, args })
            | (Named { path, args }, Float)
                if args.len() <= 1 =>
            {
                let pname = path.segments.last().map(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => "",
                }).unwrap_or("");
                if self.is_tensor_family(pname) {
                    Ok(Substitution::new())
                } else {
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }
            // Bool ↔ tensor coercion (for relu_vjp etc.)
            // Uses data-driven tensor_family_types set.
            (Bool, Generic { name, args })
            | (Generic { name, args }, Bool)
                if args.len() == 1
                    && self.is_tensor_family(name.as_str())
                    && matches!(&args[0], Type::Bool | Type::Var(_)) =>
            {
                Ok(Substitution::new())
            }
            // Int ↔ tensor coercion (for tensor indexing/shapes)
            // Uses data-driven tensor_family_types set.
            (Int, Generic { name, args })
            | (Generic { name, args }, Int)
                if args.len() == 1
                    && self.is_tensor_family(name.as_str()) =>
            {
                Ok(Substitution::new())
            }
            // NOTE: Named ↔ Named coercion for tensor types is handled in the main
            // Named match arm above (at the segment/args length mismatch check).

            // Slice ↔ element coercion: [Byte] vs UInt8/Byte
            // FFI functions often use raw bytes where Verum uses byte slices
            (Slice { element }, Named { path, args, .. })
            | (Named { path, args, .. }, Slice { element })
                if args.is_empty() =>
            {
                let pname = path.segments.last().map(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => "",
                }).unwrap_or("");
                if matches!(pname, "UInt8" | "Byte" | "U8" | "u8") {
                    // [Byte] vs UInt8 - allow coercion
                    Ok(Substitution::new())
                } else {
                    // Try to unify the element type with the named type
                    self.unify_inner(element, &Named { path: path.clone(), args: args.clone() }, span).map_err(|_| TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        })
                }
            }

            // Int ↔ Variant coercion for FFI wrappers
            // FFI functions return Int (error codes) while Verum wrappers return
            // success/failure variant types. This uses STRUCTURAL checks on the
            // variant constructor names (not nominal type names) to determine if
            // the variant looks like an error-code wrapper (has Ok/Err constructors)
            // or an optional value (has None/Some constructors).
            // This is stdlib-agnostic: ANY user-defined sum type with these variant
            // names gets the coercion, which is the correct behavior.
            (Int, Variant(variants)) | (Variant(variants), Int) => {
                let has_ok = variants.contains_key(&verum_common::Text::from("Ok"));
                let has_err = variants.contains_key(&verum_common::Text::from("Err"));
                if has_ok || has_err {
                    Ok(Substitution::new())
                } else {
                    let has_none = variants.contains_key(&verum_common::Text::from("None"));
                    let has_some = variants.contains_key(&verum_common::Text::from("Some"));
                    if has_none || has_some {
                        Ok(Substitution::new())
                    } else {
                        Err(TypeError::Mismatch {
                            expected: t2.to_text(),
                            actual: t1.to_text(),
                            span,
                        })
                    }
                }
            }

            // Named type ↔ record structural coercion
            // When a Named type (like SocketAddrV4) is expected but a structural record
            // is provided, allow coercion. Named types are often aliases for records
            // and strict matching causes false positives in cross-module code.
            (Named { path: _, args: named_args, .. }, Record(_))
            | (Record(_), Named { path: _, args: named_args, .. }) if named_args.is_empty() => {
                Ok(Substitution::new())
            }

            // Tuple ↔ Named type coercion for tuple-like newtypes
            (Tuple(_), Named { .. }) | (Named { .. }, Tuple(_)) => {
                // Allow tuple ↔ named type when it looks like a newtype pattern
                // e.g., VisibleRow = (List<Int>, Int, Line, Bool, Bool)
                Ok(Substitution::new())
            }

            // Generic/Named collection ↔ scalar coercion
            // List<USize> vs Int, Range<Int> vs (Maybe<USize>, Maybe<USize>), etc.
            // These arise from type inference through indexing and slicing operations.
            // Uses data-driven indexable_collection_types / range_like_types sets.
            // TODO: Replace with Indexable / RangeLike protocol checks.
            (Generic { name, .. }, Int) | (Int, Generic { name, .. })
                if self.is_indexable_collection(name.as_str()) =>
            {
                Ok(Substitution::new())
            }
            (Generic { name, .. }, Tuple(_)) | (Tuple(_), Generic { name, .. })
                if self.is_range_like(name.as_str()) =>
            {
                Ok(Substitution::new())
            }

            // Future type unification: Future<A> ↔ Future<B>
            // Both sides are the built-in Future type variant
            (Future { output: o1 }, Future { output: o2 }) => {
                self.unify_inner(o1, o2, span)
            }

            // Future cross-representation: Type::Future ↔ Named/Generic { "Future" }
            // async blocks produce Type::Future { output }, while parsed types produce
            // Named { path: "Future", args: [T] } or Generic { name: "Future", args: [T] }
            (Future { output }, Named { path, args })
            | (Named { path, args }, Future { output })
                if args.len() == 1 =>
            {
                let path_name = path.segments.last().map(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                    _ => "",
                }).unwrap_or("");
                if path_name == "Future" {
                    self.unify_inner(output, &args[0], span)
                } else {
                    Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    })
                }
            }
            (Future { output }, Generic { name, args })
            | (Generic { name, args }, Future { output })
                if args.len() == 1 && name.as_str() == "Future" =>
            {
                self.unify_inner(output, &args[0], span)
            }

            // Type mismatch
            _ => Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            }),
        }
    }

    /// Bind a type variable to a type.
    ///
    /// Performs the occurs check to prevent infinite types.
    fn bind_var(&self, var: TypeVar, ty: &Type, span: Span) -> Result<Substitution> {
        // Don't bind to itself
        if let Type::Var(v) = ty
            && *v == var
        {
            return Ok(Substitution::new());
        }

        // Occurs check: var must not appear in ty
        if ty.free_vars().contains(&var) {
            // When the cycle is through a reference or container type (e.g., T = &T,
            // T = List<T>), break the cycle by replacing the recursive occurrence
            // with a fresh type variable. This allows type inference to proceed for
            // common patterns like `let x = &x` or method chains returning &Self.
            match ty {
                Type::Reference { inner, mutable } if inner.free_vars().contains(&var) => {
                    let fresh = Type::Var(TypeVar::fresh());
                    let broken = Type::Reference {
                        inner: Box::new(fresh),
                        mutable: *mutable,
                    };
                    let mut subst = Substitution::new();
                    subst.insert(var, broken);
                    return Ok(subst);
                }
                Type::CheckedReference { inner, mutable } if inner.free_vars().contains(&var) => {
                    let fresh = Type::Var(TypeVar::fresh());
                    let broken = Type::CheckedReference {
                        inner: Box::new(fresh),
                        mutable: *mutable,
                    };
                    let mut subst = Substitution::new();
                    subst.insert(var, broken);
                    return Ok(subst);
                }
                Type::UnsafeReference { inner, mutable } if inner.free_vars().contains(&var) => {
                    let fresh = Type::Var(TypeVar::fresh());
                    let broken = Type::UnsafeReference {
                        inner: Box::new(fresh),
                        mutable: *mutable,
                    };
                    let mut subst = Substitution::new();
                    subst.insert(var, broken);
                    return Ok(subst);
                }
                _ => {
                    return Err(TypeError::InfiniteType {
                        var: var.to_text(),
                        ty: ty.to_text(),
                        span,
                    });
                }
            }
        }

        let mut subst = Substitution::new();
        subst.insert(var, ty.clone());
        Ok(subst)
    }

    // ============================================
    // DEPENDENT TYPE HELPER FUNCTIONS
    // ============================================

    /// Rename bound variable in a type (for alpha-equivalence).
    ///
    /// Used when unifying Pi/Sigma types with different parameter names.
    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Alpha-equivalence for dependent types
    fn rename_bound_var(ty: &Type, from: &Text, to: &Text) -> Type {
        match ty {
            // Pi type: rename in return type if the bound variable matches
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => {
                if param_name == from {
                    // Shadow - don't rename inside
                    ty.clone()
                } else {
                    Type::Pi {
                        param_name: param_name.clone(),
                        param_type: Box::new(Self::rename_bound_var(param_type, from, to)),
                        return_type: Box::new(Self::rename_bound_var(return_type, from, to)),
                    }
                }
            }

            // Sigma type: rename in second type if the bound variable matches
            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => {
                if fst_name == from {
                    // Shadow - don't rename inside
                    ty.clone()
                } else {
                    Type::Sigma {
                        fst_name: fst_name.clone(),
                        fst_type: Box::new(Self::rename_bound_var(fst_type, from, to)),
                        snd_type: Box::new(Self::rename_bound_var(snd_type, from, to)),
                    }
                }
            }

            // Meta types may reference the bound variable
            Type::Meta {
                name,
                ty: inner,
                refinement,
            } => {
                let new_name = if name == from {
                    to.clone()
                } else {
                    name.clone()
                };
                Type::Meta {
                    name: new_name,
                    ty: Box::new(Self::rename_bound_var(inner, from, to)),
                    refinement: refinement.clone(),
                }
            }

            // Function types
            Type::Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| Self::rename_bound_var(p, from, to))
                    .collect(),
                return_type: Box::new(Self::rename_bound_var(return_type, from, to)),
                type_params: type_params.clone(),
                contexts: contexts.clone(),
                properties: properties.clone(),
            },

            // Tuple types
            Type::Tuple(ts) => Type::Tuple(
                ts.iter()
                    .map(|t| Self::rename_bound_var(t, from, to))
                    .collect(),
            ),

            // Array types
            Type::Array { element, size } => Type::Array {
                element: Box::new(Self::rename_bound_var(element, from, to)),
                size: *size,
            },

            // Slice types
            Type::Slice { element } => Type::Slice {
                element: Box::new(Self::rename_bound_var(element, from, to)),
            },

            // Reference types
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::UnsafeReference { mutable, inner } => Type::UnsafeReference {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::Ownership { mutable, inner } => Type::Ownership {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::Pointer { mutable, inner } => Type::Pointer {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::VolatilePointer { mutable, inner } => Type::VolatilePointer {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            // Generic types
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| Self::rename_bound_var(a, from, to))
                    .collect(),
            },

            // Named types
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| Self::rename_bound_var(a, from, to))
                    .collect(),
            },

            // Record types
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
            ),

            // Variant types
            Type::Variant(variants) => Type::Variant(
                variants
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
            ),

            // Refined types
            Type::Refined { base, predicate } => Type::Refined {
                base: Box::new(Self::rename_bound_var(base, from, to)),
                predicate: predicate.clone(),
            },

            // Equality types
            Type::Eq {
                ty: inner,
                lhs,
                rhs,
            } => Type::Eq {
                ty: Box::new(Self::rename_bound_var(inner, from, to)),
                lhs: lhs.clone(),
                rhs: rhs.clone(),
            },

            // Cubical path types - rename in space type; left/right are cubical terms, not types
            Type::PathType { space, left, right } => Type::PathType {
                space: Box::new(Self::rename_bound_var(space, from, to)),
                left: left.clone(),
                right: right.clone(),
            },

            // Partial element types - rename in element_type; face is a cubical term, not a type
            Type::Partial { element_type, face } => Type::Partial {
                element_type: Box::new(Self::rename_bound_var(element_type, from, to)),
                face: face.clone(),
            },

            // Interval type I - no inner types to rename
            Type::Interval => ty.clone(),

            // Inductive types
            Type::Inductive {
                name,
                params,
                indices,
                universe,
                constructors,
            } => Type::Inductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                indices: indices
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                universe: *universe,
                constructors: constructors.clone(),
            },

            // Coinductive types
            Type::Coinductive {
                name,
                params,
                destructors,
            } => Type::Coinductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                destructors: destructors.clone(),
            },

            // Higher inductive types
            Type::HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => Type::HigherInductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                point_constructors: point_constructors.clone(),
                path_constructors: path_constructors.clone(),
            },

            // Quantified types
            Type::Quantified { inner, quantity } => Type::Quantified {
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
                quantity: *quantity,
            },

            // Exists type
            Type::Exists { var, body } => Type::Exists {
                var: *var,
                body: Box::new(Self::rename_bound_var(body, from, to)),
            },

            // Forall type
            Type::Forall { vars, body } => Type::Forall {
                vars: vars.clone(),
                body: Box::new(Self::rename_bound_var(body, from, to)),
            },

            // Future type
            Type::Future { output } => Type::Future {
                output: Box::new(Self::rename_bound_var(output, from, to)),
            },

            // Generator type
            Type::Generator {
                yield_ty,
                return_ty,
            } => Type::Generator {
                yield_ty: Box::new(Self::rename_bound_var(yield_ty, from, to)),
                return_ty: Box::new(Self::rename_bound_var(return_ty, from, to)),
            },

            // Tensor type
            Type::Tensor {
                element,
                shape,
                strides,
                span,
            } => Type::Tensor {
                element: Box::new(Self::rename_bound_var(element, from, to)),
                shape: shape.clone(),
                strides: strides.clone(),
                span: *span,
            },

            // GenRef type
            Type::GenRef { inner } => Type::GenRef {
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            // TypeApp type
            Type::TypeApp { constructor, args } => Type::TypeApp {
                constructor: Box::new(Self::rename_bound_var(constructor, from, to)),
                args: args
                    .iter()
                    .map(|a| Self::rename_bound_var(a, from, to))
                    .collect(),
            },

            // Primitive and other types - no renaming needed
            Type::Unit
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Never
            | Type::Var(_)
            | Type::Universe { .. }
            | Type::Prop
            | Type::Lifetime { .. }
            | Type::TypeConstructor { .. }
            | Type::Placeholder { .. } => ty.clone(),

            // ExtensibleRecord - rename in fields
            Type::ExtensibleRecord { fields, row_var } => Type::ExtensibleRecord {
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
                row_var: *row_var,
            },

            // CapabilityRestricted - rename in base type
            Type::CapabilityRestricted { base, capabilities } => Type::CapabilityRestricted {
                base: Box::new(Self::rename_bound_var(base, from, to)),
                capabilities: capabilities.clone(),
            },

            // Unknown type - no inner types to rename
            Type::Unknown => ty.clone(),

            // DynProtocol - rename in associated type bindings
            Type::DynProtocol { bounds, bindings } => Type::DynProtocol {
                bounds: bounds.clone(),
                bindings: bindings
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
            },
        }
    }

    /// Check if two equality type terms are definitionally equal.
    ///
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — Equality types
    fn eq_terms_equal(t1: &EqTerm, t2: &EqTerm) -> bool {
        match (t1, t2) {
            (EqTerm::Var(v1), EqTerm::Var(v2)) => v1 == v2,
            (EqTerm::Const(c1), EqTerm::Const(c2)) => c1 == c2,
            (EqTerm::App { func: f1, args: a1 }, EqTerm::App { func: f2, args: a2 }) => {
                Self::eq_terms_equal(f1, f2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(x, y)| Self::eq_terms_equal(x, y))
            }
            (
                EqTerm::Lambda {
                    param: p1,
                    body: b1,
                },
                EqTerm::Lambda {
                    param: p2,
                    body: b2,
                },
            ) => {
                // Alpha-equivalence for lambda terms
                if p1 == p2 {
                    Self::eq_terms_equal(b1, b2)
                } else {
                    // Rename p2 to p1 in b2 and compare
                    let b2_renamed = Self::rename_eq_term(b2, p2, p1);
                    Self::eq_terms_equal(b1, &b2_renamed)
                }
            }
            (
                EqTerm::Proj {
                    pair: pair1,
                    component: c1,
                },
                EqTerm::Proj {
                    pair: pair2,
                    component: c2,
                },
            ) => c1 == c2 && Self::eq_terms_equal(pair1, pair2),
            (EqTerm::Refl(inner1), EqTerm::Refl(inner2)) => Self::eq_terms_equal(inner1, inner2),
            (
                EqTerm::J {
                    proof: p1,
                    motive: m1,
                    base: b1,
                },
                EqTerm::J {
                    proof: p2,
                    motive: m2,
                    base: b2,
                },
            ) => {
                Self::eq_terms_equal(p1, p2)
                    && Self::eq_terms_equal(m1, m2)
                    && Self::eq_terms_equal(b1, b2)
            }
            _ => false,
        }
    }

    /// Rename a variable in an equality term (for alpha-equivalence).
    fn rename_eq_term(term: &EqTerm, from: &Text, to: &Text) -> EqTerm {
        match term {
            EqTerm::Var(v) => {
                if v == from {
                    EqTerm::Var(to.clone())
                } else {
                    term.clone()
                }
            }
            EqTerm::Const(_) => term.clone(),
            EqTerm::App { func, args } => EqTerm::App {
                func: Box::new(Self::rename_eq_term(func, from, to)),
                args: args
                    .iter()
                    .map(|a| Self::rename_eq_term(a, from, to))
                    .collect(),
            },
            EqTerm::Lambda { param, body } => {
                if param == from {
                    // Shadow - don't rename inside
                    term.clone()
                } else {
                    EqTerm::Lambda {
                        param: param.clone(),
                        body: Box::new(Self::rename_eq_term(body, from, to)),
                    }
                }
            }
            EqTerm::Proj { pair, component } => EqTerm::Proj {
                pair: Box::new(Self::rename_eq_term(pair, from, to)),
                component: *component,
            },
            EqTerm::Refl(inner) => EqTerm::Refl(Box::new(Self::rename_eq_term(inner, from, to))),
            EqTerm::J {
                proof,
                motive,
                base,
            } => EqTerm::J {
                proof: Box::new(Self::rename_eq_term(proof, from, to)),
                motive: Box::new(Self::rename_eq_term(motive, from, to)),
                base: Box::new(Self::rename_eq_term(base, from, to)),
            },
        }
    }

    /// Check if two universe levels can unify.
    ///
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe hierarchy
    fn universe_levels_unify(l1: &UniverseLevel, l2: &UniverseLevel) -> bool {
        match (l1, l2) {
            // Concrete levels must match exactly
            (UniverseLevel::Concrete(n1), UniverseLevel::Concrete(n2)) => n1 == n2,

            // Level variables unify (constraint recorded elsewhere)
            (UniverseLevel::Variable(_), _) | (_, UniverseLevel::Variable(_)) => true,

            // Max of levels - both components must match
            (UniverseLevel::Max(a1, b1), UniverseLevel::Max(a2, b2)) => a1 == a2 && b1 == b2,

            // Successor levels must have same base
            (UniverseLevel::Succ(n1), UniverseLevel::Succ(n2)) => n1 == n2,

            _ => false,
        }
    }

    /// Check if two quantities are compatible for unification.
    ///
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .4 - Quantitative Type Theory
    fn quantities_compatible(q1: &Quantity, q2: &Quantity) -> bool {
        match (q1, q2) {
            // Same quantities always compatible
            (Quantity::Zero, Quantity::Zero)
            | (Quantity::One, Quantity::One)
            | (Quantity::Omega, Quantity::Omega) => true,

            // Erased (0) is subquantity of everything
            (Quantity::Zero, _) => true,

            // Linear (1) is subquantity of unrestricted (ω)
            (Quantity::One, Quantity::Omega) => true,

            // AtMost(n) is compatible if n1 <= n2
            (Quantity::AtMost(n1), Quantity::AtMost(n2)) => n1 <= n2,
            (Quantity::AtMost(_), Quantity::Omega) => true,
            (Quantity::One, Quantity::AtMost(n)) => *n >= 1,

            // Graded quantities must have same parameter
            (Quantity::Graded(n1), Quantity::Graded(n2)) => n1 == n2,

            // Other combinations don't unify directly
            _ => false,
        }
    }

    // ============================================
    // PROJECTION NORMALIZATION SUPPORT
    // ============================================

    /// Check if a type contains a projection (associated type access)
    ///
    /// Projections are represented as `Type::Generic { name: "T.Item", ... }`
    /// or `Type::Generic { name: "T::Item", ... }`.
    ///
    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    pub fn contains_projection(ty: &Type) -> bool {
        match ty {
            Type::Generic { name, args } => {
                // Check if this type name looks like a projection
                let is_projection = name.contains(".") || name.contains("::");

                // Also check args for nested projections
                is_projection || args.iter().any(Self::contains_projection)
            }

            // Recursively check compound types
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(Self::contains_projection)
                    || Self::contains_projection(return_type)
            }

            Type::Named { args, .. } => args.iter().any(Self::contains_projection),

            Type::Tuple(types) => types.iter().any(Self::contains_projection),

            Type::Array { element, .. } => Self::contains_projection(element),

            Type::Slice { element } => Self::contains_projection(element),

            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => Self::contains_projection(inner),

            Type::Refined { base, .. } => Self::contains_projection(base),

            Type::Future { output } => Self::contains_projection(output),

            Type::GenRef { inner } => Self::contains_projection(inner),

            Type::TypeApp { constructor, args } => {
                Self::contains_projection(constructor) || args.iter().any(Self::contains_projection)
            }

            _ => false,
        }
    }

    /// Unify with projection normalization
    ///
    /// This method attempts to normalize any projections in the types before
    /// unifying. If normalization fails (e.g., because a type variable is not
    /// yet resolved), it defers the projection and continues with regular unification.
    ///
    /// # Arguments
    ///
    /// * `t1` - First type to unify
    /// * `t2` - Second type to unify
    /// * `protocol_checker` - Protocol checker for resolving projections
    /// * `span` - Source span for error messages
    ///
    /// # Returns
    ///
    /// * `Ok((subst, deferred))` - Substitution and any deferred projections
    /// * `Err(TypeError)` - Unification failed
    ///
    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    pub fn unify_with_projections(
        &mut self,
        t1: &Type,
        t2: &Type,
        protocol_checker: &crate::protocol::ProtocolChecker,
        span: Span,
    ) -> Result<(Substitution, Vec<crate::projection::DeferredProjection>)> {
        use crate::projection::{ProjectionResolver, ProjectionResult};

        let mut deferred = Vec::new();
        let resolver = ProjectionResolver::new(protocol_checker, span);

        // Try to normalize t1
        let t1_normalized = if Self::contains_projection(t1) {
            match resolver.normalize(t1) {
                Ok(normalized) => normalized,
                Err(_) => t1.clone(), // Keep original on error
            }
        } else {
            t1.clone()
        };

        // Try to normalize t2
        let t2_normalized = if Self::contains_projection(t2) {
            match resolver.normalize(t2) {
                Ok(normalized) => normalized,
                Err(_) => t2.clone(), // Keep original on error
            }
        } else {
            t2.clone()
        };

        // Now unify the (possibly normalized) types
        let subst = self.unify(&t1_normalized, &t2_normalized, span)?;

        // Check for remaining projections that couldn't be normalized
        // These become deferred constraints
        if Self::contains_projection(&t1_normalized) {
            if let Some(proj) = crate::projection::parse_projection(&t1_normalized, span) {
                match resolver.resolve_projection(&proj) {
                    Ok(ProjectionResult::Deferred(d)) => deferred.push(d),
                    _ => {} // Already resolved or error
                }
            }
        }

        if Self::contains_projection(&t2_normalized) {
            if let Some(proj) = crate::projection::parse_projection(&t2_normalized, span) {
                match resolver.resolve_projection(&proj) {
                    Ok(ProjectionResult::Deferred(d)) => deferred.push(d),
                    _ => {} // Already resolved or error
                }
            }
        }

        Ok((subst, deferred))
    }

    /// Normalize a type by resolving all projections
    ///
    /// This is a convenience method that wraps the ProjectionResolver.
    ///
    /// # Arguments
    ///
    /// * `ty` - Type to normalize
    /// * `protocol_checker` - Protocol checker for resolving projections
    /// * `span` - Source span for error messages
    ///
    /// # Returns
    ///
    /// The normalized type, or the original type if normalization fails.
    pub fn normalize_projections(
        ty: &Type,
        protocol_checker: &crate::protocol::ProtocolChecker,
        span: Span,
    ) -> Type {
        if !Self::contains_projection(ty) {
            return ty.clone();
        }

        let resolver = crate::projection::ProjectionResolver::new(protocol_checker, span);
        resolver.normalize(ty).unwrap_or_else(|_| ty.clone())
    }

    /// Check if a type lives in the Prop universe (proof-irrelevant).
    /// Inductive types: recursive type definitions with structural recursion, termination checking — .1
    ///
    /// Returns true if the type's universe is Prop, meaning all inhabitants
    /// are definitionally equal (proof irrelevance).
    ///
    /// # Examples
    /// - Prop itself is in Prop (but we check the type-of)
    /// - Equality types Eq<A, x, y> can be in Prop
    /// - Inductive types declared in Prop universe
    /// - Sigma types with Prop in the second component
    fn is_type_in_prop(&self, ty: &Type) -> bool {
        match ty {
            // Prop itself is proof-irrelevant
            Type::Prop => true,

            // Equality types are often in Prop
            // (they live in the same universe as their carrier type)
            Type::Eq { ty: carrier_ty, .. } => self.is_type_in_prop(carrier_ty),

            // Inductive types: check if they're declared in a Prop-level universe
            // For simplicity, we check if universe level is 0 and named with Prop convention
            Type::Inductive { universe, .. } => {
                matches!(universe, UniverseLevel::Concrete(0))
            }

            // Sigma types with Prop in second component
            // (x: A, P(x)) where P(x) : Prop
            Type::Sigma { snd_type, .. } => self.is_type_in_prop(snd_type),

            // Types explicitly marked as proof-irrelevant via quantification
            Type::Quantified {
                quantity: Quantity::Zero,
                ..
            } => true,

            // Refinement types: check the base type
            Type::Refined { base, .. } => self.is_type_in_prop(base),

            // Other types are not proof-irrelevant
            _ => false,
        }
    }
}

impl Default for Unifier {
    fn default() -> Self {
        Self::new()
    }
}

// Tests moved to tests/unify_tests.rs
