//! Staged Compilation Pipeline for N-Level Metaprogramming
//!
//! This module implements the multi-stage compilation pipeline for Verum's
//! N-level staged metaprogramming system. It orchestrates the execution of
//! staged functions from highest stage to lowest.
//!
//! # Staged Metaprogramming Model
//!
//! Verum supports N-level staged metaprogramming where functions execute at
//! different compilation stages:
//!
//! ```text
//! Stage N   ──►  generates  ──►  Stage N-1  ──►  ...  ──►  Stage 0 (runtime)
//! (meta(N))                       (meta(N-1))              (normal code)
//! ```
//!
//! ## Stage Semantics
//!
//! | Stage | Syntax | Execution | Description |
//! |-------|--------|-----------|-------------|
//! | 0 | `fn f()` | Runtime | Normal runtime functions |
//! | 1 | `meta fn f()` | Compile-time | Standard meta functions |
//! | 2 | `meta(2) fn f()` | Pre-compile | Generates meta functions |
//! | N | `meta(N) fn f()` | Stage N | Generates Stage N-1 code |
//!
//! ## Stage Coherence Rule
//!
//! The fundamental rule of staged metaprogramming:
//!
//! > **A Stage N function can only DIRECTLY generate Stage N-1 code.**
//!
//! This means:
//! - `meta(2)` can only directly generate `meta` (stage 1) code
//! - To generate runtime (stage 0) code from `meta(2)`, the output must contain
//!   a `meta` function that performs the final generation
//! - Each `quote { ... }` lowers the stage by 1
//!
//! # Compilation Flow
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │                     StagedPipeline                               │
//! ├──────────────────────────────────────────────────────────────────┤
//! │                                                                  │
//! │  Input: Module with meta(N), meta(N-1), ..., meta, fn           │
//! │                                                                  │
//! │  Stage N:                                                        │
//! │    1. Collect all meta(N) functions                             │
//! │    2. Execute meta(N) functions → TokenStream                   │
//! │    3. Parse TokenStream → Stage N-1 AST fragments               │
//! │    4. Inject into codebase                                       │
//! │                                                                  │
//! │  Stage N-1:                                                      │
//! │    1. Collect all meta(N-1) functions (including generated)     │
//! │    2. Execute meta(N-1) functions → TokenStream                 │
//! │    3. Parse TokenStream → Stage N-2 AST fragments               │
//! │    4. Inject into codebase                                       │
//! │                                                                  │
//! │  ...repeat until Stage 1...                                      │
//! │                                                                  │
//! │  Stage 1 (meta):                                                 │
//! │    1. Execute meta functions → runtime code                     │
//! │    2. All code is now Stage 0 (runtime)                         │
//! │                                                                  │
//! │  Output: Pure runtime code (Stage 0)                            │
//! │                                                                  │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```verum
//! // Stage 2: Generates stage 1 code
//! meta(2) fn derive_factory<T>() -> TokenStream {
//!     quote {
//!         meta fn derive_impl() -> TokenStream {
//!             quote {
//!                 impl Factory for @T {
//!                     fn create() -> Self { Self::default() }
//!                 }
//!             }
//!         }
//!     }
//! }
//!
//! // Invocation at stage 2
//! @derive_factory<MyType>()
//!
//! // After stage 2 execution, this is injected:
//! meta fn derive_impl() -> TokenStream {
//!     quote {
//!         impl Factory for MyType {
//!             fn create() -> Self { Self::default() }
//!         }
//!     }
//! }
//!
//! // After stage 1 execution:
//! impl Factory for MyType {
//!     fn create() -> Self { Self::default() }
//! }
//! ```
//!
//! # Integration
//!
//! The StagedPipeline integrates with:
//! - **verum_types/stage_checker**: Validates stage constraints
//! - **verum_compiler/meta_registry**: Stores meta functions per stage
//! - **verum_compiler/lint**: Emits E1001-E1005 and W1001-W1002 diagnostics
//! - **verum_vbc/interpreter**: Executes meta functions

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::hash::{ChangeKind, ContentHash, ItemHashes, compute_item_hashes_from_module};

use anyhow::Result;
use tracing::{debug, info, trace, warn};
use verum_ast::{Item, Module, Span, decl::ItemKind};
use verum_common::{List, Map, Set, Text};
use verum_diagnostics::Diagnostic;
use verum_lexer::Token;
use verum_types::{StageChecker, StageConfig, StageError};

use crate::lint::{LintConfig, StagedMetaDiagnostics};
use crate::meta::MetaRegistry;
use crate::meta::vbc_executor::VbcExecutor;
use crate::quote::TokenStream;

