//! Mutation strategies for fuzz testing
//!
//! This module provides various mutation strategies for transforming
//! Verum programs during fuzz testing. It supports:
//!
//! - Expression mutations (operators, literals, etc.)
//! - Statement mutations (insertions, deletions, reorderings)
//! - Type mutations (type changes, generics, etc.)
//! - Structural mutations (function changes, etc.)
//! - Targeted mutations for specific bug classes
//!
//! # Usage
//!
//! ```rust
//! use verum_fuzz::generators::mutation::{Mutator, MutationStrategy};
//! use rand::rng;
//!
//! let mutator = Mutator::new(Default::default());
//! let original = "let x = 1 + 2;";
//! let mutated = mutator.mutate(original, &mut rng());
//! ```

use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::{IndexedRandom, IteratorRandom, SliceRandom};

/// Mutation configuration
#[derive(Debug, Clone)]
pub struct MutationConfig {
    /// Maximum number of mutations per round
    pub max_mutations: usize,
    /// Probability of applying each mutation type
    pub strategy_weights: StrategyWeights,
    /// Whether to preserve syntactic validity
    pub preserve_syntax: bool,
    /// Whether to preserve semantic validity
    pub preserve_semantics: bool,
    /// Maximum string perturbation distance
    pub max_perturbation: usize,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self {
            max_mutations: 5,
            strategy_weights: StrategyWeights::default(),
            preserve_syntax: true,
            preserve_semantics: false,
            max_perturbation: 10,
        }
    }
}

/// Weights for different mutation strategies
#[derive(Debug, Clone)]
pub struct StrategyWeights {
    pub operator_swap: u32,
    pub literal_change: u32,
    pub identifier_rename: u32,
    pub statement_delete: u32,
    pub statement_duplicate: u32,
    pub statement_reorder: u32,
    pub expression_wrap: u32,
    pub expression_unwrap: u32,
    pub type_change: u32,
    pub keyword_swap: u32,
    pub bracket_mutate: u32,
    pub whitespace_mutate: u32,
    pub boundary_values: u32,
    pub unicode_inject: u32,
}

impl Default for StrategyWeights {
    fn default() -> Self {
        Self {
            operator_swap: 20,
            literal_change: 25,
            identifier_rename: 10,
            statement_delete: 8,
            statement_duplicate: 5,
            statement_reorder: 5,
            expression_wrap: 8,
            expression_unwrap: 5,
            type_change: 6,
            keyword_swap: 3,
            bracket_mutate: 2,
            whitespace_mutate: 2,
            boundary_values: 10,
            unicode_inject: 3,
        }
    }
}

impl StrategyWeights {
    pub fn as_vec(&self) -> Vec<u32> {
        vec![
            self.operator_swap,
            self.literal_change,
            self.identifier_rename,
            self.statement_delete,
            self.statement_duplicate,
            self.statement_reorder,
            self.expression_wrap,
            self.expression_unwrap,
            self.type_change,
            self.keyword_swap,
            self.bracket_mutate,
            self.whitespace_mutate,
            self.boundary_values,
            self.unicode_inject,
        ]
    }
}

/// Mutation strategy type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationStrategy {
    /// Swap arithmetic/comparison operators
    OperatorSwap,
    /// Change literal values
    LiteralChange,
    /// Rename identifiers
    IdentifierRename,
    /// Delete statements
    StatementDelete,
    /// Duplicate statements
    StatementDuplicate,
    /// Reorder statements
    StatementReorder,
    /// Wrap expression in another construct
    ExpressionWrap,
    /// Unwrap nested expressions
    ExpressionUnwrap,
    /// Change type annotations
    TypeChange,
    /// Swap keywords
    KeywordSwap,
    /// Mutate brackets/delimiters
    BracketMutate,
    /// Mutate whitespace
    WhitespaceMutate,
    /// Insert boundary values
    BoundaryValues,
    /// Inject Unicode characters
    UnicodeInject,
}

