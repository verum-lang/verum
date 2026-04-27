//! Protocol coherence checking and orphan rule validation.
//!
//! Implements the coherence system from Section 4 of the specification:
//! - Orphan rule validation (Section 4.1.1)
//! - Overlap prevention (Section 4.1.3)
//! - Specialization tracking (Section 4.1.4)
//! - Cross-crate conflict detection (Section 4.2)
//!
//! # Overview
//!
//! Protocol coherence ensures that there is exactly one implementation of a
//! protocol for any given type across the entire program. This prevents
//! ambiguity in method dispatch and enables modular reasoning.
//!
//! # Key Principles
//!
//! **Orphan Rule** (Section 4.1.1):
//! For `implement Protocol for Type` to be valid, at least one of:
//! - Protocol is defined in the current crate, OR
//! - Type is defined in the current crate (or uses a local type parameter)
//!
//! **Overlap Prevention** (Section 4.1.3):
//! Two implementations overlap if there exists a type substitution that makes
//! them apply to the same Protocol-Type pair. Overlap is a compile error
//! unless using explicit specialization.
//!
//! **Specialization** (Section 4.1.4):
//! Specialized implementations can override more general implementations when:
//! - Marked with `@specialize` annotation
//! - Strictly more specific than the general implementation
//! - In the same crate as the general implementation
//!
//! These rules ensure global coherence of the type system across the entire
//! dependency graph.

// Unused imports are OK - they're used in the error types
#[allow(unused_imports)]
use crate::error::{ModuleError, ModuleResult};
use crate::path::{ModuleId, ModulePath};
use std::collections::HashMap;
use verum_ast::{Path, Span, Type as AstType};
use verum_common::{List, Maybe, Text};

/// Error type for coherence violations.
#[derive(Debug, Clone, PartialEq)]
pub enum CoherenceError {
    /// Orphan implementation - neither protocol nor type is local
    OrphanImpl {
        protocol: Text,
        for_type: Text,
        protocol_crate: Text,
        type_crate: Text,
        current_crate: Text,
        span: Option<Span>,
    },

    /// Overlapping implementations without specialization
    OverlappingImpl {
        protocol: Text,
        type1: Text,
        type2: Text,
        impl1_location: ModulePath,
        impl2_location: ModulePath,
        span: Option<Span>,
    },

    /// Invalid specialization hierarchy
    InvalidSpecialization {
        protocol: Text,
        specialized_type: Text,
        base_type: Text,
        reason: Text,
        span: Option<Span>,
    },

    /// Conflicting implementations from different cogs
    ConflictingCrateImpl {
        protocol: Text,
        for_type: Text,
        crate1: Text,
        crate2: Text,
        span: Option<Span>,
    },
}

impl std::fmt::Display for CoherenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoherenceError::OrphanImpl {
                protocol,
                for_type,
                protocol_crate,
                type_crate,
                current_crate,
                ..
            } => {
                write!(
                    f,
                    "Orphan implementation: `implement {} for {}`\n\
                     Protocol '{}' is defined in cog '{}'\n\
                     Type '{}' is defined in cog '{}'\n\
                     Implementation is in cog '{}'\n\n\
                     HELP: Either:\n\
                       1. Define a newtype wrapper: `type My{} is {}`\n\
                       2. Implement for a local type instead\n\
                       3. Request upstream to add this implementation",
                    protocol,
                    for_type,
                    protocol,
                    protocol_crate,
                    for_type,
                    type_crate,
                    current_crate,
                    for_type,
                    for_type
                )
            }
            CoherenceError::OverlappingImpl {
                protocol,
                type1,
                type2,
                impl1_location,
                impl2_location,
                ..
            } => {
                write!(
                    f,
                    "Overlapping implementations for `{}`:\n\
                     - `implement {} for {}` in {}\n\
                     - `implement {} for {}` in {}\n\n\
                     These implementations overlap and the compiler cannot choose which to use.\n\n\
                     HELP: Use `@specialize` to create an explicit specialization hierarchy",
                    protocol, protocol, type1, impl1_location, protocol, type2, impl2_location
                )
            }
            CoherenceError::InvalidSpecialization {
                protocol,
                specialized_type,
                base_type,
                reason,
                ..
            } => {
                write!(
                    f,
                    "Invalid specialization of `{} for {}`:\n\
                     Cannot specialize `{} for {}`: {}\n\n\
                     Specialization requires:\n\
                       1. `@specialize` annotation\n\
                       2. Strictly more specific than base impl\n\
                       3. Same cog as base impl",
                    protocol, specialized_type, protocol, base_type, reason
                )
            }
            CoherenceError::ConflictingCrateImpl {
                protocol,
                for_type,
                crate1,
                crate2,
                ..
            } => {
                write!(
                    f,
                    "Conflicting protocol implementations detected:\n\
                     `implement {} for {}` defined in both:\n\
                     - cog '{}'\n\
                     - cog '{}'\n\n\
                     HELP: Use features to select one implementation:\n\
                     [dependencies]\n\
                     {} = {{ version = \"1.0\", features = [\"impl\"] }}\n\
                     {} = {{ version = \"1.0\", default-features = false }}",
                    protocol, for_type, crate1, crate2, crate1, crate2
                )
            }
        }
    }
}

impl std::error::Error for CoherenceError {}

/// Entry for a protocol implementation.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplEntry {
    /// The protocol being implemented
    pub protocol: Text,
    /// The protocol path (for cog identification)
    pub protocol_path: ModulePath,
    /// The type implementing the protocol
    pub for_type: Text,
    /// The type path (for cog identification)
    pub type_path: Maybe<ModulePath>,
    /// Module containing the implementation
    pub impl_module: ModulePath,
    /// Module ID of the implementation
    pub impl_module_id: ModuleId,
    /// Whether this is a specialized implementation
    pub is_specialized: bool,
    /// Type parameters on the implementation
    pub type_params: List<Text>,
    /// Where clauses / constraints
    pub constraints: List<Text>,
    /// Source span
    pub span: Option<Span>,
    /// @cfg predicates guarding this implementation (e.g., ["target_os = \"linux\""])
    /// Implementations with mutually exclusive cfg predicates are not considered overlapping.
    pub cfg_predicates: List<Text>,
}

impl ImplEntry {
    /// Create a new implementation entry.
    pub fn new(
        protocol: Text,
        protocol_path: ModulePath,
        for_type: Text,
        impl_module: ModulePath,
        impl_module_id: ModuleId,
    ) -> Self {
        Self {
            protocol,
            protocol_path,
            for_type,
            type_path: Maybe::None,
            impl_module,
            impl_module_id,
            is_specialized: false,
            type_params: List::new(),
            constraints: List::new(),
            span: None,
            cfg_predicates: List::new(),
        }
    }

    /// Set the type path.
    pub fn with_type_path(mut self, path: ModulePath) -> Self {
        self.type_path = Maybe::Some(path);
        self
    }

    /// Mark as specialized.
    pub fn with_specialized(mut self) -> Self {
        self.is_specialized = true;
        self
    }

    /// Add type parameters.
    pub fn with_type_params(mut self, params: List<Text>) -> Self {
        self.type_params = params;
        self
    }

    /// Add constraints.
    pub fn with_constraints(mut self, constraints: List<Text>) -> Self {
        self.constraints = constraints;
        self
    }

    /// Set the span.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Set cfg predicates guarding this implementation.
    pub fn with_cfg_predicates(mut self, predicates: List<Text>) -> Self {
        self.cfg_predicates = predicates;
        self
    }

    /// Check if this impl has cfg predicates.
    pub fn has_cfg_predicates(&self) -> bool {
        !self.cfg_predicates.is_empty()
    }

    /// Check if this implementation is generic (has type parameters).
    pub fn is_generic(&self) -> bool {
        !self.type_params.is_empty()
    }

    /// Get the cog name from impl module.
    pub fn crate_name(&self) -> Text {
        self.impl_module
            .segments()
            .first()
            .map(|s| Text::from(s.as_str()))
            .unwrap_or_else(|| Text::from(""))
    }

