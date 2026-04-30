//! Compiler Diagnostics (Tier 1 - Requires CompileDiag)
//!
//! Functions for emitting compile-time errors and warnings.
//!
//! ## Functions
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `compile_error(msg)` | `(Text) -> !` | Emit compile-time error |
//! | `compile_warning(msg)` | `(Text) -> ()` | Emit compile-time warning |
//!
//! ## Context Requirements
//!
//! **Tier 1**: Requires `using [CompileDiag]` context.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_common::{List, Text};

use crate::meta::builtins::context_requirements::{BuiltinInfo, BuiltinRegistry};
use crate::meta::context::{ConstValue, MetaContext};
use crate::meta::error::MetaError;

/// Register diagnostics builtins
pub fn register_builtins(map: &mut BuiltinRegistry) {
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

    // Resolve `ctx.call_site_span` (AST Span = byte offsets) into a
    // proper LineColSpan via the global source-file registry so the
    // emitted diagnostic anchors at the user's invocation site (file
    // path + line:column) rather than the file="<meta>", line=byte
    // offset, column=1 garbage the previous construction produced.
    // Closes #239 (compile-error span fidelity) for the
    // `meta_compile_error` builtin: pre-fix the LineColSpan put
    // start byte-offset where line was expected, hardcoded column=1,
    // and put end byte-offset where end_column was expected — so a
    // `compile_error("…")` from a meta fn surfaced at "<meta>:0:1"
    // regardless of where the user invoked the macro.
    let span = verum_common::global_span_to_line_col(ctx.call_site_span);
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

    // Resolve `ctx.call_site_span` via the global source-file
    // registry (sibling fix to `meta_compile_error`).  See its
    // comment for the rationale; closes #239 for the compile_warning
    // builtin.
    let span = verum_common::global_span_to_line_col(ctx.call_site_span);
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
    use verum_ast::Span;
    use verum_common::span::FileId;

    #[test]
    fn test_compile_warning() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("test warning"))]);
        let result = meta_compile_warning(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Unit);
        assert_eq!(ctx.warning_count, 1);
    }

    /// Pin: when `MetaContext.call_site_span` is set to a real
    /// (registered) AST span, `meta_compile_error`'s emitted
    /// diagnostic carries a LineColSpan resolved from the global
    /// source-file registry — file path + line:column — instead of
    /// the file="<meta>", line=byte-offset, column=1 garbage the
    /// previous construction produced.  Closes #239 for
    /// `meta_compile_error`.
    #[test]
    fn compile_error_anchors_at_call_site_when_source_registered() {
        // Register a synthetic source file with the global registry
        // so the byte-offset → line:col conversion has content to
        // index into.  The text "abcde\nfghij" gives line 1 (cols
        // 1–6) and line 2 (cols 1–6).
        let file_id = FileId::new(0xCAFE_BABE);
        verum_common::register_source_file(file_id, "test.vr", "abcde\nfghij");

        let mut ctx = MetaContext::new();
        // call_site_span at byte offsets 6..=10 — that's line 2
        // (after the newline at offset 5), columns 1..=5 of "fghij".
        ctx.call_site_span = Span::new(6, 10, file_id);

        let args = List::from(vec![ConstValue::Text(Text::from("oops"))]);
        let result = meta_compile_error(&mut ctx, args);
        assert!(result.is_err(),
            "meta_compile_error must surface MetaError::CompileError");
        assert_eq!(ctx.error_count, 1);
        let diag = ctx.diagnostics.iter().last()
            .expect("diagnostic must be pushed");
        let span = diag.primary_span().expect("diagnostic must carry a primary span");
        assert_eq!(
            span.file.as_str(),
            "test.vr",
            "diagnostic file must be the registered name, not <meta>"
        );
        assert_eq!(
            span.line, 2,
            "diagnostic line must be the resolved line (2), not the start byte offset (6)"
        );
    }

    /// Pin: when `call_site_span` is left at the default Span::dummy(),
    /// the global resolver returns "<generated>" — proves the fix
    /// gracefully handles unregistered/dummy call sites instead of
    /// panicking or constructing a malformed LineColSpan.
    #[test]
    fn compile_error_handles_dummy_call_site() {
        let mut ctx = MetaContext::new();
        // ctx.call_site_span defaults to Span::dummy() — no setup.
        let args = List::from(vec![ConstValue::Text(Text::from("oops"))]);
        let _ = meta_compile_error(&mut ctx, args);
        let diag = ctx.diagnostics.iter().last().unwrap();
        assert_eq!(
            diag.primary_span().expect("primary span").file.as_str(),
            "<generated>",
            "dummy call_site_span must produce <generated> file (the documented fallback)"
        );
    }
}