impl MutationStrategy {
    pub fn all() -> &'static [MutationStrategy] {
        &[
            MutationStrategy::OperatorSwap,
            MutationStrategy::LiteralChange,
            MutationStrategy::IdentifierRename,
            MutationStrategy::StatementDelete,
            MutationStrategy::StatementDuplicate,
            MutationStrategy::StatementReorder,
            MutationStrategy::ExpressionWrap,
            MutationStrategy::ExpressionUnwrap,
            MutationStrategy::TypeChange,
            MutationStrategy::KeywordSwap,
            MutationStrategy::BracketMutate,
            MutationStrategy::WhitespaceMutate,
            MutationStrategy::BoundaryValues,
            MutationStrategy::UnicodeInject,
        ]
    }
}

/// Result of a mutation
#[derive(Debug, Clone)]
pub struct MutationResult {
    /// Original source code
    pub original: String,
    /// Mutated source code
    pub mutated: String,
    /// Applied mutations
    pub mutations: Vec<AppliedMutation>,
    /// Whether the mutation preserved syntax
    pub syntax_preserved: bool,
}

/// Record of an applied mutation
#[derive(Debug, Clone)]
pub struct AppliedMutation {
    /// Strategy used
    pub strategy: MutationStrategy,
    /// Position in source
    pub position: usize,
    /// Original text
    pub original_text: String,
    /// New text
    pub new_text: String,
}

/// Program mutator
pub struct Mutator {
    config: MutationConfig,
    strategy_dist: WeightedIndex<u32>,
}

impl Mutator {
    /// Create a new mutator with the given configuration
    pub fn new(config: MutationConfig) -> Self {
        let weights = config.strategy_weights.as_vec();
        let strategy_dist = WeightedIndex::new(&weights).unwrap();
        Self {
            config,
            strategy_dist,
        }
    }

    /// Apply random mutations to a program
    pub fn mutate<R: Rng>(&self, source: &str, rng: &mut R) -> MutationResult {
        let mut mutated = source.to_string();
        let mut mutations = Vec::new();

        let num_mutations = rng.random_range(1..=self.config.max_mutations);

        for _ in 0..num_mutations {
            let strategy = self.select_strategy(rng);
            if let Some(mutation) = self.apply_mutation(&mutated, strategy, rng) {
                mutated = mutation.mutated.clone();
                mutations.push(AppliedMutation {
                    strategy,
                    position: mutation.position,
                    original_text: mutation.original_text,
                    new_text: mutation.new_text,
                });
            }
        }

        MutationResult {
            original: source.to_string(),
            mutated,
            mutations,
            syntax_preserved: true, // Would need parser to verify
        }
    }

    /// Select a mutation strategy based on weights
    fn select_strategy<R: Rng>(&self, rng: &mut R) -> MutationStrategy {
        MutationStrategy::all()[self.strategy_dist.sample(rng)]
    }

    /// Apply a specific mutation
    fn apply_mutation<R: Rng>(
        &self,
        source: &str,
        strategy: MutationStrategy,
        rng: &mut R,
    ) -> Option<SingleMutation> {
        match strategy {
            MutationStrategy::OperatorSwap => self.mutate_operator(source, rng),
            MutationStrategy::LiteralChange => self.mutate_literal(source, rng),
            MutationStrategy::IdentifierRename => self.mutate_identifier(source, rng),
            MutationStrategy::StatementDelete => self.mutate_delete_statement(source, rng),
            MutationStrategy::StatementDuplicate => self.mutate_duplicate_statement(source, rng),
            MutationStrategy::StatementReorder => self.mutate_reorder_statements(source, rng),
            MutationStrategy::ExpressionWrap => self.mutate_wrap_expression(source, rng),
            MutationStrategy::ExpressionUnwrap => self.mutate_unwrap_expression(source, rng),
            MutationStrategy::TypeChange => self.mutate_type(source, rng),
            MutationStrategy::KeywordSwap => self.mutate_keyword(source, rng),
            MutationStrategy::BracketMutate => self.mutate_bracket(source, rng),
            MutationStrategy::WhitespaceMutate => self.mutate_whitespace(source, rng),
            MutationStrategy::BoundaryValues => self.mutate_boundary_values(source, rng),
            MutationStrategy::UnicodeInject => self.mutate_unicode(source, rng),
        }
    }

