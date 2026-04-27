#![allow(unexpected_cfgs)]
//! Abstract Syntax Tree (AST) for the Verum language.
//!
//! This crate provides complete AST node definitions for the Verum compiler,
//! including all expression types, statements, declarations, patterns, and
//! the critical refinement type system.
//!
//! # Overview
//!
//! The AST is organized into several modules:
//!
//! - [`span`]: Source location tracking for error reporting
//! - [`literal`]: Literal values (integers, floats, strings, etc.)
//! - [`ty`]: Type system including refinement types
//! - [`pattern`]: Pattern matching constructs
//! - [`expr`][]: Expressions (the core of the language)
//! - [`stmt`][]: Statements
//! - [`decl`]: Top-level declarations (functions, types, protocols, etc.)
//! - [`visitor`]: AST traversal using the visitor pattern
//!
//! # Key Features
//!
//! ## Refinement Types
//!
//! Verum's unique value proposition is its refinement type system. The AST
//! fully supports refinement predicates:
//!
//! ```verum
//! type Positive is Int{> 0}
//! type Email is Text{is_email(it)}
//! type SortedList<T> is List<T>{is_sorted(it)}
//! ```
//!
//! These are represented in the AST as [`TypeKind::Refined`] nodes.
//!
//! ## Stream Comprehensions
//!
//! First-class support for lazy stream processing:
//!
//! ```verum
//! stream [x * 2 for x in source if x > 0]
//! ```
//!
//! Represented as [`ExprKind::StreamComprehension`].
//!
//! ## Three-Tier Reference Model
//!
//! Support for Verum's three-tier reference system:
//!
//! - Safe references: `&T`, `&mut T` ([`TypeKind::Reference`])
//! - Checked references: `&checked T` (runtime bounds checking)
//! - Unsafe references: `&unsafe T` (no bounds checking)
//!
//! # Example Usage
//!
//! ```rust
//! use verum_ast::*;
//! use verum_ast::span::{Span, FileId};
//! use verum_ast::expr::{Expr, ExprKind, BinOp};
//! use verum_ast::literal::Literal;
//! use verum_common::Heap;
//!
//! // Create a simple binary expression: 1 + 2
//! let span = Span::new(0, 5, FileId::new(0));
//! let left = Heap::new(Expr::literal(Literal::int(1, span)));
//! let right = Heap::new(Expr::literal(Literal::int(2, span)));
//! let expr = Expr::new(
//!     ExprKind::Binary {
//!         op: BinOp::Add,
//!         left,
//!         right,
//!     },
//!     span,
//! );
//! ```
//!
//! # Design Principles
//!
//! 1. **Explicit over implicit**: All information is represented explicitly
//! 2. **Memory efficient**: Use `SmallVec` for common cases
//! 3. **Serializable**: All nodes derive `Serialize` and `Deserialize`
//! 4. **Spanned**: All nodes track their source location
//! 5. **Refinements first-class**: Refinement predicates are not bolted on

#![deny(missing_debug_implementations)]
#![deny(rust_2018_idioms)]
#![allow(dead_code)]
// Suppress informational clippy lints
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
// `from_str` methods intentionally return Maybe<Self> not Result, so they don't implement FromStr
#![allow(clippy::should_implement_trait)]
// Builder pattern: new() returns a builder, not Self
#![allow(clippy::new_ret_no_self)]
// `into_*` methods sometimes take &self for convenience
#![allow(clippy::wrong_self_convention)]

// Import v6.0-BALANCED semantic types from verum_common
use verum_common::List;

pub mod attr;
pub mod bitfield;
pub mod cfg;
pub mod context;
pub mod decl;
pub mod expr;
pub mod ffi;
pub mod literal;
pub mod meta_value;
pub mod pattern;
pub mod pretty;
pub mod span;
pub mod stmt;
pub mod ty;
pub mod visitor;

// Re-export smallvec for use in creating AST nodes
pub use smallvec;

