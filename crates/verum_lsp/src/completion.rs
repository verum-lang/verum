//! Auto-completion support
//!
//! Provides context-aware code completions for keywords, types, functions, and variables.
//! Includes member access completion for struct/enum fields, module import completion,
//! and attribute completion with full registry support.

use crate::ast_format::{format_params, format_type};
use crate::document::{DocumentState, SymbolKind};
use tower_lsp::lsp_types::*;
use verum_ast::attr::{ArgSpec, ArgType, AttributeTarget};
use verum_common::List;
use verum_types::attr::{AttributeCategory, AttributeMetadata, registry};

/// Keywords in the Verum language
const KEYWORDS: &[&str] = &[
    // Core language constructs
    "fn",
    "let",
    "mut",
    "if",
    "else",
    "match",
    "for",
    "while",
    "loop",
    "break",
    "continue",
    "return",
    "type",
    "struct",
    "enum",
    "protocol",
    "impl",
    "implement",
    "mod",
    "use",
    "pub",
    // Async/concurrency keywords
    "async",
    "await",
    "spawn",
    "nursery",
    "select",
    "stream",
    // Error handling keywords
    "defer",
    "errdefer",
    "throws",
    "recover",
    "finally",
    // Verification/constraint keywords
    "verify",
    "requires",
    "ensures",
    "invariant",
    "decreases",
    "assert",
    "assume",
    // Linearity keywords
    "affine",
    "linear",
    // Reference keywords
    "ref",
    "checked",
    "unsafe",
    // Operators and misc
    "as",
    "in",
    "is",
    "true",
    "false",
    "null",
    "self",
    // Proof keywords
    "theorem",
    "lemma",
    "axiom",
    "corollary",
    "proof",
    "by",
    "qed",
    "have",
    "show",
    "suffices",
    "obtain",
    "calc",
    // Quantifiers
    "forall",
    "exists",
    // Context system
    "context",
    "provide",
    "using",
    // Meta/staging
    "meta",
    "quote",
    "lift",
];

/// Built-in types in Verum
const BUILTIN_TYPES: &[&str] = &[
    "Int", "Float", "Bool", "Char", "Text", "Unit", "List", "Map", "Set", "Maybe", "Result",
    "Heap", "Shared", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
    "Float32", "Float64",
];

/// Generate completions for a document at a given position.
///
/// Each item carries a `data` payload (`{"uri": ..., "name": ...}`) that the
/// `completionItem/resolve` handler can use to lazily fill documentation and
/// type details without computing them up-front for every candidate.
pub fn complete_at_position(document: &DocumentState, position: Position) -> List<CompletionItem> {
    let mut completions = List::new();

    // Get context at position
    let line = document.get_line(position.line);
    let trigger_context = line.and_then(|l| get_trigger_context(l, position.character));

    match trigger_context {
        Some(TriggerContext::Attribute {
            partial_name,
            target,
        }) => {
            add_attribute_completions(&mut completions, partial_name.as_deref(), target);
        }
        Some(TriggerContext::Type) => {
            add_type_completions(&mut completions);
        }
        Some(TriggerContext::Expression) | None => {
            add_keyword_completions(&mut completions);
            add_type_completions(&mut completions);

            if let Some(module) = &document.module {
                add_module_completions(&mut completions, module);
            }
        }
        Some(TriggerContext::Member) => {
            if let Some(line_str) = line
                && let Some(receiver) = get_receiver_name(line_str, position.character)
            {
                add_member_completions(&mut completions, document, &receiver);
            }
        }
        Some(TriggerContext::Import) => {
            if let Some(line_str) = line {
                add_import_completions(&mut completions, document, line_str);
            }
        }
        Some(TriggerContext::ProofTactic) => {
            add_tactic_completions(&mut completions);
        }
        Some(TriggerContext::UsingContext) => {
            add_context_completions(&mut completions);
        }
        Some(TriggerContext::TaggedLiteral) => {
            add_tagged_literal_completions(&mut completions);
        }
    }

    completions
}

/// Attach a resolve-data payload to a completion item.
///
/// The payload is a small JSON object that the `completionItem/resolve`
/// handler can use to look up full documentation and type info without
/// the initial completion request having to compute it for every item.
pub fn attach_resolve_data(item: &mut CompletionItem, uri: &str, name: &str) {
    item.data = Some(serde_json::json!({
        "uri": uri,
        "name": name,
    }));
}

/// Context in which completion was triggered
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerContext {
    /// After a colon (type position)
    Type,
    /// After a dot (member access)
    Member,
    /// After 'use' keyword (import)
    Import,
    /// After '@' (attribute)
    Attribute {
        /// Partial attribute name typed so far (if any)
        partial_name: Option<String>,
        /// Inferred target context for filtering attributes
        target: AttributeTarget,
    },
    /// General expression context
    Expression,
    /// After 'proof by' - suggest tactics
    ProofTactic,
    /// Inside using [...] - suggest contexts
    UsingContext,
    /// After format tag prefix (e.g., 'sql#', 'rx#') - suggest tag completions
    TaggedLiteral,
}

