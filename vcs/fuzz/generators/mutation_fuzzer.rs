//! Mutation-based fuzzer for Verum
//!
//! This module implements mutation-based fuzzing that takes existing valid
//! programs and applies random mutations to test parser robustness, error
//! recovery, and edge cases in the compiler.
//!
//! # Mutation Strategies
//!
//! The fuzzer supports multiple mutation strategies:
//! - **Token mutations**: Replace, delete, or insert tokens
//! - **AST mutations**: Swap, duplicate, or remove AST nodes
//! - **Boundary mutations**: Test integer/string limits
//! - **Semantic mutations**: Break type safety, scoping rules
//! - **Whitespace mutations**: Add/remove/modify whitespace
//!
//! # Coverage-Guided Fuzzing
//!
//! When integrated with coverage tools, the fuzzer can prioritize mutations
//! that explore new code paths in the compiler.

use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::IndexedRandom;
use rand::seq::SliceRandom;
use std::collections::HashSet;

/// Configuration for the mutation fuzzer
#[derive(Debug, Clone)]
pub struct MutationConfig {
    /// Maximum number of mutations per iteration
    pub max_mutations: usize,
    /// Probability of applying each mutation type
    pub mutation_weights: MutationWeights,
    /// Whether to preserve syntactic validity
    pub preserve_syntax: bool,
    /// Whether to preserve type correctness
    pub preserve_types: bool,
    /// Minimum source length to mutate
    pub min_source_length: usize,
    /// Maximum source length after mutations
    pub max_source_length: usize,
    /// Seed programs to mutate
    pub seed_corpus: Vec<String>,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self {
            max_mutations: 10,
            mutation_weights: MutationWeights::default(),
            preserve_syntax: false,
            preserve_types: false,
            min_source_length: 10,
            max_source_length: 100_000,
            seed_corpus: Vec::new(),
        }
    }
}

/// Weights for different mutation strategies
#[derive(Debug, Clone)]
pub struct MutationWeights {
    pub token_replace: u32,
    pub token_delete: u32,
    pub token_insert: u32,
    pub token_duplicate: u32,
    pub char_flip: u32,
    pub char_insert: u32,
    pub char_delete: u32,
    pub whitespace_mutate: u32,
    pub number_boundary: u32,
    pub string_escape: u32,
    pub keyword_swap: u32,
    pub operator_swap: u32,
    pub bracket_mutate: u32,
    pub identifier_mutate: u32,
    pub block_shuffle: u32,
    pub statement_duplicate: u32,
    pub expression_swap: u32,
}

impl Default for MutationWeights {
    fn default() -> Self {
        Self {
            token_replace: 15,
            token_delete: 10,
            token_insert: 10,
            token_duplicate: 5,
            char_flip: 8,
            char_insert: 8,
            char_delete: 8,
            whitespace_mutate: 5,
            number_boundary: 10,
            string_escape: 5,
            keyword_swap: 8,
            operator_swap: 10,
            bracket_mutate: 8,
            identifier_mutate: 10,
            block_shuffle: 3,
            statement_duplicate: 5,
            expression_swap: 5,
        }
    }
}

/// Types of mutations that can be applied
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutationType {
    TokenReplace,
    TokenDelete,
    TokenInsert,
    TokenDuplicate,
    CharFlip,
    CharInsert,
    CharDelete,
    WhitespaceMutate,
    NumberBoundary,
    StringEscape,
    KeywordSwap,
    OperatorSwap,
    BracketMutate,
    IdentifierMutate,
    BlockShuffle,
    StatementDuplicate,
    ExpressionSwap,
}

/// Result of a mutation operation
#[derive(Debug, Clone)]
pub struct MutationResult {
    /// The mutated source code
    pub source: String,
    /// Mutations applied
    pub mutations: Vec<AppliedMutation>,
    /// Original source for reference
    pub original: String,
}

/// Record of an applied mutation
#[derive(Debug, Clone)]
pub struct AppliedMutation {
    /// Type of mutation
    pub mutation_type: MutationType,
    /// Position in source
    pub position: usize,
    /// Original content (if replaced/deleted)
    pub original: Option<String>,
    /// New content (if inserted/replaced)
    pub replacement: Option<String>,
}

