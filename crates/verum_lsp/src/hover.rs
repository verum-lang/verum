//! Hover support for showing type information and documentation
//!
//! When the user hovers over a symbol, we show:
//! - Type information
//! - Refinement constraints
//! - Documentation comments
//! - Function signatures
//! - CBGR cost information (for references and functions)
//! - Attribute documentation (for @attribute syntax)

use crate::ast_format::{format_function_signature, format_type_decl, get_builtin_info};
use crate::cbgr_hints::CbgrHintProvider;
use crate::document::{CbgrCostInfo, DocumentState, SymbolKind};
use tower_lsp::lsp_types::*;
use verum_ast::attr::{ArgSpec, ArgType, AttributeTarget};
use verum_types::attr::registry;

/// Generate hover information for a position in the document.
///
/// When `cbgr` is provided and the cursor is on a `&` sigil, a detailed
/// CBGR analysis (tier, escape, promotion availability) is returned instead
/// of the usual symbol hover.
pub fn hover_at_position(
    document: &DocumentState,
    cbgr: Option<&CbgrHintProvider>,
    position: Position,
) -> Option<Hover> {
    // Reference sigil? This runs first so `&panes[i]` gives CBGR info even
    // though `&` is not a word.
    if let Some(cbgr) = cbgr {
        if let Some(analysis) = cbgr.analyze_at_position(document, position) {
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: cbgr.format_hover_markdown(&analysis),
                }),
                range: Some(analysis.range),
            });
        }
    }

    // First, check if we're hovering over an attribute (after '@')
    if let Some(attr_hover) = get_attribute_hover(document, position) {
        return Some(attr_hover);
    }

    // Get the word at the position
    let word = document.word_at_position(position)?;

    // Try to get info from the symbol table (includes CBGR cost)
    if let Some(symbol) = document.get_symbol(&word) {
        let info = format_symbol_hover(document, symbol, &word);
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Try to find information about this symbol from AST
    if let Some(info) = get_symbol_info(document, &word) {
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Check if it's a proof keyword or tactic
    if let Some(info) = get_proof_keyword_hover(&word) {
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Check if it's a reference-related keyword
    if let Some(info) = get_reference_keyword_hover(&word) {
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info,
            }),
            range: None,
        });
    }

    // Check if it's a built-in type or keyword
    get_builtin_info(&word).map(|info| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: info,
        }),
        range: None,
    })
}

/// Format hover information for a symbol from the symbol table
fn format_symbol_hover(
    document: &DocumentState,
    symbol: &crate::document::SymbolInfo,
    name: &str,
) -> String {
    let mut hover = String::new();

    match symbol.kind {
        SymbolKind::Function => {
            // Get full function signature from AST
            if let Some(module) = &document.module {
                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Function(func) = &item.kind
                        && func.name.as_str() == name
                    {
                        hover.push_str(&format_function_hover(func));

                        // Add CBGR cost information
                        if let Some(ref cost) = symbol.cbgr_cost {
                            hover.push_str(&format_cbgr_cost(cost));
                        }

                        break;
                    }
                }
            }
        }
        SymbolKind::Type => {
            if let Some(module) = &document.module {
                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Type(type_decl) = &item.kind
                        && type_decl.name.as_str() == name
                    {
                        hover.push_str(&format_type_decl_hover(type_decl));
                        break;
                    }
                }
            }
        }
        SymbolKind::Variable | SymbolKind::Parameter => {
            hover.push_str(&format!("```verum\nlet {}\n```\n", name));
            if symbol.kind == SymbolKind::Parameter {
                hover.push_str("\n*function parameter*\n");
            }
        }
        SymbolKind::Field => {
            hover.push_str(&format!("```verum\n{}\n```\n", name));
            hover.push_str("\n*record field*\n");
        }
        SymbolKind::Variant => {
            hover.push_str(&format!("```verum\n{}\n```\n", name));
            hover.push_str("\n*enum variant*\n");
        }
        SymbolKind::Protocol => {
            if let Some(module) = &document.module {
                for item in module.items.iter() {
                    if let verum_ast::ItemKind::Protocol(protocol) = &item.kind
                        && protocol.name.as_str() == name
                    {
                        hover.push_str(&format_protocol_hover(protocol));
                        break;
                    }
                }
            }
        }
        SymbolKind::Module => {
            hover.push_str(&format!("```verum\nmod {}\n```\n", name));
            hover.push_str("\n*module*\n");
        }
        SymbolKind::Constant => {
            hover.push_str(&format!("```verum\nconst {}\n```\n", name));
            hover.push_str("\n*constant*\n");
        }
    }

    // Add documentation if available
    if let Some(ref docs) = symbol.docs {
        hover.push_str("\n---\n");
        hover.push_str(docs);
    }

    hover
}

