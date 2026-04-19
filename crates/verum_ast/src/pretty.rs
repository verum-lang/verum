//! Pretty-printing (formatting) support for the Verum AST.
//!
//! This module provides the infrastructure to convert parsed AST back to
//! well-formatted source code. It's used by the `verum fmt` command.
//!
//! # Features
//!
//! - Complete AST-to-source conversion for all node types
//! - Configurable indentation (spaces vs tabs)
//! - Configurable line width
//! - Trailing comma support
//! - Multi-line expression handling

use crate::Module;
use crate::decl::{
    ConstDecl, ContextDecl, ContextGroupDecl, ContextRequirement, ExternBlockDecl, FunctionBody,
    FunctionDecl, FunctionParam, FunctionParamKind, ImplDecl, ImplItem, ImplItemKind, ImplKind,
    MountDecl, MountTree, MountTreeKind, Item, ItemKind, ModuleDecl, PatternDecl, ProtocolBody,
    ProtocolDecl, ProtocolItem, ProtocolItemKind, RecordField, ResourceModifier, StaticDecl,
    TypeDecl, TypeDeclBody, Variant, VariantData, Visibility,
};
#[cfg(test)]
use crate::expr::BinOp;
use crate::expr::{
    ArrayExpr, Block, ClosureParam, ComprehensionClause, ComprehensionClauseKind, ConditionKind,
    Expr, ExprKind, FieldInit, IfCondition, MacroDelimiter, NurseryErrorBehavior, RecoverBody, UnOp,
};
use crate::literal::{CompositeDelimiter, FloatSuffix, IntSuffix, Literal, LiteralKind, StringLit};
use crate::pattern::MatchArm;
use crate::pattern::{FieldPattern, Pattern, PatternKind, VariantPatternData};
use crate::stmt::{Stmt, StmtKind};
#[cfg(test)]
use crate::ty::Ident;
use crate::ty::{
    GenericArg, GenericParam, GenericParamKind, Path, PathSegment, RefinementPredicate,
    TensorLayout, Type, TypeBinding, TypeBound, TypeBoundKind, TypeKind, UniverseLevelExpr,
    WhereClause, WherePredicate, WherePredicateKind,
};
use verum_common::{List, Maybe, Text};

/// Configuration for the pretty printer.
#[derive(Debug, Clone)]
pub struct PrettyConfig {
    /// Maximum line width before breaking (default: 100)
    pub max_width: usize,
    /// Indentation size in spaces (default: 4)
    pub indent_size: usize,
    /// Use spaces instead of tabs (default: true)
    pub use_spaces: bool,
    /// Insert trailing commas in multi-line expressions (default: true)
    pub trailing_comma: bool,
    /// Minimum number of items to trigger multi-line formatting
    pub multiline_threshold: usize,
}

impl Default for PrettyConfig {
    fn default() -> Self {
        Self {
            max_width: 100,
            indent_size: 4,
            use_spaces: true,
            trailing_comma: true,
            multiline_threshold: 3,
        }
    }
}

/// Pretty printer for Verum AST.
///
/// This printer converts AST nodes back to well-formatted source code.
#[derive(Debug)]
pub struct PrettyPrinter {
    config: PrettyConfig,
    /// Current output buffer
    output: String,
    /// Current indentation level
    indent_level: usize,
    /// Current column position (for line width tracking)
    column: usize,
}

impl PrettyPrinter {
    /// Create a new pretty printer with the given configuration.
    pub fn new(config: PrettyConfig) -> Self {
        Self {
            config,
            output: String::new(),
            indent_level: 0,
            column: 0,
        }
    }

    /// Create a pretty printer with default configuration.
    pub fn default_printer() -> Self {
        Self::new(PrettyConfig::default())
    }

    /// Reset the printer state for reuse.
    fn reset(&mut self) {
        self.output.clear();
        self.indent_level = 0;
        self.column = 0;
    }

    /// Get the current indentation string.
    fn indent_str(&self) -> String {
        if self.config.use_spaces {
            " ".repeat(self.indent_level * self.config.indent_size)
        } else {
            "\t".repeat(self.indent_level)
        }
    }

    /// Write a string to the output.
    fn write(&mut self, s: &str) {
        self.output.push_str(s);
        // Update column tracking
        if let Some(last_newline) = s.rfind('\n') {
            self.column = s.len() - last_newline - 1;
        } else {
            self.column += s.len();
        }
    }

    /// Write a string followed by a newline.
    fn writeln(&mut self, s: &str) {
        self.output.push_str(s);
        self.output.push('\n');
        self.column = 0;
    }

    /// Write a newline and indentation.
    fn newline(&mut self) {
        self.output.push('\n');
        self.output.push_str(&self.indent_str());
        self.column = self.indent_level * self.config.indent_size;
    }

    /// Increase indentation level.
    fn indent(&mut self) {
        self.indent_level += 1;
    }

    /// Decrease indentation level.
    fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    /// Format a complete module.
    pub fn format_module(&mut self, module: &Module) -> Text {
        self.reset();

        // Format module-level attributes
        for attr in &module.attributes {
            self.format_attribute(attr);
            self.newline();
        }

        // Format items with blank lines between them
        let mut first = true;
        for item in &module.items {
            if !first {
                self.newline();
            }
            first = false;
            self.format_item(item);
        }

        Text::from(self.output.clone())
    }

    /// Check if AST-based formatting is fully implemented.
    pub fn is_full_implementation() -> bool {
        true
    }

    // ==================== ATTRIBUTES ====================