/// Determine the trigger context from the line content
///
/// The order of checks is important:
/// 1. Attribute context must be checked first (after '@')
/// 2. Proof tactic context (after 'by' in proof context)
/// 3. Using context (inside using [...])
/// 4. Tagged literal context (after tag#)
/// 5. Import context (because "use std::" ends with ':' but should be import, not type)
/// 6. Type context (after ':' in type annotations, or after '->')
/// 7. Member access (after '.')
/// 8. Default: expression context
pub fn get_trigger_context(line: &str, character: u32) -> Option<TriggerContext> {
    let prefix = &line[..character.min(line.len() as u32) as usize];

    // Check for attribute context FIRST (after '@')
    if let Some(attr_context) = check_attribute_context(prefix) {
        return Some(attr_context);
    }

    // Check for proof tactic context (after 'by' keyword)
    if check_proof_tactic_context(prefix) {
        return Some(TriggerContext::ProofTactic);
    }

    // Check for using context (inside using [...])
    if check_using_context(prefix) {
        return Some(TriggerContext::UsingContext);
    }

    // Check for tagged literal context (after tag#)
    if check_tagged_literal_context(prefix) {
        return Some(TriggerContext::TaggedLiteral);
    }

    // Check for import context (takes priority over type context for "use path::")
    // This ensures "use std::" is recognized as Import, not Type
    if prefix.contains("use ") {
        return Some(TriggerContext::Import);
    }

    // Check for type context (after ':' or '->')
    // Only check if not already identified as import
    if prefix.ends_with(':') || prefix.contains("->") {
        return Some(TriggerContext::Type);
    }

    // Check for member access (after '.')
    if prefix.ends_with('.') {
        return Some(TriggerContext::Member);
    }

    Some(TriggerContext::Expression)
}

/// Check if we're in a proof tactic context (after 'by' keyword)
fn check_proof_tactic_context(prefix: &str) -> bool {
    let trimmed = prefix.trim();
    // Match patterns like "proof by ", "by ", after "have ... by "
    trimmed.ends_with(" by") || trimmed.ends_with(" by ")
}

/// Check if we're inside a using clause (using [...])
fn check_using_context(prefix: &str) -> bool {
    // Check if we have "using [" without a closing "]"
    if let Some(using_pos) = prefix.rfind("using") {
        let after_using = &prefix[using_pos..];
        if let Some(bracket_pos) = after_using.find('[') {
            let after_bracket = &after_using[bracket_pos..];
            // We're inside the brackets if there's no closing bracket
            return !after_bracket.contains(']');
        }
    }
    false
}

/// Check if we're in a tagged literal context (e.g., sql#, rx#)
fn check_tagged_literal_context(prefix: &str) -> bool {
    // Check if the last non-whitespace sequence ends with '#'
    let trimmed = prefix.trim_end();
    if let Some(before_hash) = trimmed.strip_suffix('#') {
        // Verify there's an identifier before the #
        if let Some(last_char) = before_hash.chars().last() {
            return last_char.is_alphanumeric() || last_char == '_';
        }
    }
    false
}

/// Check if we're in an attribute context (after '@')
///
/// Returns the partial attribute name if any characters have been typed after '@'.
/// Also tries to infer the attribute target from surrounding context.
fn check_attribute_context(prefix: &str) -> Option<TriggerContext> {
    // Find the last '@' in the prefix
    let at_pos = prefix.rfind('@')?;

    // Get what's after the '@'
    let after_at = &prefix[at_pos + 1..];

    // If there's a '(' or ')' after '@', we're past the attribute name
    // (either in arguments or past the attribute entirely)
    if after_at.contains('(') || after_at.contains(')') {
        return None;
    }

    // Check if everything after '@' is a valid identifier prefix
    let partial_name = if after_at.is_empty() {
        None
    } else if after_at.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Some(after_at.to_string())
    } else {
        // Invalid characters after '@', not an attribute context
        return None;
    };

    // Infer the target from context
    let target = infer_attribute_target(prefix, at_pos);

    Some(TriggerContext::Attribute {
        partial_name,
        target,
    })
}

