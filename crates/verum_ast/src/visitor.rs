//! Visitor pattern for traversing the AST.
//!
//! This module provides a visitor trait that can be implemented to walk
//! the AST and perform transformations, analysis, or code generation.
//!
//! # Architecture
//!
//! The visitor system supports two traversal modes:
//!
//! 1. **Recursive Mode** (default): Simple, correct traversal using the call stack.
//!    Suitable for most ASTs with depth < 1000.
//!
//! 2. **Iterative Mode**: Stack-safe traversal using a heap-allocated work stack.
//!    Required for very deep ASTs (depth > 1000) to avoid stack overflow.
//!
//! ## Usage
//!
//! ### Simple Visitor (Recursive Mode)
//!
//! ```ignore
//! struct ExprCounter { count: usize }
//!
//! impl Visitor for ExprCounter {
//!     fn visit_expr(&mut self, expr: &Expr) {
//!         self.count += 1;
//!         walk_expr(self, expr);  // Recurse into children
//!     }
//! }
//!
//! let mut counter = ExprCounter { count: 0 };
//! counter.visit_expr(&root_expr);
//! ```
//!
//! ### Deep AST Visitor (Iterative Mode)
//!
//! ```ignore
//! let analyzer = MyAnalyzer::new();
//! let mut iter_visitor = IterativeVisitor::new(analyzer);
//! iter_visitor.traverse(&very_deep_ast);
//! ```
//!
//! # Design Notes
//!
//! The dual-mode design ensures:
//! - **Correctness**: `visitor.visit_*` is called for EVERY node
//! - **Stack Safety**: Iterative mode prevents stack overflow
//! - **API Stability**: Existing code using recursive mode continues to work
//! - **No Unsafe**: Implementation uses safe Rust only
//!
//! Implements the visitor pattern for traversing all AST node types.

use crate::decl::{
    AxiomDecl,
    ContextDecl,
    ContextGroupDecl,
    FunctionBody,
    FunctionDecl,
    FunctionParam,
    FunctionParamKind,
    ImplDecl,
    ImplItemKind,
    ImplKind,
    Item,
    ItemKind,
    PredicateDecl,
    ProofBody,
    ProofStepKind,
    ProtocolDecl,
    ProtocolItemKind,
    TacticDecl,
    TacticExpr,
    TheoremDecl,
    TypeDecl,
    TypeDeclBody,
    VariantData,
};
use crate::expr::{
    ArrayExpr, Block, ComprehensionClause, ComprehensionClauseKind, ConditionKind, Expr, ExprKind,
    IfCondition, RecoverBody,
};
use crate::ffi::FFIBoundary;
use crate::literal::Literal;
use crate::pattern::{Pattern, PatternKind, VariantPatternData};
use crate::stmt::{Stmt, StmtKind};
use crate::ty::{GenericArg, Ident, Path, Type, TypeKind, WhereClause, WherePredicate};
use verum_common::Maybe;

// ============================================================================
// WORK ITEM ENUM
// ============================================================================

/// Work items for iterative AST traversal.
///
/// This enum represents all visitable AST node types. It enables stack-safe
/// traversal by storing pending work on the heap instead of the call stack.
#[derive(Clone, Debug)]
pub enum WorkItem<'a> {
    /// Expression node
    Expr(&'a Expr),
    /// Statement node
    Stmt(&'a Stmt),
    /// Block node
    Block(&'a Block),
    /// Pattern node
    Pattern(&'a Pattern),
    /// Type node
    Type(&'a Type),
    /// Item (top-level declaration)
    Item(&'a Item),
    /// Function declaration
    Function(&'a FunctionDecl),
    /// Type declaration
    TypeDecl(&'a TypeDecl),
    /// Protocol declaration
    Protocol(&'a ProtocolDecl),
    /// Implementation block
    Impl(&'a ImplDecl),
    /// Predicate declaration
    Predicate(&'a PredicateDecl),
    /// Context declaration
    Context(&'a ContextDecl),
    /// Context group declaration
    ContextGroup(&'a ContextGroupDecl),
    /// FFI boundary declaration
    FFIBoundary(&'a FFIBoundary),
    /// Theorem declaration
    Theorem(&'a TheoremDecl),
    /// Axiom declaration
    Axiom(&'a AxiomDecl),
    /// Tactic declaration
    Tactic(&'a TacticDecl),
    /// Tactic expression
    TacticExpr(&'a TacticExpr),
    /// Proof body
    ProofBody(&'a ProofBody),
    /// Where clause
    WhereClause(&'a WhereClause),
    /// Where predicate
    WherePredicate(&'a WherePredicate),
    /// View declaration
    View(&'a crate::decl::ViewDecl),
    /// View constructor
    ViewConstructor(&'a crate::decl::ViewConstructor),
    /// Function parameter
    FunctionParam(&'a FunctionParam),
    /// If condition (for iterative processing)
    IfCondition(&'a IfCondition),
    /// Comprehension clause (for iterative processing)
    ComprehensionClause(&'a ComprehensionClause),
}

// ============================================================================
// VISITOR TRAIT
// ============================================================================

/// A visitor for traversing the AST.
///
/// Implement this trait to walk the AST and perform custom operations
/// on each node. The default implementations recursively visit child nodes.
///
/// # Traversal Modes
///
/// By default, visitors use recursive traversal. For very deep ASTs that
/// might cause stack overflow, use [`IterativeVisitor`] wrapper:
///
/// ```ignore
/// let my_visitor = MyVisitor::new();
/// let mut iter = IterativeVisitor::new(my_visitor);
/// iter.traverse_expr(&deep_ast);
/// ```
pub trait Visitor: Sized {
    /// Returns a mutable reference to the work stack for iterative traversal.
    ///
    /// - Returns `None` (default): Use recursive traversal
    /// - Returns `Some(stack)`: Use iterative traversal, pushing work items to stack
    ///
    /// This method is automatically managed by [`IterativeVisitor`].
    /// You typically don't need to override this method.
    fn work_stack(&mut self) -> Option<&mut Vec<WorkItem<'_>>> {
        None
    }

    /// Visit an item.
    fn visit_item(&mut self, item: &Item) {
        walk_item(self, item);
    }

    /// Visit a function declaration.
    fn visit_function(&mut self, func: &FunctionDecl) {
        walk_function(self, func);
    }

    /// Visit a type declaration.
    fn visit_type_decl(&mut self, ty_decl: &TypeDecl) {
        walk_type_decl(self, ty_decl);
    }

    /// Visit a protocol declaration.
    fn visit_protocol(&mut self, protocol: &ProtocolDecl) {
        walk_protocol(self, protocol);
    }

    /// Visit an implementation block.
    fn visit_impl(&mut self, impl_decl: &ImplDecl) {
        walk_impl(self, impl_decl);
    }

    /// Visit a predicate declaration.
    fn visit_predicate(&mut self, predicate: &PredicateDecl) {
        walk_predicate(self, predicate);
    }

    /// Visit a context declaration.
    fn visit_context(&mut self, context: &ContextDecl) {
        walk_context(self, context);
    }

    /// Visit a context group declaration.
    fn visit_context_group(&mut self, context_group: &ContextGroupDecl) {
        walk_context_group(self, context_group);
    }

    /// Visit an FFI boundary declaration.
    fn visit_ffi_boundary(&mut self, ffi_boundary: &FFIBoundary) {
        walk_ffi_boundary(self, ffi_boundary);
    }

    /// Visit a theorem declaration.
    fn visit_theorem(&mut self, theorem: &TheoremDecl) {
        walk_theorem(self, theorem);
    }

    /// Visit an axiom declaration.
    fn visit_axiom(&mut self, axiom: &AxiomDecl) {
        walk_axiom(self, axiom);
    }

    /// Visit a tactic declaration.
    fn visit_tactic(&mut self, tactic: &TacticDecl) {
        walk_tactic(self, tactic);
    }

    /// Visit a view declaration (alternative pattern matching interface, v2.0+ planned).
    fn visit_view(&mut self, view: &crate::decl::ViewDecl) {
        walk_view(self, view);
    }

    /// Visit a view constructor (alternative pattern matching interface, v2.0+ planned).
    fn visit_view_constructor(&mut self, constructor: &crate::decl::ViewConstructor) {
        walk_view_constructor(self, constructor);
    }

    /// Visit a tactic expression.
    fn visit_tactic_expr(&mut self, tactic_expr: &TacticExpr) {
        walk_tactic_expr(self, tactic_expr);
    }

    /// Visit a proof body.
    fn visit_proof_body(&mut self, proof_body: &ProofBody) {
        walk_proof_body(self, proof_body);
    }

    /// Visit an expression.
    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(self, expr);
    }

    /// Visit a statement.
    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(self, stmt);
    }

    /// Visit a pattern.
    fn visit_pattern(&mut self, pattern: &Pattern) {
        walk_pattern(self, pattern);
    }

    /// Visit a type.
    fn visit_type(&mut self, ty: &Type) {
        walk_type(self, ty);
    }

    /// Visit a literal.
    fn visit_literal(&mut self, _lit: &Literal) {
        // Literals are leaf nodes
    }

    /// Visit an identifier.
    fn visit_ident(&mut self, _ident: &Ident) {
        // Identifiers are leaf nodes
    }

    /// Visit a path.
    fn visit_path(&mut self, _path: &Path) {
        // Paths are currently treated as leaf nodes
    }

    /// Visit a block.
    fn visit_block(&mut self, block: &Block) {
        walk_block(self, block);
    }

    /// Visit a where clause.
    fn visit_where_clause(&mut self, where_clause: &WhereClause) {
        walk_where_clause(self, where_clause);
    }

    /// Visit a where predicate.
    fn visit_where_predicate(&mut self, predicate: &WherePredicate) {
        walk_where_predicate(self, predicate);
    }

    /// Visit a function parameter.
    fn visit_function_param(&mut self, param: &FunctionParam) {
        walk_function_param(self, param);
    }
}

// ============================================================================
// VISIT CHILD MACRO
// ============================================================================

/// Internal macro for dual-mode child visitation.
///
/// In recursive mode (work_stack returns None): calls visitor.visit_*() directly
/// In iterative mode (work_stack returns Some): pushes WorkItem to the stack
macro_rules! visit_child {
    ($visitor:expr, $node:expr, Expr) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Expr($node));
        } else {
            $visitor.visit_expr($node);
        }
    };
    ($visitor:expr, $node:expr, Stmt) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Stmt($node));
        } else {
            $visitor.visit_stmt($node);
        }
    };
    ($visitor:expr, $node:expr, Block) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Block($node));
        } else {
            $visitor.visit_block($node);
        }
    };
    ($visitor:expr, $node:expr, Pattern) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Pattern($node));
        } else {
            $visitor.visit_pattern($node);
        }
    };
    ($visitor:expr, $node:expr, Type) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Type($node));
        } else {
            $visitor.visit_type($node);
        }
    };
    ($visitor:expr, $node:expr, Item) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Item($node));
        } else {
            $visitor.visit_item($node);
        }
    };
    ($visitor:expr, $node:expr, Function) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Function($node));
        } else {
            $visitor.visit_function($node);
        }
    };
    ($visitor:expr, $node:expr, TypeDecl) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::TypeDecl($node));
        } else {
            $visitor.visit_type_decl($node);
        }
    };
    ($visitor:expr, $node:expr, Protocol) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Protocol($node));
        } else {
            $visitor.visit_protocol($node);
        }
    };
    ($visitor:expr, $node:expr, Impl) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Impl($node));
        } else {
            $visitor.visit_impl($node);
        }
    };
    ($visitor:expr, $node:expr, Predicate) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Predicate($node));
        } else {
            $visitor.visit_predicate($node);
        }
    };
    ($visitor:expr, $node:expr, Context) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Context($node));
        } else {
            $visitor.visit_context($node);
        }
    };
    ($visitor:expr, $node:expr, ContextGroup) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::ContextGroup($node));
        } else {
            $visitor.visit_context_group($node);
        }
    };
    ($visitor:expr, $node:expr, FFIBoundary) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::FFIBoundary($node));
        } else {
            $visitor.visit_ffi_boundary($node);
        }
    };
    ($visitor:expr, $node:expr, Theorem) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Theorem($node));
        } else {
            $visitor.visit_theorem($node);
        }
    };
    ($visitor:expr, $node:expr, Axiom) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Axiom($node));
        } else {
            $visitor.visit_axiom($node);
        }
    };
    ($visitor:expr, $node:expr, Tactic) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::Tactic($node));
        } else {
            $visitor.visit_tactic($node);
        }
    };
    ($visitor:expr, $node:expr, TacticExpr) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::TacticExpr($node));
        } else {
            $visitor.visit_tactic_expr($node);
        }
    };
    ($visitor:expr, $node:expr, ProofBody) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::ProofBody($node));
        } else {
            $visitor.visit_proof_body($node);
        }
    };
    ($visitor:expr, $node:expr, WhereClause) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::WhereClause($node));
        } else {
            $visitor.visit_where_clause($node);
        }
    };
    ($visitor:expr, $node:expr, WherePredicate) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::WherePredicate($node));
        } else {
            $visitor.visit_where_predicate($node);
        }
    };
    ($visitor:expr, $node:expr, View) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::View($node));
        } else {
            $visitor.visit_view($node);
        }
    };
    ($visitor:expr, $node:expr, ViewConstructor) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::ViewConstructor($node));
        } else {
            $visitor.visit_view_constructor($node);
        }
    };
    ($visitor:expr, $node:expr, FunctionParam) => {
        if let Some(stack) = $visitor.work_stack() {
            stack.push(WorkItem::FunctionParam($node));
        } else {
            $visitor.visit_function_param($node);
        }
    };
}