/// Format CBGR cost information for display
fn format_cbgr_cost(cost: &CbgrCostInfo) -> String {
    let mut result = String::new();
    result.push_str("\n---\n");
    result.push_str("### CBGR Cost Analysis\n\n");

    let tier_badge = match cost.tier {
        0 => "**Tier 0** (CBGR-managed)",
        1 => "**Tier 1** (statically verified)",
        2 => "**Tier 2** (unsafe)",
        _ => "Unknown tier",
    };

    result.push_str(&format!("- Reference Tier: {}\n", tier_badge));
    result.push_str(&format!(
        "- Cost per dereference: ~{}ns\n",
        cost.deref_cost_ns
    ));
    result.push_str(&format!("- {}\n", cost.description));

    if cost.tier == 0 {
        result.push_str("\n> **Note**: CBGR-managed references provide runtime memory safety ");
        result.push_str("with automatic generation tracking. The ~15ns overhead ensures ");
        result.push_str("no use-after-free errors.\n");
    } else if cost.tier == 1 {
        result.push_str("\n> **Note**: Statically verified references have zero runtime overhead ");
        result.push_str("because the compiler proves safety at compile time.\n");
    } else if cost.tier == 2 {
        result.push_str("\n> **Warning**: Unsafe references bypass all safety checks. ");
        result.push_str("Use only when you can manually guarantee memory safety.\n");
    }

    result
}

/// Get information about a symbol from the module
fn get_symbol_info(document: &DocumentState, symbol: &str) -> Option<String> {
    let module = document.module.as_ref()?;

    use verum_ast::ItemKind;

    for item in module.items.iter() {
        match &item.kind {
            ItemKind::Function(func) if func.name.as_str() == symbol => {
                return Some(format_function_hover(func));
            }
            ItemKind::Type(type_decl) if type_decl.name.as_str() == symbol => {
                return Some(format_type_decl_hover(type_decl));
            }
            ItemKind::Protocol(protocol) if protocol.name.as_str() == symbol => {
                return Some(format_protocol_hover(protocol));
            }
            _ => {}
        }
    }

    None
}

/// Format hover information for a function
fn format_function_hover(func: &verum_ast::FunctionDecl) -> String {
    let signature = format_function_signature(func);
    let hover = format!("```verum\n{}\n```\n", signature);

    // Note: Verum AST doesn't have requires/ensures on FunctionDecl yet
    // These would be in attributes or a separate verification section

    hover
}

/// Format hover information for a type declaration
fn format_type_decl_hover(type_decl: &verum_ast::TypeDecl) -> String {
    let formatted = format_type_decl(type_decl);
    format!("```verum\n{}\n```\n", formatted)
}

/// Format hover information for a protocol
fn format_protocol_hover(protocol: &verum_ast::ProtocolDecl) -> String {
    let mut hover = format!("```verum\nprotocol {} {{\n", protocol.name.as_str());
    hover.push_str("    // protocol methods...\n");
    hover.push_str("}\n```\n");
    hover
}

// =============================================================================
// ATTRIBUTE HOVER SUPPORT
// =============================================================================

/// Get hover information for an attribute at the given position
///
/// Checks if the position is on an attribute (text starting with '@')
/// and returns documentation from the AttributeRegistry.
fn get_attribute_hover(document: &DocumentState, position: Position) -> Option<Hover> {
    let line = document.get_line(position.line)?;
    let offset = position.character as usize;

    // Find if we're inside an attribute name (after '@')
    let attr_name = extract_attribute_at_position(line, offset)?;

    // Look up the attribute in the registry
    let reg = registry();
    let meta = reg.get(&attr_name)?;

    // Build the hover documentation
    let doc = format_attribute_hover(meta);

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc,
        }),
        range: None,
    })
}

