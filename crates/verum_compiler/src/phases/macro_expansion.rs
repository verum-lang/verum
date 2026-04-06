//! Phase 3: Macro Expansion & Literal Processing
//!
//! Executes meta code and expands macros in a sandboxed environment.
//!
//! ## Multi-Pass Architecture
//!
//! This phase implements Pass 2 of the three-pass compilation:
//! - Pass 1: Parse + Register (done in MetaRegistryPhase)
//! - **Pass 2: Expand Macros** (this phase)
//! - Pass 3: Semantic Analysis (done in type checking)
//!
//! ## Features
//!
//! - @derive macro expansion using DeriveRegistry
//! - Tagged literal processing (d#"...", rx#"...", etc.)
//! - Interpolation handler invocation (sql"...", html"...", etc.)
//! - Meta function execution in sandboxed environment
//! - Meta Linter integration for safety validation
//! - Compile-time code generation via quote!
//! - Cross-file macro resolution via MetaRegistry
//!
//! ## Safety
//!
//! All meta code is validated by the MetaLinter before execution:
//! - @safe functions must pass all safety checks
//! - @unsafe functions emit warnings
//! - I/O operations are forbidden (except via `using BuildAssets` context)
//!
//! Phase 3: Macro expansion and literal processing. Executes @derive macros
//! in sandboxed const_eval, parses tagged literals, processes interpolated
//! strings with safe injection prevention, validates numeric suffixes.
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use anyhow::Result;
use std::time::Instant;
use verum_ast::attr::Attribute;
use verum_ast::decl::{FunctionBody, FunctionDecl, ItemKind, TypeDecl};
use verum_ast::expr::{ArrayExpr, Block, Expr, ExprKind};
use verum_ast::literal::{Literal, StringLit};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::{Item, LiteralKind, Module, Span};
use verum_common::{Heap, List, Maybe, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};
use crate::derives::{DeriveError, DeriveRegistry};
use crate::literal_registry::{LiteralRegistry, ParsedLiteral};
use crate::meta::{ConstValue, MetaContext, MetaError, MetaRegistry};
use crate::meta::linter::MetaLinter;

/// Phase 3: Macro Expansion
///
/// Processes all macro invocations and compile-time code generation.
/// Integrates MetaLinter for safety validation before execution.
///
/// # Architecture
///
/// ```text
/// MacroExpansionPhase
/// ├── DeriveRegistry      - @derive(Debug, Clone, etc.)
/// ├── MetaRegistry        - Cross-file meta function lookup
/// ├── MetaLinter          - Safety validation (@safe/@unsafe)
/// ├── LiteralRegistry     - Tagged literal handlers (d#, rx#, etc.)
/// ├── MetaSandbox         - Execution isolation
/// └── MetaContext         - Execution environment
/// ```
pub struct MacroExpansionPhase {
    /// Registry of derive macros (Debug, Clone, Serialize, etc.)
    derive_registry: DeriveRegistry,
    /// Registry of meta functions and macros (cross-file)
    meta_registry: MetaRegistry,
    /// Meta Linter for safety validation
    meta_linter: MetaLinter,
    /// Registry for tagged literal handlers
    literal_registry: LiteralRegistry,
    /// Meta execution context (reserved for user meta function execution)
    #[allow(dead_code)]
    meta_context: MetaContext,
    /// Current module path (for scoped lookup)
    current_module: Text,
    /// Current file ID being processed (for literal parsing source context)
    current_file_id: verum_ast::FileId,
    /// Expansion statistics
    stats: ExpansionStats,
    /// Accumulated lint warnings
    lint_warnings: List<Diagnostic>,
}

/// Statistics for macro expansion
#[derive(Debug, Clone, Default)]
pub struct ExpansionStats {
    /// Number of @derive attributes expanded
    pub derives_expanded: usize,
    /// Number of tagged literals processed
    pub tagged_literals_processed: usize,
    /// Number of interpolations processed
    pub interpolations_processed: usize,
    /// Number of meta functions executed
    pub meta_functions_executed: usize,
    /// Number of attribute macros expanded
    pub attribute_macros_expanded: usize,
    /// Number of items generated
    pub items_generated: usize,
    /// Number of meta functions linted
    pub meta_functions_linted: usize,
    /// Number of lint warnings emitted
    pub lint_warnings: usize,
    /// Errors encountered
    pub errors: usize,
}

impl MacroExpansionPhase {
    /// Create a new macro expansion phase
    pub fn new() -> Self {
        let literal_registry = LiteralRegistry::new();
        literal_registry.register_builtin_handlers();

        Self {
            derive_registry: DeriveRegistry::new(),
            meta_registry: MetaRegistry::new(),
            meta_linter: MetaLinter::new(),
            literal_registry,
            meta_context: MetaContext::new(),
            current_module: Text::from(""),
            current_file_id: verum_ast::FileId::new(0),
            stats: ExpansionStats::default(),
            lint_warnings: List::new(),
        }
    }