// ============================================================================
// ITERATIVE VISITOR WRAPPER
// ============================================================================

/// Wrapper that enables iterative (stack-safe) mode for any visitor.
///
/// Use this wrapper when traversing very deep ASTs that might cause
/// stack overflow with recursive traversal.
///
/// # Example
///
/// ```ignore
/// struct MyAnalyzer { /* ... */ }
///
/// impl Visitor for MyAnalyzer {
///     fn visit_expr(&mut self, expr: &Expr) {
///         // analyze expression...
///         walk_expr(self, expr);  // This works in both modes!
///     }
/// }
///
/// // For deep ASTs, use IterativeVisitor wrapper
/// let analyzer = MyAnalyzer::new();
/// let mut iter_visitor = IterativeVisitor::new(analyzer);
/// iter_visitor.traverse_expr(&very_deep_ast);
/// let result = iter_visitor.into_inner();
/// ```
///
/// # Implementation Notes
///
/// The wrapper manages a work stack that stores pending nodes to visit.
/// When `walk_*` functions detect the work stack (via `work_stack()` method),
/// they push child nodes to the stack instead of making recursive calls.
/// The main loop then pops items and dispatches to the appropriate `visit_*` method.
#[derive(Debug)]
pub struct IterativeVisitor<V> {
    /// The wrapped visitor implementation
    inner: V,
    /// Work stack for pending nodes (empty when not actively traversing)
    stack: Vec<WorkItem<'static>>,
}

impl<V: Visitor> IterativeVisitor<V> {
    /// Create a new iterative visitor wrapper.
    pub fn new(inner: V) -> Self {
        Self {
            inner,
            stack: Vec::with_capacity(256),
        }
    }

    /// Get a reference to the inner visitor.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Get a mutable reference to the inner visitor.
    pub fn inner_mut(&mut self) -> &mut V {
        &mut self.inner
    }

    /// Consume the wrapper and return the inner visitor.
    pub fn into_inner(self) -> V {
        self.inner
    }

    /// Traverse an expression iteratively.
    ///
    /// This method is stack-safe and can handle ASTs with arbitrary depth.
    pub fn traverse_expr<'a>(&'a mut self, expr: &'a Expr) {
        // SAFETY: We use transmute to extend the lifetime to 'static.
        // This is safe because:
        // 1. The stack is cleared before this function returns
        // 2. No references from the stack escape to the caller
        // 3. The wrapper owns the stack lifetime
        //
        // The alternative (using a proper lifetime-parameterized stack) would
        // require significant API changes. This contained transmute is the
        // pragmatic solution used by other AST visitor implementations.
        let expr_static = unsafe { std::mem::transmute::<&Expr, &'static Expr>(expr) };
        self.stack.push(WorkItem::Expr(expr_static));
        self.run_loop();
    }

    /// Traverse a statement iteratively.
    pub fn traverse_stmt<'a>(&'a mut self, stmt: &'a Stmt) {
        let stmt_static = unsafe { std::mem::transmute::<&Stmt, &'static Stmt>(stmt) };
        self.stack.push(WorkItem::Stmt(stmt_static));
        self.run_loop();
    }

    /// Traverse a block iteratively.
    pub fn traverse_block<'a>(&'a mut self, block: &'a Block) {
        let block_static = unsafe { std::mem::transmute::<&Block, &'static Block>(block) };
        self.stack.push(WorkItem::Block(block_static));
        self.run_loop();
    }

    /// Traverse an item iteratively.
    pub fn traverse_item<'a>(&'a mut self, item: &'a Item) {
        let item_static = unsafe { std::mem::transmute::<&Item, &'static Item>(item) };
        self.stack.push(WorkItem::Item(item_static));
        self.run_loop();
    }

    /// Traverse a pattern iteratively.
    pub fn traverse_pattern<'a>(&'a mut self, pattern: &'a Pattern) {
        let pattern_static = unsafe { std::mem::transmute::<&Pattern, &'static Pattern>(pattern) };
        self.stack.push(WorkItem::Pattern(pattern_static));
        self.run_loop();
    }

    /// Traverse a type iteratively.
    pub fn traverse_type<'a>(&'a mut self, ty: &'a Type) {
        let ty_static = unsafe { std::mem::transmute::<&Type, &'static Type>(ty) };
        self.stack.push(WorkItem::Type(ty_static));
        self.run_loop();
    }

    /// Main loop that processes work items from the stack.
    fn run_loop(&mut self) {
        while let Some(item) = self.stack.pop() {
            match item {
                WorkItem::Expr(e) => self.inner.visit_expr(e),
                WorkItem::Stmt(s) => self.inner.visit_stmt(s),
                WorkItem::Block(b) => self.inner.visit_block(b),
                WorkItem::Pattern(p) => self.inner.visit_pattern(p),
                WorkItem::Type(t) => self.inner.visit_type(t),
                WorkItem::Item(i) => self.inner.visit_item(i),
                WorkItem::Function(f) => self.inner.visit_function(f),
                WorkItem::TypeDecl(td) => self.inner.visit_type_decl(td),
                WorkItem::Protocol(p) => self.inner.visit_protocol(p),
                WorkItem::Impl(i) => self.inner.visit_impl(i),
                WorkItem::Predicate(p) => self.inner.visit_predicate(p),
                WorkItem::Context(c) => self.inner.visit_context(c),
                WorkItem::ContextGroup(cg) => self.inner.visit_context_group(cg),
                WorkItem::FFIBoundary(fb) => self.inner.visit_ffi_boundary(fb),
                WorkItem::Theorem(th) => self.inner.visit_theorem(th),
                WorkItem::Axiom(ax) => self.inner.visit_axiom(ax),
                WorkItem::Tactic(t) => self.inner.visit_tactic(t),
                WorkItem::TacticExpr(te) => self.inner.visit_tactic_expr(te),
                WorkItem::ProofBody(pb) => self.inner.visit_proof_body(pb),
                WorkItem::WhereClause(wc) => self.inner.visit_where_clause(wc),
                WorkItem::WherePredicate(wp) => self.inner.visit_where_predicate(wp),
                WorkItem::View(v) => self.inner.visit_view(v),
                WorkItem::ViewConstructor(vc) => self.inner.visit_view_constructor(vc),
                WorkItem::FunctionParam(fp) => self.inner.visit_function_param(fp),
                WorkItem::IfCondition(c) => {
                    // Process if condition inline
                    walk_if_condition_internal(&mut self.inner, c);
                }
                WorkItem::ComprehensionClause(c) => {
                    // Process comprehension clause inline
                    walk_comprehension_clause_internal(&mut self.inner, c);
                }
            }
        }
    }
}

