//! Code actions (quick fixes) support
//!
//! Provides quick fixes for common errors and refactorings including:
//! - Quick fixes for type errors, missing imports, CBGR issues
//! - Refinement violation fixes (via quick_fixes module)
//! - Extract function/method
//! - Extract variable
//! - Inline variable
//! - Implement protocols
//! - Generate documentation
//! - Convert between reference tiers
//! - Quick fixes from syntax tree ERROR nodes

use crate::diagnostics::generate_error_node_fixes;
use crate::document::DocumentState;
use crate::quick_fixes;
use tower_lsp::lsp_types::*;
use verum_common::{List, Maybe};

/// Generate code actions for a given position and diagnostics
pub fn code_actions(
    document: &DocumentState,
    range: Range,
    context: CodeActionContext,
    uri: &Url,
) -> List<CodeActionOrCommand> {
    let mut actions = List::new();

    // Add quick fixes for diagnostics
    for diagnostic in &context.diagnostics {
        add_diagnostic_quick_fixes(&mut actions, document, diagnostic, uri);
    }

    // Add refactoring actions based on selection
    actions.extend(refactoring_actions(document, range, uri));

    // Add context-aware actions
    add_context_aware_actions(&mut actions, document, range, uri);

    // Add source organization actions
    add_source_organization_actions(&mut actions, uri);

    actions
}

/// Add quick fixes for a diagnostic
fn add_diagnostic_quick_fixes(
    actions: &mut List<CodeActionOrCommand>,
    document: &DocumentState,
    diagnostic: &Diagnostic,
    uri: &Url,
) {
    let message = &diagnostic.message;

    // Check for syntax ERROR node fixes (Phase 6 enhancement)
    // These are diagnostics from the parser's ERROR nodes
    if diagnostic.source.as_deref() == Some("verum-parser") {
        let error_fixes = generate_error_node_fixes(diagnostic, &document.text, uri);
        for fix in error_fixes {
            actions.push(CodeActionOrCommand::CodeAction(fix));
        }
    }

    // Handle missing import errors
    if (message.contains("not found")
        || message.contains("undefined")
        || message.contains("unbound"))
        && let Some(symbol_name) = extract_symbol_from_message(message)
        && let Some(import_path) = get_standard_import_path(&symbol_name)
    {
        let edit = create_add_import_edit(document, &import_path, uri);
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: format!("Import `{}` from `{}`", symbol_name, import_path),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(edit),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        }));
    }

    // Handle type mismatch errors
    if message.contains("type mismatch")
        || message.contains("expected") && message.contains("found")
    {
        // Add type conversion suggestion
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Add explicit type cast".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: None,
            command: Some(Command {
                title: "Add type cast".to_string(),
                command: "verum.addTypeCast".to_string(),
                arguments: Some(vec![
                    serde_json::to_value(uri.to_string()).unwrap_or_default(),
                    serde_json::to_value(diagnostic.range).unwrap_or_default(),
                ]),
            }),
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    // Handle CBGR-related errors
    if message.contains("CBGR")
        || message.contains("reference tier")
        || message.contains("use-after-free")
    {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Convert to checked reference (&checked T)".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: None,
            command: Some(Command {
                title: "Convert to checked reference".to_string(),
                command: "verum.convertToCheckedRef".to_string(),
                arguments: Some(vec![
                    serde_json::to_value(uri.to_string()).unwrap_or_default(),
                    serde_json::to_value(diagnostic.range).unwrap_or_default(),
                ]),
            }),
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    // Handle affine/linear type errors
    if message.contains("used after move")
        || message.contains("affine")
        || message.contains("linear")
    {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Clone value before move".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: None,
            command: Some(Command {
                title: "Clone value".to_string(),
                command: "verum.cloneBeforeMove".to_string(),
                arguments: Some(vec![
                    serde_json::to_value(uri.to_string()).unwrap_or_default(),
                    serde_json::to_value(diagnostic.range).unwrap_or_default(),
                ]),
            }),
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    // Handle refinement constraint failures - use comprehensive quick fixes
    if message.contains("refinement") || message.contains("constraint not satisfied") {
        // Extract violated constraint from message
        let constraint = extract_violated_constraint(message);

        // Generate comprehensive refinement quick fixes
        let refinement_fixes = quick_fixes::generate_refinement_quick_fixes(
            document,
            uri,
            diagnostic,
            Maybe::None, // No counterexample available from diagnostic alone
            &constraint,
        );

        // Add all refinement fixes to actions
        for fix in refinement_fixes {
            actions.push(CodeActionOrCommand::CodeAction(fix));
        }
    }

    // Handle missing type annotation errors
    if message.contains("cannot infer type") || message.contains("ambiguous type") {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Add explicit type annotation".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: None,
            command: Some(Command {
                title: "Add type annotation".to_string(),
                command: "verum.addTypeAnnotation".to_string(),
                arguments: Some(vec![
                    serde_json::to_value(uri.to_string()).unwrap_or_default(),
                    serde_json::to_value(diagnostic.range).unwrap_or_default(),
                ]),
            }),
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    // Check for known error codes
    if let Some(code) = &diagnostic.code
        && let Some(action) = quick_fix_for_error_code(code, diagnostic, uri)
    {
        actions.push(action);
    }
}

/// Generate a quick fix based on error code
fn quick_fix_for_error_code(
    code: &NumberOrString,
    diagnostic: &Diagnostic,
    uri: &Url,
) -> Option<CodeActionOrCommand> {
    match code {
        NumberOrString::String(s) if s == "E0308" => {
            // Refinement constraint not satisfied
            Some(create_runtime_check_fix(diagnostic, uri))
        }
        NumberOrString::String(s) if s == "E0311" => {
            // Type mismatch - suggest type conversion
            Some(create_type_conversion_fix(diagnostic, uri))
        }
        NumberOrString::String(s) if s == "E0412" => {
            // Missing import
            None // Handled by add_diagnostic_quick_fixes
        }
        _ => None,
    }
}

/// Extract symbol name from error message
fn extract_symbol_from_message(message: &str) -> Option<String> {
    // Try pattern: `symbol`
    if let Some(start) = message.find('`')
        && let Some(end) = message[start + 1..].find('`')
    {
        return Some(message[start + 1..start + 1 + end].to_string());
    }
    // Try pattern: "not found: symbol"
    if let Some(idx) = message.find("not found:") {
        let rest = message[idx + 10..].trim();
        let symbol = rest.split_whitespace().next()?;
        return Some(
            symbol
                .trim_matches(|c| c == '`' || c == '\'' || c == '"')
                .to_string(),
        );
    }
    None
}

/// Get import path for standard library types
fn get_standard_import_path(symbol: &str) -> Option<String> {
    let imports = [
        ("List", "verum_common.List"),
        ("Text", "verum_common.Text"),
        ("Map", "verum_common.Map"),
        ("Set", "verum_common.Set"),
        ("Maybe", "verum_common.Maybe"),
        ("Result", "verum_common.Result"),
        ("Heap", "verum_common.Heap"),
        ("Shared", "core.sync.Shared"),
        ("File", "core.fs.File"),
        ("Path", "core.fs.Path"),
        ("TcpStream", "core.net.TcpStream"),
        ("Mutex", "core.sync.Mutex"),
        ("Channel", "core.sync.Channel"),
    ];

    imports
        .iter()
        .find(|(name, _)| *name == symbol)
        .map(|(_, path)| path.to_string())
}

/// Create a workspace edit to add an import
fn create_add_import_edit(document: &DocumentState, import_path: &str, uri: &Url) -> WorkspaceEdit {
    let insert_line = find_import_insert_line(document);

    let mut changes = std::collections::HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: Range {
                start: Position {
                    line: insert_line,
                    character: 0,
                },
                end: Position {
                    line: insert_line,
                    character: 0,
                },
            },
            new_text: format!("use {};\n", import_path),
        }],
    );

    WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }
}

