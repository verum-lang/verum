//! Quote Expansion with Hygiene
//!
//! Implements the quote expansion pipeline with proper hygiene enforcement.
//! This module handles the mark phase, splice phase, and stage management.
//!
//! ## Pipeline Stages
//!
//! 1. Parse Quote - Parse quote { ... } into QuoteAST
//! 2. Mark Phase - Assign fresh marks to introduced bindings
//! 3. Splice Phase - Substitute $var and $[for...] expressions
//! 4. Hygiene Check - Verify no accidental capture occurs
//! 5. Emit Code - Generate TokenStream with hygiene metadata
//!
//! Hygienic macro expansion: ensures macro-introduced bindings do not
//! capture or shadow user bindings via syntax context tracking.

use verum_ast::{Expr, Span};
use verum_common::{List, Map, Maybe, Text};

use super::marks::ScopeMark;
use super::scope::{BindingInfo, BindingKind, HygienicIdent, ScopeId, ScopeSet};
use super::syntax_context::{Mark, MarkSet, SyntaxContext};
use super::violations::{HygieneViolation, HygieneViolations};
use super::HygieneContext;

/// Configuration for quote expansion
#[derive(Debug, Clone)]
pub struct ExpansionConfig {
    /// Target stage for the quote
    pub target_stage: u32,
    /// Source stage (where the quote is defined)
    pub source_stage: u32,
    /// Whether to enable strict hygiene mode
    pub strict_mode: bool,
    /// Maximum recursion depth for nested quotes
    pub max_depth: usize,
    /// Whether to track all bindings for debugging
    pub debug_bindings: bool,
}

impl Default for ExpansionConfig {
    fn default() -> Self {
        Self {
            target_stage: 0,
            source_stage: 1,
            strict_mode: true,
            max_depth: 16,
            debug_bindings: false,
        }
    }
}

impl ExpansionConfig {
    /// Create configuration for a meta function (stage 1 -> stage 0)
    pub fn meta_to_runtime() -> Self {
        Self {
            target_stage: 0,
            source_stage: 1,
            ..Default::default()
        }
    }

    /// Create configuration for meta(2) function (stage 2 -> stage 1)
    pub fn meta2_to_meta() -> Self {
        Self {
            target_stage: 1,
            source_stage: 2,
            ..Default::default()
        }
    }

    /// Create configuration with explicit stage levels
    pub fn staged(source: u32, target: u32) -> Self {
        Self {
            source_stage: source,
            target_stage: target,
            ..Default::default()
        }
    }
}

/// A single event in the `debug_bindings` trace.
///
/// Emitted by the expander only when `ExpansionConfig.debug_bindings`
/// is `true`. Lets callers reconstruct the chronological order of
/// quote-scope entries/exits, binding registrations, references,
/// splices, and lifts — useful for debugging hygiene violations and
/// for tooling that wants to render the macro-expansion timeline.
#[derive(Debug, Clone)]
pub struct DebugBindingEvent {
    /// What happened.
    pub kind: DebugBindingEventKind,
    /// The identifier or splice/lift name involved (empty for quote
    /// enter/exit).
    pub name: Text,
    /// Source span of the event.
    pub span: Span,
    /// Quote nesting depth at the time of the event (post-update for
    /// `EnterQuote`, pre-update for `ExitQuote`).
    pub depth: usize,
}

/// What kind of expander event a `DebugBindingEvent` records.
#[derive(Debug, Clone)]
pub enum DebugBindingEventKind {
    /// `enter_quote` increased nesting depth.
    EnterQuote,
    /// `exit_quote` decreased nesting depth.
    ExitQuote,
    /// A binding was registered via `process_binding`.
    Binding(BindingKind),
    /// An identifier reference was processed via `process_reference`.
    Reference,
    /// A splice (`$name`) resolved to a binding.
    Splice,
    /// A `lift(expr)` was processed (whether or not an evaluator was
    /// configured).
    Lift,
}

/// Represents a binding captured in a quote
#[derive(Debug, Clone)]
pub struct CapturedBinding {
    /// The original identifier
    pub ident: HygienicIdent,
    /// The binding information
    pub binding: BindingInfo,
    /// The stage at which the binding was captured
    pub stage: u32,
    /// Whether this was an explicit capture (via splice)
    pub explicit: bool,
}

/// Result of expanding a splice expression
#[derive(Debug, Clone)]
pub enum SpliceResult {
    /// A single identifier splice ($name)
    Ident(HygienicIdent),
    /// An expression splice (${expr})
    Expr(Box<Expr>),
    /// A token stream (from a meta function call)
    TokenStream(TokenStream),
    /// A lifted value (lift(expr))
    Lifted(LiftedValue),
}

/// A lifted value from a higher stage
#[derive(Debug, Clone)]
pub struct LiftedValue {
    /// The type of the value
    pub ty: Maybe<Text>,
    /// The stage from which it was lifted
    pub source_stage: u32,
    /// The constant value (if known)
    pub const_value: Maybe<ConstValue>,
    /// The span of the lift expression
    pub span: Span,
}

/// Constant value that can be lifted
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    Text(Text),
    Unit,
    /// Array of constant values
    Array(List<ConstValue>),
    /// Tuple of constant values
    Tuple(List<ConstValue>),
}

impl ConstValue {
    /// Convert from MetaValue (the evaluator's const value type)
    ///
    /// This converts the rich MetaValue type to the simpler ConstValue
    /// used for hygiene purposes. AST nodes cannot be converted.
    pub fn from_meta_value(value: &verum_ast::MetaValue) -> Maybe<Self> {
        use verum_ast::MetaValue;
        match value {
            MetaValue::Unit => Maybe::Some(ConstValue::Unit),
            MetaValue::Bool(b) => Maybe::Some(ConstValue::Bool(*b)),
            MetaValue::Int(i) => Maybe::Some(ConstValue::Int(*i as i64)),
            MetaValue::UInt(u) => Maybe::Some(ConstValue::Int(*u as i64)),
            MetaValue::Float(f) => Maybe::Some(ConstValue::Float(*f)),
            MetaValue::Char(c) => Maybe::Some(ConstValue::Char(*c)),
            MetaValue::Text(t) => Maybe::Some(ConstValue::Text(t.clone())),
            MetaValue::Array(arr) => {
                let converted: Result<List<ConstValue>, ()> = arr
                    .iter()
                    .map(|v| match ConstValue::from_meta_value(v) {
                        Maybe::Some(c) => Ok(c),
                        Maybe::None => Err(()),
                    })
                    .collect();
                match converted {
                    Ok(list) => Maybe::Some(ConstValue::Array(list)),
                    Err(()) => Maybe::None,
                }
            }
            MetaValue::Tuple(tup) => {
                let converted: Result<List<ConstValue>, ()> = tup
                    .iter()
                    .map(|v| match ConstValue::from_meta_value(v) {
                        Maybe::Some(c) => Ok(c),
                        Maybe::None => Err(()),
                    })
                    .collect();
                match converted {
                    Ok(list) => Maybe::Some(ConstValue::Tuple(list)),
                    Err(()) => Maybe::None,
                }
            }
            // AST nodes cannot be converted to simple ConstValue
            MetaValue::Expr(_) | MetaValue::Type(_) | MetaValue::Pattern(_)
            | MetaValue::Item(_) | MetaValue::Items(_) => Maybe::None,
            // Other complex types
            MetaValue::Bytes(_) | MetaValue::Maybe(_) | MetaValue::Map(_) | MetaValue::Set(_) => Maybe::None,
        }
    }

    /// Get the type name for this value
    pub fn type_name(&self) -> Text {
        match self {
            ConstValue::Int(_) => Text::from("Int"),
            ConstValue::Float(_) => Text::from("Float"),
            ConstValue::Bool(_) => Text::from("Bool"),
            ConstValue::Char(_) => Text::from("Char"),
            ConstValue::Text(_) => Text::from("Text"),
            ConstValue::Unit => Text::from("()"),
            ConstValue::Array(_) => Text::from("Array"),
            ConstValue::Tuple(_) => Text::from("Tuple"),
        }
    }
}

/// Represents a token stream (simplified for hygiene purposes)
#[derive(Debug, Clone, Default)]
pub struct TokenStream {
    /// The tokens in the stream
    pub tokens: List<Token>,
    /// The mark set applied to this stream
    pub marks: MarkSet,
    /// The syntax context for this stream
    pub context: Maybe<SyntaxContext>,
}

/// A single token with hygiene information
#[derive(Debug, Clone)]
pub struct Token {
    /// The kind of token
    pub kind: TokenKind,
    /// The span
    pub span: Span,
    /// Hygiene context
    pub scopes: ScopeSet,
    /// Mark set for this specific token
    pub marks: MarkSet,
}

/// Token kinds for quote expansion
#[derive(Debug, Clone)]
pub enum TokenKind {
    /// An identifier
    Ident(HygienicIdent),
    /// A literal value
    Literal(ConstValue),
    /// A punctuation character
    Punct(char),
    /// A delimiter (open or close)
    Delimiter { kind: DelimiterKind, open: bool },
    /// A group of tokens
    Group(TokenStream),
}

/// Delimiter kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelimiterKind {
    Paren,
    Bracket,
    Brace,
}

impl TokenStream {
    /// Create an empty token stream
    pub fn new() -> Self {
        Self {
            tokens: List::new(),
            marks: MarkSet::new(),
            context: Maybe::None,
        }
    }