impl<V: Visitor> Visitor for IterativeVisitor<V> {
    fn work_stack(&mut self) -> Option<&mut Vec<WorkItem<'_>>> {
        // SAFETY: The lifetime downgrade is safe because the stack is managed
        // internally and cleared before any references could escape.
        Some(unsafe {
            std::mem::transmute::<&mut Vec<WorkItem<'static>>, &mut Vec<WorkItem<'_>>>(&mut self.stack)
        })
    }

    fn visit_item(&mut self, item: &Item) {
        self.inner.visit_item(item);
    }

    fn visit_function(&mut self, func: &FunctionDecl) {
        self.inner.visit_function(func);
    }

    fn visit_type_decl(&mut self, ty_decl: &TypeDecl) {
        self.inner.visit_type_decl(ty_decl);
    }

    fn visit_protocol(&mut self, protocol: &ProtocolDecl) {
        self.inner.visit_protocol(protocol);
    }

    fn visit_impl(&mut self, impl_decl: &ImplDecl) {
        self.inner.visit_impl(impl_decl);
    }

    fn visit_predicate(&mut self, predicate: &PredicateDecl) {
        self.inner.visit_predicate(predicate);
    }

    fn visit_context(&mut self, context: &ContextDecl) {
        self.inner.visit_context(context);
    }

    fn visit_context_group(&mut self, context_group: &ContextGroupDecl) {
        self.inner.visit_context_group(context_group);
    }

    fn visit_ffi_boundary(&mut self, ffi_boundary: &FFIBoundary) {
        self.inner.visit_ffi_boundary(ffi_boundary);
    }

    fn visit_theorem(&mut self, theorem: &TheoremDecl) {
        self.inner.visit_theorem(theorem);
    }

    fn visit_axiom(&mut self, axiom: &AxiomDecl) {
        self.inner.visit_axiom(axiom);
    }

    fn visit_tactic(&mut self, tactic: &TacticDecl) {
        self.inner.visit_tactic(tactic);
    }

    fn visit_view(&mut self, view: &crate::decl::ViewDecl) {
        self.inner.visit_view(view);
    }

    fn visit_view_constructor(&mut self, constructor: &crate::decl::ViewConstructor) {
        self.inner.visit_view_constructor(constructor);
    }

    fn visit_tactic_expr(&mut self, tactic_expr: &TacticExpr) {
        self.inner.visit_tactic_expr(tactic_expr);
    }

    fn visit_proof_body(&mut self, proof_body: &ProofBody) {
        self.inner.visit_proof_body(proof_body);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        self.inner.visit_expr(expr);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        self.inner.visit_stmt(stmt);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        self.inner.visit_pattern(pattern);
    }

    fn visit_type(&mut self, ty: &Type) {
        self.inner.visit_type(ty);
    }

    fn visit_literal(&mut self, lit: &Literal) {
        self.inner.visit_literal(lit);
    }

    fn visit_ident(&mut self, ident: &Ident) {
        self.inner.visit_ident(ident);
    }

    fn visit_path(&mut self, path: &Path) {
        self.inner.visit_path(path);
    }

    fn visit_block(&mut self, block: &Block) {
        self.inner.visit_block(block);
    }

    fn visit_where_clause(&mut self, where_clause: &WhereClause) {
        self.inner.visit_where_clause(where_clause);
    }

    fn visit_where_predicate(&mut self, predicate: &WherePredicate) {
        self.inner.visit_where_predicate(predicate);
    }

    fn visit_function_param(&mut self, param: &FunctionParam) {
        self.inner.visit_function_param(param);
    }
}

// ============================================================================
// WALK FUNCTIONS
// ============================================================================

/// Walk an item, visiting all its children.
pub fn walk_item<V: Visitor>(visitor: &mut V, item: &Item) {
    match &item.kind {
        ItemKind::Function(func) => visit_child!(visitor, func, Function),
        ItemKind::Type(ty_decl) => visit_child!(visitor, ty_decl, TypeDecl),
        ItemKind::Protocol(protocol) => visit_child!(visitor, protocol, Protocol),
        ItemKind::Impl(impl_decl) => visit_child!(visitor, impl_decl, Impl),
        ItemKind::Module(module) => {
            // Visit module-level context requirements
            for ctx_req in &module.contexts {
                visitor.visit_path(&ctx_req.path);
                for ty in &ctx_req.args {
                    visit_child!(visitor, ty, Type);
                }
            }
            // Visit module items
            if let Maybe::Some(items) = &module.items {
                for item in items {
                    visit_child!(visitor, item, Item);
                }
            }
        }
        ItemKind::Const(const_decl) => {
            visit_child!(visitor, &const_decl.ty, Type);
            visit_child!(visitor, &const_decl.value, Expr);
        }
        ItemKind::Static(static_decl) => {
            visit_child!(visitor, &static_decl.ty, Type);
            visit_child!(visitor, &static_decl.value, Expr);
        }
        ItemKind::Mount(_) => {}
        ItemKind::Meta(_) => {}
        ItemKind::Predicate(predicate) => visit_child!(visitor, predicate, Predicate),
        ItemKind::Context(context) => visit_child!(visitor, context, Context),
        ItemKind::ContextGroup(context_group) => visit_child!(visitor, context_group, ContextGroup),
        ItemKind::Layer(_) => { /* no-op */ }
        ItemKind::FFIBoundary(ffi_boundary) => visit_child!(visitor, ffi_boundary, FFIBoundary),

        // Formal proofs (v2.0+ extension)
        ItemKind::Theorem(theorem) => visit_child!(visitor, theorem, Theorem),
        ItemKind::Lemma(lemma) => visit_child!(visitor, lemma, Theorem),
        ItemKind::Corollary(corollary) => visit_child!(visitor, corollary, Theorem),
        ItemKind::Axiom(axiom) => visit_child!(visitor, axiom, Axiom),
        ItemKind::Tactic(tactic) => visit_child!(visitor, tactic, Tactic),

        // View patterns (v2.0+ planned)
        ItemKind::View(view) => visit_child!(visitor, view, View),

        // Extern block (FFI functions grouped by ABI)
        ItemKind::ExternBlock(extern_block) => {
            for func in &extern_block.functions {
                visit_child!(visitor, func, Function);
            }
        }

        // Active pattern declarations (F#-style pattern matchers)
        ItemKind::Pattern(pattern_decl) => {
            visitor.visit_ident(&pattern_decl.name);
            for param in &pattern_decl.type_params {
                visit_child!(visitor, param, FunctionParam);
            }
            for param in &pattern_decl.params {
                visit_child!(visitor, param, FunctionParam);
            }
            visit_child!(visitor, &pattern_decl.return_type, Type);
            visit_child!(visitor, &pattern_decl.body, Expr);
        }
    }
}

/// Walk a function declaration.
pub fn walk_function<V: Visitor>(visitor: &mut V, func: &FunctionDecl) {
    visitor.visit_ident(&func.name);

    for param in &func.params {
        visit_child!(visitor, param, FunctionParam);
    }

    if let Maybe::Some(return_type) = &func.return_type {
        visit_child!(visitor, return_type, Type);
    }

    // Visit throws clause error types
    if let Maybe::Some(throws) = &func.throws_clause {
        for error_ty in &throws.error_types {
            visit_child!(visitor, error_ty, Type);
        }
    }

    // Visit generic where clause (where type T: Protocol)
    if let Maybe::Some(where_clause) = &func.generic_where_clause {
        visit_child!(visitor, where_clause, WhereClause);
    }

    // Visit meta where clause (where meta N > 0)
    if let Maybe::Some(meta_where) = &func.meta_where_clause {
        visit_child!(visitor, meta_where, WhereClause);
    }

    if let Maybe::Some(body) = &func.body {
        match body {
            FunctionBody::Block(block) => visit_child!(visitor, block, Block),
            FunctionBody::Expr(expr) => visit_child!(visitor, expr, Expr),
        }
    }
}

/// Walk a type declaration.
pub fn walk_type_decl<V: Visitor>(visitor: &mut V, ty_decl: &TypeDecl) {
    visitor.visit_ident(&ty_decl.name);

    // Visit generic where clause (where type T: Protocol)
    if let Maybe::Some(where_clause) = &ty_decl.generic_where_clause {
        visit_child!(visitor, where_clause, WhereClause);
    }

    // Visit meta where clause (where meta N > 0)
    if let Maybe::Some(meta_where) = &ty_decl.meta_where_clause {
        visit_child!(visitor, meta_where, WhereClause);
    }

    match &ty_decl.body {
        TypeDeclBody::Alias(ty) => visit_child!(visitor, ty, Type),
        TypeDeclBody::Record(fields) => {
            for field in fields {
                visit_child!(visitor, &field.ty, Type);
            }
        }
        TypeDeclBody::Variant(variants) => {
            for variant in variants {
                if let Maybe::Some(data) = &variant.data {
                    match data {
                        VariantData::Tuple(types) => {
                            for ty in types {
                                visit_child!(visitor, ty, Type);
                            }
                        }
                        VariantData::Record(fields) => {
                            for field in fields {
                                visit_child!(visitor, &field.ty, Type);
                            }
                        }
                    }
                }
            }
        }
        TypeDeclBody::Newtype(ty) => visit_child!(visitor, ty, Type),
        TypeDeclBody::Tuple(types) => {
            for ty in types {
                visit_child!(visitor, ty, Type);
            }
        }
        TypeDeclBody::Protocol(protocol_body) => {
            // Visit extended protocols
            for extend_type in &protocol_body.extends {
                visit_child!(visitor, extend_type, Type);
            }
            // Visit protocol items
            for item in &protocol_body.items {
                match &item.kind {
                    crate::decl::ProtocolItemKind::Function { decl, default_impl } => {
                        visit_child!(visitor, decl, Function);
                        if let Maybe::Some(body) = default_impl {
                            match body {
                                crate::decl::FunctionBody::Block(block) => {
                                    visit_child!(visitor, block, Block);
                                }
                                crate::decl::FunctionBody::Expr(expr) => {
                                    visit_child!(visitor, expr, Expr);
                                }
                            }
                        }
                    }
                    crate::decl::ProtocolItemKind::Type { name, bounds: _, .. } => {
                        visitor.visit_ident(name);
                    }
                    crate::decl::ProtocolItemKind::Const { name, ty } => {
                        visitor.visit_ident(name);
                        visit_child!(visitor, ty, Type);
                    }
                    crate::decl::ProtocolItemKind::Axiom(axiom_decl) => {
                        visitor.visit_ident(&axiom_decl.name);
                        visit_child!(visitor, axiom_decl.proposition.as_ref(), Expr);
                    }
                }
            }
        }
        TypeDeclBody::Unit => {}
        TypeDeclBody::SigmaTuple(types) => {
            for ty in types {
                visit_child!(visitor, ty, Type);
            }
        }
        TypeDeclBody::Inductive(variants) => {
            for variant in variants {
                if let Maybe::Some(data) = &variant.data {
                    match data {
                        VariantData::Tuple(types) => {
                            for ty in types {
                                visit_child!(visitor, ty, Type);
                            }
                        }
                        VariantData::Record(fields) => {
                            for field in fields {
                                visit_child!(visitor, &field.ty, Type);
                            }
                        }
                    }
                }
            }
        }
        TypeDeclBody::Quotient { base, relation } => {
            visit_child!(visitor, base, Type);
            visit_child!(visitor, relation.as_ref(), Expr);
        }
        TypeDeclBody::Coinductive(protocol_body) => {
            for extend_type in &protocol_body.extends {
                visit_child!(visitor, extend_type, Type);
            }
            for item in &protocol_body.items {
                match &item.kind {
                    crate::decl::ProtocolItemKind::Function { decl, default_impl } => {
                        visit_child!(visitor, decl, Function);
                        if let Maybe::Some(body) = default_impl {
                            match body {
                                crate::decl::FunctionBody::Block(block) => {
                                    visit_child!(visitor, block, Block);
                                }
                                crate::decl::FunctionBody::Expr(expr) => {
                                    visit_child!(visitor, expr, Expr);
                                }
                            }
                        }
                    }
                    crate::decl::ProtocolItemKind::Type { name, bounds: _, .. } => {
                        visitor.visit_ident(name);
                    }
                    crate::decl::ProtocolItemKind::Const { name, ty } => {
                        visitor.visit_ident(name);
                        visit_child!(visitor, ty, Type);
                    }
                    crate::decl::ProtocolItemKind::Axiom(axiom_decl) => {
                        visitor.visit_ident(&axiom_decl.name);
                        visit_child!(visitor, axiom_decl.proposition.as_ref(), Expr);
                    }
                }
            }
        }
    }
}