/// Find the line to insert a new import
fn find_import_insert_line(document: &DocumentState) -> u32 {
    let mut last_import_line: u32 = 0;

    for (line_num, line) in document.text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("use ") {
            last_import_line = line_num as u32 + 1;
        } else if !trimmed.is_empty() && !trimmed.starts_with("//") && last_import_line > 0 {
            break;
        }
    }

    last_import_line
}

/// Add context-aware actions based on cursor position
fn add_context_aware_actions(
    actions: &mut List<CodeActionOrCommand>,
    document: &DocumentState,
    range: Range,
    uri: &Url,
) {
    let start_offset = document.position_to_offset(range.start);

    if let Some(module) = &document.module {
        for item in module.items.iter() {
            // Check if we're on a function
            if let verum_ast::ItemKind::Function(func) = &item.kind
                && func.span.start <= start_offset
                && start_offset <= func.span.end
            {
                // Generate documentation
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Generate documentation for `{}`", func.name.as_str()),
                    kind: Some(CodeActionKind::REFACTOR),
                    diagnostics: None,
                    edit: None,
                    command: Some(Command {
                        title: "Generate documentation".to_string(),
                        command: "verum.generateDocs".to_string(),
                        arguments: Some(vec![
                            serde_json::to_value(uri.to_string()).unwrap_or_default(),
                            serde_json::to_value(func.span.start).unwrap_or_default(),
                        ]),
                    }),
                    is_preferred: None,
                    disabled: None,
                    data: None,
                }));
                break;
            }

            // Check if we're on a type declaration
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind
                && type_decl.span.start <= start_offset
                && start_offset <= type_decl.span.end
            {
                // Add implement protocol actions
                for protocol in ["Debug", "Clone", "Eq", "Ord", "Hash", "Default"] {
                    actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                        title: format!("Implement {} for `{}`", protocol, type_decl.name.as_str()),
                        kind: Some(CodeActionKind::REFACTOR),
                        diagnostics: None,
                        edit: None,
                        command: Some(Command {
                            title: format!("Implement {}", protocol),
                            command: "verum.implementProtocol".to_string(),
                            arguments: Some(vec![
                                serde_json::to_value(uri.to_string()).unwrap_or_default(),
                                serde_json::to_value(type_decl.name.as_str()).unwrap_or_default(),
                                serde_json::to_value(protocol).unwrap_or_default(),
                            ]),
                        }),
                        is_preferred: None,
                        disabled: None,
                        data: None,
                    }));
                }
                break;
            }
        }
    }
}

