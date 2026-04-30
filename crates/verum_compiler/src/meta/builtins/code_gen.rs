//! Code Generation Intrinsics
//!
//! This module provides compile-time code generation functions organized by tier.
//!
//! ## Tier 0: Pure AST/Text Manipulation (Always Available)
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `quote(expr)` | `(Expr) -> Ast` | Quote an expression as AST |
//! | `unquote(ast)` | `(Ast) -> Expr` | Unquote AST back to value |
//! | `stringify(value)` | `(any) -> Text` | Convert value to text representation |
//! | `concat_idents(parts...)` | `(...Text) -> Text` | Concatenate identifiers |
//! | `format_ident(fmt, args...)` | `(Text, ...) -> Text` | Format identifier name |
//!
//! ## Tier 1: Compiler Diagnostics (Requires CompileDiag)
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `compile_error(msg)` | `(Text) -> !` | Emit compile-time error |
//! | `compile_warning(msg)` | `(Text) -> ()` | Emit compile-time warning |
//!
//! ## Context Requirements
//!
//! - **Tier 0**: `quote`, `unquote`, `stringify`, `concat_idents`, `format_ident` - No context required
//! - **Tier 1**: `compile_error`, `compile_warning` - Requires `using [CompileDiag]`
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use std::sync::atomic::{AtomicU64, Ordering};

use verum_ast::{Expr, ExprKind, span::Span};
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Global counter for gensym to ensure unique identifiers
static GENSYM_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Register all code generation builtins (Tier 0 + Tier 1)
///
/// This registers both pure AST manipulation functions (Tier 0)
/// and compiler diagnostics (Tier 1).
pub fn register_builtins(map: &mut BuiltinRegistry) {
    register_tier0_builtins(map);
    register_tier1_builtins(map);
}

/// Register only Tier 0 code generation builtins (pure AST manipulation)
///
/// These functions are always available without requiring any context.
pub fn register_tier0_builtins(map: &mut BuiltinRegistry) {
    map.insert(
        Text::from("quote"),
        BuiltinInfo::tier0(
            meta_quote,
            "Quote an expression as AST node",
            "(Expr) -> Ast",
        ),
    );
    map.insert(
        Text::from("unquote"),
        BuiltinInfo::tier0(
            meta_unquote,
            "Unquote AST node back to expression",
            "(Ast) -> Expr",
        ),
    );
    map.insert(
        Text::from("stringify"),
        BuiltinInfo::tier0(
            meta_stringify,
            "Convert value to text representation",
            "(any) -> Text",
        ),
    );
    map.insert(
        Text::from("concat_idents"),
        BuiltinInfo::tier0(
            meta_concat_idents,
            "Concatenate identifier parts",
            "(...Text) -> Text",
        ),
    );
    map.insert(
        Text::from("format_ident"),
        BuiltinInfo::tier0(
            meta_format_ident,
            "Format identifier with substitutions",
            "(Text, ...) -> Text",
        ),
    );

    // Hygiene builtins
    map.insert(
        Text::from("gensym"),
        BuiltinInfo::tier0(
            meta_gensym,
            "Generate unique identifier with given prefix",
            "(Text) -> Text",
        ),
    );

    // Identifier creation
    map.insert(
        Text::from("ident"),
        BuiltinInfo::tier0(
            meta_ident,
            "Create an identifier from text",
            "(Text) -> Ident",
        ),
    );
}

/// Register only Tier 1 code generation builtins (compiler diagnostics)
///
/// These functions require the CompileDiag context.
pub fn register_tier1_builtins(map: &mut BuiltinRegistry) {
    map.insert(
        Text::from("compile_error"),
        BuiltinInfo::compile_diag(
            meta_compile_error,
            "Emit a compile-time error",
            "(Text) -> !",
        ),
    );
    map.insert(
        Text::from("compile_warning"),
        BuiltinInfo::compile_diag(
            meta_compile_warning,
            "Emit a compile-time warning",
            "(Text) -> ()",
        ),
    );
}