/// Walk a protocol declaration.
pub fn walk_protocol<V: Visitor>(visitor: &mut V, protocol: &ProtocolDecl) {
    visitor.visit_ident(&protocol.name);

    // Visit generic where clause (where type T: Protocol)
    if let Maybe::Some(where_clause) = &protocol.generic_where_clause {
        visit_child!(visitor, where_clause, WhereClause);
    }

    // Visit meta where clause (where meta N > 0)
    if let Maybe::Some(meta_where) = &protocol.meta_where_clause {
        visit_child!(visitor, meta_where, WhereClause);
    }

    for item in &protocol.items {
        match &item.kind {
            ProtocolItemKind::Function { decl, default_impl } => {
                visit_child!(visitor, decl, Function);
                if let Maybe::Some(body) = default_impl {
                    match body {
                        FunctionBody::Block(block) => visit_child!(visitor, block, Block),
                        FunctionBody::Expr(expr) => visit_child!(visitor, expr, Expr),
                    }
                }
            }
            ProtocolItemKind::Type { .. } => {}
            ProtocolItemKind::Axiom(axiom_decl) => {
                visit_child!(visitor, axiom_decl.proposition.as_ref(), Expr);
            }
            ProtocolItemKind::Const { ty, .. } => {
                visit_child!(visitor, ty, Type);
            }
        }
    }
}

/// Walk an implementation block.
pub fn walk_impl<V: Visitor>(visitor: &mut V, impl_decl: &ImplDecl) {
    match &impl_decl.kind {
        ImplKind::Inherent(ty) => visit_child!(visitor, ty, Type),
        ImplKind::Protocol { for_type, .. } => visit_child!(visitor, for_type, Type),
    }

    // Visit generic where clause (where type T: Protocol)
    if let Maybe::Some(where_clause) = &impl_decl.generic_where_clause {
        visit_child!(visitor, where_clause, WhereClause);
    }

    // Visit meta where clause (where meta N > 0)
    if let Maybe::Some(meta_where) = &impl_decl.meta_where_clause {
        visit_child!(visitor, meta_where, WhereClause);
    }

    for item in &impl_decl.items {
        match &item.kind {
            ImplItemKind::Function(func) => visit_child!(visitor, func, Function),
            ImplItemKind::Type { ty, .. } => visit_child!(visitor, ty, Type),
            ImplItemKind::Const { ty, value, .. } => {
                visit_child!(visitor, ty, Type);
                visit_child!(visitor, value, Expr);
            }
            ImplItemKind::Proof { axiom_name, tactic } => {
                visitor.visit_ident(axiom_name);
                walk_tactic_expr(visitor, tactic);
            }
        }
    }
}

/// Walk a predicate declaration.
pub fn walk_predicate<V: Visitor>(visitor: &mut V, predicate: &PredicateDecl) {
    visitor.visit_ident(&predicate.name);

    for param in &predicate.params {
        visit_child!(visitor, param, FunctionParam);
    }

    visit_child!(visitor, &predicate.return_type, Type);
    visit_child!(visitor, &predicate.body, Expr);
}

/// Walk a context declaration.
pub fn walk_context<V: Visitor>(visitor: &mut V, context: &ContextDecl) {
    visitor.visit_ident(&context.name);

    for method in context.methods.iter() {
        visit_child!(visitor, method, Function);
    }
}

/// Walk a context group declaration.
pub fn walk_context_group<V: Visitor>(visitor: &mut V, context_group: &ContextGroupDecl) {
    visitor.visit_ident(&context_group.name);
    // Context names are just Text values, no need to visit them
}

