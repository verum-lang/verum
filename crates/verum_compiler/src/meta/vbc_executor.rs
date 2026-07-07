//! VBC Executor for Meta Functions
//!

//! This module provides VBC-based execution for meta functions during staged
//! metaprogramming. It integrates the VBC codegen and interpreter to execute
//! meta functions that generate code at compile-time.
//!

//! ## Architecture
//!

//! ```text
//! MetaFunction (AST body)
//!  ↓
//! Synthetic Module (wraps body in function)
//!  ↓
//! VbcCodegen → VbcModule (bytecode)
//!  ↓
//! Interpreter → Value (NaN-boxed result)
//!  ↓
//! TokenStream extraction
//!  ↓
//! Parse → AST fragments
//! ```
//!

//! ## Performance Targets
//!

//! - Meta function startup: < 1ms (Tier 0 interpreter)
//! - Token stream extraction: < 100μs
//! - Code generation throughput: > 10K tokens/sec
//!

//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use std::sync::Arc;

use tracing::{debug, trace, warn};
use verum_ast::{
    FileId, Item, Module, Span,
    decl::{FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, ItemKind, Visibility},
    expr::{Block, Expr, ExprKind},
    pattern::{Pattern, PatternKind},
    ty::{Ident, Path, PathSegment, Type, TypeKind},
};
use verum_common::{Heap, List, Map, Maybe, Text};
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
    module::FunctionId,
    value::Value,
};

use super::registry::MetaFunction;
use crate::quote::{ParseError, TokenStream};

/// Errors that can occur during VBC execution of meta functions.
#[derive(Debug, Clone)]
pub enum VbcExecutionError {
    /// Failed to compile meta function to VBC.
    CompilationFailed { function_name: Text, error: Text },
    /// Failed to execute the VBC bytecode.
    ExecutionFailed { function_name: Text, error: Text },
    /// The meta function didn't return a TokenStream.
    InvalidReturnType {
        function_name: Text,
        expected: Text,
        got: Text,
    },
    /// Failed to parse the generated token stream.
    ParseFailed {
        function_name: Text,
        error: ParseError,
    },
    /// Function not found in compiled module.
    FunctionNotFound { function_name: Text },
}

impl std::fmt::Display for VbcExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VbcExecutionError::CompilationFailed {
                function_name,
                error,
            } => {
                write!(
                    f,
                    "Failed to compile meta function '{}': {}",
                    function_name, error
                )
            }
            VbcExecutionError::ExecutionFailed {
                function_name,
                error,
            } => {
                write!(
                    f,
                    "Failed to execute meta function '{}': {}",
                    function_name, error
                )
            }
            VbcExecutionError::InvalidReturnType {
                function_name,
                expected,
                got,
            } => {
                write!(
                    f,
                    "Meta function '{}' returned {} but expected {}",
                    function_name, got, expected
                )
            }
            VbcExecutionError::ParseFailed {
                function_name,
                error,
            } => {
                write!(
                    f,
                    "Failed to parse output of meta function '{}': {:?}",
                    function_name, error
                )
            }
            VbcExecutionError::FunctionNotFound { function_name } => {
                write!(
                    f,
                    "Meta function '{}' not found in compiled module",
                    function_name
                )
            }
        }
    }
}

impl std::error::Error for VbcExecutionError {}

/// Result type for VBC execution.
pub type VbcExecutionResult<T> = std::result::Result<T, VbcExecutionError>;

/// Result of a raw (value-level) meta function execution.
///

/// Produced by [`VbcExecutor::execute_raw`]. Carries the NaN-boxed result
/// [`Value`] together with the [`Interpreter`] that produced it: heap-backed
/// results (Text, List, Map, records) are only decodable while the
/// interpreter — and therefore its heap — is still alive. Callers decode
/// through `interpreter.state` (e.g. `read_text` / `list_elements` /
/// `map_entries`).
pub struct RawExecution {
    /// The meta function's return value (NaN-boxed; may point into the heap).
    pub value: Value,
    /// The interpreter that executed the function, kept alive for heap access.
    pub interpreter: Interpreter,
}

/// VBC executor for meta functions.
///