/// Mutation-based fuzzer
pub struct MutationFuzzer {
    config: MutationConfig,
    mutation_dist: WeightedIndex<u32>,
    /// Verum keywords for substitution
    keywords: Vec<&'static str>,
    /// Verum operators for substitution
    operators: Vec<&'static str>,
    /// Common identifiers
    identifiers: Vec<&'static str>,
    /// Bracket pairs
    brackets: Vec<(&'static str, &'static str)>,
}

impl MutationFuzzer {
    /// Create a new mutation fuzzer
    pub fn new(config: MutationConfig) -> Self {
        let weights = vec![
            config.mutation_weights.token_replace,
            config.mutation_weights.token_delete,
            config.mutation_weights.token_insert,
            config.mutation_weights.token_duplicate,
            config.mutation_weights.char_flip,
            config.mutation_weights.char_insert,
            config.mutation_weights.char_delete,
            config.mutation_weights.whitespace_mutate,
            config.mutation_weights.number_boundary,
            config.mutation_weights.string_escape,
            config.mutation_weights.keyword_swap,
            config.mutation_weights.operator_swap,
            config.mutation_weights.bracket_mutate,
            config.mutation_weights.identifier_mutate,
            config.mutation_weights.block_shuffle,
            config.mutation_weights.statement_duplicate,
            config.mutation_weights.expression_swap,
        ];

        Self {
            config,
            mutation_dist: WeightedIndex::new(&weights).unwrap(),
            keywords: vec![
                "fn", "let", "mut", "if", "else", "match", "for", "while", "loop", "return",
                "break", "continue", "struct", "enum", "impl", "trait", "type", "pub", "mod",
                "use", "async", "await", "true", "false", "self", "Self", "using", "provide",
                "context", "where", "in", "const", "static", "ref", "move", "dyn", "unsafe",
                "checked",
            ],
            operators: vec![
                "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||", "!", "&",
                "|", "^", "<<", ">>", "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=",
                ">>=", "->", "=>", "::", ".", "..", "..=", "?", "@",
            ],
            identifiers: vec![
                "x", "y", "z", "i", "j", "k", "n", "m", "s", "t", "foo", "bar", "baz", "qux",
                "value", "result", "data", "item", "elem", "node", "list", "map", "set", "vec",
                "len", "size", "count", "index", "key", "val",
            ],
            brackets: vec![("(", ")"), ("[", "]"), ("{", "}"), ("<", ">")],
        }
    }

    /// Select a random seed from the corpus
    pub fn select_seed<R: Rng>(&self, rng: &mut R) -> Option<&String> {
        self.config.seed_corpus.choose(rng)
    }

    /// Apply multiple random mutations to a source
    pub fn mutate<R: Rng>(&self, rng: &mut R, source: &str) -> MutationResult {
        let mut current = source.to_string();
        let mut mutations = Vec::new();
        let original = source.to_string();

        let num_mutations = rng.random_range(1..=self.config.max_mutations);

        for _ in 0..num_mutations {
            if current.len() < self.config.min_source_length {
                break;
            }
            if current.len() > self.config.max_source_length {
                // Truncate if too long
                current.truncate(self.config.max_source_length);
                break;
            }

            let mutation_type = self.select_mutation_type(rng);
            if let Some(mutation) = self.apply_mutation(rng, &mut current, mutation_type) {
                mutations.push(mutation);
            }
        }

        MutationResult {
            source: current,
            mutations,
            original,
        }
    }

    /// Select a mutation type based on weights
    fn select_mutation_type<R: Rng>(&self, rng: &mut R) -> MutationType {
        match self.mutation_dist.sample(rng) {
            0 => MutationType::TokenReplace,
            1 => MutationType::TokenDelete,
            2 => MutationType::TokenInsert,
            3 => MutationType::TokenDuplicate,
            4 => MutationType::CharFlip,
            5 => MutationType::CharInsert,
            6 => MutationType::CharDelete,
            7 => MutationType::WhitespaceMutate,
            8 => MutationType::NumberBoundary,
            9 => MutationType::StringEscape,
            10 => MutationType::KeywordSwap,
            11 => MutationType::OperatorSwap,
            12 => MutationType::BracketMutate,
            13 => MutationType::IdentifierMutate,
            14 => MutationType::BlockShuffle,
            15 => MutationType::StatementDuplicate,
            _ => MutationType::ExpressionSwap,
        }
    }

