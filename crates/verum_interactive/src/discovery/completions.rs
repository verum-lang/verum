//! Smart completion system for interactive Verum programming

use std::collections::HashMap;

use super::index::DiscoveryIndex;
use super::docs::DocKind;

/// A completion item
#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// The text to insert
    pub label: String,
    /// Kind of completion
    pub kind: CompletionKind,
    /// Brief description
    pub detail: Option<String>,
    /// Documentation (shown in popup)
    pub documentation: Option<String>,
    /// Sort priority (lower is shown first)
    pub sort_priority: u8,
    /// Text to insert (if different from label)
    pub insert_text: Option<String>,
    /// Whether this is a snippet with placeholders
    pub is_snippet: bool,
}

/// Kind of completion item
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    /// Keyword (let, fn, type, etc.)
    Keyword,
    /// Type name
    Type,
    /// Protocol
    Protocol,
    /// Function
    Function,
    /// Method
    Method,
    /// Variable binding
    Variable,
    /// Field of a struct
    Field,
    /// Module name
    Module,
    /// Snippet/template
    Snippet,
    /// Constant
    Constant,
}

impl CompletionItem {
    /// Create a new completion item
    pub fn new(label: impl Into<String>, kind: CompletionKind) -> Self {
        Self {
            label: label.into(),
            kind,
            detail: None,
            documentation: None,
            sort_priority: 100,
            insert_text: None,
            is_snippet: false,
        }
    }

    /// Set detail
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Set documentation
    pub fn with_documentation(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }

    /// Set sort priority
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.sort_priority = priority;
        self
    }

    /// Set insert text
    pub fn with_insert(mut self, text: impl Into<String>) -> Self {
        self.insert_text = Some(text.into());
        self
    }

    /// Mark as snippet
    pub fn as_snippet(mut self) -> Self {
        self.is_snippet = true;
        self
    }

    /// Get the text to insert
    pub fn insert_text(&self) -> &str {
        self.insert_text.as_deref().unwrap_or(&self.label)
    }

    /// Format for display in a completion list
    pub fn list_display(&self) -> String {
        let kind_icon = match self.kind {
            CompletionKind::Keyword => "K",
            CompletionKind::Type => "T",
            CompletionKind::Protocol => "P",
            CompletionKind::Function => "f",
            CompletionKind::Method => "m",
            CompletionKind::Variable => "v",
            CompletionKind::Field => ".",
            CompletionKind::Module => "M",
            CompletionKind::Snippet => "S",
            CompletionKind::Constant => "C",
        };

        if let Some(ref detail) = self.detail {
            format!("[{}] {} - {}", kind_icon, self.label, detail)
        } else {
            format!("[{}] {}", kind_icon, self.label)
        }
    }
}

/// Completion context information
#[derive(Debug, Clone)]
pub struct CompletionContext {
    /// Current line text
    pub line: String,
    /// Cursor position in line
    pub cursor: usize,
    /// Prefix being typed
    pub prefix: String,
    /// Whether after a dot (method completion)
    pub after_dot: bool,
    /// Type context (for method completion)
    pub receiver_type: Option<String>,
    /// Local bindings in scope
    pub local_bindings: HashMap<String, String>,
    /// Inside a function signature
    pub in_signature: bool,
    /// Inside a type annotation
    pub in_type_position: bool,
}