/// Serialize a list of tokens to source code for caching.
///
/// This converts tokens back to their textual representation so they can
/// be stored and later reparsed. This is used for staged pipeline caching.
fn serialize_tokens_to_source(tokens: &List<Token>) -> String {
    use verum_lexer::TokenKind;

    let mut result = String::new();
    let mut prev_needs_space = false;

    for token in tokens.iter() {
        let token_str = match &token.kind {
            TokenKind::Ident(s) => s.to_string(),
            TokenKind::Integer(lit) => lit.raw_value.to_string(),
            TokenKind::Float(lit) => format!("{}", lit.value),
            TokenKind::Text(s) => format!("\"{}\"", s),
            TokenKind::Char(c) => format!("'{}'", c),
            TokenKind::ByteChar(b) => format!("b'{}'", *b as char),
            TokenKind::ByteString(bytes) => format!("b\"{}\"", String::from_utf8_lossy(bytes)),
            TokenKind::True => "true".to_string(),
            TokenKind::False => "false".to_string(),

            // Keywords
            TokenKind::Let => "let".to_string(),
            TokenKind::Fn => "fn".to_string(),
            TokenKind::Is => "is".to_string(),
            TokenKind::If => "if".to_string(),
            TokenKind::Else => "else".to_string(),
            TokenKind::Match => "match".to_string(),
            TokenKind::For => "for".to_string(),
            TokenKind::While => "while".to_string(),
            TokenKind::Loop => "loop".to_string(),
            TokenKind::Return => "return".to_string(),
            TokenKind::Break => "break".to_string(),
            TokenKind::Continue => "continue".to_string(),
            TokenKind::Type => "type".to_string(),
            TokenKind::Mount => "mount".to_string(),
            TokenKind::Pub => "pub".to_string(),
            TokenKind::Mut => "mut".to_string(),
            TokenKind::Async => "async".to_string(),
            TokenKind::Await => "await".to_string(),
            TokenKind::Where => "where".to_string(),
            TokenKind::In => "in".to_string(),
            TokenKind::As => "as".to_string(),

            // Operators and punctuation
            TokenKind::Plus => "+".to_string(),
            TokenKind::Minus => "-".to_string(),
            TokenKind::Star => "*".to_string(),
            TokenKind::Slash => "/".to_string(),
            TokenKind::Percent => "%".to_string(),
            TokenKind::Caret => "^".to_string(),
            TokenKind::Ampersand => "&".to_string(),
            TokenKind::Pipe => "|".to_string(),
            TokenKind::AmpersandAmpersand => "&&".to_string(),
            TokenKind::PipePipe => "||".to_string(),
            TokenKind::Eq => "=".to_string(),
            TokenKind::EqEq => "==".to_string(),
            TokenKind::BangEq => "!=".to_string(),
            TokenKind::Lt => "<".to_string(),
            TokenKind::LtEq => "<=".to_string(),
            TokenKind::Gt => ">".to_string(),
            TokenKind::GtEq => ">=".to_string(),
            TokenKind::RArrow => "->".to_string(),
            TokenKind::FatArrow => "=>".to_string(),
            TokenKind::Dot => ".".to_string(),
            TokenKind::DotDot => "..".to_string(),
            TokenKind::DotDotEq => "..=".to_string(),
            TokenKind::Comma => ",".to_string(),
            TokenKind::Colon => ":".to_string(),
            TokenKind::ColonColon => "::".to_string(),
            TokenKind::Semicolon => ";".to_string(),
            TokenKind::Question => "?".to_string(),
            TokenKind::At => "@".to_string(),
            TokenKind::Hash => "#".to_string(),
            TokenKind::Tilde => "~".to_string(),
            TokenKind::Bang => "!".to_string(),

            // Delimiters
            TokenKind::LParen => "(".to_string(),
            TokenKind::RParen => ")".to_string(),
            TokenKind::LBracket => "[".to_string(),
            TokenKind::RBracket => "]".to_string(),
            TokenKind::LBrace => "{".to_string(),
            TokenKind::RBrace => "}".to_string(),

            // Context/provide tokens
            TokenKind::Context => "context".to_string(),
            TokenKind::Provide => "provide".to_string(),
            TokenKind::Using => "using".to_string(),

            // Block comments are skipped in serialization
            // Note: whitespace/line comments are already skipped by logos
            TokenKind::BlockComment => {
                continue;
            }
            TokenKind::Eof => break,
            TokenKind::Error => continue, // Skip error tokens

            // Default: use debug representation for uncommon tokens
            _ => format!("{:?}", token.kind),
        };

        // Add space between tokens that need it
        let needs_leading_space = matches!(
            token.kind,
            TokenKind::Ident(_)
                | TokenKind::Integer(_)
                | TokenKind::Float(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Let
                | TokenKind::Fn
                | TokenKind::Is
                | TokenKind::If
                | TokenKind::Else
                | TokenKind::Match
                | TokenKind::For
                | TokenKind::While
                | TokenKind::Loop
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Type
                | TokenKind::Mount
                | TokenKind::Pub
                | TokenKind::Mut
                | TokenKind::Async
                | TokenKind::Await
                | TokenKind::Where
                | TokenKind::In
                | TokenKind::As
        );

        if prev_needs_space && needs_leading_space {
            result.push(' ');
        }

        result.push_str(&token_str);
        prev_needs_space = needs_leading_space;
    }

    result
}

/// Extract function call dependencies from a function declaration's body.
fn extract_call_dependencies(func_decl: &verum_ast::decl::FunctionDecl) -> List<Text> {
    use verum_ast::{Block, ConditionKind, Expr, ExprKind, StmtKind};
    use verum_ast::decl::FunctionBody;

    fn deps_block(block: &Block, deps: &mut Set<Text>) {
        for stmt in block.stmts.iter() {
            match &stmt.kind {
                StmtKind::Let { value: verum_common::Maybe::Some(expr), .. } => deps_expr(expr, deps),
                StmtKind::LetElse { value, .. } => deps_expr(value, deps),
                StmtKind::Expr { expr, .. } => deps_expr(expr, deps),
                StmtKind::Defer(expr) => deps_expr(expr, deps),
                StmtKind::Item(item) => {
                    if let ItemKind::Function(ref f) = item.kind {
                        if let verum_common::Maybe::Some(ref body) = f.body {
                            match body {
                                FunctionBody::Block(b) => deps_block(b, deps),
                                FunctionBody::Expr(e) => deps_expr(e, deps),
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if let verum_common::Maybe::Some(tail) = &block.expr {
            deps_expr(tail, deps);
        }
    }

    fn deps_expr(expr: &Expr, deps: &mut Set<Text>) {
        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    deps.insert(Text::from(format!("{}", path)));
                }
                deps_expr(func, deps);
                for a in args.iter() { deps_expr(a, deps); }
            }
            ExprKind::MethodCall { receiver, method, args, .. } => {
                deps.insert(Text::from(method.name.as_str()));
                deps_expr(receiver, deps);
                for a in args.iter() { deps_expr(a, deps); }
            }
            ExprKind::Binary { left, right, .. } | ExprKind::Pipeline { left, right } => {
                deps_expr(left, deps); deps_expr(right, deps);
            }
            ExprKind::Unary { expr: inner, .. }
            | ExprKind::Field { expr: inner, .. }
            | ExprKind::Try(inner)
            | ExprKind::TryBlock(inner)
            | ExprKind::Cast { expr: inner, .. }
            | ExprKind::Closure { body: inner, .. } => deps_expr(inner, deps),
            ExprKind::Block(block) => deps_block(block, deps),
            ExprKind::If { condition, then_branch, else_branch, .. } => {
                for c in condition.conditions.iter() {
                    match c {
                        ConditionKind::Expr(e) => deps_expr(e, deps),
                        ConditionKind::Let { value, .. } => deps_expr(value, deps),
                    }
                }
                deps_block(then_branch, deps);
                if let verum_common::Maybe::Some(el) = else_branch { deps_expr(el, deps); }
            }
            ExprKind::Match { expr: scrutinee, arms, .. } => {
                deps_expr(scrutinee, deps);
                for arm in arms.iter() {
                    deps_expr(&arm.body, deps);
                    if let verum_common::Maybe::Some(g) = &arm.guard { deps_expr(g, deps); }
                }
            }
            ExprKind::Return(verum_common::Maybe::Some(inner)) => deps_expr(inner, deps),
            ExprKind::Index { expr: base, index, .. } => {
                deps_expr(base, deps); deps_expr(index, deps);
            }
            ExprKind::Tuple(elems) => {
                for e in elems.iter() { deps_expr(e, deps); }
            }
            ExprKind::While { condition, body, .. } => {
                deps_expr(condition, deps); deps_block(body, deps);
            }
            ExprKind::Loop { body, .. } => deps_block(body, deps),
            ExprKind::For { iter, body, .. } => {
                deps_expr(iter, deps); deps_block(body, deps);
            }
            _ => {}
        }
    }

    let mut deps = Set::<Text>::new();
    if let verum_common::Maybe::Some(ref body) = func_decl.body {
        match body {
            FunctionBody::Block(block) => deps_block(block, &mut deps),
            FunctionBody::Expr(expr) => deps_expr(expr, &mut deps),
        }
    }
    deps.into_iter().collect()
}

/// Analyze a function body to determine the minimum stage level required.
///
/// Returns the minimum stage needed based on compile-time constructs:
/// - `quote { ... }` requires at least stage 1
/// - `StageEscape { stage: N, .. }` requires at least stage N + 1
/// - `MacroCall` requires at least stage 1
/// - `Lift` requires at least stage 1
fn analyze_minimum_stage(func_decl: &verum_ast::decl::FunctionDecl) -> u32 {
    use verum_ast::{Block, ConditionKind, Expr, ExprKind, StmtKind};
    use verum_ast::decl::FunctionBody;

    fn stage_block(block: &Block) -> u32 {
        let mut max = 0u32;
        for stmt in block.stmts.iter() {
            let s = match &stmt.kind {
                StmtKind::Let { value: verum_common::Maybe::Some(expr), .. } => stage_expr(expr),
                StmtKind::LetElse { value, .. } => stage_expr(value),
                StmtKind::Expr { expr, .. } => stage_expr(expr),
                StmtKind::Defer(expr) => stage_expr(expr),
                _ => 0,
            };
            max = max.max(s);
        }
        if let verum_common::Maybe::Some(tail) = &block.expr {
            max = max.max(stage_expr(tail));
        }
        max
    }

    fn stage_expr(expr: &Expr) -> u32 {
        match &expr.kind {
            ExprKind::Quote { target_stage, .. } => target_stage.map_or(1, |s| s + 1),
            ExprKind::StageEscape { stage, expr: inner } => (*stage + 1).max(stage_expr(inner)),
            ExprKind::Lift { expr: inner } => 1u32.max(stage_expr(inner)),
            ExprKind::MacroCall { .. } => 1,
            ExprKind::Call { func, args, .. } => {
                let mut m = stage_expr(func);
                for a in args.iter() { m = m.max(stage_expr(a)); }
                m
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                let mut m = stage_expr(receiver);
                for a in args.iter() { m = m.max(stage_expr(a)); }
                m
            }
            ExprKind::Binary { left, right, .. } | ExprKind::Pipeline { left, right } => {
                stage_expr(left).max(stage_expr(right))
            }
            ExprKind::Unary { expr: inner, .. }
            | ExprKind::Field { expr: inner, .. }
            | ExprKind::Try(inner)
            | ExprKind::TryBlock(inner)
            | ExprKind::Cast { expr: inner, .. }
            | ExprKind::Closure { body: inner, .. } => stage_expr(inner),
            ExprKind::Block(block) => stage_block(block),
            ExprKind::If { condition, then_branch, else_branch, .. } => {
                let mut m = 0u32;
                for c in condition.conditions.iter() {
                    match c {
                        ConditionKind::Expr(e) => m = m.max(stage_expr(e)),
                        ConditionKind::Let { value, .. } => m = m.max(stage_expr(value)),
                    }
                }
                m = m.max(stage_block(then_branch));
                if let verum_common::Maybe::Some(e) = else_branch { m = m.max(stage_expr(e)); }
                m
            }
            ExprKind::Match { expr: scrutinee, arms, .. } => {
                let mut m = stage_expr(scrutinee);
                for arm in arms.iter() {
                    m = m.max(stage_expr(&arm.body));
                    if let verum_common::Maybe::Some(g) = &arm.guard { m = m.max(stage_expr(g)); }
                }
                m
            }
            ExprKind::Return(verum_common::Maybe::Some(inner)) => stage_expr(inner),
            ExprKind::Index { expr: base, index, .. } => stage_expr(base).max(stage_expr(index)),
            ExprKind::While { condition, body, .. } => stage_expr(condition).max(stage_block(body)),
            ExprKind::Loop { body, .. } => stage_block(body),
            ExprKind::For { iter, body, .. } => stage_expr(iter).max(stage_block(body)),
            _ => 0,
        }
    }

    if let verum_common::Maybe::Some(ref body) = func_decl.body {
        match body {
            FunctionBody::Block(block) => stage_block(block),
            FunctionBody::Expr(expr) => stage_expr(expr),
        }
    } else {
        0
    }
}

/// Configuration for staged compilation.
#[derive(Debug, Clone)]
pub struct StagedConfig {
    /// Maximum allowed stage level (default: 2).
    ///
    /// Higher stages require more compilation passes but enable more
    /// sophisticated metaprogramming patterns.
    pub max_stage: u32,

    /// Whether to enable stage-aware caching.
    ///
    /// When enabled, each stage's output is cached separately, allowing
    /// incremental recompilation when only certain stages change.
    pub enable_caching: bool,

    /// Whether to emit unused stage warnings (W1001).
    pub warn_unused_stages: bool,

    /// Whether to emit stage downgrade suggestions (W1002).
    pub suggest_stage_downgrade: bool,

    /// Lint configuration for staged meta diagnostics.
    pub lint_config: LintConfig,
}

impl Default for StagedConfig {
    fn default() -> Self {
        Self {
            max_stage: 2,
            enable_caching: true,
            warn_unused_stages: true,
            suggest_stage_downgrade: true,
            lint_config: LintConfig::default(),
        }
    }
}

/// Cache for a single compilation stage.
///
/// Stores the results of executing meta functions at a specific stage,
/// allowing incremental recompilation when inputs haven't changed.
///
/// # Cache Invalidation Strategy
///
/// The cache uses a multi-level invalidation strategy:
///
/// 1. **Hash-based**: If the input AST hash changes, cache is invalid
/// 2. **Fine-grained**: Uses ItemHashes to distinguish signature vs body changes
///    - Signature change → full re-execution required
///    - Body-only change → dependents need re-verification, not re-execution
/// 3. **Dependency-based**: If any dependency file changes, cache is invalid
/// 4. **Stage cascade**: Invalidating stage N invalidates all stages < N
/// 5. **Time-based**: Optional TTL for long-running compilations
///
/// # Fragment Storage
///
/// Generated code is stored as serialized token streams, enabling:
/// - Fast retrieval without re-execution
/// - Dependency tracking per fragment
/// - Partial cache reuse when only some functions change
#[derive(Debug, Clone)]
pub struct StageCache {
    /// The stage level this cache is for.
    stage: u32,

    /// Hash of the input AST for this stage.
    input_hash: u64,

    /// Fine-grained item hashes for signature vs body change detection.
    /// Used to determine if only function bodies changed (which may not
    /// require full re-execution of dependent meta functions).
    item_hashes: Option<ItemHashes>,

    /// Cached generated code fragments indexed by function identifier.
    generated_fragments: Map<Text, GeneratedFragment>,

    /// File dependencies that affect this cache.
    /// If any of these files change, cache is invalidated.
    file_dependencies: Set<Text>,

    /// Hash of dependency file contents at cache creation time.
    dependency_hashes: Map<Text, u64>,

    /// Functions that were executed to generate cache.
    executed_functions: Set<Text>,

    /// Timestamp when cache was created (Unix epoch millis).
    created_at_ms: u64,

    /// Whether this cache is valid.
    valid: bool,

    /// Cache hit count for statistics.
    hit_count: u64,

    /// Cache miss count for statistics.
    miss_count: u64,
}

impl StageCache {
    /// Create a new empty cache for the given stage.
    pub fn new(stage: u32) -> Self {
        Self {
            stage,
            input_hash: 0,
            item_hashes: None,
            generated_fragments: Map::new(),
            file_dependencies: Set::new(),
            dependency_hashes: Map::new(),
            executed_functions: Set::new(),
            created_at_ms: Self::current_timestamp_ms(),
            valid: false,
            hit_count: 0,
            miss_count: 0,
        }
    }

    /// Get current timestamp in milliseconds.
    fn current_timestamp_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Get the stage level this cache is for.
    pub fn stage(&self) -> u32 {
        self.stage
    }

    /// Get cache hit count.
    pub fn hit_count(&self) -> u64 {
        self.hit_count
    }

    /// Get cache miss count.
    pub fn miss_count(&self) -> u64 {
        self.miss_count
    }

    /// Invalidate this cache.
    pub fn invalidate(&mut self) {
        self.valid = false;
        self.item_hashes = None;
        self.generated_fragments.clear();
        self.file_dependencies.clear();
        self.dependency_hashes.clear();
        self.executed_functions.clear();
    }

    /// Check if the cache is valid for the given input hash.
    pub fn is_valid_for(&self, input_hash: u64) -> bool {
        self.valid && self.input_hash == input_hash
    }

    /// Check if cache is valid considering all factors.
    ///
    /// # Arguments
    /// - `input_hash`: Hash of the current input AST
    /// - `ttl_ms`: Optional time-to-live in milliseconds (0 = no TTL)
    pub fn is_valid_with_ttl(&self, input_hash: u64, ttl_ms: u64) -> bool {
        if !self.valid || self.input_hash != input_hash {
            return false;
        }

        // Check TTL if specified
        if ttl_ms > 0 {
            let age = Self::current_timestamp_ms().saturating_sub(self.created_at_ms);
            if age > ttl_ms {
                return false;
            }
        }

        true
    }

    /// Record a cache hit.
    pub fn record_hit(&mut self) {
        self.hit_count += 1;
    }

    /// Record a cache miss.
    pub fn record_miss(&mut self) {
        self.miss_count += 1;
    }

    /// Update the cache with new generated fragments.
    pub fn update(&mut self, input_hash: u64, fragments: Map<Text, GeneratedFragment>) {
        self.input_hash = input_hash;
        self.generated_fragments = fragments;
        self.created_at_ms = Self::current_timestamp_ms();
        self.valid = true;
    }

    /// Update cache with fragments and dependencies.
    pub fn update_with_dependencies(
        &mut self,
        input_hash: u64,
        fragments: Map<Text, GeneratedFragment>,
        dependencies: Set<Text>,
        dependency_hashes: Map<Text, u64>,
        executed_functions: Set<Text>,
    ) {
        self.input_hash = input_hash;
        self.generated_fragments = fragments;
        self.file_dependencies = dependencies;
        self.dependency_hashes = dependency_hashes;
        self.executed_functions = executed_functions;
        self.created_at_ms = Self::current_timestamp_ms();
        self.valid = true;
    }

    /// Add a file dependency to this cache.
    pub fn add_dependency(&mut self, path: Text, content_hash: u64) {
        self.file_dependencies.insert(path.clone());
        self.dependency_hashes.insert(path, content_hash);
    }

    /// Check if dependencies have changed.
    ///
    /// Returns true if any dependency file's hash has changed from
    /// what was recorded when cache was created.
    ///
    /// # Arguments
    /// - `current_hashes`: Function that returns current hash for a file path
    pub fn dependencies_changed<F>(&self, current_hashes: F) -> bool
    where
        F: Fn(&Text) -> Option<u64>,
    {
        for (path, recorded_hash) in self.dependency_hashes.iter() {
            match current_hashes(path) {
                Some(current_hash) if current_hash != *recorded_hash => {
                    trace!(
                        "Dependency {} changed: {} -> {}",
                        path,
                        recorded_hash,
                        current_hash
                    );
                    return true;
                }
                None => {
                    // Dependency file no longer exists
                    trace!("Dependency {} no longer exists", path);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// Get a cached fragment by function identifier.
    pub fn get_fragment(&self, func_id: &Text) -> Option<&GeneratedFragment> {
        self.generated_fragments.get(func_id)
    }

    /// Get all cached fragments.
    pub fn fragments(&self) -> &Map<Text, GeneratedFragment> {
        &self.generated_fragments
    }

    /// Get executed function identifiers.
    pub fn executed_functions(&self) -> &Set<Text> {
        &self.executed_functions
    }

    /// Get the stored input hash.
    pub fn input_hash(&self) -> u64 {
        self.input_hash
    }

    /// Check if a specific function was executed in this cache.
    pub fn was_function_executed(&self, func_id: &Text) -> bool {
        self.executed_functions.contains(func_id)
    }

    /// Get cache statistics.
    pub fn statistics(&self) -> StageCacheStatistics {
        StageCacheStatistics {
            stage: self.stage,
            fragment_count: self.generated_fragments.len(),
            dependency_count: self.file_dependencies.len(),
            hit_count: self.hit_count,
            miss_count: self.miss_count,
            age_ms: Self::current_timestamp_ms().saturating_sub(self.created_at_ms),
            valid: self.valid,
        }
    }

    /// Get the stored item hashes.
    pub fn item_hashes(&self) -> Option<&ItemHashes> {
        self.item_hashes.as_ref()
    }

    /// Store item hashes for fine-grained comparison.
    pub fn set_item_hashes(&mut self, hashes: ItemHashes) {
        self.item_hashes = Some(hashes);
    }

    /// Compare current item hashes with cached hashes to determine change kind.
    ///
    /// Returns:
    /// - `ChangeKind::NoChange` if cache is valid and nothing changed
    /// - `ChangeKind::BodyOnly` if only function bodies changed (not signatures)
    /// - `ChangeKind::Signature` if any signature changed (requires full re-execution)
    ///
    /// # Arguments
    /// - `current_hashes`: The item hashes computed from the current module
    pub fn compare_item_hashes(&self, current_hashes: &ItemHashes) -> ChangeKind {
        match &self.item_hashes {
            Some(cached_hashes) => cached_hashes.compare(current_hashes),
            None => ChangeKind::Signature, // No cached hashes means full rebuild needed
        }
    }

    /// Check if cache is valid using fine-grained item hash comparison.
    ///
    /// This extends the basic hash check with fine-grained comparison that can
    /// detect when only function bodies changed (not requiring full re-execution).
    ///
    /// Returns a tuple of (is_valid, change_kind):
    /// - (true, NoChange) - cache is fully valid, use cached results
    /// - (false, BodyOnly) - only bodies changed, may use partial cache
    /// - (false, Signature) - signatures changed, full re-execution needed
    pub fn is_valid_fine_grained(&self, input_hash: u64, current_hashes: &ItemHashes) -> (bool, ChangeKind) {
        // First check basic validity
        if !self.valid {
            return (false, ChangeKind::Signature);
        }

        // If input hash differs, do fine-grained comparison
        if self.input_hash != input_hash {
            let change_kind = self.compare_item_hashes(current_hashes);
            match change_kind {
                ChangeKind::NoChange => {
                    // Hash changed but items are identical - accept cache
                    // (This can happen with non-deterministic hashing)
                    (true, ChangeKind::NoChange)
                }
                ChangeKind::BodyOnly => {
                    // Only bodies changed - cache is partially valid
                    // Meta functions that only depend on signatures can use cache
                    (false, ChangeKind::BodyOnly)
                }
                ChangeKind::Signature => {
                    // Signatures changed - full rebuild needed
                    (false, ChangeKind::Signature)
                }
            }
        } else {
            // Hash matches exactly - cache is valid
            (true, ChangeKind::NoChange)
        }
    }
}

/// Statistics for a single stage cache.
#[derive(Debug, Clone)]
pub struct StageCacheStatistics {
    /// Stage level.
    pub stage: u32,

    /// Number of cached fragments.
    pub fragment_count: usize,

    /// Number of file dependencies.
    pub dependency_count: usize,

    /// Cache hit count.
    pub hit_count: u64,

    /// Cache miss count.
    pub miss_count: u64,

    /// Age of cache in milliseconds.
    pub age_ms: u64,

    /// Whether cache is currently valid.
    pub valid: bool,
}

/// A generated code fragment from a meta function execution.
#[derive(Debug, Clone)]
pub struct GeneratedFragment {
    /// The meta function that generated this fragment.
    pub source_function: Text,

    /// The source module path.
    pub source_module: Text,

    /// The stage at which this was generated.
    pub generated_at_stage: u32,

    /// The target stage of the generated code.
    pub target_stage: u32,

    /// The generated code as source text (can be reparsed).
    /// This contains the actual generated code, not just a placeholder.
    pub code: Text,

    /// Source span for error reporting.
    pub span: Option<Span>,

    /// Serialized AST items (JSON) for direct injection without reparsing.
    /// This is optional - if present, items are deserialized directly.
    /// If absent, the code field is reparsed.
    pub serialized_items: Option<Text>,

    /// Number of items that were generated.
    pub item_count: usize,
}

/// Information about a staged function.
#[derive(Debug, Clone)]
pub struct StagedFunction {
    /// Function name.
    pub name: Text,

    /// Module where defined.
    pub module: Text,

    /// Stage level (0 = runtime, 1+ = meta stages).
    pub stage: u32,

    /// Whether this function was generated by a higher stage.
    pub is_generated: bool,

    /// Functions this one depends on (for ordering).
    pub dependencies: List<Text>,

    /// Invocation count (for unused detection).
    pub invocation_count: u32,

    /// Minimum stage level required by the function body's constructs.
    pub min_required_stage: u32,

    /// Source span.
    pub span: Span,
}

/// Result of staged compilation.
#[derive(Debug, Clone)]
pub struct StagedResult {
    /// Final stage 0 (runtime) code.
    pub runtime_code: Module,

    /// Diagnostics generated during staged compilation.
    pub diagnostics: List<Diagnostic>,

    /// Statistics about the compilation.
    pub stats: StagedStats,
}

/// Statistics about staged compilation.
#[derive(Debug, Clone, Default)]
pub struct StagedStats {
    /// Number of stages processed.
    pub stages_processed: u32,

    /// Functions executed per stage.
    pub functions_per_stage: HashMap<u32, u32>,

    /// Total meta functions executed.
    pub total_meta_executions: u32,

    /// Cache hits per stage.
    pub cache_hits: HashMap<u32, u32>,

    /// Cache misses per stage.
    pub cache_misses: HashMap<u32, u32>,

    /// Total cached fragments per stage.
    pub cached_fragments: HashMap<u32, u32>,

    /// Time spent per stage (nanoseconds).
    pub stage_time_ns: HashMap<u32, u64>,

    /// Total time spent in staged compilation (nanoseconds).
    pub total_time_ns: u64,
}

impl StagedStats {
    /// Calculate overall cache hit rate.
    pub fn cache_hit_rate(&self) -> f64 {
        let hits: u32 = self.cache_hits.values().sum();
        let misses: u32 = self.cache_misses.values().sum();
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get total functions executed across all stages.
    pub fn total_functions(&self) -> u32 {
        self.functions_per_stage.values().sum()
    }

    /// Get cache hit count for a specific stage.
    pub fn hits_for_stage(&self, stage: u32) -> u32 {
        *self.cache_hits.get(&stage).unwrap_or(&0)
    }

    /// Get cache miss count for a specific stage.
    pub fn misses_for_stage(&self, stage: u32) -> u32 {
        *self.cache_misses.get(&stage).unwrap_or(&0)
    }

    /// Format as human-readable summary.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("Staged Compilation Statistics:"));
        lines.push(format!("  Stages processed: {}", self.stages_processed));
        lines.push(format!("  Total meta executions: {}", self.total_meta_executions));
        lines.push(format!("  Cache hit rate: {:.1}%", self.cache_hit_rate() * 100.0));
        lines.push(format!(
            "  Total time: {:.2}ms",
            self.total_time_ns as f64 / 1_000_000.0
        ));

        if !self.functions_per_stage.is_empty() {
            lines.push(format!("  Per-stage breakdown:"));
            for stage in 1..=self.stages_processed {
                let funcs = *self.functions_per_stage.get(&stage).unwrap_or(&0);
                let hits = *self.cache_hits.get(&stage).unwrap_or(&0);
                let time_ns = *self.stage_time_ns.get(&stage).unwrap_or(&0);
                lines.push(format!(
                    "    Stage {}: {} functions, {} cache hits, {:.2}ms",
                    stage,
                    funcs,
                    hits,
                    time_ns as f64 / 1_000_000.0
                ));
            }
        }

        lines.join("\n")
    }
}

/// Multi-stage compilation pipeline.
///
/// Orchestrates the execution of N-level staged metaprogramming, compiling
/// from highest stage down to runtime code.
pub struct StagedPipeline {
    /// Configuration for staged compilation.
    config: StagedConfig,

    /// Stage caches (one per stage level).
    stage_caches: Vec<StageCache>,

    /// Meta registries (one per stage level).
    stage_registries: Vec<MetaRegistry>,

    /// Stage checker for validation.
    stage_checker: StageChecker,

    /// VBC executor for meta function execution.
    vbc_executor: VbcExecutor,

    /// Collected diagnostics.
    diagnostics: List<Diagnostic>,

    /// Staged function information.
    functions: Map<(Text, Text), StagedFunction>,

    /// Statistics.
    stats: StagedStats,
}

impl StagedPipeline {
    /// Create a new staged pipeline with the given configuration.
    pub fn new(config: StagedConfig) -> Self {
        let max_stage = config.max_stage;
        let stage_config = StageConfig {
            max_stage,
            ..Default::default()
        };

        // Initialize caches and registries for each stage
        let mut stage_caches = Vec::with_capacity(max_stage as usize + 1);
        let mut stage_registries = Vec::with_capacity(max_stage as usize + 1);

        for stage in 0..=max_stage {
            stage_caches.push(StageCache::new(stage));
            stage_registries.push(MetaRegistry::new());
        }

        Self {
            config,
            stage_caches,
            stage_registries,
            stage_checker: StageChecker::new(stage_config),
            vbc_executor: VbcExecutor::new(),
            diagnostics: List::new(),
            functions: Map::new(),
            stats: StagedStats::default(),
        }
    }

    /// Create with default configuration.
    pub fn default() -> Self {
        Self::new(StagedConfig::default())
    }

    /// Get the maximum stage level.
    pub fn max_stage(&self) -> u32 {
        self.config.max_stage
    }

    /// Get the stage checker for external validation.
    pub fn stage_checker(&self) -> &StageChecker {
        &self.stage_checker
    }

    /// Get the meta registry for a specific stage.
    pub fn registry_for_stage(&self, stage: u32) -> Option<&MetaRegistry> {
        self.stage_registries.get(stage as usize)
    }

    /// Get mutable meta registry for a specific stage.
    pub fn registry_for_stage_mut(&mut self, stage: u32) -> Option<&mut MetaRegistry> {
        self.stage_registries.get_mut(stage as usize)
    }

    /// Import meta functions from an external MetaRegistry.
    ///
    /// This method stores meta functions from the pipeline's MetaRegistry into
    /// StagedPipeline's internal storage for execution during staged compilation.
    /// Stage levels are determined from the module AST.
    ///
    /// # Arguments
    ///
    /// * `external_registry` - The MetaRegistry from the compilation pipeline
    /// * `module` - The module AST to extract stage levels from
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut staged = StagedPipeline::new(StagedConfig::default());
    /// staged.import_from_registry(&pipeline.meta_registry, &module);
    /// let result = staged.compile(module)?;
    /// ```
    pub fn import_from_registry(&mut self, external_registry: &MetaRegistry, module: &Module) {
        // Phase 1: Build a map from function name to stage level from the module AST.
        // This is necessary because MetaFunction doesn't store stage_level directly;
        // it's derived from the FunctionDecl.stage_level in the AST.
        let mut stage_levels: HashMap<Text, u32> = HashMap::new();
        let module_path = Text::from(format!("module_{}", module.file_id.raw()));

        for item in module.items.iter() {
            if let ItemKind::Function(ref func_decl) = item.kind {
                let stage = func_decl.stage_level;
                if stage > 0 {
                    // Key by both name and module for proper scoping
                    let key = Text::from(format!("{}::{}", module_path, func_decl.name.as_str()));
                    stage_levels.insert(key, stage);
                    // Also store by name alone for fallback lookup
                    stage_levels.insert(Text::from(func_decl.name.as_str()), stage);
                }
            }
        }

        // Phase 2: Import all meta functions with proper stage routing.
        // Each function is registered in its corresponding stage registry.
        let all_meta_fns = external_registry.all_meta_functions();
        let mut stats_by_stage: HashMap<u32, u32> = HashMap::new();
        let mut total_imported = 0u32;
        let mut skipped_overflow = 0u32;

        for meta_fn in all_meta_fns.iter() {
            // Determine stage level for this function.
            // Try qualified name first, then unqualified name, default to stage 1.
            let qualified_key = Text::from(format!("{}::{}", meta_fn.module, meta_fn.name));
            let stage = stage_levels
                .get(&qualified_key)
                .or_else(|| stage_levels.get(&meta_fn.name))
                .copied()
                .unwrap_or(1); // Default to stage 1 for standard meta functions

            // Validate stage against max_stage
            if stage > self.config.max_stage {
                warn!(
                    "Meta function '{}::{}' has stage {} exceeding max_stage {}, skipping",
                    meta_fn.module, meta_fn.name, stage, self.config.max_stage
                );
                skipped_overflow += 1;
                continue;
            }

            // Ensure we have a registry for this stage
            if let Some(registry) = self.stage_registries.get_mut(stage as usize) {
                // Register the meta function in the appropriate stage registry
                match registry.register_meta_fn_direct(meta_fn.clone()) {
                    Ok(()) => {
                        *stats_by_stage.entry(stage).or_insert(0) += 1;
                        total_imported += 1;
                        trace!(
                            "Imported '{}::{}' into stage {} registry",
                            meta_fn.module, meta_fn.name, stage
                        );
                    }
                    Err(e) => {
                        // Log duplicate registration (can happen with re-exported functions)
                        debug!(
                            "Skipped duplicate meta function '{}::{}': {}",
                            meta_fn.module, meta_fn.name, e
                        );
                    }
                }
            } else {
                warn!(
                    "No registry available for stage {} (max_stage={}), skipping '{}::{}'",
                    stage, self.config.max_stage, meta_fn.module, meta_fn.name
                );
            }
        }

        // Phase 3: Also import macros for proper @derive/@attribute resolution
        let all_macros = external_registry.all_macros();
        let mut macros_imported = 0u32;

        for macro_def in all_macros.iter() {
            // Macros are typically stage 1 (standard compile-time)
            if let Some(registry) = self.stage_registries.get_mut(1) {
                match registry.register_macro(
                    &macro_def.module,
                    macro_def.name.clone(),
                    macro_def.kind,
                    macro_def.expander.clone(),
                    macro_def.span,
                ) {
                    Ok(()) => {
                        macros_imported += 1;
                        trace!(
                            "Imported macro '{}::{}' into stage 1 registry",
                            macro_def.module, macro_def.name
                        );
                    }
                    Err(e) => {
                        debug!(
                            "Skipped duplicate macro '{}::{}': {}",
                            macro_def.module, macro_def.name, e
                        );
                    }
                }
            }
        }

        // Phase 4: Copy module dependencies for proper cross-module resolution
        // (This ensures that imported meta functions can resolve calls to other modules)
        // Note: Dependencies are already in external_registry, we need to copy them
        // For now, we rely on the module-level dependency tracking in stage_registries

        // Log import summary
        let stage_summary: String = stats_by_stage
            .iter()
            .map(|(stage, count)| format!("stage {}={}", stage, count))
            .collect::<Vec<_>>()
            .join(", ");

        info!(
            "Imported {} meta functions ({}) and {} macros from external registry{}",
            total_imported,
            if stage_summary.is_empty() { "none".to_string() } else { stage_summary },
            macros_imported,
            if skipped_overflow > 0 {
                format!(" (skipped {} overflow)", skipped_overflow)
            } else {
                String::new()
            }
        );
    }

    /// Reset the pipeline state while keeping configuration.
    ///
    /// This clears all accumulated state (functions, diagnostics, caches, stats)
    /// while preserving the configuration. Use this when processing multiple
    /// modules sequentially with the same pipeline instance.
    pub fn reset(&mut self) {
        // Clear accumulated state
        self.functions.clear();
        self.diagnostics.clear();
        self.stats = StagedStats::default();

        // Reset stage checker
        self.stage_checker = StageChecker::new(StageConfig {
            max_stage: self.config.max_stage,
            ..Default::default()
        });

        // Clear stage registries (reinitialize with empty registries)
        for registry in &mut self.stage_registries {
            *registry = MetaRegistry::new();
        }

        // Note: We intentionally do NOT clear stage_caches to preserve
        // cached fragments across module compilations for better hit rates.
        // Cache invalidation is handled by fine-grained hash comparison.

        debug!("StagedPipeline reset (caches preserved)");
    }

    /// Register a staged function.
    pub fn register_function(&mut self, func: StagedFunction) -> Result<()> {
        // Validate stage level
        if func.stage > self.config.max_stage {
            let diag = StagedMetaDiagnostics::new(&self.config.lint_config);
            self.diagnostics.push(diag.stage_overflow(
                func.stage,
                self.config.max_stage,
                &func.name,
                Some(func.span),
            ));
            return Ok(()); // Continue with registration, error is in diagnostics
        }

        // Register in stage checker
        if let Err(e) = self.stage_checker.register_function(
            func.name.clone(),
            func.stage,
            func.span,
        ) {
            // Convert stage error to diagnostic
            let diag = StagedMetaDiagnostics::new(&self.config.lint_config);
            let diagnostic = match e {
                StageError::StageOverflow {
                    used_stage,
                    max_stage,
                    function_name,
                    span,
                } => diag.stage_overflow(used_stage, max_stage, &function_name, Some(span)),
                _ => diag.stage_overflow(func.stage, self.config.max_stage, &func.name, Some(func.span)),
            };
            self.diagnostics.push(diagnostic);
        }

        // Store function info
        let key = (func.module.clone(), func.name.clone());
        self.functions.insert(key, func);

        Ok(())
    }

    /// Compile the module through all stages.
    ///
    /// This is the main entry point for staged compilation. It processes
    /// stages from highest (N) down to lowest (0), executing meta functions
    /// and injecting generated code at each step.
    pub fn compile(&mut self, module: Module) -> Result<StagedResult> {
        let start = std::time::Instant::now();
        info!(
            "Starting staged compilation with max_stage={}",
            self.config.max_stage
        );

        // Phase 1: Collect all staged functions from the module
        self.collect_staged_functions(&module)?;

        // Phase 2: Validate stage constraints
        self.validate_stages()?;

        // Phase 3: Execute stages from highest to lowest
        let mut current_module = module;
        for stage in (1..=self.config.max_stage).rev() {
            let stage_start = std::time::Instant::now();
            current_module = self.execute_stage(stage, current_module)?;
            let stage_elapsed = stage_start.elapsed();
            self.stats.stage_time_ns.insert(stage, stage_elapsed.as_nanos() as u64);
            self.stats.stages_processed += 1;
        }

        // Phase 4: Check for unused stages
        if self.config.warn_unused_stages {
            self.check_unused_stages();
        }

        // Phase 5: Check for stage downgrade opportunities
        if self.config.suggest_stage_downgrade {
            self.check_stage_downgrades();
        }

        let elapsed = start.elapsed();
        self.stats.total_time_ns = elapsed.as_nanos() as u64;

        info!(
            "Staged compilation completed: {} stages, {} meta executions, {:.2}ms",
            self.stats.stages_processed,
            self.stats.total_meta_executions,
            elapsed.as_secs_f64() * 1000.0
        );

        Ok(StagedResult {
            runtime_code: current_module,
            diagnostics: self.diagnostics.clone(),
            stats: self.stats.clone(),
        })
    }

    /// Collect all staged functions from the module.
    fn collect_staged_functions(&mut self, module: &Module) -> Result<()> {
        // Get module path from file_id
        let module_path = Text::from(format!("module_{}", module.file_id.raw()));

        for item in module.items.iter() {
            if let ItemKind::Function(ref func_decl) = item.kind {
                // FunctionDecl has stage_level directly, not through modifiers
                let stage = func_decl.stage_level;
                if stage > 0 {
                    let func = StagedFunction {
                        name: Text::from(func_decl.name.as_str()),
                        module: module_path.clone(),
                        stage,
                        is_generated: false,
                        dependencies: extract_call_dependencies(func_decl),
                        invocation_count: 0,
                        min_required_stage: analyze_minimum_stage(func_decl),
                        span: func_decl.span,
                    };
                    self.register_function(func)?;
                }
            }
        }
        Ok(())
    }

    /// Validate all stage constraints.
    fn validate_stages(&mut self) -> Result<()> {
        // Get collected errors from stage checker
        let errors: Vec<_> = self.stage_checker.errors().to_vec();

        let diag = StagedMetaDiagnostics::new(&self.config.lint_config);

        for error in errors {
            let diagnostic = match error {
                StageError::StageMismatch {
                    current_stage,
                    target_stage,
                    expected_stage,
                    span,
                    hint,
                } => diag.stage_mismatch(
                    current_stage,
                    target_stage,
                    expected_stage,
                    &hint,
                    Some(span),
                ),

                StageError::CrossStageCall {
                    caller_stage,
                    callee_stage,
                    callee_name,
                    span,
                    hint,
                } => diag.cross_stage_call(
                    caller_stage,
                    callee_stage,
                    &callee_name,
                    &hint,
                    Some(span),
                ),

                StageError::StageOverflow {
                    used_stage,
                    max_stage,
                    function_name,
                    span,
                } => diag.stage_overflow(used_stage, max_stage, &function_name, Some(span)),

                StageError::CyclicStage { cycle, span, .. } => {
                    let cycle_strs: Vec<&str> = cycle.iter().map(|s| s.as_str()).collect();
                    diag.cyclic_stage(&cycle_strs, Some(span))
                }

                StageError::InvalidStageEscape {
                    escape_stage,
                    current_stage,
                    valid_range,
                    span,
                } => {
                    diag.invalid_stage_escape(escape_stage, current_stage, &valid_range, Some(span))
                }
            };
            self.diagnostics.push(diagnostic);
        }

        Ok(())
    }

    /// Execute all meta functions at a specific stage.
    ///
    /// This method implements the core staged expansion loop:
    /// 1. Execute all meta functions at Stage N
    /// 2. Parse generated TokenStreams back into AST items
    /// 3. Inject generated items into the module
    /// 4. Register any newly generated meta functions for Stage N-1
    ///
    /// The stage coherence rule ensures Stage N can only directly generate
    /// Stage N-1 code, maintaining the staged metaprogramming invariant.
    fn execute_stage(&mut self, stage: u32, module: Module) -> Result<Module> {
        debug!("Executing stage {}", stage);

        // Get functions for this stage
        let stage_functions: Vec<_> = self
            .functions
            .iter()
            .filter(|(_, f)| f.stage == stage)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        if stage_functions.is_empty() {
            debug!("No functions at stage {}", stage);
            return Ok(module);
        }

        let func_count = stage_functions.len() as u32;
        *self.stats.functions_per_stage.entry(stage).or_insert(0) += func_count;

        // Compute input hash and item hashes for caching
        let input_hash = self.compute_module_hash(&module);
        let item_hashes = compute_item_hashes_from_module(&module);

        // Check cache with fine-grained invalidation
        if self.config.enable_caching {
            if let Some(cache) = self.stage_caches.get_mut(stage as usize) {
                // Check dependencies first
                let deps_changed = cache.dependencies_changed(|_| {
                    Some(0) // Return 0 to indicate no change (matches initial state)
                });

                if !deps_changed {
                    // Use fine-grained item hash comparison
                    let (cache_valid, change_kind) = cache.is_valid_fine_grained(input_hash, &item_hashes);

                    match (cache_valid, change_kind) {
                        (true, ChangeKind::NoChange) => {
                            cache.record_hit();
                            info!(
                                "Stage {} cache hit ({} fragments, {} functions)",
                                stage,
                                cache.fragments().len(),
                                cache.executed_functions().len()
                            );
                            *self.stats.cache_hits.entry(stage).or_insert(0) += 1;

                            // Apply cached fragments to module
                            return self.apply_cached_fragments(stage, module);
                        }
                        (false, ChangeKind::BodyOnly) => {
                            // Only function bodies changed - dependents need re-verification
                            // but meta functions that only depend on signatures may reuse cache
                            debug!(
                                "Stage {} partial cache hit: only function bodies changed",
                                stage
                            );
                            // For now, treat as cache miss but log the opportunity
                            // Future optimization: selective re-execution based on dependencies
                            cache.record_miss();
                        }
                        (false, ChangeKind::Signature) | (false, ChangeKind::NoChange) => {
                            // Signature changed or inconsistent state - full rebuild
                            cache.record_miss();
                            debug!("Stage {} cache miss: signature changed", stage);
                        }
                        (true, ChangeKind::BodyOnly) | (true, ChangeKind::Signature) => {
                            // Shouldn't happen - if valid, should be NoChange
                            // But treat as cache hit since validity check passed
                            cache.record_hit();
                            *self.stats.cache_hits.entry(stage).or_insert(0) += 1;
                            return self.apply_cached_fragments(stage, module);
                        }
                    }
                } else {
                    cache.record_miss();
                    debug!("Stage {} cache miss: dependency changed", stage);
                }
            }
        }

        // Execute each meta function and collect generated items
        let mut result_module = module;
        let mut generated_fragments = Map::new();
        let mut executed_func_ids = Set::new();
        let mut generated_items: List<Item> = List::new();

        for ((module_path, func_name), func) in stage_functions {
            let func_id = Text::from(format!("{}::{}", module_path, func_name));
            executed_func_ids.insert(func_id.clone());

            if let Some(f) = self.functions.get_mut(&(module_path.clone(), func_name.clone())) {
                f.invocation_count += 1;
            }

            // Execute the meta function
            let registry = &self.stage_registries[stage as usize];
            let execution_result = match registry.get_user_meta_fn(&module_path, &func_name) {
                verum_common::Maybe::Some(meta_func) => {
                    match self.vbc_executor.execute(&meta_func, &[]) {
                        Ok(token_stream) => {
                            let token_count = token_stream.len();
                            trace!(
                                "VBC execution of '{}::{}' produced {} tokens",
                                module_path,
                                func_name,
                                token_count
                            );

                            // Parse the generated TokenStream back into AST items
                            match token_stream.parse_as_items() {
                                Ok(items) => {
                                    info!(
                                        "Stage {} meta fn '{}::{}' generated {} items",
                                        stage,
                                        module_path,
                                        func_name,
                                        items.len()
                                    );

                                    // Store generated items for injection
                                    for item in items.iter() {
                                        generated_items.push(item.clone());
                                    }

                                    // Carry items alongside tokens for AST serialization
                                    Some((items.len(), token_stream.tokens().clone(), items))
                                }
                                Err(e) => {
                                    // Emit diagnostic for parse failure
                                    debug!(
                                        "Failed to parse generated code from '{}::{}': {:?}",
                                        module_path, func_name, e
                                    );
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            debug!(
                                "VBC execution error for '{}::{}': {}",
                                module_path, func_name, e
                            );
                            None
                        }
                    }
                }
                verum_common::Maybe::None => {
                    debug!(
                        "Meta function '{}::{}' not found in registry for stage {}",
                        module_path, func_name, stage
                    );
                    None
                }
            };

            // Create fragment for caching with actual serialized code
            let (generated_code, item_count, serialized_items) = match &execution_result {
                Some((count, tokens, items)) => {
                    // Serialize the token stream to source code for caching
                    let code = serialize_tokens_to_source(tokens);
                    // Serialize AST items to JSON for fast reloading without reparsing
                    let serialized = serde_json::to_string(items.as_slice())
                        .ok()
                        .map(Text::from);
                    (code, *count, serialized)
                }
                None => (String::new(), 0, None),
            };

            let fragment = GeneratedFragment {
                source_function: func.name.clone(),
                source_module: module_path.clone(),
                generated_at_stage: stage,
                target_stage: stage.saturating_sub(1),
                code: Text::from(generated_code),
                span: Some(func.span),
                serialized_items,
                item_count,
            };
            generated_fragments.insert(func_id, fragment);

            self.stats.total_meta_executions += 1;
            debug!(
                "Executed meta function {} at stage {} (target stage {})",
                func.name,
                func.stage,
                stage.saturating_sub(1)
            );
        }

        // Inject generated items into the module
        if !generated_items.is_empty() {
            info!(
                "Injecting {} generated items into module at stage {}",
                generated_items.len(),
                stage
            );

            // Clone existing items and extend with generated ones
            let mut new_items = result_module.items.clone();
            for item in generated_items.iter() {
                new_items.push(item.clone());

                // Register any generated meta functions for lower stages
                self.register_generated_function(item, &result_module, stage)?;
            }

            // Create new module with updated items
            result_module = Module {
                items: new_items,
                ..result_module
            };
        }

        // Update cache with item hashes for fine-grained invalidation
        if self.config.enable_caching {
            let output_hash = self.compute_module_hash(&result_module);
            let output_item_hashes = compute_item_hashes_from_module(&result_module);
            if let Some(cache) = self.stage_caches.get_mut(stage as usize) {
                cache.update_with_dependencies(
                    output_hash,
                    generated_fragments,
                    Set::new(),
                    Map::new(),
                    executed_func_ids,
                );
                // Store item hashes for fine-grained comparison on next compilation
                cache.set_item_hashes(output_item_hashes);
                trace!(
                    "Stage {} cache updated: {} fragments, {} function hashes, {} type hashes",
                    stage,
                    cache.fragments().len(),
                    cache.item_hashes().map(|h| h.functions.len()).unwrap_or(0),
                    cache.item_hashes().map(|h| h.types.len()).unwrap_or(0)
                );
            }
        }

        Ok(result_module)
    }

    /// Apply cached fragments to a module.
    ///
    /// This reconstructs the generated items from cache without re-executing
    /// the meta functions. The cached fragments contain serialized source code
    /// that is reparsed and injected into the module.
    fn apply_cached_fragments(&mut self, stage: u32, module: Module) -> Result<Module> {
        let cache = match self.stage_caches.get(stage as usize) {
            Some(c) => c,
            None => {
                warn!("No cache found for stage {}", stage);
                return Ok(module);
            }
        };

        let fragments: Vec<_> = cache.fragments().values().cloned().collect();
        let fragment_count = fragments.len();

        if fragment_count == 0 {
            debug!("No cached fragments to apply for stage {}", stage);
            return Ok(module);
        }

        debug!(
            "Applying {} cached fragments for stage {}",
            fragment_count, stage
        );

        let mut result_module = module;
        let mut total_items_injected = 0;

        for fragment in fragments {
            // Skip empty fragments
            if fragment.code.is_empty() || fragment.item_count == 0 {
                continue;
            }

            // Reparse the cached code
            let file_id = result_module.file_id;
            match TokenStream::from_str(fragment.code.as_str(), file_id) {
                Ok(token_stream) => {
                    match token_stream.parse_as_items() {
                        Ok(items) => {
                            if items.len() != fragment.item_count {
                                warn!(
                                    "Cached fragment '{}' item count mismatch: expected {}, got {}",
                                    fragment.source_function,
                                    fragment.item_count,
                                    items.len()
                                );
                            }

                            // Inject items into the module
                            let mut new_items = result_module.items.clone();
                            for item in items.iter() {
                                new_items.push(item.clone());

                                // Register generated meta functions for lower stages
                                if let Err(e) = self.register_generated_function(
                                    item,
                                    &result_module,
                                    stage,
                                ) {
                                    warn!(
                                        "Failed to register generated function from cache: {}",
                                        e
                                    );
                                }
                            }

                            total_items_injected += items.len();

                            result_module = Module {
                                items: new_items,
                                ..result_module
                            };
                        }
                        Err(e) => {
                            warn!(
                                "Failed to parse cached fragment '{}': {:?}",
                                fragment.source_function, e
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to tokenize cached fragment '{}': {:?}",
                        fragment.source_function, e
                    );
                }
            }
        }

        info!(
            "Stage {} cache application complete: {} items injected from {} fragments",
            stage, total_items_injected, fragment_count
        );

        Ok(result_module)
    }

    /// Register a generated function for the appropriate stage.
    ///
    /// When a Stage N meta function generates code containing meta functions,
    /// those generated functions are registered for Stage N-1.
    fn register_generated_function(&mut self, item: &Item, module: &Module, current_stage: u32) -> Result<()> {
        use verum_ast::decl::ItemKind;

        let module_path = Text::from(format!("module_{}", module.file_id.raw()));

        if let ItemKind::Function(ref func_decl) = item.kind {
            let target_stage = func_decl.stage_level;

            // Only register if this is a meta function (stage > 0)
            // and it's for the expected target stage (current_stage - 1)
            if target_stage > 0 {
                // Validate stage coherence
                if target_stage != current_stage.saturating_sub(1) {
                    debug!(
                        "Warning: Generated function '{}' has stage {} but expected stage {}",
                        func_decl.name,
                        target_stage,
                        current_stage.saturating_sub(1)
                    );
                }

                let func = StagedFunction {
                    name: Text::from(func_decl.name.as_str()),
                    module: module_path,
                    stage: target_stage,
                    is_generated: true,
                    dependencies: extract_call_dependencies(func_decl),
                    invocation_count: 0,
                    min_required_stage: analyze_minimum_stage(func_decl),
                    span: func_decl.span,
                };

                self.register_function(func)?;
            }
        }

        Ok(())
    }

    /// Check for unused staged functions.
    fn check_unused_stages(&mut self) {
        let diag = StagedMetaDiagnostics::new(&self.config.lint_config);

        for ((_, _), func) in self.functions.iter() {
            if func.stage > 0 && func.invocation_count == 0 && !func.is_generated {
                if let Some(diagnostic) =
                    diag.unused_stage(func.stage, &func.name, Some(func.span))
                {
                    self.diagnostics.push(diagnostic);
                }
            }
        }
    }

    /// Check for stage downgrade opportunities.
    ///
    /// Uses the pre-computed `min_required_stage` on each `StagedFunction`
    /// to suggest lowering the declared stage when the body doesn't need it.
    fn check_stage_downgrades(&mut self) {
        let diag = StagedMetaDiagnostics::new(&self.config.lint_config);

        let mut suggestions = List::new();
        for ((_, _), func) in self.functions.iter() {
            // Skip generated functions and stage 0/1
            if func.is_generated || func.stage <= 1 {
                continue;
            }

            // min_required_stage was computed during collect_staged_functions
            // by analyzing Quote, StageEscape, MacroCall, and Lift constructs
            if func.min_required_stage < func.stage {
                if let Some(diagnostic) = diag.stage_downgrade(
                    func.stage,
                    func.min_required_stage,
                    &func.name,
                    "function body only requires constructs from a lower stage",
                    Some(func.span),
                ) {
                    suggestions.push(diagnostic);
                }
            }
        }
        for d in suggestions.iter() {
            self.diagnostics.push(d.clone());
        }
    }

    /// Compute a hash of the module for caching.
    ///
    /// Uses a structural hash of the module content to detect changes.
    /// The hash includes:
    /// - File ID
    /// - Number of items
    /// - Item kinds and basic structure
    /// - Stage levels of functions
    fn compute_module_hash(&self, module: &Module) -> u64 {
        let mut hasher = ContentHash::new();

        // Hash file identity
        hasher.update(&module.file_id.raw().to_le_bytes());

        // Hash item count
        hasher.update(&module.items.len().to_le_bytes());

        // Hash each item's structure
        for item in module.items.iter() {
            // Hash item kind discriminant
            let discriminant = std::mem::discriminant(&item.kind);
            hasher.update_str(&format!("{:?}", discriminant));

            // Hash additional item-specific data
            match &item.kind {
                ItemKind::Function(func) => {
                    hasher.update_str(func.name.as_str());
                    hasher.update(&func.stage_level.to_le_bytes());
                    hasher.update(&func.params.len().to_le_bytes());
                    hasher.update(if func.is_async { b"1" } else { b"0" });
                }
                ItemKind::Type(type_decl) => {
                    hasher.update_str(type_decl.name.as_str());
                }
                ItemKind::Mount(_import) => {
                    // Just hash the discriminant, tree structure is complex
                }
                _ => {
                    // Basic structure only for other items
                }
            }
        }

        hasher.finalize().to_u64()
    }

    /// Compute hash of a file's content for dependency tracking using Blake3.
    pub fn compute_file_hash(&self, content: &[u8]) -> u64 {
        let mut hasher = ContentHash::new();
        hasher.update(content);
        hasher.finalize().to_u64()
    }

    /// Compute hash of a text string using Blake3.
    pub fn compute_text_hash(&self, text: &str) -> u64 {
        let mut hasher = ContentHash::new();
        hasher.update_str(text);
        hasher.finalize().to_u64()
    }

    /// Get all collected diagnostics.
    pub fn diagnostics(&self) -> &List<Diagnostic> {
        &self.diagnostics
    }

    /// Get compilation statistics.
    pub fn stats(&self) -> &StagedStats {
        &self.stats
    }

    /// Invalidate all caches.
    pub fn invalidate_caches(&mut self) {
        for cache in &mut self.stage_caches {
            cache.invalidate();
        }
    }

    /// Invalidate cache for a specific stage.
    pub fn invalidate_stage_cache(&mut self, stage: u32) {
        if let Some(cache) = self.stage_caches.get_mut(stage as usize) {
            cache.invalidate();
        }
        // Also invalidate all lower stages since they depend on higher stages
        for s in 0..stage {
            if let Some(cache) = self.stage_caches.get_mut(s as usize) {
                cache.invalidate();
            }
        }
    }

    /// Get cache statistics for all stages.
    pub fn cache_statistics(&self) -> Vec<StageCacheStatistics> {
        self.stage_caches.iter().map(|c| c.statistics()).collect()
    }

    /// Get overall cache hit rate across all stages.
    pub fn cache_hit_rate(&self) -> f64 {
        let (hits, misses): (u64, u64) = self.stage_caches.iter().fold((0, 0), |(h, m), c| {
            (h + c.hit_count(), m + c.miss_count())
        });
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get total fragment count across all caches.
    pub fn total_cached_fragments(&self) -> usize {
        self.stage_caches.iter().map(|c| c.fragments().len()).sum()
    }

    /// Check if any stage has cached data.
    pub fn has_cached_data(&self) -> bool {
        self.stage_caches.iter().any(|c| !c.fragments().is_empty())
    }

    /// Pre-warm cache from a previous compilation result.
    ///
    /// This allows reusing cache data across incremental compilations.
    pub fn prewarm_cache(&mut self, previous: &StagedPipeline) {
        for (stage, cache) in previous.stage_caches.iter().enumerate() {
            if let Some(our_cache) = self.stage_caches.get_mut(stage) {
                // Only copy if the previous cache was valid
                if cache.is_valid_for(cache.input_hash) {
                    our_cache.update_with_dependencies(
                        cache.input_hash,
                        cache.fragments().clone(),
                        cache.file_dependencies.clone(),
                        cache.dependency_hashes.clone(),
                        cache.executed_functions().clone(),
                    );
                    // Also copy item hashes for fine-grained invalidation
                    if let Some(hashes) = cache.item_hashes() {
                        our_cache.set_item_hashes(hashes.clone());
                    }
                    debug!("Pre-warmed cache for stage {} from previous compilation", stage);
                }
            }
        }
    }

    /// Register a file dependency for the current stage being executed.
    ///
    /// Call this during meta function execution to track dependencies.
    pub fn register_dependency(&mut self, stage: u32, file_path: Text, content_hash: u64) {
        if let Some(cache) = self.stage_caches.get_mut(stage as usize) {
            cache.add_dependency(file_path, content_hash);
        }
    }

    /// Get the number of cached fragments for a specific stage.
    pub fn cached_fragment_count(&self, stage: u32) -> usize {
        self.stage_caches
            .get(stage as usize)
            .map(|c| c.fragments().len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::FileId;

    fn create_test_module() -> Module {
        Module::empty(FileId::dummy())
    }

    #[test]
    fn test_staged_pipeline_creation() {
        let pipeline = StagedPipeline::default();
        assert_eq!(pipeline.max_stage(), 2);
        assert!(pipeline.diagnostics().is_empty());
    }

    #[test]
    fn test_staged_pipeline_with_custom_config() {
        let config = StagedConfig {
            max_stage: 5,
            enable_caching: false,
            ..Default::default()
        };
        let pipeline = StagedPipeline::new(config);
        assert_eq!(pipeline.max_stage(), 5);
    }

    #[test]
    fn test_stage_cache() {
        let mut cache = StageCache::new(1);
        assert!(!cache.is_valid_for(123));

        cache.update(123, Map::new());
        assert!(cache.is_valid_for(123));
        assert!(!cache.is_valid_for(456));

        cache.invalidate();
        assert!(!cache.is_valid_for(123));
    }

    #[test]
    fn test_staged_config_default() {
        let config = StagedConfig::default();
        assert_eq!(config.max_stage, 2);
        assert!(config.enable_caching);
        assert!(config.warn_unused_stages);
        assert!(config.suggest_stage_downgrade);
    }

    #[test]
    fn test_empty_module_compilation() {
        let mut pipeline = StagedPipeline::default();
        let module = create_test_module();

        let result = pipeline.compile(module).expect("should compile");
        assert!(result.diagnostics.is_empty());
        assert_eq!(result.stats.stages_processed, 2);
        assert_eq!(result.stats.total_meta_executions, 0);
    }

    // =========================================================================
    // Phase 8: Comprehensive Tests
    // =========================================================================

    #[test]
    fn test_cache_invalidation_cascade() {
        // Test that invalidating a higher stage invalidates lower stages
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 3,
            ..Default::default()
        });

        // Update caches for stages 1, 2, 3
        for stage in 1..=3 {
            if let Some(cache) = pipeline.stage_caches.get_mut(stage as usize) {
                cache.update(100 + stage as u64, Map::new());
            }
        }

        // Verify all caches are valid
        for stage in 1..=3 {
            assert!(pipeline.stage_caches.get(stage as usize).unwrap().is_valid_for(100 + stage as u64));
        }

        // Invalidate stage 2 - should also invalidate stage 1
        pipeline.invalidate_stage_cache(2);

        // Stage 3 should still be valid
        assert!(pipeline.stage_caches.get(3).unwrap().is_valid_for(103));

        // Stages 1 and 2 should be invalid
        assert!(!pipeline.stage_caches.get(1).unwrap().is_valid_for(101));
        assert!(!pipeline.stage_caches.get(2).unwrap().is_valid_for(102));
    }

    #[test]
    fn test_cache_hit_rate_calculation() {
        let mut cache = StageCache::new(1);

        // Record some hits and misses
        cache.record_hit();
        cache.record_hit();
        cache.record_miss();

        let stats = cache.statistics();
        assert_eq!(stats.hit_count, 2);
        assert_eq!(stats.miss_count, 1);
    }

    #[test]
    fn test_cache_with_dependencies() {
        let mut cache = StageCache::new(1);

        // Add dependencies
        cache.add_dependency(Text::from("file1.vr"), 123);
        cache.add_dependency(Text::from("file2.vr"), 456);

        let stats = cache.statistics();
        assert_eq!(stats.dependency_count, 2);

        // Test dependency change detection
        assert!(!cache.dependencies_changed(|path| {
            match path.as_str() {
                "file1.vr" => Some(123), // Same hash
                "file2.vr" => Some(456), // Same hash
                _ => None,
            }
        }));

        // Changed dependency should be detected
        assert!(cache.dependencies_changed(|path| {
            match path.as_str() {
                "file1.vr" => Some(999), // Different hash!
                "file2.vr" => Some(456),
                _ => None,
            }
        }));

        // Missing dependency should be detected
        assert!(cache.dependencies_changed(|path| {
            match path.as_str() {
                "file1.vr" => Some(123),
                // file2.vr is missing!
                _ => None,
            }
        }));
    }

    #[test]
    fn test_cache_with_executed_functions() {
        let mut cache = StageCache::new(2);

        let mut executed = Set::new();
        executed.insert(Text::from("module::func1"));
        executed.insert(Text::from("module::func2"));

        cache.update_with_dependencies(
            42,
            Map::new(),
            Set::new(),
            Map::new(),
            executed,
        );

        assert!(cache.was_function_executed(&Text::from("module::func1")));
        assert!(cache.was_function_executed(&Text::from("module::func2")));
        assert!(!cache.was_function_executed(&Text::from("module::func3")));
    }

    #[test]
    fn test_generated_fragment() {
        let fragment = GeneratedFragment {
            source_function: Text::from("my_meta_fn"),
            source_module: Text::from("my_module"),
            generated_at_stage: 2,
            target_stage: 1,
            code: Text::from("fn generated() {}"),
            span: None,
            serialized_items: None,
            item_count: 1,
        };

        assert_eq!(fragment.source_function.as_str(), "my_meta_fn");
        assert_eq!(fragment.generated_at_stage, 2);
        assert_eq!(fragment.target_stage, 1);
        assert_eq!(fragment.item_count, 1);
    }

    #[test]
    fn test_staged_function_info() {
        let func = StagedFunction {
            name: Text::from("stage2_generator"),
            module: Text::from("my_module"),
            stage: 2,
            is_generated: false,
            dependencies: List::new(),
            invocation_count: 0,
            min_required_stage: 0,
            span: Span::dummy(),
        };

        assert_eq!(func.stage, 2);
        assert!(!func.is_generated);
        assert_eq!(func.invocation_count, 0);
    }

    #[test]
    fn test_stats_summary() {
        let mut stats = StagedStats::default();
        stats.stages_processed = 2;
        stats.total_meta_executions = 5;
        stats.functions_per_stage.insert(1, 3);
        stats.functions_per_stage.insert(2, 2);
        stats.cache_hits.insert(1, 1);
        stats.cache_misses.insert(1, 2);
        stats.total_time_ns = 1_500_000; // 1.5ms

        let summary = stats.summary();
        assert!(summary.contains("Staged Compilation Statistics"));
        assert!(summary.contains("2")); // stages processed
        assert!(summary.contains("5")); // meta executions
    }

    #[test]
    fn test_stats_cache_hit_rate() {
        let mut stats = StagedStats::default();

        // No hits or misses = 0% hit rate
        assert_eq!(stats.cache_hit_rate(), 0.0);

        // 2 hits, 2 misses = 50% hit rate
        stats.cache_hits.insert(1, 2);
        stats.cache_misses.insert(1, 2);
        assert!((stats.cache_hit_rate() - 0.5).abs() < 0.001);

        // Add more hits across stages
        stats.cache_hits.insert(2, 3);
        // Total: 5 hits, 2 misses = 5/7 ≈ 71.4%
        let expected = 5.0 / 7.0;
        assert!((stats.cache_hit_rate() - expected).abs() < 0.001);
    }

    #[test]
    fn test_module_hash_consistency() {
        let pipeline = StagedPipeline::default();

        let module1 = create_test_module();
        let module2 = create_test_module();

        // Same module content should produce same hash
        let hash1 = pipeline.compute_module_hash(&module1);
        let hash2 = pipeline.compute_module_hash(&module2);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_file_and_text_hash() {
        let pipeline = StagedPipeline::default();

        let hash1 = pipeline.compute_file_hash(b"hello world");
        let hash2 = pipeline.compute_file_hash(b"hello world");
        let hash3 = pipeline.compute_file_hash(b"different content");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);

        let text_hash1 = pipeline.compute_text_hash("test string");
        let text_hash2 = pipeline.compute_text_hash("test string");
        let text_hash3 = pipeline.compute_text_hash("different");

        assert_eq!(text_hash1, text_hash2);
        assert_ne!(text_hash1, text_hash3);
    }

    #[test]
    fn test_register_dependency() {
        let mut pipeline = StagedPipeline::default();

        pipeline.register_dependency(1, Text::from("config.toml"), 12345);

        let stats = pipeline.cache_statistics();
        // Stage 1 should have 1 dependency
        let stage1_stats = stats.get(1).unwrap();
        assert_eq!(stage1_stats.dependency_count, 1);
    }

    #[test]
    fn test_cache_statistics_collection() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 3,
            ..Default::default()
        });

        // Update some caches
        if let Some(cache) = pipeline.stage_caches.get_mut(1) {
            cache.update(100, Map::new());
            cache.record_hit();
        }
        if let Some(cache) = pipeline.stage_caches.get_mut(2) {
            cache.update(200, Map::new());
            cache.record_miss();
            cache.record_miss();
        }

        let stats = pipeline.cache_statistics();
        assert_eq!(stats.len(), 4); // Stages 0, 1, 2, 3

        // Verify stage 1 has 1 hit
        assert_eq!(stats[1].hit_count, 1);
        assert_eq!(stats[1].miss_count, 0);
        assert!(stats[1].valid);

        // Verify stage 2 has 2 misses
        assert_eq!(stats[2].hit_count, 0);
        assert_eq!(stats[2].miss_count, 2);
        assert!(stats[2].valid);
    }

    #[test]
    fn test_overall_cache_hit_rate() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 2,
            ..Default::default()
        });

        // Initially 0% hit rate (no hits or misses)
        assert_eq!(pipeline.cache_hit_rate(), 0.0);

        // Add some hits and misses
        if let Some(cache) = pipeline.stage_caches.get_mut(1) {
            cache.record_hit();
            cache.record_hit();
            cache.record_miss();
        }

        // 2 hits, 1 miss = 66.7%
        let rate = pipeline.cache_hit_rate();
        assert!((rate - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_has_cached_data() {
        let mut pipeline = StagedPipeline::default();

        // Initially no cached data
        assert!(!pipeline.has_cached_data());

        // Add a fragment
        let mut fragments = Map::new();
        fragments.insert(
            Text::from("func1"),
            GeneratedFragment {
                source_function: Text::from("func1"),
                source_module: Text::from("mod"),
                generated_at_stage: 1,
                target_stage: 0,
                code: Text::from(""),
                span: None,
                serialized_items: None,
                item_count: 0,
            },
        );

        if let Some(cache) = pipeline.stage_caches.get_mut(1) {
            cache.update(100, fragments);
        }

        // Now has cached data
        assert!(pipeline.has_cached_data());
        assert_eq!(pipeline.total_cached_fragments(), 1);
        assert_eq!(pipeline.cached_fragment_count(1), 1);
        assert_eq!(pipeline.cached_fragment_count(2), 0);
    }

    #[test]
    fn test_cache_invalidate_all() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 3,
            ..Default::default()
        });

        // Update all caches
        for stage in 0..=3 {
            if let Some(cache) = pipeline.stage_caches.get_mut(stage) {
                cache.update(stage as u64 * 100, Map::new());
            }
        }

        // Verify all are valid
        for stage in 0..=3 {
            assert!(pipeline.stage_caches[stage].is_valid_for(stage as u64 * 100));
        }

        // Invalidate all
        pipeline.invalidate_caches();

        // All should be invalid
        for stage in 0..=3 {
            assert!(!pipeline.stage_caches[stage].is_valid_for(stage as u64 * 100));
        }
    }

    #[test]
    fn test_cache_ttl_validation() {
        let mut cache = StageCache::new(1);
        cache.update(100, Map::new());

        // Should be valid with no TTL
        assert!(cache.is_valid_with_ttl(100, 0));

        // Should be valid with long TTL
        assert!(cache.is_valid_with_ttl(100, 1_000_000)); // 1000 seconds

        // Note: Testing actual expiry would require time mocking
        // which is out of scope for unit tests
    }

    #[test]
    fn test_stage_cache_statistics_complete() {
        let mut cache = StageCache::new(2);

        // Initial state
        let stats = cache.statistics();
        assert_eq!(stats.stage, 2);
        assert_eq!(stats.fragment_count, 0);
        assert_eq!(stats.dependency_count, 0);
        assert_eq!(stats.hit_count, 0);
        assert_eq!(stats.miss_count, 0);
        assert!(!stats.valid);

        // After operations
        cache.add_dependency(Text::from("dep.vr"), 999);
        cache.record_hit();
        cache.record_hit();
        cache.record_miss();

        let mut fragments = Map::new();
        fragments.insert(Text::from("f1"), GeneratedFragment {
            source_function: Text::from("f1"),
            source_module: Text::from("m"),
            generated_at_stage: 2,
            target_stage: 1,
            code: Text::from(""),
            span: None,
            serialized_items: None,
            item_count: 0,
        });
        cache.update(42, fragments);

        let stats = cache.statistics();
        assert_eq!(stats.fragment_count, 1);
        assert_eq!(stats.dependency_count, 1);
        assert_eq!(stats.hit_count, 2);
        assert_eq!(stats.miss_count, 1);
        assert!(stats.valid);
    }

    // =========================================================================
    // Fine-Grained Cache Invalidation Tests
    // =========================================================================

    #[test]
    fn test_item_hashes_storage() {
        use crate::hash::{FunctionHashBuilder, ItemHashes};

        let mut cache = StageCache::new(1);
        assert!(cache.item_hashes().is_none());

        // Create and store item hashes
        let mut hashes = ItemHashes::new();
        hashes.add_function(
            "my_func".to_string(),
            FunctionHashBuilder::new()
                .with_name("my_func")
                .with_return_type("Int")
                .with_bytecode(&[0x01, 0x02])
                .finish(),
        );
        hashes.add_type("MyType".to_string(), crate::hash::hash_str("type_def"));

        cache.set_item_hashes(hashes);
        assert!(cache.item_hashes().is_some());

        let stored = cache.item_hashes().unwrap();
        assert_eq!(stored.functions.len(), 1);
        assert_eq!(stored.types.len(), 1);
        assert!(stored.functions.contains_key("my_func"));
        assert!(stored.types.contains_key("MyType"));
    }

    #[test]
    fn test_fine_grained_cache_no_change() {
        use crate::hash::{FunctionHashBuilder, ItemHashes};

        let mut cache = StageCache::new(1);

        // Create initial hashes
        let mut hashes1 = ItemHashes::new();
        hashes1.add_function(
            "func".to_string(),
            FunctionHashBuilder::new()
                .with_name("func")
                .with_return_type("Int")
                .with_bytecode(&[0x01])
                .finish(),
        );

        // Update cache with hash 100 and item hashes
        cache.update(100, Map::new());
        cache.set_item_hashes(hashes1.clone());

        // Check with same hash - should be valid with NoChange
        let (valid, change_kind) = cache.is_valid_fine_grained(100, &hashes1);
        assert!(valid);
        assert_eq!(change_kind, ChangeKind::NoChange);
    }

    #[test]
    fn test_fine_grained_cache_body_only_change() {
        use crate::hash::{FunctionHashBuilder, ItemHashes};

        let mut cache = StageCache::new(1);

        // Create initial hashes
        let mut hashes1 = ItemHashes::new();
        hashes1.add_function(
            "func".to_string(),
            FunctionHashBuilder::new()
                .with_name("func")
                .with_return_type("Int")
                .with_bytecode(&[0x01]) // Original body
                .finish(),
        );

        cache.update(100, Map::new());
        cache.set_item_hashes(hashes1);

        // Create new hashes with only body change
        let mut hashes2 = ItemHashes::new();
        hashes2.add_function(
            "func".to_string(),
            FunctionHashBuilder::new()
                .with_name("func")
                .with_return_type("Int") // Same signature
                .with_bytecode(&[0x02, 0x03]) // Different body!
                .finish(),
        );

        // Check with different hash but same signature - should detect body-only change
        let (valid, change_kind) = cache.is_valid_fine_grained(101, &hashes2);
        assert!(!valid); // Cache invalid because hash changed
        assert_eq!(change_kind, ChangeKind::BodyOnly); // But only body changed
    }

    #[test]
    fn test_fine_grained_cache_signature_change() {
        use crate::hash::{FunctionHashBuilder, ItemHashes};

        let mut cache = StageCache::new(1);

        // Create initial hashes
        let mut hashes1 = ItemHashes::new();
        hashes1.add_function(
            "func".to_string(),
            FunctionHashBuilder::new()
                .with_name("func")
                .with_return_type("Int")
                .with_bytecode(&[0x01])
                .finish(),
        );

        cache.update(100, Map::new());
        cache.set_item_hashes(hashes1);

        // Create new hashes with signature change
        let mut hashes2 = ItemHashes::new();
        hashes2.add_function(
            "func".to_string(),
            FunctionHashBuilder::new()
                .with_name("func")
                .with_return_type("Bool") // Signature changed!
                .with_bytecode(&[0x01])
                .finish(),
        );

        // Check with different hash and different signature
        let (valid, change_kind) = cache.is_valid_fine_grained(101, &hashes2);
        assert!(!valid);
        assert_eq!(change_kind, ChangeKind::Signature);
    }

    #[test]
    fn test_fine_grained_cache_no_cached_hashes() {
        use crate::hash::{FunctionHashBuilder, ItemHashes};

        let mut cache = StageCache::new(1);
        cache.update(100, Map::new());
        // Don't set item_hashes - should be None

        let mut current_hashes = ItemHashes::new();
        current_hashes.add_function(
            "func".to_string(),
            FunctionHashBuilder::new()
                .with_name("func")
                .finish(),
        );

        // Without cached hashes, should return Signature (needs full rebuild)
        let (valid, change_kind) = cache.is_valid_fine_grained(101, &current_hashes);
        assert!(!valid);
        assert_eq!(change_kind, ChangeKind::Signature);
    }

    #[test]
    fn test_fine_grained_cache_invalidation_clears_hashes() {
        use crate::hash::{FunctionHashBuilder, ItemHashes};

        let mut cache = StageCache::new(1);

        let mut hashes = ItemHashes::new();
        hashes.add_function(
            "func".to_string(),
            FunctionHashBuilder::new().with_name("func").finish(),
        );

        cache.update(100, Map::new());
        cache.set_item_hashes(hashes);
        assert!(cache.item_hashes().is_some());

        // Invalidate cache
        cache.invalidate();

        // Item hashes should be cleared
        assert!(cache.item_hashes().is_none());
    }

    // =========================================================================
    // import_from_registry Tests
    // =========================================================================

    use verum_ast::Expr;

    /// Helper to create a test MetaFunction
    fn make_test_meta_fn(name: &str, module: &str) -> crate::meta::MetaFunction {
        use verum_ast::expr::ExprKind;
        crate::meta::MetaFunction {
            name: Text::from(name),
            module: Text::from(module),
            params: List::new(),
            return_type: verum_ast::ty::Type::unit(Span::dummy()),
            body: Expr::new(ExprKind::Tuple(List::new()), Span::dummy()),
            contexts: List::new(),
            is_async: false,
            is_transparent: false,
            stage_level: 1,
            span: Span::dummy(),
        }
    }

    #[test]
    fn test_import_from_registry_empty() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 2,
            ..Default::default()
        });

        let external_registry = MetaRegistry::new();
        let module = create_test_module();

        // Should not panic with empty registry
        pipeline.import_from_registry(&external_registry, &module);

        // Verify registries are still empty
        for stage in 0..=2 {
            let registry = pipeline.registry_for_stage(stage).unwrap();
            assert_eq!(registry.all_meta_functions().len(), 0);
        }
    }

    #[test]
    fn test_import_from_registry_default_stage() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 2,
            ..Default::default()
        });

        // Create external registry with a meta function
        let mut external_registry = MetaRegistry::new();
        let meta_fn = make_test_meta_fn("default_stage_fn", "default_mod");
        external_registry.register_meta_fn_direct(meta_fn).unwrap();

        // Use empty module (no stage info in AST)
        let module = create_test_module();

        pipeline.import_from_registry(&external_registry, &module);

        // Without stage info in AST, should default to stage 1
        let stage1_registry = pipeline.registry_for_stage(1).unwrap();
        let resolved = stage1_registry.resolve_meta_call(
            &Text::from("default_mod"),
            &Text::from("default_stage_fn"),
        );

        assert!(resolved.is_some(), "Meta function should default to stage 1");
    }

    #[test]
    fn test_import_from_registry_macros() {
        use crate::meta::MacroKind;

        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 2,
            ..Default::default()
        });

        // Create external registry with a macro
        let mut external_registry = MetaRegistry::new();
        external_registry.register_macro(
            &Text::from("macro_mod"),
            Text::from("test_derive"),
            MacroKind::Derive,
            Text::from("test_derive_impl"),
            Span::dummy(),
        ).unwrap();

        let module = create_test_module();
        pipeline.import_from_registry(&external_registry, &module);

        // Macros should be in stage 1 registry
        let stage1_registry = pipeline.registry_for_stage(1).unwrap();
        let resolved = stage1_registry.resolve_macro(
            &Text::from("macro_mod"),
            &Text::from("test_derive"),
        );

        assert!(resolved.is_some(), "Macro should be imported to stage 1");
        assert_eq!(resolved.unwrap().kind, MacroKind::Derive);
    }

    #[test]
    fn test_reset_clears_registries() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 2,
            ..Default::default()
        });

        // Add a meta function to stage 1 registry
        let mut external_registry = MetaRegistry::new();
        let meta_fn = make_test_meta_fn("will_be_cleared", "clear_mod");
        external_registry.register_meta_fn_direct(meta_fn).unwrap();

        let module = create_test_module();
        pipeline.import_from_registry(&external_registry, &module);

        // Verify it's there
        let stage1 = pipeline.registry_for_stage(1).unwrap();
        assert!(stage1.resolve_meta_call(
            &Text::from("clear_mod"),
            &Text::from("will_be_cleared")
        ).is_some());

        // Reset
        pipeline.reset();

        // Should be gone
        let stage1_after = pipeline.registry_for_stage(1).unwrap();
        assert!(stage1_after.resolve_meta_call(
            &Text::from("clear_mod"),
            &Text::from("will_be_cleared")
        ).is_none());
    }

    #[test]
    fn test_import_multiple_functions() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 2,
            ..Default::default()
        });

        // Create external registry with multiple meta functions
        let mut external_registry = MetaRegistry::new();
        for i in 0..5 {
            let meta_fn = make_test_meta_fn(&format!("fn_{}", i), "multi_mod");
            external_registry.register_meta_fn_direct(meta_fn).unwrap();
        }

        let module = create_test_module();
        pipeline.import_from_registry(&external_registry, &module);

        // All should be in stage 1 (default)
        let stage1_registry = pipeline.registry_for_stage(1).unwrap();
        let all_fns = stage1_registry.all_meta_functions();
        assert_eq!(all_fns.len(), 5);

        // Verify each is resolvable
        for i in 0..5 {
            let resolved = stage1_registry.resolve_meta_call(
                &Text::from("multi_mod"),
                &Text::from(format!("fn_{}", i)),
            );
            assert!(resolved.is_some(), "Function fn_{} should be resolvable", i);
        }
    }

    #[test]
    fn test_import_preserves_function_metadata() {
        let mut pipeline = StagedPipeline::new(StagedConfig {
            max_stage: 2,
            ..Default::default()
        });

        // Create meta function with specific attributes
        let mut external_registry = MetaRegistry::new();
        let meta_fn = crate::meta::MetaFunction {
            name: Text::from("async_transparent_fn"),
            module: Text::from("meta_mod"),
            params: List::new(),
            return_type: verum_ast::ty::Type::unit(Span::dummy()),
            body: Expr::new(verum_ast::expr::ExprKind::Tuple(List::new()), Span::dummy()),
            contexts: List::new(),
            is_async: true,
            is_transparent: true,
            stage_level: 1,
            span: Span::dummy(),
        };
        external_registry.register_meta_fn_direct(meta_fn).unwrap();

        let module = create_test_module();
        pipeline.import_from_registry(&external_registry, &module);

        // Verify metadata is preserved
        let stage1_registry = pipeline.registry_for_stage(1).unwrap();
        let resolved = stage1_registry.resolve_meta_call(
            &Text::from("meta_mod"),
            &Text::from("async_transparent_fn"),
        );

        assert!(resolved.is_some());
        let func = resolved.unwrap();
        assert!(func.is_async, "is_async should be preserved");
        assert!(func.is_transparent, "is_transparent should be preserved");
    }
}