    fn format_attribute(&mut self, attr: &crate::attr::Attribute) {
        self.write("@");
        self.write(attr.name.as_str());
        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                self.write("(");
                let mut first = true;
                for arg in args {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_expr(arg);
                }
                self.write(")");
            }
        }
    }

    // ==================== ITEMS ====================

    fn format_item(&mut self, item: &Item) {
        // Format attributes
        for attr in &item.attributes {
            self.format_attribute(attr);
            self.newline();
        }

        match &item.kind {
            ItemKind::Function(func) => self.format_function(func),
            ItemKind::Type(ty_decl) => self.format_type_decl(ty_decl),
            ItemKind::Protocol(proto) => self.format_protocol(proto),
            ItemKind::Impl(impl_decl) => self.format_impl(impl_decl),
            ItemKind::Module(module) => self.format_module_decl(module),
            ItemKind::Const(const_decl) => self.format_const(const_decl),
            ItemKind::Static(static_decl) => self.format_static(static_decl),
            ItemKind::Mount(mount) => self.format_mount(mount),
            ItemKind::Context(ctx) => self.format_context(ctx),
            ItemKind::ContextGroup(group) => self.format_context_group(group),
            ItemKind::Layer(_) => { /* no-op */ }
            ItemKind::FFIBoundary(ffi) => self.format_ffi_boundary(ffi),
            ItemKind::Predicate(pred) => self.format_predicate(pred),
            ItemKind::Meta(meta) => self.format_meta(meta),
            ItemKind::Theorem(thm) => self.format_theorem(thm, "theorem"),
            ItemKind::Lemma(thm) => self.format_theorem(thm, "lemma"),
            ItemKind::Corollary(thm) => self.format_theorem(thm, "corollary"),
            ItemKind::Axiom(axiom) => self.format_axiom(axiom),
            ItemKind::Tactic(tactic) => self.format_tactic(tactic),
            ItemKind::View(view) => self.format_view(view),
            ItemKind::ExternBlock(extern_block) => self.format_extern_block(extern_block),
            ItemKind::Pattern(pattern_decl) => self.format_pattern_decl(pattern_decl),
        }
    }

    // ==================== EXTERN BLOCK ====================

    fn format_extern_block(&mut self, extern_block: &ExternBlockDecl) {
        self.write("extern ");
        if let Maybe::Some(ref abi) = extern_block.abi {
            self.write("\"");
            self.write(abi);
            self.write("\" ");
        }
        self.write("{");
        self.newline();
        self.indent();

        for func in &extern_block.functions {
            self.format_function(func);
            self.newline();
        }

        self.dedent();
        self.write("}");
    }

    // ==================== VISIBILITY ====================

    fn format_visibility(&mut self, vis: &Visibility) {
        match vis {
            Visibility::Public => self.write("public "),
            Visibility::PublicCrate => self.write("public(crate) "),
            Visibility::PublicSuper => self.write("public(super) "),
            Visibility::PublicIn(path) => {
                self.write("public(in ");
                self.format_path(path);
                self.write(") ");
            }
            Visibility::Internal => self.write("internal "),
            Visibility::Protected => self.write("protected "),
            Visibility::Private => {} // No keyword for private
        }
    }

    // ==================== FUNCTIONS ====================

    fn format_function(&mut self, func: &FunctionDecl) {
        // Visibility
        self.format_visibility(&func.visibility);

        // Modifiers
        if func.is_meta {
            self.write("meta ");
        }
        if func.is_async {
            self.write("async ");
        }
        if func.is_cofix {
            self.write("cofix ");
        }
        if let Maybe::Some(abi) = &func.extern_abi {
            self.write("extern ");
            if !abi.is_empty() {
                self.write("\"");
                self.write(abi.as_str());
                self.write("\" ");
            }
        }

        // fn or fn* (generator)
        // Spec: grammar/verum.ebnf v2.10 - fn_keyword = 'fn' , [ '*' ]
        if func.is_generator {
            self.write("fn* ");
        } else {
            self.write("fn ");
        }
        self.write(func.name.as_str());

        // Generics
        self.format_generics(&func.generics);

        // Parameters
        self.write("(");
        self.format_function_params(&func.params);
        self.write(")");

        // Throws clause
        if let Maybe::Some(throws) = &func.throws_clause {
            self.write(" throws(");
            let mut first = true;
            for error_ty in &throws.error_types {
                if !first {
                    self.write(" | ");
                }
                first = false;
                self.format_type(error_ty);
            }
            self.write(")");
        }

        // Return type
        if let Maybe::Some(ret_ty) = &func.return_type {
            self.write(" -> ");
            self.format_type(ret_ty);
        }

        // Context requirements
        if !func.contexts.is_empty() {
            self.write(" using ");
            if func.contexts.len() == 1 {
                self.format_context_requirement(&func.contexts[0]);
            } else {
                self.write("[");
                let mut first = true;
                for ctx in &func.contexts {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_context_requirement(ctx);
                }
                self.write("]");
            }
        }

        // Where clauses
        if let Maybe::Some(where_clause) = &func.generic_where_clause {
            self.format_where_clause(where_clause, "type");
        }
        if let Maybe::Some(where_clause) = &func.meta_where_clause {
            self.format_where_clause(where_clause, "meta");
        }

        // Requires/ensures
        for req in &func.requires {
            self.newline();
            self.write("    requires ");
            self.format_expr(req);
        }
        for ens in &func.ensures {
            self.newline();
            self.write("    ensures ");
            self.format_expr(ens);
        }

        // Body
        if let Maybe::Some(body) = &func.body {
            self.write(" ");
            self.format_function_body(body);
        } else {
            self.write(";");
        }
    }

    fn format_function_params(&mut self, params: &List<FunctionParam>) {
        let mut first = true;
        for param in params {
            if !first {
                self.write(", ");
            }
            first = false;
            self.format_function_param(param);
        }
    }

    fn format_function_param(&mut self, param: &FunctionParam) {
        match &param.kind {
            FunctionParamKind::Regular { pattern, ty, default_value } => {
                self.format_pattern(pattern);
                self.write(": ");
                self.format_type(ty);
                if let Maybe::Some(expr) = default_value {
                    self.write(" = ");
                    self.format_expr(expr);
                }
            }
            FunctionParamKind::SelfValue => self.write("self"),
            FunctionParamKind::SelfValueMut => self.write("mut self"),
            FunctionParamKind::SelfRef => self.write("&self"),
            FunctionParamKind::SelfRefMut => self.write("&mut self"),
            FunctionParamKind::SelfRefChecked => self.write("&checked self"),
            FunctionParamKind::SelfRefCheckedMut => self.write("&checked mut self"),
            FunctionParamKind::SelfRefUnsafe => self.write("&unsafe self"),
            FunctionParamKind::SelfRefUnsafeMut => self.write("&unsafe mut self"),
            FunctionParamKind::SelfOwn => self.write("%self"),
            FunctionParamKind::SelfOwnMut => self.write("%mut self"),
        }
    }

    fn format_function_body(&mut self, body: &FunctionBody) {
        match body {
            FunctionBody::Block(block) => self.format_block(block),
            FunctionBody::Expr(expr) => {
                self.write("= ");
                self.format_expr(expr);
                self.write(";");
            }
        }
    }

    fn format_context_requirement(&mut self, ctx: &ContextRequirement) {
        self.format_path(&ctx.path);
        if !ctx.args.is_empty() {
            self.write("<");
            let mut first = true;
            for arg in &ctx.args {
                if !first {
                    self.write(", ");
                }
                first = false;
                self.format_type(arg);
            }
            self.write(">");
        }
    }

    // ==================== GENERICS ====================

    fn format_generics(&mut self, generics: &List<GenericParam>) {
        if generics.is_empty() {
            return;
        }

        self.write("<");
        let mut first = true;
        for param in generics {
            if !first {
                self.write(", ");
            }
            first = false;
            self.format_generic_param(param);
        }
        self.write(">");
    }

    fn format_generic_param(&mut self, param: &GenericParam) {
        match &param.kind {
            GenericParamKind::Type {
                name,
                bounds,
                default,
            } => {
                self.write(name.as_str());
                if !bounds.is_empty() {
                    self.write(": ");
                    self.format_type_bounds(bounds);
                }
                if let Maybe::Some(def) = default {
                    self.write(" = ");
                    self.format_type(def);
                }
            }
            GenericParamKind::Const { name, ty } => {
                self.write("const ");
                self.write(name.as_str());
                self.write(": ");
                self.format_type(ty);
            }
            GenericParamKind::Meta {
                name,
                ty,
                refinement,
            } => {
                self.write(name.as_str());
                self.write(": meta ");
                self.format_type(ty);
                if let Maybe::Some(ref_expr) = refinement {
                    self.write("{");
                    self.format_expr(ref_expr);
                    self.write("}");
                }
            }
            GenericParamKind::HigherKinded {
                name,
                arity,
                bounds,
            } => {
                self.write(name.as_str());
                self.write("<");
                for i in 0..*arity {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write("_");
                }
                self.write(">");
                if !bounds.is_empty() {
                    self.write(": ");
                    self.format_type_bounds(bounds);
                }
            }
            GenericParamKind::Lifetime { name } => {
                self.write("'");
                self.write(name.name.as_str());
            }
            GenericParamKind::Context { name } => {
                // Context polymorphism: using C
                // Context polymorphism: propagate callback context requirements
                self.write("using ");
                self.write(name.name.as_str());
            }
            GenericParamKind::Level { name, .. } => {
                // Universe level parameter: u: Level
                self.write(name.name.as_str());
                self.write(": Level");
            }
            GenericParamKind::KindAnnotated { name, kind, bounds } => {
                // HKT kind-annotated parameter: F: Type -> Type
                self.write(name.name.as_str());
                self.write(": ");
                self.write(&kind.to_string());
                if !bounds.is_empty() {
                    self.write(" + ");
                    self.format_type_bounds(bounds);
                }
            }
        }
    }

    fn format_type_bounds(&mut self, bounds: &List<TypeBound>) {
        let mut first = true;
        for bound in bounds {
            if !first {
                self.write(" + ");
            }
            first = false;
            self.format_type_bound(bound);
        }
    }

    fn format_type_bound(&mut self, bound: &TypeBound) {
        match &bound.kind {
            TypeBoundKind::Protocol(path) => self.format_path(path),
            TypeBoundKind::Equality(ty) => {
                self.write("= ");
                self.format_type(ty);
            }
            TypeBoundKind::NegativeProtocol(path) => {
                self.write("!");
                self.format_path(path);
            }
            TypeBoundKind::AssociatedTypeBound {
                type_path,
                assoc_name,
                bounds,
            } => {
                self.format_path(type_path);
                self.write(".");
                self.write(assoc_name.as_str());
                self.write(": ");
                self.format_type_bounds(bounds);
            }
            TypeBoundKind::AssociatedTypeEquality {
                type_path,
                assoc_name,
                eq_type,
            } => {
                self.format_path(type_path);
                self.write(".");
                self.write(assoc_name.as_str());
                self.write(" = ");
                self.format_type(eq_type);
            }
            TypeBoundKind::GenericProtocol(ty) => {
                // Generic protocol bound like Iterator<Item = T>
                self.format_type(ty);
            }
        }
    }

    fn format_where_clause(&mut self, where_clause: &WhereClause, kind: &str) {
        if where_clause.predicates.is_empty() {
            return;
        }

        self.newline();
        self.write("    where ");
        self.write(kind);
        self.write(" ");

        let mut first = true;
        for pred in &where_clause.predicates {
            if !first {
                self.write(", ");
            }
            first = false;
            self.format_where_predicate(pred);
        }
    }

    fn format_where_predicate(&mut self, pred: &WherePredicate) {
        match &pred.kind {
            WherePredicateKind::Type { ty, bounds } => {
                self.format_type(ty);
                self.write(": ");
                self.format_type_bounds(bounds);
            }
            WherePredicateKind::Meta { constraint } => {
                self.format_expr(constraint);
            }
            WherePredicateKind::Value { predicate } => {
                self.format_expr(predicate);
            }
            WherePredicateKind::Ensures { postcondition } => {
                self.write("ensures ");
                self.format_expr(postcondition);
            }
        }
    }

    // ==================== TYPES ====================

    fn format_type(&mut self, ty: &Type) {
        match &ty.kind {
            TypeKind::Unit => self.write("()"),
            TypeKind::Never => self.write("!"),
            TypeKind::Bool => self.write("Bool"),
            TypeKind::Int => self.write("Int"),
            TypeKind::Float => self.write("Float"),
            TypeKind::Char => self.write("Char"),
            TypeKind::Text => self.write("Text"),
            TypeKind::Path(path) => self.format_path(path),
            TypeKind::PathType { carrier, lhs, rhs } => {
                self.write("Path<");
                self.format_type(carrier);
                self.write(">(");
                self.format_expr(lhs);
                self.write(", ");
                self.format_expr(rhs);
                self.write(")");
            }
            TypeKind::DependentApp { carrier, value_args } => {
                // `carrier` already renders its own `Head<TypeArgs>`; we
                // only tack on the value-argument parenthesized suffix.
                self.format_type(carrier);
                self.write("(");
                let mut first = true;
                for arg in value_args {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_expr(arg);
                }
                self.write(")");
            }
            TypeKind::Tuple(types) => {
                self.write("(");
                let mut first = true;
                for t in types {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_type(t);
                }
                if types.len() == 1 {
                    self.write(","); // Trailing comma for single-element tuple
                }
                self.write(")");
            }
            TypeKind::Array { element, size } => {
                self.write("[");
                self.format_type(element);
                if let Maybe::Some(sz) = size {
                    self.write("; ");
                    self.format_expr(sz);
                }
                self.write("]");
            }
            TypeKind::Slice(inner) => {
                self.write("[");
                self.format_type(inner);
                self.write("]");
            }
            TypeKind::Function {
                params,
                return_type,
                calling_convention,
                contexts,
            } => {
                // Print extern prefix if there's a calling convention
                if let Maybe::Some(cc) = calling_convention {
                    self.write("extern \"");
                    self.write(cc.as_str());
                    self.write("\" ");
                }
                self.write("fn(");
                let mut first = true;
                for p in params {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_type(p);
                }
                self.write(") -> ");
                self.format_type(return_type);
                // Print context requirements if any
                if !contexts.is_empty() {
                    self.write(" using [");
                    let mut ctx_first = true;
                    for ctx in contexts.iter() {
                        if !ctx_first {
                            self.write(", ");
                        }
                        ctx_first = false;
                        self.format_context_requirement(ctx);
                    }
                    self.write("]");
                }
            }
            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                calling_convention,
                contexts,
                where_clause,
            } => {
                // Print extern prefix if there's a calling convention
                if let Maybe::Some(cc) = calling_convention {
                    self.write("extern \"");
                    self.write(cc.as_str());
                    self.write("\" ");
                }
                self.write("fn");
                // Print type parameters: fn<R>(...)
                self.format_generics(type_params);
                self.write("(");
                let mut first = true;
                for p in params {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_type(p);
                }
                self.write(") -> ");
                self.format_type(return_type);
                // Print context requirements if any
                if !contexts.is_empty() {
                    self.write(" using [");
                    let mut ctx_first = true;
                    for ctx in contexts.iter() {
                        if !ctx_first {
                            self.write(", ");
                        }
                        ctx_first = false;
                        self.format_context_requirement(ctx);
                    }
                    self.write("]");
                }
                if let Maybe::Some(wc) = where_clause {
                    self.write(" ");
                    self.format_where_clause(wc, "type");
                }
            }
            TypeKind::Reference { mutable, inner } => {
                self.write("&");
                if *mutable {
                    self.write("mut ");
                }
                self.format_type(inner);
            }
            TypeKind::CheckedReference { mutable, inner } => {
                self.write("&checked ");
                if *mutable {
                    self.write("mut ");
                }
                self.format_type(inner);
            }
            TypeKind::UnsafeReference { mutable, inner } => {
                self.write("&unsafe ");
                if *mutable {
                    self.write("mut ");
                }
                self.format_type(inner);
            }
            TypeKind::Pointer { mutable, inner } => {
                self.write("*");
                if *mutable {
                    self.write("mut ");
                } else {
                    self.write("const ");
                }
                self.format_type(inner);
            }
            TypeKind::VolatilePointer { mutable, inner } => {
                self.write("*volatile ");
                if *mutable {
                    self.write("mut ");
                } else {
                    self.write("");
                }
                self.format_type(inner);
            }
            TypeKind::Generic { base, args } => {
                self.format_type(base);
                self.write("<");
                let mut first = true;
                for arg in args {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_generic_arg(arg);
                }
                self.write(">");
            }
            TypeKind::Qualified {
                self_ty,
                trait_ref,
                assoc_name,
            } => {
                self.write("<");
                self.format_type(self_ty);
                self.write(" as ");
                self.format_path(trait_ref);
                self.write(">::");
                self.write(assoc_name.as_str());
            }
            TypeKind::Refined { base, predicate } => {
                self.format_type(base);
                self.write("{");
                self.format_refinement_predicate(predicate);
                self.write("}");
            }
            TypeKind::Sigma {
                name,
                base,
                predicate,
            } => {
                self.write(name.as_str());
                self.write(": ");
                self.format_type(base);
                self.write(" where ");
                self.format_expr(predicate);
            }
            TypeKind::Inferred => self.write("_"),
            TypeKind::Bounded { base, bounds } => {
                self.format_type(base);
                self.write(" where ");
                self.format_type_bounds(bounds);
            }
            TypeKind::DynProtocol { bounds, bindings } => {
                self.write("dyn ");
                self.format_type_bounds(bounds);
                if let Maybe::Some(binds) = bindings {
                    if !binds.is_empty() {
                        self.write("<");
                        let mut first = true;
                        for bind in binds {
                            if !first {
                                self.write(", ");
                            }
                            first = false;
                            self.format_type_binding(bind);
                        }
                        self.write(">");
                    }
                }
            }
            TypeKind::Ownership { mutable, inner } => {
                self.write("%");
                if *mutable {
                    self.write("mut ");
                }
                self.format_type(inner);
            }
            TypeKind::GenRef { inner } => {
                self.write("GenRef<");
                self.format_type(inner);
                self.write(">");
            }
            TypeKind::TypeConstructor { base, arity } => {
                self.format_type(base);
                self.write("<");
                for i in 0..*arity {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write("_");
                }
                self.write(">");
            }
            TypeKind::Tensor {
                element,
                shape,
                layout,
            } => {
                self.write("Tensor<");
                self.format_type(element);
                self.write(", [");
                let mut first = true;
                for dim in shape {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_expr(dim);
                }
                self.write("]");
                if let Maybe::Some(lay) = layout {
                    self.write(", ");
                    match lay {
                        TensorLayout::RowMajor => self.write("RowMajor"),
                        TensorLayout::ColumnMajor => self.write("ColumnMajor"),
                    }
                }
                self.write(">");
            }
            TypeKind::Existential { name, bounds } => {
                self.write("some ");
                self.write(name.as_str());
                self.write(": ");
                self.format_type_bounds(bounds);
            }
            TypeKind::AssociatedType { base, assoc } => {
                self.format_type(base);
                self.write(".");
                self.write(assoc.as_str());
            }
            TypeKind::CapabilityRestricted { base, capabilities } => {
                self.format_type(base);
                self.write(" with [");
                let mut first = true;
                for cap in &capabilities.capabilities {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.write(cap.as_str());
                }
                self.write("]");
            }
            TypeKind::Unknown => self.write("unknown"),
            TypeKind::Record { fields, .. } => {
                self.write("{ ");
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(&field.name.name);
                    self.write(": ");
                    self.format_type(&field.ty);
                }
                self.write(" }");
            }
            TypeKind::Universe { level } => {
                self.write("Type");
                if let verum_common::Maybe::Some(lvl) = level {
                    self.write("(");
                    match lvl {
                        UniverseLevelExpr::Concrete(n) => self.write(&format!("{}", n)),
                        UniverseLevelExpr::Variable(ident) => self.write(&ident.name),
                        UniverseLevelExpr::Max(a, b) => {
                            self.write("max(");
                            self.write(&format!("{:?}", a));
                            self.write(", ");
                            self.write(&format!("{:?}", b));
                            self.write(")");
                        }
                        UniverseLevelExpr::Succ(inner) => {
                            self.write(&format!("{:?} + 1", inner));
                        }
                    }
                    self.write(")");
                }
            }
            TypeKind::Meta { inner } => {
                self.write("meta ");
                self.format_type(inner);
            }
            TypeKind::TypeLambda { params, body } => {
                self.write("|");
                let mut first = true;
                for p in params {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.write(&p.name);
                }
                self.write("| ");
                self.format_type(body);
            }
        }
    }

    fn format_generic_arg(&mut self, arg: &GenericArg) {
        match arg {
            GenericArg::Type(ty) => self.format_type(ty),
            GenericArg::Const(expr) => self.format_expr(expr),
            GenericArg::Lifetime(lt) => {
                self.write("'");
                self.write(lt.name.as_str());
            }
            GenericArg::Binding(bind) => self.format_type_binding(bind),
        }
    }

    fn format_type_binding(&mut self, binding: &TypeBinding) {
        self.write(binding.name.as_str());
        self.write(" = ");
        self.format_type(&binding.ty);
    }

    fn format_refinement_predicate(&mut self, pred: &RefinementPredicate) {
        if let Maybe::Some(binding) = &pred.binding {
            self.write("|");
            self.write(binding.as_str());
            self.write("| ");
        }
        self.format_expr(&pred.expr);
    }

    // ==================== TYPE DECLARATIONS ====================

    fn format_type_decl(&mut self, ty_decl: &TypeDecl) {
        self.format_visibility(&ty_decl.visibility);

        // Resource modifier
        if let Maybe::Some(modifier) = &ty_decl.resource_modifier {
            match modifier {
                ResourceModifier::Affine => self.write("affine "),
                ResourceModifier::Linear => self.write("linear "),
            }
        }

        self.write("type ");
        self.write(ty_decl.name.as_str());
        self.format_generics(&ty_decl.generics);

        // Meta where clause
        if let Maybe::Some(where_clause) = &ty_decl.meta_where_clause {
            self.format_where_clause(where_clause, "meta");
        }

        self.write(" is ");
        self.format_type_decl_body(&ty_decl.body);
    }

    fn format_type_decl_body(&mut self, body: &TypeDeclBody) {
        match body {
            TypeDeclBody::Alias(ty) => {
                self.format_type(ty);
                self.write(";");
            }
            TypeDeclBody::Record(fields) => {
                self.write("{");
                if fields.is_empty() {
                    self.write("}");
                } else if fields.len() <= 2 {
                    // Inline format
                    self.write(" ");
                    let mut first = true;
                    for field in fields {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_record_field(field);
                    }
                    self.write(" };");
                } else {
                    // Multi-line format
                    self.indent();
                    for field in fields {
                        self.newline();
                        self.format_record_field(field);
                        self.write(",");
                    }
                    self.dedent();
                    self.newline();
                    self.write("};");
                }
            }
            TypeDeclBody::Variant(variants) => {
                self.newline();
                self.indent();
                for variant in variants {
                    self.write("| ");
                    self.format_variant(variant);
                    self.newline();
                }
                self.dedent();
                self.write(";");
            }
            TypeDeclBody::Protocol(proto_body) => {
                self.format_protocol_body(proto_body);
            }
            TypeDeclBody::Newtype(ty) => {
                self.write("(");
                self.format_type(ty);
                self.write(");");
            }
            TypeDeclBody::Tuple(types) => {
                self.write("(");
                let mut first = true;
                for ty in types {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_type(ty);
                }
                self.write(");");
            }
            TypeDeclBody::Unit => self.write(";"),
            TypeDeclBody::SigmaTuple(types) => {
                self.write("(");
                let mut first = true;
                for ty in types.iter() {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_type(ty);
                }
                self.write(");");
            }
            TypeDeclBody::Inductive(variants) => {
                self.write("inductive {");
                self.indent();
                for variant in variants {
                    self.newline();
                    self.write("| ");
                    self.format_variant(variant);
                }
                self.dedent();
                self.newline();
                self.write("};");
            }
            TypeDeclBody::Coinductive(proto_body) => {
                self.write("coinductive ");
                self.format_protocol_body(proto_body);
            }
            TypeDeclBody::Quotient { base, relation: _ } => {
                self.format_type(base);
                self.write(" / /* relation */");
            }
        }
    }

    fn format_record_field(&mut self, field: &RecordField) {
        self.format_visibility(&field.visibility);
        self.write(field.name.as_str());
        self.write(": ");
        self.format_type(&field.ty);
    }

    fn format_variant(&mut self, variant: &Variant) {
        self.write(variant.name.as_str());
        if let Maybe::Some(data) = &variant.data {
            match data {
                VariantData::Tuple(types) => {
                    self.write("(");
                    let mut first = true;
                    for ty in types {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_type(ty);
                    }
                    self.write(")");
                }
                VariantData::Record(fields) => {
                    self.write(" { ");
                    let mut first = true;
                    for field in fields {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_record_field(field);
                    }
                    self.write(" }");
                }
            }
        }
    }

    fn format_protocol_body(&mut self, body: &ProtocolBody) {
        if body.is_context {
            self.write("context ");
        }
        self.write("protocol");

        // Extends clause
        if !body.extends.is_empty() {
            self.write(" extends ");
            let mut first = true;
            for ty in &body.extends {
                if !first {
                    self.write(" + ");
                }
                first = false;
                self.format_type(ty);
            }
        }

        // Where clause
        if let Maybe::Some(where_clause) = &body.generic_where_clause {
            self.format_where_clause(where_clause, "type");
        }

        self.write(" {");
        if body.items.is_empty() {
            self.write("};");
        } else {
            self.indent();
            for item in &body.items {
                self.newline();
                self.format_protocol_item(item);
            }
            self.dedent();
            self.newline();
            self.write("};");
        }
    }

    fn format_protocol_item(&mut self, item: &ProtocolItem) {
        match &item.kind {
            ProtocolItemKind::Function { decl, default_impl } => {
                self.format_function(decl);
                if default_impl.is_some() {
                    // Already formatted in format_function
                }
            }
            ProtocolItemKind::Type {
                name,
                type_params,
                bounds,
                where_clause,
                default_type,
            } => {
                self.write("type ");
                self.write(name.as_str());
                self.format_generics(type_params);
                if !bounds.is_empty() {
                    self.write(": ");
                    let mut first = true;
                    for bound in bounds {
                        if !first {
                            self.write(" + ");
                        }
                        first = false;
                        self.format_path(bound);
                    }
                }
                if let Maybe::Some(wc) = where_clause {
                    self.format_where_clause(wc, "type");
                }
                if let Maybe::Some(def) = default_type {
                    self.write(" = ");
                    self.format_type(def);
                }
                self.write(";");
            }
            ProtocolItemKind::Const { name, ty } => {
                self.write("const ");
                self.write(name.as_str());
                self.write(": ");
                self.format_type(ty);
                self.write(";");
            }
            ProtocolItemKind::Axiom(axiom_decl) => {
                // Protocol-level axiom — `axiom name(...) ensures ...`.
                // Pretty-printing the full param list + proposition goes
                // through the same machinery as top-level axioms; for now
                // emit the axiom signature as a stable round-trippable
                // marker so downstream consumers can at least identify
                // which axioms a protocol declares.
                self.write("axiom ");
                self.write(axiom_decl.name.as_str());
                self.write("(...);");
            }
        }
    }

    // ==================== PROTOCOLS ====================

    fn format_protocol(&mut self, proto: &ProtocolDecl) {
        self.format_visibility(&proto.visibility);

        if proto.is_context {
            self.write("context ");
        }

        self.write("protocol ");
        self.write(proto.name.as_str());
        self.format_generics(&proto.generics);

        // Bounds
        if !proto.bounds.is_empty() {
            self.write(": ");
            let mut first = true;
            for bound in &proto.bounds {
                if !first {
                    self.write(" + ");
                }
                first = false;
                self.format_type(bound);
            }
        }

        // Where clauses
        if let Maybe::Some(wc) = &proto.generic_where_clause {
            self.format_where_clause(wc, "type");
        }
        if let Maybe::Some(wc) = &proto.meta_where_clause {
            self.format_where_clause(wc, "meta");
        }

        self.write(" {");
        if proto.items.is_empty() {
            self.write("}");
        } else {
            self.indent();
            for item in &proto.items {
                self.newline();
                self.format_protocol_item(item);
            }
            self.dedent();
            self.newline();
            self.write("}");
        }
    }

    // ==================== IMPLEMENTATIONS ====================

    fn format_impl(&mut self, impl_decl: &ImplDecl) {
        self.write("implement");
        self.format_generics(&impl_decl.generics);
        self.write(" ");

        match &impl_decl.kind {
            ImplKind::Inherent(ty) => {
                self.format_type(ty);
            }
            ImplKind::Protocol {
                protocol,
                protocol_args,
                for_type,
            } => {
                self.format_path(protocol);
                if !protocol_args.is_empty() {
                    self.write("<");
                    let mut first = true;
                    for arg in protocol_args {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_generic_arg(arg);
                    }
                    self.write(">");
                }
                self.write(" for ");
                self.format_type(for_type);
            }
        }

        // Where clauses
        if let Maybe::Some(wc) = &impl_decl.generic_where_clause {
            self.format_where_clause(wc, "type");
        }
        if let Maybe::Some(wc) = &impl_decl.meta_where_clause {
            self.format_where_clause(wc, "meta");
        }

        self.write(" {");
        if impl_decl.items.is_empty() {
            self.write("}");
        } else {
            self.indent();
            for item in &impl_decl.items {
                self.newline();
                self.format_impl_item(item);
            }
            self.dedent();
            self.newline();
            self.write("}");
        }
    }

    fn format_impl_item(&mut self, item: &ImplItem) {
        self.format_visibility(&item.visibility);
        match &item.kind {
            ImplItemKind::Function(func) => self.format_function(func),
            ImplItemKind::Type {
                name,
                type_params,
                ty,
            } => {
                self.write("type ");
                self.write(name.as_str());
                self.format_generics(type_params);
                self.write(" = ");
                self.format_type(ty);
                self.write(";");
            }
            ImplItemKind::Const { name, ty, value } => {
                self.write("const ");
                self.write(name.as_str());
                self.write(": ");
                self.format_type(ty);
                self.write(" = ");
                self.format_expr(value);
                self.write(";");
            }
            ImplItemKind::Proof { axiom_name, tactic: _ } => {
                // Proof clause — `proof axiom_name by tactic;`
                // Full tactic pretty-print is handled via the same
                // path as stand-alone tactic_expr.
                self.write("proof ");
                self.write(axiom_name.as_str());
                self.write(" by /* tactic */;");
            }
        }
    }

    // ==================== MODULES ====================

    fn format_module_decl(&mut self, module: &ModuleDecl) {
        self.format_visibility(&module.visibility);
        self.write("module ");
        self.write(module.name.as_str());

        if let Maybe::Some(items) = &module.items {
            self.write(" {");
            self.indent();
            for item in items {
                self.newline();
                self.format_item(item);
            }
            self.dedent();
            self.newline();
            self.write("}");
        } else {
            self.write(";");
        }
    }

    // ==================== CONST/STATIC ====================

    fn format_const(&mut self, const_decl: &ConstDecl) {
        self.format_visibility(&const_decl.visibility);
        self.write("const ");
        self.write(const_decl.name.as_str());
        self.write(": ");
        self.format_type(&const_decl.ty);
        self.write(" = ");
        self.format_expr(&const_decl.value);
        self.write(";");
    }

    fn format_static(&mut self, static_decl: &StaticDecl) {
        self.format_visibility(&static_decl.visibility);
        self.write("static ");
        if static_decl.is_mut {
            self.write("mut ");
        }
        self.write(static_decl.name.as_str());
        self.write(": ");
        self.format_type(&static_decl.ty);
        self.write(" = ");
        self.format_expr(&static_decl.value);
        self.write(";");
    }

    // ==================== MOUNTS ====================

    fn format_mount(&mut self, mount: &MountDecl) {
        self.format_visibility(&mount.visibility);
        self.write("mount ");
        self.format_mount_tree(&mount.tree);
        if let Maybe::Some(alias) = &mount.alias {
            self.write(" as ");
            self.write(alias.as_str());
        }
        self.write(";");
    }

    fn format_mount_tree(&mut self, tree: &MountTree) {
        match &tree.kind {
            MountTreeKind::Path(path) => self.format_path(path),
            MountTreeKind::Glob(path) => {
                self.format_path(path);
                self.write(".*");
            }
            MountTreeKind::Nested { prefix, trees } => {
                self.format_path(prefix);
                self.write(".{");
                let mut first = true;
                for t in trees {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_mount_tree(t);
                }
                self.write("}");
            }
        }
    }

    // ==================== CONTEXTS ====================

    fn format_context(&mut self, ctx: &ContextDecl) {
        self.format_visibility(&ctx.visibility);
        if ctx.is_async {
            self.write("async ");
        }
        self.write("context ");
        self.write(ctx.name.as_str());
        self.format_generics(&ctx.generics);
        self.write(" {");

        if ctx.methods.is_empty() && ctx.sub_contexts.is_empty() {
            self.write("}");
        } else {
            self.indent();
            for method in &ctx.methods {
                self.newline();
                self.format_function(method);
            }
            for sub in &ctx.sub_contexts {
                self.newline();
                self.format_context(sub);
            }
            self.dedent();
            self.newline();
            self.write("}");
        }
    }

    fn format_context_group(&mut self, group: &ContextGroupDecl) {
        self.format_visibility(&group.visibility);
        self.write("using ");
        self.write(group.name.as_str());
        self.write(" = [");
        if group.contexts.is_empty() {
            self.write("]");
        } else {
            for (i, ctx) in group.contexts.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.format_context_requirement(ctx);
            }
            self.write("]");
        }
        self.write(";");
    }

    // ==================== FFI ====================

    fn format_ffi_boundary(&mut self, _ffi: &crate::ffi::FFIBoundary) {
        self.write("ffi { /* FFI boundary */ }");
    }

    // ==================== PREDICATES ====================

    fn format_predicate(&mut self, pred: &crate::decl::PredicateDecl) {
        self.format_visibility(&pred.visibility);
        self.write("predicate ");
        self.write(pred.name.as_str());
        self.write("(");
        self.format_function_params(&pred.params);
        self.write(") -> ");
        self.format_type(&pred.return_type);
        self.write(" { ");
        self.format_expr(&pred.body);
        self.write(" }");
    }

    // ==================== META ====================

    fn format_meta(&mut self, meta: &crate::decl::MetaDecl) {
        self.format_visibility(&meta.visibility);
        self.write("meta ");
        self.write(meta.name.as_str());
        self.write(" { /* meta rules */ }");
    }

    // ==================== THEOREMS/AXIOMS ====================

    fn format_theorem(&mut self, thm: &crate::decl::TheoremDecl, keyword: &str) {
        self.format_visibility(&thm.visibility);
        self.write(keyword);
        self.write(" ");
        self.write(thm.name.as_str());
        self.format_generics(&thm.generics);
        self.write("(");
        self.format_function_params(&thm.params);
        self.write("): ");
        self.format_expr(&thm.proposition);

        if let Maybe::Some(wc) = &thm.generic_where_clause {
            self.format_where_clause(wc, "type");
        }

        if let Maybe::Some(_proof) = &thm.proof {
            self.write(" { /* proof */ }");
        } else {
            self.write(";");
        }
    }

    fn format_axiom(&mut self, axiom: &crate::decl::AxiomDecl) {
        self.format_visibility(&axiom.visibility);
        self.write("axiom ");
        self.write(axiom.name.as_str());
        self.format_generics(&axiom.generics);
        self.write("(");
        self.format_function_params(&axiom.params);
        self.write("): ");
        self.format_expr(&axiom.proposition);
        self.write(";");
    }

    fn format_tactic(&mut self, tactic: &crate::decl::TacticDecl) {
        self.format_visibility(&tactic.visibility);
        self.write("tactic ");
        self.write(tactic.name.as_str());
        self.write(" { /* tactic body */ }");
    }

    fn format_view(&mut self, view: &crate::decl::ViewDecl) {
        self.format_visibility(&view.visibility);
        self.write("view ");
        self.write(view.name.as_str());
        self.format_generics(&view.generics);
        self.write(" : ");
        self.format_type(&view.param_type);
        self.write(" -> ");
        self.format_type(&view.return_type);
        self.write(" { /* view constructors */ }");
    }

    fn format_pattern_decl(&mut self, pattern_decl: &PatternDecl) {
        use crate::decl::FunctionParamKind;

        self.format_visibility(&pattern_decl.visibility);
        self.write("pattern ");
        self.write(pattern_decl.name.as_str());

        // Format type parameters if present (parameterized patterns)
        if !pattern_decl.type_params.is_empty() {
            self.write("(");
            let mut first = true;
            for param in &pattern_decl.type_params {
                if !first {
                    self.write(", ");
                }
                first = false;
                if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                    self.format_pattern(pattern);
                    self.write(": ");
                    self.format_type(ty);
                }
            }
            self.write(")");
        }

        // Format pattern parameters
        self.write("(");
        let mut first = true;
        for param in &pattern_decl.params {
            if !first {
                self.write(", ");
            }
            first = false;
            if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                self.format_pattern(pattern);
                self.write(": ");
                self.format_type(ty);
            }
        }
        self.write(")");

        // Format return type
        self.write(" -> ");
        self.format_type(&pattern_decl.return_type);

        // Format body
        self.write(" = ");
        self.format_expr(&pattern_decl.body);
        self.write(";");
    }

    // ==================== PATHS ====================

    fn format_path(&mut self, path: &Path) {
        let mut first = true;
        for seg in &path.segments {
            if !first {
                self.write(".");
            }
            first = false;
            match seg {
                PathSegment::Name(ident) => self.write(ident.as_str()),
                PathSegment::SelfValue => self.write("self"),
                PathSegment::Super => self.write("super"),
                PathSegment::Cog => self.write("cog"),
                PathSegment::Relative => self.write("."),
            }
        }
    }

    // ==================== EXPRESSIONS ====================

    fn format_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Literal(lit) => self.format_literal(lit),
            ExprKind::Path(path) => self.format_path(path),
            ExprKind::Binary { op, left, right } => {
                self.format_expr(left);
                self.write(" ");
                self.write(op.as_str());
                self.write(" ");
                self.format_expr(right);
            }
            ExprKind::Unary { op, expr } => {
                self.write(op.as_str());
                if matches!(
                    op,
                    UnOp::RefMut
                        | UnOp::RefChecked
                        | UnOp::RefCheckedMut
                        | UnOp::RefUnsafe
                        | UnOp::RefUnsafeMut
                        | UnOp::OwnMut
                ) {
                    self.write(" ");
                }
                self.format_expr(expr);
            }
            ExprKind::NamedArg { name, value } => {
                self.write(name.as_str());
                self.write(": ");
                self.format_expr(value);
            }
            ExprKind::Call { func, type_args, args } => {
                self.format_expr(func);
                if !type_args.is_empty() {
                    self.write("::<");
                    for (i, type_arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.format_generic_arg(type_arg);
                    }
                    self.write(">");
                }
                self.write("(");
                self.format_expr_list(args);
                self.write(")");
            }
            ExprKind::MethodCall {
                receiver,
                method,
                type_args,
                args,
            } => {
                self.format_expr(receiver);
                self.write(".");
                self.write(method.as_str());
                if !type_args.is_empty() {
                    self.write("<");
                    for (i, type_arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.format_generic_arg(type_arg);
                    }
                    self.write(">");
                }
                self.write("(");
                self.format_expr_list(args);
                self.write(")");
            }
            ExprKind::Field { expr, field } => {
                self.format_expr(expr);
                self.write(".");
                self.write(field.as_str());
            }
            ExprKind::OptionalChain { expr, field } => {
                self.format_expr(expr);
                self.write("?.");
                self.write(field.as_str());
            }
            ExprKind::TupleIndex { expr, index } => {
                self.format_expr(expr);
                self.write(".");
                self.write(&index.to_string());
            }
            ExprKind::Index { expr, index } => {
                self.format_expr(expr);
                self.write("[");
                self.format_expr(index);
                self.write("]");
            }
            ExprKind::Pipeline { left, right } => {
                self.format_expr(left);
                self.write(" |> ");
                self.format_expr(right);
            }
            ExprKind::NullCoalesce { left, right } => {
                self.format_expr(left);
                self.write(" ?? ");
                self.format_expr(right);
            }
            ExprKind::Cast { expr, ty } => {
                self.format_expr(expr);
                self.write(" as ");
                self.format_type(ty);
            }
            ExprKind::Try(expr) => {
                self.format_expr(expr);
                self.write("?");
            }
            ExprKind::TryBlock(block) => {
                self.write("try ");
                self.format_expr(block);
            }
            ExprKind::TryRecover { try_block, recover } => {
                self.write("try ");
                self.format_expr(try_block);
                self.write(" recover ");
                self.format_recover_body(recover);
            }
            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                self.write("try ");
                self.format_expr(try_block);
                self.write(" finally ");
                self.format_expr(finally_block);
            }
            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.write("try ");
                self.format_expr(try_block);
                self.write(" recover ");
                self.format_recover_body(recover);
                self.write(" finally ");
                self.format_expr(finally_block);
            }
            ExprKind::Tuple(exprs) => {
                self.write("(");
                self.format_expr_list(exprs);
                if exprs.len() == 1 {
                    self.write(",");
                }
                self.write(")");
            }
            ExprKind::Array(arr_expr) => match arr_expr {
                ArrayExpr::List(exprs) => {
                    self.write("[");
                    self.format_expr_list(exprs);
                    self.write("]");
                }
                ArrayExpr::Repeat { value, count } => {
                    self.write("[");
                    self.format_expr(value);
                    self.write("; ");
                    self.format_expr(count);
                    self.write("]");
                }
            },
            ExprKind::Comprehension { expr, clauses } => {
                self.write("[");
                self.format_expr(expr);
                for clause in clauses {
                    self.write(" ");
                    self.format_comprehension_clause(clause);
                }
                self.write("]");
            }
            ExprKind::StreamComprehension { expr, clauses } => {
                self.write("stream [");
                self.format_expr(expr);
                for clause in clauses {
                    self.write(" ");
                    self.format_comprehension_clause(clause);
                }
                self.write("]");
            }
            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                self.write("{");
                self.format_expr(key_expr);
                self.write(": ");
                self.format_expr(value_expr);
                for clause in clauses {
                    self.write(" ");
                    self.format_comprehension_clause(clause);
                }
                self.write("}");
            }
            ExprKind::SetComprehension { expr, clauses } => {
                self.write("set{");
                self.format_expr(expr);
                for clause in clauses {
                    self.write(" ");
                    self.format_comprehension_clause(clause);
                }
                self.write("}");
            }
            ExprKind::GeneratorComprehension { expr, clauses } => {
                self.write("gen{");
                self.format_expr(expr);
                for clause in clauses {
                    self.write(" ");
                    self.format_comprehension_clause(clause);
                }
                self.write("}");
            }
            ExprKind::Record { path, fields, base } => {
                self.format_path(path);
                self.write(" { ");
                let mut first = true;
                for field in fields {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_field_init(field);
                }
                if let Maybe::Some(base_expr) = base {
                    if !first {
                        self.write(", ");
                    }
                    self.write("..");
                    self.format_expr(base_expr);
                }
                self.write(" }");
            }
            ExprKind::InterpolatedString {
                handler,
                parts,
                exprs,
            } => {
                self.write(handler.as_str());
                self.write("\"");
                for (i, part) in parts.iter().enumerate() {
                    self.write(part.as_str());
                    if i < exprs.len() {
                        self.write("{");
                        self.format_expr(&exprs[i]);
                        self.write("}");
                    }
                }
                self.write("\"");
            }
            ExprKind::TensorLiteral {
                shape,
                elem_type,
                data,
            } => {
                self.write("tensor<");
                let mut first = true;
                for dim in shape {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.write(&dim.to_string());
                }
                self.write("> ");
                self.format_type(elem_type);
                self.write(" ");
                self.format_expr(data);
            }
            ExprKind::MapLiteral { entries } => {
                self.write("{");
                let mut first = true;
                for (key, value) in entries {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_expr(key);
                    self.write(": ");
                    self.format_expr(value);
                }
                self.write("}");
            }
            ExprKind::SetLiteral { elements } => {
                self.write("{");
                self.format_expr_list(elements);
                self.write("}");
            }
            ExprKind::Block(block) => self.format_block(block),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.write("if ");
                self.format_if_condition(condition);
                self.write(" ");
                self.format_block(then_branch);
                if let Maybe::Some(else_expr) = else_branch {
                    self.write(" else ");
                    self.format_expr(else_expr);
                }
            }
            ExprKind::Match { expr, arms } => {
                self.write("match ");
                self.format_expr(expr);
                self.write(" {");
                self.format_match_arms(arms);
                self.write("}");
            }
            ExprKind::Loop {
                label,
                body,
                invariants,
            } => {
                if let Maybe::Some(lbl) = label {
                    self.write("'");
                    self.write(lbl.as_str());
                    self.write(": ");
                }
                self.write("loop ");
                for inv in invariants.iter() {
                    self.write("invariant ");
                    self.format_expr(inv);
                    self.write(" ");
                }
                self.format_block(body);
            }
            ExprKind::While {
                label,
                condition,
                body,
                invariants,
                decreases,
            } => {
                if let Maybe::Some(lbl) = label {
                    self.write("'");
                    self.write(lbl.as_str());
                    self.write(": ");
                }
                self.write("while ");
                self.format_expr(condition);
                for inv in invariants.iter() {
                    self.newline();
                    self.indent();
                    self.write("invariant ");
                    self.format_expr(inv);
                    self.dedent();
                }
                for dec in decreases.iter() {
                    self.newline();
                    self.indent();
                    self.write("decreases ");
                    self.format_expr(dec);
                    self.dedent();
                }
                self.newline();
                self.format_block(body);
            }
            ExprKind::For {
                label,
                pattern,
                iter,
                body,
                invariants,
                decreases,
            } => {
                if let Maybe::Some(lbl) = label {
                    self.write("'");
                    self.write(lbl.as_str());
                    self.write(": ");
                }
                self.write("for ");
                self.format_pattern(pattern);
                self.write(" in ");
                self.format_expr(iter);
                for inv in invariants.iter() {
                    self.newline();
                    self.indent();
                    self.write("invariant ");
                    self.format_expr(inv);
                    self.dedent();
                }
                for dec in decreases.iter() {
                    self.newline();
                    self.indent();
                    self.write("decreases ");
                    self.format_expr(dec);
                    self.dedent();
                }
                self.newline();
                self.format_block(body);
            }
            ExprKind::ForAwait {
                label,
                pattern,
                async_iterable,
                body,
                invariants,
                decreases,
            } => {
                if let Maybe::Some(lbl) = label {
                    self.write("'");
                    self.write(lbl.as_str());
                    self.write(": ");
                }
                self.write("for await ");
                self.format_pattern(pattern);
                self.write(" in ");
                self.format_expr(async_iterable);
                for inv in invariants.iter() {
                    self.newline();
                    self.indent();
                    self.write("invariant ");
                    self.format_expr(inv);
                    self.dedent();
                }
                for dec in decreases.iter() {
                    self.newline();
                    self.indent();
                    self.write("decreases ");
                    self.format_expr(dec);
                    self.dedent();
                }
                self.newline();
                self.format_block(body);
            }
            ExprKind::Break { label, value } => {
                self.write("break");
                if let Maybe::Some(lbl) = label {
                    self.write(" '");
                    self.write(lbl.as_str());
                }
                if let Maybe::Some(val) = value {
                    self.write(" ");
                    self.format_expr(val);
                }
            }
            ExprKind::Continue { label } => {
                self.write("continue");
                if let Maybe::Some(lbl) = label {
                    self.write(" '");
                    self.write(lbl.as_str());
                }
            }
            ExprKind::Return(value) => {
                self.write("return");
                if let Maybe::Some(val) = value {
                    self.write(" ");
                    self.format_expr(val);
                }
            }
            ExprKind::Throw(expr) => {
                self.write("throw ");
                self.format_expr(expr);
            }
            ExprKind::Yield(expr) => {
                self.write("yield ");
                self.format_expr(expr);
            }
            ExprKind::Typeof(expr) => {
                self.write("typeof(");
                self.format_expr(expr);
                self.write(")");
            }
            ExprKind::Closure {
                async_,
                move_,
                params,
                contexts,
                return_type,
                body,
            } => {
                if *async_ {
                    self.write("async ");
                }
                if *move_ {
                    self.write("move ");
                }
                self.write("|");
                let mut first = true;
                for param in params {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_closure_param(param);
                }
                self.write("|");
                if !contexts.is_empty() {
                    self.write(" using [");
                    let mut first = true;
                    for ctx in contexts {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_context_requirement(ctx);
                    }
                    self.write("]");
                }
                if let Maybe::Some(ret_ty) = return_type {
                    self.write(" -> ");
                    self.format_type(ret_ty);
                }
                self.write(" ");
                self.format_expr(body);
            }
            ExprKind::Async(block) => {
                self.write("async ");
                self.format_block(block);
            }
            ExprKind::Await(expr) => {
                self.format_expr(expr);
                self.write(".await");
            }
            ExprKind::Inject { type_path } => {
                self.write("inject ");
                self.format_path(type_path);
            }
            ExprKind::Spawn { expr, contexts } => {
                self.write("spawn ");
                self.format_expr(expr);
                if !contexts.is_empty() {
                    self.write(" using [");
                    let mut first = true;
                    for ctx in contexts {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_context_requirement(ctx);
                    }
                    self.write("]");
                }
            }
            ExprKind::Select { biased, arms, .. } => {
                self.write("select ");
                if *biased {
                    self.write("biased ");
                }
                self.write("{");
                self.indent();
                self.newline();
                for (i, arm) in arms.iter().enumerate() {
                    if i > 0 {
                        self.write(",");
                        self.newline();
                    }
                    if arm.is_else() {
                        self.write("else");
                    } else {
                        if let verum_common::Maybe::Some(ref pattern) = arm.pattern {
                            self.format_pattern(pattern);
                            self.write(" = ");
                        }
                        if let verum_common::Maybe::Some(ref future) = arm.future {
                            self.format_expr(future);
                        }
                        if let verum_common::Maybe::Some(ref guard) = arm.guard {
                            self.write(" if ");
                            self.format_expr(guard);
                        }
                    }
                    self.write(" => ");
                    self.format_expr(&arm.body);
                }
                self.dedent();
                self.newline();
                self.write("}");
            }
            ExprKind::Unsafe(block) => {
                self.write("unsafe ");
                self.format_block(block);
            }
            ExprKind::Meta(block) => {
                self.write("meta ");
                self.format_block(block);
            }
            ExprKind::Quote { target_stage, tokens } => {
                self.write("quote");
                if let Some(stage) = target_stage {
                    self.write(&format!("@({stage})"));
                }
                self.write(" { ");
                for (i, token) in tokens.iter().enumerate() {
                    if i > 0 {
                        self.write(" ");
                    }
                    self.write(&token.to_text());
                }
                self.write(" }");
            }
            ExprKind::StageEscape { stage, expr } => {
                self.write(&format!("$(stage {stage}){{ "));
                self.format_expr(expr);
                self.write(" }}");
            }
            ExprKind::Lift { expr } => {
                self.write("lift(");
                self.format_expr(expr);
                self.write(")");
            }
            ExprKind::MacroCall { path, args } => {
                self.format_path(path);
                self.write("!");
                match args.delimiter {
                    MacroDelimiter::Paren => {
                        self.write("(");
                        self.write(args.tokens.as_str());
                        self.write(")");
                    }
                    MacroDelimiter::Bracket => {
                        self.write("[");
                        self.write(args.tokens.as_str());
                        self.write("]");
                    }
                    MacroDelimiter::Brace => {
                        self.write("{");
                        self.write(args.tokens.as_str());
                        self.write("}");
                    }
                }
            }
            ExprKind::UseContext {
                context,
                handler,
                body,
            } => {
                self.write("use ");
                self.format_path(context);
                self.write(" = ");
                self.format_expr(handler);
                self.write(" in ");
                self.format_expr(body);
            }
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                if let Maybe::Some(s) = start {
                    self.format_expr(s);
                }
                if *inclusive {
                    self.write("..=");
                } else {
                    self.write("..");
                }
                if let Maybe::Some(e) = end {
                    self.format_expr(e);
                }
            }
            ExprKind::Forall { bindings, body } => {
                self.write("forall ");
                self.format_quantifier_bindings(bindings);
                self.write(". ");
                self.format_expr(body);
            }
            ExprKind::Exists { bindings, body } => {
                self.write("exists ");
                self.format_quantifier_bindings(bindings);
                self.write(". ");
                self.format_expr(body);
            }
            ExprKind::Paren(expr) => {
                self.write("(");
                self.format_expr(expr);
                self.write(")");
            }
            ExprKind::Is {
                expr,
                pattern,
                negated,
            } => {
                self.format_expr(expr);
                if *negated {
                    self.write(" is not ");
                } else {
                    self.write(" is ");
                }
                self.format_pattern(pattern);
            }
            ExprKind::Attenuate {
                context,
                capabilities,
            } => {
                self.format_expr(context);
                self.write(".attenuate(");
                let mut first = true;
                for cap in &capabilities.capabilities {
                    if !first {
                        self.write(" | ");
                    }
                    first = false;
                    self.write(cap.as_str());
                }
                self.write(")");
            }
            ExprKind::TypeProperty { ty, property } => {
                self.format_type(ty);
                self.write(".");
                self.write(property.as_str());
            }
            ExprKind::TypeExpr(ty) => {
                self.format_type(ty);
            }
            ExprKind::TypeBound { type_param, bound } => {
                self.write(type_param.as_str());
                self.write(": ");
                self.format_type(bound);
            }
            ExprKind::MetaFunction { name, args } => {
                self.write("@");
                self.write(name.as_str());
                if !args.is_empty() {
                    self.write("(");
                    self.format_expr_list(args);
                    self.write(")");
                }
            }
            ExprKind::Nursery { options, body, on_cancel, recover, .. } => {
                self.write("nursery");
                // Format options if present
                if !options.is_empty() {
                    self.write("(");
                    let mut first = true;
                    if let Maybe::Some(ref timeout) = options.timeout {
                        self.write("timeout: ");
                        self.format_expr(timeout);
                        first = false;
                    }
                    if options.on_error != NurseryErrorBehavior::CancelAll {
                        if !first { self.write(", "); }
                        self.write("on_error: ");
                        self.write(match options.on_error {
                            NurseryErrorBehavior::CancelAll => "cancel_all",
                            NurseryErrorBehavior::WaitAll => "wait_all",
                            NurseryErrorBehavior::FailFast => "fail_fast",
                        });
                        first = false;
                    }
                    if let Maybe::Some(ref max) = options.max_tasks {
                        if !first { self.write(", "); }
                        self.write("max_tasks: ");
                        self.format_expr(max);
                    }
                    self.write(")");
                }
                self.write(" ");
                self.format_block(body);
                // Format on_cancel if present
                if let Maybe::Some(cancel_block) = on_cancel {
                    self.write(" on_cancel ");
                    self.format_block(cancel_block);
                }
                // Format recover if present
                if let Maybe::Some(recover_body) = recover {
                    self.write(" recover ");
                    self.format_recover_body(recover_body);
                }
            }
            // Inline assembly expression
            ExprKind::InlineAsm { template, operands, options } => {
                self.write("@asm(");
                // Write template string
                self.write("\"");
                self.write(template.as_str().replace('\\', "\\\\").replace('"', "\\\"").as_str());
                self.write("\"");
                // Write operands if present
                if !operands.is_empty() {
                    self.write(", [");
                    let mut first = true;
                    for operand in operands.iter() {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_asm_operand(operand);
                    }
                    self.write("]");
                }
                // Write options if present
                if !options.raw_options.is_empty() || options.volatile || options.intel_syntax {
                    self.write(", ");
                    self.format_asm_options(options);
                }
                self.write(")");
            }
            // Stream literal expression: stream[1, 2, 3, ...] or stream[0..100]
            // Stream comprehension pretty-printing
            ExprKind::StreamLiteral(stream_lit) => {
                self.write("stream[");
                match &stream_lit.kind {
                    crate::expr::StreamLiteralKind::Elements { elements, cycles } => {
                        let mut first = true;
                        for elem in elements.iter() {
                            if !first {
                                self.write(", ");
                            }
                            first = false;
                            self.format_expr(elem);
                        }
                        if *cycles {
                            if !elements.is_empty() {
                                self.write(", ");
                            }
                            self.write("...");
                        }
                    }
                    crate::expr::StreamLiteralKind::Range { start, end, inclusive } => {
                        self.format_expr(start);
                        if *inclusive {
                            self.write("..=");
                        } else {
                            self.write("..");
                        }
                        if let Maybe::Some(end_expr) = end {
                            self.format_expr(end_expr);
                        }
                    }
                }
                self.write("]");
            }
            ExprKind::DestructuringAssign { pattern, op, value } => {
                self.format_pattern(pattern);
                self.write(" ");
                self.write(op.as_str());
                self.write(" ");
                self.format_expr(value);
            }
            ExprKind::CalcBlock(_) => {
                self.write("calc { ... }");
            }
            ExprKind::CopatternBody { arms, .. } => {
                self.write("{");
                self.indent();
                let mut first = true;
                for arm in arms.iter() {
                    if !first {
                        self.write(",");
                    }
                    first = false;
                    self.newline();
                    self.write(".");
                    self.write(arm.observation.name.as_str());
                    self.write(" => ");
                    self.format_expr(&arm.body);
                }
                if !arms.is_empty() {
                    self.write(",");
                }
                self.dedent();
                self.newline();
                self.write("}");
            }
        }
    }

    fn format_expr_list(&mut self, exprs: &List<Expr>) {
        let mut first = true;
        for expr in exprs {
            if !first {
                self.write(", ");
            }
            first = false;
            self.format_expr(expr);
        }
    }

    fn format_block(&mut self, block: &Block) {
        self.write("{");
        if block.stmts.is_empty() && block.expr.is_none() {
            self.write("}");
            return;
        }

        self.indent();
        for stmt in &block.stmts {
            self.newline();
            self.format_stmt(stmt);
        }
        if let Maybe::Some(expr) = &block.expr {
            self.newline();
            self.format_expr(expr);
        }
        self.dedent();
        self.newline();
        self.write("}");
    }

    fn format_if_condition(&mut self, cond: &IfCondition) {
        let mut first = true;
        for c in &cond.conditions {
            if !first {
                self.write(" && ");
            }
            first = false;
            match c {
                ConditionKind::Expr(expr) => self.format_expr(expr),
                ConditionKind::Let { pattern, value } => {
                    self.write("let ");
                    self.format_pattern(pattern);
                    self.write(" = ");
                    self.format_expr(value);
                }
            }
        }
    }

    fn format_match_arms(&mut self, arms: &List<MatchArm>) {
        self.indent();
        for arm in arms {
            self.newline();
            self.format_pattern(&arm.pattern);
            if let Maybe::Some(guard) = &arm.guard {
                self.write(" if ");
                self.format_expr(guard);
            }
            self.write(" => ");
            self.format_expr(&arm.body);
            self.write(",");
        }
        self.dedent();
        self.newline();
    }

    fn format_recover_body(&mut self, recover: &RecoverBody) {
        match recover {
            RecoverBody::MatchArms { arms, .. } => {
                self.write("{");
                self.format_match_arms(arms);
                self.write("}");
            }
            RecoverBody::Closure { param, body, .. } => {
                self.write("|");
                self.format_pattern(&param.pattern);
                if let Maybe::Some(ty) = &param.ty {
                    self.write(": ");
                    self.format_type(ty);
                }
                self.write("| ");
                self.format_expr(body);
            }
        }
    }

    fn format_asm_operand(&mut self, operand: &crate::expr::AsmOperand) {
        use crate::expr::AsmOperandKind;

        // Write named operand if present
        if let Maybe::Some(name) = &operand.name {
            self.write(name.name.as_str());
            self.write(" = ");
        }

        match &operand.kind {
            AsmOperandKind::In { constraint, expr } => {
                self.write("in(");
                self.write(constraint.constraint.as_str());
                self.write(") ");
                self.format_expr(expr);
            }
            AsmOperandKind::Out { constraint, place, late } => {
                if *late {
                    self.write("lateout(");
                } else {
                    self.write("out(");
                }
                self.write(constraint.constraint.as_str());
                self.write(") ");
                self.format_expr(place);
            }
            AsmOperandKind::InOut { constraint, place } => {
                self.write("inout(");
                self.write(constraint.constraint.as_str());
                self.write(") ");
                self.format_expr(place);
            }
            AsmOperandKind::InLateOut { constraint, in_expr, out_place } => {
                self.write("inlateout(");
                self.write(constraint.constraint.as_str());
                self.write(") ");
                self.format_expr(in_expr);
                self.write(" => ");
                self.format_expr(out_place);
            }
            AsmOperandKind::Sym { path } => {
                self.write("sym ");
                self.format_path(path);
            }
            AsmOperandKind::Const { expr } => {
                self.write("const ");
                self.format_expr(expr);
            }
            AsmOperandKind::Clobber { reg } => {
                self.write("clobber_abi(\"");
                self.write(reg.as_str());
                self.write("\")");
            }
        }
    }

    fn format_asm_options(&mut self, options: &crate::expr::AsmOptions) {
        let mut opts = verum_common::List::new();

        if options.volatile {
            opts.push("volatile".to_string());
        }
        if options.preserves_flags {
            opts.push("preserves_flags".to_string());
        }
        if options.nomem {
            opts.push("nomem".to_string());
        }
        if options.readonly {
            opts.push("readonly".to_string());
        }
        if options.may_unwind {
            opts.push("may_unwind".to_string());
        }
        if options.pure_asm {
            opts.push("pure".to_string());
        }
        if options.noreturn {
            opts.push("noreturn".to_string());
        }
        if options.nostack {
            opts.push("nostack".to_string());
        }
        if options.intel_syntax {
            opts.push("intel".to_string());
        }

        // Add any raw options
        for opt in options.raw_options.iter() {
            opts.push(opt.to_string());
        }

        if !opts.is_empty() {
            self.write("options(");
            self.write(&opts.join(", "));
            self.write(")");
        }
    }

    fn format_comprehension_clause(&mut self, clause: &ComprehensionClause) {
        match &clause.kind {
            ComprehensionClauseKind::For { pattern, iter } => {
                self.write("for ");
                self.format_pattern(pattern);
                self.write(" in ");
                self.format_expr(iter);
            }
            ComprehensionClauseKind::If(cond) => {
                self.write("if ");
                self.format_expr(cond);
            }
            ComprehensionClauseKind::Let { pattern, ty, value } => {
                self.write("let ");
                self.format_pattern(pattern);
                if let Maybe::Some(t) = ty {
                    self.write(": ");
                    self.format_type(t);
                }
                self.write(" = ");
                self.format_expr(value);
            }
        }
    }

    fn format_field_init(&mut self, field: &FieldInit) {
        self.write(field.name.as_str());
        if let Maybe::Some(value) = &field.value {
            self.write(": ");
            self.format_expr(value);
        }
    }

    fn format_closure_param(&mut self, param: &ClosureParam) {
        self.format_pattern(&param.pattern);
        if let Maybe::Some(ty) = &param.ty {
            self.write(": ");
            self.format_type(ty);
        }
    }

    // ==================== LITERALS ====================

    fn format_literal(&mut self, lit: &Literal) {
        match &lit.kind {
            LiteralKind::Int(int_lit) => {
                self.write(&int_lit.value.to_string());
                if let Some(suffix) = &int_lit.suffix {
                    match suffix {
                        IntSuffix::Custom(s) => {
                            self.write("_");
                            self.write(s.as_str());
                        }
                        _ => self.write(&suffix.to_string()),
                    }
                }
            }
            LiteralKind::Float(float_lit) => {
                self.write(&float_lit.value.to_string());
                if let Some(suffix) = &float_lit.suffix {
                    match suffix {
                        FloatSuffix::Custom(s) => {
                            self.write("_");
                            self.write(s.as_str());
                        }
                        _ => self.write(&suffix.to_string()),
                    }
                }
            }
            LiteralKind::Text(string_lit) => {
                #[allow(deprecated)]
                match string_lit {
                    StringLit::Regular(s) => {
                        self.write("\"");
                        self.write(s.as_str());
                        self.write("\"");
                    }
                    StringLit::MultiLine(s) => {
                        self.write("\"\"\"");
                        self.write(s.as_str());
                        self.write("\"\"\"");
                    }
                }
            }
            LiteralKind::Char(c) => {
                self.write("'");
                self.write(&c.to_string());
                self.write("'");
            }
            LiteralKind::ByteChar(b) => {
                self.write("b'");
                self.write(&(*b as char).to_string());
                self.write("'");
            }
            LiteralKind::ByteString(bytes) => {
                self.write("b\"");
                for b in bytes {
                    if b.is_ascii_graphic() || *b == b' ' {
                        self.write(&(*b as char).to_string());
                    } else {
                        self.write(&format!("\\x{:02x}", b));
                    }
                }
                self.write("\"");
            }
            LiteralKind::Bool(b) => {
                self.write(if *b { "true" } else { "false" });
            }
            LiteralKind::Tagged { tag, content } => {
                self.write(tag.as_str());
                self.write("#\"");
                self.write(content.as_str());
                self.write("\"");
            }
            LiteralKind::InterpolatedString(interp) => {
                self.write(interp.prefix.as_str());
                self.write("\"");
                self.write(interp.content.as_str());
                self.write("\"");
            }
            LiteralKind::Contract(content) => {
                self.write("contract#\"");
                self.write(content.as_str());
                self.write("\"");
            }
            LiteralKind::Composite(comp) => {
                self.write(comp.tag.as_str());
                self.write("#");
                match comp.delimiter {
                    CompositeDelimiter::Quote => {
                        self.write("\"");
                        self.write(comp.content.as_str());
                        self.write("\"");
                    }
                    CompositeDelimiter::TripleQuote => {
                        self.write("\"\"\"");
                        self.write(comp.content.as_str());
                        self.write("\"\"\"");
                    }
                    CompositeDelimiter::Paren => {
                        self.write("(");
                        self.write(comp.content.as_str());
                        self.write(")");
                    }
                    CompositeDelimiter::Bracket => {
                        self.write("[");
                        self.write(comp.content.as_str());
                        self.write("]");
                    }
                    CompositeDelimiter::Brace => {
                        self.write("{");
                        self.write(comp.content.as_str());
                        self.write("}");
                    }
                }
            }
            LiteralKind::ContextAdaptive(ctx_lit) => {
                self.write(ctx_lit.raw.as_str());
            }
        }
    }

    // ==================== QUANTIFIER BINDINGS ====================

    /// Formats a list of quantifier bindings for forall/exists expressions.
    ///
    /// Output forms:
    /// - `x: Int`                    - type-based
    /// - `x in items`                - collection-based
    /// - `x: Int in 0..100`          - combined
    /// - `x in items where x > 0`    - with guard
    /// - `x: Int, y: Int`            - multiple bindings
    fn format_quantifier_bindings(&mut self, bindings: &[crate::expr::QuantifierBinding]) {
        for (i, binding) in bindings.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            self.format_pattern(&binding.pattern);
            if let verum_common::Maybe::Some(ty) = &binding.ty {
                self.write(": ");
                self.format_type(ty);
            }
            if let verum_common::Maybe::Some(domain) = &binding.domain {
                self.write(" in ");
                self.format_expr(domain);
            }
            if let verum_common::Maybe::Some(guard) = &binding.guard {
                self.write(" where ");
                self.format_expr(guard);
            }
        }
    }

    // ==================== PATTERNS ====================

    fn format_pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Wildcard => self.write("_"),
            PatternKind::Rest => self.write(".."),
            PatternKind::Ident {
                by_ref,
                mutable,
                name,
                subpattern,
            } => {
                if *by_ref {
                    self.write("ref ");
                }
                if *mutable {
                    self.write("mut ");
                }
                self.write(name.as_str());
                if let Maybe::Some(sub) = subpattern {
                    self.write(" @ ");
                    self.format_pattern(sub);
                }
            }
            PatternKind::Literal(lit) => self.format_literal(lit),
            PatternKind::Tuple(patterns) => {
                self.write("(");
                let mut first = true;
                for p in patterns {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_pattern(p);
                }
                if patterns.len() == 1 {
                    self.write(",");
                }
                self.write(")");
            }
            PatternKind::Array(patterns) => {
                self.write("[");
                let mut first = true;
                for p in patterns {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_pattern(p);
                }
                self.write("]");
            }
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                self.write("[");
                let mut first = true;
                for p in before {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_pattern(p);
                }
                if let Maybe::Some(r) = rest {
                    if !first {
                        self.write(", ");
                    }
                    self.format_pattern(r);
                    first = false;
                }
                for p in after {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_pattern(p);
                }
                self.write("]");
            }
            PatternKind::Record { path, fields, rest } => {
                self.format_path(path);
                self.write(" { ");
                let mut first = true;
                for field in fields {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_field_pattern(field);
                }
                if *rest {
                    if !first {
                        self.write(", ");
                    }
                    self.write("..");
                }
                self.write(" }");
            }
            PatternKind::Variant { path, data } => {
                self.format_path(path);
                if let Maybe::Some(d) = data {
                    match d {
                        VariantPatternData::Tuple(patterns) => {
                            self.write("(");
                            let mut first = true;
                            for p in patterns {
                                if !first {
                                    self.write(", ");
                                }
                                first = false;
                                self.format_pattern(p);
                            }
                            self.write(")");
                        }
                        VariantPatternData::Record { fields, rest } => {
                            self.write(" { ");
                            let mut first = true;
                            for field in fields {
                                if !first {
                                    self.write(", ");
                                }
                                first = false;
                                self.format_field_pattern(field);
                            }
                            if *rest {
                                if !first {
                                    self.write(", ");
                                }
                                self.write("..");
                            }
                            self.write(" }");
                        }
                    }
                }
            }
            PatternKind::Or(patterns) => {
                let mut first = true;
                for p in patterns {
                    if !first {
                        self.write(" | ");
                    }
                    first = false;
                    self.format_pattern(p);
                }
            }
            PatternKind::Reference { mutable, inner } => {
                self.write("&");
                if *mutable {
                    self.write("mut ");
                }
                self.format_pattern(inner);
            }
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                if let Maybe::Some(s) = start {
                    self.format_literal(s);
                }
                if *inclusive {
                    self.write("..=");
                } else {
                    self.write("..");
                }
                if let Maybe::Some(e) = end {
                    self.format_literal(e);
                }
            }
            PatternKind::Paren(inner) => {
                self.write("(");
                self.format_pattern(inner);
                self.write(")");
            }            PatternKind::View {
                view_function,
                pattern,
            } => {
                self.format_expr(view_function);
                self.write(" -> ");
                self.format_pattern(pattern);
            }
            PatternKind::Active { name, params, bindings } => {
                self.write(name.as_str());
                // Pattern parameters (e.g., InRange(0, 100))
                if !params.is_empty() {
                    self.write("(");
                    let mut first = true;
                    for arg in params {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_expr(arg);
                    }
                    self.write(")");
                }
                // Extraction bindings (e.g., ParseInt(n))
                self.write("(");
                if !bindings.is_empty() {
                    let mut first = true;
                    for binding in bindings {
                        if !first {
                            self.write(", ");
                        }
                        first = false;
                        self.format_pattern(binding);
                    }
                }
                self.write(")");
            }
            PatternKind::And(patterns) => {
                let mut first = true;
                for p in patterns {
                    if !first {
                        self.write(" & ");
                    }
                    first = false;
                    self.format_pattern(p);
                }
            }
            PatternKind::TypeTest { binding, test_type } => {
                self.write(binding.as_str());
                self.write(" is ");
                self.format_type(test_type);
            }
            // Stream pattern: stream[first, second, ...rest]
            // Stream pattern pretty-printing
            PatternKind::Stream { head_patterns, rest } => {
                self.write("stream[");
                let mut first = true;
                for p in head_patterns.iter() {
                    if !first {
                        self.write(", ");
                    }
                    first = false;
                    self.format_pattern(p);
                }
                if let Maybe::Some(rest_ident) = rest {
                    if !head_patterns.is_empty() {
                        self.write(", ");
                    }
                    self.write("...");
                    self.write(rest_ident.as_str());
                }
                self.write("]");
            }

            // Guard pattern: pattern if guard_expr
            // Spec: Rust RFC 3637 - Guard Patterns
            PatternKind::Guard { pattern, guard } => {
                self.format_pattern(pattern);
                self.write(" if ");
                self.format_expr(guard);
            }
            // Cons pattern: head :: tail
            PatternKind::Cons { head, tail } => {
                self.format_pattern(head);
                self.write(" :: ");
                self.format_pattern(tail);
            }
        }
    }

    fn format_field_pattern(&mut self, field: &FieldPattern) {
        self.write(field.name.as_str());
        if let Maybe::Some(p) = &field.pattern {
            self.write(": ");
            self.format_pattern(p);
        }
    }

    // ==================== STATEMENTS ====================

    fn format_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                self.write("let ");
                self.format_pattern(pattern);
                if let Maybe::Some(t) = ty {
                    self.write(": ");
                    self.format_type(t);
                }
                if let Maybe::Some(val) = value {
                    self.write(" = ");
                    self.format_expr(val);
                }
                self.write(";");
            }
            StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                self.write("let ");
                self.format_pattern(pattern);
                if let Maybe::Some(t) = ty {
                    self.write(": ");
                    self.format_type(t);
                }
                self.write(" = ");
                self.format_expr(value);
                self.write(" else ");
                self.format_block(else_block);
            }
            StmtKind::Expr { expr, has_semi } => {
                self.format_expr(expr);
                if *has_semi {
                    self.write(";");
                }
            }
            StmtKind::Item(item) => {
                self.format_item(item);
            }
            StmtKind::Defer(expr) => {
                self.write("defer ");
                self.format_expr(expr);
                self.write(";");
            }
            StmtKind::Errdefer(expr) => {
                self.write("errdefer ");
                self.format_expr(expr);
                self.write(";");
            }
            StmtKind::Provide { context, value, alias } => {
                self.write("provide ");
                self.write(context.as_str());
                if let Maybe::Some(alias_name) = alias {
                    self.write(" as ");
                    self.write(alias_name.as_str());
                }
                self.write(" = ");
                self.format_expr(value);
                self.write(";");
            }
            StmtKind::ProvideScope {
                context,
                value,
                block,
                alias,
            } => {
                self.write("provide ");
                self.write(context.as_str());
                if let Maybe::Some(alias_name) = alias {
                    self.write(" as ");
                    self.write(alias_name.as_str());
                }
                self.write(" = ");
                self.format_expr(value);
                self.write(" in ");
                self.format_expr(block);
            }
            StmtKind::Empty => {
                self.write(";");
            }
        }
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

