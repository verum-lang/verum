//! Execution pipeline: Parse → Codegen → Execute.
//!
//! This module orchestrates the transformation of Verum source code into
//! executed results via the VBC interpreter.

use std::sync::Arc;
use std::time::{Duration, Instant};

use verum_common::{List, Text};
use verum_ast::FileId;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionId, VbcModule};
use verum_vbc::value::Value;

use crate::{IncrementalScriptParser, ScriptParseResult};

use super::context::{ExecutionContext, FunctionInfo};
use super::value_format::{format_value_with_type, ValueDisplayOptions};
use crate::CellId;

/// Result type for execution operations.
pub type ExecutionResult<T> = Result<T, ExecutionError>;

/// Result of parsing source to module.
/// Contains: (module, result_type, new_bindings, new_functions)
type ParseToModuleResult = (verum_ast::Module, Text, Vec<(Text, Text)>, Vec<Text>);

/// Errors that can occur during execution.
#[derive(Debug, Clone)]
pub enum ExecutionError {
    /// Parse error.
    Parse(Vec<String>),
    /// Codegen error.
    Codegen(String),
    /// Runtime error.
    Runtime(String),
    /// Type error.
    Type(String),
    /// Invalid cell state.
    InvalidState(String),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionError::Parse(errors) => {
                write!(f, "Parse error: {}", errors.join("; "))
            }
            ExecutionError::Codegen(msg) => write!(f, "Codegen error: {}", msg),
            ExecutionError::Runtime(msg) => write!(f, "Runtime error: {}", msg),
            ExecutionError::Type(msg) => write!(f, "Type error: {}", msg),
            ExecutionError::InvalidState(msg) => write!(f, "Invalid state: {}", msg),
        }
    }
}

impl std::error::Error for ExecutionError {}

/// A compiled cell ready for execution.
#[derive(Debug)]
pub struct CompiledCell {
    /// The VBC module containing the compiled code.
    pub module: Arc<VbcModule>,
    /// Entry function ID.
    pub entry_func: FunctionId,
    /// New bindings introduced by this cell (name, type).
    pub new_bindings: Vec<(Text, Text)>,
    /// Functions defined by this cell.
    pub new_functions: Vec<Text>,
    /// Result type (for display).
    pub result_type: Text,
    /// Compilation time.
    pub compile_time: Duration,
    /// Whether this cell defines a `main` function.
    pub has_main: bool,
    /// Variable → type-name mappings from VBC codegen context.
    /// Accurate for all types including generics (e.g., "List<Int>").
    pub codegen_type_names: std::collections::HashMap<String, String>,
}

/// Output from executing a cell.
#[derive(Debug, Clone)]
pub struct CellExecutionOutput {
    /// The resulting value (if any).
    pub value: Option<Value>,
    /// Formatted display representation.
    pub display: Text,
    /// Type information.
    pub type_info: Text,
    /// Captured stdout.
    pub stdout: Text,
    /// Captured stderr.
    pub stderr: Text,
    /// Execution time (wall clock).
    pub execution_time: Duration,
    /// Number of VBC instructions executed (0 if counting disabled).
    pub instructions_executed: u64,
    /// Peak stack depth reached during execution.
    pub peak_stack_depth: usize,
}

/// The execution pipeline.
///
/// Manages the flow from source code to executed results:
/// 1. Parse source using `IncrementalScriptParser`
/// 2. Compile AST to VBC using `VbcCodegen`
/// 3. Execute VBC using `Interpreter`
/// 4. Format results for display
///
/// Format an AST TypeKind as a readable string (used by TypeChecker integration).
fn format_ast_type(kind: &verum_ast::TypeKind) -> String {
    match kind {
        verum_ast::TypeKind::Path(path) => {
            let segs: Vec<String> = path.segments.iter().filter_map(|s| {
                if let verum_ast::ty::PathSegment::Name(ident) = s {
                    Some(ident.as_str().to_string())
                } else { None }
            }).collect();
            segs.join(".")
        }
        verum_ast::TypeKind::Bool => "Bool".into(),
        verum_ast::TypeKind::Int => "Int".into(),
        verum_ast::TypeKind::Float => "Float".into(),
        verum_ast::TypeKind::Char => "Char".into(),
        verum_ast::TypeKind::Text => "Text".into(),
        verum_ast::TypeKind::Unit => "()".into(),
        _ => format!("{:?}", kind),
    }
}

