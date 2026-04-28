//! Phase 0: Entry Point Detection with Context Validation
//!
//! Detects `async fn main()` or `fn main()` entry point and validates
//! context system compliance.
//!
//! ## Responsibilities
//!
//! 1. Find the main function across all modules
//! 2. Determine if it's sync or async
//! 3. Validate that all used contexts are explicitly declared with 'using'
//! 4. Ensure all contexts have corresponding 'provide' statements
//! 5. Detect circular context group references
//! 6. Validate async/sync consistency for contexts
//!
//! ## Output
//!
//! - `MainConfig`: Configuration for entry point validation
//! - `EntryContextValidation`: Context validation results
//!
//! ## Context System Compliance
//!
//! Context system integration:
//! - ALL contexts must be explicitly declared with 'using'
//! - ALL contexts must be provided with 'provide' statements
//! - NO automatic provision of contexts (auto-provide feature removed)
//!
//! Phase 0: Detects main() function, validates signature (fn main() or
//! fn main(args: List<Text>)), and configures execution mode.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use verum_ast::decl::{FunctionBody, FunctionDecl, ItemKind};
use verum_ast::{Expr, ExprKind, Module, Stmt, StmtKind};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_common::{List, Text};

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};

/// Entry point detection phase with context validation
pub struct EntryDetectionPhase {
    /// Known context names (built-in and registered)
    known_contexts: HashSet<Text>,
    /// Known context groups
    context_groups: HashMap<Text, HashSet<Text>>,
}

impl EntryDetectionPhase {
    pub fn new() -> Self {
        let mut known_contexts: HashSet<Text> = HashSet::new();

        // Built-in contexts from the standard library
        known_contexts.insert(Text::from("Database"));
        known_contexts.insert(Text::from("Logger"));
        known_contexts.insert(Text::from("FileSystem"));
        known_contexts.insert(Text::from("Network"));
        known_contexts.insert(Text::from("Clock"));
        known_contexts.insert(Text::from("Random"));
        known_contexts.insert(Text::from("Config"));
        known_contexts.insert(Text::from("Cache"));
        known_contexts.insert(Text::from("Metrics"));
        known_contexts.insert(Text::from("Tracer"));
        known_contexts.insert(Text::from("HttpClient"));
        known_contexts.insert(Text::from("Scheduler"));

        // Built-in context groups
        let mut context_groups: HashMap<Text, HashSet<Text>> = HashMap::new();

        // WebContext group
        let mut web_contexts: HashSet<Text> = HashSet::new();
        web_contexts.insert(Text::from("HttpClient"));
        web_contexts.insert(Text::from("Logger"));
        web_contexts.insert(Text::from("Config"));
        context_groups.insert(Text::from("WebContext"), web_contexts);

        // DatabaseContext group
        let mut db_contexts: HashSet<Text> = HashSet::new();
        db_contexts.insert(Text::from("Database"));
        db_contexts.insert(Text::from("Logger"));
        db_contexts.insert(Text::from("Metrics"));
        context_groups.insert(Text::from("DatabaseContext"), db_contexts);

        Self {
            known_contexts,
            context_groups,
        }
    }

    /// Register a custom context
    pub fn register_context(&mut self, name: Text) {
        self.known_contexts.insert(name);
    }

    /// Register a context group
    pub fn register_context_group(&mut self, name: Text, contexts: HashSet<Text>) {
        self.context_groups.insert(name, contexts);
    }