impl CompletionContext {
    /// Create a new completion context from line and cursor position
    pub fn from_line(line: &str, cursor: usize) -> Self {
        let line = line.to_string();
        let before_cursor = &line[..cursor.min(line.len())];

        // Find prefix (identifier being typed)
        let prefix = before_cursor
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect();

        // Check if after dot
        let prefix_len = before_cursor.len() - before_cursor.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_').len();
        let before_prefix = &before_cursor[..before_cursor.len() - prefix_len];
        let after_dot = before_prefix.trim_end().ends_with('.');

        // Try to detect receiver type (simplified)
        let receiver_type = if after_dot {
            // Look for identifier before the dot
            let before_dot = before_prefix.trim_end();
            let before_dot = before_dot.strip_suffix('.').unwrap_or(before_dot);
            let ident: String = before_dot
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !ident.is_empty() {
                Some(ident)
            } else {
                None
            }
        } else {
            None
        };

        // Check for type position (after :, after <, etc.)
        let in_type_position = before_prefix.trim_end().ends_with(':')
            || before_prefix.trim_end().ends_with('<')
            || before_prefix.trim_end().ends_with(',') && before_cursor.contains('<');

        // Check if in function signature
        let in_signature = before_cursor.contains("fn ") && !before_cursor.contains('{');

        Self {
            line,
            cursor,
            prefix,
            after_dot,
            receiver_type,
            local_bindings: HashMap::new(),
            in_signature,
            in_type_position,
        }
    }

    /// Add a local binding
    pub fn with_binding(mut self, name: impl Into<String>, ty: impl Into<String>) -> Self {
        self.local_bindings.insert(name.into(), ty.into());
        self
    }
}

/// Completion provider
pub struct CompletionProvider {
    /// Discovery index for type/function info
    index: DiscoveryIndex,
    /// Cached keywords
    keywords: Vec<CompletionItem>,
    /// Cached snippets
    snippets: Vec<CompletionItem>,
}

impl Default for CompletionProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionProvider {
    /// Create a new completion provider
    pub fn new() -> Self {
        Self {
            index: DiscoveryIndex::standard_library(),
            keywords: Self::build_keywords(),
            snippets: Self::build_snippets(),
        }
    }

    /// Create with custom index
    pub fn with_index(index: DiscoveryIndex) -> Self {
        Self {
            index,
            keywords: Self::build_keywords(),
            snippets: Self::build_snippets(),
        }
    }

    /// Get completions for a context
    pub fn complete(&self, ctx: &CompletionContext) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        // Method completions after dot
        if ctx.after_dot {
            items.extend(self.method_completions(ctx));
        }
        // Type completions in type position
        else if ctx.in_type_position {
            items.extend(self.type_completions(ctx));
        }
        // General completions
        else {
            // Keywords
            items.extend(self.keyword_completions(ctx));

            // Types (for constructors like List::from)
            items.extend(self.type_completions(ctx));

            // Functions
            items.extend(self.function_completions(ctx));

            // Local variables
            items.extend(self.variable_completions(ctx));

            // Snippets
            if ctx.prefix.is_empty() || ctx.line.trim().is_empty() {
                items.extend(self.snippets.clone());
            }
        }

        // Filter by prefix
        if !ctx.prefix.is_empty() {
            let prefix_lower = ctx.prefix.to_lowercase();
            items.retain(|item| item.label.to_lowercase().starts_with(&prefix_lower));
        }

        // Sort by priority then alphabetically
        items.sort_by(|a, b| {
            a.sort_priority
                .cmp(&b.sort_priority)
                .then_with(|| a.label.cmp(&b.label))
        });

        // Limit results
        items.truncate(50);

