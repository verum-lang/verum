//! Find references support
//!
//! Finds all references to a symbol in the document using both text-based
//! and AST-based search for accurate results.

use crate::document::DocumentState;
use crate::position_utils::ast_span_to_range;
use tower_lsp::lsp_types::*;
use verum_ast::expr::RecoverBody;
use verum_ast::{ExprKind, ItemKind, PatternKind, StmtKind};
use verum_common::List;

/// Check if a character can be part of an identifier
pub fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Reference kind for categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// Definition of the symbol
    Definition,
    /// Read access to the symbol
    Read,
    /// Write access to the symbol
    Write,
    /// Call of a function
    Call,
}

/// A reference with additional metadata
#[derive(Debug, Clone)]
pub struct Reference {
    pub location: Location,
    pub kind: ReferenceKind,
}

/// Find all references to a symbol at the given position
pub fn find_references(
    document: &DocumentState,
    position: Position,
    uri: &Url,
    include_declaration: bool,
) -> List<Location> {
    let mut locations = List::new();

    // Get the word at the position
    let Some(word) = document.word_at_position(position) else {
        return locations;
    };

    // First, try AST-based search for more accurate results
    if let Some(module) = &document.module {
        let refs = find_ast_references(module, &word, uri, &document.text);
        for reference in refs {
            // Skip definition if not requested
            if !include_declaration && reference.kind == ReferenceKind::Definition {
                continue;
            }
            locations.push(reference.location);
        }

        // If AST search found results, return them
        if !locations.is_empty() {
            return locations;
        }
    }

    // Fall back to text-based search
    find_text_references(document, &word, uri, include_declaration)
}

/// Find references using AST traversal, returning categorized results
pub fn find_ast_references(
    module: &verum_ast::Module,
    symbol: &str,
    uri: &Url,
    text: &str,
) -> List<Reference> {
    let mut refs = List::new();

    for item in module.items.iter() {
        match &item.kind {
            ItemKind::Function(func) => {
                // Check function name (definition)
                if func.name.as_str() == symbol {
                    refs.push(Reference {
                        location: span_to_location(&func.name.span, uri, text),
                        kind: ReferenceKind::Definition,
                    });
                }

                // Check parameters
                for param in func.params.iter() {
                    if let verum_ast::decl::FunctionParamKind::Regular { pattern, .. } = &param.kind
                    {
                        find_pattern_references(
                            pattern,
                            symbol,
                            uri,
                            text,
                            &mut refs,
                            ReferenceKind::Definition,
                        );
                    }
                }

                // Check function body
                if let Some(body) = &func.body
                    && let verum_ast::decl::FunctionBody::Block(block) = body
                {
                    find_block_references(block, symbol, uri, text, &mut refs);
                }
            }
            ItemKind::Type(type_decl) => {
                // Check type name
                if type_decl.name.as_str() == symbol {
                    refs.push(Reference {
                        location: span_to_location(&type_decl.name.span, uri, text),
                        kind: ReferenceKind::Definition,
                    });
                }

                // Check fields and variants
                match &type_decl.body {
                    verum_ast::decl::TypeDeclBody::Record(fields) => {
                        for field in fields {
                            if field.name.as_str() == symbol {
                                refs.push(Reference {
                                    location: span_to_location(&field.span, uri, text),
                                    kind: ReferenceKind::Definition,
                                });
                            }
                            // Check field type for type references
                            find_type_references(&field.ty, symbol, uri, text, &mut refs);
                        }
                    }
                    verum_ast::decl::TypeDeclBody::Variant(variants) => {
                        for variant in variants {
                            if variant.name.as_str() == symbol {
                                refs.push(Reference {
                                    location: span_to_location(&variant.span, uri, text),
                                    kind: ReferenceKind::Definition,
                                });
                            }
                        }
                    }
                    verum_ast::decl::TypeDeclBody::Alias(ty) => {
                        find_type_references(ty, symbol, uri, text, &mut refs);
                    }
                    verum_ast::decl::TypeDeclBody::Newtype(ty) => {
                        find_type_references(ty, symbol, uri, text, &mut refs);
                    }
                    _ => {}
                }
            }
            ItemKind::Protocol(protocol) => {
                if protocol.name.as_str() == symbol {
                    refs.push(Reference {
                        location: span_to_location(&protocol.name.span, uri, text),
                        kind: ReferenceKind::Definition,
                    });
                }
            }
            ItemKind::Const(const_decl) => {
                if const_decl.name.as_str() == symbol {
                    refs.push(Reference {
                        location: span_to_location(&const_decl.span, uri, text),
                        kind: ReferenceKind::Definition,
                    });
                }
                find_expr_references(&const_decl.value, symbol, uri, text, &mut refs);
            }
            _ => {}
        }
    }

    refs
}