pub struct ExecutionPipeline {
    /// Parser for incremental parsing.
    parser: IncrementalScriptParser,
    /// File ID for parsing.
    file_id: FileId,
    /// Compiled cell cache: cell_id → CompiledCell.
    cell_cache: std::collections::HashMap<u64, CompiledCell>,
    /// Module ID counter for unique module names.
    module_counter: u32,
    /// Display options for value formatting.
    display_options: ValueDisplayOptions,
}

impl Default for ExecutionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionPipeline {
    /// Creates a new execution pipeline.
    pub fn new() -> Self {
        Self {
            parser: IncrementalScriptParser::new(),
            file_id: FileId::new(1),
            cell_cache: std::collections::HashMap::new(),
            module_counter: 0,
            display_options: ValueDisplayOptions::default(),
        }
    }

    /// Creates a new pipeline with a specific file ID.
    pub fn with_file_id(file_id: FileId) -> Self {
        Self {
            parser: IncrementalScriptParser::new(),
            file_id,
            cell_cache: std::collections::HashMap::new(),
            module_counter: 0,
            display_options: ValueDisplayOptions::default(),
        }
    }

    /// Sets display options for value formatting.
    pub fn set_display_options(&mut self, options: ValueDisplayOptions) {
        self.display_options = options;
    }

    /// Parses and compiles a cell's source code.
    pub fn compile(&mut self, source: &str, line_number: usize) -> ExecutionResult<CompiledCell> {
        let start = Instant::now();

        // Parse the source
        let parse_result = self
            .parser
            .parse_line(source, line_number, self.file_id)
            .map_err(|errors| {
                ExecutionError::Parse(errors.iter().map(|e| format!("{}", e)).collect())
            })?;

        // Generate unique module name
        self.module_counter += 1;
        let module_name = format!("cell_{}", self.module_counter);

        // Convert parse result to AST module for codegen
        let (ast_module, result_type, new_bindings, new_functions) =
            self.parse_result_to_module(parse_result, &module_name)?;


        // ── Type Check via verum_types::TypeChecker ────────────────────
        let mut inferred_types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        {
            let mut checker = verum_types::TypeChecker::with_minimal_context();
            checker.register_builtins();
            for item in ast_module.items.iter() {
                let _ = checker.check_item(item); // collect errors silently
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    // Extract let-binding types from function body
                    if let Some(verum_ast::decl::FunctionBody::Block(block)) = func.body.as_ref() {
                        for stmt in block.stmts.iter() {
                            if let verum_ast::StmtKind::Let { pattern, ty, value, .. } = &stmt.kind
                                && let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind {
                                    let name_str = name.as_str().to_string();
                                    // Try type annotation first
                                    inferred_types.entry(name_str.clone()).or_insert_with(|| {
                                        ty.as_ref().map(|t| format_ast_type(&t.kind)).unwrap_or_else(|| "<inferred>".to_string())
                                    });
                                    // Try TypeChecker synth_expr on initializer
                                    if let Some(init) = value
                                        && let Ok(result) = checker.synth_expr(init) {
                                            let ty_str = format!("{}", result.ty);
                                            if ty_str != "()" && ty_str != "_" {
                                                inferred_types.insert(name_str, ty_str);
                                            }
                                        }
                                }
                        }
                        // Infer tail expression type
                        if let verum_common::Maybe::Some(tail) = &block.expr
                            && let Ok(result) = checker.synth_expr(tail) {
                                inferred_types.insert("__return__".to_string(), format!("{}", result.ty));
                            }
                    }
                }
            }
        }

        // Compile to VBC
        let config = CodegenConfig::new(&module_name);
        let mut codegen = VbcCodegen::with_config(config);

        let vbc_module = codegen
            .compile_module(&ast_module)
            .map_err(|e| ExecutionError::Codegen(format!("{:?}", e)))?;