    /// Get the cog name from protocol path.
    pub fn protocol_crate(&self) -> Text {
        self.protocol_path
            .segments()
            .first()
            .map(|s| Text::from(s.as_str()))
            .unwrap_or_else(|| Text::from(""))
    }

    /// Get the cog name from type path.
    pub fn type_crate(&self) -> Maybe<Text> {
        self.type_path.as_ref().map(|path| {
            path.segments()
                .first()
                .map(|s| Text::from(s.as_str()))
                .unwrap_or_else(|| Text::from(""))
        })
    }
}

/// Structured representation of a type for overlap checking.
///
/// This enum provides a parsed view of type strings that enables
/// precise overlap detection through structural comparison.
#[derive(Debug, Clone, PartialEq)]
enum TypeStructure<'a> {
    /// A concrete named type (e.g., "Int", "String")
    Named(&'a str),
    /// A type variable (e.g., "T", "U")
    Variable(&'a str),
    /// A generic type with arguments (e.g., "List<T>", "Map<K, V>")
    Generic { base: &'a str, args: List<&'a str> },
    /// A tuple type (e.g., "(A, B)")
    Tuple(List<&'a str>),
    /// A reference type (e.g., "&T", "&mut T")
    Reference { mutable: bool, inner: &'a str },
    /// An array type (e.g., "[T; N]")
    Array {
        element: &'a str,
        size: Option<&'a str>,
    },
    /// A slice type (e.g., "[T]")
    Slice(&'a str),
}

/// Coherence checker - validates protocol implementations.
///
/// Implements the coherence checking algorithm from Section 4:
/// 1. Collect all implementations from dependency graph
/// 2. Build implementation table: (Protocol, Type) → [Implementations]
/// 3. Detect conflicts: If implementations.len() > 1 for same pair
/// 4. Validate orphan rules for each implementation
/// 5. Check specialization hierarchy
///
/// Algorithm: (1) collect all implementations from dependency graph,
/// (2) build implementation table: (Protocol, Type) -> [Implementations],
/// (3) detect conflicts where implementations.len() > 1 for same pair,
/// (4) validate orphan rules, (5) check specialization hierarchy.
#[derive(Debug)]
pub struct CoherenceChecker {
    /// Current cog name
    current_crate: Text,
    /// Implementation table: (protocol, type) → implementations
    impl_table: HashMap<(Text, Text), List<ImplEntry>>,
    /// All collected implementations
    all_impls: List<ImplEntry>,
    /// Specialization hierarchy: base impl → specialized impls
    specializations: HashMap<(Text, Text), List<ImplEntry>>,
    /// Known local types (defined in current cog)
    local_types: HashMap<Text, ModulePath>,
    /// Known local protocols (defined in current cog)
    local_protocols: HashMap<Text, ModulePath>,
    /// Trusted cogs that are allowed to bypass orphan rule
    /// These are typically core library cogs that need blanket implementations
    trusted_crates: std::collections::HashSet<Text>,
}

impl CoherenceChecker {
    /// Create a new coherence checker for the given cog.
    pub fn new(current_crate: impl Into<Text>) -> Self {
        Self {
            current_crate: current_crate.into(),
            impl_table: HashMap::new(),
            all_impls: List::new(),
            specializations: HashMap::new(),
            local_types: HashMap::new(),
            local_protocols: HashMap::new(),
            trusted_crates: std::collections::HashSet::new(),
        }
    }

    /// Mark a cog as trusted.
    /// Trusted cogs are allowed to bypass the orphan rule, enabling blanket implementations.
    /// This is used for core library cogs like stdlib.
    pub fn add_trusted_crate(&mut self, crate_name: impl Into<Text>) {
        self.trusted_crates.insert(crate_name.into());
    }

    /// Check if a cog is trusted.
    pub fn is_trusted_crate(&self, crate_name: &str) -> bool {
        self.trusted_crates.contains(crate_name)
    }

    /// Register a local type definition.
    pub fn register_local_type(&mut self, name: Text, path: ModulePath) {
        self.local_types.insert(name, path);
    }

    /// Register a local protocol definition.
    pub fn register_local_protocol(&mut self, name: Text, path: ModulePath) {
        self.local_protocols.insert(name, path);
    }

    /// Check if a type is local to the current cog.
    fn is_local_type(&self, type_name: &str) -> bool {
        self.local_types.contains_key(type_name)
    }

    /// Check if a protocol is local to the current cog.
    fn is_local_protocol(&self, protocol_name: &str) -> bool {
        self.local_protocols.contains_key(protocol_name)
    }

    /// Check if a type uses a local type parameter.
    ///
    /// For generic implementations like `implement Protocol for List<MyLocalType>`,
    /// the implementation is valid if any type parameter is local.
    ///
    /// This function performs comprehensive type analysis:
    /// 1. Checks explicit type parameters
    /// 2. Parses the for_type string to extract nested types
    /// 3. Checks all extracted types against local type registry
    ///
    /// Orphan Rule: for `implement Protocol for Type` to be valid, at least
    /// one of Protocol or Type must be defined in the current cog.
    fn uses_local_type_param(&self, impl_entry: &ImplEntry) -> bool {
        // Check if any explicit type parameter matches a local type
        for param in &impl_entry.type_params {
            if self.is_local_type(param.as_str()) {
                return true;
            }
        }

        // Parse the for_type to extract all type components
        let extracted_types = self.extract_all_type_names(&impl_entry.for_type);

        // Check if any extracted type is local
        for type_name in &extracted_types {
            if self.is_local_type(type_name.as_str()) {
                return true;
            }
        }

        false
    }

    /// Extract all type names from a type string representation.
    ///
    /// This function parses type strings like:
    /// - `List<MyType>` -> ["List", "MyType"]
    /// - `Map<Key, List<Value>>` -> ["Map", "Key", "List", "Value"]
    /// - `Result<Maybe<T>, Error>` -> ["Result", "Maybe", "T", "Error"]
    /// - `fn(A, B) -> C` -> ["A", "B", "C"]
    /// - `&mut T` -> ["T"]
    ///
    /// Orphan rule check: LocalProtocol(P, C) OR LocalType(T, C) must hold.
    /// LocalType includes generic types with local type parameters.
    fn extract_all_type_names(&self, type_str: &Text) -> List<Text> {
        let mut types = List::new();
        let s = type_str.as_str();

        // Track current identifier being built
        let mut current = String::new();
        let chars = s.chars().peekable();

        for ch in chars {
            match ch {
                // Identifier characters
                'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => {
                    current.push(ch);
                }
                // Separators and delimiters
                '<' | '>' | ',' | '(' | ')' | '[' | ']' | '{' | '}' | ' ' | '&' | '*' | ':'
                | '|' | '.' | '+' | '-' => {
                    if !current.is_empty() {
                        // Skip keywords like 'mut', 'fn', 'dyn', 'impl', 'where', 'for'
                        if !is_type_keyword(&current) {
                            types.push(Text::from(current.clone()));
                        }
                        current.clear();
                    }
                }
                // Handle arrow (->)
                _ => {
                    if !current.is_empty() {
                        if !is_type_keyword(&current) {
                            types.push(Text::from(current.clone()));
                        }
                        current.clear();
                    }
                }
            }
        }

        // Don't forget the last identifier
        if !current.is_empty() && !is_type_keyword(&current) {
            types.push(Text::from(current));
        }

        types
    }

    /// Add an implementation to be checked.
    pub fn add_impl(&mut self, entry: ImplEntry) {
        let key = (entry.protocol.clone(), entry.for_type.clone());

        // Add to implementation table
        self.impl_table
            .entry(key.clone())
            .or_default()
            .push(entry.clone());

        // Track specializations separately
        if entry.is_specialized {
            self.specializations
                .entry(key)
                .or_default()
                .push(entry.clone());
        }

        self.all_impls.push(entry);
    }

    /// Check orphan rule for a single implementation.
    ///
    /// Orphan rule check: LocalProtocol(P, C) OR LocalType(T, C) must hold.
    /// LocalType includes generic types with local type parameters.
    ///
    /// For `implement Protocol for Type` to be valid:
    /// LocalProtocol(Protocol, CurrentCrate) ∨ LocalType(Type, CurrentCrate)
    pub fn check_orphan_rule(&self, entry: &ImplEntry) -> Result<(), CoherenceError> {
        // Check if implementation is in current cog
        let impl_in_current_crate = entry.crate_name() == self.current_crate;

        // If implementation is not in current cog, we don't validate it
        // (it should have been validated when that cog was compiled)
        if !impl_in_current_crate {
            return Ok(());
        }

        // Trusted cogs are allowed to bypass orphan rule
        // This enables blanket implementations in core library cogs
        if self.is_trusted_crate(entry.crate_name().as_str()) {
            return Ok(());
        }

        let protocol_is_local = self.is_local_protocol(entry.protocol.as_str());
        let type_is_local =
            self.is_local_type(entry.for_type.as_str()) || self.uses_local_type_param(entry);

        // For local implementations, either protocol or type must be local
        if !protocol_is_local && !type_is_local {
            return Err(CoherenceError::OrphanImpl {
                protocol: entry.protocol.clone(),
                for_type: entry.for_type.clone(),
                protocol_crate: entry.protocol_crate(),
                type_crate: entry.type_crate().unwrap_or_else(|| Text::from("external")),
                current_crate: self.current_crate.clone(),
                span: entry.span,
            });
        }

        Ok(())
    }

    /// Check for overlapping implementations.
    ///
    /// Overlap check: two implementations overlap if there exists a type
    /// substitution sigma such that Type1[sigma] = Type2[sigma] AND
    /// Protocol1[sigma] = Protocol2[sigma]. Overlap is a compile error
    /// unless using explicit @specialize annotation.
    ///
    /// Two implementations overlap if there exists a substitution that makes them
    /// apply to the same (Protocol, Type) pair.
    pub fn check_overlap(
        &self,
        impl1: &ImplEntry,
        impl2: &ImplEntry,
    ) -> Result<(), CoherenceError> {
        // Same implementation (ignore)
        if impl1.impl_module == impl2.impl_module && impl1.span == impl2.span {
            return Ok(());
        }

        // If one is a specialization of the other, that's OK
        if impl1.is_specialized || impl2.is_specialized {
            // Specializations are allowed to overlap with their base
            return Ok(());
        }

        // Implementations guarded by mutually exclusive @cfg predicates
        // cannot coexist at runtime, so they are not considered overlapping.
        // For example, @cfg(target_os = "linux") and @cfg(target_os = "macos")
        // are mutually exclusive because target_os can only have one value.
        if Self::cfg_predicates_mutually_exclusive(&impl1.cfg_predicates, &impl2.cfg_predicates) {
            return Ok(());
        }

        // Two implementations from *different* stdlib modules (trusted crates)
        // implementing the same protocol for a type with the same simple name
        // are NOT overlapping because Verum uses nominal typing. Types with the
        // same name in different modules are distinct types. For example:
        //   - implement Debug for StackFrame in core.runtime.thread
        //   - implement Debug for StackFrame in core.base.error
        // These implement Debug for two DIFFERENT StackFrame types.
        if impl1.impl_module != impl2.impl_module {
            let mod1_str = impl1.impl_module.to_string();
            let mod2_str = impl2.impl_module.to_string();
            let is_mod1_trusted = self.trusted_crates.iter().any(|tc| {
                mod1_str.starts_with(tc.as_str())
            });
            let is_mod2_trusted = self.trusted_crates.iter().any(|tc| {
                mod2_str.starts_with(tc.as_str())
            });
            if is_mod1_trusted && is_mod2_trusted {
                return Ok(());
            }
        }

        // Check if types overlap using structural unification
        if self.types_may_overlap(&impl1.for_type, &impl2.for_type) {
            return Err(CoherenceError::OverlappingImpl {
                protocol: impl1.protocol.clone(),
                type1: impl1.for_type.clone(),
                type2: impl2.for_type.clone(),
                impl1_location: impl1.impl_module.clone(),
                impl2_location: impl2.impl_module.clone(),
                span: impl1.span,
            });
        }

        Ok(())
    }

    /// Check if two types may overlap.
    ///
    /// This implements a unification-based overlap check. Two types overlap if
    /// there exists a substitution that makes them equal. This is more precise
    /// than simple string comparison.
    ///
    /// # Algorithm
    ///
    /// 1. Exact match: types are identical
    /// 2. Type variable match: either type is a variable (can unify with anything)
    ///
    /// Check if two sets of @cfg predicates are mutually exclusive.
    ///
    /// Two predicate sets are mutually exclusive when both are non-empty and
    /// they contain conflicting values for the same key. For example:
    /// - `target_os = "linux"` and `target_os = "macos"` are mutually exclusive
    /// - `target_os = "linux"` and `feature = "foo"` are NOT mutually exclusive
    ///
    /// Known single-valued cfg keys: `target_os`, `target_arch`, `target_env`,
    /// `target_vendor`, `target_endian`, `target_pointer_width`, `runtime`.
    fn cfg_predicates_mutually_exclusive(preds1: &[Text], preds2: &[Text]) -> bool {
        // Both must have cfg predicates to be mutually exclusive
        if preds1.is_empty() || preds2.is_empty() {
            return false;
        }

        // Single-valued cfg keys (only one value can be active at a time)
        const SINGLE_VALUED_KEYS: &[&str] = &[
            "target_os",
            "target_arch",
            "target_env",
            "target_vendor",
            "target_endian",
            "target_pointer_width",
            "runtime",
        ];

        // Extract key=value pairs from predicate strings
        fn parse_cfg_kv(pred: &str) -> Option<(&str, &str)> {
            let pred = pred.trim();
            if let Some(eq_pos) = pred.find('=') {
                let key = pred[..eq_pos].trim();
                let value = pred[eq_pos + 1..].trim().trim_matches('"');
                Some((key, value))
            } else {
                None
            }
        }

        for p1 in preds1 {
            if let Some((k1, v1)) = parse_cfg_kv(p1.as_str()) {
                for p2 in preds2 {
                    if let Some((k2, v2)) = parse_cfg_kv(p2.as_str()) {
                        // Same single-valued key with different values => mutually exclusive
                        if k1 == k2
                            && v1 != v2
                            && SINGLE_VALUED_KEYS.contains(&k1)
                        {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// 3. Generic instantiation match: same base type with overlapping parameters
    /// 4. Recursive check: for compound types, check component overlap
    ///
    /// Overlap check: two implementations overlap if there exists a type
    /// substitution sigma such that Type1[sigma] = Type2[sigma] AND
    /// Protocol1[sigma] = Protocol2[sigma]. Overlap is a compile error
    /// unless using explicit @specialize annotation. - Overlap Prevention
    fn types_may_overlap(&self, type1: &Text, type2: &Text) -> bool {
        // Exact match - definitely overlap
        if type1 == type2 {
            return true;
        }

        let type1_str = type1.as_str();
        let type2_str = type2.as_str();

        // If either is a type variable, they may overlap
        // Type variables can unify with any other type
        if self.is_type_variable(type1_str) || self.is_type_variable(type2_str) {
            return true;
        }

        // Parse both types into structured form
        let parsed1 = self.parse_type_structure(type1_str);
        let parsed2 = self.parse_type_structure(type2_str);

        // Check structural overlap
        self.type_structures_overlap(&parsed1, &parsed2)
    }

    /// Parse a type string into a structured representation for overlap checking.
    fn parse_type_structure<'a>(&self, type_str: &'a str) -> TypeStructure<'a> {
        // Reference types
        if type_str.starts_with("&mut ") {
            return TypeStructure::Reference {
                mutable: true,
                inner: type_str.strip_prefix("&mut ").unwrap_or(""),
            };
        }
        if type_str.starts_with('&') {
            return TypeStructure::Reference {
                mutable: false,
                inner: type_str.strip_prefix('&').unwrap_or(""),
            };
        }

        // Tuple types
        if type_str.starts_with('(') && type_str.ends_with(')') {
            let inner = &type_str[1..type_str.len() - 1];
            let elements = self.split_type_args(inner);
            return TypeStructure::Tuple(elements);
        }

        // Array/slice types
        if type_str.starts_with('[') && type_str.ends_with(']') {
            let inner = &type_str[1..type_str.len() - 1];
            if let Some(idx) = inner.find(';') {
                return TypeStructure::Array {
                    element: inner[..idx].trim(),
                    size: Some(inner[idx + 1..].trim()),
                };
            } else {
                return TypeStructure::Slice(inner);
            }
        }

        // Generic types: Name<Args>
        if let Some(idx) = type_str.find('<')
            && type_str.ends_with('>')
        {
            let base = &type_str[..idx];
            let args_str = &type_str[idx + 1..type_str.len() - 1];
            let args = self.split_type_args(args_str);
            return TypeStructure::Generic { base, args };
        }

        // Simple named type or type variable
        if self.is_type_variable(type_str) {
            TypeStructure::Variable(type_str)
        } else {
            TypeStructure::Named(type_str)
        }
    }

    /// Split comma-separated type arguments, respecting nested generics.
    fn split_type_args<'a>(&self, args_str: &'a str) -> List<&'a str> {
        let mut result = List::new();
        let mut depth = 0;
        let mut start = 0;

        for (i, ch) in args_str.char_indices() {
            match ch {
                '<' | '(' | '[' => depth += 1,
                '>' | ')' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    result.push(args_str[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }

        // Add the last argument
        let last = args_str[start..].trim();
        if !last.is_empty() {
            result.push(last);
        }

        result
    }

    /// Check if two type structures may overlap.
    fn type_structures_overlap(&self, t1: &TypeStructure<'_>, t2: &TypeStructure<'_>) -> bool {
        use TypeStructure::*;

        match (t1, t2) {
            // Variables overlap with anything
            (Variable(_), _) | (_, Variable(_)) => true,

            // Named types overlap only if same name
            (Named(n1), Named(n2)) => n1 == n2,

            // Generics with same base may overlap if args could unify
            (Generic { base: b1, args: a1 }, Generic { base: b2, args: a2 }) => {
                if b1 != b2 {
                    return false;
                }
                if a1.len() != a2.len() {
                    return false;
                }
                // Check if all argument pairs may overlap
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    if !self.types_may_overlap(&Text::from(*arg1), &Text::from(*arg2)) {
                        return false;
                    }
                }
                true
            }

            // Tuples overlap if same arity and all elements may overlap
            (Tuple(e1), Tuple(e2)) => {
                if e1.len() != e2.len() {
                    return false;
                }
                for (t1, t2) in e1.iter().zip(e2.iter()) {
                    if !self.types_may_overlap(&Text::from(*t1), &Text::from(*t2)) {
                        return false;
                    }
                }
                true
            }

            // References overlap if same mutability and inner types overlap
            (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && self.types_may_overlap(&Text::from(*i1), &Text::from(*i2)),

            // Arrays overlap if element types overlap
            (Array { element: e1, .. }, Array { element: e2, .. }) => {
                self.types_may_overlap(&Text::from(*e1), &Text::from(*e2))
            }

            // Slices overlap if element types overlap
            (Slice(e1), Slice(e2)) => self.types_may_overlap(&Text::from(*e1), &Text::from(*e2)),

            // A variable (in generic form) overlaps with any compatible structure
            (Generic { base: _, args }, Named(_)) | (Named(_), Generic { base: _, args }) => {
                // Check if any argument is a type variable
                args.iter().any(|a| self.is_type_variable(a))
            }

            // Different structures don't overlap
            _ => false,
        }
    }

    /// Check if a type name is a type variable.
    fn is_type_variable(&self, type_name: &str) -> bool {
        // Single uppercase letter or common type parameter names
        type_name.len() == 1
            && type_name
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
            || type_name == "T"
            || type_name == "U"
            || type_name == "V"
            || type_name == "K"
            || type_name == "E"
    }

    /// Extract base type from a generic type.
    fn extract_base_type<'a>(&self, type_name: &'a str) -> Option<&'a str> {
        if let Some(idx) = type_name.find('<') {
            Some(&type_name[..idx])
        } else {
            None
        }
    }

    /// Validate specialization hierarchy.
    ///
    /// Specialization: specialized implementations override general ones when
    /// marked with @specialize, strictly more specific, and in the same crate.
    /// The specialization hierarchy must form a tree (no diamonds).
    ///
    /// Specialization rules:
    /// 1. Specialized impl must be strictly more specific than general impl
    /// 2. Must use @specialize annotation
    /// 3. Must be in same crate as general impl
    /// 4. Specialization hierarchy must form tree (no diamonds)
    pub fn check_specialization(
        &self,
        specialized: &ImplEntry,
        base: &ImplEntry,
    ) -> Result<(), CoherenceError> {
        // Check same crate requirement
        if specialized.crate_name() != base.crate_name() {
            return Err(CoherenceError::InvalidSpecialization {
                protocol: specialized.protocol.clone(),
                specialized_type: specialized.for_type.clone(),
                base_type: base.for_type.clone(),
                reason: Text::from(format!(
                    "Specialization must be in same crate as base impl. Base is in '{}', specialized is in '{}'",
                    base.crate_name(),
                    specialized.crate_name()
                )),
                span: specialized.span,
            });
        }

        // Check that specialized is more specific
        if !self.is_more_specific(&specialized.for_type, &base.for_type) {
            return Err(CoherenceError::InvalidSpecialization {
                protocol: specialized.protocol.clone(),
                specialized_type: specialized.for_type.clone(),
                base_type: base.for_type.clone(),
                reason: Text::from(
                    "Specialized implementation is not strictly more specific than base",
                ),
                span: specialized.span,
            });
        }

        Ok(())
    }

    /// Check if type1 is more specific than type2.
    ///
    /// A type is more specific if it can be obtained from a more general type
    /// through substitution of type variables with concrete types or more
    /// constrained type variables.
    ///
    /// # Specificity Rules
    ///
    /// 1. Concrete type > Type variable (e.g., `Int` > `T`)
    /// 2. Constrained type > Unconstrained type (e.g., `T: Clone` > `T`)
    /// 3. Same base with more concrete params > more generic params
    ///    (e.g., `List<Int>` > `List<T>`)
    /// 4. Nested specificity applies recursively
    ///
    /// Specialization: specialized implementations override general ones when
    /// marked with @specialize, strictly more specific, and in the same crate.
    /// The specialization hierarchy must form a tree (no diamonds). - Specialization
    fn is_more_specific(&self, specific: &Text, general: &Text) -> bool {
        let specific_str = specific.as_str();
        let general_str = general.as_str();

        // Exact same type is not more specific
        if specific_str == general_str {
            return false;
        }

        // Parse both types into structured forms
        let specific_struct = self.parse_type_structure(specific_str);
        let general_struct = self.parse_type_structure(general_str);

        // Use structured comparison
        self.structure_is_more_specific(&specific_struct, &general_struct, specific, general)
    }

    /// Check if a type structure is more specific than another.
    fn structure_is_more_specific(
        &self,
        specific: &TypeStructure<'_>,
        general: &TypeStructure<'_>,
        specific_text: &Text,
        general_text: &Text,
    ) -> bool {
        use TypeStructure::*;

        match (specific, general) {
            // A variable is never more specific than anything else
            (Variable(_), _) => false,

            // Any concrete type is more specific than a variable
            (Named(_), Variable(_))
            | (Generic { .. }, Variable(_))
            | (Tuple(_), Variable(_))
            | (Reference { .. }, Variable(_))
            | (Array { .. }, Variable(_))
            | (Slice(_), Variable(_)) => true,

            // Same named type - check constraints
            (Named(n1), Named(n2)) => {
                if n1 != n2 {
                    return false;
                }
                // Check if specific has more constraints via the original text
                self.has_more_constraints(specific_text, general_text)
            }

            // Generics with same base - check parameter specificity
            (Generic { base: b1, args: a1 }, Generic { base: b2, args: a2 }) => {
                if b1 != b2 {
                    return false;
                }
                if a1.len() != a2.len() {
                    return false;
                }

                // Count how many args are more specific
                let mut has_more_specific = false;
                let mut has_less_specific = false;

                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    let arg1_text = Text::from(*arg1);
                    let arg2_text = Text::from(*arg2);

                    if arg1 != arg2 {
                        if self.is_more_specific(&arg1_text, &arg2_text) {
                            has_more_specific = true;
                        } else if self.is_more_specific(&arg2_text, &arg1_text) {
                            has_less_specific = true;
                        }
                    }
                }

                // More specific if at least one arg is more specific and none are less
                has_more_specific && !has_less_specific
            }

            // Tuples - check element-wise specificity
            (Tuple(e1), Tuple(e2)) => {
                if e1.len() != e2.len() {
                    return false;
                }

                let mut has_more_specific = false;
                let mut has_less_specific = false;

                for (elem1, elem2) in e1.iter().zip(e2.iter()) {
                    let elem1_text = Text::from(*elem1);
                    let elem2_text = Text::from(*elem2);

                    if elem1 != elem2 {
                        if self.is_more_specific(&elem1_text, &elem2_text) {
                            has_more_specific = true;
                        } else if self.is_more_specific(&elem2_text, &elem1_text) {
                            has_less_specific = true;
                        }
                    }
                }

                has_more_specific && !has_less_specific
            }

            // References - check inner type specificity
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
                if m1 != m2 {
                    return false;
                }
                self.is_more_specific(&Text::from(*i1), &Text::from(*i2))
            }

            // Arrays - check element type specificity
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
                // Sized array is more specific than unsized
                if s1.is_some() && s2.is_none() {
                    return true;
                }
                if s1.is_none() && s2.is_some() {
                    return false;
                }
                self.is_more_specific(&Text::from(*e1), &Text::from(*e2))
            }

            // Slices - check element type specificity
            (Slice(e1), Slice(e2)) => self.is_more_specific(&Text::from(*e1), &Text::from(*e2)),

            // A generic instantiation with concrete params can be more specific than a named type
            (Generic { base, args }, Named(n)) if base == n => {
                // List<Int> is more specific than List only if List implies List<T>
                // This is context-dependent; assume it's more specific if args are concrete
                args.iter().all(|a| !self.is_type_variable(a))
            }

            // Different structures are incomparable
            _ => false,
        }
    }

    /// Check if one type has more constraints than another.
    ///
    /// This checks for "where" clauses and other constraint indicators
    /// in the original type text.
    fn has_more_constraints(&self, specific: &Text, general: &Text) -> bool {
        let specific_str = specific.as_str();
        let general_str = general.as_str();

        // Count constraint indicators
        let specific_constraints = self.count_constraints(specific_str);
        let general_constraints = self.count_constraints(general_str);

        specific_constraints > general_constraints
    }

    /// Count the number of constraints in a type string.
    fn count_constraints(&self, type_str: &str) -> usize {
        let mut count = 0;

        // Count "where" clauses
        count += type_str.matches("where").count();

        // Count protocol bounds (indicated by ":")
        count += type_str.matches(':').count();

        // Count "+" for combined bounds
        count += type_str.matches('+').count();

        count
    }

    /// Extract type parameters from a generic type.
    /// E.g., "List<T, U>" -> ["T", "U"]
    fn extract_type_params<'a>(&self, type_name: &'a str) -> List<&'a str> {
        let mut params = List::new();
        if let Some(start) = type_name.find('<')
            && let Some(end) = type_name.rfind('>')
        {
            let params_str = &type_name[start + 1..end];
            // Simple split - doesn't handle nested generics perfectly
            // but good enough for basic cases
            for param in params_str.split(',') {
                let trimmed = param.trim();
                // Handle nested types by extracting just the outer type
                if let Some(idx) = trimmed.find('<') {
                    params.push(&trimmed[..idx]);
                } else {
                    params.push(trimmed);
                }
            }
        }
        params
    }

    /// Check for cross-crate conflicts.
    ///
    /// Cross-crate conflict detection: collects all implementations from the
    /// dependency graph, builds (Protocol, Type) -> [Implementations] table,
    /// and emits errors if multiple implementations exist for the same pair.
    pub fn check_cross_crate_conflicts(&self) -> List<CoherenceError> {
        let mut errors = List::new();

        for ((_protocol, _for_type), impls) in &self.impl_table {
            if impls.len() <= 1 {
                continue;
            }

            // Group by crate
            let mut crate_impls: HashMap<Text, List<&ImplEntry>> = HashMap::new();
            for imp in impls {
                crate_impls.entry(imp.crate_name()).or_default().push(imp);
            }

            // If more than one crate provides impl, it's a conflict
            if crate_impls.len() > 1 {
                let crate_names: List<Text> = crate_impls.keys().cloned().collect();
                if let (Some(crate1), Some(crate2)) = (crate_names.first(), crate_names.get(1))
                    && let Some(first_impl) = impls.first()
                {
                    errors.push(CoherenceError::ConflictingCrateImpl {
                        protocol: first_impl.protocol.clone(),
                        for_type: first_impl.for_type.clone(),
                        crate1: crate1.clone(),
                        crate2: crate2.clone(),
                        span: first_impl.span,
                    });
                }
            }
        }

        errors
    }

    /// Run all coherence checks.
    ///
    /// Returns a list of all coherence errors found.
    pub fn check_all(&self) -> List<CoherenceError> {
        let mut errors = List::new();

        // Check orphan rules
        for entry in &self.all_impls {
            if let Err(e) = self.check_orphan_rule(entry) {
                errors.push(e);
            }
        }

        // Check overlaps within each (protocol, type) group
        for impls in self.impl_table.values() {
            for i in 0..impls.len() {
                for j in (i + 1)..impls.len() {
                    if let (Some(impl1), Some(impl2)) = (impls.get(i), impls.get(j))
                        && let Err(e) = self.check_overlap(impl1, impl2)
                    {
                        errors.push(e);
                    }
                }
            }
        }

        // Check specialization hierarchies
        for ((_protocol, _for_type), specialized_impls) in &self.specializations {
            for specialized in specialized_impls {
                // Find base implementation
                let key = (specialized.protocol.clone(), specialized.for_type.clone());
                if let Some(all_impls) = self.impl_table.get(&key) {
                    for base in all_impls.iter() {
                        if !base.is_specialized
                            && base != specialized
                            && let Err(e) = self.check_specialization(specialized, base)
                        {
                            errors.push(e);
                        }
                    }
                }
            }
        }

        // Check cross-crate conflicts
        errors.extend(self.check_cross_crate_conflicts());

        errors
    }

    /// Validate all implementations and return first error or Ok.
    pub fn validate(&self) -> Result<(), CoherenceError> {
        let errors = self.check_all();
        if let Some(first_error) = errors.first() {
            Err(first_error.clone())
        } else {
            Ok(())
        }
    }

    /// Get all implementations for a protocol.
    pub fn impls_for_protocol(&self, protocol: &str) -> List<&ImplEntry> {
        self.all_impls
            .iter()
            .filter(|e| e.protocol.as_str() == protocol)
            .collect()
    }

    /// Get implementation for a specific (protocol, type) pair.
    pub fn get_impl(&self, protocol: &str, for_type: &str) -> Maybe<&ImplEntry> {
        let key = (Text::from(protocol), Text::from(for_type));
        if let Some(impls) = self.impl_table.get(&key) {
            // Return most specialized implementation
            for imp in impls {
                if imp.is_specialized {
                    return Maybe::Some(imp);
                }
            }
            if let Some(first) = impls.first() {
                return Maybe::Some(first);
            }
        }
        Maybe::None
    }

    /// Clear all registered implementations.
    pub fn clear(&mut self) {
        self.impl_table.clear();
        self.all_impls.clear();
        self.specializations.clear();
    }
}

impl Default for CoherenceChecker {
    fn default() -> Self {
        Self::new("")
    }
}

/// Convert AST type to string representation for coherence checking.
///
/// This function produces a canonical string representation of types for
/// comparison purposes in coherence checking. The representation is designed
/// to be stable and comparable across different type instances.
///
/// Full coherence check: validates orphan rules, detects overlapping
/// implementations, and verifies specialization hierarchy across all crates.
pub fn type_to_string(ty: &AstType) -> Text {
    type_kind_to_string(&ty.kind)
}

/// Convert AST TypeKind to string representation.
fn type_kind_to_string(kind: &verum_ast::TypeKind) -> Text {
    use verum_ast::TypeKind;

    match kind {
        TypeKind::Unit => Text::from("()"),
        TypeKind::Bool => Text::from(verum_common::well_known_types::type_names::BOOL),
        TypeKind::Int => Text::from(verum_common::well_known_types::type_names::INT),
        TypeKind::Float => Text::from(verum_common::well_known_types::type_names::FLOAT),
        TypeKind::Char => Text::from(verum_common::well_known_types::type_names::CHAR),
        TypeKind::Text => Text::from(verum_common::well_known_types::type_names::TEXT),
        TypeKind::Never => Text::from("!"),

        TypeKind::Path(path) => path_to_string(path),

        TypeKind::Tuple(types) => {
            let parts: List<String> = types
                .iter()
                .map(|t| type_to_string(t).to_string())
                .collect();
            Text::from(format!("({})", parts.join(", ")))
        }

        TypeKind::Array { element, size } => {
            let elem_str = type_to_string(element);
            match size {
                verum_common::Maybe::Some(size_expr) => {
                    Text::from(format!("[{}; {:?}]", elem_str, size_expr))
                }
                verum_common::Maybe::None => Text::from(format!("[{}]", elem_str)),
            }
        }

        TypeKind::Slice(inner) => Text::from(format!("[{}]", type_to_string(inner))),

        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            let param_strs: List<String> = params
                .iter()
                .map(|t| type_to_string(t).to_string())
                .collect();
            let ret_str = type_to_string(return_type);
            Text::from(format!("fn({}) -> {}", param_strs.join(", "), ret_str))
        }

        TypeKind::Rank2Function {
            type_params,
            params,
            return_type,
            ..
        } => {
            let type_param_strs: List<String> = type_params
                .iter()
                .map(generic_param_to_string)
                .collect();
            let param_strs: List<String> = params
                .iter()
                .map(|t| type_to_string(t).to_string())
                .collect();
            let ret_str = type_to_string(return_type);
            Text::from(format!(
                "fn<{}>({}) -> {}",
                type_param_strs.join(", "),
                param_strs.join(", "),
                ret_str
            ))
        }

        TypeKind::Reference { mutable, inner } => {
            if *mutable {
                Text::from(format!("&mut {}", type_to_string(inner)))
            } else {
                Text::from(format!("&{}", type_to_string(inner)))
            }
        }

        TypeKind::CheckedReference { mutable, inner } => {
            if *mutable {
                Text::from(format!("&checked mut {}", type_to_string(inner)))
            } else {
                Text::from(format!("&checked {}", type_to_string(inner)))
            }
        }

        TypeKind::UnsafeReference { mutable, inner } => {
            if *mutable {
                Text::from(format!("&unsafe mut {}", type_to_string(inner)))
            } else {
                Text::from(format!("&unsafe {}", type_to_string(inner)))
            }
        }

        TypeKind::Pointer { mutable, inner } => {
            if *mutable {
                Text::from(format!("*mut {}", type_to_string(inner)))
            } else {
                Text::from(format!("*{}", type_to_string(inner)))
            }
        }

        TypeKind::VolatilePointer { mutable, inner } => {
            if *mutable {
                Text::from(format!("*volatile mut {}", type_to_string(inner)))
            } else {
                Text::from(format!("*volatile {}", type_to_string(inner)))
            }
        }

        TypeKind::Generic { base, args } => {
            let args_strs: List<String> = args.iter().map(generic_arg_to_string).collect();
            Text::from(format!(
                "{}<{}>",
                type_to_string(base),
                args_strs.join(", ")
            ))
        }

        TypeKind::Qualified {
            self_ty,
            trait_ref,
            assoc_name,
        } => Text::from(format!(
            "<{} as {}>::{}",
            type_to_string(self_ty),
            path_to_string(trait_ref),
            assoc_name.name
        )),

        TypeKind::Refined { base, predicate } => {
            // Refinement types carry all three surface forms :
            // inline `T{pred}`, declarative `T where p`, and sigma
            // `x: T where p(x)`. Render the sigma form when the
            // predicate carries an explicit binder.
            match &predicate.binding {
                verum_common::Maybe::Some(binder) => Text::from(format!(
                    "{}: {} where ...",
                    binder.name,
                    type_to_string(base)
                )),
                verum_common::Maybe::None => {
                    Text::from(format!("{{{}: _ | ... }}", type_to_string(base)))
                }
            }
        }

        TypeKind::Inferred => Text::from("_"),

        TypeKind::Bounded { base, bounds } => {
            let bounds_strs: List<String> = bounds.iter().map(type_bound_to_string).collect();
            Text::from(format!(
                "{} where {}",
                type_to_string(base),
                bounds_strs.join(" + ")
            ))
        }

        TypeKind::DynProtocol { bounds, bindings } => {
            let mut parts: List<String> = bounds.iter().map(type_bound_to_string).collect();
            if let verum_common::Maybe::Some(bindings_list) = bindings {
                for binding in bindings_list.iter() {
                    parts.push(format!(
                        "{} = {}",
                        binding.name.name,
                        type_to_string(&binding.ty)
                    ));
                }
            }
            Text::from(format!("dyn {}", parts.join(" + ")))
        }

        TypeKind::Ownership { mutable, inner } => {
            if *mutable {
                Text::from(format!("%mut {}", type_to_string(inner)))
            } else {
                Text::from(format!("%{}", type_to_string(inner)))
            }
        }

        TypeKind::GenRef { inner } => Text::from(format!("GenRef<{}>", type_to_string(inner))),

        TypeKind::TypeConstructor { base, arity } => Text::from(format!(
            "TypeConstructor<{}, arity={}>",
            type_to_string(base),
            arity
        )),

        TypeKind::Tensor { element, shape, .. } => {
            let shape_strs: List<String> = shape.iter().map(|s| format!("{:?}", s)).collect();
            Text::from(format!(
                "Tensor<{}, [{}]>",
                type_to_string(element),
                shape_strs.join(", ")
            ))
        }

        TypeKind::Existential { name, bounds } => {
            let bounds_strs: List<String> = bounds.iter().map(type_bound_to_string).collect();
            if bounds.is_empty() {
                Text::from(format!("some {}", name.name))
            } else {
                Text::from(format!("some {}: {}", name.name, bounds_strs.join(" + ")))
            }
        }

        TypeKind::AssociatedType { base, assoc } => {
            Text::from(format!("{}.{}", type_to_string(base), assoc.name))
        }

        TypeKind::CapabilityRestricted { base, capabilities } => {
            let cap_strs: List<String> = capabilities
                .capabilities
                .iter()
                .map(|c| c.as_str().to_string())
                .collect();
            Text::from(format!(
                "{} with [{}]",
                type_to_string(base),
                cap_strs.join(", ")
            ))
        }

        // Unknown type - a safe top type
        TypeKind::Unknown => Text::from("Unknown"),

        // Record types - anonymous record format
        TypeKind::Record { fields, .. } => {
            let field_strs: List<String> = fields
                .iter()
                .map(|f| format!("{}: {}", f.name.name, type_to_string(&f.ty)))
                .collect();
            Text::from(format!("{{ {} }}", field_strs.join(", ")))
        }

        // Universe types: Type, Type(0), Type(1), Type(u)
        TypeKind::Universe { level } => {
            match level {
                verum_common::Maybe::None => Text::from("Type"),
                verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Concrete(n)) => {
                    Text::from(format!("Type({})", n))
                }
                verum_common::Maybe::Some(verum_ast::UniverseLevelExpr::Variable(ident)) => {
                    Text::from(format!("Type({})", ident.name))
                }
                verum_common::Maybe::Some(_) => Text::from("Type"),
            }
        }

        // Meta types: meta T
        TypeKind::Meta { inner } => {
            Text::from(format!("meta {}", type_to_string(inner)))
        }

        // Type lambdas: |x| T
        TypeKind::TypeLambda { params, body } => {
            let param_strs: List<String> = params.iter().map(|p| p.name.to_string()).collect();
            Text::from(format!("|{}| {}", param_strs.join(", "), type_to_string(body)))
        }

        // Path equality type: Path<A>(lhs, rhs)
        TypeKind::PathType { carrier, lhs, rhs } => {
            Text::from(format!("Path<{}>({:?}, {:?})", type_to_string(carrier), lhs, rhs))
        }
        // General dependent-type application: T<A1..>(v1, v2, ..).
        TypeKind::DependentApp { carrier, value_args } => {
            let parts: Vec<String> = value_args.iter().map(|v| format!("{:?}", v)).collect();
            Text::from(format!(
                "{}({})",
                type_to_string(carrier),
                parts.join(", ")
            ))
        }
    }
}