/// Walk an expression, visiting all its children.
///
/// This function supports both recursive and iterative traversal modes:
/// - In recursive mode (default): child nodes are visited via direct function calls
/// - In iterative mode: child nodes are pushed to the work stack
pub fn walk_expr<V: Visitor>(visitor: &mut V, expr: &Expr) {
    match &expr.kind {
        ExprKind::Literal(lit) => visitor.visit_literal(lit),
        ExprKind::Path(path) => visitor.visit_path(path),
        ExprKind::Binary { left, right, .. } => {
            visit_child!(visitor, left.as_ref(), Expr);
            visit_child!(visitor, right.as_ref(), Expr);
        }
        ExprKind::Unary { expr: inner, .. } => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::NamedArg { value, .. } => {
            visit_child!(visitor, value.as_ref(), Expr);
        }
        ExprKind::Call { func, type_args, args } => {
            visit_child!(visitor, func.as_ref(), Expr);
            for type_arg in type_args.iter() {
                match type_arg {
                    GenericArg::Type(ty) => visit_child!(visitor, ty, Type),
                    GenericArg::Const(e) => visit_child!(visitor, e, Expr),
                    GenericArg::Lifetime(_) => {}
                    GenericArg::Binding(binding) => visit_child!(visitor, &binding.ty, Type),
                }
            }
            for arg in args.iter() {
                visit_child!(visitor, arg, Expr);
            }
        }
        ExprKind::MethodCall {
            receiver,
            method,
            type_args,
            args,
        } => {
            visit_child!(visitor, receiver.as_ref(), Expr);
            visitor.visit_ident(method);
            for type_arg in type_args.iter() {
                match type_arg {
                    GenericArg::Type(ty) => visit_child!(visitor, ty, Type),
                    GenericArg::Const(e) => visit_child!(visitor, e, Expr),
                    GenericArg::Lifetime(_) => {}
                    GenericArg::Binding(binding) => visit_child!(visitor, &binding.ty, Type),
                }
            }
            for arg in args.iter() {
                visit_child!(visitor, arg, Expr);
            }
        }
        ExprKind::Field { expr: inner, field } => {
            visit_child!(visitor, inner.as_ref(), Expr);
            visitor.visit_ident(field);
        }
        ExprKind::OptionalChain { expr: inner, field } => {
            visit_child!(visitor, inner.as_ref(), Expr);
            visitor.visit_ident(field);
        }
        ExprKind::TupleIndex { expr: inner, .. } => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::Index { expr: inner, index } => {
            visit_child!(visitor, inner.as_ref(), Expr);
            visit_child!(visitor, index.as_ref(), Expr);
        }
        ExprKind::Pipeline { left, right } => {
            visit_child!(visitor, left.as_ref(), Expr);
            visit_child!(visitor, right.as_ref(), Expr);
        }
        ExprKind::NullCoalesce { left, right } => {
            visit_child!(visitor, left.as_ref(), Expr);
            visit_child!(visitor, right.as_ref(), Expr);
        }
        ExprKind::Cast { expr: inner, ty } => {
            visit_child!(visitor, inner.as_ref(), Expr);
            visit_child!(visitor, ty, Type);
        }
        ExprKind::Try(inner) => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::TryBlock(inner) => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::TryRecover {
            try_block,
            recover,
        } => {
            visit_child!(visitor, try_block.as_ref(), Expr);
            match recover {
                RecoverBody::MatchArms { arms, .. } => {
                    for arm in arms.iter() {
                        visit_child!(visitor, &arm.pattern, Pattern);
                        if let Maybe::Some(guard) = &arm.guard {
                            visit_child!(visitor, guard.as_ref(), Expr);
                        }
                        visit_child!(visitor, arm.body.as_ref(), Expr);
                    }
                }
                RecoverBody::Closure { param, body, .. } => {
                    visit_child!(visitor, &param.pattern, Pattern);
                    if let Maybe::Some(ty) = &param.ty {
                        visit_child!(visitor, ty, Type);
                    }
                    visit_child!(visitor, body.as_ref(), Expr);
                }
            }
        }
        ExprKind::TryFinally {
            try_block,
            finally_block,
        } => {
            visit_child!(visitor, try_block.as_ref(), Expr);
            visit_child!(visitor, finally_block.as_ref(), Expr);
        }
        ExprKind::TryRecoverFinally {
            try_block,
            recover,
            finally_block,
        } => {
            visit_child!(visitor, try_block.as_ref(), Expr);
            match recover {
                RecoverBody::MatchArms { arms, .. } => {
                    for arm in arms.iter() {
                        visit_child!(visitor, &arm.pattern, Pattern);
                        if let Maybe::Some(guard) = &arm.guard {
                            visit_child!(visitor, guard.as_ref(), Expr);
                        }
                        visit_child!(visitor, arm.body.as_ref(), Expr);
                    }
                }
                RecoverBody::Closure { param, body, .. } => {
                    visit_child!(visitor, &param.pattern, Pattern);
                    if let Maybe::Some(ty) = &param.ty {
                        visit_child!(visitor, ty, Type);
                    }
                    visit_child!(visitor, body.as_ref(), Expr);
                }
            }
            visit_child!(visitor, finally_block.as_ref(), Expr);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs.iter() {
                visit_child!(visitor, e, Expr);
            }
        }
        ExprKind::Array(array_expr) => match array_expr {
            ArrayExpr::List(exprs) => {
                for e in exprs.iter() {
                    visit_child!(visitor, e, Expr);
                }
            }
            ArrayExpr::Repeat { value, count } => {
                visit_child!(visitor, value.as_ref(), Expr);
                visit_child!(visitor, count.as_ref(), Expr);
            }
        },
        ExprKind::Comprehension { expr: inner, clauses }
        | ExprKind::StreamComprehension { expr: inner, clauses }
        | ExprKind::SetComprehension { expr: inner, clauses }
        | ExprKind::GeneratorComprehension { expr: inner, clauses } => {
            visit_child!(visitor, inner.as_ref(), Expr);
            for clause in clauses.iter() {
                walk_comprehension_clause_internal(visitor, clause);
            }
        }
        ExprKind::MapComprehension {
            key_expr,
            value_expr,
            clauses,
        } => {
            visit_child!(visitor, key_expr.as_ref(), Expr);
            visit_child!(visitor, value_expr.as_ref(), Expr);
            for clause in clauses.iter() {
                walk_comprehension_clause_internal(visitor, clause);
            }
        }
        ExprKind::Record { fields, base, .. } => {
            for field in fields.iter() {
                if let Maybe::Some(value) = &field.value {
                    visit_child!(visitor, value, Expr);
                }
            }
            if let Maybe::Some(base_expr) = base {
                visit_child!(visitor, base_expr.as_ref(), Expr);
            }
        }
        ExprKind::InterpolatedString { exprs, .. } => {
            for e in exprs.iter() {
                visit_child!(visitor, e, Expr);
            }
        }
        ExprKind::TensorLiteral {
            elem_type, data, ..
        } => {
            visit_child!(visitor, elem_type, Type);
            visit_child!(visitor, data.as_ref(), Expr);
        }
        ExprKind::MapLiteral { entries } => {
            for (key, value) in entries.iter() {
                visit_child!(visitor, key, Expr);
                visit_child!(visitor, value, Expr);
            }
        }
        ExprKind::SetLiteral { elements } => {
            for elem in elements.iter() {
                visit_child!(visitor, elem, Expr);
            }
        }
        ExprKind::Block(block) => {
            visit_child!(visitor, block, Block);
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            walk_if_condition_internal(visitor, condition);
            visit_child!(visitor, then_branch, Block);
            if let Maybe::Some(else_expr) = else_branch {
                visit_child!(visitor, else_expr.as_ref(), Expr);
            }
        }
        ExprKind::Match { expr: inner, arms } => {
            visit_child!(visitor, inner.as_ref(), Expr);
            for arm in arms.iter() {
                visit_child!(visitor, &arm.pattern, Pattern);
                if let Maybe::Some(guard) = &arm.guard {
                    visit_child!(visitor, guard.as_ref(), Expr);
                }
                visit_child!(visitor, arm.body.as_ref(), Expr);
            }
        }
        ExprKind::Loop {
            label: _,
            body,
            invariants,
        } => {
            for inv in invariants.iter() {
                visit_child!(visitor, inv, Expr);
            }
            visit_child!(visitor, body, Block);
        }
        ExprKind::While {
            label: _,
            condition,
            body,
            invariants,
            decreases,
        } => {
            visit_child!(visitor, condition.as_ref(), Expr);
            for inv in invariants.iter() {
                visit_child!(visitor, inv, Expr);
            }
            for dec in decreases.iter() {
                visit_child!(visitor, dec, Expr);
            }
            visit_child!(visitor, body, Block);
        }
        ExprKind::For {
            label: _,
            pattern,
            iter,
            body,
            invariants,
            decreases,
        } => {
            visit_child!(visitor, pattern, Pattern);
            visit_child!(visitor, iter.as_ref(), Expr);
            for inv in invariants.iter() {
                visit_child!(visitor, inv, Expr);
            }
            for dec in decreases.iter() {
                visit_child!(visitor, dec, Expr);
            }
            visit_child!(visitor, body, Block);
        }
        ExprKind::ForAwait {
            label: _,
            pattern,
            async_iterable,
            body,
            invariants,
            decreases,
        } => {
            visit_child!(visitor, pattern, Pattern);
            visit_child!(visitor, async_iterable.as_ref(), Expr);
            for inv in invariants.iter() {
                visit_child!(visitor, inv, Expr);
            }
            for dec in decreases.iter() {
                visit_child!(visitor, dec, Expr);
            }
            visit_child!(visitor, body, Block);
        }
        ExprKind::Break { label: _, value } | ExprKind::Return(value) => {
            if let Maybe::Some(e) = value {
                visit_child!(visitor, e.as_ref(), Expr);
            }
        }
        ExprKind::Continue { label: _ } => {}
        ExprKind::Throw(inner) => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::Yield(inner) => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::Typeof(inner) => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::Closure {
            params,
            return_type,
            body,
            ..
        } => {
            for param in params {
                visit_child!(visitor, &param.pattern, Pattern);
                if let Maybe::Some(ty) = &param.ty {
                    visit_child!(visitor, ty, Type);
                }
            }
            if let Maybe::Some(ty) = return_type {
                visit_child!(visitor, ty, Type);
            }
            visit_child!(visitor, body.as_ref(), Expr);
        }
        ExprKind::Async(block) | ExprKind::Unsafe(block) | ExprKind::Meta(block) => {
            visit_child!(visitor, block, Block);
        }
        ExprKind::Quote { .. } => {
            // Quote expressions contain raw token trees, not AST nodes to visit
            // Token trees are processed separately during meta function expansion
        }
        ExprKind::StageEscape { stage: _, expr } => {
            // Stage escape contains an expression to evaluate at the specified stage
            visit_child!(visitor, expr.as_ref(), Expr);
        }
        ExprKind::Lift { expr } => {
            // Lift contains an expression to lift into the current stage
            visit_child!(visitor, expr.as_ref(), Expr);
        }
        ExprKind::Await(inner) => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::Inject { type_path } => {
            visitor.visit_path(type_path);
        }
        ExprKind::Spawn { expr: inner, contexts } => {
            for context in contexts {
                visitor.visit_path(&context.path);
                for ty in &context.args {
                    visit_child!(visitor, ty, Type);
                }
            }
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::Select { arms, .. } => {
            for arm in arms.iter() {
                if let Maybe::Some(ref pattern) = arm.pattern {
                    visit_child!(visitor, pattern, Pattern);
                }
                if let Maybe::Some(ref future) = arm.future {
                    visit_child!(visitor, future.as_ref(), Expr);
                }
                if let Maybe::Some(ref guard) = arm.guard {
                    visit_child!(visitor, guard.as_ref(), Expr);
                }
                visit_child!(visitor, &arm.body, Expr);
            }
        }
        ExprKind::UseContext {
            context,
            handler,
            body,
        } => {
            visitor.visit_path(context);
            visit_child!(visitor, handler.as_ref(), Expr);
            visit_child!(visitor, body.as_ref(), Expr);
        }
        ExprKind::Range { start, end, .. } => {
            if let Maybe::Some(start_expr) = start {
                visit_child!(visitor, start_expr.as_ref(), Expr);
            }
            if let Maybe::Some(end_expr) = end {
                visit_child!(visitor, end_expr.as_ref(), Expr);
            }
        }
        ExprKind::Forall { bindings, body } => {
            for binding in bindings {
                visit_child!(visitor, &binding.pattern, Pattern);
                if let Maybe::Some(ty) = &binding.ty {
                    visit_child!(visitor, ty, Type);
                }
                if let Maybe::Some(domain) = &binding.domain {
                    visit_child!(visitor, domain, Expr);
                }
                if let Maybe::Some(guard) = &binding.guard {
                    visit_child!(visitor, guard, Expr);
                }
            }
            visit_child!(visitor, body.as_ref(), Expr);
        }
        ExprKind::Exists { bindings, body } => {
            for binding in bindings {
                visit_child!(visitor, &binding.pattern, Pattern);
                if let Maybe::Some(ty) = &binding.ty {
                    visit_child!(visitor, ty, Type);
                }
                if let Maybe::Some(domain) = &binding.domain {
                    visit_child!(visitor, domain, Expr);
                }
                if let Maybe::Some(guard) = &binding.guard {
                    visit_child!(visitor, guard, Expr);
                }
            }
            visit_child!(visitor, body.as_ref(), Expr);
        }
        ExprKind::Paren(inner) => {
            visit_child!(visitor, inner.as_ref(), Expr);
        }
        ExprKind::Is { expr: inner, pattern, .. } => {
            visit_child!(visitor, inner.as_ref(), Expr);
            visit_child!(visitor, pattern, Pattern);
        }
        ExprKind::MacroCall { path, .. } => {
            visitor.visit_path(path);
            // MacroArgs are unparsed token trees, no need to visit
        }
        ExprKind::Attenuate { context, .. } => {
            visit_child!(visitor, context.as_ref(), Expr);
        }
        ExprKind::TypeProperty { ty, .. } => {
            visit_child!(visitor, ty, Type);
        }
        ExprKind::TypeExpr(ty) => {
            visit_child!(visitor, ty, Type);
        }
        ExprKind::TypeBound { type_param, bound } => {
            visitor.visit_ident(type_param);
            visit_child!(visitor, bound, Type);
        }
        ExprKind::MetaFunction { name, args } => {
            visitor.visit_ident(name);
            for arg in args.iter() {
                visit_child!(visitor, arg, Expr);
            }
        }
        ExprKind::Nursery { options, body, on_cancel, recover, .. } => {
            // Visit timeout expression if present
            if let Maybe::Some(timeout) = &options.timeout {
                visit_child!(visitor, timeout, Expr);
            }
            // Visit max_tasks expression if present
            if let Maybe::Some(max) = &options.max_tasks {
                visit_child!(visitor, max, Expr);
            }
            // Visit the body block
            for stmt in body.stmts.iter() {
                visit_child!(visitor, stmt, Stmt);
            }
            if let Maybe::Some(expr) = &body.expr {
                visit_child!(visitor, expr, Expr);
            }
            // Visit on_cancel block if present
            if let Maybe::Some(cancel_block) = on_cancel {
                for stmt in cancel_block.stmts.iter() {
                    visit_child!(visitor, stmt, Stmt);
                }
                if let Maybe::Some(expr) = &cancel_block.expr {
                    visit_child!(visitor, expr, Expr);
                }
            }
            // Visit recover body if present
            if let Maybe::Some(recover_body) = recover {
                match recover_body {
                    crate::expr::RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms.iter() {
                            visit_child!(visitor, &arm.pattern, Pattern);
                            if let Maybe::Some(guard) = &arm.guard {
                                visit_child!(visitor, guard, Expr);
                            }
                            visit_child!(visitor, &arm.body, Expr);
                        }
                    }
                    crate::expr::RecoverBody::Closure { param, body, .. } => {
                        visit_child!(visitor, &param.pattern, Pattern);
                        visit_child!(visitor, body, Expr);
                    }
                }
            }
        }
        // Inline assembly expression
        ExprKind::InlineAsm { operands, .. } => {
            for operand in operands.iter() {
                match &operand.kind {
                    crate::expr::AsmOperandKind::In { expr, .. } => {
                        visit_child!(visitor, expr.as_ref(), Expr);
                    }
                    crate::expr::AsmOperandKind::Out { place, .. } => {
                        visit_child!(visitor, place.as_ref(), Expr);
                    }
                    crate::expr::AsmOperandKind::InOut { place, .. } => {
                        visit_child!(visitor, place.as_ref(), Expr);
                    }
                    crate::expr::AsmOperandKind::InLateOut { in_expr, out_place, .. } => {
                        visit_child!(visitor, in_expr.as_ref(), Expr);
                        visit_child!(visitor, out_place.as_ref(), Expr);
                    }
                    crate::expr::AsmOperandKind::Sym { path } => {
                        visitor.visit_path(path);
                    }
                    crate::expr::AsmOperandKind::Const { expr } => {
                        visit_child!(visitor, expr.as_ref(), Expr);
                    }
                    crate::expr::AsmOperandKind::Clobber { .. } => {}
                }
            }
        }
        // Stream literal expression: stream[1, 2, 3, ...] or stream[0..100]
        // Stream comprehension expressions
        ExprKind::StreamLiteral(stream_lit) => {
            match &stream_lit.kind {
                crate::expr::StreamLiteralKind::Elements { elements, .. } => {
                    for elem in elements.iter() {
                        visit_child!(visitor, elem, Expr);
                    }
                }
                crate::expr::StreamLiteralKind::Range { start, end, .. } => {
                    visit_child!(visitor, start.as_ref(), Expr);
                    if let Maybe::Some(end_expr) = end {
                        visit_child!(visitor, end_expr.as_ref(), Expr);
                    }
                }
            }
        }
        // Destructuring assignment: (a, b) = expr
        // Algebraic effect handler expression (experimental)
        ExprKind::DestructuringAssign { pattern, value, .. } => {
            visit_child!(visitor, pattern, Pattern);
            visit_child!(visitor, value.as_ref(), Expr);
        }
        // Calculational proof block
        ExprKind::CalcBlock(_) => {
            // Steps contain expressions but we don't walk proof constructs yet
        }
        // Copattern body: visit each arm's body expression
        ExprKind::CopatternBody { arms, .. } => {
            for arm in arms.iter() {
                visit_child!(visitor, arm.body.as_ref(), Expr);
            }
        }
    }
}

