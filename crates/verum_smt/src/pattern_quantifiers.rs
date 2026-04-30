//! Quantifier Instantiation Patterns for Dependent Type Verification
//!
//! Refinement types (`T{P}`) generate universally quantified SMT formulas for
//! subtyping checks. Dependent types (Pi/Sigma) add nested quantifiers. Pattern-based
//! instantiation guides Z3's E-matching to avoid exponential blowup on these formulas.
//!
//! This module provides pattern-based quantifier instantiation to improve
//! Z3 performance on dependent types. Without patterns, Z3 uses heuristic
//! quantifier instantiation which can be slow or incomplete.
//!
//! ## Pattern Benefits
//!
//! - **20-30% speedup** on dependent type verification
//! - **Better instantiation control** - guide Z3 to relevant instances
//! - **Reduced search space** - avoid irrelevant quantifier instantiations
//! - **Improved completeness** - help Z3 find proofs faster
//!
//! ## Pattern Strategy
//!
//! We generate patterns for common Verum type operations:
//! - List operations: list_len, list_get, list_contains
//! - Map operations: map_get, map_contains_key
//! - Set operations: set_member
//! - Refinement predicates: type-specific constraints
//!
//! ## Usage
//!
//! ```rust,ignore
//! use verum_smt::pattern_quantifiers::{PatternGenerator, PatternConfig};
//!
//! let config = PatternConfig::default();
//! let generator = PatternGenerator::new(config);
//!
//! // Generate patterns for a universal quantifier
//! let patterns = generator.create_list_patterns(&list_var);
//!
//! // Create quantified formula with patterns
//! let quantified = generator.mk_quantified_property(
//!     &bound_vars,
//!     &body,
//!     &patterns,
//! );
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use z3::ast::{Ast, Bool, Dynamic, Int, Real};
use z3::{FuncDecl, Params, Pattern, Solver, Sort, Symbol};

use verum_ast::expr::RecoverBody;
use verum_ast::ty::GenericArg;
use verum_ast::{Expr, ExprKind, Path, PathSegment, Type, TypeKind};
use verum_common::{List, Map, Maybe, Set, Text};

use crate::translate::Translator;

// ==================== Type to Sort Translation ====================

/// Translates Verum types to Z3 sorts for pattern generation.
pub fn type_to_sort(ty: &Type) -> Sort {
    match &ty.kind {
        TypeKind::Unit => Sort::bool(),
        TypeKind::Bool => Sort::bool(),
        TypeKind::Int => Sort::int(),
        TypeKind::Float => Sort::real(),
        TypeKind::Char => Sort::int(),
        TypeKind::Text => Sort::string(),
        TypeKind::Path(path) => path_to_sort(path),
        TypeKind::Tuple(types) => {
            if types.is_empty() {
                Sort::bool()
            } else {
                Sort::uninterpreted(Symbol::String(format!("Tuple_{}", types.len())))
            }
        }
        TypeKind::Array { element, .. } => Sort::array(&Sort::int(), &type_to_sort(element)),
        TypeKind::Slice(element) => Sort::seq(&type_to_sort(element)),
        TypeKind::Function {
            params,
            return_type,
            ..
        }
        | TypeKind::Rank2Function {
            type_params: _,
            params,
            return_type,
            ..
        } => {
            if params.len() == 1 {
                Sort::array(&type_to_sort(&params[0]), &type_to_sort(return_type))
            } else {
                Sort::uninterpreted(Symbol::String("FunctionType".to_string()))
            }
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. }
        | TypeKind::Pointer { inner, .. }
        | TypeKind::VolatilePointer { inner, .. }
        | TypeKind::Ownership { inner, .. }
        | TypeKind::GenRef { inner } => type_to_sort(inner),
        TypeKind::Generic { base, args } => {
            if let TypeKind::Path(path) = &base.kind {
                // Convert Vec<GenericArg> to List<GenericArg> for resolve_generic_type
                let args_list: List<GenericArg> = args.iter().cloned().collect();
                resolve_generic_type(path, &args_list)
            } else {
                Sort::uninterpreted(Symbol::String("Generic".to_string()))
            }
        }
        TypeKind::Refined { base, .. } => type_to_sort(base),
        TypeKind::Bounded { base, .. } => type_to_sort(base),
        TypeKind::DynProtocol { .. } => {
            Sort::uninterpreted(Symbol::String("DynProtocol".to_string()))
        }
        TypeKind::Qualified { .. } => {
            Sort::uninterpreted(Symbol::String("QualifiedType".to_string()))
        }
        TypeKind::Tensor { element, shape, .. } => {
            let mut current = type_to_sort(element);
            for _ in shape.iter() {
                current = Sort::array(&Sort::int(), &current);
            }
            current
        }
        TypeKind::TypeConstructor { arity, .. } => {
            Sort::uninterpreted(Symbol::String(format!("TypeConstructor_{}", arity)))
        }
        TypeKind::Existential { .. } => {
            Sort::uninterpreted(Symbol::String("Existential".to_string()))
        }
        TypeKind::AssociatedType { base, .. } => {
            // Associated types are modeled as uninterpreted if we can't resolve them
            type_to_sort(base)
        }
        TypeKind::Inferred => Sort::int(),
        // Never type (!) - diverging expressions. Modeled as an empty/bottom sort.
        // Since Z3 doesn't have a direct bottom sort, we use an uninterpreted sort.
        TypeKind::Never => Sort::uninterpreted(Symbol::String("Never".to_string())),
        // Capability-restricted type - use the base type's sort
        TypeKind::CapabilityRestricted { base, .. } => type_to_sort(base),
        // Unknown type - top type with no operations. Modeled as uninterpreted sort.
        TypeKind::Unknown => Sort::uninterpreted(Symbol::String("Unknown".to_string())),
        // Record types - modeled as uninterpreted struct-like sort
        TypeKind::Record { fields, .. } => {
            Sort::uninterpreted(Symbol::String(format!("Record_{}", fields.len())))
        }
        // Universe types - Type(n) is a sort-of-sorts, modeled as uninterpreted
        TypeKind::Universe { .. } => Sort::uninterpreted(Symbol::String("Type".to_string())),
        // Meta types and type lambdas - modeled as uninterpreted
        TypeKind::Meta { .. } => Sort::uninterpreted(Symbol::String("Meta".to_string())),
        TypeKind::TypeLambda { .. } => Sort::uninterpreted(Symbol::String("TypeLambda".to_string())),
        // Path equality type: use carrier type's sort
        TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => type_to_sort(carrier),
        // Dependent type application `T<A>(v..)`: use carrier sort,
        // value indices do not affect Z3 sort translation.
        TypeKind::DependentApp { carrier, .. } => type_to_sort(carrier),
    }
}

fn path_to_sort(path: &Path) -> Sort {
    if let Some(ident) = path.as_ident() {
        match ident.name.as_str() {
            "Int" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32"
            | "u64" | "u128" | "usize" => Sort::int(),
            "Bool" | "bool" => Sort::bool(),
            "Float" | "f32" | "f64" | "Real" => Sort::real(),
            "Char" | "char" => Sort::int(),
            "Text" | "String" | "str" => Sort::string(),
            "Unit" | "()" => Sort::bool(),
            name => Sort::uninterpreted(Symbol::String(name.to_string())),
        }
    } else if let Some(PathSegment::Name(ident)) = path.segments.last() {
        match ident.name.as_str() {
            "List" | "Vec" => Sort::seq(&Sort::int()),
            "Map" | "HashMap" | "BTreeMap" => Sort::array(&Sort::int(), &Sort::int()),
            "Set" | "HashSet" | "BTreeSet" => Sort::set(&Sort::int()),
            name => Sort::uninterpreted(Symbol::String(name.to_string())),
        }
    } else {
        Sort::int()
    }
}