// Re-export commonly used types for convenience
pub use attr::{
    // Attribute system: @-prefixed compile-time annotations for functions, types, fields, etc.
    // Attributes control optimization (@inline, @cold), serialization (@derive), validation,
    // layout (@repr, @align), concurrency (@lock_level), meta-system, and FFI behavior.
    ArgSpec,
    ArgType,
    // Core attributes
    Attribute,
    AttributeCategory,
    AttributeMetadata,
    AttributeTarget,
    // Bitfield attributes
    BitfieldAttr,
    BitOffsetAttr,
    BitsAttr,
    EndianAttr,
    FeatureAttr,
    FromAttribute,
    NamedArgSpec,
    Profile,
    ProfileAttr,
    Stability,
    StdAttr,
    TaggedLiteralAttr,
};
pub use cfg::{
    // Conditional compilation via @cfg(...) predicates. Supports target_os, target_arch,
    // target_family, feature flags, and boolean combinators (all/any/not). Only the C ABI
    // is stable for FFI; platform-specific code uses @cfg to select implementations.
    CfgEvaluator,
    CfgPredicate,
    CfgPredicateKind,
    HasAttributes,
    TargetConfig,
};
pub use decl::{
    AxiomDecl,
    CalcRelation,
    CalculationChain,
    CalculationStep,
    ContextDecl,
    ContextGroupDecl,
    // Context system types: capability-based dependency injection using `provide`/`using` keywords.
    // Functions declare required contexts with `using [Ctx1, Ctx2]` after the return type.
    // Contexts are provided lexically with `provide Context = expr`.
    ContextList,
    ContextRequirement,
    ContextTransform,
    FunctionBody,
    FunctionDecl,
    FunctionParam,
    FunctionParamKind,
    ImplDecl,
    MountDecl,
    MountTree,
    MountTreeKind,
    Item,
    ItemKind,
    ProofBody,
    ProofCase,
    ProofMethod,
    ProofStep,
    ProofStepKind,
    ProofStructure,
    ProtocolDecl,
    ResourceModifier,
    TacticBody,
    TacticDecl,
    TacticExpr,
    TacticParam,
    TacticParamKind,
    // Formal proofs system (v2.0+ extension): theorem/lemma/corollary declarations with
    // proof terms, induction, tactics, and SMT integration for machine-checkable proofs.
    TheoremDecl,
    TypeDecl,
    TypeDeclBody,
    Visibility,
};
pub use bitfield::{
    // Bitfield system for low-level bit manipulation
    BitLayout, BitSpec, BitWidth, ByteOrder, ResolvedBitField, ResolvedBitLayout,
};
pub use expr::{
    ArrayExpr, BinOp, Block, Capability, CapabilitySet, ClosureParam, ComprehensionClause,
    ComprehensionClauseKind, ConditionKind, Expr, ExprKind, FieldInit, IfCondition, MacroArgs,
    MacroArgsExt, MacroDelimiter, RecoverBody, RecoverClosureParam, TokenTree, TokenTreeKind,
    TokenTreeToken, TypeProperty, UnOp,
};
pub use ffi::{
    CallingConvention, ErrorProtocol, FFIBoundary, FFIFunction, FFISignature, MemoryEffects,
    Ownership,
};
pub use literal::{FloatLit, IntLit, Literal, LiteralKind};
pub use meta_value::{MetaValue, MetaValueConversionError};
pub use pattern::{FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData};
pub use pretty::{PrettyConfig, PrettyPrinter};
pub use span::{FileId, SourceFile, Span, Spanned};
pub use stmt::{Stmt, StmtKind};
pub use ty::{
    GenericParam, Ident, Path, PathSegment, RefinementPredicate, Type, TypeBinding, TypeKind,
    UniverseLevelExpr, WhereClause, WherePredicate, WherePredicateKind,
};
pub use visitor::{
    Visitor, walk_context, walk_context_group, walk_expr, walk_ffi_boundary, walk_item,
    walk_pattern, walk_stmt, walk_type,
};

/// What kind of compilation unit a `Module` represents. The vast majority of
/// modules are libraries; binaries are determined by the presence of a
/// `fn main()` and the project manifest. Scripts are a third category: a
/// single source file with a `#!` shebang, optional `// /// script`
/// frontmatter, and a top-level body that is not required to be wrapped in
/// `fn main()`. Scripts bypass `verum.toml` discovery and are executed via
/// `verum run path.vr` (or directly via the shebang chain).
///
/// The kind is encoded as a synthetic module attribute (`@![__verum_kind(...)]`)
/// to avoid breaking the `Module` struct's field layout — there are dozens of
/// struct-literal construction sites across the codebase. Use
/// [`CogKind::of`] / [`CogKind::set_on_module`] to inspect or set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum CogKind {
    /// Library cog. Default. No `fn main()` is required; one may exist for
    /// developer convenience but is not the entry point.
    #[default]
    Library,
    /// Binary cog. `fn main()` is required and is the entry point.
    Binary,
    /// Script cog. Single-file, shebang- or frontmatter-detected. Top-level
    /// statements are allowed and are wrapped in a synthesised
    /// `fn __verum_script_main()` by the compiler.
    Script,
}

impl CogKind {
    /// Synthetic module attribute name used to encode the cog kind.
    pub const ATTR_NAME: &'static str = "__verum_kind";