// ============================================================================
// Gensym (Hygienic Symbol Generation)
// ============================================================================

/// Generate a unique identifier with given prefix
///
/// This is essential for hygienic macro expansion to avoid name collisions.
/// Each call returns a unique identifier with the format `__verum_gensym_<prefix>_<counter>`.
/// This format is recognized by the hygiene checker and guaranteed never to collide
/// with user-written code.
///
/// Example: `gensym("tmp")` might return `__verum_gensym_tmp_42`
fn meta_gensym(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    let prefix = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            });
        }
    };

    // Generate unique ID using atomic counter with hygiene prefix
    // Format: __verum_gensym_<prefix>_<counter>
    // This format is recognized by hygiene checker (M403 check)
    let counter = GENSYM_COUNTER.fetch_add(1, Ordering::SeqCst);
    let unique_name = format!("__verum_gensym_{}_{}", prefix.as_str(), counter);

    Ok(ConstValue::Text(Text::from(unique_name)))
}

// ============================================================================
// Gensym Utilities
// ============================================================================

/// The prefix used for gensym-generated identifiers
pub const GENSYM_PREFIX: &str = "__verum_gensym_";

/// Check if an identifier was generated by gensym
///
/// Returns true if the identifier starts with the gensym prefix.
pub fn is_gensym(name: &str) -> bool {
    name.starts_with(GENSYM_PREFIX)
}

/// Extract the base name and counter from a gensym identifier
///
/// For example, `__verum_gensym_tmp_42` returns `Some(("tmp", 42))`
pub fn parse_gensym(name: &str) -> Option<(&str, u64)> {
    if !name.starts_with(GENSYM_PREFIX) {
        return None;
    }

    let rest = &name[GENSYM_PREFIX.len()..];
    // Find the last underscore which separates base from counter
    if let Some(last_underscore) = rest.rfind('_') {
        let base = &rest[..last_underscore];
        let counter_str = &rest[last_underscore + 1..];
        if let Ok(counter) = counter_str.parse::<u64>() {
            return Some((base, counter));
        }
    }
    None
}

/// Get the current gensym counter value (for debugging/testing)
pub fn gensym_counter() -> u64 {
    GENSYM_COUNTER.load(Ordering::Relaxed)
}

// ============================================================================
// Identifier Creation
// ============================================================================