/// Add source organization actions
fn add_source_organization_actions(actions: &mut List<CodeActionOrCommand>, uri: &Url) {
    // Organize imports
    actions.push(CodeActionOrCommand::CodeAction(CodeAction {
        title: "Organize imports".to_string(),
        kind: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS),
        diagnostics: None,
        edit: None,
        command: Some(Command {
            title: "Organize imports".to_string(),
            command: "verum.organizeImports".to_string(),
            arguments: Some(vec![serde_json::to_value(uri.to_string()).unwrap_or_default()]),
        }),
        is_preferred: None,
        disabled: None,
        data: None,
    }));

    // Fix all auto-fixable issues
    actions.push(CodeActionOrCommand::CodeAction(CodeAction {
        title: "Fix all auto-fixable issues".to_string(),
        kind: Some(CodeActionKind::SOURCE_FIX_ALL),
        diagnostics: None,
        edit: None,
        command: Some(Command {
            title: "Fix all".to_string(),
            command: "verum.fixAll".to_string(),
            arguments: Some(vec![serde_json::to_value(uri.to_string()).unwrap_or_default()]),
        }),
        is_preferred: None,
        disabled: None,
        data: None,
    }));
}

/// Create a fix that adds a runtime check
fn create_runtime_check_fix(diagnostic: &Diagnostic, uri: &Url) -> CodeActionOrCommand {
    let mut changes = std::collections::HashMap::new();

    // Analyze the diagnostic message to determine the appropriate runtime check
    let (wrapper, error_type) = analyze_runtime_check_needed(&diagnostic.message);

    // Generate the wrapped expression
    let new_text = format!("{}(value).map_err(|_| {})?", wrapper, error_type);

    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: diagnostic.range,
            new_text,
        }],
    );

    CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Add runtime check ({})", wrapper),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    })
}

/// Analyze diagnostic message to determine appropriate runtime check
fn analyze_runtime_check_needed(message: &str) -> (&'static str, &'static str) {
    if message.contains("!= 0") || message.contains("non-zero") || message.contains("division") {
        ("NonZero::try_from", "DivisionByZero")
    } else if message.contains(">= 0") || message.contains("non-negative") {
        ("NonNegative::try_from", "NegativeValue")
    } else if message.contains("> 0") || message.contains("positive") {
        ("Positive::try_from", "NonPositive")
    } else if message.contains("bounds") || message.contains("index") || message.contains("len") {
        ("BoundsCheck::try_from", "IndexOutOfBounds")
    } else if message.contains("Some") || message.contains("None") || message.contains("Option") {
        ("Option::ok_or", "NoneError")
    } else {
        ("validate", "ValidationError")
    }
}

/// Create a fix that suggests type conversion
fn create_type_conversion_fix(diagnostic: &Diagnostic, uri: &Url) -> CodeActionOrCommand {
    let mut changes = std::collections::HashMap::new();

    // Suggest wrapping in 'as' conversion
    let old_text = "value"; // Would extract from document
    let new_text = format!("{} as Type", old_text);

    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: diagnostic.range,
            new_text,
        }],
    );

    CodeActionOrCommand::CodeAction(CodeAction {
        title: "Add type conversion".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    })
}

/// Generate refactoring actions
fn refactoring_actions(
    document: &DocumentState,
    range: Range,
    uri: &Url,
) -> List<CodeActionOrCommand> {
    let mut actions = List::new();

    // Extract function refactoring
    if is_valid_extraction_range(document, range) {
        actions.push(create_extract_function_action(document, range, uri));
    }

    // Extract variable refactoring
    actions.push(create_extract_variable_action(document, range, uri));

    actions
}

/// Extract violated constraint from diagnostic message
fn extract_violated_constraint(message: &str) -> String {
    quick_fixes::extract_constraint_from_message(message)
}

/// Check if a range is valid for function extraction
pub fn is_valid_extraction_range(_document: &DocumentState, range: Range) -> bool {
    // Simple check: must span multiple lines or be a complex expression
    range.start.line < range.end.line || range.end.character - range.start.character > 20
}