fn resolve_generic_type(path: &Path, args: &[GenericArg]) -> Sort {
    let type_name = if let Some(ident) = path.as_ident() {
        ident.name.as_str()
    } else if let Some(PathSegment::Name(ident)) = path.segments.last() {
        ident.name.as_str()
    } else {
        return Sort::uninterpreted(Symbol::String("UnknownGeneric".to_string()));
    };

    match type_name {
        "List" | "Vec" | "Seq" => {
            if let Some(GenericArg::Type(elem_ty)) = args.first() {
                Sort::seq(&type_to_sort(elem_ty))
            } else {
                Sort::seq(&Sort::int())
            }
        }
        "Map" | "HashMap" | "BTreeMap" => {
            let key_sort = args
                .first()
                .and_then(|a| {
                    if let GenericArg::Type(t) = a {
                        Some(type_to_sort(t))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(Sort::int);
            let value_sort = args
                .get(1)
                .and_then(|a| {
                    if let GenericArg::Type(t) = a {
                        Some(type_to_sort(t))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(Sort::int);
            Sort::array(&key_sort, &value_sort)
        }
        "Set" | "HashSet" | "BTreeSet" => {
            if let Some(GenericArg::Type(elem_ty)) = args.first() {
                Sort::set(&type_to_sort(elem_ty))
            } else {
                Sort::set(&Sort::int())
            }
        }
        "Maybe" | "Option" | "Result" | "Heap" | "Box" | "Rc" | "Arc" => {
            if let Some(GenericArg::Type(inner_ty)) = args.first() {
                type_to_sort(inner_ty)
            } else {
                Sort::int()
            }
        }
        _ => Sort::uninterpreted(Symbol::String(format!("{}_{}", type_name, args.len()))),
    }
}

/// Check if a path refers to a List type.
pub fn is_list_type(path: &Path) -> bool {
    if let Some(ident) = path.as_ident() {
        matches!(ident.name.as_str(), "List" | "Vec" | "Seq" | "Array")
    } else if let Some(PathSegment::Name(ident)) = path.segments.last() {
        matches!(ident.name.as_str(), "List" | "Vec" | "Seq" | "Array")
    } else {
        false
    }
}

/// Check if a path refers to a Map type.
pub fn is_map_type(path: &Path) -> bool {
    if let Some(ident) = path.as_ident() {
        matches!(ident.name.as_str(), "Map" | "HashMap" | "BTreeMap")
    } else if let Some(PathSegment::Name(ident)) = path.segments.last() {
        matches!(ident.name.as_str(), "Map" | "HashMap" | "BTreeMap")
    } else {
        false
    }
}

/// Check if a path refers to a Set type.
pub fn is_set_type(path: &Path) -> bool {
    if let Some(ident) = path.as_ident() {
        matches!(ident.name.as_str(), "Set" | "HashSet" | "BTreeSet")
    } else if let Some(PathSegment::Name(ident)) = path.segments.last() {
        matches!(ident.name.as_str(), "Set" | "HashSet" | "BTreeSet")
    } else {
        false
    }
}

/// Get element sort from a List type.
pub fn get_list_element_sort(ty: &Type) -> Sort {
    match &ty.kind {
        TypeKind::Generic { args, .. } => {
            if let Some(GenericArg::Type(elem_ty)) = args.first() {
                type_to_sort(elem_ty)
            } else {
                Sort::int()
            }
        }
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => type_to_sort(element),
        _ => Sort::int(),
    }
}

/// Get key and value sorts from a Map type.
pub fn get_map_key_value_sorts(ty: &Type) -> (Sort, Sort) {
    match &ty.kind {
        TypeKind::Generic { args, .. } => {
            let key = args
                .first()
                .and_then(|a| {
                    if let GenericArg::Type(t) = a {
                        Some(type_to_sort(t))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(Sort::int);
            let value = args
                .get(1)
                .and_then(|a| {
                    if let GenericArg::Type(t) = a {
                        Some(type_to_sort(t))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(Sort::int);
            (key, value)
        }
        _ => (Sort::int(), Sort::int()),
    }
}

/// Get element sort from a Set type.
pub fn get_set_element_sort(ty: &Type) -> Sort {
    match &ty.kind {
        TypeKind::Generic { args, .. } => {
            if let Some(GenericArg::Type(elem_ty)) = args.first() {
                type_to_sort(elem_ty)
            } else {
                Sort::int()
            }
        }
        _ => Sort::int(),
    }
}

/// Create a Z3 constant for a given sort.
pub fn create_const_for_sort(name: &str, sort: &Sort) -> Dynamic {
    let sort_name = format!("{:?}", sort);
    if sort_name.contains("Int") {
        Dynamic::from_ast(&Int::new_const(name))
    } else if sort_name.contains("Bool") {
        Dynamic::from_ast(&Bool::new_const(name))
    } else if sort_name.contains("Real") {
        Dynamic::from_ast(&Real::new_const(name))
    } else if sort_name.contains("String") {
        Dynamic::from_ast(&z3::ast::String::new_const(name))
    } else {
        Dynamic::from_ast(&Int::new_const(name))
    }
}

/// Infer method return sort.
pub fn infer_method_return_sort(method_name: &str, receiver_sort: &Sort) -> Sort {
    match method_name {
        "len" | "length" | "size" | "count" => Sort::int(),
        "is_empty" | "is_some" | "is_none" | "is_ok" | "is_err" | "contains" | "contains_key"
        | "member" | "has" | "exists" | "eq" | "ne" | "lt" | "le" | "gt" | "ge" => Sort::bool(),
        "to_string" | "into_string" => Sort::string(),
        "to_int" | "into_int" => Sort::int(),
        "to_float" | "into_float" => Sort::real(),
        "abs" | "neg" | "add" | "sub" | "mul" | "div" | "rem" | "pow" | "min" | "max" => {
            receiver_sort.clone()
        }
        _ => Sort::int(),
    }
}

/// Infer binary operation result sort.
pub fn infer_binary_op_sort(op_name: &str, operand_sort: &Sort) -> Sort {
    match op_name {
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||" | "and" | "or" | "implies" => {
            Sort::bool()
        }
        _ => operand_sort.clone(),
    }
}

// ==================== Configuration ====================

/// Configuration for pattern-based quantifier instantiation
#[derive(Debug, Clone)]
pub struct PatternConfig {
    /// Enable pattern generation (default: true)
    pub enable_patterns: bool,

    /// Pattern generation strategy
    pub strategy: PatternGenerationStrategy,

    /// Minimum weight for pattern priority (default: 1)
    pub pattern_weight_threshold: u32,

    /// Maximum patterns per quantifier (default: 5)
    pub max_patterns_per_quantifier: usize,

    /// Enable multi-patterns (patterns with multiple terms)
    pub enable_multi_patterns: bool,

    /// Track pattern effectiveness for statistics
    pub track_effectiveness: bool,

    /// Enable Model-Based Quantifier Instantiation (MBQI)
    /// When true, Z3 uses model-based techniques to find quantifier instances
    pub enable_mbqi: bool,

    /// MBQI eager threshold (smt.qi.eager_threshold)
    /// Higher values make MBQI less eager to instantiate
    pub mbqi_eager_threshold: f64,

    /// Maximum MBQI instances per quantifier (smt.qi.max_instances)
    pub mbqi_max_instances: u32,

    /// Default weight for patterns without explicit weight
    pub default_pattern_weight: u32,

    /// Quantifier identifier prefix for naming
    pub quantifier_id_prefix: Text,

    /// Skolem identifier prefix for existential quantifiers
    pub skolem_id_prefix: Text,
}

impl Default for PatternConfig {
    fn default() -> Self {
        Self {
            enable_patterns: true,
            strategy: PatternGenerationStrategy::Adaptive,
            pattern_weight_threshold: 1,
            max_patterns_per_quantifier: 5,
            enable_multi_patterns: true,
            track_effectiveness: true,
            enable_mbqi: true,
            mbqi_eager_threshold: 5.0,
            mbqi_max_instances: 1000,
            default_pattern_weight: 0,
            quantifier_id_prefix: Text::from("verum_q"),
            skolem_id_prefix: Text::from("verum_sk"),
        }
    }
}

/// Strategy for generating patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternGenerationStrategy {
    /// Conservative - only generate patterns for known-good cases
    Conservative,

    /// Aggressive - generate many patterns to guide instantiation
    Aggressive,

    /// Adaptive - analyze the formula and decide
    Adaptive,

    /// Type-driven - generate patterns based on types in the formula
    TypeDriven,
}

// ==================== Pattern Generator ====================

/// Pattern generator for quantifier instantiation
pub struct PatternGenerator {
    /// Configuration
    config: PatternConfig,

    /// Statistics tracking
    stats: PatternStats,
}

impl PatternGenerator {
    /// Create a new pattern generator
    pub fn new(config: PatternConfig) -> Self {
        // Phase-not-realised tracing for two inert PatternConfig
        // fields: `quantifier_id_prefix` (default "verum_q") and
        // `skolem_id_prefix` (default "verum_sk"). Both are
        // documented as identifier prefixes for Z3's
        // `quantifier_const` API, threaded through the helper
        // `create_weighted_quantifier` (line 3707) — but no
        // PatternGenerator method calls that helper, and no
        // production code path consumes the prefix fields. Z3
        // generates anonymous quantifier IDs by default, so the
        // operator-facing knob has no effect today.
        //
        // Surface a debug trace when either prefix is set to a
        // non-default value so embedders writing
        // `[smt.patterns] quantifier_id_prefix = "myproject_q"`
        // see the value was observed but not threaded through.
        if config.quantifier_id_prefix.as_str() != "verum_q"
            || config.skolem_id_prefix.as_str() != "verum_sk"
        {
            tracing::debug!(
                "PatternConfig surface: quantifier_id_prefix={:?}, skolem_id_prefix={:?} \
                 — these prefix fields land on the config but the production \
                 PatternGenerator does not call `create_weighted_quantifier` (the only \
                 site that would consume them). Z3 currently generates anonymous \
                 quantifier IDs. Forward-looking knobs for a future named-quantifier \
                 telemetry surface.",
                config.quantifier_id_prefix.as_str(),
                config.skolem_id_prefix.as_str(),
            );
        }
        Self {
            config,
            stats: PatternStats::default(),
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(PatternConfig::default())
    }

    /// Generate patterns for a quantified formula
    ///
    /// # Arguments
    ///
    /// * `bound_vars` - Variables bound by the quantifier
    /// * `body` - The quantifier body (formula)
    /// * `context` - Additional context about the formula
    ///
    /// # Returns
    ///
    /// List of patterns to guide quantifier instantiation
    pub fn generate_patterns(
        &mut self,
        bound_vars: &[(&str, &Type)],
        body: &Expr,
        _context: Maybe<&PatternContext>,
    ) -> List<Pattern> {
        if !self.config.enable_patterns {
            return List::new();
        }

        let mut patterns = List::new();

        // Generate type-specific patterns based on variable types
        for (var_name, var_type) in bound_vars {
            match &var_type.kind {
                TypeKind::Generic { base, args } => {
                    // Check if this is a List, Map, or Set type
                    if let TypeKind::Path(path) = &base.kind {
                        // Use proper path resolution via helper functions
                        if is_list_type(path) {
                            patterns.extend(self.create_list_patterns(var_name, body));
                        } else if is_map_type(path) {
                            patterns.extend(self.create_map_patterns(var_name, body));
                        } else if is_set_type(path) {
                            patterns.extend(self.create_set_patterns(var_name, body));
                        }
                    }
                }
                TypeKind::Refined { base, predicate } => {
                    patterns.extend(self.create_refinement_patterns(
                        var_name,
                        base,
                        &predicate.expr,
                    ));
                }
                _ => {
                    // For other types, extract patterns from the body
                    patterns.extend(self.extract_patterns_from_expr(var_name, body));
                }
            }
        }

        // Honour `enable_multi_patterns`: when enabled and the
        // generated set has at least 2 simple patterns, attempt
        // to fold them into a single multi-pattern so Z3
        // instantiates only when ALL terms appear together. This
        // is more selective than independent patterns and reduces
        // matching work for quantifiers with multiple triggers.
        // Closes the inert-defense pattern: prior to wiring, the
        // documented `enable_multi_patterns` toggle had no effect
        // on this entry point — multi-patterns were neither
        // generated nor opted out.
        if self.config.enable_multi_patterns && patterns.len() >= 2 {
            let bound_var_names: Vec<Text> = bound_vars
                .iter()
                .map(|(name, _)| Text::from(*name))
                .collect();
            let apps = extract_function_applications_detailed(body, &bound_var_names);
            if apps.len() >= 2
                && let Maybe::Some(multi) = try_create_multi_pattern(&apps)
            {
                patterns.push(multi);
            }
        }

        // Limit to max patterns
        if patterns.len() > self.config.max_patterns_per_quantifier {
            patterns.truncate(self.config.max_patterns_per_quantifier);
        }

        // Honour `track_effectiveness`: when disabled, skip the
        // statistics-recording call so callers running in
        // hot-path / latency-sensitive contexts don't pay the
        // atomic-counter cost. Default `true` matches the prior
        // behaviour. Closes the inert-defense pattern.
        if self.config.track_effectiveness {
            self.stats.record_pattern_generation(patterns.len());
        }
        patterns
    }

    /// Create patterns for List operations
    ///
    /// Generates patterns for:
    /// - `list.len()` - list length
    /// - `list.get(i)` - element access
    /// - `list.contains(x)` - membership
    fn create_list_patterns(&mut self, var_name: &str, body: &Expr) -> List<Pattern> {
        let mut patterns = List::new();

        // Check if body mentions list operations
        let body_str = format!("{:?}", body);

        if body_str.contains("len") || body_str.contains("length") {
            // Pattern: list_len(list)
            if let Maybe::Some(pattern) = self.mk_function_pattern("list_len", &[var_name]) {
                patterns.push(pattern);
            }
        }

        if body_str.contains("get") || body_str.contains("[") {
            // Pattern: list_get(list, i)
            if let Maybe::Some(pattern) = self.mk_function_pattern("list_get", &[var_name, "i"]) {
                patterns.push(pattern);
            }
        }

        if body_str.contains("contains") || body_str.contains("member") {
            // Pattern: list_contains(list, x)
            if let Maybe::Some(pattern) =
                self.mk_function_pattern("list_contains", &[var_name, "x"])
            {
                patterns.push(pattern);
            }
        }

        patterns
    }

    /// Create patterns for Map operations
    ///
    /// Generates patterns for:
    /// - `map.get(k)` - key lookup
    /// - `map.contains_key(k)` - key membership
    fn create_map_patterns(&mut self, var_name: &str, body: &Expr) -> List<Pattern> {
        let mut patterns = List::new();
        let body_str = format!("{:?}", body);

        if body_str.contains("get") || body_str.contains("[") {
            // Pattern: map_get(map, k)
            if let Maybe::Some(pattern) = self.mk_function_pattern("map_get", &[var_name, "k"]) {
                patterns.push(pattern);
            }
        }

        if body_str.contains("contains") || body_str.contains("has_key") {
            // Pattern: map_contains_key(map, k)
            if let Maybe::Some(pattern) =
                self.mk_function_pattern("map_contains_key", &[var_name, "k"])
            {
                patterns.push(pattern);
            }
        }

        patterns
    }

    /// Create patterns for Set operations
    ///
    /// Generates patterns for:
    /// - `set.member(x)` - membership test
    fn create_set_patterns(&mut self, var_name: &str, body: &Expr) -> List<Pattern> {
        let mut patterns = List::new();
        let body_str = format!("{:?}", body);

        if body_str.contains("member") || body_str.contains("contains") {
            // Pattern: set_member(set, x)
            if let Maybe::Some(pattern) = self.mk_function_pattern("set_member", &[var_name, "x"]) {
                patterns.push(pattern);
            }
        }

        patterns
    }

    /// Create patterns for refinement types
    ///
    /// For refinement types T{φ}, we create patterns based on the predicate φ
    fn create_refinement_patterns(
        &mut self,
        var_name: &str,
        _base: &Type,
        predicate: &Expr,
    ) -> List<Pattern> {
        // Extract key terms from the predicate to use as patterns
        self.extract_patterns_from_expr(var_name, predicate)
    }

    /// Extract patterns from an expression
    ///
    /// Looks for function applications and operations that would benefit from patterns
    fn extract_patterns_from_expr(&mut self, var_name: &str, expr: &Expr) -> List<Pattern> {
        let mut patterns = List::new();

        // Collect pattern candidates from the expression
        let mut collector = PatternTermCollector::new(var_name.into());
        collector.collect(expr);

        // Maximum arity we support for pattern generation
        // This is a reasonable limit to prevent excessive pattern generation
        const MAX_PATTERN_ARITY: usize = 16;

        // Generate patterns for function applications that mention our variable
        for (func_name, arity) in collector.function_calls.iter() {
            // Limit arity to prevent excessive pattern generation
            let effective_arity = (*arity).min(MAX_PATTERN_ARITY);

            // Build argument names - first argument is the variable we're tracking
            // Additional arguments get generated placeholder names
            let mut arg_names: Vec<String> = Vec::with_capacity(effective_arity);
            arg_names.push(var_name.to_string());

            // Add placeholder arguments for remaining arity
            for i in 1..effective_arity {
                arg_names.push(format!("__arg{}", i));
            }

            // Create references for the function call
            let arg_refs: Vec<&str> = arg_names.iter().map(|s| s.as_str()).collect();
            if let Maybe::Some(pattern) = self.mk_function_pattern(func_name.as_str(), &arg_refs) {
                patterns.push(pattern);
            }
        }

        // Generate patterns for method calls on the variable
        for method_name in collector.method_calls.iter() {
            // Method calls become function applications: method(receiver, ...)
            let func_name = format!("{}", method_name);
            if let Maybe::Some(pattern) = self.mk_function_pattern(&func_name, &[var_name]) {
                patterns.push(pattern);
            }
        }

        // Generate patterns for binary operations that involve the variable
        // These can be important for arithmetic reasoning
        for op_name in collector.binary_ops.iter() {
            // Create a pattern for the operation
            let func_name = format!("op_{}", op_name);
            if let Maybe::Some(pattern) = self.mk_function_pattern(&func_name, &[var_name, "__rhs"])
            {
                patterns.push(pattern);
            }
        }

        // Generate patterns for field accesses (projections)
        for field_name in collector.field_accesses.iter() {
            let func_name = format!("field_{}", field_name);
            if let Maybe::Some(pattern) = self.mk_function_pattern(&func_name, &[var_name]) {
                patterns.push(pattern);
            }
        }

        // Generate patterns for index operations
        if collector.has_index_access
            && let Maybe::Some(pattern) = self.mk_function_pattern("index", &[var_name, "__idx"])
        {
            patterns.push(pattern);
        }

        patterns
    }

    /// Create a function application pattern
    ///
    /// # Arguments
    ///
    /// * `func_name` - Name of the function
    /// * `arg_names` - Names of arguments (variables or constants)
    ///
    /// # Returns
    ///
    /// Pattern if successful, None if pattern creation fails
    fn mk_function_pattern(&self, func_name: &str, arg_names: &[&str]) -> Maybe<Pattern> {
        // Create Z3 function declaration
        let arg_sorts: List<Sort> = arg_names.iter().map(|_| Sort::int()).collect();
        let arg_sort_refs: List<&Sort> = arg_sorts.iter().collect();
        let range = Sort::int();

        // Create function symbol
        let func_decl = FuncDecl::new(func_name, &arg_sort_refs, &range);

        // Create argument terms as Ast trait objects
        let args: List<Int> = arg_names.iter().map(|name| Int::new_const(*name)).collect();

        let arg_refs: List<&dyn Ast> = args.iter().map(|a| a as &dyn Ast).collect();

        // Create function application
        let app = func_decl.apply(&arg_refs);

        // Create pattern from the application
        let pattern = Pattern::new(&[&app]);

        Maybe::Some(pattern)
    }

    /// Create a quantified formula with patterns
    ///
    /// # Arguments
    ///
    /// * `translator` - Z3 translator for creating terms
    /// * `bound_vars` - Variables to quantify over
    /// * `body` - Formula body
    /// * `patterns` - Patterns for instantiation guidance
    /// * `universal` - True for ∀, false for ∃
    ///
    /// # Returns
    ///
    /// Quantified Z3 formula with patterns
    pub fn mk_quantified_property<'ctx>(
        &mut self,
        _translator: &Translator<'ctx>,
        bound_vars: &[(Text, Dynamic)],
        body: &Bool,
        patterns: &[Pattern],
        universal: bool,
    ) -> Maybe<Bool> {
        if bound_vars.is_empty() {
            return Maybe::Some(body.clone());
        }

        // Extract bound variables as Ast trait objects
        let bound_consts: List<&dyn Ast> =
            bound_vars.iter().map(|(_, var)| var as &dyn Ast).collect();

        // Extract pattern references
        let pattern_refs: List<&Pattern> = patterns.iter().collect();

        // Create quantifier with patterns using module-level functions
        let quantified = if universal {
            z3::ast::forall_const(&bound_consts, &pattern_refs, body)
        } else {
            z3::ast::exists_const(&bound_consts, &pattern_refs, body)
        };

        self.stats.record_quantifier_creation(patterns.len());

        Maybe::Some(quantified)
    }

    /// Assign weights to patterns for priority control
    ///
    /// Higher weight = higher priority for instantiation
    ///
    /// # Arguments
    ///
    /// * `patterns` - Patterns to assign weights to
    /// * `weights` - Corresponding weights (must match pattern count)
    ///
    /// # Returns
    ///
    /// Indices of patterns that meet the weight threshold
    pub fn assign_pattern_weights(&self, patterns: &[Pattern], weights: &[u32]) -> List<usize> {
        assert_eq!(
            patterns.len(),
            weights.len(),
            "pattern and weight counts must match"
        );

        let mut weighted_indices = List::new();
        for (index, &weight) in weights.iter().enumerate() {
            if weight >= self.config.pattern_weight_threshold {
                weighted_indices.push(index);
            }
        }

        weighted_indices
    }

    /// Detect when patterns would help with a formula
    ///
    /// Returns true if the formula would benefit from pattern-based instantiation
    pub fn should_use_patterns(&self, bound_vars: &[(&str, &Type)], body: &Expr) -> bool {
        if !self.config.enable_patterns {
            return false;
        }

        match self.config.strategy {
            PatternGenerationStrategy::Conservative => {
                // Only use patterns for known-good cases (lists, maps, sets)
                bound_vars.iter().any(|(_, ty)| {
                    matches!(ty.kind, TypeKind::Generic { .. } | TypeKind::Refined { .. })
                })
            }
            PatternGenerationStrategy::Aggressive => {
                // Always use patterns when quantifiers are present
                !bound_vars.is_empty()
            }
            PatternGenerationStrategy::Adaptive => {
                // Use patterns if we have complex types or refinements
                bound_vars.iter().any(|(_, ty)| {
                    matches!(ty.kind, TypeKind::Generic { .. } | TypeKind::Refined { .. })
                }) || self.estimate_quantifier_complexity(body) > 50
            }
            PatternGenerationStrategy::TypeDriven => {
                // Use patterns based on types mentioned in the formula
                !bound_vars.is_empty()
            }
        }
    }

    /// Estimate the complexity of a quantified formula
    ///
    /// Used to decide whether patterns would help
    ///
    /// Complexity is computed based on:
    /// - Expression depth (nested structures increase complexity)
    /// - Number of function/method calls (each adds solving difficulty)
    /// - Number of quantifier-relevant operations (binary ops, index access)
    /// - Presence of control flow (if/match/loop increases complexity)
    /// - Use of closures or comprehensions (higher-order reasoning)
    fn estimate_quantifier_complexity(&self, expr: &Expr) -> u32 {
        let mut analyzer = ComplexityAnalyzer::new();
        analyzer.analyze(expr);
        analyzer.compute_complexity()
    }

    /// Get statistics about pattern generation
    pub fn stats(&self) -> &PatternStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = PatternStats::default();
    }

    /// Get the current configuration
    pub fn config(&self) -> &PatternConfig {
        &self.config
    }
}

// ==================== Pattern Context ====================

/// Additional context for pattern generation
#[derive(Debug, Clone)]
pub struct PatternContext {
    /// Known function symbols in the formula
    pub function_symbols: Set<Text>,

    /// Expected complexity level
    pub complexity_hint: Maybe<u32>,

    /// Type environment
    pub type_env: Map<Text, Type>,
}

impl PatternContext {
    pub fn new() -> Self {
        Self {
            function_symbols: Set::new(),
            complexity_hint: Maybe::None,
            type_env: Map::new(),
        }
    }

    pub fn with_functions(mut self, functions: Set<Text>) -> Self {
        self.function_symbols = functions;
        self
    }

    pub fn with_complexity(mut self, complexity: u32) -> Self {
        self.complexity_hint = Maybe::Some(complexity);
        self
    }

    pub fn with_type_env(mut self, env: Map<Text, Type>) -> Self {
        self.type_env = env;
        self
    }
}

impl Default for PatternContext {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Statistics ====================

/// Statistics about pattern generation and effectiveness
#[derive(Debug, Default)]
pub struct PatternStats {
    /// Number of pattern generation calls
    patterns_generated: AtomicU64,

    /// Number of quantifiers created with patterns
    quantifiers_created: AtomicU64,

    /// Total patterns created
    total_patterns: AtomicU64,

    /// Number of times patterns improved solving
    pattern_successes: AtomicU64,

    /// Number of times patterns didn't help
    pattern_failures: AtomicU64,
}

impl Clone for PatternStats {
    fn clone(&self) -> Self {
        Self {
            patterns_generated: AtomicU64::new(self.patterns_generated.load(Ordering::Relaxed)),
            quantifiers_created: AtomicU64::new(self.quantifiers_created.load(Ordering::Relaxed)),
            total_patterns: AtomicU64::new(self.total_patterns.load(Ordering::Relaxed)),
            pattern_successes: AtomicU64::new(self.pattern_successes.load(Ordering::Relaxed)),
            pattern_failures: AtomicU64::new(self.pattern_failures.load(Ordering::Relaxed)),
        }
    }
}

impl PatternStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_pattern_generation(&self, count: usize) {
        self.patterns_generated.fetch_add(1, Ordering::Relaxed);
        self.total_patterns
            .fetch_add(count as u64, Ordering::Relaxed);
    }

    pub fn record_quantifier_creation(&self, pattern_count: usize) {
        self.quantifiers_created.fetch_add(1, Ordering::Relaxed);
        self.total_patterns
            .fetch_add(pattern_count as u64, Ordering::Relaxed);
    }

    pub fn record_success(&self) {
        self.pattern_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.pattern_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn patterns_generated(&self) -> u64 {
        self.patterns_generated.load(Ordering::Relaxed)
    }

    pub fn quantifiers_created(&self) -> u64 {
        self.quantifiers_created.load(Ordering::Relaxed)
    }

    pub fn total_patterns(&self) -> u64 {
        self.total_patterns.load(Ordering::Relaxed)
    }

    pub fn success_rate(&self) -> f64 {
        let successes = self.pattern_successes.load(Ordering::Relaxed);
        let total = successes + self.pattern_failures.load(Ordering::Relaxed);

        if total == 0 {
            0.0
        } else {
            successes as f64 / total as f64
        }
    }

    pub fn avg_patterns_per_quantifier(&self) -> f64 {
        let quantifiers = self.quantifiers_created.load(Ordering::Relaxed);
        let patterns = self.total_patterns.load(Ordering::Relaxed);

        if quantifiers == 0 {
            0.0
        } else {
            patterns as f64 / quantifiers as f64
        }
    }
}

// ==================== Helper Functions ====================

/// Check if a type needs pattern-based quantifier instantiation
pub fn needs_patterns(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Generic { .. } | TypeKind::Refined { .. })
}

/// Extract function applications from an expression for pattern generation
///
/// Traverses the expression tree and collects all function names that are applied.
/// This is useful for determining which functions need patterns for quantifier instantiation.
///
/// # Arguments
///
/// * `expr` - The expression to analyze
///
/// # Returns
///
/// List of function names (as Text) found in the expression
pub fn extract_function_applications(expr: &Expr) -> List<Text> {
    let mut collector = FunctionApplicationCollector::new();
    collector.collect(expr);
    collector.functions
}

/// Create default patterns for a dependent type
pub fn default_patterns_for_type(_ty: &Type) -> List<Text> {
    // Returns pattern templates as strings
    List::new()
}

// ==================== Pattern Term Collector ====================

/// Collects pattern-relevant terms from an expression.
///
/// This analyzer traverses an expression and extracts:
/// - Function calls with their arity
/// - Method calls on the target variable
/// - Binary operations
/// - Field accesses
/// - Index operations
///
/// These are used to generate SMT patterns for quantifier instantiation.
struct PatternTermCollector {
    /// The variable name we're generating patterns for
    target_var: Text,

    /// Function calls found: (function_name, arity)
    function_calls: List<(Text, usize)>,

    /// Method calls found on the target variable
    method_calls: Set<Text>,

    /// Binary operations that involve the target variable
    binary_ops: Set<Text>,

    /// Field accesses on the target variable
    field_accesses: Set<Text>,

    /// Whether the target variable is used in index operations
    has_index_access: bool,

    /// Current expression depth (for tracking nested patterns)
    current_depth: usize,
}

impl PatternTermCollector {
    fn new(target_var: Text) -> Self {
        Self {
            target_var,
            function_calls: List::new(),
            method_calls: Set::new(),
            binary_ops: Set::new(),
            field_accesses: Set::new(),
            has_index_access: false,
            current_depth: 0,
        }
    }


    /// Collect pattern terms from an expression
    fn collect(&mut self, expr: &Expr) {
        self.current_depth += 1;
        self.visit_expr(expr);
        self.current_depth -= 1;
    }

    /// Check if an expression references our target variable
    fn references_target(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Path(path) => {
                // Check if this is a simple reference to our target variable
                if let Some(ident) = path.as_ident() {
                    ident.name.as_str() == self.target_var.as_str()
                } else {
                    false
                }
            }
            ExprKind::Field { expr, .. } => self.references_target(expr),
            ExprKind::Index { expr, .. } => self.references_target(expr),
            ExprKind::Unary { expr, .. } => self.references_target(expr),
            ExprKind::Paren(inner) => self.references_target(inner),
            ExprKind::Attenuate { context, .. } => {
                // Attenuate expressions restrict capability, recursively process the context
                self.references_target(context)
            }
            _ => false,
        }
    }

    /// Extract function name from a call expression
    fn extract_func_name(&self, func: &Expr) -> Maybe<Text> {
        match &func.kind {
            ExprKind::Path(path) => {
                // Get the last segment as the function name
                if let Some(ident) = path.as_ident() {
                    Maybe::Some(ident.name.to_string().into())
                } else if !path.segments.is_empty() {
                    // Multi-segment path - use last segment
                    match path.segments.last() {
                        Some(verum_ast::PathSegment::Name(ident)) => {
                            Maybe::Some(ident.name.to_string().into())
                        }
                        _ => Maybe::None,
                    }
                } else {
                    Maybe::None
                }
            }
            ExprKind::Field { field, .. } => {
                // Field access being called - treat field name as function
                Maybe::Some(field.name.to_string().into())
            }
            _ => Maybe::None,
        }
    }

    /// Visit an expression and collect pattern terms
    fn visit_expr(&mut self, expr: &Expr) {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Attenuate { context, .. } => {
                // Visit the context expression being attenuated
                self.visit_expr(context);
            }

            ExprKind::Throw(inner) => {
                // Visit the thrown expression
                self.visit_expr(inner);
            }

            ExprKind::Typeof(inner) => {
                // Visit the expression inside typeof for pattern analysis
                self.visit_expr(inner);
            }

            ExprKind::Select { arms, .. } => {
                // Visit all select arms
                for arm in arms.iter() {
                    if let Some(future) = &arm.future {
                        self.visit_expr(future);
                    }
                    self.visit_expr(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                }
            }

            ExprKind::Nursery { body, on_cancel, recover, options, .. } => {
                // Visit timeout expression if present
                if let verum_common::Maybe::Some(timeout) = &options.timeout {
                    self.visit_expr(timeout);
                }
                // Visit max_tasks expression if present
                if let verum_common::Maybe::Some(max) = &options.max_tasks {
                    self.visit_expr(max);
                }
                // Visit body statements and expression
                for stmt in body.stmts.iter() {
                    if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                        self.visit_expr(expr);
                    }
                }
                if let verum_common::Maybe::Some(expr) = &body.expr {
                    self.visit_expr(expr);
                }
                // Visit on_cancel block if present
                if let verum_common::Maybe::Some(cancel_block) = on_cancel {
                    for stmt in cancel_block.stmts.iter() {
                        if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                            self.visit_expr(expr);
                        }
                    }
                    if let verum_common::Maybe::Some(expr) = &cancel_block.expr {
                        self.visit_expr(expr);
                    }
                }
                // Visit recover body if present
                if let verum_common::Maybe::Some(recover_body) = recover {
                    match recover_body {
                        verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                            for arm in arms.iter() {
                                self.visit_expr(&arm.body);
                                if let verum_common::Maybe::Some(guard) = &arm.guard {
                                    self.visit_expr(guard);
                                }
                            }
                        }
                        verum_ast::expr::RecoverBody::Closure { body, .. } => {
                            self.visit_expr(body);
                        }
                    }
                }
            }

            ExprKind::Is { expr: inner, .. } => {
                // Visit the expression being tested
                self.visit_expr(inner);
            }

            ExprKind::Call { func, args, .. } => {
                // Check if any argument references our target variable
                let involves_target = args.iter().any(|arg| self.references_target(arg))
                    || self.references_target(func);

                if involves_target && let Maybe::Some(func_name) = self.extract_func_name(func) {
                    // Record the function call with arity
                    self.function_calls.push((func_name, args.len()));
                }

                // Recursively visit function and arguments
                self.visit_expr(func);
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                // Check if receiver is our target variable
                if self.references_target(receiver) {
                    self.method_calls.insert(method.name.to_string().into());
                }

                // Recursively visit
                self.visit_expr(receiver);
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }

            ExprKind::Binary { op, left, right } => {
                // Check if either operand references our target
                if self.references_target(left) || self.references_target(right) {
                    self.binary_ops.insert(op.as_str().into());
                }

                self.visit_expr(left);
                self.visit_expr(right);
            }

            ExprKind::Unary { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::Field { expr: inner, field } => {
                if self.references_target(inner) {
                    self.field_accesses.insert(field.name.to_string().into());
                }
                self.visit_expr(inner);
            }

            ExprKind::Index { expr: arr, index } => {
                if self.references_target(arr) {
                    self.has_index_access = true;
                }
                self.visit_expr(arr);
                self.visit_expr(index);
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Visit condition
                for cond in condition.conditions.iter() {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => self.visit_expr(e),
                        verum_ast::expr::ConditionKind::Let { value, .. } => self.visit_expr(value),
                    }
                }
                // Visit then branch
                self.visit_block(then_branch);
                // Visit else branch if present
                if let Some(else_expr) = else_branch {
                    self.visit_expr(else_expr);
                }
            }

            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.visit_expr(scrutinee);
                for arm in arms.iter() {
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                    self.visit_expr(&arm.body);
                }
            }

            ExprKind::Block(block) => {
                self.visit_block(block);
            }

            ExprKind::Tuple(exprs) => {
                for e in exprs.iter() {
                    self.visit_expr(e);
                }
            }

            ExprKind::Array(arr) => match arr {
                verum_ast::expr::ArrayExpr::List(exprs) => {
                    for e in exprs.iter() {
                        self.visit_expr(e);
                    }
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.visit_expr(value);
                    self.visit_expr(count);
                }
            },

            ExprKind::Closure { body, .. } => {
                self.visit_expr(body);
            }

            ExprKind::Paren(inner) => {
                self.visit_expr(inner);
            }

            ExprKind::Cast { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::Try(inner) => {
                self.visit_expr(inner);
            }

            ExprKind::TryBlock(inner) => {
                // Visit the inner block of a try block expression
                self.visit_expr(inner);
            }

            ExprKind::Await(inner) => {
                self.visit_expr(inner);
            }

            ExprKind::Return(maybe_expr) => {
                if let Some(e) = maybe_expr {
                    self.visit_expr(e);
                }
            }

            ExprKind::Break { label: _, value } => {
                if let Some(e) = value {
                    self.visit_expr(e);
                }
            }

            ExprKind::Yield(inner) => {
                self.visit_expr(inner);
            }

            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.visit_expr(s);
                }
                if let Some(e) = end {
                    self.visit_expr(e);
                }
            }

            ExprKind::Pipeline { left, right } | ExprKind::NullCoalesce { left, right } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }

            ExprKind::Comprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::StreamComprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::SetComprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::GeneratorComprehension {
                expr: inner,
                clauses,
            } => {
                self.visit_expr(inner);
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.visit_expr(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.visit_expr(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.visit_expr(value);
                        }
                    }
                }
            }

            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                self.visit_expr(key_expr);
                self.visit_expr(value_expr);
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.visit_expr(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.visit_expr(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.visit_expr(value);
                        }
                    }
                }
            }

            ExprKind::Record { fields, base, .. } => {
                for field in fields.iter() {
                    if let Some(val) = &field.value {
                        self.visit_expr(val);
                    }
                }
                if let Some(b) = base {
                    self.visit_expr(b);
                }
            }

            ExprKind::Loop {
                label: _,
                body,
                invariants: _,
            }
            | ExprKind::Async(body)
            | ExprKind::Unsafe(body)
            | ExprKind::Meta(body) => {
                self.visit_block(body);
            }

            // Quote expressions contain token trees, not AST nodes to visit
            ExprKind::Quote { .. } => {}

            // Stage escape expressions contain an inner expression to evaluate
            ExprKind::StageEscape { expr, .. } => {
                self.visit_expr(expr);
            }

            // Lift expressions contain an inner expression to evaluate
            ExprKind::Lift { expr } => {
                self.visit_expr(expr);
            }

            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.visit_expr(condition);
                self.visit_block(body);
            }

            ExprKind::For {
                label: _,
                pattern: _,
                iter,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.visit_expr(iter);
                self.visit_block(body);
            }

            ExprKind::ForAwait {
                label: _,
                pattern: _,
                async_iterable,
                body,
                invariants: _,
                decreases: _,
            } => {
                // for-await loop desugars to: loop { match iter.next().await { ... } }
                //
                // Pattern-relevant terms in for-await:
                // 1. The async_iterable creates an AsyncIterator - collect patterns from it
                // 2. Implicit .next() method call on the iterator (AsyncIterator protocol)
                // 3. The body may contain pattern-relevant terms
                //
                // We record "next" as a method call since it's implicitly called on the
                // async iterator. This helps generate appropriate SMT patterns for
                // quantifiers involving async iteration.
                self.method_calls.insert(Text::from("next"));

                self.visit_expr(async_iterable);
                self.visit_block(body);
            }

            ExprKind::TryRecover { try_block, recover } => {
                self.visit_expr(try_block);
                // Visit recover body - either match arms or closure
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms {
                            // Pattern analysis handled separately
                            if let Some(guard) = &arm.guard {
                                self.visit_expr(guard);
                            }
                            self.visit_expr(&arm.body);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.visit_expr(body);
                    }
                }
            }

            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                self.visit_expr(try_block);
                self.visit_expr(finally_block);
            }

            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.visit_expr(try_block);
                // Visit recover body - either match arms or closure
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms {
                            // Pattern analysis handled separately
                            if let Some(guard) = &arm.guard {
                                self.visit_expr(guard);
                            }
                            self.visit_expr(&arm.body);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.visit_expr(body);
                    }
                }
                self.visit_expr(finally_block);
            }

            // Leaf nodes - no recursion needed
            ExprKind::Literal(_) | ExprKind::Path(_) | ExprKind::Continue { .. } => {}

            // Handle remaining expression types
            ExprKind::OptionalChain { expr: inner, .. }
            | ExprKind::TupleIndex { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::InterpolatedString { exprs, .. } => {
                for e in exprs.iter() {
                    self.visit_expr(e);
                }
            }

            ExprKind::TensorLiteral { data, .. } => {
                self.visit_expr(data);
            }

            ExprKind::MapLiteral { entries } => {
                for (k, v) in entries.iter() {
                    self.visit_expr(k);
                    self.visit_expr(v);
                }
            }

            ExprKind::SetLiteral { elements } => {
                for e in elements.iter() {
                    self.visit_expr(e);
                }
            }

            ExprKind::Spawn { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::Inject { .. } => {}
            ExprKind::CalcBlock(_) => {}

            ExprKind::UseContext { handler, body, .. } => {
                self.visit_expr(handler);
                self.visit_expr(body);
            }

            ExprKind::Forall { body, .. } => {
                // Visit the body expression for pattern extraction
                self.visit_expr(body);
            }

            ExprKind::Exists { body, .. } => {
                // Visit the body expression for pattern extraction
                self.visit_expr(body);
            }

            ExprKind::MacroCall { .. } => {
                // Macro calls are not analyzed for quantifier patterns
                // They should be expanded before this phase
            }

            ExprKind::TypeProperty { .. } => {
                // Type property expressions (T.size, T.alignment, etc.)
                // are compile-time constants and don't need pattern extraction
            }

            ExprKind::TypeExpr(_) => {
                // Type expressions in expression position (List<T>.new())
                // are used for method calls and don't need pattern extraction
            }

            ExprKind::Throw(expr) => {
                // Visit the expression being thrown
                self.visit_expr(expr);
            }

            ExprKind::Select { arms, .. } => {
                // Visit all select arms
                for arm in arms.iter() {
                    if let Some(future) = &arm.future {
                        self.visit_expr(future);
                    }
                    self.visit_expr(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                }
            }

            ExprKind::Is { expr, .. } => {
                // Visit the expression (pattern/negated don't need visiting)
                self.visit_expr(expr);
            }

            ExprKind::TypeBound { .. } => {
                // Type bound expressions (T: Protocol) are compile-time conditions
                // evaluated during context checking, no pattern extraction needed
            }

            ExprKind::MetaFunction { args, .. } => {
                // Visit arguments of meta-functions
                for arg in args {
                    self.visit_expr(arg);
                }
            }

            // Stream literal: stream[1, 2, 3, ...] or stream[0..100]
            // Stream literal expressions: `stream[1, 2, 3, ...]` or `stream[0..100]`
            ExprKind::StreamLiteral(stream_lit) => {
                match &stream_lit.kind {
                    verum_ast::expr::StreamLiteralKind::Elements { elements, .. } => {
                        for elem in elements {
                            self.visit_expr(elem);
                        }
                    }
                    verum_ast::expr::StreamLiteralKind::Range { start, end, .. } => {
                        self.visit_expr(start);
                        if let verum_common::Maybe::Some(end_expr) = end {
                            self.visit_expr(end_expr);
                        }
                    }
                }
            }

            // Inline assembly - visit operand expressions (PatternTermExtractor)
            ExprKind::InlineAsm { operands, .. } => {
                for operand in operands {
                    match &operand.kind {
                        verum_ast::expr::AsmOperandKind::In { expr, .. } => {
                            self.visit_expr(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Out { place, .. } => {
                            self.visit_expr(place);
                        }
                        verum_ast::expr::AsmOperandKind::InOut { place, .. } => {
                            self.visit_expr(place);
                        }
                        verum_ast::expr::AsmOperandKind::InLateOut { in_expr, out_place, .. } => {
                            self.visit_expr(in_expr);
                            self.visit_expr(out_place);
                        }
                        verum_ast::expr::AsmOperandKind::Const { expr } => {
                            self.visit_expr(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Sym { .. }
                        | verum_ast::expr::AsmOperandKind::Clobber { .. } => {}
                    }
                }
            }

            // Destructuring assignment - visit the value expression
            // Pattern bindings are analyzed separately if needed
            ExprKind::DestructuringAssign { value, .. } => {
                self.visit_expr(value);
            }

            // Named arguments - visit the value expression
            ExprKind::NamedArg { value, .. } => {
                self.visit_expr(value);
            }

            // Inject expressions (DI resolution) - no pattern extraction needed
            ExprKind::Inject { .. } => {}
            ExprKind::CalcBlock(_) => {}

            // Copattern bodies - no pattern extraction needed
            ExprKind::CopatternBody { .. } => {}
        }
    }

    fn visit_block(&mut self, block: &verum_ast::Block) {
        for stmt in block.stmts.iter() {
            self.visit_stmt(stmt);
        }
        if let Some(expr) = &block.expr {
            self.visit_expr(expr);
        }
    }

    fn visit_stmt(&mut self, stmt: &verum_ast::Stmt) {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Some(v) = value {
                    self.visit_expr(v);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.visit_expr(value);
                self.visit_block(else_block);
            }
            StmtKind::Expr { expr, .. } => {
                self.visit_expr(expr);
            }
            StmtKind::Defer(expr) => {
                self.visit_expr(expr);
            }
            StmtKind::Errdefer(expr) => {
                // Errdefer is like defer but only on error path
                self.visit_expr(expr);
            }
            StmtKind::Provide { value, .. } => {
                self.visit_expr(value);
            }
            StmtKind::ProvideScope { value, block, .. } => {
                self.visit_expr(value);
                self.visit_expr(block);
            }
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }
}

// ==================== Complexity Analyzer ====================

/// Analyzes the complexity of an expression for SMT solving.
///
/// This is used to decide whether pattern-based quantifier instantiation
/// would be beneficial. Higher complexity expressions benefit more from patterns.
///
/// Complexity metrics:
/// - Depth: Nested expressions increase complexity exponentially
/// - Function calls: Each call adds to solving difficulty
/// - Binary operations: Arithmetic/logical operations add complexity
/// - Control flow: Branching increases path complexity
/// - Higher-order features: Closures and comprehensions are complex
struct ComplexityAnalyzer {
    /// Maximum nesting depth encountered
    max_depth: u32,

    /// Current traversal depth
    current_depth: u32,

    /// Number of function/method calls
    function_call_count: u32,

    /// Number of binary operations
    binary_op_count: u32,

    /// Number of control flow constructs (if, match, loops)
    control_flow_count: u32,

    /// Number of higher-order constructs (closures, comprehensions)
    higher_order_count: u32,

    /// Number of quantifier-relevant operations (index, field access)
    accessor_count: u32,

    /// Number of distinct variables/paths referenced
    variable_count: u32,
}

impl ComplexityAnalyzer {
    fn new() -> Self {
        Self {
            max_depth: 0,
            current_depth: 0,
            function_call_count: 0,
            binary_op_count: 0,
            control_flow_count: 0,
            higher_order_count: 0,
            accessor_count: 0,
            variable_count: 0,
        }
    }

    /// Analyze an expression and collect complexity metrics
    fn analyze(&mut self, expr: &Expr) {
        self.current_depth += 1;
        if self.current_depth > self.max_depth {
            self.max_depth = self.current_depth;
        }
        self.visit_expr(expr);
        self.current_depth -= 1;
    }

    /// Compute the final complexity score
    ///
    /// Returns a score where:
    /// - 0-25: Simple expression, patterns likely not needed
    /// - 26-50: Moderate complexity, patterns may help
    /// - 51-100: High complexity, patterns recommended
    /// - 100+: Very high complexity, patterns strongly recommended
    fn compute_complexity(&self) -> u32 {
        // Base complexity from depth (exponential impact)
        let depth_score = if self.max_depth > 0 {
            (self.max_depth - 1) * 5
        } else {
            0
        };

        // Function calls have significant impact on SMT solving
        let function_score = self.function_call_count * 8;

        // Binary operations add linear complexity
        let binary_score = self.binary_op_count * 3;

        // Control flow multiplies paths
        let control_score = self.control_flow_count * 15;

        // Higher-order features are expensive
        let higher_order_score = self.higher_order_count * 20;

        // Accessors are moderately complex
        let accessor_score = self.accessor_count * 4;

        // Variable count adds to instantiation space
        let variable_score = self.variable_count * 2;

        depth_score
            + function_score
            + binary_score
            + control_score
            + higher_order_score
            + accessor_score
            + variable_score
    }

    fn visit_expr(&mut self, expr: &Expr) {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Attenuate { context, .. } => {
                // Visit the context expression being attenuated
                self.visit_expr(context);
            }

            ExprKind::Literal(_) => {
                // Literals are simple
            }

            ExprKind::Path(_) => {
                // Each variable reference adds to complexity
                self.variable_count += 1;
            }

            ExprKind::Binary { left, right, .. } => {
                self.binary_op_count += 1;
                self.analyze(left);
                self.analyze(right);
            }

            ExprKind::Unary { expr: inner, .. } => {
                self.analyze(inner);
            }

            ExprKind::Call { func, args, .. } => {
                self.function_call_count += 1;
                self.analyze(func);
                for arg in args.iter() {
                    self.analyze(arg);
                }
            }

            ExprKind::MethodCall { receiver, args, .. } => {
                self.function_call_count += 1;
                self.analyze(receiver);
                for arg in args.iter() {
                    self.analyze(arg);
                }
            }

            ExprKind::Field { expr: inner, .. } => {
                self.accessor_count += 1;
                self.analyze(inner);
            }

            ExprKind::Index { expr: arr, index } => {
                self.accessor_count += 1;
                self.analyze(arr);
                self.analyze(index);
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.control_flow_count += 1;
                for cond in condition.conditions.iter() {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => self.analyze(e),
                        verum_ast::expr::ConditionKind::Let { value, .. } => self.analyze(value),
                    }
                }
                self.visit_block(then_branch);
                if let Some(else_expr) = else_branch {
                    self.analyze(else_expr);
                }
            }

            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.control_flow_count += 1;
                // Each arm adds to complexity
                self.control_flow_count += arms.len() as u32;
                self.analyze(scrutinee);
                for arm in arms.iter() {
                    if let Some(guard) = &arm.guard {
                        self.analyze(guard);
                    }
                    self.analyze(&arm.body);
                }
            }

            ExprKind::Loop {
                label: _,
                body,
                invariants: _,
            } => {
                self.control_flow_count += 1;
                self.visit_block(body);
            }

            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.control_flow_count += 1;
                self.analyze(condition);
                self.visit_block(body);
            }

            ExprKind::For {
                label: _,
                pattern: _,
                iter,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.control_flow_count += 1;
                self.analyze(iter);
                self.visit_block(body);
            }

            ExprKind::ForAwait {
                label: _,
                pattern: _,
                async_iterable,
                body,
                invariants: _,
                decreases: _,
            } => {
                // for-await loop desugars to: loop { match iter.next().await { ... } }
                //
                // Complexity analysis for for-await:
                // - Control flow: for-await is a loop construct (like for/while)
                // - Function call: Implicit .next() call on each iteration
                // - Higher-order: Async iteration involves futures/state machines
                //
                // The async nature adds complexity beyond a regular for loop because:
                // 1. Each iteration involves await points (state machine transitions)
                // 2. The iterator produces futures that must be polled
                // 3. SMT solving for async iteration may need specialized patterns
                self.control_flow_count += 1;
                self.function_call_count += 1; // Implicit next() call
                self.higher_order_count += 1; // Async/future handling

                self.analyze(async_iterable);
                self.visit_block(body);
            }

            ExprKind::Closure { body, .. } => {
                self.higher_order_count += 1;
                self.analyze(body);
            }

            ExprKind::Comprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::StreamComprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::SetComprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::GeneratorComprehension {
                expr: inner,
                clauses,
            } => {
                self.higher_order_count += 1;
                self.analyze(inner);
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.analyze(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.analyze(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.analyze(value);
                        }
                    }
                }
            }

            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                self.higher_order_count += 1;
                self.analyze(key_expr);
                self.analyze(value_expr);
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.analyze(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.analyze(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.analyze(value);
                        }
                    }
                }
            }

            ExprKind::Block(block) => {
                self.visit_block(block);
            }

            ExprKind::Tuple(exprs) => {
                for e in exprs.iter() {
                    self.analyze(e);
                }
            }

            ExprKind::Array(arr) => match arr {
                verum_ast::expr::ArrayExpr::List(exprs) => {
                    for e in exprs.iter() {
                        self.analyze(e);
                    }
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.analyze(value);
                    self.analyze(count);
                }
            },

            ExprKind::Record { fields, base, .. } => {
                for field in fields.iter() {
                    if let Some(val) = &field.value {
                        self.analyze(val);
                    }
                }
                if let Some(b) = base {
                    self.analyze(b);
                }
            }

            ExprKind::Paren(inner) => {
                self.analyze(inner);
            }

            ExprKind::Cast { expr: inner, .. } => {
                self.analyze(inner);
            }

            ExprKind::Try(inner) | ExprKind::Await(inner) | ExprKind::Yield(inner) | ExprKind::TryBlock(inner) => {
                self.analyze(inner);
            }

            ExprKind::Return(maybe_expr) => {
                if let Some(e) = maybe_expr {
                    self.analyze(e);
                }
            }

            ExprKind::Break { label: _, value } => {
                if let Some(e) = value {
                    self.analyze(e);
                }
            }

            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.analyze(s);
                }
                if let Some(e) = end {
                    self.analyze(e);
                }
            }

            ExprKind::Pipeline { left, right } | ExprKind::NullCoalesce { left, right } => {
                self.analyze(left);
                self.analyze(right);
            }

            ExprKind::TryRecover { try_block, recover } => {
                self.control_flow_count += 1;
                self.analyze(try_block);
                // Analyze recover body - either match arms or closure
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms {
                            if let Some(guard) = &arm.guard {
                                self.analyze(guard);
                            }
                            self.analyze(&arm.body);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.analyze(body);
                    }
                }
            }

            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                self.control_flow_count += 1;
                self.analyze(try_block);
                self.analyze(finally_block);
            }

            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.control_flow_count += 2;
                self.analyze(try_block);
                // Analyze recover body - either match arms or closure
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms {
                            if let Some(guard) = &arm.guard {
                                self.analyze(guard);
                            }
                            self.analyze(&arm.body);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.analyze(body);
                    }
                }
                self.analyze(finally_block);
            }

            ExprKind::Async(block) | ExprKind::Unsafe(block) | ExprKind::Meta(block) => {
                self.visit_block(block);
            }

            // Quote expressions contain token trees, not AST nodes to visit
            ExprKind::Quote { .. } => {}

            // Stage escape expressions contain an inner expression to evaluate
            ExprKind::StageEscape { expr, .. } => {
                self.analyze(expr);
            }

            // Lift expressions contain an inner expression to evaluate
            ExprKind::Lift { expr } => {
                self.analyze(expr);
            }

            ExprKind::Continue { .. } => {}

            ExprKind::OptionalChain { expr: inner, .. }
            | ExprKind::TupleIndex { expr: inner, .. } => {
                self.accessor_count += 1;
                self.analyze(inner);
            }

            ExprKind::InterpolatedString { exprs, .. } => {
                for e in exprs.iter() {
                    self.analyze(e);
                }
            }

            ExprKind::TensorLiteral { data, .. } => {
                self.analyze(data);
            }

            ExprKind::MapLiteral { entries } => {
                for (k, v) in entries.iter() {
                    self.analyze(k);
                    self.analyze(v);
                }
            }

            ExprKind::SetLiteral { elements } => {
                for e in elements.iter() {
                    self.analyze(e);
                }
            }

            ExprKind::Spawn { expr: inner, .. } => {
                self.higher_order_count += 1;
                self.analyze(inner);
            }

            ExprKind::Inject { .. } => {}
            ExprKind::CalcBlock(_) => {}

            ExprKind::UseContext { handler, body, .. } => {
                self.higher_order_count += 1;
                self.analyze(handler);
                self.analyze(body);
            }

            ExprKind::Forall { body, .. } => {
                // Quantifiers add complexity
                self.higher_order_count += 1;
                self.analyze(body);
            }

            ExprKind::Exists { body, .. } => {
                // Quantifiers add complexity
                self.higher_order_count += 1;
                self.analyze(body);
            }

            ExprKind::MacroCall { .. } => {
                // Macro calls are compile-time expanded, treat as minimal complexity
            }

            ExprKind::TypeProperty { .. } => {
                // Type property expressions (T.size, T.alignment, etc.)
                // are compile-time constants with minimal complexity
            }

            ExprKind::TypeExpr(_) => {
                // Type expressions in expression position (List<T>.new())
                // are used for method calls with minimal complexity
            }

            ExprKind::Throw(expr) => {
                // Throwing adds control flow complexity
                self.analyze(expr);
            }

            ExprKind::Typeof(inner) => {
                // Type introspection - analyze the expression's type
                self.analyze(inner);
            }

            ExprKind::Select { arms, .. } => {
                // Select adds concurrency complexity
                self.higher_order_count += 1;
                for arm in arms.iter() {
                    if let Some(future) = &arm.future {
                        self.analyze(future);
                    }
                    self.analyze(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.analyze(guard);
                    }
                }
            }

            ExprKind::Nursery { body, on_cancel, recover, options, .. } => {
                // Nursery adds structured concurrency complexity
                self.higher_order_count += 1;
                // Analyze options
                if let verum_common::Maybe::Some(timeout) = &options.timeout {
                    self.analyze(timeout);
                }
                if let verum_common::Maybe::Some(max) = &options.max_tasks {
                    self.analyze(max);
                }
                // Analyze body
                self.visit_block(body);
                // Analyze on_cancel
                if let verum_common::Maybe::Some(cancel_block) = on_cancel {
                    self.visit_block(cancel_block);
                }
                // Analyze recover
                if let verum_common::Maybe::Some(recover_body) = recover {
                    match recover_body {
                        verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                            for arm in arms.iter() {
                                self.analyze(&arm.body);
                                if let verum_common::Maybe::Some(guard) = &arm.guard {
                                    self.analyze(guard);
                                }
                            }
                        }
                        verum_ast::expr::RecoverBody::Closure { body, .. } => {
                            self.analyze(body);
                        }
                    }
                }
            }

            ExprKind::Is { expr, .. } => {
                // Pattern matching is simple
                self.analyze(expr);
            }

            ExprKind::TypeBound { .. } => {
                // Type bound expressions (T: Protocol) are compile-time conditions
                // with minimal complexity for SMT solving
            }

            ExprKind::MetaFunction { args, .. } => {
                // Analyze arguments of meta-functions
                for arg in args {
                    self.analyze(arg);
                }
            }

            // Stream literal: stream[1, 2, 3, ...] or stream[0..100]
            // Stream literal expressions: `stream[1, 2, 3, ...]` or `stream[0..100]` (ComplexityAnalyzer)
            ExprKind::StreamLiteral(stream_lit) => {
                match &stream_lit.kind {
                    verum_ast::expr::StreamLiteralKind::Elements { elements, .. } => {
                        for elem in elements {
                            self.analyze(elem);
                        }
                    }
                    verum_ast::expr::StreamLiteralKind::Range { start, end, .. } => {
                        self.analyze(start);
                        if let verum_common::Maybe::Some(end_expr) = end {
                            self.analyze(end_expr);
                        }
                    }
                }
            }

            // Inline assembly - analyze operand expressions for complexity
            ExprKind::InlineAsm { operands, .. } => {
                // Each operand with an expression adds to complexity
                for operand in operands {
                    match &operand.kind {
                        verum_ast::expr::AsmOperandKind::In { expr, .. } => {
                            self.analyze(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Out { place, .. } => {
                            self.analyze(place);
                        }
                        verum_ast::expr::AsmOperandKind::InOut { place, .. } => {
                            self.analyze(place);
                        }
                        verum_ast::expr::AsmOperandKind::InLateOut { in_expr, out_place, .. } => {
                            self.analyze(in_expr);
                            self.analyze(out_place);
                        }
                        verum_ast::expr::AsmOperandKind::Const { expr } => {
                            self.analyze(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Sym { .. }
                        | verum_ast::expr::AsmOperandKind::Clobber { .. } => {}
                    }
                }
            }

            // Destructuring assignment - analyze the value expression
            ExprKind::DestructuringAssign { value, .. } => {
                self.analyze(value);
            }

            // Named arguments - analyze the value expression
            ExprKind::NamedArg { value, .. } => {
                self.analyze(value);
            }

            // Inject expressions (DI resolution) - minimal complexity
            ExprKind::Inject { .. } => {}
            ExprKind::CalcBlock(_) => {}

            // Copattern bodies - minimal complexity
            ExprKind::CopatternBody { .. } => {}
        }
    }

    fn visit_block(&mut self, block: &verum_ast::Block) {
        for stmt in block.stmts.iter() {
            self.visit_stmt(stmt);
        }
        if let Some(expr) = &block.expr {
            self.analyze(expr);
        }
    }

    fn visit_stmt(&mut self, stmt: &verum_ast::Stmt) {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Some(v) = value {
                    self.analyze(v);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.analyze(value);
                self.visit_block(else_block);
            }
            StmtKind::Expr { expr, .. } => {
                self.analyze(expr);
            }
            StmtKind::Defer(expr) => {
                self.analyze(expr);
            }
            StmtKind::Errdefer(expr) => {
                // Errdefer is like defer but only on error path
                self.analyze(expr);
            }
            StmtKind::Provide { value, .. } => {
                self.analyze(value);
            }
            StmtKind::ProvideScope { value, block, .. } => {
                self.analyze(value);
                self.analyze(block);
            }
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }
}

// ==================== Function Application Collector ====================

/// Collects all function applications from an expression.
///
/// This traverses the entire expression tree and extracts the names
/// of all functions that are called. Useful for determining which
/// SMT function symbols need pattern support.
struct FunctionApplicationCollector {
    /// Collected function names
    functions: List<Text>,

    /// Set for deduplication
    seen: Set<Text>,
}

impl FunctionApplicationCollector {
    fn new() -> Self {
        Self {
            functions: List::new(),
            seen: Set::new(),
        }
    }

    /// Collect function applications from an expression
    fn collect(&mut self, expr: &Expr) {
        self.visit_expr(expr);
    }

    /// Add a function name if not already seen
    fn add_function(&mut self, name: Text) {
        if !self.seen.contains(&name) {
            self.seen.insert(name.clone());
            self.functions.push(name);
        }
    }

    /// Extract function name from a call target
    fn extract_func_name(&self, func: &Expr) -> Maybe<Text> {
        match &func.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    Maybe::Some(ident.name.to_string().into())
                } else if !path.segments.is_empty() {
                    match path.segments.last() {
                        Some(verum_ast::PathSegment::Name(ident)) => {
                            Maybe::Some(ident.name.to_string().into())
                        }
                        _ => Maybe::None,
                    }
                } else {
                    Maybe::None
                }
            }
            ExprKind::Field { field, .. } => Maybe::Some(field.name.to_string().into()),
            _ => Maybe::None,
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Attenuate { context, .. } => {
                // Visit the context expression being attenuated
                self.visit_expr(context);
            }

            ExprKind::Throw(inner) => {
                // Visit the thrown expression
                self.visit_expr(inner);
            }

            ExprKind::Typeof(inner) => {
                // Visit the expression inside typeof
                self.visit_expr(inner);
            }

            ExprKind::Select { arms, .. } => {
                // Visit all select arms
                for arm in arms.iter() {
                    if let Some(future) = &arm.future {
                        self.visit_expr(future);
                    }
                    self.visit_expr(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                }
            }

            ExprKind::Nursery { body, on_cancel, recover, options, .. } => {
                // Visit timeout expression if present
                if let verum_common::Maybe::Some(timeout) = &options.timeout {
                    self.visit_expr(timeout);
                }
                // Visit max_tasks expression if present
                if let verum_common::Maybe::Some(max) = &options.max_tasks {
                    self.visit_expr(max);
                }
                // Visit body statements and expression
                for stmt in body.stmts.iter() {
                    if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                        self.visit_expr(expr);
                    }
                }
                if let verum_common::Maybe::Some(expr) = &body.expr {
                    self.visit_expr(expr);
                }
                // Visit on_cancel block if present
                if let verum_common::Maybe::Some(cancel_block) = on_cancel {
                    for stmt in cancel_block.stmts.iter() {
                        if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                            self.visit_expr(expr);
                        }
                    }
                    if let verum_common::Maybe::Some(expr) = &cancel_block.expr {
                        self.visit_expr(expr);
                    }
                }
                // Visit recover body if present
                if let verum_common::Maybe::Some(recover_body) = recover {
                    match recover_body {
                        verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                            for arm in arms.iter() {
                                self.visit_expr(&arm.body);
                                if let verum_common::Maybe::Some(guard) = &arm.guard {
                                    self.visit_expr(guard);
                                }
                            }
                        }
                        verum_ast::expr::RecoverBody::Closure { body, .. } => {
                            self.visit_expr(body);
                        }
                    }
                }
            }

            ExprKind::Is { expr: inner, .. } => {
                // Visit the expression being tested
                self.visit_expr(inner);
            }

            ExprKind::Call { func, args, .. } => {
                if let Maybe::Some(name) = self.extract_func_name(func) {
                    self.add_function(name);
                }
                self.visit_expr(func);
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                self.add_function(method.name.to_string().into());
                self.visit_expr(receiver);
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }

            ExprKind::Binary { left, right, .. } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }

            ExprKind::Unary { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::Field { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::Index { expr: arr, index } => {
                self.visit_expr(arr);
                self.visit_expr(index);
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                for cond in condition.conditions.iter() {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => self.visit_expr(e),
                        verum_ast::expr::ConditionKind::Let { value, .. } => self.visit_expr(value),
                    }
                }
                self.visit_block(then_branch);
                if let Some(else_expr) = else_branch {
                    self.visit_expr(else_expr);
                }
            }

            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.visit_expr(scrutinee);
                for arm in arms.iter() {
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                    self.visit_expr(&arm.body);
                }
            }

            ExprKind::Block(block) => {
                self.visit_block(block);
            }

            ExprKind::Tuple(exprs) => {
                for e in exprs.iter() {
                    self.visit_expr(e);
                }
            }

            ExprKind::Array(arr) => match arr {
                verum_ast::expr::ArrayExpr::List(exprs) => {
                    for e in exprs.iter() {
                        self.visit_expr(e);
                    }
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.visit_expr(value);
                    self.visit_expr(count);
                }
            },

            ExprKind::Closure { body, .. } => {
                self.visit_expr(body);
            }

            ExprKind::Comprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::StreamComprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::SetComprehension {
                expr: inner,
                clauses,
            }
            | ExprKind::GeneratorComprehension {
                expr: inner,
                clauses,
            } => {
                self.visit_expr(inner);
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.visit_expr(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.visit_expr(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.visit_expr(value);
                        }
                    }
                }
            }

            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                self.visit_expr(key_expr);
                self.visit_expr(value_expr);
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.visit_expr(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.visit_expr(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.visit_expr(value);
                        }
                    }
                }
            }

            ExprKind::Record { fields, base, .. } => {
                for field in fields.iter() {
                    if let Some(val) = &field.value {
                        self.visit_expr(val);
                    }
                }
                if let Some(b) = base {
                    self.visit_expr(b);
                }
            }

            ExprKind::Paren(inner) => {
                self.visit_expr(inner);
            }

            ExprKind::Cast { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::Try(inner) | ExprKind::Await(inner) | ExprKind::Yield(inner) | ExprKind::TryBlock(inner) => {
                self.visit_expr(inner);
            }

            ExprKind::Return(maybe_expr) => {
                if let Some(e) = maybe_expr {
                    self.visit_expr(e);
                }
            }

            ExprKind::Break { label: _, value } => {
                if let Some(e) = value {
                    self.visit_expr(e);
                }
            }

            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.visit_expr(s);
                }
                if let Some(e) = end {
                    self.visit_expr(e);
                }
            }

            ExprKind::Pipeline { left, right } | ExprKind::NullCoalesce { left, right } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }

            ExprKind::Loop {
                label: _,
                body,
                invariants: _,
            }
            | ExprKind::Async(body)
            | ExprKind::Unsafe(body)
            | ExprKind::Meta(body) => {
                self.visit_block(body);
            }

            // Quote expressions contain token trees, not AST nodes to visit
            ExprKind::Quote { .. } => {}

            // Stage escape expressions contain an inner expression to evaluate
            ExprKind::StageEscape { expr, .. } => {
                self.visit_expr(expr);
            }

            // Lift expressions contain an inner expression to evaluate
            ExprKind::Lift { expr } => {
                self.visit_expr(expr);
            }

            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.visit_expr(condition);
                self.visit_block(body);
            }

            ExprKind::For {
                label: _,
                pattern: _,
                iter,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.visit_expr(iter);
                self.visit_block(body);
            }

            ExprKind::ForAwait {
                label: _,
                pattern: _,
                async_iterable,
                body,
                invariants: _,
                decreases: _,
            } => {
                // for-await loop desugars to: loop { match iter.next().await { ... } }
                //
                // Function application extraction for for-await:
                // Record the implicit "next" function call from the AsyncIterator protocol.
                // This is important for SMT pattern generation because:
                // 1. The next() method is called on every iteration
                // 2. SMT patterns for async iteration may need to match on next() applications
                // 3. Quantifier instantiation benefits from knowing about iteration methods
                let next_fn = Text::from("next");
                if !self.seen.contains(&next_fn) {
                    self.seen.insert(next_fn.clone());
                    self.functions.push(next_fn);
                }

                self.visit_expr(async_iterable);
                self.visit_block(body);
            }

            ExprKind::TryRecover { try_block, recover } => {
                self.visit_expr(try_block);
                // Visit recover body - either match arms or closure
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms {
                            // Pattern analysis handled separately
                            if let Some(guard) = &arm.guard {
                                self.visit_expr(guard);
                            }
                            self.visit_expr(&arm.body);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.visit_expr(body);
                    }
                }
            }

            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                self.visit_expr(try_block);
                self.visit_expr(finally_block);
            }

            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.visit_expr(try_block);
                // Visit recover body - either match arms or closure
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms {
                            // Pattern analysis handled separately
                            if let Some(guard) = &arm.guard {
                                self.visit_expr(guard);
                            }
                            self.visit_expr(&arm.body);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.visit_expr(body);
                    }
                }
                self.visit_expr(finally_block);
            }

            // Leaf nodes and nodes without function calls
            ExprKind::Literal(_) | ExprKind::Path(_) | ExprKind::Continue { .. } => {}

            ExprKind::OptionalChain { expr: inner, .. }
            | ExprKind::TupleIndex { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::InterpolatedString { exprs, .. } => {
                for e in exprs.iter() {
                    self.visit_expr(e);
                }
            }

            ExprKind::TensorLiteral { data, .. } => {
                self.visit_expr(data);
            }

            ExprKind::MapLiteral { entries } => {
                for (k, v) in entries.iter() {
                    self.visit_expr(k);
                    self.visit_expr(v);
                }
            }

            ExprKind::SetLiteral { elements } => {
                for e in elements.iter() {
                    self.visit_expr(e);
                }
            }

            ExprKind::Spawn { expr: inner, .. } => {
                self.visit_expr(inner);
            }

            ExprKind::Inject { .. } => {}
            ExprKind::CalcBlock(_) => {}

            ExprKind::UseContext { handler, body, .. } => {
                self.visit_expr(handler);
                self.visit_expr(body);
            }

            ExprKind::Forall { body, .. } => {
                // Visit the body expression for pattern extraction
                self.visit_expr(body);
            }

            ExprKind::Exists { body, .. } => {
                // Visit the body expression for pattern extraction
                self.visit_expr(body);
            }

            ExprKind::MacroCall { .. } => {
                // Macro calls are not analyzed for quantifier patterns
                // They should be expanded before this phase
            }

            ExprKind::TypeProperty { .. } => {
                // Type property expressions (T.size, T.alignment, etc.)
                // are compile-time constants and don't need pattern extraction
            }

            ExprKind::TypeExpr(_) => {
                // Type expressions in expression position (List<T>.new())
                // are used for method calls and don't need pattern extraction
            }

            ExprKind::Throw(expr) => {
                // Visit the expression being thrown
                self.visit_expr(expr);
            }

            ExprKind::Select { arms, .. } => {
                // Visit all select arms
                for arm in arms.iter() {
                    if let Some(future) = &arm.future {
                        self.visit_expr(future);
                    }
                    self.visit_expr(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                }
            }

            ExprKind::Is { expr, .. } => {
                // Visit the expression (pattern/negated don't need visiting)
                self.visit_expr(expr);
            }

            ExprKind::TypeBound { .. } => {
                // Type bound expressions (T: Protocol) are compile-time conditions
                // evaluated during context checking, no pattern extraction needed
            }

            ExprKind::MetaFunction { args, .. } => {
                // Visit arguments of meta-functions
                for arg in args {
                    self.visit_expr(arg);
                }
            }

            // Stream literal: stream[1, 2, 3, ...] or stream[0..100]
            // Stream literal expressions: `stream[1, 2, 3, ...]` or `stream[0..100]`
            ExprKind::StreamLiteral(stream_lit) => {
                match &stream_lit.kind {
                    verum_ast::expr::StreamLiteralKind::Elements { elements, .. } => {
                        for elem in elements {
                            self.visit_expr(elem);
                        }
                    }
                    verum_ast::expr::StreamLiteralKind::Range { start, end, .. } => {
                        self.visit_expr(start);
                        if let verum_common::Maybe::Some(end_expr) = end {
                            self.visit_expr(end_expr);
                        }
                    }
                }
            }

            // Inline assembly - visit operand expressions (TriggerExtractor)
            ExprKind::InlineAsm { operands, .. } => {
                for operand in operands {
                    match &operand.kind {
                        verum_ast::expr::AsmOperandKind::In { expr, .. } => {
                            self.visit_expr(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Out { place, .. } => {
                            self.visit_expr(place);
                        }
                        verum_ast::expr::AsmOperandKind::InOut { place, .. } => {
                            self.visit_expr(place);
                        }
                        verum_ast::expr::AsmOperandKind::InLateOut { in_expr, out_place, .. } => {
                            self.visit_expr(in_expr);
                            self.visit_expr(out_place);
                        }
                        verum_ast::expr::AsmOperandKind::Const { expr } => {
                            self.visit_expr(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Sym { .. }
                        | verum_ast::expr::AsmOperandKind::Clobber { .. } => {}
                    }
                }
            }

            // Destructuring assignment - visit the value expression
            ExprKind::DestructuringAssign { value, .. } => {
                self.visit_expr(value);
            }

            // Named arguments - visit the value expression
            ExprKind::NamedArg { value, .. } => {
                self.visit_expr(value);
            }

            // Inject expressions (DI resolution) - no pattern extraction needed
            ExprKind::Inject { .. } => {}
            ExprKind::CalcBlock(_) => {}

            // Copattern bodies - no pattern extraction needed
            ExprKind::CopatternBody { .. } => {}
        }
    }

    fn visit_block(&mut self, block: &verum_ast::Block) {
        for stmt in block.stmts.iter() {
            self.visit_stmt(stmt);
        }
        if let Some(expr) = &block.expr {
            self.visit_expr(expr);
        }
    }

    fn visit_stmt(&mut self, stmt: &verum_ast::Stmt) {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Some(v) = value {
                    self.visit_expr(v);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.visit_expr(value);
                self.visit_block(else_block);
            }
            StmtKind::Expr { expr, .. } => {
                self.visit_expr(expr);
            }
            StmtKind::Defer(expr) => {
                self.visit_expr(expr);
            }
            StmtKind::Errdefer(expr) => {
                // Errdefer is like defer but only on error path
                self.visit_expr(expr);
            }
            StmtKind::Provide { value, .. } => {
                self.visit_expr(value);
            }
            StmtKind::ProvideScope { value, block, .. } => {
                self.visit_expr(value);
                self.visit_expr(block);
            }
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }
}

// ==================== Function Application ====================

/// Represents a function application extracted from a formula.
///
/// This is used for pattern synthesis - function applications in the quantifier
/// body are good candidates for patterns as they trigger instantiation when
/// the function is called elsewhere.
#[derive(Debug, Clone)]
pub struct FunctionApp {
    /// Name of the function
    pub name: Text,
    /// Arity (number of arguments)
    pub arity: usize,
    /// Argument types (Z3 sorts)
    pub arg_sorts: List<Sort>,
    /// Return type (Z3 sort)
    pub return_sort: Sort,
    /// Whether this is a method call
    pub is_method: bool,
    /// Receiver type if method call
    pub receiver_type: Maybe<Sort>,
}

impl FunctionApp {
    /// Create a new function application
    pub fn new(name: Text, arg_sorts: List<Sort>, return_sort: Sort) -> Self {
        Self {
            name,
            arity: arg_sorts.len(),
            arg_sorts,
            return_sort,
            is_method: false,
            receiver_type: Maybe::None,
        }
    }

    /// Create a method call application
    pub fn method(
        name: Text,
        receiver_sort: Sort,
        arg_sorts: List<Sort>,
        return_sort: Sort,
    ) -> Self {
        Self {
            name,
            arity: arg_sorts.len() + 1, // Include receiver
            arg_sorts,
            return_sort,
            is_method: true,
            receiver_type: Maybe::Some(receiver_sort),
        }
    }
}

// ==================== Pattern Synthesis ====================

/// Synthesizes patterns from a formula for quantifier instantiation.
///
/// This function analyzes a formula and extracts function applications
/// that can be used as patterns. Good patterns should:
/// - Mention all quantified variables
/// - Be selective (not match too many terms)
/// - Avoid arithmetic operations
///
/// # Arguments
///
/// * `formula` - The quantifier body formula to analyze
/// * `bound_vars` - Names of bound variables to track
///
/// # Returns
///
/// List of synthesized patterns
pub fn synthesize_patterns(formula: &Expr, bound_vars: &[Text]) -> List<Pattern> {
    let mut patterns = List::new();

    // Extract all function applications from the formula
    let apps = extract_function_applications_detailed(formula, bound_vars);

    // Create patterns from the applications
    for app in apps.iter() {
        if let Maybe::Some(pattern) = create_pattern_from_app(app) {
            patterns.push(pattern);
        }
    }

    // If multi-patterns would help, try to create them
    if patterns.len() >= 2 {
        if let Maybe::Some(multi_pattern) = try_create_multi_pattern(&apps) {
            patterns.push(multi_pattern);
        }
    }

    patterns
}

/// Extract function applications from a formula with detailed type information.
///
/// This is an enhanced version of extract_function_applications that includes
/// type information needed for pattern creation.
///
/// # Arguments
///
/// * `formula` - The formula to analyze
/// * `bound_vars` - Names of bound variables to track
///
/// # Returns
///
/// List of FunctionApp structures with full type information
pub fn extract_function_applications_detailed(
    formula: &Expr,
    bound_vars: &[Text],
) -> List<FunctionApp> {
    let mut apps = List::new();
    let bound_var_set: Set<Text> = bound_vars.iter().cloned().collect();

    // Use a recursive visitor to find all function applications
    extract_apps_recursive(formula, &bound_var_set, &mut apps);

    apps
}

fn extract_apps_recursive(expr: &Expr, bound_vars: &Set<Text>, apps: &mut List<FunctionApp>) {
    match &expr.kind {
        ExprKind::Call { func, args, .. } => {
            // Check if any argument references a bound variable
            let involves_bound_var = args.iter().any(|arg| expr_references_vars(arg, bound_vars));

            if involves_bound_var {
                // Extract function name
                if let Maybe::Some(name) = extract_call_func_name(func) {
                    // Create arg sorts (simplified - use Int for all)
                    let arg_sorts: List<Sort> = args.iter().map(|_| Sort::int()).collect();
                    let return_sort = Sort::int();

                    apps.push(FunctionApp::new(name, arg_sorts, return_sort));
                }
            }

            // Recursively visit arguments
            for arg in args.iter() {
                extract_apps_recursive(arg, bound_vars, apps);
            }
        }

        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            // Check if receiver or any argument references a bound variable
            let involves_bound_var = expr_references_vars(receiver, bound_vars)
                || args.iter().any(|arg| expr_references_vars(arg, bound_vars));

            if involves_bound_var {
                let receiver_sort = Sort::int(); // Simplified
                let arg_sorts: List<Sort> = args.iter().map(|_| Sort::int()).collect();
                let return_sort = infer_method_return_sort(method.name.as_str(), &receiver_sort);

                apps.push(FunctionApp::method(
                    method.name.to_string().into(),
                    receiver_sort,
                    arg_sorts,
                    return_sort,
                ));
            }

            // Recursively visit
            extract_apps_recursive(receiver, bound_vars, apps);
            for arg in args.iter() {
                extract_apps_recursive(arg, bound_vars, apps);
            }
        }

        ExprKind::Index { expr: arr, index } => {
            // Index operations can be patterns
            if expr_references_vars(arr, bound_vars) || expr_references_vars(index, bound_vars) {
                let arg_sorts: List<Sort> = List::from_iter([Sort::int(), Sort::int()]);
                apps.push(FunctionApp::new(
                    Text::from("select"),
                    arg_sorts,
                    Sort::int(),
                ));
            }

            extract_apps_recursive(arr, bound_vars, apps);
            extract_apps_recursive(index, bound_vars, apps);
        }

        ExprKind::Field { expr: inner, field } => {
            if expr_references_vars(inner, bound_vars) {
                let arg_sorts: List<Sort> = List::from_iter([Sort::int()]);
                apps.push(FunctionApp::new(
                    Text::from(format!("field_{}", field.name)),
                    arg_sorts,
                    Sort::int(),
                ));
            }
            extract_apps_recursive(inner, bound_vars, apps);
        }

        // Recursively visit other expression types
        ExprKind::Binary { left, right, .. } => {
            extract_apps_recursive(left, bound_vars, apps);
            extract_apps_recursive(right, bound_vars, apps);
        }

        ExprKind::Unary { expr: inner, .. } => {
            extract_apps_recursive(inner, bound_vars, apps);
        }

        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            for cond in condition.conditions.iter() {
                match cond {
                    verum_ast::expr::ConditionKind::Expr(e) => {
                        extract_apps_recursive(e, bound_vars, apps)
                    }
                    verum_ast::expr::ConditionKind::Let { value, .. } => {
                        extract_apps_recursive(value, bound_vars, apps)
                    }
                }
            }
            for stmt in then_branch.stmts.iter() {
                if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                    extract_apps_recursive(expr, bound_vars, apps);
                }
            }
            if let Some(expr) = &then_branch.expr {
                extract_apps_recursive(expr, bound_vars, apps);
            }
            if let Some(else_expr) = else_branch {
                extract_apps_recursive(else_expr, bound_vars, apps);
            }
        }

        ExprKind::Paren(inner) => {
            extract_apps_recursive(inner, bound_vars, apps);
        }

        ExprKind::Tuple(exprs) => {
            for e in exprs.iter() {
                extract_apps_recursive(e, bound_vars, apps);
            }
        }

        ExprKind::Array(arr) => match arr {
            verum_ast::expr::ArrayExpr::List(exprs) => {
                for e in exprs.iter() {
                    extract_apps_recursive(e, bound_vars, apps);
                }
            }
            verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                extract_apps_recursive(value, bound_vars, apps);
                extract_apps_recursive(count, bound_vars, apps);
            }
        },

        ExprKind::Forall { body, .. } | ExprKind::Exists { body, .. } => {
            extract_apps_recursive(body, bound_vars, apps);
        }

        // Other expressions - skip or recurse as needed
        _ => {}
    }
}