    /// Create with existing registries (for incremental compilation)
    pub fn with_registries(
        derive_registry: DeriveRegistry,
        meta_registry: MetaRegistry,
        literal_registry: LiteralRegistry,
    ) -> Self {
        Self {
            derive_registry,
            meta_registry,
            meta_linter: MetaLinter::new(),
            literal_registry,
            meta_context: MetaContext::new(),
            current_module: Text::from(""),
            current_file_id: verum_ast::FileId::new(0),
            stats: ExpansionStats::default(),
            lint_warnings: List::new(),
        }
    }

    /// Execute a user-defined meta function
    ///
    /// This looks up the meta function in the registry and executes it
    /// with the provided arguments.
    #[allow(dead_code)] // Reserved for user-defined meta function execution
    fn execute_meta_function(
        &mut self,
        module: &Text,
        func_name: &Text,
        args: List<ConstValue>,
    ) -> Result<ConstValue, MetaError> {
        // Look up the meta function
        match self.meta_registry.get_user_meta_fn(module, func_name) {
            Maybe::Some(meta_func) => {
                // Execute the function
                self.meta_context
                    .execute_user_meta_fn(&meta_func, args.into_iter().collect())
            }
            Maybe::None => Err(MetaError::MetaFunctionNotFound(func_name.as_str().to_string().into())),
        }
    }

    /// Expand macros in all modules
    fn expand_modules(&mut self, modules: &[Module]) -> Result<List<Module>, List<Diagnostic>> {
        let mut expanded_modules = List::new();
        let mut all_diagnostics = List::new();

        for module in modules {
            match self.expand_module(module) {
                Ok(expanded) => expanded_modules.push(expanded),
                Err(diags) => {
                    self.stats.errors += diags.len();
                    for d in diags.iter() {
                        all_diagnostics.push(d.clone());
                    }
                }
            }
        }

        if self.stats.errors > 0 {
            Err(all_diagnostics)
        } else {
            Ok(expanded_modules)
        }
    }

    /// Expand macros in a single module
    fn expand_module(&mut self, module: &Module) -> Result<Module, List<Diagnostic>> {
        tracing::debug!("Expanding macros in module (file_id: {:?})", module.file_id);

        // Set current file ID for literal parsing context
        self.current_file_id = module.file_id;

        let mut expanded_items = List::new();
        let mut diagnostics = List::new();

        for item in &module.items {
            match self.expand_item(item) {
                Ok(items) => {
                    self.stats.items_generated += items.len().saturating_sub(1);
                    for i in items.iter() {
                        expanded_items.push(i.clone());
                    }
                }
                Err(diag) => {
                    diagnostics.push(diag);
                    // Keep original item on error for better error recovery
                    expanded_items.push(item.clone());
                }
            }
        }

        if !diagnostics.is_empty() {
            Err(diagnostics)
        } else {
            Ok(Module {
                items: expanded_items.into_iter().collect(),
                attributes: module.attributes.clone(),
                file_id: module.file_id,
                span: module.span,
            })
        }
    }

    /// Expand macros in a single item
    ///
    /// Processes:
    /// 1. @derive attributes on types
    /// 2. Meta functions (with linting)
    /// 3. Tagged literals in function bodies
    /// 4. Interpolation expressions
    fn expand_item(&mut self, item: &Item) -> Result<List<Item>, Diagnostic> {
        match &item.kind {
            // Type declarations may have @derive attributes
            ItemKind::Type(type_decl) => self.expand_type_derives(item, type_decl),

            // Meta declarations - lint and register
            ItemKind::Meta(meta_decl) => {
                tracing::debug!("Processing meta declaration: {}", meta_decl.name);
                self.stats.meta_functions_executed += 1;
                Ok(List::from(vec![item.clone()]))
            }

            // Functions - lint meta functions, process tagged literals and interpolations
            ItemKind::Function(func) => self.process_function(item, func),

            // Other items - check for tagged literals in any expressions
            _ => Ok(List::from(vec![item.clone()])),
        }
    }

    /// Process a function declaration
    ///
    /// 1. If meta function, lint it with MetaLinter
    /// 2. Process tagged literals in body
    /// 3. Process interpolations in body
    fn process_function(
        &mut self,
        item: &Item,
        func: &FunctionDecl,
    ) -> Result<List<Item>, Diagnostic> {
        // Lint meta functions before any processing
        if func.is_meta {
            self.lint_meta_function(func)?;
        }

        // Process function body if present
        let modified_item = if let Some(ref body) = func.body {
            let mut modified_func = func.clone();

            // Process tagged literals and interpolations in the body
            let modified_body = self.process_function_body(body)?;
            modified_func.body = Some(modified_body);

            Item {
                kind: ItemKind::Function(modified_func),
                attributes: item.attributes.clone(),
                span: item.span,
            }
        } else {
            item.clone()
        };

        Ok(List::from(vec![modified_item]))
    }