        // Extract variable type names from codegen context.
        let mut codegen_type_names: std::collections::HashMap<String, String> =
            codegen.variable_type_names().clone();

        // Merge: TypeChecker types (authoritative) override codegen types
        for (k, v) in inferred_types {
            codegen_type_names.insert(k, v);
        }

        let compile_time = start.elapsed();

        let has_main = new_functions.iter().any(|n| n.as_str() == "main");

        let vbc_module = Arc::new(vbc_module);

        // Find the actual entry point — "main" function in the compiled module.
        // VbcCodegen prepends standard library stubs, so FunctionId(0) is NOT
        // necessarily the user's code.
        let entry_func = self
            .find_function_by_name(&vbc_module, "main")
            .unwrap_or(FunctionId(0));

        Ok(CompiledCell {
            module: vbc_module,
            entry_func,
            new_bindings,
            new_functions,
            result_type,
            compile_time,
            has_main,
            codegen_type_names,
        })
    }

    /// Executes a compiled cell.
    pub fn execute(
        &self,
        compiled: &CompiledCell,
        context: &mut ExecutionContext,
        cell_id: Option<CellId>,
    ) -> ExecutionResult<CellExecutionOutput> {
        let start = Instant::now();

        // Create interpreter
        let mut interpreter = Interpreter::new(Arc::clone(&compiled.module));

        // Enable output capture
        interpreter.state.capture_output = true;

        // Enable instruction counting for diagnostics
        interpreter.state.config.count_instructions = true;

        // Set instruction limit as a safety cap to prevent runaway execution
        interpreter.state.config.max_instructions = 100_000_000;

        // Inject existing bindings from context
        context.inject_bindings(&mut interpreter.state);

        // Inject active contexts
        context.inject_contexts(&mut interpreter.state);

        // Execute the entry function (FunctionId(0) — the wrapper or single definition)
        let mut result = interpreter
            .execute_function(compiled.entry_func)
            .map_err(|e| {
                let msg = format!("{:?}", e);
                if msg.contains("InstructionLimitExceeded") || msg.contains("instruction limit") {
                    ExecutionError::Runtime(
                        "Execution timed out: instruction limit exceeded (100M instructions). \
                         Possible infinite loop? Use :set timeout <ms> to adjust.".to_string()
                    )
                } else {
                    ExecutionError::Runtime(msg)
                }
            })?;

        // If the module defines a `main` function that ISN'T the entry point
        // (i.e., entry was a different function like a stdlib stub, and main is
        // a user-defined Item/Module), invoke it explicitly.
        if result.is_unit() && compiled.has_main
            && let Some(main_id) = self.find_function_by_name(&compiled.module, "main")
                && main_id != compiled.entry_func {
                    result = interpreter
                        .execute_function(main_id)
                        .map_err(|e| {
                            let msg = format!("{:?}", e);
                            if msg.contains("InstructionLimitExceeded") || msg.contains("instruction limit") {
                                ExecutionError::Runtime(
                                    "Execution timed out: instruction limit exceeded (100M instructions). \
                                     Possible infinite loop? Use :set timeout <ms> to adjust.".to_string()
                                )
                            } else {
                                ExecutionError::Runtime(msg)
                            }
                        })?;
                }

        let execution_time = start.elapsed();

        // Populate sidebar: extract bindings and functions from the compiled module
        if let Some(cid) = cell_id {
            // For let-bindings, the tail expression returns the value directly.
            // Use the VBC module's type metadata for accurate type display.
            if !result.is_unit() && compiled.new_bindings.len() == 1 {
                let (name, _) = &compiled.new_bindings[0];

                // Resolve type from three sources (best to worst):
                // 1. Codegen's variable_type_names (captures generics: List<Int>)
                // 2. VBC module's FunctionDescriptor return_type
                // 3. Runtime value type tag (Int, Float, Bool, Text)
                let type_info = compiled.codegen_type_names
                    .get(name.as_str())
                    .filter(|t| !t.is_empty() && *t != "()")
                    .map(|t| Text::from(t.as_str()))
                    .or_else(|| {
                        compiled.module.get_function(compiled.entry_func)
                            .map(|f| compiled.module.display_type_ref(&f.return_type))
                            .filter(|t| t != "()")
                            .map(|t| Text::from(t.as_str()))
                    })
                    .unwrap_or_else(|| Self::infer_type_from_value(&result));

                context.set_binding(super::context::BindingInfo {
                    name: name.clone(),
                    value: result,
                    type_info,
                    defined_in: cid,
                    is_mutable: false,
                });
                context.dependencies.add_definition(cid, name.clone());
            }

            // TLS-based extraction for any other bindings
            context.extract_bindings(compiled, &interpreter.state, cid);

            // Register functions with full signatures from VBC metadata
            for func_name in &compiled.new_functions {
                if let Some(main_id) = self.find_function_by_name(&compiled.module, func_name.as_str())
                    && let Some(desc) = compiled.module.get_function(main_id) {
                        let params: Vec<(Text, Text)> = desc.params.iter().map(|p| {
                            let pname = compiled.module.get_string(p.name)
                                .unwrap_or("_").to_string();
                            let ptype = compiled.module.display_type_ref(&p.type_ref);
                            (Text::from(pname.as_str()), Text::from(ptype.as_str()))
                        }).collect();
                        let ret = compiled.module.display_type_ref(&desc.return_type);

                        context.set_function(FunctionInfo {
                            name: func_name.clone(),
                            func_id: main_id,
                            defined_in: cid,
                            params,
                            return_type: Text::from(ret.as_str()),
                        });
                        continue;
                    }
                // Fallback if function not found by name
                context.set_function(FunctionInfo {
                    name: func_name.clone(),
                    func_id: compiled.entry_func,
                    defined_in: cid,
                    params: Vec::new(),
                    return_type: compiled.result_type.clone(),
                });
            }
        }

        // Capture stdout/stderr
        let stdout = Text::from(interpreter.state.get_stdout());
        let stderr = Text::from(interpreter.state.get_stderr());

        // Format result using cascading type sources:
        // 1. Codegen variable_type_names (generics, custom types)
        // 2. VBC FunctionDescriptor.return_type
        // 3. Runtime value type tag (Int, Float, Bool, Text)
        let real_type_hint = if !compiled.codegen_type_names.is_empty() {
            // If codegen captured variable types, the first binding's type is best
            compiled.codegen_type_names.values().next()
                .filter(|t| !t.is_empty() && *t != "()")
                .map(|t| Text::from(t.as_str()))
        } else {
            None
        }.or_else(|| {
            compiled.module.get_function(compiled.entry_func)
                .map(|f| compiled.module.display_type_ref(&f.return_type))
                .filter(|t| t != "()")
                .map(|t| Text::from(t.as_str()))
        }).unwrap_or_else(|| Self::infer_type_from_value(&result));

        let (display, type_info) =
            format_value_with_type(&result, &real_type_hint, &self.display_options);

        // Determine if we have a displayable value
        let value = if result.is_unit() { None } else { Some(result) };

        // Capture execution stats
        let instructions_executed = interpreter.state.stats.instructions;
        let peak_stack_depth = interpreter.state.stats.max_stack_depth;

        Ok(CellExecutionOutput {
            value,
            display,
            type_info,
            stdout,
            stderr,
            execution_time,
            instructions_executed,
            peak_stack_depth,
        })
    }

    /// Compiles and executes a cell in one step.
    pub fn compile_and_execute(
        &mut self,
        source: &str,
        line_number: usize,
        context: &mut ExecutionContext,
    ) -> ExecutionResult<CellExecutionOutput> {
        let compiled = self.compile(source, line_number)?;
        self.execute(&compiled, context, None)
    }

    /// Compiles and executes, tracking bindings for a specific cell.
    ///
    /// Cross-cell state: prior bindings from `context` are injected as
    /// `let name = <literal>;` preamble before the user's source. This
    /// makes variables from earlier cells available without modifying VBC.
    pub fn compile_and_execute_for_cell(
        &mut self,
        source: &str,
        line_number: usize,
        context: &mut ExecutionContext,
        cell_id: CellId,
    ) -> ExecutionResult<CellExecutionOutput> {
        // Inject prior bindings as a preamble
        let augmented_source = self.inject_bindings_as_preamble(source, context);
        let compiled = self.compile(&augmented_source, line_number)?;
        self.execute(&compiled, context, Some(cell_id))
    }

    /// Generate let-binding preamble from execution context.
    ///
    /// For each primitive binding (Int, Float, Bool, Text), emits a
    /// `let name = literal;` line so the VBC codegen treats it as a
    /// normal local variable. This enables cross-cell variable reuse
    /// without any VBC interpreter changes.
    fn inject_bindings_as_preamble(&self, source: &str, context: &ExecutionContext) -> String {
        let mut preamble = String::new();
        for (name, info) in &context.bindings {
            if let Some(literal) = Self::value_to_literal(&info.value) {
                preamble.push_str(&format!("let {} = {};\n", name, literal));
            }
        }
        if preamble.is_empty() {
            source.to_string()
        } else {
            format!("{}{}", preamble, source)
        }
    }

    /// Convert a VBC Value to a Verum literal string.
    /// Returns None for complex types that can't be represented as literals.
    fn value_to_literal(value: &Value) -> Option<String> {
        if value.is_int() {
            Some(format!("{}", value.as_i64()))
        } else if value.is_float() {
            let f = value.as_f64();
            if f.fract() == 0.0 {
                Some(format!("{}.0", f))
            } else {
                Some(format!("{}", f))
            }
        } else if value.is_bool() {
            Some(if value.as_bool() { "true" } else { "false" }.to_string())
        } else if value.is_small_string() {
            let s = value.as_small_string();
            Some(format!("\"{}\"", s.as_str().replace('\\', "\\\\").replace('"', "\\\"")))
        } else {
            None // Unit values and complex types (pointers, objects) not yet supported
        }
    }

    /// Compiles and executes source code, returning a CellOutput.
    ///
    /// This is a convenience method for async execution that takes a cell ID
    /// and returns the playbook's CellOutput type.
    ///
    /// Note: This method clones the context internally for thread-safe async execution.
    pub fn execute_source(
        &mut self,
        source: &str,
        cell_id: usize,
        context: &ExecutionContext,
    ) -> ExecutionResult<crate::playbook::session::CellOutput> {
        use crate::playbook::session::CellOutput;

        // Clone context for this execution (thread-safe for async)
        let mut exec_context = context.clone();

        let result = self.compile_and_execute(source, cell_id, &mut exec_context)?;

        // Convert to CellOutput
        if let Some(value) = result.value {
            Ok(CellOutput::Value {
                repr: result.display.clone(),
                type_info: result.type_info.clone(),
                raw: Some(value),
            })
        } else if !result.stdout.is_empty() || !result.stderr.is_empty() {
            Ok(CellOutput::Stream {
                stdout: result.stdout.clone(),
                stderr: result.stderr.clone(),
            })
        } else {
            Ok(CellOutput::Empty)
        }
    }

    /// Resets the parser state.
    pub fn reset_parser(&mut self) {
        self.parser.reset();
    }

    /// Invalidates cache from a specific line.
    pub fn invalidate_from(&mut self, line_number: usize) {
        self.parser.invalidate_from_line(line_number);
    }

    /// Converts a parse result to an AST module suitable for codegen.
    fn parse_result_to_module(
        &self,
        result: ScriptParseResult,
        module_name: &str,
    ) -> ExecutionResult<ParseToModuleResult> {
        use verum_ast::{
            Block, Expr, FunctionBody, FunctionDecl, Item, ItemKind, Module, Stmt,
            StmtKind, decl::Visibility,
        };

        let mut new_bindings = Vec::new();
        let mut new_functions = Vec::new();
        let result_type;

        // Create a wrapper function that executes the cell content
        let body = match result {
            ScriptParseResult::Expression(expr) => {
                result_type = Text::from("<expr>");

                // For expressions, we create a block with the expression as the return value
                Block::new(
                    List::new(),
                    verum_common::Maybe::Some(verum_common::Heap::new(expr.clone())),
                    expr.span,
                )
            }
            ScriptParseResult::Statement(stmt) => {
                // Extract binding info and generate a tail expression that
                // returns the bound value so the playground can capture it.
                let tail_expr = if let StmtKind::Let { pattern, ty, .. } = &stmt.kind
                    && let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind
                {
                    let type_str = ty
                        .as_ref()
                        .map(|t| format!("{:?}", t))
                        .unwrap_or_else(|| "<inferred>".to_string());
                    new_bindings.push((
                        Text::from(name.as_str()),
                        Text::from(type_str.as_str()),
                    ));
                    result_type = Text::from("<expr>");

                    // Tail expression: return the variable so its value is captured
                    verum_common::Maybe::Some(verum_common::Heap::new(Expr::ident(
                        verum_ast::Ident::new(name.as_str().to_string(), verum_ast::Span::default()),
                    )))
                } else {
                    result_type = Text::from("()");
                    verum_common::Maybe::None
                };

                Block::new(
                    List::from(vec![stmt.clone()]),
                    tail_expr,
                    stmt.span,
                )
            }
            ScriptParseResult::Item(item) => {
                result_type = Text::from("()");

                if let ItemKind::Function(ref func) = item.kind {
                    new_functions.push(Text::from(func.name.as_str()));
                }

                let module = Module {
                    items: List::from(vec![item]),
                    attributes: List::new(),
                    file_id: self.file_id,
                    span: verum_ast::Span::default(),
                };
                return Ok((module, result_type, new_bindings, new_functions));
            }
            ScriptParseResult::Module(module) => {
                result_type = Text::from("()");

                for item in module.items.iter() {
                    if let ItemKind::Function(func) = &item.kind {
                        new_functions.push(Text::from(func.name.as_str()));
                    }
                }

                return Ok((module, result_type, new_bindings, new_functions));
            }
            ScriptParseResult::Incomplete(buffer) => {
                return Err(ExecutionError::Parse(vec![format!(
                    "Incomplete input: {}",
                    buffer
                )]));
            }
            ScriptParseResult::Empty => {
                result_type = Text::from("()");
                Block::new(
                    List::new(),
                    verum_common::Maybe::None,
                    verum_ast::Span::default(),
                )
            }
        };

        // Create a main function wrapping the cell content
        let main_func = FunctionDecl {
            visibility: Visibility::Public,
            is_async: false,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: verum_common::Maybe::None,
            is_variadic: false,
            name: verum_ast::Ident::new("main".to_string(), verum_ast::Span::default()),
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
            body: verum_common::Maybe::Some(FunctionBody::Block(body)),
            span: verum_ast::Span::default(),
        };

        let main_item = Item::new(ItemKind::Function(main_func), verum_ast::Span::default());

        let module = Module {
            items: List::from(vec![main_item]),
            attributes: List::new(),
            file_id: self.file_id,
            span: verum_ast::Span::default(),
        };

        Ok((module, result_type, new_bindings, new_functions))
    }

    /// Infer type name from a runtime Value using VBC type tags.
    fn infer_type_from_value(value: &Value) -> Text {
        if value.is_int() { Text::from("Int") }
        else if value.is_float() { Text::from("Float") }
        else if value.is_bool() { Text::from("Bool") }
        else if value.is_small_string() || value.is_ptr() { Text::from("Text") }
        else if value.is_unit() { Text::from("()") }
        else if value.is_nil() { Text::from("Nil") }
        else if value.is_func_ref() { Text::from("Function") }
        else { Text::from("<value>") }
    }

    /// Look up a function by name in a VBC module.
    fn find_function_by_name(&self, module: &VbcModule, name: &str) -> Option<FunctionId> {
        for func in &module.functions {
            if let Some(func_name) = module.get_string(func.name)
                && func_name == name {
                    return Some(func.id);
                }
        }
        None
    }

    /// Gets cached cell if available.
    pub fn get_cached(&self, cell_id: u64) -> Option<&CompiledCell> {
        self.cell_cache.get(&cell_id)
    }

    /// Caches a compiled cell.
    pub fn cache_cell(&mut self, cell_id: u64, compiled: CompiledCell) {
        self.cell_cache.insert(cell_id, compiled);
    }

    /// Invalidates cache for a cell.
    pub fn invalidate_cell(&mut self, cell_id: u64) {
        self.cell_cache.remove(&cell_id);
    }

    /// Clears all cached cells.
    pub fn clear_cache(&mut self) {
        self.cell_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation() {
        let pipeline = ExecutionPipeline::new();
        assert_eq!(pipeline.module_counter, 0);
    }

    #[test]
    fn test_cache_operations() {
        let mut pipeline = ExecutionPipeline::new();

        // Create a mock compiled cell
        let module = Arc::new(VbcModule::new("test".to_string()));
        let compiled = CompiledCell {
            module,
            entry_func: FunctionId(0),
            new_bindings: vec![],
            new_functions: vec![],
            result_type: Text::from("Int"),
            compile_time: Duration::from_millis(10),
            has_main: false,
            codegen_type_names: std::collections::HashMap::new(),
        };

        pipeline.cache_cell(1, compiled);
        assert!(pipeline.get_cached(1).is_some());
        assert!(pipeline.get_cached(2).is_none());

        pipeline.invalidate_cell(1);
        assert!(pipeline.get_cached(1).is_none());
    }

    #[test]
    fn test_println_captures_stdout() {
        let mut pipeline = ExecutionPipeline::new();
        let mut context = ExecutionContext::new();

        let output = pipeline
            .compile_and_execute(r#"println("ok")"#, 1, &mut context)
            .expect("println should execute");

        assert_eq!(output.stdout.as_str(), "ok\n");
        assert!(output.value.is_none(), "println returns unit");
    }

    #[test]
    fn test_multi_statement_execution() {
        let mut pipeline = ExecutionPipeline::new();
        let mut context = ExecutionContext::new();

        let output = pipeline
            .compile_and_execute(
                r#"println("hello"); let x = 42; println(x);"#,
                1,
                &mut context,
            )
            .expect("multi-statement should execute");

        assert!(
            output.stdout.as_str().contains("hello"),
            "should contain first println, got: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.as_str().contains("42"),
            "should contain second println, got: {:?}",
            output.stdout
        );
    }

    #[test]
    fn test_let_binding_captured_in_context() {
        let mut pipeline = ExecutionPipeline::new();
        let mut context = ExecutionContext::new();
        let cell_id = CellId::new();

        let output = pipeline
            .compile_and_execute_for_cell("let a = 42", 1, &mut context, cell_id)
            .expect("let should execute");

        // The value should be captured in the execution context
        let binding = context.get_binding("a");
        assert!(binding.is_some(), "binding 'a' should exist in context");
        let info = binding.unwrap();
        assert_eq!(info.value.as_i64(), 42);
        assert_eq!(info.type_info.as_str(), "Int");

        // The display should show the value, not ()
        assert_eq!(output.display.as_str(), "42");
    }

    #[test]
    fn test_cross_cell_binding() {
        let mut pipeline = ExecutionPipeline::new();
        let mut context = ExecutionContext::new();
        let cell1 = CellId::new();
        let cell2 = CellId::new();

        // Cell 1: define a = 10
        pipeline
            .compile_and_execute_for_cell("let a = 10", 1, &mut context, cell1)
            .expect("cell 1 should execute");

        assert!(context.get_binding("a").is_some(), "a should be in context");
        assert_eq!(context.get_binding("a").unwrap().value.as_i64(), 10);

        // Cell 2: use a from cell 1
        let output = pipeline
            .compile_and_execute_for_cell("a + 5", 2, &mut context, cell2)
            .expect("cell 2 should execute using a from cell 1");

        assert_eq!(output.display.as_str(), "15");
    }

    #[test]
    fn test_expression_returns_value() {
        let mut pipeline = ExecutionPipeline::new();
        let mut context = ExecutionContext::new();

        let output = pipeline
            .compile_and_execute("42", 1, &mut context)
            .expect("42 should execute");

        assert_eq!(output.display.as_str(), "42");
        assert_eq!(output.type_info.as_str(), "Int");
        assert!(output.value.is_some());
        assert_eq!(output.value.unwrap().as_i64(), 42);
    }
}