fn expr_references_vars(expr: &Expr, bound_vars: &Set<Text>) -> bool {
    match &expr.kind {
        ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                bound_vars.contains(&Text::from(ident.name.as_str()))
            } else {
                false
            }
        }
        ExprKind::Field { expr, .. } => expr_references_vars(expr, bound_vars),
        ExprKind::Index { expr, index } => {
            expr_references_vars(expr, bound_vars) || expr_references_vars(index, bound_vars)
        }
        ExprKind::Call { func, args, .. } => {
            expr_references_vars(func, bound_vars)
                || args.iter().any(|a| expr_references_vars(a, bound_vars))
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            expr_references_vars(receiver, bound_vars)
                || args.iter().any(|a| expr_references_vars(a, bound_vars))
        }
        ExprKind::Binary { left, right, .. } => {
            expr_references_vars(left, bound_vars) || expr_references_vars(right, bound_vars)
        }
        ExprKind::Unary { expr, .. } => expr_references_vars(expr, bound_vars),
        ExprKind::Paren(inner) => expr_references_vars(inner, bound_vars),
        _ => false,
    }
}

fn extract_call_func_name(func: &Expr) -> Maybe<Text> {
    match &func.kind {
        ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                Maybe::Some(Text::from(ident.name.as_str()))
            } else if let Some(PathSegment::Name(ident)) = path.segments.last() {
                Maybe::Some(Text::from(ident.name.as_str()))
            } else {
                Maybe::None
            }
        }
        ExprKind::Field { field, .. } => Maybe::Some(Text::from(field.name.as_str())),
        _ => Maybe::None,
    }
}

