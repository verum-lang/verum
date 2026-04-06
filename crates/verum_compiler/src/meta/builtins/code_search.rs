//! Code Search Intrinsics (Tier 1 - Requires MetaTypes)
//!
//! Provides compile-time code search functionality for meta-programming.
//! Uses the type registry, usage indices, and module registry from MetaContext
//! to search for functions, types, usages, and module contents.
//!
//! ## Function Search
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `search_find_functions_with_attr(attr)` | `(Text) -> List<Map>` | Find functions with attribute |
//! | `search_find_functions_by_pattern(pattern)` | `(Text) -> List<Map>` | Find functions by name pattern |
//! | `search_find_functions_in_module(module)` | `(Text) -> List<Map>` | Find functions in module |
//!
//! ## Type Search
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `search_find_types_implementing(protocol)` | `(Text) -> List<Map>` | Find types implementing protocol |
//! | `search_find_types_with_attr(attr)` | `(Text) -> List<Map>` | Find types with attribute |
//! | `search_find_types_in_module(module)` | `(Text) -> List<Map>` | Find types in module |
//!
//! ## Usage Search
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `search_find_function_usages(path)` | `(Text) -> List<Map>` | Find function usages |
//! | `search_find_type_usages(path)` | `(Text) -> List<Map>` | Find type usages |
//!
//! ## Module Search
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `search_all_modules()` | `() -> List<Text>` | List all modules |
//! | `search_module_public_items(module)` | `(Text) -> List<Map>` | List public items in module |
//! | `search_module_dependencies(module)` | `(Text) -> List<Text>` | List module dependencies |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [MetaTypes]` context.

use verum_common::{List, OrderedMap, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register code search builtins with context requirements
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Function Search (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("search_find_functions_with_attr"),
        BuiltinInfo::meta_types(
            meta_search_find_functions_with_attr,
            "Find functions that have a specific attribute",
            "(Text) -> List<Map>",
        ),
    );
    map.insert(
        Text::from("search_find_functions_by_pattern"),
        BuiltinInfo::meta_types(
            meta_search_find_functions_by_pattern,
            "Find functions whose name matches a pattern",
            "(Text) -> List<Map>",
        ),
    );
    map.insert(
        Text::from("search_find_functions_in_module"),
        BuiltinInfo::meta_types(
            meta_search_find_functions_in_module,
            "Find all functions defined in a module",
            "(Text) -> List<Map>",
        ),
    );

    // ========================================================================
    // Type Search (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("search_find_types_implementing"),
        BuiltinInfo::meta_types(
            meta_search_find_types_implementing,
            "Find types that implement a specific protocol",
            "(Text) -> List<Map>",
        ),
    );
    map.insert(
        Text::from("search_find_types_with_attr"),
        BuiltinInfo::meta_types(
            meta_search_find_types_with_attr,
            "Find types that have a specific attribute",
            "(Text) -> List<Map>",
        ),
    );
    map.insert(
        Text::from("search_find_types_in_module"),
        BuiltinInfo::meta_types(
            meta_search_find_types_in_module,
            "Find all types defined in a module",
            "(Text) -> List<Map>",
        ),
    );

    // ========================================================================
    // Usage Search (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("search_find_function_usages"),
        BuiltinInfo::meta_types(
            meta_search_find_function_usages,
            "Find all usages of a function by path",
            "(Text) -> List<Map>",
        ),
    );
    map.insert(
        Text::from("search_find_type_usages"),
        BuiltinInfo::meta_types(
            meta_search_find_type_usages,
            "Find all usages of a type by path",
            "(Text) -> List<Map>",
        ),
    );

    // ========================================================================
    // Module Search (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("search_all_modules"),
        BuiltinInfo::meta_types(
            meta_search_all_modules,
            "List all registered modules",
            "() -> List<Text>",
        ),
    );
    map.insert(
        Text::from("search_module_public_items"),
        BuiltinInfo::meta_types(
            meta_search_module_public_items,
            "List public items exported by a module",
            "(Text) -> List<Map>",
        ),
    );
    map.insert(
        Text::from("search_module_dependencies"),
        BuiltinInfo::meta_types(
            meta_search_module_dependencies,
            "List dependencies of a module",
            "(Text) -> List<Text>",
        ),
    );
}

// ============================================================================
// Helpers
// ============================================================================

fn extract_text_arg(args: &List<ConstValue>, index: usize) -> Result<Text, MetaError> {
    match args.get(index) {
        Some(ConstValue::Text(t)) => Ok(t.clone()),
        Some(other) => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: other.type_name(),
        }),
        None => Err(MetaError::ArityMismatch {
            expected: index + 1,
            got: index,
        }),
    }
}