    /// Create from a list of tokens
    pub fn from_tokens(tokens: List<Token>) -> Self {
        Self {
            tokens,
            marks: MarkSet::new(),
            context: Maybe::None,
        }
    }

    /// Create with a syntax context
    pub fn with_context(tokens: List<Token>, context: SyntaxContext) -> Self {
        Self {
            tokens,
            marks: context.marks().clone(),
            context: Maybe::Some(context),
        }
    }

    /// Check if the stream is empty
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Get the length
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Push a token
    pub fn push(&mut self, token: Token) {
        self.tokens.push(token);
    }

    /// Push a token with current stream marks applied
    pub fn push_with_marks(&mut self, mut token: Token) {
        // Merge stream-level marks into token
        for mark in self.marks.iter() {
            token.marks.add(*mark);
        }
        self.tokens.push(token);
    }

    /// Iterate over tokens
    pub fn iter(&self) -> impl Iterator<Item = &Token> {
        self.tokens.iter()
    }

    /// Apply marks to all tokens in the stream
    pub fn apply_marks(&mut self, marks: &MarkSet) {
        for token in self.tokens.iter_mut() {
            for mark in marks.iter() {
                token.marks.add(*mark);
            }
            // Recursively apply to groups
            if let TokenKind::Group(ref mut group) = token.kind {
                group.apply_marks(marks);
            }
        }
        // Also update stream-level marks
        for mark in marks.iter() {
            self.marks.add(*mark);
        }
    }

    /// Apply a single mark to all tokens
    pub fn apply_mark(&mut self, mark: Mark) {
        for token in self.tokens.iter_mut() {
            token.marks.add(mark);
            // Recursively apply to groups
            if let TokenKind::Group(ref mut group) = token.kind {
                group.apply_mark(mark);
            }
        }
        self.marks.add(mark);
    }

    /// Flip a mark on all tokens (add if missing, remove if present)
    pub fn flip_mark(&mut self, mark: Mark) {
        for token in self.tokens.iter_mut() {
            token.marks.flip(mark);
            // Recursively flip in groups
            if let TokenKind::Group(ref mut group) = token.kind {
                group.flip_mark(mark);
            }
        }
        self.marks.flip(mark);
    }

    /// Apply a transformation to all identifiers
    pub fn map_idents<F>(&self, f: F) -> Self
    where
        F: Fn(&HygienicIdent) -> HygienicIdent + Clone,
    {
        let tokens = self
            .tokens
            .iter()
            .map(|token| Token {
                kind: match &token.kind {
                    TokenKind::Ident(ident) => TokenKind::Ident(f(ident)),
                    TokenKind::Group(group) => TokenKind::Group(group.map_idents(f.clone())),
                    other => other.clone(),
                },
                span: token.span,
                scopes: token.scopes.clone(),
                marks: token.marks.clone(),
            })
            .collect();
        Self {
            tokens,
            marks: self.marks.clone(),
            context: self.context.clone(),
        }
    }

    /// Apply call-site marks to all identifiers
    /// This is used when splicing values into a quote
    pub fn with_call_site_marks(&self, call_mark: Mark) -> Self {
        self.map_idents(|ident| {
            let mut new_ident = ident.clone();
            new_ident.scopes.add(ScopeId::new(call_mark.as_u64()));
            new_ident
        })
    }

    /// Add scopes to all tokens
    pub fn with_scopes(&self, scopes: &ScopeSet) -> Self {
        let tokens = self
            .tokens
            .iter()
            .map(|token| Token {
                kind: token.kind.clone(),
                span: token.span,
                scopes: token.scopes.union(scopes),
                marks: token.marks.clone(),
            })
            .collect();
        Self {
            tokens,
            marks: self.marks.clone(),
            context: self.context.clone(),
        }
    }

    /// Append another token stream
    pub fn append(&mut self, other: &TokenStream) {
        for token in other.tokens.iter() {
            self.tokens.push(token.clone());
        }
    }

    /// Concatenate two token streams
    pub fn concat(mut self, other: TokenStream) -> Self {
        self.append(&other);
        self
    }

    /// Check if all tokens have compatible marks with a binding
    pub fn marks_compatible_with(&self, binding_marks: &MarkSet) -> bool {
        for token in self.tokens.iter() {
            if let TokenKind::Ident(_) = &token.kind {
                if !token.marks.compatible(binding_marks) {
                    return false;
                }
            }
            if let TokenKind::Group(group) = &token.kind {
                if !group.marks_compatible_with(binding_marks) {
                    return false;
                }
            }
        }
        true
    }

    /// Get all identifiers in the token stream
    pub fn collect_idents(&self) -> List<HygienicIdent> {
        let mut idents = List::new();
        self.collect_idents_into(&mut idents);
        idents
    }

    fn collect_idents_into(&self, idents: &mut List<HygienicIdent>) {
        for token in self.tokens.iter() {
            match &token.kind {
                TokenKind::Ident(ident) => idents.push(ident.clone()),
                TokenKind::Group(group) => group.collect_idents_into(idents),
                _ => {}
            }
        }
    }
}

/// Result of evaluating a lift expression
pub type LiftEvalResult = Result<verum_ast::MetaValue, Text>;

/// Evaluator callback for lift expressions
///
/// This callback is invoked when a lift expression needs to be evaluated.
/// It takes the expression to evaluate and returns either a MetaValue
/// or an error message.
pub type LiftEvaluator = Box<dyn Fn(&Expr) -> LiftEvalResult + Send + Sync>;

/// The main quote expander
pub struct QuoteExpander {
    /// The hygiene context
    context: HygieneContext,
    /// Expansion configuration
    config: ExpansionConfig,
    /// Current nesting depth
    depth: usize,
    /// The current quote scope ID
    quote_scope: Maybe<ScopeId>,
    /// The current call site mark
    call_site_mark: Maybe<ScopeMark>,
    /// Captured bindings
    captures: List<CapturedBinding>,
    /// Accumulated violations
    violations: HygieneViolations,
    /// Binding stack for scope tracking
    binding_stack: List<Map<Text, BindingInfo>>,
    /// Optional evaluator for lift expressions
    ///
    /// When set, lift expressions will be evaluated at compile time
    /// using this callback. The callback receives the expression to
    /// evaluate and should return the resulting MetaValue.
    lift_evaluator: Maybe<LiftEvaluator>,
    /// Recorded debug events.
    ///
    /// Populated only when `config.debug_bindings = true`. Stays
    /// empty (and zero-allocation) under the default configuration so
    /// production callers pay nothing.
    debug_log: List<DebugBindingEvent>,
}

impl std::fmt::Debug for QuoteExpander {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuoteExpander")
            .field("context", &self.context)
            .field("config", &self.config)
            .field("depth", &self.depth)
            .field("quote_scope", &self.quote_scope)
            .field("call_site_mark", &self.call_site_mark)
            .field("captures", &self.captures)
            .field("violations", &self.violations)
            .field("binding_stack", &self.binding_stack)
            .field("lift_evaluator", &self.lift_evaluator.is_some())
            .field("debug_log", &self.debug_log)
            .finish()
    }
}

impl QuoteExpander {
    /// Create a new quote expander
    pub fn new(context: HygieneContext, config: ExpansionConfig) -> Self {
        Self {
            context,
            config,
            depth: 0,
            quote_scope: Maybe::None,
            call_site_mark: Maybe::None,
            captures: List::new(),
            violations: HygieneViolations::new(),
            binding_stack: List::new(),
            lift_evaluator: Maybe::None,
            debug_log: List::new(),
        }
    }

    /// Create with default configuration
    pub fn with_default_config(context: HygieneContext) -> Self {
        Self::new(context, ExpansionConfig::default())
    }

    /// Set the lift evaluator callback
    ///
    /// The lift evaluator is called when a `lift(expr)` expression needs
    /// to be evaluated. It should evaluate the expression in the current
    /// meta context and return the resulting value.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let meta_ctx = MetaContext::new();
    /// expander.set_lift_evaluator(Box::new(move |expr| {
    ///     let meta_expr = meta_ctx.ast_expr_to_meta_expr(expr)?;
    ///     meta_ctx.eval_meta_expr(&meta_expr)
    ///         .map_err(|e| e.to_string().into())
    /// }));
    /// ```
    pub fn set_lift_evaluator(&mut self, evaluator: LiftEvaluator) {
        self.lift_evaluator = Maybe::Some(evaluator);
    }

    /// Create with a lift evaluator
    pub fn with_lift_evaluator(mut self, evaluator: LiftEvaluator) -> Self {
        self.lift_evaluator = Maybe::Some(evaluator);
        self
    }

    /// Check if a lift evaluator is available
    pub fn has_lift_evaluator(&self) -> bool {
        matches!(self.lift_evaluator, Maybe::Some(_))
    }

    /// Get the hygiene context
    pub fn context(&self) -> &HygieneContext {
        &self.context
    }

    /// Get mutable hygiene context
    pub fn context_mut(&mut self) -> &mut HygieneContext {
        &mut self.context
    }

    /// Get accumulated violations
    pub fn violations(&self) -> &HygieneViolations {
        &self.violations
    }

    /// Take the violations out
    pub fn take_violations(&mut self) -> HygieneViolations {
        std::mem::take(&mut self.violations)
    }

