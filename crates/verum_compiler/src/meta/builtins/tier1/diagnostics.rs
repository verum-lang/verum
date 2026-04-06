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
    fn test_compile_warning() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("test warning"))]);
        let result = meta_compile_warning(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Unit);
        assert_eq!(ctx.warning_count, 1);
    }
}