/// Compiles meta functions to VBC bytecode and executes them using the
/// VBC interpreter to generate code at compile-time.
pub struct VbcExecutor {
    /// Configuration for codegen.
    config: CodegenConfig,
    /// Synthetic file ID for generated modules.
    synthetic_file_id: FileId,
    /// Cache of compiled modules (keyed by function signature).
    module_cache: Map<Text, Arc<VbcModule>>,
}

impl VbcExecutor {
    /// Creates a new VBC executor.
    pub fn new() -> Self {
        Self {
            config: CodegenConfig::default(),
            synthetic_file_id: FileId::dummy(),
            module_cache: Map::new(),
        }
    }

    /// Creates a VBC executor with custom configuration.
    pub fn with_config(config: CodegenConfig) -> Self {
        Self {
            config,
            synthetic_file_id: FileId::dummy(),
            module_cache: Map::new(),
        }
    }

    /// Executes a meta function and returns the generated TokenStream.
    ///

    /// # Arguments
    ///

    /// * `meta_func` - The meta function to execute
    /// * `args` - Arguments to pass to the function (as VBC Values)
    ///

    /// # Returns
    ///

    /// The TokenStream generated by the meta function.
    pub fn execute(
        &mut self,
        meta_func: &MetaFunction,
        args: &[Value],
    ) -> VbcExecutionResult<TokenStream> {
        let func_name = &meta_func.name;
        debug!("Executing meta function '{}' via VBC", func_name);

        // Step 1: Create synthetic module containing the meta function.
        // The main wrapper is typed as returning TokenStream — the historical
        // (and for `execute()` correct) contract of this entry point.
        let wrapper_return = Self::token_stream_type(meta_func.span);
        let synthetic_module = self.create_synthetic_module(meta_func, wrapper_return);

        // Step 2: Compile to VBC
        let vbc_module = self.compile_module(&synthetic_module, func_name, func_name.clone())?;

        // Step 3: Execute with interpreter
        let (result, interpreter) = self.execute_module(vbc_module, func_name, args)?;

        // Step 4: Extract TokenStream from result
        self.extract_token_stream(result, &interpreter, func_name)
    }

    /// Executes a meta function and returns its raw result [`Value`].
    ///

    /// Unlike [`execute`](Self::execute), the synthetic `main` wrapper is
    /// typed with the meta function's *own* declared return type instead of
    /// the hardcoded `TokenStream`, and the result is returned as-is — no
    /// TokenStream extraction. This is the entry point for value-level
    /// (pure-computation) meta functions: with the TokenStream-typed wrapper
    /// their scalar results were coerced to Unit on the raw path.
    ///

    /// The returned [`RawExecution`] carries the interpreter so heap-backed
    /// results (Text, collections) remain decodable by the caller.
    ///

    /// Raw and TokenStream executions of the same function are cached under
    /// distinct keys — the two synthetic modules differ in wrapper typing.
    pub fn execute_raw(
        &mut self,
        meta_func: &MetaFunction,
        args: &[Value],
    ) -> VbcExecutionResult<RawExecution> {
        let func_name = &meta_func.name;
        debug!("Executing meta function '{}' via VBC (raw value mode)", func_name);

        // Step 1: Synthetic module whose main wrapper is typed with the meta
        // function's own return type (NOT TokenStream).
        let synthetic_module =
            self.create_synthetic_module(meta_func, meta_func.return_type.clone());

        // Step 2: Compile to VBC under a raw-mode cache key so the
        // TokenStream-typed module for the same function is never reused.
        let cache_key = Text::from(format!("{}::__raw", func_name));
        let vbc_module = self.compile_module(&synthetic_module, func_name, cache_key)?;

        // Step 3: Execute with interpreter; skip TokenStream extraction.
        let (value, interpreter) = self.execute_module_raw(vbc_module, func_name, args)?;

        Ok(RawExecution { value, interpreter })
    }

    /// Executes the compiled raw-mode module and returns the result value
    /// and the interpreter (for heap decoding).
    ///

    /// Differs from [`execute_module`](Self::execute_module) in two ways,
    /// both required for correct value-level execution:
    ///