    /// Detect entry point across multiple modules and validate context usage
    pub fn detect_entry_point(&self, modules: &[Module]) -> Result<MainConfig, List<Diagnostic>> {
        let _start = Instant::now();

        // Verum execution-mode contract — strict separation:
        //
        //   • **Application** = no shebang, declares `fn main()`. Runs
        //     via interpreter or AOT. `main` is THE entry point.
        //
        //   • **Script** = shebang at byte 0, no `fn main()`. Top-level
        //     statements are folded into a synthesised
        //     `__verum_script_main` wrapper which is THE entry point.
        //     A `fn main` declared inside a script-tagged module is a
        //     regular function — callable but **not** treated as the
        //     program entry.
        //
        // The two roles do not overlap: `main` only ever drives
        // application modules, `__verum_script_main` only ever drives
        // script modules.

        // Application path: find `fn main` in any non-script module.
        let main_fn = modules
            .iter()
            .filter(|m| !m.is_script())
            .flat_map(|m| &m.items)
            .filter_map(|item| {
                if let ItemKind::Function(func) = &item.kind {
                    if func.name.as_str() == "main" {
                        Some(func)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .next();

        // Script path: the parser synthesises `__verum_script_main`
        // from top-level statements in script-tagged modules. Used
        // when no application `fn main` resolved above. A `fn main`
        // in a script module is intentionally ignored here so the
        // boundary between application and script entry semantics
        // stays unambiguous.
        let script_entry = if main_fn.is_none() {
            modules
                .iter()
                .filter(|m| m.is_script())
                .flat_map(|m| &m.items)
                .filter_map(|item| {
                    if let ItemKind::Function(func) = &item.kind {
                        if func.name.as_str() == "__verum_script_main" {
                            Some(func)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .next()
        } else {
            None
        };

        let main_fn = match main_fn.or(script_entry) {
            Some(func) => func,
            None => {
                // Verum execution-mode contract:
                //   1. Library/binary mode: a `fn main()` (or `async fn main()`)
                //      defines the entry.
                //   2. Script mode: the file MUST start with a `#!` shebang
                //      line; the parser then folds top-level statements into
                //      `__verum_script_main` and tags the module with
                //      `@![__verum_kind("script")]`.
                //
                // If neither path produced an entry, surface BOTH options so
                // the user can pick the one that fits — without a shebang the
                // file is treated as library code and needs `fn main()`; with
                // a shebang top-level statements (`print("hi");`) work
                // directly and no `fn main()` is required.
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message(Text::from("No entry point found"))
                    .help(Text::from(
                        "Add 'fn main() { ... }' (library/binary mode), \
                         or start the file with a `#!/usr/bin/env verum` \
                         shebang line and use top-level statements (script mode).",
                    ))
                    .build();
                return Err(vec![diag].into());
            }
        };

        // Check for multiple main functions
        let main_count = modules
            .iter()
            .flat_map(|m| &m.items)
            .filter(|item| {
                if let ItemKind::Function(func) = &item.kind {
                    func.name.as_str() == "main"
                } else {
                    false
                }
            })
            .count();

        if main_count > 1 {
            let diag = DiagnosticBuilder::new(Severity::Error)
                .message(Text::from(format!("Multiple main functions found ({})", main_count)))
                .help(Text::from("Only one main function is allowed"))
                .build();
            return Err(vec![diag].into());
        }

        // Perform full context validation
        let mut validation = EntryContextValidation::new();
        self.validate_context_usage(main_fn, modules, &mut validation)?;

        let config = if main_fn.is_async {
            MainConfig::Async
        } else {
            MainConfig::Sync
        };

        Ok(config)
    }

    /// Validate that all used contexts are declared with 'using' and provided with 'provide'
    ///
    /// Context system validation rules:
    /// - ALL contexts must be explicitly declared with `using [Context1, Context2]`
    ///   after the function return type
    /// - ALL contexts must be provided with `provide Context = impl` statements
    ///   in a lexically enclosing scope before use
    /// - NO automatic/implicit provision of contexts (auto-provide removed)
    /// - Context lookup at runtime uses task-local storage (~5-30ns overhead)
    /// - Context groups (e.g., `using WebContext = [Database, Logger]`) are expanded
    ///   at compile time
    fn validate_context_usage(
        &self,
        func: &FunctionDecl,
        modules: &[Module],
        validation: &mut EntryContextValidation,
    ) -> Result<(), List<Diagnostic>> {
        let mut diagnostics = List::new();

        // Step 1: Extract declared contexts from 'using' clause
        self.extract_declared_contexts(func, validation);

        // Step 2: Validate all declared contexts are registered
        self.validate_declarations(validation, &mut diagnostics);

        // Step 3: Expand context groups
        self.expand_context_groups(validation, &mut diagnostics)?;

        // Step 4: Scan function body for context accesses
        if let Some(body) = &func.body {
            match body {
                FunctionBody::Block(block) => {
                    for stmt in &block.stmts {
                        self.scan_stmt_for_contexts(stmt, validation);
                    }
                    if let Some(expr) = &block.expr {
                        self.scan_expr_for_contexts(expr, validation);
                    }
                }
                FunctionBody::Expr(expr) => {
                    self.scan_expr_for_contexts(expr, validation);
                }
            }
        }

        // Step 5: Verify all accessed contexts are declared
        self.validate_all_declared(validation, &mut diagnostics);

        // Step 6: Check for 'provide' statements
        self.check_provide_statements(modules, validation, &mut diagnostics);

        // Step 7: Validate async/sync consistency
        if func.is_async {
            self.validate_async_contexts(validation, &mut diagnostics);
        }

        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        Ok(())
    }

    /// Extract contexts from the function's 'using' clause
    fn extract_declared_contexts(
        &self,
        func: &FunctionDecl,
        validation: &mut EntryContextValidation,
    ) {
        // The contexts field contains ContextRequirement from 'using [...]' clause
        for context_req in &func.contexts {
            let context_name = context_req
                .path
                .segments
                .last()
                .and_then(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        Some(ident.name.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| Text::from("unknown"));

            validation.declared_contexts.insert(context_name.clone());

            // Track source location for error reporting
            validation
                .context_spans
                .insert(context_name, context_req.span);
        }
    }

    /// Validate that all declared contexts exist in the registry
    fn validate_declarations(
        &self,
        validation: &EntryContextValidation,
        diagnostics: &mut List<Diagnostic>,
    ) {
        for context_name in &validation.declared_contexts {
            // Check if it's a known context or a context group
            if !self.known_contexts.contains(context_name)
                && !self.context_groups.contains_key(context_name)
            {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message(Text::from(format!("Context '{}' is not declared", context_name)))
                    .help(Text::from(format!(
                        "Register the context with 'context {} {{ ... }}' or check spelling",
                        context_name
                    )))
                    .build();
                diagnostics.push(diag);
            }
        }
    }

    /// Expand context groups to their constituent contexts
    fn expand_context_groups(
        &self,
        validation: &mut EntryContextValidation,
        diagnostics: &mut List<Diagnostic>,
    ) -> Result<(), List<Diagnostic>> {
        let mut visited = HashSet::new();
        let groups_to_expand: Vec<_> = validation
            .declared_contexts
            .iter()
            .filter(|name| self.context_groups.contains_key(*name))
            .cloned()
            .collect();

        for group_name in groups_to_expand {
            self.expand_group_recursive(&group_name, validation, &mut visited, diagnostics)?;
        }

        Ok(())
    }

    /// Recursively expand a context group, detecting cycles
    fn expand_group_recursive(
        &self,
        group_name: &Text,
        validation: &mut EntryContextValidation,
        visited: &mut HashSet<Text>,
        diagnostics: &mut List<Diagnostic>,
    ) -> Result<(), List<Diagnostic>> {
        // Check for circular reference
        if visited.contains(group_name) {
            let diag = DiagnosticBuilder::new(Severity::Error)
                .message(Text::from(format!(
                    "Circular context group reference detected: '{}'",
                    group_name
                )))
                .help(Text::from("Remove the circular dependency between context groups"))
                .build();
            diagnostics.push(diag);
            return Err(diagnostics.clone());
        }

        visited.insert(group_name.clone());

        if let Some(contexts) = self.context_groups.get(group_name) {
            for context in contexts {
                validation
                    .expanded_groups
                    .entry(group_name.clone())
                    .or_insert_with(HashSet::new)
                    .insert(context.clone());

                // If this context is itself a group, expand it
                if self.context_groups.contains_key(context) {
                    self.expand_group_recursive(context, validation, visited, diagnostics)?;
                }
            }
        }

        visited.remove(group_name);
        Ok(())
    }

    /// Recursively scan an expression for context accesses
    fn scan_expr_for_contexts(&self, expr: &Expr, validation: &mut EntryContextValidation) {
        match &expr.kind {
            // Method calls on contexts: Context.method(...)
            ExprKind::MethodCall {
                receiver,
                method: _,
                args,
                ..
            } => {
                // Check if receiver is a context name
                if let ExprKind::Path(path) = &receiver.kind {
                    if let Some(name) = path.as_ident() {
                        if self.known_contexts.contains(&name.name)
                            || self.context_groups.contains_key(&name.name)
                        {
                            validation.accessed_contexts.insert(name.name.clone());
                        }
                    }
                }

                // Scan receiver and arguments
                self.scan_expr_for_contexts(receiver, validation);
                for arg in args {
                    self.scan_expr_for_contexts(arg, validation);
                }
            }

            // Field access: Context.field
            ExprKind::Field { expr: base, .. } => {
                if let ExprKind::Path(path) = &base.kind {
                    if let Some(name) = path.as_ident() {
                        if self.known_contexts.contains(&name.name)
                            || self.context_groups.contains_key(&name.name)
                        {
                            validation.accessed_contexts.insert(name.name.clone());
                        }
                    }
                }
                self.scan_expr_for_contexts(base, validation);
            }

            // Direct context reference
            ExprKind::Path(path) => {
                if let Some(name) = path.as_ident() {
                    if self.known_contexts.contains(&name.name)
                        || self.context_groups.contains_key(&name.name)
                    {
                        validation.accessed_contexts.insert(name.name.clone());
                    }
                }
            }

            // Function call with context argument
            ExprKind::Call { func, args, .. } => {
                self.scan_expr_for_contexts(func, validation);
                for arg in args {
                    self.scan_expr_for_contexts(arg, validation);
                }
            }

            // If expression
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Scan conditions (which may contain let patterns with expressions)
                for cond_kind in &condition.conditions {
                    match cond_kind {
                        verum_ast::expr::ConditionKind::Expr(expr) => {
                            self.scan_expr_for_contexts(expr, validation);
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.scan_expr_for_contexts(value, validation);
                        }
                    }
                }
                // Scan then branch (which is a Block)
                for stmt in &then_branch.stmts {
                    self.scan_stmt_for_contexts(stmt, validation);
                }
                if let Some(expr) = &then_branch.expr {
                    self.scan_expr_for_contexts(expr, validation);
                }
                // Scan else branch if present
                if let Some(else_expr) = else_branch {
                    self.scan_expr_for_contexts(else_expr, validation);
                }
            }

            // Match expression
            ExprKind::Match { expr, arms } => {
                self.scan_expr_for_contexts(expr, validation);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.scan_expr_for_contexts(guard, validation);
                    }
                    self.scan_expr_for_contexts(&arm.body, validation);
                }
            }

            // Binary operations
            ExprKind::Binary { left, right, .. } => {
                self.scan_expr_for_contexts(left, validation);
                self.scan_expr_for_contexts(right, validation);
            }

            // Unary operations
            ExprKind::Unary { expr, .. } => {
                self.scan_expr_for_contexts(expr, validation);
            }

            // Closures - scan body
            ExprKind::Closure { body, .. } => {
                self.scan_expr_for_contexts(body, validation);
            }

            // Tuples
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.scan_expr_for_contexts(elem, validation);
                }
            }

            // Arrays
            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::expr::ArrayExpr::List(elements) => {
                    for elem in elements {
                        self.scan_expr_for_contexts(elem, validation);
                    }
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.scan_expr_for_contexts(value, validation);
                    self.scan_expr_for_contexts(count, validation);
                }
            },

            // Record initialization
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let Some(value) = &field.value {
                        self.scan_expr_for_contexts(value, validation);
                    }
                }
                if let Some(base_expr) = base {
                    self.scan_expr_for_contexts(base_expr, validation);
                }
            }

            // Return expression
            ExprKind::Return(expr_opt) => {
                if let Some(inner) = expr_opt {
                    self.scan_expr_for_contexts(inner, validation);
                }
            }

            // Await expression (for async contexts)
            ExprKind::Await(inner) => {
                self.scan_expr_for_contexts(inner, validation);
            }

            // Other expressions - no recursion needed
            _ => {}
        }
    }

    /// Scan a statement for context accesses
    fn scan_stmt_for_contexts(&self, stmt: &Stmt, validation: &mut EntryContextValidation) {
        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Some(init_expr) = value {
                    self.scan_expr_for_contexts(init_expr, validation);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.scan_expr_for_contexts(value, validation);
                for stmt in &else_block.stmts {
                    self.scan_stmt_for_contexts(stmt, validation);
                }
                if let Some(expr) = &else_block.expr {
                    self.scan_expr_for_contexts(expr, validation);
                }
            }
            StmtKind::Expr { expr, .. } => {
                self.scan_expr_for_contexts(expr, validation);
            }
            StmtKind::Provide { context, value, .. } => {
                // Note: 'provide' introduces a context
                validation.provided_contexts.insert(context.clone());
                self.scan_expr_for_contexts(value, validation);
            }
            StmtKind::ProvideScope {
                context,
                value,
                block,
                ..
            } => {
                // Block-scoped provide introduces a context for the block
                validation.provided_contexts.insert(context.clone());
                self.scan_expr_for_contexts(value, validation);
                self.scan_expr_for_contexts(block, validation);
            }
            StmtKind::Defer(expr) => {
                self.scan_expr_for_contexts(expr, validation);
            }
            StmtKind::Errdefer(expr) => {
                self.scan_expr_for_contexts(expr, validation);
            }
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }

    /// Verify all accessed contexts are declared in 'using' clause
    fn validate_all_declared(
        &self,
        validation: &EntryContextValidation,
        diagnostics: &mut List<Diagnostic>,
    ) {
        // Get all contexts that should be declared (including from expanded groups)
        let mut effective_declared = validation.declared_contexts.clone();

        for (_group, contexts) in &validation.expanded_groups {
            effective_declared.extend(contexts.clone());
        }

        // Find undeclared accesses
        for accessed in &validation.accessed_contexts {
            if !effective_declared.contains(accessed)
                && !validation.provided_contexts.contains(accessed)
            {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message(Text::from(format!(
                        "Context '{}' not declared with 'using [...]'",
                        accessed
                    )))
                    .help(Text::from(format!(
                        "Add 'using [{}]' to the function signature",
                        accessed
                    )))
                    .build();
                diagnostics.push(diag);
            }
        }
    }

    /// Check for 'provide' statements for required contexts
    fn check_provide_statements(
        &self,
        modules: &[Module],
        validation: &mut EntryContextValidation,
        diagnostics: &mut List<Diagnostic>,
    ) {
        // Scan all modules for 'provide' statements
        for module in modules {
            for item in &module.items {
                if let ItemKind::Function(func) = &item.kind {
                    if let Some(body) = &func.body {
                        match body {
                            FunctionBody::Block(block) => {
                                for stmt in &block.stmts {
                                    self.scan_stmt_for_provide(stmt, validation);
                                }
                                if let Some(expr) = &block.expr {
                                    self.scan_for_provide_stmts(expr, validation);
                                }
                            }
                            FunctionBody::Expr(expr) => {
                                self.scan_for_provide_stmts(expr, validation);
                            }
                        }
                    }
                }
            }
        }

        // Get all contexts that should be declared
        let mut effective_declared = validation.declared_contexts.clone();
        for (_group, contexts) in &validation.expanded_groups {
            effective_declared.extend(contexts.clone());
        }

        // Check that all declared contexts have a provide statement
        for declared in &effective_declared {
            // Skip if it's a context group (those expand to individual contexts)
            if self.context_groups.contains_key(declared) {
                continue;
            }

            if !validation.provided_contexts.contains(declared) {
                let diag = DiagnosticBuilder::new(Severity::Warning)
                    .message(Text::from(format!(
                        "Context '{}' declared but no 'provide' statement found",
                        declared
                    )))
                    .help(Text::from(format!(
                        "Add 'provide {} = implementation' before using the context",
                        declared
                    )))
                    .build();
                diagnostics.push(diag);
            }
        }
    }

    /// Scan statement for 'provide' statements
    fn scan_stmt_for_provide(&self, stmt: &Stmt, validation: &mut EntryContextValidation) {
        match &stmt.kind {
            StmtKind::Provide { context, .. } => {
                validation.provided_contexts.insert(context.clone());
            }
            StmtKind::Expr { expr, .. } => {
                self.scan_for_provide_stmts(expr, validation);
            }
            StmtKind::Let { value, .. } => {
                if let Some(expr) = value {
                    self.scan_for_provide_stmts(expr, validation);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.scan_for_provide_stmts(value, validation);
                for stmt in &else_block.stmts {
                    self.scan_stmt_for_provide(stmt, validation);
                }
                if let Some(expr) = &else_block.expr {
                    self.scan_for_provide_stmts(expr, validation);
                }
            }
            _ => {}
        }
    }

    /// Scan expression for 'provide' statements
    fn scan_for_provide_stmts(&self, expr: &Expr, validation: &mut EntryContextValidation) {
        match &expr.kind {
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                // Scan then branch (Block)
                for stmt in &then_branch.stmts {
                    self.scan_stmt_for_provide(stmt, validation);
                }
                if let Some(expr) = &then_branch.expr {
                    self.scan_for_provide_stmts(expr, validation);
                }
                // Scan else branch if present
                if let Some(else_expr) = else_branch {
                    self.scan_for_provide_stmts(else_expr, validation);
                }
            }
            ExprKind::Match { arms, .. } => {
                for arm in arms {
                    self.scan_for_provide_stmts(&arm.body, validation);
                }
            }
            _ => {}
        }
    }

    /// Validate async/sync consistency for contexts
    fn validate_async_contexts(
        &self,
        validation: &EntryContextValidation,
        diagnostics: &mut List<Diagnostic>,
    ) {
        // Some contexts are sync-only
        let sync_only_contexts: HashSet<Text> = HashSet::from([
            Text::from("Random"), // Non-deterministic, should be sync
        ]);

        for declared in &validation.declared_contexts {
            if sync_only_contexts.contains(declared) {
                let diag = DiagnosticBuilder::new(Severity::Warning)
                    .message(Text::from(format!(
                        "Context '{}' is sync-only but used in async function",
                        declared
                    )))
                    .help(Text::from("Consider using an async-compatible alternative"))
                    .build();
                diagnostics.push(diag);
            }
        }
    }
}

impl Default for EntryDetectionPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for EntryDetectionPhase {
    fn name(&self) -> &str {
        "Phase 0: Entry Point Detection"
    }

