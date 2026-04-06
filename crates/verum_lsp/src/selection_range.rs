//! Deep selection range support
//!
//! Provides nested selection ranges by walking the AST to find
//! the innermost expression containing the cursor and building
//! a chain from innermost to outermost scope.

use crate::document::DocumentState;
use crate::position_utils::ast_span_to_range;
use tower_lsp::lsp_types::*;
use verum_ast::{ExprKind, ItemKind, StmtKind};

/// Compute selection ranges for given positions in a document
pub fn compute_selection_ranges(
    document: &DocumentState,
    positions: &[Position],
) -> Vec<SelectionRange> {
    let module = match &document.module {
        Some(m) => m,
        None => return positions.iter().map(|_| file_range(document)).collect(),
    };

    positions
        .iter()
        .map(|pos| {
            let offset = position_to_offset(*pos, &document.text);
            let mut ranges = Vec::new();

            // Find word range at cursor
            if let Some(word_range) = word_range_at(pos, &document.text) {
                ranges.push(word_range);
            }

            // Walk AST to find enclosing scopes
            for item in module.items.iter() {
                if item.span.start <= offset && offset <= item.span.end {
                    collect_ranges_in_item(&item.kind, offset, &document.text, &mut ranges);
                    ranges.push(ast_span_to_range(&item.span, &document.text));
                }
            }

            // Add file-level range
            let file_range = full_file_range(&document.text);
            ranges.push(file_range);

            // Deduplicate and sort from innermost to outermost
            ranges.dedup_by(|a, b| a == b);

            // Build nested SelectionRange chain (innermost first)
            let mut selection: Option<SelectionRange> = None;
            for range in ranges.into_iter().rev() {
                selection = Some(SelectionRange {
                    range,
                    parent: selection.map(Box::new),
                });
            }

            selection.unwrap_or_else(|| file_range_selection(document))
        })
        .collect()
}

/// Collect ranges within an item (function, type, etc.)
fn collect_ranges_in_item(
    kind: &ItemKind,
    offset: u32,
    text: &str,
    ranges: &mut Vec<Range>,
) {
    match kind {
        ItemKind::Function(func) => {
            // Function name
            if func.name.span.start <= offset && offset <= func.name.span.end {
                ranges.push(ast_span_to_range(&func.name.span, text));
            }

            // Parameters
            for param in func.params.iter() {
                if param.span.start <= offset && offset <= param.span.end {
                    ranges.push(ast_span_to_range(&param.span, text));
                }
            }

            // Function body
            if let Some(body) = &func.body {
                if let verum_ast::decl::FunctionBody::Block(block) = body {
                    if block.span.start <= offset && offset <= block.span.end {
                        collect_ranges_in_block(block, offset, text, ranges);
                        ranges.push(ast_span_to_range(&block.span, text));
                    }
                }
            }
        }
        ItemKind::Type(type_decl) => {
            if type_decl.name.span.start <= offset && offset <= type_decl.name.span.end {
                ranges.push(ast_span_to_range(&type_decl.name.span, text));
            }
        }
        ItemKind::Protocol(protocol) => {
            if protocol.name.span.start <= offset && offset <= protocol.name.span.end {
                ranges.push(ast_span_to_range(&protocol.name.span, text));
            }
        }
        ItemKind::Impl(impl_block) => {
            if impl_block.span.start <= offset && offset <= impl_block.span.end {
                for impl_item in impl_block.items.iter() {
                    if impl_item.span.start <= offset && offset <= impl_item.span.end {
                        if let verum_ast::decl::ImplItemKind::Function(func) = &impl_item.kind {
                            collect_ranges_in_item(
                                &ItemKind::Function(func.clone()),
                                offset,
                                text,
                                ranges,
                            );
                        }
                        ranges.push(ast_span_to_range(&impl_item.span, text));
                    }
                }
            }
        }
        _ => {}
    }
}

/// Collect ranges in a block
fn collect_ranges_in_block(
    block: &verum_ast::expr::Block,
    offset: u32,
    text: &str,
    ranges: &mut Vec<Range>,
) {
    for stmt in block.stmts.iter() {
        if stmt.span.start <= offset && offset <= stmt.span.end {
            ranges.push(ast_span_to_range(&stmt.span, text));

            match &stmt.kind {
                StmtKind::Let { value, .. } => {
                    if let Some(init) = value {
                        if init.span.start <= offset && offset <= init.span.end {
                            collect_ranges_in_expr(init, offset, text, ranges);
                        }
                    }
                }
                StmtKind::Expr { expr, .. } => {
                    collect_ranges_in_expr(expr, offset, text, ranges);
                }
                _ => {}
            }
        }
    }

    if let Some(expr) = &block.expr {
        if expr.span.start <= offset && offset <= expr.span.end {
            collect_ranges_in_expr(expr, offset, text, ranges);
        }
    }
}

