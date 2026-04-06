//! AST Sink - Converts green/red syntax tree to semantic AST.
//!
//! This module provides the `AstSink` that traverses a `SyntaxNode` (red tree)
//! and builds the semantic AST types (Module, Item, Expr, Type, etc.).
//!
//! This enables single-pass parsing architecture:
//!
//! ```text
//! Source → Events → GreenTree → AstSink → Module (semantic AST)
//! ```
//!
//! # Architecture
//!
//! The AstSink traverses the red tree and converts each SyntaxKind
//! to the appropriate AST type:
//!
//! - SOURCE_FILE → Module
//! - FN_DEF → Item(FunctionDecl)
//! - TYPE_DEF → Item(TypeDecl)
//! - EXPR_* → Expr
//! - TYPE_* → Type
//! - etc.
//!
//! # Error Handling
//!
//! The sink handles ERROR nodes gracefully by creating placeholder AST nodes
//! that allow the rest of the tree to be processed. This enables IDE features
//! to work on incomplete code.

use verum_ast::{
    Attribute, BinOp, Block, ConditionKind, ContextList, Expr, ExprKind, FileId, FunctionBody,
    FunctionDecl, FunctionParam, FunctionParamKind, GenericParam, Ident, IfCondition, MountDecl,
    MountTree, MountTreeKind, Item, ItemKind, Literal, LiteralKind, MatchArm, Module, Path,
    PathSegment, Pattern, PatternKind, Span, Stmt, StmtKind, Type, TypeDecl, TypeDeclBody,
    TypeKind, UnOp, Visibility, WhereClause,
    decl::{ConstDecl, ContextDecl, ContextRequirement, ImplDecl, ModuleDecl, ProtocolDecl,
           ProtocolItem, ProtocolItemKind, RecordField, StaticDecl, Variant},
    expr::{RecoverBody, RecoverClosureParam},
    literal::StringLit,
    pattern::{FieldPattern, VariantPatternData},
    ty::GenericArg,
};
use verum_ast::smallvec::smallvec;
use verum_common::{Heap, List, Maybe, Text};
use verum_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

use crate::ParseError;

/// Result of converting a syntax tree to semantic AST.
#[derive(Debug)]
pub struct AstSinkResult {
    /// The resulting module.
    pub module: Module,
    /// Any errors encountered during conversion.
    pub errors: Vec<ParseError>,
}

/// Converts a lossless syntax tree to a semantic AST.
///
/// The sink traverses the red tree (SyntaxNode) and builds the corresponding
/// verum_ast types. It handles ERROR nodes gracefully to support incomplete code.
pub struct AstSink {
    /// File ID for span construction.
    file_id: FileId,
    /// Accumulated errors during conversion.
    errors: Vec<ParseError>,
    /// Source text for extracting token values.
    source: String,
}

impl AstSink {
    /// Create a new AST sink.
    pub fn new(source: &str, file_id: FileId) -> Self {
        Self {
            file_id,
            errors: Vec::new(),
            source: source.to_string(),
        }
    }

    /// Convert a syntax tree root to a Module.
    pub fn convert(mut self, root: &SyntaxNode) -> AstSinkResult {
        let module = self.convert_module(root);
        AstSinkResult {
            module,
            errors: self.errors,
        }
    }