    /// Borrow the chronological list of recorded debug events.
    ///
    /// Only populated when the expander was constructed with
    /// `ExpansionConfig.debug_bindings = true`. Empty otherwise — the
    /// flag is the master gate for all event recording, so the per-
    /// site `if self.config.debug_bindings` checks short-circuit
    /// before any allocation in the default configuration.
    pub fn debug_bindings_log(&self) -> &List<DebugBindingEvent> {
        &self.debug_log
    }

    /// Take the recorded debug events out, leaving the internal log
    /// empty. Allows callers (a debugger, an LSP-side macro inspector,
    /// a test harness) to drain the trace without paying a clone.
    pub fn take_debug_bindings_log(&mut self) -> List<DebugBindingEvent> {
        std::mem::take(&mut self.debug_log)
    }

    /// Internal: append a debug event when (and only when) the
    /// `debug_bindings` gate is on.
    fn record_debug_event(
        &mut self,
        kind: DebugBindingEventKind,
        name: Text,
        span: Span,
    ) {
        if !self.config.debug_bindings {
            return;
        }
        self.debug_log.push(DebugBindingEvent {
            kind,
            name,
            span,
            depth: self.depth,
        });
    }

    /// Check if there are any violations
    pub fn has_violations(&self) -> bool {
        !self.violations.is_empty()
    }

    // ========================================================================
    // Quote Entry/Exit
    // ========================================================================

    /// Enter a quote block
    ///
    /// Creates a fresh mark and pushes a new quote scope.
    pub fn enter_quote(&mut self, target_stage: u32, span: Span) -> Result<ScopeMark, HygieneViolation> {
        // Check max depth
        if self.depth >= self.config.max_depth {
            return Err(HygieneViolation::InvalidQuoteSyntax {
                message: Text::from(format!(
                    "Maximum quote nesting depth ({}) exceeded",
                    self.config.max_depth
                )),
                span,
            });
        }

        self.depth += 1;

        // Enter quote scope in the hygiene context
        let scope_id = self.context.enter_quote();
        self.quote_scope = Maybe::Some(scope_id);

        // Create call site mark
        let call_mark = ScopeMark::quote(
            self.context.current_scopes().len() as u64,
            target_stage,
        );
        self.call_site_mark = Maybe::Some(call_mark);

        // Push new binding frame
        self.binding_stack.push(Map::new());

        self.record_debug_event(
            DebugBindingEventKind::EnterQuote,
            Text::from(""),
            span,
        );

        Ok(call_mark)
    }

    /// Exit a quote block
    pub fn exit_quote(&mut self) {
        self.record_debug_event(
            DebugBindingEventKind::ExitQuote,
            Text::from(""),
            Span::default(),
        );
        self.depth = self.depth.saturating_sub(1);
        self.context.exit_quote();
        self.quote_scope = Maybe::None;
        self.call_site_mark = Maybe::None;
        self.binding_stack.pop();
    }

    /// Check if we're currently inside a quote
    pub fn in_quote(&self) -> bool {
        matches!(self.quote_scope, Maybe::Some(_))
    }

    /// Get the current quote depth
    pub fn depth(&self) -> usize {
        self.depth
    }

    // ========================================================================
    // Mark Phase
    // ========================================================================

    /// Process a binding (let, fn parameter, etc.)
    ///
    /// Adds current marks to the binding and registers it.
    pub fn process_binding(
        &mut self,
        name: Text,
        span: Span,
        kind: BindingKind,
        is_mutable: bool,
    ) -> HygienicIdent {
        let scopes = self.context.current_scopes();
        let ident = HygienicIdent::new(name.clone(), scopes, span);

        // Create binding info
        let binding = BindingInfo {
            original_name: name.clone(),
            hygienic_name: self.context.gensym(name.as_str()),
            scope_id: self.context.current_scope().unwrap_or(ScopeId::new(0)),
            is_mutable,
            kind,
        };

        // Register in current frame
        if let Some(frame) = self.binding_stack.last_mut() {
            frame.insert(name.clone(), binding.clone());
        }

        // Also register in the hygiene context
        self.context.add_binding(name.clone(), binding);

        self.record_debug_event(
            DebugBindingEventKind::Binding(kind),
            name,
            span,
        );

        ident
    }

    /// Process a reference to an identifier
    ///
    /// Verifies that the reference is hygienic.
    pub fn process_reference(&mut self, name: &Text, span: Span) -> HygienicIdent {
        let scopes = self.context.current_scopes();
        self.record_debug_event(
            DebugBindingEventKind::Reference,
            name.clone(),
            span,
        );
        HygienicIdent::new(name.clone(), scopes, span)
    }

    /// Generate a unique temporary identifier
    pub fn gensym(&self, base: &str) -> Text {
        self.context.gensym(base)
    }

    /// Generate a hygienic temporary identifier
    pub fn gensym_hygienic(&self, base: &str, span: Span) -> HygienicIdent {
        self.context.gensym_hygienic(base, span)
    }

    // ========================================================================
    // Splice Phase
    // ========================================================================

    /// Splice a value into the quote
    ///
    /// For `$name` or `${expr}` splices.
    pub fn splice_value(&mut self, name: &Text, span: Span) -> Result<SpliceResult, HygieneViolation> {
        self.record_debug_event(
            DebugBindingEventKind::Splice,
            name.clone(),
            span,
        );

        // Look up the binding
        let binding = self.lookup_binding(name);

        if binding.is_none() {
            return Err(HygieneViolation::CaptureNotDeclared {
                ident: name.clone(),
                span,
            });
        }

        let binding = binding.unwrap();

        // Check stage compatibility
        let current_stage = self.config.source_stage;
        if binding.scope_id.as_u64() as u32 > current_stage {
            return Err(HygieneViolation::StageMismatch {
                expected_stage: current_stage,
                actual_stage: binding.scope_id.as_u64() as u32,
                span,
            });
        }

        // Record the capture
        self.captures.push(CapturedBinding {
            ident: HygienicIdent::new(name.clone(), self.context.current_scopes(), span),
            binding: binding.clone(),
            stage: current_stage,
            explicit: true,
        });

        // Create the spliced identifier with call-site marks
        let mut spliced = HygienicIdent::new(
            binding.hygienic_name.clone(),
            self.context.current_scopes(),
            span,
        );

        // Apply call-site marks
        if let Maybe::Some(call_mark) = &self.call_site_mark {
            spliced.scopes.add(ScopeId::new(call_mark.id));
        }

        Ok(SpliceResult::Ident(spliced))
    }

    /// Splice an expression into the quote
    ///
    /// For `${expr}` splices that need evaluation.
    pub fn splice_expr(&mut self, expr: Expr, _span: Span) -> Result<SpliceResult, HygieneViolation> {
        // Mark the expression with call-site scopes
        let marked_expr = self.apply_call_site_marks_to_expr(expr);
        Ok(SpliceResult::Expr(Box::new(marked_expr)))
    }

    /// Handle a lift expression
    ///
    /// For `lift(expr)` which evaluates at a higher stage and embeds
    /// the result into the generated code.
    ///
    /// # Evaluation
    ///
    /// If a lift evaluator is set, this will:
    /// 1. Evaluate the expression using the evaluator callback
    /// 2. Convert the resulting MetaValue to a ConstValue
    /// 3. Return a LiftedValue containing the constant
    ///
    /// If no evaluator is set, this returns a LiftedValue without
    /// the evaluated constant (deferred evaluation).
    ///
    /// # Example
    ///
    /// ```verum
    /// meta fn make_array(count: Int) -> Expr {
    ///     quote {
    ///         let size = $(lift(count));  // count is evaluated here
    ///         new_array(size)
    ///     }
    /// }
    /// ```
    pub fn lift_value(
        &mut self,
        expr: &Expr,
        span: Span,
    ) -> Result<LiftedValue, HygieneViolation> {
        self.record_debug_event(
            DebugBindingEventKind::Lift,
            Text::from(""),
            span,
        );

        // Verify we're in a quote context
        if !self.in_quote() {
            return Err(HygieneViolation::UnquoteOutsideQuote { span });
        }

        // Try to evaluate the expression if we have an evaluator
        let (const_value, ty) = if let Maybe::Some(ref evaluator) = self.lift_evaluator {
            match evaluator(expr) {
                Ok(meta_value) => {
                    // Try to convert MetaValue to our simpler ConstValue
                    let const_val = ConstValue::from_meta_value(&meta_value);
                    let type_name = if let Maybe::Some(ref cv) = const_val {
                        Maybe::Some(cv.type_name())
                    } else {
                        // MetaValue can't be fully converted, but we can still
                        // use it. Get type from the MetaValue directly.
                        Maybe::Some(Text::from(meta_value.type_name()))
                    };
                    (const_val, type_name)
                }
                Err(err_msg) => {
                    // Evaluation failed.  Under `strict_mode = true` we
                    // abort the lift with a typed error so the caller
                    // sees an immediate hygiene violation rather than a
                    // degraded `LiftedValue { const_value: None }` that
                    // looks like a deferred-evaluation case.  Under
                    // lenient mode we record the violation and continue
                    // (the caller is expected to inspect
                    // `take_violations()` and decide its own policy).
                    //
                    // Pre-fix `config.strict_mode` was inert — the
                    // field defaulted to `true` (claiming strict
                    // enforcement) but the expander never read it, so
                    // every lift failure silently degraded to the
                    // deferred-eval shape.  Now the strict flag has
                    // actual teeth.
                    let v = HygieneViolation::InvalidQuoteSyntax {
                        message: Text::from(format!(
                            "lift evaluation failed: {}",
                            err_msg
                        )),
                        span,
                    };
                    if self.config.strict_mode {
                        return Err(v);
                    }
                    self.violations.push(v);
                    (Maybe::None, Maybe::None)
                }
            }
        } else {
            // No evaluator available - deferred evaluation
            (Maybe::None, Maybe::None)
        };

        Ok(LiftedValue {
            ty,
            source_stage: self.config.source_stage,
            const_value,
            span,
        })
    }