/// Collect ranges in an expression
fn collect_ranges_in_expr(
    expr: &verum_ast::Expr,
    offset: u32,
    text: &str,
    ranges: &mut Vec<Range>,
) {
    if expr.span.start > offset || offset > expr.span.end {
        return;
    }

    ranges.push(ast_span_to_range(&expr.span, text));

    match &expr.kind {
        ExprKind::Block(block) => {
            collect_ranges_in_block(block, offset, text, ranges);
        }
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            if then_branch.span.start <= offset && offset <= then_branch.span.end {
                collect_ranges_in_block(then_branch, offset, text, ranges);
                ranges.push(ast_span_to_range(&then_branch.span, text));
            }
            if let Some(else_expr) = else_branch {
                collect_ranges_in_expr(else_expr, offset, text, ranges);
            }
        }
        ExprKind::Match { expr: scrutinee, arms } => {
            collect_ranges_in_expr(scrutinee, offset, text, ranges);
            for arm in arms {
                if arm.body.span.start <= offset && offset <= arm.body.span.end {
                    collect_ranges_in_expr(&arm.body, offset, text, ranges);
                }
            }
        }
        ExprKind::For { iter, body, .. } => {
            collect_ranges_in_expr(iter, offset, text, ranges);
            if body.span.start <= offset && offset <= body.span.end {
                collect_ranges_in_block(body, offset, text, ranges);
                ranges.push(ast_span_to_range(&body.span, text));
            }
        }
        ExprKind::While { condition, body, .. } => {
            collect_ranges_in_expr(condition, offset, text, ranges);
            if body.span.start <= offset && offset <= body.span.end {
                collect_ranges_in_block(body, offset, text, ranges);
                ranges.push(ast_span_to_range(&body.span, text));
            }
        }
        ExprKind::Loop { body, .. } => {
            if body.span.start <= offset && offset <= body.span.end {
                collect_ranges_in_block(body, offset, text, ranges);
                ranges.push(ast_span_to_range(&body.span, text));
            }
        }
        ExprKind::Closure { body, .. } => {
            collect_ranges_in_expr(body, offset, text, ranges);
        }
        ExprKind::Call { func, args, .. } => {
            collect_ranges_in_expr(func, offset, text, ranges);
            for arg in args {
                collect_ranges_in_expr(arg, offset, text, ranges);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_ranges_in_expr(receiver, offset, text, ranges);
            for arg in args {
                collect_ranges_in_expr(arg, offset, text, ranges);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            collect_ranges_in_expr(left, offset, text, ranges);
            collect_ranges_in_expr(right, offset, text, ranges);
        }
        ExprKind::Unary { expr: inner, .. } => {
            collect_ranges_in_expr(inner, offset, text, ranges);
        }
        ExprKind::Field { expr: inner, .. } => {
            collect_ranges_in_expr(inner, offset, text, ranges);
        }
        ExprKind::Index { expr: inner, index } => {
            collect_ranges_in_expr(inner, offset, text, ranges);
            collect_ranges_in_expr(index, offset, text, ranges);
        }
        ExprKind::Return(Some(inner)) | ExprKind::Await(inner) => {
            collect_ranges_in_expr(inner, offset, text, ranges);
        }
        ExprKind::Tuple(elements) => {
            for elem in elements {
                collect_ranges_in_expr(elem, offset, text, ranges);
            }
        }
        ExprKind::TryRecover { try_block, .. } => {
            collect_ranges_in_expr(try_block, offset, text, ranges);
        }
        _ => {}
    }
}

/// Create a file-level selection range
fn file_range(document: &DocumentState) -> SelectionRange {
    SelectionRange {
        range: full_file_range(&document.text),
        parent: None,
    }
}

fn file_range_selection(document: &DocumentState) -> SelectionRange {
    file_range(document)
}

fn full_file_range(text: &str) -> Range {
    let line_count = text.lines().count();
    let last_line_len = text.lines().last().map_or(0, |l| l.len());
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: line_count.saturating_sub(1) as u32,
            character: last_line_len as u32,
        },
    }
}

/// Find the word range at a position
fn word_range_at(position: &Position, text: &str) -> Option<Range> {
    let line = text.lines().nth(position.line as usize)?;
    let col = position.character as usize;

    if col >= line.len() {
        return None;
    }

    let bytes = line.as_bytes();
    if !bytes[col].is_ascii_alphanumeric() && bytes[col] != b'_' {
        return None;
    }

    let mut start = col;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    let mut end = col;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }

    Some(Range {
        start: Position {
            line: position.line,
            character: start as u32,
        },
        end: Position {
            line: position.line,
            character: end as u32,
        },
    })
}

/// Convert LSP Position to byte offset
fn position_to_offset(position: Position, text: &str) -> u32 {
    let mut offset: u32 = 0;
    let mut current_line: u32 = 0;
    let mut current_char: u32 = 0;

    for ch in text.chars() {
        if current_line == position.line && current_char == position.character {
            return offset;
        }
        if ch == '\n' {
            if current_line == position.line {
                return offset;
            }
            current_line += 1;
            current_char = 0;
        } else {
            current_char += 1;
        }
        offset += ch.len_utf8() as u32;
    }
    offset
}