    /// Convert a SOURCE_FILE node to a Module.
    fn convert_module(&mut self, node: &SyntaxNode) -> Module {
        let mut items = List::new();
        let mut attributes = List::new();

        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::ATTRIBUTE => {
                    if let Some(attr) = self.convert_attribute(&child) {
                        if self.is_inner_attribute(&child) {
                            attributes.push(attr);
                        }
                    }
                }
                // Match item-level node kinds (not keywords)
                SyntaxKind::FN_DEF
                | SyntaxKind::TYPE_DEF
                | SyntaxKind::PROTOCOL_DEF
                | SyntaxKind::IMPL_BLOCK
                | SyntaxKind::MOUNT_STMT
                | SyntaxKind::CONST_DEF
                | SyntaxKind::STATIC_DEF
                | SyntaxKind::CONTEXT_DEF
                | SyntaxKind::MODULE_DEF
                | SyntaxKind::META_DEF
                | SyntaxKind::FFI_BLOCK
                | SyntaxKind::THEOREM_DEF
                | SyntaxKind::AXIOM_DEF
                | SyntaxKind::LEMMA_DEF => {
                    if let Some(item) = self.convert_item(&child) {
                        items.push(item);
                    }
                }
                SyntaxKind::ERROR => {
                    let range = child.text_range();
                    self.error_at(range, "syntax error in module");
                }
                _ => {}
            }
        }

        let span = self.range_to_span(node.text_range());
        Module::new_with_attrs(items, attributes, self.file_id, span)
    }

    /// Convert a declaration node to an Item.
    fn convert_item(&mut self, node: &SyntaxNode) -> Option<Item> {
        let span = self.range_to_span(node.text_range());
        let attributes = self.collect_attributes(node);

        let kind = match node.kind() {
            SyntaxKind::FN_DEF => {
                let func = self.convert_function(node)?;
                ItemKind::Function(func)
            }
            SyntaxKind::TYPE_DEF => {
                let type_decl = self.convert_type_def(node)?;
                ItemKind::Type(type_decl)
            }
            SyntaxKind::PROTOCOL_DEF => {
                let protocol = self.convert_protocol_def(node)?;
                ItemKind::Protocol(protocol)
            }
            SyntaxKind::IMPL_BLOCK => {
                let impl_decl = self.convert_impl_block(node)?;
                ItemKind::Impl(impl_decl)
            }
            SyntaxKind::MOUNT_STMT => {
                let mount = self.convert_mount(node)?;
                ItemKind::Mount(mount)
            }
            SyntaxKind::CONST_DEF => {
                let const_decl = self.convert_const_def(node)?;
                ItemKind::Const(const_decl)
            }
            SyntaxKind::STATIC_DEF => {
                let static_decl = self.convert_static_def(node)?;
                ItemKind::Static(static_decl)
            }
            SyntaxKind::CONTEXT_DEF => {
                let ctx = self.convert_context_def(node)?;
                ItemKind::Context(ctx)
            }
            SyntaxKind::MODULE_DEF => {
                let mod_decl = self.convert_module_def(node)?;
                ItemKind::Module(mod_decl)
            }
            SyntaxKind::ERROR => {
                self.error_at(node.text_range(), "syntax error in item");
                return None;
            }
            _ => {
                return None;
            }
        };

        Some(Item::new_with_attrs(kind, attributes, span))
    }

    /// Convert a FN_DEF node to a FunctionDecl.
    fn convert_function(&mut self, node: &SyntaxNode) -> Option<FunctionDecl> {
        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut is_async = false;
        let mut is_meta = false;
        let mut is_pure = false;
        let mut is_generator = false;
        let mut is_cofix = false;
        let mut is_unsafe = false;
        let mut extern_abi = Maybe::None;
        let mut name = None;
        let mut generics = List::new();
        let mut params = List::new();
        let mut return_type = Maybe::None;
        let mut body = Maybe::None;
        let mut where_clause = Maybe::None;
        let mut requires = List::new();
        let mut ensures = List::new();
        let mut contexts = List::new();
        let mut attributes = List::new();
        let mut seen_arrow = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        SyntaxKind::ASYNC_KW => is_async = true,
                        SyntaxKind::META_KW => is_meta = true,
                        SyntaxKind::PURE_KW => is_pure = true,
                        SyntaxKind::EXTERN_KW => extern_abi = Maybe::Some(Text::from("C")),
                        SyntaxKind::COFIX_KW => is_cofix = true,
                        SyntaxKind::UNSAFE_KW => is_unsafe = true,
                        SyntaxKind::IDENT => {
                            if name.is_none() {
                                name = Some(self.token_to_ident(&token));
                            }
                        }
                        SyntaxKind::STAR => is_generator = true,
                        SyntaxKind::ARROW => seen_arrow = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            generics = self.convert_generic_params(&child_node);
                        }
                        SyntaxKind::PARAM_LIST => {
                            params = self.convert_param_list(&child_node);
                        }
                        SyntaxKind::WHERE_CLAUSE => {
                            where_clause = self.convert_where_clause(&child_node);
                        }
                        SyntaxKind::REQUIRES_CLAUSE => {
                            if let Some(expr) = self.convert_clause_expr(&child_node) {
                                requires.push(expr);
                            }
                        }
                        SyntaxKind::ENSURES_CLAUSE => {
                            if let Some(expr) = self.convert_clause_expr(&child_node) {
                                ensures.push(expr);
                            }
                        }
                        SyntaxKind::USING_CLAUSE => {
                            contexts = self.convert_using_clause(&child_node);
                        }
                        SyntaxKind::BLOCK => {
                            body = Maybe::Some(FunctionBody::Block(
                                self.convert_block(&child_node)
                            ));
                        }
                        SyntaxKind::ATTRIBUTE => {
                            if let Some(attr) = self.convert_attribute(&child_node) {
                                attributes.push(attr);
                            }
                        }
                        // Explicit type node kinds that EventBasedParser emits
                        SyntaxKind::PATH_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::TUPLE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE
                            | SyntaxKind::REFINED_TYPE
                            | SyntaxKind::GENERIC_TYPE => {
                            if seen_arrow && return_type.is_none() {
                                if let Some(ty) = self.convert_type(&child_node) {
                                    return_type = Maybe::Some(ty);
                                }
                            }
                        }
                        kind if kind.can_start_type() && seen_arrow && return_type.is_none() => {
                            if let Some(ty) = self.convert_type(&child_node) {
                                return_type = Maybe::Some(ty);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let name = name.unwrap_or_else(|| Ident::new(Text::from("_error_"), span));

        // Check for @transparent attribute
        let is_transparent = attributes.iter().any(|a| a.name.as_str() == "transparent");

        Some(FunctionDecl {
            visibility,
            is_async,
            is_meta,
            // Stage level: 0 for runtime, 1 for meta functions
            // CST parser doesn't yet support meta(N) syntax, so default to 1 if meta
            stage_level: if is_meta { 1 } else { 0 },
            is_pure,
            is_generator,
            is_cofix,
            is_unsafe,
            is_transparent,
            extern_abi,
            is_variadic: false,
            name,
            generics,
            params,
            return_type,
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts,
            generic_where_clause: where_clause.clone(),
            meta_where_clause: Maybe::None,
            requires,
            ensures,
            attributes,
            body,
            span,
        })
    }

    /// Convert a TYPE_DEF node to a TypeDecl.
    fn convert_type_def(&mut self, node: &SyntaxNode) -> Option<TypeDecl> {
        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut name = None;
        let mut generics = List::new();
        let mut body = None;
        let mut where_clause = Maybe::None;
        let mut attributes = List::new();

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        SyntaxKind::IDENT => {
                            if name.is_none() {
                                name = Some(self.token_to_ident(&token));
                            }
                        }
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            generics = self.convert_generic_params(&child_node);
                        }
                        SyntaxKind::WHERE_CLAUSE => {
                            where_clause = self.convert_where_clause(&child_node);
                        }
                        SyntaxKind::FIELD_LIST => {
                            body = Some(self.convert_record_body(&child_node));
                        }
                        SyntaxKind::VARIANT_LIST => {
                            body = Some(self.convert_variant_body(&child_node));
                        }
                        // Explicit type node kinds that EventBasedParser emits
                        SyntaxKind::PATH_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::TUPLE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE
                            | SyntaxKind::REFINED_TYPE
                            | SyntaxKind::GENERIC_TYPE if body.is_none() => {
                            if let Some(ty) = self.convert_type(&child_node) {
                                body = Some(TypeDeclBody::Alias(ty));
                            }
                        }
                        kind if kind.can_start_type() && body.is_none() => {
                            if let Some(ty) = self.convert_type(&child_node) {
                                body = Some(TypeDeclBody::Alias(ty));
                            }
                        }
                        SyntaxKind::ATTRIBUTE => {
                            if let Some(attr) = self.convert_attribute(&child_node) {
                                attributes.push(attr);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let name = name.unwrap_or_else(|| Ident::new(Text::from("_error_"), span));
        let body = body.unwrap_or(TypeDeclBody::Unit);

        Some(TypeDecl {
            visibility,
            name,
            generics,
            body,
            resource_modifier: Maybe::None,
            generic_where_clause: where_clause, // Use the parsed where clause
            meta_where_clause: Maybe::None,
            attributes,
            span,
        })
    }

    // ==================== Expression Conversion ====================

    /// Convert an expression node to Expr.
    fn convert_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());

        let kind = match node.kind() {
            SyntaxKind::LITERAL_EXPR => {
                let lit = self.convert_literal(node)?;
                ExprKind::Literal(lit)
            }
            SyntaxKind::PATH_EXPR => {
                let path = self.convert_path(node)?;
                ExprKind::Path(path)
            }
            SyntaxKind::BINARY_EXPR => return self.convert_binary_expr(node),
            SyntaxKind::PREFIX_EXPR => return self.convert_prefix_expr(node),
            SyntaxKind::CALL_EXPR => return self.convert_call_expr(node),
            SyntaxKind::METHOD_CALL_EXPR => return self.convert_method_call_expr(node),
            SyntaxKind::FIELD_EXPR => return self.convert_field_expr(node),
            SyntaxKind::INDEX_EXPR => return self.convert_index_expr(node),
            SyntaxKind::IF_EXPR => return self.convert_if_expr(node),
            SyntaxKind::MATCH_EXPR => return self.convert_match_expr(node),
            SyntaxKind::BLOCK_EXPR | SyntaxKind::BLOCK => {
                let block = self.convert_block(node);
                ExprKind::Block(block)
            }
            SyntaxKind::TUPLE_EXPR => return self.convert_tuple_expr(node),
            SyntaxKind::ARRAY_EXPR => return self.convert_array_expr(node),
            SyntaxKind::RECORD_EXPR => return self.convert_record_expr(node),
            SyntaxKind::CLOSURE_EXPR => return self.convert_closure_expr(node),
            SyntaxKind::ASYNC_EXPR => return self.convert_async_expr(node),
            SyntaxKind::AWAIT_EXPR => return self.convert_await_expr(node),
            // Return, break, continue are handled as statements, not expressions
            // They become expression statements when used as expressions
            SyntaxKind::LOOP_EXPR => return self.convert_loop_expr(node),
            SyntaxKind::WHILE_EXPR => return self.convert_while_expr(node),
            SyntaxKind::FOR_EXPR => return self.convert_for_expr(node),
            SyntaxKind::RANGE_EXPR => return self.convert_range_expr(node),
            SyntaxKind::PAREN_EXPR => {
                for child in node.child_nodes() {
                    if let Some(expr) = self.convert_expr(&child) {
                        return Some(expr);
                    }
                }
                return None;
            }
            SyntaxKind::TRY_EXPR => return self.convert_try_expr(node),
            SyntaxKind::THROW_EXPR => return self.convert_throw_expr(node),
            SyntaxKind::PIPELINE_EXPR => return self.convert_pipeline_expr(node),
            SyntaxKind::REF_EXPR => return self.convert_ref_expr(node),
            SyntaxKind::DEREF_EXPR => return self.convert_deref_expr(node),
            SyntaxKind::CAST_EXPR => return self.convert_cast_expr(node),
            SyntaxKind::ERROR => {
                self.error_at(node.text_range(), "syntax error in expression");
                return None;
            }
            _ => {
                for child in node.child_nodes() {
                    if let Some(expr) = self.convert_expr(&child) {
                        return Some(expr);
                    }
                }
                return None;
            }
        };

        Some(Expr::new(kind, span))
    }

    /// Convert a binary expression.
    fn convert_binary_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut left = None;
        let mut op = None;
        let mut right = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if op.is_none() && left.is_some() {
                        op = self.token_to_binop(&token);
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if left.is_none() {
                        left = self.convert_expr(&child_node);
                    } else if right.is_none() {
                        right = self.convert_expr(&child_node);
                    }
                }
            }
        }

        Some(Expr::new(
            ExprKind::Binary {
                op: op?,
                left: Heap::new(left?),
                right: Heap::new(right?),
            },
            span,
        ))
    }

    /// Convert a prefix/unary expression.
    fn convert_prefix_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut op = None;
        let mut operand = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if op.is_none() {
                        op = self.token_to_unop(&token);
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if operand.is_none() {
                        operand = self.convert_expr(&child_node);
                    }
                }
            }
        }

        Some(Expr::new(
            ExprKind::Unary {
                op: op?,
                expr: Heap::new(operand?),
            },
            span,
        ))
    }

    /// Convert a call expression.
    fn convert_call_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut callee = None;
        let mut args = List::new();

        for child in node.child_nodes() {
            if callee.is_none() {
                callee = self.convert_expr(&child);
            } else if child.kind() == SyntaxKind::ARG_LIST {
                for arg in child.child_nodes() {
                    if let Some(expr) = self.convert_expr(&arg) {
                        args.push(expr);
                    }
                }
            }
        }

        Some(Expr::new(
            ExprKind::Call {
                func: Heap::new(callee?),
                type_args: List::new(),
                args,
            },
            span,
        ))
    }

    /// Convert a method call expression.
    fn convert_method_call_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut receiver = None;
        let mut method = None;
        let mut args = List::new();

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && receiver.is_some() && method.is_none() {
                        method = Some(self.token_to_ident(&token));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if receiver.is_none() {
                        receiver = self.convert_expr(&child_node);
                    } else if child_node.kind() == SyntaxKind::ARG_LIST {
                        for arg in child_node.child_nodes() {
                            if let Some(expr) = self.convert_expr(&arg) {
                                args.push(expr);
                            }
                        }
                    }
                }
            }
        }

        Some(Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(receiver?),
                method: method?,
                type_args: List::new(),
                args,
            },
            span,
        ))
    }

    /// Convert a field expression.
    fn convert_field_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut receiver = None;
        let mut field = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && receiver.is_some() {
                        field = Some(self.token_to_ident(&token));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if receiver.is_none() {
                        receiver = self.convert_expr(&child_node);
                    }
                }
            }
        }

        Some(Expr::new(
            ExprKind::Field {
                expr: Heap::new(receiver?),
                field: field?,
            },
            span,
        ))
    }

    /// Convert an index expression.
    fn convert_index_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut base = None;
        let mut index = None;

        for child in node.child_nodes() {
            if base.is_none() {
                base = self.convert_expr(&child);
            } else if index.is_none() {
                index = self.convert_expr(&child);
            }
        }

        Some(Expr::new(
            ExprKind::Index {
                expr: Heap::new(base?),
                index: Heap::new(index?),
            },
            span,
        ))
    }

    /// Convert an if expression.
    fn convert_if_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut condition = None;
        let mut then_branch = None;
        let mut else_branch = Maybe::None;

        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::BLOCK if then_branch.is_none() && condition.is_some() => {
                    then_branch = Some(self.convert_block(&child));
                }
                SyntaxKind::BLOCK if then_branch.is_some() => {
                    else_branch = Maybe::Some(self.convert_block(&child));
                }
                SyntaxKind::IF_EXPR => {
                    if let Some(elif) = self.convert_if_expr(&child) {
                        else_branch = Maybe::Some(Block::new(
                            List::new(),
                            Maybe::Some(Heap::new(elif)),
                            self.range_to_span(child.text_range()),
                        ));
                    }
                }
                kind if kind.can_start_expr() && condition.is_none() => {
                    condition = self.convert_expr(&child);
                }
                _ => {}
            }
        }

        let cond = condition?;
        let cond_span = cond.span;
        let if_condition = IfCondition {
            conditions: smallvec![ConditionKind::Expr(cond)],
            span: cond_span,
        };

        Some(Expr::new(
            ExprKind::If {
                condition: Heap::new(if_condition),
                then_branch: then_branch?,
                else_branch: else_branch.map(|b| Heap::new(Expr::new(ExprKind::Block(b), span))),
            },
            span,
        ))
    }

    /// Convert a match expression.
    fn convert_match_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut scrutinee = None;
        let mut arms = List::new();

        for child in node.child_nodes() {
            if scrutinee.is_none() {
                scrutinee = self.convert_expr(&child);
            } else if child.kind() == SyntaxKind::MATCH_ARM_LIST {
                for arm_node in child.child_nodes() {
                    if arm_node.kind() == SyntaxKind::MATCH_ARM {
                        if let Some(arm) = self.convert_match_arm(&arm_node) {
                            arms.push(arm);
                        }
                    }
                }
            }
        }

        Some(Expr::new(
            ExprKind::Match {
                expr: Heap::new(scrutinee?),
                arms,
            },
            span,
        ))
    }

    /// Convert a match arm.
    fn convert_match_arm(&mut self, node: &SyntaxNode) -> Option<MatchArm> {
        let span = self.range_to_span(node.text_range());
        let mut pattern = None;
        let guard: Option<Heap<Expr>> = None;
        let mut body = None;

        for child in node.child_nodes() {
            if pattern.is_none() {
                pattern = self.convert_pattern(&child);
            } else if body.is_none() {
                body = self.convert_expr(&child);
            }
        }

        Some(MatchArm {
            pattern: pattern?,
            guard,
            with_clause: Maybe::None,
            body: Heap::new(body?),
            attributes: List::new(),
            span,
        })
    }

    /// Convert a tuple expression.
    fn convert_tuple_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut elements = List::new();

        for child in node.child_nodes() {
            if let Some(expr) = self.convert_expr(&child) {
                elements.push(expr);
            }
        }

        Some(Expr::new(ExprKind::Tuple(elements), span))
    }

    /// Convert a literal.
    fn convert_literal(&mut self, node: &SyntaxNode) -> Option<Literal> {
        let span = self.range_to_span(node.text_range());

        for child in node.child_tokens() {
            match child.kind() {
                SyntaxKind::INT_LITERAL => {
                    let text = child.text();
                    let value = self.parse_int_literal(text);
                    return Some(Literal::new(
                        LiteralKind::Int(verum_ast::IntLit::new(value)),
                        span,
                    ));
                }
                SyntaxKind::FLOAT_LITERAL => {
                    let text = child.text();
                    let value = text.parse().unwrap_or(0.0);
                    return Some(Literal::new(
                        LiteralKind::Float(verum_ast::FloatLit::new(value)),
                        span,
                    ));
                }
                SyntaxKind::STRING_LITERAL => {
                    let text = child.text();
                    let value = self.parse_string_literal(text);
                    return Some(Literal::new(
                        LiteralKind::Text(StringLit::Regular(value)),
                        span,
                    ));
                }
                SyntaxKind::CHAR_LITERAL => {
                    let text = child.text();
                    let value = self.parse_char_literal(text);
                    return Some(Literal::new(LiteralKind::Char(value), span));
                }
                SyntaxKind::TRUE_KW => {
                    return Some(Literal::new(LiteralKind::Bool(true), span));
                }
                SyntaxKind::FALSE_KW => {
                    return Some(Literal::new(LiteralKind::Bool(false), span));
                }
                _ => {}
            }
        }

        None
    }

    // ==================== Statement Conversion ====================

    /// Convert a statement node to Stmt.
    fn convert_stmt(&mut self, node: &SyntaxNode) -> Option<Stmt> {
        let span = self.range_to_span(node.text_range());

        let kind = match node.kind() {
            SyntaxKind::LET_STMT => return self.convert_let_stmt(node),
            SyntaxKind::EXPR_STMT => {
                for child in node.child_nodes() {
                    if let Some(expr) = self.convert_expr(&child) {
                        let has_semi = self.has_token(node, SyntaxKind::SEMICOLON);
                        return Some(Stmt::new(StmtKind::Expr { expr, has_semi }, span));
                    }
                }
                return None;
            }
            SyntaxKind::DEFER_STMT => {
                let expr = node.child_nodes().find_map(|n| self.convert_expr(&n))?;
                StmtKind::Defer(expr)
            }
            SyntaxKind::ERRDEFER_STMT => {
                let expr = node.child_nodes().find_map(|n| self.convert_expr(&n))?;
                StmtKind::Errdefer(expr)
            }
            SyntaxKind::PROVIDE_STMT => return self.convert_provide_stmt(node),
            SyntaxKind::ERROR => {
                self.error_at(node.text_range(), "syntax error in statement");
                return None;
            }
            _ => {
                if let Some(expr) = self.convert_expr(node) {
                    StmtKind::Expr { expr, has_semi: false }
                } else {
                    return None;
                }
            }
        };

        Some(Stmt::new(kind, span))
    }

    /// Convert a let statement.
    fn convert_let_stmt(&mut self, node: &SyntaxNode) -> Option<Stmt> {
        let span = self.range_to_span(node.text_range());
        let mut pattern = None;
        let mut ty = Maybe::None;
        let mut init = Maybe::None;

        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::IDENT_PAT | SyntaxKind::TUPLE_PAT |
                SyntaxKind::RECORD_PAT | SyntaxKind::WILDCARD_PAT => {
                    if pattern.is_none() {
                        pattern = self.convert_pattern(&child);
                    }
                }
                kind if kind.can_start_type() && ty.is_none() => {
                    ty = self.convert_type(&child).map(Maybe::Some).unwrap_or(Maybe::None);
                }
                kind if kind.can_start_expr() && pattern.is_some() => {
                    init = self.convert_expr(&child).map(Maybe::Some).unwrap_or(Maybe::None);
                }
                _ => {}
            }
        }

        Some(Stmt::new(
            StmtKind::Let {
                pattern: pattern?,
                ty,
                value: init,
            },
            span,
        ))
    }

    /// Convert a provide statement.
    fn convert_provide_stmt(&mut self, node: &SyntaxNode) -> Option<Stmt> {
        let span = self.range_to_span(node.text_range());
        let mut context_name = None;
        let mut value = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && context_name.is_none() {
                        context_name = Some(token.text().to_string());
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if value.is_none() {
                        value = self.convert_expr(&child_node);
                    }
                }
            }
        }

        Some(Stmt::new(
            StmtKind::Provide {
                context: Text::from(context_name?),
                alias: Maybe::None,
                value: Heap::new(value?),
            },
            span,
        ))
    }

    /// Convert a block.
    fn convert_block(&mut self, node: &SyntaxNode) -> Block {
        let span = self.range_to_span(node.text_range());
        let mut stmts = List::new();
        let mut expr = Maybe::None;

        let children: Vec<_> = node.child_nodes().collect();
        let len = children.len();

        for (i, child) in children.into_iter().enumerate() {
            let is_last = i == len - 1;

            match child.kind() {
                SyntaxKind::LET_STMT | SyntaxKind::DEFER_STMT | SyntaxKind::ERRDEFER_STMT |
                SyntaxKind::PROVIDE_STMT => {
                    if let Some(stmt) = self.convert_stmt(&child) {
                        stmts.push(stmt);
                    }
                }
                SyntaxKind::EXPR_STMT => {
                    if let Some(stmt) = self.convert_stmt(&child) {
                        stmts.push(stmt);
                    }
                }
                kind if kind.can_start_expr() => {
                    if let Some(e) = self.convert_expr(&child) {
                        if is_last && !self.has_trailing_semi(&child) {
                            expr = Maybe::Some(Heap::new(e));
                        } else {
                            stmts.push(Stmt::new(
                                StmtKind::Expr { expr: e, has_semi: true },
                                self.range_to_span(child.text_range()),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }

        Block::new(stmts, expr, span)
    }

    // ==================== Type Conversion ====================

    /// Convert a type node to Type.
    fn convert_type(&mut self, node: &SyntaxNode) -> Option<Type> {
        let span = self.range_to_span(node.text_range());

        let kind = match node.kind() {
            SyntaxKind::PATH_TYPE => {
                let path = self.convert_path(node)?;
                TypeKind::Path(path)
            }
            SyntaxKind::REFERENCE_TYPE => return self.convert_reference_type(node),
            SyntaxKind::TUPLE_TYPE => return self.convert_tuple_type(node),
            SyntaxKind::ARRAY_TYPE => return self.convert_array_type(node),
            SyntaxKind::FUNCTION_TYPE => return self.convert_function_type(node),
            SyntaxKind::REFINED_TYPE => return self.convert_refined_type(node),
            SyntaxKind::GENERIC_TYPE => return self.convert_generic_type(node),
            SyntaxKind::INFER_TYPE => TypeKind::Inferred,
            SyntaxKind::ERROR => {
                self.error_at(node.text_range(), "syntax error in type");
                return None;
            }
            _ => {
                if let Some(path) = self.convert_path(node) {
                    TypeKind::Path(path)
                } else {
                    return None;
                }
            }
        };

        Some(Type::new(kind, span))
    }

    // ==================== Pattern Conversion ====================

    /// Convert a pattern node to Pattern.
    fn convert_pattern(&mut self, node: &SyntaxNode) -> Option<Pattern> {
        let span = self.range_to_span(node.text_range());

        let kind = match node.kind() {
            SyntaxKind::WILDCARD_PAT => PatternKind::Wildcard,
            SyntaxKind::IDENT_PAT => {
                let name = self.find_ident_in_node(node)?;
                let is_mut = self.has_token(node, SyntaxKind::MUT_KW);
                let by_ref = self.has_token(node, SyntaxKind::REF_KW);
                PatternKind::Ident {
                    by_ref,
                    mutable: is_mut,
                    name,
                    subpattern: Maybe::None,
                }
            }
            SyntaxKind::LITERAL_PAT => {
                let lit = self.convert_literal_from_pattern(node)?;
                PatternKind::Literal(lit)
            }
            SyntaxKind::TUPLE_PAT => {
                let patterns = self.convert_patterns_list(node);
                PatternKind::Tuple(patterns)
            }
            SyntaxKind::RECORD_PAT => return self.convert_record_pattern(node),
            SyntaxKind::VARIANT_PAT => return self.convert_variant_pattern(node),
            SyntaxKind::REST_PAT => PatternKind::Rest,
            SyntaxKind::OR_PAT => {
                let patterns = self.convert_patterns_list(node);
                PatternKind::Or(patterns)
            }
            SyntaxKind::ERROR => {
                self.error_at(node.text_range(), "syntax error in pattern");
                return None;
            }
            _ => return None,
        };

        Some(Pattern::new(kind, span))
    }

    // ==================== Helper Methods ====================

    /// Convert a TextRange to Span.
    fn range_to_span(&self, range: TextRange) -> Span {
        Span::new(range.start(), range.end(), self.file_id)
    }

    /// Record an error.
    fn error_at(&mut self, range: TextRange, message: impl Into<String>) {
        self.errors.push(ParseError::invalid_syntax(
            message.into(),
            self.range_to_span(range),
        ));
    }

    /// Convert a token to an identifier.
    fn token_to_ident(&self, token: &SyntaxToken) -> Ident {
        let span = self.range_to_span(token.text_range());
        Ident::new(Text::from(token.text()), span)
    }

    /// Check if node contains a specific token kind.
    fn has_token(&self, node: &SyntaxNode, kind: SyntaxKind) -> bool {
        node.child_tokens().any(|t| t.kind() == kind)
    }

    /// Check if expression has trailing semicolon.
    fn has_trailing_semi(&self, node: &SyntaxNode) -> bool {
        let text = node.text();
        text.trim_end().ends_with(';')
    }

    /// Find an identifier in a node.
    fn find_ident_in_node(&self, node: &SyntaxNode) -> Option<Ident> {
        node.child_tokens()
            .find(|t| t.kind() == SyntaxKind::IDENT)
            .map(|t| self.token_to_ident(&t))
    }

    /// Check if an attribute is an inner attribute.
    fn is_inner_attribute(&self, node: &SyntaxNode) -> bool {
        node.child_tokens().any(|t| t.kind() == SyntaxKind::BANG)
    }

    /// Collect attributes from a node.
    fn collect_attributes(&mut self, node: &SyntaxNode) -> List<Attribute> {
        let mut attrs = List::new();
        for child in node.child_nodes() {
            if child.kind() == SyntaxKind::ATTRIBUTE {
                if let Some(attr) = self.convert_attribute(&child) {
                    attrs.push(attr);
                }
            }
        }
        attrs
    }

    /// Find a label in a node as Maybe<Text>.
    fn find_label_text(&self, node: &SyntaxNode) -> Maybe<Text> {
        node.child_nodes()
            .find(|n| n.kind() == SyntaxKind::LABEL)
            .and_then(|n| self.find_ident_in_node(&n))
            .map(|ident| Maybe::Some(ident.name))
            .unwrap_or(Maybe::None)
    }

    /// Convert token to binary operator.
    fn token_to_binop(&self, token: &SyntaxToken) -> Option<BinOp> {
        match token.kind() {
            SyntaxKind::PLUS => Some(BinOp::Add),
            SyntaxKind::MINUS => Some(BinOp::Sub),
            SyntaxKind::STAR => Some(BinOp::Mul),
            SyntaxKind::SLASH => Some(BinOp::Div),
            SyntaxKind::PERCENT => Some(BinOp::Rem),
            SyntaxKind::STAR_STAR => Some(BinOp::Pow),
            SyntaxKind::EQ_EQ => Some(BinOp::Eq),
            SyntaxKind::BANG_EQ => Some(BinOp::Ne),
            SyntaxKind::L_ANGLE => Some(BinOp::Lt),
            SyntaxKind::R_ANGLE => Some(BinOp::Gt),
            SyntaxKind::LT_EQ => Some(BinOp::Le),
            SyntaxKind::GT_EQ => Some(BinOp::Ge),
            SyntaxKind::AMP_AMP => Some(BinOp::And),
            SyntaxKind::PIPE_PIPE => Some(BinOp::Or),
            SyntaxKind::AMP => Some(BinOp::BitAnd),
            SyntaxKind::PIPE => Some(BinOp::BitOr),
            SyntaxKind::CARET => Some(BinOp::BitXor),
            SyntaxKind::LT_LT => Some(BinOp::Shl),
            SyntaxKind::GT_GT => Some(BinOp::Shr),
            _ => None,
        }
    }

    /// Convert token to unary operator.
    fn token_to_unop(&self, token: &SyntaxToken) -> Option<UnOp> {
        match token.kind() {
            SyntaxKind::MINUS => Some(UnOp::Neg),
            SyntaxKind::BANG => Some(UnOp::Not),
            SyntaxKind::TILDE => Some(UnOp::BitNot),
            SyntaxKind::STAR => Some(UnOp::Deref),
            SyntaxKind::AMP => Some(UnOp::Ref),
            _ => None,
        }
    }

    /// Parse an integer literal.
    fn parse_int_literal(&self, text: &str) -> i128 {
        let text = text.replace('_', "");
        if text.starts_with("0x") || text.starts_with("0X") {
            i128::from_str_radix(&text[2..], 16).unwrap_or(0)
        } else if text.starts_with("0b") || text.starts_with("0B") {
            i128::from_str_radix(&text[2..], 2).unwrap_or(0)
        } else if text.starts_with("0o") || text.starts_with("0O") {
            i128::from_str_radix(&text[2..], 8).unwrap_or(0)
        } else {
            text.parse().unwrap_or(0)
        }
    }

    /// Parse a string literal.
    fn parse_string_literal(&self, text: &str) -> Text {
        let inner = if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
            &text[1..text.len()-1]
        } else {
            text
        };
        Text::from(self.unescape_string(inner))
    }

    /// Parse a char literal.
    fn parse_char_literal(&self, text: &str) -> char {
        let inner = if text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2 {
            &text[1..text.len()-1]
        } else {
            text
        };
        self.unescape_string(inner).chars().next().unwrap_or('\0')
    }

    /// Unescape string escape sequences.
    fn unescape_string(&self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('r') => result.push('\r'),
                    Some('t') => result.push('\t'),
                    Some('\\') => result.push('\\'),
                    Some('\'') => result.push('\''),
                    Some('"') => result.push('"'),
                    Some('0') => result.push('\0'),
                    Some(c) => {
                        result.push('\\');
                        result.push(c);
                    }
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    // ==================== Stub Methods ====================

    /// Convert an ATTRIBUTE node to an Attribute.
    ///
    /// Attributes in Verum use the `@` prefix syntax:
    /// - `@inline` - simple attribute without arguments
    /// - `@derive(Clone, Debug)` - attribute with expression arguments
    /// - `@verify(runtime)` - attribute with single argument
    ///
    /// The ATTRIBUTE node structure is:
    /// - AT token (`@`)
    /// - IDENT token (attribute name)
    /// - Optionally: L_PAREN, argument tokens, R_PAREN
    fn convert_attribute(&mut self, node: &SyntaxNode) -> Option<Attribute> {
        let span = self.range_to_span(node.text_range());
        let mut name: Option<Text> = None;
        let mut args: List<Expr> = List::new();
        let mut in_parens = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::IDENT if name.is_none() => {
                            // First identifier is the attribute name
                            name = Some(Text::from(token.text()));
                        }
                        SyntaxKind::L_PAREN => {
                            in_parens = true;
                        }
                        SyntaxKind::R_PAREN => {
                            in_parens = false;
                        }
                        // Handle simple identifier arguments (e.g., @derive(Clone, Debug))
                        SyntaxKind::IDENT if in_parens => {
                            let arg_span = self.range_to_span(token.text_range());
                            let ident = Ident::new(Text::from(token.text()), arg_span);
                            let path = Path::from_ident(ident);
                            args.push(Expr::new(ExprKind::Path(path), arg_span));
                        }
                        // Handle literal arguments
                        SyntaxKind::INT_LITERAL if in_parens => {
                            let arg_span = self.range_to_span(token.text_range());
                            let value = self.parse_int_literal(token.text());
                            let lit = Literal::new(
                                LiteralKind::Int(verum_ast::IntLit::new(value)),
                                arg_span,
                            );
                            args.push(Expr::new(ExprKind::Literal(lit), arg_span));
                        }
                        SyntaxKind::STRING_LITERAL if in_parens => {
                            let arg_span = self.range_to_span(token.text_range());
                            let value = self.parse_string_literal(token.text());
                            let lit = Literal::new(
                                LiteralKind::Text(StringLit::Regular(value)),
                                arg_span,
                            );
                            args.push(Expr::new(ExprKind::Literal(lit), arg_span));
                        }
                        SyntaxKind::TRUE_KW if in_parens => {
                            let arg_span = self.range_to_span(token.text_range());
                            let lit = Literal::new(LiteralKind::Bool(true), arg_span);
                            args.push(Expr::new(ExprKind::Literal(lit), arg_span));
                        }
                        SyntaxKind::FALSE_KW if in_parens => {
                            let arg_span = self.range_to_span(token.text_range());
                            let lit = Literal::new(LiteralKind::Bool(false), arg_span);
                            args.push(Expr::new(ExprKind::Literal(lit), arg_span));
                        }
                        // Skip punctuation tokens (AT, COMMA, etc.)
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    // Handle complex expression arguments (e.g., function calls, paths)
                    if in_parens {
                        if let Some(expr) = self.convert_expr(&child_node) {
                            args.push(expr);
                        }
                    }
                }
            }
        }

        // Attribute must have a name
        let name = name?;

        // Convert args list to Maybe<List<Expr>>
        let args = if args.is_empty() {
            Maybe::None
        } else {
            Maybe::Some(args)
        };

        Some(Attribute::new(name, args, span))
    }
    /// Convert a PROTOCOL_DEF node to a ProtocolDecl.
    ///
    /// Parses protocol definitions of the form:
    /// - `protocol { ... }` (inside type is)
    /// - `protocol extends Base { ... }`
    /// - `protocol extends A + B where T: Clone { ... }`
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// protocol_def = 'protocol' , [ 'extends' , trait_path , { '+' , trait_path } ] ,
    ///                [ generic_where_clause ] ,
    ///                '{' , protocol_items , '}' ;
    /// protocol_item = protocol_function | protocol_type | protocol_const ;
    /// ```
    fn convert_protocol_def(&mut self, node: &SyntaxNode) -> Option<ProtocolDecl> {
        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut is_context = false;
        let mut name = None;
        let mut generics = List::new();
        let mut bounds = List::new(); // extends bounds
        let mut items = List::new();
        let mut generic_where_clause = Maybe::None;
        let meta_where_clause = Maybe::None;
        let mut seen_extends = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        SyntaxKind::CONTEXT_KW => is_context = true,
                        SyntaxKind::IDENT => {
                            if name.is_none() {
                                name = Some(self.token_to_ident(&token));
                            }
                        }
                        SyntaxKind::EXTENDS_KW => seen_extends = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            generics = self.convert_generic_params(&child_node);
                        }
                        SyntaxKind::WHERE_CLAUSE => {
                            generic_where_clause = self.convert_where_clause(&child_node);
                        }
                        SyntaxKind::PROTOCOL_ITEM => {
                            if let Some(item) = self.convert_protocol_item(&child_node) {
                                items.push(item);
                            }
                        }
                        // Handle function definitions directly (may appear without PROTOCOL_ITEM wrapper)
                        SyntaxKind::FN_DEF => {
                            if let Some(func) = self.convert_function(&child_node) {
                                let item_span = self.range_to_span(child_node.text_range());
                                items.push(ProtocolItem {
                                    kind: ProtocolItemKind::Function {
                                        decl: func,
                                        default_impl: Maybe::None,
                                    },
                                    span: item_span,
                                });
                            }
                        }
                        // Handle extends bounds - paths after 'extends' keyword
                        SyntaxKind::PATH | SyntaxKind::PATH_TYPE if seen_extends => {
                            if let Some(ty) = self.convert_type(&child_node) {
                                bounds.push(ty);
                            }
                        }
                        // Handle TYPE_BOUND for extends bounds
                        SyntaxKind::TYPE_BOUND if seen_extends => {
                            if let Some(ty) = self.convert_type(&child_node) {
                                bounds.push(ty);
                            }
                        }
                        SyntaxKind::BOUND_LIST if seen_extends => {
                            for bound_child in child_node.child_nodes() {
                                if let Some(ty) = self.convert_type(&bound_child) {
                                    bounds.push(ty);
                                }
                            }
                        }
                        // Associated type definitions
                        SyntaxKind::ASSOC_TYPE | SyntaxKind::TYPE_DEF => {
                            if let Some(item) = self.convert_protocol_type_item(&child_node) {
                                items.push(item);
                            }
                        }
                        // Associated const definitions
                        SyntaxKind::ASSOC_CONST | SyntaxKind::CONST_DEF => {
                            if let Some(item) = self.convert_protocol_const_item(&child_node) {
                                items.push(item);
                            }
                        }
                        SyntaxKind::ATTRIBUTE => {
                            // Attributes on protocol - handled by caller
                        }
                        _ => {}
                    }
                }
            }
        }

        // Name is optional for protocol definitions inside `type X is protocol { ... }`
        // In that case, the name comes from the type definition
        let name = name.unwrap_or_else(|| Ident::new(Text::from("_protocol_"), span));

        Some(ProtocolDecl {
            visibility,
            is_context,
            name,
            generics,
            bounds,
            items,
            generic_where_clause,
            meta_where_clause,
            span,
        })
    }

    /// Convert a PROTOCOL_ITEM node to ProtocolItem.
    fn convert_protocol_item(&mut self, node: &SyntaxNode) -> Option<ProtocolItem> {
        let span = self.range_to_span(node.text_range());

        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::FN_DEF | SyntaxKind::PROTOCOL_FN => {
                    if let Some(func) = self.convert_function(&child) {
                        // Check for default implementation
                        let default_impl = self.find_default_impl(&child);
                        return Some(ProtocolItem {
                            kind: ProtocolItemKind::Function {
                                decl: func,
                                default_impl,
                            },
                            span,
                        });
                    }
                }
                SyntaxKind::ASSOC_TYPE | SyntaxKind::TYPE_DEF => {
                    return self.convert_protocol_type_item(&child);
                }
                SyntaxKind::ASSOC_CONST | SyntaxKind::CONST_DEF => {
                    return self.convert_protocol_const_item(&child);
                }
                _ => {}
            }
        }

        None
    }

    /// Convert an associated type definition in a protocol.
    fn convert_protocol_type_item(&mut self, node: &SyntaxNode) -> Option<ProtocolItem> {
        let span = self.range_to_span(node.text_range());
        let mut name = None;
        let mut type_params = List::new();
        let mut bounds = List::new();
        let mut where_clause = Maybe::None;
        let mut default_type = Maybe::None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && name.is_none() {
                        name = Some(self.token_to_ident(&token));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            type_params = self.convert_generic_params(&child_node);
                        }
                        SyntaxKind::BOUND_LIST | SyntaxKind::TYPE_BOUND => {
                            for bound_child in child_node.child_nodes() {
                                if let Some(path) = self.convert_path(&bound_child) {
                                    bounds.push(path);
                                }
                            }
                            if child_node.kind() == SyntaxKind::TYPE_BOUND {
                                if let Some(path) = self.convert_path(&child_node) {
                                    bounds.push(path);
                                }
                            }
                        }
                        SyntaxKind::WHERE_CLAUSE => {
                            where_clause = self.convert_where_clause(&child_node);
                        }
                        kind if kind.can_start_type() && default_type.is_none() => {
                            if let Some(ty) = self.convert_type(&child_node) {
                                default_type = Maybe::Some(ty);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let name = name?;
        Some(ProtocolItem {
            kind: ProtocolItemKind::Type {
                name,
                type_params,
                bounds,
                where_clause,
                default_type,
            },
            span,
        })
    }

    /// Convert an associated const definition in a protocol.
    fn convert_protocol_const_item(&mut self, node: &SyntaxNode) -> Option<ProtocolItem> {
        let span = self.range_to_span(node.text_range());
        let mut name = None;
        let mut ty = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && name.is_none() {
                        name = Some(self.token_to_ident(&token));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if child_node.kind().can_start_type() && ty.is_none() {
                        ty = self.convert_type(&child_node);
                    }
                }
            }
        }

        let name = name?;
        let ty = ty?;
        Some(ProtocolItem {
            kind: ProtocolItemKind::Const { name, ty },
            span,
        })
    }

    /// Find a default implementation block in a protocol method.
    fn find_default_impl(&mut self, node: &SyntaxNode) -> Maybe<FunctionBody> {
        for child in node.child_nodes() {
            if child.kind() == SyntaxKind::BLOCK {
                return Maybe::Some(FunctionBody::Block(self.convert_block(&child)));
            }
        }
        Maybe::None
    }

    /// Convert an IMPL_BLOCK node to an ImplDecl.
    ///
    /// Parses implementation blocks of the form:
    /// - Inherent impl: `implement Type { ... }`
    /// - Protocol impl: `implement Protocol for Type { ... }`
    /// - With generics: `implement<T> Protocol for Type<T> { ... }`
    /// - With where clause: `implement<T> Protocol for Type<T> where T: Clone { ... }`
    fn convert_impl_block(&mut self, node: &SyntaxNode) -> Option<ImplDecl> {
        use verum_ast::decl::{ImplItem, ImplItemKind, ImplKind};

        let span = self.range_to_span(node.text_range());

        let mut generics = List::new();
        let mut generic_where_clause = Maybe::None;
        let meta_where_clause = Maybe::None;
        let mut items = List::new();

        // Track state for parsing impl kind
        let mut first_type: Option<Type> = None;
        let mut seen_for = false;
        let mut for_type: Option<Type> = None;
        let mut is_unsafe = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::FOR_KW {
                        seen_for = true;
                    } else if token.kind() == SyntaxKind::UNSAFE_KW {
                        is_unsafe = true;
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::ATTRIBUTE => {
                            // Attributes are collected but not fully converted yet
                        }
                        SyntaxKind::GENERIC_PARAMS => {
                            generics = self.convert_generic_params(&child_node);
                        }
                        SyntaxKind::WHERE_CLAUSE => {
                            // Convert where clause
                            let wc = self.convert_where_clause(&child_node);
                            if wc.is_some() {
                                generic_where_clause = wc;
                            }
                        }
                        SyntaxKind::IMPL_ITEM => {
                            if let Some(item) = self.convert_impl_item(&child_node) {
                                items.push(item);
                            }
                        }
                        // Handle type nodes that may appear as protocol or implementing type
                        kind if kind.can_start_type() => {
                            if let Some(ty) = self.convert_type(&child_node) {
                                if !seen_for && first_type.is_none() {
                                    first_type = Some(ty);
                                } else if seen_for && for_type.is_none() {
                                    for_type = Some(ty);
                                }
                            }
                        }
                        // Handle FN_DEF directly as impl items (in case no IMPL_ITEM wrapper)
                        SyntaxKind::FN_DEF => {
                            if let Some(func) = self.convert_function(&child_node) {
                                let item_span = self.range_to_span(child_node.text_range());
                                items.push(ImplItem {
                                    attributes: List::new(),
                                    visibility: func.visibility.clone(),
                                    kind: ImplItemKind::Function(func),
                                    span: item_span,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Determine the ImplKind
        let kind = if seen_for {
            // Protocol implementation: implement Protocol for Type
            let protocol_type = first_type?;
            let implementing_type = for_type?;

            // Extract protocol path and type arguments
            let (protocol, protocol_args) = match protocol_type.kind {
                TypeKind::Path(path) => {
                    // Simple protocol without type arguments
                    (path, List::new())
                }
                TypeKind::Generic { base, args } => {
                    // Protocol with type arguments: Iterator<Item = T>
                    match base.kind {
                        TypeKind::Path(path) => {
                            // args is already List<GenericArg>, use directly
                            (path, args)
                        }
                        _ => {
                            self.error_at(
                                node.text_range(),
                                "expected protocol path in impl block",
                            );
                            return None;
                        }
                    }
                }
                _ => {
                    self.error_at(
                        node.text_range(),
                        "expected protocol path in impl block",
                    );
                    return None;
                }
            };

            ImplKind::Protocol {
                protocol,
                protocol_args,
                for_type: implementing_type,
            }
        } else {
            // Inherent implementation: implement Type
            let implementing_type = first_type?;
            ImplKind::Inherent(implementing_type)
        };

        // For now, we don't extract @specialize attribute in the sink
        let specialize_attr = Maybe::None;

        Some(ImplDecl {
            is_unsafe,
            generics,
            kind,
            generic_where_clause,
            meta_where_clause,
            specialize_attr,
            items,
            span,
        })
    }

    /// Convert an IMPL_ITEM node to an ImplItem.
    fn convert_impl_item(&mut self, node: &SyntaxNode) -> Option<verum_ast::decl::ImplItem> {
        use verum_ast::decl::{ImplItem, ImplItemKind};

        let span = self.range_to_span(node.text_range());
        let mut visibility = Visibility::Private;

        // First pass: collect modifiers
        for child in node.children() {
            if let SyntaxElement::Token(token) = child {
                if token.kind() == SyntaxKind::PUB_KW {
                    visibility = Visibility::Public;
                }
            }
        }

        // Second pass: find the actual item
        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::FN_DEF => {
                    if let Some(func) = self.convert_function(&child) {
                        return Some(ImplItem {
                            attributes: List::new(),
                            visibility,
                            kind: ImplItemKind::Function(func),
                            span,
                        });
                    }
                }
                SyntaxKind::TYPE_DEF | SyntaxKind::IMPL_TYPE => {
                    // Associated type: type Item = T;
                    if let Some((name, type_params, ty)) = self.convert_impl_type_alias(&child) {
                        return Some(ImplItem {
                            attributes: List::new(),
                            visibility,
                            kind: ImplItemKind::Type {
                                name,
                                type_params,
                                ty,
                            },
                            span,
                        });
                    }
                }
                SyntaxKind::CONST_DEF | SyntaxKind::IMPL_CONST => {
                    // Associated const: const VALUE: Type = expr;
                    if let Some((name, ty, value)) = self.convert_impl_const(&child) {
                        return Some(ImplItem {
                            attributes: List::new(),
                            visibility,
                            kind: ImplItemKind::Const { name, ty, value },
                            span,
                        });
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Convert an associated type alias in an impl block.
    /// Handles: type Name<T> = Type;
    fn convert_impl_type_alias(
        &mut self,
        node: &SyntaxNode,
    ) -> Option<(Ident, List<GenericParam>, Type)> {
        let mut name = None;
        let mut type_params = List::new();
        let mut ty = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && name.is_none() {
                        name = Some(self.token_to_ident(&token));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            type_params = self.convert_generic_params(&child_node);
                        }
                        kind if kind.can_start_type() && ty.is_none() => {
                            ty = self.convert_type(&child_node);
                        }
                        _ => {}
                    }
                }
            }
        }

        Some((name?, type_params, ty?))
    }

    /// Convert an associated const in an impl block.
    /// Handles: const NAME: Type = expr;
    fn convert_impl_const(&mut self, node: &SyntaxNode) -> Option<(Ident, Type, Expr)> {
        let mut name = None;
        let mut ty = None;
        let mut value = None;
        let mut seen_colon = false;
        let mut seen_eq = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::IDENT if name.is_none() => {
                            name = Some(self.token_to_ident(&token));
                        }
                        SyntaxKind::COLON => seen_colon = true,
                        SyntaxKind::EQ => seen_eq = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if seen_colon && !seen_eq && ty.is_none() {
                        // Type comes after colon, before equals
                        if child_node.kind().can_start_type() {
                            ty = self.convert_type(&child_node);
                        }
                    } else if seen_eq && value.is_none() {
                        // Value comes after equals
                        if child_node.kind().can_start_expr() {
                            value = self.convert_expr(&child_node);
                        }
                    }
                }
            }
        }

        Some((name?, ty?, value?))
    }

    /// Convert a MOUNT_STMT node to a MountDecl.
    ///
    /// Parses mount statements of the form:
    /// - `mount std.io.File;`
    /// - `mount std.io.File as MyFile;`
    /// - `mount std.io.{File, Read, Write};`
    /// - `mount std.io.*;`
    /// - `public mount std.io.File;` (re-export)
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// mount_stmt = 'mount' , mount_tree , [ 'as' , identifier ] , ';' ;
    /// mount_tree = path | path , '.' , '{' , mount_list , '}' | path , '.' , '*' ;
    /// mount_list = mount_tree , { ',' , mount_tree } ;
    /// ```
    fn convert_mount(&mut self, node: &SyntaxNode) -> Option<MountDecl> {
        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut alias = Maybe::None;
        let mut tree = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        // Check for alias after 'as' keyword
                        SyntaxKind::IDENT => {
                            // This could be the alias (after 'as')
                            // We'll set it here, and reset if we find MOUNT_TREE
                        }
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::MOUNT_TREE => {
                            tree = self.convert_mount_tree(&child_node);
                        }
                        SyntaxKind::PATH | SyntaxKind::PATH_EXPR => {
                            // Simple path mount
                            if let Some(path) = self.convert_path(&child_node) {
                                tree = Some(MountTree {
                                    kind: MountTreeKind::Path(path),
                                    alias: Maybe::None,
                                    span: self.range_to_span(child_node.text_range()),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Look for alias after 'as' keyword
        let mut seen_as = false;
        for child in node.children() {
            if let SyntaxElement::Token(token) = child {
                match token.kind() {
                    SyntaxKind::AS_KW => seen_as = true,
                    SyntaxKind::IDENT if seen_as => {
                        alias = Maybe::Some(self.token_to_ident(&token));
                    }
                    _ => {}
                }
            }
        }

        Some(MountDecl {
            visibility,
            tree: tree?,
            alias,
            span,
        })
    }

    /// Convert a MOUNT_TREE node to MountTree.
    fn convert_mount_tree(&mut self, node: &SyntaxNode) -> Option<MountTree> {
        let span = self.range_to_span(node.text_range());

        // Check for glob mount (*)
        if self.has_token(node, SyntaxKind::STAR) {
            // Find the path prefix
            for child in node.child_nodes() {
                if matches!(child.kind(), SyntaxKind::PATH | SyntaxKind::PATH_EXPR) {
                    if let Some(path) = self.convert_path(&child) {
                        return Some(MountTree {
                            kind: MountTreeKind::Glob(path),
                            alias: Maybe::None,
                            span,
                        });
                    }
                }
            }
            return None;
        }

        // Check for nested mount { ... }
        if self.has_token(node, SyntaxKind::L_BRACE) {
            let mut prefix = None;
            let mut nested_trees = List::new();

            for child in node.child_nodes() {
                match child.kind() {
                    SyntaxKind::PATH | SyntaxKind::PATH_EXPR if prefix.is_none() => {
                        prefix = self.convert_path(&child);
                    }
                    SyntaxKind::MOUNT_LIST => {
                        for list_child in child.child_nodes() {
                            if list_child.kind() == SyntaxKind::MOUNT_TREE {
                                if let Some(nested) = self.convert_mount_tree(&list_child) {
                                    nested_trees.push(nested);
                                }
                            } else if matches!(list_child.kind(), SyntaxKind::PATH | SyntaxKind::PATH_EXPR) {
                                if let Some(path) = self.convert_path(&list_child) {
                                    nested_trees.push(MountTree {
                                        kind: MountTreeKind::Path(path),
                                        alias: Maybe::None,
                                        span: self.range_to_span(list_child.text_range()),
                                    });
                                }
                            }
                        }
                    }
                    SyntaxKind::MOUNT_TREE => {
                        if let Some(nested) = self.convert_mount_tree(&child) {
                            nested_trees.push(nested);
                        }
                    }
                    _ => {}
                }
            }

            // Also check for IDENT tokens directly in braces
            let mut in_braces = false;
            for child in node.children() {
                if let SyntaxElement::Token(token) = child { match token.kind() {
                    SyntaxKind::L_BRACE => in_braces = true,
                    SyntaxKind::R_BRACE => in_braces = false,
                    SyntaxKind::IDENT if in_braces => {
                        let ident = self.token_to_ident(&token);
                        let path = Path::single(ident);
                        nested_trees.push(MountTree {
                            kind: MountTreeKind::Path(path),
                            alias: Maybe::None,
                            span: self.range_to_span(token.text_range()),
                        });
                    }
                    _ => {}
                } }
            }

            if let Some(prefix_path) = prefix {
                return Some(MountTree {
                    kind: MountTreeKind::Nested {
                        prefix: prefix_path,
                        trees: nested_trees,
                    },
                    alias: Maybe::None,
                    span,
                });
            }
        }

        // Simple path mount
        for child in node.child_nodes() {
            if matches!(child.kind(), SyntaxKind::PATH | SyntaxKind::PATH_EXPR) {
                if let Some(path) = self.convert_path(&child) {
                    return Some(MountTree {
                        kind: MountTreeKind::Path(path),
                        alias: Maybe::None,
                        span,
                    });
                }
            }
        }

        // Try to build path from identifiers
        let mut segments = List::new();
        for child in node.children() {
            if let SyntaxElement::Token(token) = child {
                if token.kind() == SyntaxKind::IDENT {
                    segments.push(PathSegment::Name(self.token_to_ident(&token)));
                }
            }
        }

        if !segments.is_empty() {
            return Some(MountTree {
                kind: MountTreeKind::Path(Path::new(segments, span)),
                alias: Maybe::None,
                span,
            });
        }

        None
    }

    /// Convert a CONST_DEF node to a ConstDecl.
    ///
    /// Parses const declarations of the form:
    /// - `const MAX_SIZE: Int = 100;`
    /// - `public const PI: Float = 3.14159;`
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// const_def = visibility , 'const' , identifier , ':' , type_expr
    ///           , '=' , const_expr , ';' ;
    /// ```
    fn convert_const_def(&mut self, node: &SyntaxNode) -> Option<ConstDecl> {
        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut name = None;
        let mut ty = None;
        let mut value = None;
        let mut seen_colon = false;
        let mut seen_eq = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        SyntaxKind::IDENT if name.is_none() => {
                            name = Some(self.token_to_ident(&token));
                        }
                        SyntaxKind::COLON => seen_colon = true,
                        SyntaxKind::EQ => seen_eq = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if seen_colon && !seen_eq && ty.is_none() {
                        // Type comes after colon, before equals
                        if child_node.kind().can_start_type() {
                            ty = self.convert_type(&child_node);
                        }
                    } else if seen_eq && value.is_none() {
                        // Value comes after equals
                        if child_node.kind().can_start_expr() {
                            value = self.convert_expr(&child_node);
                        }
                    }
                }
            }
        }

        Some(ConstDecl {
            visibility,
            name: name?,
            generics: List::new(),
            ty: ty?,
            value: value?,
            span,
        })
    }

    /// Convert a STATIC_DEF node to a StaticDecl.
    ///
    /// Parses static declarations of the form:
    /// - `static COUNTER: Int = 0;`
    /// - `static mut CACHE: Map<Text, Data> = Map.new();`
    /// - `public static CONFIG: Config = Config.default();`
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// static_def = visibility , 'static' , [ 'mut' ] , identifier
    ///            , ':' , type_expr , '=' , const_expr , ';' ;
    /// ```
    fn convert_static_def(&mut self, node: &SyntaxNode) -> Option<StaticDecl> {
        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut is_mut = false;
        let mut name = None;
        let mut ty = None;
        let mut value = None;
        let mut seen_colon = false;
        let mut seen_eq = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        SyntaxKind::MUT_KW => is_mut = true,
                        SyntaxKind::IDENT if name.is_none() => {
                            name = Some(self.token_to_ident(&token));
                        }
                        SyntaxKind::COLON => seen_colon = true,
                        SyntaxKind::EQ => seen_eq = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if seen_colon && !seen_eq && ty.is_none() {
                        // Type comes after colon, before equals
                        if child_node.kind().can_start_type() {
                            ty = self.convert_type(&child_node);
                        }
                    } else if seen_eq && value.is_none() {
                        // Value comes after equals
                        if child_node.kind().can_start_expr() {
                            value = self.convert_expr(&child_node);
                        }
                    }
                }
            }
        }

        Some(StaticDecl {
            visibility,
            is_mut,
            name: name?,
            ty: ty?,
            value: value?,
            span,
        })
    }

    /// Convert a CONTEXT_DEF node to a ContextDecl.
    ///
    /// Parses context declarations of the form:
    /// - `context Database { fn query(&self) -> Data; }`
    /// - `context async Logger<T> { fn log(&self, msg: T); }`
    /// - `public context FileSystem { ... }`
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// context_def = visibility , 'context' , identifier , [ generics ]
    ///             , '{' , { context_item } , '}' ;
    /// context_item = context_function | context_type | context_const ;
    /// ```
    fn convert_context_def(&mut self, node: &SyntaxNode) -> Option<ContextDecl> {
        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut is_async = false;
        let mut name = None;
        let mut generics = List::new();
        let mut methods = List::new();
        let mut sub_contexts = List::new();

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        SyntaxKind::ASYNC_KW => is_async = true,
                        SyntaxKind::IDENT if name.is_none() => {
                            name = Some(self.token_to_ident(&token));
                        }
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            generics = self.convert_generic_params(&child_node);
                        }
                        SyntaxKind::FN_DEF => {
                            if let Some(func) = self.convert_function(&child_node) {
                                methods.push(func);
                            }
                        }
                        // Nested context definition (sub-context)
                        SyntaxKind::CONTEXT_DEF => {
                            if let Some(sub_ctx) = self.convert_context_def(&child_node) {
                                sub_contexts.push(sub_ctx);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Some(ContextDecl {
            visibility,
            is_async,
            name: name?,
            generics,
            methods,
            sub_contexts,
            associated_types: List::new(),
            associated_consts: List::new(),
            span,
        })
    }

    /// Convert a MODULE_DEF node to a ModuleDecl.
    ///
    /// Parses module declarations of the form:
    /// - `module utils { ... }`
    /// - `module utils;` (external module)
    /// - `public module api { ... }`
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// module_def = visibility , 'module' , identifier , module_body ;
    /// module_body = '{' , { program_item } , '}' | ';' ;
    /// ```
    fn convert_module_def(&mut self, node: &SyntaxNode) -> Option<ModuleDecl> {
        use verum_ast::decl::{FeatureAttr, ProfileAttr};

        let span = self.range_to_span(node.text_range());

        let mut visibility = Visibility::Private;
        let mut name = None;
        let mut items = Maybe::None;
        let mut contexts = List::new();

        // Check if this is an external module (ends with semicolon, no body)
        let is_external = self.has_token(node, SyntaxKind::SEMICOLON)
            && !self.has_token(node, SyntaxKind::L_BRACE);

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PUB_KW => visibility = Visibility::Public,
                        SyntaxKind::IDENT if name.is_none() => {
                            name = Some(self.token_to_ident(&token));
                        }
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        // Module body contains items
                        kind if kind.can_start_item() => {
                            let item_list = items.unwrap_or_else(List::new);
                            let mut new_items = item_list;
                            if let Some(item) = self.convert_item(&child_node) {
                                new_items.push(item);
                            }
                            items = Maybe::Some(new_items);
                        }
                        SyntaxKind::USING_CLAUSE => {
                            contexts = self.convert_using_clause(&child_node);
                        }
                        SyntaxKind::ATTRIBUTE => {
                            // Handle @using attribute for module-level contexts
                            if let Some(attr) = self.convert_attribute(&child_node) {
                                if attr.name.as_str() == "using" {
                                    // Extract context requirements from attribute args
                                    // For now, leave contexts handling to attribute processing
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // For external modules, items is None
        if is_external {
            items = Maybe::None;
        }

        Some(ModuleDecl {
            visibility,
            name: name?,
            items,
            profile: Maybe::None,
            features: Maybe::None,
            contexts,
            span,
        })
    }


    /// Convert a GENERIC_PARAMS node to a list of GenericParam.
    fn convert_generic_params(&mut self, node: &SyntaxNode) -> List<GenericParam> {
        use verum_ast::ty::{GenericParamKind, TypeBound, TypeBoundKind};
        let mut params = List::new();
        for child in node.child_nodes() {
            if !matches!(child.kind(), SyntaxKind::GENERIC_PARAM | SyntaxKind::TYPE_PARAM | SyntaxKind::META_PARAM) { continue; }
            let span = self.range_to_span(child.text_range());
            let mut name = None;
            let mut bounds = List::new();
            let mut default_type = Maybe::None;
            let mut is_meta = false;
            let mut meta_ty = None;
            let mut meta_refinement = Maybe::None;
            for elem in child.children() {
                match elem {
                    SyntaxElement::Token(token) => match token.kind() {
                        SyntaxKind::IDENT => { if name.is_none() { name = Some(self.token_to_ident(&token)); } }
                        SyntaxKind::META_KW => { is_meta = true; }
                        _ => {}
                    },
                    SyntaxElement::Node(child_node) => match child_node.kind() {
                        SyntaxKind::BOUND_LIST | SyntaxKind::TYPE_BOUND => {
                            for bound_child in child_node.child_nodes() {
                                if let Some(path) = self.convert_path(&bound_child) {
                                    bounds.push(TypeBound { kind: TypeBoundKind::Protocol(path), span: self.range_to_span(bound_child.text_range()) });
                                }
                            }
                            if child_node.kind() == SyntaxKind::TYPE_BOUND {
                                if let Some(path) = self.convert_path(&child_node) {
                                    bounds.push(TypeBound { kind: TypeBoundKind::Protocol(path), span: self.range_to_span(child_node.text_range()) });
                                }
                            }
                        }
                        kind if kind.can_start_type() => {
                            if is_meta && meta_ty.is_none() { meta_ty = self.convert_type(&child_node); }
                            else if !is_meta && default_type.is_none() { if let Some(ty) = self.convert_type(&child_node) { default_type = Maybe::Some(ty); } }
                        }
                        SyntaxKind::REFINEMENT_EXPR => {
                            if let Some(expr) = child_node.child_nodes().find_map(|n| self.convert_expr(&n)) { meta_refinement = Maybe::Some(Heap::new(expr)); }
                        }
                        _ => {}
                    },
                }
            }
            if let Some(ident) = name {
                let param_kind = if is_meta {
                    GenericParamKind::Meta { name: ident, ty: meta_ty.unwrap_or_else(|| Type::new(TypeKind::Int, span)), refinement: meta_refinement }
                } else {
                    GenericParamKind::Type { name: ident, bounds, default: default_type }
                };
                // Note: The CST-based parser doesn't currently support implicit syntax.
                // Implicit parameters would need to be detected from the source text.
                params.push(GenericParam { kind: param_kind, is_implicit: false, span });
            }
        }
        params
    }

    /// Convert a PARAM_LIST node to a list of FunctionParam.
    fn convert_param_list(&mut self, node: &SyntaxNode) -> List<FunctionParam> {
        let mut params = List::new();
        for child in node.child_nodes() {
            if !matches!(child.kind(), SyntaxKind::PARAM | SyntaxKind::SELF_PARAM) { continue; }
            let span = self.range_to_span(child.text_range());
            if child.kind() == SyntaxKind::SELF_PARAM {
                let has_ref = self.has_token(&child, SyntaxKind::AMP);
                let has_mut = self.has_token(&child, SyntaxKind::MUT_KW);
                let has_own = self.has_token(&child, SyntaxKind::PERCENT);
                let param_kind = match (has_ref, has_mut, has_own) {
                    (true, true, false) => FunctionParamKind::SelfRefMut,
                    (true, false, false) => FunctionParamKind::SelfRef,
                    (false, true, true) => FunctionParamKind::SelfOwnMut,
                    (false, false, true) => FunctionParamKind::SelfOwn,
                    (false, true, false) => FunctionParamKind::SelfValueMut,
                    _ => FunctionParamKind::SelfValue,
                };
                params.push(FunctionParam::new(param_kind, span));
                continue;
            }
            let mut pattern = None;
            let mut ty = None;
            for elem in child.children() {
                match elem {
                    SyntaxElement::Token(token) => {
                        if token.kind() == SyntaxKind::SELF_VALUE_KW {
                            let has_ref = self.has_token(&child, SyntaxKind::AMP);
                            let has_mut = self.has_token(&child, SyntaxKind::MUT_KW);
                            let has_own = self.has_token(&child, SyntaxKind::PERCENT);
                            let param_kind = match (has_ref, has_mut, has_own) {
                                (true, true, false) => FunctionParamKind::SelfRefMut,
                                (true, false, false) => FunctionParamKind::SelfRef,
                                (false, true, true) => FunctionParamKind::SelfOwnMut,
                                (false, false, true) => FunctionParamKind::SelfOwn,
                                (false, true, false) => FunctionParamKind::SelfValueMut,
                                _ => FunctionParamKind::SelfValue,
                            };
                            params.push(FunctionParam::new(param_kind, span));
                            pattern = None;
                            break;
                        }
                    }
                    SyntaxElement::Node(child_node) => match child_node.kind() {
                        SyntaxKind::IDENT_PAT | SyntaxKind::TUPLE_PAT | SyntaxKind::RECORD_PAT | SyntaxKind::WILDCARD_PAT => {
                            if pattern.is_none() { pattern = self.convert_pattern(&child_node); }
                        }
                        // Explicit type node kinds that EventBasedParser emits
                        SyntaxKind::PATH_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::TUPLE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE
                            | SyntaxKind::REFINED_TYPE
                            | SyntaxKind::GENERIC_TYPE => {
                            if ty.is_none() { ty = self.convert_type(&child_node); }
                        }
                        kind if kind.can_start_type() && pattern.is_some() => { if ty.is_none() { ty = self.convert_type(&child_node); } }
                        _ => {}
                    },
                }
            }
            match (pattern, ty) {
                (Some(pat), Some(param_ty)) => { params.push(FunctionParam::new(FunctionParamKind::Regular { pattern: pat, ty: param_ty, default_value: Maybe::None }, span)); }
                (Some(pat), None) => { params.push(FunctionParam::new(FunctionParamKind::Regular { pattern: pat, ty: Type::new(TypeKind::Inferred, span), default_value: Maybe::None }, span)); }
                _ => {}
            }
        }
        params
    }

    /// Convert a WHERE_CLAUSE node to a WhereClause.
    /// Handles type bounds like `where T: Protocol` and meta constraints like `where meta N > 0`
    fn convert_where_clause(&mut self, node: &SyntaxNode) -> Maybe<WhereClause> {
        use verum_ast::ty::{TypeBound, TypeBoundKind, WherePredicate, WherePredicateKind};
        let span = self.range_to_span(node.text_range());
        let mut predicates = List::new();
        for child in node.child_nodes() {
            if child.kind() != SyntaxKind::WHERE_PRED { continue; }
            let pred_span = self.range_to_span(child.text_range());
            let mut is_meta_pred = false;
            let mut pred_ty = None;
            let mut bounds = List::new();
            let mut constraint_expr = None;
            for elem in child.children() {
                match elem {
                    SyntaxElement::Token(token) => {
                        if token.kind() == SyntaxKind::META_KW { is_meta_pred = true; }
                    },
                    SyntaxElement::Node(child_node) => match child_node.kind() {
                        // Explicit type node kinds that EventBasedParser emits
                        SyntaxKind::PATH_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::TUPLE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE
                            | SyntaxKind::REFINED_TYPE
                            | SyntaxKind::GENERIC_TYPE if !is_meta_pred => {
                            // First type is the predicate type (e.g., T in T: Debug)
                            // Additional types after colon are bounds (e.g., Debug)
                            if pred_ty.is_none() {
                                pred_ty = self.convert_type(&child_node);
                            } else {
                                // Additional types are bounds
                                if let Some(path) = self.convert_path(&child_node) {
                                    bounds.push(TypeBound { kind: TypeBoundKind::Protocol(path), span: self.range_to_span(child_node.text_range()) });
                                }
                            }
                        }
                        kind if kind.can_start_type() && pred_ty.is_none() && !is_meta_pred => {
                            pred_ty = self.convert_type(&child_node);
                        }
                        SyntaxKind::BOUND_LIST | SyntaxKind::TYPE_BOUND => {
                            for bound_child in child_node.child_nodes() {
                                if let Some(path) = self.convert_path(&bound_child) {
                                    bounds.push(TypeBound { kind: TypeBoundKind::Protocol(path), span: self.range_to_span(bound_child.text_range()) });
                                }
                            }
                            if child_node.kind() == SyntaxKind::TYPE_BOUND {
                                if let Some(path) = self.convert_path(&child_node) {
                                    bounds.push(TypeBound { kind: TypeBoundKind::Protocol(path), span: self.range_to_span(child_node.text_range()) });
                                }
                            }
                        }
                        kind if kind.can_start_expr() && is_meta_pred => {
                            if constraint_expr.is_none() { constraint_expr = self.convert_expr(&child_node); }
                        }
                        _ => {}
                    },
                }
            }
            let pred_kind = if is_meta_pred {
                if let Some(expr) = constraint_expr { WherePredicateKind::Meta { constraint: expr } } else { continue; }
            } else if let Some(ty) = pred_ty {
                WherePredicateKind::Type { ty, bounds }
            } else { continue; };
            predicates.push(WherePredicate { kind: pred_kind, span: pred_span });
        }
        if predicates.is_empty() { Maybe::None } else { Maybe::Some(WhereClause::new(predicates, span)) }
    }

    fn convert_clause_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        node.child_nodes().find_map(|n| self.convert_expr(&n))
    }

    /// Convert a USING_CLAUSE node to a list of ContextRequirement.
    fn convert_using_clause(&mut self, node: &SyntaxNode) -> List<ContextRequirement> {
        let mut contexts = List::new();
        for child in node.child_nodes() {
            if let Some(req) = self.convert_single_context_requirement(&child) { contexts.push(req); }
        }
        contexts
    }

    fn convert_single_context_requirement(&mut self, node: &SyntaxNode) -> Option<ContextRequirement> {
        let span = self.range_to_span(node.text_range());
        let mut path = None;
        let mut type_args = List::new();
        let mut is_negative = false;
        let mut alias = Maybe::None;
        for elem in node.children() {
            match elem {
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::BANG => { is_negative = true; }
                    SyntaxKind::IDENT => {
                        if path.is_none() {
                            let ident = self.token_to_ident(&token);
                            path = Some(Path::new(List::from_iter([PathSegment::Name(ident)]), span));
                        } else if alias.is_none() { alias = Maybe::Some(self.token_to_ident(&token)); }
                    }
                    _ => {}
                },
                SyntaxElement::Node(child_node) => match child_node.kind() {
                    SyntaxKind::PATH | SyntaxKind::PATH_TYPE | SyntaxKind::PATH_EXPR => { if path.is_none() { path = self.convert_path(&child_node); } }
                    SyntaxKind::GENERIC_ARGS => { for arg_child in child_node.child_nodes() { if let Some(ty) = self.convert_type(&arg_child) { type_args.push(ty); } } }
                    _ => {}
                },
            }
        }
        Some(ContextRequirement { path: path?, args: type_args, is_negative, alias, name: Maybe::None, condition: Maybe::None, transforms: List::new(), span })
    }

    /// Convert a FIELD_LIST node to a TypeDeclBody::Record.
    fn convert_record_body(&mut self, node: &SyntaxNode) -> TypeDeclBody {
        let mut fields = List::new();
        for child in node.child_nodes() {
            if !matches!(child.kind(), SyntaxKind::FIELD_DEF | SyntaxKind::RECORD_FIELD) { continue; }
            let field_span = self.range_to_span(child.text_range());
            let mut visibility = Visibility::Private;
            let mut field_name = None;
            let mut field_ty = None;
            let mut field_attrs = List::new();
            for elem in child.children() {
                match elem {
                    SyntaxElement::Token(token) => match token.kind() {
                        SyntaxKind::PUB_KW => { visibility = Visibility::Public; }
                        SyntaxKind::IDENT => { if field_name.is_none() { field_name = Some(self.token_to_ident(&token)); } }
                        _ => {}
                    },
                    SyntaxElement::Node(child_node) => match child_node.kind() {
                        // Explicit type node kinds that EventBasedParser emits
                        SyntaxKind::PATH_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::TUPLE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE
                            | SyntaxKind::REFINED_TYPE
                            | SyntaxKind::GENERIC_TYPE if field_ty.is_none() => { field_ty = self.convert_type(&child_node); }
                        kind if kind.can_start_type() && field_ty.is_none() => { field_ty = self.convert_type(&child_node); }
                        SyntaxKind::ATTRIBUTE => { if let Some(attr) = self.convert_attribute(&child_node) { field_attrs.push(attr); } }
                        _ => {}
                    },
                }
            }
            if let (Some(name), Some(ty)) = (field_name, field_ty) {
                fields.push(RecordField::with_attributes(visibility, name, ty, field_attrs, field_span));
            }
        }
        TypeDeclBody::Record(fields)
    }

    /// Convert a VARIANT_LIST node to a TypeDeclBody::Variant.
    fn convert_variant_body(&mut self, node: &SyntaxNode) -> TypeDeclBody {
        use verum_ast::decl::VariantData;
        let mut variants = List::new();
        for child in node.child_nodes() {
            if child.kind() != SyntaxKind::VARIANT_DEF { continue; }
            let variant_span = self.range_to_span(child.text_range());
            let mut variant_name = None;
            let mut variant_data = Maybe::None;
            let mut variant_attrs = List::new();
            for elem in child.children() {
                match elem {
                    SyntaxElement::Token(token) => {
                        // Variant names can be IDENT, NONE_KW, or SOME_KW
                        if matches!(token.kind(), SyntaxKind::IDENT | SyntaxKind::NONE_KW | SyntaxKind::SOME_KW) && variant_name.is_none() {
                            variant_name = Some(self.token_to_ident(&token));
                        }
                    }
                    SyntaxElement::Node(child_node) => match child_node.kind() {
                        SyntaxKind::TUPLE_TYPE => {
                            let mut tuple_types = List::new();
                            for type_child in child_node.child_nodes() { if let Some(ty) = self.convert_type(&type_child) { tuple_types.push(ty); } }
                            if !tuple_types.is_empty() { variant_data = Maybe::Some(VariantData::Tuple(tuple_types)); }
                        }
                        // Explicit type node kinds that EventBasedParser emits directly
                        SyntaxKind::PATH_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE
                            | SyntaxKind::REFINED_TYPE
                            | SyntaxKind::GENERIC_TYPE if variant_name.is_some() => {
                            // Collect all type nodes for tuple variant
                            if let Some(ty) = self.convert_type(&child_node) {
                                if let Maybe::Some(VariantData::Tuple(ref mut types)) = variant_data {
                                    types.push(ty);
                                } else {
                                    variant_data = Maybe::Some(VariantData::Tuple(List::from_iter([ty])));
                                }
                            }
                        }
                        kind if kind.can_start_type() && variant_name.is_some() && variant_data.is_none() => {
                            if let Some(ty) = self.convert_type(&child_node) { variant_data = Maybe::Some(VariantData::Tuple(List::from_iter([ty]))); }
                        }
                        SyntaxKind::FIELD_LIST => {
                            if let TypeDeclBody::Record(fields) = self.convert_record_body(&child_node) {
                                if !fields.is_empty() { variant_data = Maybe::Some(VariantData::Record(fields)); }
                            }
                        }
                        SyntaxKind::ATTRIBUTE => { if let Some(attr) = self.convert_attribute(&child_node) { variant_attrs.push(attr); } }
                        _ => {}
                    },
                }
            }
            if let Some(name) = variant_name { variants.push(Variant::with_attributes(name, variant_data, variant_attrs, variant_span)); }
        }
        TypeDeclBody::Variant(variants)
    }

    fn convert_path(&mut self, node: &SyntaxNode) -> Option<Path> {
        let mut segments = List::new();
        for child in node.children() {
            if let SyntaxElement::Token(token) = child {
                match token.kind() {
                    SyntaxKind::IDENT => {
                        segments.push(PathSegment::Name(self.token_to_ident(&token)));
                    }
                    SyntaxKind::SELF_VALUE_KW | SyntaxKind::SELF_TYPE_KW => {
                        segments.push(PathSegment::SelfValue);
                    }
                    SyntaxKind::SUPER_KW => {
                        segments.push(PathSegment::Super);
                    }
                    SyntaxKind::COG_KW => {
                        segments.push(PathSegment::Cog);
                    }
                    _ => {}
                }
            } else if let SyntaxElement::Node(child_node) = child {
                if child_node.kind() == SyntaxKind::PATH_SEGMENT {
                    if let Some(ident) = self.find_ident_in_node(&child_node) {
                        segments.push(PathSegment::Name(ident));
                    }
                }
            }
        }

        if segments.is_empty() {
            return None;
        }

        let span = self.range_to_span(node.text_range());
        Some(Path::new(segments, span))
    }

    /// Convert a REFERENCE_TYPE node to Type.
    ///
    /// Handles three-tier reference types from grammar:
    /// - `&T` - managed reference with CBGR
    /// - `&mut T` - mutable managed reference
    /// - `&checked T` - compile-time verified reference (0ns overhead)
    /// - `&checked mut T` - mutable checked reference
    /// - `&unsafe T` - unsafe reference with no checks
    /// - `&unsafe mut T` - mutable unsafe reference
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// managed_reference_type = '&' , [ 'mut' ] , type_expr ;
    /// checked_reference_type = '&' , 'checked' , [ 'mut' ] , type_expr ;
    /// unsafe_reference_type = '&' , 'unsafe' , [ 'mut' ] , type_expr ;
    /// ```
    fn convert_reference_type(&mut self, node: &SyntaxNode) -> Option<Type> {
        let span = self.range_to_span(node.text_range());

        let mut is_checked = false;
        let mut is_unsafe = false;
        let mut is_mutable = false;
        let mut inner_type = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::CHECKED_KW => is_checked = true,
                    SyntaxKind::UNSAFE_KW => is_unsafe = true,
                    SyntaxKind::MUT_KW => is_mutable = true,
                    SyntaxKind::AMP => {} // Skip the ampersand token
                    _ => {}
                },
                SyntaxElement::Node(child_node) => {
                    // The inner type should be the first type node we find
                    // Check for explicit type node kinds that EventBasedParser emits
                    let is_type_node = matches!(
                        child_node.kind(),
                        SyntaxKind::PATH_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::TUPLE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE
                            | SyntaxKind::REFINED_TYPE
                            | SyntaxKind::GENERIC_TYPE
                    ) || child_node.kind().can_start_type();
                    if inner_type.is_none() && is_type_node {
                        inner_type = self.convert_type(&child_node);
                    }
                }
            }
        }

        let inner = Heap::new(inner_type?);

        let kind = if is_checked {
            TypeKind::CheckedReference {
                mutable: is_mutable,
                inner,
            }
        } else if is_unsafe {
            TypeKind::UnsafeReference {
                mutable: is_mutable,
                inner,
            }
        } else {
            TypeKind::Reference {
                mutable: is_mutable,
                inner,
            }
        };

        Some(Type::new(kind, span))
    }

    /// Convert a TUPLE_TYPE node to Type.
    ///
    /// Handles tuple types like `(A, B, C)`.
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// tuple_type = '(' , type_expr , { ',' , type_expr } , ')' ;
    /// ```
    fn convert_tuple_type(&mut self, node: &SyntaxNode) -> Option<Type> {
        let span = self.range_to_span(node.text_range());
        let mut element_types = List::new();

        for child in node.child_nodes() {
            // Convert each type child node
            if let Some(ty) = self.convert_type(&child) {
                element_types.push(ty);
            }
        }

        // Single-element tuple is still a tuple (needed for trailing comma case)
        // Empty parens is unit type, but that's typically handled elsewhere
        if element_types.is_empty() {
            return Some(Type::new(TypeKind::Unit, span));
        }

        Some(Type::new(TypeKind::Tuple(element_types), span))
    }

    /// Convert an ARRAY_TYPE node to Type.
    ///
    /// Handles array types like `[T; N]` and slice types like `[T]`.
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// array_type = '[' , type_expr , ';' , expression , ']' ;
    /// slice_type = '[' , type_expr , ']' ;
    /// ```
    fn convert_array_type(&mut self, node: &SyntaxNode) -> Option<Type> {
        let span = self.range_to_span(node.text_range());

        let mut element_type = None;
        let mut size_expr = None;
        let mut has_semicolon = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::SEMICOLON {
                        has_semicolon = true;
                    }
                }
                SyntaxElement::Node(child_node) => {
                    // First type node is the element type
                    if element_type.is_none() && child_node.kind().can_start_type() {
                        element_type = self.convert_type(&child_node);
                    } else if has_semicolon && size_expr.is_none() {
                        // After semicolon, look for the size expression
                        size_expr = self.convert_expr(&child_node);
                    }
                }
            }
        }

        let element = Heap::new(element_type?);

        let kind = if has_semicolon {
            // Array type: [T; N]
            TypeKind::Array {
                element,
                size: size_expr.map(Heap::new),
            }
        } else {
            // Slice type: [T]
            TypeKind::Slice(element)
        };

        Some(Type::new(kind, span))
    }

    /// Convert a FUNCTION_TYPE node to Type.
    ///
    /// Handles function types like `fn(A, B) -> C`.
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// function_type = 'fn' , '(' , type_list , ')' , [ '->' , type_expr ] ;
    /// type_list     = [ type_expr , { ',' , type_expr } ] ;
    /// ```
    fn convert_function_type(&mut self, node: &SyntaxNode) -> Option<Type> {
        let span = self.range_to_span(node.text_range());

        let mut param_types = List::new();
        let mut return_type = None;
        let mut in_params = false;
        let mut after_arrow = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::L_PAREN => in_params = true,
                    SyntaxKind::R_PAREN => in_params = false,
                    SyntaxKind::ARROW => after_arrow = true,
                    SyntaxKind::FN_KW => {} // Skip fn keyword
                    SyntaxKind::COMMA => {} // Skip comma separators
                    _ => {}
                },
                SyntaxElement::Node(child_node) => {
                    if child_node.kind() == SyntaxKind::PARAM_LIST {
                        // Handle PARAM_LIST node - extract types from parameters
                        for param in child_node.child_nodes() {
                            // Look for type nodes within parameters
                            for param_child in param.child_nodes() {
                                if let Some(ty) = self.convert_type(&param_child) {
                                    param_types.push(ty);
                                    break; // Only take first type per param
                                }
                            }
                        }
                    } else if after_arrow && return_type.is_none() {
                        // Return type after ->
                        return_type = self.convert_type(&child_node);
                    } else if in_params {
                        // Parameter types directly in the parentheses
                        if let Some(ty) = self.convert_type(&child_node) {
                            param_types.push(ty);
                        }
                    }
                }
            }
        }

        // Default to unit return type if not specified
        let ret_type = return_type.unwrap_or_else(|| Type::unit(span));

        Some(Type::new(
            TypeKind::Function {
                params: param_types,
                return_type: Heap::new(ret_type),
                calling_convention: Maybe::None,
                contexts: ContextList::empty(),
            },
            span,
        ))
    }

    /// Convert a REFINED_TYPE node to Type.
    ///
    /// Handles refinement types like `{ x: T | predicate }`.
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// type_refinement       = inline_refinement | value_where_clause ;
    /// inline_refinement     = '{' , refinement_predicates , '}' ;
    /// refinement_predicates = refinement_predicate , { ',' , refinement_predicate } ;
    /// refinement_predicate  = identifier , ':' , expression | expression ;
    /// ```
    ///
    /// Five Binding Rules: inline {pred}, declarative `where pred`, sigma `n: T where f(n)`
    fn convert_refined_type(&mut self, node: &SyntaxNode) -> Option<Type> {
        use verum_ast::ty::RefinementPredicate;

        let span = self.range_to_span(node.text_range());

        let mut base_type = None;
        let mut predicate_expr = None;
        let mut binding_name = None;

        // Look for the base type and the refinement predicate
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    // Look for binding name before colon: { x: T | pred } or T{ > 0 }
                    if token.kind() == SyntaxKind::IDENT && binding_name.is_none() {
                        binding_name = Some(self.token_to_ident(&token));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        // The base type
                        kind if kind.can_start_type() && base_type.is_none() => {
                            base_type = self.convert_type(&child_node);
                        }
                        // The refinement expression
                        SyntaxKind::REFINEMENT_EXPR => {
                            // Extract the expression from the refinement block
                            for refinement_child in child_node.child_nodes() {
                                if predicate_expr.is_none() {
                                    predicate_expr = self.convert_expr(&refinement_child);
                                }
                            }
                        }
                        // Direct expression as predicate
                        _ if predicate_expr.is_none() && base_type.is_some() => {
                            predicate_expr = self.convert_expr(&child_node);
                        }
                        _ => {}
                    }
                }
            }
        }

        // If we don't have a base type, try to find it differently
        // (Some refined types have the structure T{pred} where T comes first)
        let base = base_type?;
        let pred = predicate_expr?;

        let refinement = RefinementPredicate::with_binding(
            pred,
            binding_name,
            span,
        );

        Some(Type::new(
            TypeKind::Refined {
                base: Heap::new(base),
                predicate: Heap::new(refinement),
            },
            span,
        ))
    }

    /// Convert a GENERIC_TYPE node to Type.
    ///
    /// Handles generic types like `List<T>`, `Map<K, V>`, `Array<T, 10>`.
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// generic_type    = path , type_args ;
    /// type_args       = '<' , type_arg , { ',' , type_arg } , '>' ;
    /// type_arg        = extended_type_arg ;
    /// extended_type_arg = type_expr | expression | type_level_literal | meta_type_expr ;
    /// ```
    fn convert_generic_type(&mut self, node: &SyntaxNode) -> Option<Type> {
        let span = self.range_to_span(node.text_range());

        let mut base_type = None;
        let mut type_args = List::new();
        let mut in_type_args = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::L_ANGLE => in_type_args = true,
                    SyntaxKind::R_ANGLE => in_type_args = false,
                    SyntaxKind::COMMA => {} // Skip comma separators
                    SyntaxKind::IDENT if base_type.is_none() && !in_type_args => {
                        // Single identifier as base type
                        let ident = self.token_to_ident(&token);
                        let path = Path::single(ident);
                        base_type = Some(Type::new(TypeKind::Path(path), span));
                    }
                    _ => {}
                },
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        // The base type (path or simple type)
                        SyntaxKind::PATH | SyntaxKind::PATH_TYPE if base_type.is_none() => {
                            base_type = self.convert_type(&child_node);
                        }
                        // Generic arguments node
                        SyntaxKind::GENERIC_ARGS => {
                            for arg_child in child_node.child_nodes() {
                                if let Some(arg) = self.convert_generic_arg(&arg_child) {
                                    type_args.push(arg);
                                }
                            }
                        }
                        // Individual type argument
                        SyntaxKind::TYPE_ARG => {
                            if let Some(arg) = self.convert_generic_arg(&child_node) {
                                type_args.push(arg);
                            }
                        }
                        // Type directly in angle brackets
                        _ if in_type_args => {
                            if let Some(arg) = self.convert_generic_arg(&child_node) {
                                type_args.push(arg);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let base = base_type?;

        // If no type args found, this might not be a generic type
        if type_args.is_empty() {
            return Some(base);
        }

        Some(Type::new(
            TypeKind::Generic {
                base: Heap::new(base),
                args: type_args,
            },
            span,
        ))
    }

    /// Convert a type argument node to GenericArg.
    ///
    /// Type arguments can be:
    /// - Type arguments: `List<Int>`
    /// - Const arguments: `Array<T, 10>`
    /// - Associated type bindings: `Deref<Target=Vec<Int>>`
    fn convert_generic_arg(&mut self, node: &SyntaxNode) -> Option<GenericArg> {
        // Check if this is a type binding (Name = Type)
        let mut has_eq = false;
        let mut binding_name = None;
        let mut binding_type = None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::EQ => has_eq = true,
                    SyntaxKind::IDENT if binding_name.is_none() && !has_eq => {
                        binding_name = Some(self.token_to_ident(&token));
                    }
                    _ => {}
                },
                SyntaxElement::Node(child_node) if has_eq => {
                    if binding_type.is_none() {
                        binding_type = self.convert_type(&child_node);
                    }
                }
                _ => {}
            }
        }

        // If we have Name = Type, it's a binding
        if has_eq {
            if let (Some(name), Some(ty)) = (binding_name, binding_type) {
                use verum_ast::ty::TypeBinding;
                let span = self.range_to_span(node.text_range());
                return Some(GenericArg::Binding(TypeBinding::new(name, ty, span)));
            }
        }

        // Try to parse as a type first
        if let Some(ty) = self.convert_type(node) {
            return Some(GenericArg::Type(ty));
        }

        // Try to parse as a const expression (e.g., integer literal)
        if let Some(expr) = self.convert_expr(node) {
            return Some(GenericArg::Const(expr));
        }

        None
    }

    fn convert_literal_from_pattern(&mut self, node: &SyntaxNode) -> Option<Literal> {
        for child in node.child_nodes() {
            if child.kind() == SyntaxKind::LITERAL_EXPR {
                return self.convert_literal(&child);
            }
        }
        None
    }

    fn convert_patterns_list(&mut self, node: &SyntaxNode) -> List<Pattern> {
        let mut patterns = List::new();
        for child in node.child_nodes() {
            if let Some(pat) = self.convert_pattern(&child) {
                patterns.push(pat);
            }
        }
        patterns
    }

    /// Convert a RECORD_PAT node to Pattern.
    ///
    /// Handles record patterns: `Point { x, y }` or `Point { x: px, .. }`
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// record_pattern = path , '{' , field_patterns , '}' ;
    /// field_patterns = [ field_pattern , { ',' , field_pattern } , [ ',' , '..' ] ] ;
    /// field_pattern  = identifier , [ ':' , pattern ] ;
    /// ```
    fn convert_record_pattern(&mut self, node: &SyntaxNode) -> Option<Pattern> {
        let span = self.range_to_span(node.text_range());

        // Extract the path (type name)
        let path = self.extract_path_from_pattern(node)?;

        // Extract field patterns and rest flag
        let (fields, rest) = self.extract_field_patterns(node);

        Some(Pattern::new(
            PatternKind::Record { path, fields, rest },
            span,
        ))
    }

    /// Convert a VARIANT_PAT node to Pattern.
    ///
    /// Handles variant patterns:
    /// - Unit variant: `None`, `Color.Red`
    /// - Tuple variant: `Some(x)`, `Result.Ok(value)`
    /// - Record variant: `Event.UserCreated { id, name }`
    ///
    /// Grammar (from verum.ebnf):
    /// ```text
    /// variant_pattern = path , [ variant_pattern_data ] ;
    /// variant_pattern_data = '(' , pattern_list , ')'
    ///                      | '{' , field_patterns , '}' ;
    /// ```
    fn convert_variant_pattern(&mut self, node: &SyntaxNode) -> Option<Pattern> {
        let span = self.range_to_span(node.text_range());

        // Extract the path (type/variant name)
        let path = self.extract_path_from_pattern(node)?;

        // Check for variant data: tuple `(...)` or record `{...}`
        let data = self.extract_variant_data(node);

        Some(Pattern::new(
            PatternKind::Variant { path, data },
            span,
        ))
    }

    /// Extract a path from a pattern node.
    ///
    /// Looks for IDENT tokens or PATH/PATH_TYPE child nodes to build the path.
    fn extract_path_from_pattern(&mut self, node: &SyntaxNode) -> Option<Path> {
        let mut segments = List::new();

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    // Collect identifiers as path segments
                    // Stop when we hit punctuation that starts data (parens or braces)
                    match token.kind() {
                        SyntaxKind::IDENT => {
                            segments.push(PathSegment::Name(self.token_to_ident(&token)));
                        }
                        // Stop at delimiters that start the data part
                        SyntaxKind::L_PAREN | SyntaxKind::L_BRACE => break,
                        // Skip dots between path segments
                        SyntaxKind::DOT => {}
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    // Handle PATH or PATH_TYPE nodes
                    if matches!(child_node.kind(), SyntaxKind::PATH | SyntaxKind::PATH_TYPE) {
                        if let Some(path) = self.convert_path(&child_node) {
                            return Some(path);
                        }
                    }
                    // Handle PATH_SEGMENT nodes
                    if child_node.kind() == SyntaxKind::PATH_SEGMENT {
                        if let Some(ident) = self.find_ident_in_node(&child_node) {
                            segments.push(PathSegment::Name(ident));
                        }
                    }
                }
            }
        }

        if segments.is_empty() {
            return None;
        }

        let span = self.range_to_span(node.text_range());
        Some(Path::new(segments, span))
    }

    /// Extract field patterns from a record or variant pattern.
    ///
    /// Returns a tuple of (field patterns list, has rest flag).
    fn extract_field_patterns(&mut self, node: &SyntaxNode) -> (List<FieldPattern>, bool) {
        let mut fields = List::new();
        let mut has_rest = false;
        let mut in_braces = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::L_BRACE => in_braces = true,
                        SyntaxKind::R_BRACE => in_braces = false,
                        SyntaxKind::DOT_DOT if in_braces => has_rest = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    // Look for field pattern nodes
                    if in_braces {
                        if let Some(field) = self.convert_field_pattern(&child_node) {
                            fields.push(field);
                        }
                    }
                }
            }
        }

        (fields, has_rest)
    }

    /// Convert a single field pattern node.
    ///
    /// Handles:
    /// - Shorthand: `x` (equivalent to `x: x`)
    /// - Full: `x: pattern`
    fn convert_field_pattern(&mut self, node: &SyntaxNode) -> Option<FieldPattern> {
        let span = self.range_to_span(node.text_range());
        let mut name = None;
        let mut pattern = Maybe::None;
        let mut seen_colon = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::IDENT if name.is_none() => {
                            name = Some(self.token_to_ident(&token));
                        }
                        SyntaxKind::COLON => seen_colon = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    // If we have seen a colon, look for the pattern
                    if seen_colon && pattern.is_none() {
                        if let Some(pat) = self.convert_pattern(&child_node) {
                            pattern = Maybe::Some(pat);
                        }
                    }
                    // Also check if child is a pattern node without explicit identifier
                    if name.is_none() && child_node.kind() == SyntaxKind::IDENT_PAT {
                        if let Some(ident) = self.find_ident_in_node(&child_node) {
                            name = Some(ident);
                        }
                    }
                }
            }
        }

        // If no explicit name found, try to find one in the node text
        if name.is_none() {
            name = self.find_ident_in_node(node);
        }

        let name = name?;
        Some(FieldPattern::new(name, pattern, span))
    }

    /// Extract variant pattern data (tuple or record style).
    fn extract_variant_data(&mut self, node: &SyntaxNode) -> Maybe<VariantPatternData> {
        let mut in_parens = false;
        let mut in_braces = false;
        let mut tuple_patterns = List::new();
        let mut field_patterns = List::new();
        let mut has_rest = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::L_PAREN => in_parens = true,
                        SyntaxKind::R_PAREN => in_parens = false,
                        SyntaxKind::L_BRACE => in_braces = true,
                        SyntaxKind::R_BRACE => in_braces = false,
                        SyntaxKind::DOT_DOT if in_braces => has_rest = true,
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if in_parens {
                        // Tuple variant data
                        if let Some(pat) = self.convert_pattern(&child_node) {
                            tuple_patterns.push(pat);
                        }
                    } else if in_braces {
                        // Record variant data
                        if let Some(field) = self.convert_field_pattern(&child_node) {
                            field_patterns.push(field);
                        }
                    }
                }
            }
        }

        if !tuple_patterns.is_empty() {
            Maybe::Some(VariantPatternData::Tuple(tuple_patterns))
        } else if !field_patterns.is_empty() || has_rest {
            Maybe::Some(VariantPatternData::Record {
                fields: field_patterns,
                rest: has_rest,
            })
        } else {
            // Check if there were parens (even empty)
            let has_parens = node.child_tokens().any(|t| t.kind() == SyntaxKind::L_PAREN);
            let has_braces = node.child_tokens().any(|t| t.kind() == SyntaxKind::L_BRACE);

            if has_parens {
                // Empty tuple variant: `SomeVariant()`
                Maybe::Some(VariantPatternData::Tuple(List::new()))
            } else if has_braces {
                // Empty record variant with rest only: `SomeVariant { .. }`
                Maybe::Some(VariantPatternData::Record {
                    fields: List::new(),
                    rest: has_rest,
                })
            } else {
                // Unit variant: no data
                Maybe::None
            }
        }
    }

    /// Convert an array expression.
    ///
    /// Grammar: array_expr = '[' , array_elements , ']'
    ///          array_elements = expression_list | expression , ';' , expression
    ///
    /// Examples:
    /// - `[1, 2, 3]`  -> ArrayExpr::List
    /// - `[0; 10]`    -> ArrayExpr::Repeat { value, count }
    fn convert_array_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        use verum_ast::ArrayExpr;

        let span = self.range_to_span(node.text_range());
        let mut elements = List::new();
        let mut has_semicolon = false;
        let mut repeat_value = None;
        let mut repeat_count = None;

        // Check for semicolon to determine if this is a repeat expression
        for token in node.child_tokens() {
            if token.kind() == SyntaxKind::SEMICOLON {
                has_semicolon = true;
                break;
            }
        }

        for child in node.child_nodes() {
            if let Some(expr) = self.convert_expr(&child) {
                if has_semicolon {
                    // Repeat syntax: [value; count]
                    if repeat_value.is_none() {
                        repeat_value = Some(expr);
                    } else {
                        repeat_count = Some(expr);
                    }
                } else {
                    // List syntax: [a, b, c]
                    elements.push(expr);
                }
            }
        }

        let array_expr = if has_semicolon {
            ArrayExpr::Repeat {
                value: Heap::new(repeat_value?),
                count: Heap::new(repeat_count?),
            }
        } else {
            ArrayExpr::List(elements)
        };

        Some(Expr::new(ExprKind::Array(array_expr), span))
    }

    /// Convert a record expression.
    ///
    /// Grammar: record_expr = path , '{' , field_inits , '}'
    ///          field_inits = [ field_init , { ',' , field_init } , [ '..' , expression ] ]
    ///          field_init  = identifier , [ ':' , expression ]
    ///
    /// Examples:
    /// - `Point { x: 1, y: 2 }`
    /// - `Point { x, y }`               (shorthand)
    /// - `Point { x: 1, ..other }`      (struct update)
    fn convert_record_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        use verum_ast::FieldInit;

        let span = self.range_to_span(node.text_range());
        let mut path = None;
        let mut fields = List::new();
        let mut base = Maybe::None;

        for child in node.children() {
            match child {
                SyntaxElement::Node(child_node) => match child_node.kind() {
                    SyntaxKind::PATH | SyntaxKind::PATH_EXPR | SyntaxKind::PATH_SEGMENT => {
                        if path.is_none() {
                            path = self.convert_path(&child_node);
                        }
                    }
                    SyntaxKind::FIELD_LIST => {
                        // Parse field list
                        for field_child in child_node.child_nodes() {
                            if field_child.kind() == SyntaxKind::RECORD_FIELD {
                                if let Some(field) = self.convert_field_init(&field_child) {
                                    fields.push(field);
                                }
                            }
                        }
                    }
                    SyntaxKind::RECORD_FIELD => {
                        // Direct record field
                        if let Some(field) = self.convert_field_init(&child_node) {
                            fields.push(field);
                        }
                    }
                    _ => {
                        // Check for base expression after `..`
                        if path.is_some() && child_node.kind().can_start_expr() {
                            if let Some(expr) = self.convert_expr(&child_node) {
                                base = Maybe::Some(Heap::new(expr));
                            }
                        }
                    }
                },
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && path.is_none() {
                        let ident = self.token_to_ident(&token);
                        path = Some(Path::single(ident));
                    }
                }
            }
        }

        // If we didn't find explicit field nodes, scan for field inits directly
        if fields.is_empty() {
            let mut after_brace = false;
            let mut current_ident: Option<Ident> = None;
            let mut expect_value = false;

            for child in node.children() {
                match child {
                    SyntaxElement::Token(token) => match token.kind() {
                        SyntaxKind::L_BRACE => after_brace = true,
                        SyntaxKind::COLON if current_ident.is_some() => expect_value = true,
                        SyntaxKind::COMMA | SyntaxKind::R_BRACE => {
                            if let Some(ident) = current_ident.take() {
                                let field_span = ident.span;
                                fields.push(FieldInit::new(ident, Maybe::None, field_span));
                            }
                            expect_value = false;
                        }
                        SyntaxKind::IDENT if after_brace && !expect_value => {
                            // Save any pending field
                            if let Some(ident) = current_ident.take() {
                                let field_span = ident.span;
                                fields.push(FieldInit::new(ident, Maybe::None, field_span));
                            }
                            current_ident = Some(self.token_to_ident(&token));
                        }
                        SyntaxKind::DOT_DOT => {
                            // Struct update syntax
                            if let Some(ident) = current_ident.take() {
                                let field_span = ident.span;
                                fields.push(FieldInit::new(ident, Maybe::None, field_span));
                            }
                        }
                        _ => {}
                    },
                    SyntaxElement::Node(child_node) if expect_value => {
                        if let Some(expr) = self.convert_expr(&child_node) {
                            if let Some(ident) = current_ident.take() {
                                let field_span = ident.span;
                                fields.push(FieldInit::new(ident, Maybe::Some(expr), field_span));
                            }
                            expect_value = false;
                        }
                    }
                    _ => {}
                }
            }

            // Handle any remaining ident
            if let Some(ident) = current_ident {
                let field_span = ident.span;
                fields.push(FieldInit::new(ident, Maybe::None, field_span));
            }
        }

        Some(Expr::new(
            ExprKind::Record {
                path: path?,
                fields,
                base,
            },
            span,
        ))
    }

    /// Convert a field initialization in a record expression.
    fn convert_field_init(&mut self, node: &SyntaxNode) -> Option<verum_ast::FieldInit> {
        use verum_ast::FieldInit;

        let span = self.range_to_span(node.text_range());
        let mut name = None;
        let mut value = Maybe::None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && name.is_none() {
                        name = Some(self.token_to_ident(&token));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if name.is_some() && value.is_none() {
                        if let Some(expr) = self.convert_expr(&child_node) {
                            value = Maybe::Some(expr);
                        }
                    }
                }
            }
        }

        Some(FieldInit::new(name?, value, span))
    }

    /// Convert a closure expression.
    ///
    /// Grammar: closure_expr = [ 'async' ] , closure_params , [ '->' , type_expr ] , expression
    ///          closure_params = '|' , [ param_list_lambda ] , '|'
    ///
    /// Examples:
    /// - `|x| x + 1`
    /// - `|x: Int| -> Int { x + 1 }`
    /// - `async |x| fetch(x).await`
    /// - `move |x| capture(x)`
    fn convert_closure_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        use verum_ast::ClosureParam;

        let span = self.range_to_span(node.text_range());
        let mut is_async = false;
        let mut is_move = false;
        let mut params = List::new();
        let mut return_type = Maybe::None;
        let mut body = None;
        let mut in_params = false;
        let mut seen_arrow = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::ASYNC_KW => is_async = true,
                    SyntaxKind::MOVE_KW => is_move = true,
                    SyntaxKind::PIPE => {
                        in_params = !in_params;
                    }
                    SyntaxKind::ARROW => seen_arrow = true,
                    SyntaxKind::IDENT if in_params => {
                        // Simple identifier parameter without type
                        let ident = self.token_to_ident(&token);
                        let param_span = self.range_to_span(token.text_range());
                        let pattern = Pattern::new(
                            PatternKind::Ident {
                                by_ref: false,
                                mutable: false,
                                name: ident,
                                subpattern: Maybe::None,
                            },
                            param_span,
                        );
                        params.push(ClosureParam::new(pattern, Maybe::None, param_span));
                    }
                    _ => {}
                },
                SyntaxElement::Node(child_node) => match child_node.kind() {
                    SyntaxKind::PARAM_LIST => {
                        // Parse parameters from param list
                        for param_child in child_node.child_nodes() {
                            if let Some((pat, ty)) = self.convert_closure_param(&param_child) {
                                let param_span = self.range_to_span(param_child.text_range());
                                params.push(ClosureParam::new(pat, ty, param_span));
                            }
                        }
                    }
                    SyntaxKind::PARAM => {
                        // Direct param
                        if let Some((pat, ty)) = self.convert_closure_param(&child_node) {
                            let param_span = self.range_to_span(child_node.text_range());
                            params.push(ClosureParam::new(pat, ty, param_span));
                        }
                    }
                    // Explicit type node kinds that EventBasedParser emits
                    SyntaxKind::PATH_TYPE
                        | SyntaxKind::REFERENCE_TYPE
                        | SyntaxKind::TUPLE_TYPE
                        | SyntaxKind::FUNCTION_TYPE
                        | SyntaxKind::ARRAY_TYPE
                        | SyntaxKind::NEVER_TYPE
                        | SyntaxKind::INFER_TYPE
                        | SyntaxKind::REFINED_TYPE
                        | SyntaxKind::GENERIC_TYPE if seen_arrow && return_type.is_none() => {
                        return_type =
                            self.convert_type(&child_node).map(Maybe::Some).unwrap_or(Maybe::None);
                    }
                    kind if kind.can_start_type() && seen_arrow && return_type.is_none() => {
                        return_type =
                            self.convert_type(&child_node).map(Maybe::Some).unwrap_or(Maybe::None);
                    }
                    kind if kind.can_start_expr() || kind == SyntaxKind::BLOCK => {
                        if body.is_none() {
                            body = self.convert_expr(&child_node);
                        }
                    }
                    _ => {}
                },
            }
        }

        Some(Expr::new(
            ExprKind::Closure {
                async_: is_async,
                move_: is_move,
                params,
                contexts: List::new(),
                return_type,
                body: Heap::new(body?),
            },
            span,
        ))
    }

    /// Convert a closure parameter (pattern with optional type).
    fn convert_closure_param(&mut self, node: &SyntaxNode) -> Option<(Pattern, Maybe<Type>)> {
        let mut pattern = None;
        let mut ty = Maybe::None;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IDENT && pattern.is_none() {
                        let ident = self.token_to_ident(&token);
                        let param_span = self.range_to_span(token.text_range());
                        pattern = Some(Pattern::new(
                            PatternKind::Ident {
                                by_ref: false,
                                mutable: false,
                                name: ident,
                                subpattern: Maybe::None,
                            },
                            param_span,
                        ));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if pattern.is_none() {
                        pattern = self.convert_pattern(&child_node);
                    } else if ty.is_none() && child_node.kind().can_start_type() {
                        ty = self.convert_type(&child_node).map(Maybe::Some).unwrap_or(Maybe::None);
                    }
                }
            }
        }

        pattern.map(|p| (p, ty))
    }

    /// Convert an async expression.
    ///
    /// Grammar: async_expr = 'async' , block_expr
    ///
    /// Example: `async { fetch().await }`
    fn convert_async_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());

        for child in node.child_nodes() {
            if child.kind() == SyntaxKind::BLOCK || child.kind() == SyntaxKind::BLOCK_EXPR {
                let block = self.convert_block(&child);
                return Some(Expr::new(ExprKind::Async(block), span));
            }
        }

        None
    }

    /// Convert an await expression.
    ///
    /// Grammar: postfix_op = '.' , 'await'
    ///
    /// Example: `future.await`
    fn convert_await_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());

        for child in node.child_nodes() {
            if let Some(expr) = self.convert_expr(&child) {
                return Some(Expr::new(ExprKind::Await(Heap::new(expr)), span));
            }
        }

        None
    }

    /// Convert a loop expression.
    ///
    /// Grammar: infinite_loop = 'loop' , { loop_annotation } , block_expr
    ///
    /// Examples:
    /// - `loop { ... }`
    /// - `'label: loop { ... }`
    /// - `loop invariant x > 0 { ... }`
    fn convert_loop_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let label = self.find_label_text(node);
        let mut body = None;
        let mut invariants = Vec::new();

        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::BLOCK | SyntaxKind::BLOCK_EXPR => {
                    body = Some(self.convert_block(&child));
                }
                SyntaxKind::INVARIANT_CLAUSE => {
                    if let Some(expr) = self.convert_clause_expr(&child) {
                        invariants.push(expr);
                    }
                }
                _ => {}
            }
        }

        Some(Expr::new(
            ExprKind::Loop {
                label,
                body: body?,
                invariants: invariants.into(),
            },
            span,
        ))
    }

    /// Convert a while expression.
    ///
    /// Grammar: while_loop = 'while' , expression , { loop_annotation } , block_expr
    ///          loop_annotation = 'invariant' , expression | 'decreases' , expression
    ///
    /// Examples:
    /// - `while cond { ... }`
    /// - `'label: while x > 0 { ... }`
    /// - `while x > 0 invariant x >= 0 decreases x { ... }`
    fn convert_while_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let label = self.find_label_text(node);
        let mut condition = None;
        let mut body = None;
        let mut invariants = Vec::new();
        let mut decreases = Vec::new();

        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::BLOCK | SyntaxKind::BLOCK_EXPR => {
                    body = Some(self.convert_block(&child));
                }
                SyntaxKind::INVARIANT_CLAUSE => {
                    if let Some(expr) = self.convert_clause_expr(&child) {
                        invariants.push(expr);
                    }
                }
                SyntaxKind::DECREASES_CLAUSE => {
                    if let Some(expr) = self.convert_clause_expr(&child) {
                        decreases.push(expr);
                    }
                }
                kind if kind.can_start_expr() && condition.is_none() => {
                    condition = self.convert_expr(&child);
                }
                _ => {}
            }
        }

        Some(Expr::new(
            ExprKind::While {
                label,
                condition: Heap::new(condition?),
                body: body?,
                invariants: invariants.into(),
                decreases: decreases.into(),
            },
            span,
        ))
    }

    /// Convert a for expression.
    ///
    /// Grammar: for_loop = 'for' , pattern , 'in' , expression , { loop_annotation } , block_expr
    ///
    /// Examples:
    /// - `for x in iter { ... }`
    /// - `for (k, v) in map { ... }`
    /// - `'label: for i in 0..n invariant i <= n decreases n - i { ... }`
    fn convert_for_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let label = self.find_label_text(node);
        let mut pattern = None;
        let mut iter_expr = None;
        let mut body = None;
        let mut invariants = Vec::new();
        let mut decreases = Vec::new();
        let mut seen_in = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if token.kind() == SyntaxKind::IN_KW {
                        seen_in = true;
                    }
                }
                SyntaxElement::Node(child_node) => match child_node.kind() {
                    SyntaxKind::BLOCK | SyntaxKind::BLOCK_EXPR => {
                        body = Some(self.convert_block(&child_node));
                    }
                    SyntaxKind::INVARIANT_CLAUSE => {
                        if let Some(expr) = self.convert_clause_expr(&child_node) {
                            invariants.push(expr);
                        }
                    }
                    SyntaxKind::DECREASES_CLAUSE => {
                        if let Some(expr) = self.convert_clause_expr(&child_node) {
                            decreases.push(expr);
                        }
                    }
                    _ if !seen_in => {
                        // Before 'in', look for pattern
                        if pattern.is_none() {
                            // Try to parse as pattern
                            if let Some(pat) = self.convert_pattern(&child_node) {
                                pattern = Some(pat);
                            } else if let Some(ident) = self.find_ident_in_node(&child_node) {
                                // Simple identifier pattern
                                let pat_span = self.range_to_span(child_node.text_range());
                                pattern = Some(Pattern::new(
                                    PatternKind::Ident {
                                        by_ref: false,
                                        mutable: false,
                                        name: ident,
                                        subpattern: Maybe::None,
                                    },
                                    pat_span,
                                ));
                            }
                        }
                    }
                    kind if seen_in && kind.can_start_expr() && iter_expr.is_none() => {
                        // After 'in', look for iterator expression
                        iter_expr = self.convert_expr(&child_node);
                    }
                    _ => {}
                },
            }
        }

        // Handle simple ident pattern from token
        if pattern.is_none() {
            for child in node.child_tokens() {
                if child.kind() == SyntaxKind::IDENT {
                    let ident = self.token_to_ident(&child);
                    let pat_span = self.range_to_span(child.text_range());
                    pattern = Some(Pattern::new(
                        PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: ident,
                            subpattern: Maybe::None,
                        },
                        pat_span,
                    ));
                    break;
                }
            }
        }

        Some(Expr::new(
            ExprKind::For {
                label,
                pattern: pattern?,
                iter: Heap::new(iter_expr?),
                body: body?,
                invariants: invariants.into(),
                decreases: decreases.into(),
            },
            span,
        ))
    }

    /// Convert a range expression.
    ///
    /// Grammar: range_expr = logical_or_expr , [ range_op , logical_or_expr ]
    ///          range_op = '..' | '..='
    ///
    /// Examples:
    /// - `0..10`    (exclusive)
    /// - `0..=10`   (inclusive)
    /// - `..10`     (from start)
    /// - `0..`      (to end)
    /// - `..`       (full range)
    fn convert_range_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut start = Maybe::None;
        let mut end = Maybe::None;
        let mut inclusive = false;
        let mut seen_range_op = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::DOT_DOT => {
                        seen_range_op = true;
                        inclusive = false;
                    }
                    SyntaxKind::DOT_DOT_EQ => {
                        seen_range_op = true;
                        inclusive = true;
                    }
                    _ => {}
                },
                SyntaxElement::Node(child_node) => {
                    if let Some(expr) = self.convert_expr(&child_node) {
                        if !seen_range_op {
                            start = Maybe::Some(Heap::new(expr));
                        } else {
                            end = Maybe::Some(Heap::new(expr));
                        }
                    }
                }
            }
        }

        Some(Expr::new(
            ExprKind::Range {
                start,
                end,
                inclusive,
            },
            span,
        ))
    }

    /// Convert a try expression from CST to AST.
    ///
    /// Grammar (EBNF v2.8):
    /// ```ebnf
    /// try_expr        = 'try' , block_expr , [ try_handlers ] ;
    /// try_handlers    = try_recovery , [ try_finally ]
    ///                 | try_finally ;
    /// try_recovery    = 'recover' , recover_body ;
    /// recover_body    = recover_match_arms | recover_closure ;
    /// recover_match_arms = '{' , match_arms , '}' ;
    /// recover_closure = closure_params , recover_closure_body ;
    /// recover_closure_body = block_expr | expression ;
    /// try_finally     = 'finally' , block_expr ;
    /// ```
    ///
    /// Try expression with structured error recovery: `try { body } recover(e) { handler } finally { cleanup }`
    /// The recover clause binds the error, and finally runs unconditionally.
    fn convert_try_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut try_block: Option<Expr> = None;
        let mut recover_body: Option<RecoverBody> = None;
        let mut finally_block: Option<Expr> = None;
        let mut seen_finally_kw = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::TRY_KW | SyntaxKind::RECOVER_KW => {
                            // Skip try/recover keywords
                        }
                        SyntaxKind::FINALLY_KW => {
                            seen_finally_kw = true;
                        }
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::BLOCK | SyntaxKind::BLOCK_EXPR => {
                            let block = self.convert_block(&child_node);
                            let block_span = self.range_to_span(child_node.text_range());
                            let block_expr = Expr::new(ExprKind::Block(block), block_span);
                            if try_block.is_none() {
                                // First block is the try block
                                try_block = Some(block_expr);
                            } else if seen_finally_kw && finally_block.is_none() {
                                // Block after 'finally' keyword
                                finally_block = Some(block_expr);
                            }
                        }
                        SyntaxKind::RECOVER_ARMS => {
                            // Match arms syntax: recover { pattern => expr, ... }
                            recover_body = self.convert_recover_arms(&child_node);
                        }
                        SyntaxKind::RECOVER_CLOSURE => {
                            // Closure syntax: recover |e| expr
                            recover_body = self.convert_recover_closure(&child_node);
                        }
                        kind if kind.can_start_expr() => {
                            // Generic expression handling for edge cases
                            if let Some(expr) = self.convert_expr(&child_node) {
                                if try_block.is_none() {
                                    try_block = Some(expr);
                                } else if seen_finally_kw && finally_block.is_none() {
                                    finally_block = Some(expr);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Build the appropriate expression based on what was found
        let try_block = try_block?;

        match (recover_body, finally_block) {
            (Some(recover), Some(finally_expr)) => {
                // try { } recover { } finally { }
                Some(Expr::new(
                    ExprKind::TryRecoverFinally {
                        try_block: Heap::new(try_block),
                        recover,
                        finally_block: Heap::new(finally_expr),
                    },
                    span,
                ))
            }
            (Some(recover), None) => {
                // try { } recover { }
                Some(Expr::new(
                    ExprKind::TryRecover {
                        try_block: Heap::new(try_block),
                        recover,
                    },
                    span,
                ))
            }
            (None, Some(finally_expr)) => {
                // try { } finally { }
                Some(Expr::new(
                    ExprKind::TryFinally {
                        try_block: Heap::new(try_block),
                        finally_block: Heap::new(finally_expr),
                    },
                    span,
                ))
            }
            (None, None) => {
                // Just try { } - return the block itself
                Some(try_block)
            }
        }
    }

    /// Convert recover match arms: recover { pattern => expr, ... }
    fn convert_recover_arms(&mut self, node: &SyntaxNode) -> Option<RecoverBody> {
        let span = self.range_to_span(node.text_range());
        let mut arms = List::new();

        for child in node.child_nodes() {
            match child.kind() {
                SyntaxKind::MATCH_ARM_LIST => {
                    // Nested match arm list
                    for arm_node in child.child_nodes() {
                        if arm_node.kind() == SyntaxKind::MATCH_ARM {
                            if let Some(arm) = self.convert_match_arm(&arm_node) {
                                arms.push(arm);
                            }
                        }
                    }
                }
                SyntaxKind::MATCH_ARM => {
                    // Direct match arm
                    if let Some(arm) = self.convert_match_arm(&child) {
                        arms.push(arm);
                    }
                }
                _ => {}
            }
        }

        Some(RecoverBody::MatchArms { arms, span })
    }

    /// Convert recover closure: recover |e| expr or recover |e| { ... }
    fn convert_recover_closure(&mut self, node: &SyntaxNode) -> Option<RecoverBody> {
        let span = self.range_to_span(node.text_range());
        let mut param_pattern: Option<Pattern> = None;
        let mut param_ty: Maybe<Type> = Maybe::None;
        let mut body: Option<Expr> = None;
        let mut seen_first_pipe = false;
        let mut seen_second_pipe = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PIPE => {
                            if !seen_first_pipe {
                                seen_first_pipe = true;
                            } else {
                                seen_second_pipe = true;
                            }
                        }
                        SyntaxKind::IDENT if seen_first_pipe && !seen_second_pipe => {
                            // Parameter identifier
                            let name_span = self.range_to_span(token.text_range());
                            param_pattern = Some(Pattern::new(
                                PatternKind::Ident {
                                    by_ref: false,
                                    mutable: false,
                                    name: Ident {
                                        name: Text::from(token.text()),
                                        span: name_span,
                                    },
                                    subpattern: Maybe::None,
                                },
                                name_span,
                            ));
                        }
                        SyntaxKind::UNDERSCORE if seen_first_pipe && !seen_second_pipe => {
                            // Wildcard pattern
                            let name_span = self.range_to_span(token.text_range());
                            param_pattern = Some(Pattern::new(PatternKind::Wildcard, name_span));
                        }
                        _ => {}
                    }
                }
                SyntaxElement::Node(child_node) => {
                    let kind = child_node.kind();
                    if seen_first_pipe && !seen_second_pipe {
                        // Inside closure params: |pattern: Type|
                        match kind {
                            // Pattern kinds
                            SyntaxKind::IDENT_PAT
                            | SyntaxKind::WILDCARD_PAT
                            | SyntaxKind::TUPLE_PAT
                            | SyntaxKind::RECORD_PAT
                            | SyntaxKind::VARIANT_PAT => {
                                param_pattern = self.convert_pattern(&child_node);
                            }
                            // Type annotation kinds
                            SyntaxKind::PATH_TYPE
                            | SyntaxKind::GENERIC_TYPE
                            | SyntaxKind::REFERENCE_TYPE
                            | SyntaxKind::TUPLE_TYPE
                            | SyntaxKind::FUNCTION_TYPE
                            | SyntaxKind::ARRAY_TYPE
                            | SyntaxKind::NEVER_TYPE
                            | SyntaxKind::INFER_TYPE => {
                                if let Some(ty) = self.convert_type(&child_node) {
                                    param_ty = Maybe::Some(ty);
                                }
                            }
                            _ => {}
                        }
                    } else if seen_second_pipe && body.is_none() {
                        // Body expression (after second pipe)
                        body = self.convert_expr(&child_node);
                    }
                }
            }
        }

        let param_pattern = param_pattern?;
        let param_span = param_pattern.span;
        let param = RecoverClosureParam::new(param_pattern, param_ty, param_span);
        let body = body?;

        Some(RecoverBody::Closure {
            param,
            body: Heap::new(body),
            span,
        })
    }

    fn convert_throw_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let value = node.child_nodes().find_map(|n| self.convert_expr(&n))?;
        Some(Expr::new(ExprKind::Throw(Heap::new(value)), span))
    }

    fn convert_pipeline_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut left = None;
        let mut right = None;

        for child in node.child_nodes() {
            if left.is_none() {
                left = self.convert_expr(&child);
            } else if right.is_none() {
                right = self.convert_expr(&child);
            }
        }

        Some(Expr::new(
            ExprKind::Pipeline {
                left: Heap::new(left?),
                right: Heap::new(right?),
            },
            span,
        ))
    }

    fn convert_ref_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let expr = node.child_nodes().find_map(|n| self.convert_expr(&n))?;
        Some(Expr::new(
            ExprKind::Unary {
                op: UnOp::Ref,
                expr: Heap::new(expr),
            },
            span,
        ))
    }

    fn convert_deref_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let expr = node.child_nodes().find_map(|n| self.convert_expr(&n))?;
        Some(Expr::new(
            ExprKind::Unary {
                op: UnOp::Deref,
                expr: Heap::new(expr),
            },
            span,
        ))
    }

    fn convert_cast_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        let span = self.range_to_span(node.text_range());
        let mut expr = None;
        let mut ty = None;

        for child in node.child_nodes() {
            if expr.is_none() {
                expr = self.convert_expr(&child);
            } else if ty.is_none() {
                ty = self.convert_type(&child);
            }
        }

        Some(Expr::new(
            ExprKind::Cast {
                expr: Heap::new(expr?),
                ty: ty?,
            },
            span,
        ))
    }
}

