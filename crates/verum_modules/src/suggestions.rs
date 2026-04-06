//! Module name and item suggestion utilities.
//!
//! Provides fuzzy matching and suggestions for improved error diagnostics.
//! Uses Levenshtein distance for string similarity with optimizations for
//! common module naming patterns.
//!
//! Used by import resolution and name resolution error paths to provide
//! "did you mean?" suggestions when items or modules are not found.

use crate::path::ModulePath;
use verum_common::{List, Text};

/// Maximum Levenshtein distance to consider a suggestion useful.
/// Beyond this threshold, suggestions are unlikely to be helpful.
const MAX_EDIT_DISTANCE: usize = 3;

/// Maximum number of suggestions to return.
const MAX_SUGGESTIONS: usize = 5;

/// Minimum similarity ratio to consider a match (0.0-1.0).
const MIN_SIMILARITY_RATIO: f64 = 0.5;

/// Compute the Levenshtein edit distance between two strings.
///
/// This is the minimum number of single-character edits (insertions,
/// deletions, or substitutions) required to transform `a` into `b`.
///
/// Uses Wagner-Fischer algorithm with O(min(m,n)) space complexity.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    // Optimization: empty strings
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    // Optimization: equal strings
    if a == b {
        return 0;
    }

    // Ensure a is the shorter string for space efficiency
    let (a, b) = if a.len() > b.len() { (b, a) } else { (a, b) };

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    // Two-row optimization: we only need the previous row and current row
    let mut prev_row: Vec<usize> = (0..=m).collect();
    let mut curr_row: Vec<usize> = vec![0; m + 1];

    for j in 1..=n {
        curr_row[0] = j;

        for i in 1..=m {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };

            curr_row[i] = (prev_row[i] + 1) // deletion
                .min(curr_row[i - 1] + 1) // insertion
                .min(prev_row[i - 1] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[m]
}

/// Compute similarity ratio between two strings (0.0-1.0).
///
/// Returns 1.0 for identical strings, 0.0 for completely different strings.
pub fn similarity_ratio(a: &str, b: &str) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }

    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }

    let distance = levenshtein_distance(a, b);
    1.0 - (distance as f64 / max_len as f64)
}

/// A suggestion with its similarity score.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// The suggested value
    pub value: Text,
    /// Similarity score (0.0-1.0, higher is more similar)
    pub score: f64,
    /// Edit distance from the query
    pub distance: usize,
}

impl Suggestion {
    pub fn new(value: impl Into<Text>, score: f64, distance: usize) -> Self {
        Self {
            value: value.into(),
            score,
            distance,
        }
    }
}

/// Find similar strings from a list of candidates.
///
/// Returns suggestions sorted by similarity (most similar first).
pub fn find_similar<'a>(
    query: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> List<Suggestion> {
    let query_lower = query.to_lowercase();

    let mut suggestions: Vec<Suggestion> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let candidate_lower = candidate.to_lowercase();
            let distance = levenshtein_distance(&query_lower, &candidate_lower);

            // Early rejection: too different
            if distance > MAX_EDIT_DISTANCE {
                // Check for prefix/suffix match which might still be useful
                if !candidate_lower.starts_with(&query_lower)
                    && !candidate_lower.ends_with(&query_lower)
                    && !query_lower.starts_with(&candidate_lower)
                {
                    return None;
                }
            }

            let score = similarity_ratio(&query_lower, &candidate_lower);

            // Reject if similarity is too low
            if score < MIN_SIMILARITY_RATIO && distance > MAX_EDIT_DISTANCE {
                return None;
            }

            Some(Suggestion::new(candidate, score, distance))
        })
        .collect();

    // Sort by score descending, then by distance ascending, then alphabetically
    suggestions.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.distance.cmp(&b.distance))
            .then_with(|| a.value.cmp(&b.value))
    });

    // Limit suggestions
    suggestions.truncate(MAX_SUGGESTIONS);

    List::from_iter(suggestions)
}

