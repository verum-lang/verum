//! Macro / meta-function invocation collection + expansion.
//!
//! Extracted from `pipeline.rs` (#106 Phase 21). Houses the
//! AST-visitor that walks a module to find macro / meta-function
//! invocations and expands them via the meta registry.
//!
//! Surface:
//!
//!   * `InvocationArgs` — enum distinguishing traditional
//!     `macro!()` token-tree args from `@meta()` parsed-Expr args.
//!   * `MacroInvocation` — record for a single found invocation
//!     (name + args + span).
//!   * `MacroExpander<'a>` — visitor that holds the meta registry,
//!     a meta-execution context, the current module path, and a
//!     list of found invocations.
//!   * `Visitor for MacroExpander<'a>` — AST walk that records
//!     each `Macro` / `Meta` expression as a `MacroInvocation`.
//!   * `MacroExpander::collect_macro_invocations` /
//!     `MacroExpander::expand_macro` — driver methods.
//!
//! Plus: `reset_test_isolation()` — public test-harness helper
//! lives at the bottom of this concern (it resets the same global
//! tables the macro expander touches).

use anyhow::Result;

use verum_ast::Item;
use verum_ast::Span;
use verum_common::{List, Text};

use crate::meta::MetaRegistry;// ==================== MACRO EXPANSION ====================

/// Types of macro/meta function arguments
#[derive(Debug, Clone)]
enum InvocationArgs {
    /// Traditional macro args (unparsed token tree from macro!())
    MacroArgs(verum_ast::expr::MacroArgs),
    /// Meta function args (parsed expressions from @meta())
    MetaArgs(List<verum_ast::expr::Expr>),
}

/// A macro or meta function invocation found in the AST
#[derive(Debug, Clone)]
struct MacroInvocation {
    /// Name of the macro/meta function being invoked
    macro_name: Text,
    /// Arguments to the invocation
    args: InvocationArgs,
    /// Span of the invocation
    span: Span,
}

/// Visitor that collects and expands macro invocations
struct MacroExpander<'a> {
    /// Reference to the meta registry
    registry: &'a MetaRegistry,
    /// Meta execution context
    context: crate::meta::MetaContext,
    /// Current module path
    module_path: Text,
    /// Collected macro invocations
    expansions: List<MacroInvocation>,
}

impl<'a> MacroExpander<'a> {
    /// Collect macro invocations from an item
    fn collect_macro_invocations(&mut self, item: &Item) {
        use verum_ast::visitor::Visitor;
        self.visit_item(item);
    }