/// Create an identifier from text
///
/// Lifts a `Text` into an AST `Expr` representing the identifier as a
/// single-segment `Path`, ready for use in `quote { ... }` splices.
/// Example: `let name = ident("foo"); quote { let $name = 42; }`
///
/// Pre-fix `meta_ident("foo")` returned `ConstValue::Text("foo")` —
/// quote splice consumers expect `ConstValue::Expr`, so the splice
/// dropped the value rather than emitting an identifier reference.
/// The pre-fix comment acknowledged it: `For now, an identifier is
/// just a Text value / In a full implementation, this would create
/// an Ident AST node`.
///
/// The text is validated as a syntactically-valid identifier before
/// the lift — empty strings, leading digits, and non-ident characters
/// produce `MetaError::Other` so a typo at compile time surfaces here
/// instead of as a parse error later.
fn meta_ident(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(t) => {
            let s = t.as_str();
            if !is_valid_ident(s) {
                return Err(MetaError::Other(Text::from(format!(
                    "ident({:?}) — argument is not a valid identifier (must start with \
                     letter or `_`, then letters / digits / `_`)",
                    s
                ))));
            }
            // Span::dummy is the right span here — the identifier is
            // synthesized at meta-eval time and has no source location.
            // The downstream quote splice replaces it with the call
            // site's span when it's spliced in.
            let span = Span::dummy();
            let path = Path::single(Ident::new(s, span));
            Ok(ConstValue::Expr(Expr::new(ExprKind::Path(path), span)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Verify that `s` is a syntactically-valid Verum identifier:
/// non-empty, starts with letter or `_`, then alphanumerics / `_`.
fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ============================================================================
// Quote/Unquote
// ============================================================================

/// Quote an expression as AST
fn meta_quote(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Expr(expr) => Ok(ConstValue::Expr(expr.clone())),
        _ => {
            // For non-expressions, create a literal expression
            // This is a simplified implementation
            Ok(args[0].clone())
        }
    }
}

/// Unquote AST back to value
fn meta_unquote(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Expr(expr) => Ok(ConstValue::Expr(expr.clone())),
        _ => Ok(args[0].clone()),
    }
}

// ============================================================================
// Stringify and Identifier Manipulation
// ============================================================================

/// Convert expression to string representation
fn meta_stringify(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    let text = match &args[0] {
        ConstValue::Expr(expr) => {
            // Pretty-print the expression
            verum_ast::pretty::format_expr(expr).to_string()
        }
        ConstValue::Type(ty) => {
            verum_ast::pretty::format_type(ty).to_string()
        }
        ConstValue::Pattern(pat) => {
            verum_ast::pretty::format_pattern(pat).to_string()
        }
        ConstValue::Item(item) => {
            verum_ast::pretty::format_item(item).to_string()
        }
        ConstValue::Text(t) => t.to_string(),
        ConstValue::Int(i) => i.to_string(),
        ConstValue::UInt(u) => u.to_string(),
        ConstValue::Float(f) => f.to_string(),
        ConstValue::Bool(b) => b.to_string(),
        ConstValue::Char(c) => c.to_string(),
        ConstValue::Unit => "()".to_string(),
        ConstValue::Array(arr) => {
            let elements: Vec<String> = arr.iter().map(|v| format!("{:?}", v)).collect();
            format!("[{}]", elements.join(", "))
        }
        ConstValue::Tuple(tup) => {
            let elements: Vec<String> = tup.iter().map(|v| format!("{:?}", v)).collect();
            format!("({})", elements.join(", "))
        }
        _ => format!("{:?}", args[0]),
    };

    Ok(ConstValue::Text(Text::from(text)))
}

/// Concatenate identifiers
fn meta_concat_idents(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 1, got: 0 });
    }

    let mut result = String::new();
    for arg in args {
        match arg {
            ConstValue::Text(t) => result.push_str(t.as_str()),
            ConstValue::Int(i) => result.push_str(&i.to_string()),
            ConstValue::UInt(u) => result.push_str(&u.to_string()),
            _ => {
                return Err(MetaError::TypeMismatch {
                    expected: Text::from("Text or Int"),
                    found: arg.type_name(),
                });
            }
        }
    }

    Ok(ConstValue::Text(Text::from(result)))
}

/// Format identifier name using simple substitution
fn meta_format_ident(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 1, got: 0 });
    }

    let format_str = match &args[0] {
        ConstValue::Text(t) => t.to_string(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            });
        }
    };

    // Simple {} substitution
    let mut result = format_str;
    for (i, arg) in args.iter().skip(1).enumerate() {
        let replacement = match arg {
            ConstValue::Text(t) => t.to_string(),
            ConstValue::Int(i) => i.to_string(),
            ConstValue::UInt(u) => u.to_string(),
            _ => format!("{:?}", arg),
        };

        // Replace positional {} or {n}
        result = result.replacen("{}", &replacement, 1);
        result = result.replace(&format!("{{{}}}", i), &replacement);
    }

    Ok(ConstValue::Text(Text::from(result)))
}

// ============================================================================
// Compile-Time Error/Warning
// ============================================================================

/// Emit a compile error
fn meta_compile_error(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 1, got: 0 });
    }

    let message = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => Text::from(format!("{:?}", args[0])),
    };

    // Record the error in the context
    ctx.error_count += 1;

    // Also emit a diagnostic if we have the diagnostics infrastructure
    let span = verum_common::LineColSpan::new(
        "<meta>",
        ctx.call_site_span.start as usize,
        1,
        ctx.call_site_span.end as usize,
    );
    let diagnostic = verum_diagnostics::Diagnostic::new_error(
        message.to_string(),
        span,
        "E0000",
    );
    ctx.diagnostics.push(diagnostic);

    Err(MetaError::CompileError(message))
}