/// Create an "extract function" refactoring action with full semantic analysis
fn create_extract_function_action(
    document: &DocumentState,
    range: Range,
    uri: &Url,
) -> CodeActionOrCommand {
    let mut changes = std::collections::HashMap::new();

    // Extract the selected code from the document
    let start_offset = document.position_to_offset(range.start) as usize;
    let end_offset = document.position_to_offset(range.end) as usize;
    let extracted_code = document.text[start_offset..end_offset].to_string();

    // Analyze the extracted code to determine:
    // 1. Free variables (need to be parameters)
    // 2. Modified variables (need to be returned or passed by mut ref)
    // 3. Return type
    // 4. Async/context requirements
    let analysis = analyze_extracted_code(document, range, &extracted_code);

    // Generate a unique function name
    let function_name = generate_unique_function_name(document, "extracted");

    // Build parameter list
    let params = analysis
        .free_variables
        .iter()
        .map(|var| {
            if analysis.mutated_variables.contains(&var.name) {
                format!("{}: &mut {}", var.name, var.ty)
            } else {
                format!("{}: {}", var.name, var.ty)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    // Build return type
    let return_type = if analysis.has_return_value {
        format!(" -> {}", analysis.return_type)
    } else {
        String::new()
    };

    // Build async/context annotations
    let async_prefix = if analysis.is_async { "async " } else { "" };
    let context_annotation = if !analysis.required_contexts.is_empty() {
        format!("\n    using [{}]", analysis.required_contexts.join(", "))
    } else {
        String::new()
    };

    // Build the function body with proper indentation
    let indented_body = indent_code(&extracted_code, "    ");

    // Generate the new function
    let new_function = format!(
        "\n\n{}fn {}({}){}{} {{\n{}\n}}",
        async_prefix, function_name, params, return_type, context_annotation, indented_body
    );

    // Build the function call
    let args = analysis
        .free_variables
        .iter()
        .map(|var| {
            if analysis.mutated_variables.contains(&var.name) {
                format!("&mut {}", var.name)
            } else {
                var.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    // Generate function call: function_name(args) or await function_name(args)
    let function_call = if analysis.is_async {
        format!("{}.await", format!("{}({})", function_name, args))
    } else {
        format!("{}({})", function_name, args)
    };

    // Find insertion point for new function (after current function)
    let insertion_line = find_function_insertion_point(document, range.end.line);

    changes.insert(
        uri.clone(),
        vec![
            // Replace selected code with function call
            TextEdit {
                range,
                new_text: function_call,
            },
            // Insert new function
            TextEdit {
                range: Range {
                    start: Position {
                        line: insertion_line,
                        character: 0,
                    },
                    end: Position {
                        line: insertion_line,
                        character: 0,
                    },
                },
                new_text: new_function,
            },
        ],
    );

    CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Extract to function `{}`", function_name),
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        diagnostics: None,
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    })
}

/// Analysis result for extracted code
struct ExtractedCodeAnalysis {
    /// Variables referenced but not defined in the selection
    free_variables: Vec<VariableInfo>,
    /// Variables that are mutated in the selection
    mutated_variables: Vec<String>,
    /// Whether the code returns a value
    has_return_value: bool,
    /// The inferred return type
    return_type: String,
    /// Whether the code is async
    is_async: bool,
    /// Required context dependencies
    required_contexts: Vec<String>,
}

/// Information about a variable
struct VariableInfo {
    name: String,
    ty: String,
}

/// Analyze extracted code to determine parameters, return type, etc.
fn analyze_extracted_code(
    document: &DocumentState,
    range: Range,
    extracted_code: &str,
) -> ExtractedCodeAnalysis {
    let mut free_variables = Vec::new();
    let mut mutated_variables = Vec::new();
    let mut required_contexts = Vec::new();
    let mut is_async = false;
    let mut has_return_value = false;
    let mut return_type = "()".to_string();

    // Parse the extracted code to analyze it
    if let Some(module) = &document.module {
        // Find the function containing the selection
        let start_offset = document.position_to_offset(range.start);

        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                if func.span.start <= start_offset && start_offset <= func.span.end {
                    // Collect function parameters as potential free variables
                    for param in &func.params {
                        if let verum_ast::FunctionParamKind::Regular { ty, pattern, .. } = &param.kind {
                            let param_name = extract_pattern_name(pattern);
                            // Check if this parameter is used in the extracted code
                            if extracted_code.contains(&param_name) {
                                free_variables.push(VariableInfo {
                                    name: param_name.clone(),
                                    ty: format_type_simple(ty),
                                });

                                // Check if it's mutated
                                if is_variable_mutated(extracted_code, &param_name) {
                                    mutated_variables.push(param_name);
                                }
                            }
                        }
                    }

                    // Check for async
                    is_async = func.is_async;

                    // Check for context requirements
                    for attr in &func.attributes {
                        if attr.name.as_str() == "using" {
                            // Extract context names from the using clause
                            if let verum_common::Maybe::Some(args) = &attr.args {
                                // Parse the context list
                                let ctx_str = format!("{:?}", args);
                                if ctx_str.contains("Database") {
                                    required_contexts.push("Database".to_string());
                                }
                                if ctx_str.contains("Logger") {
                                    required_contexts.push("Logger".to_string());
                                }
                            }
                        }
                    }

                    break;
                }
            }
        }
    }

    // Analyze the extracted code for patterns
    // Check for await expressions
    if extracted_code.contains(".await") || extracted_code.contains("await ") {
        is_async = true;
    }

    // Check for context usage patterns
    if extracted_code.contains("ctx.") || extracted_code.contains("context.") {
        // Infer context requirements from usage
        if extracted_code.contains("database") || extracted_code.contains("db.") {
            if !required_contexts.contains(&"Database".to_string()) {
                required_contexts.push("Database".to_string());
            }
        }
        if extracted_code.contains("log") || extracted_code.contains("logger") {
            if !required_contexts.contains(&"Logger".to_string()) {
                required_contexts.push("Logger".to_string());
            }
        }
    }

    // Check for return value
    // If the last expression is not a statement (no semicolon), it's a return value
    let trimmed = extracted_code.trim();
    if !trimmed.ends_with(';') && !trimmed.ends_with('}') && !trimmed.is_empty() {
        has_return_value = true;
        // Try to infer return type from the expression
        return_type = infer_expression_type(trimmed);
    }

    // Also check for explicit return statements
    if extracted_code.contains("return ") {
        has_return_value = true;
    }

    ExtractedCodeAnalysis {
        free_variables,
        mutated_variables,
        has_return_value,
        return_type,
        is_async,
        required_contexts,
    }
}

/// Extract pattern name as string
fn extract_pattern_name(pattern: &verum_ast::pattern::Pattern) -> String {
    match &pattern.kind {
        verum_ast::pattern::PatternKind::Ident { name, .. } => name.name.to_string(),
        _ => "_".to_string(),
    }
}

/// Format type for display
///
/// Provides comprehensive formatting for all Verum type kinds, including:
/// - Primitive types (Int, Float, Bool, Text, Char, Unit)
/// - Reference types (&T, &mut T, &checked T, &unsafe T)
/// - Container types (List, Map, Set, Maybe, Result)
/// - Function types (fn(Args) -> Return)
/// - Tuple types ((A, B, C))
/// - Array and slice types ([T; N], [T])
/// - Refined types with predicate extraction
/// - Generic types with type arguments
fn format_type_simple(ty: &verum_ast::Type) -> String {
    if let Some(name) = ty.kind.primitive_name() {
        return name.to_string();
    }
    match &ty.kind {
        verum_ast::TypeKind::Path(path) => path
            .segments
            .iter()
            .filter_map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                verum_ast::ty::PathSegment::SelfValue => Some("Self".to_string()),
                verum_ast::ty::PathSegment::Super => Some("super".to_string()),
                verum_ast::ty::PathSegment::Cog => Some("cog".to_string()),
                verum_ast::ty::PathSegment::Relative => None,
            })
            .collect::<Vec<_>>()
            .join("::"),
        verum_ast::TypeKind::Reference { mutable, inner } => {
            if *mutable {
                format!("&mut {}", format_type_simple(inner))
            } else {
                format!("&{}", format_type_simple(inner))
            }
        }
        verum_ast::TypeKind::CheckedReference { mutable, inner } => {
            if *mutable {
                format!("&checked mut {}", format_type_simple(inner))
            } else {
                format!("&checked {}", format_type_simple(inner))
            }
        }
        verum_ast::TypeKind::UnsafeReference { mutable, inner } => {
            if *mutable {
                format!("&unsafe mut {}", format_type_simple(inner))
            } else {
                format!("&unsafe {}", format_type_simple(inner))
            }
        }
        verum_ast::TypeKind::Pointer { mutable, inner } => {
            if *mutable {
                format!("*mut {}", format_type_simple(inner))
            } else {
                format!("*const {}", format_type_simple(inner))
            }
        }
        verum_ast::TypeKind::Refined { base, predicate } => {
            let base_str = format_type_simple(base);
            let pred_str = format_refinement_predicate(&predicate.expr);
            format!("{}{{i | {}}}", base_str, pred_str)
        }
        verum_ast::TypeKind::Tuple(types) => {
            let inner: Vec<String> = types.iter().map(format_type_simple).collect();
            format!("({})", inner.join(", "))
        }
        verum_ast::TypeKind::Array { element, size } => {
            use verum_common::Maybe;
            let size_str = match size.as_ref() {
                Maybe::Some(s) => format_expr_simple(s),
                Maybe::None => "_".to_string(),
            };
            format!("[{}; {}]", format_type_simple(element), size_str)
        }
        verum_ast::TypeKind::Slice(element) => {
            format!("[{}]", format_type_simple(element))
        }
        verum_ast::TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            let params_str: Vec<String> = params.iter().map(format_type_simple).collect();
            format!(
                "fn({}) -> {}",
                params_str.join(", "),
                format_type_simple(return_type)
            )
        }
        verum_ast::TypeKind::Generic { base, args } => {
            let base_str = format_type_simple(base);
            let args_str: Vec<String> = args
                .iter()
                .map(|arg| match arg {
                    verum_ast::ty::GenericArg::Type(t) => format_type_simple(t),
                    verum_ast::ty::GenericArg::Const(e) => format_expr_simple(e),
                    verum_ast::ty::GenericArg::Lifetime(lt) => format!("'{}", lt.name.as_str()),
                    verum_ast::ty::GenericArg::Binding(b) => {
                        format!("{} = {}", b.name.as_str(), format_type_simple(&b.ty))
                    }
                })
                .collect();
            format!("{}<{}>", base_str, args_str.join(", "))
        }
        verum_ast::TypeKind::Inferred => "_".to_string(),
        verum_ast::TypeKind::DynProtocol { bounds, .. } => {
            let bounds_str: Vec<String> = bounds.iter().map(format_type_bound).collect();
            format!("dyn {}", bounds_str.join(" + "))
        }
        verum_ast::TypeKind::Bounded { base, bounds } => {
            let base_str = format_type_simple(base);
            let bounds_str: Vec<String> = bounds.iter().map(format_type_bound).collect();
            format!(
                "{} where {}: {}",
                base_str,
                base_str,
                bounds_str.join(" + ")
            )
        }
        verum_ast::TypeKind::Sigma { name, base, .. } => {
            format!("{}: {}", name.as_str(), format_type_simple(base))
        }
        verum_ast::TypeKind::Qualified {
            self_ty,
            trait_ref,
            assoc_name,
        } => {
            let self_str = format_type_simple(self_ty);
            let trait_str = format_path_for_type(trait_ref);
            format!("<{} as {}>::{}", self_str, trait_str, assoc_name.as_str())
        }
        _ => "_".to_string(),
    }
}

