//! Dependency Analysis Phase for Embedded Constraints
//!
//! This module implements dependency analysis for tracking item requirements
//! (`needs_alloc`, `needs_os`, `needs_runtime`) and validating them against
//! target constraints for embedded and no_alloc builds.
//!
//! ## Architecture
//!
//! The unified stdlib uses compiler-driven dependency analysis rather than
//! physical separation into separate cogs (like Rust's core/alloc/std).
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                    STDLIB LOGICAL LAYERS                                 │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │  Layer 3: OS-dependent (async/, io/, net/, sys/)                        │
//! │           @requires(os) @requires(runtime)                               │
//! │                                                                          │
//! │  Layer 2: Allocation-dependent (collections/, text/, mem/)              │
//! │           @requires(alloc)                                               │
//! │                                                                          │
//! │  Layer 1: Core (core/, intrinsics) - NO REQUIREMENTS                    │
//! │           Primitives, Maybe, Result, Eq, Ord, Clone, etc.               │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Dependency analysis: validates items against target profile constraints
//! (no_alloc, no_std, embedded, cbgr_static_only, no_gpu).

use std::collections::HashMap;

use verum_ast::{Expr, ExprKind, Item, ItemKind, Module, Span, Type, TypeKind};
use verum_common::{List, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Span as DiagSpan};

// ============================================================================
// Item Requirements
// ============================================================================

/// Dependency requirements for an item
///
/// These requirements are propagated through the dependency graph during
/// semantic analysis. Any item that uses another item inherits its requirements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemRequirements {
    /// Requires heap allocation (Heap<T>, List<T>, Text, Map, Set)
    pub needs_alloc: bool,

    /// Requires operating system (File, TcpStream, spawn, etc.)
    pub needs_os: bool,

    /// Requires async runtime (async fn, await, spawn)
    pub needs_runtime: bool,

    /// Requires CBGR runtime checks (Tier 0 references)
    pub needs_cbgr_runtime: bool,

    /// Requires GPU backend
    pub needs_gpu: bool,
}

impl ItemRequirements {
    /// Create empty requirements (Layer 1 core - always available)
    pub const fn none() -> Self {
        Self {
            needs_alloc: false,
            needs_os: false,
            needs_runtime: false,
            needs_cbgr_runtime: false,
            needs_gpu: false,
        }
    }

    /// Create requirements for allocation-dependent items (Layer 2)
    pub const fn alloc() -> Self {
        Self {
            needs_alloc: true,
            needs_os: false,
            needs_runtime: false,
            needs_cbgr_runtime: false,
            needs_gpu: false,
        }
    }

    /// Create requirements for OS-dependent items (Layer 3)
    pub const fn os() -> Self {
        Self {
            needs_alloc: true,  // OS features typically need allocation
            needs_os: true,
            needs_runtime: false,
            needs_cbgr_runtime: false,
            needs_gpu: false,
        }
    }

    /// Create requirements for async/runtime items
    pub const fn runtime() -> Self {
        Self {
            needs_alloc: true,
            needs_os: true,
            needs_runtime: true,
            needs_cbgr_runtime: false,
            needs_gpu: false,
        }
    }

    /// Create requirements for GPU items
    pub const fn gpu() -> Self {
        Self {
            needs_alloc: true,
            needs_os: true,
            needs_runtime: true,
            needs_cbgr_runtime: false,
            needs_gpu: true,
        }
    }

    /// Merge requirements from another item (union of requirements)
    pub fn merge(&mut self, other: &ItemRequirements) {
        self.needs_alloc |= other.needs_alloc;
        self.needs_os |= other.needs_os;
        self.needs_runtime |= other.needs_runtime;
        self.needs_cbgr_runtime |= other.needs_cbgr_runtime;
        self.needs_gpu |= other.needs_gpu;
    }

    /// Check if item is compatible with embedded target (no_alloc + no_std)
    pub fn is_embedded_compatible(&self) -> bool {
        !self.needs_alloc && !self.needs_os && !self.needs_runtime
    }

    /// Check if item is compatible with no_alloc target
    pub fn is_no_alloc_compatible(&self) -> bool {
        !self.needs_alloc
    }

    /// Check if item is compatible with no_std target
    pub fn is_no_std_compatible(&self) -> bool {
        !self.needs_os
    }