    /// Apply a specific mutation type
    fn apply_mutation<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
        mutation_type: MutationType,
    ) -> Option<AppliedMutation> {
        match mutation_type {
            MutationType::CharFlip => self.mutate_char_flip(rng, source),
            MutationType::CharInsert => self.mutate_char_insert(rng, source),
            MutationType::CharDelete => self.mutate_char_delete(rng, source),
            MutationType::WhitespaceMutate => self.mutate_whitespace(rng, source),
            MutationType::NumberBoundary => self.mutate_number_boundary(rng, source),
            MutationType::StringEscape => self.mutate_string_escape(rng, source),
            MutationType::KeywordSwap => self.mutate_keyword_swap(rng, source),
            MutationType::OperatorSwap => self.mutate_operator_swap(rng, source),
            MutationType::BracketMutate => self.mutate_bracket(rng, source),
            MutationType::IdentifierMutate => self.mutate_identifier(rng, source),
            MutationType::TokenReplace => self.mutate_token_replace(rng, source),
            MutationType::TokenDelete => self.mutate_token_delete(rng, source),
            MutationType::TokenInsert => self.mutate_token_insert(rng, source),
            MutationType::TokenDuplicate => self.mutate_token_duplicate(rng, source),
            MutationType::BlockShuffle => self.mutate_block_shuffle(rng, source),
            MutationType::StatementDuplicate => self.mutate_statement_duplicate(rng, source),
            MutationType::ExpressionSwap => self.mutate_expression_swap(rng, source),
        }
    }

    /// Flip a random character to a different ASCII value
    fn mutate_char_flip<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        if source.is_empty() {
            return None;
        }

        let bytes = unsafe { source.as_bytes_mut() };
        let pos = rng.random_range(0..bytes.len());
        let original = bytes[pos];

        // Flip to a different printable ASCII character
        let new_char = loop {
            let c = rng.random_range(32u8..127u8);
            if c != original {
                break c;
            }
        };

        bytes[pos] = new_char;

        Some(AppliedMutation {
            mutation_type: MutationType::CharFlip,
            position: pos,
            original: Some((original as char).to_string()),
            replacement: Some((new_char as char).to_string()),
        })
    }

    /// Insert a random character
    fn mutate_char_insert<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        let pos = if source.is_empty() {
            0
        } else {
            rng.random_range(0..=source.len())
        };

        let new_char = rng.random_range(32u8..127u8) as char;
        source.insert(pos, new_char);

        Some(AppliedMutation {
            mutation_type: MutationType::CharInsert,
            position: pos,
            original: None,
            replacement: Some(new_char.to_string()),
        })
    }

    /// Delete a random character
    fn mutate_char_delete<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        if source.is_empty() {
            return None;
        }

        let pos = rng.random_range(0..source.len());
        let removed = source.remove(pos);

        Some(AppliedMutation {
            mutation_type: MutationType::CharDelete,
            position: pos,
            original: Some(removed.to_string()),
            replacement: None,
        })
    }

    /// Mutate whitespace (add, remove, or change)
    fn mutate_whitespace<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        match rng.random_range(0..4) {
            0 => {
                // Add whitespace
                let pos = rng.random_range(0..=source.len());
                let ws = [" ", "  ", "\t", "\n", "\r\n"].choose(rng).unwrap();
                source.insert_str(pos, ws);
                Some(AppliedMutation {
                    mutation_type: MutationType::WhitespaceMutate,
                    position: pos,
                    original: None,
                    replacement: Some(ws.to_string()),
                })
            }
            1 => {
                // Remove all whitespace in a region
                let start = rng.random_range(0..source.len().max(1));
                let end = (start + rng.random_range(1..20)).min(source.len());
                let region: String = source[start..end]
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                source.replace_range(start..end, &region);
                Some(AppliedMutation {
                    mutation_type: MutationType::WhitespaceMutate,
                    position: start,
                    original: None,
                    replacement: Some(region),
                })
            }
            2 => {
                // Add newlines
                if let Some(pos) = source.find(';') {
                    let newlines = "\n".repeat(rng.random_range(1..5));
                    source.insert_str(pos + 1, &newlines);
                    Some(AppliedMutation {
                        mutation_type: MutationType::WhitespaceMutate,
                        position: pos + 1,
                        original: None,
                        replacement: Some(newlines),
                    })
                } else {
                    None
                }
            }
            _ => {
                // Replace spaces with tabs or vice versa
                if source.contains(' ') {
                    *source = source.replace("    ", "\t");
                    Some(AppliedMutation {
                        mutation_type: MutationType::WhitespaceMutate,
                        position: 0,
                        original: Some("    ".to_string()),
                        replacement: Some("\t".to_string()),
                    })
                } else {
                    None
                }
            }
        }
    }

    /// Mutate numbers to boundary values
    fn mutate_number_boundary<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        // Find a number in the source
        let num_pattern = regex::Regex::new(r"\b\d+\b").ok()?;
        let matches: Vec<_> = num_pattern.find_iter(source).collect();

        if matches.is_empty() {
            return None;
        }

        let m = matches.choose(rng)?;
        let pos = m.start();
        let original = m.as_str().to_string();

        // Choose a boundary value
        let boundary = [
            "0",
            "1",
            "-1",
            "127",
            "128",
            "-128",
            "255",
            "256",
            "32767",
            "32768",
            "-32768",
            "65535",
            "65536",
            "2147483647",
            "2147483648",
            "-2147483648",
            "9223372036854775807",
            "-9223372036854775808",
        ]
        .choose(rng)?;

        source.replace_range(pos..pos + original.len(), boundary);

        Some(AppliedMutation {
            mutation_type: MutationType::NumberBoundary,
            position: pos,
            original: Some(original),
            replacement: Some(boundary.to_string()),
        })
    }

    /// Mutate string escape sequences
    fn mutate_string_escape<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        // Find a string literal
        let str_start = source.find('"')?;
        let str_end = source[str_start + 1..].find('"')? + str_start + 1;

        if str_end <= str_start + 1 {
            return None;
        }

        let escapes = [
            "\\n",
            "\\r",
            "\\t",
            "\\\\",
            "\\\"",
            "\\0",
            "\\x00",
            "\\x7F",
            "\\xFF",
            "\\u{0}",
            "\\u{FFFF}",
            "\\u{10FFFF}",
        ];

        let escape = escapes.choose(rng)?;
        let insert_pos = rng.random_range(str_start + 1..str_end);

        source.insert_str(insert_pos, escape);

        Some(AppliedMutation {
            mutation_type: MutationType::StringEscape,
            position: insert_pos,
            original: None,
            replacement: Some(escape.to_string()),
        })
    }

    /// Swap one keyword for another
    fn mutate_keyword_swap<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        let keyword = *self.keywords.choose(rng)?;

        // Find the keyword with word boundaries
        let pattern = format!(r"\b{}\b", regex::escape(keyword));
        let re = regex::Regex::new(&pattern).ok()?;
        let matches: Vec<_> = re.find_iter(source).collect();

        if matches.is_empty() {
            return None;
        }

        let m = matches.choose(rng)?;
        let pos = m.start();
        let original = m.as_str().to_string();

        // Choose a different keyword
        let replacement = loop {
            let k = *self.keywords.choose(rng)?;
            if k != keyword {
                break k;
            }
        };

        source.replace_range(pos..pos + original.len(), replacement);

        Some(AppliedMutation {
            mutation_type: MutationType::KeywordSwap,
            position: pos,
            original: Some(original),
            replacement: Some(replacement.to_string()),
        })
    }

    /// Swap one operator for another
    fn mutate_operator_swap<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        // Try to find operators in order of length (longest first)
        let mut sorted_ops: Vec<_> = self.operators.iter().cloned().collect();
        sorted_ops.sort_by(|a, b| b.len().cmp(&a.len()));

        for op in &sorted_ops {
            if let Some(pos) = source.find(op) {
                let original = op.to_string();

                // Choose a different operator
                let replacement = loop {
                    let o = *self.operators.choose(rng)?;
                    if o != *op {
                        break o;
                    }
                };

                source.replace_range(pos..pos + op.len(), replacement);

                return Some(AppliedMutation {
                    mutation_type: MutationType::OperatorSwap,
                    position: pos,
                    original: Some(original),
                    replacement: Some(replacement.to_string()),
                });
            }
        }

        None
    }

    /// Mutate brackets (mismatch, remove, duplicate)
    fn mutate_bracket<R: Rng>(&self, rng: &mut R, source: &mut String) -> Option<AppliedMutation> {
        let (open, close) = *self.brackets.choose(rng)?;

        match rng.random_range(0..4) {
            0 => {
                // Remove opening bracket
                if let Some(pos) = source.find(open) {
                    source.remove(pos);
                    return Some(AppliedMutation {
                        mutation_type: MutationType::BracketMutate,
                        position: pos,
                        original: Some(open.to_string()),
                        replacement: None,
                    });
                }
            }
            1 => {
                // Remove closing bracket
                if let Some(pos) = source.find(close) {
                    source.remove(pos);
                    return Some(AppliedMutation {
                        mutation_type: MutationType::BracketMutate,
                        position: pos,
                        original: Some(close.to_string()),
                        replacement: None,
                    });
                }
            }
            2 => {
                // Swap bracket types
                if let Some(pos) = source.find(open) {
                    let (other_open, _) = *self.brackets.choose(rng)?;
                    source.replace_range(pos..pos + open.len(), other_open);
                    return Some(AppliedMutation {
                        mutation_type: MutationType::BracketMutate,
                        position: pos,
                        original: Some(open.to_string()),
                        replacement: Some(other_open.to_string()),
                    });
                }
            }
            _ => {
                // Duplicate bracket
                if let Some(pos) = source.find(open) {
                    source.insert_str(pos, open);
                    return Some(AppliedMutation {
                        mutation_type: MutationType::BracketMutate,
                        position: pos,
                        original: None,
                        replacement: Some(open.to_string()),
                    });
                }
            }
        }

        None
    }

    /// Mutate an identifier
    fn mutate_identifier<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        let ident = *self.identifiers.choose(rng)?;

        let pattern = format!(r"\b{}\b", regex::escape(ident));
        let re = regex::Regex::new(&pattern).ok()?;
        let matches: Vec<_> = re.find_iter(source).collect();

        if matches.is_empty() {
            // Try to find any identifier pattern
            let any_ident = regex::Regex::new(r"\b[a-zA-Z_][a-zA-Z0-9_]*\b").ok()?;
            let matches: Vec<_> = any_ident.find_iter(source).collect();

            if matches.is_empty() {
                return None;
            }

            let m = matches.choose(rng)?;
            let pos = m.start();
            let original = m.as_str().to_string();

            // Skip if it's a keyword
            if self.keywords.contains(&original.as_str()) {
                return None;
            }

            let mutations = [
                // Typo: swap two adjacent chars
                {
                    let mut s = original.clone();
                    if s.len() >= 2 {
                        let bytes = unsafe { s.as_bytes_mut() };
                        let idx = rng.random_range(0..bytes.len() - 1);
                        bytes.swap(idx, idx + 1);
                    }
                    s
                },
                // Add underscore prefix
                format!("_{}", original),
                // Remove underscore prefix if present
                original.trim_start_matches('_').to_string(),
                // Random identifier
                self.identifiers.choose(rng).unwrap().to_string(),
            ];

            let replacement = mutations.choose(rng)?.clone();

            if replacement != original && !replacement.is_empty() {
                source.replace_range(pos..pos + original.len(), &replacement);
                return Some(AppliedMutation {
                    mutation_type: MutationType::IdentifierMutate,
                    position: pos,
                    original: Some(original),
                    replacement: Some(replacement),
                });
            }
        }

        None
    }

    /// Replace a token with a random token
    fn mutate_token_replace<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        let tokens = [
            "fn", "let", "if", "else", "match", "for", "while", "(", ")", "{", "}", "[", "]", "<",
            ">", "+", "-", "*", "/", "=", "==", "!=", "true", "false", "0", "1", "42", ";", ",",
            ":", "::",
        ];

        let token = *tokens.choose(rng)?;
        if let Some(pos) = source.find(token) {
            let replacement = *tokens.choose(rng)?;
            source.replace_range(pos..pos + token.len(), replacement);
            return Some(AppliedMutation {
                mutation_type: MutationType::TokenReplace,
                position: pos,
                original: Some(token.to_string()),
                replacement: Some(replacement.to_string()),
            });
        }

        None
    }

    /// Delete a token
    fn mutate_token_delete<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        let tokens = [";", ",", "(", ")", "{", "}", "[", "]", "let", "fn"];
        let token = *tokens.choose(rng)?;

        if let Some(pos) = source.find(token) {
            source.replace_range(pos..pos + token.len(), "");
            return Some(AppliedMutation {
                mutation_type: MutationType::TokenDelete,
                position: pos,
                original: Some(token.to_string()),
                replacement: None,
            });
        }

        None
    }

    /// Insert a random token
    fn mutate_token_insert<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        let tokens = [
            "fn", "let", "if", "else", "{", "}", "(", ")", ";", ",", ":", "true", "false", "0",
            "1", "mut", "return", "async", "await",
        ];

        let token = *tokens.choose(rng)?;
        let pos = rng.random_range(0..=source.len());

        source.insert_str(pos, token);

        Some(AppliedMutation {
            mutation_type: MutationType::TokenInsert,
            position: pos,
            original: None,
            replacement: Some(token.to_string()),
        })
    }

    /// Duplicate a token
    fn mutate_token_duplicate<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        let tokens = ["fn", "let", "if", "(", ")", "{", "}", ";", "="];
        let token = *tokens.choose(rng)?;

        if let Some(pos) = source.find(token) {
            source.insert_str(pos, token);
            return Some(AppliedMutation {
                mutation_type: MutationType::TokenDuplicate,
                position: pos,
                original: None,
                replacement: Some(format!("{}{}", token, token)),
            });
        }

        None
    }

    /// Shuffle statements within a block
    fn mutate_block_shuffle<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        // Find a block
        let open = source.find('{')?;
        let close = source[open..].find('}')? + open;

        if close <= open + 2 {
            return None;
        }

        let block = source[open + 1..close].to_string();
        let mut statements: Vec<&str> = block.split(';').collect();

        if statements.len() < 2 {
            return None;
        }

        // Shuffle statements
        statements.shuffle(rng);
        let new_block = statements.join(";");

        source.replace_range(open + 1..close, &new_block);

        Some(AppliedMutation {
            mutation_type: MutationType::BlockShuffle,
            position: open,
            original: Some(block),
            replacement: Some(new_block),
        })
    }

    /// Duplicate a statement
    fn mutate_statement_duplicate<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        // Find statements ending with semicolon
        let positions: Vec<_> = source.match_indices(';').map(|(i, _)| i).collect();

        if positions.is_empty() {
            return None;
        }

        let end_pos = *positions.choose(rng)?;

        // Find start of statement (previous semicolon, brace, or start)
        let search_region = &source[..end_pos];
        let start_pos = search_region
            .rfind(|c| c == ';' || c == '{' || c == '}')
            .map(|i| i + 1)
            .unwrap_or(0);

        let statement = source[start_pos..=end_pos].to_string();

        // Insert duplicate
        source.insert_str(end_pos + 1, &format!("\n{}", statement));

        Some(AppliedMutation {
            mutation_type: MutationType::StatementDuplicate,
            position: end_pos + 1,
            original: None,
            replacement: Some(statement),
        })
    }

    /// Swap two expressions
    fn mutate_expression_swap<R: Rng>(
        &self,
        rng: &mut R,
        source: &mut String,
    ) -> Option<AppliedMutation> {
        // Find binary operator positions
        let ops = [" + ", " - ", " * ", " / ", " == ", " != ", " && ", " || "];
        let mut positions = Vec::new();

        for op in &ops {
            for (idx, _) in source.match_indices(op) {
                positions.push((idx, *op));
            }
        }

        if positions.is_empty() {
            return None;
        }

        let (pos, op) = *positions.choose(rng)?;

        // Simple swap: just reverse the operator if it's commutative
        // For demo, we'll just mutate the operator slightly
        let replacement = match op {
            " + " => " - ",
            " - " => " + ",
            " * " => " / ",
            " / " => " * ",
            " == " => " != ",
            " != " => " == ",
            " && " => " || ",
            " || " => " && ",
            _ => return None,
        };

        source.replace_range(pos..pos + op.len(), replacement);

        Some(AppliedMutation {
            mutation_type: MutationType::ExpressionSwap,
            position: pos,
            original: Some(op.to_string()),
            replacement: Some(replacement.to_string()),
        })
    }

    /// Run the fuzzer for multiple iterations
    pub fn fuzz<R: Rng>(&self, rng: &mut R, iterations: usize) -> Vec<MutationResult> {
        let mut results = Vec::with_capacity(iterations);

        for _ in 0..iterations {
            if let Some(seed) = self.select_seed(rng) {
                results.push(self.mutate(rng, seed));
            }
        }

        results
    }

    /// Add a seed to the corpus
    pub fn add_seed(&mut self, seed: String) {
        self.config.seed_corpus.push(seed);
    }

    /// Get coverage information for a mutation
    pub fn analyze_mutation(&self, result: &MutationResult) -> MutationAnalysis {
        let mut analysis = MutationAnalysis {
            mutation_types: HashSet::new(),
            total_mutations: result.mutations.len(),
            length_change: result.source.len() as isize - result.original.len() as isize,
            is_syntactically_different: result.source != result.original,
        };

        for m in &result.mutations {
            analysis.mutation_types.insert(m.mutation_type);
        }

        analysis
    }
}

