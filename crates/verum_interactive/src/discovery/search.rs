//! Search functionality for discovery

use super::docs::{DocEntry, DocKind};
use super::examples::Example;
use super::index::DiscoveryIndex;

/// A search query
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// The search text
    pub text: String,
    /// Filter by kind
    pub kind_filter: Option<DocKind>,
    /// Filter by module
    pub module_filter: Option<String>,
    /// Filter by tags
    pub tag_filter: Vec<String>,
    /// Maximum results to return
    pub limit: usize,
}

impl SearchQuery {
    /// Create a new search query
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind_filter: None,
            module_filter: None,
            tag_filter: Vec::new(),
            limit: 20,
        }
    }

    /// Filter by kind
    pub fn kind(mut self, kind: DocKind) -> Self {
        self.kind_filter = Some(kind);
        self
    }

    /// Filter by module
    pub fn module(mut self, module: impl Into<String>) -> Self {
        self.module_filter = Some(module.into());
        self
    }

    /// Filter by tag
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag_filter.push(tag.into());
        self
    }

    /// Set result limit
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// A search result
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Name of the matched item
    pub name: String,
    /// Kind of match
    pub kind: SearchResultKind,
    /// Relevance score (higher is better)
    pub score: f32,
    /// Matched snippet for display
    pub snippet: String,
    /// Module path
    pub module: String,
}

/// Kind of search result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchResultKind {
    /// Documentation entry
    Doc(DocKind),
    /// Example
    Example,
    /// Module
    Module,
}

impl SearchResult {
    /// Create a doc result
    pub fn from_doc(doc: &DocEntry, score: f32) -> Self {
        Self {
            name: doc.name.clone(),
            kind: SearchResultKind::Doc(doc.kind),
            score,
            snippet: doc.summary.clone(),
            module: doc.module.clone(),
        }
    }

    /// Create an example result
    pub fn from_example(example: &Example, score: f32) -> Self {
        Self {
            name: example.title.clone(),
            kind: SearchResultKind::Example,
            score,
            snippet: example.description.clone(),
            module: format!("examples.{}", example.category),
        }
    }

    /// Format for list display
    pub fn list_display(&self) -> String {
        let kind_str = match self.kind {
            SearchResultKind::Doc(k) => format!("[{}]", k),
            SearchResultKind::Example => "[example]".to_string(),
            SearchResultKind::Module => "[module]".to_string(),
        };
        format!("{} {} - {}", kind_str, self.name, self.snippet)
    }
}

/// Search the discovery index
pub fn search(index: &DiscoveryIndex, query: &SearchQuery) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let query_lower = query.text.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();

    // Search docs
    for doc in index.docs.values() {
        // Apply filters
        if let Some(kind) = query.kind_filter
            && doc.kind != kind {
                continue;
            }

        if let Some(ref module) = query.module_filter
            && !doc.module.starts_with(module) {
                continue;
            }

        if !query.tag_filter.is_empty() {
            let has_all_tags = query.tag_filter.iter().all(|t| doc.tags.contains(t));
            if !has_all_tags {
                continue;
            }
        }

        // Calculate relevance score
        let score = calculate_doc_score(doc, &terms);
        if score > 0.0 {
            results.push(SearchResult::from_doc(doc, score));
        }
    }

    // Search examples
    for example in index.examples.values() {
        let score = calculate_example_score(example, &terms);
        if score > 0.0 {
            results.push(SearchResult::from_example(example, score));
        }
    }

    // Search modules
    for module in index.modules.modules.values() {
        let score = calculate_module_score(module, &terms);
        if score > 0.0 {
            results.push(SearchResult {
                name: module.name.clone(),
                kind: SearchResultKind::Module,
                score,
                snippet: module.description.clone(),
                module: module.path.clone(),
            });
        }
    }

    // Sort by score descending
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // Limit results
    results.truncate(query.limit);

    results
}