    /// * The entry function is resolved **by name** (the synthetic `main`
    ///   wrapper, falling back to the meta function itself) — codegen
    ///   registers stdlib/intrinsic functions first, so `FunctionId(0)` is
    ///   not in general the wrapper.
    /// * Arguments are passed through
    ///   [`Interpreter::execute_function_with_args`] — the host-facing
    ///   fresh-entry path. `Interpreter::call` is the re-entrant path for
    ///   use *while already executing bytecode*; calling it host-side
    ///   panics on frame-base accounting ("Invalid frame base").
    fn execute_module_raw(
        &self,
        vbc_module: Arc<VbcModule>,
        func_name: &Text,
        args: &[Value],
    ) -> VbcExecutionResult<(Value, Interpreter)> {
        // Resolve the entry point by name. Functions may be registered
        // fully-qualified (e.g. "<module>.main"), so fall back to the
        // unique-bare-suffix lookup, then to the meta function itself.
        let entry_id = vbc_module
            .find_function_by_name("main")
            .or_else(|| vbc_module.find_function_by_unique_bare_suffix("main"))
            .or_else(|| vbc_module.find_function_by_name(func_name.as_str()))
            .or_else(|| vbc_module.find_function_by_unique_bare_suffix(func_name.as_str()))
            .ok_or_else(|| VbcExecutionError::FunctionNotFound {
                function_name: func_name.clone(),
            })?;

        let mut interpreter = Interpreter::new(vbc_module);
        let result = interpreter.execute_function_with_args(entry_id, args);

        result
            .map(|v| (v, interpreter))
            .map_err(|e| VbcExecutionError::ExecutionFailed {
                function_name: func_name.clone(),
                error: Text::from(format!("{:?}", e)),
            })
    }

    /// Creates a synthetic AST module wrapping the meta function.
    ///

    /// The synthetic module contains:
    /// 1. A main wrapper function that calls the meta function
    /// 2. The original meta function body
    ///

    /// `wrapper_return` is the declared return type of the synthetic main
    /// wrapper: `TokenStream` for [`execute`](Self::execute), the meta
    /// function's own return type for [`execute_raw`](Self::execute_raw).
    fn create_synthetic_module(&self, meta_func: &MetaFunction, wrapper_return: Type) -> Module {
        // Create the function declaration from the MetaFunction
        let func_decl = self.meta_func_to_decl(meta_func);

        // Create a main function that just calls the meta function
        // This is the entry point for the interpreter
        let main_func = self.create_main_wrapper(
            &meta_func.name,
            &meta_func.params,
            meta_func.span,
            wrapper_return,
        );

        // Build the module with both functions
        let items: List<Item> = vec![
            Item {
                kind: ItemKind::Function(func_decl),
                attributes: List::new(),
                span: meta_func.span,
            },
            Item {
                kind: ItemKind::Function(main_func),
                attributes: List::new(),
                span: meta_func.span,
            },
        ]
        .into_iter()
        .collect();

        Module::new(items, self.synthetic_file_id, meta_func.span)
    }