    /// Get description of requirements for error messages
    pub fn describe(&self) -> Text {
        let mut parts = Vec::new();
        if self.needs_alloc {
            parts.push("allocator");
        }
        if self.needs_os {
            parts.push("operating system");
        }
        if self.needs_runtime {
            parts.push("async runtime");
        }
        if self.needs_cbgr_runtime {
            parts.push("CBGR runtime");
        }
        if self.needs_gpu {
            parts.push("GPU backend");
        }
        if parts.is_empty() {
            Text::from("none")
        } else {
            Text::from(parts.join(", "))
        }
    }
}

// ============================================================================
// Target Profile
// ============================================================================

/// Target profile with constraint flags
///
/// These flags are set based on `@cfg(no_alloc)`, `@cfg(no_std)`, etc.
#[derive(Debug, Clone, Default)]
pub struct TargetProfile {
    /// Target name (e.g., "x86_64-unknown-linux-gnu")
    pub name: Text,

    /// Disallow heap allocation
    pub no_alloc: bool,

    /// Disallow OS dependencies
    pub no_std: bool,

    /// Embedded target (no_alloc + no_std + static CBGR only)
    pub embedded: bool,

    /// Require static CBGR only (no Tier 0 runtime checks)
    pub cbgr_static_only: bool,

    /// Disallow GPU
    pub no_gpu: bool,
}

impl TargetProfile {
    /// Create a default profile (all features available)
    pub fn default_profile() -> Self {
        Self {
            name: Text::from("default"),
            no_alloc: false,
            no_std: false,
            embedded: false,
            cbgr_static_only: false,
            no_gpu: false,
        }
    }

    /// Create an embedded profile (maximum restrictions)
    pub fn embedded() -> Self {
        Self {
            name: Text::from("embedded"),
            no_alloc: true,
            no_std: true,
            embedded: true,
            cbgr_static_only: true,
            no_gpu: true,
        }
    }

    /// Create a no_std profile
    pub fn no_std() -> Self {
        Self {
            name: Text::from("no_std"),
            no_alloc: false,
            no_std: true,
            embedded: false,
            cbgr_static_only: false,
            no_gpu: false,
        }
    }

    /// Create a no_alloc profile
    pub fn no_alloc() -> Self {
        Self {
            name: Text::from("no_alloc"),
            no_alloc: true,
            no_std: false,
            embedded: false,
            cbgr_static_only: false,
            no_gpu: false,
        }
    }
}

// ============================================================================
// Target Validation Errors
// ============================================================================

/// Error when an item's requirements don't match target constraints
#[derive(Debug, Clone)]
pub enum TargetError {
    /// Item requires allocator but target is no_alloc
    RequiresAllocator {
        item: Text,
        span: Span,
        suggestion: Option<Text>,
    },

    /// Item requires OS but target is no_std
    RequiresOs {
        item: Text,
        span: Span,
    },

    /// Item requires async runtime but not available
    RequiresRuntime {
        item: Text,
        span: Span,
    },

    /// Item uses Tier 0 CBGR but target requires static-only
    CbgrTierViolation {
        item: Text,
        span: Span,
        message: Text,
    },

    /// Item requires GPU but target doesn't support it
    RequiresGpu {
        item: Text,
        span: Span,
    },
}

impl TargetError {
    /// Convert to diagnostic
    pub fn to_diagnostic(&self, span_converter: impl Fn(Span) -> DiagSpan) -> Diagnostic {
        match self {
            TargetError::RequiresAllocator { item, span, suggestion } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("Cannot use `{}` - requires allocator", item.as_str()))
                    .span_label(span_converter(*span), "requires allocator");

                if let Some(sug) = suggestion {
                    builder = builder.add_note(sug.as_str());
                }
                builder.build()
            }
            TargetError::RequiresOs { item, span } => {
                DiagnosticBuilder::error()
                    .message(format!("Cannot use `{}` - requires operating system", item.as_str()))
                    .span_label(span_converter(*span), "requires OS")
                    .build()
            }
            TargetError::RequiresRuntime { item, span } => {
                DiagnosticBuilder::error()
                    .message(format!("Cannot use `{}` - requires async runtime", item.as_str()))
                    .span_label(span_converter(*span), "requires runtime")
                    .add_note("Use synchronous version or provide custom runtime")
                    .build()
            }
            TargetError::CbgrTierViolation { item, span, message } => {
                DiagnosticBuilder::error()
                    .message(format!("CBGR tier violation for `{}`: {}", item.as_str(), message.as_str()))
                    .span_label(span_converter(*span), "tier violation")
                    .add_note("Embedded targets require Tier 1 or Tier 2 references only")
                    .build()
            }
            TargetError::RequiresGpu { item, span } => {
                DiagnosticBuilder::error()
                    .message(format!("Cannot use `{}` - requires GPU backend", item.as_str()))
                    .span_label(span_converter(*span), "requires GPU")
                    .build()
            }
        }
    }
}