/// Build a function search result as a Map
fn make_function_result(name: &Text, return_type: &Text, attributes: &List<Text>) -> ConstValue {
    let mut result = OrderedMap::new();
    result.insert(Text::from("name"), ConstValue::Text(name.clone()));
    result.insert(
        Text::from("return_type"),
        ConstValue::Text(return_type.clone()),
    );
    let attr_values: List<ConstValue> = attributes
        .iter()
        .map(|a| ConstValue::Text(a.clone()))
        .collect();
    result.insert(Text::from("attributes"), ConstValue::Array(attr_values));
    ConstValue::Map(result)
}

/// Build a type search result as a Map
fn make_type_result(name: &Text, protocols: &List<Text>, attributes: &List<Text>) -> ConstValue {
    let mut result = OrderedMap::new();
    result.insert(Text::from("name"), ConstValue::Text(name.clone()));
    let proto_values: List<ConstValue> = protocols
        .iter()
        .map(|p| ConstValue::Text(p.clone()))
        .collect();
    result.insert(Text::from("protocols"), ConstValue::Array(proto_values));
    let attr_values: List<ConstValue> = attributes
        .iter()
        .map(|a| ConstValue::Text(a.clone()))
        .collect();
    result.insert(Text::from("attributes"), ConstValue::Array(attr_values));
    ConstValue::Map(result)
}

/// Build a usage result as a Map
fn make_usage_result(context: &Text, span_start: u32, span_end: u32) -> ConstValue {
    let mut result = OrderedMap::new();
    result.insert(Text::from("context"), ConstValue::Text(context.clone()));
    result.insert(Text::from("span_start"), ConstValue::Int(span_start as i128));
    result.insert(Text::from("span_end"), ConstValue::Int(span_end as i128));
    result.insert(
        Text::from("span"),
        ConstValue::Text(Text::from(format!("{}..{}", span_start, span_end))),
    );
    ConstValue::Map(result)
}

/// Build an item result as a Map
fn make_item_result(name: &Text, kind: &str) -> ConstValue {
    let mut result = OrderedMap::new();
    result.insert(Text::from("name"), ConstValue::Text(name.clone()));
    result.insert(Text::from("kind"), ConstValue::Text(Text::from(kind)));
    ConstValue::Map(result)
}

// ============================================================================
// Function Search Implementations
// ============================================================================

/// Find functions that have a specific attribute
fn meta_search_find_functions_with_attr(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let attr = extract_text_arg(&args, 0)?;
    let mut results = List::new();

    for (name, info) in ctx.type_registry.iter() {
        match info {
            crate::meta::CodeSearchTypeInfo::Function {
                return_type,
                attributes,
                ..
            } => {
                if attributes.iter().any(|a| a == &attr) {
                    results.push(make_function_result(name, return_type, attributes));
                }
            }
            _ => {}
        }
    }

    Ok(ConstValue::Array(results))
}

/// Find functions whose name matches a pattern (substring match)
fn meta_search_find_functions_by_pattern(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let pattern = extract_text_arg(&args, 0)?;
    let mut results = List::new();

    for (name, info) in ctx.type_registry.iter() {
        match info {
            crate::meta::CodeSearchTypeInfo::Function {
                return_type,
                attributes,
                ..
            } => {
                if name.as_str().contains(pattern.as_str()) {
                    results.push(make_function_result(name, return_type, attributes));
                }
            }
            _ => {}
        }
    }

    Ok(ConstValue::Array(results))
}

/// Find all functions defined in a module
fn meta_search_find_functions_in_module(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;
    let module_prefix = format!("{}.", module);
    let mut results = List::new();

    // Search type_registry for functions whose path starts with the module prefix
    for (name, info) in ctx.type_registry.iter() {
        match info {
            crate::meta::CodeSearchTypeInfo::Function {
                return_type,
                attributes,
                ..
            } => {
                if name.as_str().starts_with(&module_prefix) || name == &module {
                    results.push(make_function_result(name, return_type, attributes));
                }
            }
            _ => {}
        }
    }

    // Also check module_registry for public function items
    if let Some(module_info) = ctx.module_registry.get(&module) {
        for item in module_info.public_items.iter() {
            if item.kind == crate::meta::ItemKind::Function {
                let full_name = Text::from(format!("{}.{}", module, item.name));
                // Avoid duplicates: check if already in results by name
                let already_found = results.iter().any(|r| {
                    if let ConstValue::Map(m) = r {
                        m.get(&Text::from("name"))
                            == Some(&ConstValue::Text(full_name.clone()))
                    } else {
                        false
                    }
                });
                if !already_found {
                    results.push(make_function_result(
                        &full_name,
                        &Text::from(""),
                        &List::new(),
                    ));
                }
            }
        }
    }

    Ok(ConstValue::Array(results))
}