    fn description(&self) -> &str {
        "Detect main entry point and validate context usage"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        let modules = match &input.data {
            PhaseData::AstModules(modules) => modules,
            _ => {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message(Text::from("Invalid input for entry detection phase"))
                    .build();
                return Err(vec![diag].into());
            }
        };

        let main_config = self.detect_entry_point(modules)?;

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);

        match &main_config {
            MainConfig::Sync => {
                metrics.add_custom_metric("entry_type", "sync");
            }
            MainConfig::Async => {
                metrics.add_custom_metric("entry_type", "async");
            }
        }

        Ok(PhaseOutput {
            data: input.data, // Pass through AST modules
            warnings: List::new(),
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        false // Entry point detection must happen before other phases
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

/// Context validation state for entry point
#[derive(Debug, Clone, Default)]
pub struct EntryContextValidation {
    /// Contexts declared in main's 'using' clause
    pub declared_contexts: HashSet<Text>,

    /// Context groups expanded from nested groups
    pub expanded_groups: HashMap<Text, HashSet<Text>>,

    /// Contexts accessed in function body
    pub accessed_contexts: HashSet<Text>,

    /// Contexts with 'provide' statements
    pub provided_contexts: HashSet<Text>,

    /// Source spans for declared contexts (for error reporting)
    pub context_spans: HashMap<Text, verum_ast::Span>,
}

impl EntryContextValidation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a context is declared
    pub fn is_declared(&self, name: &Text) -> bool {
        self.declared_contexts.contains(name)
    }

    /// Check if a context is provided
    pub fn is_provided(&self, name: &Text) -> bool {
        self.provided_contexts.contains(name)
    }

    /// Get all undeclared but accessed contexts
    pub fn get_undeclared_accesses(&self) -> HashSet<Text> {
        self.accessed_contexts
            .difference(&self.declared_contexts)
            .cloned()
            .collect()
    }

    /// Get all declared but unprovided contexts
    pub fn get_unprovided_contexts(&self) -> HashSet<Text> {
        self.declared_contexts
            .difference(&self.provided_contexts)
            .cloned()
            .collect()
    }
}

/// Main function configuration
///
/// Main function configuration for context system integration.
///
/// Context system rules for main():
/// - NO automatic provision of contexts (auto-provide feature removed)
/// - ALL contexts must be explicitly declared with `using [...]`
/// - ALL contexts must be provided with `provide` statements
/// - main() may be sync or async; async main gets an implicit executor context
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainConfig {
    /// Synchronous main function
    Sync,

    /// Asynchronous main function
    Async,
}

impl MainConfig {
    /// Is this an async main function?
    pub fn is_async(&self) -> bool {
        matches!(self, MainConfig::Async)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_context_validation_new() {
        let validation = EntryContextValidation::new();
        assert!(validation.declared_contexts.is_empty());
        assert!(validation.accessed_contexts.is_empty());
        assert!(validation.provided_contexts.is_empty());
    }

    #[test]
    fn test_entry_context_validation_is_declared() {
        let mut validation = EntryContextValidation::new();
        validation.declared_contexts.insert(Text::from("Logger"));

        assert!(validation.is_declared(&Text::from("Logger")));
        assert!(!validation.is_declared(&Text::from("Database")));
    }

    #[test]
    fn test_get_undeclared_accesses() {
        let mut validation = EntryContextValidation::new();
        validation.declared_contexts.insert(Text::from("Logger"));
        validation.accessed_contexts.insert(Text::from("Logger"));
        validation.accessed_contexts.insert(Text::from("Database")); // Not declared

        let undeclared = validation.get_undeclared_accesses();
        assert!(undeclared.contains(&Text::from("Database")));
        assert!(!undeclared.contains(&Text::from("Logger")));
    }

    #[test]
    fn test_entry_detection_phase_new() {
        let phase = EntryDetectionPhase::new();

        // Check built-in contexts are registered
        assert!(phase.known_contexts.contains(&Text::from("Logger")));
        assert!(phase.known_contexts.contains(&Text::from("Database")));
        assert!(phase.known_contexts.contains(&Text::from("HttpClient")));

        // Check context groups are registered
        assert!(phase.context_groups.contains_key(&Text::from("WebContext")));
        assert!(
            phase
                .context_groups
                .contains_key(&Text::from("DatabaseContext"))
        );
    }

    #[test]
    fn test_register_context() {
        let mut phase = EntryDetectionPhase::new();
        phase.register_context(Text::from("CustomContext"));

        assert!(phase.known_contexts.contains(&Text::from("CustomContext")));
    }

    #[test]
    fn test_register_context_group() {
        let mut phase = EntryDetectionPhase::new();
        let mut contexts: HashSet<Text> = HashSet::new();
        contexts.insert(Text::from("Ctx1"));
        contexts.insert(Text::from("Ctx2"));

        phase.register_context_group(Text::from("MyGroup"), contexts);

        assert!(phase.context_groups.contains_key(&Text::from("MyGroup")));
        assert!(phase.context_groups[&Text::from("MyGroup")].contains(&Text::from("Ctx1")));
    }

    // P1.3: script-mode entry detection.
    // Helper to build a minimal Module with the given items + script-kind tag.

    fn fid() -> verum_ast::FileId {
        verum_ast::FileId::new(0)
    }

    fn dummy_span() -> verum_ast::Span {
        verum_ast::Span::new(0, 0, fid())
    }

    fn empty_block() -> verum_ast::expr::Block {
        verum_ast::expr::Block::empty(dummy_span())
    }

    fn build_fn(name: &str, is_async: bool) -> verum_ast::Item {
        let func = verum_ast::decl::FunctionDecl {
            visibility: verum_ast::decl::Visibility::Private,
            is_async,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: verum_common::Maybe::None,
            is_variadic: false,
            name: verum_ast::Ident::new(Text::from(name), dummy_span()),
            generics: List::new(),
            params: List::new(),
            return_type: verum_common::Maybe::None,
            throws_clause: verum_common::Maybe::None,
            std_attr: verum_common::Maybe::None,
            contexts: List::new(),
            generic_where_clause: verum_common::Maybe::None,
            meta_where_clause: verum_common::Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(empty_block())),
            span: dummy_span(),
        };
        verum_ast::Item::new(verum_ast::ItemKind::Function(func), dummy_span())
    }

    fn make_module(items: Vec<verum_ast::Item>, script: bool) -> Module {
        let items_list: List<verum_ast::Item> = items.into_iter().collect();
        let mut module = Module::new(items_list, fid(), dummy_span());
        if script {
            verum_ast::CogKind::Script.set_on_module(&mut module);
        }
        module
    }

    #[test]
    fn test_entry_script_main_used_when_no_main_in_script_module() {
        let phase = EntryDetectionPhase::new();
        let module = make_module(vec![build_fn("__verum_script_main", false)], true);
        let modules = vec![module];
        let cfg = phase
            .detect_entry_point(&modules)
            .expect("script entry should be discovered");
        assert!(matches!(cfg, MainConfig::Sync));
    }

    #[test]
    fn test_entry_script_main_ignored_in_non_script_module() {
        // A non-script module that happens to define __verum_script_main
        // (unlikely, but possible) MUST NOT be treated as the entry —
        // only modules tagged as Script can pin __verum_script_main.
        let phase = EntryDetectionPhase::new();
        let module = make_module(vec![build_fn("__verum_script_main", false)], false);
        let modules = vec![module];
        let result = phase.detect_entry_point(&modules);
        assert!(
            result.is_err(),
            "non-script module with __verum_script_main must still error \
             out without a real `main`"
        );
    }

    #[test]
    fn test_entry_main_takes_precedence_over_script_main() {
        // If a script-mode module ALSO declares an explicit `main`,
        // the explicit one wins — that lets users gradually migrate
        // a script to a regular main without a parser flip.
        let phase = EntryDetectionPhase::new();
        let module = make_module(
            vec![
                build_fn("__verum_script_main", false),
                build_fn("main", true), // async — distinguishes from script_main
            ],
            true,
        );
        let modules = vec![module];
        let cfg = phase.detect_entry_point(&modules).expect("entry found");
        // explicit `main` is async → MainConfig::Async
        assert!(matches!(cfg, MainConfig::Async));
    }

    #[test]
    fn test_entry_no_main_no_script_errors_clearly() {
        // Pure-decl library module without main — original behaviour
        // preserved: error with the "Add 'fn main()..." help.
        let phase = EntryDetectionPhase::new();
        let module = make_module(vec![build_fn("helper", false)], false);
        let modules = vec![module];
        let result = phase.detect_entry_point(&modules);
        assert!(result.is_err());
    }
}