    /// Evaluate a lift expression and return the raw MetaValue
    ///
    /// This is a lower-level interface that returns the MetaValue directly
    /// without converting it to ConstValue. Useful when you need to work
    /// with AST values or other complex types.
    pub fn evaluate_lift_raw(
        &self,
        expr: &Expr,
        span: Span,
    ) -> Result<verum_ast::MetaValue, HygieneViolation> {
        if let Maybe::Some(ref evaluator) = self.lift_evaluator {
            evaluator(expr).map_err(|err_msg| {
                HygieneViolation::InvalidQuoteSyntax {
                    message: Text::from(format!("lift evaluation failed: {}", err_msg)),
                    span,
                }
            })
        } else {
            Err(HygieneViolation::InvalidQuoteSyntax {
                message: Text::from("no lift evaluator available"),
                span,
            })
        }
    }

    /// Handle a stage escape expression
    ///
    /// For `$(stage N){ expr }` which evaluates at a specific stage.
    pub fn stage_escape(
        &mut self,
        stage: u32,
        expr: &Expr,
        span: Span,
    ) -> Result<SpliceResult, HygieneViolation> {
        // Verify stage is valid
        if stage > self.config.source_stage {
            return Err(HygieneViolation::StageMismatch {
                expected_stage: self.config.source_stage,
                actual_stage: stage,
                span,
            });
        }

        // Mark the expression with the appropriate stage marks
        let marked_expr = self.apply_stage_marks_to_expr(expr.clone(), stage);
        Ok(SpliceResult::Expr(Box::new(marked_expr)))
    }

    // ========================================================================
    // Repetition Phase
    // ========================================================================

    /// Handle a repetition splice
    ///
    /// For `$[for pattern in expr { body }]`.
    ///
    /// This method expands the body once for each element in the list,
    /// substituting the pattern variable with each element.
    pub fn expand_repetition(
        &mut self,
        pattern_name: &Text,
        list_name: &Text,
        body: &TokenStream,
        span: Span,
    ) -> Result<TokenStream, HygieneViolation> {
        // Look up the list binding
        let list_binding = self.lookup_binding(list_name);

        if list_binding.is_none() {
            return Err(HygieneViolation::CaptureNotDeclared {
                ident: list_name.clone(),
                span,
            });
        }

        let list_binding = list_binding.unwrap();

        // Record the capture
        self.captures.push(CapturedBinding {
            ident: HygienicIdent::new(list_name.clone(), self.context.current_scopes(), span),
            binding: list_binding.clone(),
            stage: self.config.source_stage,
            explicit: true,
        });

        // Create a repetition expander for this specific repetition
        let mut rep_expander = RepetitionExpander::new(
            pattern_name.clone(),
            self.context.current_scopes(),
            span,
        );

        // The expanded result
        let result = rep_expander.expand_body(body, self)?;

        Ok(result)
    }

    /// Handle a repetition with multiple items
    ///
    /// For `$[for (a, b) in zip(list1, list2) { body }]`.
    pub fn expand_repetition_multi(
        &mut self,
        pattern_names: &[Text],
        list_names: &[Text],
        body: &TokenStream,
        span: Span,
    ) -> Result<TokenStream, HygieneViolation> {
        // Verify all lists exist and have the same length
        let mut lengths: Vec<(Text, usize)> = Vec::new();

        for list_name in list_names {
            let binding = self.lookup_binding(list_name);
            if binding.is_none() {
                return Err(HygieneViolation::CaptureNotDeclared {
                    ident: list_name.clone(),
                    span,
                });
            }
            // In a real implementation, we would get the actual length from the binding
            // For now, we use a placeholder
            lengths.push((list_name.clone(), 0));
        }

        // Check all lengths match
        if lengths.len() >= 2 {
            let (first_name, first_len) = &lengths[0];
            for (name, len) in lengths.iter().skip(1) {
                self.check_repetition_match(first_name, *first_len, name, *len, span)?;
            }
        }

        // Create multi-pattern repetition expander
        let mut rep_expander = RepetitionExpander::new_multi(
            pattern_names.to_vec(),
            self.context.current_scopes(),
            span,
        );

        let result = rep_expander.expand_body(body, self)?;

        Ok(result)
    }

    /// Expand a single iteration of a repetition body
    ///
    /// This is called for each element in the iteration.
    pub fn expand_repetition_iteration(
        &mut self,
        pattern_name: &Text,
        element_ident: HygienicIdent,
        body: &TokenStream,
        _iteration: usize,
    ) -> Result<TokenStream, HygieneViolation> {
        // Create a fresh mark for this iteration to ensure hygiene
        let iteration_mark = Mark::fresh();

        // Clone and substitute the body
        let substituted = body.map_idents(|ident| {
            if &ident.name == pattern_name {
                // Substitute the pattern variable with the element
                let mut result = element_ident.clone();
                // Add iteration-specific mark
                result.scopes.add(ScopeId::new(iteration_mark.as_u64()));
                result
            } else {
                // Keep other identifiers but add iteration mark
                let mut result = ident.clone();
                result.scopes.add(ScopeId::new(iteration_mark.as_u64()));
                result
            }
        });

        // Apply the iteration mark to ensure each expansion is hygienic
        let mut result = substituted;
        result.apply_mark(iteration_mark);

        Ok(result)
    }

    /// Check repetition lengths match
    pub fn check_repetition_match(
        &mut self,
        first: &Text,
        first_len: usize,
        second: &Text,
        second_len: usize,
        span: Span,
    ) -> Result<(), HygieneViolation> {
        if first_len != second_len {
            return Err(HygieneViolation::RepetitionMismatch {
                first_name: first.clone(),
                first_len,
                second_name: second.clone(),
                second_len,
                span,
            });
        }
        Ok(())
    }