/// Calculate relevance score for a doc entry
fn calculate_doc_score(doc: &DocEntry, terms: &[&str]) -> f32 {
    let mut score = 0.0;
    let name_lower = doc.name.to_lowercase();
    let summary_lower = doc.summary.to_lowercase();
    let desc_lower = doc.description.to_lowercase();

    for term in terms {
        // Exact name match is highest priority
        if name_lower == *term {
            score += 10.0;
        } else if name_lower.contains(term) {
            score += 5.0;
        }

        // Summary match
        if summary_lower.contains(term) {
            score += 2.0;
        }

        // Description match
        if desc_lower.contains(term) {
            score += 1.0;
        }

        // Tag match
        for tag in &doc.tags {
            if tag.to_lowercase().contains(term) {
                score += 1.5;
            }
        }
    }

    // Bonus for common types
    match doc.kind {
        DocKind::Type | DocKind::Protocol => score *= 1.2,
        DocKind::Function => score *= 1.1,
        _ => {}
    }

    score
}

/// Calculate relevance score for an example
fn calculate_example_score(example: &Example, terms: &[&str]) -> f32 {
    let mut score = 0.0;
    let title_lower = example.title.to_lowercase();
    let desc_lower = example.description.to_lowercase();
    let source_lower = example.source.to_lowercase();

    for term in terms {
        if title_lower.contains(term) {
            score += 5.0;
        }
        if desc_lower.contains(term) {
            score += 2.0;
        }
        if source_lower.contains(term) {
            score += 1.0;
        }

        for tag in &example.tags {
            if tag.to_lowercase().contains(term) {
                score += 1.5;
            }
        }
    }

    // Lower priority than docs
    score * 0.8
}

/// Calculate relevance score for a module
fn calculate_module_score(module: &super::index::ModuleInfo, terms: &[&str]) -> f32 {
    let mut score = 0.0;
    let name_lower = module.name.to_lowercase();
    let desc_lower = module.description.to_lowercase();

    for term in terms {
        if name_lower == *term {
            score += 8.0;
        } else if name_lower.contains(term) {
            score += 4.0;
        }
        if desc_lower.contains(term) {
            score += 1.0;
        }
    }

    // Slightly lower priority than type docs
    score * 0.9
}

/// Helper functions for common searches
impl DiscoveryIndex {
    /// Search for a term
    pub fn search(&self, text: &str) -> Vec<SearchResult> {
        search(self, &SearchQuery::new(text))
    }

    /// Search with query builder
    pub fn search_query(&self, query: &SearchQuery) -> Vec<SearchResult> {
        search(self, query)
    }

    /// Find similar items to a given name
    pub fn find_similar(&self, name: &str, limit: usize) -> Vec<SearchResult> {
        // Get the doc entry
        if let Some(doc) = self.get_doc(name) {
            // Search using tags
            let mut results = Vec::new();
            for tag in &doc.tags {
                for item_name in self.by_tag(tag) {
                    if item_name != name
                        && let Some(similar_doc) = self.get_doc(item_name) {
                            results.push(SearchResult::from_doc(similar_doc, 1.0));
                        }
                }
            }
            results.truncate(limit);
            results
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_basic() {
        let index = DiscoveryIndex::standard_library();
        let results = index.search("list");

        assert!(!results.is_empty());
        // List should be in the top results
        assert!(results.iter().any(|r| r.name.to_lowercase() == "list"));
    }

    #[test]
    fn test_search_with_filter() {
        let index = DiscoveryIndex::standard_library();
        let results = search(
            &index,
            &SearchQuery::new("tensor").kind(DocKind::Type),
        );

        // Docs with Type filter should only return types
        let doc_results: Vec<_> = results
            .iter()
            .filter(|r| matches!(r.kind, SearchResultKind::Doc(_)))
            .collect();

        for result in &doc_results {
            assert!(matches!(result.kind, SearchResultKind::Doc(DocKind::Type)));
        }
    }

    #[test]
    fn test_search_examples() {
        let index = DiscoveryIndex::standard_library();
        let results = index.search("fibonacci");

        assert!(!results.is_empty());
        assert!(results.iter().any(|r| matches!(r.kind, SearchResultKind::Example)));
    }

    #[test]
    fn test_find_similar() {
        let index = DiscoveryIndex::standard_library();
        let similar = index.find_similar("List", 5);

        // Should find other collection types
        // Map and Set share the "collection" tag
        assert!(similar.iter().any(|r| r.name == "Map" || r.name == "Set"));
    }
}
