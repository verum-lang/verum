//! Source Map Intrinsics (Tier 1 - Requires SourceMap)
//!
//! Provides compile-time source map management for tracking generated code
//! provenance. All functions require the `SourceMap` context.
//!
//! ## Scope Management
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `source_map_enter_generated(name)` | `(Text) -> ()` | Push name onto scope stack |
//! | `source_map_exit_generated()` | `() -> ()` | Pop scope stack |
//! | `source_map_current_scope()` | `() -> Text` | Current scope name |
//! | `source_map_scope_path()` | `() -> Text` | Full dot-separated path |
//!
//! ## Span Mapping
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `source_map_map_span(gen, src)` | `(Span, Span) -> ()` | Record span mapping |
//! | `source_map_get_source_span(gen)` | `(Span) -> Maybe<Span>` | Lookup source span |
//! | `source_map_synthetic_span(msg)` | `(Text) -> Span` | Create synthetic span |
//! | `source_map_get_mappings()` | `() -> List<SpanMapping>` | All mappings |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [SourceMap]` context.

use verum_common::{Heap, List, Maybe, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register source map builtins with context requirements
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Scope Management (Tier 1 - SourceMap)
    // ========================================================================

    map.insert(
        Text::from("source_map_enter_generated"),
        BuiltinInfo::source_map(
            meta_source_map_enter_generated,
            "Push a generated code scope name onto the stack",
            "(Text) -> ()",
        ),
    );
    map.insert(
        Text::from("source_map_exit_generated"),
        BuiltinInfo::source_map(
            meta_source_map_exit_generated,
            "Pop the current generated code scope from the stack",
            "() -> ()",
        ),
    );
    map.insert(
        Text::from("source_map_current_scope"),
        BuiltinInfo::source_map(
            meta_source_map_current_scope,
            "Get the name of the current generated code scope",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("source_map_scope_path"),
        BuiltinInfo::source_map(
            meta_source_map_scope_path,
            "Get the full dot-separated scope path",
            "() -> Text",
        ),
    );

    // ========================================================================
    // Span Mapping (Tier 1 - SourceMap)
    // ========================================================================

    map.insert(
        Text::from("source_map_map_span"),
        BuiltinInfo::source_map(
            meta_source_map_map_span,
            "Record a mapping from generated span to source span",
            "(Span, Span) -> ()",
        ),
    );
    map.insert(
        Text::from("source_map_get_source_span"),
        BuiltinInfo::source_map(
            meta_source_map_get_source_span,
            "Look up the source span for a generated span",
            "(Span) -> Maybe<Span>",
        ),
    );
    map.insert(
        Text::from("source_map_synthetic_span"),
        BuiltinInfo::source_map(
            meta_source_map_synthetic_span,
            "Create a synthetic span with a descriptive message",
            "(Text) -> Span",
        ),
    );
    map.insert(
        Text::from("source_map_get_mappings"),
        BuiltinInfo::source_map(
            meta_source_map_get_mappings,
            "Get all recorded span mappings",
            "() -> List<(Span, Text)>",
        ),
    );
}

// ============================================================================
// Scope Management
// ============================================================================

/// Push a generated code scope name onto the stack
fn meta_source_map_enter_generated(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(name) => {
            ctx.source_map_scope_stack.push(name.clone());
            Ok(ConstValue::Unit)
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Pop the current generated code scope from the stack
fn meta_source_map_exit_generated(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }

    if ctx.source_map_scope_stack.is_empty() {
        return Err(MetaError::Other(Text::from(
            "source_map_exit_generated: scope stack is empty",
        )));
    }

    ctx.source_map_scope_stack.pop();
    Ok(ConstValue::Unit)
}

/// Get the name of the current generated code scope
fn meta_source_map_current_scope(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }

    let scope = ctx
        .source_map_scope_stack
        .last()
        .cloned()
        .unwrap_or_else(|| Text::from(""));

    Ok(ConstValue::Text(scope))
}

/// Get the full dot-separated scope path
fn meta_source_map_scope_path(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }

    let path: Vec<String> = ctx
        .source_map_scope_stack
        .iter()
        .map(|s| s.to_string())
        .collect();
    let joined = path.join(".");

    Ok(ConstValue::Text(Text::from(joined)))
}

// ============================================================================
// Span Mapping
// ============================================================================