    /// Lint a meta function before execution
    ///
    /// Uses MetaLinter to detect unsafe patterns like:
    /// - String concatenation (injection risk)
    /// - I/O operations (forbidden in meta context)
    /// - Unbounded loops/recursion
    fn lint_meta_function(&mut self, func: &FunctionDecl) -> Result<(), Diagnostic> {
        self.stats.meta_functions_linted += 1;

        let lint_result = self.meta_linter.lint_function(func);

        // Process lint results
        if !lint_result.is_safe {
            // Check if function is marked @unsafe (warnings only)
            let is_unsafe = func.attributes.iter().any(|a| a.name.as_str() == "unsafe");

            if is_unsafe {
                // Just emit warnings for @unsafe functions
                let warnings = self.meta_linter.to_diagnostics(&lint_result, func);
                for warning in warnings.iter() {
                    self.lint_warnings.push(warning.clone());
                }
                self.stats.lint_warnings += warnings.len();
            } else {
                // Check if @safe annotation is violated
                let is_safe = func.attributes.iter().any(|a| a.name.as_str() == "safe");
                if is_safe {
                    // @safe annotation violated - error
                    return Err(DiagnosticBuilder::new(Severity::Error)
                        .message(Text::from(format!(
                            "Meta function '{}' marked @safe but contains unsafe patterns",
                            func.name.name
                        )))
                        .help(Text::from("Fix the unsafe patterns or change to @unsafe"))
                        .build());
                } else {
                    // No annotation - emit warnings and continue
                    let warnings = self.meta_linter.to_diagnostics(&lint_result, func);
                    for warning in warnings.iter() {
                        self.lint_warnings.push(warning.clone());
                    }
                    self.stats.lint_warnings += warnings.len();

                    tracing::warn!(
                        "Meta function '{}' has {} unsafe patterns",
                        func.name.name,
                        lint_result.unsafe_patterns.len()
                    );
                }
            }
        }

        Ok(())
    }

    /// Process function body for tagged literals and interpolations
    fn process_function_body(&mut self, body: &FunctionBody) -> Result<FunctionBody, Diagnostic> {
        match body {
            FunctionBody::Block(block) => {
                let processed_block = self.process_block(block)?;
                Ok(FunctionBody::Block(processed_block))
            }
            FunctionBody::Expr(expr) => {
                let processed_expr = self.process_expr(expr)?;
                Ok(FunctionBody::Expr(processed_expr))
            }
        }
    }

    /// Process a block for tagged literals and interpolations
    fn process_block(&mut self, block: &Block) -> Result<Block, Diagnostic> {
        let mut processed_stmts = List::new();

        for stmt in block.stmts.iter() {
            let processed = self.process_stmt(stmt)?;
            processed_stmts.push(processed);
        }

        let processed_expr = if let Some(ref expr) = block.expr {
            Some(Box::new(self.process_expr(expr)?))
        } else {
            None
        };

        Ok(Block {
            stmts: processed_stmts.into_iter().collect(),
            expr: processed_expr,
            span: block.span,
        })
    }