/// Create a Z3 Pattern from a FunctionApp.
///
/// # Arguments
///
/// * `app` - The function application to convert
///
/// # Returns
///
/// A Z3 Pattern if creation succeeds, None otherwise
pub fn create_pattern_from_app(app: &FunctionApp) -> Maybe<Pattern> {
    // Create the function declaration
    let arg_sort_refs: List<&Sort> = app.arg_sorts.iter().collect();
    let func_decl = FuncDecl::new(app.name.as_str(), &arg_sort_refs, &app.return_sort);

    // Create placeholder arguments
    let args: List<Int> = (0..app.arity)
        .map(|i| Int::new_const(format!("__pat_arg_{}", i)))
        .collect();

    let arg_refs: List<&dyn Ast> = args.iter().map(|a| a as &dyn Ast).collect();

    // Apply function to create the term
    let func_app = func_decl.apply(&arg_refs);

    // Create pattern from the application
    Maybe::Some(Pattern::new(&[&func_app]))
}

/// Try to create a multi-pattern from multiple function applications.
///
/// Multi-patterns help Z3 instantiate quantifiers when multiple function
/// applications appear together. This is particularly useful for:
/// - Map get/contains pairs
/// - List len/get pairs
/// - Array select/store pairs
///
/// # Arguments
///
/// * `apps` - List of function applications to combine
///
/// # Returns
///
/// A multi-pattern if creation succeeds, None otherwise
fn try_create_multi_pattern(apps: &List<FunctionApp>) -> Maybe<Pattern> {
    if apps.len() < 2 {
        return Maybe::None;
    }

    // Create function applications for the first two distinct functions
    let mut terms: List<Dynamic> = List::new();
    let mut seen_names: Set<Text> = Set::new();

    for app in apps.iter() {
        if seen_names.contains(&app.name) {
            continue;
        }
        seen_names.insert(app.name.clone());

        let arg_sort_refs: List<&Sort> = app.arg_sorts.iter().collect();
        let func_decl = FuncDecl::new(app.name.as_str(), &arg_sort_refs, &app.return_sort);

        let args: List<Int> = (0..app.arity)
            .map(|i| Int::new_const(format!("__mp_arg_{}_{}", app.name, i)))
            .collect();

        let arg_refs: List<&dyn Ast> = args.iter().map(|a| a as &dyn Ast).collect();
        let func_app = func_decl.apply(&arg_refs);
        terms.push(func_app);

        if terms.len() >= 2 {
            break;
        }
    }

    if terms.len() < 2 {
        return Maybe::None;
    }

    // Create multi-pattern from the terms
    let term_refs: List<&dyn Ast> = terms.iter().map(|t| t as &dyn Ast).collect();
    Maybe::Some(Pattern::new(&term_refs))
}

