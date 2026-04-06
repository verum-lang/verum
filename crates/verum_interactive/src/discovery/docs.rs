//! Documentation entries for core/ items

use std::fmt;

/// Kind of documented item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocKind {
    /// Module (e.g., `core.collections`)
    Module,
    /// Type definition (struct, enum, newtype)
    Type,
    /// Protocol (trait-like interface)
    Protocol,
    /// Function
    Function,
    /// Method on a type
    Method,
    /// Constant value
    Constant,
    /// Context type (for dependency injection)
    Context,
}

impl fmt::Display for DocKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocKind::Module => write!(f, "module"),
            DocKind::Type => write!(f, "type"),
            DocKind::Protocol => write!(f, "protocol"),
            DocKind::Function => write!(f, "fn"),
            DocKind::Method => write!(f, "method"),
            DocKind::Constant => write!(f, "const"),
            DocKind::Context => write!(f, "context"),
        }
    }
}

/// A documentation entry for a core/ item
#[derive(Debug, Clone)]
pub struct DocEntry {
    /// Fully qualified name (e.g., `core.collections.List`)
    pub name: String,
    /// Module path (e.g., `core.collections`)
    pub module: String,
    /// Kind of item
    pub kind: DocKind,
    /// Type signature or declaration
    pub signature: String,
    /// Brief description (first line of docs)
    pub summary: String,
    /// Full description with formatting
    pub description: String,
    /// Example usage snippets
    pub examples: Vec<String>,
    /// Related items to explore
    pub see_also: Vec<String>,
    /// Tags for categorization
    pub tags: Vec<String>,
}

impl DocEntry {
    /// Create a new documentation entry
    pub fn new(name: impl Into<String>, module: impl Into<String>, kind: DocKind) -> Self {
        Self {
            name: name.into(),
            module: module.into(),
            kind,
            signature: String::new(),
            summary: String::new(),
            description: String::new(),
            examples: Vec::new(),
            see_also: Vec::new(),
            tags: Vec::new(),
        }
    }

    /// Set the signature
    pub fn with_signature(mut self, sig: impl Into<String>) -> Self {
        self.signature = sig.into();
        self
    }

    /// Set the summary
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Set the full description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Add an example
    pub fn with_example(mut self, example: impl Into<String>) -> Self {
        self.examples.push(example.into());
        self
    }

    /// Add multiple examples
    pub fn with_examples(mut self, examples: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.examples.extend(examples.into_iter().map(|e| e.into()));
        self
    }

    /// Add see-also references
    pub fn with_see_also(mut self, references: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.see_also.extend(references.into_iter().map(|r| r.into()));
        self
    }

    /// Add tags
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags.extend(tags.into_iter().map(|t| t.into()));
        self
    }

    /// Get short display name (without module path)
    pub fn short_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }

    /// Format for display in a list
    pub fn list_display(&self) -> String {
        format!("{} {} - {}", self.kind, self.short_name(), self.summary)
    }

    /// Format full documentation
    pub fn full_display(&self) -> String {
        let mut output = String::new();

        // Header
        output.push_str(&format!("# {} ({})\n\n", self.name, self.kind));

        // Signature
        if !self.signature.is_empty() {
            output.push_str("```verum\n");
            output.push_str(&self.signature);
            output.push_str("\n```\n\n");
        }

        // Summary
        if !self.summary.is_empty() {
            output.push_str(&self.summary);
            output.push_str("\n\n");
        }

        // Full description
        if !self.description.is_empty() {
            output.push_str(&self.description);
            output.push_str("\n\n");
        }

        // Examples
        if !self.examples.is_empty() {
            output.push_str("## Examples\n\n");
            for (i, example) in self.examples.iter().enumerate() {
                if self.examples.len() > 1 {
                    output.push_str(&format!("### Example {}\n\n", i + 1));
                }
                output.push_str("```verum\n");
                output.push_str(example);
                output.push_str("\n```\n\n");
            }
        }

        // See also
        if !self.see_also.is_empty() {
            output.push_str("## See Also\n\n");
            for item in &self.see_also {
                output.push_str(&format!("- {}\n", item));
            }
            output.push('\n');
        }

        // Tags
        if !self.tags.is_empty() {
            output.push_str("Tags: ");
            output.push_str(&self.tags.join(", "));
            output.push('\n');
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doc_entry_creation() {
        let doc = DocEntry::new("List", "core.collections", DocKind::Type)
            .with_signature("type List<T> is { ... }")
            .with_summary("Growable, heap-allocated sequence of elements")
            .with_example("let nums = List::from([1, 2, 3])")
            .with_tags(["collection", "sequence"]);

        assert_eq!(doc.short_name(), "List");
        assert_eq!(doc.kind, DocKind::Type);
        assert_eq!(doc.tags.len(), 2);
    }

    #[test]
    fn test_doc_display() {
        let doc = DocEntry::new("map", "core.collections.List", DocKind::Method)
            .with_summary("Transform each element");

        let display = doc.list_display();
        assert!(display.contains("method"));
        assert!(display.contains("map"));
        assert!(display.contains("Transform"));
    }
}