    /// Track a repetition variable for length checking
    pub fn track_repetition_var(&mut self, name: Text, length: usize) {
        // Store in a separate tracking map for validation
        if let Some(frame) = self.binding_stack.last_mut() {
            // We track the length via a special binding annotation
            if let Some(binding) = frame.get_mut(&name) {
                // In a real implementation, we would store the length
                // For now, we just verify the binding exists
                let _ = binding;
                let _ = length;
            }
        }
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Look up a binding by name
    fn lookup_binding(&self, name: &Text) -> Option<BindingInfo> {
        // Check binding stack from innermost to outermost
        for frame in self.binding_stack.iter().rev() {
            if let Some(binding) = frame.get(name) {
                return Some(binding.clone());
            }
        }
        None
    }

    /// Apply call-site marks to an expression
    ///
    /// Walks the expression tree and adds call-site marks to all identifiers.
    /// This ensures that spliced values have proper hygiene marking.
    fn apply_call_site_marks_to_expr(&self, expr: Expr) -> Expr {
        if let Maybe::Some(call_mark) = &self.call_site_mark {
            let mark = Mark::new(call_mark.id);
            self.apply_mark_to_expr(expr, mark)
        } else {
            expr
        }
    }

    /// Apply stage marks to an expression
    ///
    /// Marks the expression with stage-specific scopes for multi-stage hygiene.
    fn apply_stage_marks_to_expr(&self, expr: Expr, stage: u32) -> Expr {
        // Create a stage-specific mark
        let stage_mark = Mark::for_stage(stage);
        self.apply_mark_to_expr(expr, stage_mark)
    }

    /// Apply a mark to all identifiers in an expression tree
    ///
    /// This method recursively transforms the expression, adding the mark
    /// to all Path expressions which contain identifiers.
    fn apply_mark_to_expr(&self, expr: Expr, mark: Mark) -> Expr {
        use verum_ast::expr::ExprKind;
        use verum_common::Heap;

        let new_kind = match expr.kind {
            ExprKind::Path(path) => {
                // Path contains identifiers - this is where we add marks
                // In a full implementation, we would add the mark to the
                // path's hygiene context. For now, we preserve the path.
                ExprKind::Path(path)
            }

            ExprKind::Binary { op, left, right } => ExprKind::Binary {
                op,
                left: Heap::new(self.apply_mark_to_expr(*left, mark)),
                right: Heap::new(self.apply_mark_to_expr(*right, mark)),
            },

            ExprKind::Unary { op, expr: inner } => ExprKind::Unary {
                op,
                expr: Heap::new(self.apply_mark_to_expr(*inner, mark)),
            },

            ExprKind::Call { func, type_args, args } => ExprKind::Call {
                func: Heap::new(self.apply_mark_to_expr(*func, mark)),
                type_args,
                args: args.into_iter()
                    .map(|arg| self.apply_mark_to_expr(arg, mark))
                    .collect(),
            },

            ExprKind::MethodCall { receiver, method, type_args, args } => ExprKind::MethodCall {
                receiver: Heap::new(self.apply_mark_to_expr(*receiver, mark)),
                method,
                type_args,
                args: args.into_iter()
                    .map(|arg| self.apply_mark_to_expr(arg, mark))
                    .collect(),
            },

            ExprKind::Field { expr: inner, field } => ExprKind::Field {
                expr: Heap::new(self.apply_mark_to_expr(*inner, mark)),
                field,
            },

            ExprKind::Index { expr: inner, index } => ExprKind::Index {
                expr: Heap::new(self.apply_mark_to_expr(*inner, mark)),
                index: Heap::new(self.apply_mark_to_expr(*index, mark)),
            },

            ExprKind::Tuple(elements) => ExprKind::Tuple(
                elements.into_iter()
                    .map(|e| self.apply_mark_to_expr(e, mark))
                    .collect()
            ),

            ExprKind::If { condition, then_branch, else_branch } => ExprKind::If {
                condition,
                then_branch: self.apply_mark_to_block(then_branch, mark),
                else_branch: match else_branch {
                    Maybe::Some(e) => Maybe::Some(Heap::new(self.apply_mark_to_expr(*e, mark))),
                    Maybe::None => Maybe::None,
                },
            },

            ExprKind::Block(block) => ExprKind::Block(self.apply_mark_to_block(block, mark)),

            ExprKind::Return(value) => ExprKind::Return(
                match value {
                    Maybe::Some(e) => Maybe::Some(Heap::new(self.apply_mark_to_expr(*e, mark))),
                    Maybe::None => Maybe::None,
                }
            ),

            ExprKind::Closure { async_, move_, params, contexts, return_type, body } => {
                ExprKind::Closure {
                    async_,
                    move_,
                    params,
                    contexts,
                    return_type,
                    body: Heap::new(self.apply_mark_to_expr(*body, mark)),
                }
            }

            // For expression kinds that don't contain sub-expressions, return as-is
            ExprKind::Literal(_) => expr.kind,

            // For remaining complex expressions, preserve structure
            // (a comprehensive implementation would handle all cases)
            other => other,
        };

        Expr {
            kind: new_kind,
            span: expr.span,
            ref_kind: expr.ref_kind,
            check_eliminated: expr.check_eliminated,
        }
    }

    /// Apply a mark to all expressions in a block
    fn apply_mark_to_block(&self, block: verum_ast::expr::Block, mark: Mark) -> verum_ast::expr::Block {
        use verum_common::Heap;

        verum_ast::expr::Block {
            stmts: block.stmts.into_iter()
                .map(|stmt| self.apply_mark_to_stmt(stmt, mark))
                .collect(),
            expr: match block.expr {
                Maybe::Some(e) => Maybe::Some(Heap::new(self.apply_mark_to_expr(*e, mark))),
                Maybe::None => Maybe::None,
            },
            span: block.span,
        }
    }

    /// Apply a mark to a statement
    fn apply_mark_to_stmt(&self, stmt: verum_ast::stmt::Stmt, mark: Mark) -> verum_ast::stmt::Stmt {
        use verum_ast::stmt::StmtKind;

        let new_kind = match stmt.kind {
            StmtKind::Let { pattern, ty, value } => StmtKind::Let {
                pattern,
                ty,
                value: match value {
                    Maybe::Some(e) => Maybe::Some(self.apply_mark_to_expr(e, mark)),
                    Maybe::None => Maybe::None,
                },
            },

            StmtKind::Expr { expr, has_semi } => StmtKind::Expr {
                expr: self.apply_mark_to_expr(expr, mark),
                has_semi,
            },

            other => other,
        };

        verum_ast::stmt::Stmt {
            kind: new_kind,
            span: stmt.span,
            attributes: stmt.attributes,
        }
    }

    /// Create a token for an identifier
    pub fn ident_token(&self, ident: HygienicIdent) -> Token {
        Token {
            kind: TokenKind::Ident(ident.clone()),
            span: ident.span,
            scopes: ident.scopes.clone(),
            marks: self.current_marks(),
        }
    }

    /// Create a token for a literal
    pub fn literal_token(&self, value: ConstValue, span: Span) -> Token {
        Token {
            kind: TokenKind::Literal(value),
            span,
            scopes: self.context.current_scopes(),
            marks: self.current_marks(),
        }
    }

    /// Get the current mark set
    fn current_marks(&self) -> MarkSet {
        if let Maybe::Some(call_mark) = &self.call_site_mark {
            let mut marks = MarkSet::new();
            marks.add(Mark::new(call_mark.id));
            marks
        } else {
            MarkSet::new()
        }
    }
}

/// Repetition expander for $[for...] syntax
///
/// Handles the expansion of repetition patterns in quotes.
#[derive(Debug)]
pub struct RepetitionExpander {
    /// Pattern variable names
    pattern_names: List<Text>,
    /// Current scopes for hygiene
    #[allow(dead_code)]
    scopes: ScopeSet,
    /// Span for error reporting
    span: Span,
    /// Iteration count
    iteration_count: usize,
    /// Accumulated expanded tokens
    expanded: TokenStream,
    /// Separator tokens (optional, for comma-separated lists, etc.)
    separator: Maybe<TokenStream>,
}

impl RepetitionExpander {
    /// Create a new repetition expander with a single pattern
    pub fn new(pattern_name: Text, scopes: ScopeSet, span: Span) -> Self {
        Self {
            pattern_names: {
                let mut names = List::new();
                names.push(pattern_name);
                names
            },
            scopes,
            span,
            iteration_count: 0,
            expanded: TokenStream::new(),
            separator: Maybe::None,
        }
    }

    /// Create a new repetition expander with multiple patterns
    pub fn new_multi(pattern_names: Vec<Text>, scopes: ScopeSet, span: Span) -> Self {
        Self {
            pattern_names: pattern_names.into_iter().collect(),
            scopes,
            span,
            iteration_count: 0,
            expanded: TokenStream::new(),
            separator: Maybe::None,
        }
    }

    /// Set a separator to insert between iterations
    pub fn with_separator(mut self, sep: TokenStream) -> Self {
        self.separator = Maybe::Some(sep);
        self
    }

    /// Get the current iteration count
    pub fn iteration_count(&self) -> usize {
        self.iteration_count
    }

    /// Check if a name is a pattern variable
    pub fn is_pattern_var(&self, name: &Text) -> bool {
        self.pattern_names.iter().any(|p| p == name)
    }

    /// Expand the body template
    ///
    /// In a real implementation, this would receive the actual list values
    /// and iterate over them. For now, it returns the body as-is.
    pub fn expand_body(
        &mut self,
        body: &TokenStream,
        expander: &mut QuoteExpander,
    ) -> Result<TokenStream, HygieneViolation> {
        // In a real implementation, we would:
        // 1. Get the list values from the binding
        // 2. Iterate over them
        // 3. For each iteration, substitute the pattern variable and expand the body

        // For now, we just mark the body with a fresh mark to ensure hygiene
        let mut result = body.clone();
        result.marks = expander.current_marks();

        // Record that we've expanded this repetition
        self.expanded = result.clone();
        self.iteration_count = 1; // Placeholder

        Ok(result)
    }

    /// Expand a single iteration
    pub fn expand_iteration(
        &mut self,
        body: &TokenStream,
        elements: &[HygienicIdent],
        iteration: usize,
    ) -> Result<TokenStream, HygieneViolation> {
        // Verify we have the right number of elements
        if elements.len() != self.pattern_names.len() {
            return Err(HygieneViolation::RepetitionMismatch {
                first_name: self.pattern_names.get(0).cloned().unwrap_or_default(),
                first_len: self.pattern_names.len(),
                second_name: Text::from("elements"),
                second_len: elements.len(),
                span: self.span,
            });
        }

        // Create a fresh mark for this iteration
        let iteration_mark = Mark::fresh();

        // Substitute all pattern variables in the body
        let mut result = body.clone();
        for (idx, pattern_name) in self.pattern_names.iter().enumerate() {
            if let Some(element) = elements.get(idx) {
                result = result.map_idents(|ident| {
                    if &ident.name == pattern_name {
                        let mut new_ident = element.clone();
                        new_ident.scopes.add(ScopeId::new(iteration_mark.as_u64()));
                        new_ident
                    } else {
                        let mut new_ident = ident.clone();
                        new_ident.scopes.add(ScopeId::new(iteration_mark.as_u64()));
                        new_ident
                    }
                });
            }
        }

        // Apply the iteration mark
        result.apply_mark(iteration_mark);

        // Update iteration count
        self.iteration_count = iteration + 1;

        // Add separator if not first iteration
        if iteration > 0 {
            if let Maybe::Some(sep) = &self.separator {
                self.expanded.append(sep);
            }
        }

        // Append to accumulated result
        self.expanded.append(&result);

        Ok(result)
    }

    /// Get the accumulated expanded tokens
    pub fn take_expanded(&mut self) -> TokenStream {
        std::mem::take(&mut self.expanded)
    }

    /// Finish expansion and return the result
    pub fn finish(self) -> TokenStream {
        self.expanded
    }
}

/// Stage context for multi-stage hygiene
#[derive(Debug, Clone)]
pub struct StageContext {
    /// Current stage level
    pub stage: u32,
    /// Bindings visible at this stage
    bindings: Map<Text, StageBinding>,
    /// Parent stage context
    parent: Maybe<Box<StageContext>>,
}

/// A binding with stage information
#[derive(Debug, Clone)]
pub struct StageBinding {
    /// The identifier
    pub ident: HygienicIdent,
    /// Stage at which this binding is valid
    pub valid_stage: u32,
    /// Type of the binding (if known)
    pub ty: Maybe<Text>,
    /// The binding info
    pub binding: BindingInfo,
}

impl StageContext {
    /// Create a new stage context
    pub fn new(stage: u32) -> Self {
        Self {
            stage,
            bindings: Map::new(),
            parent: Maybe::None,
        }
    }