/// Convert a generic argument to string.
fn generic_arg_to_string(arg: &verum_ast::ty::GenericArg) -> String {
    use verum_ast::ty::GenericArg;
    match arg {
        GenericArg::Type(ty) => type_to_string(ty).to_string(),
        GenericArg::Const(expr) => format!("{:?}", expr),
        GenericArg::Lifetime(lt) => format!("'{}", lt.name),
        GenericArg::Binding(binding) => {
            format!("{} = {}", binding.name.name, type_to_string(&binding.ty))
        }
    }
}

/// Convert a generic parameter to string.
fn generic_param_to_string(param: &verum_ast::ty::GenericParam) -> String {
    use verum_ast::ty::GenericParamKind;
    match &param.kind {
        GenericParamKind::Type { name, bounds, .. } => {
            if bounds.is_empty() {
                name.name.to_string()
            } else {
                let bounds_str: Vec<String> = bounds.iter().map(type_bound_to_string).collect();
                format!("{}: {}", name.name, bounds_str.join(" + "))
            }
        }
        GenericParamKind::HigherKinded { name, arity, bounds, .. } => {
            let arity_str = "_".repeat(*arity);
            if bounds.is_empty() {
                format!("{}<{}>", name.name, arity_str)
            } else {
                let bounds_str: Vec<String> = bounds.iter().map(type_bound_to_string).collect();
                format!("{}<{}>: {}", name.name, arity_str, bounds_str.join(" + "))
            }
        }
        GenericParamKind::Const { name, ty, .. } => {
            format!("const {}: {}", name.name, type_to_string(ty))
        }
        GenericParamKind::Lifetime { name, .. } => {
            format!("'{}", name.name)
        }
        GenericParamKind::Meta { name, ty, .. } => {
            format!("{}: meta {}", name.name, type_to_string(ty))
        }
        GenericParamKind::Context { name } => {
            format!("using {}", name.name)
        }
        GenericParamKind::Level { name } => {
            format!("{}: Level", name.name)
        }
        GenericParamKind::KindAnnotated { name, kind, bounds } => {
            if bounds.is_empty() {
                format!("{}: {}", name.name, kind)
            } else {
                let bounds_str: Vec<String> = bounds.iter().map(type_bound_to_string).collect();
                format!("{}: {} + {}", name.name, kind, bounds_str.join(" + "))
            }
        }
    }
}