    /// Expand a macro or meta function invocation
    fn expand_macro(&mut self, invocation: &MacroInvocation) -> Result<List<Item>> {
        use crate::meta::ConstValue;

        let module_path_std = Text::from(self.module_path.as_str());
        let macro_name_text = Text::from(invocation.macro_name.as_str());


        // Try to resolve as a user-defined meta function first (@meta_name())
        // This handles functions declared with `meta fn name() -> TokenStream { ... }`
        if let Some(meta_fn) = self.registry.resolve_meta_call(&module_path_std, &macro_name_text) {
            debug!(
                "Expanding user-defined meta function '@{}'",
                invocation.macro_name.as_str()
            );

            // Convert args to ConstValue based on the invocation type
            let args = match &invocation.args {
                InvocationArgs::MetaArgs(exprs) => {
                    // Meta function calls have parsed expressions as arguments
                    // For now, we convert them to ConstValue::Expr for the meta function to use
                    exprs.iter()
                        .map(|e| ConstValue::Expr(e.clone()))
                        .collect::<Vec<_>>()
                }
                InvocationArgs::MacroArgs(macro_args) => {
                    // Traditional macro calls have unparsed token trees
                    vec![ConstValue::Text(macro_args.tokens.clone())]
                }
            };

            // Execute the meta function
            let result = self
                .context
                .execute_user_meta_fn(&meta_fn, args)
                .map_err(|e| {
                    anyhow::anyhow!("Meta function execution failed: {}", e)
                })?;

            // For expression-level meta functions, just return empty items
            // The expansion should happen inline where the expression was
            // For now, we log the result and return empty
            debug!(
                "Meta function '{}' returned: {:?}",
                invocation.macro_name.as_str(),
                result.type_name()
            );

            // Return empty items - the expansion is used inline, not as new items
            return Ok(List::new());
        }

        // Otherwise, try to resolve as a traditional macro (macro!())
        let macro_def = match self
            .registry
            .resolve_macro(&module_path_std, &macro_name_text)
        {
            Maybe::Some(def) => def,
            Maybe::None => {
                // Neither a meta function nor a macro was found
                // This might be a built-in meta function like @cfg, @const, etc.
                // Those are handled elsewhere, so we skip them here
                debug!(
                    "Skipping unknown/built-in meta function: {}",
                    invocation.macro_name.as_str()
                );
                return Ok(List::new());
            }
        };

        debug!(
            "Expanding macro '{}' using expander '{}'",
            invocation.macro_name.as_str(),
            macro_def.expander.as_str()
        );

        // Look up the expander meta function
        let meta_fn = match self
            .registry
            .resolve_meta_call(&macro_def.module, &macro_def.expander)
        {
            Some(func) => func,
            None => {
                return Err(anyhow::anyhow!(
                    "Meta function '{}' not found for macro expansion",
                    macro_def.expander.as_str()
                ));
            }
        };

        // Convert macro arguments to ConstValue
        let args = match &invocation.args {
            InvocationArgs::MacroArgs(macro_args) => {
                vec![ConstValue::Text(macro_args.tokens.clone())]
            }
            InvocationArgs::MetaArgs(exprs) => {
                vec![ConstValue::Text(format!("{:?}", exprs).into())]
            }
        };

        // Execute the meta function
        let result = self
            .context
            .execute_user_meta_fn(&meta_fn, args)
            .map_err(|e| anyhow::anyhow!("Meta function execution failed: {}", e))?;

        // Convert result back to AST items
        // The result should be ConstValue::Items(List<ConstValue::Expr>)
        match result {
            ConstValue::Items(items) => {
                // Convert items to AST items by parsing each ConstValue
                debug!("Generated {} items from macro expansion", items.len());

                let mut ast_items = List::new();
                for (idx, const_val) in items.iter().enumerate() {
                    match const_val {
                        ConstValue::Expr(expr) => {
                            // Convert the expression to a token stream, then parse as an item
                            use crate::quote::ToTokens;
                            let token_stream = expr.into_token_stream();

                            match token_stream.parse_as_item() {
                                Ok(item) => {
                                    ast_items.push(item);
                                }
                                Err(e) => {
                                    return Err(anyhow::anyhow!(
                                        "Failed to parse item {} from macro expansion: {}",
                                        idx,
                                        e
                                    ));
                                }
                            }
                        }
                        ConstValue::Text(code) => {
                            // Parse text as an item directly
                            let file_id = invocation.span.file_id;
                            match crate::quote::TokenStream::from_str(code.as_str(), file_id) {
                                Ok(token_stream) => match token_stream.parse_as_item() {
                                    Ok(item) => {
                                        ast_items.push(item);
                                    }
                                    Err(e) => {
                                        return Err(anyhow::anyhow!(
                                            "Failed to parse text item {} from macro expansion: {}",
                                            idx,
                                            e
                                        ));
                                    }
                                },
                                Err(e) => {
                                    return Err(anyhow::anyhow!(
                                        "Failed to tokenize text item {} from macro expansion: {}",
                                        idx,
                                        e
                                    ));
                                }
                            }
                        }
                        _ => {
                            return Err(anyhow::anyhow!(
                                "Invalid item type in macro expansion at index {}: expected Expr or Text, found {}",
                                idx,
                                const_val.type_name()
                            ));
                        }
                    }
                }

                debug!(
                    "Successfully converted {} items from macro expansion",
                    ast_items.len()
                );
                Ok(ast_items)
            }
            ConstValue::Expr(expr) => {
                // Single expression - try to parse it as an item
                debug!("Generated single expression from macro expansion");

                use crate::quote::ToTokens;
                let token_stream = expr.into_token_stream();

                match token_stream.parse_as_item() {
                    Ok(item) => {
                        let mut result_list = List::new();
                        result_list.push(item);
                        debug!("Successfully parsed expression as item");
                        Ok(result_list)
                    }
                    Err(e) => {
                        // If it can't be parsed as an item, it might be meant for expression context
                        // In that case, we should probably error out since we're in item context
                        Err(anyhow::anyhow!(
                            "Macro expansion returned an expression that cannot be used as an item: {}",
                            e
                        ))
                    }
                }
            }
            ConstValue::Text(code) => {
                // Text result - parse it as code
                debug!("Generated text from macro expansion, parsing as items");

                let file_id = invocation.span.file_id;
                match crate::quote::TokenStream::from_str(code.as_str(), file_id) {
                    Ok(token_stream) => {
                        // Try to parse as a single item
                        match token_stream.parse_as_item() {
                            Ok(item) => {
                                let mut result_list = List::new();
                                result_list.push(item);
                                debug!("Successfully parsed text as item");
                                Ok(result_list)
                            }
                            Err(e) => Err(anyhow::anyhow!(
                                "Failed to parse generated text as item: {}",
                                e
                            )),
                        }
                    }
                    Err(e) => Err(anyhow::anyhow!("Failed to tokenize generated text: {}", e)),
                }
            }
            _ => Err(anyhow::anyhow!(
                "Macro expansion returned unexpected type: {}. Expected Items, Expr, or Text",
                result.type_name()
            )),
        }
    }
}

impl<'a> verum_ast::visitor::Visitor for MacroExpander<'a> {
    fn visit_expr(&mut self, expr: &verum_ast::expr::Expr) {
        use verum_ast::expr::ExprKind;
        use verum_ast::visitor::walk_expr;

        match &expr.kind {
            // Check if this is a traditional macro call (name!())
            ExprKind::MacroCall { path, args } => {
                // Extract macro name from path
                if let Some(ident) = path.as_ident() {
                    let macro_name = Text::from(ident.as_str());

                    debug!("Found macro invocation: {}", macro_name.as_str());

                    // Record this invocation
                    self.expansions.push(MacroInvocation {
                        macro_name,
                        args: InvocationArgs::MacroArgs(args.clone()),
                        span: expr.span,
                    });
                }
            }

            // Check if this is a meta function call (@name())
            // User-defined meta functions use this syntax
            ExprKind::MetaFunction { name, args } => {
                let meta_name = Text::from(name.name.as_str());

                debug!("Found meta function invocation: @{}", meta_name.as_str());

                // Record this invocation for expansion
                // Note: We check if this is a user-defined meta function in expand_macro
                self.expansions.push(MacroInvocation {
                    macro_name: meta_name,
                    args: InvocationArgs::MetaArgs(args.clone()),
                    span: expr.span,
                });
            }

            _ => {}
        }

        // Continue walking
        walk_expr(self, expr);
    }
}