/// Infer the attribute target from the surrounding context
///
/// This is a heuristic based on common patterns:
/// - Lines starting with '@' at the beginning are likely module-level
/// - Lines before 'fn' are function attributes
/// - Lines before 'type' are type attributes
/// - Lines inside blocks might be statement/expression attributes
fn infer_attribute_target(prefix: &str, at_pos: usize) -> AttributeTarget {
    let before_at = &prefix[..at_pos];
    let trimmed = before_at.trim();

    // If there's content before '@' on this line, check what follows
    // For now, use a broad target set since we can't see the next line

    // Check if we're at the start of a line (just whitespace before '@')
    if trimmed.is_empty() {
        // Could be any item-level attribute
        // Return a broad set of common targets
        return AttributeTarget::Function
            | AttributeTarget::Type
            | AttributeTarget::Module
            | AttributeTarget::Protocol
            | AttributeTarget::Impl;
    }

    // If there's code before '@', we might be in an expression/statement context
    if trimmed.ends_with('{') || trimmed.ends_with(';') || trimmed.ends_with(',') {
        // Inside a block or after a statement - could be for statements/expressions
        return AttributeTarget::Stmt | AttributeTarget::Expr | AttributeTarget::Field;
    }

    // If we're after 'match' we might be in a match arm context
    if trimmed.contains("match") || trimmed.ends_with("=>") {
        return AttributeTarget::MatchArm;
    }

    // Default: allow all common targets
    AttributeTarget::All
}

/// Add keyword completions
pub fn add_keyword_completions(completions: &mut List<CompletionItem>) {
    for keyword in KEYWORDS {
        completions.push(CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("keyword".to_string()),
            documentation: Some(Documentation::String(format!("Verum keyword: {}", keyword))),
            insert_text: Some(keyword.to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        });
    }
}