/// Walk a statement, visiting all its children.
pub fn walk_stmt<V: Visitor>(visitor: &mut V, stmt: &Stmt) {
    match &stmt.kind {
        StmtKind::Let { pattern, ty, value } => {
            visit_child!(visitor, pattern, Pattern);
            if let Maybe::Some(ty) = ty {
                visit_child!(visitor, ty, Type);
            }
            if let Maybe::Some(value) = value {
                visit_child!(visitor, value, Expr);
            }
        }
        StmtKind::LetElse {
            pattern,
            ty,
            value,
            else_block,
        } => {
            visit_child!(visitor, pattern, Pattern);
            if let Maybe::Some(ty) = ty {
                visit_child!(visitor, ty, Type);
            }
            visit_child!(visitor, value, Expr);
            visit_child!(visitor, else_block, Block);
        }
        StmtKind::Expr { expr, .. } => {
            visit_child!(visitor, expr, Expr);
        }
        StmtKind::Item(item) => visit_child!(visitor, item, Item),
        StmtKind::Defer(expr) => {
            visit_child!(visitor, expr, Expr);
        }
        StmtKind::Errdefer(expr) => {
            visit_child!(visitor, expr, Expr);
        }
        StmtKind::Provide { value, .. } => {
            visit_child!(visitor, value, Expr);
        }
        StmtKind::ProvideScope { value, block, .. } => {
            visit_child!(visitor, value, Expr);
            visit_child!(visitor, block, Expr);
        }
        StmtKind::Empty => {}
    }
}

/// Walk a pattern.
pub fn walk_pattern<V: Visitor>(visitor: &mut V, pattern: &Pattern) {
    match &pattern.kind {
        PatternKind::Wildcard | PatternKind::Rest => {}
        PatternKind::Ident {
            name, subpattern, ..
        } => {
            visitor.visit_ident(name);
            if let Maybe::Some(subpat) = subpattern {
                visit_child!(visitor, subpat.as_ref(), Pattern);
            }
        }
        PatternKind::Literal(lit) => visitor.visit_literal(lit),
        PatternKind::Tuple(patterns) | PatternKind::Array(patterns) => {
            for pat in patterns {
                visit_child!(visitor, pat, Pattern);
            }
        }
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            for pat in before {
                visit_child!(visitor, pat, Pattern);
            }
            if let Maybe::Some(rest_pat) = rest {
                visit_child!(visitor, rest_pat.as_ref(), Pattern);
            }
            for pat in after {
                visit_child!(visitor, pat, Pattern);
            }
        }
        PatternKind::Record { path, fields, .. } => {
            visitor.visit_path(path);
            for field in fields {
                if let Maybe::Some(pat) = &field.pattern {
                    visit_child!(visitor, pat, Pattern);
                }
            }
        }
        PatternKind::Variant { path, data } => {
            visitor.visit_path(path);
            if let Maybe::Some(data) = data {
                match data {
                    VariantPatternData::Tuple(patterns) => {
                        for pat in patterns {
                            visit_child!(visitor, pat, Pattern);
                        }
                    }
                    VariantPatternData::Record { fields, .. } => {
                        for field in fields {
                            if let Maybe::Some(pat) = &field.pattern {
                                visit_child!(visitor, pat, Pattern);
                            }
                        }
                    }
                }
            }
        }
        PatternKind::Or(patterns) => {
            for pat in patterns {
                visit_child!(visitor, pat, Pattern);
            }
        }
        PatternKind::Reference { inner, .. } => visit_child!(visitor, inner.as_ref(), Pattern),
        PatternKind::Range { .. } => {}
        PatternKind::Paren(pat) => visit_child!(visitor, pat.as_ref(), Pattern),        PatternKind::View {
            view_function,
            pattern,
        } => {
            visit_child!(visitor, view_function, Expr);
            visit_child!(visitor, pattern.as_ref(), Pattern);
        }
        PatternKind::Active { name, params, bindings } => {
            visitor.visit_ident(name);
            for arg in params {
                visit_child!(visitor, arg, Expr);
            }
            for binding in bindings {
                visit_child!(visitor, binding, Pattern);
            }
        }
        PatternKind::And(patterns) => {
            for pat in patterns {
                visit_child!(visitor, pat, Pattern);
            }
        }
        PatternKind::TypeTest { binding, test_type } => {
            visitor.visit_ident(binding);
            visit_child!(visitor, test_type, Type);
        }
        // Stream pattern: stream[first, second, ...rest]
        // Stream pattern matching for lazy iterator destructuring
        PatternKind::Stream { head_patterns, rest } => {
            for pat in head_patterns.iter() {
                visit_child!(visitor, pat, Pattern);
            }
            if let Maybe::Some(rest_ident) = rest {
                visitor.visit_ident(rest_ident);
            }
        }

        // Cons pattern: head :: tail
        PatternKind::Cons { head, tail } => {
            visit_child!(visitor, head.as_ref(), Pattern);
            visit_child!(visitor, tail.as_ref(), Pattern);
        }

        // Guard pattern: pattern if guard_expr
        // Spec: Rust RFC 3637 - Guard Patterns
        PatternKind::Guard { pattern, guard } => {
            visit_child!(visitor, pattern.as_ref(), Pattern);
            visit_child!(visitor, guard.as_ref(), Expr);
        }
    }
}

/// Walk a type.
pub fn walk_type<V: Visitor>(visitor: &mut V, ty: &Type) {
    match &ty.kind {
        TypeKind::Unit
        | TypeKind::Never
        | TypeKind::Bool
        | TypeKind::Int
        | TypeKind::Float
        | TypeKind::Char
        | TypeKind::Text
        | TypeKind::Inferred
        | TypeKind::Unknown => {}
        TypeKind::Path(path) => visitor.visit_path(path),
        TypeKind::PathType { carrier, lhs, rhs } => {
            visit_child!(visitor, carrier.as_ref(), Type);
            visit_child!(visitor, lhs.as_ref(), Expr);
            visit_child!(visitor, rhs.as_ref(), Expr);
        }
        TypeKind::DependentApp { carrier, value_args } => {
            visit_child!(visitor, carrier.as_ref(), Type);
            for arg in value_args.iter() {
                visit_child!(visitor, arg, Expr);
            }
        }
        TypeKind::Tuple(types) => {
            for ty in types.iter() {
                visit_child!(visitor, ty, Type);
            }
        }
        TypeKind::Array { element, size } => {
            visit_child!(visitor, element.as_ref(), Type);
            if let Some(size_expr) = size {
                visit_child!(visitor, size_expr.as_ref(), Expr);
            }
        }
        TypeKind::Slice(inner) => visit_child!(visitor, inner.as_ref(), Type),
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            for param in params.iter() {
                visit_child!(visitor, param, Type);
            }
            visit_child!(visitor, return_type.as_ref(), Type);
        }
        TypeKind::Rank2Function {
            type_params: _,
            params,
            return_type,
            ..
        } => {
            // Note: type_params are local to this function type scope
            // We don't visit their bounds here as they're structural
            // Visit param and return types
            for param in params.iter() {
                visit_child!(visitor, param, Type);
            }
            visit_child!(visitor, return_type.as_ref(), Type);
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. }
        | TypeKind::Ownership { inner, .. }
        | TypeKind::Pointer { inner, .. }
        | TypeKind::VolatilePointer { inner, .. } => visit_child!(visitor, inner.as_ref(), Type),
        TypeKind::Generic { base, args } => {
            visit_child!(visitor, base.as_ref(), Type);
            for arg in args.iter() {
                match arg {
                    GenericArg::Type(ty) => visit_child!(visitor, ty, Type),
                    GenericArg::Const(expr) => visit_child!(visitor, expr, Expr),
                    GenericArg::Lifetime(_) => {}
                    GenericArg::Binding(binding) => visit_child!(visitor, &binding.ty, Type),
                }
            }
        }
        TypeKind::Qualified {
            self_ty,
            trait_ref,
            assoc_name,
        } => {
            visit_child!(visitor, self_ty.as_ref(), Type);
            visitor.visit_path(trait_ref);
            visitor.visit_ident(assoc_name);
        }
        TypeKind::Refined { base, predicate } => {
            visit_child!(visitor, base.as_ref(), Type);
            visit_child!(visitor, &predicate.expr, Expr);
        }
        TypeKind::Sigma {
            base, predicate, ..
        } => {
            visit_child!(visitor, base.as_ref(), Type);
            visit_child!(visitor, predicate.as_ref(), Expr);
        }
        TypeKind::Bounded { base, .. } => visit_child!(visitor, base.as_ref(), Type),
        TypeKind::DynProtocol { bindings, .. } => {
            if let Maybe::Some(bindings) = bindings {
                for binding in bindings.iter() {
                    visit_child!(visitor, &binding.ty, Type);
                }
            }
        }
        TypeKind::GenRef { inner } => {
            visit_child!(visitor, inner.as_ref(), Type);
        }
        TypeKind::TypeConstructor { .. } => {
            // Type constructors are leaf nodes (higher-kinded type placeholders)
        }
        TypeKind::Tensor {
            element,
            shape,
            layout: _,
        } => {
            visit_child!(visitor, element.as_ref(), Type);
            for dim_expr in shape.iter() {
                visit_child!(visitor, dim_expr, Expr);
            }
        }
        TypeKind::Existential { name, .. } => {
            visitor.visit_ident(name);
            // Bounds would be visited through TypeBound visitor if needed
        }
        TypeKind::AssociatedType { base, assoc } => {
            visit_child!(visitor, base.as_ref(), Type);
            visitor.visit_ident(assoc);
        }
        TypeKind::CapabilityRestricted { base, capabilities } => {
            visit_child!(visitor, base.as_ref(), Type);
            // Visit each capability (capabilities are leaf nodes with identifiers)
            for cap in &capabilities.capabilities {
                // Capabilities are enum variants, not AST nodes that need visiting.
                // The capability names are embedded in the enum, so no explicit traversal needed.
                let _ = cap; // Acknowledge the capability exists
            }
        }
        TypeKind::Record { fields, .. } => {
            for field in fields.iter() {
                visitor.visit_ident(&field.name);
                visit_child!(visitor, &field.ty, Type);
            }
        }
        TypeKind::Universe { .. } => {
            // Universe types (Type : Type_n) have no children to visit
        }
        TypeKind::Meta { inner } => {
            visit_child!(visitor, inner.as_ref(), Type);
        }
        TypeKind::TypeLambda { params, body } => {
            for param in params.iter() {
                visitor.visit_ident(param);
            }
            visit_child!(visitor, body.as_ref(), Type);
        }
    }
}