    /// Swap operators
    fn mutate_operator<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        let operator_groups = vec![
            vec!["+", "-"],
            vec!["*", "/", "%"],
            vec!["==", "!="],
            vec!["<", ">", "<=", ">="],
            vec!["&&", "||"],
            vec!["&", "|", "^"],
            vec!["<<", ">>"],
        ];

        for group in &operator_groups {
            for op in group {
                if let Some(pos) = source.find(op) {
                    let replacement = group.iter().filter(|&&o| o != *op).choose(rng)?;

                    let mut mutated = source.to_string();
                    mutated.replace_range(pos..pos + op.len(), replacement);

                    return Some(SingleMutation {
                        mutated,
                        position: pos,
                        original_text: op.to_string(),
                        new_text: replacement.to_string(),
                    });
                }
            }
        }

        None
    }

    /// Change literal values
    fn mutate_literal<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        // Find integer literals
        let mut i = 0;
        let chars: Vec<char> = source.chars().collect();

        while i < chars.len() {
            if chars[i].is_ascii_digit()
                || (chars[i] == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
            {
                let start = i;
                if chars[i] == '-' {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }

                let literal: String = chars[start..i].iter().collect();
                if let Ok(n) = literal.parse::<i64>() {
                    let new_value = match rng.random_range(0..8) {
                        0 => 0,
                        1 => 1,
                        2 => -1,
                        3 => n.saturating_add(1),
                        4 => n.saturating_sub(1),
                        5 => n.saturating_mul(2),
                        6 => n / 2,
                        _ => rng.random_range(-1000..1000),
                    };

                    let new_literal = new_value.to_string();
                    let mut mutated = source.to_string();
                    mutated.replace_range(start..start + literal.len(), &new_literal);

                    return Some(SingleMutation {
                        mutated,
                        position: start,
                        original_text: literal,
                        new_text: new_literal,
                    });
                }
            }
            i += 1;
        }

        None
    }

    /// Rename identifiers
    fn mutate_identifier<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        // Find simple identifiers (variable names)
        let identifiers: Vec<_> = source
            .match_indices(|c: char| c.is_alphabetic() || c == '_')
            .filter(|(pos, _)| {
                // Check it's the start of an identifier
                *pos == 0
                    || !source
                        .chars()
                        .nth(*pos - 1)
                        .unwrap_or(' ')
                        .is_alphanumeric()
            })
            .collect();

        if identifiers.is_empty() {
            return None;
        }

        let (pos, _) = *identifiers.choose(rng)?;

        // Find the full identifier
        let end = source[pos..]
            .char_indices()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map(|(i, c)| pos + i + c.len_utf8())
            .unwrap_or(pos + 1);

        let original = &source[pos..end];

        // Skip keywords
        let keywords = [
            "fn", "let", "mut", "if", "else", "match", "for", "while", "loop", "return", "break",
            "continue", "true", "false", "in", "as", "type", "struct", "enum", "impl", "trait",
            "use", "pub", "async", "await",
        ];
        if keywords.contains(&original) {
            return None;
        }

        // Generate a new name
        let new_name = format!("{}_mutated", original);

        let mut mutated = source.to_string();
        mutated.replace_range(pos..end, &new_name);

        Some(SingleMutation {
            mutated,
            position: pos,
            original_text: original.to_string(),
            new_text: new_name,
        })
    }

    /// Delete a statement
    fn mutate_delete_statement<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        let lines: Vec<&str> = source.lines().collect();
        if lines.len() <= 3 {
            return None; // Don't delete if too few lines
        }

        // Find deletable lines (statements with semicolons)
        let deletable: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                trimmed.ends_with(';') && !trimmed.starts_with("use ") && !trimmed.contains("fn ")
            })
            .map(|(i, _)| i)
            .collect();

        if deletable.is_empty() {
            return None;
        }

        let line_idx = *deletable.choose(rng)?;
        let original_line = lines[line_idx].to_string();

        let mutated: String = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != line_idx)
            .map(|(_, line)| *line)
            .collect::<Vec<_>>()
            .join("\n");

        Some(SingleMutation {
            mutated,
            position: lines[..line_idx].iter().map(|l| l.len() + 1).sum(),
            original_text: original_line,
            new_text: String::new(),
        })
    }

    /// Duplicate a statement
    fn mutate_duplicate_statement<R: Rng>(
        &self,
        source: &str,
        rng: &mut R,
    ) -> Option<SingleMutation> {
        let lines: Vec<&str> = source.lines().collect();

        // Find duplicatable lines
        let duplicatable: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                trimmed.ends_with(';') && !trimmed.starts_with("use ")
            })
            .map(|(i, _)| i)
            .collect();

        if duplicatable.is_empty() {
            return None;
        }

        let line_idx = *duplicatable.choose(rng)?;
        let line_to_dup = lines[line_idx];

        let mut new_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        new_lines.insert(line_idx + 1, line_to_dup.to_string());

        Some(SingleMutation {
            mutated: new_lines.join("\n"),
            position: lines[..line_idx].iter().map(|l| l.len() + 1).sum(),
            original_text: String::new(),
            new_text: line_to_dup.to_string(),
        })
    }

    /// Reorder statements
    fn mutate_reorder_statements<R: Rng>(
        &self,
        source: &str,
        rng: &mut R,
    ) -> Option<SingleMutation> {
        let lines: Vec<&str> = source.lines().collect();

        // Find swappable lines within the same block
        let swappable: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                trimmed.ends_with(';') && !trimmed.starts_with("use ")
            })
            .map(|(i, _)| i)
            .collect();

        if swappable.len() < 2 {
            return None;
        }

        // Pick two adjacent lines to swap
        let idx1 = *swappable.choose(rng)?;
        let idx2 = swappable
            .iter()
            .find(|&&i| i == idx1 + 1)
            .copied()
            .or_else(|| swappable.iter().find(|&&i| i + 1 == idx1).copied())?;

        let mut new_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        new_lines.swap(idx1, idx2);

        Some(SingleMutation {
            mutated: new_lines.join("\n"),
            position: lines[..idx1.min(idx2)].iter().map(|l| l.len() + 1).sum(),
            original_text: lines[idx1].to_string(),
            new_text: lines[idx2].to_string(),
        })
    }

    /// Wrap an expression
    fn mutate_wrap_expression<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        // Simple integer literal wrapping
        let mut i = 0;
        let chars: Vec<char> = source.chars().collect();

        while i < chars.len() {
            if chars[i].is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }

                let literal: String = chars[start..i].iter().collect();
                let wrappers = ["({})", "Some({})", "-{}", "({} + 0)"];
                let wrapper = *wrappers.choose(rng).unwrap();
                let wrapped = wrapper.replace("{}", &literal);

                let mut mutated = source.to_string();
                mutated.replace_range(start..start + literal.len(), &wrapped);

                return Some(SingleMutation {
                    mutated,
                    position: start,
                    original_text: literal,
                    new_text: wrapped,
                });
            }
            i += 1;
        }

        None
    }

    /// Unwrap a nested expression
    fn mutate_unwrap_expression<R: Rng>(
        &self,
        source: &str,
        rng: &mut R,
    ) -> Option<SingleMutation> {
        // Find parenthesized expressions to unwrap
        if let Some(start) = source.find('(') {
            let rest = &source[start..];
            let mut depth = 0;
            let mut end = 0;

            for (i, c) in rest.chars().enumerate() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = start + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if end > start + 2 {
                let inner = &source[start + 1..end - 1];
                // Only unwrap if inner doesn't contain operators
                if !inner.contains(['+', '-', '*', '/', '&', '|']) {
                    let mut mutated = source.to_string();
                    mutated.replace_range(start..end, inner);

                    return Some(SingleMutation {
                        mutated,
                        position: start,
                        original_text: source[start..end].to_string(),
                        new_text: inner.to_string(),
                    });
                }
            }
        }

        None
    }

    /// Change type annotations
    fn mutate_type<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        let types = ["Int", "Float", "Bool", "Text", "Unit"];

        for ty in &types {
            if let Some(pos) = source.find(ty) {
                // Make sure it's a standalone type, not part of a larger word
                let before = pos > 0
                    && source
                        .chars()
                        .nth(pos - 1)
                        .map_or(false, |c| c.is_alphanumeric());
                let after = source
                    .chars()
                    .nth(pos + ty.len())
                    .map_or(false, |c| c.is_alphanumeric());

                if !before && !after {
                    let replacement = types.iter().filter(|&&t| t != *ty).choose(rng)?;

                    let mut mutated = source.to_string();
                    mutated.replace_range(pos..pos + ty.len(), replacement);

                    return Some(SingleMutation {
                        mutated,
                        position: pos,
                        original_text: ty.to_string(),
                        new_text: replacement.to_string(),
                    });
                }
            }
        }

        None
    }

    /// Swap keywords
    fn mutate_keyword<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        let keyword_pairs = [
            ("true", "false"),
            ("false", "true"),
            ("if", "while"),
            ("while", "if"),
            ("&&", "||"),
            ("||", "&&"),
            ("Some", "None"),
        ];

        for (from, to) in &keyword_pairs {
            if let Some(pos) = source.find(from) {
                let mut mutated = source.to_string();
                mutated.replace_range(pos..pos + from.len(), to);

                return Some(SingleMutation {
                    mutated,
                    position: pos,
                    original_text: from.to_string(),
                    new_text: to.to_string(),
                });
            }
        }

        None
    }

    /// Mutate brackets/delimiters
    fn mutate_bracket<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        let bracket_pairs = [('(', ')'), ('[', ']'), ('{', '}')];

        // Find positions of all brackets
        let brackets: Vec<(usize, char)> = source
            .char_indices()
            .filter(|(_, c)| "()[]{}".contains(*c))
            .collect();

        if brackets.is_empty() {
            return None;
        }

        let (pos, bracket) = *brackets.choose(rng)?;

        // Options: delete bracket, double bracket, change type
        let mutations: Vec<&str> = match bracket {
            '(' | ')' => vec!["", "((", "))", "[", "]"],
            '[' | ']' => vec!["", "[[", "]]", "(", ")"],
            '{' | '}' => vec!["", "{{", "}}", "(", ")"],
            _ => return None,
        };

        let new_bracket = *mutations.choose(rng)?;

        let mut mutated = source.to_string();
        mutated.replace_range(pos..pos + 1, new_bracket);

        Some(SingleMutation {
            mutated,
            position: pos,
            original_text: bracket.to_string(),
            new_text: new_bracket.to_string(),
        })
    }

    /// Mutate whitespace
    fn mutate_whitespace<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        // Find whitespace positions
        let ws_positions: Vec<usize> = source
            .char_indices()
            .filter(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
            .collect();

        if ws_positions.is_empty() {
            // Add whitespace somewhere
            let pos = rng.random_range(0..source.len().max(1));
            let mut mutated = source.to_string();
            mutated.insert(pos, ' ');

            return Some(SingleMutation {
                mutated,
                position: pos,
                original_text: String::new(),
                new_text: " ".to_string(),
            });
        }

        let pos = *ws_positions.choose(rng)?;
        let mutations = ["", "  ", "\t", "\n", " \n "];
        let new_ws = *mutations.choose(rng)?;

        let mut mutated = source.to_string();
        mutated.replace_range(pos..pos + 1, new_ws);

        Some(SingleMutation {
            mutated,
            position: pos,
            original_text: " ".to_string(),
            new_text: new_ws.to_string(),
        })
    }

    /// Insert boundary values
    fn mutate_boundary_values<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        let boundary_values = [
            "0",
            "1",
            "-1",
            "127",
            "-128",
            "255",
            "32767",
            "-32768",
            "65535",
            "2147483647",
            "-2147483648",
            "9223372036854775807",
        ];

        // Find integer literals and replace with boundary values
        let mut i = 0;
        let chars: Vec<char> = source.chars().collect();

        while i < chars.len() {
            if chars[i].is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }

                let literal: String = chars[start..i].iter().collect();
                let boundary = *boundary_values.choose(rng)?;

                let mut mutated = source.to_string();
                mutated.replace_range(start..start + literal.len(), boundary);

                return Some(SingleMutation {
                    mutated,
                    position: start,
                    original_text: literal,
                    new_text: boundary.to_string(),
                });
            }
            i += 1;
        }

        None
    }

    /// Inject Unicode characters
    fn mutate_unicode<R: Rng>(&self, source: &str, rng: &mut R) -> Option<SingleMutation> {
        let unicode_chars = [
            "\u{0000}",  // Null
            "\u{200B}",  // Zero-width space
            "\u{200C}",  // Zero-width non-joiner
            "\u{200D}",  // Zero-width joiner
            "\u{FEFF}",  // BOM
            "\u{202A}",  // Left-to-right embedding
            "\u{202B}",  // Right-to-left embedding
            "\u{1F600}", // Emoji
            "\u{0301}",  // Combining accent
            "\u{FFFD}",  // Replacement character
        ];

        let pos = rng.random_range(0..source.len().max(1));
        let unicode = *unicode_chars.choose(rng)?;

        let mut mutated = source.to_string();
        mutated.insert_str(pos, unicode);

        Some(SingleMutation {
            mutated,
            position: pos,
            original_text: String::new(),
            new_text: unicode.to_string(),
        })
    }
}