    /// Converts a MetaFunction to a FunctionDecl.
    fn meta_func_to_decl(&self, meta_func: &MetaFunction) -> FunctionDecl {
        let span = meta_func.span;

        // Convert params
        let params: List<FunctionParam> = meta_func
            .params
            .iter()
            .map(|p| FunctionParam {
                kind: FunctionParamKind::Regular {
                    pattern: Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: Ident::new(p.name.to_string(), span),
                            subpattern: Maybe::None,
                        },
                        span,
                    },
                    ty: p.ty.clone(),
                    default_value: Maybe::None,
                },
                attributes: List::new(),
                span,
            })
            .collect();

        // Create function body as a block containing the expression
        let body_block = Block {
            stmts: List::new(),
            expr: Maybe::Some(Heap::new(meta_func.body.clone())),
            span: meta_func.body.span,
        };

        FunctionDecl {
            visibility: Visibility::Private,
            is_async: meta_func.is_async,
            is_meta: false, // Runtime equivalent for VBC execution
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: meta_func.is_transparent,
            extern_abi: Maybe::None,
            is_variadic: false,
            name: Ident::new(meta_func.name.to_string(), span),
            generics: List::new(),
            params,
            return_type: Maybe::Some(meta_func.return_type.clone()),
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: meta_func.contexts.clone(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Block(body_block)),
            span,
        }
    }

    /// The `TokenStream` type node used as the wrapper return type on the
    /// classic [`execute`](Self::execute) path.
    fn token_stream_type(span: Span) -> Type {
        Type {
            kind: TypeKind::Path(Path {
                segments: vec![PathSegment::Name(Ident::new(
                    "TokenStream".to_string(),
                    span,
                ))]
                .into_iter()
                .collect(),
                span,
            }),
            span,
        }
    }

    /// Creates a main wrapper function that calls the meta function.
    ///

    /// `return_type` is the wrapper's declared return type. It is a
    /// parameter (rather than hardcoded `TokenStream`) so that value-level
    /// executions can type the wrapper with the meta function's own return
    /// type — otherwise pure-value results coerce to Unit on the raw path.
    fn create_main_wrapper(
        &self,
        func_name: &Text,
        params: &List<super::registry::MetaParam>,
        span: Span,
        return_type: Type,
    ) -> FunctionDecl {
        // Build the call expression: func_name(arg0, arg1, ...)
        let call_args: List<Expr> = params
            .iter()
            .enumerate()
            .map(|(i, _)| {
                // Reference the parameter by name (will be passed as arguments to main)
                let arg_name = format!("__arg{}", i);
                Expr::new(
                    ExprKind::Path(Path {
                        segments: vec![PathSegment::Name(Ident::new(arg_name, span))]
                            .into_iter()
                            .collect(),
                        span,
                    }),
                    span,
                )
            })
            .collect();

        let call_expr = Expr::new(
            ExprKind::Call {
                func: Heap::new(Expr::new(
                    ExprKind::Path(Path {
                        segments: vec![PathSegment::Name(Ident::new(func_name.to_string(), span))]
                            .into_iter()
                            .collect(),
                        span,
                    }),
                    span,
                )),
                type_args: List::new(),
                args: call_args,
            },
            span,
        );

        // Create parameters for main (matching the meta function's params)
        let main_params: List<FunctionParam> = params
            .iter()
            .enumerate()
            .map(|(i, p)| FunctionParam {
                kind: FunctionParamKind::Regular {
                    pattern: Pattern {
                        kind: PatternKind::Ident {
                            by_ref: false,
                            mutable: false,
                            name: Ident::new(format!("__arg{}", i), span),
                            subpattern: Maybe::None,
                        },
                        span,
                    },
                    ty: p.ty.clone(),
                    default_value: Maybe::None,
                },
                attributes: List::new(),
                span,
            })
            .collect();

        let body_block = Block {
            stmts: List::new(),
            expr: Maybe::Some(Heap::new(call_expr)),
            span,
        };

        // Return type is supplied by the caller (TokenStream on the classic
        // path, the meta function's own return type on the raw path).
        FunctionDecl {
            visibility: Visibility::Private,
            is_async: false,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            name: Ident::new("main".to_string(), span),
            generics: List::new(),
            params: main_params,
            return_type: Maybe::Some(return_type),
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Block(body_block)),
            span,
        }
    }

    /// Compiles the synthetic module to VBC bytecode.
    ///

    /// `cache_key` identifies the compiled artifact in the module cache.
    /// It equals the function name on the classic TokenStream path and a
    /// `"<name>::__raw"` variant on the raw path — the two synthetic modules
    /// differ (wrapper return type), so they must never share a cache slot.
    fn compile_module(
        &mut self,
        module: &Module,
        func_name: &Text,
        cache_key: Text,
    ) -> VbcExecutionResult<Arc<VbcModule>> {
        // Check cache first
        if let Some(cached) = self.module_cache.get(&cache_key) {
            trace!("Using cached VBC module for '{}'", cache_key);
            return Ok(cached.clone());
        }

        // Compile the module
        let mut codegen = VbcCodegen::with_config(self.config.clone());
        let vbc_module =
            codegen
                .compile_module(module)
                .map_err(|e| VbcExecutionError::CompilationFailed {
                    function_name: func_name.clone(),
                    error: Text::from(format!("{:?}", e)),
                })?;

        let module_arc = Arc::new(vbc_module);

        // Cache the compiled module
        self.module_cache.insert(cache_key, module_arc.clone());

        debug!(
            "Compiled meta function '{}' to VBC ({} bytes)",
            func_name,
            module_arc.bytecode.len()
        );

        Ok(module_arc)
    }

    /// Executes the compiled VBC module and returns the result and interpreter.
    ///

    /// The interpreter is returned so that the heap can be accessed for
    /// extracting TokenStream objects from the result value.
    fn execute_module(
        &self,
        vbc_module: Arc<VbcModule>,
        func_name: &Text,
        args: &[Value],
    ) -> VbcExecutionResult<(Value, Interpreter)> {
        // Create interpreter
        let mut interpreter = Interpreter::new(vbc_module);

        // Find the main function (function 0 by convention)
        let main_id = FunctionId(0);

        // Execute with arguments
        let result = if args.is_empty() {
            interpreter.execute_function(main_id)
        } else {
            interpreter.call(main_id, args)
        };

        result
            .map(|v| (v, interpreter))
            .map_err(|e| VbcExecutionError::ExecutionFailed {
                function_name: func_name.clone(),
                error: Text::from(format!("{:?}", e)),
            })
    }

    /// Extracts a TokenStream from the VBC Value result.
    ///

    /// The result value should be a pointer to a TokenStream object on the heap.
    /// This method extracts and reconstructs the TokenStream.
    ///

    /// # Architecture
    ///

    /// 1. Check if value.is_ptr() indicating a heap object
    /// 2. Get the Object from the interpreter's heap
    /// 3. Deserialize the TokenStream data using verum_vbc::token_stream
    /// 4. Convert verum_lexer::Token list to compiler TokenStream
    fn extract_token_stream(
        &self,
        value: Value,
        interpreter: &Interpreter,
        func_name: &Text,
    ) -> VbcExecutionResult<TokenStream> {
        // Handle unit value (empty result)
        if value.is_unit() {
            trace!("Meta function '{}' returned unit", func_name);
            return Ok(TokenStream::new());
        }

        // Check for pointer type (heap object)
        if value.is_ptr() {
            let ptr = value.as_ptr::<u8>();
            if ptr.is_null() {
                trace!("Meta function '{}' returned null pointer", func_name);
                return Ok(TokenStream::new());
            }

            // Get the Object from the heap
            let obj = interpreter.state.heap.get_object(ptr);
            match obj {
                Some(obj) => {
                    // Extract TokenStream data from the object
                    match verum_vbc::token_stream::extract_token_stream_from_object(&obj) {
                        Ok((tokens, span)) => {
                            // Convert to compiler TokenStream
                            let token_list: List<verum_lexer::Token> = tokens.into_iter().collect();
                            let mut ts = TokenStream::from_tokens(token_list);
                            if let Some(s) = span {
                                ts = ts.with_span(s);
                            }
                            debug!(
                                "Extracted TokenStream with {} tokens from meta function '{}'",
                                ts.len(),
                                func_name
                            );
                            return Ok(ts);
                        }
                        Err(e) => {
                            return Err(VbcExecutionError::ExecutionFailed {
                                function_name: func_name.clone(),
                                error: Text::from(format!("Failed to extract TokenStream: {}", e)),
                            });
                        }
                    }
                }
                None => {
                    // Pointer doesn't point to a valid heap object
                    warn!(
                        "Meta function '{}' returned invalid pointer {:p}",
                        func_name, ptr
                    );
                    return Ok(TokenStream::new());
                }
            }
        }

        // Handle integer result (could be error code or special value)
        if value.is_int() {
            let i = value.as_i64();
            if i == 0 {
                // Success with empty result
                return Ok(TokenStream::new());
            }
            // Non-zero could indicate an error
            trace!("Meta function '{}' returned integer: {}", func_name, i);
        }

        // For other types, return empty TokenStream with warning
        warn!(
            "Meta function '{}' returned unexpected type, returning empty TokenStream",
            func_name
        );
        Ok(TokenStream::new())
    }

    /// Clears the module cache.
    pub fn clear_cache(&mut self) {
        self.module_cache.clear();
    }

    /// Returns cache statistics.
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.module_cache.len(), 0) // (entries, estimated bytes)
    }
}

impl Default for VbcExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vbc_executor_new() {
        let executor = VbcExecutor::new();
        assert_eq!(executor.cache_stats().0, 0);
    }

    #[test]
    fn test_vbc_executor_clear_cache() {
        let mut executor = VbcExecutor::new();
        executor.clear_cache();
        assert_eq!(executor.cache_stats().0, 0);
    }
}