/// Find similar module paths from a list of available modules.
pub fn find_similar_modules(
    query: &ModulePath,
    available: impl IntoIterator<Item = ModulePath>,
) -> List<ModulePath> {
    let query_str = query.to_string();
    let query_name = query.segments().last().map(|s| s.as_str()).unwrap_or("");

    let mut scored: Vec<(ModulePath, f64)> = available
        .into_iter()
        .filter_map(|path| {
            let path_str = path.to_string();
            let path_name = path.segments().last().map(|s| s.as_str()).unwrap_or("");

            // Calculate similarity based on both full path and final segment
            let full_score = similarity_ratio(&query_str, &path_str);
            let name_score = similarity_ratio(query_name, path_name);

            // Weight final segment more heavily (users often get paths wrong but names right)
            let combined_score = full_score * 0.4 + name_score * 0.6;

            if combined_score >= MIN_SIMILARITY_RATIO {
                Some((path, combined_score))
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.to_string().cmp(&b.0.to_string()))
    });

    // Limit and extract paths
    scored.truncate(MAX_SUGGESTIONS);
    List::from_iter(scored.into_iter().map(|(path, _)| path))
}

/// Find similar item names from a list of available items.
///
/// Considers common naming patterns:
/// - Case differences (HashMap vs hashmap)
/// - Underscore vs camelCase (get_item vs getItem)
/// - Common typos
pub fn find_similar_items(query: &str, available: &[Text]) -> List<Text> {
    // Normalize the query for comparison
    let query_normalized = normalize_identifier(query);

    let mut scored: Vec<(Text, f64)> = available
        .iter()
        .filter_map(|item| {
            let item_normalized = normalize_identifier(item.as_str());

            // Calculate similarity on normalized forms
            let normalized_score = similarity_ratio(&query_normalized, &item_normalized);

            // Also check raw similarity for exact-ish matches
            let raw_score = similarity_ratio(query, item.as_str());

            let score = normalized_score.max(raw_score);

            // Check for prefix match (user typing partial name)
            let prefix_bonus = if item.as_str().to_lowercase().starts_with(&query.to_lowercase()) {
                0.2
            } else {
                0.0
            };

            let final_score = score + prefix_bonus;

            if final_score >= MIN_SIMILARITY_RATIO {
                Some((item.clone(), final_score.min(1.0)))
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    // Limit and extract items
    scored.truncate(MAX_SUGGESTIONS);
    List::from_iter(scored.into_iter().map(|(item, _)| item))
}

/// Normalize an identifier for comparison.
///
/// Converts to lowercase and normalizes underscores/camelCase.
/// Both `get_item` and `getItem` become `get item`.
fn normalize_identifier(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    let mut prev_was_separator = true; // Start as if preceded by separator

    for c in s.chars() {
        if c == '_' {
            // Convert underscore to space if result is non-empty and doesn't end with space
            if !result.is_empty() && !result.ends_with(' ') {
                result.push(' ');
            }
            prev_was_separator = true;
            continue;
        }

        // Insert space before uppercase letters (except at start or after separator)
        if c.is_uppercase() && !prev_was_separator && !result.is_empty() {
            result.push(' ');
        }

        result.push(c.to_ascii_lowercase());
        prev_was_separator = false;
    }

    result
}

/// Format suggestions for display in error messages.
pub fn format_suggestions(suggestions: &[Text]) -> String {
    if suggestions.is_empty() {
        return String::new();
    }

    let mut result = String::from("\nDid you mean:");
    for suggestion in suggestions {
        result.push_str("\n  - ");
        result.push_str(suggestion.as_str());
    }
    result
}

/// Format module path suggestions for display.
pub fn format_module_suggestions(suggestions: &[ModulePath]) -> String {
    if suggestions.is_empty() {
        return String::new();
    }

    let mut result = String::from("\nDid you mean:");
    for suggestion in suggestions {
        result.push_str("\n  - ");
        result.push_str(&suggestion.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn test_levenshtein_distance_one_empty() {
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert_eq!(levenshtein_distance("", "world"), 5);
    }

    #[test]
    fn test_levenshtein_distance_single_edit() {
        // Substitution
        assert_eq!(levenshtein_distance("cat", "bat"), 1);
        // Insertion
        assert_eq!(levenshtein_distance("cat", "cats"), 1);
        // Deletion
        assert_eq!(levenshtein_distance("cats", "cat"), 1);
    }

    #[test]
    fn test_levenshtein_distance_multiple_edits() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("flaw", "lawn"), 2);
        assert_eq!(levenshtein_distance("saturday", "sunday"), 3);
    }

    #[test]
    fn test_similarity_ratio() {
        assert!((similarity_ratio("hello", "hello") - 1.0).abs() < 0.001);
        assert!((similarity_ratio("hello", "hallo") - 0.8).abs() < 0.001);
        assert!(similarity_ratio("abc", "xyz") < 0.5);
    }

    #[test]
    fn test_find_similar() {
        let candidates = vec!["HashMap", "HashSet", "BTreeMap", "Vector", "List"];
        let suggestions = find_similar("HashMop", candidates.into_iter());

        assert!(!suggestions.is_empty());
        // HashMap should be first (closest match)
        assert_eq!(suggestions[0].value.as_str(), "HashMap");
    }

    #[test]
    fn test_find_similar_case_insensitive() {
        let candidates = vec!["HashMap", "hashmap", "HASHMAP"];
        let suggestions = find_similar("hashMap", candidates.into_iter());

        assert!(!suggestions.is_empty());
    }

    #[test]
    fn test_find_similar_items_camelcase() {
        let available = vec![
            Text::from("getItem"),
            Text::from("setItem"),
            Text::from("deleteItem"),
        ];

        let suggestions = find_similar_items("get_item", &available);
        assert!(!suggestions.is_empty());
        // getItem should match get_item
        assert!(suggestions.iter().any(|s| s.as_str() == "getItem"));
    }

    #[test]
    fn test_find_similar_items_prefix() {
        let available = vec![
            Text::from("getUserById"),
            Text::from("getUsers"),
            Text::from("getAllUsers"),
            Text::from("deleteUser"),
        ];

        let suggestions = find_similar_items("getUs", &available);
        assert!(!suggestions.is_empty());
        // Should suggest items starting with "getUs"
        assert!(suggestions.iter().any(|s| s.as_str().starts_with("getUs")));
    }

    #[test]
    fn test_normalize_identifier() {
        // camelCase
        assert_eq!(normalize_identifier("getItem"), "get item");
        // snake_case
        assert_eq!(normalize_identifier("get_item"), "get item");
        // PascalCase with multiple words
        assert_eq!(normalize_identifier("GetUserById"), "get user by id");
        // ALL_CAPS acronym (each letter is uppercase, spaces between)
        assert_eq!(normalize_identifier("API"), "a p i");
        // Mixed case (XMLHttp has consecutive uppercase which get spaced)
        assert_eq!(normalize_identifier("XMLHttpRequest"), "x m l http request");
        // Simple lowercase
        assert_eq!(normalize_identifier("hello"), "hello");
        // Already spaced snake_case
        assert_eq!(normalize_identifier("get_user_by_id"), "get user by id");
    }

    #[test]
    fn test_find_similar_modules() {
        let available = vec![
            ModulePath::from_str("std.collections.HashMap"),
            ModulePath::from_str("std.collections.HashSet"),
            ModulePath::from_str("std.io.File"),
        ];

        let query = ModulePath::from_str("std.collections.HashMop");
        let suggestions = find_similar_modules(&query, available);

        assert!(!suggestions.is_empty());
        // HashMap should be suggested
        assert!(suggestions
            .iter()
            .any(|p| p.to_string().contains("HashMap")));
    }

    #[test]
    fn test_format_suggestions_empty() {
        let suggestions: Vec<Text> = vec![];
        assert_eq!(format_suggestions(&suggestions), "");
    }

    #[test]
    fn test_format_suggestions() {
        let suggestions = vec![Text::from("HashMap"), Text::from("HashSet")];
        let formatted = format_suggestions(&suggestions);

        assert!(formatted.contains("Did you mean:"));
        assert!(formatted.contains("HashMap"));
        assert!(formatted.contains("HashSet"));
    }

    #[test]
    fn test_max_edit_distance_filtering() {
        let candidates = vec!["a", "completely_different_name"];
        let suggestions = find_similar("test", candidates.into_iter());

        // "completely_different_name" should be filtered out
        assert!(suggestions
            .iter()
            .all(|s| s.value.as_str() != "completely_different_name"));
    }
}