/// Find references in a pattern
fn find_pattern_references(
    pattern: &verum_ast::Pattern,
    symbol: &str,
    uri: &Url,
    text: &str,
    refs: &mut List<Reference>,
    kind: ReferenceKind,
) {
    match &pattern.kind {
        PatternKind::Ident { name, .. } if name.as_str() == symbol => {
            refs.push(Reference {
                location: span_to_location(&pattern.span, uri, text),
                kind,
            });
        }
        PatternKind::Tuple(patterns) => {
            for p in patterns.iter() {
                find_pattern_references(p, symbol, uri, text, refs, kind);
            }
        }
        PatternKind::Variant { path, data, .. } => {
            // Check if variant name matches
            if let Some(seg) = path.segments.last()
                && let verum_ast::ty::PathSegment::Name(ident) = seg
                && ident.as_str() == symbol
            {
                refs.push(Reference {
                    location: span_to_location(&pattern.span, uri, text),
                    kind: ReferenceKind::Read,
                });
            }

            if let Some(inner) = data {
                match inner {
                    verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                        for p in patterns.iter() {
                            find_pattern_references(p, symbol, uri, text, refs, kind);
                        }
                    }
                    verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                        for field in fields.iter() {
                            if let Some(p) = &field.pattern {
                                find_pattern_references(p, symbol, uri, text, refs, kind);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Find references in a block
fn find_block_references(
    block: &verum_ast::expr::Block,
    symbol: &str,
    uri: &Url,
    text: &str,
    refs: &mut List<Reference>,
) {
    for stmt in block.stmts.iter() {
        match &stmt.kind {
            StmtKind::Let { pattern, value, ty } => {
                find_pattern_references(
                    pattern,
                    symbol,
                    uri,
                    text,
                    refs,
                    ReferenceKind::Definition,
                );
                if let Some(init) = value {
                    find_expr_references(init, symbol, uri, text, refs);
                }
                if let Some(t) = ty {
                    find_type_references(t, symbol, uri, text, refs);
                }
            }
            StmtKind::Expr { expr, .. } => {
                find_expr_references(expr, symbol, uri, text, refs);
            }
            StmtKind::Item(_item) => {
                // Nested item definitions
            }
            StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => {
                find_expr_references(expr, symbol, uri, text, refs);
            }
            _ => {}
        }
    }

    if let Some(expr) = &block.expr {
        find_expr_references(expr, symbol, uri, text, refs);
    }
}

/// Find references in an expression
fn find_expr_references(
    expr: &verum_ast::Expr,
    symbol: &str,
    uri: &Url,
    text: &str,
    refs: &mut List<Reference>,
) {
    match &expr.kind {
        ExprKind::Path(path) => {
            // Check if path references the symbol
            if let Some(seg) = path.segments.first()
                && let verum_ast::ty::PathSegment::Name(ident) = seg
                && ident.as_str() == symbol
            {
                refs.push(Reference {
                    location: span_to_location(&expr.span, uri, text),
                    kind: ReferenceKind::Read,
                });
            }
        }
        ExprKind::Call { func, args, .. } => {
            // Check if calling the symbol
            if let ExprKind::Path(path) = &func.kind {
                if let Some(seg) = path.segments.first()
                    && let verum_ast::ty::PathSegment::Name(ident) = seg
                    && ident.as_str() == symbol
                {
                    refs.push(Reference {
                        location: span_to_location(&func.span, uri, text),
                        kind: ReferenceKind::Call,
                    });
                }
            } else {
                find_expr_references(func, symbol, uri, text, refs);
            }
            for arg in args {
                find_expr_references(arg, symbol, uri, text, refs);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            find_expr_references(receiver, symbol, uri, text, refs);
            for arg in args {
                find_expr_references(arg, symbol, uri, text, refs);
            }
        }
        ExprKind::Field { expr: inner, .. } => {
            find_expr_references(inner, symbol, uri, text, refs);
        }
        ExprKind::Binary { left, right, .. } => {
            find_expr_references(left, symbol, uri, text, refs);
            find_expr_references(right, symbol, uri, text, refs);
        }
        ExprKind::Unary { expr: inner, .. } => {
            find_expr_references(inner, symbol, uri, text, refs);
        }
        ExprKind::Block(block) => {
            find_block_references(block, symbol, uri, text, refs);
        }
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            // condition is IfCondition, not a simple Expr
            find_block_references(then_branch, symbol, uri, text, refs);
            if let Some(else_expr) = else_branch {
                find_expr_references(else_expr, symbol, uri, text, refs);
            }
        }
        ExprKind::Match {
            expr: scrutinee,
            arms,
        } => {
            find_expr_references(scrutinee, symbol, uri, text, refs);
            for arm in arms {
                find_pattern_references(
                    &arm.pattern,
                    symbol,
                    uri,
                    text,
                    refs,
                    ReferenceKind::Definition,
                );
                if let Some(guard) = &arm.guard {
                    find_expr_references(guard, symbol, uri, text, refs);
                }
                find_expr_references(&arm.body, symbol, uri, text, refs);
            }
        }
        ExprKind::For {
            label: _,
            pattern,
            iter,
            body,
            invariants: _,
            decreases: _,
        } => {
            find_pattern_references(pattern, symbol, uri, text, refs, ReferenceKind::Definition);
            find_expr_references(iter, symbol, uri, text, refs);
            find_block_references(body, symbol, uri, text, refs);
        }
        ExprKind::While {
            label: _,
            condition,
            body,
            invariants: _,
            decreases: _,
        } => {
            find_expr_references(condition, symbol, uri, text, refs);
            find_block_references(body, symbol, uri, text, refs);
        }
        ExprKind::Loop {
            label: _,
            body,
            invariants: _,
        } => {
            find_block_references(body, symbol, uri, text, refs);
        }
        ExprKind::Closure {
            params,
            body,
            return_type,
            ..
        } => {
            for param in params {
                find_pattern_references(
                    &param.pattern,
                    symbol,
                    uri,
                    text,
                    refs,
                    ReferenceKind::Definition,
                );
            }
            find_expr_references(body, symbol, uri, text, refs);
            if let Some(ret_ty) = return_type {
                find_type_references(ret_ty, symbol, uri, text, refs);
            }
        }
        ExprKind::Return(maybe_inner) => {
            if let Some(inner) = maybe_inner {
                find_expr_references(inner, symbol, uri, text, refs);
            }
        }
        ExprKind::Tuple(elements) => {
            for elem in elements {
                find_expr_references(elem, symbol, uri, text, refs);
            }
        }
        ExprKind::Array(array_expr) => match array_expr {
            verum_ast::expr::ArrayExpr::List(elements) => {
                for elem in elements.iter() {
                    find_expr_references(elem, symbol, uri, text, refs);
                }
            }
            verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                find_expr_references(value, symbol, uri, text, refs);
                find_expr_references(count, symbol, uri, text, refs);
            }
        },
        ExprKind::Index { expr: inner, index } => {
            find_expr_references(inner, symbol, uri, text, refs);
            find_expr_references(index, symbol, uri, text, refs);
        }
        ExprKind::Cast { expr: inner, ty } => {
            find_expr_references(inner, symbol, uri, text, refs);
            find_type_references(ty, symbol, uri, text, refs);
        }
        ExprKind::Await(inner) => {
            find_expr_references(inner, symbol, uri, text, refs);
        }
        ExprKind::TryRecover { try_block, recover } => {
            find_expr_references(try_block, symbol, uri, text, refs);
            find_recover_body_references(recover, symbol, uri, text, refs);
        }
        ExprKind::TryRecoverFinally {
            try_block,
            recover,
            finally_block,
        } => {
            find_expr_references(try_block, symbol, uri, text, refs);
            find_recover_body_references(recover, symbol, uri, text, refs);
            find_expr_references(finally_block, symbol, uri, text, refs);
        }
        ExprKind::DestructuringAssign { pattern, value, .. } => {
            // Find references in the pattern (identifiers being assigned to)
            find_pattern_references(pattern, symbol, uri, text, refs, ReferenceKind::Write);
            // Find references in the value expression
            find_expr_references(value, symbol, uri, text, refs);
        }
        _ => {}
    }
}

/// Find references in a recover body (for try-recover expressions)
fn find_recover_body_references(
    recover: &RecoverBody,
    symbol: &str,
    uri: &Url,
    text: &str,
    refs: &mut List<Reference>,
) {
    match recover {
        RecoverBody::MatchArms { arms, .. } => {
            for arm in arms {
                find_pattern_references(
                    &arm.pattern,
                    symbol,
                    uri,
                    text,
                    refs,
                    ReferenceKind::Definition,
                );
                if let Some(guard) = &arm.guard {
                    find_expr_references(guard, symbol, uri, text, refs);
                }
                find_expr_references(&arm.body, symbol, uri, text, refs);
            }
        }
        RecoverBody::Closure { param, body, .. } => {
            find_pattern_references(&param.pattern, symbol, uri, text, refs, ReferenceKind::Definition);
            find_expr_references(body, symbol, uri, text, refs);
        }
    }
}

/// Find references in a type
fn find_type_references(
    ty: &verum_ast::Type,
    symbol: &str,
    uri: &Url,
    text: &str,
    refs: &mut List<Reference>,
) {
    use verum_ast::ty::TypeKind;

    match &ty.kind {
        TypeKind::Path(path) => {
            if let Some(seg) = path.segments.first()
                && let verum_ast::ty::PathSegment::Name(ident) = seg
                && ident.as_str() == symbol
            {
                refs.push(Reference {
                    location: span_to_location(&ty.span, uri, text),
                    kind: ReferenceKind::Read,
                });
            }
        }
        TypeKind::Tuple(types) => {
            for t in types {
                find_type_references(t, symbol, uri, text, refs);
            }
        }
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            for param in params {
                find_type_references(param, symbol, uri, text, refs);
            }
            find_type_references(return_type, symbol, uri, text, refs);
        }
        TypeKind::Reference { inner, .. } => {
            find_type_references(inner, symbol, uri, text, refs);
        }
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
            find_type_references(element, symbol, uri, text, refs);
        }
        _ => {}
    }
}

/// Convert AST span to LSP Location using proper byte-to-line conversion
fn span_to_location(span: &verum_ast::Span, uri: &Url, text: &str) -> Location {
    Location {
        uri: uri.clone(),
        range: ast_span_to_range(span, text),
    }
}

/// Find all categorized references to a named symbol in a document.
///
/// Unlike `find_references` (which takes a cursor position), this searches
/// by symbol name directly. Used by code lens to count references.
pub fn find_references_by_name(
    document: &crate::document::DocumentState,
    symbol: &str,
    uri: &Url,
) -> List<Reference> {
    if let Some(module) = &document.module {
        let refs = find_ast_references(module, symbol, uri, &document.text);
        if !refs.is_empty() {
            return refs;
        }
    }
    List::new()
}

/// Fall back to text-based reference search
fn find_text_references(
    document: &DocumentState,
    word: &str,
    uri: &Url,
    _include_declaration: bool,
) -> List<Location> {
    let mut locations = List::new();
    let lines: Vec<&str> = document.text.lines().collect();

    for (line_num, line) in lines.iter().enumerate() {
        let mut start_pos = 0;
        while let Some(pos) = line[start_pos..].find(word) {
            let actual_pos = start_pos + pos;

            // Check if it's a whole word match
            let is_whole_word = {
                let before_ok = actual_pos == 0
                    || !line
                        .chars()
                        .nth(actual_pos - 1)
                        .map(is_identifier_char)
                        .unwrap_or(false);
                let after_ok = actual_pos + word.len() >= line.len()
                    || !line
                        .chars()
                        .nth(actual_pos + word.len())
                        .map(is_identifier_char)
                        .unwrap_or(false);
                before_ok && after_ok
            };

            if is_whole_word {
                locations.push(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: Position {
                            line: line_num as u32,
                            character: actual_pos as u32,
                        },
                        end: Position {
                            line: line_num as u32,
                            character: (actual_pos + word.len()) as u32,
                        },
                    },
                });
            }

            start_pos = actual_pos + 1;
        }
    }

    locations
}