    /// Stable string tag for serialisation (attribute argument value).
    pub fn as_tag(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Binary => "binary",
            Self::Script => "script",
        }
    }

    /// Parse the tag back into a `CogKind`.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "library" => Some(Self::Library),
            "binary" => Some(Self::Binary),
            "script" => Some(Self::Script),
            _ => None,
        }
    }

    /// Resolve the kind of `module` by inspecting its synthetic
    /// `@![__verum_kind(...)]` attribute. Defaults to [`CogKind::Library`]
    /// when no such attribute is present.
    ///
    /// The attribute carries the tag as a string token. Attribute argument
    /// representation is introspected via `Debug` formatting rather than a
    /// typed walk because the attr-arg AST layout intentionally varies
    /// across attribute kinds; matching on the canonical tag strings keeps
    /// this resolver decoupled from those evolving shapes.
    pub fn of(module: &Module) -> Self {
        for attr in module.attributes.iter() {
            if attr.name.as_str() != Self::ATTR_NAME {
                continue;
            }
            let dbg = format!("{:?}", attr);
            if dbg.contains("\"script\"") {
                return Self::Script;
            }
            if dbg.contains("\"binary\"") {
                return Self::Binary;
            }
            if dbg.contains("\"library\"") {
                return Self::Library;
            }
        }
        Self::Library
    }
}

#[cfg(test)]
mod cog_kind_tests {
    use super::*;
    use crate::span::FileId;

    #[test]
    fn default_module_is_library() {
        let m = Module::empty(FileId::new(0));
        assert_eq!(CogKind::of(&m), CogKind::Library);
        assert!(!m.is_script());
    }

    #[test]
    fn tag_round_trip() {
        for k in [CogKind::Library, CogKind::Binary, CogKind::Script] {
            assert_eq!(CogKind::from_tag(k.as_tag()), Some(k));
        }
        assert_eq!(CogKind::from_tag("nonsense"), None);
    }
}

/// The root of an AST - a complete module or file.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Module {
    /// The items in this module
    pub items: List<Item>,
    /// Module-level attributes (e.g., @![no_implicit_prelude])
    /// Controls module-wide behavior such as disabling the implicit prelude import,
    /// setting module-wide context requirements via @using, or setting language profiles.
    pub attributes: List<Attribute>,
    /// The source file this module came from
    pub file_id: FileId,
    /// The span covering the entire module
    pub span: Span,
}

impl Module {
    pub fn new(items: List<Item>, file_id: FileId, span: Span) -> Self {
        Self {
            items,
            attributes: List::new(),
            file_id,
            span,
        }
    }

    pub fn new_with_attrs(
        items: List<Item>,
        attributes: List<Attribute>,
        file_id: FileId,
        span: Span,
    ) -> Self {
        Self {
            items,
            attributes,
            file_id,
            span,
        }
    }

    pub fn empty(file_id: FileId) -> Self {
        Self {
            items: List::new(),
            attributes: List::new(),
            file_id,
            span: Span::new(0, 0, file_id),
        }
    }

    /// True iff this module was parsed in script mode. Scripts allow
    /// top-level statements without an enclosing `fn main()`. Encoded as a
    /// synthetic `@![__verum_kind("script")]` attribute (see [`CogKind`]).
    pub fn is_script(&self) -> bool {
        matches!(CogKind::of(self), CogKind::Script)
    }

    /// Check if this module has the @![no_implicit_prelude] attribute.
    ///
    /// When set, the module does not automatically import the standard prelude
    /// (core types like List, Text, Map, Maybe, etc.). All types must be
    /// explicitly imported via `mount` statements.
    pub fn has_no_implicit_prelude(&self) -> bool {
        self.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "no_implicit_prelude")
    }

    /// Check if this module has a specific attribute.
    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes
            .iter()
            .any(|attr| attr.name.as_str() == name)
    }
}

impl Spanned for Module {
    fn span(&self) -> Span {
        self.span
    }
}

/// Self-referential `AsRef` so callers that take `M: AsRef<Module>`
/// generic bounds can pass either `Module` or `&Module` uniformly.
/// Required by `verum_compiler::stdlib_coercion_registry::scan_protocol_implementations`
/// and similar generic over-iter-of-modules entry points.
impl AsRef<Module> for Module {
    fn as_ref(&self) -> &Module {
        self
    }
}

/// A complete compilation unit (potentially multiple modules).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompilationUnit {
    /// All modules in this compilation unit
    pub modules: List<Module>,
}

impl CompilationUnit {
    pub fn new(modules: List<Module>) -> Self {
        Self { modules }
    }

    pub fn single(module: Module) -> Self {
        let modules = vec![module].into();
        Self { modules }
    }
}