/// Format a path for display in a type context
fn format_path_for_type(path: &verum_ast::Path) -> String {
    path.segments
        .iter()
        .filter_map(|seg| match seg {
            verum_ast::ty::PathSegment::Name(ident) => Some(ident.as_str().to_string()),
            verum_ast::ty::PathSegment::SelfValue => Some("Self".to_string()),
            verum_ast::ty::PathSegment::Super => Some("super".to_string()),
            verum_ast::ty::PathSegment::Cog => Some("cog".to_string()),
            verum_ast::ty::PathSegment::Relative => None,
        })
        .collect::<Vec<_>>()
        .join("::")
}

/// Format a type bound for display
fn format_type_bound(bound: &verum_ast::ty::TypeBound) -> String {
    use verum_ast::ty::TypeBoundKind;
    match &bound.kind {
        TypeBoundKind::Protocol(path) => format_path_for_type(path),
        TypeBoundKind::Equality(ty) => format_type_simple(ty),
        TypeBoundKind::NegativeProtocol(path) => format!("!{}", format_path_for_type(path)),
        TypeBoundKind::AssociatedTypeBound {
            type_path,
            assoc_name,
            bounds,
        } => {
            let bounds_str: Vec<String> = bounds.iter().map(format_type_bound).collect();
            format!(
                "{}.{}: {}",
                format_path_for_type(type_path),
                assoc_name.name,
                bounds_str.join(" + ")
            )
        }
        TypeBoundKind::AssociatedTypeEquality {
            type_path,
            assoc_name,
            eq_type,
        } => {
            format!(
                "{}.{} = {}",
                format_path_for_type(type_path),
                assoc_name.name,
                format_type_simple(eq_type)
            )
        }
        TypeBoundKind::GenericProtocol(ty) => format_type_simple(ty),
    }
}