/// Public function to convert a syntax tree to semantic AST.
pub fn syntax_to_ast(source: &str, root: &SyntaxNode, file_id: FileId) -> AstSinkResult {
    let sink = AstSink::new(source, file_id);
    sink.convert(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax_bridge::LosslessParser;

    // Helper function to convert source to AST using full pipeline
    fn parse_to_ast(source: &str) -> AstSinkResult {
        let file_id = FileId::new(0);
        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);
        AstSinkResult {
            module: result.module,
            errors: result.errors,
        }
    }

    // ========================================================================
    // Basic Sink Tests - Unit tests for internal methods
    // ========================================================================

    #[test]
    fn test_ast_sink_basic() {
        let sink = AstSink::new("fn foo() {}", FileId::new(0));
        assert!(sink.errors.is_empty());
    }

    #[test]
    fn test_parse_int_literal() {
        let sink = AstSink::new("", FileId::new(0));
        assert_eq!(sink.parse_int_literal("42"), 42);
        assert_eq!(sink.parse_int_literal("0xff"), 255);
        assert_eq!(sink.parse_int_literal("0b1010"), 10);
        assert_eq!(sink.parse_int_literal("0o77"), 63);
        assert_eq!(sink.parse_int_literal("1_000_000"), 1_000_000);
    }

    #[test]
    fn test_unescape_string() {
        let sink = AstSink::new("", FileId::new(0));
        assert_eq!(sink.unescape_string("hello\\nworld"), "hello\nworld");
        assert_eq!(sink.unescape_string("tab\\there"), "tab\there");
        assert_eq!(sink.unescape_string("quote\\\"test"), "quote\"test");
    }

    #[test]
    fn test_unescape_string_backslash() {
        let sink = AstSink::new("", FileId::new(0));
        assert_eq!(sink.unescape_string("back\\\\slash"), "back\\slash");
        assert_eq!(sink.unescape_string("\\r\\n"), "\r\n");
    }

    // ========================================================================
    // Int Literal Edge Cases
    // ========================================================================

    #[test]
    fn test_parse_int_literal_edge_cases() {
        let sink = AstSink::new("", FileId::new(0));

        // Zero
        assert_eq!(sink.parse_int_literal("0"), 0);

        // Large numbers
        assert_eq!(sink.parse_int_literal("999999999"), 999999999);

        // Hex with uppercase
        assert_eq!(sink.parse_int_literal("0xFF"), 255);
        assert_eq!(sink.parse_int_literal("0XFF"), 255);

        // Binary edge cases
        assert_eq!(sink.parse_int_literal("0b0"), 0);
        assert_eq!(sink.parse_int_literal("0b1"), 1);
        assert_eq!(sink.parse_int_literal("0b11111111"), 255);

        // Octal edge cases
        assert_eq!(sink.parse_int_literal("0o0"), 0);
        assert_eq!(sink.parse_int_literal("0o7"), 7);
    }

    // ========================================================================
    // String Escape Sequence Tests
    // ========================================================================

    #[test]
    fn test_unescape_string_all_escapes() {
        let sink = AstSink::new("", FileId::new(0));

        // All standard escape sequences
        assert_eq!(sink.unescape_string("\\n"), "\n");
        assert_eq!(sink.unescape_string("\\r"), "\r");
        assert_eq!(sink.unescape_string("\\t"), "\t");
        assert_eq!(sink.unescape_string("\\\\"), "\\");
        assert_eq!(sink.unescape_string("\\0"), "\0");
        assert_eq!(sink.unescape_string("\\'"), "'");
        assert_eq!(sink.unescape_string("\\\""), "\"");
    }

    #[test]
    fn test_unescape_string_mixed_content() {
        let sink = AstSink::new("", FileId::new(0));

        // Mixed content
        assert_eq!(
            sink.unescape_string("line1\\nline2\\nline3"),
            "line1\nline2\nline3"
        );
        assert_eq!(
            sink.unescape_string("\\tindented\\tcontent"),
            "\tindented\tcontent"
        );
    }

    #[test]
    fn test_unescape_string_no_escapes() {
        let sink = AstSink::new("", FileId::new(0));

        // No escapes
        assert_eq!(sink.unescape_string("hello world"), "hello world");
        assert_eq!(sink.unescape_string(""), "");
    }

    // ========================================================================
    // End-to-End Module Conversion Tests - Basic structure
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_empty_module() {
        let result = parse_to_ast("");
        assert!(result.errors.is_empty());
        assert!(result.module.items.is_empty());
    }

    #[test]
    fn test_syntax_to_ast_simple_function() {
        let result = parse_to_ast("fn foo() { }");

        // Module should have one item
        assert_eq!(result.module.items.len(), 1);

        // Item should be a function
        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    fn test_syntax_to_ast_function_name_parsing() {
        let result = parse_to_ast("fn my_function_name() { }");

        assert_eq!(result.module.items.len(), 1);
        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "my_function_name");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    fn test_syntax_to_ast_multiple_functions() {
        let result = parse_to_ast("fn foo() { } fn bar() { }");

        assert_eq!(result.module.items.len(), 2);

        if let ItemKind::Function(func1) = &result.module.items[0].kind {
            assert_eq!(func1.name.name.as_str(), "foo");
        } else {
            panic!("Expected first function");
        }

        if let ItemKind::Function(func2) = &result.module.items[1].kind {
            assert_eq!(func2.name.name.as_str(), "bar");
        } else {
            panic!("Expected second function");
        }
    }

    // ========================================================================
    // Type Definition Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_type_def() {
        let result = parse_to_ast("type Point is { x: Float, y: Float };");

        assert_eq!(result.module.items.len(), 1);
        if let ItemKind::Type(type_decl) = &result.module.items[0].kind {
            assert_eq!(type_decl.name.name.as_str(), "Point");
        } else {
            panic!("Expected type item, got {:?}", result.module.items[0].kind);
        }
    }

    #[test]
    fn test_syntax_to_ast_type_def_name() {
        let result = parse_to_ast("type MyCustomType is { };");

        if result.module.items.len() == 1 {
            if let ItemKind::Type(type_decl) = &result.module.items[0].kind {
                assert_eq!(type_decl.name.name.as_str(), "MyCustomType");
            }
        }
    }

    // ========================================================================
    // Span Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_preserves_spans() {
        let result = parse_to_ast("fn foo() { }");

        // Module should have a span covering the entire source
        assert!(result.module.span.start <= result.module.span.end);

        // Function should have a span
        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.span.start <= func.span.end);
        } else {
            panic!("Expected function");
        }
    }

    // ========================================================================
    // Visibility Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_preserves_visibility_default() {
        let result = parse_to_ast("fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            // Default visibility should be private
            assert_eq!(func.visibility, Visibility::Private);
        } else {
            panic!("Expected function");
        }
    }

    // ========================================================================
    // AstSinkResult Tests
    // ========================================================================

    #[test]
    fn test_ast_sink_result_module_file_id() {
        let result = parse_to_ast("fn foo() { }");
        assert_eq!(result.module.file_id, FileId::new(0));
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_with_error_nodes() {
        // Invalid syntax that creates ERROR nodes
        let result = parse_to_ast("fn foo( { }");

        // Should still produce a module (possibly incomplete)
        // The parser creates ERROR nodes for malformed sections
        assert!(result.module.items.len() <= 1);
    }

    #[test]
    fn test_syntax_to_ast_recovers_from_errors() {
        // First function malformed, second should still parse
        let result = parse_to_ast("fn foo( fn bar() { }");

        // May have partial recovery
        // The module should exist even with errors
        let _ = result.module;
    }

    // ========================================================================
    // Multiple Item Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_three_functions() {
        let result = parse_to_ast("fn a() { } fn b() { } fn c() { }");

        assert_eq!(result.module.items.len(), 3);
    }

    #[test]
    fn test_syntax_to_ast_mixed_items() {
        // Function followed by type
        let result = parse_to_ast("fn foo() { } type Bar is { };");

        assert_eq!(result.module.items.len(), 2);

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
        }
        if let ItemKind::Type(ty) = &result.module.items[1].kind {
            assert_eq!(ty.name.name.as_str(), "Bar");
        }
    }

    // ========================================================================
    // Function Body Tests - Check body exists
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_function_has_body() {
        let result = parse_to_ast("fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.body.is_some(), "Function should have a body");
        } else {
            panic!("Expected function item");
        }
    }

    // ========================================================================
    // Public Visibility Tests
    // NOTE: Requires EventBasedParser to emit PUB_KW tokens in FN_DEF nodes
    // ========================================================================

    #[test]    fn test_syntax_to_ast_pub_function() {
        let result = parse_to_ast("pub fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.visibility, Visibility::Public);
            assert_eq!(func.name.name.as_str(), "foo");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_pub_type() {
        let result = parse_to_ast("pub type Foo is { };");

        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            assert_eq!(ty.visibility, Visibility::Public);
            assert_eq!(ty.name.name.as_str(), "Foo");
        } else {
            panic!("Expected type item");
        }
    }

    // ========================================================================
    // Async/Pure/Meta Function Tests
    // NOTE: Requires EventBasedParser to emit modifier tokens in FN_DEF nodes
    // ========================================================================

    #[test]    fn test_syntax_to_ast_async_function() {
        let result = parse_to_ast("async fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.is_async);
            assert_eq!(func.name.name.as_str(), "foo");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_pure_function() {
        let result = parse_to_ast("pure fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.is_pure);
            assert_eq!(func.name.name.as_str(), "foo");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_meta_function() {
        let result = parse_to_ast("meta fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.is_meta);
            assert_eq!(func.stage_level, 1);
            assert_eq!(func.name.name.as_str(), "foo");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_unsafe_function() {
        let result = parse_to_ast("unsafe fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.is_unsafe);
            assert_eq!(func.name.name.as_str(), "foo");
        } else {
            panic!("Expected function item");
        }
    }

    // ========================================================================
    // Function Parameter Tests
    // NOTE: Requires EventBasedParser to emit PARAM_LIST and PARAM nodes
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_function_with_params() {
        let result = parse_to_ast("fn foo(x: Int) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
            assert_eq!(func.params.len(), 1);

            if let FunctionParamKind::Regular { pattern, ty, .. } = &func.params[0].kind {
                if let PatternKind::Ident { name, .. } = &pattern.kind {
                    assert_eq!(name.name.as_str(), "x");
                }
                assert!(matches!(ty.kind, TypeKind::Path(_)));
            } else {
                panic!("Expected regular param");
            }
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_function_multiple_params() {
        let result = parse_to_ast("fn foo(x: Int, y: Int) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.params.len(), 2);
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_function_self_param() {
        let result = parse_to_ast("fn foo(self) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.params.len(), 1);
            assert!(matches!(func.params[0].kind, FunctionParamKind::SelfValue));
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_function_ref_self_param() {
        let result = parse_to_ast("fn foo(&self) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.params.len(), 1);
            assert!(matches!(func.params[0].kind, FunctionParamKind::SelfRef));
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_function_ref_mut_self_param() {
        let result = parse_to_ast("fn foo(&mut self) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.params.len(), 1);
            assert!(matches!(func.params[0].kind, FunctionParamKind::SelfRefMut));
        } else {
            panic!("Expected function item");
        }
    }

    // ========================================================================
    // Return Type Tests
    // NOTE: Requires EventBasedParser to emit return type nodes
    // ========================================================================

    #[test]    fn test_syntax_to_ast_function_with_return_type() {
        let result = parse_to_ast("fn foo() -> Int { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.return_type.is_some());
            if let Maybe::Some(ref ret_ty) = func.return_type {
                assert!(matches!(ret_ty.kind, TypeKind::Path(_)));
            }
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    fn test_syntax_to_ast_function_without_return_type() {
        let result = parse_to_ast("fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            // No return type should be None
            assert!(func.return_type.is_none());
        } else {
            panic!("Expected function item");
        }
    }

    // ========================================================================
    // Generic Parameter Tests
    // NOTE: Requires EventBasedParser to emit GENERIC_PARAMS nodes
    // ========================================================================

    #[test]    fn test_syntax_to_ast_generic_function() {
        let result = parse_to_ast("fn foo<T>() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
            assert_eq!(func.generics.len(), 1);
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_multiple_generic_params() {
        let result = parse_to_ast("fn foo<T, U, V>() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.generics.len(), 3);
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_generic_type() {
        let result = parse_to_ast("type Container<T> is { value: T };");

        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            assert_eq!(ty.name.name.as_str(), "Container");
            assert_eq!(ty.generics.len(), 1);
        } else {
            panic!("Expected type item");
        }
    }

    // ========================================================================
    // Type Definition Body Tests
    // NOTE: Requires EventBasedParser to emit FIELD_LIST/VARIANT_LIST nodes
    // ========================================================================

    #[test]    fn test_syntax_to_ast_record_type_with_fields() {
        let result = parse_to_ast("type Point is { x: Float, y: Float };");

        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            if let TypeDeclBody::Record(fields) = &ty.body {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name.name.as_str(), "x");
                assert_eq!(fields[1].name.name.as_str(), "y");
            } else {
                panic!("Expected record body");
            }
        } else {
            panic!("Expected type item");
        }
    }

    #[test]    fn test_syntax_to_ast_variant_type() {
        let result = parse_to_ast("type Option<T> is None | Some(T);");

        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            assert_eq!(ty.name.name.as_str(), "Option");
            if let TypeDeclBody::Variant(variants) = &ty.body {
                assert_eq!(variants.len(), 2);
            } else {
                panic!("Expected variant body, got {:?}", ty.body);
            }
        } else {
            panic!("Expected type item");
        }
    }

    #[test]
    fn test_syntax_to_ast_unit_type() {
        let result = parse_to_ast("type Unit is ();");

        // This might work since it's a simple type alias to unit
        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            assert_eq!(ty.name.name.as_str(), "Unit");
        }
    }

    #[test]    fn test_syntax_to_ast_type_alias() {
        let result = parse_to_ast("type IntList is List<Int>;");

        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            assert_eq!(ty.name.name.as_str(), "IntList");
            assert!(matches!(ty.body, TypeDeclBody::Alias(_)));
        } else {
            panic!("Expected type item");
        }
    }

    // ========================================================================
    // Impl Block Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_impl_block() {
        let result = parse_to_ast("implement Point { }");

        if result.module.items.len() == 1 {
            assert!(matches!(result.module.items[0].kind, ItemKind::Impl(_)));
        }
    }

    #[test]
    fn test_syntax_to_ast_impl_with_method() {
        let result = parse_to_ast("implement Point { fn new() { } }");

        if result.module.items.len() == 1 {
            if let ItemKind::Impl(impl_decl) = &result.module.items[0].kind {
                let _ = impl_decl;
            }
        }
    }

    // ========================================================================
    // Module and Link Statement Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_link_statement() {
        let result = parse_to_ast("link std.io;");

        if result.module.items.len() == 1 {
            assert!(matches!(result.module.items[0].kind, ItemKind::Mount(_)));
        }
    }

    // ========================================================================
    // Block and Statement Tests
    // ========================================================================

    #[test]    fn test_syntax_to_ast_block_with_let() {
        let result = parse_to_ast("fn foo() { let x = 1; }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            if let Maybe::Some(FunctionBody::Block(block)) = &func.body {
                assert!(!block.stmts.is_empty() || block.expr.is_some());
            }
        }
    }

    // ========================================================================
    // Attribute Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_function_with_attribute() {
        let result = parse_to_ast("@inline fn foo() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
        } else {
            panic!("Expected function item");
        }
    }

    // ========================================================================
    // Complex Scenarios
    // NOTE: These test multiple features that need EventBasedParser support
    // ========================================================================

    #[test]    fn test_syntax_to_ast_complex_function() {
        let result = parse_to_ast("pub async fn fetch<T>(url: Text) -> T { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.visibility, Visibility::Public);
            assert!(func.is_async);
            assert_eq!(func.name.name.as_str(), "fetch");
            assert_eq!(func.generics.len(), 1);
            assert_eq!(func.params.len(), 1);
            assert!(func.return_type.is_some());
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    #[ignore = "generator fn* syntax not yet supported by event-based parser"]
    fn test_syntax_to_ast_generator_function() {
        let result = parse_to_ast("fn* gen() { }");

        assert!(
            !result.module.items.is_empty(),
            "Parser should produce at least one item for 'fn* gen() {{ }}'",
        );
        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert!(func.is_generator);
            assert_eq!(func.name.name.as_str(), "gen");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_function_with_where_clause() {
        let result = parse_to_ast("fn foo<T>() where T: Debug { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
            assert!(func.generic_where_clause.is_some());
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_pub_type_with_pub_fields() {
        let result = parse_to_ast("pub type Point is { pub x: Float, pub y: Float };");

        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            assert_eq!(ty.visibility, Visibility::Public);
            if let TypeDeclBody::Record(fields) = &ty.body {
                assert!(fields.iter().all(|f| f.visibility == Visibility::Public));
            }
        }
    }

    // ========================================================================
    // Edge Cases
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_unicode_identifier() {
        let result = parse_to_ast("fn foo_αβγ() { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo_αβγ");
        } else {
            panic!("Expected function item");
        }
    }

    #[test]    fn test_syntax_to_ast_deeply_nested_generics() {
        let result = parse_to_ast("type Deep is Map<Text, List<Maybe<Int>>>;");

        if let ItemKind::Type(ty) = &result.module.items[0].kind {
            assert_eq!(ty.name.name.as_str(), "Deep");
            assert!(matches!(ty.body, TypeDeclBody::Alias(_)));
        }
    }

    #[test]
    fn test_syntax_to_ast_whitespace_preservation() {
        let result = parse_to_ast("fn   foo  (  )   {   }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
        }
    }

    #[test]    fn test_syntax_to_ast_multiline_function() {
        let result = parse_to_ast(
            r#"fn foo(
    x: Int,
    y: Int
) -> Int {
}"#,
        );

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.name.name.as_str(), "foo");
            assert_eq!(func.params.len(), 2);
            assert!(func.return_type.is_some());
        }
    }

    // ========================================================================
    // Protocol Definition Tests
    // ========================================================================

    #[test]    fn test_syntax_to_ast_protocol_def() {
        let result = parse_to_ast("type Printable is protocol { fn print(&self); };");

        if let ItemKind::Protocol(proto) = &result.module.items[0].kind {
            assert_eq!(proto.name.name.as_str(), "Printable");
            assert!(!proto.items.is_empty());
        }
    }

    #[test]    fn test_syntax_to_ast_empty_protocol() {
        let result = parse_to_ast("type Empty is protocol { };");

        if let ItemKind::Protocol(proto) = &result.module.items[0].kind {
            assert_eq!(proto.name.name.as_str(), "Empty");
        }
    }

    // ========================================================================
    // Const and Static Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_const_def() {
        let result = parse_to_ast("const PI: Float = 3.14;");

        if result.module.items.len() == 1 {
            assert!(matches!(result.module.items[0].kind, ItemKind::Const(_)));
        }
    }

    #[test]
    fn test_syntax_to_ast_static_def() {
        let result = parse_to_ast("static COUNTER: Int = 0;");

        if result.module.items.len() == 1 {
            assert!(matches!(result.module.items[0].kind, ItemKind::Static(_)));
        }
    }

    // ========================================================================
    // Tuple Type Tests
    // ========================================================================

    #[test]    fn test_syntax_to_ast_tuple_type_param() {
        let result = parse_to_ast("fn foo(pair: (Int, Int)) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.params.len(), 1);
            if let FunctionParamKind::Regular { ty, .. } = &func.params[0].kind {
                assert!(matches!(ty.kind, TypeKind::Tuple(_)));
            }
        }
    }

    // ========================================================================
    // Reference Type Tests
    // ========================================================================

    #[test]
    fn test_syntax_to_ast_reference_param() {
        let result = parse_to_ast("fn foo(x: &Int) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            assert_eq!(func.params.len(), 1);
            if let FunctionParamKind::Regular { ty, .. } = &func.params[0].kind {
                assert!(matches!(ty.kind, TypeKind::Reference { .. }));
            }
        }
    }

    #[test]    fn test_syntax_to_ast_mut_reference_param() {
        let result = parse_to_ast("fn foo(x: &mut Int) { }");

        if let ItemKind::Function(func) = &result.module.items[0].kind {
            if let FunctionParamKind::Regular { ty, .. } = &func.params[0].kind {
                if let TypeKind::Reference { mutable, .. } = &ty.kind {
                    assert!(*mutable);
                }
            }
        }
    }
}