// ============================================================================
// Type Search Implementations
// ============================================================================

/// Find types that implement a specific protocol
fn meta_search_find_types_implementing(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let protocol = extract_text_arg(&args, 0)?;
    let mut results = List::new();

    // Search type_registry for types that implement the protocol
    for (name, info) in ctx.type_registry.iter() {
        match info {
            crate::meta::CodeSearchTypeInfo::Type {
                protocols,
                attributes,
                ..
            } => {
                if protocols.iter().any(|p| p == &protocol) {
                    results.push(make_type_result(name, protocols, attributes));
                }
            }
            _ => {}
        }
    }

    // Also check protocol_implementations in the context
    let implementors = ctx.get_implementors(&protocol);
    for impl_type in implementors.iter() {
        // Check if already in results
        let already_found = results.iter().any(|r| {
            if let ConstValue::Map(m) = r {
                m.get(&Text::from("name")) == Some(&ConstValue::Text(impl_type.clone()))
            } else {
                false
            }
        });
        if !already_found {
            let type_protocols = ctx.get_implemented_protocols(impl_type);
            let type_attrs = ctx.get_type_attributes(impl_type);
            results.push(make_type_result(impl_type, &type_protocols, &type_attrs));
        }
    }

    Ok(ConstValue::Array(results))
}

/// Find types that have a specific attribute
fn meta_search_find_types_with_attr(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let attr = extract_text_arg(&args, 0)?;
    let mut results = List::new();

    // Search type_registry
    for (name, info) in ctx.type_registry.iter() {
        match info {
            crate::meta::CodeSearchTypeInfo::Type {
                protocols,
                attributes,
                ..
            } => {
                if attributes.iter().any(|a| a == &attr) {
                    results.push(make_type_result(name, protocols, attributes));
                }
            }
            _ => {}
        }
    }

    // Also search type_definitions for attribute matches
    for (name, _type_def) in ctx.type_definitions.iter() {
        if ctx.type_has_attribute(name, &attr) {
            // Check if already in results
            let already_found = results.iter().any(|r| {
                if let ConstValue::Map(m) = r {
                    m.get(&Text::from("name")) == Some(&ConstValue::Text(name.clone()))
                } else {
                    false
                }
            });
            if !already_found {
                let type_protocols = ctx.get_implemented_protocols(name);
                let type_attrs = ctx.get_type_attributes(name);
                results.push(make_type_result(name, &type_protocols, &type_attrs));
            }
        }
    }

    Ok(ConstValue::Array(results))
}

/// Find all types defined in a module
fn meta_search_find_types_in_module(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;
    let module_prefix = format!("{}.", module);
    let mut results = List::new();

    // Search type_registry for types in this module
    for (name, info) in ctx.type_registry.iter() {
        match info {
            crate::meta::CodeSearchTypeInfo::Type {
                protocols,
                attributes,
                ..
            } => {
                if name.as_str().starts_with(&module_prefix) || name == &module {
                    results.push(make_type_result(name, protocols, attributes));
                }
            }
            _ => {}
        }
    }

    // Also check module_registry for public type items
    if let Some(module_info) = ctx.module_registry.get(&module) {
        for item in module_info.public_items.iter() {
            if item.kind == crate::meta::ItemKind::Type {
                let full_name = Text::from(format!("{}.{}", module, item.name));
                let already_found = results.iter().any(|r| {
                    if let ConstValue::Map(m) = r {
                        m.get(&Text::from("name"))
                            == Some(&ConstValue::Text(full_name.clone()))
                    } else {
                        false
                    }
                });
                if !already_found {
                    results.push(make_type_result(
                        &full_name,
                        &List::new(),
                        &List::new(),
                    ));
                }
            }
        }
    }

    Ok(ConstValue::Array(results))
}

// ============================================================================
// Usage Search Implementations
// ============================================================================

/// Find all usages of a function by path
fn meta_search_find_function_usages(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let path = extract_text_arg(&args, 0)?;
    let mut results = List::new();

    if let Some(usages) = ctx.usage_index.get(&path) {
        for usage in usages.iter() {
            results.push(make_usage_result(
                &usage.context,
                usage.span.start,
                usage.span.end,
            ));
        }
    }

    Ok(ConstValue::Array(results))
}