    /// Process a statement for tagged literals
    fn process_stmt(&mut self, stmt: &Stmt) -> Result<Stmt, Diagnostic> {
        match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                let processed_value = if let Some(val) = value {
                    Some(self.process_expr(val)?)
                } else {
                    None
                };

                Ok(Stmt {
                    kind: StmtKind::Let {
                        pattern: pattern.clone(),
                        ty: ty.clone(),
                        value: processed_value,
                    },
                    span: stmt.span,
                    attributes: stmt.attributes.clone(),
                })
            }
            StmtKind::Expr { expr, has_semi } => {
                let processed = self.process_expr(expr)?;
                Ok(Stmt {
                    kind: StmtKind::Expr {
                        expr: processed,
                        has_semi: *has_semi,
                    },
                    span: stmt.span,
                    attributes: stmt.attributes.clone(),
                })
            }
            // Other statements pass through
            _ => Ok(stmt.clone()),
        }
    }

    /// Process an expression for tagged literals and interpolations
    ///
    /// This is the core of compile-time literal processing.
    fn process_expr(&mut self, expr: &Expr) -> Result<Expr, Diagnostic> {
        match &expr.kind {
            // Tagged literals: d#"2024-01-15", rx#"pattern", etc.
            ExprKind::Literal(lit) => {
                if let LiteralKind::Tagged { tag, content } = &lit.kind {
                    self.process_tagged_literal(tag, content, expr.span)
                } else if let LiteralKind::InterpolatedString(interp) = &lit.kind {
                    self.process_interpolated_string(interp, expr.span)
                } else {
                    Ok(expr.clone())
                }
            }

            // Interpolated string expressions: sql"SELECT * WHERE id = {id}"
            ExprKind::InterpolatedString {
                handler,
                parts,
                exprs,
            } => self.process_interpolation(handler, parts, exprs, expr.span),

            // Recurse into compound expressions
            ExprKind::Binary { op, left, right } => {
                let processed_left = self.process_expr(left)?;
                let processed_right = self.process_expr(right)?;
                Ok(Expr {
                    kind: ExprKind::Binary {
                        op: *op,
                        left: Box::new(processed_left),
                        right: Box::new(processed_right),
                    },
                    span: expr.span,
                    ref_kind: None,
                    check_eliminated: false,
                })
            }

            ExprKind::Call { func, args, .. } => {
                let processed_func = self.process_expr(func)?;
                let processed_args: Result<List<Expr>, Diagnostic> =
                    args.iter().map(|a| self.process_expr(a)).collect();
                Ok(Expr {
                    kind: ExprKind::Call {
                        func: Heap::new(processed_func),
                        type_args: List::new(),
                        args: processed_args?,
                    },
                    span: expr.span,
                    ref_kind: None,
                    check_eliminated: false,
                })
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let processed_cond = self.process_if_condition(condition)?;
                let processed_then = self.process_block(then_branch)?;
                let processed_else = if let Some(e) = else_branch {
                    Some(Heap::new(self.process_expr(e)?))
                } else {
                    None
                };
                Ok(Expr {
                    kind: ExprKind::If {
                        condition: Heap::new(processed_cond),
                        then_branch: processed_then,
                        else_branch: processed_else,
                    },
                    span: expr.span,
                    ref_kind: None,
                    check_eliminated: false,
                })
            }

            ExprKind::Block(block) => {
                let processed_block = self.process_block(block)?;
                Ok(Expr {
                    kind: ExprKind::Block(processed_block),
                    span: expr.span,
                    ref_kind: None,
                    check_eliminated: false,
                })
            }

            // Other expressions pass through unchanged
            _ => Ok(expr.clone()),
        }
    }

    /// Process an if condition (handles both expr and let patterns)
    fn process_if_condition(
        &mut self,
        condition: &verum_ast::expr::IfCondition,
    ) -> Result<verum_ast::expr::IfCondition, Diagnostic> {
        use verum_ast::expr::ConditionKind;

        let mut processed_conditions = smallvec::SmallVec::new();

        for cond in condition.conditions.iter() {
            let processed = match cond {
                ConditionKind::Expr(expr) => ConditionKind::Expr(self.process_expr(expr)?),
                ConditionKind::Let { pattern, value } => ConditionKind::Let {
                    pattern: pattern.clone(),
                    value: self.process_expr(value)?,
                },
            };
            processed_conditions.push(processed);
        }

        Ok(verum_ast::expr::IfCondition {
            conditions: processed_conditions,
            span: condition.span,
        })
    }

    /// Process a tagged literal like d#"2024-01-15" or rx#"[a-z]+"
    ///
    /// 1. Look up handler in LiteralRegistry
    /// 2. Execute handler at compile-time
    /// 3. Replace with validated/parsed value
    fn process_tagged_literal(
        &mut self,
        tag: &Text,
        content: &Text,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        self.stats.tagged_literals_processed += 1;

        tracing::debug!(
            "Processing tagged literal: {}#\"{}\"",
            tag.as_str(),
            content.as_str()
        );

        // Look up handler
        match self.literal_registry.get_handler(tag) {
            Maybe::Some(handler) => {
                // Execute compile-time parsing
                if handler.compile_time {
                    // Note: We don't have access to SourceFile here, only FileId
                    // Passing None is safe - it just means error spans won't have full source context
                    match self
                        .literal_registry
                        .parse_literal(tag, content, span, None)
                    {
                        Ok(parsed) => {
                            // Convert parsed literal back to expression
                            self.parsed_literal_to_expr(parsed, span)
                        }
                        Err(diag) => Err(diag),
                    }
                } else {
                    // Runtime-only handler - keep as-is
                    Ok(Expr {
                        kind: ExprKind::Literal(Literal {
                            kind: LiteralKind::Tagged {
                                tag: tag.clone(),
                                content: content.clone(),
                            },
                            span,
                        }),
                        span,
                        ref_kind: None,
                        check_eliminated: false,
                    })
                }
            }
            Maybe::None => {
                // Not in LiteralRegistry - check MetaRegistry for user-defined
                // @tagged_literal handlers
                let handler_key = Text::from(format!("tagged_literal_{}", tag.as_str()));
                match self
                    .meta_registry
                    .resolve_meta_call(&self.current_module, &handler_key)
                {
                    Maybe::Some(meta_fn) => {
                        // Found a user-defined tagged literal handler in MetaRegistry.
                        // Execute it with the literal content as argument.
                        tracing::debug!(
                            "Executing user-defined tagged literal handler: {}",
                            handler_key.as_str()
                        );
                        let args = vec![ConstValue::Text(content.clone())];
                        match self.meta_context.execute_user_meta_fn(&meta_fn, args) {
                            Ok(result) => self.const_value_to_expr(result, span),
                            Err(meta_err) => Err(DiagnosticBuilder::error()
                                .message(Text::from(format!(
                                    "Tagged literal handler '{}' failed: {}",
                                    tag.as_str(),
                                    meta_err
                                )))
                                .build()),
                        }
                    }
                    Maybe::None => {
                        // Unknown tag - emit warning but keep expression
                        tracing::warn!("Unknown tagged literal tag: {}", tag.as_str());
                        Ok(Expr {
                            kind: ExprKind::Literal(Literal {
                                kind: LiteralKind::Tagged {
                                    tag: tag.clone(),
                                    content: content.clone(),
                                },
                                span,
                            }),
                            span,
                            ref_kind: None,
                            check_eliminated: false,
                        })
                    }
                }
            }
        }
    }

    /// Convert a parsed literal back to an AST expression
    fn parsed_literal_to_expr(
        &self,
        parsed: ParsedLiteral,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        use verum_ast::literal::IntSuffix;

        let lit_kind = match parsed {
            ParsedLiteral::DateTime(timestamp) => {
                // Convert to integer literal
                LiteralKind::Int(verum_ast::literal::IntLit {
                    value: timestamp as i128,
                    suffix: None,
                })
            }
            ParsedLiteral::Duration(nanos) => LiteralKind::Int(verum_ast::literal::IntLit {
                value: nanos as i128,
                suffix: Some(IntSuffix::U64), // Use a proper suffix
            }),
            ParsedLiteral::Regex(pattern) => LiteralKind::Text(StringLit::Regular(pattern)),
            ParsedLiteral::Interval {
                start,
                end,
                inclusive_start,
                inclusive_end,
            } => {
                // Keep as tagged for now - would need Interval type
                LiteralKind::Tagged {
                    tag: "interval".to_string().into(),
                    content: format!(
                        "{}{}..{}{}",
                        if inclusive_start { "[" } else { "(" },
                        start,
                        end,
                        if inclusive_end { "]" } else { ")" }
                    )
                    .into(),
                }
            }
            ParsedLiteral::Matrix { rows, cols, data } => {
                // Keep as tagged for now - would need Matrix type
                let data_str: Vec<String> = data.iter().map(|f| f.to_string()).collect();
                LiteralKind::Tagged {
                    tag: "mat".to_string().into(),
                    content: format!("[{}x{}]:{}", rows, cols, data_str.join(",")).into(),
                }
            }
            ParsedLiteral::Uri(uri) => LiteralKind::Text(StringLit::Regular(uri.into())),
            ParsedLiteral::Email(email) => LiteralKind::Text(StringLit::Regular(email.into())),
            ParsedLiteral::Uuid(uuid) => LiteralKind::Text(StringLit::Regular(uuid.into())),
            ParsedLiteral::Json(json) => LiteralKind::Text(StringLit::Regular(json.into())),
            ParsedLiteral::Xml(xml) => LiteralKind::Text(StringLit::Regular(xml.into())),
            ParsedLiteral::Yaml(yaml) => LiteralKind::Text(StringLit::Regular(yaml.into())),
            ParsedLiteral::Custom { tag, value } => LiteralKind::Tagged {
                tag,
                content: value,
            },
        };

        Ok(Expr {
            kind: ExprKind::Literal(Literal {
                kind: lit_kind,
                span,
            }),
            span,
            ref_kind: None,
            check_eliminated: false,
        })
    }

    /// Convert a ConstValue (from meta function execution) to an AST Expr
    ///
    /// This is used when a user-defined tagged literal handler or interpolation
    /// handler returns a compile-time value that needs to be spliced back into
    /// the AST.
    fn const_value_to_expr(
        &self,
        value: ConstValue,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        let lit_kind = match value {
            ConstValue::Int(n) => LiteralKind::Int(verum_ast::literal::IntLit {
                value: n,
                suffix: None,
            }),
            ConstValue::Float(f) => LiteralKind::Float(verum_ast::literal::FloatLit {
                value: f,
                suffix: None,
            }),
            ConstValue::Bool(b) => LiteralKind::Bool(b),
            ConstValue::Text(s) => LiteralKind::Text(StringLit::Regular(s)),
            ConstValue::Unit => {
                // Unit value -> unit expression (empty tuple)
                return Ok(Expr {
                    kind: ExprKind::Tuple(List::new()),
                    span,
                    ref_kind: None,
                    check_eliminated: false,
                });
            }
            ConstValue::Array(items) => {
                // Array of const values -> array expression
                let exprs: Result<List<Expr>, Diagnostic> = items
                    .into_iter()
                    .map(|item| self.const_value_to_expr(item, span))
                    .collect();
                return Ok(Expr {
                    kind: ExprKind::Array(ArrayExpr::List(exprs?)),
                    span,
                    ref_kind: None,
                    check_eliminated: false,
                });
            }
            other => {
                // For complex types (Map, Tuple, etc.), convert to a text representation
                // and emit as a string literal. User handlers should return simple types.
                LiteralKind::Text(StringLit::Regular(
                    Text::from(format!("{}", other)),
                ))
            }
        };

        Ok(Expr {
            kind: ExprKind::Literal(Literal {
                kind: lit_kind,
                span,
            }),
            span,
            ref_kind: None,
            check_eliminated: false,
        })
    }

    /// Process an interpolated string literal
    fn process_interpolated_string(
        &mut self,
        interp: &verum_ast::literal::InterpolatedStringLit,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        self.stats.interpolations_processed += 1;

        // For now, keep as-is - would need runtime desugaring
        Ok(Expr {
            kind: ExprKind::Literal(Literal {
                kind: LiteralKind::InterpolatedString(interp.clone()),
                span,
            }),
            span,
            ref_kind: None,
            check_eliminated: false,
        })
    }

    /// Process interpolation expression like sql"SELECT * WHERE id = {id}"
    ///
    /// 1. Look up @interpolation_handler for the handler name
    /// 2. Execute handler at compile-time
    /// 3. Replace with generated code
    fn process_interpolation(
        &mut self,
        handler: &Text,
        parts: &List<Text>,
        exprs: &List<Expr>,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        self.stats.interpolations_processed += 1;

        tracing::debug!(
            "Processing interpolation: {}\"...\" with {} expressions",
            handler.as_str(),
            exprs.len()
        );

        // Look up interpolation handler
        let handler_key = Text::from(format!("interpolation_{}", handler.as_str()));
        match self
            .meta_registry
            .resolve_macro(&self.current_module, &handler_key)
        {
            Maybe::Some(macro_def) => {
                tracing::debug!(
                    "Found interpolation handler: {}",
                    macro_def.expander.as_str()
                );

                // For built-in SQL handler, use dedicated processing
                if handler.as_str() == "sql" {
                    return self.process_sql_interpolation(parts, exprs, span);
                }

                // Execute user-defined interpolation handler via MetaRegistry.
                // Look up the expander function and invoke it with the parts and
                // expression representations as arguments.
                let expander_name = macro_def.expander.clone();
                match self
                    .meta_registry
                    .resolve_meta_call(&self.current_module, &expander_name)
                {
                    Maybe::Some(meta_fn) => {
                        // Build arguments: parts as array of text, exprs as array of text
                        let parts_val = ConstValue::Array(
                            parts.iter().map(|p| ConstValue::Text(p.clone())).collect(),
                        );
                        let exprs_val = ConstValue::Array(
                            exprs
                                .iter()
                                .map(|e| ConstValue::Text(Text::from(format!("{:?}", e.kind))))
                                .collect(),
                        );
                        let args = vec![parts_val, exprs_val];

                        match self.meta_context.execute_user_meta_fn(&meta_fn, args) {
                            Ok(result) => self.const_value_to_expr(result, span),
                            Err(meta_err) => Err(DiagnosticBuilder::error()
                                .message(Text::from(format!(
                                    "Interpolation handler '{}' failed: {}",
                                    handler.as_str(),
                                    meta_err
                                )))
                                .build()),
                        }
                    }
                    Maybe::None => {
                        // Expander function not found - keep original expression
                        tracing::warn!(
                            "Interpolation handler expander '{}' not found",
                            expander_name.as_str()
                        );
                        Ok(Expr {
                            kind: ExprKind::InterpolatedString {
                                handler: handler.clone(),
                                parts: parts.iter().cloned().collect(),
                                exprs: exprs.iter().cloned().collect(),
                            },
                            span,
                            ref_kind: None,
                            check_eliminated: false,
                        })
                    }
                }
            }
            Maybe::None => {
                // Check for built-in handlers
                match handler.as_str() {
                    "f" => self.process_format_interpolation(parts, exprs, span),
                    "sql" => self.process_sql_interpolation(parts, exprs, span),
                    "html" => self.process_html_interpolation(parts, exprs, span),
                    _ => {
                        // Unknown handler - keep as-is
                        Ok(Expr {
                            kind: ExprKind::InterpolatedString {
                                handler: handler.clone(),
                                parts: parts.iter().cloned().collect(),
                                exprs: exprs.iter().cloned().collect(),
                            },
                            span,
                            ref_kind: None,
                            check_eliminated: false,
                        })
                    }
                }
            }
        }
    }

    /// Process format string: f"Hello {name}!"
    ///
    /// Format strings are kept as InterpolatedString expressions.
    /// The type checker handles them directly, inferring Text type
    /// and type-checking embedded expressions.
    fn process_format_interpolation(
        &mut self,
        parts: &List<Text>,
        exprs: &List<Expr>,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        // Keep as InterpolatedString - the type checker handles this directly
        // No desugaring to format() function call needed
        //
        // The type checker at verum_types/src/infer.rs handles InterpolatedString:
        // - Type checks all embedded expressions
        // - Returns Type::text() as the result type
        Ok(Expr {
            kind: ExprKind::InterpolatedString {
                handler: Text::from("f"),
                parts: parts.iter().cloned().collect(),
                exprs: exprs.iter().cloned().collect(),
            },
            span,
            ref_kind: None,
            check_eliminated: false,
        })
    }

    /// DEPRECATED: Old format desugaring (kept for reference)
    #[allow(dead_code)]
    fn process_format_interpolation_desugar(
        &mut self,
        parts: &List<Text>,
        exprs: &List<Expr>,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        // Desugar to format!() call
        // f"Hello {name}!" => format!("Hello {}!", name)

        // Build format string with {} placeholders
        let mut format_str = String::new();
        for (i, part) in parts.iter().enumerate() {
            format_str.push_str(part.as_str());
            if i < exprs.len() {
                format_str.push_str("{}");
            }
        }

        // Generate call to format function
        let format_path = verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                "format",
                span
            ),)],
            span,
        };

        let mut args = Vec::new();
        // First arg: format string
        args.push(Expr {
            kind: ExprKind::Literal(Literal {
                kind: LiteralKind::Text(StringLit::Regular(format_str.into())),
                span,
            }),
            span,
            ref_kind: None,
            check_eliminated: false,
        });
        // Remaining args: interpolated expressions
        for expr in exprs.iter() {
            args.push(expr.clone());
        }

        Ok(Expr {
            kind: ExprKind::Call {
                func: Box::new(Expr {
                    kind: ExprKind::Path(format_path),
                    span,
                    ref_kind: None,
                    check_eliminated: false,
                }),
                type_args: List::new(),
                args: args.into(),
            },
            span,
            ref_kind: None,
            check_eliminated: false,
        })
    }

    /// Process SQL interpolation: sql"SELECT * WHERE id = {id}"
    ///
    /// SECURITY: Generates parameterized query to prevent SQL injection
    fn process_sql_interpolation(
        &mut self,
        parts: &List<Text>,
        exprs: &List<Expr>,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        use crate::interpolation::sql::SqlInterpolationHandler;

        // Build template with ? placeholders
        let mut template = String::new();
        for (i, part) in parts.iter().enumerate() {
            template.push_str(part.as_str());
            if i < exprs.len() {
                template.push('?'); // Parameterized placeholder
            }
        }

        // Validate template for dangerous patterns
        SqlInterpolationHandler::validate_template(&Text::from(template.clone()), span)?;

        // Generate SqlQuery.with_params(template, [params...])
        let sql_query_path = verum_ast::ty::Path {
            segments: smallvec::smallvec![
                verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                    "SqlQuery",
                    span,
                )),
                verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                    "with_params",
                    span,
                )),
            ],
            span,
        };

        let mut args = Vec::new();

        // First arg: template string
        args.push(Expr {
            kind: ExprKind::Literal(Literal {
                kind: LiteralKind::Text(StringLit::Regular(template.into())),
                span,
            }),
            span,
            ref_kind: None,
            check_eliminated: false,
        });

        // Second arg: list of parameters
        let params_list = Expr {
            kind: ExprKind::Array(ArrayExpr::List(exprs.iter().cloned().collect())),
            span,
            ref_kind: None,
            check_eliminated: false,
        };
        args.push(params_list);

        Ok(Expr {
            kind: ExprKind::Call {
                func: Box::new(Expr {
                    kind: ExprKind::Path(sql_query_path),
                    span,
                    ref_kind: None,
                    check_eliminated: false,
                }),
                type_args: List::new(),
                args: args.into(),
            },
            span,
            ref_kind: None,
            check_eliminated: false,
        })
    }

    /// Process HTML interpolation: html"<h1>{title}</h1>"
    ///
    /// SECURITY: Auto-escapes interpolated values to prevent XSS
    fn process_html_interpolation(
        &mut self,
        parts: &List<Text>,
        exprs: &List<Expr>,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        // Generate HtmlBuilder with escaped values
        let html_builder_path = verum_ast::ty::Path {
            segments: smallvec::smallvec![
                verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                    "HtmlBuilder",
                    span,
                )),
                verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                    "from_template",
                    span,
                )),
            ],
            span,
        };

        // Convert parts to list of strings
        let parts_exprs: Vec<Expr> = parts
            .iter()
            .map(|p| Expr {
                kind: ExprKind::Literal(Literal {
                    kind: LiteralKind::Text(StringLit::Regular(p.clone().into())),
                    span,
                }),
                span,
                ref_kind: None,
                check_eliminated: false,
            })
            .collect();

        // Wrap expressions with html_escape
        let escaped_exprs: Vec<Expr> = exprs
            .iter()
            .map(|e| {
                let escape_path = verum_ast::ty::Path {
                    segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                        verum_ast::Ident::new("html_escape", span),
                    )],
                    span,
                };
                Expr {
                    kind: ExprKind::Call {
                        func: Heap::new(Expr {
                            kind: ExprKind::Path(escape_path),
                            span,
                            ref_kind: None,
                            check_eliminated: false,
                        }),
                        type_args: List::new(),
                        args: List::from(vec![e.clone()]),
                    },
                    span,
                    ref_kind: None,
                    check_eliminated: false,
                }
            })
            .collect();

        let mut args: Vec<Expr> = Vec::new();
        args.push(Expr {
            kind: ExprKind::Array(ArrayExpr::List(parts_exprs.into_iter().collect())),
            span,
            ref_kind: None,
            check_eliminated: false,
        });
        args.push(Expr {
            kind: ExprKind::Array(ArrayExpr::List(escaped_exprs.into_iter().collect())),
            span,
            ref_kind: None,
            check_eliminated: false,
        });

        Ok(Expr {
            kind: ExprKind::Call {
                func: Box::new(Expr {
                    kind: ExprKind::Path(html_builder_path),
                    span,
                    ref_kind: None,
                    check_eliminated: false,
                }),
                type_args: List::new(),
                args: args.into(),
            },
            span,
            ref_kind: None,
            check_eliminated: false,
        })
    }

    /// Expand @derive attributes on a type declaration
    fn expand_type_derives(
        &mut self,
        item: &Item,
        type_decl: &TypeDecl,
    ) -> Result<List<Item>, Diagnostic> {
        let mut result_items = List::new();

        // First, keep the original type declaration
        result_items.push(item.clone());

        // Find all @derive attributes
        let derives = self.extract_derive_names(&item.attributes);

        if derives.is_empty() {
            return Ok(result_items);
        }

        tracing::debug!(
            "Expanding derives for type {}: {:?}",
            type_decl.name,
            derives
        );

        // Expand each derive
        for derive_name in derives.iter() {
            match self
                .derive_registry
                .expand(derive_name.as_str(), type_decl, item.span)
            {
                Ok(impl_item) => {
                    self.stats.derives_expanded += 1;
                    result_items.push(impl_item);
                    tracing::trace!(
                        "Generated {} implementation for {}",
                        derive_name,
                        type_decl.name
                    );
                }
                Err(err) => {
                    // Convert DeriveError to Diagnostic
                    return Err(self.derive_error_to_diagnostic(&err));
                }
            }
        }

        Ok(result_items)
    }

    /// Extract derive names from attributes
    ///
    /// Parses @derive(Debug, Clone, Serialize) style attributes
    fn extract_derive_names(&self, attributes: &[Attribute]) -> List<Text> {
        let mut derives = List::new();

        for attr in attributes {
            if attr.name.as_str() == "derive" {
                // Parse derive arguments from Option<Vec<Expr>>
                if let Some(ref args) = attr.args {
                    for arg in args.iter() {
                        // Each arg should be a Path expression like "Debug" or "Clone"
                        if let ExprKind::Path(path) = &arg.kind {
                            if let Some(ident) = path.as_ident() {
                                derives.push(Text::from(ident.as_str()));
                            }
                        }
                    }
                }
            }
        }

        derives
    }

    /// Convert DeriveError to Diagnostic
    fn derive_error_to_diagnostic(&self, err: &DeriveError) -> Diagnostic {
        match err {
            DeriveError::UnknownDerive { name, .. } => DiagnosticBuilder::error()
                .message(Text::from(format!("Unknown derive macro: `{}`", name.as_str())))
                .help(Text::from("Available derives: Debug, Clone, PartialEq, Default, Serialize, Deserialize"))
                .build(),
            DeriveError::UnsupportedTypeKind { kind, hint, .. } => DiagnosticBuilder::error()
                .message(Text::from(format!("Cannot derive for {}", kind.as_str())))
                .help(Text::from(hint.to_string()))
                .build(),
            DeriveError::FieldNotImplement {
                field_name,
                protocol,
                ..
            } => DiagnosticBuilder::error()
                .message(Text::from(format!(
                    "Field `{}` does not implement `{}`",
                    field_name.as_str(),
                    protocol.as_str()
                )))
                .help(Text::from(format!(
                    "Add @derive({}) to the field's type or implement it manually",
                    protocol.as_str()
                )))
                .build(),
            _ => DiagnosticBuilder::error().message(Text::from(err.to_string())).build(),
        }
    }

    /// Get expansion statistics
    pub fn stats(&self) -> &ExpansionStats {
        &self.stats
    }
}