// ============================================================================
// Dependency Analyzer
// ============================================================================

/// Analyzer for tracking and validating item requirements
pub struct DependencyAnalyzer {
    /// Requirements cache by item path
    requirements: HashMap<Text, ItemRequirements>,

    /// Target profile for validation
    profile: TargetProfile,

    /// Built-in type requirements
    builtin_requirements: HashMap<&'static str, ItemRequirements>,
}

impl DependencyAnalyzer {
    /// Create a new analyzer with the given target profile
    pub fn new(profile: TargetProfile) -> Self {
        let mut analyzer = Self {
            requirements: HashMap::new(),
            profile,
            builtin_requirements: HashMap::new(),
        };
        analyzer.init_builtin_requirements();
        analyzer
    }

    /// Initialize requirements for built-in types
    fn init_builtin_requirements(&mut self) {
        // Layer 1: Core types (no requirements)
        self.builtin_requirements.insert("Bool", ItemRequirements::none());
        self.builtin_requirements.insert("Int", ItemRequirements::none());
        self.builtin_requirements.insert("Float", ItemRequirements::none());
        self.builtin_requirements.insert("Char", ItemRequirements::none());
        self.builtin_requirements.insert("Unit", ItemRequirements::none());
        self.builtin_requirements.insert("Never", ItemRequirements::none());
        self.builtin_requirements.insert("Array", ItemRequirements::none());
        self.builtin_requirements.insert("Maybe", ItemRequirements::none());
        self.builtin_requirements.insert("Result", ItemRequirements::none());
        self.builtin_requirements.insert("Ordering", ItemRequirements::none());

        // Layer 2: Allocation-dependent types
        self.builtin_requirements.insert("Heap", ItemRequirements::alloc());
        self.builtin_requirements.insert("List", ItemRequirements::alloc());
        self.builtin_requirements.insert("Text", ItemRequirements::alloc());
        self.builtin_requirements.insert("Map", ItemRequirements::alloc());
        self.builtin_requirements.insert("Set", ItemRequirements::alloc());
        self.builtin_requirements.insert("Vec", ItemRequirements::alloc());
        self.builtin_requirements.insert("String", ItemRequirements::alloc());
        self.builtin_requirements.insert("HashMap", ItemRequirements::alloc());
        self.builtin_requirements.insert("HashSet", ItemRequirements::alloc());

        // Layer 3: OS-dependent types
        self.builtin_requirements.insert("File", ItemRequirements::os());
        self.builtin_requirements.insert("Path", ItemRequirements::os());
        self.builtin_requirements.insert("TcpStream", ItemRequirements::os());
        self.builtin_requirements.insert("TcpListener", ItemRequirements::os());
        self.builtin_requirements.insert("UdpSocket", ItemRequirements::os());
        self.builtin_requirements.insert("Thread", ItemRequirements::os());
        self.builtin_requirements.insert("Process", ItemRequirements::os());

        // Runtime-dependent types
        self.builtin_requirements.insert("Future", ItemRequirements::runtime());
        self.builtin_requirements.insert("Task", ItemRequirements::runtime());
        self.builtin_requirements.insert("Channel", ItemRequirements::runtime());

        // GPU types
        self.builtin_requirements.insert("GpuBuffer", ItemRequirements::gpu());
        self.builtin_requirements.insert("GpuKernel", ItemRequirements::gpu());
        self.builtin_requirements.insert("Tensor", ItemRequirements::gpu());
    }

    /// Get requirements for a built-in type
    pub fn get_builtin_requirements(&self, name: &str) -> Option<&ItemRequirements> {
        self.builtin_requirements.get(name)
    }