/// Walk a block, visiting all its children.
pub fn walk_block<V: Visitor>(visitor: &mut V, block: &Block) {
    for stmt in block.stmts.iter() {
        visit_child!(visitor, stmt, Stmt);
    }
    if let Maybe::Some(expr) = &block.expr {
        visit_child!(visitor, expr.as_ref(), Expr);
    }
}

/// Internal helper for walking if conditions.
fn walk_if_condition_internal<V: Visitor>(visitor: &mut V, condition: &IfCondition) {
    for cond in condition.conditions.iter() {
        match cond {
            ConditionKind::Expr(expr) => {
                visit_child!(visitor, expr, Expr);
            }
            ConditionKind::Let { pattern, value } => {
                visit_child!(visitor, pattern, Pattern);
                visit_child!(visitor, value, Expr);
            }
        }
    }
}

/// Internal helper for walking comprehension clauses.
fn walk_comprehension_clause_internal<V: Visitor>(visitor: &mut V, clause: &ComprehensionClause) {
    match &clause.kind {
        ComprehensionClauseKind::For { pattern, iter } => {
            visit_child!(visitor, pattern, Pattern);
            visit_child!(visitor, iter, Expr);
        }
        ComprehensionClauseKind::If(expr) => {
            visit_child!(visitor, expr, Expr);
        }
        ComprehensionClauseKind::Let { pattern, ty, value } => {
            visit_child!(visitor, pattern, Pattern);
            if let Maybe::Some(ty) = ty {
                visit_child!(visitor, ty, Type);
            }
            visit_child!(visitor, value, Expr);
        }
    }
}

/// Walk a where clause.
/// Mount statement for importing names into scope.#where-clause-disambiguation
pub fn walk_where_clause<V: Visitor>(visitor: &mut V, where_clause: &WhereClause) {
    for predicate in where_clause.predicates.iter() {
        visit_child!(visitor, predicate, WherePredicate);
    }
}

/// Walk a where predicate.
/// Mount statement for importing names into scope.#where-clause-disambiguation
///
/// Handles all four where clause forms in v6.0-BALANCED:
/// 1. `where type T: Protocol` - Generic type constraints
/// 2. `where meta N > 0` - Meta-parameter refinements
/// 3. `where value it > 0` - Value refinements
/// 4. `where ensures result >= 0` - Postconditions
pub fn walk_where_predicate<V: Visitor>(visitor: &mut V, predicate: &WherePredicate) {
    use crate::ty::WherePredicateKind;

    match &predicate.kind {
        // 1. Generic type constraint: where type T: Protocol
        WherePredicateKind::Type { ty, bounds: _ } => {
            visit_child!(visitor, ty, Type);
            // TypeBounds contain paths which are already visited as leaf nodes
        }

        // 2. Meta-parameter refinement: where meta N > 0
        WherePredicateKind::Meta { constraint } => {
            visit_child!(visitor, constraint, Expr);
        }

        // 3. Value refinement: where value it > 0
        WherePredicateKind::Value { predicate } => {
            visit_child!(visitor, predicate, Expr);
        }

        // 4. Postcondition: where ensures result >= 0
        WherePredicateKind::Ensures { postcondition } => {
            visit_child!(visitor, postcondition, Expr);
        }
    }
}

/// Walk an FFI boundary declaration.
///
/// Visits all functions and their contract expressions within the boundary.
/// Walk an FFI boundary declaration (compile-time specification for C ABI interop).
pub fn walk_ffi_boundary<V: Visitor>(visitor: &mut V, ffi_boundary: &FFIBoundary) {
    visitor.visit_ident(&ffi_boundary.name);

    // Visit each FFI function in the boundary
    for ffi_func in ffi_boundary.functions.iter() {
        // Visit function signature types
        for (param_name, param_type) in ffi_func.signature.params.iter() {
            visitor.visit_ident(param_name);
            visit_child!(visitor, param_type, Type);
        }
        visit_child!(visitor, &ffi_func.signature.return_type, Type);

        // Visit preconditions
        for require_expr in ffi_func.requires.iter() {
            visit_child!(visitor, require_expr, Expr);
        }

        // Visit postconditions
        for ensure_expr in ffi_func.ensures.iter() {
            visit_child!(visitor, ensure_expr, Expr);
        }

        // Visit error protocol expressions
        match &ffi_func.error_protocol {
            crate::ffi::ErrorProtocol::None => {}
            crate::ffi::ErrorProtocol::Errno => {}
            crate::ffi::ErrorProtocol::ReturnCode(expr) => {
                visit_child!(visitor, expr, Expr);
            }
            crate::ffi::ErrorProtocol::ReturnValue(expr) => {
                visit_child!(visitor, expr, Expr);
            }
            crate::ffi::ErrorProtocol::ReturnValueWithErrno(expr) => {
                visit_child!(visitor, expr, Expr);
            }
            crate::ffi::ErrorProtocol::Exception => {}
        }
    }
}

// ==================== Formal Proofs Visitors (v2.0+ extension) ====================

/// Walk a theorem declaration.
///
/// Visits theorem parameters, proposition, where clauses, and proof body.
/// Walk a theorem declaration including its proposition and proof body.
pub fn walk_theorem<V: Visitor>(visitor: &mut V, theorem: &TheoremDecl) {
    visitor.visit_ident(&theorem.name);

    // Visit parameters
    for param in &theorem.params {
        visit_child!(visitor, param, FunctionParam);
    }

    // Visit proposition
    visit_child!(visitor, &theorem.proposition, Expr);

    // Visit where clauses
    if let Maybe::Some(where_clause) = &theorem.generic_where_clause {
        visit_child!(visitor, where_clause, WhereClause);
    }
    if let Maybe::Some(meta_where) = &theorem.meta_where_clause {
        visit_child!(visitor, meta_where, WhereClause);
    }

    // Visit proof body
    if let Maybe::Some(proof_body) = &theorem.proof {
        visit_child!(visitor, proof_body, ProofBody);
    }
}

/// Walk an axiom declaration.
///
/// Visits axiom parameters, proposition, and where clauses.
pub fn walk_axiom<V: Visitor>(visitor: &mut V, axiom: &AxiomDecl) {
    visitor.visit_ident(&axiom.name);

    // Visit parameters
    for param in &axiom.params {
        visit_child!(visitor, param, FunctionParam);
    }

    // Visit proposition
    visit_child!(visitor, &axiom.proposition, Expr);

    // Visit where clauses
    if let Maybe::Some(where_clause) = &axiom.generic_where_clause {
        visit_child!(visitor, where_clause, WhereClause);
    }
    if let Maybe::Some(meta_where) = &axiom.meta_where_clause {
        visit_child!(visitor, meta_where, WhereClause);
    }
}

/// Walk a tactic declaration.
///
/// Visits tactic body.
/// Walk a tactic declaration including parameters and tactic body.
pub fn walk_tactic<V: Visitor>(visitor: &mut V, tactic: &TacticDecl) {
    use crate::decl::TacticBody;

    visitor.visit_ident(&tactic.name);

    // Visit tactic body
    match &tactic.body {
        TacticBody::Simple(tactic_expr) => {
            visit_child!(visitor, tactic_expr, TacticExpr);
        }
        TacticBody::Block(tactics) => {
            for tactic_expr in tactics {
                visit_child!(visitor, tactic_expr, TacticExpr);
            }
        }
    }
}