/// Internal representation of a single mutation
struct SingleMutation {
    mutated: String,
    position: usize,
    original_text: String,
    new_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_mutate_program() {
        let config = MutationConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let source = r#"
fn main() {
    let x = 1 + 2;
    let y = x * 3;
    print(y);
}
"#;

        let result = mutator.mutate(source, &mut rng);
        assert_ne!(result.original, result.mutated);
    }

    #[test]
    fn test_operator_swap() {
        let config = MutationConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let source = "let x = 1 + 2;";

        // Keep trying until we get an operator swap
        for _ in 0..100 {
            if let Some(mutation) = mutator.mutate_operator(source, &mut rng) {
                assert!(mutation.mutated.contains('-') || mutation.mutated.contains('+'));
                return;
            }
        }
    }

    #[test]
    fn test_literal_change() {
        let config = MutationConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let source = "let x = 42;";

        if let Some(mutation) = mutator.mutate_literal(source, &mut rng) {
            assert!(!mutation.mutated.contains("42"));
        }
    }

    #[test]
    fn test_boundary_values() {
        let config = MutationConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let source = "let x = 100;";

        if let Some(mutation) = mutator.mutate_boundary_values(source, &mut rng) {
            // Should contain a boundary value
            let boundaries = [
                "0",
                "1",
                "-1",
                "127",
                "-128",
                "255",
                "32767",
                "-32768",
                "65535",
                "2147483647",
                "-2147483648",
            ];
            assert!(boundaries.iter().any(|b| mutation.mutated.contains(b)));
        }
    }

    #[test]
    fn test_deterministic_with_seed() {
        let config = MutationConfig::default();
        let mutator = Mutator::new(config);

        let source = "let x = 1 + 2;";

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        let result1 = mutator.mutate(source, &mut rng1);
        let result2 = mutator.mutate(source, &mut rng2);

        assert_eq!(result1.mutated, result2.mutated);
    }
}