/// Emit a compile warning
fn meta_compile_warning(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 1, got: 0 });
    }

    let message = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => Text::from(format!("{:?}", args[0])),
    };

    // Record the warning in the context
    ctx.warning_count += 1;

    // Emit diagnostic
    let span = verum_common::LineColSpan::new(
        "<meta>",
        ctx.call_site_span.start as usize,
        1,
        ctx.call_site_span.end as usize,
    );
    let diagnostic = verum_diagnostics::Diagnostic::new_warning(
        message.to_string(),
        span,
        "W0000",
    );
    ctx.diagnostics.push(diagnostic);

    Ok(ConstValue::Unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stringify_int() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(42)]);
        let result = meta_stringify(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("42")));
    }

    #[test]
    fn test_stringify_text() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("hello"))]);
        let result = meta_stringify(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("hello")));
    }

    #[test]
    fn test_concat_idents() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("foo")),
            ConstValue::Text(Text::from("_")),
            ConstValue::Int(42),
        ]);
        let result = meta_concat_idents(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("foo_42")));
    }

    #[test]
    fn test_format_ident() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("get_{}_by_{}")),
            ConstValue::Text(Text::from("user")),
            ConstValue::Text(Text::from("id")),
        ]);
        let result = meta_format_ident(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("get_user_by_id")));
    }

    #[test]
    fn test_compile_warning() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("test warning"))]);
        let result = meta_compile_warning(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Unit);
        assert_eq!(ctx.warning_count, 1);
    }

    #[test]
    fn ident_lifts_text_to_path_expr() {
        // The headline regression: pre-fix `ident("foo")` returned
        // `ConstValue::Text("foo")` and quote-splice consumers
        // dropped the value because they expect `ConstValue::Expr`.
        // Post-fix it returns an Expr wrapping a single-segment
        // `Path(foo)`.
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("foo"))]);
        let result = meta_ident(&mut ctx, args).unwrap();
        match result {
            ConstValue::Expr(expr) => match &expr.kind {
                ExprKind::Path(path) => match path.as_ident() {
                    Some(id) => assert_eq!(id.as_str(), "foo"),
                    None => panic!("expected single-segment Path"),
                },
                other => panic!("expected ExprKind::Path, got {:?}", other),
            },
            other => panic!("expected ConstValue::Expr, got {:?}", other),
        }
    }

    #[test]
    fn ident_accepts_underscore_and_alnum_names() {
        let mut ctx = MetaContext::new();
        for name in ["_priv", "abc123", "_x", "counter_42"] {
            let args = List::from(vec![ConstValue::Text(Text::from(name))]);
            let result = meta_ident(&mut ctx, args)
                .unwrap_or_else(|e| panic!("`{}` should be valid: {:?}", name, e));
            match result {
                ConstValue::Expr(_) => {}
                other => panic!("`{}` produced {:?}", name, other),
            }
        }
    }

    #[test]
    fn ident_rejects_empty_string() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::new())]);
        let result = meta_ident(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::Other(_))));
    }

    #[test]
    fn ident_rejects_leading_digit() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("42foo"))]);
        let result = meta_ident(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::Other(_))));
    }

    #[test]
    fn ident_rejects_punctuation_in_name() {
        let mut ctx = MetaContext::new();
        for bad in ["foo bar", "foo-bar", "foo.bar", "foo!", "f@oo"] {
            let args = List::from(vec![ConstValue::Text(Text::from(bad))]);
            let result = meta_ident(&mut ctx, args);
            assert!(
                matches!(result, Err(MetaError::Other(_))),
                "{:?} should be rejected, got {:?}",
                bad,
                result
            );
        }
    }

    #[test]
    fn ident_rejects_non_text_argument() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(42)]);
        let result = meta_ident(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::TypeMismatch { .. })));
    }

    #[test]
    fn ident_rejects_wrong_arity() {
        let mut ctx = MetaContext::new();
        let result = meta_ident(&mut ctx, List::new());
        assert!(matches!(result, Err(MetaError::ArityMismatch { .. })));
    }
}