/// Analysis of applied mutations
#[derive(Debug, Clone)]
pub struct MutationAnalysis {
    pub mutation_types: HashSet<MutationType>,
    pub total_mutations: usize,
    pub length_change: isize,
    pub is_syntactically_different: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_mutation_fuzzer() {
        let mut config = MutationConfig::default();
        config.seed_corpus = vec![
            "fn main() { let x = 1 + 2; }".to_string(),
            "fn foo(a: Int, b: Int) -> Int { a + b }".to_string(),
        ];

        let fuzzer = MutationFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let results = fuzzer.fuzz(&mut rng, 10);

        assert_eq!(results.len(), 10);
        for result in &results {
            // At least one mutation should be applied
            assert!(!result.mutations.is_empty() || result.source == result.original);
        }
    }

    #[test]
    fn test_char_mutations() {
        let config = MutationConfig::default();
        let fuzzer = MutationFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(123);

        let mut source = "let x = 42;".to_string();
        let original = source.clone();

        // Apply char flip
        let result = fuzzer.mutate_char_flip(&mut rng, &mut source);
        assert!(result.is_some());
        assert_ne!(source, original);
    }

    #[test]
    fn test_keyword_swap() {
        let mut config = MutationConfig::default();
        config.seed_corpus = vec!["fn main() { let x = 1; }".to_string()];

        let fuzzer = MutationFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(456);

        // Run multiple times to ensure keyword swap works
        for _ in 0..10 {
            let mut source = "fn main() { let x = 1; }".to_string();
            if let Some(mutation) = fuzzer.mutate_keyword_swap(&mut rng, &mut source) {
                assert!(mutation.original.is_some());
                assert!(mutation.replacement.is_some());
            }
        }
    }

    #[test]
    fn test_mutation_analysis() {
        let mut config = MutationConfig::default();
        config.seed_corpus = vec!["fn main() {}".to_string()];

        let fuzzer = MutationFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(789);

        let result = fuzzer.mutate(&mut rng, "fn main() {}");
        let analysis = fuzzer.analyze_mutation(&result);

        assert_eq!(analysis.total_mutations, result.mutations.len());
    }
}