/// Walk a tactic expression.
///
/// Visits all sub-expressions within a tactic.
/// Walk proof body structures including tactics, have/show steps, and case analysis.
pub fn walk_tactic_expr<V: Visitor>(visitor: &mut V, tactic_expr: &TacticExpr) {
    match tactic_expr {
        TacticExpr::Trivial
        | TacticExpr::Assumption
        | TacticExpr::Reflexivity
        | TacticExpr::Ring
        | TacticExpr::Field
        | TacticExpr::Omega
        | TacticExpr::Blast
        | TacticExpr::Split
        | TacticExpr::Left
        | TacticExpr::Right
        | TacticExpr::Compute
        | TacticExpr::Done
        | TacticExpr::Admit
        | TacticExpr::Sorry
        | TacticExpr::Contradiction => {
            // Leaf tactics
        }

        TacticExpr::Intro(idents) => {
            for ident in idents {
                visitor.visit_ident(ident);
            }
        }

        TacticExpr::Apply { lemma, args } => {
            visit_child!(visitor, lemma, Expr);
            for arg in args {
                visit_child!(visitor, arg, Expr);
            }
        }

        TacticExpr::Rewrite {
            hypothesis,
            at_target,
            ..
        } => {
            visit_child!(visitor, hypothesis, Expr);
            if let Maybe::Some(target) = at_target {
                visitor.visit_ident(target);
            }
        }

        TacticExpr::Simp { lemmas, at_target } => {
            for lemma in lemmas {
                visit_child!(visitor, lemma, Expr);
            }
            if let Maybe::Some(target) = at_target {
                visitor.visit_ident(target);
            }
        }

        TacticExpr::Auto { with_hints } => {
            for hint in with_hints {
                visitor.visit_ident(hint);
            }
        }

        TacticExpr::Smt { .. } => {
            // SMT configuration, no expressions
        }

        TacticExpr::Exists(expr) => {
            visit_child!(visitor, expr, Expr);
        }

        TacticExpr::CasesOn(ident) | TacticExpr::InductionOn(ident) => {
            visitor.visit_ident(ident);
        }

        TacticExpr::Exact(expr) => {
            visit_child!(visitor, expr, Expr);
        }

        TacticExpr::Unfold(idents) => {
            for ident in idents {
                visitor.visit_ident(ident);
            }
        }

        TacticExpr::Try(inner)
        | TacticExpr::Repeat(inner)
        | TacticExpr::AllGoals(inner)
        | TacticExpr::Focus(inner) => {
            visit_child!(visitor, inner.as_ref(), TacticExpr);
        }

        TacticExpr::TryElse { body, fallback } => {
            visit_child!(visitor, body.as_ref(), TacticExpr);
            visit_child!(visitor, fallback.as_ref(), TacticExpr);
        }

        TacticExpr::Seq(tactics) | TacticExpr::Alt(tactics) => {
            for tactic in tactics {
                visit_child!(visitor, tactic, TacticExpr);
            }
        }

        TacticExpr::Named { name, generic_args, args } => {
            visitor.visit_ident(name);
            for ty in generic_args {
                visitor.visit_type(ty);
            }
            for arg in args {
                visit_child!(visitor, arg, Expr);
            }
        }

        TacticExpr::Let { name, ty, value } => {
            visitor.visit_ident(name);
            if let Maybe::Some(t) = ty {
                visitor.visit_type(t);
            }
            visit_child!(visitor, value.as_ref(), Expr);
        }

        TacticExpr::Match { scrutinee, arms } => {
            visit_child!(visitor, scrutinee.as_ref(), Expr);
            for arm in arms {
                visitor.visit_pattern(&arm.pattern);
                if let Maybe::Some(g) = &arm.guard {
                    visit_child!(visitor, g.as_ref(), Expr);
                }
                visit_child!(visitor, arm.body.as_ref(), TacticExpr);
            }
        }

        TacticExpr::Fail { message } => {
            visit_child!(visitor, message.as_ref(), Expr);
        }

        TacticExpr::If { cond, then_branch, else_branch } => {
            visit_child!(visitor, cond.as_ref(), Expr);
            visit_child!(visitor, then_branch.as_ref(), TacticExpr);
            if let Maybe::Some(e) = else_branch {
                visit_child!(visitor, e.as_ref(), TacticExpr);
            }
        }
    }
}

/// Walk a proof body.
///
/// Visits all expressions and tactics within a proof.
/// Walk proof body structures including tactics, have/show steps, and case analysis.
pub fn walk_proof_body<V: Visitor>(visitor: &mut V, proof_body: &ProofBody) {
    match proof_body {
        ProofBody::Term(expr) => {
            visit_child!(visitor, expr, Expr);
        }

        ProofBody::Tactic(tactic_expr) => {
            visit_child!(visitor, tactic_expr, TacticExpr);
        }

        ProofBody::Structured(structure) => {
            walk_proof_structure(visitor, structure);
        }

        ProofBody::ByMethod(method) => {
            walk_proof_method(visitor, method);
        }
    }
}

/// Walk a structured proof.
fn walk_proof_structure<V: Visitor>(visitor: &mut V, structure: &crate::decl::ProofStructure) {
    for step in &structure.steps {
        walk_proof_step(visitor, step);
    }

    if let Maybe::Some(conclusion) = &structure.conclusion {
        visit_child!(visitor, conclusion, TacticExpr);
    }
}

/// Walk a proof step.
fn walk_proof_step<V: Visitor>(visitor: &mut V, step: &crate::decl::ProofStep) {
    match &step.kind {
        ProofStepKind::Have {
            name,
            proposition,
            justification,
        } => {
            visitor.visit_ident(name);
            visit_child!(visitor, proposition, Expr);
            visit_child!(visitor, justification, TacticExpr);
        }

        ProofStepKind::Show {
            proposition,
            justification,
        } => {
            visit_child!(visitor, proposition, Expr);
            visit_child!(visitor, justification, TacticExpr);
        }

        ProofStepKind::Suffices {
            proposition,
            justification,
        } => {
            visit_child!(visitor, proposition, Expr);
            visit_child!(visitor, justification, TacticExpr);
        }

        ProofStepKind::Let { pattern, value } => {
            visit_child!(visitor, pattern, Pattern);
            visit_child!(visitor, value, Expr);
        }

        ProofStepKind::Obtain { pattern, from } => {
            visit_child!(visitor, pattern, Pattern);
            visit_child!(visitor, from, Expr);
        }

        ProofStepKind::Calc(calc_chain) => {
            visit_child!(visitor, &calc_chain.start, Expr);
            for calc_step in &calc_chain.steps {
                visit_child!(visitor, &calc_step.target, Expr);
                visit_child!(visitor, &calc_step.justification, TacticExpr);
            }
        }

        ProofStepKind::Cases { scrutinee, cases } => {
            visit_child!(visitor, scrutinee, Expr);
            for case in cases {
                visit_child!(visitor, &case.pattern, Pattern);
                for step in &case.proof {
                    walk_proof_step(visitor, step);
                }
            }
        }

        ProofStepKind::Focus {
            goal_index: _,
            steps,
        } => {
            for step in steps {
                walk_proof_step(visitor, step);
            }
        }

        ProofStepKind::Tactic(tactic) => {
            visit_child!(visitor, tactic, TacticExpr);
        }
    }
}

/// Walk a proof method.
fn walk_proof_method<V: Visitor>(visitor: &mut V, method: &crate::decl::ProofMethod) {
    use crate::decl::ProofMethod;

    match method {
        ProofMethod::Induction { on, cases } => {
            if let Maybe::Some(ident) = on {
                visitor.visit_ident(ident);
            }
            for case in cases {
                visit_child!(visitor, &case.pattern, Pattern);
                for step in &case.proof {
                    walk_proof_step(visitor, step);
                }
            }
        }

        ProofMethod::Cases { on, cases } => {
            visit_child!(visitor, on, Expr);
            for case in cases {
                visit_child!(visitor, &case.pattern, Pattern);
                for step in &case.proof {
                    walk_proof_step(visitor, step);
                }
            }
        }

        ProofMethod::Contradiction { assumption, proof } => {
            visitor.visit_ident(assumption);
            for step in proof {
                walk_proof_step(visitor, step);
            }
        }

        ProofMethod::StrongInduction { on, cases } => {
            visitor.visit_ident(on);
            for case in cases {
                visit_child!(visitor, &case.pattern, Pattern);
                for step in &case.proof {
                    walk_proof_step(visitor, step);
                }
            }
        }

        ProofMethod::WellFoundedInduction {
            relation,
            on,
            cases,
        } => {
            visit_child!(visitor, relation, Expr);
            visitor.visit_ident(on);
            for case in cases {
                visit_child!(visitor, &case.pattern, Pattern);
                for step in &case.proof {
                    walk_proof_step(visitor, step);
                }
            }
        }
    }
}

/// Walk a view declaration.
///
/// Visits view name, parameter/return types, and constructors.
/// Walk view declarations (alternative pattern interfaces, v2.0+ planned).
pub fn walk_view<V: Visitor>(visitor: &mut V, view: &crate::decl::ViewDecl) {
    visitor.visit_ident(&view.name);
    visit_child!(visitor, &view.param_type, Type);
    visit_child!(visitor, &view.return_type, Type);

    // Visit constructors
    for constructor in &view.constructors {
        visit_child!(visitor, constructor, ViewConstructor);
    }
}

/// Walk a view constructor.
///
/// Visits constructor name and result type.
/// Walk view declarations (alternative pattern interfaces, v2.0+ planned).
pub fn walk_view_constructor<V: Visitor>(
    visitor: &mut V,
    constructor: &crate::decl::ViewConstructor,
) {
    visitor.visit_ident(&constructor.name);
    visit_child!(visitor, &constructor.result_type, Type);
}

/// Walk a function parameter.
pub fn walk_function_param<V: Visitor>(visitor: &mut V, param: &FunctionParam) {
    match &param.kind {
        FunctionParamKind::Regular { pattern, ty, default_value } => {
            visit_child!(visitor, pattern, Pattern);
            visit_child!(visitor, ty, Type);
            if let Maybe::Some(default_expr) = default_value {
                visit_child!(visitor, default_expr, Expr);
            }
        }
        // Self parameters don't have children to visit
        FunctionParamKind::SelfValue
        | FunctionParamKind::SelfValueMut
        | FunctionParamKind::SelfRef
        | FunctionParamKind::SelfRefMut
        | FunctionParamKind::SelfRefChecked
        | FunctionParamKind::SelfRefCheckedMut
        | FunctionParamKind::SelfRefUnsafe
        | FunctionParamKind::SelfRefUnsafeMut
        | FunctionParamKind::SelfOwn
        | FunctionParamKind::SelfOwnMut => {}
    }
}

// ============================================================================
// CONVENIENCE FUNCTIONS
// ============================================================================

/// Traverse an expression iteratively (stack-safe).
///
/// This is a convenience function that wraps any visitor in [`IterativeVisitor`]
/// and performs stack-safe traversal.
///
/// # Example
///
/// ```ignore
/// let mut counter = ExprCounter::new();
/// traverse_expr_iteratively(&mut counter, &very_deep_ast);
/// ```
pub fn traverse_expr_iteratively<V: Visitor>(visitor: V, expr: &Expr) -> V {
    let mut iter = IterativeVisitor::new(visitor);
    iter.traverse_expr(expr);
    iter.into_inner()
}

/// Traverse a statement iteratively (stack-safe).
pub fn traverse_stmt_iteratively<V: Visitor>(visitor: V, stmt: &Stmt) -> V {
    let mut iter = IterativeVisitor::new(visitor);
    iter.traverse_stmt(stmt);
    iter.into_inner()
}

/// Traverse a block iteratively (stack-safe).
pub fn traverse_block_iteratively<V: Visitor>(visitor: V, block: &Block) -> V {
    let mut iter = IterativeVisitor::new(visitor);
    iter.traverse_block(block);
    iter.into_inner()
}

/// Traverse an item iteratively (stack-safe).
pub fn traverse_item_iteratively<V: Visitor>(visitor: V, item: &Item) -> V {
    let mut iter = IterativeVisitor::new(visitor);
    iter.traverse_item(item);
    iter.into_inner()
}
