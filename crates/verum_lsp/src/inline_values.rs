//! Inline values support
//!
//! Shows inline values for let bindings with literal initializers
//! or type annotations during debugging.

use crate::document::DocumentState;
use crate::position_utils::ast_span_to_range;
use tower_lsp::lsp_types::*;
use verum_ast::{ExprKind, ItemKind, StmtKind, PatternKind};

/// Inline value information
#[derive(Debug, Clone)]
pub struct InlineValueInfo {
    pub range: Range,
    pub text: String,
}

/// Compute inline values for a range in a document
pub fn compute_inline_values(
    document: &DocumentState,
    visible_range: Range,
) -> Vec<InlineValue> {
    let module = match &document.module {
        Some(m) => m,
        None => return Vec::new(),
    };

    let mut values = Vec::new();

    for item in module.items.iter() {
        if let ItemKind::Function(func) = &item.kind {
            if let Some(body) = &func.body {
                if let verum_ast::decl::FunctionBody::Block(block) = body {
                    collect_inline_values_from_block(block, &document.text, visible_range, &mut values);
                }
            }
        }
    }

    values
}

fn collect_inline_values_from_block(
    block: &verum_ast::expr::Block,
    text: &str,
    visible_range: Range,
    values: &mut Vec<InlineValue>,
) {
    for stmt in block.stmts.iter() {
        let stmt_range = ast_span_to_range(&stmt.span, text);

        // Skip statements outside visible range
        if stmt_range.end.line < visible_range.start.line
            || stmt_range.start.line > visible_range.end.line
        {
            continue;
        }

        if let StmtKind::Let { pattern, value, ty } = &stmt.kind {
            // Extract variable name from pattern
            let var_name = match &pattern.kind {
                PatternKind::Ident { name, .. } => name.as_str().to_string(),
                _ => continue,
            };

            let var_range = ast_span_to_range(&pattern.span, text);

            // If there's a type annotation, show it as inline value
            if let Some(ty) = ty {
                let _type_range = ast_span_to_range(&ty.span, text);
                let type_text = extract_text_from_span(text, ty.span);
                values.push(InlineValue::Text(InlineValueText {
                    range: var_range,
                    text: format!(": {}", type_text),
                }));
            }

            // If there's a literal initializer, show the value
            if let Some(init) = value {
                if let Some(_literal_text) = extract_literal_value(init) {
                    values.push(InlineValue::EvaluatableExpression(
                        InlineValueEvaluatableExpression {
                            range: ast_span_to_range(&init.span, text),
                            expression: Some(var_name),
                        },
                    ));
                }
            }
        }
    }
}

/// Extract literal value text from an expression
fn extract_literal_value(expr: &verum_ast::Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Literal(lit) => {
            // Use the span text as a simple representation
            Some(format!("{:?}", lit.kind))
        }
        _ => None,
    }
}

/// Extract text from a span
fn extract_text_from_span(text: &str, span: verum_ast::Span) -> String {
    let start = span.start as usize;
    let end = (span.end as usize).min(text.len());
    if start < end && start < text.len() {
        text[start..end].to_string()
    } else {
        String::new()
    }
}