/// Convert a type bound to string representation.
fn type_bound_to_string(bound: &verum_ast::ty::TypeBound) -> String {
    use verum_ast::ty::TypeBoundKind;

    // TypeBound contains kind which has the protocol path
    match &bound.kind {
        TypeBoundKind::Protocol(path) => path_to_string(path).to_string(),
        TypeBoundKind::Equality(ty) => format!("= {:?}", ty),
        TypeBoundKind::NegativeProtocol(path) => format!("!{}", path_to_string(path)),
        TypeBoundKind::AssociatedTypeBound {
            type_path,
            assoc_name,
            bounds,
        } => {
            let bounds_str: Vec<String> = bounds.iter().map(type_bound_to_string).collect();
            format!(
                "{}.{}: {}",
                path_to_string(type_path),
                assoc_name.name,
                bounds_str.join(" + ")
            )
        }
        TypeBoundKind::AssociatedTypeEquality {
            type_path,
            assoc_name,
            eq_type,
        } => {
            format!(
                "{}.{} = {}",
                path_to_string(type_path),
                assoc_name.name,
                type_to_string(eq_type)
            )
        }
        TypeBoundKind::GenericProtocol(ty) => type_to_string(ty).into_string(),
    }
}

/// Check if a string is a type system keyword (should not be treated as a type name).
fn is_type_keyword(s: &str) -> bool {
    matches!(
        s,
        "mut"
            | "fn"
            | "dyn"
            | "impl"
            | "where"
            | "for"
            | "in"
            | "const"
            | "static"
            | "ref"
            | "checked"
            | "unsafe"
            | "linear"
            | "affine"
            | "exists"
            | "typeof"
    )
}