/// Find all usages of a type by path
fn meta_search_find_type_usages(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let path = extract_text_arg(&args, 0)?;
    let mut results = List::new();

    if let Some(usages) = ctx.type_usage_index.get(&path) {
        for usage in usages.iter() {
            results.push(make_usage_result(
                &usage.context,
                usage.span.start,
                usage.span.end,
            ));
        }
    }

    Ok(ConstValue::Array(results))
}

// ============================================================================
// Module Search Implementations
// ============================================================================

/// List all registered modules
fn meta_search_all_modules(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let mut modules: List<ConstValue> = ctx
        .module_registry
        .keys()
        .map(|k| ConstValue::Text(k.clone()))
        .collect();

    // Sort for deterministic output
    modules.sort_by(|a, b| {
        if let (ConstValue::Text(ta), ConstValue::Text(tb)) = (a, b) {
            ta.cmp(tb)
        } else {
            std::cmp::Ordering::Equal
        }
    });

    Ok(ConstValue::Array(modules))
}

/// List public items exported by a module
fn meta_search_module_public_items(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;
    let mut results = List::new();

    if let Some(module_info) = ctx.module_registry.get(&module) {
        for item in module_info.public_items.iter() {
            results.push(make_item_result(&item.name, item.kind.as_str()));
        }
    }

    Ok(ConstValue::Array(results))
}