// ==================== MBQI Configuration ====================

/// Configure solver parameters for MBQI (Model-Based Quantifier Instantiation).
///
/// This sets Z3 parameters that control how quantifiers are instantiated:
/// - smt.mbqi: Enable/disable model-based quantifier instantiation
/// - smt.qi.eager_threshold: Threshold for eager instantiation
/// - smt.qi.max_instances: Maximum instances per quantifier
///
/// # Arguments
///
/// * `solver` - The Z3 solver to configure
/// * `config` - Pattern configuration containing MBQI settings
///
/// # Example
///
/// ```rust,ignore
/// let mut solver = Solver::new();
/// let config = PatternConfig::default();
/// configure_mbqi(&solver, &config);
/// ```
pub fn configure_mbqi(solver: &Solver, config: &PatternConfig) {
    let mut params = Params::new();

    // Enable/disable MBQI
    params.set_bool("smt.mbqi", config.enable_mbqi);

    // Set eager threshold
    params.set_f64("smt.qi.eager_threshold", config.mbqi_eager_threshold);

    // Set max instances
    params.set_u32("smt.qi.max_instances", config.mbqi_max_instances);

    // If patterns are enabled, configure pattern-based instantiation
    if config.enable_patterns {
        // Enable pattern-based matching
        params.set_bool("smt.auto_config", false);
        // Prioritize user-provided patterns
        params.set_u32("smt.qi.cost", 1);
    }

    solver.set_params(&params);
}