/// Convert AST path to string representation.
pub fn path_to_string(path: &Path) -> Text {
    let segments: List<String> = path
        .segments
        .iter()
        .filter_map(|seg| match seg {
            verum_ast::PathSegment::Name(ident) => Some(ident.name.to_string()),
            _ => None,
        })
        .collect();
    segments.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_impl(protocol: &str, for_type: &str, module: &str) -> ImplEntry {
        ImplEntry::new(
            Text::from(protocol),
            ModulePath::from_str(protocol),
            Text::from(for_type),
            ModulePath::from_str(module),
            ModuleId::new(0),
        )
    }

    #[test]
    fn test_orphan_rule_local_protocol() {
        let mut checker = CoherenceChecker::new("my_crate");
        checker.register_local_protocol(
            Text::from("MyProtocol"),
            ModulePath::from_str("my_crate.protocols"),
        );

        let entry = create_impl("MyProtocol", "List<Int>", "my_crate.impls");
        assert!(checker.check_orphan_rule(&entry).is_ok());
    }

    #[test]
    fn test_orphan_rule_local_type() {
        let mut checker = CoherenceChecker::new("my_crate");
        checker.register_local_type(Text::from("MyType"), ModulePath::from_str("my_crate.types"));

        let entry = create_impl("std.Display", "MyType", "my_crate.impls");
        assert!(checker.check_orphan_rule(&entry).is_ok());
    }

    #[test]
    fn test_orphan_rule_violation() {
        let checker = CoherenceChecker::new("my_crate");

        // Neither Display nor List is local
        let entry = create_impl("std.Display", "std.List<Int>", "my_crate.impls");
        assert!(checker.check_orphan_rule(&entry).is_err());
    }

    #[test]
    fn test_orphan_rule_local_type_param() {
        let mut checker = CoherenceChecker::new("my_crate");
        checker.register_local_type(
            Text::from("MyWrapper"),
            ModulePath::from_str("my_crate.types"),
        );

        // List<MyWrapper> uses a local type parameter
        let entry = create_impl("std.Iterator", "List<MyWrapper>", "my_crate.impls");
        assert!(checker.check_orphan_rule(&entry).is_ok());
    }

    #[test]
    fn test_overlap_detection_same_type() {
        let checker = CoherenceChecker::new("my_crate");

        let entry1 = create_impl("Display", "Maybe<T>", "my_crate.impls1");
        let entry2 = create_impl("Display", "Maybe<T>", "my_crate.impls2");

        assert!(checker.check_overlap(&entry1, &entry2).is_err());
    }

    #[test]
    fn test_overlap_with_specialization() {
        let checker = CoherenceChecker::new("my_crate");

        let entry1 = create_impl("Display", "Maybe<T>", "my_crate.impls");
        let entry2 = create_impl("Display", "Maybe<T>", "my_crate.impls").with_specialized();

        // Specialization allows overlap
        assert!(checker.check_overlap(&entry1, &entry2).is_ok());
    }

    #[test]
    fn test_no_overlap_different_types() {
        let checker = CoherenceChecker::new("my_crate");

        let entry1 = create_impl("Display", "Int", "my_crate.impls");
        let entry2 = create_impl("Display", "Float", "my_crate.impls");

        assert!(checker.check_overlap(&entry1, &entry2).is_ok());
    }

    #[test]
    fn test_specialization_same_crate() {
        let checker = CoherenceChecker::new("my_crate");

        let base = create_impl("Clone", "List<T>", "my_crate.impls");
        let specialized = create_impl("Clone", "List<u8>", "my_crate.impls").with_specialized();

        assert!(checker.check_specialization(&specialized, &base).is_ok());
    }

    #[test]
    fn test_specialization_different_crate() {
        let checker = CoherenceChecker::new("my_crate");

        let base = create_impl("Clone", "List<T>", "other_crate.impls");
        let specialized = create_impl("Clone", "List<u8>", "my_crate.impls").with_specialized();

        assert!(checker.check_specialization(&specialized, &base).is_err());
    }

    #[test]
    fn test_cross_crate_conflict() {
        let mut checker = CoherenceChecker::new("my_crate");

        checker.add_impl(create_impl("Display", "ExternalType", "crate_a.impls"));
        checker.add_impl(create_impl("Display", "ExternalType", "crate_b.impls"));

        let conflicts = checker.check_cross_crate_conflicts();
        assert!(!conflicts.is_empty());
    }

    #[test]
    fn test_check_all_valid() {
        let mut checker = CoherenceChecker::new("my_crate");
        checker.register_local_type(Text::from("MyType"), ModulePath::from_str("my_crate.types"));
        checker.register_local_protocol(
            Text::from("MyProtocol"),
            ModulePath::from_str("my_crate.protocols"),
        );

        checker.add_impl(create_impl("MyProtocol", "Int", "my_crate.impls"));
        checker.add_impl(create_impl("std.Display", "MyType", "my_crate.impls"));

        let errors = checker.check_all();
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_get_impl() {
        let mut checker = CoherenceChecker::new("my_crate");

        let entry = create_impl("Display", "Int", "my_crate.impls");
        checker.add_impl(entry.clone());

        let result = checker.get_impl("Display", "Int");
        assert!(matches!(result, Maybe::Some(_)));
    }

    #[test]
    fn test_impls_for_protocol() {
        let mut checker = CoherenceChecker::new("my_crate");

        checker.add_impl(create_impl("Display", "Int", "my_crate.impls"));
        checker.add_impl(create_impl("Display", "Float", "my_crate.impls"));
        checker.add_impl(create_impl("Clone", "Int", "my_crate.impls"));

        let display_impls = checker.impls_for_protocol("Display");
        assert_eq!(display_impls.len(), 2);

        let clone_impls = checker.impls_for_protocol("Clone");
        assert_eq!(clone_impls.len(), 1);
    }

    #[test]
    fn test_extract_all_type_names() {
        let checker = CoherenceChecker::new("my_crate");

        // Simple type
        let types = checker.extract_all_type_names(&Text::from("Int"));
        assert!(types.contains(&Text::from("Int")));

        // Generic type
        let types = checker.extract_all_type_names(&Text::from("List<MyType>"));
        assert!(types.contains(&Text::from("List")));
        assert!(types.contains(&Text::from("MyType")));

        // Nested generic
        let types = checker.extract_all_type_names(&Text::from("Map<Key, List<Value>>"));
        assert!(types.contains(&Text::from("Map")));
        assert!(types.contains(&Text::from("Key")));
        assert!(types.contains(&Text::from("List")));
        assert!(types.contains(&Text::from("Value")));

        // Reference type
        let types = checker.extract_all_type_names(&Text::from("&mut T"));
        assert!(types.contains(&Text::from("T")));

        // Function type
        let types = checker.extract_all_type_names(&Text::from("fn(A, B) -> C"));
        assert!(types.contains(&Text::from("A")));
        assert!(types.contains(&Text::from("B")));
        assert!(types.contains(&Text::from("C")));
    }

    #[test]
    fn test_types_may_overlap_exact_match() {
        let checker = CoherenceChecker::new("my_crate");
        assert!(checker.types_may_overlap(&Text::from("Int"), &Text::from("Int")));
    }

    #[test]
    fn test_types_may_overlap_type_variable() {
        let checker = CoherenceChecker::new("my_crate");
        // Type variable overlaps with anything
        assert!(checker.types_may_overlap(&Text::from("T"), &Text::from("Int")));
        assert!(checker.types_may_overlap(&Text::from("List<Int>"), &Text::from("T")));
    }

    #[test]
    fn test_types_may_overlap_different_concrete() {
        let checker = CoherenceChecker::new("my_crate");
        // Different concrete types don't overlap
        assert!(!checker.types_may_overlap(&Text::from("Int"), &Text::from("Float")));
    }

    #[test]
    fn test_types_may_overlap_generic_same_base() {
        let checker = CoherenceChecker::new("my_crate");
        // Same base type with overlapping params
        assert!(checker.types_may_overlap(&Text::from("List<T>"), &Text::from("List<Int>")));
        // Different arities don't overlap
        assert!(!checker.types_may_overlap(&Text::from("Result<T, E>"), &Text::from("Result<T>")));
    }

    #[test]
    fn test_types_may_overlap_tuples() {
        let checker = CoherenceChecker::new("my_crate");
        // Tuples with same arity and overlapping elements
        assert!(checker.types_may_overlap(&Text::from("(A, B)"), &Text::from("(Int, Float)")));
        // Different arities
        assert!(!checker.types_may_overlap(&Text::from("(A, B)"), &Text::from("(A, B, C)")));
    }

    #[test]
    fn test_is_more_specific_concrete_vs_variable() {
        let checker = CoherenceChecker::new("my_crate");
        // Concrete type is more specific than type variable
        assert!(checker.is_more_specific(&Text::from("Int"), &Text::from("T")));
        assert!(!checker.is_more_specific(&Text::from("T"), &Text::from("Int")));
    }

    #[test]
    fn test_is_more_specific_same_type() {
        let checker = CoherenceChecker::new("my_crate");
        // Same type is not more specific
        assert!(!checker.is_more_specific(&Text::from("Int"), &Text::from("Int")));
    }

    #[test]
    fn test_is_more_specific_generic_params() {
        let checker = CoherenceChecker::new("my_crate");
        // List<Int> is more specific than List<T>
        assert!(checker.is_more_specific(&Text::from("List<Int>"), &Text::from("List<T>")));
        assert!(!checker.is_more_specific(&Text::from("List<T>"), &Text::from("List<Int>")));
    }

    #[test]
    fn test_orphan_rule_nested_local_type() {
        let mut checker = CoherenceChecker::new("my_crate");
        checker.register_local_type(
            Text::from("LocalType"),
            ModulePath::from_str("my_crate.types"),
        );

        // Map<String, LocalType> should pass orphan rule because it contains LocalType
        let entry = create_impl("std.Serialize", "Map<String, LocalType>", "my_crate.impls");
        assert!(checker.check_orphan_rule(&entry).is_ok());
    }

    #[test]
    fn test_overlap_with_references() {
        let checker = CoherenceChecker::new("my_crate");

        // Same reference type
        let entry1 = create_impl("Display", "&Int", "my_crate.impls1");
        let entry2 = create_impl("Display", "&Int", "my_crate.impls2");
        assert!(checker.check_overlap(&entry1, &entry2).is_err());

        // Different mutability - should not overlap
        let entry3 = create_impl("Display", "&Int", "my_crate.impls1");
        let entry4 = create_impl("Display", "&mut Int", "my_crate.impls2");
        assert!(checker.check_overlap(&entry3, &entry4).is_ok());
    }
}