/// List dependencies of a module
fn meta_search_module_dependencies(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;

    if let Some(module_info) = ctx.module_registry.get(&module) {
        let deps: List<ConstValue> = module_info
            .dependencies
            .iter()
            .map(|d| ConstValue::Text(d.clone()))
            .collect();
        Ok(ConstValue::Array(deps))
    } else {
        // Module not found, return empty list
        Ok(ConstValue::Array(List::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::subsystems::code_search::{
        CodeSearchTypeInfo, ItemInfo, ItemKind, ModuleInfo, UsageInfo,
    };
    use verum_ast::Span;

    fn create_test_context() -> MetaContext {
        let mut ctx = MetaContext::new();

        // Register some functions in the type registry
        ctx.type_registry.insert(
            Text::from("math.add"),
            CodeSearchTypeInfo::Function {
                return_type: Text::from("Int"),
                attributes: List::from(vec![Text::from("inline"), Text::from("pure")]),
                span: Span::dummy(),
            },
        );
        ctx.type_registry.insert(
            Text::from("math.subtract"),
            CodeSearchTypeInfo::Function {
                return_type: Text::from("Int"),
                attributes: List::from(vec![Text::from("inline")]),
                span: Span::dummy(),
            },
        );
        ctx.type_registry.insert(
            Text::from("io.print"),
            CodeSearchTypeInfo::Function {
                return_type: Text::from("()"),
                attributes: List::new(),
                span: Span::dummy(),
            },
        );

        // Register some types
        ctx.type_registry.insert(
            Text::from("math.Vector"),
            CodeSearchTypeInfo::Type {
                protocols: List::from(vec![Text::from("Eq"), Text::from("Debug")]),
                attributes: List::from(vec![Text::from("derive")]),
                span: Span::dummy(),
            },
        );
        ctx.type_registry.insert(
            Text::from("math.Matrix"),
            CodeSearchTypeInfo::Type {
                protocols: List::from(vec![Text::from("Debug")]),
                attributes: List::new(),
                span: Span::dummy(),
            },
        );

        // Register usage info
        ctx.usage_index.insert(
            Text::from("math.add"),
            List::from(vec![
                UsageInfo::new(Span::new(10, 20, verum_ast::FileId::new(0)), Text::from("call")),
                UsageInfo::new(Span::new(50, 60, verum_ast::FileId::new(0)), Text::from("call")),
            ]),
        );

        ctx.type_usage_index.insert(
            Text::from("math.Vector"),
            List::from(vec![UsageInfo::new(
                Span::new(100, 110, verum_ast::FileId::new(0)),
                Text::from("type annotation"),
            )]),
        );

        // Register modules
        let mut math_module = ModuleInfo::new();
        math_module
            .public_items
            .push(ItemInfo::new(Text::from("add"), ItemKind::Function));
        math_module
            .public_items
            .push(ItemInfo::new(Text::from("Vector"), ItemKind::Type));
        math_module
            .dependencies
            .push(Text::from("core"));

        let mut io_module = ModuleInfo::new();
        io_module
            .public_items
            .push(ItemInfo::new(Text::from("print"), ItemKind::Function));

        ctx.module_registry
            .insert(Text::from("math"), math_module);
        ctx.module_registry.insert(Text::from("io"), io_module);

        ctx
    }

    #[test]
    fn test_find_functions_with_attr() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("inline"))]);
        let result = meta_search_find_functions_with_attr(&mut ctx, args).unwrap();
        if let ConstValue::Array(funcs) = result {
            assert_eq!(funcs.len(), 2); // math.add and math.subtract
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_functions_with_attr_no_matches() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("deprecated"))]);
        let result = meta_search_find_functions_with_attr(&mut ctx, args).unwrap();
        if let ConstValue::Array(funcs) = result {
            assert!(funcs.is_empty());
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_functions_by_pattern() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("math"))]);
        let result = meta_search_find_functions_by_pattern(&mut ctx, args).unwrap();
        if let ConstValue::Array(funcs) = result {
            assert_eq!(funcs.len(), 2); // math.add, math.subtract
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_functions_in_module() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("math"))]);
        let result = meta_search_find_functions_in_module(&mut ctx, args).unwrap();
        if let ConstValue::Array(funcs) = result {
            // math.add and math.subtract from type_registry
            assert!(funcs.len() >= 2);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_types_implementing() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("Debug"))]);
        let result = meta_search_find_types_implementing(&mut ctx, args).unwrap();
        if let ConstValue::Array(types) = result {
            assert_eq!(types.len(), 2); // Vector and Matrix
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_types_implementing_eq() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("Eq"))]);
        let result = meta_search_find_types_implementing(&mut ctx, args).unwrap();
        if let ConstValue::Array(types) = result {
            assert_eq!(types.len(), 1); // Only Vector
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_types_with_attr() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("derive"))]);
        let result = meta_search_find_types_with_attr(&mut ctx, args).unwrap();
        if let ConstValue::Array(types) = result {
            assert_eq!(types.len(), 1); // Vector has derive
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_types_in_module() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("math"))]);
        let result = meta_search_find_types_in_module(&mut ctx, args).unwrap();
        if let ConstValue::Array(types) = result {
            assert!(types.len() >= 2); // Vector and Matrix
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_function_usages() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("math.add"))]);
        let result = meta_search_find_function_usages(&mut ctx, args).unwrap();
        if let ConstValue::Array(usages) = result {
            assert_eq!(usages.len(), 2);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_function_usages_no_match() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("nonexistent"))]);
        let result = meta_search_find_function_usages(&mut ctx, args).unwrap();
        if let ConstValue::Array(usages) = result {
            assert!(usages.is_empty());
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_type_usages() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("math.Vector"))]);
        let result = meta_search_find_type_usages(&mut ctx, args).unwrap();
        if let ConstValue::Array(usages) = result {
            assert_eq!(usages.len(), 1);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_all_modules() {
        let mut ctx = create_test_context();
        let result = meta_search_all_modules(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(modules) = result {
            assert_eq!(modules.len(), 2); // math, io
            // Should be sorted
            assert_eq!(modules[0], ConstValue::Text(Text::from("io")));
            assert_eq!(modules[1], ConstValue::Text(Text::from("math")));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_module_public_items() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("math"))]);
        let result = meta_search_module_public_items(&mut ctx, args).unwrap();
        if let ConstValue::Array(items) = result {
            assert_eq!(items.len(), 2); // add, Vector
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_module_public_items_not_found() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("nonexistent"))]);
        let result = meta_search_module_public_items(&mut ctx, args).unwrap();
        if let ConstValue::Array(items) = result {
            assert!(items.is_empty());
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_module_dependencies() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("math"))]);
        let result = meta_search_module_dependencies(&mut ctx, args).unwrap();
        if let ConstValue::Array(deps) = result {
            assert_eq!(deps.len(), 1);
            assert_eq!(deps[0], ConstValue::Text(Text::from("core")));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_module_dependencies_not_found() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("nonexistent"))]);
        let result = meta_search_module_dependencies(&mut ctx, args).unwrap();
        if let ConstValue::Array(deps) = result {
            assert!(deps.is_empty());
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_arity_errors() {
        let mut ctx = create_test_context();

        // Too many args
        let args = List::from(vec![
            ConstValue::Text(Text::from("a")),
            ConstValue::Text(Text::from("b")),
        ]);
        assert!(meta_search_find_functions_with_attr(&mut ctx, args).is_err());

        // Missing args
        let args = List::new();
        assert!(meta_search_find_functions_with_attr(&mut ctx, args).is_err());

        // Wrong type
        let args = List::from(vec![ConstValue::Int(42)]);
        assert!(meta_search_find_functions_with_attr(&mut ctx, args).is_err());
    }

    #[test]
    fn test_search_all_modules_not_empty_args() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("extra"))]);
        assert!(meta_search_all_modules(&mut ctx, args).is_err());
    }
}