        items
    }

    /// Get keyword completions
    fn keyword_completions(&self, ctx: &CompletionContext) -> Vec<CompletionItem> {
        self.keywords
            .iter()
            .filter(|k| !ctx.after_dot)
            .cloned()
            .collect()
    }

    /// Get type completions
    fn type_completions(&self, ctx: &CompletionContext) -> Vec<CompletionItem> {
        self.index
            .all_types()
            .iter()
            .map(|doc| {
                CompletionItem::new(&doc.name, CompletionKind::Type)
                    .with_detail(&doc.signature)
                    .with_documentation(&doc.summary)
                    .with_priority(if ctx.in_type_position { 10 } else { 50 })
            })
            .chain(self.index.all_protocols().iter().map(|doc| {
                CompletionItem::new(&doc.name, CompletionKind::Protocol)
                    .with_detail(&doc.signature)
                    .with_documentation(&doc.summary)
                    .with_priority(if ctx.in_type_position { 15 } else { 55 })
            }))
            .collect()
    }

    /// Get function completions
    fn function_completions(&self, ctx: &CompletionContext) -> Vec<CompletionItem> {
        self.index
            .all_functions()
            .iter()
            .map(|doc| {
                let insert = format!("{}()", doc.short_name());
                CompletionItem::new(doc.short_name(), CompletionKind::Function)
                    .with_detail(&doc.signature)
                    .with_documentation(&doc.summary)
                    .with_insert(insert)
                    .with_priority(40)
            })
            .collect()
    }

    /// Get method completions for a type
    fn method_completions(&self, ctx: &CompletionContext) -> Vec<CompletionItem> {
        // If we know the receiver type, get its methods
        if let Some(ref receiver_type) = ctx.receiver_type {
            // Check if receiver is a known type
            if let Some(_doc) = self.index.get_doc(receiver_type) {
                return self
                    .index
                    .methods_for(receiver_type)
                    .iter()
                    .map(|method| {
                        CompletionItem::new(method.short_name(), CompletionKind::Method)
                            .with_detail(&method.signature)
                            .with_documentation(&method.summary)
                            .with_priority(10)
                    })
                    .collect();
            }
        }

        // Return common methods that work on many types
        vec![
            CompletionItem::new("map", CompletionKind::Method)
                .with_detail("fn map<B>(self, f: fn(T) -> B) -> Self<B>")
                .with_documentation("Transform each element")
                .with_priority(10),
            CompletionItem::new("filter", CompletionKind::Method)
                .with_detail("fn filter(self, f: fn(T) -> Bool) -> Self")
                .with_documentation("Keep elements matching predicate")
                .with_priority(10),
            CompletionItem::new("reduce", CompletionKind::Method)
                .with_detail("fn reduce<B>(self, init: B, f: fn(B, T) -> B) -> B")
                .with_documentation("Combine all elements")
                .with_priority(10),
            CompletionItem::new("len", CompletionKind::Method)
                .with_detail("fn len(&self) -> Int")
                .with_documentation("Get number of elements")
                .with_priority(15),
            CompletionItem::new("is_empty", CompletionKind::Method)
                .with_detail("fn is_empty(&self) -> Bool")
                .with_documentation("Check if empty")
                .with_priority(15),
            CompletionItem::new("iter", CompletionKind::Method)
                .with_detail("fn iter(&self) -> Iterator<T>")
                .with_documentation("Get an iterator")
                .with_priority(15),
            CompletionItem::new("collect", CompletionKind::Method)
                .with_detail("fn collect<C: FromIterator>(self) -> C")
                .with_documentation("Collect into a collection")
                .with_priority(15),
        ]
    }

    /// Get variable completions from local bindings
    fn variable_completions(&self, ctx: &CompletionContext) -> Vec<CompletionItem> {
        ctx.local_bindings
            .iter()
            .map(|(name, ty)| {
                CompletionItem::new(name, CompletionKind::Variable)
                    .with_detail(ty)
                    .with_priority(5) // Local variables are high priority
            })
            .collect()
    }

    /// Build keyword completion list
    fn build_keywords() -> Vec<CompletionItem> {
        vec![
            CompletionItem::new("let", CompletionKind::Keyword)
                .with_detail("Variable binding")
                .with_insert("let ")
                .with_priority(20),
            CompletionItem::new("let mut", CompletionKind::Keyword)
                .with_detail("Mutable variable binding")
                .with_insert("let mut ")
                .with_priority(21),
            CompletionItem::new("fn", CompletionKind::Keyword)
                .with_detail("Function definition")
                .with_insert("fn ")
                .with_priority(20),
            CompletionItem::new("type", CompletionKind::Keyword)
                .with_detail("Type definition")
                .with_insert("type ")
                .with_priority(20),
            CompletionItem::new("if", CompletionKind::Keyword)
                .with_detail("Conditional")
                .with_insert("if ")
                .with_priority(25),
            CompletionItem::new("else", CompletionKind::Keyword)
                .with_detail("Else branch")
                .with_insert("else ")
                .with_priority(26),
            CompletionItem::new("match", CompletionKind::Keyword)
                .with_detail("Pattern matching")
                .with_insert("match ")
                .with_priority(25),
            CompletionItem::new("for", CompletionKind::Keyword)
                .with_detail("For loop")
                .with_insert("for ")
                .with_priority(25),
            CompletionItem::new("while", CompletionKind::Keyword)
                .with_detail("While loop")
                .with_insert("while ")
                .with_priority(25),
            CompletionItem::new("loop", CompletionKind::Keyword)
                .with_detail("Infinite loop")
                .with_insert("loop ")
                .with_priority(25),
            CompletionItem::new("return", CompletionKind::Keyword)
                .with_detail("Return from function")
                .with_insert("return ")
                .with_priority(30),
            CompletionItem::new("break", CompletionKind::Keyword)
                .with_detail("Break from loop")
                .with_priority(30),
            CompletionItem::new("continue", CompletionKind::Keyword)
                .with_detail("Continue to next iteration")
                .with_priority(30),
            CompletionItem::new("async", CompletionKind::Keyword)
                .with_detail("Async function")
                .with_insert("async ")
                .with_priority(25),
            CompletionItem::new("await", CompletionKind::Keyword)
                .with_detail("Await future")
                .with_insert("await ")
                .with_priority(25),
            CompletionItem::new("yield", CompletionKind::Keyword)
                .with_detail("Yield from generator")
                .with_insert("yield ")
                .with_priority(25),
            CompletionItem::new("provide", CompletionKind::Keyword)
                .with_detail("Provide context")
                .with_insert("provide ")
                .with_priority(30),
            CompletionItem::new("using", CompletionKind::Keyword)
                .with_detail("Require context")
                .with_insert("using [")
                .with_priority(30),
            CompletionItem::new("mount", CompletionKind::Keyword)
                .with_detail("Import module")
                .with_insert("mount ")
                .with_priority(15),
            CompletionItem::new("implement", CompletionKind::Keyword)
                .with_detail("Implement protocol")
                .with_insert("implement ")
                .with_priority(22),
        ]
    }

    /// Build snippet completion list
    fn build_snippets() -> Vec<CompletionItem> {
        vec![
            CompletionItem::new("fn_def", CompletionKind::Snippet)
                .with_detail("Function definition")
                .with_insert("fn ${1:name}(${2:params}) -> ${3:ReturnType} {\n    ${0}\n}")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("type_struct", CompletionKind::Snippet)
                .with_detail("Struct type definition")
                .with_insert("type ${1:Name} is {\n    ${2:field}: ${3:Type},\n};")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("type_enum", CompletionKind::Snippet)
                .with_detail("Enum type definition")
                .with_insert("type ${1:Name} is\n    | ${2:Variant1}\n    | ${3:Variant2};")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("match_expr", CompletionKind::Snippet)
                .with_detail("Match expression")
                .with_insert("match ${1:value} {\n    ${2:pattern} => ${3:result},\n    _ => ${0},\n}")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("for_loop", CompletionKind::Snippet)
                .with_detail("For loop")
                .with_insert("for ${1:item} in ${2:iter} {\n    ${0}\n}")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("if_let", CompletionKind::Snippet)
                .with_detail("If let pattern")
                .with_insert("if let ${1:Some(value)} = ${2:expr} {\n    ${0}\n}")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("async_fn", CompletionKind::Snippet)
                .with_detail("Async function")
                .with_insert("async fn ${1:name}(${2:params}) -> ${3:ReturnType} {\n    ${0}\n}")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("generator", CompletionKind::Snippet)
                .with_detail("Generator function")
                .with_insert("fn* ${1:name}() -> ${2:YieldType} {\n    ${0}\n}")
                .as_snippet()
                .with_priority(80),
            CompletionItem::new("test_fn", CompletionKind::Snippet)
                .with_detail("Test function")
                .with_insert("@test\nfn test_${1:name}() {\n    ${0}\n}")
                .as_snippet()
                .with_priority(85),
            CompletionItem::new("context_using", CompletionKind::Snippet)
                .with_detail("Function with context")
                .with_insert("fn ${1:name}(${2:params}) using [${3:Context}] {\n    ${0}\n}")
                .as_snippet()
                .with_priority(85),
        ]
    }
}