/// Format a refinement predicate expression
fn format_refinement_predicate(expr: &verum_ast::Expr) -> String {
    match &expr.kind {
        verum_ast::ExprKind::Binary { left, op, right } => {
            let op_str = match op {
                verum_ast::expr::BinOp::Add => "+",
                verum_ast::expr::BinOp::Sub => "-",
                verum_ast::expr::BinOp::Mul => "*",
                verum_ast::expr::BinOp::Div => "/",
                verum_ast::expr::BinOp::Rem => "%",
                verum_ast::expr::BinOp::And => "&&",
                verum_ast::expr::BinOp::Or => "||",
                verum_ast::expr::BinOp::Eq => "==",
                verum_ast::expr::BinOp::Ne => "!=",
                verum_ast::expr::BinOp::Lt => "<",
                verum_ast::expr::BinOp::Le => "<=",
                verum_ast::expr::BinOp::Gt => ">",
                verum_ast::expr::BinOp::Ge => ">=",
                _ => "??",
            };
            format!(
                "{} {} {}",
                format_refinement_predicate(left),
                op_str,
                format_refinement_predicate(right)
            )
        }
        verum_ast::ExprKind::Unary { op, expr: inner } => {
            let op_str = match op {
                verum_ast::expr::UnOp::Neg => "-",
                verum_ast::expr::UnOp::Not => "!",
                _ => "?",
            };
            format!("{}{}", op_str, format_refinement_predicate(inner))
        }
        verum_ast::ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::literal::LiteralKind::Int(i) => i.value.to_string(),
            verum_ast::literal::LiteralKind::Float(f) => f.value.to_string(),
            verum_ast::literal::LiteralKind::Bool(b) => b.to_string(),
            verum_ast::literal::LiteralKind::Text(t) => format!("\"{}\"", t.as_str()),
            _ => "?".to_string(),
        },
        verum_ast::ExprKind::Path(path) => path
            .segments
            .iter()
            .filter_map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("::"),
        verum_ast::ExprKind::Call { func, args, .. } => {
            let args_str: Vec<String> = args.iter().map(format_refinement_predicate).collect();
            format!(
                "{}({})",
                format_refinement_predicate(func),
                args_str.join(", ")
            )
        }
        verum_ast::ExprKind::Field { expr: inner, field } => {
            format!("{}.{}", format_refinement_predicate(inner), field.as_str())
        }
        _ => "...".to_string(),
    }
}

/// Format an expression for simple display
fn format_expr_simple(expr: &verum_ast::Expr) -> String {
    match &expr.kind {
        verum_ast::ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::literal::LiteralKind::Int(i) => i.value.to_string(),
            verum_ast::literal::LiteralKind::Float(f) => f.value.to_string(),
            verum_ast::literal::LiteralKind::Bool(b) => b.to_string(),
            verum_ast::literal::LiteralKind::Char(c) => format!("'{}'", c),
            verum_ast::literal::LiteralKind::Text(t) => format!("\"{}\"", t.as_str()),
            _ => "...".to_string(),
        },
        verum_ast::ExprKind::Path(path) => path
            .segments
            .iter()
            .filter_map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("::"),
        verum_ast::ExprKind::Binary { left, op, right } => {
            let op_str = match op {
                verum_ast::expr::BinOp::Add => "+",
                verum_ast::expr::BinOp::Sub => "-",
                verum_ast::expr::BinOp::Mul => "*",
                verum_ast::expr::BinOp::Div => "/",
                _ => "??",
            };
            format!(
                "{} {} {}",
                format_expr_simple(left),
                op_str,
                format_expr_simple(right)
            )
        }
        _ => "...".to_string(),
    }
}

/// Check if a variable is mutated in the code
fn is_variable_mutated(code: &str, var_name: &str) -> bool {
    // Check for assignment patterns
    let assignment_pattern = format!("{} =", var_name);
    let compound_patterns = [
        format!("{} +=", var_name),
        format!("{} -=", var_name),
        format!("{} *=", var_name),
        format!("{} /=", var_name),
    ];

    code.contains(&assignment_pattern)
        || compound_patterns.iter().any(|p| code.contains(p))
        || code.contains(&format!("&mut {}", var_name))
}