/// Extract attribute name if the position is within an attribute
///
/// Returns the attribute name (without '@') if the cursor is positioned
/// on an attribute like `@inline`, `@derive(...)`, etc.
fn extract_attribute_at_position(line: &str, offset: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();

    if offset >= chars.len() {
        return None;
    }

    // Find the '@' before our position
    let mut at_pos = None;
    for i in (0..=offset.min(chars.len() - 1)).rev() {
        if chars[i] == '@' {
            at_pos = Some(i);
            break;
        }
        // Stop if we hit something that can't be in an attribute name
        if !chars[i].is_alphanumeric() && chars[i] != '_' {
            break;
        }
    }

    let at_pos = at_pos?;

    // Extract the attribute name (identifier after '@')
    let mut name_end = at_pos + 1;
    while name_end < chars.len() && (chars[name_end].is_alphanumeric() || chars[name_end] == '_') {
        name_end += 1;
    }

    // Check if our offset is within the attribute name
    if offset < at_pos || offset > name_end {
        return None;
    }

    let name: String = chars[at_pos + 1..name_end].iter().collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Format hover documentation for an attribute
fn format_attribute_hover(meta: &verum_types::attr::AttributeMetadata) -> String {
    let mut doc = String::new();

    // Header with attribute name
    doc.push_str(&format!("# @{}\n\n", meta.name.as_str()));

    // Category badge
    doc.push_str(&format!(
        "**Category:** {}\n\n",
        meta.category.display_name()
    ));

    // Main documentation
    doc.push_str(meta.doc.as_str());
    doc.push_str("\n\n");

    // Syntax section
    doc.push_str("## Syntax\n\n");
    doc.push_str("```verum\n");
    doc.push_str(&format_attribute_syntax(meta));
    doc.push_str("\n```\n\n");

    // Arguments section
    if !matches!(meta.args, ArgSpec::None) {
        doc.push_str("## Arguments\n\n");
        doc.push_str(&format_arg_spec_hover(&meta.args));
        doc.push_str("\n");
    }

    // Valid targets section
    doc.push_str("## Valid On\n\n");
    doc.push_str(&format_targets_hover(meta.targets));
    doc.push_str("\n\n");

    // Conflicts section
    if !meta.conflicts_with.is_empty() {
        doc.push_str("## Conflicts With\n\n");
        for conflict in meta.conflicts_with.iter() {
            doc.push_str(&format!("- `@{}`\n", conflict));
        }
        doc.push_str("\n");
    }

    // Requirements section
    if !meta.requires.is_empty() {
        doc.push_str("## Requires\n\n");
        for req in meta.requires.iter() {
            doc.push_str(&format!("- `@{}`\n", req));
        }
        doc.push_str("\n");
    }

    // Deprecation notice
    if let verum_common::Maybe::Some(notice) = &meta.deprecated {
        doc.push_str("---\n\n");
        doc.push_str(&format!("> **Deprecated** since {}\n", notice.since));
        if let verum_common::Maybe::Some(reason) = &notice.reason {
            doc.push_str(&format!("> \n> {}\n", reason));
        }
        if let verum_common::Maybe::Some(replacement) = &notice.replacement {
            doc.push_str(&format!("> \n> Use `@{}` instead.\n", replacement));
        }
        if let verum_common::Maybe::Some(removal) = &notice.removal {
            doc.push_str(&format!("> \n> Will be removed in {}.\n", removal));
        }
        doc.push_str("\n");
    }

    // Stability notice
    if meta.stability.requires_feature() {
        doc.push_str("---\n\n");
        doc.push_str(&format!(
            "> **{}** - This attribute may change in future versions.\n",
            meta.stability.display_name()
        ));
        if let verum_common::Maybe::Some(feature) = &meta.feature_gate {
            doc.push_str(&format!("> \n> Requires feature: `{}`\n", feature));
        }
        doc.push_str("\n");
    }

    // Extended documentation
    if let verum_common::Maybe::Some(extended) = &meta.doc_extended {
        doc.push_str("---\n\n");
        doc.push_str(extended.as_str());
        doc.push_str("\n");
    }

    doc
}

/// Format the syntax example for an attribute
fn format_attribute_syntax(meta: &verum_types::attr::AttributeMetadata) -> String {
    match &meta.args {
        ArgSpec::None => format!("@{}", meta.name),
        ArgSpec::Required(t) => format!("@{}(<{}>)", meta.name, t.description()),
        ArgSpec::Optional(t) => format!("@{}\n@{}(<{}>)", meta.name, meta.name, t.description()),
        ArgSpec::Variadic(t) => format!("@{}(<{}>...)", meta.name, t.description()),
        ArgSpec::Named(specs) => {
            let args: Vec<String> = specs
                .iter()
                .map(|s| {
                    if s.required {
                        format!("{} = <{}>", s.name, s.ty.description())
                    } else {
                        format!("{} = <{}>?", s.name, s.ty.description())
                    }
                })
                .collect();
            format!("@{}({})", meta.name, args.join(", "))
        }
        ArgSpec::Mixed { positional, named } => {
            let mut parts = Vec::new();
            if let verum_common::Maybe::Some(t) = positional {
                parts.push(format!("<{}>", t.description()));
            }
            for s in named.iter() {
                if s.required {
                    parts.push(format!("{} = <{}>", s.name, s.ty.description()));
                } else {
                    parts.push(format!("{} = <{}>?", s.name, s.ty.description()));
                }
            }
            format!("@{}({})", meta.name, parts.join(", "))
        }
        ArgSpec::Either { positional, named } => {
            let named_args: Vec<String> = named
                .iter()
                .map(|s| format!("{} = <{}>", s.name, s.ty.description()))
                .collect();
            format!(
                "@{}(<{}>)\n@{}({})",
                meta.name,
                positional.description(),
                meta.name,
                named_args.join(", ")
            )
        }
        ArgSpec::Custom { description } => format!("@{}({})", meta.name, description),
    }
}

/// Format argument specification for hover documentation
fn format_arg_spec_hover(args: &ArgSpec) -> String {
    match args {
        ArgSpec::None => String::new(),
        ArgSpec::Required(t) => format!(
            "- **Required**: {} - {}\n",
            t.description(),
            examples_for_type(t)
        ),
        ArgSpec::Optional(t) => format!(
            "- **Optional**: {} - {}\n",
            t.description(),
            examples_for_type(t)
        ),
        ArgSpec::Variadic(t) => format!(
            "- **Variadic**: One or more {} values - {}\n",
            t.description(),
            examples_for_type(t)
        ),
        ArgSpec::Named(specs) => {
            let mut s = String::new();
            for spec in specs.iter() {
                let req_str = if spec.required {
                    "(required)"
                } else {
                    "(optional)"
                };
                s.push_str(&format!(
                    "- `{}` {} - {} - {}\n",
                    spec.name,
                    req_str,
                    spec.ty.description(),
                    examples_for_type(&spec.ty)
                ));
                if let verum_common::Maybe::Some(doc) = &spec.doc {
                    s.push_str(&format!("  - {}\n", doc));
                }
                if let verum_common::Maybe::Some(default) = &spec.default {
                    s.push_str(&format!("  - Default: `{}`\n", default));
                }
            }
            s
        }
        ArgSpec::Mixed { positional, named } => {
            let mut s = String::new();
            if let verum_common::Maybe::Some(t) = positional {
                s.push_str(&format!(
                    "- **Positional** (optional): {} - {}\n",
                    t.description(),
                    examples_for_type(t)
                ));
            }
            for spec in named.iter() {
                let req_str = if spec.required {
                    "(required)"
                } else {
                    "(optional)"
                };
                s.push_str(&format!(
                    "- `{}` {} - {} - {}\n",
                    spec.name,
                    req_str,
                    spec.ty.description(),
                    examples_for_type(&spec.ty)
                ));
            }
            s
        }
        ArgSpec::Either { positional, named } => {
            let mut s = String::from("Either positional or named form:\n\n");
            s.push_str(&format!(
                "**Positional:** {} - {}\n\n",
                positional.description(),
                examples_for_type(positional)
            ));
            s.push_str("**Named:**\n");
            for spec in named.iter() {
                let req_str = if spec.required {
                    "(required)"
                } else {
                    "(optional)"
                };
                s.push_str(&format!(
                    "- `{}` {} - {}\n",
                    spec.name,
                    req_str,
                    spec.ty.description()
                ));
            }
            s
        }
        ArgSpec::Custom { description } => format!("Custom format: {}\n", description),
    }
}

/// Get example values for an argument type
fn examples_for_type(t: &ArgType) -> String {
    let examples = t.examples();
    if examples.is_empty() {
        String::new()
    } else if examples.len() == 1 {
        format!("e.g., `{}`", examples[0])
    } else {
        format!("e.g., `{}`, `{}`", examples[0], examples[1])
    }
}

/// Format valid targets for hover documentation
fn format_targets_hover(targets: AttributeTarget) -> String {
    let mut items = Vec::new();

    if targets.contains(AttributeTarget::Function) {
        items.push("- Functions (`fn`)");
    }
    if targets.contains(AttributeTarget::Type) {
        items.push("- Types (`type`)");
    }
    if targets.contains(AttributeTarget::Field) {
        items.push("- Record fields");
    }
    if targets.contains(AttributeTarget::Variant) {
        items.push("- Enum variants");
    }
    if targets.contains(AttributeTarget::Param) {
        items.push("- Function parameters");
    }
    if targets.contains(AttributeTarget::Module) {
        items.push("- Modules (`mod`)");
    }
    if targets.contains(AttributeTarget::Protocol) {
        items.push("- Protocols (`protocol`)");
    }
    if targets.contains(AttributeTarget::Impl) {
        items.push("- Implementation blocks (`implement`)");
    }
    if targets.contains(AttributeTarget::Stmt) {
        items.push("- Statements");
    }
    if targets.contains(AttributeTarget::Expr) {
        items.push("- Expressions");
    }
    if targets.contains(AttributeTarget::MatchArm) {
        items.push("- Match arms");
    }
    if targets.contains(AttributeTarget::Loop) {
        items.push("- Loops");
    }
    if targets.contains(AttributeTarget::Static) {
        items.push("- Static values");
    }
    if targets.contains(AttributeTarget::Const) {
        items.push("- Constants");
    }

    if items.is_empty() || targets == AttributeTarget::All {
        return "All items".to_string();
    }

    items.join("\n")
}

// =============================================================================
// PROOF CONSTRUCT HOVER SUPPORT
// =============================================================================

/// Get hover information for proof keywords (theorem, lemma, tactics, etc.)
pub fn get_proof_keyword_hover(word: &str) -> Option<String> {
    match word {
        // Declaration keywords
        "theorem" => Some("# `theorem`\n\n\
            Declares a theorem - a proven mathematical statement.\n\n\
            ### Syntax\n\
            ```verum\n\
            theorem name(params) -> T\n\
            requires pre_condition\n\
            ensures post_condition\n\
            proof by tactic\n\
            ```\n\n\
            ### Example\n\
            ```verum\n\
            theorem sqrt_positive(x: Float)\n\
                requires x >= 0.0\n\
                ensures result >= 0.0\n\
            proof by smt\n\
            ```".to_string()),
        "lemma" => Some("# `lemma`\n\n\
            Declares a lemma - a helper theorem used in proofs.\n\n\
            Lemmas are typically used to break down complex proofs into smaller, \
            reusable parts.\n\n\
            ### Syntax\n\
            ```verum\n\
            lemma name(params)\n\
                requires conditions\n\
                ensures result\n\
            proof by tactic\n\
            ```".to_string()),
        "axiom" => Some("# `axiom`\n\n\
            Declares an axiom - an assumed truth without proof.\n\n\
            ⚠️ **Warning**: Axioms are assumed true without verification. \
            Use sparingly and only for fundamental assumptions.\n\n\
            ### Syntax\n\
            ```verum\n\
            axiom name: Property\n\
            ```\n\n\
            ### Example\n\
            ```verum\n\
            axiom excluded_middle: forall P. P || !P\n\
            ```".to_string()),
        "corollary" => Some("# `corollary`\n\n\
            Declares a corollary - a result that follows directly from a theorem.\n\n\
            Corollaries are typically straightforward consequences of previously \
            proven theorems.".to_string()),

        // Proof structure
        "proof" => Some("# `proof`\n\n\
            Begins a proof block for a theorem or lemma.\n\n\
            ### Syntax\n\
            ```verum\n\
            proof by <tactic>\n\
            // or\n\
            proof {\n\
                // proof steps\n\
            }\n\
            ```".to_string()),
        "qed" => Some("# `qed`\n\n\
            Marks the end of a proof (\"quod erat demonstrandum\").\n\n\
            Indicates that the proof is complete.".to_string()),
        "by" => Some("# `by`\n\n\
            Specifies the tactic used to prove a goal.\n\n\
            ### Syntax\n\
            ```verum\n\
            proof by <tactic>\n\
            have P by <tactic>\n\
            ```".to_string()),

        // Proof helpers
        "have" => Some("# `have`\n\n\
            Introduces a local fact in a proof.\n\n\
            ### Syntax\n\
            ```verum\n\
            have fact: P by tactic\n\
            ```\n\n\
            The fact becomes available for subsequent proof steps.".to_string()),
        "show" => Some("# `show`\n\n\
            Declares what the proof is trying to show.\n\n\
            ### Syntax\n\
            ```verum\n\
            show P by tactic\n\
            ```".to_string()),
        "suffices" => Some("# `suffices`\n\n\
            Reduces the current goal to a sufficient condition.\n\n\
            ### Syntax\n\
            ```verum\n\
            suffices P by tactic\n\
            ```\n\n\
            If `P` can be proven and `P => Goal`, then the goal is proven.".to_string()),
        "obtain" => Some("# `obtain`\n\n\
            Destructs an existential statement to get a witness.\n\n\
            ### Syntax\n\
            ```verum\n\
            obtain x such that P from hypothesis\n\
            ```".to_string()),
        "calc" => Some("# `calc`\n\n\
            Begins a calculational proof chain.\n\n\
            ### Syntax\n\
            ```verum\n\
            calc {\n\
                a = b by tactic1\n\
                  = c by tactic2\n\
                  < d by tactic3\n\
            }\n\
            ```\n\n\
            Each step justifies the relation with the previous term.".to_string()),

        // Quantifiers
        "forall" => Some("# `forall`\n\n\
            Universal quantifier - \"for all\".\n\n\
            ### Syntax\n\
            ```verum\n\
            forall x: T. P(x)\n\
            ```\n\n\
            Asserts that property P holds for all values x of type T.".to_string()),
        "exists" => Some("# `exists`\n\n\
            Existential quantifier - \"there exists\".\n\n\
            ### Syntax\n\
            ```verum\n\
            exists x: T. P(x)\n\
            ```\n\n\
            Asserts that there is some value x of type T for which P holds.".to_string()),

        // Tactics
        "auto" => Some(hover_for_tactic("auto", "Automatic proof search",
            "Attempts to discharge the goal using available hypotheses and lemmas. \
            Combines simplification, assumption matching, and basic reasoning.")),
        "simp" => Some(hover_for_tactic("simp", "Simplification",
            "Applies simplification lemmas repeatedly to rewrite the goal into a simpler form. \
            Can be configured with `simp [lemma1, lemma2]` to use specific lemmas.")),
        "ring" => Some(hover_for_tactic("ring", "Ring equation solver",
            "Proves algebraic equations in ring structures using ring axioms. \
            Works on expressions involving `+`, `-`, `*` over integers, rationals, etc.")),
        "field" => Some(hover_for_tactic("field", "Field theory solver",
            "Proves equations in field structures. Handles division and extends ring tactics.")),
        "omega" => Some(hover_for_tactic("omega", "Linear arithmetic solver",
            "Solves linear arithmetic constraints over integers. \
            Handles equations and inequalities with `+`, `-`, `*` (by constants), and comparisons.")),
        "blast" => Some(hover_for_tactic("blast", "DPLL-based proof search",
            "Aggressive automated proving using DPLL algorithm. \
            Good for propositional goals and first-order logic problems.")),
        "smt" => Some(hover_for_tactic("smt", "SMT solver invocation",
            "Sends the goal to an external SMT solver (Z3 or CVC5). \
            Very powerful for complex arithmetic, arrays, and quantified formulas.")),
        "induction" => Some(hover_for_tactic("induction", "Proof by induction",
            "Applies structural induction on a term. \
            Use `induction on x` to specify the induction variable.")),
        "cases" => Some(hover_for_tactic("cases", "Case analysis",
            "Splits the goal into cases based on constructors or conditions. \
            Use `cases h` to case-split on a hypothesis or value.")),
        "trivial" => Some(hover_for_tactic("trivial", "Trivial proof",
            "Solves reflexivity and simple equalities like `x == x` or `true`.")),
        "assumption" => Some(hover_for_tactic("assumption", "Proof by assumption",
            "Searches hypotheses for an exact match with the goal.")),
        "contradiction" => Some(hover_for_tactic("contradiction", "Proof by contradiction",
            "Derives `False` from contradictory hypotheses.")),
        "rewrite" => Some(hover_for_tactic("rewrite", "Rewrite using equality",
            "Replaces terms in the goal using an equality lemma. \
            Use `rewrite h` where `h: a = b` to replace `a` with `b`.")),
        "apply" => Some(hover_for_tactic("apply", "Apply lemma/theorem",
            "Matches the goal with the conclusion of a lemma and generates subgoals for premises. \
            Use `apply lemma_name` to apply a specific lemma.")),
        "exact" => Some(hover_for_tactic("exact", "Exact proof term",
            "Provides an explicit proof term that exactly matches the goal.")),

        _ => None,
    }
}

/// Helper to format tactic hover information
fn hover_for_tactic(name: &str, short_desc: &str, long_desc: &str) -> String {
    format!(
        "# `{}`\n\n\
        **{}**\n\n\
        {}\n\n\
        ### Usage\n\
        ```verum\n\
        proof by {}\n\
        ```",
        name, short_desc, long_desc, name
    )
}

// =============================================================================
// REFERENCE TIER HOVER SUPPORT
// =============================================================================

/// Reference tier information for hover
pub enum RefTier {
    /// Tier 0: CBGR-managed (&T, &mut T)
    Managed,
    /// Tier 1: Compile-time verified (&checked T)
    Checked,
    /// Tier 2: Manual safety proof (&unsafe T)
    Unsafe,
}

/// Format hover information for a reference type with tier details
pub fn format_reference_type_hover(tier: RefTier, is_mut: bool, inner_type: &str) -> String {
    let (syntax, overhead, description, note) = match tier {
        RefTier::Managed => (
            if is_mut { "&mut" } else { "&" },
            "~15ns per dereference",
            "**CBGR-managed reference** (Tier 0)\n\n\
            Full CBGR protection with generation tracking. \
            Memory layout: ThinRef (16 bytes) or FatRef (24 bytes).",
            "> **Note**: CBGR-managed references provide runtime memory safety \
            with automatic generation tracking. The ~15ns overhead ensures \
            no use-after-free errors."
        ),
        RefTier::Checked => (
            if is_mut { "&checked mut" } else { "&checked" },
            "0ns (zero overhead)",
            "**Statically verified reference** (Tier 1)\n\n\
            The compiler has proven this reference is safe through escape analysis. \
            No runtime overhead.",
            "> **Note**: Statically verified references have zero runtime overhead \
            because the compiler proves safety at compile time."
        ),
        RefTier::Unsafe => (
            if is_mut { "&unsafe mut" } else { "&unsafe" },
            "0ns (no checks)",
            "**Unsafe reference** (Tier 2)\n\n\
            ⚠️ All safety checks are bypassed. You must manually guarantee memory safety.",
            "> **Warning**: Unsafe references bypass all safety checks. \
            Use only when you can manually guarantee memory safety."
        ),
    };

    format!(
        "```verum\n{} {}\n```\n\n{}\n\n\
        ### Performance\n\
        - **Overhead**: {}\n\n\
        ---\n\n{}",
        syntax, inner_type, description, overhead, note
    )
}

/// Get hover for reference-related keywords
pub fn get_reference_keyword_hover(word: &str) -> Option<String> {
    match word {
        "checked" => Some("# `checked`\n\n\
            Reference tier modifier for statically verified references.\n\n\
            ### Syntax\n\
            ```verum\n\
            &checked T      // immutable checked reference\n\
            &checked mut T  // mutable checked reference\n\
            ```\n\n\
            ### Properties\n\
            - **Overhead**: 0ns (compile-time verified)\n\
            - **Safety**: Compiler proves memory safety via escape analysis\n\
            - **Use case**: Hot paths where CBGR overhead is unacceptable\n\n\
            The compiler will reject `&checked` if it cannot prove safety.".to_string()),
        "unsafe" => Some("# `unsafe`\n\n\
            Reference tier modifier for unchecked references, or code block marker.\n\n\
            ### As Reference Modifier\n\
            ```verum\n\
            &unsafe T      // unchecked immutable reference\n\
            &unsafe mut T  // unchecked mutable reference\n\
            ```\n\n\
            ⚠️ **Warning**: Bypasses all safety checks. Only use when you can \
            manually prove memory safety.\n\n\
            ### As Block Marker\n\
            ```verum\n\
            unsafe {\n\
                // code with manual safety obligations\n\
            }\n\
            ```".to_string()),
        _ => None,
    }
}