    /// Analyze requirements for a type
    pub fn analyze_type(&mut self, ty: &Type) -> ItemRequirements {
        let mut reqs = ItemRequirements::none();

        match &ty.kind {
            TypeKind::Path(path) => {
                // Check if this is a known type
                if let Some(ident) = path.as_ident() {
                    if let Some(builtin_reqs) = self.get_builtin_requirements(ident.as_str()) {
                        reqs.merge(builtin_reqs);
                    }
                }
            }
            TypeKind::Generic { base, args } => {
                // Analyze base type - need to extract path from type's kind
                if let TypeKind::Path(path) = &base.kind {
                    if let Some(ident) = path.as_ident() {
                        if let Some(builtin_reqs) = self.get_builtin_requirements(ident.as_str()) {
                            reqs.merge(builtin_reqs);
                        }
                    }
                }
                // Analyze type arguments
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(arg_ty) = arg {
                        let arg_reqs = self.analyze_type(arg_ty);
                        reqs.merge(&arg_reqs);
                    }
                }
            }
            TypeKind::Reference { inner, .. } => {
                let inner_reqs = self.analyze_type(inner);
                reqs.merge(&inner_reqs);
            }
            TypeKind::Tuple(elems) => {
                for elem in elems {
                    let elem_reqs = self.analyze_type(elem);
                    reqs.merge(&elem_reqs);
                }
            }
            TypeKind::Function { params, return_type, .. } => {
                for param in params {
                    let param_reqs = self.analyze_type(param);
                    reqs.merge(&param_reqs);
                }
                let ret_reqs = self.analyze_type(return_type);
                reqs.merge(&ret_reqs);
            }
            _ => {}
        }