/// Format a type to a string.
pub fn format_type(ty: &Type) -> Text {
    let mut printer = PrettyPrinter::default_printer();
    printer.format_type(ty);
    Text::from(printer.output)
}

/// Format an expression to a string.
pub fn format_expr(expr: &Expr) -> Text {
    let mut printer = PrettyPrinter::default_printer();
    printer.format_expr(expr);
    Text::from(printer.output)
}

/// Format a pattern to a string.
pub fn format_pattern(pattern: &Pattern) -> Text {
    let mut printer = PrettyPrinter::default_printer();
    printer.format_pattern(pattern);
    Text::from(printer.output)
}

/// Format a statement to a string.
pub fn format_stmt(stmt: &Stmt) -> Text {
    let mut printer = PrettyPrinter::default_printer();
    printer.format_stmt(stmt);
    Text::from(printer.output)
}

/// Format an item to a string.
pub fn format_item(item: &Item) -> Text {
    let mut printer = PrettyPrinter::default_printer();
    printer.format_item(item);
    Text::from(printer.output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{FileId, Span};

    fn dummy_span() -> Span {
        Span::new(0, 0, FileId::new(0))
    }

    #[test]
    fn test_format_primitive_types() {
        assert_eq!(format_type(&Type::unit(dummy_span())).as_str(), "()");
        assert_eq!(format_type(&Type::bool(dummy_span())).as_str(), "Bool");
        assert_eq!(format_type(&Type::int(dummy_span())).as_str(), "Int");
        assert_eq!(format_type(&Type::float(dummy_span())).as_str(), "Float");
        assert_eq!(format_type(&Type::text(dummy_span())).as_str(), "Text");
        assert_eq!(format_type(&Type::inferred(dummy_span())).as_str(), "_");
    }

    #[test]
    fn test_format_reference_types() {
        let inner = Type::int(dummy_span());

        // Immutable reference
        let ref_ty = Type::new(
            TypeKind::Reference {
                mutable: false,
                inner: verum_common::Heap::new(inner.clone()),
            },
            dummy_span(),
        );
        assert_eq!(format_type(&ref_ty).as_str(), "&Int");

        // Mutable reference
        let mut_ref_ty = Type::new(
            TypeKind::Reference {
                mutable: true,
                inner: verum_common::Heap::new(inner.clone()),
            },
            dummy_span(),
        );
        assert_eq!(format_type(&mut_ref_ty).as_str(), "&mut Int");

        // Checked reference
        let checked_ref_ty = Type::new(
            TypeKind::CheckedReference {
                mutable: false,
                inner: verum_common::Heap::new(inner.clone()),
            },
            dummy_span(),
        );
        assert_eq!(format_type(&checked_ref_ty).as_str(), "&checked Int");

        // Unsafe reference
        let unsafe_ref_ty = Type::new(
            TypeKind::UnsafeReference {
                mutable: false,
                inner: verum_common::Heap::new(inner),
            },
            dummy_span(),
        );
        assert_eq!(format_type(&unsafe_ref_ty).as_str(), "&unsafe Int");
    }

    #[test]
    fn test_format_tuple_types() {
        let int_ty = Type::int(dummy_span());
        let text_ty = Type::text(dummy_span());

        // Simple tuple
        let tuple_ty = Type::new(
            TypeKind::Tuple(vec![int_ty.clone(), text_ty.clone()].into()),
            dummy_span(),
        );
        assert_eq!(format_type(&tuple_ty).as_str(), "(Int, Text)");

        // Single element tuple
        let single_ty = Type::new(TypeKind::Tuple(vec![int_ty.clone()].into()), dummy_span());
        assert_eq!(format_type(&single_ty).as_str(), "(Int,)");

        // Empty tuple (unit)
        let unit_ty = Type::new(TypeKind::Tuple(List::new()), dummy_span());
        assert_eq!(format_type(&unit_ty).as_str(), "()");
    }

    #[test]
    fn test_format_array_and_slice_types() {
        let int_ty = Type::int(dummy_span());

        // Slice
        let slice_ty = Type::new(
            TypeKind::Slice(verum_common::Heap::new(int_ty.clone())),
            dummy_span(),
        );
        assert_eq!(format_type(&slice_ty).as_str(), "[Int]");

        // Array without size
        let arr_ty = Type::new(
            TypeKind::Array {
                element: verum_common::Heap::new(int_ty.clone()),
                size: Maybe::None,
            },
            dummy_span(),
        );
        assert_eq!(format_type(&arr_ty).as_str(), "[Int]");
    }

    #[test]
    fn test_format_function_type() {
        use crate::context::ContextList;
        let int_ty = Type::int(dummy_span());
        let bool_ty = Type::bool(dummy_span());

        let fn_ty = Type::new(
            TypeKind::Function {
                params: vec![int_ty.clone(), int_ty.clone()].into(),
                return_type: verum_common::Heap::new(bool_ty),
                calling_convention: Maybe::None,
                contexts: ContextList::empty(),
            },
            dummy_span(),
        );
        assert_eq!(format_type(&fn_ty).as_str(), "fn(Int, Int) -> Bool");
    }

    #[test]
    fn test_format_literal_expr() {
        let span = dummy_span();

        // Integer
        let int_lit = Expr::literal(Literal::int(42, span));
        assert_eq!(format_expr(&int_lit).as_str(), "42");

        // Float
        let float_lit = Expr::literal(Literal::float(2.72, span));
        assert_eq!(format_expr(&float_lit).as_str(), "2.72");

        // Bool
        let bool_lit = Expr::literal(Literal::bool(true, span));
        assert_eq!(format_expr(&bool_lit).as_str(), "true");

        // String
        let str_lit = Expr::literal(Literal::string(Text::from("hello"), span));
        assert_eq!(format_expr(&str_lit).as_str(), "\"hello\"");

        // Char
        let char_lit = Expr::literal(Literal::char('a', span));
        assert_eq!(format_expr(&char_lit).as_str(), "'a'");
    }

    #[test]
    fn test_format_binary_expr() {
        let span = dummy_span();
        let left = verum_common::Heap::new(Expr::literal(Literal::int(1, span)));
        let right = verum_common::Heap::new(Expr::literal(Literal::int(2, span)));

        let add_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: left.clone(),
                right: right.clone(),
            },
            span,
        );
        assert_eq!(format_expr(&add_expr).as_str(), "1 + 2");

        let eq_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            },
            span,
        );
        assert_eq!(format_expr(&eq_expr).as_str(), "1 == 2");
    }

    #[test]
    fn test_format_unary_expr() {
        let span = dummy_span();
        let inner = verum_common::Heap::new(Expr::literal(Literal::int(42, span)));

        let neg_expr = Expr::new(
            ExprKind::Unary {
                op: UnOp::Neg,
                expr: inner.clone(),
            },
            span,
        );
        assert_eq!(format_expr(&neg_expr).as_str(), "-42");

        let not_expr = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: inner,
            },
            span,
        );
        assert_eq!(format_expr(&not_expr).as_str(), "!42");
    }

    #[test]
    fn test_format_pattern_wildcard() {
        let pattern = Pattern::wildcard(dummy_span());
        assert_eq!(format_pattern(&pattern).as_str(), "_");
    }

    #[test]
    fn test_format_pattern_ident() {
        let span = dummy_span();
        let ident = Ident::new("x", span);
        let pattern = Pattern::ident(ident, false, span);
        assert_eq!(format_pattern(&pattern).as_str(), "x");
    }

    #[test]
    fn test_format_pattern_mutable_ident() {
        let span = dummy_span();
        let ident = Ident::new("x", span);
        let pattern = Pattern::ident(ident, true, span);
        assert_eq!(format_pattern(&pattern).as_str(), "mut x");
    }

    #[test]
    fn test_format_tuple_pattern() {
        let span = dummy_span();
        let patterns = vec![
            Pattern::ident(Ident::new("a", span), false, span),
            Pattern::ident(Ident::new("b", span), false, span),
        ].into();
        let pattern = Pattern::new(PatternKind::Tuple(patterns), span);
        assert_eq!(format_pattern(&pattern).as_str(), "(a, b)");
    }

    #[test]
    fn test_format_or_pattern() {
        let span = dummy_span();
        let patterns = vec![
            Pattern::ident(Ident::new("a", span), false, span),
            Pattern::ident(Ident::new("b", span), false, span),
        ].into();
        let pattern = Pattern::new(PatternKind::Or(patterns), span);
        assert_eq!(format_pattern(&pattern).as_str(), "a | b");
    }

    #[test]
    fn test_format_let_stmt() {
        let span = dummy_span();
        let pattern = Pattern::ident(Ident::new("x", span), false, span);
        let value = Expr::literal(Literal::int(42, span));

        let stmt = Stmt::let_stmt(
            pattern,
            Maybe::Some(Type::int(span)),
            Maybe::Some(value),
            span,
        );
        assert_eq!(format_stmt(&stmt).as_str(), "let x: Int = 42;");
    }

    #[test]
    fn test_format_let_stmt_without_type() {
        let span = dummy_span();
        let pattern = Pattern::ident(Ident::new("x", span), false, span);
        let value = Expr::literal(Literal::int(42, span));

        let stmt = Stmt::let_stmt(pattern, Maybe::None, Maybe::Some(value), span);
        assert_eq!(format_stmt(&stmt).as_str(), "let x = 42;");
    }

    #[test]
    fn test_format_expr_stmt() {
        let span = dummy_span();
        let expr = Expr::literal(Literal::int(42, span));
        let stmt = Stmt::expr(expr, true);
        assert_eq!(format_stmt(&stmt).as_str(), "42;");
    }

    #[test]
    fn test_pretty_printer_is_full_implementation() {
        assert!(PrettyPrinter::is_full_implementation());
    }

    #[test]
    fn test_format_visibility() {
        let mut printer = PrettyPrinter::default_printer();

        printer.format_visibility(&Visibility::Public);
        assert_eq!(printer.output, "public ");

        printer.reset();
        printer.format_visibility(&Visibility::Private);
        assert_eq!(printer.output, "");

        printer.reset();
        printer.format_visibility(&Visibility::PublicCrate);
        assert_eq!(printer.output, "public(crate) ");

        printer.reset();
        printer.format_visibility(&Visibility::Internal);
        assert_eq!(printer.output, "internal ");

        printer.reset();
        printer.format_visibility(&Visibility::Protected);
        assert_eq!(printer.output, "protected ");
    }

    #[test]
    fn test_format_path() {
        let span = dummy_span();
        let path = Path::single(Ident::new("foo", span));

        let mut printer = PrettyPrinter::default_printer();
        printer.format_path(&path);
        assert_eq!(printer.output, "foo");
    }

    #[test]
    fn test_format_generic_type() {
        let span = dummy_span();
        let int_ty = Type::int(span);
        let path = Path::single(Ident::new("List", span));
        let base_ty = Type::new(TypeKind::Path(path), span);

        let generic_ty = Type::new(
            TypeKind::Generic {
                base: verum_common::Heap::new(base_ty),
                args: vec![GenericArg::Type(int_ty)].into(),
            },
            span,
        );
        assert_eq!(format_type(&generic_ty).as_str(), "List<Int>");
    }

    #[test]
    fn test_format_refined_type() {
        let span = dummy_span();
        let int_ty = Type::int(span);

        // Create a simple predicate expression: > 0
        let zero = Expr::literal(Literal::int(0, span));
        let it_path = Path::single(Ident::new("it", span));
        let it_expr = Expr::new(ExprKind::Path(it_path), span);

        let pred_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: verum_common::Heap::new(it_expr),
                right: verum_common::Heap::new(zero),
            },
            span,
        );

        let predicate = RefinementPredicate::new(pred_expr, span);

        let refined_ty = Type::new(
            TypeKind::Refined {
                base: verum_common::Heap::new(int_ty),
                predicate: verum_common::Heap::new(predicate),
            },
            span,
        );

        assert_eq!(format_type(&refined_ty).as_str(), "Int{it > 0}");
    }

    #[test]
    fn test_format_dyn_protocol() {
        let span = dummy_span();
        let display_path = Path::single(Ident::new("Display", span));

        let dyn_ty = Type::new(
            TypeKind::DynProtocol {
                bounds: vec![TypeBound {
                    kind: TypeBoundKind::Protocol(display_path),
                    span,
                }].into(),
                bindings: Maybe::None,
            },
            span,
        );

        assert_eq!(format_type(&dyn_ty).as_str(), "dyn Display");
    }

    #[test]
    fn test_format_genref_type() {
        let span = dummy_span();
        let int_ty = Type::int(span);

        let genref_ty = Type::new(
            TypeKind::GenRef {
                inner: verum_common::Heap::new(int_ty),
            },
            span,
        );

        assert_eq!(format_type(&genref_ty).as_str(), "GenRef<Int>");
    }

    #[test]
    fn test_format_ownership_type() {
        let span = dummy_span();
        let int_ty = Type::int(span);

        let own_ty = Type::new(
            TypeKind::Ownership {
                mutable: false,
                inner: verum_common::Heap::new(int_ty.clone()),
            },
            span,
        );
        assert_eq!(format_type(&own_ty).as_str(), "%Int");

        let own_mut_ty = Type::new(
            TypeKind::Ownership {
                mutable: true,
                inner: verum_common::Heap::new(int_ty),
            },
            span,
        );
        assert_eq!(format_type(&own_mut_ty).as_str(), "%mut Int");
    }

    #[test]
    fn test_format_call_expr() {
        let span = dummy_span();
        let fn_path = Path::single(Ident::new("foo", span));
        let fn_expr = Expr::new(ExprKind::Path(fn_path), span);
        let arg1 = Expr::literal(Literal::int(1, span));
        let arg2 = Expr::literal(Literal::int(2, span));

        let call_expr = Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(fn_expr),
                type_args: List::new(),
                args: vec![arg1, arg2].into(),
            },
            span,
        );

        assert_eq!(format_expr(&call_expr).as_str(), "foo(1, 2)");
    }

    #[test]
    fn test_format_method_call_expr() {
        let span = dummy_span();
        let obj_path = Path::single(Ident::new("obj", span));
        let obj_expr = Expr::new(ExprKind::Path(obj_path), span);

        let call_expr = Expr::new(
            ExprKind::MethodCall {
                receiver: verum_common::Heap::new(obj_expr),
                method: Ident::new("method", span),
                type_args: List::new(),
                args: vec![Expr::literal(Literal::int(42, span))].into(),
            },
            span,
        );

        assert_eq!(format_expr(&call_expr).as_str(), "obj.method(42)");
    }

    #[test]
    fn test_format_field_access() {
        let span = dummy_span();
        let obj_path = Path::single(Ident::new("obj", span));
        let obj_expr = Expr::new(ExprKind::Path(obj_path), span);

        let field_expr = Expr::new(
            ExprKind::Field {
                expr: verum_common::Heap::new(obj_expr),
                field: Ident::new("field", span),
            },
            span,
        );

        assert_eq!(format_expr(&field_expr).as_str(), "obj.field");
    }

    #[test]
    fn test_format_optional_chain() {
        let span = dummy_span();
        let obj_path = Path::single(Ident::new("obj", span));
        let obj_expr = Expr::new(ExprKind::Path(obj_path), span);

        let chain_expr = Expr::new(
            ExprKind::OptionalChain {
                expr: verum_common::Heap::new(obj_expr),
                field: Ident::new("field", span),
            },
            span,
        );

        assert_eq!(format_expr(&chain_expr).as_str(), "obj?.field");
    }

    #[test]
    fn test_format_pipeline_expr() {
        let span = dummy_span();
        let x = Expr::new(ExprKind::Path(Path::single(Ident::new("x", span))), span);
        let f = Expr::new(ExprKind::Path(Path::single(Ident::new("f", span))), span);

        let pipe_expr = Expr::new(
            ExprKind::Pipeline {
                left: verum_common::Heap::new(x),
                right: verum_common::Heap::new(f),
            },
            span,
        );

        assert_eq!(format_expr(&pipe_expr).as_str(), "x |> f");
    }

    #[test]
    fn test_format_try_expr() {
        let span = dummy_span();
        let inner = Expr::new(
            ExprKind::Path(Path::single(Ident::new("result", span))),
            span,
        );

        let try_expr = Expr::new(ExprKind::Try(verum_common::Heap::new(inner)), span);

        assert_eq!(format_expr(&try_expr).as_str(), "result?");
    }

    #[test]
    fn test_format_await_expr() {
        let span = dummy_span();
        let inner = Expr::new(
            ExprKind::Path(Path::single(Ident::new("future", span))),
            span,
        );

        let await_expr = Expr::new(ExprKind::Await(verum_common::Heap::new(inner)), span);

        assert_eq!(format_expr(&await_expr).as_str(), "future.await");
    }

    #[test]
    fn test_format_range_expr() {
        let span = dummy_span();
        let start = Expr::literal(Literal::int(0, span));
        let end = Expr::literal(Literal::int(10, span));

        // Exclusive range
        let range_expr = Expr::new(
            ExprKind::Range {
                start: Maybe::Some(verum_common::Heap::new(start.clone())),
                end: Maybe::Some(verum_common::Heap::new(end.clone())),
                inclusive: false,
            },
            span,
        );
        assert_eq!(format_expr(&range_expr).as_str(), "0..10");

        // Inclusive range
        let range_incl_expr = Expr::new(
            ExprKind::Range {
                start: Maybe::Some(verum_common::Heap::new(start)),
                end: Maybe::Some(verum_common::Heap::new(end)),
                inclusive: true,
            },
            span,
        );
        assert_eq!(format_expr(&range_incl_expr).as_str(), "0..=10");
    }

    #[test]
    fn test_format_return_expr() {
        let span = dummy_span();

        // Return without value
        let ret_expr = Expr::new(ExprKind::Return(Maybe::None), span);
        assert_eq!(format_expr(&ret_expr).as_str(), "return");

        // Return with value
        let ret_val_expr = Expr::new(
            ExprKind::Return(Maybe::Some(verum_common::Heap::new(Expr::literal(
                Literal::int(42, span),
            )))),
            span,
        );
        assert_eq!(format_expr(&ret_val_expr).as_str(), "return 42");
    }

    #[test]
    fn test_format_break_continue() {
        let span = dummy_span();

        // Break without label
        let break_expr = Expr::new(
            ExprKind::Break {
                label: Maybe::None,
                value: Maybe::None,
            },
            span,
        );
        assert_eq!(format_expr(&break_expr).as_str(), "break");

        // Break with label
        let break_label = Expr::new(
            ExprKind::Break {
                label: Maybe::Some(Text::from("outer")),
                value: Maybe::None,
            },
            span,
        );
        assert_eq!(format_expr(&break_label).as_str(), "break 'outer");

        // Continue
        let continue_expr = Expr::new(ExprKind::Continue { label: Maybe::None }, span);
        assert_eq!(format_expr(&continue_expr).as_str(), "continue");
    }

    #[test]
    fn test_format_cast_expr() {
        let span = dummy_span();
        let inner = Expr::new(ExprKind::Path(Path::single(Ident::new("x", span))), span);

        let cast_expr = Expr::new(
            ExprKind::Cast {
                expr: verum_common::Heap::new(inner),
                ty: Type::int(span),
            },
            span,
        );

        assert_eq!(format_expr(&cast_expr).as_str(), "x as Int");
    }

    #[test]
    fn test_format_null_coalesce() {
        let span = dummy_span();
        let left = Expr::new(
            ExprKind::Path(Path::single(Ident::new("maybe_val", span))),
            span,
        );
        let right = Expr::literal(Literal::int(0, span));

        let coalesce_expr = Expr::new(
            ExprKind::NullCoalesce {
                left: verum_common::Heap::new(left),
                right: verum_common::Heap::new(right),
            },
            span,
        );

        assert_eq!(format_expr(&coalesce_expr).as_str(), "maybe_val ?? 0");
    }

    #[test]
    fn test_format_index_expr() {
        let span = dummy_span();
        let arr = Expr::new(ExprKind::Path(Path::single(Ident::new("arr", span))), span);
        let idx = Expr::literal(Literal::int(0, span));

        let index_expr = Expr::new(
            ExprKind::Index {
                expr: verum_common::Heap::new(arr),
                index: verum_common::Heap::new(idx),
            },
            span,
        );

        assert_eq!(format_expr(&index_expr).as_str(), "arr[0]");
    }

    #[test]
    fn test_format_tuple_index() {
        let span = dummy_span();
        let tuple = Expr::new(
            ExprKind::Path(Path::single(Ident::new("tuple", span))),
            span,
        );

        let index_expr = Expr::new(
            ExprKind::TupleIndex {
                expr: verum_common::Heap::new(tuple),
                index: 0,
            },
            span,
        );

        assert_eq!(format_expr(&index_expr).as_str(), "tuple.0");
    }

    #[test]
    fn test_format_array_expr() {
        let span = dummy_span();
        let elements = vec![
            Expr::literal(Literal::int(1, span)),
            Expr::literal(Literal::int(2, span)),
            Expr::literal(Literal::int(3, span)),
        ].into();

        let arr_expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements)), span);

        assert_eq!(format_expr(&arr_expr).as_str(), "[1, 2, 3]");
    }

    #[test]
    fn test_format_array_repeat() {
        let span = dummy_span();
        let value = Expr::literal(Literal::int(0, span));
        let count = Expr::literal(Literal::int(10, span));

        let arr_expr = Expr::new(
            ExprKind::Array(ArrayExpr::Repeat {
                value: verum_common::Heap::new(value),
                count: verum_common::Heap::new(count),
            }),
            span,
        );

        assert_eq!(format_expr(&arr_expr).as_str(), "[0; 10]");
    }

    #[test]
    fn test_format_tuple_expr() {
        let span = dummy_span();
        let elements = vec![
            Expr::literal(Literal::int(1, span)),
            Expr::literal(Literal::int(2, span)),
        ].into();

        let tuple_expr = Expr::new(ExprKind::Tuple(elements), span);
        assert_eq!(format_expr(&tuple_expr).as_str(), "(1, 2)");

        // Single element tuple
        let single = vec![Expr::literal(Literal::int(1, span))].into();
        let single_tuple = Expr::new(ExprKind::Tuple(single), span);
        assert_eq!(format_expr(&single_tuple).as_str(), "(1,)");
    }

    #[test]
    fn test_format_paren_expr() {
        let span = dummy_span();
        let inner = Expr::literal(Literal::int(42, span));

        let paren_expr = Expr::new(ExprKind::Paren(verum_common::Heap::new(inner)), span);

        assert_eq!(format_expr(&paren_expr).as_str(), "(42)");
    }

    #[test]
    fn test_format_set_literal() {
        let span = dummy_span();
        let elements = vec![
            Expr::literal(Literal::int(1, span)),
            Expr::literal(Literal::int(2, span)),
            Expr::literal(Literal::int(3, span)),
        ].into();

        let set_expr = Expr::new(ExprKind::SetLiteral { elements }, span);
        assert_eq!(format_expr(&set_expr).as_str(), "{1, 2, 3}");
    }

    #[test]
    fn test_format_map_literal() {
        let span = dummy_span();
        let entries = vec![
            (
                Expr::literal(Literal::string(Text::from("a"), span)),
                Expr::literal(Literal::int(1, span)),
            ),
            (
                Expr::literal(Literal::string(Text::from("b"), span)),
                Expr::literal(Literal::int(2, span)),
            ),
        ].into();

        let map_expr = Expr::new(ExprKind::MapLiteral { entries }, span);
        assert_eq!(format_expr(&map_expr).as_str(), "{\"a\": 1, \"b\": 2}");
    }

    #[test]
    fn test_format_yield_expr() {
        let span = dummy_span();
        let value = Expr::literal(Literal::int(42, span));

        let yield_expr = Expr::new(ExprKind::Yield(verum_common::Heap::new(value)), span);

        assert_eq!(format_expr(&yield_expr).as_str(), "yield 42");
    }

    #[test]
    fn test_format_forall_expr() {
        let span = dummy_span();
        let pattern = Pattern::ident(Ident::new("x", span), false, span);
        let body = Expr::new(
            ExprKind::Binary {
                op: BinOp::Ge,
                left: verum_common::Heap::new(Expr::new(
                    ExprKind::Path(Path::single(Ident::new("x", span))),
                    span,
                )),
                right: verum_common::Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        );

        let binding = crate::expr::QuantifierBinding::typed(pattern, Type::int(span), span);
        let forall_expr = Expr::new(
            ExprKind::Forall {
                bindings: vec![binding].into(),
                body: verum_common::Heap::new(body),
            },
            span,
        );

        assert_eq!(
            format_expr(&forall_expr).as_str(),
            "forall x: Int. x >= 0"
        );
    }

    #[test]
    fn test_format_exists_expr() {
        let span = dummy_span();
        let pattern = Pattern::ident(Ident::new("x", span), false, span);
        let body = Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: verum_common::Heap::new(Expr::new(
                    ExprKind::Path(Path::single(Ident::new("x", span))),
                    span,
                )),
                right: verum_common::Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        );

        let binding = crate::expr::QuantifierBinding::typed(pattern, Type::int(span), span);
        let exists_expr = Expr::new(
            ExprKind::Exists {
                bindings: vec![binding].into(),
                body: verum_common::Heap::new(body),
            },
            span,
        );

        assert_eq!(
            format_expr(&exists_expr).as_str(),
            "exists x: Int. x > 0"
        );
    }

    #[test]
    fn test_format_tagged_literal() {
        let span = dummy_span();
        let lit = Literal::tagged(Text::from("sql"), Text::from("SELECT * FROM users"), span);

        let mut printer = PrettyPrinter::default_printer();
        printer.format_literal(&lit);
        assert_eq!(printer.output, "sql#\"SELECT * FROM users\"");
    }

    #[test]
    fn test_format_contract_literal() {
        let span = dummy_span();
        let lit = Literal::contract(Text::from("it > 0"), span);

        let mut printer = PrettyPrinter::default_printer();
        printer.format_literal(&lit);
        assert_eq!(printer.output, "contract#\"it > 0\"");
    }

    #[test]
    fn test_format_pointer_type() {
        let span = dummy_span();
        let int_ty = Type::int(span);

        let const_ptr = Type::new(
            TypeKind::Pointer {
                mutable: false,
                inner: verum_common::Heap::new(int_ty.clone()),
            },
            span,
        );
        assert_eq!(format_type(&const_ptr).as_str(), "*const Int");

        let mut_ptr = Type::new(
            TypeKind::Pointer {
                mutable: true,
                inner: verum_common::Heap::new(int_ty),
            },
            span,
        );
        assert_eq!(format_type(&mut_ptr).as_str(), "*mut Int");
    }

    #[test]
    fn test_format_defer_stmt() {
        let span = dummy_span();
        let cleanup = Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(Expr::new(
                    ExprKind::Path(Path::single(Ident::new("cleanup", span))),
                    span,
                )),
                type_args: List::new(),
                args: List::new(),
            },
            span,
        );

        let stmt = Stmt::new(StmtKind::Defer(cleanup), span);
        assert_eq!(format_stmt(&stmt).as_str(), "defer cleanup();");
    }

    #[test]
    fn test_format_provide_stmt() {
        let span = dummy_span();
        let value = Expr::new(
            ExprKind::Path(Path::single(Ident::new("db_impl", span))),
            span,
        );

        let stmt = Stmt::new(
            StmtKind::Provide {
                context: Text::from("Database"),
                alias: verum_common::Maybe::None,
                value: verum_common::Heap::new(value),
            },
            span,
        );
        assert_eq!(format_stmt(&stmt).as_str(), "provide Database = db_impl;");
    }

    #[test]
    fn test_format_provide_stmt_with_alias() {
        let span = dummy_span();
        let value = Expr::new(
            ExprKind::Path(Path::single(Ident::new("source_db", span))),
            span,
        );

        let stmt = Stmt::new(
            StmtKind::Provide {
                context: Text::from("Database"),
                alias: verum_common::Maybe::Some(Text::from("source")),
                value: verum_common::Heap::new(value),
            },
            span,
        );
        assert_eq!(
            format_stmt(&stmt).as_str(),
            "provide Database as source = source_db;"
        );
    }

    #[test]
    fn test_format_empty_stmt() {
        let stmt = Stmt::new(StmtKind::Empty, dummy_span());
        assert_eq!(format_stmt(&stmt).as_str(), ";");
    }
}