// ==================== Advanced Quantifier Creation ====================

/// Create a weighted quantifier with patterns, identifier, and skolem naming.
///
/// This provides the full Z3 quantifier API including:
/// - Pattern-based instantiation hints
/// - Weight for instantiation priority
/// - Named quantifier and skolem identifiers for debugging
/// - No-pattern terms to exclude from auto-pattern generation
///
/// # Arguments
///
/// * `is_forall` - True for universal (forall), false for existential (exists)
/// * `weight` - Instantiation weight (0 = default priority)
/// * `quantifier_id` - Identifier for the quantifier (debugging/profiling)
/// * `skolem_id` - Identifier for skolem constants
/// * `bounds` - Bound variables (constants to quantify over)
/// * `patterns` - Explicit patterns for instantiation
/// * `no_patterns` - Terms to exclude from auto-pattern generation
/// * `body` - The quantifier body formula
///
/// # Returns
///
/// The quantified formula
pub fn create_weighted_quantifier(
    is_forall: bool,
    weight: u32,
    quantifier_id: &str,
    skolem_id: &str,
    bounds: &[&dyn Ast],
    patterns: &[&Pattern],
    no_patterns: &[&dyn Ast],
    body: &Bool,
) -> Bool {
    z3::ast::quantifier_const(
        is_forall,
        weight,
        quantifier_id,
        skolem_id,
        bounds,
        patterns,
        no_patterns,
        body,
    )
}