        reqs
    }

    /// Analyze requirements for an expression
    pub fn analyze_expr(&mut self, expr: &Expr) -> ItemRequirements {
        let mut reqs = ItemRequirements::none();

        match &expr.kind {
            ExprKind::Await(inner) => {
                reqs.needs_runtime = true;
                let inner_reqs = self.analyze_expr(inner);
                reqs.merge(&inner_reqs);
            }
            ExprKind::Call { func, args, .. } => {
                let func_reqs = self.analyze_expr(func);
                reqs.merge(&func_reqs);
                for arg in args {
                    let arg_reqs = self.analyze_expr(arg);
                    reqs.merge(&arg_reqs);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                let recv_reqs = self.analyze_expr(receiver);
                reqs.merge(&recv_reqs);
                for arg in args {
                    let arg_reqs = self.analyze_expr(arg);
                    reqs.merge(&arg_reqs);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                let left_reqs = self.analyze_expr(left);
                let right_reqs = self.analyze_expr(right);
                reqs.merge(&left_reqs);
                reqs.merge(&right_reqs);
            }
            ExprKind::Unary { expr: inner, .. } => {
                let inner_reqs = self.analyze_expr(inner);
                reqs.merge(&inner_reqs);
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                let cond_reqs = self.analyze_if_condition(condition);
                reqs.merge(&cond_reqs);
                let then_reqs = self.analyze_block(then_branch);
                reqs.merge(&then_reqs);
                if let Some(else_expr) = else_branch {
                    let else_reqs = self.analyze_expr(else_expr);
                    reqs.merge(&else_reqs);
                }
            }
            ExprKind::Block(block) => {
                let block_reqs = self.analyze_block(block);
                reqs.merge(&block_reqs);
            }
            ExprKind::Async(block) => {
                reqs.needs_runtime = true;
                let block_reqs = self.analyze_block(block);
                reqs.merge(&block_reqs);
            }
            ExprKind::Spawn { expr: inner, .. } => {
                reqs.needs_runtime = true;
                reqs.needs_os = true;
                let spawn_reqs = self.analyze_expr(inner);
                reqs.merge(&spawn_reqs);
            }
            _ => {}
        }

        reqs
    }

    /// Analyze requirements for an if condition
    fn analyze_if_condition(&mut self, condition: &verum_ast::expr::IfCondition) -> ItemRequirements {
        use verum_ast::expr::ConditionKind;
        let mut reqs = ItemRequirements::none();

        for cond in &condition.conditions {
            match cond {
                ConditionKind::Expr(expr) => {
                    let expr_reqs = self.analyze_expr(expr);
                    reqs.merge(&expr_reqs);
                }
                ConditionKind::Let { value, .. } => {
                    let value_reqs = self.analyze_expr(value);
                    reqs.merge(&value_reqs);
                }
            }
        }

        reqs
    }

    /// Analyze requirements for a block
    fn analyze_block(&mut self, block: &verum_ast::expr::Block) -> ItemRequirements {
        let mut reqs = ItemRequirements::none();

        for stmt in &block.stmts {
            if let verum_ast::stmt::StmtKind::Let { value, .. } = &stmt.kind {
                if let Some(init_expr) = value.as_ref() {
                    let init_reqs = self.analyze_expr(init_expr);
                    reqs.merge(&init_reqs);
                }
            }
        }

        if let Some(result_expr) = &block.expr {
            let result_reqs = self.analyze_expr(result_expr);
            reqs.merge(&result_reqs);
        }

        reqs
    }

    /// Analyze requirements for an item
    pub fn analyze_item(&mut self, item: &Item) -> ItemRequirements {
        let mut reqs = ItemRequirements::none();

        match &item.kind {
            ItemKind::Function(func) => {
                // Analyze return type
                if let Some(ret_ty) = &func.return_type {
                    let ret_reqs = self.analyze_type(ret_ty);
                    reqs.merge(&ret_reqs);
                }

                // Analyze parameters
                for param in &func.params {
                    if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &param.kind {
                        let param_reqs = self.analyze_type(ty);
                        reqs.merge(&param_reqs);
                    }
                }

                // Check if async
                if func.is_async {
                    reqs.needs_runtime = true;
                }

                // Analyze body
                if let Some(body) = &func.body {
                    let body_reqs = match body {
                        verum_ast::decl::FunctionBody::Block(block) => self.analyze_block(block),
                        verum_ast::decl::FunctionBody::Expr(expr) => self.analyze_expr(expr),
                    };
                    reqs.merge(&body_reqs);
                }
            }
            ItemKind::Type(type_decl) => {
                // Analyze the type body
                match &type_decl.body {
                    verum_ast::decl::TypeDeclBody::Alias(ty) => {
                        let ty_reqs = self.analyze_type(ty);
                        reqs.merge(&ty_reqs);
                    }
                    verum_ast::decl::TypeDeclBody::Newtype(ty) => {
                        let ty_reqs = self.analyze_type(ty);
                        reqs.merge(&ty_reqs);
                    }
                    verum_ast::decl::TypeDeclBody::Record(fields) => {
                        for field in fields {
                            let field_reqs = self.analyze_type(&field.ty);
                            reqs.merge(&field_reqs);
                        }
                    }
                    verum_ast::decl::TypeDeclBody::Tuple(types) => {
                        for ty in types {
                            let ty_reqs = self.analyze_type(ty);
                            reqs.merge(&ty_reqs);
                        }
                    }
                    verum_ast::decl::TypeDeclBody::Variant(variants) => {
                        for variant in variants {
                            if let Some(data) = variant.data.as_ref() {
                                match data {
                                    verum_ast::decl::VariantData::Tuple(types) => {
                                        for ty in types {
                                            let ty_reqs = self.analyze_type(ty);
                                            reqs.merge(&ty_reqs);
                                        }
                                    }
                                    verum_ast::decl::VariantData::Record(fields) => {
                                        for field in fields {
                                            let field_reqs = self.analyze_type(&field.ty);
                                            reqs.merge(&field_reqs);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Cache the result
        let name = self.get_item_name(item);
        self.requirements.insert(Text::from(name), reqs.clone());

        reqs
    }

    /// Validate item against target profile
    pub fn validate_item(
        &self,
        item_name: &str,
        reqs: &ItemRequirements,
        span: Span,
    ) -> Result<(), TargetError> {
        if self.profile.no_alloc && reqs.needs_alloc {
            return Err(TargetError::RequiresAllocator {
                item: Text::from(item_name),
                span,
                suggestion: suggest_no_alloc_alternative(item_name),
            });
        }

        if self.profile.no_std && reqs.needs_os {
            return Err(TargetError::RequiresOs {
                item: Text::from(item_name),
                span,
            });
        }

        if self.profile.embedded && reqs.needs_runtime {
            return Err(TargetError::RequiresRuntime {
                item: Text::from(item_name),
                span,
            });
        }

        if self.profile.cbgr_static_only && reqs.needs_cbgr_runtime {
            return Err(TargetError::CbgrTierViolation {
                item: Text::from(item_name),
                span,
                message: Text::from("Tier 0 references not allowed in static-only mode"),
            });
        }

        if self.profile.no_gpu && reqs.needs_gpu {
            return Err(TargetError::RequiresGpu {
                item: Text::from(item_name),
                span,
            });
        }

        Ok(())
    }

    /// Analyze and validate a module
    pub fn analyze_module(&mut self, module: &Module) -> List<TargetError> {
        let mut errors = List::new();

        for item in &module.items {
            let reqs = self.analyze_item(item);
            let name = self.get_item_name(item);

            if let Err(err) = self.validate_item(name, &reqs, item.span) {
                errors.push(err);
            }
        }

        errors
    }

    /// Extract name from an item
    fn get_item_name<'a>(&self, item: &'a Item) -> &'a str {
        match &item.kind {
            ItemKind::Function(f) => f.name.as_str(),
            ItemKind::Type(t) => t.name.as_str(),
            ItemKind::Protocol(p) => p.name.as_str(),
            ItemKind::Const(c) => c.name.as_str(),
            ItemKind::Static(s) => s.name.as_str(),
            ItemKind::Meta(m) => m.name.as_str(),
            _ => "anonymous",
        }
    }
}

// ============================================================================
// Suggestions
// ============================================================================

/// Suggest alternatives for no_alloc context
fn suggest_no_alloc_alternative(item_name: &str) -> Option<Text> {
    match item_name {
        "List" | "Vec" => Some(Text::from("Use Array<T, N> (fixed-size, stack-allocated)")),
        "Text" | "String" => Some(Text::from("Use &[Byte] or [Byte; N] for string data")),
        "Map" | "HashMap" => Some(Text::from("Use sorted Array for small collections")),
        "Set" | "HashSet" => Some(Text::from("Use sorted Array for small collections")),
        "Heap" | "Box" => Some(Text::from("Use stack allocation or static storage")),
        _ => None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_item_requirements_none() {
        let reqs = ItemRequirements::none();
        assert!(reqs.is_embedded_compatible());
        assert!(reqs.is_no_alloc_compatible());
        assert!(reqs.is_no_std_compatible());
    }

    #[test]
    fn test_item_requirements_alloc() {
        let reqs = ItemRequirements::alloc();
        assert!(!reqs.is_embedded_compatible());
        assert!(!reqs.is_no_alloc_compatible());
        assert!(reqs.is_no_std_compatible());
    }

    #[test]
    fn test_item_requirements_os() {
        let reqs = ItemRequirements::os();
        assert!(!reqs.is_embedded_compatible());
        assert!(!reqs.is_no_alloc_compatible());
        assert!(!reqs.is_no_std_compatible());
    }

    #[test]
    fn test_item_requirements_merge() {
        let mut reqs1 = ItemRequirements::none();
        let reqs2 = ItemRequirements::alloc();
        reqs1.merge(&reqs2);
        assert!(reqs1.needs_alloc);
        assert!(!reqs1.needs_os);
    }

    #[test]
    fn test_target_profile_default() {
        let profile = TargetProfile::default_profile();
        assert!(!profile.no_alloc);
        assert!(!profile.no_std);
        assert!(!profile.embedded);
    }

    #[test]
    fn test_target_profile_embedded() {
        let profile = TargetProfile::embedded();
        assert!(profile.no_alloc);
        assert!(profile.no_std);
        assert!(profile.embedded);
        assert!(profile.cbgr_static_only);
    }

    #[test]
    fn test_analyzer_builtin_requirements() {
        let analyzer = DependencyAnalyzer::new(TargetProfile::default_profile());

        // Core types should have no requirements
        let int_reqs = analyzer.get_builtin_requirements("Int").unwrap();
        assert!(int_reqs.is_embedded_compatible());

        // List should need allocation
        let list_reqs = analyzer.get_builtin_requirements("List").unwrap();
        assert!(list_reqs.needs_alloc);
        assert!(!list_reqs.is_no_alloc_compatible());

        // File should need OS
        let file_reqs = analyzer.get_builtin_requirements("File").unwrap();
        assert!(file_reqs.needs_os);
        assert!(!file_reqs.is_no_std_compatible());
    }

    #[test]
    fn test_validation_no_alloc() {
        let analyzer = DependencyAnalyzer::new(TargetProfile::no_alloc());
        let reqs = ItemRequirements::alloc();
        let span = Span::dummy();

        let result = analyzer.validate_item("MyType", &reqs, span);
        assert!(result.is_err());

        if let Err(TargetError::RequiresAllocator { item, .. }) = result {
            assert_eq!(item.as_str(), "MyType");
        }
    }

    #[test]
    fn test_validation_no_std() {
        let analyzer = DependencyAnalyzer::new(TargetProfile::no_std());
        let reqs = ItemRequirements::os();
        let span = Span::dummy();

        let result = analyzer.validate_item("MyType", &reqs, span);
        assert!(result.is_err());

        if let Err(TargetError::RequiresOs { item, .. }) = result {
            assert_eq!(item.as_str(), "MyType");
        }
    }

    #[test]
    fn test_suggestions() {
        assert!(suggest_no_alloc_alternative("List").is_some());
        assert!(suggest_no_alloc_alternative("Text").is_some());
        assert!(suggest_no_alloc_alternative("UnknownType").is_none());
    }
}