/// Inline help information
#[derive(Debug, Clone)]
pub struct InlineHelp {
    /// The symbol being hovered
    pub symbol: String,
    /// Kind of symbol
    pub kind: CompletionKind,
    /// Type signature
    pub signature: Option<String>,
    /// Documentation
    pub documentation: Option<String>,
    /// Quick example
    pub example: Option<String>,
}

impl InlineHelp {
    /// Format for display
    pub fn display(&self) -> String {
        let mut output = String::new();

        // Header
        output.push_str(&format!("**{}**", self.symbol));
        if let Some(ref sig) = self.signature {
            output.push_str(&format!("\n\n```verum\n{}\n```", sig));
        }

        // Documentation
        if let Some(ref doc) = self.documentation {
            output.push_str("\n\n");
            output.push_str(doc);
        }

        // Example
        if let Some(ref example) = self.example {
            output.push_str("\n\n**Example:**\n```verum\n");
            output.push_str(example);
            output.push_str("\n```");
        }

        output
    }
}

/// Get inline help for a symbol
pub fn get_inline_help(index: &DiscoveryIndex, symbol: &str) -> Option<InlineHelp> {
    // Try to find in docs
    if let Some(doc) = index.get_doc(symbol) {
        return Some(InlineHelp {
            symbol: doc.name.clone(),
            kind: match doc.kind {
                DocKind::Type => CompletionKind::Type,
                DocKind::Protocol => CompletionKind::Protocol,
                DocKind::Function => CompletionKind::Function,
                DocKind::Method => CompletionKind::Method,
                DocKind::Module => CompletionKind::Module,
                DocKind::Constant => CompletionKind::Constant,
                DocKind::Context => CompletionKind::Variable,
            },
            signature: Some(doc.signature.clone()),
            documentation: Some(doc.description.clone()),
            example: doc.examples.first().cloned(),
        });
    }

    // Try modules
    if let Some(module) = index.modules.get(symbol) {
        return Some(InlineHelp {
            symbol: module.name.clone(),
            kind: CompletionKind::Module,
            signature: None,
            documentation: Some(module.description.clone()),
            example: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_context() {
        let ctx = CompletionContext::from_line("let x = List::from([1, 2, 3]).m", 31);

        assert_eq!(ctx.prefix, "m");
        assert!(ctx.after_dot);
    }

    #[test]
    fn test_completion_provider() {
        let provider = CompletionProvider::new();
        let ctx = CompletionContext::from_line("let x = ", 8);

        let completions = provider.complete(&ctx);
        assert!(!completions.is_empty());

        // Should include keywords
        assert!(completions.iter().any(|c| c.kind == CompletionKind::Keyword));
    }

    #[test]
    fn test_method_completions() {
        let provider = CompletionProvider::new();
        let ctx = CompletionContext::from_line("list.m", 6);

        let completions = provider.complete(&ctx);
        assert!(completions.iter().any(|c| c.label == "map"));
    }

    #[test]
    fn test_type_completions() {
        let provider = CompletionProvider::new();
        let ctx = CompletionContext::from_line("let x: L", 8);

        let completions = provider.complete(&ctx);
        assert!(completions.iter().any(|c| c.label == "List"));
    }

    #[test]
    fn test_inline_help() {
        let index = DiscoveryIndex::standard_library();
        let help = get_inline_help(&index, "List");

        assert!(help.is_some());
        let help = help.unwrap();
        assert_eq!(help.symbol, "List");
        assert!(help.documentation.is_some());
    }
}