/// Create a universal quantifier with patterns.
///
/// Convenience wrapper around forall_const with pattern support.
///
/// # Arguments
///
/// * `bounds` - Bound variables
/// * `patterns` - Patterns for instantiation
/// * `body` - Quantifier body
///
/// # Returns
///
/// The universally quantified formula
pub fn create_forall_with_patterns(
    bounds: &[&dyn Ast],
    patterns: &[&Pattern],
    body: &Bool,
) -> Bool {
    z3::ast::forall_const(bounds, patterns, body)
}

/// Create an existential quantifier with patterns.
///
/// Convenience wrapper around exists_const with pattern support.
///
/// # Arguments
///
/// * `bounds` - Bound variables
/// * `patterns` - Patterns for instantiation
/// * `body` - Quantifier body
///
/// # Returns
///
/// The existentially quantified formula
pub fn create_exists_with_patterns(
    bounds: &[&dyn Ast],
    patterns: &[&Pattern],
    body: &Bool,
) -> Bool {
    z3::ast::exists_const(bounds, patterns, body)
}

// ==================== Pattern Weight Assignment ====================

/// Weighted pattern metadata without owning the pattern.
///
/// Since z3::Pattern doesn't implement Clone, we store the metadata separately
/// and reconstruct the pattern when needed.
#[derive(Debug, Clone)]
pub struct WeightedPatternMeta {
    /// Weight for instantiation priority (higher = more important)
    pub weight: u32,
    /// Description for debugging
    pub description: Text,
    /// Function name that creates the pattern
    pub func_name: Text,
    /// Arity for recreation
    pub arity: usize,
}