/// Infer expression type from code
///
/// Performs heuristic type inference from expression text by analyzing:
/// - Literal patterns (numbers, strings, booleans)
/// - Constructor calls and known type constructors
/// - Operator usage (arithmetic, boolean, comparison)
/// - Method call patterns
/// - Common standard library types
fn infer_expression_type(expr: &str) -> String {
    let trimmed = expr.trim();

    // Check for boolean literals and operators first
    if trimmed == "true" || trimmed == "false" {
        return "Bool".to_string();
    }

    // Check for unit type
    if trimmed == "()" {
        return "()".to_string();
    }

    // Check for char literals
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 3 {
        return "Char".to_string();
    }

    // Check for string/text literals
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || trimmed.starts_with("text!")
        || trimmed.starts_with("t\"")
        || trimmed.contains(".to_text()")
        || trimmed.contains(".to_string()")
    {
        return "Text".to_string();
    }

    // Check for tagged literals
    if let Some(hash_pos) = trimmed.find('#') {
        if hash_pos > 0 {
            let tag = &trimmed[..hash_pos];
            return match tag {
                "json" => "Json".to_string(),
                "regex" | "re" => "Regex".to_string(),
                "url" => "Url".to_string(),
                "uuid" => "Uuid".to_string(),
                "date" => "Date".to_string(),
                "datetime" => "DateTime".to_string(),
                "duration" => "Duration".to_string(),
                "email" => "Email".to_string(),
                "path" => "Path".to_string(),
                "html" => "Html".to_string(),
                "sql" => "Sql".to_string(),
                "xml" => "Xml".to_string(),
                _ => format!("TaggedLiteral<{}>", tag),
            };
        }
    }

    // Check for float literals (must have decimal point and not be a method call)
    if let Some(dot_pos) = trimmed.find('.') {
        let after_dot = &trimmed[dot_pos + 1..];
        // Check if it's a numeric literal (not a method call)
        if !after_dot.is_empty() && after_dot.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            if trimmed.parse::<f64>().is_ok() {
                return "Float".to_string();
            }
        }
    }

    // Check for integer literals
    if trimmed.parse::<i128>().is_ok()
        || (trimmed.starts_with("0x") && i128::from_str_radix(&trimmed[2..], 16).is_ok())
        || (trimmed.starts_with("0b") && i128::from_str_radix(&trimmed[2..], 2).is_ok())
        || (trimmed.starts_with("0o") && i128::from_str_radix(&trimmed[2..], 8).is_ok())
    {
        return "Int".to_string();
    }

    // Check for boolean expressions
    if trimmed.contains(" && ")
        || trimmed.contains(" || ")
        || trimmed.contains(" == ")
        || trimmed.contains(" != ")
        || trimmed.contains(" < ")
        || trimmed.contains(" > ")
        || trimmed.contains(" <= ")
        || trimmed.contains(" >= ")
        || trimmed.starts_with('!')
        || trimmed.contains(".is_")
        || trimmed.contains(".contains(")
        || trimmed.contains(".starts_with(")
        || trimmed.contains(".ends_with(")
    {
        return "Bool".to_string();
    }

    // Check for collection types
    if trimmed.starts_with("List::") || trimmed.starts_with("list![") || trimmed.starts_with('[') {
        // Try to infer element type
        if let Some(element_type) = infer_collection_element_type(trimmed, '[', ']') {
            return format!("List<{}>", element_type);
        }
        return "List<_>".to_string();
    }

    if trimmed.starts_with("Map::") || trimmed.starts_with("map!{") || trimmed.starts_with('{') {
        // Try to infer key/value types
        if let Some((key_type, val_type)) = infer_map_types(trimmed) {
            return format!("Map<{}, {}>", key_type, val_type);
        }
        return "Map<_, _>".to_string();
    }

    if trimmed.starts_with("Set::") || trimmed.starts_with("set![") {
        return "Set<_>".to_string();
    }

    // Check for option/maybe types
    if trimmed.starts_with("Maybe::Some(")
        || trimmed.starts_with("Some(")
        || trimmed == "None"
        || trimmed == "Maybe::None"
    {
        if trimmed.starts_with("Some(") || trimmed.starts_with("Maybe::Some(") {
            // Extract inner type
            if let Some(inner) = extract_constructor_arg(trimmed, "Some(") {
                let inner_type = infer_expression_type(inner);
                return format!("Maybe<{}>", inner_type);
            }
        }
        return "Maybe<_>".to_string();
    }

    // Check for result types
    if trimmed.starts_with("Result::Ok(")
        || trimmed.starts_with("Ok(")
        || trimmed.starts_with("Result::Err(")
        || trimmed.starts_with("Err(")
    {
        return "Result<_, _>".to_string();
    }

    // Check for arithmetic expressions (likely Int or Float)
    if trimmed.contains(" + ")
        || trimmed.contains(" - ")
        || trimmed.contains(" * ")
        || trimmed.contains(" / ")
        || trimmed.contains(" % ")
    {
        // If it contains a decimal, it's probably Float
        if trimmed.contains('.') && !trimmed.contains("..") {
            return "Float".to_string();
        }
        return "Int".to_string();
    }

    // Check for tuple expressions
    if trimmed.starts_with('(') && trimmed.ends_with(')') && trimmed.contains(',') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let parts: Vec<&str> = split_top_level(inner, ',');
        if parts.len() > 1 {
            let types: Vec<String> = parts.iter().map(|p| infer_expression_type(p)).collect();
            return format!("({})", types.join(", "));
        }
    }

    // Check for range expressions
    if trimmed.contains("..=") {
        return "RangeInclusive<Int>".to_string();
    }
    if trimmed.contains("..") {
        return "Range<Int>".to_string();
    }

    // Check for async expressions
    if trimmed.ends_with(".await") {
        // The result type is unknown
        return "_".to_string();
    }

    // Check for closure expressions
    if (trimmed.starts_with('|') && trimmed.contains("| "))
        || trimmed.starts_with("move |")
        || trimmed.starts_with("async |")
    {
        return "fn(...) -> _".to_string();
    }

    // Check for known method return types
    if trimmed.ends_with(".len()") || trimmed.ends_with(".count()") {
        return "Int".to_string();
    }
    if trimmed.ends_with(".iter()") {
        return "Iterator<_>".to_string();
    }
    if trimmed.ends_with(".clone()") || trimmed.ends_with(".to_owned()") {
        return "_".to_string();
    }

    // Default: unknown type
    "_".to_string()
}