/// Add type completions
pub fn add_type_completions(completions: &mut List<CompletionItem>) {
    for type_name in BUILTIN_TYPES {
        let documentation = match *type_name {
            "Int" => "Arbitrary-precision integer",
            "Float" => "64-bit floating point number",
            "Bool" => "Boolean type (true or false)",
            "Char" => "Unicode character",
            "Text" => "UTF-8 string",
            "Unit" => "Unit type (empty tuple)",
            "List" => "Dynamic array (heap-allocated)",
            "Map" => "Hash map",
            "Set" => "Hash set",
            "Maybe" => "Optional value (Some or None)",
            "Result" => "Result type (Ok or Err)",
            "Heap" => "Heap-allocated reference",
            "Shared" => "Thread-safe shared reference",
            _ => "Built-in type",
        };

        completions.push(CompletionItem {
            label: type_name.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some("type".to_string()),
            documentation: Some(Documentation::String(documentation.to_string())),
            insert_text: Some(type_name.to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        });
    }
}

/// Add completions from the module.
///
/// Function and type items carry a lightweight `data` payload so that the
/// resolve handler can lazily look up full documentation and type info from
/// the symbol table without paying that cost for every candidate in the
/// initial list.
fn add_module_completions(completions: &mut List<CompletionItem>, module: &verum_ast::Module) {
    use verum_ast::ItemKind;
    use verum_ast::decl::TypeDeclBody;

    for item in module.items.iter() {
        match &item.kind {
            ItemKind::Function(func) => {
                let name = func.name.as_str();
                let params_str = format_params(&func.params);
                let return_type = func
                    .return_type
                    .as_ref()
                    .map(format_type)
                    .unwrap_or_else(|| "()".to_string());
                let signature = format!("fn {}({}) -> {}", name, params_str, return_type);

                let mut item = CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some(signature),
                    // Documentation deferred to resolve handler
                    documentation: None,
                    insert_text: Some(format!("{}($1)", name)),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                };
                // Store name for resolve — URI is filled by the backend
                // which has access to the document URI.
                item.data = Some(serde_json::json!({ "name": name }));
                completions.push(item);
            }
            ItemKind::Type(type_decl) => {
                let name = type_decl.name.as_str();
                let detail = match &type_decl.body {
                    TypeDeclBody::Alias(ty) => {
                        format!("type {} = {}", name, format_type(ty))
                    }
                    TypeDeclBody::Record(_) => format!("type {} (record)", name),
                    TypeDeclBody::Variant(_) => format!("type {} (variant)", name),
                    TypeDeclBody::Newtype(ty) => {
                        format!("type {} = {}", name, format_type(ty))
                    }
                    _ => format!("type {}", name),
                };

                let mut item = CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(detail),
                    documentation: None,
                    insert_text: Some(name.to_string()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                };
                item.data = Some(serde_json::json!({ "name": name }));
                completions.push(item);

                // Add variants for variant types
                if let TypeDeclBody::Variant(variants) = &type_decl.body {
                    for variant in variants {
                        let variant_label =
                            format!("{}::{}", name, variant.name.as_str());
                        completions.push(CompletionItem {
                            label: variant_label.clone(),
                            kind: Some(CompletionItemKind::ENUM_MEMBER),
                            detail: Some(format!("variant of {}", name)),
                            insert_text: Some(variant_label),
                            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                            ..Default::default()
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Get the receiver name before a dot for member access
fn get_receiver_name(line: &str, character: u32) -> Option<String> {
    let prefix = &line[..character.min(line.len() as u32) as usize];

    // Find the dot and get the identifier before it
    if let Some(dot_pos) = prefix.rfind('.') {
        let before_dot = &prefix[..dot_pos];

        // Extract the identifier (going backwards from the dot)
        let mut start = before_dot.len();
        for (i, ch) in before_dot.chars().rev().enumerate() {
            if !ch.is_alphanumeric() && ch != '_' {
                start = before_dot.len() - i;
                break;
            }
            if i == before_dot.len() - 1 {
                start = 0;
            }
        }

        let receiver = &before_dot[start..];
        if !receiver.is_empty() {
            return Some(receiver.to_string());
        }
    }
    None
}

/// Add member completions for a receiver type
fn add_member_completions(
    completions: &mut List<CompletionItem>,
    document: &DocumentState,
    receiver: &str,
) {
    use verum_ast::decl::TypeDeclBody;

    // First, try to find the receiver in the symbol table
    if let Some(symbol) = document.get_symbol(receiver) {
        // If it's a type, get its fields/variants
        if symbol.kind == SymbolKind::Type
            && let Some(module) = &document.module
        {
            for item in module.items.iter() {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind
                    && type_decl.name.as_str() == receiver
                {
                    // Add fields or variants as completions
                    match &type_decl.body {
                        TypeDeclBody::Record(fields) => {
                            for field in fields {
                                let type_str = format_type(&field.ty);
                                completions.push(CompletionItem {
                                    label: field.name.as_str().to_string(),
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: Some(type_str.clone()),
                                    documentation: Some(Documentation::String(format!(
                                        "Field `{}` of type `{}`",
                                        field.name.as_str(),
                                        type_str
                                    ))),
                                    insert_text: Some(field.name.as_str().to_string()),
                                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                                    ..Default::default()
                                });
                            }
                        }
                        TypeDeclBody::Variant(variants) => {
                            for variant in variants {
                                completions.push(CompletionItem {
                                    label: variant.name.as_str().to_string(),
                                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                                    detail: Some(format!("variant of {}", receiver)),
                                    insert_text: Some(variant.name.as_str().to_string()),
                                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                                    ..Default::default()
                                });
                            }
                        }
                        _ => {}
                    }
                    break;
                }
            }
        }
    }

    // Also look for fields prefixed with the receiver name (for record access patterns)
    for (name, _symbol) in document.symbols.iter() {
        if name.starts_with(&format!("{}.", receiver)) {
            let field_name = name.strip_prefix(&format!("{}.", receiver)).unwrap_or(name);
            completions.push(CompletionItem {
                label: field_name.to_string(),
                kind: Some(CompletionItemKind::FIELD),
                detail: Some("field".to_string()),
                insert_text: Some(field_name.to_string()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            });
        }
    }

    // Add common methods available on all types
    add_common_methods(completions);
}

/// Add common methods available on most types
fn add_common_methods(completions: &mut List<CompletionItem>) {
    let common_methods = [
        (
            "clone",
            "fn clone(&self) -> Self",
            "Creates a copy of the value",
        ),
        (
            "to_text",
            "fn to_text(&self) -> Text",
            "Converts the value to a Text string",
        ),
        (
            "debug",
            "fn debug(&self) -> Text",
            "Returns a debug representation",
        ),
    ];

    for (name, signature, doc) in common_methods {
        completions.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some(signature.to_string()),
            documentation: Some(Documentation::String(doc.to_string())),
            insert_text: Some(format!("{}()", name)),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        });
    }
}

/// Add import completions for use statements
fn add_import_completions(
    completions: &mut List<CompletionItem>,
    document: &DocumentState,
    line: &str,
) {
    // Standard library modules
    let std_modules = [
        ("stdlib", "Verum standard library"),
        ("core.io", "I/O operations"),
        ("core.fs", "File system operations"),
        ("core.net", "Networking primitives"),
        ("core.sync", "Synchronization primitives"),
        ("core.async", "Async runtime and utilities"),
        ("core.collections", "Additional collections"),
        ("core.math", "Mathematical functions"),
    ];

    // Core types that can be imported from verum_common
    let core_types = [
        ("List", "Dynamic array type"),
        ("Text", "UTF-8 string type"),
        ("Map", "Hash map type"),
        ("Set", "Hash set type"),
        ("Maybe", "Optional value type (Some/None)"),
        ("Result", "Result type (Ok/Err)"),
        ("Heap", "CBGR-managed heap allocation"),
        ("Shared", "Thread-safe shared reference"),
    ];

    // Determine what's already typed to filter completions
    let prefix = line.trim_start().strip_prefix("use ").unwrap_or("");

    // If starting with stdlib, show submodules
    if prefix.starts_with("core.") {
        let remaining = prefix.strip_prefix("core.").unwrap_or("");

        if remaining.is_empty() {
            // Show core types
            for (name, doc) in core_types {
                completions.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(doc.to_string()),
                    insert_text: Some(name.to_string()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }
    } else {
        // Show available modules
        for (module, doc) in std_modules {
            completions.push(CompletionItem {
                label: module.to_string(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some(doc.to_string()),
                insert_text: Some(module.to_string()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            });
        }
    }

    // Also add modules defined in the current document
    for symbol in document.get_symbols_by_kind(SymbolKind::Module) {
        completions.push(CompletionItem {
            label: symbol.name.clone(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("local module".to_string()),
            insert_text: Some(symbol.name.clone()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        });
    }
}

// =============================================================================
// ATTRIBUTE COMPLETION
// =============================================================================

/// Add attribute completions from the global attribute registry
///
/// Attributes are grouped by category and filtered by the inferred target context.
/// Each completion item includes documentation, argument hints, and category information.
fn add_attribute_completions(
    completions: &mut List<CompletionItem>,
    partial_name: Option<&str>,
    target: AttributeTarget,
) {
    let reg = registry();

    // Get all categories and sort them for consistent ordering
    let mut categories: Vec<AttributeCategory> = reg.categories().into_iter().collect();
    categories.sort_by_key(|c| c.display_name());

    // Track sort order by category for grouping in IDE
    let mut sort_index = 0u32;

    for category in categories {
        let attrs = reg.by_category(category);

        for meta in attrs {
            // Filter by partial name if provided
            if let Some(prefix) = partial_name {
                if !meta.name.as_str().starts_with(prefix) {
                    continue;
                }
            }

            // Filter by target - only show attributes valid for the inferred context
            // If target is All, show everything; otherwise intersect
            if target != AttributeTarget::All && !meta.targets.intersects(target) {
                continue;
            }

            let completion = create_attribute_completion(meta, category, sort_index);
            completions.push(completion);
        }

        sort_index += 100; // Leave room for items within category
    }
}

/// Create a completion item for an attribute
fn create_attribute_completion(
    meta: &AttributeMetadata,
    category: AttributeCategory,
    sort_index: u32,
) -> CompletionItem {
    // Build the insert text with argument placeholders
    let insert_text = build_attribute_insert_text(meta);
    let insert_format = if insert_text.contains('$') {
        InsertTextFormat::SNIPPET
    } else {
        InsertTextFormat::PLAIN_TEXT
    };

    // Build documentation
    let documentation = build_attribute_documentation(meta);

    // Build detail string showing category and argument spec
    let detail = format!(
        "[{}] {}",
        category.display_name(),
        format_arg_spec(&meta.args)
    );

    // Create sort text to group by category
    let sort_text = format!("{:04}_{}", sort_index, meta.name.as_str());

    // Label tags for deprecated attributes
    let tags = if meta.is_deprecated() {
        Some(vec![CompletionItemTag::DEPRECATED])
    } else {
        None
    };

    CompletionItem {
        label: meta.name.as_str().to_string(),
        kind: Some(CompletionItemKind::KEYWORD), // LSP uses KEYWORD for decorators/attributes
        detail: Some(detail),
        documentation: Some(documentation),
        insert_text: Some(insert_text),
        insert_text_format: Some(insert_format),
        sort_text: Some(sort_text),
        tags,
        // Filter text without '@' so typing '@inl' matches 'inline'
        filter_text: Some(meta.name.as_str().to_string()),
        ..Default::default()
    }
}

/// Build the insert text for an attribute, with snippet placeholders for arguments.
///
/// When the attribute name matches a known enum-valued argument (see
/// `known_attr_choices`), emits an LSP `${1|a,b,c|}` choice snippet so
/// editors offer the allowed values inline. Other attributes fall back
/// to a typed placeholder.
fn build_attribute_insert_text(meta: &AttributeMetadata) -> String {
    if let Some(choices) = known_attr_choices(meta.name.as_str()) {
        return format!("{}(${{1|{}|}})", meta.name.as_str(), choices.join(","));
    }
    match &meta.args {
        ArgSpec::None => {
            // No arguments, just the name
            meta.name.as_str().to_string()
        }
        ArgSpec::Required(arg_type) => {
            // Required argument - add placeholder
            format!("{}(${{1:{}}})", meta.name.as_str(), arg_type.description())
        }
        ArgSpec::Optional(_) => {
            // Optional argument - just the name, user can add parens if needed
            meta.name.as_str().to_string()
        }
        ArgSpec::Variadic(_) => {
            // Variadic - add parens with placeholder
            format!("{}(${{1:...}})", meta.name.as_str())
        }
        ArgSpec::Named(specs) => {
            // Named arguments - show all required ones
            let required: Vec<_> = specs.iter().filter(|s| s.required).collect();
            if required.is_empty() {
                meta.name.as_str().to_string()
            } else {
                let args: Vec<String> = required
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("{} = ${{{}:{}}}", s.name, i + 1, s.ty.description()))
                    .collect();
                format!("{}({})", meta.name.as_str(), args.join(", "))
            }
        }
        ArgSpec::Mixed { positional, named } => {
            let mut parts = Vec::new();
            let mut idx = 1;

            // Add positional args (positional is Maybe<ArgType>)
            if let verum_common::Maybe::Some(arg) = positional {
                parts.push(format!("${{{}:{}}}", idx, arg.description()));
                idx += 1;
            }

            // Add required named args
            for spec in named.iter().filter(|s| s.required) {
                parts.push(format!(
                    "{} = ${{{}:{}}}",
                    spec.name,
                    idx,
                    spec.ty.description()
                ));
                idx += 1;
            }

            if parts.is_empty() {
                meta.name.as_str().to_string()
            } else {
                format!("{}({})", meta.name.as_str(), parts.join(", "))
            }
        }
        ArgSpec::Either {
            positional,
            named: _,
        } => {
            // Prefer positional form
            format!(
                "{}(${{1:{}}})",
                meta.name.as_str(),
                positional.description()
            )
        }
        ArgSpec::Custom { description: _ } => {
            // Custom validation - just add parens
            format!("{}(${{1:...}})", meta.name.as_str())
        }
    }
}

/// Known enum-valued arguments for specific attributes. Used by
/// `build_attribute_insert_text` to emit LSP choice snippets so that
/// typing `@inline(` offers `always`, `never`, `hint`, `release` inline
/// rather than the generic placeholder "identifier".
///
/// Values mirror `verum_vbc::codegen::extract_optimization_hints` and
/// `extract_type_layout_hints`; keep in sync when adding/removing
/// supported argument names on those attributes.
fn known_attr_choices(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "inline" => Some(&["always", "never", "hint", "release"]),
        "repr" => Some(&["C", "packed", "transparent", "cache_optimal"]),
        "optimize" => Some(&["none", "size", "speed", "balanced"]),
        "device" => Some(&["cpu", "gpu"]),
        _ => None,
    }
}

/// Build documentation for an attribute completion
fn build_attribute_documentation(meta: &AttributeMetadata) -> Documentation {
    let mut doc = String::new();

    // Main documentation
    doc.push_str(&format!("**@{}**\n\n", meta.name.as_str()));
    doc.push_str(meta.doc.as_str());
    doc.push_str("\n\n");

    // Arguments section
    if !matches!(meta.args, ArgSpec::None) {
        doc.push_str("**Arguments:**\n");
        doc.push_str(&format_arg_spec_detailed(&meta.args));
        doc.push('\n');
    }

    // Valid targets
    doc.push_str("**Valid on:** ");
    doc.push_str(&format_targets(meta.targets));
    doc.push_str("\n\n");

    // Deprecation notice
    if let verum_common::Maybe::Some(notice) = &meta.deprecated {
        doc.push_str(&format!(
            "> **Deprecated** since {}: {}\n\n",
            notice.since,
            notice
                .reason
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("Use an alternative")
        ));
    }

    // Stability notice
    if meta.stability.requires_feature() {
        doc.push_str(&format!(
            "> **{}** - This attribute may change in future versions.\n\n",
            meta.stability.display_name()
        ));
    }

    // Extended documentation if available
    if let verum_common::Maybe::Some(extended) = &meta.doc_extended {
        doc.push_str("---\n\n");
        doc.push_str(extended.as_str());
    }

    Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: doc,
    })
}

/// Format argument specification for detail line
fn format_arg_spec(args: &ArgSpec) -> String {
    match args {
        ArgSpec::None => "no arguments".to_string(),
        ArgSpec::Required(t) => format!("({})", format_arg_type(t)),
        ArgSpec::Optional(t) => format!("({}?)", format_arg_type(t)),
        ArgSpec::Variadic(t) => format!("({}...)", format_arg_type(t)),
        ArgSpec::Named(specs) => {
            let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
            format!("({})", names.join(", "))
        }
        ArgSpec::Mixed { .. } => "(mixed args)".to_string(),
        ArgSpec::Either { .. } => "(positional or named)".to_string(),
        ArgSpec::Custom { description } => format!("({})", description),
    }
}

/// Format argument type for display
fn format_arg_type(t: &ArgType) -> String {
    match t {
        ArgType::Ident => "identifier".to_string(),
        ArgType::String => "string".to_string(),
        ArgType::Int => "integer".to_string(),
        ArgType::UInt => "unsigned".to_string(),
        ArgType::Float => "float".to_string(),
        ArgType::Bool => "bool".to_string(),
        ArgType::Expr => "expression".to_string(),
        ArgType::Path => "path".to_string(),
        ArgType::Type => "type".to_string(),
        ArgType::Duration => "duration".to_string(),
        ArgType::Size => "size".to_string(),
    }
}

/// Format argument specification with detailed descriptions
fn format_arg_spec_detailed(args: &ArgSpec) -> String {
    match args {
        ArgSpec::None => String::new(),
        ArgSpec::Required(t) => format!("- Required: {}\n", format_arg_type(t)),
        ArgSpec::Optional(t) => format!("- Optional: {}\n", format_arg_type(t)),
        ArgSpec::Variadic(t) => format!("- Variadic: {}...\n", format_arg_type(t)),
        ArgSpec::Named(specs) => {
            let mut s = String::new();
            for spec in specs.iter() {
                let req = if spec.required { " (required)" } else { "" };
                s.push_str(&format!(
                    "- `{}`: {}{}\n",
                    spec.name,
                    format_arg_type(&spec.ty),
                    req
                ));
            }
            s
        }
        ArgSpec::Mixed { positional, named } => {
            let mut s = String::new();
            if let verum_common::Maybe::Some(t) = positional {
                s.push_str(&format!("- Position 1: {}\n", format_arg_type(t)));
            }
            for spec in named.iter() {
                let req = if spec.required { " (required)" } else { "" };
                s.push_str(&format!(
                    "- `{}`: {}{}\n",
                    spec.name,
                    format_arg_type(&spec.ty),
                    req
                ));
            }
            s
        }
        ArgSpec::Either { positional, named } => {
            let mut s = String::from("Either:\n");
            s.push_str(&format!("- Positional: {}\n", format_arg_type(positional)));
            s.push_str("- Or named:\n");
            for spec in named.iter() {
                let req = if spec.required { " (required)" } else { "" };
                s.push_str(&format!(
                    "  - `{}`: {}{}\n",
                    spec.name,
                    format_arg_type(&spec.ty),
                    req
                ));
            }
            s
        }
        ArgSpec::Custom { description } => format!("Custom: {}\n", description),
    }
}

/// Format attribute targets for display
fn format_targets(targets: AttributeTarget) -> String {
    let mut parts = Vec::new();

    if targets.contains(AttributeTarget::Function) {
        parts.push("functions");
    }
    if targets.contains(AttributeTarget::Type) {
        parts.push("types");
    }
    if targets.contains(AttributeTarget::Field) {
        parts.push("fields");
    }
    if targets.contains(AttributeTarget::Variant) {
        parts.push("variants");
    }
    if targets.contains(AttributeTarget::Param) {
        parts.push("parameters");
    }
    if targets.contains(AttributeTarget::Module) {
        parts.push("modules");
    }
    if targets.contains(AttributeTarget::Protocol) {
        parts.push("protocols");
    }
    if targets.contains(AttributeTarget::Impl) {
        parts.push("implementations");
    }
    if targets.contains(AttributeTarget::Stmt) {
        parts.push("statements");
    }
    if targets.contains(AttributeTarget::Expr) {
        parts.push("expressions");
    }
    if targets.contains(AttributeTarget::MatchArm) {
        parts.push("match arms");
    }
    if targets.contains(AttributeTarget::Loop) {
        parts.push("loops");
    }
    if targets.contains(AttributeTarget::Static) {
        parts.push("statics");
    }
    if targets.contains(AttributeTarget::Const) {
        parts.push("constants");
    }

    if parts.is_empty() || targets == AttributeTarget::All {
        return "all items".to_string();
    }

    parts.join(", ")
}

// =============================================================================
// PROOF TACTIC COMPLETION
// =============================================================================

/// Tactics available in proof contexts
const TACTICS: &[(&str, &str)] = &[
    ("auto", "Automatic proof search - attempts to discharge goal using available hypotheses and lemmas"),
    ("simp", "Simplification - applies simplification lemmas repeatedly to simplify the goal"),
    ("ring", "Ring equation solver - proves algebraic equations using ring axioms"),
    ("field", "Field theory solver - proves equations in field structures"),
    ("omega", "Linear arithmetic solver - solves linear arithmetic over integers"),
    ("blast", "DPLL-based proof search - aggressive automated proving"),
    ("smt", "SMT solver invocation - sends the goal to an external SMT solver (Z3/CVC5)"),
    ("induction", "Proof by induction - applies structural induction on a term"),
    ("cases", "Case analysis - splits goal into cases based on constructors"),
    ("trivial", "Trivial proof - solves reflexivity and simple equalities"),
    ("assumption", "Proof by assumption - searches hypotheses for exact match"),
    ("contradiction", "Proof by contradiction - derives False from hypotheses"),
    ("rewrite", "Rewrite using equality - replaces terms using an equality lemma"),
    ("apply", "Apply lemma/theorem - matches goal with the conclusion of a lemma"),
    ("exact", "Exact proof term - provides an explicit proof term for the goal"),
    ("unfold", "Unfold definition - expands a definition in the goal"),
    ("intro", "Introduction - introduces assumptions or forall-bound variables"),
    ("split", "Split conjunction - splits a goal A ∧ B into two subgoals"),
    ("left", "Prove left disjunct - proves A when goal is A ∨ B"),
    ("right", "Prove right disjunct - proves B when goal is A ∨ B"),
    ("exists", "Witness existential - provides a witness for ∃x.P(x)"),
];

/// Add proof tactic completions
fn add_tactic_completions(completions: &mut List<CompletionItem>) {
    for (name, doc) in TACTICS {
        completions.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("tactic".to_string()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("**{}**\n\n{}", name, doc),
            })),
            insert_text: Some(name.to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        });
    }
}

// =============================================================================
// CONTEXT COMPLETION
// =============================================================================

/// Common context types used in using clauses
const COMMON_CONTEXTS: &[(&str, &str)] = &[
    ("Database", "Database connection context for SQL operations"),
    ("Logger", "Logging context for structured logging"),
    ("Config", "Application configuration context"),
    ("Auth", "Authentication/authorization context"),
    ("Http", "HTTP client context for making requests"),
    ("Cache", "Caching context for memoization"),
    ("Metrics", "Metrics collection context"),
    ("Tracer", "Distributed tracing context"),
    ("FileSystem", "File system access context"),
    ("Network", "Network access context"),
    ("Clock", "Time/clock context for testability"),
    ("Random", "Randomness context for testability"),
    ("Allocator", "Custom memory allocator context"),
];

/// Add context type completions for using clause
fn add_context_completions(completions: &mut List<CompletionItem>) {
    for (name, doc) in COMMON_CONTEXTS {
        completions.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::INTERFACE),
            detail: Some("context".to_string()),
            documentation: Some(Documentation::String(doc.to_string())),
            insert_text: Some(name.to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        });
    }
}

// =============================================================================
// TAGGED LITERAL COMPLETION
// =============================================================================

/// Tagged literal format tags and their descriptions
const FORMAT_TAGS: &[(&str, &str, &str)] = &[
    // Data formats
    ("json", "JSON data", "json#\"{ \"key\": \"value\" }\""),
    ("yaml", "YAML data", "yaml#\"key: value\""),
    ("toml", "TOML configuration", "toml#\"[section]\\nkey = \\\"value\\\"\""),
    ("xml", "XML document", "xml#\"<root><child/></root>\""),
    ("html", "HTML markup", "html#\"<div class=\\\"foo\\\">content</div>\""),
    ("css", "CSS styles", "css#\".class { color: red; }\""),

    // Query languages
    ("sql", "SQL query (compile-time validated)", "sql#\"SELECT * FROM users WHERE id = ?\""),
    ("gql", "GraphQL query", "gql#\"{ user(id: 1) { name } }\""),
    ("cypher", "Cypher query (Neo4j)", "cypher#\"MATCH (n:Person) RETURN n\""),

    // Pattern matching
    ("rx", "Regular expression (compile-time validated)", "rx#\"[a-zA-Z][a-zA-Z0-9_]*\""),
    ("regex", "Regular expression (alias)", "regex#\"\\\\d{3}-\\\\d{4}\""),
    ("glob", "Glob pattern", "glob#\"**/*.rs\""),

    // Identifiers and URIs
    ("url", "URL (validated)", "url#\"https://example.com/path?query=value\""),
    ("uri", "URI (validated)", "uri#\"mailto:user@example.com\""),
    ("email", "Email address (validated)", "email#\"user@example.com\""),
    ("path", "File path", "path#\"/usr/local/bin\""),
    ("uuid", "UUID (validated)", "uuid#\"550e8400-e29b-41d4-a716-446655440000\""),

    // Time formats
    ("d", "ISO 8601 datetime", "d#\"2024-01-21T08:30:00Z\""),
    ("date", "Date only", "date#\"2024-01-21\""),
    ("time", "Time only", "time#\"08:30:00\""),
    ("dur", "Duration", "dur#\"5m30s\""),
    ("duration", "Duration (alias)", "duration#\"1h30m\""),
    ("cron", "Cron expression", "cron#\"0 0 * * *\""),

    // Encoding formats
    ("b64", "Base64 encoded", "b64#\"SGVsbG8gV29ybGQ=\""),
    ("base64", "Base64 encoded (alias)", "base64#\"SGVsbG8=\""),
    ("hex", "Hexadecimal", "hex#\"deadbeef\""),

    // Semantic strings
    ("semver", "Semantic version", "semver#\"1.2.3-beta.1\""),
    ("ipv4", "IPv4 address", "ipv4#\"192.168.1.1\""),
    ("ipv6", "IPv6 address", "ipv6#\"::1\""),
    ("jwt", "JWT token", "jwt#\"eyJhbGciOiJIUzI1NiJ9...\""),
];

/// Add tagged literal format completions
fn add_tagged_literal_completions(completions: &mut List<CompletionItem>) {
    for (tag, description, example) in FORMAT_TAGS {
        completions.push(CompletionItem {
            label: format!("{}#", tag),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some(description.to_string()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!(
                    "**{}** - {}\n\n### Example\n```verum\n{}\n```",
                    tag, description, example
                ),
            })),
            insert_text: Some(format!("{}#\"${{1:}}\"", tag)),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        });
    }
}

// Formatting functions are now imported from ast_format module