/// Helper: extract a span tuple (file: Text, line: Int, col: Int) from a ConstValue
fn extract_span_key(value: &ConstValue) -> Result<String, MetaError> {
    match value {
        ConstValue::Tuple(fields) if fields.len() >= 3 => {
            let file = match &fields[0] {
                ConstValue::Text(t) => t.to_string(),
                _ => return Err(MetaError::TypeMismatch {
                    expected: Text::from("Text"),
                    found: fields[0].type_name(),
                }),
            };
            let line = match &fields[1] {
                ConstValue::Int(n) => *n as i64,
                _ => return Err(MetaError::TypeMismatch {
                    expected: Text::from("Int"),
                    found: fields[1].type_name(),
                }),
            };
            let col = match &fields[2] {
                ConstValue::Int(n) => *n as i64,
                _ => return Err(MetaError::TypeMismatch {
                    expected: Text::from("Int"),
                    found: fields[2].type_name(),
                }),
            };
            Ok(format!("{}:{}:{}", file, line, col))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Tuple(Text, Int, Int)"),
            found: value.type_name(),
        }),
    }
}

/// Record a mapping from generated span to source span
fn meta_source_map_map_span(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    let gen_key = extract_span_key(&args[0])?;

    // Extract source span as a LineColSpan for storage
    let src_span = match &args[1] {
        ConstValue::Tuple(fields) if fields.len() >= 3 => {
            let file = match &fields[0] {
                ConstValue::Text(t) => t.clone(),
                _ => return Err(MetaError::TypeMismatch {
                    expected: Text::from("Text"),
                    found: fields[0].type_name(),
                }),
            };
            let line = match &fields[1] {
                ConstValue::Int(n) => *n as usize,
                _ => return Err(MetaError::TypeMismatch {
                    expected: Text::from("Int"),
                    found: fields[1].type_name(),
                }),
            };
            let col = match &fields[2] {
                ConstValue::Int(n) => *n as usize,
                _ => return Err(MetaError::TypeMismatch {
                    expected: Text::from("Int"),
                    found: fields[2].type_name(),
                }),
            };
            verum_common::span::LineColSpan::new(file.to_string(), line, col, col.saturating_add(1))
        }
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Tuple(Text, Int, Int)"),
                found: args[1].type_name(),
            });
        }
    };

    ctx.generated_to_source_map
        .insert(Text::from(gen_key), src_span);

    Ok(ConstValue::Unit)
}

/// Look up the source span for a generated span
fn meta_source_map_get_source_span(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    let gen_key = extract_span_key(&args[0])?;

    match ctx.generated_to_source_map.get(&Text::from(gen_key)) {
        Some(src_span) => {
            let span_tuple = ConstValue::Tuple(List::from(vec![
                ConstValue::Text(src_span.file.clone()),
                ConstValue::Int(src_span.line as i128),
                ConstValue::Int(src_span.column as i128),
            ]));
            Ok(ConstValue::Maybe(Maybe::Some(Heap::new(span_tuple))))
        }
        None => Ok(ConstValue::Maybe(Maybe::None)),
    }
}