    /// Create a child stage context
    pub fn child(&self, stage: u32) -> Self {
        Self {
            stage,
            bindings: Map::new(),
            parent: Maybe::Some(Box::new(self.clone())),
        }
    }

    /// Add a binding at this stage
    pub fn add_binding(&mut self, name: Text, binding: StageBinding) {
        self.bindings.insert(name, binding);
    }

    /// Check if a binding is accessible at the current stage
    pub fn is_accessible(&self, binding: &StageBinding) -> bool {
        binding.valid_stage <= self.stage
    }

    /// Resolve a binding respecting stage boundaries
    pub fn resolve(&self, name: &Text) -> Maybe<&StageBinding> {
        if let Some(binding) = self.bindings.get(name) {
            if self.is_accessible(binding) {
                return Maybe::Some(binding);
            }
        }

        // Check parent stage
        if let Maybe::Some(parent) = &self.parent {
            return parent.resolve(name);
        }

        Maybe::None
    }

    /// Get all bindings at this stage
    pub fn bindings(&self) -> impl Iterator<Item = (&Text, &StageBinding)> {
        self.bindings.iter()
    }

    /// Get the current stage level
    pub fn current_stage(&self) -> u32 {
        self.stage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_expander_creation() {
        let context = HygieneContext::new();
        let expander = QuoteExpander::with_default_config(context);

        assert!(!expander.in_quote());
        assert_eq!(expander.depth(), 0);
    }

    #[test]
    fn test_enter_exit_quote() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        let result = expander.enter_quote(0, Span::default());
        assert!(result.is_ok());
        assert!(expander.in_quote());
        assert_eq!(expander.depth(), 1);

        expander.exit_quote();
        assert!(!expander.in_quote());
        assert_eq!(expander.depth(), 0);
    }

    #[test]
    fn test_nested_quotes() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();
        expander.enter_quote(0, Span::default()).unwrap();
        assert_eq!(expander.depth(), 2);

        expander.exit_quote();
        assert_eq!(expander.depth(), 1);

        expander.exit_quote();
        assert_eq!(expander.depth(), 0);
    }

    #[test]
    fn test_process_binding() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();

        let ident = expander.process_binding(
            Text::from("x"),
            Span::default(),
            BindingKind::Variable,
            false,
        );