impl WeightedPatternMeta {
    /// Create a new weighted pattern metadata
    pub fn new(weight: u32, description: impl Into<Text>, func_name: Text, arity: usize) -> Self {
        Self {
            weight,
            description: description.into(),
            func_name,
            arity,
        }
    }

    /// Recreate the pattern from metadata
    pub fn to_pattern(&self) -> Maybe<Pattern> {
        let arg_sorts: List<Sort> = (0..self.arity).map(|_| Sort::int()).collect();
        let arg_sort_refs: List<&Sort> = arg_sorts.iter().collect();
        let func_decl = FuncDecl::new(self.func_name.as_str(), &arg_sort_refs, &Sort::int());

        let args: List<Int> = (0..self.arity)
            .map(|i| Int::new_const(format!("__wpat_arg_{}", i)))
            .collect();

        let arg_refs: List<&dyn Ast> = args.iter().map(|a| a as &dyn Ast).collect();
        let func_app = func_decl.apply(&arg_refs);

        Maybe::Some(Pattern::new(&[&func_app]))
    }
}

/// Weighted pattern with importance score (non-Clone version).
#[derive(Debug)]
pub struct WeightedPattern {
    /// The Z3 pattern
    pub pattern: Pattern,
    /// Weight for instantiation priority (higher = more important)
    pub weight: u32,
    /// Description for debugging
    pub description: Text,
}

impl WeightedPattern {
    /// Create a new weighted pattern
    pub fn new(pattern: Pattern, weight: u32, description: impl Into<Text>) -> Self {
        Self {
            pattern,
            weight,
            description: description.into(),
        }
    }
}

/// Assign weights to patterns based on their characteristics.
///
/// Weight assignment heuristics:
/// - Single function applications: weight = arity + base
/// - Multi-patterns: weight = sum of component weights
/// - Collection operations: higher weight (more selective)
/// - Arithmetic operations: lower weight (less selective)
///
/// # Arguments
///
/// * `apps` - Function applications to weight
/// * `config` - Configuration with default weight
///
/// # Returns
///
/// List of weighted patterns
pub fn assign_weights_to_patterns(
    apps: &List<FunctionApp>,
    config: &PatternConfig,
) -> List<WeightedPattern> {
    let mut weighted = List::new();

    for app in apps.iter() {
        let weight = compute_pattern_weight(app, config);

        if let Maybe::Some(pattern) = create_pattern_from_app(app) {
            weighted.push(WeightedPattern::new(
                pattern,
                weight,
                format!("{}(arity={})", app.name, app.arity),
            ));
        }
    }

    // Sort by weight (descending)
    weighted.sort_by(|a, b| b.weight.cmp(&a.weight));

    weighted
}

fn compute_pattern_weight(app: &FunctionApp, config: &PatternConfig) -> u32 {
    let mut weight = config.default_pattern_weight;

    // Higher arity = more selective
    weight += app.arity as u32;

    // Collection operations are good patterns
    let name = app.name.as_str();
    if name.contains("len") || name.contains("get") || name.contains("contains") {
        weight += 10;
    }

    // Field accesses are moderately selective
    if name.starts_with("field_") {
        weight += 5;
    }

    // Index operations are good
    if name == "select" || name == "store" {
        weight += 8;
    }

    // Method calls on known types
    if app.is_method {
        weight += 3;
    }

    weight
}