/// Create a synthetic span with a descriptive message
fn meta_source_map_synthetic_span(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(msg) => {
            let id = ctx.next_synthetic_span_id;
            ctx.next_synthetic_span_id += 1;

            // Return a synthetic span tuple: (file="<synthetic:ID>", line=0, col=0)
            let synthetic_file = Text::from(format!("<synthetic:{}:{}>", id, msg));
            Ok(ConstValue::Tuple(List::from(vec![
                ConstValue::Text(synthetic_file),
                ConstValue::Int(0i128),
                ConstValue::Int(0i128),
            ])))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get all recorded span mappings
fn meta_source_map_get_mappings(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch { expected: 0, got: args.len() });
    }

    let mappings: List<ConstValue> = ctx
        .generated_to_source_map
        .iter()
        .map(|(gen_key, src_span)| {
            ConstValue::Tuple(List::from(vec![
                ConstValue::Text(gen_key.clone()),
                ConstValue::Tuple(List::from(vec![
                    ConstValue::Text(src_span.file.clone()),
                    ConstValue::Int(src_span.line as i128),
                    ConstValue::Int(src_span.column as i128),
                ])),
            ]))
        })
        .collect();

    Ok(ConstValue::Array(mappings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context() -> MetaContext {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts
            .enable(super::super::context_requirements::RequiredContext::SourceMap);
        ctx
    }

    #[test]
    fn test_enter_exit_scope() {
        let mut ctx = create_test_context();

        // Enter a scope
        let args = List::from(vec![ConstValue::Text(Text::from("derive_debug"))]);
        let result = meta_source_map_enter_generated(&mut ctx, args);
        assert!(result.is_ok());
        assert_eq!(ctx.source_map_scope_stack.len(), 1);

        // Check current scope
        let result = meta_source_map_current_scope(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("derive_debug")));

        // Exit scope
        let result = meta_source_map_exit_generated(&mut ctx, List::new());
        assert!(result.is_ok());
        assert!(ctx.source_map_scope_stack.is_empty());
    }

    #[test]
    fn test_scope_path() {
        let mut ctx = create_test_context();

        meta_source_map_enter_generated(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("module"))]),
        )
        .unwrap();
        meta_source_map_enter_generated(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("derive"))]),
        )
        .unwrap();
        meta_source_map_enter_generated(
            &mut ctx,
            List::from(vec![ConstValue::Text(Text::from("Debug"))]),
        )
        .unwrap();

        let result = meta_source_map_scope_path(&mut ctx, List::new()).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("module.derive.Debug")));
    }

    #[test]
    fn test_exit_empty_stack_errors() {
        let mut ctx = create_test_context();
        let result = meta_source_map_exit_generated(&mut ctx, List::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_synthetic_span() {
        let mut ctx = create_test_context();

        let args = List::from(vec![ConstValue::Text(Text::from("generated by derive"))]);
        let result = meta_source_map_synthetic_span(&mut ctx, args).unwrap();

        if let ConstValue::Tuple(fields) = result {
            assert_eq!(fields.len(), 3);
            if let ConstValue::Text(file) = &fields[0] {
                assert!(file.as_str().contains("synthetic"));
                assert!(file.as_str().contains("generated by derive"));
            } else {
                panic!("Expected Text for synthetic span file");
            }
        } else {
            panic!("Expected Tuple");
        }
    }

    #[test]
    fn test_map_and_get_span() {
        let mut ctx = create_test_context();

        // Map a generated span to a source span
        let gen_span = ConstValue::Tuple(List::from(vec![
            ConstValue::Text(Text::from("gen.vr")),
            ConstValue::Int(10i128),
            ConstValue::Int(5i128),
        ]));
        let src_span = ConstValue::Tuple(List::from(vec![
            ConstValue::Text(Text::from("source.vr")),
            ConstValue::Int(42i128),
            ConstValue::Int(1i128),
        ]));

        let result =
            meta_source_map_map_span(&mut ctx, List::from(vec![gen_span.clone(), src_span]));
        assert!(result.is_ok());

        // Look it up
        let result =
            meta_source_map_get_source_span(&mut ctx, List::from(vec![gen_span])).unwrap();
        if let ConstValue::Maybe(Maybe::Some(_)) = &result {
            // Found the mapping
        } else {
            panic!("Expected Maybe::Some");
        }

        // Look up non-existent span
        let missing = ConstValue::Tuple(List::from(vec![
            ConstValue::Text(Text::from("missing.vr")),
            ConstValue::Int(1i128),
            ConstValue::Int(1i128),
        ]));
        let result =
            meta_source_map_get_source_span(&mut ctx, List::from(vec![missing])).unwrap();
        if let ConstValue::Maybe(Maybe::None) = &result {
            // Not found, as expected
        } else {
            panic!("Expected Maybe::None");
        }
    }

    #[test]
    fn test_get_mappings() {
        let mut ctx = create_test_context();

        // Add a mapping
        let gen_span = ConstValue::Tuple(List::from(vec![
            ConstValue::Text(Text::from("gen.vr")),
            ConstValue::Int(1i128),
            ConstValue::Int(1i128),
        ]));
        let src_span = ConstValue::Tuple(List::from(vec![
            ConstValue::Text(Text::from("src.vr")),
            ConstValue::Int(10i128),
            ConstValue::Int(5i128),
        ]));
        meta_source_map_map_span(&mut ctx, List::from(vec![gen_span, src_span])).unwrap();

        let result = meta_source_map_get_mappings(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(mappings) = result {
            assert_eq!(mappings.len(), 1);
        } else {
            panic!("Expected Array");
        }
    }
}