impl Default for MacroExpansionPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for MacroExpansionPhase {
    fn name(&self) -> &str {
        "Phase 3: Macro Expansion & Literal Processing"
    }

    fn description(&self) -> &str {
        "Expand @derive macros, process tagged literals, execute meta functions in sandboxed environment"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract modules from input
        let modules = match &input.data {
            PhaseData::AstModules(modules) => modules,
            _ => {
                let diag = DiagnosticBuilder::error()
                    .message(Text::from("Invalid input for macro expansion phase: expected AST modules"))
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Create mutable phase for expansion
        let mut phase = Self::new();

        // Expand macros
        let expanded_modules = phase.expand_modules(modules)?;

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);

        // Record statistics
        metrics.add_custom_metric("derives_expanded", phase.stats.derives_expanded.to_string());
        metrics.add_custom_metric("items_generated", phase.stats.items_generated.to_string());
        metrics.add_custom_metric(
            "tagged_literals_processed",
            phase.stats.tagged_literals_processed.to_string(),
        );
        metrics.add_custom_metric(
            "meta_functions_executed",
            phase.stats.meta_functions_executed.to_string(),
        );

        tracing::info!(
            "Macro expansion complete: {} derives, {} items generated, {:.2}ms",
            phase.stats.derives_expanded,
            phase.stats.items_generated,
            duration.as_secs_f64() * 1000.0
        );

        Ok(PhaseOutput {
            data: PhaseData::AstModules(expanded_modules),
            warnings: List::new(),
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        true
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::List;

    fn create_test_module() -> Module {
        let span = Span::default();
        Module {
            items: List::new(),
            attributes: List::new(),
            file_id: verum_ast::FileId::new(0),
            span,
        }
    }

    #[test]
    fn test_phase_creation() {
        let phase = MacroExpansionPhase::new();
        assert_eq!(phase.stats.derives_expanded, 0);
        assert_eq!(phase.stats.items_generated, 0);
    }

    #[test]
    fn test_empty_module_expansion() {
        let mut phase = MacroExpansionPhase::new();
        let module = create_test_module();

        let result = phase.expand_module(&module);
        assert!(result.is_ok());

        let expanded = result.unwrap();
        assert!(expanded.items.is_empty());
    }

    #[test]
    fn test_stats_tracking() {
        let mut phase = MacroExpansionPhase::new();
        let module = create_test_module();

        let _ = phase.expand_module(&module);

        // Stats should be tracked even for empty module
        assert_eq!(phase.stats.errors, 0);
    }
}