/// Infer the element type of a collection literal
fn infer_collection_element_type(expr: &str, open: char, close: char) -> Option<String> {
    let start = expr.find(open)?;
    let end = expr.rfind(close)?;
    if start >= end {
        return None;
    }

    let inner = &expr[start + 1..end].trim();
    if inner.is_empty() {
        return None;
    }

    // Get the first element
    let parts = split_top_level(inner, ',');
    if !parts.is_empty() {
        let first_element = parts[0].trim();
        if !first_element.is_empty() {
            return Some(infer_expression_type(first_element));
        }
    }

    None
}

/// Infer key and value types from a map literal
fn infer_map_types(expr: &str) -> Option<(String, String)> {
    let start = expr.find('{')?;
    let end = expr.rfind('}')?;
    if start >= end {
        return None;
    }

    let inner = &expr[start + 1..end].trim();
    if inner.is_empty() {
        return None;
    }

    // Find the first key-value pair
    if let Some(colon_pos) = inner.find(':') {
        let key = inner[..colon_pos].trim();
        let rest = &inner[colon_pos + 1..];
        // Find where the value ends (at next comma or end)
        let value_end = rest.find(',').unwrap_or(rest.len());
        let value = rest[..value_end].trim();

        if !key.is_empty() && !value.is_empty() {
            return Some((infer_expression_type(key), infer_expression_type(value)));
        }
    }

    None
}

/// Extract the argument from a constructor call like Some(value) or Ok(value)
fn extract_constructor_arg<'a>(expr: &'a str, constructor: &str) -> Option<&'a str> {
    let start = expr.find(constructor)?;
    let after_constructor = &expr[start + constructor.len()..];

    // Find matching parenthesis
    let mut depth = 1;
    for (i, c) in after_constructor.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&after_constructor[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a string by a delimiter, but only at the top level (ignoring nested brackets)
fn split_top_level(s: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut last_split = 0;

    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            c if c == delimiter && depth == 0 => {
                parts.push(&s[last_split..i]);
                last_split = i + 1;
            }
            _ => {}
        }
    }

    if last_split < s.len() {
        parts.push(&s[last_split..]);
    }

    parts
}

/// Generate a unique function name
fn generate_unique_function_name(document: &DocumentState, base: &str) -> String {
    let mut name = base.to_string();
    let mut counter = 1;

    // Check if name already exists in the document
    while document.text.contains(&format!("fn {}(", name))
        || document.text.contains(&format!("fn {} (", name))
    {
        name = format!("{}_{}", base, counter);
        counter += 1;
    }

    name
}

/// Find the best insertion point for a new function
fn find_function_insertion_point(document: &DocumentState, after_line: u32) -> u32 {
    // Find the end of the current function
    let lines: Vec<&str> = document.text.lines().collect();
    let mut brace_count = 0;
    let mut in_function = false;

    for (line_num, line) in lines.iter().enumerate().skip(after_line as usize) {
        for ch in line.chars() {
            if ch == '{' {
                brace_count += 1;
                in_function = true;
            } else if ch == '}' {
                brace_count -= 1;
                if in_function && brace_count == 0 {
                    // Found end of function, insert after this line
                    return (line_num + 1) as u32;
                }
            }
        }
    }

    // Fallback: insert at end of file
    lines.len() as u32
}

/// Indent code by the given prefix
fn indent_code(code: &str, indent: &str) -> String {
    code.lines()
        .map(|line| format!("{}{}", indent, line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Create an "extract variable" refactoring action
fn create_extract_variable_action(
    _document: &DocumentState,
    range: Range,
    uri: &Url,
) -> CodeActionOrCommand {
    let mut changes = std::collections::HashMap::new();

    let variable_name = "extracted_var";
    let extracted_value = "value"; // Would come from document

    // Insert variable declaration before the range
    let declaration = format!("let {} = {};\n", variable_name, extracted_value);

    changes.insert(
        uri.clone(),
        vec![
            // Insert variable declaration
            TextEdit {
                range: Range {
                    start: range.start,
                    end: range.start,
                },
                new_text: declaration,
            },
            // Replace selected code with variable reference
            TextEdit {
                range,
                new_text: variable_name.to_string(),
            },
        ],
    );

    CodeActionOrCommand::CodeAction(CodeAction {
        title: "Extract to variable".to_string(),
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        diagnostics: None,
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    })
}