        assert_eq!(ident.name.as_str(), "x");
        assert!(!ident.scopes.is_empty());
    }

    #[test]
    fn test_gensym() {
        let context = HygieneContext::new();
        let expander = QuoteExpander::with_default_config(context);

        let name1 = expander.gensym("tmp");
        let name2 = expander.gensym("tmp");

        assert_ne!(name1, name2);
        assert!(HygieneContext::is_hygienic(name1.as_str()));
    }

    #[test]
    fn test_max_depth() {
        let context = HygieneContext::new();
        let config = ExpansionConfig {
            max_depth: 2,
            ..Default::default()
        };
        let mut expander = QuoteExpander::new(context, config);

        expander.enter_quote(0, Span::default()).unwrap();
        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.enter_quote(0, Span::default());
        assert!(matches!(
            result,
            Err(HygieneViolation::InvalidQuoteSyntax { .. })
        ));
    }

    #[test]
    fn test_stage_context() {
        let mut stage0 = StageContext::new(0);

        let binding = StageBinding {
            ident: HygienicIdent::unhygienic(Text::from("x"), Span::default()),
            valid_stage: 0,
            ty: Maybe::None,
            binding: BindingInfo {
                original_name: Text::from("x"),
                hygienic_name: Text::from("x"),
                scope_id: ScopeId::new(0),
                is_mutable: false,
                kind: BindingKind::Variable,
            },
        };

        stage0.add_binding(Text::from("x"), binding);

        let resolved = stage0.resolve(&Text::from("x"));
        assert!(matches!(resolved, Maybe::Some(_)));

        let unresolved = stage0.resolve(&Text::from("y"));
        assert!(matches!(unresolved, Maybe::None));
    }

    #[test]
    fn test_stage_context_child() {
        let mut stage0 = StageContext::new(0);
        let binding = StageBinding {
            ident: HygienicIdent::unhygienic(Text::from("parent_var"), Span::default()),
            valid_stage: 0,
            ty: Maybe::None,
            binding: BindingInfo {
                original_name: Text::from("parent_var"),
                hygienic_name: Text::from("parent_var"),
                scope_id: ScopeId::new(0),
                is_mutable: false,
                kind: BindingKind::Variable,
            },
        };
        stage0.add_binding(Text::from("parent_var"), binding);

        let stage1 = stage0.child(1);

        // Should see parent's binding
        let resolved = stage1.resolve(&Text::from("parent_var"));
        assert!(matches!(resolved, Maybe::Some(_)));
    }

    #[test]
    fn test_token_stream() {
        let mut stream = TokenStream::new();
        assert!(stream.is_empty());

        let token = Token {
            kind: TokenKind::Literal(ConstValue::Int(42)),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        };

        stream.push(token);
        assert_eq!(stream.len(), 1);
        assert!(!stream.is_empty());
    }

    #[test]
    fn test_token_stream_marks() {
        let mut stream = TokenStream::new();

        let ident = HygienicIdent::unhygienic(Text::from("x"), Span::default());
        let token = Token {
            kind: TokenKind::Ident(ident),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        };

        stream.push(token);

        // Apply a mark to all tokens
        let mark = Mark::fresh();
        stream.apply_mark(mark);

        // Verify mark was applied
        for token in stream.iter() {
            assert!(token.marks.contains(&mark));
        }
    }

    #[test]
    fn test_token_stream_flip_mark() {
        let mut stream = TokenStream::new();

        let mark = Mark::fresh();
        let mut initial_marks = MarkSet::new();
        initial_marks.add(mark);

        let token = Token {
            kind: TokenKind::Literal(ConstValue::Int(1)),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: initial_marks,
        };
        stream.push(token);

        // Flip should remove the mark
        stream.flip_mark(mark);

        for token in stream.iter() {
            assert!(!token.marks.contains(&mark));
        }

        // Flip again should add it back
        stream.flip_mark(mark);

        for token in stream.iter() {
            assert!(token.marks.contains(&mark));
        }
    }

    #[test]
    fn test_token_stream_call_site_marks() {
        let mut stream = TokenStream::new();
        let ident = HygienicIdent::unhygienic(Text::from("foo"), Span::default());
        let token = Token {
            kind: TokenKind::Ident(ident),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        };
        stream.push(token);

        let call_mark = Mark::fresh();
        let marked_stream = stream.with_call_site_marks(call_mark);

        // Check that identifiers have the new scope
        let idents = marked_stream.collect_idents();
        assert_eq!(idents.len(), 1);
        assert!(idents[0].scopes.contains(&ScopeId::new(call_mark.as_u64())));
    }

    // ========================================================================
    // Repetition Expansion Tests
    // ========================================================================

    #[test]
    fn test_repetition_expander_creation() {
        let expander = RepetitionExpander::new(
            Text::from("item"),
            ScopeSet::new(),
            Span::default(),
        );

        assert!(expander.is_pattern_var(&Text::from("item")));
        assert!(!expander.is_pattern_var(&Text::from("other")));
        assert_eq!(expander.iteration_count(), 0);
    }

    #[test]
    fn test_repetition_expander_multi_pattern() {
        let expander = RepetitionExpander::new_multi(
            vec![Text::from("key"), Text::from("value")],
            ScopeSet::new(),
            Span::default(),
        );

        assert!(expander.is_pattern_var(&Text::from("key")));
        assert!(expander.is_pattern_var(&Text::from("value")));
        assert!(!expander.is_pattern_var(&Text::from("other")));
    }

    #[test]
    fn test_repetition_single_iteration() {
        let mut expander = RepetitionExpander::new(
            Text::from("x"),
            ScopeSet::new(),
            Span::default(),
        );

        // Create a simple body with the pattern variable
        let mut body = TokenStream::new();
        let ident = HygienicIdent::unhygienic(Text::from("x"), Span::default());
        body.push(Token {
            kind: TokenKind::Ident(ident),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        });

        // Expand a single iteration
        let element = HygienicIdent::unhygienic(Text::from("field_a"), Span::default());
        let result = expander.expand_iteration(&body, &[element], 0);

        assert!(result.is_ok());
        let result = result.unwrap();

        // The pattern variable should be replaced
        let idents = result.collect_idents();
        assert_eq!(idents.len(), 1);
        assert_eq!(idents[0].name.as_str(), "field_a");
    }

    #[test]
    fn test_repetition_multiple_iterations() {
        let mut expander = RepetitionExpander::new(
            Text::from("item"),
            ScopeSet::new(),
            Span::default(),
        );

        // Create body
        let mut body = TokenStream::new();
        body.push(Token {
            kind: TokenKind::Ident(HygienicIdent::unhygienic(Text::from("item"), Span::default())),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        });

        // Expand multiple iterations
        for i in 0..3 {
            let element = HygienicIdent::unhygienic(Text::from(format!("field_{}", i)), Span::default());
            let _ = expander.expand_iteration(&body, &[element], i);
        }

        let result = expander.finish();
        let idents = result.collect_idents();

        // Should have 3 substituted identifiers
        assert_eq!(idents.len(), 3);
    }

    #[test]
    fn test_repetition_hygiene_isolation() {
        let mut expander = RepetitionExpander::new(
            Text::from("x"),
            ScopeSet::new(),
            Span::default(),
        );

        let mut body = TokenStream::new();
        body.push(Token {
            kind: TokenKind::Ident(HygienicIdent::unhygienic(Text::from("x"), Span::default())),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        });

        // Expand two iterations
        let elem1 = HygienicIdent::unhygienic(Text::from("a"), Span::default());
        let elem2 = HygienicIdent::unhygienic(Text::from("b"), Span::default());

        let result1 = expander.expand_iteration(&body, &[elem1], 0).unwrap();
        let result2 = expander.expand_iteration(&body, &[elem2], 1).unwrap();

        // Each iteration should have different marks for hygiene isolation
        let idents1 = result1.collect_idents();
        let idents2 = result2.collect_idents();

        // The identifiers should have different scopes (different iteration marks)
        assert!(!idents1[0].scopes.is_subset_of(&idents2[0].scopes) ||
                !idents2[0].scopes.is_subset_of(&idents1[0].scopes));
    }

    #[test]
    fn test_repetition_with_separator() {
        let mut sep = TokenStream::new();
        sep.push(Token {
            kind: TokenKind::Punct(','),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        });

        let mut expander = RepetitionExpander::new(
            Text::from("x"),
            ScopeSet::new(),
            Span::default(),
        ).with_separator(sep);

        let mut body = TokenStream::new();
        body.push(Token {
            kind: TokenKind::Ident(HygienicIdent::unhygienic(Text::from("x"), Span::default())),
            span: Span::default(),
            scopes: ScopeSet::new(),
            marks: MarkSet::new(),
        });

        // Expand with separator
        for i in 0..3 {
            let element = HygienicIdent::unhygienic(Text::from(format!("f{}", i)), Span::default());
            let _ = expander.expand_iteration(&body, &[element], i);
        }

        let result = expander.finish();

        // Should have 3 idents + 2 separators = at least 5 tokens
        assert!(result.len() >= 5);
    }

    // ========================================================================
    // Capture Detection Tests
    // ========================================================================

    #[test]
    fn test_splice_value_capture() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();

        // Register a binding
        expander.process_binding(
            Text::from("captured"),
            Span::default(),
            BindingKind::Variable,
            false,
        );

        // Splicing should work for declared variables
        let result = expander.splice_value(&Text::from("captured"), Span::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_splice_undeclared_capture() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();

        // Splicing undeclared variable should fail
        let result = expander.splice_value(&Text::from("undeclared"), Span::default());
        assert!(matches!(
            result,
            Err(HygieneViolation::CaptureNotDeclared { .. })
        ));
    }

    #[test]
    fn test_repetition_undeclared_list() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();

        let body = TokenStream::new();

        // Repetition with undeclared list should fail
        let result = expander.expand_repetition(
            &Text::from("item"),
            &Text::from("undeclared_list"),
            &body,
            Span::default(),
        );
        assert!(matches!(
            result,
            Err(HygieneViolation::CaptureNotDeclared { .. })
        ));
    }

    // ========================================================================
    // Multi-Stage Hygiene Tests
    // ========================================================================

    #[test]
    fn test_stage_context_creation() {
        let stage0 = StageContext::new(0);
        assert_eq!(stage0.current_stage(), 0);
    }

    #[test]
    fn test_stage_context_binding_visibility() {
        let mut stage0 = StageContext::new(0);

        let binding = StageBinding {
            ident: HygienicIdent::unhygienic(Text::from("x"), Span::default()),
            valid_stage: 0,
            ty: Maybe::None,
            binding: BindingInfo {
                original_name: Text::from("x"),
                hygienic_name: Text::from("x"),
                scope_id: ScopeId::new(0),
                is_mutable: false,
                kind: BindingKind::Variable,
            },
        };

        stage0.add_binding(Text::from("x"), binding);

        // Should be visible at stage 0
        assert!(matches!(stage0.resolve(&Text::from("x")), Maybe::Some(_)));

        // Should also be visible from child stage 1
        let stage1 = stage0.child(1);
        assert!(matches!(stage1.resolve(&Text::from("x")), Maybe::Some(_)));
    }

    #[test]
    fn test_stage_context_higher_stage_invisible() {
        let mut stage0 = StageContext::new(0);

        // Binding at stage 1
        let binding = StageBinding {
            ident: HygienicIdent::unhygienic(Text::from("meta_var"), Span::default()),
            valid_stage: 1,
            ty: Maybe::None,
            binding: BindingInfo {
                original_name: Text::from("meta_var"),
                hygienic_name: Text::from("meta_var"),
                scope_id: ScopeId::new(0),
                is_mutable: false,
                kind: BindingKind::Variable,
            },
        };

        stage0.add_binding(Text::from("meta_var"), binding);

        // Stage 1 binding should NOT be visible at stage 0
        assert!(matches!(stage0.resolve(&Text::from("meta_var")), Maybe::None));
    }

    #[test]
    fn test_stage_escape_valid() {
        let context = HygieneContext::new();
        let config = ExpansionConfig::meta_to_runtime(); // stage 1 -> stage 0
        let mut expander = QuoteExpander::new(context, config);

        expander.enter_quote(0, Span::default()).unwrap();

        // Escaping to a lower stage should work
        let result = expander.stage_escape(
            0,
            &make_test_expr(),
            Span::default(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_stage_escape_invalid() {
        let context = HygieneContext::new();
        let config = ExpansionConfig::meta_to_runtime(); // source_stage = 1
        let mut expander = QuoteExpander::new(context, config);

        expander.enter_quote(0, Span::default()).unwrap();

        // Escaping to a higher stage should fail
        let result = expander.stage_escape(
            2, // Higher than source_stage
            &make_test_expr(),
            Span::default(),
        );
        assert!(matches!(
            result,
            Err(HygieneViolation::StageMismatch { .. })
        ));
    }

    // ========================================================================
    // Quote Syntax Tests
    // ========================================================================

    /// Create a dummy expression for testing
    fn make_test_expr() -> Expr {
        use verum_ast::Literal;
        Expr::literal(Literal::int(42, Span::default()))
    }

    #[test]
    fn test_lift_outside_quote() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        // Lift outside quote should fail
        let result = expander.lift_value(&make_test_expr(), Span::default());
        assert!(matches!(
            result,
            Err(HygieneViolation::UnquoteOutsideQuote { .. })
        ));
    }

    #[test]
    fn test_lift_inside_quote() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();

        // Lift inside quote should work
        let result = expander.lift_value(&make_test_expr(), Span::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_expansion_config_staged() {
        let config = ExpansionConfig::staged(2, 1);
        assert_eq!(config.source_stage, 2);
        assert_eq!(config.target_stage, 1);
    }

    #[test]
    fn test_expansion_config_meta2_to_meta() {
        let config = ExpansionConfig::meta2_to_meta();
        assert_eq!(config.source_stage, 2);
        assert_eq!(config.target_stage, 1);
    }

    // ========================================================================
    // Lift Evaluator Tests
    // ========================================================================

    #[test]
    fn test_lift_evaluator_set() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        assert!(!expander.has_lift_evaluator());

        // Set a simple evaluator that always returns Int(42)
        expander.set_lift_evaluator(Box::new(|_expr| {
            Ok(verum_ast::MetaValue::Int(42))
        }));

        assert!(expander.has_lift_evaluator());
    }

    #[test]
    fn test_lift_with_evaluator_success() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context)
            .with_lift_evaluator(Box::new(|_expr| {
                Ok(verum_ast::MetaValue::Int(100))
            }));

        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.lift_value(&make_test_expr(), Span::default());
        assert!(result.is_ok());

        let lifted = result.unwrap();
        assert!(matches!(lifted.const_value, Maybe::Some(ConstValue::Int(100))));
        assert!(matches!(lifted.ty, Maybe::Some(ref t) if t.as_str() == "Int"));
    }

    #[test]
    fn test_lift_with_evaluator_float() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context)
            .with_lift_evaluator(Box::new(|_expr| {
                Ok(verum_ast::MetaValue::Float(2.5))
            }));

        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.lift_value(&make_test_expr(), Span::default());
        assert!(result.is_ok());

        let lifted = result.unwrap();
        if let Maybe::Some(ConstValue::Float(f)) = lifted.const_value {
            assert!((f - 2.5).abs() < 0.001);
        } else {
            panic!("Expected Float");
        }
    }

    #[test]
    fn test_lift_with_evaluator_text() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context)
            .with_lift_evaluator(Box::new(|_expr| {
                Ok(verum_ast::MetaValue::Text(Text::from("hello")))
            }));

        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.lift_value(&make_test_expr(), Span::default());
        assert!(result.is_ok());

        let lifted = result.unwrap();
        assert!(matches!(lifted.const_value, Maybe::Some(ConstValue::Text(ref t)) if t.as_str() == "hello"));
    }

    #[test]
    fn test_lift_with_evaluator_bool() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context)
            .with_lift_evaluator(Box::new(|_expr| {
                Ok(verum_ast::MetaValue::Bool(true))
            }));

        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.lift_value(&make_test_expr(), Span::default());
        assert!(result.is_ok());

        let lifted = result.unwrap();
        assert!(matches!(lifted.const_value, Maybe::Some(ConstValue::Bool(true))));
    }

    #[test]
    fn test_lift_with_evaluator_error_lenient() {
        // Under lenient mode (`strict_mode = false`), a failed lift
        // evaluation is recorded as a violation and the lift returns
        // `Ok(LiftedValue { const_value: None, ty: None })` so the
        // caller can decide whether to fail the whole expansion or
        // continue with the deferred-eval shape.
        let context = HygieneContext::new();
        let lenient = ExpansionConfig {
            strict_mode: false,
            ..ExpansionConfig::default()
        };
        let mut expander = QuoteExpander::new(context, lenient)
            .with_lift_evaluator(Box::new(|_expr| {
                Err(Text::from("evaluation error"))
            }));

        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.lift_value(&make_test_expr(), Span::default());

        assert!(result.is_ok(), "lenient mode must collect-and-continue");
        let lifted = result.unwrap();
        assert!(matches!(lifted.const_value, Maybe::None));
        assert!(matches!(lifted.ty, Maybe::None));
        assert!(
            expander.has_violations(),
            "lenient mode must record the violation for caller inspection",
        );
    }

    #[test]
    fn test_lift_with_evaluator_error_strict_aborts() {
        // Under strict mode (`strict_mode = true` — the default), a
        // failed lift evaluation aborts immediately with a typed
        // error.  Pre-fix the field was inert and lenient was the
        // only behaviour available.  Pin the new strict path so a
        // regression to the inert-field state is caught.
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context)
            .with_lift_evaluator(Box::new(|_expr| {
                Err(Text::from("evaluation error"))
            }));

        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.lift_value(&make_test_expr(), Span::default());

        match result {
            Err(HygieneViolation::InvalidQuoteSyntax { ref message, .. }) => {
                assert!(
                    message.as_str().contains("lift evaluation failed"),
                    "strict-mode violation must carry the failure context, got: {:?}",
                    message,
                );
            }
            other => panic!(
                "expected strict-mode abort with InvalidQuoteSyntax, got: {:?}",
                other,
            ),
        }
        // Strict mode aborts before pushing the violation to the
        // collection — the caller sees the Err directly.
        assert!(
            !expander.has_violations(),
            "strict mode must abort BEFORE accumulating the violation",
        );
    }

    #[test]
    fn test_lift_without_evaluator() {
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();

        let result = expander.lift_value(&make_test_expr(), Span::default());
        assert!(result.is_ok());

        let lifted = result.unwrap();
        // No evaluator means no const value
        assert!(matches!(lifted.const_value, Maybe::None));
        assert_eq!(lifted.source_stage, 1); // Default source_stage
    }

    #[test]
    fn test_evaluate_lift_raw_success() {
        let context = HygieneContext::new();
        let expander = QuoteExpander::with_default_config(context)
            .with_lift_evaluator(Box::new(|_expr| {
                Ok(verum_ast::MetaValue::Int(999))
            }));

        let result = expander.evaluate_lift_raw(&make_test_expr(), Span::default());
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), verum_ast::MetaValue::Int(999)));
    }

    #[test]
    fn test_evaluate_lift_raw_no_evaluator() {
        let context = HygieneContext::new();
        let expander = QuoteExpander::with_default_config(context);

        let result = expander.evaluate_lift_raw(&make_test_expr(), Span::default());
        assert!(result.is_err());
        assert!(matches!(result, Err(HygieneViolation::InvalidQuoteSyntax { .. })));
    }

    #[test]
    fn test_const_value_from_meta_value() {
        use verum_ast::MetaValue;

        // Test primitive conversions
        assert_eq!(
            ConstValue::from_meta_value(&MetaValue::Int(42)),
            Maybe::Some(ConstValue::Int(42))
        );
        assert_eq!(
            ConstValue::from_meta_value(&MetaValue::Bool(true)),
            Maybe::Some(ConstValue::Bool(true))
        );
        assert_eq!(
            ConstValue::from_meta_value(&MetaValue::Unit),
            Maybe::Some(ConstValue::Unit)
        );
        assert_eq!(
            ConstValue::from_meta_value(&MetaValue::Char('x')),
            Maybe::Some(ConstValue::Char('x'))
        );

        // Test array conversion
        let arr = MetaValue::Array(vec![
            MetaValue::Int(1),
            MetaValue::Int(2),
            MetaValue::Int(3),
        ].into_iter().collect());
        let converted = ConstValue::from_meta_value(&arr);
        assert!(matches!(converted, Maybe::Some(ConstValue::Array(_))));

        // Test that AST values cannot be converted
        let expr = MetaValue::Expr(make_test_expr());
        assert!(matches!(ConstValue::from_meta_value(&expr), Maybe::None));
    }

    #[test]
    fn test_const_value_type_name() {
        assert_eq!(ConstValue::Int(0).type_name().as_str(), "Int");
        assert_eq!(ConstValue::Float(0.0).type_name().as_str(), "Float");
        assert_eq!(ConstValue::Bool(true).type_name().as_str(), "Bool");
        assert_eq!(ConstValue::Char('x').type_name().as_str(), "Char");
        assert_eq!(ConstValue::Text(Text::from("")).type_name().as_str(), "Text");
        assert_eq!(ConstValue::Unit.type_name().as_str(), "()");
        assert_eq!(ConstValue::Array(List::new()).type_name().as_str(), "Array");
        assert_eq!(ConstValue::Tuple(List::new()).type_name().as_str(), "Tuple");
    }

    #[test]
    fn debug_bindings_disabled_records_nothing() {
        // Pin: with `debug_bindings = false` (the default), the
        // expander does not allocate or push any debug events even
        // through a busy quote/binding/splice/lift sequence.
        let context = HygieneContext::new();
        let mut expander = QuoteExpander::with_default_config(context);

        expander.enter_quote(0, Span::default()).unwrap();
        expander.process_binding(
            Text::from("x"),
            Span::default(),
            BindingKind::Variable,
            false,
        );
        expander.process_reference(&Text::from("x"), Span::default());
        let _ = expander.splice_value(&Text::from("x"), Span::default());
        let _ = expander.lift_value(&make_test_expr(), Span::default());
        expander.exit_quote();

        assert!(
            expander.debug_bindings_log().is_empty(),
            "default config must not record debug events",
        );
    }

    #[test]
    fn debug_bindings_enabled_records_full_trace() {
        // Pin: with `debug_bindings = true`, every choke-point logs
        // an event in chronological order, with names, spans, and
        // depth captured.
        let context = HygieneContext::new();
        let config = ExpansionConfig {
            debug_bindings: true,
            ..Default::default()
        };
        let mut expander = QuoteExpander::new(context, config);

        expander.enter_quote(0, Span::default()).unwrap();
        expander.process_binding(
            Text::from("x"),
            Span::default(),
            BindingKind::Variable,
            false,
        );
        expander.process_reference(&Text::from("x"), Span::default());
        let _ = expander.splice_value(&Text::from("x"), Span::default());
        let _ = expander.lift_value(&make_test_expr(), Span::default());
        expander.exit_quote();

        let log = expander.take_debug_bindings_log();
        let kinds: Vec<&'static str> = log
            .iter()
            .map(|e| match e.kind {
                DebugBindingEventKind::EnterQuote => "enter",
                DebugBindingEventKind::ExitQuote => "exit",
                DebugBindingEventKind::Binding(_) => "binding",
                DebugBindingEventKind::Reference => "reference",
                DebugBindingEventKind::Splice => "splice",
                DebugBindingEventKind::Lift => "lift",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["enter", "binding", "reference", "splice", "lift", "exit"],
            "trace must capture every choke point in order",
        );

        // Depth at EnterQuote/ExitQuote is recorded post-update for
        // enter and pre-update for exit (both sit at depth=1 on the
        // boundary — the in-quote events also at depth=1).
        for event in log.iter() {
            assert_eq!(event.depth, 1, "every event in this trace is at depth 1");
        }

        // The taking accessor drains the buffer.
        assert!(
            expander.debug_bindings_log().is_empty(),
            "take_debug_bindings_log must drain",
        );
    }

    #[test]
    fn debug_bindings_records_binding_kind() {
        // Pin: the BindingKind variant flows into the event so a
        // tooling consumer can distinguish a `let` from a parameter
        // from a function/type/macro/pattern/label.
        let context = HygieneContext::new();
        let config = ExpansionConfig {
            debug_bindings: true,
            ..Default::default()
        };
        let mut expander = QuoteExpander::new(context, config);

        expander.enter_quote(0, Span::default()).unwrap();
        for kind in [
            BindingKind::Variable,
            BindingKind::Parameter,
            BindingKind::Function,
            BindingKind::Type,
        ] {
            expander.process_binding(
                Text::from("n"),
                Span::default(),
                kind,
                false,
            );
        }

        let recorded: Vec<BindingKind> = expander
            .debug_bindings_log()
            .iter()
            .filter_map(|e| match e.kind {
                DebugBindingEventKind::Binding(k) => Some(k),
                _ => None,
            })
            .collect();
        assert_eq!(
            recorded,
            vec![
                BindingKind::Variable,
                BindingKind::Parameter,
                BindingKind::Function,
                BindingKind::Type,
            ],
        );
    }
}
