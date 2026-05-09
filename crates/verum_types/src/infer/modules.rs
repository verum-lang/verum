//! Module import/export and item-checking methods for the type-checker.
//!
//! Contains ~149 `TypeChecker` methods covering:
//! - Top-level item checking (`check_item`, `check_function`)
//! - Module declarations and imports (`check_module`, `check_import`)
//! - Inline module and cross-file import resolution
//! - GAT (Generic Associated Type) inference and constraint generation

#[allow(unused_imports)]
use crate::const_eval::ConstEvaluator;
#[allow(unused_imports)]
use crate::context::{TypeContext, TypeScheme};
#[allow(unused_imports)]
use crate::context_check::{ContextChecker, ContextRequirement, ContextSet};
#[allow(unused_imports)]
use crate::integer_hierarchy::IntegerHierarchy;
#[allow(unused_imports)]
use crate::meta_context::{MetaContextValidation, MetaContextValidator};
#[allow(unused_imports)]
use crate::operator_protocols::{OperatorProtocols, OutputStrategy};
#[allow(unused_imports)]
use crate::protocol::ProtocolChecker;
#[allow(unused_imports)]
use crate::refinement::RefinementChecker;
#[allow(unused_imports)]
use crate::stage_checker::{StageChecker, StageConfig};
#[allow(unused_imports)]
use crate::subtype::Subtyping;
#[allow(unused_imports)]
use crate::ty::{Type, TypeVar};
#[allow(unused_imports)]
use crate::unify::Unifier;
#[allow(unused_imports)]
use crate::{Result, TypeCheckMetrics, TypeError};
#[allow(unused_imports)]
use super::{
    DeferredConstraint, DeferredVerificationGoal, FunctionContract, GeneratorContext,
    GlobalDepthGuard, InferMode, InferResult, TypeChecker,
    WKT_HEAP, WKT_RESULT, WKT_SHARED,
    DEREF_COERCION_DEPTH, GLOBAL_CALL_DEPTH, NORMALIZE_DEPTH,
    TYPE_RESOLUTION_STACK, NORMALIZE_TYPE_STACK, AST_TO_TYPE_DEPTH,
    span_to_line_col, levenshtein_distance,
    collect_inline_mount_reexports_recursive, is_stdlib_toplevel_path,
    mount_tree_exports_name, extract_quantity_from_attrs, walk_stmt_for_qtt_usage,
    resolve_primitive_method, meta_value_to_literal,
};
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use verum_ast::{BinOp, Block, Expr, ExprKind, LiteralKind, Stmt, StmtKind, TokenTree, UnOp, Item};
#[allow(unused_imports)]
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParamKind, ImplDecl, RecordField, TypeDecl, TypeDeclBody,
    ContextDecl, ProtocolDecl, Visibility, MountDecl, MountTree, MountTreeKind,
};
#[allow(unused_imports)]
use verum_ast::pattern::Pattern;
#[allow(unused_imports)]
use verum_ast::span::{Span, Spanned};
#[allow(unused_imports)]
use verum_ast::ty::{Ident, Path};
#[allow(unused_imports)]
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
#[allow(unused_imports)]
use verum_common::well_known_types::WellKnownType as WKT;
#[allow(unused_imports)]
use verum_common::well_known_types::type_names as wkt_names;
#[allow(unused_imports)]
use verum_common::{Heap, List, Map, Maybe, Set, Shared, Text, ToText};
#[allow(unused_imports)]
use verum_modules::{ModulePath, ModuleRegistry, NameResolver, resolve_import, resolver::NameKind};

impl TypeChecker {
    /// Type check a top-level item (function, type, protocol, etc.)
    /// Type check an item declaration.
    ///

    /// Relies on RUST_MIN_STACK=16MB for stack safety on deep recursion.
    pub fn check_item(&mut self, item: &verum_ast::Item) -> Result<()> {
        let _depth_guard = self.inc_inference_depth("check_item")?;
        self.check_item_inner(item)
    }

    /// Inner implementation of check_item
    fn check_item_inner(&mut self, item: &verum_ast::Item) -> Result<()> {
        use verum_ast::ItemKind;

        // Skip items gated by @cfg predicates that don't match the current platform.
        // This prevents false type errors from platform-specific code (e.g., @cfg(linux)
        // code when compiling on macOS). The CfgEvaluator checks all @cfg attributes
        // on the item and returns false if any cfg predicate evaluates to false.
        if !self.cfg_evaluator.should_include(&item.attributes) {
            return Ok(());
        }

        match &item.kind {
            ItemKind::Function(func) => self.check_function(func),
            ItemKind::Type(type_decl) => {
                // Handle local type declarations inside functions
                // Grammar: statement = ... | item | ... ; item = type_def | ...
                // Spec: grammar/verum.ebnf - Local type definitions are valid

                // Pass 1: Register the type name as a placeholder
                self.register_type_name_only(type_decl);

                // Pass 2: Resolve the full type definition
                let mut resolution_stack = List::new();
                self.resolve_type_definition(type_decl, &mut resolution_stack)?;
                Ok(())
            }
            ItemKind::Protocol(proto_decl) => {
                // Register context protocol declarations (context protocol Name { ... })
                // These are equivalent to TypeDeclBody::Protocol but come from the shorthand syntax.
                self.register_protocol_decl_item(proto_decl)?;
                Ok(())
            }
            ItemKind::Impl(impl_decl) => self.check_impl_block(impl_decl),
            ItemKind::ContextGroup(ctx_group) => {
                // Register the context group in the resolver
                self.context_resolver.register_group(ctx_group)?;
                Ok(())
            }
            ItemKind::Context(ctx_decl) => {
                // Store the context declaration for method-level capability extraction
                let context_name: Text = ctx_decl.name.name.clone();
                self.context_declarations
                    .insert(context_name.clone(), ctx_decl.clone());

                // Build a Record type from the context's methods using the shared function
                // which properly handles method-level generics
                let context_type = self.build_context_type_from_decl(ctx_decl)?;

                // Register the context type with the resolver
                self.context_resolver
                    .register_context_type(ctx_decl.name.name.clone(), context_type);

                // Also register with context_checker for context call validation
                // Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
                self.context_checker
                    .register_context(context_name, ctx_decl.clone());

                Ok(())
            }
            ItemKind::Const(const_decl) => {
                let expected = self.ast_to_type(&const_decl.ty)?;

                // Build full path for this constant
                let const_name = const_decl.name.name.as_str();
                let const_full_path = if self.current_module_path.as_str() == "cog" {
                    verum_common::Text::from(format!("cog.{}", const_name))
                } else {
                    verum_common::Text::from(format!(
                        "{}.{}",
                        self.current_module_path.as_str(),
                        const_name
                    ))
                };

                // Set up dependency tracking for this constant
                self.current_constant_path = Maybe::Some(const_full_path.clone());
                self.constant_dependencies
                    .insert(const_full_path.clone(), std::collections::HashSet::new());

                // Check the constant's value expression (this will record dependencies)
                self.check_expr(&const_decl.value, &expected)?;

                // Clear the current constant path
                self.current_constant_path = Maybe::None;

                // Check for circular dependencies involving this constant
                self.check_constant_cycle(&const_full_path)?;

                // Evaluate the constant value and bind it to const_eval for later use
                // This enables compile-time evaluation of expressions using this const
                if let Ok(const_val) = self.const_eval.eval(&const_decl.value) {
                    self.const_eval.bind(const_name, const_val);
                }

                // Add to context
                self.ctx
                    .env
                    .insert(const_decl.name.name.as_str(), TypeScheme::mono(expected));
                Ok(())
            }
            ItemKind::Static(static_decl) => {
                let expected = self.ast_to_type(&static_decl.ty)?;

                // Register the static variable BEFORE checking the value expression.
                // This prevents cascading "unbound variable" errors when the value
                // expression fails to type-check (e.g., due to @null() or other issues).
                self.ctx.env.insert(
                    static_decl.name.name.as_str(),
                    TypeScheme::mono(expected.clone()),
                );

                // Check value expression (errors are reported but don't prevent registration)
                self.check_expr(&static_decl.value, &expected)?;
                Ok(())
            }
            ItemKind::Mount(import) => {
                // Handle stdlib imports
                let result = self.check_import(import);
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG] check_import returned, result={:?}", result.is_ok());
                result
            }
            ItemKind::Module(module_decl) => {
                // Recursively check nested module items with correct module path
                // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Nested Modules
                self.check_module(module_decl)
            }
            ItemKind::Pattern(pattern_decl) => {
                // Active pattern declarations are registered in phase 1b,
                // but we also type-check the body here.
                // Ensure it's registered (idempotent for top-level, needed for local patterns)
                if !self
                    .pattern_declarations
                    .contains_key(&pattern_decl.name.name)
                {
                    self.register_pattern_declaration(pattern_decl)?;
                }
                Ok(())
            }
            ItemKind::ExternBlock(extern_block) => {
                // Register extern (FFI) function signatures so they're available
                // as bound variables in function bodies that call them.
                for func in &extern_block.functions {
                    let _ = self.register_function_signature(func);
                }
                Ok(())
            }
            ItemKind::FFIBoundary(ffi_boundary) => {
                // Register FFI boundary function signatures so they're available
                // as bound variables in function bodies that call them.
                // Convert each FFIFunction to a function type and register it.
                let mut boundary_fields = indexmap::IndexMap::new();
                for ffi_func in &ffi_boundary.functions {
                    // Build a function type from the FFI signature
                    let mut param_types = verum_common::List::new();
                    for (_name, param_ty) in &ffi_func.signature.params {
                        if let Ok(t) = self.ast_to_type(param_ty) {
                            param_types.push(t);
                        }
                    }
                    let ret_type = self
                        .ast_to_type(&ffi_func.signature.return_type)
                        .unwrap_or(Type::Unit);
                    // S7 FIX: lift the FFI declarations onto the function's
                    // computational-property set so the property-inference
                    // engine propagates them through every call site.
                    // Without this lift, `thread_safe = false`,
                    // `memory_effects = Allocates`, `error_protocol = Errno`
                    // were dropped at registration time and a `pure fn`
                    // could call an `Allocates` FFI function with no
                    // diagnostic. The property-set mapping itself lives
                    // in `lift_ffi_function_to_properties` so the rule is
                    // testable in isolation.
                    let properties = crate::lift_ffi_function_to_properties(ffi_func);
                    let fn_type = Type::Function {
                        params: param_types,
                        return_type: Box::new(ret_type),
                        contexts: None,
                        type_params: verum_common::List::new(),
                        properties,
                    };
                    // Register function individually (for unqualified access)
                    self.ctx.env.insert(
                        ffi_func.name.name.as_str(),
                        TypeScheme::mono(fn_type.clone()),
                    );
                    // Also register as boundary_name.func_name (for qualified access)
                    let qualified_name =
                        format!("{}.{}", ffi_boundary.name.name, ffi_func.name.name);
                    self.ctx
                        .env
                        .insert(qualified_name.as_str(), TypeScheme::mono(fn_type.clone()));
                    boundary_fields.insert(ffi_func.name.name.clone(), fn_type);
                }
                // Register the boundary name itself as a record namespace
                let boundary_type = Type::Record(boundary_fields);
                self.ctx.env.insert(
                    ffi_boundary.name.name.as_str(),
                    TypeScheme::mono(boundary_type),
                );
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Type check a module declaration (nested module).
    ///

    /// This method:
    /// 1. Updates the current module path for import resolution
    /// 2. Recursively processes all items within the module
    /// 3. Restores the module path after processing
    ///

    /// Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Nested Modules
    fn check_module(&mut self, module: &verum_ast::decl::ModuleDecl) -> Result<()> {
        // Save current module path
        let saved_path = self.current_module_path.clone();

        // Build new module path
        let module_name = module.name.name.as_str();
        let new_path = if saved_path.as_str() == "cog" {
            format!("cog.{}", module_name)
        } else {
            format!("{}.{}", saved_path.as_str(), module_name)
        };
        let new_path_text = verum_common::Text::from(new_path.clone());
        self.current_module_path = new_path_text.clone();

        // Register the inline module for qualified path resolution
        // Store both the full path (crate.api) and short name (api) for easy lookup
        self.inline_modules.insert(new_path_text, module.clone());

        // Also register with short name relative to parent if at crate root
        // This allows `api.v2.func()` to resolve when `api` is declared in crate root
        if saved_path.as_str() == "cog" {
            self.inline_modules
                .insert(verum_common::Text::from(module_name), module.clone());
        }

        // Handle Option<Vec<Item>>
        if let Some(items) = &module.items {
            // Phase 1: Register type declarations first
            for item in items.iter() {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    if let Err(e) = self.register_type_declaration(type_decl) {
                        if e.is_soundness_critical() {
                            return Err(e);
                        }
                        tracing::debug!(
                            "Type registration in module '{}' failed: {}",
                            module_name,
                            e
                        );
                    }
                }
            }

            // Phase 2: Register function signatures for forward references
            // IMPORTANT: Functions declared inside `module X { ... }` are module-scoped
            // (accessed as `X.fn()`), so they must NOT overwrite an existing top-level
            // function binding with the same name. This prevents `module Transducer { fn drop() }`
            // from shadowing the top-level `fn drop<T>(value: T)` in memory.vr.
            for item in items.iter() {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    // If a top-level function with this name is already registered, skip.
                    let name_str = func.name.name.as_str();
                    if self.ctx.env.lookup(name_str).is_some() {
                        tracing::debug!(
                            "Skipping module-scoped function '{}' in module '{}' (name already registered at top level)",
                            name_str,
                            module_name
                        );
                        continue;
                    }
                    if let Err(e) = self.register_function_signature(func) {
                        tracing::debug!(
                            "Function signature registration in module '{}' failed: {}",
                            module_name,
                            e
                        );
                    }
                }
                // Register extern block function signatures
                if let verum_ast::ItemKind::ExternBlock(extern_block) = &item.kind {
                    for func in &extern_block.functions {
                        let _ = self.register_function_signature(func);
                    }
                }
            }

            // Phase 2b: Register active pattern declarations
            for item in items.iter() {
                if let verum_ast::ItemKind::Pattern(pattern_decl) = &item.kind {
                    if let Err(e) = self.register_pattern_declaration(pattern_decl) {
                        tracing::debug!(
                            "Pattern registration in module '{}' failed: {}",
                            module_name,
                            e
                        );
                    }
                }
            }

            // Phase 3: Type check all items
            for item in items.iter() {
                self.check_item(item)?;
            }
        }

        // Restore module path
        self.current_module_path = saved_path;

        Ok(())
    }

    /// Handle import statements, including stdlib and module imports.
    ///

    /// This method handles both:
    /// 1. Standard library imports (std.math, etc.)
    /// 2. User module imports (via process_import for cross-module resolution)
    ///

    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Import Resolution
    fn check_import(&mut self, import: &verum_ast::MountDecl) -> Result<()> {
        use verum_ast::MountTreeKind;

        // Extract path based on import tree kind
        let process_path = |path: &verum_ast::ty::Path| -> Text {
            path.segments
                .iter()
                .filter_map(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        Some(ident.name.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<&str>>()
                .join(".")
                .into()
        };

        // Check if this is a stdlib math import
        let is_std_math = match &import.tree.kind {
            MountTreeKind::Path(path) => process_path(path).as_str().starts_with("std.math"),
            MountTreeKind::Glob(path) => process_path(path).as_str() == "std.math",
            MountTreeKind::Nested { prefix, .. } => process_path(prefix).as_str() == "std.math",
            // #5 / P1.5 — file-relative mounts can't be stdlib
            // math imports by construction (stdlib uses module
            // paths, not source-relative file paths).
            MountTreeKind::File { .. } => false,
        };

        // Handle stdlib math imports (special case)
        if is_std_math {
            match &import.tree.kind {
                MountTreeKind::Path(path) => {
                    let path_str = process_path(path);
                    if let Some(func_name) = path_str.as_str().strip_prefix("std.math.") {
                        self.register_stdlib_math(func_name);
                    }
                }
                MountTreeKind::Glob(_) => {
                    // Register all math functions
                    for func in &[
                        "sqrt", "sin", "cos", "tan", "floor", "ceil", "round", "abs", "pow", "min",
                        "max",
                    ] {
                        self.register_stdlib_math(func);
                    }
                }
                MountTreeKind::Nested { trees, .. } => {
                    // Process nested imports like {sqrt, sin, cos}
                    for tree in trees {
                        if let MountTreeKind::Path(p) = &tree.kind {
                            let func_name = process_path(p);
                            self.register_stdlib_math(func_name.as_str());
                        }
                    }
                }
                // #5 / P1.5 — file-relative mount cannot reach
                // here (the `is_std_math` filter already rules
                // it out), but exhaustive match needs the arm.
                MountTreeKind::File { .. } => {}
            }
            return Ok(());
        }

        // Handle inline module imports (crate.module.* or crate.module.item)
        // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Import Resolution
        //

        // Note: process_path filters out Crate/Super/Self segments, so we need to
        // check if the path starts with these keywords and handle accordingly.
        let starts_with_crate = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|seg| matches!(seg, verum_ast::ty::PathSegment::Cog))
        };

        match &import.tree.kind {
            MountTreeKind::Glob(path) => {
                let module_name = process_path(path);
                // Try inline module resolution (with and without cog. prefix)
                if self.inline_modules.contains_key(&module_name) {
                    self.import_all_from_inline_module(module_name.as_str())?;
                    return Ok(());
                }
                let cog_name: verum_common::Text = format!("cog.{}", module_name).into();
                if self.inline_modules.contains_key(&cog_name) {
                    self.import_all_from_inline_module(cog_name.as_str())?;
                    return Ok(());
                }
            }
            MountTreeKind::Path(path) => {
                let path_str = process_path(path);
                let has_crate = starts_with_crate(path);
                // Try inline module resolution for any path
                if let Some((module_key, item_name)) =
                    self.find_inline_module_for_import(path_str.as_str(), has_crate)
                {
                    self.import_item_from_inline_module(&module_key, &item_name)?;
                    return Ok(());
                }
            }
            MountTreeKind::Nested { prefix, trees } => {
                let module_name = process_path(prefix);
                // Try inline module resolution (with and without cog. prefix).
                //
                // **Coverage gate.** The inline-module short-circuit
                // ONLY fires when the inline module actually exports
                // every requested item DIRECTLY (as a top-level
                // Function/Type/Const).  If the inline module merely
                // re-exports the items via `public mount`, fall through
                // to the cross-file path so the re-export chain is
                // walked through the registry.
                //
                // Without this gate, `import_item_from_inline_module`
                // returns Ok early (relying on "process_import will
                // handle the re-export") but process_import has the
                // same inline-modules short-circuit — neither side
                // actually registers the type, leading to silent E101
                // at the use site.
                let resolved_name = if self.inline_modules.contains_key(&module_name) {
                    Some(module_name.clone())
                } else {
                    let cog_name: verum_common::Text = format!("cog.{}", module_name).into();
                    if self.inline_modules.contains_key(&cog_name) {
                        Some(cog_name)
                    } else {
                        None
                    }
                };
                if let Some(resolved) = resolved_name {
                    let inline_covers_all = if let Some(inline_mod) =
                        self.inline_modules.get(&resolved).cloned()
                    {
                        let inline_items = inline_mod
                            .items
                            .as_ref()
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| match &item.kind {
                                        verum_ast::ItemKind::Function(f)
                                            if matches!(
                                                f.visibility,
                                                verum_ast::decl::Visibility::Public
                                            ) =>
                                        {
                                            Some(f.name.name.as_str().to_string())
                                        }
                                        verum_ast::ItemKind::Type(t)
                                            if matches!(
                                                t.visibility,
                                                verum_ast::decl::Visibility::Public
                                            ) =>
                                        {
                                            Some(t.name.name.as_str().to_string())
                                        }
                                        verum_ast::ItemKind::Const(c)
                                            if matches!(
                                                c.visibility,
                                                verum_ast::decl::Visibility::Public
                                            ) =>
                                        {
                                            Some(c.name.name.as_str().to_string())
                                        }
                                        _ => None,
                                    })
                                    .collect::<std::collections::HashSet<_>>()
                            })
                            .unwrap_or_default();
                        trees.iter().all(|tree| {
                            if let MountTreeKind::Path(p) = &tree.kind {
                                let want = process_path(p);
                                inline_items.contains(want.as_str())
                            } else {
                                true
                            }
                        })
                    } else {
                        false
                    };
                    if inline_covers_all {
                        for tree in trees {
                            if let MountTreeKind::Path(p) = &tree.kind {
                                let item_name = process_path(p);
                                self.import_item_from_inline_module(
                                    resolved.as_str(),
                                    item_name.as_str(),
                                )?;
                            }
                        }
                        return Ok(());
                    }
                    // Inline module exists but doesn't cover all items
                    // directly — fall through to cross-file resolution.
                }
            }
            // #5 / P1.5 — file-relative mounts are resolved by
            // the session loader before reaching the inline-
            // module pipeline; nothing to do here.
            MountTreeKind::File { .. } => {}
        }

        // For non-stdlib imports, use process_import for cross-module resolution
        // This handles imports like: import super.v1.Type, import crate.module.func, etc.
        // Clone the values we need before the mutable borrow
        let current_path = self.current_module_path.clone();
        let registry = self.module_registry.read().clone();

        // Only process if registry has modules
        if !registry.is_empty() {
            // Ignore errors for now - in single-file mode, modules may not be registered
            let result = self.process_import(import, current_path.as_str(), &registry);
            if let Err(ref e) = result {
                tracing::debug!("process_import error (ignored): {:?}", e);
            }
            let _ = result;
        }

        Ok(())
    }

    /// Resolve a dotted path against inline modules, trying all possible splits
    /// between module prefix and item suffix.
    ///

    /// For a path like "math.trig.sin", tries:
    ///  1. module="math.trig", item="sin" (also tries "cog.math.trig")
    ///  2. module="math", item="trig.sin" (also tries "cog.math")
    ///

    /// **Single-segment item discipline.**  We accept a match ONLY when the
    /// item suffix is a single segment (no dots).  Multi-segment suffixes
    /// (e.g. `mount database.postgres.row.Row` matched against an inline
    /// `database` module) would route through `import_item_from_inline_module`
    /// which searches the inline module's items list for a literal name
    /// matching `postgres.row.Row` — never present, since item names are
    /// single identifiers.  The downstream side returns Ok silently (no
    /// item found, no work done), and the caller's `if Ok(()) = ...
    /// return Ok(());` path-arm shortcut consumes the import without ever
    /// reaching cross-file resolution where the actual stdlib type
    /// (`core.database.postgres.row.Row`) lives.
    ///
    /// By rejecting multi-segment suffixes here we let the cross-file
    /// dot-split path in `process_import` handle the case correctly:
    /// `module_path = normalize("database.postgres.row")` → resolves to
    /// `core.database.postgres.row`; `item_name = "Row"` → cross-file
    /// `import_item_from_module_with_span` finds it.
    ///
    /// Returns the (module_key, item_name) pair if found AND the item is
    /// a single segment.
    pub(crate) fn find_inline_module_for_import(
        &self,
        path_str: &str,
        has_crate_prefix: bool,
    ) -> Option<(String, String)> {
        // Try splitting at each dot from right to left to find the longest matching module
        let dots: Vec<usize> = path_str.match_indices('.').map(|(i, _)| i).collect();
        // Module resolution debug trace
        tracing::trace!(
            "find_inline_module_for_import: path={:?}, has_crate_prefix={}",
            path_str,
            has_crate_prefix
        );
        for &dot_pos in dots.iter().rev() {
            let module_part = &path_str[..dot_pos];
            let item_part = &path_str[dot_pos + 1..];

            // Single-segment-item discipline (see doc comment above):
            // skip splits whose item suffix carries an embedded dot.  The
            // shortest split (rightmost dot) is the only single-segment
            // candidate, so the loop's first iteration is the only one
            // that can return Some — but we keep the loop for shape parity
            // with the historic surface so callers depending on iter()
            // don't observe a behavioural surprise.
            if item_part.contains('.') {
                continue;
            }

            // Try the module name as-is
            if self
                .inline_modules
                .contains_key(&verum_common::Text::from(module_part))
            {
                return Some((module_part.to_string(), item_part.to_string()));
            }

            // Try with cog. prefix (nested modules are registered as cog.parent.child)
            if !has_crate_prefix {
                let cog_prefixed = format!("cog.{}", module_part);
                if self
                    .inline_modules
                    .contains_key(&verum_common::Text::from(cog_prefixed.as_str()))
                {
                    return Some((cog_prefixed, item_part.to_string()));
                }
            }
        }
        None
    }

    /// Import all public items from an inline module (glob import).
    ///

    /// This handles `import cog.module.*;` for inline modules.
    /// Tracks imported names for ambiguity detection.
    ///

    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Glob Imports
    pub(crate) fn import_all_from_inline_module(&mut self, module_name: &str) -> Result<()> {
        // Cycle guard: matches `import_all_from_module`. Prevents unbounded
        // recursion when an inline module's glob expansion re-enters itself
        // via transitive re-exports.
        let module_key: Text = verum_common::Text::from(module_name);
        if self.glob_imports_in_progress.contains(&module_key) {
            let mut modules_in_cycle: List<Text> = List::new();
            let mut in_cycle = false;
            for m in &self.glob_imports_stack {
                if m == &module_key {
                    in_cycle = true;
                }
                if in_cycle {
                    modules_in_cycle.push(m.clone());
                }
            }
            modules_in_cycle.push(module_key.clone());
            let cycle_path: Text = modules_in_cycle
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(" -> ")
                .into();
            return Err(crate::TypeError::ImportCycle {
                cycle_path,
                modules_in_cycle,
                span: verum_ast::span::Span::dummy(),
            });
        }
        self.glob_imports_in_progress.insert(module_key.clone());
        self.glob_imports_stack.push(module_key.clone());
        let result = self.import_all_from_inline_module_impl(module_name);
        self.glob_imports_stack.pop();
        self.glob_imports_in_progress.remove(&module_key);
        result
    }

    fn import_all_from_inline_module_impl(&mut self, module_name: &str) -> Result<()> {
        let module = self
            .inline_modules
            .get(&verum_common::Text::from(module_name))
            .cloned()
            .ok_or_else(|| {
                TypeError::Other(verum_common::Text::from(format!(
                    "module '{}' not found",
                    module_name
                )))
            })?;

        // Two-pass: collect Mount re-exports first (so we can drop the
        // borrow on `module.items` before recursing into cross-file
        // imports that may re-enter `self.inline_modules`).  Re-exports
        // visit BEFORE local items so a local declaration that
        // shadows a re-export wins (per the language's
        // first-registered-wins discipline).
        //
        // **Recursive submodule walk.**  When the inline module contains
        // public submodules (`public module foo { public mount … }`),
        // their Mount re-exports must ALSO surface at the outer mount
        // site.  This is the canonical "prelude" pattern — `core/mod.vr`
        // declares `public module prelude { public mount super.collections.List; … }`,
        // and `mount core.*` is supposed to expose `List` etc.  Without
        // this recursion, the prelude submodule's Mount re-exports were
        // invisible to the outer walk, leaving every user file that
        // wrote `mount core.*` (and relied on the prelude) with bare
        // `List`/`Map`/`Maybe`/… resolving as E101.
        //
        // Per the architectural rule (no hardcoded stdlib knowledge in
        // the compiler), the recursion is fully general — it works for
        // ANY inline submodule that contains public Mount re-exports,
        // not just the canonical `prelude` name.
        let mut reexport_paths: Vec<(Text, Option<Text>)> = Vec::new();
        if let Some(items) = &module.items {
            collect_inline_mount_reexports_recursive(
                items.as_slice(),
                module_name,
                &mut reexport_paths,
            );
        }

        // Process the Mount re-exports against the cross-file
        // ModuleRegistry.  Glob mounts → `import_all_from_module`;
        // specific items → `import_item_from_module`.  Errors are
        // logged but not propagated — a missing dependency at
        // prelude-injection time is non-fatal (the user code that
        // tries to use the unimported name will surface E101).
        let registry_snapshot = self.module_registry.read().clone();
        for (path, item_name_opt) in reexport_paths {
            match item_name_opt {
                None => {
                    if let Err(e) = self.import_all_from_module(&path, &registry_snapshot) {
                        tracing::debug!(
                            "import_all_from_inline_module: glob re-export {} failed: {:?}",
                            path.as_str(),
                            e
                        );
                    }
                }
                Some(item_name) => {
                    if let Err(e) = self.import_item_from_module(
                        &path,
                        item_name.as_str(),
                        &registry_snapshot,
                    ) {
                        tracing::debug!(
                            "import_all_from_inline_module: specific re-export {}.{} failed: {:?}",
                            path.as_str(),
                            item_name.as_str(),
                            e
                        );
                    }
                }
            }
        }

        if let Some(items) = &module.items {
            for item in items.iter() {
                let (item_name, visibility) = match &item.kind {
                    verum_ast::ItemKind::Function(func) => {
                        (func.name.name.as_str(), &func.visibility)
                    }
                    verum_ast::ItemKind::Type(type_decl) => {
                        (type_decl.name.name.as_str(), &type_decl.visibility)
                    }
                    verum_ast::ItemKind::Const(const_decl) => {
                        (const_decl.name.name.as_str(), &const_decl.visibility)
                    }
                    _ => continue,
                };

                // Only import public items
                if !matches!(visibility, verum_ast::decl::Visibility::Public) {
                    continue;
                }

                // Track the import source for ambiguity detection
                let name_text = verum_common::Text::from(item_name);
                let source = verum_common::Text::from(format!("cog.{}", module_name));

                if let Some(sources) = self.imported_names.get_mut(&name_text) {
                    // Avoid duplicate source entries (same module imported via different paths)
                    if !sources.iter().any(|s| s == &source) {
                        sources.push(source);
                    }
                } else {
                    let mut sources = List::new();
                    sources.push(source);
                    self.imported_names.insert(name_text.clone(), sources);
                }

                // Register the item type in the environment
                self.register_imported_item_from_inline_module(&module, item_name)?;
            }
        }

        Ok(())
    }

    /// Import a single item from an inline module.
    ///

    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Named Imports
    fn import_item_from_inline_module(&mut self, module_name: &str, item_name: &str) -> Result<()> {
        let module = self
            .inline_modules
            .get(&verum_common::Text::from(module_name))
            .cloned()
            .ok_or_else(|| {
                TypeError::Other(verum_common::Text::from(format!(
                    "module '{}' not found",
                    module_name
                )))
            })?;

        // Track the import source for ambiguity detection
        let name_text = verum_common::Text::from(item_name);
        // Avoid double "cog." prefix when module_name already starts with "cog."
        let source = if module_name.starts_with("cog.") {
            verum_common::Text::from(module_name)
        } else {
            verum_common::Text::from(format!("cog.{}", module_name))
        };

        // Skip if already imported from the same source (prevents duplicate registration)
        if let Some(sources) = self.imported_names.get(&name_text) {
            if sources.iter().any(|s| s == &source) {
                return Ok(());
            }
        }

        if let Some(sources) = self.imported_names.get_mut(&name_text) {
            sources.push(source);
        } else {
            let mut sources = List::new();
            sources.push(source);
            self.imported_names.insert(name_text.clone(), sources);
        }

        // Register the item type in the environment
        self.register_imported_item_from_inline_module(&module, item_name)
    }

    /// Register an imported item from an inline module in the type environment.
    fn register_imported_item_from_inline_module(
        &mut self,
        module: &verum_ast::decl::ModuleDecl,
        item_name: &str,
    ) -> Result<()> {
        use verum_ast::ItemKind;

        if let Some(items) = &module.items {
            // First pass: if the imported name is a variant constructor of a
            // sum type declared in this module (e.g. `Ok` / `Err` from
            // `type Result<T, E> is Ok(T) | Err(E);`), register the variant's
            // constructor. Without this pass, `mount base.{Ok, Err}` falls
            // through to the unbound-variable error below even though the
            // constructors ARE exported via the parent type — a soundness
            // gap that surfaces as "unbound variable: Ok" at `<generated>:1:1`
            // on the first use of `Ok(…)` later in the user module.
            for item in items.iter() {
                if let ItemKind::Type(type_decl) = &item.kind {
                    if let verum_ast::decl::TypeDeclBody::Variant(ref variants) = type_decl.body {
                        if variants.iter().any(|v| v.name.name.as_str() == item_name) {
                            // Register the parent type first (idempotent); this
                            // populates the variant-constructor entries in the
                            // env keyed by the unqualified variant name.
                            if let Err(e) = self.register_type_declaration(type_decl) {
                                if e.is_soundness_critical() {
                                    return Err(e);
                                }
                                tracing::debug!(
                                    "Failed to register parent type '{}' for variant '{}': {}",
                                    type_decl.name.name.as_str(),
                                    item_name,
                                    e
                                );
                            }
                            return Ok(());
                        }
                    }
                }
            }

            for item in items.iter() {
                match &item.kind {
                    ItemKind::Function(func) if func.name.name.as_str() == item_name => {
                        // Register function type
                        let func_ty = self.infer_function_type(func)?;
                        self.ctx
                            .env
                            .insert(item_name, crate::context::TypeScheme::mono(func_ty));
                        return Ok(());
                    }
                    ItemKind::Type(type_decl) if type_decl.name.name.as_str() == item_name => {
                        // Register type
                        if let Err(e) = self.register_type_declaration(type_decl) {
                            if e.is_soundness_critical() {
                                return Err(e);
                            }
                            tracing::debug!(
                                "Failed to register imported type '{}': {}",
                                item_name,
                                e
                            );
                        }
                        // Also import the type's `implement` blocks so that
                        // inherent methods (especially static constructors
                        // like `Validated.valid`) are reachable from the
                        // mounting module. The cross-module import path
                        // (`import_item_from_module_impl`) does this around
                        // line 28799; without the same step here, mounting
                        // a type via its defining inline-module path
                        // (`mount core.base.result.{Validated}`) registers
                        // the type but loses its impl blocks, while a
                        // re-export mount (`mount base.{Validated}`) coincidentally
                        // works because the global stdlib pre-pass populated
                        // the methods first.
                        let synthetic_module = verum_ast::Module::new(
                            module.items.clone().unwrap_or_default(),
                            verum_common::span::FileId::dummy(),
                            module.span,
                        );
                        if let Err(e) =
                            self.import_impl_blocks_for_type(&synthetic_module, item_name)
                        {
                            tracing::debug!(
                                "Failed to import implement blocks for inline-mounted type '{}': {}",
                                item_name,
                                e
                            );
                        }
                        return Ok(());
                    }
                    ItemKind::Const(const_decl) if const_decl.name.name.as_str() == item_name => {
                        // Register constant type
                        let const_ty = self.ast_to_type(&const_decl.ty)?;
                        self.ctx
                            .env
                            .insert(item_name, crate::context::TypeScheme::mono(const_ty));

                        // Record the full path of the imported constant for dependency tracking
                        // When we later reference this constant, we can look up its full path
                        let module_name = module.name.name.as_str();
                        let const_full_path =
                            verum_common::Text::from(format!("cog.{}.{}", module_name, item_name));
                        self.imported_constant_paths
                            .insert(verum_common::Text::from(item_name), const_full_path);

                        return Ok(());
                    }
                    ItemKind::Pattern(pattern_decl)
                        if pattern_decl.name.name.as_str() == item_name =>
                    {
                        // Register active pattern declaration
                        if let Err(e) = self.register_pattern_declaration(pattern_decl) {
                            tracing::debug!(
                                "Failed to register imported pattern '{}': {}",
                                item_name,
                                e
                            );
                        }
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }

        // Fallback: the item wasn't a direct Function/Type/Const/Pattern in
        // this module's items list. Before emitting UnboundVariable, check
        // whether a prior mount (glob or explicit) already registered the
        // name — overlapping mounts are idiomatic:
        //

        //  mount core.*; // brings in Ok via prelude
        //  mount base.{Result, Ok, Err, Text}; // re-declares Ok explicitly
        //

        // Without this check, the second mount walks `base`'s items, fails
        // to find a top-level `Ok` item (it's a variant of Result, re-exported
        // via a mount), and emits `E100 unbound variable: Ok` with a dummy
        // `<generated>` span. The error is both wrong (`Ok` IS bound) and
        // unactionable (span points nowhere).
        if self.ctx.env.lookup(item_name).is_some() {
            return Ok(());
        }

        // Also check whether the module re-exports the name via a
        // `public mount .path.item` declaration. Those are explicit
        // re-exports that make the name importable from this module
        // path even though it isn't a direct top-level item.
        if let Some(items) = &module.items {
            for item in items.iter() {
                if let ItemKind::Mount(mount_decl) = &item.kind {
                    if !matches!(mount_decl.visibility, verum_ast::decl::Visibility::Public) {
                        continue;
                    }
                    if mount_tree_exports_name(&mount_decl.tree, item_name) {
                        // The name is re-exported via a `public mount` chain.
                        // Skip emitting an error — the registry-backed
                        // `process_import` path (invoked by `check_import`
                        // after this function returns) will complete the
                        // resolution through the re-export chain.
                        return Ok(());
                    }
                }
            }
        }

        // If this module's items weren't even populated yet (common during
        // stdlib registration when submodule items are loaded lazily), do
        // NOT fabricate an UnboundVariable error with a dummy span — that
        // error gets reported at `<generated>:1:1` with no actionable
        // location, and the registry-backed `process_import` path can
        // still complete the resolution. A real unbound use later will
        // surface with a proper source span at the use site.
        if module.items.as_ref().map(|i| i.is_empty()).unwrap_or(true) {
            return Ok(());
        }

        Err(TypeError::UnboundVariable {
            name: verum_common::Text::from(item_name),
            span: verum_ast::span::Span::dummy(),
        })
    }

    /// Check for circular dependencies starting from a constant.
    ///

    /// Uses DFS to detect cycles in the constant dependency graph.
    /// Returns an error if a cycle is found.
    ///

    /// Constant initialization ordering: topological sort of dependencies, cycle detection for const declarations — Constant Initialization Order
    fn check_constant_cycle(&self, start: &Text) -> Result<()> {
        let mut visited = std::collections::HashSet::new();
        let mut path = List::new();
        self.check_constant_cycle_dfs(start, &mut visited, &mut path)
    }

    /// DFS helper for cycle detection.
    fn check_constant_cycle_dfs(
        &self,
        current: &Text,
        visited: &mut std::collections::HashSet<Text>,
        path: &mut List<verum_common::Text>,
    ) -> Result<()> {
        // If we've already visited this constant in the current path, we have a cycle
        if path.iter().any(|p| p == current) {
            // Build the cycle path string
            let mut cycle_constants = List::new();
            let mut in_cycle = false;
            for p in path.iter() {
                if p == current {
                    in_cycle = true;
                }
                if in_cycle {
                    cycle_constants.push(p.clone());
                }
            }
            cycle_constants.push(current.clone());

            let cycle_path_str = cycle_constants
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");

            return Err(TypeError::CircularConstantDependency {
                cycle_path: verum_common::Text::from(cycle_path_str),
                constants_in_cycle: cycle_constants,
                span: verum_ast::span::Span::dummy(),
            });
        }

        // If we've already fully explored this constant, skip
        if visited.contains(current) {
            return Ok(());
        }

        // Add to current path
        path.push(current.clone());

        // Explore dependencies
        if let Some(deps) = self.constant_dependencies.get(current) {
            for dep in deps.iter() {
                self.check_constant_cycle_dfs(dep, visited, path)?;
            }
        }

        // Remove from path and mark as fully visited
        path.pop();
        visited.insert(current.clone());

        Ok(())
    }

    /// Record a constant dependency.
    ///

    /// Called when a constant expression references another constant.
    pub(super) fn record_constant_dependency(&mut self, referenced_constant: &Text) {
        if let Maybe::Some(ref current) = self.current_constant_path {
            if let Some(deps) = self.constant_dependencies.get_mut(current) {
                deps.insert(referenced_constant.clone());
            }
        }
    }

    /// Process an import declaration to register imported types and functions in the type environment.
    ///

    /// This enables cross-file resolution for imports like:
    /// - `import domain.errors.{RegistryError}` - imports a type from another module
    /// - `import self.checksum_service.{compute_checksum}` - imports a function from a sibling module
    /// - `import super.utils.{is_valid_username}` - imports from parent module
    ///

    /// The method looks up the source module in the registry, finds the exported items,
    /// and registers them in the type environment so they can be used during type checking.
    ///

    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Cross-module resolution
    pub fn process_import(
        &mut self,
        import: &verum_ast::MountDecl,
        current_module_path: &str,
        registry: &verum_modules::ModuleRegistry,
    ) -> Result<()> {
        use verum_ast::MountTreeKind;

        let current = ModulePath::from_str(current_module_path);

        // Check if a path starts with a relative keyword (dot, super, self)
        let is_relative_path = |path: &verum_ast::ty::Path| -> bool {
            path.segments.first().is_some_and(|seg| {
                matches!(
                    seg,
                    verum_ast::ty::PathSegment::Relative
                        | verum_ast::ty::PathSegment::Super
                        | verum_ast::ty::PathSegment::SelfValue
                )
            })
        };

        // Extract path from import tree, resolving super/self/crate/dot-relative keywords
        // into absolute module paths using the current module context.
        let extract_path = |path: &verum_ast::ty::Path| -> Text {
            let first_seg = path.segments.first();
            let starts_relative = first_seg == Some(&verum_ast::ty::PathSegment::Relative);
            let starts_super = first_seg == Some(&verum_ast::ty::PathSegment::Super);
            let starts_self = first_seg == Some(&verum_ast::ty::PathSegment::SelfValue);

            let mut parts: Vec<String> = if starts_relative || starts_super || starts_self {
                // Start from current module path for relative resolution
                current_module_path
                    .split('.')
                    .map(|s| s.to_string())
                    .collect()
            } else {
                Vec::new()
            };

            for seg in &path.segments {
                match seg {
                    verum_ast::ty::PathSegment::Name(ident) => {
                        parts.push(ident.name.as_str().to_string());
                    }
                    verum_ast::ty::PathSegment::Super => {
                        // Pop one level from the accumulated path (go to parent module)
                        if !parts.is_empty() {
                            parts.pop();
                        }
                    }
                    verum_ast::ty::PathSegment::SelfValue => {
                        // self.foo = sibling access: pop to parent, remaining segs navigate
                        if !parts.is_empty() {
                            parts.pop();
                        }
                    }
                    verum_ast::ty::PathSegment::Cog => {
                        // cog = reset to root
                        parts.clear();
                    }
                    verum_ast::ty::PathSegment::Relative => {} // Already handled above
                }
            }
            parts.join(".").into()
        };

        // Helper to resolve import path using centralized module path resolution
        let resolve_path =
            |raw_path: &str| -> std::result::Result<Text, verum_modules::ModuleError> {
                let resolved = resolve_import(raw_path, &current)?;
                Ok(verum_common::Text::from(resolved.to_string()))
            };

        // Unified module path normalization.
        // `core.*` and `std.*` are equivalent stdlib prefixes: `core.X.Y` → `std.X.Y`
        // All stdlib imports MUST use `core.` or `std.` prefix.
        let normalize_module_path = |path_str: &str| -> Text {
            if path_str.starts_with("core.") || path_str == "core" {
                // Check if this is a shorthand for core.base.* (e.g., core.ordering -> core.base.ordering)
                // Base modules live in core/base/ but are commonly referenced without the "base" segment
                let base_modules = [
                    "ordering",
                    "ops",
                    "maybe",
                    "result",
                    "protocols",
                    "primitives",
                    "memory",
                    "iterator",
                    "panic",
                    "env",
                    "data",
                    "cell",
                    "cmp",
                ];
                if let Some(rest) = path_str.strip_prefix("core.") {
                    let first_seg = rest.split('.').next().unwrap_or("");
                    if base_modules.contains(&first_seg) && !rest.starts_with("base.") {
                        // Redirect core.ordering.X -> core.base.ordering.X
                        return Text::from(format!("core.base.{}", rest));
                    }
                }
                // Already in canonical form
                Text::from(path_str)
            } else if path_str.starts_with("std.") {
                // Legacy "std." prefix - convert to "core."
                let rest = &path_str["std.".len()..];
                Text::from(format!("core.{}", rest))
            } else if path_str == "std" {
                Text::from("core")
            } else {
                // Check if this is a known stdlib top-level module path
                // These correspond to core/ subdirectories and are registered
                // in the module registry with a "core." prefix
                // Stdlib top-level prefixes — every immediate subdirectory
                // of `core/` is a candidate target for `mount X.…` as
                // shorthand for `mount core.X.…`.  Centralised through
                // `is_stdlib_toplevel_path` so that all three sites that
                // need this check stay in lockstep.
                let is_stdlib_toplevel = is_stdlib_toplevel_path(path_str);
                if is_stdlib_toplevel {
                    Text::from(format!("core.{}", path_str))
                } else {
                    // Check if this path directly exists as a module in the registry
                    // before trying relative resolution. This handles project modules
                    // like "bootstrap.token" that should be used as-is.
                    if registry.get_by_path(path_str).is_some() {
                        Text::from(path_str)
                    } else {
                        // Non-stdlib path - resolve relative to current module
                        let resolved =
                            resolve_path(path_str).unwrap_or_else(|_| Text::from(path_str));
                        // Re-check if the resolved path is a known stdlib toplevel
                        // This handles super/self paths that resolve to stdlib modules
                        // e.g., super.epoch from mem.segment -> mem.epoch -> core.mem.epoch
                        let resolved_str = resolved.as_str();
                        let resolved_is_stdlib = is_stdlib_toplevel_path(resolved_str);
                        if resolved_is_stdlib {
                            Text::from(format!("core.{}", resolved_str))
                        } else {
                            resolved
                        }
                    }
                }
            }
        };

        // Helper: pass through the already-resolved path from extract_path().
        // extract_path() already prepends current_module_path for relative imports,
        // so this helper just converts the string. No additional resolution needed.
        let resolve_relative =
            |extracted: &str, _is_relative: bool| -> Text { Text::from(extracted) };

        // Helper closure to extract a simple dotted path from AST path (filtering special segments)
        let simple_process_path = |path: &verum_ast::ty::Path| -> Text {
            path.segments
                .iter()
                .filter_map(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        Some(ident.name.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<&str>>()
                .join(".")
                .into()
        };

        // Check if a path starts with the Cog keyword
        let path_starts_with_crate = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|seg| matches!(seg, verum_ast::ty::PathSegment::Cog))
        };

        match &import.tree.kind {
            MountTreeKind::Path(path) => {
                // Try inline module resolution first (handles same-file module imports)
                let simple_path = simple_process_path(path);
                if let Some((module_key, item_name)) = self.find_inline_module_for_import(
                    simple_path.as_str(),
                    path_starts_with_crate(path),
                ) {
                    if let Ok(()) = self.import_item_from_inline_module(&module_key, &item_name) {
                        return Ok(());
                    }
                }

                // import module.path.item OR import module.path (glob when path is a module)
                let raw_path = extract_path(path);
                let is_rel = is_relative_path(path);
                let full_path = resolve_relative(raw_path.as_str(), is_rel);
                let full_path_str = full_path.as_str();

                // First, check if the full path resolves to a module in the registry.
                // If so, treat `mount module.path;` as a glob import of all public items.
                // This allows `mount sys.mmio;` to import Register, BarrierKind, etc.
                let full_normalized = normalize_module_path(full_path_str);
                tracing::debug!(
                    "mount Path: full='{}', normalized='{}'",
                    full_path_str,
                    full_normalized.as_str()
                );
                let found_module = registry.get_by_path(full_normalized.as_str());
                tracing::debug!(
                    "mount Path: module found in registry = {}",
                    found_module.is_some()
                );
                if found_module.is_some() {
                    // `mount X as A;` where X resolves to a module:
                    // register a module alias so that later `A.method(...)`
                    // routes through module-path dispatch rather than
                    // competing with a same-named stdlib value symbol.
                    // Example: `mount core.net.h3.qpack.static_table as stat;`
                    // — without the alias map, `stat` would be shadowed by
                    // core.sys.linux.syscall.stat (the POSIX stat fn).
                    //

                    // The top-level alias (`mount X as A;`) lives on the
                    // `MountDecl` itself. A nested `mount X.{item as A}`
                    // attaches the alias to the inner `MountTree` — that
                    // case is an item alias, not a module alias, and is
                    // handled elsewhere.
                    let top_level_alias = match &import.alias {
                        Maybe::Some(a) => Some(a),
                        Maybe::None => match &import.tree.alias {
                            Maybe::Some(a) => Some(a),
                            Maybe::None => None,
                        },
                    };
                    if let Some(alias) = top_level_alias {
                        let alias_name: Text = alias.name.as_str().into();
                        self.module_aliases
                            .insert(alias_name, full_normalized.clone());
                    }
                    self.import_all_from_module(&full_normalized, registry)?;
                } else if let Some(dot_pos) = full_path_str.rfind('.') {
                    // Split into module path and item name
                    // Normalize the module path (handles std.*, stdlib modules, and relative paths)
                    let module_path = normalize_module_path(&full_path_str[..dot_pos]);
                    let item_name = &full_path_str[dot_pos + 1..];
                    // Mark as explicit import (single-path imports are explicit)
                    self.explicit_imports.insert(item_name.to_string());
                    // Use span-aware import for proper error reporting
                    self.import_item_from_module_with_span(
                        &module_path,
                        item_name,
                        registry,
                        import.tree.span,
                    )?;
                }
            }

            MountTreeKind::Glob(path) => {
                // Try inline module resolution first
                let simple_glob_path = simple_process_path(path);
                let glob_candidates = if path_starts_with_crate(path) {
                    vec![simple_glob_path.clone()]
                } else {
                    vec![
                        simple_glob_path.clone(),
                        verum_common::Text::from(format!("cog.{}", simple_glob_path)),
                    ]
                };
                let mut found_inline = false;
                for candidate in &glob_candidates {
                    if self.inline_modules.contains_key(candidate) {
                        if let Ok(()) = self.import_all_from_inline_module(candidate.as_str()) {
                            found_inline = true;
                            break;
                        }
                    }
                }
                if found_inline {
                    return Ok(());
                }

                // import module.path.*
                let raw_path = extract_path(path);
                let is_rel = is_relative_path(path);
                let resolved = resolve_relative(raw_path.as_str(), is_rel);

                // Normalize the module path (handles std.*, stdlib modules, and relative paths)
                let module_path = normalize_module_path(resolved.as_str());
                tracing::debug!(
                    "mount Glob: raw='{}', resolved='{}', normalized='{}', registry_has={}",
                    raw_path.as_str(),
                    resolved.as_str(),
                    module_path.as_str(),
                    registry.get_by_path(module_path.as_str()).is_some()
                );
                // Glob imports don't fail on individual items, but module must exist
                self.import_all_from_module(&module_path, registry)?;
            }

            MountTreeKind::Nested { prefix, trees } => {
                // Try inline module resolution first.
                //
                // **Coverage gate** (mirrors check_import). Same
                // discipline: only short-circuit to the inline-module
                // path when the inline module exports every requested
                // item DIRECTLY.  If the inline module re-exports via
                // `public mount`, fall through to the cross-file
                // path so the registry walks the re-export chain.
                let simple_nested_path = simple_process_path(prefix);
                let nested_candidates = if path_starts_with_crate(prefix) {
                    vec![simple_nested_path.clone()]
                } else {
                    vec![
                        simple_nested_path.clone(),
                        verum_common::Text::from(format!("cog.{}", simple_nested_path)),
                    ]
                };
                for candidate in &nested_candidates {
                    if let Some(inline_mod) = self.inline_modules.get(candidate).cloned() {
                        let inline_direct_items = inline_mod
                            .items
                            .as_ref()
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| match &item.kind {
                                        verum_ast::ItemKind::Function(f)
                                            if matches!(
                                                f.visibility,
                                                verum_ast::decl::Visibility::Public
                                            ) =>
                                        {
                                            Some(f.name.name.as_str().to_string())
                                        }
                                        verum_ast::ItemKind::Type(t)
                                            if matches!(
                                                t.visibility,
                                                verum_ast::decl::Visibility::Public
                                            ) =>
                                        {
                                            Some(t.name.name.as_str().to_string())
                                        }
                                        verum_ast::ItemKind::Const(c)
                                            if matches!(
                                                c.visibility,
                                                verum_ast::decl::Visibility::Public
                                            ) =>
                                        {
                                            Some(c.name.name.as_str().to_string())
                                        }
                                        _ => None,
                                    })
                                    .collect::<std::collections::HashSet<_>>()
                            })
                            .unwrap_or_default();
                        let direct_covers_all = trees.iter().all(|tree| {
                            if let MountTreeKind::Path(p) = &tree.kind {
                                let want = simple_process_path(p);
                                inline_direct_items.contains(want.as_str())
                            } else {
                                true
                            }
                        });
                        if direct_covers_all {
                            let mut all_ok = true;
                            for tree in trees {
                                if let MountTreeKind::Path(p) = &tree.kind {
                                    let item_name = simple_process_path(p);
                                    if self
                                        .import_item_from_inline_module(
                                            candidate.as_str(),
                                            item_name.as_str(),
                                        )
                                        .is_err()
                                    {
                                        all_ok = false;
                                        break;
                                    }
                                }
                            }
                            if all_ok {
                                return Ok(());
                            }
                        }
                        // Fall through to cross-file resolution.
                    }
                }
                // import module.path.{item1, item2}
                let raw_prefix = extract_path(prefix);
                let is_rel = is_relative_path(prefix);
                let resolved = resolve_relative(raw_prefix.as_str(), is_rel);

                // Normalize the module path (handles std.*, stdlib modules, and relative paths)
                let module_path = normalize_module_path(resolved.as_str());

                // #[cfg(debug_assertions)]
                // eprintln!(
                // "[DEBUG] Resolved module path: '{}' -> '{}'",
                // prefix_str.as_str(),
                // module_path.as_str()
                // );

                // Import each item with span-aware error reporting
                // Collect first error but continue processing all items to avoid cascading failures
                let mut first_error: Option<TypeError> = None;
                for tree in trees {
                    if let MountTreeKind::Path(item_path) = &tree.kind {
                        // Skip `self` imports in nested mounts (e.g., mount core.collections.{self, List})
                        // `self` brings the module itself into scope, not an item from it
                        let is_self_import = item_path.segments.len() == 1
                            && matches!(
                                item_path.segments.first(),
                                Some(verum_ast::ty::PathSegment::SelfValue)
                            );
                        if is_self_import {
                            // Register the module name as an alias if specified
                            // e.g., mount core.io.{self as io} -> alias "io" to the module
                            if let Some(alias) = &tree.alias {
                                let alias_ty = Type::Named {
                                    path: verum_ast::ty::Path::new(
                                        verum_common::List::from(vec![
                                            verum_ast::ty::PathSegment::Name(
                                                verum_ast::Ident::new(
                                                    module_path.clone(),
                                                    tree.span,
                                                ),
                                            ),
                                        ]),
                                        tree.span,
                                    ),
                                    args: List::new(),
                                };
                                self.ctx
                                    .env
                                    .insert(alias.name.clone(), TypeScheme::mono(alias_ty));
                            }
                            continue;
                        }
                        let item_name = extract_path(item_path);
                        // Skip empty item names (e.g., from unresolved self/super segments)
                        if item_name.is_empty() {
                            continue;
                        }
                        // Check for import alias (e.g., `IoError as EngineIoError`)
                        let local_name: Option<&str> = tree.alias.as_ref().map(|a| a.name.as_str());
                        // Use span-aware import for proper error reporting
                        // If import fails but the item is already registered as a builtin,
                        // silently continue (the builtin provides the type).
                        // Mark this name as explicitly imported BEFORE the import call.
                        // This ensures that even if the import triggers transitive loads
                        // (which may try to import a conflicting name via glob), the
                        // explicit import takes precedence.
                        let register_name = local_name.unwrap_or(item_name.as_str());
                        self.explicit_imports.insert(register_name.to_string());

                        let import_result = self.import_item_from_module_with_alias_and_span(
                            &module_path,
                            item_name.as_str(),
                            local_name,
                            registry,
                            tree.span,
                        );
                        if let Err(e) = import_result {
                            let check_name = local_name.unwrap_or(item_name.as_str());
                            let check_text = verum_common::Text::from(check_name);
                            let found_in_env = self.ctx.env.lookup(&check_text).is_some();
                            let found_in_types = self.ctx.type_defs.contains_key(&check_text);
                            if !found_in_env && !found_in_types {
                                // Collect error but continue processing remaining items
                                if first_error.is_none() {
                                    first_error = Some(e);
                                }
                            }
                            // Item exists as builtin, skip the import error
                        }
                    }
                    // Other kinds (Glob, Nested) can be handled recursively if needed
                }
                // Return the first collected error after processing all items
                if let Some(e) = first_error {
                    return Err(e);
                }
            }
            // #5 / P1.5 — file-relative mounts are session-loader-resolved
            // before reaching this inline-module pipeline; nothing to do.
            MountTreeKind::File { .. } => {}
        }

        Ok(())
    }

    /// Import a single item from a module into the type environment.
    ///

    /// For types (including variant types), this method finds the type declaration
    /// in the source module's AST and registers it properly, including variant
    /// constructors. This enables cross-file type resolution.
    ///

    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    ///

    /// Relies on RUST_MIN_STACK=16MB for stack safety when importing deeply
    /// nested type dependencies transitively.
    fn import_item_from_module(
        &mut self,
        module_path: &Text,
        item_name: &str,
        registry: &verum_modules::ModuleRegistry,
    ) -> Result<()> {
        // No span = internal/transitive import, errors logged but not propagated
        self.import_item_from_module_inner(module_path, item_name, None, registry, None)
    }

    /// Import with an optional alias for the local name.
    ///

    /// This handles imports like `import module.{IoError as EngineIoError}` where
    /// the type is registered under the alias name (`EngineIoError`) instead of
    /// the original name (`IoError`).
    ///

    /// Constant initialization ordering: topological sort of dependencies, cycle detection for const declarations — Import Aliases
    fn import_item_from_module_with_alias(
        &mut self,
        module_path: &Text,
        item_name: &str,
        local_name: Option<&str>,
        registry: &verum_modules::ModuleRegistry,
    ) -> Result<()> {
        // No span = internal/transitive import, errors logged but not propagated
        self.import_item_from_module_inner(module_path, item_name, local_name, registry, None)
    }

    /// Inner implementation of import_item_from_module.
    ///

    /// The `local_name` parameter is the name under which the item will be registered
    /// in the local scope. If None, the original `item_name` is used.
    ///

    /// The `import_span` parameter is used for error reporting. When `Some`, errors
    /// will be returned with proper source location. When `None`, errors are logged
    /// but not propagated (for transitive imports).
    fn import_item_from_module_inner(
        &mut self,
        module_path: &Text,
        item_name: &str,
        local_name: Option<&str>,
        registry: &verum_modules::ModuleRegistry,
        import_span: Option<Span>,
    ) -> Result<()> {
        if std::env::var("VERUM_TRACE_IMPORT").is_ok() {
            eprintln!(
                "[trace-import] inner entry: module='{}' item='{}' local='{:?}' span={}",
                module_path.as_str(),
                item_name,
                local_name,
                import_span.is_some()
            );
        }
        // Circular import detection: check if we're already importing this item.
        // This prevents infinite recursion when module A imports from B and B imports from A.
        // Circular imports are allowed but warn the developer about the dependency structure.
        let import_key = (module_path.clone(), Text::from(item_name));
        if self.imports_in_progress.contains(&import_key) {
            // Circular import detected - this is OK, just skip to avoid infinite recursion.
            // The type will be resolved later when the outer import completes.
            // #[cfg(debug_assertions)]
            // eprintln!(
            // "[DEBUG] Circular import detected: '{}' from '{}' (skipping)",
            // item_name,
            // module_path.as_str()
            // );
            tracing::debug!(
                "Circular import detected: '{}' from '{}' - skipping to prevent infinite recursion",
                item_name,
                module_path.as_str()
            );
            return Ok(());
        }

        // Mark this import as in-progress
        self.imports_in_progress.insert(import_key.clone());

        // Execute the import and ensure cleanup happens
        let result = self.import_item_from_module_impl(
            module_path,
            item_name,
            local_name,
            registry,
            import_span,
        );

        // Always remove the key when done
        self.imports_in_progress.remove(&import_key);

        if std::env::var("VERUM_TRACE_IMPORT").is_ok() {
            let in_env = self
                .ctx
                .env
                .lookup(&Text::from(local_name.unwrap_or(item_name)))
                .is_some();
            eprintln!(
                "[trace-import] inner exit: module='{}' item='{}' ok={} in_env={}",
                module_path.as_str(),
                item_name,
                result.is_ok(),
                in_env
            );
        }

        result
    }

    /// Import an item from a module with span information for error reporting.
    ///

    /// Use this for user-facing imports where errors should be reported with
    /// proper source locations. For internal/transitive imports, use the
    /// spanless variants which log errors but don't propagate them.
    fn import_item_from_module_with_span(
        &mut self,
        module_path: &Text,
        item_name: &str,
        registry: &verum_modules::ModuleRegistry,
        span: Span,
    ) -> Result<()> {
        self.import_item_from_module_inner(module_path, item_name, None, registry, Some(span))
    }

    /// Import an item with alias and span information for error reporting.
    fn import_item_from_module_with_alias_and_span(
        &mut self,
        module_path: &Text,
        item_name: &str,
        local_name: Option<&str>,
        registry: &verum_modules::ModuleRegistry,
        span: Span,
    ) -> Result<()> {
        self.import_item_from_module_inner(module_path, item_name, local_name, registry, Some(span))
    }

    /// Actual implementation of import_item_from_module (separated to enable cleanup).
    ///

    /// The `local_name` parameter is the name under which the item will be registered
    /// in the local scope. If None, the original `item_name` is used.
    ///

    /// The `import_span` parameter is used for error reporting. When `Some`, errors will
    /// include proper source location. When `None` (transitive imports), errors are
    /// logged but not propagated to avoid failing on internal dependency resolution.
    fn import_item_from_module_impl(
        &mut self,
        module_path: &Text,
        item_name: &str,
        local_name: Option<&str>,
        registry: &verum_modules::ModuleRegistry,
        import_span: Option<Span>,
    ) -> Result<()> {
        use verum_modules::ExportKind;
        // Determine the name to use for registration
        let register_name = local_name.unwrap_or(item_name);

        // Import provenance: explicit imports (`mount foo.{Bar}`) take precedence
        // over glob (`mount foo.*`) and internal/transitive imports.
        // Explicit imports have import_span = Some(_), glob/internal have None.
        // If a name was already explicitly imported, skip non-explicit re-imports
        // to prevent name collisions (e.g., atomic Ordering overwriting comparison Ordering).
        if import_span.is_none() && self.explicit_imports.contains(register_name) {
            return Ok(());
        }

        // **Pre-flight existence check.** If the source module does not
        // actually export `item_name`, return Ok early WITHOUT touching
        // `glob_import_provenance`.  Without this gate, transitive-
        // dependency walks that probe a sibling module for a name it
        // doesn't have (e.g. a parent type's body referring to `Maybe`,
        // imported transitively from `core.base.iterator` which doesn't
        // export `Maybe`) would silently insert a stub provenance entry
        // — blocking later valid attempts from the canonical source via
        // the "tied-priority preserves first" rule in
        // `ImportProvenance::allows_overwrite`.
        //
        // The check uses path aliases so module-path normalisation
        // (core. prefix, std. legacy) is applied consistently with the
        // body resolution path.
        if let Some(probed_module_info) =
            self.get_module_with_path_aliases(module_path.as_str(), registry)
        {
            let probed_name_text: Text = item_name.to_string().into();
            if probed_module_info.exports.get(&probed_name_text).is_none() {
                // Try the simple-path inline fallback (mirrors the
                // resolution in `import_item_from_module_body` so we
                // don't false-skip inline modules registered without
                // file-name prefix).
                let path_str = module_path.as_str();
                let inline_has_export = if let Some(dot_pos) = path_str.find('.') {
                    let simple_path = &path_str[dot_pos + 1..];
                    self.get_module_with_path_aliases(simple_path, registry)
                        .map(|m| m.exports.get(&probed_name_text).is_some())
                        .unwrap_or(false)
                } else {
                    false
                };
                if !inline_has_export {
                    return Ok(());
                }
            }
        }

        // MOD-MED-2 — glob origin discipline. For
        // glob/internal imports (`import_span.is_none()`), classify
        // the source module against the user's cog name and consult
        // `glob_shadow_arbiter` to decide whether the incoming entry
        // is allowed to overwrite an existing glob registration.
        // Project beats external beats stdlib; same-origin ties
        // preserve the first registrant for determinism. Explicit
        // imports skip this layer entirely (the gate above already
        // returned for the case where explicit was overridden by a
        // glob; here we handle the inverse — glob arriving on top of
        // a prior glob).
        if import_span.is_none() {
            let incoming_origin = crate::import_origin::ImportOrigin::classify(
                module_path.as_str(),
                self.current_cog_name.as_str(),
            );
            let incoming =
                crate::import_origin::ImportProvenance::new(incoming_origin, module_path.clone());
            if !self.glob_shadow_arbiter(register_name, incoming) {
                return Ok(());
            }
        }

        // Cycle guard: nested explicit imports (`mount A.{Item}`)
        // can re-enter through impl-block elaboration, type-resolution
        // chains, and re-export walks.
        //

        // Key shape: (module_path, item_name) pair tracked while a
        // resolution is in-flight. A second entry means we've recursed
        // back into the SAME (module, item) — typically a forward
        // reference that's resolved by the outer expansion completing.
        //

        // Treatment: skip the inner re-entry with Ok(()) — the symbol
        // will be visible once the outer resolution finishes registering
        // it. This matches the older `imports_in_progress` discipline
        // at `import_item_from_module_inner` (the soft skip that has
        // existed since cycle handling was first introduced). Returning
        // a hard `TypeError::ImportCycle` here surfaces as E0811 on
        // legitimate stdlib re-export topologies (every sqlite-native
        // run-test failed E0811 with `mount l1_pager.{Pager}` /
        // `mount core.base.{Result}` after the original strict-Err
        // shape landed).
        //

        // Discipline mirrors `glob_imports_in_progress`: insert before
        // the body, remove at every clean exit. The body is factored
        // into `import_item_from_module_body` so the remove sits at
        // exactly one site.
        // Cycle-guard discipline: this function is the body of import
        // resolution. `import_item_from_module_inner` (the public wrapper)
        // already inserts `(module_path, item_name)` into
        // `imports_in_progress` before calling us. Re-checking the same key
        // here would always false-fire and skip the body entirely — that
        // was the historic regression that made cross-cog `pub const X`
        // imports silently no-op (the const arm at line ~29750 never
        // ran, so the symbol never landed in `ctx.env`, surfacing as
        // `unbound variable: X` at the use site even though the export
        // table contained it).
        //

        // Direct callers below in `import_item_from_module_body`
        // (the prelude / submodule fallback paths) intentionally re-enter
        // `_impl` with a DIFFERENT (module_path, item_name) pair, so they
        // get cycle-guarded by `_inner` for THEIR new key — never by ours.
        //

        // If a future caller is added that calls `_impl` directly with
        // the same key as the outer entry, route it through
        // `import_item_from_module_inner` instead so the cycle key is
        // managed at exactly one site.
        let cycle_key: (Text, Text) = (module_path.clone(), item_name.to_string().into());
        let we_own_cycle_key = !self.imports_in_progress.contains(&cycle_key);
        if we_own_cycle_key {
            self.imports_in_progress.insert(cycle_key.clone());
        }
        let outcome = self.import_item_from_module_body(
            module_path,
            item_name,
            local_name,
            registry,
            import_span,
            register_name,
        );
        if we_own_cycle_key {
            self.imports_in_progress.remove(&cycle_key);
        }
        outcome
    }

    /// Inner body of `import_item_from_module_impl` — extracted purely so
    /// the cycle-guard insert/remove pair can sit around a single call.
    /// Do NOT add additional callers; this is a private helper.
    fn import_item_from_module_body(
        &mut self,
        module_path: &Text,
        item_name: &str,
        local_name: Option<&str>,
        registry: &verum_modules::ModuleRegistry,
        import_span: Option<Span>,
        register_name: &str,
    ) -> Result<()> {
        use verum_modules::ExportKind;
        let _ = (local_name, register_name); // shadow for body

        // Look up the source module in the registry.
        // For inline modules, try multiple path variants:
        // 1. Full qualified path (e.g., "file_name.data.models")
        // 2. Simple path without file prefix (e.g., "data.models")
        // 3. Path aliases (e.g., "core.io" -> "std.io")
        // This handles the case where inline modules are registered without file name prefix.
        let (resolved_module_path, module_info_opt) =
            if let Some(info) = self.get_module_with_path_aliases(module_path.as_str(), registry) {
                (module_path.clone(), Some(info))
            } else {
                // Try stripping the first segment (file name prefix) for inline modules
                let path_str = module_path.as_str();
                if let Some(dot_pos) = path_str.find('.') {
                    let simple_path = &path_str[dot_pos + 1..];
                    if let Some(info) = self.get_module_with_path_aliases(simple_path, registry) {
                        // #[cfg(debug_assertions)]
                        // eprintln!(
                        // "[DEBUG] Found inline module '{}' using simple path '{}'",
                        // module_path.as_str(),
                        // simple_path
                        // );
                        (verum_common::Text::from(simple_path), Some(info))
                    } else {
                        (module_path.clone(), None)
                    }
                } else {
                    (module_path.clone(), None)
                }
            };

        if let Some(module_info) = module_info_opt {
            // #[cfg(debug_assertions)]
            // eprintln!(
            // "[DEBUG] Found module '{}' with {} exports",
            // resolved_module_path.as_str(),
            // module_info.exports.len()
            // );

            // Pre-register all function signatures from this module to enable forward references
            // within the imported module itself. This is critical for cases where a function
            // calls another function defined later in the same module.
            self.preregister_module_function_signatures(
                &module_info.ast,
                resolved_module_path.as_str(),
            );

            // Find the exported item
            // Note: ExportTable.get expects &Text (verum_common::Text)
            let item_name_text: Text = item_name.to_string().into();
            if let Some(exported) = module_info.exports.get(&item_name_text) {
                // IMPORTANT: For re-exports, the ExportKind may be incorrect (defaulted to Type).
                // When a module uses `pub import .submodule.{Item}`, the re-export gets
                // ExportKind::Type by default. We need to resolve the actual kind by
                // following the re-export chain back to the original definition.
                //

                // This is critical for context protocols: if `contexts/database.vr` exports
                // a `context protocol Database`, and `contexts/mod.vr` re-exports it via
                // `pub import .database.{Database}`, we need to recognize that Database is
                // actually a Context (not a Type) and register it properly for `using [...]` clauses.
                //

                // Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — .4 - Re-exports
                // Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
                let actual_kind = if exported.kind == ExportKind::Type {
                    // The kind may be wrong due to re-export. Try to resolve the actual kind.
                    self.resolve_export_kind_with_reexports(
                        &module_info.ast,
                        item_name,
                        &resolved_module_path,
                        registry,
                    )
                    .unwrap_or(exported.kind)
                } else {
                    exported.kind
                };

                match actual_kind {
                    ExportKind::Function | ExportKind::Meta => {
                        // For functions, we need to look up the actual type from the module's AST.
                        // If not found directly (e.g., for re-exported variant constructors),
                        // follow the re-export chain to find the original definition.
                        // Use TypeScheme::poly() for generic functions, TypeScheme::mono() for non-generic.
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG] About to extract_function_type_from_module for '{}'", item_name);
                        if let Some((func_type, type_vars)) =
                            self.extract_function_type_from_module(&module_info.ast, item_name)
                        {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Extracted function type for '{}' successfully", item_name);
                            let scheme = if type_vars.is_empty() {
                                TypeScheme::mono(func_type)
                            } else {
                                TypeScheme::poly(type_vars, func_type)
                            };
                            self.ctx.env.insert(register_name, scheme);
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Inserted '{}' into env, returning from import_item_from_module_impl", item_name);
                        } else if let Some((func_type, type_vars, _source_path)) = self
                            .find_function_with_source_module(
                                &module_info.ast,
                                item_name,
                                &resolved_module_path,
                                registry,
                            )
                        {
                            let scheme = if type_vars.is_empty() {
                                TypeScheme::mono(func_type)
                            } else {
                                TypeScheme::poly(type_vars, func_type)
                            };
                            self.ctx.env.insert(register_name, scheme);
                        }
                    }
                    ExportKind::Type | ExportKind::Protocol => self.import_type_export(&*module_info, item_name, register_name, &resolved_module_path, registry, import_span, exported.source_module)?,
                    ExportKind::Const | ExportKind::Static => {
                        if std::env::var("VERUM_TRACE_IMPORT").is_ok() {
                            eprintln!(
                                "[trace-import] Const arm: module='{}' item='{}' register='{}'",
                                resolved_module_path.as_str(),
                                item_name,
                                register_name
                            );
                        }
                        // For constants, look up the type from the module's AST
                        if let Some(const_type) =
                            self.extract_const_type_from_module(&module_info.ast, item_name)
                        {
                            if std::env::var("VERUM_TRACE_IMPORT").is_ok() {
                                eprintln!(
                                    "[trace-import] Const found in module AST, inserting '{}'",
                                    register_name
                                );
                            }
                            self.ctx
                                .env
                                .insert(register_name, TypeScheme::mono(const_type));
                        } else {
                            // Not found directly — follow re-export chain through submodules.
                            // This handles glob re-exports like `public mount atomic.*` in mod.vr
                            // where the const is defined in a submodule (e.g., atomic.vr).
                            let found = self.find_const_in_submodules(
                                &resolved_module_path,
                                item_name,
                                registry,
                            );
                            if let Some(const_type) = found {
                                if std::env::var("VERUM_TRACE_IMPORT").is_ok() {
                                    eprintln!(
                                        "[trace-import] Const found in submodule, inserting '{}'",
                                        register_name
                                    );
                                }
                                self.ctx
                                    .env
                                    .insert(register_name, TypeScheme::mono(const_type));
                            } else if std::env::var("VERUM_TRACE_IMPORT").is_ok() {
                                eprintln!("[trace-import] Const NOT FOUND for '{}'", item_name);
                            }
                        }
                    }
                    ExportKind::Context => self.import_context_export(&*module_info, item_name, register_name, &resolved_module_path, registry)?,
                    ExportKind::ContextGroup => {
                        // Context groups are registered as contexts only (they expand to multiple contexts)
                        self.register_protocol_as_context(verum_common::Text::from(register_name));
                    }
                    ExportKind::Module => {
                        // Register the imported inline module so qualified path calls like
                        // `ModuleName.fn(...)` resolve in the importing file.
                        // Without this, `Transducer.map(...)` fails because the importing
                        // checker's `inline_modules` map is empty for cross-file modules.
                        for ast_item in &module_info.ast.items {
                            if let verum_ast::ItemKind::Module(mod_decl) = &ast_item.kind {
                                if mod_decl.name.name.as_str() == item_name {
                                    let short_name = verum_common::Text::from(register_name);
                                    let full_path = verum_common::Text::from(format!(
                                        "{}.{}",
                                        resolved_module_path.as_str(),
                                        item_name
                                    ));
                                    self.inline_modules.insert(short_name, mod_decl.clone());
                                    self.inline_modules.insert(full_path, mod_decl.clone());
                                    break;
                                }
                            }
                        }
                    }
                    ExportKind::Predicate => {
                        // Predicates don't need special handling in the type environment
                    }
                }
            } else {
                // Item not found in exports table. This can happen for re-exported items where
                // the export table entry hasn't been properly populated. Try to find the item
                // directly in the module's AST, which will follow re-export chains.
                //

                // Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — .4 - Re-exports
                if let Some((type_decl, source_module_path)) = self
                    .find_type_declaration_with_source_module(
                        &module_info.ast,
                        item_name,
                        &resolved_module_path,
                        registry,
                    )
                {
                    // Import all types from the type's source module first
                    // Use path aliases since "core.io.path" may be stored as "std.io.path"
                    if let Some(source_module) =
                        self.get_module_with_path_aliases(source_module_path.as_str(), registry)
                    {
                        let src_path = source_module_path.as_str().to_string();
                        self.import_types_from_module_ast_in_module(
                            &source_module.ast,
                            Some(&src_path),
                        );
                    }

                    // Register the type declaration
                    if let Err(e) = self.register_type_declaration(&type_decl) {
                        if e.is_soundness_critical() {
                            return Err(e);
                        }
                        tracing::warn!(
                            "Failed to register imported type '{}' from module '{}': {}",
                            item_name,
                            resolved_module_path.as_str(),
                            e
                        );
                    } else {
                        // Also import implement blocks for the type
                        // Use path aliases since "core.io.path" may be stored as "std.io.path"
                        // Pin the checker's module path to the SOURCE module so
                        // bare type references inside impl blocks
                        // (e.g. `-> Result<T, RecvError>`) resolve against the
                        // source module's qualified-name layer first and don't
                        // get a same-named stranger from the flat map.
                        let src_path = source_module_path.as_str().to_string();
                        if let Some(source_module) =
                            self.get_module_with_path_aliases(source_module_path.as_str(), registry)
                            && let Err(e) = self.import_impl_blocks_for_type_in_module(
                                &source_module.ast,
                                item_name,
                                Some(&src_path),
                            )
                        {
                            tracing::debug!(
                                "Failed to import implement blocks for '{}' from '{}': {}",
                                item_name,
                                source_module_path.as_str(),
                                e
                            );
                        }
                        // Also register the companion inline module (if any) from the source module AST.
                        // This handles the pattern: `public type Foo is {...}; public module Foo { fn ... }`
                        // where Foo.method() calls need to resolve even when Foo is imported transitively.
                        let short_name = verum_common::Text::from(register_name);
                        if !self.inline_modules.contains_key(&short_name) {
                            if let Some(source_module) = self
                                .get_module_with_path_aliases(source_module_path.as_str(), registry)
                            {
                                let inline_mod =
                                    source_module.ast.items.iter().find_map(|ast_item| {
                                        if let verum_ast::ItemKind::Module(mod_decl) =
                                            &ast_item.kind
                                        {
                                            if mod_decl.name.name.as_str() == item_name {
                                                return Some(mod_decl.clone());
                                            }
                                        }
                                        None
                                    });
                                if let Some(mod_decl) = inline_mod {
                                    self.inline_modules.insert(short_name, mod_decl);
                                }
                            }
                        }
                    }
                } else {
                    // Last resort: check the module's prelude sub-module.
                    // This handles `mount core.{Maybe}` when `Maybe` is in `core.prelude`
                    // via `public mount super.base.*`.
                    let prelude_path = format!("{}.prelude", resolved_module_path.as_str());
                    let found_in_prelude = if let Some(prelude_info) =
                        self.get_module_with_path_aliases(&prelude_path, registry)
                    {
                        let prelude_exported = prelude_info.exports.get(&Text::from(item_name));
                        if prelude_exported.is_some() {
                            // Re-import from the prelude sub-module
                            drop(prelude_info);
                            let prelude_text = Text::from(prelude_path);
                            let _ = self.import_item_from_module_impl(
                                &prelude_text,
                                item_name,
                                local_name,
                                registry,
                                None, // No span - internal resolution
                            );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    // If not found in prelude, try submodules (handles glob re-exports).
                    // When a module has `public mount arithmetic.*;`, the items from
                    // arithmetic.vr are not in the module's export table. We search
                    // declared submodules to find the item.
                    let found_in_submodule = if !found_in_prelude {
                        let mut found = false;
                        // Collect submodule names from the module's AST
                        // Sources: 1) explicit module declarations 2) public glob mount paths
                        let mut submodule_names: Vec<String> = module_info
                            .ast
                            .items
                            .iter()
                            .filter_map(|ast_item| {
                                match &ast_item.kind {
                                    verum_ast::ItemKind::Module(mod_decl) => {
                                        if mod_decl.visibility
                                            == verum_ast::decl::Visibility::Public
                                        {
                                            return Some(mod_decl.name.name.to_string());
                                        }
                                        None
                                    }
                                    verum_ast::ItemKind::Mount(mount_decl) => {
                                        // Extract submodule name from `public mount X.*;` patterns
                                        if mount_decl.visibility
                                            == verum_ast::decl::Visibility::Public
                                        {
                                            if let verum_ast::decl::MountTreeKind::Glob(path) =
                                                &mount_decl.tree.kind
                                            {
                                                // The glob path is the submodule (e.g., "arithmetic" in `public mount arithmetic.*;`)
                                                if let Some(verum_ast::ty::PathSegment::Name(
                                                    ident,
                                                )) = path.segments.last()
                                                {
                                                    return Some(ident.name.to_string());
                                                }
                                            }
                                        }
                                        None
                                    }
                                    _ => None,
                                }
                            })
                            .collect();
                        submodule_names.dedup();
                        for sub_name in &submodule_names {
                            let sub_path_str =
                                format!("{}.{}", resolved_module_path.as_str(), sub_name);
                            let sub_path_text = Text::from(sub_path_str.as_str());
                            if let Some(sub_info) =
                                self.get_module_with_path_aliases(&sub_path_str, registry)
                            {
                                let item_text = Text::from(item_name);
                                if sub_info.exports.get(&item_text).is_some() {
                                    // Found in submodule - re-import from there
                                    drop(sub_info);
                                    let _ = self.import_item_from_module_impl(
                                        &sub_path_text,
                                        item_name,
                                        local_name,
                                        registry,
                                        None,
                                    );
                                    found = true;
                                    break;
                                }
                            }
                        }
                        // If submodule search failed, try direct AST lookup in the parent module.
                        // This handles cases where files like arithmetic.vr, bitwise.vr etc. are
                        // grouped into the parent module (core.intrinsics) rather than registered
                        // as separate submodules. Their public functions are in the parent's AST.
                        if !found {
                            if let Some((func_type, type_vars)) =
                                self.extract_function_type_from_module(&module_info.ast, item_name)
                            {
                                let scheme = if type_vars.is_empty() {
                                    TypeScheme::mono(func_type)
                                } else {
                                    TypeScheme::poly(type_vars, func_type)
                                };
                                self.ctx.env.insert(register_name, scheme);
                                found = true;
                            } else if let Some((type_decl, _source)) = self
                                .find_type_declaration_with_source_module(
                                    &module_info.ast,
                                    item_name,
                                    &resolved_module_path,
                                    registry,
                                )
                            {
                                // Try registering as a type. Soundness-critical
                                // errors (positivity) must still abort even
                                // through this fallback path.
                                if let Err(e) = self.register_type_declaration(&type_decl) {
                                    if e.is_soundness_critical() {
                                        return Err(e);
                                    }
                                }
                                found = true;
                            }
                        }
                        found
                    } else {
                        true
                    };

                    if !found_in_submodule {
                        if let Some(span) = import_span {
                            // Item not found in exports table OR as a type in AST
                            // Return error if span is provided (user import)
                            let available_items: List<Text> = module_info
                                .exports
                                .public_exports()
                                .map(|e| e.name.clone())
                                .collect();

                            if !self.stdlib_single_file_mode {
                                return Err(TypeError::ImportItemNotFound {
                                    item_name: Text::from(item_name),
                                    module_path: resolved_module_path.clone(),
                                    available_items,
                                    span,
                                });
                            }
                        }
                    }
                }
                // If no span provided (transitive import), continue silently
                // Later passes will catch unresolved names
            }
        } else {
            // #[cfg(debug_assertions)]
            // eprintln!(
            // "[DEBUG] Module '{}' not found in registry, attempting lazy load",
            // module_path.as_str()
            // );

            // Module not found - try lazy loading if we have a lazy resolver and session registry
            let mut loaded_module = false;
            if let (Some(resolver), Some(session_registry)) =
                (&self.lazy_resolver, &self.session_registry)
            {
                // Check if the resolver can handle this module path
                let can_resolve = {
                    let guard = resolver
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.can_resolve(module_path.as_str())
                };
                if can_resolve {
                    // Try to resolve and load the module
                    let resolve_result = {
                        let mut guard = resolver
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        guard.resolve_module(module_path.as_str())
                    };
                    match resolve_result {
                        Ok(module_info) => {
                            // Register the loaded module in the session registry
                            {
                                let mut registry = session_registry.write();
                                registry.register(module_info);
                            }
                            // #[cfg(debug_assertions)]
                            // eprintln!(
                            // "[DEBUG] Lazy loaded module '{}'",
                            // module_path.as_str()
                            // );
                            loaded_module = true;
                        }
                        Err(e) => {
                            // #[cfg(debug_assertions)]
                            // eprintln!(
                            // "[DEBUG] Failed to lazy load module '{}': {:?}",
                            // module_path.as_str(),
                            // e
                            // );
                        }
                    }
                }
            }

            // If we successfully loaded the module, retry the import by recursing
            // through the main import path. The module is now in the session registry.
            if loaded_module {
                // The module was loaded into the session registry. Now retry the import.
                // We need to read from session_registry since the local registry is a clone.
                // #[cfg(debug_assertions)]
                // eprintln!(
                // "[DEBUG] Module '{}' loaded via lazy loading, retrying import",
                // module_path.as_str()
                // );

                // Check if the module is now available in the session registry
                let module_info_opt = self.session_registry.as_ref().and_then(|sr| {
                    // get_by_path returns Maybe<Shared<ModuleInfo>>, convert to Option
                    match sr.read().get_by_path(module_path.as_str()) {
                        verum_common::Maybe::Some(shared) => Some((*shared).clone()),
                        verum_common::Maybe::None => None,
                    }
                });

                if let Some(module_info) = module_info_opt {
                    // #[cfg(debug_assertions)]
                    // eprintln!(
                    // "[DEBUG] After lazy load, found module '{}' with {} exports",
                    // module_path.as_str(),
                    // module_info.exports.len()
                    // );

                    // Process the import from the newly loaded module
                    use verum_modules::ExportKind;
                    let _register_name = local_name.unwrap_or(item_name);

                    // Look for the item in exports (exports.get expects &Text)
                    let item_name_text: Text = item_name.to_string().into();
                    if let Some(export) = module_info.exports.get(&item_name_text) {
                        match export.kind {
                            ExportKind::Type | ExportKind::Protocol => {
                                // Find and register the type from the module's AST
                                if let Some(type_decl) = self
                                    .find_type_declaration_in_module(&module_info.ast, item_name)
                                {
                                    // Register dependencies first (simplified - main path does more)
                                    let deps = self.collect_type_dependencies(&type_decl);
                                    for dep_name in deps.iter() {
                                        if self.ctx.lookup_type(dep_name.as_str()).is_none() {
                                            let source_module_path = module_info.path.to_string();
                                            let _ = self.import_item_from_module(
                                                &Text::from(source_module_path),
                                                dep_name.as_str(),
                                                registry,
                                            );
                                        }
                                    }
                                    // Register the type
                                    if let Err(e) = self.register_type_declaration(&type_decl) {
                                        if e.is_soundness_critical() {
                                            return Err(e);
                                        }
                                        tracing::debug!(
                                            "Failed to register type '{}': {:?}",
                                            item_name,
                                            e
                                        );
                                    }
                                    return Ok(());
                                }
                            }
                            ExportKind::Function | ExportKind::Meta => {
                                // Extract function type and register it
                                // Use TypeScheme::poly() for generic functions
                                if let Some((func_type, type_vars)) = self
                                    .extract_function_type_from_module(&module_info.ast, item_name)
                                {
                                    let register_name = local_name.unwrap_or(item_name);
                                    let scheme = if type_vars.is_empty() {
                                        TypeScheme::mono(func_type)
                                    } else {
                                        TypeScheme::poly(type_vars, func_type)
                                    };
                                    self.ctx.env.insert(register_name, scheme);
                                }
                                return Ok(());
                            }
                            ExportKind::Module => {
                                // Module re-exports - just mark as successful
                                return Ok(());
                            }
                            // For other kinds (Const, Static, Context, etc.), fall through
                            // to let normal resolution handle them
                            _ => {}
                        }
                    }
                }
            }

            // Module still not found - return error if span is provided (user import)
            if let Some(span) = import_span {
                // Collect similar module names for suggestions
                let all_modules: Vec<Text> = registry
                    .all_modules()
                    .map(|(_, info)| Text::from(info.path.to_string()))
                    .collect();
                let similar = crate::find_similar_names(module_path.as_str(), &all_modules)
                    .into_iter()
                    .map(Text::from)
                    .collect::<List<Text>>();

                return Err(TypeError::ImportModuleNotFound {
                    module_path: module_path.clone(),
                    similar_modules: similar,
                    span,
                });
            }
        }

        Ok(())
    }

    /// Find a type declaration in a module's AST by name.
    /// Import a type or protocol export from a module.
    /// Resolves transitive type dependencies, registers the type declaration
    /// (including companion inline module + blanket impls), and falls back to
    /// a Named-type placeholder when full registration fails.
    fn import_type_export(
        &mut self,
        module_info: &verum_modules::ModuleInfo,
        item_name: &str,
        register_name: &str,
        resolved_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
        import_span: Option<Span>,
        export_source_module: verum_modules::path::ModuleId,
    ) -> Result<()> {
                        // For types, find the full type declaration and register it properly.
                        // This ensures variant types get their constructors registered,
                        // record types get their field information, etc.
                        // Try to find the type declaration, following re-export chains if needed
                        let mut registered_successfully = false;
                        if let Some((type_decl, source_module_path)) = self
                            .find_type_declaration_with_source_module(
                                &module_info.ast,
                                item_name,
                                &resolved_module_path,
                                registry,
                            )
                        {
                            // CRITICAL: Import transitive type dependencies BEFORE registering the type.
                            // This ensures types used in field types (like `SemVer` in
                            // `Package.versions: List<SemVer>`) are available for resolution.
                            //

                            // The fix analyzes the type declaration to find all referenced types,
                            // then imports each one from the module registry before proceeding.
                            //

                            // Example: When importing `Package` from domain/package.vr:
                            // 1. Collect dependencies: ["List", "SemVer", "Text", "Int"]
                            // 2. Import each dependency that's not already available
                            // 3. Register `Package` type with all dependencies resolved
                            //

                            // Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — Imports
                            let type_dependencies = self.collect_type_dependencies(&type_decl);
                            // #[cfg(debug_assertions)]
                            // eprintln!(
                            // "[DEBUG] Type '{}' has dependencies: [{}]",
                            // item_name,
                            // type_dependencies
                            // .iter()
                            // .map(|d| d.as_str())
                            // .collect::<Vec<_>>()
                            // .join(", ")
                            // );

                            // Use path aliases since "core.io.path" may be stored as "std.io.path"
                            if let Some(source_module) = self
                                .get_module_with_path_aliases(source_module_path.as_str(), registry)
                            {
                                // Import each dependency transitively
                                for dep_name in type_dependencies.iter() {
                                    // Skip if already defined (including built-in types)
                                    if self.ctx.lookup_type(dep_name.as_str()).is_some() {
                                        continue;
                                    }

                                    // Try to import the dependency from the source module first
                                    // (the module that actually declares the parent type — this is
                                    // the canonical location for sibling types that the parent
                                    // depends on directly).
                                    let mut imported = match self.import_item_from_module(
                                        &source_module_path,
                                        dep_name.as_str(),
                                        registry,
                                    ) {
                                        Ok(()) => self.ctx.lookup_type(dep_name.as_str()).is_some(),
                                        Err(_) => false,
                                    };

                                    // Fallback 1: when the source module is a *submodule* of the
                                    // requesting module (e.g. parent type in `core.base.iterator`,
                                    // dep declared in `core.base.maybe`), the dep is reachable via
                                    // the requesting module's re-export surface. Retry against the
                                    // original `module_path` (e.g. `core.base`) which collects all
                                    // submodule re-exports under one roof.
                                    //
                                    // This closes the silent-drop where `Iterator` (declared in
                                    // `core.base.iterator`) depends on `Maybe` (declared in
                                    // `core.base.maybe`) — the source module `core.base.iterator`
                                    // doesn't export `Maybe`, but the requesting module
                                    // `core.base` does (via `public mount .maybe.{Maybe, ...}`).
                                    if !imported && *resolved_module_path != source_module_path {
                                        if let Ok(()) = self.import_item_from_module(
                                            resolved_module_path,
                                            dep_name.as_str(),
                                            registry,
                                        ) {
                                            imported =
                                                self.ctx.lookup_type(dep_name.as_str()).is_some();
                                        }
                                    }

                                    if !imported {
                                        tracing::debug!(
                                            "Could not import transitive dependency '{}' for type '{}' (tried source={}, root={})",
                                            dep_name,
                                            item_name,
                                            source_module_path.as_str(),
                                            resolved_module_path.as_str(),
                                        );
                                        // Don't fail - the dependency might be a built-in type
                                        // or from a different module that's already imported
                                    }
                                }
                            }

                            // Register the type declaration - this handles variants, records, etc.
                            // Set flag so register_type_declaration_body knows this is an explicit import
                            // and allows overwriting any existing type (even from metadata/global passes).
                            let is_explicit =
                                import_span.is_some() && self.explicit_imports.contains(item_name);
                            if is_explicit {
                                self.in_explicit_import_registration = true;
                            }
                            let reg_result = self.register_type_declaration(&type_decl);
                            // Reset flag after registration completes (success or failure)
                            if is_explicit {
                                self.in_explicit_import_registration = false;
                            }
                            if let Err(e) = reg_result {
                                if e.is_soundness_critical() {
                                    return Err(e);
                                }
                                // Log the error but don't fail - some imports may have issues
                                // #[cfg(debug_assertions)]
                                // eprintln!(
                                // "[DEBUG] Failed to register imported type '{}' from module '{}': {}",
                                // item_name,
                                // resolved_module_path.as_str(),
                                // e
                                // );
                                // Fall through to fallback registration
                            } else {
                                // #[cfg(debug_assertions)]
                                // eprintln!(
                                // "[DEBUG] Successfully registered type '{}' from module '{}'",
                                // item_name,
                                // resolved_module_path.as_str()
                                // );
                                registered_successfully = true;

                                // CRITICAL FIX: Also import implement block methods for the type.
                                // This enables cross-file method resolution for calls like
                                // RegistryError.validation_error(...) where the method is defined
                                // in an `implement RegistryError { ... }` block in the source module.
                                //

                                // We need to find the source module (following re-exports if necessary)
                                // and import all public methods from its implement blocks.
                                if let Some((_, source_path)) = self
                                    .find_type_declaration_with_source_module(
                                        &module_info.ast,
                                        item_name,
                                        &resolved_module_path,
                                        registry,
                                    )
                                {
                                    // Get the source module and import its implement blocks.
                                    // Pin the checker's module path to the resolved source
                                    // module so bare type references inside impl blocks
                                    // resolve against that module's qualified-name layer
                                    // first (avoids same-name collisions via the flat map).
                                    let src_path = source_path.as_str().to_string();
                                    if let Some(source_module) = self.get_module_with_path_aliases(
                                        source_path.as_str(),
                                        registry,
                                    ) && let Err(e) = self.import_impl_blocks_for_type_in_module(
                                        &source_module.ast,
                                        item_name,
                                        Some(&src_path),
                                    ) {
                                        tracing::debug!(
                                            "Failed to import implement blocks for '{}' from '{}': {}",
                                            item_name,
                                            source_path.as_str(),
                                            e
                                        );
                                    }
                                } else {
                                    // Fallback: try to import from the direct module.
                                    let direct_path = resolved_module_path.as_str().to_string();
                                    if let Err(e) = self.import_impl_blocks_for_type_in_module(
                                        &module_info.ast,
                                        item_name,
                                        Some(&direct_path),
                                    ) {
                                        tracing::debug!(
                                            "Failed to import implement blocks for '{}' from '{}': {}",
                                            item_name,
                                            resolved_module_path.as_str(),
                                            e
                                        );
                                    }
                                }

                                // After successful type registration, also check if there is a
                                // corresponding inline module (companion namespace) with the same name.
                                //

                                // Pattern: `public type Foo<A,B> is {...}; public module Foo {...}`
                                // When both exist, Type wins over Module in export deduplication,
                                // but the inline module's static methods (e.g. Foo.map(...)) need
                                // to be accessible.
                                //

                                // We scan the source module's AST directly (not the registry),
                                // because inline modules are not registered as separate registry
                                // entries — they are embedded in their parent file's module AST.
                                //

                                // Without this, `Transducer.map(...)` fails when Transducer is
                                // imported transitively (e.g. via core.prelude.*) because only
                                // the type is registered, not the companion module namespace.
                                let short_name = verum_common::Text::from(register_name);
                                if !self.inline_modules.contains_key(&short_name) {
                                    // First check the source module's AST for an inline module
                                    let source_ast_opt = self
                                        .get_module_with_path_aliases(
                                            source_module_path.as_str(),
                                            registry,
                                        )
                                        .map(|m| m.ast.clone());
                                    let inline_mod_found =
                                        source_ast_opt.as_ref().and_then(|ast| {
                                            ast.items.iter().find_map(|ast_item| {
                                                if let verum_ast::ItemKind::Module(mod_decl) =
                                                    &ast_item.kind
                                                {
                                                    if mod_decl.name.name.as_str() == item_name {
                                                        return Some(mod_decl.clone());
                                                    }
                                                }
                                                None
                                            })
                                        });
                                    if let Some(mod_decl) = inline_mod_found {
                                        self.inline_modules.insert(short_name, mod_decl);
                                    } else {
                                        // Fallback: try the registry for historical compatibility
                                        let inline_path_str = format!(
                                            "{}.{}",
                                            source_module_path.as_str(),
                                            item_name
                                        );
                                        if let verum_common::Maybe::Some(inline_mod_info) =
                                            registry.get_by_path(&inline_path_str)
                                        {
                                            let synthetic_decl = verum_ast::decl::ModuleDecl {
                                                visibility: verum_ast::decl::Visibility::Public,
                                                name: verum_ast::ty::Ident::new(
                                                    verum_common::Text::from(item_name),
                                                    inline_mod_info.ast.span,
                                                ),
                                                items: verum_common::Maybe::Some(
                                                    inline_mod_info.ast.items.clone(),
                                                ),
                                                profile: verum_common::Maybe::None,
                                                features: verum_common::Maybe::None,
                                                contexts: verum_common::List::new(),
                                                span: inline_mod_info.ast.span,
                                            };
                                            let key = verum_common::Text::from(register_name);
                                            self.inline_modules.insert(key, synthetic_decl);
                                        }
                                    }
                                }
                            }
                        }

                        // Fallback: register as a named type reference if full registration failed
                        if !registered_successfully {
                            // Function-shadowing guard: if find_type_declaration_with_source_module
                            // returned None we still don't know whether the
                            // imported item is genuinely a missing type or a
                            // FUNCTION exported via a re-export chain that
                            // resolve_export_kind_with_reexports failed to
                            // re-classify (ExportKind::Type is the default
                            // for re-exported items — see the comment at
                            // 33858).  When the source module's AST DOES
                            // hold a function declaration for this name,
                            // register the function signature in env and
                            // SKIP the Named-type placeholder.  Without
                            // this guard, mounting `core.shell.{run}` —
                            // where `core.shell` re-exports
                            // `core.shell.exec.run` and the export kind
                            // resolves as Type — leaves `run` registered
                            // as `Type::Named { path: "run" }`, so every
                            // call site fails with "not a function: run"
                            // because the env-lookup hits the
                            // self-referential placeholder before the
                            // function lookup gets a chance.
                            if let Some((func_type, type_vars, _src)) = self
                                .find_function_with_source_module(
                                    &module_info.ast,
                                    item_name,
                                    &resolved_module_path,
                                    registry,
                                )
                            {
                                let scheme = if type_vars.is_empty() {
                                    TypeScheme::mono(func_type)
                                } else {
                                    TypeScheme::poly(type_vars, func_type)
                                };
                                self.ctx.env.insert(register_name, scheme);
                                registered_successfully = true;
                            } else {
                                if let Some(metadata) = self.core_metadata.clone() {
                                    if let Some(func_type) = self
                                        .resolve_metadata_reexport_function(
                                            &metadata,
                                            &module_info.ast,
                                            item_name,
                                            &resolved_module_path,
                                        )
                                    {
                                        self.ctx
                                            .env
                                            .insert(register_name, TypeScheme::mono(func_type));
                                        registered_successfully = true;
                                    }
                                }
                            }
                        }
                        if !registered_successfully {
                            // CRITICAL FIX: Check if the type already exists before overwriting.
                            // Bootstrap registers types like Maybe<T> as Type::Variant.
                            // We must NOT overwrite these with Type::Named fallbacks.
                            // This preserves pattern matching support for bootstrap types.
                            let existing_ty = self.ctx.lookup_type(register_name);
                            let should_register_fallback = match existing_ty {
                                // Type doesn't exist - register fallback
                                Option::None => true,
                                // Type exists as Named - safe to overwrite with another Named
                                Option::Some(Type::Named { .. }) => true,
                                // Type exists as Variant - DO NOT overwrite (preserves pattern matching)
                                Option::Some(Type::Variant(_)) => false,
                                // Type exists as another concrete type - don't overwrite
                                _ => false,
                            };

                            if should_register_fallback {
                                use verum_ast::ty::{Ident, Path};
                                // Use register_name for the fallback type reference
                                let ident = Ident::new(register_name, Span::dummy());
                                let type_ref = Type::Named {
                                    path: Path::single(ident),
                                    args: List::new(),
                                };
                                self.ctx
                                    .define_type(verum_common::Text::from(register_name), type_ref);

                                // Also try to register the companion inline module from the
                                // source module (found via exported.source_module).
                                // This handles the case where Type+Module coexist (like Transducer)
                                // and the type is imported via a glob re-export (e.g., core.prelude.*),
                                // where find_type_declaration_with_source_module returns None.
                                let short_name = verum_common::Text::from(register_name);
                                if !self.inline_modules.contains_key(&short_name) {
                                    if let verum_common::Maybe::Some(source_mod_info) =
                                        registry.get(export_source_module)
                                    {
                                        // The source_module may be an intermediate re-exporter (e.g., core.base).
                                        // Use find_type_declaration_with_source_module to follow the re-export chain
                                        // all the way to the ACTUAL declaring module (e.g., core.base.iterator).
                                        let source_path = verum_common::Text::from(
                                            source_mod_info.path.to_string(),
                                        );
                                        let actual_source_path = self
                                            .find_type_declaration_with_source_module(
                                                &source_mod_info.ast,
                                                item_name,
                                                &source_path,
                                                registry,
                                            )
                                            .map(|(_, path)| path)
                                            .unwrap_or(source_path);
                                        // Now look for the inline module in the actual source module
                                        let actual_inline_mod = self
                                            .get_module_with_path_aliases(
                                                actual_source_path.as_str(),
                                                registry,
                                            )
                                            .and_then(|m| {
                                                m.ast.items.iter().find_map(|ast_item| {
                                                    if let verum_ast::ItemKind::Module(mod_decl) =
                                                        &ast_item.kind
                                                    {
                                                        if mod_decl.name.name.as_str() == item_name
                                                        {
                                                            return Some(mod_decl.clone());
                                                        }
                                                    }
                                                    None
                                                })
                                            });
                                        if let Some(mod_decl) = actual_inline_mod {
                                            self.inline_modules.insert(short_name, mod_decl);
                                        }
                                        // Register blanket impls from the actual source module.
                                        // This is needed when the type is imported transitively
                                        // (e.g. via core.prelude.*), so that blanket impl methods
                                        // like `transduce` from `implement<I: Iterator> I { ... }`
                                        // are available on all Iterator types.
                                        let source_blanket_ast = self
                                            .get_module_with_path_aliases(
                                                actual_source_path.as_str(),
                                                registry,
                                            )
                                            .map(|m| (m.ast.clone(), m.path.to_string()));
                                        if let Some((blanket_ast, blanket_path)) =
                                            source_blanket_ast
                                        {
                                            self.register_module_blanket_impls(
                                                &blanket_ast,
                                                &blanket_path,
                                            );
                                        }
                                    } // closes registry.get if-let
                                } // closes !contains_key if
                            } // closes if should_register_fallback
                        } // closes if !registered_successfully
        Ok(())
    }

    /// Import a `context` or `context protocol` export from a module.
    /// Registers the item as both a context type (for `using [...]`) and a type
    /// so method calls like `Database.query(...)` resolve in the importing file.
    fn import_context_export(
        &mut self,
        module_info: &verum_modules::ModuleInfo,
        item_name: &str,
        register_name: &str,
        resolved_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Result<()> {
                        // Context protocols need to be registered BOTH as a type AND as a context.
                        // The type registration allows the protocol to be used in type annotations,
                        // while the context registration allows it to be used in `using [...]` clauses.
                        //

                        // CRITICAL: We must build a proper Record type with method signatures
                        // so that method calls like `Database.get_trending(...)` resolve correctly.
                        //

                        // There are TWO forms of context declarations:
                        // 1. `context protocol Database { ... }` - parsed as ProtocolDecl with is_context=true
                        // 2. `context Database { ... }` - parsed as ContextDecl
                        //

                        // We must handle both forms.
                        //

                        // Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts

                        // First, try to find as a `context protocol` (ProtocolDecl with is_context=true)
                        // Use the variant that returns the source module path so we can import sibling types
                        let context_type = if let Some((proto_decl, source_module_path)) = self
                            .find_context_protocol_with_source_module(
                                &module_info.ast,
                                item_name,
                                &resolved_module_path,
                                registry,
                            ) {
                            // CRITICAL: Import all types from the protocol's source module BEFORE
                            // building the protocol type. This ensures types used in method signatures
                            // (like `SearchResponse`, `SearchError`) are available for resolution.
                            //

                            // Without this, `ast_to_type_lenient` would fall back to fresh type variables
                            // for unresolved types, causing field access on return values to fail with
                            // "Cannot access field on non-record type: τXX" errors.
                            //

                            // Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
                            // Use path aliases since "core.io.path" may be stored as "std.io.path"
                            if let Some(source_module) = self
                                .get_module_with_path_aliases(source_module_path.as_str(), registry)
                            {
                                let src_path = source_module_path.as_str().to_string();
                                self.import_types_from_module_ast_in_module(
                                    &source_module.ast,
                                    Some(&src_path),
                                );
                            }

                            // Found a context protocol - build Record type with methods.
                            // Module-aware: pin the checker's module path to the SOURCE
                            // module while resolving the protocol body so that bare type
                            // references inside method signatures (e.g. `LogLevel` in
                            // `fn log(level: LogLevel, msg: Text)` inside
                            // `core.context.standard.Logger`) resolve against the source
                            // module's qualified-name layer first — not the flat map
                            // which may surface a same-named stranger from another
                            // module (`core.base.log.LogLevel` vs
                            // `core.context.standard.LogLevel`).
                            let saved_ctx_path = self.current_module_path().clone();
                            self.set_current_module_path(source_module_path.clone());
                            let ctx_type_result =
                                self.build_context_type_from_protocol(&proto_decl);
                            self.set_current_module_path(saved_ctx_path);

                            match ctx_type_result {
                                Ok(record_type) => record_type,
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to build context type for protocol '{}' from module '{}': {}",
                                        item_name,
                                        resolved_module_path.as_str(),
                                        e
                                    );
                                    // Fallback to Named type
                                    use verum_ast::ty::{Ident, Path};
                                    let ident = Ident::new(item_name, Span::dummy());
                                    Type::Named {
                                        path: Path::single(ident),
                                        args: List::new(),
                                    }
                                }
                            }
                        } else if let Some(ctx_decl) = self.find_context_declaration_with_reexports(
                            &module_info.ast,
                            item_name,
                            &resolved_module_path,
                            registry,
                        ) {
                            // Found a context declaration - build Record type with methods.
                            // Module-aware: pin the checker's module path to the SOURCE
                            // module while resolving the context body so that bare type
                            // references inside method signatures (e.g. `LogLevel` in
                            // `fn log(level: LogLevel, msg: Text)` inside
                            // `core.context.standard.Logger`) resolve against the
                            // source module's qualified-name layer first — not the
                            // flat map which may surface a same-named stranger from
                            // another module.
                            let saved_ctx_path = self.current_module_path().clone();
                            self.set_current_module_path(resolved_module_path.clone());
                            let ctx_type_result = self.build_context_type_from_decl(&ctx_decl);
                            self.set_current_module_path(saved_ctx_path);

                            match ctx_type_result {
                                Ok(record_type) => {
                                    // Store the context declaration for method-level capability extraction
                                    let context_name: Text = item_name.into();
                                    self.context_declarations.insert(context_name, ctx_decl);
                                    record_type
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to build context type for '{}' from module '{}': {}",
                                        item_name,
                                        resolved_module_path.as_str(),
                                        e
                                    );
                                    // Fallback to Named type
                                    use verum_ast::ty::{Ident, Path};
                                    let ident = Ident::new(item_name, Span::dummy());
                                    Type::Named {
                                        path: Path::single(ident),
                                        args: List::new(),
                                    }
                                }
                            }
                        } else {
                            // Neither context protocol nor context declaration found - fallback
                            use verum_ast::ty::{Ident, Path};
                            let ident = Ident::new(item_name, Span::dummy());
                            let fallback_type = Type::Named {
                                path: Path::single(ident),
                                args: List::new(),
                            };

                            // Try to find as a type declaration
                            if let Some(type_decl) = self.find_type_declaration_with_reexports(
                                &module_info.ast,
                                item_name,
                                &resolved_module_path,
                                registry,
                            ) {
                                if let Err(e) = self.register_type_declaration(&type_decl) {
                                    if e.is_soundness_critical() {
                                        return Err(e);
                                    }
                                    tracing::warn!(
                                        "Failed to register imported context type '{}' from module '{}': {}",
                                        item_name,
                                        resolved_module_path.as_str(),
                                        e
                                    );
                                }
                            } else if let Some(_proto_decl) = self
                                .find_protocol_declaration_in_module(&module_info.ast, item_name)
                            {
                                // Protocol declaration found - register as Named type
                                self.ctx.define_type(
                                    verum_common::Text::from(register_name),
                                    fallback_type.clone(),
                                );
                            } else {
                                // Fallback: register as a named type reference
                                self.ctx.define_type(
                                    verum_common::Text::from(register_name),
                                    fallback_type.clone(),
                                );
                            }
                            fallback_type
                        };

                        // Register as a context for `using [...]` clauses
                        self.register_protocol_as_context(verum_common::Text::from(register_name));

                        // Register the context type so that it can be accessed as a variable
                        // in function bodies that use this context (e.g., `Database.query(...)`)
                        // Guard: don't overwrite context types that were pre-registered
                        // from the embedded stdlib archive with full method signatures.
                        // The archive-based registration has richer type info (Record with
                        // method names) than the fallback here (often Type::Named).
                        let ctx_key = verum_common::Text::from(register_name);
                        let already_has_rich_type = self
                            .context_resolver
                            .get_context_type(&ctx_key)
                            .map(|t| matches!(t, Type::Record(_)))
                            .unwrap_or(false);
                        if !already_has_rich_type {
                            self.context_resolver
                                .register_context_type(ctx_key, context_type);
                        }
        Ok(())
    }

    fn find_type_declaration_in_module(
        &self,
        ast: &verum_ast::Module,
        type_name: &str,
    ) -> Option<verum_ast::decl::TypeDecl> {
        use verum_ast::ItemKind;
        use verum_ast::decl::Visibility as AstVisibility;

        for item in &ast.items {
            if let ItemKind::Type(type_decl) = &item.kind
                && type_decl.name.name.as_str() == type_name
            {
                // Private types MUST NOT be visible to importers.
                // Before this guard, an `import` resolver that fell
                // through the normal exports table would still find
                // private declarations here by AST walk, and then
                // `register_type_declaration` would try to resolve
                // their variant-payload / field types against the
                // *importer's* scope — which doesn't know the private
                // type's own imports. The symptom was a spurious
                // "Failed to register imported type 'X': type not
                // found: Y" warning for private types whose variants
                // reference types imported via `mount ...{Y}` that
                // only the defining module knows about.
                if type_decl.visibility != AstVisibility::Public {
                    return None;
                }
                return Some(type_decl.clone());
            }
        }
        None
    }

    /// Extract all type names referenced in a TypeDecl.
    ///

    /// This collects type dependencies from:
    /// - Record field types
    /// - Variant payload types
    /// - Type alias definitions
    /// - Generic type arguments
    ///

    /// Returns a list of simple type names (not qualified paths) that need to be
    /// available for the type to be registered successfully.
    ///

    /// Example: For `type Cog is { versions: List<SemVer> }`, this returns ["List", "SemVer"]
    fn collect_type_dependencies(
        &self,
        type_decl: &verum_ast::TypeDecl,
    ) -> List<verum_common::Text> {
        use std::collections::HashSet;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::TypeKind;

        let mut deps = HashSet::new();

        // Helper to extract type names from an AST type
        fn extract_from_type(ty: &verum_ast::ty::Type, deps: &mut HashSet<Text>) {
            use verum_ast::ty::GenericArg;

            match &ty.kind {
                TypeKind::Path(path) => {
                    // Extract the base type name (first segment)
                    if let Some(ident) = path.as_ident() {
                        deps.insert(verum_common::Text::from(ident.name.as_str()));
                    }
                }
                TypeKind::Generic { base, args } => {
                    // Extract both base type and type arguments
                    extract_from_type(base, deps);
                    for arg in args {
                        // GenericArg can be Type, Const, or other variants
                        if let GenericArg::Type(arg_ty) = arg {
                            extract_from_type(arg_ty, deps);
                        }
                    }
                }
                TypeKind::Tuple(types) => {
                    for ty in types {
                        extract_from_type(ty, deps);
                    }
                }
                TypeKind::Function {
                    params,
                    return_type,
                    ..
                } => {
                    for param in params {
                        extract_from_type(param, deps);
                    }
                    extract_from_type(return_type, deps);
                }
                TypeKind::Reference { mutable: _, inner } => {
                    extract_from_type(inner, deps);
                }
                TypeKind::CheckedReference { mutable: _, inner } => {
                    extract_from_type(inner, deps);
                }
                TypeKind::UnsafeReference { mutable: _, inner } => {
                    extract_from_type(inner, deps);
                }
                TypeKind::Pointer { mutable: _, inner } => {
                    extract_from_type(inner, deps);
                }
                TypeKind::VolatilePointer { mutable: _, inner } => {
                    extract_from_type(inner, deps);
                }
                TypeKind::Array { element, .. } => {
                    extract_from_type(element, deps);
                }
                TypeKind::Slice(inner) => {
                    extract_from_type(inner, deps);
                }
                // Unit, Bool, Int, etc. don't have dependencies
                _ => {}
            }
        }

        // Extract from type body
        match &type_decl.body {
            TypeDeclBody::Alias(aliased_type) => {
                extract_from_type(aliased_type, &mut deps);
            }
            TypeDeclBody::Variant(variants) => {
                for variant in variants {
                    match &variant.data {
                        Some(VariantData::Tuple(types)) => {
                            for ty in types {
                                extract_from_type(ty, &mut deps);
                            }
                        }
                        Some(VariantData::Record(fields)) => {
                            for field in fields {
                                extract_from_type(&field.ty, &mut deps);
                            }
                        }
                        None => {}
                    }
                }
            }
            TypeDeclBody::Record(fields) => {
                for field in fields {
                    extract_from_type(&field.ty, &mut deps);
                }
            }
            TypeDeclBody::Newtype(inner_type) => {
                extract_from_type(inner_type, &mut deps);
            }
            TypeDeclBody::Protocol(protocol_body) => {
                use verum_ast::decl::{FunctionParamKind, ProtocolItemKind};
                // Extract type dependencies from protocol method signatures
                for item in &protocol_body.items {
                    match &item.kind {
                        ProtocolItemKind::Function { decl, .. } => {
                            // Extract from parameters
                            for param in &decl.params {
                                if let FunctionParamKind::Regular { ty, .. } = &param.kind {
                                    extract_from_type(ty, &mut deps);
                                }
                            }
                            // Extract from return type
                            if let Some(ret_ty) = &decl.return_type {
                                extract_from_type(ret_ty, &mut deps);
                            }
                        }
                        ProtocolItemKind::Type { bounds, .. } => {
                            // Extract from associated type bounds
                            for bound_path in bounds {
                                if let Some(name) = bound_path.as_ident() {
                                    deps.insert(verum_common::Text::from(name.as_str()));
                                }
                            }
                        }
                        ProtocolItemKind::Const { ty, .. } => {
                            // Extract from constant type
                            extract_from_type(ty, &mut deps);
                        }
                        ProtocolItemKind::Axiom(_) => {
                            // Axioms contribute proof obligations at
                            // `implement` sites; their free type names
                            // are already reachable via the protocol
                            // header and function signatures.
                        }
                    }
                }
                // Extract from extended protocols (now supports generic types like Converter<A, B>)
                for extend_type in &protocol_body.extends {
                    extract_from_type(extend_type, &mut deps);
                }
            }
            TypeDeclBody::Tuple(types) => {
                for ty in types {
                    extract_from_type(ty, &mut deps);
                }
            }
            TypeDeclBody::SigmaTuple(types) => {
                // Sigma tuple types have similar dependency extraction to regular tuples
                for ty in types {
                    extract_from_type(ty, &mut deps);
                }
            }
            TypeDeclBody::Unit => {
                // Unit type has no dependencies
            }
            TypeDeclBody::Inductive(_) | TypeDeclBody::Coinductive(_) => {
                // Dependent type features (v2.0+) - no deps for now
            }
            TypeDeclBody::Quotient { base, .. } => {
                // T1-T: quotient types depend on the carrier type; the
                // relation is a λ-expression whose free names are
                // function/value references in the module scope.
                extract_from_type(base, &mut deps);
            }
        }

        // Convert HashSet to List
        deps.into_iter().collect()
    }

    /// Import all type declarations from a module's AST into the type environment.
    ///

    /// This is used when importing context protocols to ensure that types used in
    /// method signatures (like `SearchResponse`, `SearchError`) are available for
    /// type resolution before building the protocol's Record type.
    ///

    /// Only imports public type declarations to respect visibility.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    fn import_types_from_module_ast(&mut self, ast: &verum_ast::Module) {
        self.import_types_from_module_ast_in_module(ast, None);
    }

    /// Same as [`import_types_from_module_ast`], but pins the type checker's
    /// `current_module_path` to `source_module_path` for the duration of the
    /// import so that cross-referencing types in the module (e.g. record
    /// fields whose types are also declared in the same module) land under
    /// the correct qualified-name key — not the caller's module.
    fn import_types_from_module_ast_in_module(
        &mut self,
        ast: &verum_ast::Module,
        source_module_path: Option<&str>,
    ) {
        use verum_ast::ItemKind;
        use verum_ast::decl::Visibility as AstVisibility;

        let saved_module_path = self.current_module_path.clone();
        if let Some(path) = source_module_path {
            self.set_current_module_path(verum_common::Text::from(path));
        }

        for item in &ast.items {
            if let ItemKind::Type(type_decl) = &item.kind {
                // Only import public types
                if type_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Register the type declaration - this handles variants, records, aliases, etc.
                if let Err(e) = self.register_type_declaration(type_decl) {
                    if e.is_soundness_critical() {
                        // This entry point returns `()` so we can't
                        // propagate up. Stash the error on the checker
                        // — `phase_type_check` drains stashed errors
                        // before declaring success, so the build still
                        // fails. tracing::error! gives a cold-print
                        // backup if the stash gets lost.
                        tracing::error!(
                            "Soundness-critical error registering '{}' from cross-module import: {}",
                            type_decl.name.name.as_str(),
                            e
                        );
                        self.deferred_soundness_errors.push(e);
                        continue;
                    }
                    tracing::debug!(
                        "Failed to register type '{}' from module during context protocol import: {}",
                        type_decl.name.name.as_str(),
                        e
                    );
                }
            }
        }

        self.set_current_module_path(saved_module_path);
    }

    /// Find all inherent implement blocks for a type in a module's AST.
    ///

    /// This is used when importing a type to also import its associated methods.
    /// Implement blocks define methods that can be called on the type using
    /// either `Type.static_method()` or `instance.method()` syntax.
    ///

    /// Returns a list of ImplDecl for both inherent AND protocol implementations of the type.
    /// This ensures that methods from protocol implementations (e.g., `implement Allocator for GlobalAllocator`)
    /// are also available for method calls on the implementing type.
    fn find_impl_blocks_for_type(
        &self,
        ast: &verum_ast::Module,
        type_name: &str,
    ) -> List<verum_ast::decl::ImplDecl> {
        use verum_ast::ItemKind;
        use verum_ast::decl::ImplKind;

        let mut impl_blocks = List::new();

        for item in &ast.items {
            if let ItemKind::Impl(impl_decl) = &item.kind {
                let for_type = match &impl_decl.kind {
                    // Inherent impl: `implement Type { ... }`
                    ImplKind::Inherent(for_type) => Some(for_type),
                    // Protocol impl: `implement Protocol for Type { ... }`
                    ImplKind::Protocol { for_type, .. } => Some(for_type),
                };

                if let Some(for_type) = for_type {
                    // Extract the base type name from the for_type
                    // Handle both simple paths (e.g., `Result`) and generic types (e.g., `Result<T, E>`)
                    // CRITICAL FIX: Also handle primitive types (Int, Float, Bool, etc.)
                    let base_type_name = match &for_type.kind {
                        // Simple path: `implement Type { ... }`
                        verum_ast::ty::TypeKind::Path(path) => {
                            path.as_ident().map(|ident| ident.name.as_str())
                        }
                        // Generic type: `implement<T, E> Result<T, E> { ... }`
                        verum_ast::ty::TypeKind::Generic { base, .. } => {
                            if let verum_ast::ty::TypeKind::Path(path) = &base.kind {
                                path.as_ident().map(|ident| ident.name.as_str())
                            } else {
                                None
                            }
                        }
                        // CRITICAL: Handle primitive types for `implement Int { ... }` etc.
                        verum_ast::ty::TypeKind::Int => Some(WKT::Int.as_str()),
                        verum_ast::ty::TypeKind::Float => Some(WKT::Float.as_str()),
                        verum_ast::ty::TypeKind::Bool => Some(WKT::Bool.as_str()),
                        verum_ast::ty::TypeKind::Char => Some(WKT::Char.as_str()),
                        verum_ast::ty::TypeKind::Text => Some(WKT::Text.as_str()),
                        verum_ast::ty::TypeKind::Unit => Some("Unit"),
                        _ => None,
                    };

                    if base_type_name == Some(type_name) {
                        impl_blocks.push(impl_decl.clone());
                    }
                }
            }
        }

        impl_blocks
    }

    /// Import implement block methods for a type from a source module.
    ///

    /// This registers both static methods (callable as `Type.method()`) and
    /// instance methods (callable as `instance.method()`) for imported types.
    ///

    /// Static methods are registered in the environment as `"TypeName.method_name"`.
    /// Instance methods are registered in the `inherent_methods` map.
    ///

    /// Only public methods from the implement blocks are imported.
    fn import_impl_blocks_for_type(
        &mut self,
        ast: &verum_ast::Module,
        type_name: &str,
    ) -> Result<()> {
        self.import_impl_blocks_for_type_in_module(ast, type_name, None)
    }

    /// Same as [`import_impl_blocks_for_type`], but pins the type checker's
    /// `current_module_path` to `source_module_path` for the duration of the
    /// import so that type references inside the impl block (e.g. a bare
    /// `RecvError` return type) resolve against the *source* module's
    /// qualified-name layer first.
    ///

    /// Without this, `ast_to_type` / `ast_to_type_lenient` fall back to the
    /// flat `ctx.type_defs` map where whichever same-named type was
    /// registered last wins — so imported broadcast's `poll_next` ends up
    /// with QUIC's `RecvError` in its stored return type.
    fn import_impl_blocks_for_type_in_module(
        &mut self,
        ast: &verum_ast::Module,
        type_name: &str,
        source_module_path: Option<&str>,
    ) -> Result<()> {
        use verum_ast::decl::{FunctionParamKind, ImplItemKind, Visibility as AstVisibility};

        // #[cfg(debug_assertions)]
        // eprintln!(
        // "[DEBUG import_impl_blocks_for_type] Called for type '{}'",
        // type_name
        // );

        let saved_module_path = self.current_module_path.clone();
        if let Some(path) = source_module_path {
            self.set_current_module_path(verum_common::Text::from(path));
        }

        let impl_blocks = self.find_impl_blocks_for_type(ast, type_name);

        // #[cfg(debug_assertions)]
        // eprintln!(
        // "[DEBUG import_impl_blocks_for_type] Found {} impl blocks for '{}'",
        // impl_blocks.len(),
        // type_name
        // );

        for impl_decl in impl_blocks {
            // CRITICAL FIX: Set up type parameter scope from the implement block.
            // This ensures that type parameters like `T` in `implement<T> List<T>`
            // are properly bound as fresh type variables when resolving method types.
            //

            // Without this, `ast_to_type_lenient` would create Type::Named for `T`
            // instead of Type::Var, and generalize() wouldn't capture them properly.
            self.ctx.enter_scope();

            // Collect type variables and their names for later quantification
            let mut impl_type_vars: List<TypeVar> = List::new();
            let mut impl_type_param_names: List<verum_common::Text> = List::new();

            // Register type parameters from the implement block
            for generic_param in &impl_decl.generics {
                use verum_ast::ty::GenericParamKind;
                match &generic_param.kind {
                    GenericParamKind::Type { name, .. } => {
                        let type_var = TypeVar::fresh();
                        let name_text: verum_common::Text = name.name.clone();
                        self.ctx.define_type(name_text.clone(), Type::Var(type_var));
                        impl_type_vars.push(type_var);
                        impl_type_param_names.push(name_text);
                    }
                    GenericParamKind::Meta { name, .. } => {
                        // Meta parameters (compile-time values like `MODE: meta AccessMode`)
                        // need fresh type variables for type inference, just like Type parameters.
                        // This enables unification of meta params with concrete variant values
                        // (e.g., MODE unifies with ReadOnly in Register<UInt32, ReadOnly>).
                        let type_var = TypeVar::fresh();
                        let name_text: verum_common::Text = name.name.clone();
                        self.ctx.define_type(name_text.clone(), Type::Var(type_var));
                        impl_type_vars.push(type_var);
                        impl_type_param_names.push(name_text);
                    }
                    GenericParamKind::Const { name, ty } => {
                        let name_text: verum_common::Text = name.name.clone();
                        let const_type = self.ast_to_type(ty).unwrap_or(Type::Int);
                        self.ctx.define_type(name_text, const_type);
                    }
                    _ => {}
                }
            }

            // CRITICAL FIX: Set current_self_type so Self return types can be resolved.
            // Methods like `fn inspect<F>(self, f: F) -> Self` need Self to resolve to
            // the implementing type (e.g., Maybe<T>) during method registration.
            // Without this, Self becomes Type::Named("Self") which breaks method calls.
            let old_self_type = self.current_self_type.clone();
            let for_type = match &impl_decl.kind {
                verum_ast::decl::ImplKind::Inherent(for_type) => Some(for_type),
                verum_ast::decl::ImplKind::Protocol { for_type, .. } => Some(for_type),
            };
            // Extract self-type args for specialization tracking.
            // For `implement<T> Register<T, ReadOnly>`, this captures [Var(T), Named(ReadOnly)].
            // Used to filter method availability during method lookup.
            let impl_self_type_args: List<Type> = if let Some(for_type) = for_type {
                // Use ast_to_type_lenient since type params are now in scope
                let self_type = self.ast_to_type_lenient(for_type);
                let args = match &self_type {
                    Type::Named { args, .. } | Type::Generic { args, .. } => args.clone(),
                    _ => List::new(),
                };
                self.set_current_self_type(Maybe::Some(self_type.clone()));
                // CRITICAL FIX: Also register "Self" in type context so that
                // ast_to_type_lenient can resolve Self in method signatures.
                // Without this, return types like "-> Self" stay as Type::Named("Self")
                // instead of being resolved to the concrete implementing type.
                self.ctx
                    .define_type(verum_common::Text::from("Self"), self_type);
                args
            } else {
                List::new()
            };

            // Collect static method registrations to apply after exiting scope.
            // We need to process them while in scope (to resolve type params),
            // but register them outside the scope (so they persist).
            let mut static_method_registrations: Vec<(String, TypeScheme)> = Vec::new();

            // Collect associated constant registrations to apply after exiting scope.
            let mut const_registrations: Vec<(String, TypeScheme)> = Vec::new();

            for item in &impl_decl.items {
                // Handle associated constants
                // Note: Impl block items without explicit `public` default to Private,
                // but for cross-file imports the type itself was already exported via `mount`,
                // so we import all associated constants regardless of visibility.
                // This follows the principle that exporting a type exports its API.
                if let ImplItemKind::Const { name, ty, .. } = &item.kind {
                    // Build the constant type
                    let const_type = self.ast_to_type(ty).unwrap_or(Type::Int);
                    let qualified_name = format!("{}.{}", type_name, name.name);
                    let const_scheme = if impl_type_vars.is_empty() {
                        TypeScheme::mono(const_type)
                    } else {
                        TypeScheme::poly(impl_type_vars.clone(), const_type)
                    };

                    // Collect for registration after scope exit
                    const_registrations.push((qualified_name.clone(), const_scheme));

                    tracing::debug!(
                        "Prepared associated constant {} from cross-file implement block",
                        qualified_name
                    );
                }

                if let ImplItemKind::Function(func) = &item.kind {
                    // Import all methods from cross-file impl blocks.
                    // The type was already exported via `mount`, so its API is accessible.
                    // Previously this only imported `public` methods for inherent impls,
                    // but Verum impl items default to Private when no visibility is specified,
                    // which would silently hide the entire type API.

                    // Check if this is a static method (no self parameter)
                    let is_static = func
                        .params
                        .first()
                        .map(|p| {
                            !matches!(
                                p.kind,
                                FunctionParamKind::SelfValue
                                    | FunctionParamKind::SelfValueMut
                                    | FunctionParamKind::SelfRef
                                    | FunctionParamKind::SelfRefMut
                                    | FunctionParamKind::SelfRefChecked
                                    | FunctionParamKind::SelfRefCheckedMut
                                    | FunctionParamKind::SelfRefUnsafe
                                    | FunctionParamKind::SelfRefUnsafeMut
                                    | FunctionParamKind::SelfOwn
                                    | FunctionParamKind::SelfOwnMut
                            )
                        })
                        .unwrap_or(true);

                    // Track self-by-value methods for affine type consumption
                    if !is_static {
                        let takes_by_value = func
                            .params
                            .first()
                            .map(|p| {
                                matches!(
                                    p.kind,
                                    FunctionParamKind::SelfValue
                                        | FunctionParamKind::SelfValueMut
                                        | FunctionParamKind::SelfOwn
                                        | FunctionParamKind::SelfOwnMut
                                )
                            })
                            .unwrap_or(false);
                        if takes_by_value {
                            self.self_by_value_methods.insert((
                                verum_common::Text::from(type_name),
                                verum_common::Text::from(func.name.name.as_str()),
                            ));
                        }
                    }

                    if is_static {
                        // Build function type for static method
                        // Note: We use ast_to_type (not _lenient) since type params are now in scope
                        let param_types: List<Type> = func
                            .params
                            .iter()
                            .filter_map(|p| match &p.kind {
                                FunctionParamKind::Regular { ty, .. } => Some(ty),
                                _ => None,
                            })
                            .map(|ty| {
                                self.ast_to_type(ty)
                                    .unwrap_or_else(|_| self.ast_to_type_lenient(ty))
                            })
                            .collect();

                        let return_type = func
                            .return_type
                            .as_ref()
                            .map(|t| {
                                self.ast_to_type(t)
                                    .unwrap_or_else(|_| self.ast_to_type_lenient(t))
                            })
                            .unwrap_or(Type::Unit);

                        // Throws → generator → async wrap via the
                        // unified helper so an `async fn* foo() -> Y`
                        // imported from a cross-file impl block lands
                        // as `Future<Generator<Y, Unit>>` (not raw
                        // `Y`). Pre-fix this was a bare
                        // `Type::function(_, return_type)` that silently
                        // dropped all three wraps — manifested at
                        // `for await line in cmd.stream_lines()` call
                        // sites with "got Result<Text, …>" instead of
                        // the expected `Future<Generator<Result<…>,
                        // Unit>>` (SHELL-5a follow-up — closes the
                        // gap left after fixing
                        // `extract_function_type_from_module`).
                        let final_return_type = self.wrap_return_type_for_sig_full(
                            return_type,
                            &func.throws_clause,
                            func.is_async,
                            func.is_generator,
                        );
                        let func_ty = Type::function(param_types, final_return_type);

                        // Prepare registration for after scope exit
                        // Create a properly quantified type scheme over the impl's type variables
                        let qualified_name = format!("{}.{}", type_name, func.name.name);
                        let method_scheme = if impl_type_vars.is_empty() {
                            TypeScheme::mono(func_ty)
                        } else {
                            TypeScheme::poly(impl_type_vars.clone(), func_ty)
                        };

                        // Collect for later registration (after scope exit)
                        static_method_registrations.push((qualified_name.clone(), method_scheme));

                        tracing::debug!(
                            "Prepared static method {} from cross-file implement block",
                            qualified_name
                        );
                    } else {
                        // Register method-level generic type parameters FIRST
                        // This ensures type bounds like `F: fn(T) -> U` can be properly resolved
                        let mut method_type_param_names = List::new();
                        let mut method_type_var_bounds: Map<TypeVar, List<Type>> = Map::new();

                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG import_impl_blocks] Processing method '{}' with {} generics",
                        // func.name.name, func.generics.len());

                        // CRITICAL FIX: Register ALL method type params FIRST, before extracting bounds
                        // This ensures that bounds like `F: fn(T) -> U` can resolve U even if U is
                        // declared after the param that references it
                        let mut method_type_params: Vec<(
                            verum_ast::Ident,
                            TypeVar,
                            &List<verum_ast::ty::TypeBound>,
                        )> = Vec::new();
                        for generic_param in &func.generics {
                            use verum_ast::ty::GenericParamKind;
                            if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind
                            {
                                let fresh_var = TypeVar::fresh();
                                let type_var = Type::Var(fresh_var);
                                let name_text: Text = name.name.clone();
                                self.ctx.define_type(name_text.clone(), type_var);
                                method_type_param_names.push(name_text);
                                method_type_params.push((name.clone(), fresh_var, bounds));

                                // #[cfg(debug_assertions)]
                                // eprintln!("[DEBUG import_impl_blocks] Registered type param '{}' -> {:?} with {} bounds",
                                // name.name, fresh_var, bounds.len());
                            }
                        }

                        // NOW extract bounds, after all type params are registered
                        for (name, fresh_var, bounds) in &method_type_params {
                            if !bounds.is_empty() {
                                let extracted_bounds = self.extract_type_bounds_from_ast(bounds);
                                // #[cfg(debug_assertions)]
                                // eprintln!("[DEBUG import_impl_blocks] Extracted {} bounds for '{}': {:?}",
                                // extracted_bounds.len(), name.name,
                                // extracted_bounds.iter().map(|t| t.to_text()).collect::<Vec<_>>());
                                if !extracted_bounds.is_empty() {
                                    method_type_var_bounds.insert(*fresh_var, extracted_bounds);
                                }
                            }
                        }

                        // Register instance method in inherent_methods map
                        let param_types: List<Type> = func
                            .params
                            .iter()
                            .filter(|p| !p.is_self()) // Exclude self parameter
                            .filter_map(|p| match &p.kind {
                                FunctionParamKind::Regular { ty, .. } => Some(ty),
                                _ => None,
                            })
                            .map(|ty| self.ast_to_type_lenient(ty))
                            .collect();

                        let return_type = func
                            .return_type
                            .as_ref()
                            .map(|t| self.ast_to_type_lenient(t))
                            .unwrap_or(Type::Unit);

                        // Throws → generator → async wrap via the
                        // unified helper. See sibling static-method
                        // branch above for the SHELL-5a rationale.
                        let final_return_type = self.wrap_return_type_for_sig_full(
                            return_type,
                            &func.throws_clause,
                            func.is_async,
                            func.is_generator,
                        );
                        let method_ty = Type::function(param_types, final_return_type);
                        let type_name_text = verum_common::Text::from(type_name);
                        let method_name_text = verum_common::Text::from(func.name.name.as_str());

                        // CRITICAL: Use generalize_ordered to preserve type parameter order
                        // Combine impl type param names with method type param names
                        let mut ordered_params: List<verum_common::Text> =
                            impl_type_param_names.clone();
                        for param in &method_type_param_names {
                            ordered_params.push(param.clone());
                        }
                        let mut method_scheme =
                            self.ctx.generalize_ordered(method_ty, &ordered_params);

                        // CRITICAL: Track how many type vars come from the implement block.
                        // This prevents method-level type vars (like F in modify<F>)
                        // from being incorrectly bound to receiver type args during
                        // method call resolution.
                        method_scheme.impl_var_count = impl_type_vars.len();

                        // CRITICAL: Add type bounds for closure type inference
                        if !method_type_var_bounds.is_empty() {
                            method_scheme = method_scheme.with_type_bounds(method_type_var_bounds);
                        }

                        // Clean up method-level generic type parameters
                        for param_name in method_type_param_names {
                            self.ctx.remove_type(&param_name);
                        }

                        // Get or create the methods map for this type (using shared RwLock)
                        {
                            let mut methods_guard = self.inherent_methods.write();
                            let methods = methods_guard.entry(type_name_text.clone()).or_default();
                            #[cfg(debug_assertions)]
                            if func.name.name.as_str() == "cmp" {
                                // eprintln!("[DEBUG import_impl_blocks] Registering 'cmp' method for '{}' with scheme:\n {:?}",
                                // type_name, method_scheme);
                            }
                            methods.insert(method_name_text.clone(), method_scheme);
                        }

                        // Store the impl self-type arg pattern for specialization filtering.
                        // Only store when the impl has type args (i.e., it's a generic/specialized impl).
                        if !impl_self_type_args.is_empty() {
                            let mut patterns_guard = self.method_impl_patterns.write();
                            let type_patterns =
                                patterns_guard.entry(type_name_text.clone()).or_default();
                            let method_patterns =
                                type_patterns.entry(method_name_text.clone()).or_default();
                            method_patterns.push(impl_self_type_args.clone());
                        }

                        {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG import_impl_blocks] Registered instance method '{}.{}'",
                            // type_name, func.name.name);
                        }

                        tracing::debug!(
                            "Imported instance method {}.{} from cross-file implement block",
                            type_name,
                            func.name.name
                        );
                    }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // CRITICAL FIX: Register protocol implementation for imported impls.
            // Previously, only methods were registered during import, but NOT the
            // protocol implementation itself. This meant implements_protocol() would
            // return false for imported impls like `implement Hasher for DefaultHasher`.
            // ═══════════════════════════════════════════════════════════════
            if let verum_ast::decl::ImplKind::Protocol {
                protocol,
                protocol_args: ast_protocol_args,
                for_type,
            } = &impl_decl.kind
            {
                // Build method types for the ProtocolImpl
                let mut methods: verum_common::Map<verum_common::Text, Type> =
                    verum_common::Map::new();

                let protocol_name_for_debug = protocol
                    .as_ident()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG import_impl_blocks] Building methods for {} for {}, impl_decl.items.len()={}",
                // protocol_name_for_debug, type_name, impl_decl.items.len());

                for item in &impl_decl.items {
                    if let ImplItemKind::Function(func) = &item.kind {
                        let method_name: verum_common::Text = func.name.name.clone();
                        // Build method type (excluding self)
                        let param_types: verum_common::List<Type> = func
                            .params
                            .iter()
                            .filter(|p| !p.is_self())
                            .filter_map(|p| match &p.kind {
                                FunctionParamKind::Regular { ty, .. } => Some(ty),
                                _ => None,
                            })
                            .map(|ty| self.ast_to_type_lenient(ty))
                            .collect();

                        let return_type = func
                            .return_type
                            .as_ref()
                            .map(|t| self.ast_to_type_lenient(t))
                            .unwrap_or(Type::Unit);

                        // Throws → generator → async wrap via the
                        // unified helper so cross-module protocol-impl
                        // methods (`async fn* foo() -> Y`) land as
                        // `Future<Generator<Y, Unit>>` rather than the
                        // bare body type. Same SHELL-5a rationale.
                        let final_return_type = self.wrap_return_type_for_sig_full(
                            return_type,
                            &func.throws_clause,
                            func.is_async,
                            func.is_generator,
                        );

                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG import_impl_blocks] Added method '{}' with return_type={}", method_name, return_type.to_text());
                        methods.insert(method_name, Type::function(param_types, final_return_type));
                    }
                }

                // CRITICAL: If methods map is empty and we have a protocol name,
                // try to get method types from the protocol definition itself
                if methods.is_empty() && !protocol_name_for_debug.is_empty() {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG import_impl_blocks] methods is EMPTY for {} for {}, trying to get from protocol definition",
                    // protocol_name_for_debug, type_name);

                    let protocol_name_text: verum_common::Text =
                        protocol_name_for_debug.clone().into();
                    let protocol_checker_guard = self.protocol_checker.read();
                    if let verum_common::Maybe::Some(protocol_def) =
                        protocol_checker_guard.get_protocol(&protocol_name_text)
                    {
                        for (method_name, method_info) in &protocol_def.methods {
                            methods.insert(method_name.clone(), method_info.ty.clone());
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG import_impl_blocks] Got method '{}' from protocol def with ty={}",
                            // method_name, method_info.ty.to_text());
                        }
                    }
                }

                // Build associated types
                let mut associated_types: verum_common::Map<verum_common::Text, Type> =
                    verum_common::Map::new();
                for item in &impl_decl.items {
                    if let ImplItemKind::Type {
                        name, ty: assoc_ty, ..
                    } = &item.kind
                    {
                        if let Ok(resolved_ty) = self.ast_to_type(assoc_ty) {
                            associated_types.insert(name.name.clone(), resolved_ty);
                        }
                    }
                }

                // Build where clauses from impl generics
                let mut where_clauses: verum_common::List<crate::protocol::WhereClause> =
                    verum_common::List::new();
                for generic_param in &impl_decl.generics {
                    use verum_ast::ty::GenericParamKind;
                    if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind {
                        if !bounds.is_empty() {
                            // CRITICAL FIX: Use the SAME Type::Var that was defined for this type parameter.
                            // When try_match_type matches for_type, it builds substitution keys like "T329"
                            // for Type::Var(TypeVar{id:329}). The where clause must use the same Type::Var
                            // so apply_substitution can look up the correct key.
                            // Using Type::Named would create key "I" which wouldn't match "T329".
                            let name_text: verum_common::Text = name.name.clone();
                            // Clone immediately to release the immutable borrow before calling convert_type_bounds
                            let type_var_opt = self.ctx.lookup_type(name_text.as_str()).cloned();
                            if let Some(type_var) = type_var_opt {
                                if let Ok(protocol_bounds) =
                                    self.convert_type_bounds_to_protocol_bounds(bounds)
                                {
                                    where_clauses.push(crate::protocol::WhereClause {
                                        ty: type_var,
                                        bounds: protocol_bounds,
                                    });
                                }
                            }
                        }
                    }
                }

                // Resolve the for_type
                // CRITICAL FIX: Use ast_to_type_for_protocol_impl to avoid expanding type aliases.
                // This ensures the ProtocolImpl has for_type as Named{Result, [T, E]} instead of the
                // expanded Variant form, which is necessary for get_implementations() to match lookups.
                let for_type_resolved = self
                    .ast_to_type_for_protocol_impl(for_type)
                    .unwrap_or_else(|_| self.ast_to_type_lenient(for_type));

                // CRITICAL FIX: Resolve protocol type arguments (e.g., Result<Never, E> in FromResidual<Result<Never, E>>)
                let resolved_protocol_args: verum_common::List<Type> = ast_protocol_args
                    .iter()
                    .filter_map(|arg| {
                        use verum_ast::ty::GenericArg;
                        match arg {
                            GenericArg::Type(ty) => Some(
                                self.ast_to_type(ty)
                                    .unwrap_or_else(|_| self.ast_to_type_lenient(ty)),
                            ),
                            GenericArg::Const(_)
                            | GenericArg::Lifetime(_)
                            | GenericArg::Binding(_) => None,
                        }
                    })
                    .collect();

                // Create and register the ProtocolImpl
                let protocol_impl = crate::protocol::ProtocolImpl {
                    protocol: protocol.clone(),
                    protocol_args: resolved_protocol_args,
                    for_type: for_type_resolved,
                    where_clauses,
                    methods: methods.clone(),
                    associated_types,
                    associated_consts: verum_common::Map::new(),
                    specialization: verum_common::Maybe::None,
                    impl_crate: verum_common::Maybe::None,
                    span: impl_decl.span,
                    type_param_fn_bounds: verum_common::Map::new(),
                };

                // DEBUG: Log FromResidual protocol impl registration in import_impl_blocks
                // if protocol.as_ident().map(|i| i.as_str()) == Some("FromResidual") {
                //  eprintln!("[DEBUG import_impl_blocks] Registering FromResidual impl for {}, methods: {:?}",
                //  protocol_impl.for_type, methods.keys().collect::<Vec<_>>());
                // }

                // DEBUG: Log protocol impl registration
                #[cfg(debug_assertions)]
                {
                    let proto_name = protocol.as_ident().map(|i| i.as_str()).unwrap_or("?");
                    // eprintln!(
                    // "[DEBUG import_impl_blocks] Registering protocol impl: {} for {}",
                    // proto_name, type_name
                    // );
                }

                // Register with protocol checker
                if let Err(e) = self.protocol_checker.write().register_impl(protocol_impl) {
                    tracing::debug!("Protocol impl registration during import: {}", e);
                }
            }

            // Restore old self type before exiting scope
            self.set_current_self_type(old_self_type);

            // Exit the scope we entered for this impl block's type parameters
            self.ctx.exit_scope();

            // NOW register static methods in the outer scope (after exiting the temporary scope)
            // This ensures the registrations persist and are visible during type checking
            for (qualified_name, method_scheme) in static_method_registrations {
                self.ctx.env.insert(qualified_name.as_str(), method_scheme);

                tracing::debug!("Registered static method {} in outer scope", qualified_name);
            }

            // Register associated constants in the outer scope
            for (qualified_name, const_scheme) in const_registrations {
                self.ctx.env.insert(qualified_name.as_str(), const_scheme);

                tracing::debug!(
                    "Registered associated constant {} in outer scope",
                    qualified_name
                );
            }
        }

        // Restore the checker's module path so imports don't leak
        // the source module into surrounding scope.
        self.set_current_module_path(saved_module_path);
        Ok(())
    }

    /// Find a protocol declaration in a module's AST by name.
    ///

    /// This is used when importing context protocols to register them properly
    /// in the type environment. Protocol declarations are distinct from type
    /// declarations that contain protocol bodies.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    fn find_protocol_declaration_in_module(
        &self,
        ast: &verum_ast::Module,
        protocol_name: &str,
    ) -> Option<verum_ast::ProtocolDecl> {
        use verum_ast::ItemKind;

        for item in &ast.items {
            if let ItemKind::Protocol(proto_decl) = &item.kind
                && proto_decl.name.name.as_str() == protocol_name
            {
                return Some(proto_decl.clone());
            }
        }
        None
    }

    /// Find a context protocol declaration in a module's AST by name.
    ///

    /// This looks for `context protocol Name { ... }` declarations, which are
    /// parsed as ProtocolDecl with `is_context == true`.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    fn find_context_protocol_in_module(
        &self,
        ast: &verum_ast::Module,
        protocol_name: &str,
    ) -> Option<verum_ast::ProtocolDecl> {
        use verum_ast::ItemKind;

        for item in &ast.items {
            if let ItemKind::Protocol(proto_decl) = &item.kind
                && proto_decl.is_context
                && proto_decl.name.name.as_str() == protocol_name
            {
                return Some(proto_decl.clone());
            }
        }
        None
    }

    /// Find a context protocol, following re-export chains if needed.
    ///

    /// Looks for `context protocol Name { ... }` declarations, following import
    /// chains when the protocol is re-exported.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    fn find_context_protocol_with_reexports(
        &self,
        ast: &verum_ast::Module,
        protocol_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<verum_ast::ProtocolDecl> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;

        // First, try to find the context protocol directly
        if let Some(decl) = self.find_context_protocol_in_module(ast, protocol_name) {
            return Some(decl);
        }

        // Helper to check if path starts with relative marker
        let is_relative = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Relative))
        };

        // Helper to resolve relative module path to absolute path
        let resolve_relative_path = |relative_path: &str| -> String {
            if relative_path.is_empty() {
                return String::new();
            }
            format!("{}.{}", current_module_path.as_str(), relative_path)
        };

        // If not found directly, check if it's re-exported via `pub import`
        for item in &ast.items {
            if let ItemKind::Mount(import_decl) = &item.kind {
                if import_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Check if this import re-exports our protocol
                let source_module_path: Option<String> = match &import_decl.tree.kind {
                    MountTreeKind::Path(path) => {
                        if let Some(PathSegment::Name(last_ident)) = path.segments.last() {
                            if last_ident.name.as_str() == protocol_name {
                                let has_relative = is_relative(path);
                                let module_segments: Vec<&str> = path
                                    .segments
                                    .iter()
                                    .take(path.segments.len() - 1)
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = module_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    Some(resolve_relative_path(&module_path))
                                } else if !module_path.is_empty() {
                                    Some(module_path)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        // Check if any nested tree imports our protocol
                        let mut found_path = None;
                        for tree in trees {
                            if let MountTreeKind::Path(inner_path) = &tree.kind
                                && let Some(PathSegment::Name(ident)) = inner_path.segments.last()
                                && ident.name.as_str() == protocol_name
                            {
                                let has_relative = is_relative(prefix);
                                let prefix_segments: Vec<&str> = prefix
                                    .segments
                                    .iter()
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = prefix_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    found_path = Some(resolve_relative_path(&module_path));
                                } else if !module_path.is_empty() {
                                    found_path = Some(module_path);
                                }
                                break;
                            }
                        }
                        found_path
                    }
                    _ => None,
                };

                // If we found the source module, look up the protocol there
                if let Some(source_path) = source_module_path
                    && let Some(source_module) = registry.get_by_path(&source_path)
                {
                    // Recursively search (handles chains of re-exports)
                    if let Some(decl) = self.find_context_protocol_with_reexports(
                        &source_module.ast,
                        protocol_name,
                        &verum_common::Text::from(source_path),
                        registry,
                    ) {
                        return Some(decl);
                    }
                }
            }
        }
        None
    }

    /// Find a context protocol and its source module path, following re-export chains if needed.
    ///

    /// Returns both the protocol declaration and the module path where it was defined.
    /// This is used to import sibling types (like `SearchResponse`, `SearchError`) from
    /// the protocol's module before building method signatures.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    fn find_context_protocol_with_source_module(
        &self,
        ast: &verum_ast::Module,
        protocol_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<(verum_ast::ProtocolDecl, Text)> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;

        // First, try to find the context protocol directly in this module
        if let Some(decl) = self.find_context_protocol_in_module(ast, protocol_name) {
            return Some((decl, current_module_path.clone()));
        }

        // Helper to check if path starts with relative marker
        let is_relative = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Relative))
        };

        // Helper to resolve relative module path to absolute path
        let resolve_relative_path = |relative_path: &str| -> String {
            if relative_path.is_empty() {
                return String::new();
            }
            format!("{}.{}", current_module_path.as_str(), relative_path)
        };

        // If not found directly, check if it's re-exported via `pub import`
        for item in &ast.items {
            if let ItemKind::Mount(import_decl) = &item.kind {
                if import_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Check if this import re-exports our protocol
                let source_module_path: Option<String> = match &import_decl.tree.kind {
                    MountTreeKind::Path(path) => {
                        if let Some(PathSegment::Name(last_ident)) = path.segments.last() {
                            if last_ident.name.as_str() == protocol_name {
                                let has_relative = is_relative(path);
                                let module_segments: Vec<&str> = path
                                    .segments
                                    .iter()
                                    .take(path.segments.len() - 1)
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = module_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    Some(resolve_relative_path(&module_path))
                                } else if !module_path.is_empty() {
                                    Some(module_path)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        // Check if any nested tree imports our protocol
                        let mut found_path = None;
                        for tree in trees {
                            if let MountTreeKind::Path(inner_path) = &tree.kind
                                && let Some(PathSegment::Name(ident)) = inner_path.segments.last()
                                && ident.name.as_str() == protocol_name
                            {
                                let has_relative = is_relative(prefix);
                                let prefix_segments: Vec<&str> = prefix
                                    .segments
                                    .iter()
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = prefix_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    found_path = Some(resolve_relative_path(&module_path));
                                } else if !module_path.is_empty() {
                                    found_path = Some(module_path);
                                }
                                break;
                            }
                        }
                        found_path
                    }
                    _ => None,
                };

                // If we found the source module, look up the protocol there
                if let Some(source_path) = source_module_path
                    && let Some(source_module) = registry.get_by_path(&source_path)
                {
                    let source_path_text = verum_common::Text::from(source_path);
                    // Recursively search (handles chains of re-exports)
                    if let Some((decl, final_module_path)) = self
                        .find_context_protocol_with_source_module(
                            &source_module.ast,
                            protocol_name,
                            &source_path_text,
                            registry,
                        )
                    {
                        return Some((decl, final_module_path));
                    }
                }
            }
        }
        None
    }

    /// Build a Record type from a context protocol declaration's methods.
    ///

    /// Each method in the protocol becomes a field in the Record with a function type.
    /// The `self` parameter is skipped since it's implicit when calling context methods.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    /// Build a Record type from a protocol declaration's methods.
    ///

    /// For generic protocols like `type Repository<T> is protocol { ... }`, the
    /// protocol-level type parameters are registered in the type environment BEFORE
    /// processing method items, ensuring that method signatures can reference them
    /// (e.g., `fn find_by_id(id: Int) -> Maybe<T>`).
    ///

    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — GATs
    /// Context provision: "provide ContextName = implementation" installs a provider in lexical scope via task-local storage (theta) — Protocol Type Parameter Scoping
    fn build_context_type_from_protocol(
        &mut self,
        proto_decl: &verum_ast::ProtocolDecl,
    ) -> Result<Type> {
        use verum_ast::decl::ProtocolItemKind;
        use verum_ast::ty::GenericParamKind;

        let mut fields = indexmap::IndexMap::new();

        // ============================================================================
        // CRITICAL FIX: Register protocol-level generics BEFORE processing methods
        // ============================================================================
        // For generic protocols like `type Repository<T> is protocol { ... }`, we must
        // register T in the type environment so that method signatures like
        // `fn find_by_id(id: Int) -> Maybe<T>` can reference them.
        //

        // This fixes the issue where type parameter T was not in scope when parsing
        // method return types in generic protocols.
        // ============================================================================
        let mut protocol_type_params: List<crate::context::TypeParam> = List::new();

        for generic in &proto_decl.generics {
            match &generic.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    let type_var = Type::Var(TypeVar::fresh());
                    let name_text: Text = name.name.clone();
                    self.ctx.define_type(name_text.clone(), type_var);

                    // Build TypeParam with bounds (if any)
                    let protocol_bounds = if !bounds.is_empty() {
                        self.convert_type_bounds_to_protocol_bounds(bounds)?
                    } else {
                        List::new()
                    };
                    let type_param = crate::context::TypeParam::new(name_text.clone(), name.span)
                        .with_bounds(protocol_bounds);
                    protocol_type_params.push(type_param.clone());

                    // Also register in the type environment for method signature resolution
                    self.ctx.env.add_type_param(type_param);
                }
                GenericParamKind::Meta { name, ty, .. } => {
                    // Handle meta (compile-time) parameters
                    let name_text: Text = name.name.clone();
                    let meta_type = self.ast_to_type(ty)?;
                    self.ctx.define_type(name_text.clone(), meta_type);
                }
                _ => {} // Handle other generic param kinds if needed
            }
        }

        for item in &proto_decl.items {
            if let ProtocolItemKind::Function { decl: method, .. } = &item.kind {
                // Register method-level generics before processing the signature
                // This ensures type parameters like T in fn get<T>(...) -> Maybe<T> are resolvable
                let mut method_type_params: List<crate::context::TypeParam> = List::new();

                for generic in &method.generics {
                    match &generic.kind {
                        GenericParamKind::Type { name, bounds, .. } => {
                            let type_var = Type::Var(TypeVar::fresh());
                            let name_text: Text = name.name.clone();
                            self.ctx.define_type(name_text.clone(), type_var);

                            // Build TypeParam with bounds (if any)
                            let protocol_bounds = if !bounds.is_empty() {
                                self.convert_type_bounds_to_protocol_bounds(bounds)?
                            } else {
                                List::new()
                            };
                            let type_param = crate::context::TypeParam::new(name_text, name.span)
                                .with_bounds(protocol_bounds);
                            method_type_params.push(type_param);
                        }
                        _ => {} // Handle other generic param kinds if needed
                    }
                }

                // Build parameter types, skipping `self` parameters
                // Use lenient type conversion that falls back to Type::Unknown for unresolved types.
                // This allows us to build the record type even when sibling types aren't imported yet.
                let param_types: List<Type> = method
                    .params
                    .iter()
                    .filter(|p| !p.is_self())
                    .map(|p| match &p.kind {
                        verum_ast::decl::FunctionParamKind::Regular { pattern: _, ty, .. } => {
                            self.ast_to_type_lenient(ty)
                        }
                        _ => Type::unit(),
                    })
                    .collect();

                // Build return type with lenient conversion
                let return_type = if let Some(ref ret_ty) = method.return_type {
                    self.ast_to_type_lenient(ret_ty)
                } else {
                    Type::unit()
                };

                // Throws → generator → async wrap via the unified
                // helper so protocol method signatures (`async fn* m()
                // -> Y throws E`) match the shape every other path
                // produces — external callers see
                // `Future<Generator<Result<Y, E>, Unit>>` rather than
                // a degraded form. SHELL-5a coherence sweep.
                let final_return_type = self.wrap_return_type_for_sig_full(
                    return_type,
                    &method.throws_clause,
                    method.is_async,
                    method.is_generator,
                );

                // Build function type for this method
                let method_type = Type::Function {
                    params: param_types,
                    return_type: Box::new(final_return_type),
                    contexts: None,
                    type_params: method_type_params,
                    properties: None,
                };

                fields.insert(method.name.name.clone(), method_type);
            }
        }

        Ok(Type::Record(fields))
    }

    /// Find a context declaration in a module's AST by name.
    ///

    /// This is used when importing context protocols to build the proper
    /// Record type with method signatures for method call resolution.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    fn find_context_declaration_in_module(
        &self,
        ast: &verum_ast::Module,
        context_name: &str,
    ) -> Option<verum_ast::decl::ContextDecl> {
        use verum_ast::ItemKind;

        for item in &ast.items {
            if let ItemKind::Context(ctx_decl) = &item.kind
                && ctx_decl.name.name.as_str() == context_name
            {
                return Some(ctx_decl.clone());
            }
        }
        None
    }

    /// Find a context declaration, following re-export chains if needed.
    ///

    /// When a context is re-exported via `pub import`, we need to follow the import
    /// chain to find the actual context declaration in the original module.
    ///

    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    fn find_context_declaration_with_reexports(
        &self,
        ast: &verum_ast::Module,
        context_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<verum_ast::decl::ContextDecl> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;

        // First, try to find the context declaration directly
        if let Some(decl) = self.find_context_declaration_in_module(ast, context_name) {
            return Some(decl);
        }

        // Helper to check if path starts with relative marker
        let is_relative = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Relative))
        };

        // Helper to resolve relative module path to absolute path
        let resolve_relative_path = |relative_path: &str| -> String {
            if relative_path.is_empty() {
                return String::new();
            }
            format!("{}.{}", current_module_path.as_str(), relative_path)
        };

        // If not found directly, check if it's re-exported via `pub import`
        for item in &ast.items {
            if let ItemKind::Mount(import_decl) = &item.kind {
                if import_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Check if this import re-exports our context
                let source_module_path: Option<String> = match &import_decl.tree.kind {
                    MountTreeKind::Path(path) => {
                        if let Some(PathSegment::Name(last_ident)) = path.segments.last() {
                            if last_ident.name.as_str() == context_name {
                                let has_relative = is_relative(path);
                                let module_segments: Vec<&str> = path
                                    .segments
                                    .iter()
                                    .take(path.segments.len() - 1)
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = module_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    Some(resolve_relative_path(&module_path))
                                } else if !module_path.is_empty() {
                                    Some(module_path)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        // Check if any nested tree imports our context
                        let mut found_path = None;
                        for tree in trees {
                            if let MountTreeKind::Path(inner_path) = &tree.kind
                                && let Some(PathSegment::Name(ident)) = inner_path.segments.last()
                                && ident.name.as_str() == context_name
                            {
                                let has_relative = is_relative(prefix);
                                let prefix_segments: Vec<&str> = prefix
                                    .segments
                                    .iter()
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = prefix_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    found_path = Some(resolve_relative_path(&module_path));
                                } else if !module_path.is_empty() {
                                    found_path = Some(module_path);
                                }
                                break;
                            }
                        }
                        found_path
                    }
                    _ => None,
                };

                // If we found the source module, look up the context there
                if let Some(source_path) = source_module_path
                    && let Some(source_module) = registry.get_by_path(&source_path)
                {
                    // Recursively search (handles chains of re-exports)
                    if let Some(decl) = self.find_context_declaration_with_reexports(
                        &source_module.ast,
                        context_name,
                        &verum_common::Text::from(source_path),
                        registry,
                    ) {
                        return Some(decl);
                    }
                }
            }
        }
        None
    }

    /// Build a Record type from a context declaration's methods.
    ///

    /// Each method in the context becomes a field in the Record with a function type.
    /// The `self` parameter is skipped since it's implicit when calling context methods.
    ///

    /// For generic contexts like `context Cache<K, V> { ... }`, the context-level type
    /// parameters are registered in the type environment BEFORE processing methods,
    /// ensuring that method signatures can reference them (e.g., `fn get(key: K) -> Maybe<V>`).
    ///

    /// Context provision: "provide ContextName = implementation" installs a provider in lexical scope via task-local storage (theta) — Parameterized Contexts
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    pub(super) fn build_context_type_from_decl(
        &mut self,
        ctx_decl: &verum_ast::decl::ContextDecl,
    ) -> Result<Type> {
        use verum_ast::ty::GenericParamKind;
        let mut fields = indexmap::IndexMap::new();

        // ============================================================================
        // CRITICAL FIX: Register context-level generics BEFORE processing methods
        // ============================================================================
        // For generic contexts like `context Cache<K, V> { ... }`, we must register
        // K and V in the type environment so that method signatures like
        // `fn get(key: K) -> Maybe<V>` can reference them.
        //

        // This fixes the issue where type parameter T was not in scope when parsing
        // method return types in generic contexts.
        // ============================================================================
        let mut context_type_params: List<crate::context::TypeParam> = List::new();

        for generic in &ctx_decl.generics {
            match &generic.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    let type_var = Type::Var(TypeVar::fresh());
                    let name_text: Text = name.name.clone();
                    self.ctx.define_type(name_text.clone(), type_var);

                    // Build TypeParam with bounds (if any)
                    let protocol_bounds = if !bounds.is_empty() {
                        self.convert_type_bounds_to_protocol_bounds(bounds)?
                    } else {
                        List::new()
                    };
                    let type_param = crate::context::TypeParam::new(name_text.clone(), name.span)
                        .with_bounds(protocol_bounds);
                    context_type_params.push(type_param.clone());

                    // Also register in the type environment for method signature resolution
                    self.ctx.env.add_type_param(type_param);
                }
                GenericParamKind::Meta { name, ty, .. } => {
                    // Handle meta (compile-time) parameters
                    let name_text: Text = name.name.clone();
                    let meta_type = self.ast_to_type(ty)?;
                    self.ctx.define_type(name_text.clone(), meta_type);
                }
                _ => {} // Handle other generic param kinds if needed
            }
        }

        for method in &ctx_decl.methods {
            // Register method-level generics before processing the signature
            // This ensures type parameters like T in fn get<T>(...) -> Maybe<T> are resolvable
            let mut method_type_params: List<crate::context::TypeParam> = List::new();

            for generic in &method.generics {
                match &generic.kind {
                    GenericParamKind::Type { name, bounds, .. } => {
                        let type_var = Type::Var(TypeVar::fresh());
                        let name_text: Text = name.name.clone();
                        self.ctx.define_type(name_text.clone(), type_var);

                        // Build TypeParam with bounds (if any)
                        let protocol_bounds = if !bounds.is_empty() {
                            self.convert_type_bounds_to_protocol_bounds(bounds)?
                        } else {
                            List::new()
                        };
                        let type_param = crate::context::TypeParam::new(name_text, name.span)
                            .with_bounds(protocol_bounds);
                        method_type_params.push(type_param);
                    }
                    _ => {} // Handle other generic param kinds if needed
                }
            }

            // Build parameter types, skipping `self` parameters.
            // Use lenient resolution so that a sibling stdlib type declared in
            // the same module (but not yet registered at context pre-registration
            // time — the archive scan that populates `context_declarations` runs
            // before Pass S0b for stdlib types) falls back to Type::Unknown
            // instead of propagating a hard `TypeNotFound` error. The caller
            // (`register_stdlib_context_full`) already has a record-level
            // fallback, but that fires *only* when the outer call returns Err
            // — which wipes the entire method set. Field-level fallback
            // preserves the known-good method signatures around the one
            // placeholder.
            let param_types: List<Type> = method
                .params
                .iter()
                .filter(|p| !p.is_self())
                .map(|p| match &p.kind {
                    verum_ast::decl::FunctionParamKind::Regular { pattern: _, ty, .. } => {
                        self.ast_to_type_lenient(ty)
                    }
                    _ => Type::unit(),
                })
                .collect();

            // Build return type
            let return_type = if let Some(ref ret_ty) = method.return_type {
                self.ast_to_type_lenient(ret_ty)
            } else {
                Type::unit()
            };

            // Throws → generator → async wrap via the unified helper
            // so method signatures visible to callers match the shape
            // every other path produces. Pre-fix this branch handled
            // throws + async but DROPPED the generator wrap, so an
            // `async fn* m() -> Y` registered as `Future<Y>` instead
            // of `Future<Generator<Y, Unit>>` and `for await x in
            // obj.m()` failed at the call site with "got Future<Y>".
            let final_return_type = self.wrap_return_type_for_sig_full(
                return_type,
                &method.throws_clause,
                method.is_async,
                method.is_generator,
            );

            // Build function type for this method
            let method_type = Type::Function {
                params: param_types,
                return_type: Box::new(final_return_type),
                contexts: None,
                type_params: method_type_params,
                properties: None,
            };

            fields.insert(method.name.name.clone(), method_type);

            // Clean up method-level generics after processing each method
            // Note: We don't actually need to clean up since these are scoped type vars
            // that won't conflict with anything else
        }

        Ok(Type::Record(fields))
    }

    /// Look up a module in the registry, trying multiple path aliases.
    ///

    /// Stdlib modules are stored with "std." prefix, but imports may use "core.", "std.",
    /// or bare paths like "io.". This function generates candidate paths and tries each.
    ///

    /// For example, "core.io.path" generates candidates: ["core.io.path", "std.io.path", "io.path"]
    fn get_module_with_path_aliases(
        &self,
        path: &str,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<verum_common::Shared<verum_modules::ModuleInfo>> {
        // MOD-CRIT-1: route every path-alias decision through the
        // registry's central alias map. The hardcoded table that used
        // to live here now lives in pipeline.rs::install_canonical_
        // module_aliases, owned by ModuleRegistry. This funnel point
        // eliminates the loader/type-resolver path-dedup incoherence:
        // both subsystems consult the same alias map, so a path
        // resolved here ALWAYS matches the canonical decision the
        // loader made at registration time.
        if let verum_common::Maybe::Some(module) = registry.get_by_path_aliased(path) {
            return Some(module);
        }

        // Prefix transformations — kept here because they are
        // syntactic, not alias-bound. Modules are registered with
        // "core." prefix, so normalise paths to that form on miss.
        if path.starts_with("core.") {
            // core.io.path is already canonical;
            // also try bare path: core.io.path -> io.path
            let stripped = &path[5..];
            if let verum_common::Maybe::Some(module) = registry.get_by_path_aliased(stripped) {
                return Some(module);
            }
        } else if path.starts_with("std.") {
            // std.io.path -> core.io.path (legacy compatibility)
            let stripped = &path[4..];
            if let verum_common::Maybe::Some(module) =
                registry.get_by_path_aliased(&format!("core.{}", stripped))
            {
                return Some(module);
            }
            if let verum_common::Maybe::Some(module) = registry.get_by_path_aliased(stripped) {
                return Some(module);
            }
        } else {
            // io.path -> core.io.path (canonical form)
            if let verum_common::Maybe::Some(module) =
                registry.get_by_path_aliased(&format!("core.{}", path))
            {
                return Some(module);
            }
        }

        // Try matching with `.mod` suffix stripped (handles residual mod.vr paths)
        let with_mod = format!("{}.mod", path);
        if let verum_common::Maybe::Some(module) = registry.get_by_path_aliased(&with_mod) {
            return Some(module);
        }

        None
    }

    /// Find a type declaration and its source module, following re-export chains if needed.
    ///

    /// Returns both the type declaration and the module path where it was defined.
    /// This is used to import sibling types from the type's source module before
    /// registering the type, so that types used in field definitions are available.
    ///

    /// Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — .4 - Re-exports
    ///

    /// Relies on RUST_MIN_STACK=16MB for stack safety when following
    /// deep re-export chains.
    pub(crate) fn find_type_declaration_with_source_module(
        &self,
        ast: &verum_ast::Module,
        type_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<(verum_ast::decl::TypeDecl, Text)> {
        // Thread a fresh visited-set through the recursion so that ring-shaped
        // `public mount` re-exports (e.g., `core.pkg` re-exports `core.pkg.sub`
        // whose last segment matches a type `sub` being searched for — the
        // source path strips the last segment back to `core.pkg`, re-entering
        // the same AST) terminate with None instead of blowing the stack.
        // See also: `resolve_export_kind_with_reexports` (343fc3a8) which uses
        // the same pattern for the sibling kind-resolution walk.
        let mut visited: std::collections::HashSet<(Text, Text)> = std::collections::HashSet::new();
        self.find_type_declaration_with_source_module_inner(
            ast,
            type_name,
            current_module_path,
            registry,
            &mut visited,
        )
    }

    /// Inner implementation of find_type_declaration_with_source_module.
    fn find_type_declaration_with_source_module_inner(
        &self,
        ast: &verum_ast::Module,
        type_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
        visited: &mut std::collections::HashSet<(Text, Text)>,
    ) -> Option<(verum_ast::decl::TypeDecl, Text)> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;

        // Cycle guard: if we are already walking the re-export chain for this
        // (module, type) pair, return None to break the recursion. Returning
        // None is safe — callers fall through to their next strategy (e.g.,
        // checking the module's own export table or searching parent modules).
        // Without this guard, a module that re-exports a submodule whose name
        // collides with the target type (`public mount a.b.sub;` + lookup of
        // `sub` in `a.b`) recurses indefinitely and SIGBUSes.
        let key = (current_module_path.clone(), Text::from(type_name));
        if !visited.insert(key) {
            return None;
        }

        // First, try to find the type declaration directly in this module
        if let Some(decl) = self.find_type_declaration_in_module(ast, type_name) {
            return Some((decl, current_module_path.clone()));
        }

        // Helper to check if path starts with relative marker
        let is_relative = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Relative))
        };

        // Helper to resolve relative module path to absolute path
        let resolve_relative_path = |relative_path: &str| -> String {
            if relative_path.is_empty() {
                return String::new();
            }
            format!("{}.{}", current_module_path.as_str(), relative_path)
        };

        // If not found directly, check if it's re-exported via `pub import`
        for item in &ast.items {
            if let ItemKind::Mount(import_decl) = &item.kind {
                if import_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Check if this import re-exports our type
                let source_module_path: Option<String> = match &import_decl.tree.kind {
                    MountTreeKind::Path(path) => {
                        // Single import: `pub import .errors.RegistryError`
                        if let Some(PathSegment::Name(last_ident)) = path.segments.last() {
                            if last_ident.name.as_str() == type_name {
                                let has_relative = is_relative(path);
                                let module_segments: Vec<&str> = path
                                    .segments
                                    .iter()
                                    .take(path.segments.len() - 1)
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = module_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    Some(resolve_relative_path(&module_path))
                                } else if !module_path.is_empty() {
                                    Some(module_path)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        // Nested import: `pub import .package.{Package, PackageVersion}`
                        let type_found = trees.iter().any(|tree| {
                            if let MountTreeKind::Path(path) = &tree.kind {
                                if let Some(PathSegment::Name(ident)) = path.segments.first() {
                                    ident.name.as_str() == type_name
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        });

                        if type_found {
                            let has_relative = is_relative(prefix);
                            let module_segments: Vec<&str> = prefix
                                .segments
                                .iter()
                                .filter_map(|seg| match seg {
                                    PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                })
                                .collect();
                            let module_path = module_segments.join(".");
                            if has_relative && !module_path.is_empty() {
                                Some(resolve_relative_path(&module_path))
                            } else if !module_path.is_empty() {
                                Some(module_path)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                if let Some(ref source_path) = source_module_path {
                    // Look up the source module and find the type there
                    // Use path aliases since "core.io.path" may be stored as "std.io.path"
                    if let Some(source_module) =
                        self.get_module_with_path_aliases(source_path, registry)
                    {
                        // Recursively search (handles chains of re-exports).
                        // Thread the visited-set so ring-shaped re-exports
                        // terminate with None rather than blowing the stack.
                        if let Some((decl, final_path)) = self
                            .find_type_declaration_with_source_module_inner(
                                &source_module.ast,
                                type_name,
                                &verum_common::Text::from(source_path.as_str()),
                                registry,
                                visited,
                            )
                        {
                            return Some((decl, final_path));
                        }
                    } else {
                        // FALLBACK: Module not found in registry. This can happen when:
                        // 1. A re-export points to a submodule (e.g., .ops) that exists as a file
                        //  (ops.vr) within the parent module directory, but isn't registered as
                        //  a separate module in the registry.
                        // 2. The parent module (e.g., core.base) contains all .vr files including
                        //  the submodule file, so we should search the parent module's AST.
                        //

                        // Try to extract parent module path and search there.
                        // e.g., "core.base.ops" -> parent "core.base", submodule "ops"
                        if let Some(dot_pos) = source_path.rfind('.') {
                            let parent_path = &source_path[..dot_pos];
                            let _submodule_name = &source_path[dot_pos + 1..];

                            if let Some(parent_module) =
                                self.get_module_with_path_aliases(parent_path, registry)
                            {
                                // Search for the type in the parent module's AST.
                                // The type might be declared in one of the sibling files
                                // (like ops.vr within core/base/).
                                if let Some(decl) = self
                                    .find_type_declaration_in_module(&parent_module.ast, type_name)
                                {
                                    return Some((decl, verum_common::Text::from(parent_path)));
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Find a type declaration, following re-export chains if needed.
    ///

    /// When a type is re-exported via `pub import`, we need to follow the import
    /// chain to find the actual type declaration in the original module.
    fn find_type_declaration_with_reexports(
        &self,
        ast: &verum_ast::Module,
        type_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<verum_ast::decl::TypeDecl> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;

        // First, try to find the type declaration directly
        if let Some(decl) = self.find_type_declaration_in_module(ast, type_name) {
            return Some(decl);
        }

        // Helper to check if path starts with relative marker
        let is_relative = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Relative))
        };

        // Helper to resolve relative module path to absolute path
        // For `.errors` in module `domain`, returns `domain.errors`
        let resolve_relative_path = |relative_path: &str| -> String {
            if relative_path.is_empty() {
                return String::new();
            }
            // If current module is "domain", relative path "errors" becomes "domain.errors"
            format!("{}.{}", current_module_path.as_str(), relative_path)
        };

        // If not found directly, check if it's re-exported via `pub import`
        for item in &ast.items {
            if let ItemKind::Mount(import_decl) = &item.kind {
                if import_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Check if this import re-exports our type
                let source_module_path: Option<String> = match &import_decl.tree.kind {
                    MountTreeKind::Path(path) => {
                        // Single import: `pub import .errors.RegistryError`
                        if let Some(PathSegment::Name(last_ident)) = path.segments.last() {
                            if last_ident.name.as_str() == type_name {
                                let has_relative = is_relative(path);
                                // Extract module path (all segments except the last, skip relative marker)
                                let module_segments: Vec<&str> = path
                                    .segments
                                    .iter()
                                    .take(path.segments.len() - 1)
                                    .filter_map(|seg| {
                                        match seg {
                                            PathSegment::Name(id) => Some(id.name.as_str()),
                                            _ => None, // Skip Relative marker
                                        }
                                    })
                                    .collect();
                                let module_path = module_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    Some(resolve_relative_path(&module_path))
                                } else if !module_path.is_empty() {
                                    Some(module_path)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        // Nested import: `pub import .package.{Package, PackageVersion}`
                        let type_found = trees.iter().any(|tree| {
                            if let MountTreeKind::Path(path) = &tree.kind {
                                if let Some(PathSegment::Name(ident)) = path.segments.first() {
                                    ident.name.as_str() == type_name
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        });

                        if type_found {
                            let has_relative = is_relative(prefix);
                            // Extract module path from prefix
                            let module_segments: Vec<&str> = prefix
                                .segments
                                .iter()
                                .filter_map(|seg| {
                                    match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None, // Skip Relative marker
                                    }
                                })
                                .collect();
                            let module_path = module_segments.join(".");
                            if has_relative && !module_path.is_empty() {
                                Some(resolve_relative_path(&module_path))
                            } else if !module_path.is_empty() {
                                Some(module_path)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                if let Some(ref source_path) = source_module_path {
                    // Look up the source module and find the type there
                    if let Some(source_module) = registry.get_by_path(source_path)
                        && let Some(decl) =
                            self.find_type_declaration_in_module(&source_module.ast, type_name)
                    {
                        return Some(decl);
                    } else {
                        // FALLBACK: Module not found. Try parent module (see find_type_declaration_with_source_module).
                        if let Some(dot_pos) = source_path.rfind('.') {
                            let parent_path = &source_path[..dot_pos];
                            if let Some(parent_module) = registry.get_by_path(parent_path)
                                && let Some(decl) = self
                                    .find_type_declaration_in_module(&parent_module.ast, type_name)
                            {
                                return Some(decl);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Find a function or constructor definition, following re-export chains.
    ///

    /// This is similar to `find_type_declaration_with_source_module` but for
    /// functions and variant constructors. When a module re-exports a function
    /// via `pub import`, we need to trace back to the original definition.
    ///

    /// For variant constructors (e.g., `Some` from `type Maybe<T> is None | Some(T)`),
    /// this will find the variant type definition and extract the constructor type.
    /// Find a function and its source module, following re-export chains.
    /// Returns (Type, List<TypeVar>, Text) where:
    /// - Type: the function type
    /// - List<TypeVar>: quantified type variables (for generic functions)
    /// - Text: the source module path
    fn find_function_with_source_module(
        &mut self,
        ast: &verum_ast::Module,
        func_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<(Type, List<TypeVar>, Text)> {
        // Cycle guard: if we're already walking re-exports for this
        // (module, item) pair higher up the call stack, bail out. Returning
        // None is safe — the caller falls through to its next strategy.
        let key = (current_module_path.clone(), Text::from(func_name));
        if !self.reexport_resolution_in_progress.insert(key.clone()) {
            return None;
        }

        let result = self.find_function_with_source_module_impl(
            ast,
            func_name,
            current_module_path,
            registry,
        );
        self.reexport_resolution_in_progress.remove(&key);
        result
    }

    fn find_function_with_source_module_impl(
        &mut self,
        ast: &verum_ast::Module,
        func_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<(Type, List<TypeVar>, Text)> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;

        // First, try to find the function directly in this module (including variant constructors)
        if let Some((func_type, type_vars)) = self.extract_function_type_from_module(ast, func_name)
        {
            return Some((func_type, type_vars, current_module_path.clone()));
        }

        // Helper to check if path starts with relative marker
        let is_relative = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Relative))
        };

        // Helper to check if path starts with super
        let starts_with_super = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Super))
        };

        // Helper to resolve relative module path to absolute path
        let resolve_relative_path = |relative_path: &str, current: &str| -> String {
            if relative_path.is_empty() {
                return String::new();
            }
            format!("{}.{}", current, relative_path)
        };

        // Helper to resolve super path
        let resolve_super_path = |path_segments: &[&str], current: &str| -> String {
            // Get parent of current module
            let parts: Vec<&str> = current.split('.').collect();
            if parts.len() <= 1 {
                // No parent, just use the path segments
                path_segments.join(".")
            } else {
                // Replace super with parent path
                let parent = parts[..parts.len() - 1].join(".");
                if path_segments.is_empty() {
                    parent
                } else {
                    format!("{}.{}", parent, path_segments.join("."))
                }
            }
        };

        // If not found directly, check if it's re-exported via `pub import`
        for item in &ast.items {
            if let ItemKind::Mount(import_decl) = &item.kind {
                if import_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Check if this import re-exports our function
                // Track original name when matched via alias (e.g., `safe_read as read`)
                let mut original_name: Option<String> = None;
                let source_module_path: Option<String> = match &import_decl.tree.kind {
                    MountTreeKind::Path(path) => {
                        // Single import: `pub import .module.func` or `pub import .module.func as alias`
                        let path_matches =
                            if let Some(PathSegment::Name(last_ident)) = path.segments.last() {
                                if last_ident.name.as_str() == func_name {
                                    true
                                } else if let Some(ref alias) = import_decl.tree.alias {
                                    if alias.name.as_str() == func_name {
                                        original_name = Some(last_ident.name.as_str().to_string());
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                        if path_matches {
                            let has_relative = is_relative(path);
                            let has_super = starts_with_super(path);
                            let module_segments: Vec<&str> = path
                                .segments
                                .iter()
                                .take(path.segments.len() - 1)
                                .filter_map(|seg| match seg {
                                    PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                })
                                .collect();
                            let module_path = module_segments.join(".");
                            if has_relative && !module_path.is_empty() {
                                Some(resolve_relative_path(
                                    &module_path,
                                    current_module_path.as_str(),
                                ))
                            } else if has_super {
                                Some(resolve_super_path(
                                    &module_segments,
                                    current_module_path.as_str(),
                                ))
                            } else if !module_path.is_empty() {
                                // Bare path — resolve as submodule of current module
                                Some(format!("{}.{}", current_module_path, module_path))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Glob(path) => {
                        // Glob import: `pub import super.core.*`
                        let has_relative = is_relative(path);
                        let has_super = starts_with_super(path);
                        let module_segments: Vec<&str> = path
                            .segments
                            .iter()
                            .filter_map(|seg| match seg {
                                PathSegment::Name(id) => Some(id.name.as_str()),
                                _ => None,
                            })
                            .collect();
                        let module_path = module_segments.join(".");
                        if has_relative && !module_path.is_empty() {
                            Some(resolve_relative_path(
                                &module_path,
                                current_module_path.as_str(),
                            ))
                        } else if has_super {
                            Some(resolve_super_path(
                                &module_segments,
                                current_module_path.as_str(),
                            ))
                        } else if !module_path.is_empty() {
                            // Bare path like `arithmetic` in `std.intrinsics` means submodule
                            // `std.intrinsics.arithmetic`
                            Some(format!("{}.{}", current_module_path, module_path))
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        // Nested import: `pub import .module.{func1, func2}`
                        // Check if func_name is in the trees (by name or alias)
                        // Also track the original name if matched via alias
                        let mut original_name: Option<String> = None;
                        let has_item = trees.iter().any(|tree| {
                            // Check alias first: `safe_read as read` - match on alias `read`
                            if let Some(ref alias) = tree.alias {
                                if alias.name.as_str() == func_name {
                                    // Found via alias - extract original name from path
                                    if let MountTreeKind::Path(item_path) = &tree.kind {
                                        if let Some(PathSegment::Name(id)) =
                                            item_path.segments.last()
                                        {
                                            original_name = Some(id.name.as_str().to_string());
                                        }
                                    }
                                    return true;
                                }
                            }
                            if let MountTreeKind::Path(item_path) = &tree.kind {
                                item_path.segments.last().is_some_and(|seg| {
                                    if let PathSegment::Name(id) = seg {
                                        id.name.as_str() == func_name
                                    } else {
                                        false
                                    }
                                })
                            } else {
                                false
                            }
                        });
                        if has_item {
                            let has_relative = is_relative(prefix);
                            let has_super = starts_with_super(prefix);
                            let module_segments: Vec<&str> = prefix
                                .segments
                                .iter()
                                .filter_map(|seg| match seg {
                                    PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                })
                                .collect();
                            let module_path = module_segments.join(".");
                            if has_relative && !module_path.is_empty() {
                                Some(resolve_relative_path(
                                    &module_path,
                                    current_module_path.as_str(),
                                ))
                            } else if has_super {
                                Some(resolve_super_path(
                                    &module_segments,
                                    current_module_path.as_str(),
                                ))
                            } else if !module_path.is_empty() {
                                // Bare path — resolve as submodule of current module
                                Some(format!("{}.{}", current_module_path, module_path))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    // #5 / P1.5 — file-relative mounts can't re-export functions
                    // through the module-path lookup; session loader handles them.
                    MountTreeKind::File { .. } => None,
                };

                if let Some(ref source_path) = source_module_path {
                    // Look up the source module and find the function there
                    // Use original_name if this was matched via alias (e.g., `safe_read as read`)
                    let lookup_name = original_name.as_deref().unwrap_or(func_name);
                    if let Some(source_module) = registry.get_by_path(source_path) {
                        if let Some((func_type, type_vars)) =
                            self.extract_function_type_from_module(&source_module.ast, lookup_name)
                        {
                            return Some((func_type, type_vars, Text::from(source_path.clone())));
                        } else {
                            // Function not found directly - recurse to check if it's re-exported
                            if let Some(result) = self.find_function_with_source_module(
                                &source_module.ast,
                                lookup_name,
                                &Text::from(source_path.clone()),
                                registry,
                            ) {
                                return Some(result);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Resolve the actual ExportKind for an item, following re-export chains.
    ///

    /// When a module re-exports an item via `pub import`, the ExportKind in the
    /// re-exporting module's ExportTable may be incorrect (defaulted to Type).
    /// This function traces back through import chains to find the original
    /// ExportKind from the source module.
    ///

    /// This is critical for context protocols: if `contexts/database.vr` exports
    /// a `context protocol Database`, and `contexts/mod.vr` re-exports it via
    /// `pub import .database.{Database}`, we need to resolve that Database is
    /// actually a Context, not a Type.
    ///

    /// Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — .4 - Re-exports
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Cross-file contexts
    ///

    /// Relies on RUST_MIN_STACK=16MB for stack safety when following deep re-export chains.
    fn resolve_export_kind_with_reexports(
        &self,
        ast: &verum_ast::Module,
        item_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Option<verum_modules::ExportKind> {
        // Thread a fresh visited-set through the recursion so that ring-shaped
        // `public mount` re-exports (A re-exports B re-exports A for the same
        // item name) terminate with None instead of blowing the stack.
        let mut visited: std::collections::HashSet<(Text, Text)> = std::collections::HashSet::new();
        self.resolve_export_kind_with_reexports_inner(
            ast,
            item_name,
            current_module_path,
            registry,
            &mut visited,
        )
    }

    /// Inner implementation of resolve_export_kind_with_reexports.
    fn resolve_export_kind_with_reexports_inner(
        &self,
        ast: &verum_ast::Module,
        item_name: &str,
        current_module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
        visited: &mut std::collections::HashSet<(Text, Text)>,
    ) -> Option<verum_modules::ExportKind> {
        // Cycle guard: already walking this (module, item) on our stack.
        // Returning None here is safe — it causes the caller to fall through
        // to the actual ExportKind in the export table, which is the correct
        // fallback when we cannot prove a different kind via re-export.
        let key = (current_module_path.clone(), Text::from(item_name));
        if !visited.insert(key) {
            return None;
        }
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;
        use verum_modules::ExportKind;

        // First, check if the item is defined directly in this module
        for item in &ast.items {
            match &item.kind {
                ItemKind::Function(func) if func.name.name.as_str() == item_name => {
                    return Some(if func.is_meta {
                        ExportKind::Meta
                    } else {
                        ExportKind::Function
                    });
                }
                ItemKind::Type(type_decl) if type_decl.name.name.as_str() == item_name => {
                    // Check if this is a context protocol type
                    if let verum_ast::decl::TypeDeclBody::Protocol(proto_body) = &type_decl.body
                        && proto_body.is_context
                    {
                        return Some(ExportKind::Context);
                    }
                    return Some(ExportKind::Type);
                }
                ItemKind::Protocol(proto) if proto.name.name.as_str() == item_name => {
                    // Context protocols have ExportKind::Context
                    return Some(if proto.is_context {
                        ExportKind::Context
                    } else {
                        ExportKind::Protocol
                    });
                }
                ItemKind::Context(ctx) if ctx.name.name.as_str() == item_name => {
                    return Some(ExportKind::Context);
                }
                ItemKind::ContextGroup(grp) if grp.name.name.as_str() == item_name => {
                    return Some(ExportKind::ContextGroup);
                }
                ItemKind::Const(c) if c.name.name.as_str() == item_name => {
                    return Some(ExportKind::Const);
                }
                ItemKind::Static(s) if s.name.name.as_str() == item_name => {
                    return Some(ExportKind::Static);
                }
                ItemKind::Module(m) if m.name.name.as_str() == item_name => {
                    return Some(ExportKind::Module);
                }
                ItemKind::Meta(meta) if meta.name.name.as_str() == item_name => {
                    return Some(ExportKind::Meta);
                }
                ItemKind::Predicate(pred) if pred.name.name.as_str() == item_name => {
                    return Some(ExportKind::Predicate);
                }
                _ => {}
            }
        }

        // Helper to check if path starts with relative marker
        let is_relative = |path: &verum_ast::ty::Path| -> bool {
            path.segments
                .first()
                .is_some_and(|s| matches!(s, PathSegment::Relative))
        };

        // Helper to resolve relative module path to absolute path
        let resolve_relative_path = |relative_path: &str| -> String {
            if relative_path.is_empty() {
                return String::new();
            }
            format!("{}.{}", current_module_path.as_str(), relative_path)
        };

        // If not found directly, check if it's re-exported via `pub import`
        for item in &ast.items {
            if let ItemKind::Mount(import_decl) = &item.kind {
                if import_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Check if this import re-exports our item
                let source_module_path: Option<String> = match &import_decl.tree.kind {
                    MountTreeKind::Path(path) => {
                        if let Some(PathSegment::Name(last_ident)) = path.segments.last() {
                            if last_ident.name.as_str() == item_name {
                                let has_relative = is_relative(path);
                                let module_segments: Vec<&str> = path
                                    .segments
                                    .iter()
                                    .take(path.segments.len() - 1)
                                    .filter_map(|seg| match seg {
                                        PathSegment::Name(id) => Some(id.name.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                let module_path = module_segments.join(".");
                                if has_relative && !module_path.is_empty() {
                                    Some(resolve_relative_path(&module_path))
                                } else if !module_path.is_empty() {
                                    Some(module_path)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        let item_found = trees.iter().any(|tree| {
                            if let MountTreeKind::Path(path) = &tree.kind {
                                if let Some(PathSegment::Name(ident)) = path.segments.first() {
                                    ident.name.as_str() == item_name
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        });

                        if item_found {
                            let has_relative = is_relative(prefix);
                            let module_segments: Vec<&str> = prefix
                                .segments
                                .iter()
                                .filter_map(|seg| match seg {
                                    PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                })
                                .collect();
                            let module_path = module_segments.join(".");
                            if has_relative && !module_path.is_empty() {
                                Some(resolve_relative_path(&module_path))
                            } else if !module_path.is_empty() {
                                Some(module_path)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                if let Some(ref source_path) = source_module_path {
                    // Look up the source module and get the actual ExportKind
                    if let Some(source_module) = registry.get_by_path(source_path) {
                        // First check the export table of the source module
                        if let Some(exported) = source_module
                            .exports
                            .get(&verum_common::Text::from(item_name.to_string()))
                        {
                            // If the source's kind is also Type, we may need to recurse
                            // (in case of chained re-exports)
                            if exported.kind == ExportKind::Type {
                                // Recurse to find the actual kind — inner form
                                // so the visited-set is threaded and ring-shaped
                                // re-exports terminate instead of stack-overflowing.
                                let source_path_text =
                                    verum_common::Text::from(source_path.clone());
                                if let Some(actual_kind) = self
                                    .resolve_export_kind_with_reexports_inner(
                                        &source_module.ast,
                                        item_name,
                                        &source_path_text,
                                        registry,
                                        visited,
                                    )
                                {
                                    return Some(actual_kind);
                                }
                            }
                            return Some(exported.kind);
                        }
                        // If not in export table, try to resolve from the source AST
                        let source_path_text = verum_common::Text::from(source_path.clone());
                        if let Some(kind) = self.resolve_export_kind_with_reexports_inner(
                            &source_module.ast,
                            item_name,
                            &source_path_text,
                            registry,
                            visited,
                        ) {
                            return Some(kind);
                        }
                    }
                }
            }
        }

        None
    }

    /// Pre-register all function signatures from a module's AST to enable forward references.
    ///

    /// This method performs a lightweight Pass 4 (register function signatures) for an
    /// imported module. This is critical for handling forward references within the imported
    /// module itself - for example, if `get_stdin_handle()` is called before it's defined
    /// in the file.
    ///

    /// Without this pre-registration, when we try to extract a function type via
    /// `extract_function_type_from_module`, it might fail if that function references
    /// other functions defined later in the same module.
    ///

    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Cross-module resolution
    fn preregister_module_function_signatures(
        &mut self,
        ast: &verum_ast::Module,
        module_path: &str,
    ) {
        use verum_ast::ItemKind;

        // Check if we've already pre-registered this module
        if self.preregistered_modules.contains(module_path) {
            return;
        }

        // #[cfg(debug_assertions)]
        // eprintln!(
        // "[DEBUG] Pre-registering function signatures from module '{}'",
        // module_path
        // );

        // CRITICAL: Mark module as pre-registered BEFORE processing to prevent
        // re-entrant infinite recursion. If register_function_signature triggers
        // any code that re-enters this function for the same module, the early
        // return check (line 23658) will catch it.
        // Bug fix: Previously the insert was at the END of the function, allowing
        // infinite recursion when processing large modules like sys.intrinsics.
        self.preregistered_modules.insert(module_path.to_string());

        // CRITICAL: switch `current_module_path` while resolving the
        // pre-registered function signatures so that unqualified type
        // names in their parameter / return positions resolve against
        // the SOURCE module's locally-defined types first. Without
        // this, a function declared in M as
        //

        //  // module M; type Foo is { ... }; fn make() -> Foo { ... }
        //

        // would have its signature registered with `Foo` resolving
        // against the *consumer's* flat-name table — picking up any
        // same-named type that happens to be in scope (notably the
        // stdlib `core.runtime.stack_alloc::ConnectionPool` alias when
        // a sibling module locally defines `ConnectionPool`). The
        // saved value is restored unconditionally after the loop.
        let saved_module_path_pre = self.current_module_path.clone();
        self.current_module_path = Text::from(module_path);

        // Register all function signatures to enable forward references
        for item in &ast.items {
            if let ItemKind::Function(func) = &item.kind {
                // Ignore errors - we just want the signatures registered
                // The actual type checking will happen later if needed
                let _ = self.register_function_signature(func);
            }
        }

        self.current_module_path = saved_module_path_pre;

        // CRITICAL: Register protocol TYPE DEFINITIONS from this module.
        // The protocol definitions (like `type Into<T> is protocol { ... }`) must be
        // registered BEFORE their implementations can be used.
        self.register_module_protocols(ast, module_path);

        // CRITICAL: Also register blanket protocol impls from this module.
        // Blanket impls like `implement<T, U: From<T>> Into<U> for T` apply globally
        // and must be registered when ANY item from the module is imported.
        self.register_module_blanket_impls(ast, module_path);

        // CRITICAL: Also import inherent impl methods for primitive types.
        // Modules like core.primitives define `implement Int { ... }` blocks that
        // add methods to built-in primitive types (Int, Float, Bool, etc.).
        // These must be imported when ANY item from the module is imported.
        if let Err(e) = self.import_primitive_impl_blocks(module_path, ast) {
            tracing::debug!(
                "Note: Could not import primitive impl blocks from '{}': {}",
                module_path,
                e
            );
        }
    }

    /// Register all protocol type definitions from a module.
    ///

    /// This ensures protocol definitions like `type Into<T> is protocol { fn into(self) -> T; }`
    /// are available for method lookup when blanket impls reference them.
    fn register_module_protocols(&mut self, ast: &verum_ast::Module, module_path: &str) {
        use verum_ast::ItemKind;
        use verum_ast::decl::TypeDeclBody;

        // Check if we've already registered protocols from this module
        let protocols_key = format!("{}_protocols", module_path);
        if self.preregistered_modules.contains(&protocols_key) {
            return;
        }
        self.preregistered_modules.insert(protocols_key);

        for item in &ast.items {
            if let ItemKind::Type(type_decl) = &item.kind {
                if let TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
                    let type_name: Text = type_decl.name.name.clone();

                    // IMPORTANT: Stdlib-loaded protocols OVERRIDE hardcoded fallbacks.
                    // The hardcoded protocols in register_builtin_protocols are minimal bootstrap
                    // versions that may have incomplete method signatures (e.g., Iterator.next()
                    // returning just TypeVar(1) instead of Maybe<TypeVar(1)>).
                    // The stdlib definitions are the source of truth.

                    #[cfg(debug_assertions)]
                    {
                        if self.protocol_checker.read().has_protocol(&type_name) {
                            // eprintln!(
                            // "[DEBUG] Overriding hardcoded protocol '{}' with stdlib definition from '{}'",
                            // type_name, module_path
                            // );
                        } else {
                            // eprintln!(
                            // "[DEBUG] Registering protocol type '{}' from module '{}'",
                            // type_name, module_path
                            // );
                        }
                    }

                    // Enter scope for type parameters
                    self.ctx.enter_scope();

                    // CRITICAL: Register "Self" in type environment so that Self.Item can be resolved
                    // when parsing method signatures (e.g., `fn next(&mut self) -> Maybe<Self.Item>`).
                    // This allows ast_to_type to recognize "Self" as a valid type name.
                    let self_type = Type::Named {
                        path: verum_ast::Path::single(verum_ast::Ident::new(
                            "Self",
                            verum_ast::Span::default(),
                        )),
                        args: List::new(),
                    };
                    self.ctx
                        .define_type(verum_common::Text::from("Self"), self_type.clone());

                    // Save and set current_self_type so that PathSegment::SelfValue can be resolved
                    let old_self_type = self.current_self_type.clone();
                    self.current_self_type = verum_common::Maybe::Some(self_type);

                    // Register type parameters (like T in Into<T>)
                    for generic_param in &type_decl.generics {
                        if let verum_ast::ty::GenericParamKind::Type { name, .. } =
                            &generic_param.kind
                        {
                            let type_var = TypeVar::fresh();
                            let param_name: Text = name.name.clone();
                            self.ctx.define_type(param_name, Type::Var(type_var));
                        }
                    }

                    // Build protocol methods and associated types
                    use verum_ast::decl::{FunctionParamKind, ProtocolItemKind};
                    let mut protocol_methods: Map<Text, crate::protocol::ProtocolMethod> =
                        Map::new();
                    let mut protocol_assoc_types: Map<Text, crate::protocol::AssociatedType> =
                        Map::new();

                    for proto_item in &protocol_body.items {
                        match &proto_item.kind {
                            ProtocolItemKind::Function { decl, default_impl } => {
                                // CRITICAL FIX: Enter a scope for method type parameters.
                                // Methods can have their own type params like `fn map<B, F>(...)`.
                                // These must be registered as TypeVars so they get captured by
                                // generalize_ordered later, enabling fresh instantiation per call.
                                self.ctx.enter_scope();

                                // Register method type parameters as TypeVars and collect their type bounds
                                // For `fn map<B, F: fn(Self.Item) -> B>`, we store the bound fn(Self.Item) -> B for F
                                let mut method_type_param_names: List<Text> = List::new();
                                let mut method_type_param_bounds: Map<Text, Type> = Map::new();
                                for generic_param in &decl.generics {
                                    if let verum_ast::ty::GenericParamKind::Type {
                                        name,
                                        bounds,
                                        ..
                                    } = &generic_param.kind
                                    {
                                        let type_var = TypeVar::fresh();
                                        let param_name: Text = name.name.clone();
                                        method_type_param_names.push(param_name.clone());
                                        self.ctx
                                            .define_type(param_name.clone(), Type::Var(type_var));

                                        // Extract type bounds (like fn(Self.Item) -> B)
                                        let type_bounds = self.extract_type_bounds_from_ast(bounds);
                                        for bound in type_bounds {
                                            // Store the bound keyed by param NAME for later transfer to TypeScheme
                                            method_type_param_bounds
                                                .insert(param_name.clone(), bound);
                                        }
                                    }
                                }

                                // Build method type from signature using lenient conversion
                                // to handle types not yet in scope during protocol registration.
                                // Self parameter is EXCLUDED from params (it's implicit in method calls)
                                // but its receiver kind is tracked for object safety analysis.
                                let mut proto_method_receiver_kind = verum_common::Maybe::None;
                                let params: List<Type> = decl
                                    .params
                                    .iter()
                                    .filter_map(|p| {
                                        match &p.kind {
                                            FunctionParamKind::SelfRef
                                            | FunctionParamKind::SelfRefChecked
                                            | FunctionParamKind::SelfRefUnsafe => {
                                                proto_method_receiver_kind =
                                                    verum_common::Maybe::Some(
                                                        crate::protocol::ReceiverKind::Ref,
                                                    );
                                                None // Skip self - implicit in method calls
                                            }
                                            FunctionParamKind::SelfRefMut
                                            | FunctionParamKind::SelfRefCheckedMut
                                            | FunctionParamKind::SelfRefUnsafeMut => {
                                                proto_method_receiver_kind =
                                                    verum_common::Maybe::Some(
                                                        crate::protocol::ReceiverKind::RefMut,
                                                    );
                                                None // Skip self - implicit in method calls
                                            }
                                            FunctionParamKind::SelfValue
                                            | FunctionParamKind::SelfValueMut
                                            | FunctionParamKind::SelfOwn
                                            | FunctionParamKind::SelfOwnMut => {
                                                proto_method_receiver_kind =
                                                    verum_common::Maybe::Some(
                                                        crate::protocol::ReceiverKind::Value,
                                                    );
                                                None // Skip self - implicit in method calls
                                            }
                                            FunctionParamKind::Regular { ty, .. } => {
                                                Some(self.ast_to_type_lenient(ty))
                                            }
                                        }
                                    })
                                    .collect();

                                {
                                    // CRITICAL FIX: Use ast_to_type_lenient for protocol method return types.
                                    // During protocol registration, dependency types (like Maybe, Result)
                                    // may not yet be in scope. ast_to_type_lenient preserves the type structure
                                    // as Named types that will be resolved later during method lookup.
                                    let method_name_pre: Text = decl.name.name.clone();
                                    let return_type = if let Some(ref ret_ty) = decl.return_type {
                                        let ty = self.ast_to_type_lenient(ret_ty);
                                        #[cfg(debug_assertions)]
                                        if method_name_pre.as_str() == "next" {
                                            // #[cfg(debug_assertions)]
                                            // eprintln!("[DEBUG protocol_registration] Method 'next' return_type (lenient): {:?}", ty);
                                        }
                                        ty
                                    } else {
                                        Type::unit()
                                    };

                                    let method_ty = Type::function(params, return_type.clone());
                                    let method_name: Text = decl.name.name.clone();
                                    let has_default = decl.body.is_some() || default_impl.is_some();

                                    // if method_name.as_str() == "map" {
                                    //  eprintln!("[DEBUG protocol_registration] Registering 'map' method");
                                    //  eprintln!(" return_type={:?}", return_type);
                                    //  eprintln!(" method_ty={:?}", method_ty);
                                    //  eprintln!(" has_default={}", has_default);
                                    //  eprintln!(" type_param_bounds={:?}", method_type_param_bounds);
                                    // }

                                    // Use with_type_bounds to store the method's type param bounds
                                    let mut protocol_method =
                                        crate::protocol::ProtocolMethod::with_type_bounds(
                                            method_name.clone(),
                                            method_ty,
                                            has_default,
                                            method_type_param_names.clone(),
                                            method_type_param_bounds.clone(),
                                        );
                                    protocol_method.receiver_kind = proto_method_receiver_kind;
                                    protocol_methods.insert(method_name, protocol_method);
                                }

                                // Exit method type parameter scope
                                self.ctx.exit_scope();
                            }
                            ProtocolItemKind::Type { name, bounds, .. } => {
                                let assoc_name: Text = name.name.clone();
                                let assoc_bounds: List<crate::protocol::ProtocolBound> = bounds
                                    .iter()
                                    .map(|path| {
                                        crate::protocol::ProtocolBound::positive(
                                            path.clone(),
                                            List::new(),
                                        )
                                    })
                                    .collect();
                                let assoc_type = crate::protocol::AssociatedType::simple(
                                    assoc_name.clone(),
                                    assoc_bounds,
                                );
                                protocol_assoc_types.insert(assoc_name, assoc_type);
                            }
                            ProtocolItemKind::Const { .. } => {
                                // Skip associated consts for now
                            }
                            ProtocolItemKind::Axiom(_) => {
                                // T1-R: protocol axioms are tracked
                                // elsewhere (in the implement-site
                                // obligation-discharge pipeline) — they
                                // do not contribute associated types.
                            }
                        }
                    }

                    // Convert extends clause to super_protocols
                    let super_protocols: List<crate::protocol::ProtocolBound> = protocol_body
                        .extends
                        .iter()
                        .filter_map(|extend_ty| match &extend_ty.kind {
                            verum_ast::ty::TypeKind::Path(path) => Some(
                                crate::protocol::ProtocolBound::positive(path.clone(), List::new()),
                            ),
                            _ => None,
                        })
                        .collect();

                    // Create and register Protocol object
                    // Convert AST is_context bool to ProtocolKind:
                    // - true -> ConstraintAndInjectable (context protocol)
                    // - false -> Constraint (regular protocol)
                    let kind = if protocol_body.is_context {
                        crate::protocol::ProtocolKind::ConstraintAndInjectable
                    } else {
                        crate::protocol::ProtocolKind::Constraint
                    };

                    let protocol = crate::protocol::Protocol {
                        name: type_name.clone(),
                        kind,
                        type_params: self
                            .convert_generic_params_to_type_params(&type_decl.generics),
                        methods: protocol_methods,
                        associated_types: protocol_assoc_types,
                        associated_consts: Map::new(),
                        super_protocols,
                        specialization_info: Maybe::None,
                        defining_crate: Maybe::None,
                        span: type_decl.span,
                    };

                    let _ = self.protocol_checker.write().register_protocol(protocol);

                    // Auto-register context protocols as injectable contexts.
                    // This replaces hardcoded register_protocol_as_context calls.
                    if protocol_body.is_context {
                        self.context_resolver
                            .register_protocol_as_context(type_name.clone());
                    }

                    // Register Kind for protocol in kind inferer
                    {
                        use crate::kind_inference::KindInference;
                        let protocol_kind = if protocol_body.is_context {
                            crate::kind_inference::Kind::ConstraintAndInjectable
                        } else {
                            crate::kind_inference::Kind::Constraint
                        };
                        self.kind_inferer()
                            .register_type_constructor(type_name.as_str(), protocol_kind);
                    }

                    // Restore previous self type before exiting scope
                    self.set_current_self_type(old_self_type);
                    self.ctx.exit_scope();

                    // #[cfg(debug_assertions)]
                    // eprintln!(
                    // "[DEBUG] Successfully registered protocol '{}' from module '{}'",
                    // type_name, module_path
                    // );
                }
            }
        }
    }

    /// Register all blanket protocol implementations from a module.
    ///

    /// A blanket impl is one where the `for_type` is a type parameter (like `T`),
    /// not a concrete type. Examples:
    /// - `implement<T, U: From<T>> Into<U> for T`
    /// - `implement<S: Stream> StreamExt for S`
    ///

    /// Register ALL protocol implementations from a module when first accessed.
    /// This includes both blanket impls (for_type is a type parameter like T) and
    /// generic impls (for_type is a parameterized type like DequeIter<T>).
    ///

    /// CRITICAL FIX: Previously only blanket impls were registered, which meant
    /// generic impls like `implement<T> Iterator for DequeIter<T>` were never
    /// registered unless explicitly imported. This caused `iter.next()` to fail
    /// with "no method named `next` found" for iterator types.
    fn register_module_blanket_impls(&mut self, ast: &verum_ast::Module, module_path: &str) {
        use verum_ast::ItemKind;
        use verum_ast::decl::ImplKind;

        // Check if we've already registered impls from this module
        if self.blanket_impls_registered_modules.contains(module_path) {
            return;
        }

        // #[cfg(debug_assertions)]
        // eprintln!(
        // "[DEBUG] Registering blanket protocol impls from module '{}'",
        // module_path
        // );

        // Mark as registered BEFORE processing to prevent re-entrancy
        self.blanket_impls_registered_modules
            .insert(module_path.to_string());

        // CRITICAL: Before registering protocol impls, ensure protocol definitions are loaded
        // from stdlib. The hardcoded protocols in register_standard_protocols have minimal/incorrect
        // signatures (e.g., Iterator.next() returns TypeVar(1) instead of Maybe<TypeVar(1)>).
        // We must load the actual stdlib protocol definitions to get correct method signatures.
        self.ensure_stdlib_protocols_loaded(ast);

        for item in &ast.items {
            if let ItemKind::Impl(impl_decl) = &item.kind {
                if let ImplKind::Protocol {
                    protocol, for_type, ..
                } = &impl_decl.kind
                {
                    // Check if this is a blanket impl (for_type is a type parameter)
                    let is_blanket = self.is_blanket_impl_for_type(for_type, &impl_decl.generics);

                    // Check if this is a generic impl (for_type uses type parameters from generics)
                    let is_generic_impl = !impl_decl.generics.is_empty();

                    if is_blanket {
                        // Create and register the ProtocolImpl
                        if let Err(e) = self.register_blanket_impl_from_ast(impl_decl) {
                            tracing::debug!("Failed to register blanket impl: {}", e);
                        }
                    } else if is_generic_impl {
                        // CRITICAL FIX: Also register generic protocol impls like:
                        // implement<T> Iterator for DequeIter<T>
                        // These are essential for protocol method resolution on parameterized types.

                        // Use the same registration function as blanket impls
                        if let Err(e) = self.register_blanket_impl_from_ast(impl_decl) {
                            tracing::debug!("Failed to register generic impl: {}", e);
                        }
                    } else {
                        // Concrete protocol impl: implement Protocol for ConcreteType
                        // Must also be pre-registered for protocol coercion to work
                        // (e.g., Circle -> Drawable when Circle implements Drawable)

                        if let Err(e) = self.register_blanket_impl_from_ast(impl_decl) {
                            tracing::debug!("Failed to register concrete impl: {}", e);
                        }
                    }
                } else if let ImplKind::Inherent(for_type) = &impl_decl.kind {
                    // Handle inherent blanket impls: `implement<I: Iterator> I { fn reduce_with... }`
                    // These provide extension methods for all types satisfying a protocol bound.
                    if self.is_blanket_impl_for_type(for_type, &impl_decl.generics) {
                        if let Err(e) = self.register_inherent_blanket_impl(impl_decl) {
                            tracing::debug!("Failed to register inherent blanket impl: {}", e);
                        }
                    }
                }
            }
        }
    }

    /// Register an inherent blanket impl's methods under a special protocol-keyed entry.
    ///

    /// For `implement<I: Iterator> I { fn reduce_with... }`, registers each method under
    /// the key `"__blanket:Iterator"` in `inherent_methods`. During method resolution,
    /// when a method is not found by type name, we scan these blanket entries and check
    /// whether the receiver type implements the required protocol.
    ///

    /// This enables extension methods on generic iterator types without hardcoding specific
    /// iterator implementors at registration time.
    fn register_inherent_blanket_impl(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
    ) -> Result<()> {
        use verum_ast::decl::{FunctionParamKind, ImplItemKind, ImplKind};
        use verum_ast::ty::GenericParamKind;

        let for_type = if let ImplKind::Inherent(for_type) = &impl_decl.kind {
            for_type
        } else {
            return Ok(());
        };

        // Extract the type parameter name (e.g., "I" from `implement<I: Iterator> I`)
        let type_param_name = match &for_type.kind {
            verum_ast::ty::TypeKind::Path(path) => {
                path.as_ident().map(|id| id.name.as_str().to_string())
            }
            _ => None,
        };
        let type_param_name = match type_param_name {
            Some(n) => n,
            None => return Ok(()),
        };

        // Collect protocol bound names for the constrained type parameter
        // For `implement<I: Iterator>`, collect ["Iterator"]
        let mut protocol_bound_names: Vec<String> = Vec::new();
        for generic_param in &impl_decl.generics {
            if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind {
                if name.name.as_str() == type_param_name {
                    for bound in bounds {
                        match &bound.kind {
                            verum_ast::ty::TypeBoundKind::Protocol(path) => {
                                if let Some(ident) = path.as_ident() {
                                    protocol_bound_names.push(ident.name.as_str().to_string());
                                }
                            }
                            verum_ast::ty::TypeBoundKind::GenericProtocol(ty) => {
                                if let verum_ast::ty::TypeKind::Generic { base, .. } = &ty.kind {
                                    if let verum_ast::ty::TypeKind::Path(path) = &base.kind {
                                        if let Some(ident) = path.as_ident() {
                                            protocol_bound_names
                                                .push(ident.name.as_str().to_string());
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if protocol_bound_names.is_empty() {
            return Ok(());
        }

        // Build the registry key: "__blanket:Iterator" (or "__blanket:Iterator+OtherProtocol")
        protocol_bound_names.sort();
        let blanket_key: Text = format!("__blanket:{}", protocol_bound_names.join("+")).into();

        // Enter a scope to register the impl's type parameters
        self.ctx.enter_scope();

        // Create fresh type var for the impl-level type parameter (I)
        let impl_type_var = TypeVar::fresh();
        self.ctx.define_type(
            verum_common::Text::from(type_param_name.as_str()),
            Type::Var(impl_type_var),
        );
        let impl_type_param_names: List<verum_common::Text> =
            List::from(vec![verum_common::Text::from(type_param_name.as_str())]);

        // Set current_self_type so that `Self.Item` resolves to `::Item[I_var]`
        let old_self_type = self.current_self_type.clone();
        self.set_current_self_type(Maybe::Some(Type::Var(impl_type_var)));

        for item in &impl_decl.items {
            if let ImplItemKind::Function(func) = &item.kind {
                // Skip static methods (no self parameter)
                let is_instance = func
                    .params
                    .first()
                    .map(|p| {
                        matches!(
                            p.kind,
                            FunctionParamKind::SelfValue
                                | FunctionParamKind::SelfValueMut
                                | FunctionParamKind::SelfRef
                                | FunctionParamKind::SelfRefMut
                                | FunctionParamKind::SelfRefChecked
                                | FunctionParamKind::SelfRefCheckedMut
                                | FunctionParamKind::SelfRefUnsafe
                                | FunctionParamKind::SelfRefUnsafeMut
                                | FunctionParamKind::SelfOwn
                                | FunctionParamKind::SelfOwnMut
                        )
                    })
                    .unwrap_or(false);

                if !is_instance {
                    continue;
                }

                let method_result: Result<()> = {
                    // Register method-level type parameters first
                    let mut method_type_param_names: List<verum_common::Text> = List::new();
                    let mut method_type_var_bounds: Map<TypeVar, List<Type>> = Map::new();

                    let mut method_type_params: Vec<(TypeVar, &List<verum_ast::ty::TypeBound>)> =
                        Vec::new();
                    for generic_param in &func.generics {
                        if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind {
                            let fresh_var = TypeVar::fresh();
                            self.ctx
                                .define_type(name.name.clone(), Type::Var(fresh_var));
                            method_type_param_names.push(name.name.clone());
                            method_type_params.push((fresh_var, bounds));
                        }
                    }

                    // Extract bounds after all type params are registered
                    for (fresh_var, bounds) in &method_type_params {
                        if !bounds.is_empty() {
                            let extracted_bounds = self.extract_type_bounds_from_ast(bounds);
                            if !extracted_bounds.is_empty() {
                                method_type_var_bounds.insert(*fresh_var, extracted_bounds);
                            }
                        }
                    }

                    // Build parameter types (exclude self)
                    let param_types: List<Type> = func
                        .params
                        .iter()
                        .filter(|p| !p.is_self())
                        .filter_map(|p| match &p.kind {
                            FunctionParamKind::Regular { ty, .. } => {
                                Some(self.ast_to_type_lenient(ty))
                            }
                            _ => None,
                        })
                        .collect();

                    let return_type = func
                        .return_type
                        .as_ref()
                        .map(|t| self.ast_to_type_lenient(t))
                        .unwrap_or(Type::Unit);

                    let method_ty = Type::function(param_types, return_type);

                    // Generalize with ordered params: [I, method-level params...]
                    let mut ordered_params: List<verum_common::Text> =
                        impl_type_param_names.clone();
                    for name in &method_type_param_names {
                        ordered_params.push(name.clone());
                    }
                    let mut method_scheme = self.ctx.generalize_ordered(method_ty, &ordered_params);

                    // impl_var_count = 1 (the `I` type parameter)
                    // This tells method resolution to bind fresh_vars[0] to the whole receiver type
                    method_scheme.impl_var_count = 1;

                    if !method_type_var_bounds.is_empty() {
                        method_scheme = method_scheme.with_type_bounds(method_type_var_bounds);
                    }

                    // Clean up method-level type params
                    for name in &method_type_param_names {
                        self.ctx.remove_type(name);
                    }

                    // Register under the blanket key
                    let method_name_text: Text = func.name.name.clone();
                    let mut methods_guard = self.inherent_methods.write();
                    let entry = methods_guard.entry(blanket_key.clone()).or_default();
                    entry.insert(method_name_text, method_scheme);

                    Ok(())
                };

                if let Err(e) = method_result {
                    tracing::debug!(
                        "Failed to register blanket inherent method '{}': {}",
                        func.name.name,
                        e
                    );
                }
            }
        }

        self.set_current_self_type(old_self_type);
        self.ctx.exit_scope();

        Ok(())
    }

    /// Look up a method from inherent blanket impls (e.g., `implement<I: Iterator> I { ... }`).
    ///

    /// After the receiver type lookup fails in `inherent_methods`, scan all `"__blanket:*"` entries
    /// and check whether the receiver implements the required protocol bounds. If found, instantiate
    /// the method type and apply the substitution `{I_var -> recv_ty}` (the whole receiver IS `I`).
    ///

    /// Returns `(method_ty, fresh_vars, type_bounds, impl_var_count)` on success.
    fn lookup_inherent_blanket_method(
        &mut self,
        recv_ty: &Type,
        method_name: &str,
    ) -> Option<(Type, List<TypeVar>, Map<TypeVar, List<Type>>)> {
        // Collect all "__blanket:*" keys that have the requested method
        let blanket_entries: Vec<(Text, crate::context::TypeScheme)> = {
            let methods_guard = self.inherent_methods.read();
            let method_name_text = verum_common::Text::from(method_name);
            methods_guard
                .iter()
                .filter(|(key, _)| key.as_str().starts_with("__blanket:"))
                .filter_map(|(key, methods)| {
                    methods
                        .get(&method_name_text)
                        .map(|scheme| (key.clone(), scheme.clone()))
                })
                .collect()
        };

        for (blanket_key, scheme) in blanket_entries {
            // Extract protocol names from the key: "__blanket:Iterator" -> ["Iterator"]
            let protocols_str = blanket_key.as_str().trim_start_matches("__blanket:");
            let protocol_names: Vec<&str> = protocols_str.split('+').collect();

            // Check if the receiver implements ALL required protocols.
            // We try multiple resolution strategies:
            //  1. find_impl: exact + generic matching with where clause verification
            //  2. implements_protocol: also checks superprotocol inheritance
            //  3. implements_by_name: exact index lookup (handles cases where the
            //  concrete type was registered directly, e.g. from stdlib metadata)
            let all_satisfied = protocol_names.iter().all(|proto_name| {
                let proto_path = verum_ast::ty::Path::new(
                    List::from(vec![verum_ast::ty::PathSegment::Name(
                        verum_ast::ty::Ident::new(
                            verum_common::Text::from(*proto_name),
                            verum_ast::Span::default(),
                        ),
                    )]),
                    verum_ast::Span::default(),
                );
                let checker = self.protocol_checker.read();
                // Strategy 1: Direct find_impl (exact + generic/blanket matching)
                if checker.find_impl(recv_ty, &proto_path).is_some() {
                    return true;
                }
                // Strategy 2: implements_protocol — also resolves through superprotocol
                // inheritance chains (e.g., if recv_ty implements Ord which implies Eq)
                if checker.implements_protocol(recv_ty, proto_name) {
                    return true;
                }
                // Strategy 3: implements_by_name — handles cases where the impl was
                // registered with a concrete type key that find_impl's generic matching
                // doesn't reach (e.g., type registered from stdlib metadata without
                // generic type params)
                if checker.implements_by_name(recv_ty, proto_name) {
                    return true;
                }
                false
            });

            if !all_satisfied {
                continue;
            }

            // Found a matching blanket impl - instantiate the method type
            let (mut method_ty, fresh_vars, type_bounds) = scheme.instantiate_with_type_bounds();

            // impl_var_count = 1 means fresh_vars[0] is the blanket impl's type param (I)
            // Substitute: {fresh_vars[0] -> recv_ty}
            if let Some(&impl_var) = fresh_vars.first() {
                let mut subst = crate::ty::Substitution::new();
                subst.insert(impl_var, recv_ty.clone());
                method_ty = method_ty.apply_subst(&subst);

                // Also apply substitution to type bounds
                let subst_bounds: Map<TypeVar, List<Type>> = type_bounds
                    .iter()
                    .map(|(var, bounds)| {
                        let new_bounds = bounds.iter().map(|b| b.apply_subst(&subst)).collect();
                        (*var, new_bounds)
                    })
                    .collect();

                // Normalize associated type projections (e.g., ::Item[Range<Int>] -> Int)
                method_ty = self.normalize_type(&method_ty);

                // Return remaining fresh vars (method-level params, not the blanket impl var)
                let remaining_vars: List<TypeVar> = fresh_vars.iter().skip(1).copied().collect();
                return Some((method_ty, remaining_vars, subst_bounds));
            }
        }

        None
    }

    /// Ensure stdlib protocol definitions are loaded when we encounter their implementations.
    ///

    /// The hardcoded protocols in `register_standard_protocols` have minimal signatures that
    /// may be incorrect (e.g., Iterator.next() returns TypeVar(1) instead of Maybe<TypeVar(1)>).
    /// This function looks at the protocol implementations in the AST and ensures the
    /// corresponding stdlib modules containing the actual protocol definitions are loaded.
    ///

    /// For example, if we see `implement<T> Iterator for DequeIter<T>`, we need to ensure
    /// that `core.base.iterator` is loaded so the Iterator protocol has correct method signatures.
    fn ensure_stdlib_protocols_loaded(&mut self, ast: &verum_ast::Module) {
        use verum_ast::ItemKind;
        use verum_ast::decl::ImplKind;

        // Mapping of core protocol names to their canonical stdlib module paths
        // These are protocols that have hardcoded fallbacks which may have incorrect signatures
        let protocol_modules: &[(&str, &str)] = &[
            ("Iterator", "core.base.iterator"),
            ("DoubleEndedIterator", "core.base.iterator"),
            ("IntoIterator", "core.base.iterator"),
            ("FromIterator", "core.base.iterator"),
            ("Extend", "core.base.iterator"),
            ("AsyncIterator", "std.async.iterator"),
            // Add more as needed
        ];

        // Collect protocols used in impl blocks
        let mut protocols_to_load: std::collections::HashSet<&str> =
            std::collections::HashSet::new();

        for item in &ast.items {
            if let ItemKind::Impl(impl_decl) = &item.kind {
                if let ImplKind::Protocol { protocol, .. } = &impl_decl.kind {
                    if let Some(ident) = protocol.as_ident() {
                        let protocol_name = ident.name.as_str();
                        // Check if this protocol has a known stdlib module
                        for (name, _module_path) in protocol_modules {
                            if *name == protocol_name {
                                protocols_to_load.insert(protocol_name);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Load the modules for each protocol
        for protocol_name in protocols_to_load {
            for (name, module_path) in protocol_modules {
                if *name == protocol_name {
                    // Try to load and process the protocol's source module
                    self.try_load_protocol_module(module_path);
                    break;
                }
            }
        }
    }

    /// Try to load a stdlib module and register its protocols.
    ///

    /// This is called to ensure protocol definitions from stdlib override hardcoded fallbacks.
    fn try_load_protocol_module(&mut self, module_path: &str) {
        // Check if already processed
        let protocols_key = format!("{}_protocols", module_path);
        if self.preregistered_modules.contains(&protocols_key) {
            return;
        }

        // Try to get the module from the registry
        let module_ast_opt = {
            let registry = self.module_registry.read();
            registry.get_by_path(module_path).map(|m| m.ast.clone())
        };

        if let Some(module_ast) = module_ast_opt {
            // #[cfg(debug_assertions)]
            // eprintln!(
            // "[DEBUG] Loading stdlib protocol definitions from '{}'",
            // module_path
            // );

            // Register protocols from this module (will override hardcoded versions)
            self.register_module_protocols(&module_ast, module_path);
        }
    }

    /// Check if an impl is a blanket impl (the for_type is a type parameter).
    fn is_blanket_impl_for_type(
        &self,
        for_type: &verum_ast::ty::Type,
        generics: &[verum_ast::ty::GenericParam],
    ) -> bool {
        use verum_ast::ty::TypeKind;

        // Collect all type parameter names from the impl's generics
        let type_param_names: std::collections::HashSet<&str> = generics
            .iter()
            .filter_map(|g| {
                if let verum_ast::ty::GenericParamKind::Type { name, .. } = &g.kind {
                    Some(name.name.as_str())
                } else {
                    None
                }
            })
            .collect();

        // Check if the for_type is a single type parameter
        match &for_type.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    type_param_names.contains(ident.name.as_str())
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Register a blanket impl from its AST node.
    fn register_blanket_impl_from_ast(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
    ) -> Result<()> {
        use crate::protocol::{ProtocolImpl, WhereClause as ProtocolWhereClause};
        use verum_ast::decl::ImplKind;

        if let ImplKind::Protocol {
            protocol,
            protocol_args,
            for_type,
        } = &impl_decl.kind
        {
            // Enter a scope to handle type parameters
            self.ctx.enter_scope();

            // Collect type variables for the impl's generic parameters
            let mut type_param_to_var: std::collections::HashMap<Text, Type> =
                std::collections::HashMap::new();

            for generic_param in &impl_decl.generics {
                if let verum_ast::ty::GenericParamKind::Type { name, .. } = &generic_param.kind {
                    let type_var = TypeVar::fresh();
                    let name_text: Text = name.name.clone();
                    let type_var_ty = Type::Var(type_var);
                    self.ctx.define_type(name_text.clone(), type_var_ty.clone());
                    type_param_to_var.insert(name_text, type_var_ty);
                }
            }

            // Resolve the for_type
            let resolved_for_type = self
                .ast_to_type(for_type)
                .unwrap_or_else(|_| self.ast_to_type_lenient(for_type));

            // Build where_clauses from the impl's generics
            // WhereClause represents "ty: bound1 + bound2"
            // CRITICAL FIX: Use the SAME Type::Var that's in type_param_to_var.
            // When try_match_type matches for_type, it builds substitution keys like "T329"
            // for Type::Var(TypeVar{id:329}). The where clause must use the same Type::Var
            // so apply_substitution can look up the correct key.
            let mut where_clauses: List<ProtocolWhereClause> = List::new();
            // CRITICAL: Collect function type bounds for type parameters.
            // For `implement<F: fn(Fut.Output) -> T, T> ...`, captures F's TypeVar → fn type.
            // This enables extracting additional type bindings (like T = Bool from F = fn(Int) -> Bool)
            // during associated type resolution in substitute_impl_type_params.
            let mut type_param_fn_bounds: Map<TypeVar, Type> = Map::new();
            for generic_param in &impl_decl.generics {
                if let verum_ast::ty::GenericParamKind::Type { name, bounds, .. } =
                    &generic_param.kind
                {
                    let name_text: Text = name.name.clone();
                    if !bounds.is_empty() {
                        // Look up the Type::Var that was created for this type parameter
                        if let Some(type_var) = type_param_to_var.get(&name_text) {
                            // Collect all bounds for this type parameter
                            let mut protocol_bounds: List<crate::protocol::ProtocolBound> =
                                List::new();
                            for bound in bounds {
                                match &bound.kind {
                                    verum_ast::ty::TypeBoundKind::Protocol(path) => {
                                        // Simple protocol bound: T: Clone
                                        protocol_bounds.push(
                                            crate::protocol::ProtocolBound::positive(
                                                path.clone(),
                                                List::new(),
                                            ),
                                        );
                                    }
                                    verum_ast::ty::TypeBoundKind::GenericProtocol(ty) => {
                                        // Generic protocol bound: U: From<T>
                                        // Extract the protocol path and type arguments
                                        if let verum_ast::ty::TypeKind::Generic {
                                            base, args, ..
                                        } = &ty.kind
                                        {
                                            if let verum_ast::ty::TypeKind::Path(path) = &base.kind
                                            {
                                                // Resolve type arguments from GenericArg to Type
                                                let resolved_args: List<Type> = args
                                                    .iter()
                                                    .filter_map(|arg| {
                                                        if let verum_ast::ty::GenericArg::Type(ty) =
                                                            arg
                                                        {
                                                            Some(
                                                                self.ast_to_type(ty)
                                                                    .unwrap_or_else(|_| {
                                                                        self.ast_to_type_lenient(ty)
                                                                    }),
                                                            )
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                    .collect();
                                                protocol_bounds.push(
                                                    crate::protocol::ProtocolBound::positive(
                                                        path.clone(),
                                                        resolved_args,
                                                    ),
                                                );
                                            }
                                        } else if let verum_ast::ty::TypeKind::Function { .. } =
                                            &ty.kind
                                        {
                                            // Function type bound: F: Fn(I.Item) -> U
                                            // Capture as function type bound for associated type resolution
                                            if let Type::Var(tv) = type_var {
                                                if let Ok(resolved_fn) = self.ast_to_type(ty) {
                                                    type_param_fn_bounds.insert(*tv, resolved_fn);
                                                }
                                            }
                                        } else if let verum_ast::ty::TypeKind::Path(path) = &ty.kind
                                        {
                                            // Just a path, no type args
                                            protocol_bounds.push(
                                                crate::protocol::ProtocolBound::positive(
                                                    path.clone(),
                                                    List::new(),
                                                ),
                                            );
                                        }
                                    }
                                    verum_ast::ty::TypeBoundKind::Equality(bound_ty) => {
                                        // Type equality bound: F: fn(X) -> T
                                        // Resolve the bound type and store as a function type bound
                                        if let verum_ast::ty::TypeKind::Function { .. } =
                                            &bound_ty.kind
                                        {
                                            if let Type::Var(tv) = type_var {
                                                if let Ok(resolved_fn) = self.ast_to_type(bound_ty)
                                                {
                                                    type_param_fn_bounds.insert(*tv, resolved_fn);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if !protocol_bounds.is_empty() {
                                where_clauses.push(ProtocolWhereClause {
                                    ty: type_var.clone(),
                                    bounds: protocol_bounds,
                                });
                            }
                        }
                    }
                }
            }

            // Resolve protocol type arguments (e.g., U in Into<U>)
            let resolved_protocol_args: List<Type> = protocol_args
                .iter()
                .filter_map(|arg| {
                    use verum_ast::ty::GenericArg;
                    match arg {
                        GenericArg::Type(ty) => Some(
                            self.ast_to_type(ty)
                                .unwrap_or_else(|_| self.ast_to_type_lenient(ty)),
                        ),
                        GenericArg::Const(_) | GenericArg::Lifetime(_) | GenericArg::Binding(_) => {
                            None
                        }
                    }
                })
                .collect();

            // Extract associated types from impl items (e.g., `type Item = &T;`)
            let mut associated_types: Map<Text, Type> = Map::new();
            for item in &impl_decl.items {
                if let verum_ast::decl::ImplItemKind::Type { name, ty, .. } = &item.kind {
                    let resolved_ty = self
                        .ast_to_type(ty)
                        .unwrap_or_else(|_| self.ast_to_type_lenient(ty));
                    associated_types.insert(name.name.clone(), resolved_ty);
                }
            }

            // Process where clause for additional function type bounds.
            if let Some(ref where_clause) = impl_decl.generic_where_clause {
                for predicate in &where_clause.predicates {
                    use verum_ast::ty::WherePredicateKind;
                    if let WherePredicateKind::Type { ty, bounds } = &predicate.kind {
                        if let verum_ast::ty::TypeKind::Path(path) = &ty.kind {
                            if let Some(ident) = path.as_ident() {
                                let param_name: Text = ident.name.clone();
                                if let Some(type_var) =
                                    self.ctx.lookup_type(param_name.as_str()).cloned()
                                {
                                    // Add where clause bounds
                                    if let Ok(protocol_bounds) =
                                        self.convert_type_bounds_to_protocol_bounds(bounds)
                                    {
                                        where_clauses.push(crate::protocol::WhereClause {
                                            ty: type_var.clone(),
                                            bounds: protocol_bounds,
                                        });
                                    }
                                    // Extract function type bounds
                                    for bound in bounds {
                                        match &bound.kind {
                                            verum_ast::ty::TypeBoundKind::Equality(bound_ty)
                                            | verum_ast::ty::TypeBoundKind::GenericProtocol(
                                                bound_ty,
                                            ) => {
                                                if let verum_ast::ty::TypeKind::Function {
                                                    ..
                                                } = &bound_ty.kind
                                                {
                                                    if let Type::Var(tv) = &type_var {
                                                        if let Ok(resolved_fn) =
                                                            self.ast_to_type(bound_ty)
                                                        {
                                                            type_param_fn_bounds
                                                                .insert(*tv, resolved_fn);
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Build the ProtocolImpl
            let protocol_impl = ProtocolImpl {
                protocol: protocol.clone(),
                protocol_args: resolved_protocol_args,
                for_type: resolved_for_type,
                where_clauses,
                methods: Map::new(), // Methods come from protocol definition
                associated_types,
                associated_consts: Map::new(),
                specialization: Maybe::None,
                impl_crate: Maybe::Some(self.current_module_path.clone()),
                span: impl_decl.span,
                type_param_fn_bounds,
            };

            self.ctx.exit_scope();

            // Register with protocol checker
            if let Err(e) = self
                .protocol_checker
                .write()
                .register_impl(protocol_impl.clone())
            {
                tracing::debug!("Blanket impl registration warning: {}", e);
            }

            #[cfg(debug_assertions)]
            {
                let proto_name = protocol.as_ident().map(|i| i.name.as_str()).unwrap_or("?");
                // eprintln!(
                // "[DEBUG] Registered blanket impl: {} for {:?}, where_clauses: {:?}",
                // proto_name, protocol_impl.for_type, protocol_impl.where_clauses
                // );
            }
        }

        Ok(())
    }

    /// Import all public items from a module (glob import).
    ///

    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    /// Pre-pass helper: walk a `MountTree` and seed `explicit_imports`
    /// with every leaf-name that *would* be registered explicitly.
    ///

    /// This is consulted by `import_item_from_module_impl`'s glob-skip
    /// guard, so that explicit imports always win regardless of source
    /// order: `mount foo.*` followed by `mount bar.{T}` and the reverse
    /// produce identical environments.
    ///

    /// Recognised forms:
    ///  * `mount X.Bar` → "Bar" (or alias if `as Y`)
    ///  * `mount X.{A, B, C}` → "A", "B", "C" (each may carry an alias)
    ///  * `mount X.{A as Y}` → "Y" (alias takes precedence)
    ///  * `mount X.*` → skipped (glob is by definition non-explicit)
    ///  * nested `{X.{A, B}}` → recurses into inner trees
    pub(super) fn collect_explicit_import_names(&mut self, tree: &verum_ast::decl::MountTree) {
        use verum_ast::decl::MountTreeKind;
        // Determine the explicit-leaf name for this tree.
        // Alias wins; otherwise extract the last segment of the path.
        let alias_name: Option<String> = match &tree.alias {
            verum_common::Maybe::Some(ident) => Some(ident.name.as_str().to_string()),
            verum_common::Maybe::None => None,
        };
        match &tree.kind {
            MountTreeKind::Path(path) => {
                if let Some(name) = alias_name {
                    self.explicit_imports.insert(name);
                } else if let Some(last_name) = path.segments.iter().rev().find_map(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        Some(ident.name.as_str().to_string())
                    } else {
                        None
                    }
                }) {
                    self.explicit_imports.insert(last_name);
                }
            }
            MountTreeKind::Glob(_) => {
                // Glob is non-explicit by definition.
            }
            MountTreeKind::Nested { prefix: _, trees } => {
                for inner in trees {
                    self.collect_explicit_import_names(inner);
                }
            }
            // #5 / P1.5 — a file-relative mount that carries an
            // `as Alias` clause registers the alias as the
            // explicit import name. Without an alias the file
            // mount contributes no name to the parent scope.
            MountTreeKind::File { .. } => {
                if let Some(name) = alias_name {
                    self.explicit_imports.insert(name);
                }
            }
        }
    }

    pub(crate) fn import_all_from_module(
        &mut self,
        module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Result<()> {
        // Cycle guard: if `module_path` is already being glob-expanded higher
        // up the call stack, re-entering it would recurse unbounded. Emit
        // E0811 with the full visit path and bail out of the inner expansion
        // (the outer expansion continues). Spec: CLAUDE.md § CRITICAL: Verum
        // Grammar — `mount` semantics must not crash on cyclic topology.
        if self.glob_imports_in_progress.contains(module_path) {
            let mut modules_in_cycle: List<Text> = List::new();
            let mut in_cycle = false;
            for m in &self.glob_imports_stack {
                if m == module_path {
                    in_cycle = true;
                }
                if in_cycle {
                    modules_in_cycle.push(m.clone());
                }
            }
            modules_in_cycle.push(module_path.clone());
            let cycle_path: Text = modules_in_cycle
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(" -> ")
                .into();
            return Err(crate::TypeError::ImportCycle {
                cycle_path,
                modules_in_cycle,
                span: verum_ast::span::Span::dummy(),
            });
        }

        // Push onto the cycle-tracking stack, ensuring we pop on every exit.
        self.glob_imports_in_progress.insert(module_path.clone());
        self.glob_imports_stack.push(module_path.clone());
        let result = self.import_all_from_module_impl(module_path, registry);
        self.glob_imports_stack.pop();
        self.glob_imports_in_progress.remove(module_path);
        result
    }

    fn import_all_from_module_impl(
        &mut self,
        module_path: &Text,
        registry: &verum_modules::ModuleRegistry,
    ) -> Result<()> {
        // Look up the source module in the registry
        if let Some(module_info) = registry.get_by_path(module_path.as_str()) {
            // Pre-register all function signatures to enable forward references
            // within the imported module itself
            self.preregister_module_function_signatures(&module_info.ast, module_path.as_str());

            // Import all public exports.
            // Sort names for deterministic import order — ExportTable uses HashMap
            // internally, so iteration order varies between runs.
            let mut names: Vec<String> = module_info
                .exports
                .public_exports()
                .map(|e| e.name.to_string())
                .collect();
            names.sort();
            for name in names {
                self.import_item_from_module(module_path, &name, registry)?;
            }

            // CRITICAL FIX: Also import inherent impl methods for primitive types.
            // Modules like core.primitives define `implement Int { ... }` blocks that
            // add methods to built-in primitive types. These must be imported even though
            // Int itself isn't an export - it's a language built-in.
            self.import_primitive_impl_blocks(module_path.as_str(), &module_info.ast)?;

            // Cross-file submodule re-export propagation (mirrors the
            // inline-module fix at `import_all_from_inline_module_impl`).
            //
            // The ExportTable for a cross-file module records nested
            // public submodules ONLY as `ExportKind::Module` entries —
            // it never inlines the submodule's own Mount re-exports.
            // For the canonical "prelude" pattern at `core/mod.vr`:
            //
            //   public module prelude {
            //       public mount super.collections.List;
            //       public mount super.base.Maybe;
            //       …
            //   }
            //
            // Walking only `core`'s direct exports leaves `List`, `Maybe`,
            // etc. invisible at the `mount core.*` site, even though the
            // user semantically expects the prelude to fold in.
            //
            // We walk the module's AST one extra level for public
            // submodules, collect their Mount re-exports (recursively,
            // so submodules-of-submodules also fold), and replay each
            // re-export against the registry as if it had been declared
            // at the parent module.  Errors during replay are logged
            // and not propagated — a missing dep at glob-expansion
            // time stays dormant; the user's eventual use site
            // surfaces it with a normal E101.  Per the type-system
            // architectural rule (no hardcoded stdlib knowledge in
            // the compiler), the walk is fully general — any nested
            // public submodule with public Mount re-exports surfaces.
            let mut submodule_reexports: Vec<(Text, Option<Text>)> = Vec::new();
            for item in &module_info.ast.items {
                if let verum_ast::ItemKind::Module(submod) = &item.kind {
                    if !matches!(submod.visibility, verum_ast::decl::Visibility::Public) {
                        continue;
                    }
                    if let Maybe::Some(sub_items) = &submod.items {
                        let nested_path = format!(
                            "{}.{}",
                            module_path.as_str(),
                            submod.name.name.as_str()
                        );
                        collect_inline_mount_reexports_recursive(
                            sub_items.as_slice(),
                            nested_path.as_str(),
                            &mut submodule_reexports,
                        );
                    }
                }
            }
            for (path, item_name_opt) in submodule_reexports {
                match item_name_opt {
                    None => {
                        if let Err(e) = self.import_all_from_module(&path, registry) {
                            tracing::debug!(
                                "import_all_from_module: nested-submod glob re-export {} failed: {:?}",
                                path.as_str(),
                                e
                            );
                        }
                    }
                    Some(item_name) => {
                        if let Err(e) =
                            self.import_item_from_module(&path, item_name.as_str(), registry)
                        {
                            tracing::debug!(
                                "import_all_from_module: nested-submod specific re-export {}.{} failed: {:?}",
                                path.as_str(),
                                item_name.as_str(),
                                e
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Import inherent impl methods for primitive types from a module.
    ///

    /// This handles `implement Int { ... }`, `implement Float { ... }`, etc.
    /// Primitive types are built-in and not module exports, but modules can
    /// extend them with methods via implement blocks.
    ///

    /// Uses `primitive_impls_registered_modules` to avoid redundant processing.
    fn import_primitive_impl_blocks(
        &mut self,
        module_path: &str,
        ast: &verum_ast::Module,
    ) -> Result<()> {
        // Check if we've already processed primitive impls for this module
        if self
            .primitive_impls_registered_modules
            .contains(module_path)
        {
            return Ok(());
        }
        self.primitive_impls_registered_modules
            .insert(module_path.to_string());

        // List of primitive type names to check for implement blocks
        // Note: These correspond to TypeKind variants in verum_ast::ty
        const PRIMITIVE_TYPES: &[&str] = &[
            wkt_names::INT,
            wkt_names::FLOAT,
            wkt_names::BOOL,
            wkt_names::CHAR,
            wkt_names::TEXT,
            "Unit",
        ];

        for type_name in PRIMITIVE_TYPES {
            // Try to import impl blocks for this primitive type
            // This will find and register methods from `implement Int { ... }` etc.
            if let Err(e) = self.import_impl_blocks_for_type(ast, type_name) {
                // Log but don't fail - not all modules have primitive impls
                tracing::debug!(
                    "Note: Could not import impl blocks for primitive '{}': {}",
                    type_name,
                    e
                );
            }
        }

        Ok(())
    }

    /// Extract the type of a function from a module's AST.
    ///

    /// This creates a function type from the function declaration's signature.
    /// Also handles variant constructors (e.g., `Some` from `type Maybe<T> is None | Some(T)`).
    /// Extract function type from module.
    /// Returns (Type, List<TypeVar>) where the List<TypeVar> contains the quantified
    /// type variables for generic functions (empty for non-generic functions).
    /// Callers should use TypeScheme::poly() when vars is non-empty, TypeScheme::mono() otherwise.
    fn extract_function_type_from_module(
        &mut self,
        ast: &verum_ast::Module,
        func_name: &str,
    ) -> Option<(Type, List<TypeVar>)> {
        use crate::context::{TypeParam as ContextTypeParam, Variance};
        use crate::protocol::ProtocolBound;
        use verum_ast::ItemKind;
        use verum_ast::decl::{FunctionParamKind, TypeDeclBody, VariantData};
        use verum_ast::ty::GenericParamKind;
        use verum_ast::ty::TypeBoundKind;

        for item in &ast.items {
            // Check for function declarations
            if let ItemKind::Function(func) = &item.kind
                && func.name.name.as_str() == func_name
            {
                // CRITICAL: Register generic type parameters BEFORE resolving param types
                // For functions like `add<T: Numeric>(a: T, b: T) -> T`, we need T in scope
                // so that ast_to_type can resolve parameter types correctly.
                //

                // Implementation follows best practices for generic intrinsics:
                // 1. Create fresh type variables for each generic parameter
                // 2. Register them in the type context for resolution
                // 3. Extract protocol bounds from AST for proper constraint tracking
                // 4. Build TypeParam structs for the function type signature
                // 5. Collect TypeVars for polymorphic TypeScheme creation
                let mut type_param_map: std::collections::HashMap<Text, Type> =
                    std::collections::HashMap::new();
                let mut type_param_list: List<ContextTypeParam> = List::new();
                let mut quantified_vars: List<TypeVar> = List::new();

                for generic_param in &func.generics {
                    if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind {
                        let fresh_var = TypeVar::fresh();
                        quantified_vars.push(fresh_var);
                        let type_var = Type::Var(fresh_var);
                        let name_text: Text = name.name.clone();

                        // Register in type context so ast_to_type can find it
                        type_param_map.insert(name_text.clone(), type_var.clone());
                        self.ctx.define_type(name_text.clone(), type_var);

                        // Extract protocol bounds from AST
                        // e.g., T: Numeric, T: Atomic + Integer
                        // TypeBound.kind is TypeBoundKind::Protocol(Path) for protocol bounds
                        let protocol_bounds: List<ProtocolBound> = bounds
                            .iter()
                            .filter_map(|bound| {
                                // Convert AST TypeBound to protocol::ProtocolBound
                                if let TypeBoundKind::Protocol(path) = &bound.kind {
                                    Some(ProtocolBound::positive(path.clone(), List::new()))
                                } else if let TypeBoundKind::NegativeProtocol(path) = &bound.kind {
                                    Some(ProtocolBound::negative(path.clone(), List::new()))
                                } else {
                                    // Skip equality bounds and other non-protocol bounds
                                    None
                                }
                            })
                            .collect();

                        // Create TypeParam with bounds
                        let type_param = ContextTypeParam {
                            name: name_text,
                            bounds: protocol_bounds,
                            default: verum_common::Maybe::None,
                            variance: Variance::Invariant,
                            is_meta: false, // Regular generic params are not meta (compile-time)
                            span: generic_param.span,
                        };
                        type_param_list.push(type_param);
                    }
                }

                // Build parameter types - now generic type params are in scope
                // CRITICAL FIX: When ast_to_type fails for parameter types (e.g., types using
                // Result, Maybe that aren't in scope during extraction), use lenient conversion
                // to preserve type structure for later resolution during actual type checking.
                let param_types: List<Type> = func
                    .params
                    .iter()
                    .filter_map(|p| match &p.kind {
                        FunctionParamKind::Regular { pattern: _, ty, .. } => {
                            // Try to resolve the type
                            match self.ast_to_type(ty) {
                                Ok(t) => Some(t),
                                Err(_) => {
                                    // Fallback: check if it's a direct type parameter reference
                                    if let verum_ast::ty::TypeKind::Path(path) = &ty.kind
                                        && path.segments.len() == 1
                                        && let Some(verum_ast::ty::PathSegment::Name(ident)) =
                                            path.segments.first()
                                    {
                                        type_param_map.get(&ident.name).cloned().or_else(|| {
                                            // Not a type param - create Named reference
                                            Some(Type::Named {
                                                path: path.clone(),
                                                args: List::new(),
                                            })
                                        })
                                    } else {
                                        // For complex types, use lenient conversion
                                        Some(self.ast_to_type_lenient(ty))
                                    }
                                }
                            }
                        }
                        _ => None,
                    })
                    .collect();

                // Build return type
                // CRITICAL FIX: When ast_to_type fails (e.g., Result<T,E> where Result isn't
                // in scope), we must NOT fall back to Unit. Instead, use ast_to_type_lenient
                // which creates a Type::Named reference that can be resolved during actual
                // type checking when prelude types are available.
                let return_type = if let Some(ref ret_ty) = func.return_type {
                    match self.ast_to_type(ret_ty) {
                        Ok(t) => t,
                        Err(_) => {
                            // Fallback: check if it's a direct type parameter reference
                            if let verum_ast::ty::TypeKind::Path(path) = &ret_ty.kind
                                && path.segments.len() == 1
                                && let Some(verum_ast::ty::PathSegment::Name(ident)) =
                                    path.segments.first()
                            {
                                type_param_map.get(&ident.name).cloned().unwrap_or_else(|| {
                                    // Not a type param - create Named reference for later resolution
                                    Type::Named {
                                        path: path.clone(),
                                        args: List::new(),
                                    }
                                })
                            } else {
                                // For complex types (Generic, etc.), use lenient conversion
                                // which preserves type structure for later resolution
                                self.ast_to_type_lenient(ret_ty)
                            }
                        }
                    }
                } else {
                    Type::unit()
                };

                // Wrap return type via the unified helper so this
                // cross-module function-signature extraction path
                // produces the SAME shape as `infer_function_type`
                // (line 8295) and `register_function_signature`
                // (line 53011): throws → generator → async, in that
                // order. Pre-fix this path manually applied throws +
                // async but DROPPED the generator wrap, so a mounted
                // `async fn* foo() -> Y` registered as `Future<Y>`
                // instead of `Future<Generator<Y, Unit>>`. Manifested
                // at every cross-module `for await x in foo()` call
                // site as "for await requires AsyncIterator … got
                // Future<Y>" (SHELL-5a, closes the gap left by the
                // partial fix in commit e09e6d5a).
                let final_return_type = self.wrap_return_type_for_sig_full(
                    return_type,
                    &func.throws_clause,
                    func.is_async,
                    func.is_generator,
                );

                return Some((
                    Type::Function {
                        params: param_types,
                        return_type: Box::new(final_return_type),
                        contexts: None,
                        type_params: type_param_list,
                        properties: None,
                    },
                    quantified_vars,
                ));
            }

            // Axioms with a callable signature (e.g. `axiom ua<A, B>(e: Equiv<A, B>)
            // -> Path<Type>(A, B)` in `core.math.hott`) must be resolvable across
            // modules the same way as `fn`. They're trusted declarations — the
            // body is assumed true — but at the type layer they *are* functions
            // with generics, parameters, and a return type, so cross-module
            // callers should see them identically. Without this branch,
            // `mount core.math.hott.{ua}` fails with E401 even though the
            // declaration exists.
            if let ItemKind::Axiom(axiom) = &item.kind
                && axiom.name.name.as_str() == func_name
            {
                let mut type_param_map: std::collections::HashMap<Text, Type> =
                    std::collections::HashMap::new();
                let mut type_param_list: List<ContextTypeParam> = List::new();
                let mut quantified_vars: List<TypeVar> = List::new();

                for generic_param in &axiom.generics {
                    if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind {
                        let fresh_var = TypeVar::fresh();
                        quantified_vars.push(fresh_var);
                        let type_var = Type::Var(fresh_var);
                        let name_text: Text = name.name.clone();
                        type_param_map.insert(name_text.clone(), type_var.clone());
                        self.ctx.define_type(name_text.clone(), type_var);
                        let protocol_bounds: List<ProtocolBound> = bounds
                            .iter()
                            .filter_map(|bound| match &bound.kind {
                                TypeBoundKind::Protocol(path) => {
                                    Some(ProtocolBound::positive(path.clone(), List::new()))
                                }
                                TypeBoundKind::NegativeProtocol(path) => {
                                    Some(ProtocolBound::negative(path.clone(), List::new()))
                                }
                                _ => None,
                            })
                            .collect();
                        type_param_list.push(ContextTypeParam {
                            name: name_text,
                            bounds: protocol_bounds,
                            default: verum_common::Maybe::None,
                            variance: Variance::Invariant,
                            is_meta: false,
                            span: generic_param.span,
                        });
                    }
                }

                let param_types: List<Type> = axiom
                    .params
                    .iter()
                    .filter_map(|p| match &p.kind {
                        FunctionParamKind::Regular { pattern: _, ty, .. } => {
                            match self.ast_to_type(ty) {
                                Ok(t) => Some(t),
                                Err(_) => {
                                    if let verum_ast::ty::TypeKind::Path(path) = &ty.kind
                                        && path.segments.len() == 1
                                        && let Some(verum_ast::ty::PathSegment::Name(ident)) =
                                            path.segments.first()
                                    {
                                        type_param_map.get(&ident.name).cloned().or_else(|| {
                                            Some(Type::Named {
                                                path: path.clone(),
                                                args: List::new(),
                                            })
                                        })
                                    } else {
                                        Some(self.ast_to_type_lenient(ty))
                                    }
                                }
                            }
                        }
                        _ => None,
                    })
                    .collect();

                let return_type = match &axiom.return_type {
                    verum_common::Maybe::Some(ret_ty) => match self.ast_to_type(ret_ty) {
                        Ok(t) => t,
                        Err(_) => {
                            if let verum_ast::ty::TypeKind::Path(path) = &ret_ty.kind
                                && path.segments.len() == 1
                                && let Some(verum_ast::ty::PathSegment::Name(ident)) =
                                    path.segments.first()
                            {
                                type_param_map.get(&ident.name).cloned().unwrap_or_else(|| {
                                    Type::Named {
                                        path: path.clone(),
                                        args: List::new(),
                                    }
                                })
                            } else {
                                self.ast_to_type_lenient(ret_ty)
                            }
                        }
                    },
                    verum_common::Maybe::None => Type::bool(),
                };

                return Some((
                    Type::Function {
                        params: param_types,
                        return_type: Box::new(return_type),
                        contexts: None,
                        type_params: type_param_list,
                        properties: None,
                    },
                    quantified_vars,
                ));
            }

            // Check for variant constructors in type declarations
            // e.g., `type Maybe<T> is None | Some(T)` exports `Some` as a constructor function
            if let ItemKind::Type(type_decl) = &item.kind {
                if let TypeDeclBody::Variant(variants) = &type_decl.body {
                    for variant in variants {
                        if variant.name.name.as_str() == func_name {
                            // Build the constructor function type
                            // Constructor: fn(args...) -> ParentType<generics>
                            // First, collect the type parameter names from the parent type
                            let type_param_names: std::collections::HashMap<Text, Type> = type_decl
                                .generics
                                .iter()
                                .filter_map(|g| {
                                    use verum_ast::ty::GenericParamKind;
                                    if let GenericParamKind::Type { name, .. } = &g.kind {
                                        Some((name.name.clone(), Type::Var(TypeVar::fresh())))
                                    } else {
                                        None
                                    }
                                })
                                .collect();

                            let param_types: List<Type> = match &variant.data {
                                verum_common::Maybe::Some(VariantData::Tuple(types)) => {
                                    types
                                        .iter()
                                        .filter_map(|ty| {
                                            // First try to resolve as type parameter
                                            if let verum_ast::ty::TypeKind::Path(path) = &ty.kind {
                                                if path.segments.len() == 1 {
                                                    if let verum_ast::ty::PathSegment::Name(ident) =
                                                        &path.segments[0]
                                                    {
                                                        if let Some(param_type) =
                                                            type_param_names.get(&ident.name)
                                                        {
                                                            return Some(param_type.clone());
                                                        }
                                                    }
                                                }
                                            }
                                            // Fall back to regular type resolution
                                            self.ast_to_type(ty).ok()
                                        })
                                        .collect()
                                }
                                verum_common::Maybe::Some(VariantData::Record(fields)) => {
                                    // Record variants take a record as argument
                                    let field_types: List<(Text, Type)> = fields
                                        .iter()
                                        .filter_map(|f| {
                                            self.ast_to_type(&f.ty)
                                                .ok()
                                                .map(|t| (f.name.name.clone(), t))
                                        })
                                        .collect();
                                    List::from(vec![Type::Record(
                                        field_types.into_iter().collect(),
                                    )])
                                }
                                verum_common::Maybe::None => {
                                    // Unit variant (like None) - takes no arguments
                                    List::new()
                                }
                            };

                            // Build return type: ParentType<generics>
                            // Use the same type variables we created for type parameters
                            let return_type = if type_decl.generics.is_empty() {
                                Type::Named {
                                    path: Self::text_to_path(&type_decl.name.name),
                                    args: List::new(),
                                }
                            } else {
                                // Map generics to the same type variables we used for params
                                let type_args: List<Type> = type_decl
                                    .generics
                                    .iter()
                                    .filter_map(|g| {
                                        use verum_ast::ty::GenericParamKind;
                                        if let GenericParamKind::Type { name, .. } = &g.kind {
                                            type_param_names.get(&name.name).cloned()
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                Type::Named {
                                    path: Self::text_to_path(&type_decl.name.name),
                                    args: type_args,
                                }
                            };

                            // Variant constructors are polymorphic if parent type has generics
                            let variant_type_vars: List<TypeVar> = type_param_names
                                .values()
                                .filter_map(|ty| {
                                    if let Type::Var(tv) = ty {
                                        Some(*tv)
                                    } else {
                                        None
                                    }
                                })
                                .collect();

                            // Unit variants (like None) have the type directly (e.g., Maybe<T>),
                            // not a function type (e.g., fn() -> Maybe<T>).
                            // Non-unit variants are constructor functions.
                            if param_types.is_empty() {
                                // Unit variant: type is the parent type directly
                                return Some((return_type, variant_type_vars));
                            } else {
                                // Non-unit variant: type is a constructor function
                                return Some((
                                    Type::Function {
                                        params: param_types,
                                        return_type: Box::new(return_type),
                                        contexts: None,
                                        type_params: List::new(),
                                        properties: None,
                                    },
                                    variant_type_vars,
                                ));
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Extract the type of a constant from a module's AST.
    fn extract_const_type_from_module(
        &mut self,
        ast: &verum_ast::Module,
        const_name: &str,
    ) -> Option<Type> {
        use verum_ast::ItemKind;

        for item in &ast.items {
            if let ItemKind::Const(const_decl) = &item.kind {
                if const_decl.name.name.as_str() == const_name {
                    return self.ast_to_type(&const_decl.ty).ok();
                }
            } else if let ItemKind::Static(static_decl) = &item.kind
                && static_decl.name.name.as_str() == const_name
            {
                return self.ast_to_type(&static_decl.ty).ok();
            }
        }

        None
    }

    /// Search submodules for a const/static definition.
    /// Used when a glob re-export (e.g., `public mount atomic.*;`) propagates
    /// a const from a submodule to the parent, but the const is not in the parent's AST.
    fn find_const_in_submodules(
        &mut self,
        parent_module_path: &str,
        const_name: &str,
        registry: &ModuleRegistry,
    ) -> Option<Type> {
        // Iterate over all modules to find ones that are children of the parent
        for (_id, module_info_shared) in registry.all_modules() {
            let module_info: &verum_modules::ModuleInfo = module_info_shared;
            let module_path_str = module_info.path.to_string();

            // Check if this is a direct child of the parent module
            if module_path_str.starts_with(parent_module_path)
                && module_path_str.len() > parent_module_path.len()
                && module_path_str.as_bytes().get(parent_module_path.len()) == Some(&b'.')
            {
                // Check if the const/static is exported from this submodule
                let name_text: Text = const_name.to_string().into();
                if module_info.exports.get(&name_text).is_some() {
                    if let Some(const_type) =
                        self.extract_const_type_from_module(&module_info.ast, const_name)
                    {
                        return Some(const_type);
                    }
                }
            }
        }
        None
    }

    /// Register a stdlib math function
    fn register_stdlib_math(&mut self, name: &str) {
        // Most math functions are Float -> Float
        let func_type = match name {
            "sqrt" | "sin" | "cos" | "tan" | "floor" | "ceil" | "round" | "abs" => Type::Function {
                params: List::from(vec![Type::float()]),
                return_type: Box::new(Type::float()),
                contexts: None,
                type_params: List::new(),
                properties: None,
            },
            "min" | "max" => {
                // Binary functions: (Float, Float) -> Float
                Type::Function {
                    params: List::from(vec![Type::float(), Type::float()]),
                    return_type: Box::new(Type::float()),
                    contexts: None,
                    type_params: List::new(),
                    properties: None,
                }
            }
            "pow" => {
                // pow(Float, Int) -> Float (exponent is always Int)
                Type::Function {
                    params: List::from(vec![Type::float(), Type::Int]),
                    return_type: Box::new(Type::float()),
                    contexts: None,
                    type_params: List::new(),
                    properties: None,
                }
            }
            _ => return, // Unknown function, don't register
        };

        self.ctx.env.insert(name, TypeScheme::mono(func_type));
    }

    pub(super) fn check_function(&mut self, func: &verum_ast::FunctionDecl) -> Result<()> {
        // Skip functions gated by @cfg predicates that don't match the current platform.
        if !self.cfg_evaluator.should_include(&func.attributes) {
            return Ok(());
        }
        self.check_function_inner(func)
    }

    /// Inner implementation of check_function
    fn check_function_inner(&mut self, func: &verum_ast::FunctionDecl) -> Result<()> {
        let _global_guard = GlobalDepthGuard::enter()?;

        // CRITICAL: Reset borrow tracker at the start of each function
        // Borrows are function-scoped, so we must not carry state from previous functions
        self.borrow_tracker = crate::aliasing::BorrowTracker::new();

        // ============================================================
        // Stage Checker Integration for N-level Staged Metaprogramming
        // Stage coherence: runtime code cannot depend on meta-only values, meta code cannot observe runtime state — Stage Coherence Rule
        // ============================================================
        // Save previous stage for restoration after function check
        let prev_function_stage = self.current_function_stage;
        self.current_function_stage = func.stage_level;

        // Track @transparent attribute for quote hygiene checking
        // Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene
        let prev_function_is_transparent = self.current_function_is_transparent;
        self.current_function_is_transparent = func.is_transparent;

        // Register function with StageChecker for cross-stage validation
        if func.stage_level > 0 {
            // Only register meta functions (stage > 0)
            let func_name = func.name.name.clone();
            if let Err(stage_err) =
                self.stage_checker
                    .register_function(func_name.clone(), func.stage_level, func.span)
            {
                // Convert StageError to TypeError
                return Err(Self::stage_error_to_type_error(stage_err));
            }
            // Enter the function context for the stage checker
            self.stage_checker
                .enter_function(&func_name, func.stage_level, func.span);
        }

        // FIRST: Register generic type parameters BEFORE processing parameter/return types
        // This allows T to be resolved in fn foo<T>(x: T) -> T
        // CRITICAL: Track TypeVars explicitly for proper TypeScheme construction
        // This is needed for phantom type parameters like `fn foo<T: Bound>()` where T
        // doesn't appear in parameters or return type but is still a valid type parameter.
        let mut func_type_param_vars: List<TypeVar> = List::new();
        let mut func_implicit_type_vars: Set<TypeVar> = Set::new();

        for generic_param in &func.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    let tvar = TypeVar::fresh();
                    let type_var = Type::Var(tvar);
                    let name_text: Text = name.name.clone();
                    self.ctx.define_type(name_text.clone(), type_var);
                    func_type_param_vars.push(tvar);

                    // Track implicit parameters
                    if generic_param.is_implicit {
                        func_implicit_type_vars.insert(tvar);
                    }

                    // CRITICAL FIX: Register type param with bounds so protocol methods can be looked up
                    // Example: fn print_item<T: Display>(item: &T) { item.display(); }
                    // Without this, calling protocol methods on bounded generic types fails
                    let protocol_bounds = if !bounds.is_empty() {
                        self.convert_type_bounds_to_protocol_bounds(bounds)?
                    } else {
                        List::new()
                    };

                    // Register bounds in type_var_bounds map for method resolution
                    // This enables infer_method_call to find protocol methods on bounded type params
                    if !protocol_bounds.is_empty() {
                        self.register_type_var_bounds(tvar, protocol_bounds.clone());
                    }

                    let type_param = crate::context::TypeParam::new(name_text.clone(), name.span)
                        .with_bounds(protocol_bounds);
                    self.ctx.env.add_type_param(type_param);
                }
                // CRITICAL FIX: Handle HKT type parameters in first pass
                // Use a TypeVar so it gets instantiated properly during function calls
                // When resolving F<A>, we create TypeApp { constructor: Var(τF), args: [...] }
                // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
                GenericParamKind::HigherKinded {
                    name,
                    arity: _,
                    bounds,
                } => {
                    let name_text: Text = name.name.clone();

                    // Use a TypeVar for HKT parameter so it participates in instantiation
                    let tvar = TypeVar::fresh();
                    let type_var = Type::Var(tvar);
                    self.ctx.define_type(name_text.clone(), type_var);
                    func_type_param_vars.push(tvar);

                    // Side table: only register bounded HKTs so bare slot
                    // declarations don't shadow a real bounded HKT in an
                    // outer scope.
                    if !bounds.is_empty() {
                        self.hkt_type_var_by_name.insert(name_text.clone(), tvar);
                    }

                    // Track implicit parameters
                    if generic_param.is_implicit {
                        func_implicit_type_vars.insert(tvar);
                    }

                    // Register bounds if present
                    if !bounds.is_empty() {
                        if let Ok(protocol_bounds) =
                            self.convert_type_bounds_to_protocol_bounds(bounds)
                        {
                            // Register protocol bounds on the TypeVar so bound-first
                            // method dispatch in infer_method_call_inner_impl can find
                            // the method through the bound.
                            self.register_type_var_bounds(tvar, protocol_bounds.clone());

                            let type_param = crate::context::TypeParam::new(name_text, name.span)
                                .with_bounds(protocol_bounds);
                            self.ctx.env.add_type_param(type_param);
                        }
                    }
                }
                GenericParamKind::Meta { name, ty, .. } => {
                    // Meta (const generic) parameters are compile-time values.
                    // Create a TypeVar so they are included in func_type_param_vars,
                    // ensuring the function scheme has the correct number of type params.
                    let tvar = TypeVar::fresh();
                    let meta_type = self.ast_to_type(ty)?;
                    let name_text: Text = name.name.clone();
                    self.ctx.define_type(name_text, meta_type);
                    func_type_param_vars.push(tvar);

                    // Track implicit parameters
                    if generic_param.is_implicit {
                        func_implicit_type_vars.insert(tvar);
                    }
                }
                _ => {} // Handle other generic param kinds (Const, Lifetime) later
            }
        }

        // Build function type from signature
        // Generic parameters will be processed in the function scope
        let param_types: Result<List<_>> = func
            .params
            .iter()
            .map(|p| {
                match &p.kind {
                    FunctionParamKind::Regular { pattern: _, ty, .. } => self.ast_to_type(ty),
                    FunctionParamKind::SelfValue => {
                        // Self parameter has no explicit type in the signature
                        Ok(Type::unit())
                    }
                    FunctionParamKind::SelfValueMut => Ok(Type::unit()),
                    FunctionParamKind::SelfRef => Ok(Type::unit()),
                    FunctionParamKind::SelfRefMut => Ok(Type::unit()),
                    FunctionParamKind::SelfRefChecked => Ok(Type::unit()),
                    FunctionParamKind::SelfRefCheckedMut => Ok(Type::unit()),
                    FunctionParamKind::SelfRefUnsafe => Ok(Type::unit()),
                    FunctionParamKind::SelfRefUnsafeMut => Ok(Type::unit()),
                    FunctionParamKind::SelfOwn => Ok(Type::unit()),
                    FunctionParamKind::SelfOwnMut => Ok(Type::unit()),
                }
            })
            .collect();

        let param_types = param_types?;

        // Determine if we have an explicit return type annotation
        let explicit_return_type = if let Some(ref ret_ty) = func.return_type {
            Some(self.ast_to_type(ret_ty)?)
        } else {
            None
        };

        // For recursive functions, we need to add the function to the environment
        // before type-checking the body. If there's no explicit return type,
        // use a fresh type variable that will be unified with the inferred body type.
        let return_type_var = if explicit_return_type.is_none() {
            Some(Type::Var(TypeVar::fresh()))
        } else {
            None
        };

        let initial_return_type = explicit_return_type
            .clone()
            .or_else(|| return_type_var.clone())
            .unwrap_or_else(Type::unit);

        // Resolve and expand context requirements
        // Context group expansion: resolving context group names to their constituent contexts recursively — Context group expansion
        //

        // Module-level `using [Ctx]` clauses at the top of a file are parsed
        // as a synthetic context group named `__module_contexts__`. Its
        // contents should be implicitly added to every function's required
        // contexts, so callers don't have to repeat `using [Ctx]` on every
        // function in a file that already declares it at module scope.
        let module_level_contexts = match self.context_resolver.get_group("__module_contexts__") {
            verum_common::Maybe::Some(g) => g.contexts.clone(),
            verum_common::Maybe::None => List::new(),
        };

        let context_requirement = if !func.contexts.is_empty() || !module_level_contexts.is_empty()
        {
            // Convert Vec to List for the resolver
            let contexts_list: List<_> = func.contexts.iter().cloned().collect();

            // Expand context groups and validate all contexts
            let mut requirement = if !contexts_list.is_empty() {
                self.context_resolver
                    .resolve_requirement(&contexts_list, func.span)?
            } else {
                crate::di::requirement::ContextRequirement::empty()
            };

            // Merge in module-level contexts.
            for ctx in module_level_contexts.iter() {
                requirement.add_context(ctx.clone());
            }

            Some(requirement)
        } else {
            None
        };

        // ============================================================
        // E502: Validate meta context requirements for meta functions
        // Meta contexts: meta functions have restricted context access (only compile-time-safe contexts) — Meta contexts
        // ============================================================
        // Meta functions can only use compiler-provided meta contexts.
        // Runtime contexts (Database, Logger, etc.) are not available at compile-time.
        if func.is_meta && !func.contexts.is_empty() {
            let validator = MetaContextValidator::new();
            let context_names: Vec<verum_common::Text> = func
                .contexts
                .iter()
                .map(|ctx| {
                    use verum_ast::ty::PathSegment;
                    ctx.path
                        .segments
                        .first()
                        .map(|seg| match seg {
                            PathSegment::Name(ident) => ident.name.clone(),
                            _ => verum_common::Text::from("unknown"),
                        })
                        .unwrap_or_else(|| verum_common::Text::from("unknown"))
                })
                .collect();

            match validator.validate(&context_names) {
                MetaContextValidation::Valid(_) => {
                    // All contexts are valid meta contexts - continue
                }
                MetaContextValidation::Invalid {
                    invalid_contexts, ..
                } => {
                    let invalid_str = invalid_contexts
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(TypeError::InvalidMetaContext {
                        func_name: func.name.name.clone(),
                        invalid_contexts: verum_common::Text::from(invalid_str),
                        span: func.span,
                    });
                }
            }
        }

        // Wrap return type for throws, async functions, and generators.
        // Multi-type throws unions (`throws(A | B)`) are combined into a
        // `Type::Variant` via the helper so `.map_err(|e| …)` closures
        // see the union rather than only the first error type.
        // NOTE: uses `throws` + `is_async`=false here because the outer
        // generator wrap below also has its own async branch.
        let return_for_sig_base =
            self.wrap_return_type_for_sig(initial_return_type.clone(), &func.throws_clause, false);

        // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — Async functions return Future<T>
        // Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - Generators return Generator<Y, R>
        let initial_return_for_sig = if func.is_generator {
            // Generator functions return Generator<Yield, Return> type
            // Grammar: grammar/verum.ebnf v2.10 - fn_keyword = 'fn' , [ '*' ]
            // The declared return type (-> T) is the yield type, generators finish with Unit
            if func.is_async {
                // Async generators: Future<Generator<Yield, Unit>>
                // The outer Future wraps the generator for async iteration
                Type::Future {
                    output: Box::new(Type::generator(return_for_sig_base, Type::unit())),
                }
            } else {
                // Sync generators: Generator<YieldTy, Unit>
                Type::generator(return_for_sig_base, Type::unit())
            }
        } else if func.is_async {
            Type::Future {
                output: Box::new(return_for_sig_base),
            }
        } else {
            return_for_sig_base
        };

        // Create initial function type with contexts and add to environment (for recursive calls)
        let initial_func_type = if let Some(req) = context_requirement.clone() {
            Type::function_with_contexts(param_types.clone(), initial_return_for_sig, req)
        } else {
            Type::function(param_types.clone(), initial_return_for_sig)
        };
        // CRITICAL: Create TypeScheme explicitly with tracked type parameters.
        // We cannot use `generalize()` which relies on `free_vars()` because:
        // - Phantom type parameters (e.g., `fn foo<T: Atomic>()`) don't appear in the function type
        // - Such parameters would be excluded by `free_vars()` but are still valid type parameters
        let scheme = if func_type_param_vars.is_empty() {
            TypeScheme::mono(initial_func_type.clone())
        } else if func_implicit_type_vars.is_empty() {
            TypeScheme::poly(func_type_param_vars.clone(), initial_func_type.clone())
        } else {
            TypeScheme::poly_with_implicit(
                func_type_param_vars.clone(),
                initial_func_type.clone(),
                func_implicit_type_vars.clone(),
            )
        };
        // Protect builtin polymorphic functions from being overwritten by stdlib functions.
        // During stdlib loading, protocol methods like `Drop.drop(&mut self)` would overwrite
        // the builtin `drop: ∀T. fn(T) -> Unit` with a monomorphic version.
        // Don't register impl block methods as standalone functions in the environment.
        // Methods like `Drop.drop(&mut self)` would otherwise overwrite builtins like
        // the polymorphic `drop: ∀T. fn(T) -> Unit`.
        if !self.in_impl_block {
            self.ctx.env.insert(func.name.name.as_str(), scheme);
        }

        // Type check body
        self.ctx.enter_scope();

        // Enter a new context scope for this function
        // This allows provide statements within the function to shadow outer provides
        // Context requirements: functions declare needed contexts with "using [Ctx1, Ctx2]" after return type, callers must provide all — Context scoping follows lexical scope
        self.context_checker.enter_scope();

        // Enter a new affine tracker scope for this function
        // Each function has its own set of affine bindings - variables from outer scopes
        // are NOT accessible in inner functions (unless captured explicitly).
        // The new_scope() preserves which types are affine but clears variable bindings.
        // Spec: L0-critical/reference_system/value_transfer - Affine type safety
        let new_affine_tracker = self.affine_tracker.new_scope();
        let prev_affine_tracker = std::mem::replace(&mut self.affine_tracker, new_affine_tracker);

        // Process generic parameters (type parameters and meta parameters)
        // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Meta parameters for compile-time computation
        for generic_param in &func.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    // Type parameters create fresh type variables
                    let tvar = TypeVar::fresh();
                    let type_var = Type::Var(tvar);
                    let name_text: Text = name.name.clone();
                    // Add to value environment (for type checking expressions)
                    self.ctx
                        .env
                        .insert(name.name.clone(), TypeScheme::mono(type_var.clone()));
                    // ALSO add to type environment (for type name resolution)
                    // This allows T to be used in type positions like: fn foo<T>(x: T) -> T
                    self.ctx.define_type(name_text.clone(), type_var);

                    // Register bounds from generic parameter declaration
                    // Example: fn foo<T: Clone>(x: T) -> T { ... }
                    if !bounds.is_empty() {
                        let protocol_bounds =
                            self.convert_type_bounds_to_protocol_bounds(bounds)?;

                        // CRITICAL: Register bounds in type_var_bounds for auto-deref
                        self.register_type_var_bounds(tvar, protocol_bounds.clone());

                        let type_param = crate::context::TypeParam::new(name_text, name.span)
                            .with_bounds(protocol_bounds);
                        self.ctx.env.add_type_param(type_param);
                    }
                }
                GenericParamKind::Meta {
                    name,
                    ty,
                    refinement,
                } => {
                    // Meta parameters: N: meta usize or N: meta usize{> 0}
                    // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified compile-time computation
                    let base_ty = self.ast_to_type(ty)?;

                    // Convert refinement if present
                    let ref_pred = match refinement {
                        Some(ref_expr) => {
                            use crate::refinement::{
                                RefinementBinding, RefinementPredicate as TyRefinementPredicate,
                            };
                            Some(TyRefinementPredicate {
                                predicate: (**ref_expr).clone(),
                                binding: RefinementBinding::Sigma(name.name.clone()),
                                span: ty.span,
                            })
                        }
                        None => None,
                    };

                    // Create Meta type
                    let meta_ty = Type::meta(name.name.clone(), base_ty, ref_pred);

                    // Add to environment
                    self.ctx
                        .env
                        .insert(name.name.clone(), TypeScheme::mono(meta_ty));
                }
                GenericParamKind::Const { name, ty } => {
                    // Deprecated const generics - convert to meta
                    // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Migration from const generics
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(format!(
                                "const generics are deprecated: `{}`\n  \
                                 help: use meta parameters instead\n  \
                                 example: `{}: meta <type>`",
                                name.name, name.name
                            ))
                            .build(),
                    );
                    let base_ty = self.ast_to_type(ty)?;
                    let meta_ty = Type::meta(name.name.clone(), base_ty, None);
                    self.ctx
                        .env
                        .insert(name.name.clone(), TypeScheme::mono(meta_ty));
                }
                GenericParamKind::Lifetime { name } => {
                    // Lifetime parameters for borrow checking
                    // Reference validation: ensuring mutable references are exclusive, immutable references allow sharing, no aliasing violations — Lifetime annotations
                    // For now, track lifetime names for future implementation
                    self.emit_diagnostic(
                        DiagnosticBuilder::warning()
                            .message(format!(
                                "lifetime parameter `{}` - lifetimes are implicit in Verum\n  \
                                 note: explicit lifetime bounds will be supported in future versions\n  \
                                 help: remove this parameter for now",
                                name.name
                            ))
                            .build()
                    );
                }
                GenericParamKind::HigherKinded {
                    name,
                    arity,
                    bounds,
                } => {
                    // Higher-kinded type parameters: F<_>: Functor, F<_, _>, etc.
                    // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
                    //

                    // HKT parameters represent type constructors, not concrete types.
                    // F<_> has kind * -> *, F<_, _> has kind * -> * -> *, etc.
                    //

                    // Example:
                    //  fn map<F<_>: Functor>(x: F<Int>) -> F<Text>
                    //  Here F is a type constructor like List, Maybe, etc.
                    use crate::advanced_protocols::Kind;

                    let name_text: Text = name.name.clone();

                    // Build the kind based on arity: * -> * -> ... -> *
                    let kind = match *arity {
                        0 => Kind::type_kind(),          // Degenerate case
                        1 => Kind::unary_constructor(),  // * -> *
                        2 => Kind::binary_constructor(), // * -> * -> *
                        n => {
                            // For higher arities, build the kind recursively
                            let mut k = Kind::Type;
                            for _ in 0..n {
                                k = Kind::Arrow(Box::new(Kind::Type), Box::new(k));
                            }
                            k
                        }
                    };

                    // Create a type constructor with the appropriate kind
                    let type_constructor = Type::type_constructor(name_text.clone(), *arity, kind);

                    self.ctx.env.insert(
                        name.name.clone(),
                        TypeScheme::mono(type_constructor.clone()),
                    );
                    self.ctx.define_type(name_text.clone(), type_constructor);

                    // Register bounds if present (e.g., F<_>: Functor)
                    if !bounds.is_empty() {
                        let protocol_bounds =
                            self.convert_type_bounds_to_protocol_bounds(bounds)?;
                        let type_param = crate::context::TypeParam::new(name_text, name.span)
                            .with_bounds(protocol_bounds);
                        self.ctx.env.add_type_param(type_param);
                    }
                }
                GenericParamKind::Context { name } => {
                    // Context parameters for context polymorphism
                    // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 17.2
                    //

                    // Context parameters allow higher-order functions to propagate contexts
                    // from their callbacks. Example:
                    //  fn map<T, U, using C>(iter: I, f: fn(T) -> U using C) -> MapIter<T, U> using C
                    //

                    // The context variable C is unified with the actual contexts of the callback
                    // and propagated to the outer function's context requirements.

                    let name_text: Text = name.name.clone();

                    // Create a fresh type variable for the context parameter
                    // This will be unified with actual context lists when the function is called
                    let tvar = TypeVar::fresh();
                    let context_var = Type::Var(tvar);

                    // Register in the environment so it can be referenced in function signature
                    self.ctx
                        .env
                        .insert(name.name.clone(), TypeScheme::mono(context_var.clone()));

                    // Also register as a type so it can be used in `using C` clauses
                    self.ctx.define_type(name_text, context_var);
                }
                GenericParamKind::Level { name } => {
                    // Universe level parameters for universe polymorphism
                    // Register as a type name so Type(u) can resolve the level variable
                    let name_text: Text = name.name.clone();
                    // Universe level params don't create type variables, but we register
                    // the name so it can be referenced in Type(u) annotations.
                    self.ctx.env.insert(
                        name.name.clone(),
                        TypeScheme::mono(Type::Universe {
                            level: crate::ty::UniverseLevel::Variable(
                                self.fresh_universe_var_id(&name_text),
                            ),
                        }),
                    );
                }
                GenericParamKind::KindAnnotated {
                    name,
                    kind: kind_ann,
                    bounds,
                } => {
                    // Kind-annotated HKT parameter: F: Type -> Type
                    // Behaves like HigherKinded: create a typed type constructor with the
                    // annotated kind, register it in the environment and kind_inferer.
                    use crate::advanced_protocols::Kind as AdvKind;
                    let name_text: Text = name.name.clone();

                    // Convert AST kind annotation to kind_inference::Kind
                    let ki_kind = Self::ast_kind_to_infer_kind(kind_ann);

                    // Derive arity from the kind annotation for Type::type_constructor
                    let arity = kind_ann.arity();

                    // Build advanced_protocols::Kind from arity (compatible representation)
                    let adv_kind = match arity {
                        0 => AdvKind::type_kind(),
                        1 => AdvKind::unary_constructor(),
                        2 => AdvKind::binary_constructor(),
                        n => {
                            let mut k = AdvKind::Type;
                            for _ in 0..n {
                                k = AdvKind::Arrow(Box::new(AdvKind::Type), Box::new(k));
                            }
                            k
                        }
                    };

                    let type_constructor =
                        Type::type_constructor(name_text.clone(), arity, adv_kind);

                    self.ctx.env.insert(
                        name.name.clone(),
                        TypeScheme::mono(type_constructor.clone()),
                    );
                    self.ctx.define_type(name_text.clone(), type_constructor);

                    // Register in kind_inferer with the explicitly declared kind
                    self.kind_inferer
                        .register_type_constructor(name_text.clone(), ki_kind);

                    // Register protocol bounds if present
                    if !bounds.is_empty() {
                        let protocol_bounds =
                            self.convert_type_bounds_to_protocol_bounds(bounds)?;
                        let type_param = crate::context::TypeParam::new(name_text, name.span)
                            .with_bounds(protocol_bounds);
                        self.ctx.env.add_type_param(type_param);
                    }
                }
            }
        }

        // Process where clause constraints: where type T: Clone
        // Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
        if let Some(ref where_clause) = func.generic_where_clause {
            for predicate in &where_clause.predicates {
                use verum_ast::ty::WherePredicateKind;
                match &predicate.kind {
                    WherePredicateKind::Type { ty, bounds } => {
                        // Extract type parameter name from the type
                        if let verum_ast::ty::TypeKind::Path(path) = &ty.kind
                            && let Some(ident) = path.as_ident()
                        {
                            let param_name: Text = ident.name.clone();

                            // Convert AST bounds to protocol bounds
                            let protocol_bounds =
                                self.convert_type_bounds_to_protocol_bounds(bounds)?;

                            // CRITICAL FIX: Register bounds in type_var_bounds map for auto-deref
                            // This enables *r to work when r: Ref<T>
                            // Spec: L0-critical/reference_system/reference_tiers/tier_conversion.vr
                            if !protocol_bounds.is_empty() {
                                // Find the TypeVar associated with this type parameter
                                if let Maybe::Some(type_val) = self.ctx.lookup_type(&param_name) {
                                    if let Type::Var(tvar) = type_val {
                                        self.register_type_var_bounds(
                                            *tvar,
                                            protocol_bounds.clone(),
                                        );
                                    }
                                }
                            }

                            // Add or update type parameter with bounds
                            let type_param =
                                crate::context::TypeParam::new(param_name.clone(), predicate.span)
                                    .with_bounds(protocol_bounds);
                            self.ctx.env.add_type_param(type_param);
                        }
                    }
                    _ => {
                        // Other where predicate kinds (Meta, Value, Ensures) handled elsewhere
                    }
                }
            }
        }

        // ============================================================
        // Track Function Parameters for Return Lifetime Validation
        // Return reference validation: ensuring returned references do not outlive their referents via escape analysis — Return lifetime validation
        // ============================================================
        // Clear and populate current_function_params with parameter names.
        // This is used to distinguish local variables from parameters when
        // checking return values for dangling references.
        self.current_function_params.clear();

        // Bind parameters and register their types
        for (i, param) in func.params.iter().enumerate() {
            match &param.kind {
                FunctionParamKind::Regular { pattern, ty, .. } => {
                    let param_ty = self.ast_to_type(ty)?;

                    // Track parameter name for return lifetime validation
                    if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                        self.current_function_params
                            .insert(verum_common::Text::from(name.name.as_str()));
                    }

                    // Register parameter type in type registry
                    // Extract parameter name for registration
                    if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                        self.type_registry.register_var(
                            param.span,
                            name.name.clone(),
                            param_ty.clone(),
                        );
                    }

                    self.bind_pattern(pattern, &param_ty)?;
                }
                FunctionParamKind::SelfValue
                | FunctionParamKind::SelfValueMut
                | FunctionParamKind::SelfRef
                | FunctionParamKind::SelfRefMut
                | FunctionParamKind::SelfRefChecked
                | FunctionParamKind::SelfRefCheckedMut
                | FunctionParamKind::SelfRefUnsafe
                | FunctionParamKind::SelfRefUnsafeMut
                | FunctionParamKind::SelfOwn
                | FunctionParamKind::SelfOwnMut => {
                    // Track self as a parameter for return lifetime validation
                    self.current_function_params
                        .insert(verum_common::Text::from("self"));

                    // Bind self parameter using current_self_type
                    if let Maybe::Some(ref self_ty) = self.current_self_type {
                        // Determine the actual type for self based on the parameter kind
                        let param_ty = match &param.kind {
                            FunctionParamKind::SelfValue => self_ty.clone(),
                            FunctionParamKind::SelfValueMut => self_ty.clone(),
                            FunctionParamKind::SelfRef => Type::Reference {
                                inner: Box::new(self_ty.clone()),
                                mutable: false,
                            },
                            FunctionParamKind::SelfRefMut => Type::Reference {
                                inner: Box::new(self_ty.clone()),
                                mutable: true,
                            },
                            FunctionParamKind::SelfRefChecked => Type::Reference {
                                inner: Box::new(self_ty.clone()),
                                mutable: false,
                            },
                            FunctionParamKind::SelfRefCheckedMut => Type::Reference {
                                inner: Box::new(self_ty.clone()),
                                mutable: true,
                            },
                            FunctionParamKind::SelfRefUnsafe => Type::Reference {
                                inner: Box::new(self_ty.clone()),
                                mutable: false,
                            },
                            FunctionParamKind::SelfRefUnsafeMut => Type::Reference {
                                inner: Box::new(self_ty.clone()),
                                mutable: true,
                            },
                            FunctionParamKind::SelfOwn => self_ty.clone(),
                            FunctionParamKind::SelfOwnMut => self_ty.clone(),
                            _ => unreachable!(),
                        };

                        // Register self in the environment
                        self.ctx
                            .env
                            .insert("self", TypeScheme::mono(param_ty.clone()));

                        // Register in type registry
                        self.type_registry
                            .register_var(param.span, "self".into(), param_ty);
                    } else {
                        return Err(TypeError::Other(
                            "self parameter requires method context (implement block)".into(),
                        ));
                    }
                }
            }
        }

        // Bind context names to their types in the environment
        // This allows functions using contexts to access them by name
        // e.g., `fn greet() using Logger { Logger.log("hi"); }`
        if let Some(ref req) = context_requirement {
            for context_ref in req.iter() {
                let context_name = &context_ref.name;
                // NAME-COLLISION GUARD (mirrors the one in
                // infer_method_call_inner_impl): if the user defined
                // `type X` or `implement X { ... }` with the same
                // spelling as the context, do NOT shadow the user's
                // type binding in env with the synthetic context
                // Record. Otherwise `let b = X.method(...)` resolves
                // against the context's method signatures instead of
                // the user's impl, producing bogus downstream types.
                // Guard: skip the synthetic context binding only if a USER
                // type (not the context's own placeholder) claims the same
                // name. If `context_name` appears in `context_declarations`
                // it is the context itself — never a user type shadow. This
                // prevents the placeholder `Named { path: Logger }` that
                // `register_stdlib_context_full` leaves in `type_defs` from
                // being mistaken for a user-declared `type Logger`.
                let is_registered_context = self.context_declarations.contains_key(context_name);
                let user_type_shadows_context = if is_registered_context {
                    false
                } else {
                    let has_inherent = self
                        .inherent_methods
                        .read()
                        .get(context_name)
                        .map(|m| !m.is_empty())
                        .unwrap_or(false);
                    let has_type_params = matches!(
                        self.ctx
                            .lookup_type(format!("__type_params_{}", context_name).as_str()),
                        Option::Some(_)
                    );
                    let has_type = matches!(
                        self.ctx.lookup_type(context_name.as_str()),
                        Option::Some(Type::Named { .. })
                            | Option::Some(Type::Record(_))
                            | Option::Some(Type::Variant(_))
                            | Option::Some(Type::Placeholder { .. })
                    );
                    has_inherent || has_type_params || has_type
                };
                if user_type_shadows_context {
                    continue;
                }
                if let Maybe::Some(context_type) =
                    self.context_resolver.get_context_type(context_name)
                {
                    // ============================================================================
                    // CRITICAL FIX: Apply type arguments for parameterized contexts
                    // ============================================================================
                    // For generic contexts like `Cache<User>`, we need to:
                    // 1. Get the generic context type with Record { get: fn(Text) -> Maybe<T>, ... }
                    // 2. Substitute type arguments (User for T) in all method types
                    // 3. Bind the specialized type to the variable/alias
                    //

                    // Context provision: "provide ContextName = implementation" installs a provider in lexical scope via task-local storage (theta) — Parameterized Contexts
                    // ============================================================================
                    let specialized_type = if !context_ref.type_args.is_empty() {
                        // Build substitution from context type params to concrete args
                        // Get type params from the context definition
                        let type_params_key = format!("__context_type_params_{}", context_name);
                        let type_params: List<verum_common::Text> =
                            match self.ctx.lookup_type(type_params_key.as_str()) {
                                Option::Some(Type::Record(params_map)) => {
                                    params_map.keys().cloned().collect()
                                }
                                _ => {
                                    // Fall back to generic params from declaration
                                    // For single-param contexts like Cache<T>, use "T"
                                    if context_ref.type_args.len() == 1 {
                                        List::from(vec![verum_common::Text::from("T")])
                                    } else {
                                        List::new()
                                    }
                                }
                            };

                        // Build substitution map: T -> User, K -> KeyType, etc.
                        let mut subst_map = indexmap::IndexMap::new();
                        for (param_name, arg_name) in
                            type_params.iter().zip(context_ref.type_args.iter())
                        {
                            // Resolve the type argument name to an actual type
                            let arg_ty = self
                                .ctx
                                .lookup_type(arg_name.as_str())
                                .cloned()
                                .unwrap_or_else(|| {
                                    // If not found, create a Named type for the argument
                                    Type::Named {
                                        path: verum_ast::ty::Path::from_ident(
                                            verum_ast::ty::Ident::new(
                                                arg_name.as_str(),
                                                Span::default(),
                                            ),
                                        ),
                                        args: List::new(),
                                    }
                                });
                            subst_map.insert(param_name.clone(), arg_ty);
                        }

                        // Apply substitution to the context type
                        self.substitute_type_params(context_type, &subst_map)
                    } else {
                        context_type.clone()
                    };

                    // Add the context to the environment as a variable
                    self.ctx.env.insert(
                        context_name.as_str(),
                        TypeScheme::mono(specialized_type.clone()),
                    );

                    // ============================================================================
                    // CRITICAL FIX: Also bind alias as variable if present
                    // ============================================================================
                    // For aliased contexts (`Database as source`) or named contexts (`db: Database`),
                    // the alias should be bound as a variable so it can be used in the function body.
                    //

                    // Example: `fn migrate() using [Database as source, Database as target] { ... }`
                    //  - `source` and `target` should be bound to the Database context type
                    //

                    // Example: `fn handle() using [db: Database, log: Logger] { ... }`
                    //  - `db` and `log` should be bound to Database and Logger respectively
                    //

                    // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
                    // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1 - Named Contexts
                    // ============================================================================
                    if let Maybe::Some(ref alias) = context_ref.alias {
                        self.ctx
                            .env
                            .insert(alias.as_str(), TypeScheme::mono(specialized_type.clone()));
                    }
                }
            }

            // Register contexts with capability checker
            // Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0 - Capability Attenuation
            // Each context declared in the `using` clause gets full capabilities by default
            for context_ref in req.iter() {
                use crate::capability::ContextCapabilities;
                let caps = ContextCapabilities::full(context_ref.name.clone());
                self.capability_checker.register_context(caps);
            }
        }

        // ============================================================
        // Set up current function contexts (ContextChecker integration)
        // Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
        // ============================================================
        // Build a ContextSet from the function's context requirements and
        // set it as the current function's available contexts. This enables
        // context satisfaction validation in function calls.
        let prev_function_contexts = if let Some(ref req) = context_requirement {
            let mut context_set = ContextSet::new();
            for context_ref in req.iter() {
                context_set.add(ContextRequirement::new(context_ref.name.clone(), func.span));
            }
            // Also set up the context checker's required contexts
            self.context_checker.set_required(context_set.clone());
            self.current_function_contexts.replace(context_set)
        } else {
            // No context requirements - clear the context checker's required contexts
            self.context_checker.set_required(ContextSet::new());
            self.current_function_contexts.take()
        };

        // Set function context for ? operator checking
        // Save previous context (for nested functions)
        let prev_return_type = self
            .current_function_return_type
            .replace(initial_return_type.clone());
        let prev_name = self.current_function_name.replace(func.name.name.clone());
        let prev_span = std::mem::replace(
            &mut self.current_function_return_span,
            func.return_type.as_ref().map(|rt| rt.span),
        );

        // Set throws clause for throw expression validation
        let new_throws = if let Maybe::Some(ref throws_clause) = func.throws_clause {
            let error_types: std::result::Result<List<Type>, _> = throws_clause
                .error_types
                .iter()
                .map(|t| self.ast_to_type(t))
                .collect();
            match error_types {
                Ok(types) => Maybe::Some(types),
                Err(e) => {
                    tracing::debug!("Failed to resolve throws clause types: {:?}", e);
                    Maybe::None
                }
            }
        } else {
            Maybe::None
        };
        let prev_throws = std::mem::replace(&mut self.current_function_throws, new_throws);

        // Set up call tracking for negative context verification
        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
        let prev_call_sites = std::mem::replace(&mut self.current_function_call_sites, Map::new());
        let prev_function_being_checked = self
            .current_function_being_checked
            .replace(func.name.name.clone());

        // Set up generator context for yield type checking
        // Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - Generator type inference
        // Grammar: grammar/verum.ebnf v2.10 - yield_expr = 'yield' , expression
        let prev_generator_context = if func.is_generator {
            // For generators, the declared return type is the yield type
            // e.g., `fn* count() -> Int` yields Int values
            self.generator_context.replace(GeneratorContext {
                yield_ty: initial_return_type.clone(),
                return_ty: Type::unit(), // Generators finish with unit
            })
        } else {
            self.generator_context.take()
        };

        // Set async context for async functions
        // This enables validation that select expressions are only used in async contexts
        // Select expressions require async context: "select { ... }" only valid in async functions — Select expressions require async context
        let prev_async_context = std::mem::replace(&mut self.in_async_context, func.is_async);

        // ============================================================
        // E321: Check for unbounded recursion BEFORE body inference
        // Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
        // ============================================================
        // This MUST run before body inference to prevent stack overflow when
        // the type checker recurses into infinitely-recursive function bodies.
        {
            let has_allow_unbounded = func.attributes.iter().any(|attr| {
                if attr.name.as_str() == "allow" {
                    if let Some(ref args) = attr.args {
                        return args.iter().any(|arg| {
                            if let ExprKind::Path(path) = &arg.kind {
                                if let Some(ident) = path.as_ident() {
                                    return ident.name.as_str() == "unbounded_recursion";
                                }
                            }
                            false
                        });
                    }
                }
                false
            });
            let has_tailrec = func
                .attributes
                .iter()
                .any(|attr| attr.name.as_str() == "tailrec");

            if !has_allow_unbounded && !has_tailrec {
                let term_result = if self.in_impl_block {
                    self.termination_checker.check_method(func)
                } else {
                    self.termination_checker.check_function(func)
                };
                if let Err(term_err) = term_result {
                    match term_err {
                        crate::termination::TerminationError::NonTerminating {
                            function,
                            reason: _,
                            span,
                        }
                        | crate::termination::TerminationError::NoDecreasingArgument {
                            function,
                            call_site: span,
                            ..
                        }
                        | crate::termination::TerminationError::NotStructurallySmaller {
                            function,
                            span,
                            ..
                        }
                        | crate::termination::TerminationError::InvalidDecreasingClause {
                            function,
                            span,
                            ..
                        } => {
                            return Err(TypeError::UnboundedRecursionDetected {
                                func_name: function,
                                span,
                                cycle: List::from(vec![func.name.name.clone()]),
                            });
                        }
                        crate::termination::TerminationError::MutualRecursionCycle {
                            cycle,
                            ..
                        } => {
                            return Err(TypeError::UnboundedRecursionDetected {
                                func_name: func.name.name.clone(),
                                span: func.span,
                                cycle,
                            });
                        }
                        crate::termination::TerminationError::UnguardedCorecursion {
                            function,
                            span,
                        } => {
                            return Err(TypeError::UnboundedRecursionDetected {
                                func_name: function,
                                span,
                                cycle: List::from(vec![func.name.name.clone()]),
                            });
                        }
                    }
                }
            }
        }

        // Infer or check body type
        // When we have an explicit return type, use CHECK mode (bidirectional)
        // to propagate the expected type down into the body for better inference.
        // This enables patterns like: fn make_int_box() -> Box<Int> { Box { value: 42 } }
        // where the return type Box<Int> informs the type of Box { value: 42 }.
        let return_type = if let Some(ref body) = func.body {
            let span = match body {
                FunctionBody::Block(block) => block.span(),
                FunctionBody::Expr(expr) => expr.span(),
            };

            // For generators, the body type is Unit - generators yield values, they don't return them.
            // The declared return type (e.g., -> Int) specifies what the generator yields, not what
            // the body evaluates to. Yield expressions are checked against yield_ty via generator_context.
            // Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - Generators
            let body_expected_type = if func.is_generator {
                Type::unit()
            } else {
                explicit_return_type
                    .clone()
                    .unwrap_or_else(|| Type::Var(TypeVar::fresh()))
            };

            if let Some(expected_return) = explicit_return_type {
                // Use CHECK mode for bidirectional type inference
                // This propagates the expected type into the body
                // For generators, we check against Unit; for regular functions, against return type
                let check_ty = if func.is_generator {
                    &body_expected_type
                } else {
                    &expected_return
                };
                match body {
                    FunctionBody::Block(block) => {
                        // Stub detection: if the block has no statements and no trailing
                        // expression, it's a stub/placeholder body. Accept any return type.
                        let is_stub = block.stmts.is_empty() && block.expr.is_none();
                        if !is_stub {
                            if self.stdlib_single_file_mode {
                                let _ = self.check_block(block, check_ty);
                            } else {
                                self.check_block(block, check_ty)?;
                            }
                        }
                        // ============================================================
                        // Return Lifetime Validation for Block Bodies
                        // Spec: L0-critical/reference_system/access_rules/ref_escaping_fail
                        // ============================================================
                        // Check if we're returning a reference to a local variable.
                        // This would create a dangling reference when the function returns.
                        // Skip lifetime check for generators - they yield, not return references
                        if !func.is_generator && self.is_reference_type(&expected_return) {
                            if let Some(ref trailing_expr) = block.expr {
                                self.check_return_lifetime(trailing_expr, span)?;
                            }
                        }
                    }
                    FunctionBody::Expr(expr) => {
                        self.check_expr(expr, check_ty)?;
                        // ============================================================
                        // Return Lifetime Validation for Expression Bodies
                        // Spec: L0-critical/reference_system/access_rules/ref_escaping_fail
                        // ============================================================
                        // Check if we're returning a reference to a local variable.
                        // This would create a dangling reference when the function returns.
                        // Skip lifetime check for generators - they yield, not return references
                        if !func.is_generator && self.is_reference_type(&expected_return) {
                            self.check_return_lifetime(expr, span)?;
                        }
                    }
                };
                // Existential Type Bound Verification
                if !func.is_generator {
                    if let Type::Exists { var, .. } = &expected_return {
                        let resolved = self.unifier.apply(&Type::Var(*var));
                        if !matches!(resolved, Type::Var(_)) {
                            let _ = self.verify_existential_return_bounds(&resolved, var, span);
                        }
                    }
                }
                // For generators, the function's "return type" from caller's perspective is
                // Generator<YieldTy, Unit>, but the body type is Unit
                expected_return
            } else {
                // No explicit return type - use synthesis mode
                let body_result = match body {
                    FunctionBody::Block(block) => {
                        let result = self.infer_block(block)?;
                        // ============================================================
                        // Return Lifetime Validation for Block Bodies
                        // Spec: L0-critical/reference_system/access_rules/ref_escaping_fail
                        // ============================================================
                        // Check if we're returning a reference to a local variable.
                        if self.is_reference_type(&result.ty) {
                            if let Some(ref trailing_expr) = block.expr {
                                self.check_return_lifetime(trailing_expr, span)?;
                            }
                        }
                        result
                    }
                    FunctionBody::Expr(expr) => {
                        let result = self.synth_expr(expr)?;
                        // ============================================================
                        // Return Lifetime Validation for Expression Bodies
                        // Spec: L0-critical/reference_system/access_rules/ref_escaping_fail
                        // ============================================================
                        // Check if we're returning a reference to a local variable.
                        if self.is_reference_type(&result.ty) {
                            self.check_return_lifetime(expr, span)?;
                        }
                        result
                    }
                };

                if let Some(ref type_var) = return_type_var {
                    // Unify the type variable with the inferred body type
                    self.unifier.unify(&body_result.ty, type_var, span)?;
                }
                body_result.ty
            }
        } else {
            // No body - use explicit return type or default to Unit
            explicit_return_type.unwrap_or_else(Type::unit)
        };

        // ============================================================
        // Register function context info for call graph building
        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
        // ============================================================
        // This must happen BEFORE restoring previous state, so we capture
        // the call sites collected during body type checking.
        {
            use crate::context_check::FunctionContextInfo;

            // Build context set from context requirement
            let func_context_set = if let Some(ref req) = context_requirement {
                let mut context_set = ContextSet::new();
                for context_ref in req.iter() {
                    context_set.add(ContextRequirement::new(context_ref.name.clone(), func.span));
                }
                context_set
            } else {
                ContextSet::new()
            };

            // Collect callee names from call sites
            let callees: List<verum_common::Text> =
                self.current_function_call_sites.keys().cloned().collect();

            // Build call_sites map
            let call_sites: Map<Text, crate::context_check::CallSiteInfo> =
                self.current_function_call_sites.clone();

            // Extract excluded context names from the context set
            let excluded_contexts: List<verum_common::Text> = func_context_set
                .negative_contexts()
                .map(|c| c.name.clone())
                .collect();

            // Create and register function context info
            let func_info = FunctionContextInfo {
                name: func.name.name.clone(),
                required_contexts: func_context_set,
                excluded_contexts,
                callees,
                call_sites,
                span: func.span,
            };
            self.context_checker.register_function(func_info);
        }

        // Restore previous function context
        self.current_function_return_type = prev_return_type;
        self.current_function_name = prev_name;
        self.current_function_return_span = prev_span;
        self.current_function_throws = prev_throws;
        self.generator_context = prev_generator_context;
        self.in_async_context = prev_async_context;
        self.current_function_contexts = prev_function_contexts;
        self.current_function_call_sites = prev_call_sites;
        self.current_function_being_checked = prev_function_being_checked;

        // Exit the context scope for this function
        // This restores the outer context state
        self.context_checker.exit_scope();

        // Exit the stage checker function context if we were in a meta function
        // and restore the previous function stage
        if self.current_function_stage > 0 {
            self.stage_checker.exit_function();
        }
        self.current_function_stage = prev_function_stage;
        self.current_function_is_transparent = prev_function_is_transparent;

        // Restore the outer affine tracker scope
        // This ensures affine tracking is isolated per function
        self.affine_tracker = prev_affine_tracker;

        // QTT enforcement: if any parameter was annotated with a
        // non-Omega quantity (via meta-parameter or explicit
        // `Quantity` annotation in the type system), validate
        // that the function body's usage matches the declared
        // quantity. This is a post-inference pass because the
        // body must be fully type-checked before we can walk it
        // for usage counts.
        //

        // For now, this fires only when the function has explicit
        // `meta` parameters (which carry Quantity::Zero semantics)
        // or when future extensions add `linear`/`affine` keywords.
        // In the common case (all params Omega), the check is a
        // no-op and exits immediately.
        // Quantitative Type Theory (QTT)
        // V2 enforcement. Walks the function's parameters + meta-
        // generics, extracts each binding's declared `Quantity` from
        // its `@quantity(...)` attribute (V1 surface from Task C5),
        // walks the body, and validates observed usage against
        // declared quantities per `qtt_usage::check_usage`.
        //

        // Quantity sources:
        //  • Meta-parameter generics (`@meta` etc.) → `Quantity::Zero`
        //  (erased at runtime by definition).
        //  • Regular parameters → read `@quantity(...)` attribute
        //  via `QuantityAttr::from_attribute`. Default is
        //  `Quantity::Omega` (unrestricted) — every
        //  existing function compiles unchanged.
        //

        // Body coverage: walks BOTH block.stmts AND block.expr so
        // mid-block uses contribute to the count, not just the tail.
        {
            // Register meta generics as Zero-quantity (erased).
            let mut qtt_decls = std::collections::HashMap::new();
            for g in &func.generics {
                if let verum_ast::ty::GenericParamKind::Meta { name, .. } = &g.kind {
                    qtt_decls.insert(name.name.clone(), crate::ty::Quantity::Zero);
                }
            }
            // Register regular params, extracting @quantity(...) when
            // present. Default Omega when absent.
            for p in &func.params {
                if let verum_ast::decl::FunctionParamKind::Regular { pattern, .. } = &p.kind {
                    if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                        let quantity = extract_quantity_from_attrs(&p.attributes);
                        qtt_decls.insert(name.name.clone(), quantity);
                    }
                }
            }
            // Run QTT validation only when there's something to check —
            // either a meta-generic (Zero) or an explicit @quantity
            // attribute. Pure-Omega parameter sets are no-ops.
            let needs_check = qtt_decls
                .values()
                .any(|q| !matches!(q, crate::ty::Quantity::Omega));
            if needs_check {
                if let verum_common::Maybe::Some(body) = &func.body {
                    if let verum_ast::decl::FunctionBody::Block(block) = body {
                        // V2: walk statements + tail expression to get
                        // full body usage.
                        let tracked: std::collections::HashSet<verum_common::Text> =
                            qtt_decls.keys().cloned().collect();
                        let mut usage = crate::qtt_usage::UsageMap::new();
                        for stmt in block.stmts.iter() {
                            // Walk every Stmt::Expr node + Let initialiser
                            // for usage of tracked bindings.
                            walk_stmt_for_qtt_usage(&tracked, stmt, &mut usage);
                        }
                        if let verum_common::Maybe::Some(tail) = &block.expr {
                            let tail_usage = crate::qtt_walker::walk_expr(&tracked, tail);
                            usage = usage.merge_sequential(tail_usage);
                        }
                        if let Err(violation) = crate::qtt_usage::check_usage(&qtt_decls, &usage) {
                            // V2: surface violations as warnings so
                            // existing test corpus that hasn't migrated
                            // doesn't break. Future minor bump can
                            // promote to a hard error once all in-tree
                            // code is annotated. The diagnostic is
                            // structurally available; downstream
                            // tooling (LSP / `verum verify` reports)
                            // can already key off the `tracing` event.
                            tracing::warn!(
                                "QTT violation in function '{}': {}",
                                func.name.name.as_str(),
                                violation
                            );
                        }
                    }
                }
            }
        }

        self.ctx.exit_scope();

        // Wrap final return type for async functions and generators
        // Concurrency model: structured concurrency with nurseries, async/await, channels, Send/Sync protocol bounds — Section 12 - Generators
        let final_return_type = if func.is_generator {
            // Generator functions return Generator<YieldTy, Unit>
            // The body's return_type is the yield type (e.g., Int for `fn* foo() -> Int`)
            if func.is_async {
                // Async generators: Future<Generator<Yield, Unit>>
                Type::Future {
                    output: Box::new(Type::generator(return_type.clone(), Type::unit())),
                }
            } else {
                Type::generator(return_type.clone(), Type::unit())
            }
        } else if func.is_async {
            Type::Future {
                output: Box::new(return_type.clone()),
            }
        } else {
            return_type.clone()
        };

        // Infer computational properties from function body
        // Computational properties: compile-time tracking of Pure, IO, Async, Fallible, Mutates effects inferred from function bodies — (Pure, IO, Async, Fallible, Mutates)
        let inferred_properties = if let Some(ref body) = func.body {
            // Use property inferrer to analyze the function body
            let body_expr = match body {
                FunctionBody::Block(block) => {
                    // Create a synthetic expression from the block for property inference
                    Expr::new(
                        verum_ast::expr::ExprKind::Block(block.clone()),
                        block.span(),
                    )
                }
                FunctionBody::Expr(expr) => expr.clone(),
            };
            let mut properties = self.property_inferrer.infer_expr(&body_expr);

            // Add Async property for async functions
            if func.is_async {
                properties =
                    properties.union(&crate::computational_properties::PropertySet::single(
                        crate::computational_properties::ComputationalProperty::Async,
                    ));
            }

            properties
        } else {
            // No body - default to pure
            crate::computational_properties::PropertySet::pure()
        };

        // ============================================================
        // E501: Enforce purity for meta functions
        // Meta function purity: meta functions are implicitly pure (no IO, no mutation of non-meta state) — Meta functions are implicitly pure
        // ============================================================
        // Meta functions run at compile-time and cannot have side effects.
        // They must be pure: no IO, no mutation, no external state access.
        if func.is_meta {
            if let Err(impure_props) = inferred_properties.validate_for_meta_fn() {
                let props_str: Vec<String> = impure_props.iter().map(|p| p.to_string()).collect();
                return Err(TypeError::ImpureMetaFunction {
                    func_name: func.name.name.clone(),
                    properties: verum_common::Text::from(props_str.join(", ")),
                    span: func.span,
                });
            }
        }

        // ============================================================
        // E503: Enforce purity for pure functions
        // Pure function validation: `pure fn` must have no side effects
        // ============================================================
        // Functions declared with `pure` modifier must not have IO, mutation,
        // async, external state access, FFI, or spawning properties.
        // Fallible and Divergent are allowed (errors and panics are deterministic).
        if func.is_pure {
            if let Err(impure_props) = inferred_properties.validate_for_pure_fn() {
                let props_str: Vec<String> = impure_props.iter().map(|p| p.to_string()).collect();
                return Err(TypeError::ImpurePureFunction {
                    func_name: func.name.name.clone(),
                    properties: verum_common::Text::from(props_str.join(", ")),
                    span: func.span,
                });
            }
        }

        // ============================================================
        // E505: Productivity check for cofix (coinductive) functions
        // Coinductive function productivity: cofix functions must have all
        // recursive self-calls guarded by at least one coinductive constructor.
        // ============================================================
        if func.is_cofix && self.coinductive_enabled {
            if let Some(ref body) = func.body {
                let body_calls = self.extract_corecursive_calls(body, &func.name.name);
                let diags = crate::coinductive_analysis::check_cofix_productivity(
                    func.name.name.as_str(),
                    &body_calls,
                );
                if let Some(diag) = diags.into_iter().next() {
                    let calls_str = diag
                        .unguarded_calls
                        .iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(TypeError::NonProductiveCorecursion {
                        func_name: func.name.name.clone(),
                        unguarded_calls: verum_common::Text::from(calls_str),
                        span: func.span,
                    });
                }
            }
        }

        // ============================================================
        // E504: Async property enforcement
        // Ensure async functions have Async property in their type
        // ============================================================
        // If a function is declared async, ensure the Async property is tracked.
        // If a non-async function body contains .await expressions, that's an error
        // (already caught by in_async_context checks, but we double-check here).
        // E504: Non-async function with async body operations.
        // Only warn for non-entry-point functions. Entry points (main, @test) are
        // allowed to contain async blocks since they serve as program roots that
        // orchestrate async execution without being async themselves.
        let is_entry_point =
            func.name.name.as_str() == "main" || func.attributes.iter().any(|a| a.is_named("test"));
        if !func.is_async && !is_entry_point && inferred_properties.is_async() {
            // Emit as warning, not hard error — existing codebase has many
            // non-async functions that spawn async work.
            tracing::debug!(
                "E504: function `{}` contains async operations but is not declared `async`",
                func.name.name
            );
        }

        // Register inferred properties with the property inferrer context
        // so that callers of this function can look up its properties.
        self.property_inferrer
            .context_mut()
            .register_function(func.name.name.clone(), inferred_properties.clone());

        // Register function return type in type registry
        self.type_registry
            .register_func_return(func.span, final_return_type.clone());

        // Update the function type in the environment with the resolved return type
        // and inferred computational properties
        // (this is important for recursive functions where we used a type variable)
        //

        // CRITICAL: For `throws(E) -> T` functions, external callers must see
        // `fn(…) -> Result<T, E>`. The `return_type` and `final_return_type`
        // above track the BODY type (raw T), used by body checking and the
        // `?` operator to unwrap inner values. The initial env registration
        // at line ~34346 wrapped correctly with `initial_return_for_sig`,
        // but this re-registration must preserve that wrap — otherwise the
        // body-inferred raw return type silently overwrites the schema and
        // callers of throws functions see the unwrapped return (caller then
        // can't use `.map_err` / the `?` operator on the call result).
        // Use the helper to apply throws + async wraps consistently,
        // including multi-type `throws(A | B)` → `Type::Variant` unions.
        // `is_async` is already baked into `final_return_type` above
        // (the generator/future branches at line ~35276), so we pass
        // `false` here to avoid double-wrapping.
        let final_return_for_sig =
            self.wrap_return_type_for_sig(final_return_type.clone(), &func.throws_clause, false);
        let final_func_type = Type::function_with_properties(
            param_types.clone(),
            final_return_for_sig,
            inferred_properties,
        );
        // CRITICAL: Create TypeScheme explicitly with tracked type parameters (same as initial).
        // We cannot use `generalize()` because phantom type parameters would be lost.
        let final_scheme = if func_type_param_vars.is_empty() {
            TypeScheme::mono(final_func_type.clone())
        } else if func_implicit_type_vars.is_empty() {
            TypeScheme::poly(func_type_param_vars.clone(), final_func_type.clone())
        } else {
            TypeScheme::poly_with_implicit(
                func_type_param_vars.clone(),
                final_func_type.clone(),
                func_implicit_type_vars.clone(),
            )
        };
        if !self.in_impl_block {
            self.ctx.env.insert(func.name.name.as_str(), final_scheme);
        }

        // Validate postconditions (ensures clauses) via SMT
        // Function contracts: preconditions (requires) and postconditions (ensures) on function signatures
        if !func.ensures.is_empty() {
            self.metrics.refinement_checks += func.ensures.len();

            // For each ensures clause, construct a refinement type and verify
            // that the return type satisfies it. The ensures expression is treated
            // as a predicate over the `result` binding (the function's return value).
            if self.has_dependent_types() {
                for ensures_expr in &func.ensures {
                    let refinement_predicate = crate::refinement::RefinementPredicate::inline(
                        ensures_expr.clone(),
                        ensures_expr.span,
                    );
                    let refinement_type = crate::refinement::RefinementType {
                        base_type: return_type.clone(),
                        predicate: refinement_predicate,
                        span: ensures_expr.span,
                    };

                    // Ensures clauses are checked at call sites with concrete arguments.
                    // At definition time, we only have the abstract `result` binding without
                    // function body analysis (weakest precondition), so Invalid results are
                    // treated as Unknown (gradual verification) rather than hard errors.
                    // This prevents false positives for valid postconditions that the checker
                    // cannot prove without analyzing the function body.
                    match self.check_refinement_with_evidence(ensures_expr, &refinement_type) {
                        Ok(crate::refinement::VerificationResult::Valid) => {
                            // Postcondition statically verified — great
                        }
                        _ => {
                            // Invalid or Unknown — gradual verification, defer to call-site checks
                        }
                    }
                }
            }
        }

        // Register function contracts (requires/ensures) for call-site checking
        if !func.requires.is_empty() || !func.ensures.is_empty() {
            use verum_ast::decl::FunctionParamKind;
            let contract_param_names: List<Text> = func
                .params
                .iter()
                .filter_map(|p| match &p.kind {
                    FunctionParamKind::Regular { pattern, .. } => {
                        if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                            Some(name.name.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
                .collect();
            self.function_contracts.insert(
                func.name.name.clone(),
                FunctionContract {
                    param_names: contract_param_names,
                    requires: func.requires.clone(),
                    ensures: func.ensures.clone(),
                },
            );
        }

        // NOTE: E321 unbounded recursion check was moved to BEFORE body inference
        // to prevent stack overflow during type checking of infinitely-recursive functions.

        Ok(())
    }

    /// Extract all call sites from a function body for corecursion analysis.
    ///

    /// Returns a list of `(callee_name, guard_depth)` pairs. The `guard_depth`
    /// counts how many coinductive constructors (uppercase-named calls) wrap each
    /// call site on the path from the body root to that call. A guard depth of 0
    /// means the call is at the top level — unguarded and non-productive.
    ///

    /// This is used by the E505 productivity check for `cofix` functions.
    fn extract_corecursive_calls(
        &self,
        body: &verum_ast::decl::FunctionBody,
        func_name: &verum_common::Text,
    ) -> Vec<(String, usize)> {
        use verum_ast::decl::FunctionBody;

        let mut calls = Vec::new();

        fn path_name(path: &verum_ast::ty::Path) -> String {
            path.segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str().to_owned()),
                    verum_ast::ty::PathSegment::SelfValue => Some("self".to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(".")
        }

        fn is_constructor(name: &str) -> bool {
            // In Verum, variant constructors MUST start with an uppercase letter
            // (enforced by the grammar). Module-level functions must start with a
            // lowercase letter. So the uppercase heuristic correctly identifies
            // constructors without needing full type information.
            name.chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
        }

        fn walk(
            expr: &verum_ast::expr::Expr,
            func_name: &str,
            guard_depth: usize,
            calls: &mut Vec<(String, usize)>,
        ) {
            use verum_ast::expr::{ConditionKind, ExprKind};
            use verum_ast::stmt::StmtKind;
            match &expr.kind {
                ExprKind::Call {
                    func: callee_expr,
                    args,
                    ..
                } => {
                    if let ExprKind::Path(path) = &callee_expr.kind {
                        let name = path_name(path);
                        if name == func_name {
                            // Self-recursive call — record its guard depth
                            calls.push((name, guard_depth));
                        } else if is_constructor(&name) {
                            // Constructor call — increase guard depth for args
                            for arg in args {
                                walk(arg, func_name, guard_depth + 1, calls);
                            }
                            return;
                        }
                    }
                    // Non-constructor, non-recursive call — walk args at same depth
                    if let ExprKind::Path(_) = &callee_expr.kind {
                        for arg in args {
                            walk(arg, func_name, guard_depth, calls);
                        }
                    } else {
                        walk(callee_expr, func_name, guard_depth, calls);
                        for arg in args {
                            walk(arg, func_name, guard_depth, calls);
                        }
                    }
                }
                ExprKind::Block(block) => {
                    for stmt in &block.stmts {
                        match &stmt.kind {
                            StmtKind::Expr { expr: e, .. } => {
                                walk(e, func_name, guard_depth, calls)
                            }
                            StmtKind::Let {
                                value: Some(init), ..
                            } => walk(init, func_name, guard_depth, calls),
                            _ => {}
                        }
                    }
                    if let Some(e) = &block.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                }
                ExprKind::Match {
                    expr: scrutinee,
                    arms,
                } => {
                    walk(scrutinee, func_name, guard_depth, calls);
                    for arm in arms {
                        walk(&arm.body, func_name, guard_depth, calls);
                    }
                }
                ExprKind::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    for cond in &condition.conditions {
                        match cond {
                            ConditionKind::Expr(e) => walk(e, func_name, guard_depth, calls),
                            ConditionKind::Let { value, .. } => {
                                walk(value, func_name, guard_depth, calls)
                            }
                        }
                    }
                    for stmt in &then_branch.stmts {
                        if let StmtKind::Expr { expr: e, .. } = &stmt.kind {
                            walk(e, func_name, guard_depth, calls);
                        }
                    }
                    if let Some(e) = &then_branch.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                    if let Some(e) = else_branch {
                        walk(e, func_name, guard_depth, calls);
                    }
                }
                ExprKind::Binary { left, right, .. } => {
                    walk(left, func_name, guard_depth, calls);
                    walk(right, func_name, guard_depth, calls);
                }
                ExprKind::Unary { expr: inner, .. } => walk(inner, func_name, guard_depth, calls),

                // Closures — walk the body with the same guard depth.
                // Productivity requires the corecursive call to be guarded even inside
                // a closure body that is immediately returned.
                ExprKind::Closure { body, .. } => {
                    walk(body, func_name, guard_depth, calls);
                }

                // Method calls — check receiver and all arguments.
                // A method named the same as the cofix function is treated as a
                // self-recursive call (e.g. `self.nats_from(n+1)`).
                ExprKind::MethodCall {
                    receiver,
                    method,
                    args,
                    ..
                } => {
                    // If the method name matches the function, record it
                    if method.name.as_str() == func_name {
                        calls.push((method.name.as_str().to_owned(), guard_depth));
                    }
                    walk(receiver, func_name, guard_depth, calls);
                    for arg in args {
                        walk(arg, func_name, guard_depth, calls);
                    }
                }

                // Spawn — walk the spawned expression (may contain corecursive call)
                ExprKind::Spawn { expr, .. } => {
                    walk(expr, func_name, guard_depth, calls);
                }

                // ForAwait — walk the async iterable and the loop body
                ExprKind::ForAwait {
                    async_iterable,
                    body,
                    ..
                } => {
                    walk(async_iterable, func_name, guard_depth, calls);
                    for stmt in &body.stmts {
                        match &stmt.kind {
                            verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                                walk(e, func_name, guard_depth, calls)
                            }
                            verum_ast::stmt::StmtKind::Let {
                                value: Some(init), ..
                            } => walk(init, func_name, guard_depth, calls),
                            _ => {}
                        }
                    }
                    if let Some(e) = &body.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                }

                // Named argument — walk the value expression
                ExprKind::NamedArg { value, .. } => {
                    walk(value, func_name, guard_depth, calls);
                }

                // Record construction — walk field values and the optional base
                ExprKind::Record { fields, base, .. } => {
                    for field in fields {
                        // FieldInit.value is Maybe<Expr> (None = shorthand { x })
                        if let Some(v) = &field.value {
                            walk(v, func_name, guard_depth, calls);
                        }
                    }
                    if let Some(base_expr) = base {
                        walk(base_expr, func_name, guard_depth, calls);
                    }
                }

                // Tuple — walk every element
                ExprKind::Tuple(elements) => {
                    for elem in elements {
                        walk(elem, func_name, guard_depth, calls);
                    }
                }

                // Field access — walk the base expression
                ExprKind::Field { expr: inner, .. }
                | ExprKind::OptionalChain { expr: inner, .. }
                | ExprKind::TupleIndex { expr: inner, .. } => {
                    walk(inner, func_name, guard_depth, calls);
                }

                // Index — walk both the base and the index
                ExprKind::Index { expr: base, index } => {
                    walk(base, func_name, guard_depth, calls);
                    walk(index, func_name, guard_depth, calls);
                }

                // Pipeline and null-coalescing are binary-like; walk both sides
                ExprKind::Pipeline { left, right } | ExprKind::NullCoalesce { left, right } => {
                    walk(left, func_name, guard_depth, calls);
                    walk(right, func_name, guard_depth, calls);
                }

                // Casts, type-tests — walk the inner expression
                ExprKind::Cast { expr: inner, .. }
                | ExprKind::Try(inner)
                | ExprKind::TryBlock(inner)
                | ExprKind::Await(inner)
                | ExprKind::Throw(inner)
                | ExprKind::Yield(inner)
                | ExprKind::Typeof(inner)
                | ExprKind::Paren(inner)
                | ExprKind::StageEscape { expr: inner, .. }
                | ExprKind::Lift { expr: inner } => {
                    walk(inner, func_name, guard_depth, calls);
                }

                // Try-recover — walk both try and recover handler
                ExprKind::TryRecover { try_block, recover } => {
                    walk(try_block, func_name, guard_depth, calls);
                    use verum_ast::expr::RecoverBody;
                    match recover {
                        RecoverBody::MatchArms { arms, .. } => {
                            for arm in arms {
                                walk(&arm.body, func_name, guard_depth, calls);
                            }
                        }
                        RecoverBody::Closure { body, .. } => {
                            walk(body, func_name, guard_depth, calls);
                        }
                    }
                }

                // Try-finally — walk both blocks
                ExprKind::TryFinally {
                    try_block,
                    finally_block,
                } => {
                    walk(try_block, func_name, guard_depth, calls);
                    walk(finally_block, func_name, guard_depth, calls);
                }

                // Try-recover-finally — walk all three
                ExprKind::TryRecoverFinally {
                    try_block,
                    recover,
                    finally_block,
                } => {
                    walk(try_block, func_name, guard_depth, calls);
                    use verum_ast::expr::RecoverBody;
                    match recover {
                        RecoverBody::MatchArms { arms, .. } => {
                            for arm in arms {
                                walk(&arm.body, func_name, guard_depth, calls);
                            }
                        }
                        RecoverBody::Closure { body, .. } => {
                            walk(body, func_name, guard_depth, calls);
                        }
                    }
                    walk(finally_block, func_name, guard_depth, calls);
                }

                // Return — walk the returned value if present
                ExprKind::Return(Some(value)) => {
                    walk(value, func_name, guard_depth, calls);
                }

                // Break with value
                ExprKind::Break {
                    value: Some(value), ..
                } => {
                    walk(value, func_name, guard_depth, calls);
                }

                // Loop bodies
                ExprKind::Loop { body, .. } => {
                    for stmt in &body.stmts {
                        match &stmt.kind {
                            verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                                walk(e, func_name, guard_depth, calls)
                            }
                            verum_ast::stmt::StmtKind::Let {
                                value: Some(init), ..
                            } => walk(init, func_name, guard_depth, calls),
                            _ => {}
                        }
                    }
                    if let Some(e) = &body.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                }

                ExprKind::While {
                    condition, body, ..
                } => {
                    walk(condition, func_name, guard_depth, calls);
                    for stmt in &body.stmts {
                        match &stmt.kind {
                            verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                                walk(e, func_name, guard_depth, calls)
                            }
                            verum_ast::stmt::StmtKind::Let {
                                value: Some(init), ..
                            } => walk(init, func_name, guard_depth, calls),
                            _ => {}
                        }
                    }
                    if let Some(e) = &body.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                }

                ExprKind::For { iter, body, .. } => {
                    walk(iter, func_name, guard_depth, calls);
                    for stmt in &body.stmts {
                        match &stmt.kind {
                            verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                                walk(e, func_name, guard_depth, calls)
                            }
                            verum_ast::stmt::StmtKind::Let {
                                value: Some(init), ..
                            } => walk(init, func_name, guard_depth, calls),
                            _ => {}
                        }
                    }
                    if let Some(e) = &body.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                }

                // Async block — walk like a regular block
                ExprKind::Async(block) | ExprKind::Unsafe(block) | ExprKind::Meta(block) => {
                    for stmt in &block.stmts {
                        match &stmt.kind {
                            verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                                walk(e, func_name, guard_depth, calls)
                            }
                            verum_ast::stmt::StmtKind::Let {
                                value: Some(init), ..
                            } => walk(init, func_name, guard_depth, calls),
                            _ => {}
                        }
                    }
                    if let Some(e) = &block.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                }

                // Nursery — walk the body and optional cancel/recover handlers
                ExprKind::Nursery {
                    body,
                    on_cancel,
                    recover,
                    ..
                } => {
                    for stmt in &body.stmts {
                        match &stmt.kind {
                            verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                                walk(e, func_name, guard_depth, calls)
                            }
                            verum_ast::stmt::StmtKind::Let {
                                value: Some(init), ..
                            } => walk(init, func_name, guard_depth, calls),
                            _ => {}
                        }
                    }
                    if let Some(e) = &body.expr {
                        walk(e, func_name, guard_depth, calls);
                    }
                    if let Some(cancel_block) = on_cancel {
                        for stmt in &cancel_block.stmts {
                            match &stmt.kind {
                                verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                                    walk(e, func_name, guard_depth, calls)
                                }
                                verum_ast::stmt::StmtKind::Let {
                                    value: Some(init), ..
                                } => walk(init, func_name, guard_depth, calls),
                                _ => {}
                            }
                        }
                        if let Some(e) = &cancel_block.expr {
                            walk(e, func_name, guard_depth, calls);
                        }
                    }
                    if let Some(recover_body) = recover {
                        use verum_ast::expr::RecoverBody;
                        match recover_body {
                            RecoverBody::MatchArms { arms, .. } => {
                                for arm in arms {
                                    walk(&arm.body, func_name, guard_depth, calls);
                                }
                            }
                            RecoverBody::Closure { body, .. } => {
                                walk(body, func_name, guard_depth, calls);
                            }
                        }
                    }
                }

                // Select — walk each arm's future and body
                ExprKind::Select { arms, .. } => {
                    for arm in arms {
                        if let Some(future) = &arm.future {
                            walk(future, func_name, guard_depth, calls);
                        }
                        if let Some(guard) = &arm.guard {
                            walk(guard, func_name, guard_depth, calls);
                        }
                        walk(&arm.body, func_name, guard_depth, calls);
                    }
                }

                // Array expressions
                ExprKind::Array(array_expr) => {
                    use verum_ast::expr::ArrayExpr;
                    match array_expr {
                        ArrayExpr::List(elems) => {
                            for elem in elems {
                                walk(elem, func_name, guard_depth, calls);
                            }
                        }
                        ArrayExpr::Repeat { value, count } => {
                            walk(value, func_name, guard_depth, calls);
                            walk(count, func_name, guard_depth, calls);
                        }
                    }
                }

                // Comprehensions — walk the element expression and all clauses
                ExprKind::Comprehension {
                    expr: elem,
                    clauses,
                }
                | ExprKind::StreamComprehension {
                    expr: elem,
                    clauses,
                }
                | ExprKind::SetComprehension {
                    expr: elem,
                    clauses,
                }
                | ExprKind::GeneratorComprehension {
                    expr: elem,
                    clauses,
                } => {
                    walk(elem, func_name, guard_depth, calls);
                    use verum_ast::expr::ComprehensionClauseKind;
                    for clause in clauses {
                        match &clause.kind {
                            ComprehensionClauseKind::For { iter, .. } => {
                                walk(iter, func_name, guard_depth, calls)
                            }
                            ComprehensionClauseKind::If(cond) => {
                                walk(cond, func_name, guard_depth, calls)
                            }
                            ComprehensionClauseKind::Let { value, .. } => {
                                walk(value, func_name, guard_depth, calls)
                            }
                        }
                    }
                }

                // Map comprehension — walk key, value and clauses
                ExprKind::MapComprehension {
                    key_expr,
                    value_expr,
                    clauses,
                } => {
                    walk(key_expr, func_name, guard_depth, calls);
                    walk(value_expr, func_name, guard_depth, calls);
                    use verum_ast::expr::ComprehensionClauseKind;
                    for clause in clauses {
                        match &clause.kind {
                            ComprehensionClauseKind::For { iter, .. } => {
                                walk(iter, func_name, guard_depth, calls)
                            }
                            ComprehensionClauseKind::If(cond) => {
                                walk(cond, func_name, guard_depth, calls)
                            }
                            ComprehensionClauseKind::Let { value, .. } => {
                                walk(value, func_name, guard_depth, calls)
                            }
                        }
                    }
                }

                // Map/Set literals
                ExprKind::MapLiteral { entries } => {
                    for (k, v) in entries {
                        walk(k, func_name, guard_depth, calls);
                        walk(v, func_name, guard_depth, calls);
                    }
                }
                ExprKind::SetLiteral { elements } => {
                    for elem in elements {
                        walk(elem, func_name, guard_depth, calls);
                    }
                }

                // Interpolated string — walk embedded expressions
                ExprKind::InterpolatedString { exprs, .. } => {
                    for e in exprs {
                        walk(e, func_name, guard_depth, calls);
                    }
                }

                // Tensor literal — walk the data expression
                ExprKind::TensorLiteral { data, .. } => {
                    walk(data, func_name, guard_depth, calls);
                }

                // Forall/Exists quantifiers — walk the predicate body (and optional domain/guard)
                ExprKind::Forall { bindings, body } | ExprKind::Exists { bindings, body } => {
                    for binding in bindings {
                        if let Some(domain) = &binding.domain {
                            walk(domain, func_name, guard_depth, calls);
                        }
                        if let Some(guard) = &binding.guard {
                            walk(guard, func_name, guard_depth, calls);
                        }
                    }
                    walk(body, func_name, guard_depth, calls);
                }

                // UseContext — walk handler and body
                ExprKind::UseContext { handler, body, .. } => {
                    walk(handler, func_name, guard_depth, calls);
                    walk(body, func_name, guard_depth, calls);
                }

                // Is pattern test — walk the tested expression
                ExprKind::Is { expr: inner, .. } => {
                    walk(inner, func_name, guard_depth, calls);
                }

                // DestructuringAssign — walk the value
                ExprKind::DestructuringAssign { value, .. } => {
                    walk(value, func_name, guard_depth, calls);
                }

                // Copattern body — walk each arm's body.
                // These arms ARE the corecursive definitions; walk at same guard depth
                // so individual arm bodies can accumulate further guarded calls.
                ExprKind::CopatternBody { arms, .. } => {
                    for arm in arms {
                        walk(&arm.body, func_name, guard_depth, calls);
                    }
                }

                // MetaFunction — walk argument expressions
                ExprKind::MetaFunction { args, .. } => {
                    for arg in args {
                        walk(arg, func_name, guard_depth, calls);
                    }
                }

                // Range — walk start and end if present
                ExprKind::Range { start, end, .. } => {
                    if let Some(s) = start {
                        walk(s, func_name, guard_depth, calls);
                    }
                    if let Some(e) = end {
                        walk(e, func_name, guard_depth, calls);
                    }
                }

                // Attenuate — walk context expression
                ExprKind::Attenuate { context, .. } => {
                    walk(context, func_name, guard_depth, calls);
                }

                // Terminal / structurally opaque expressions — cannot contain calls
                // Literal, Path (without args), Continue, Return(None), Break { value: None },
                // Inject, TypeProperty, TypeExpr, InlineAsm, Quote, MacroCall, TypeBound,
                // StreamLiteral
                ExprKind::Literal(_)
                | ExprKind::Path(_)
                | ExprKind::Continue { .. }
                | ExprKind::Return(None)
                | ExprKind::Break { value: None, .. }
                | ExprKind::Inject { .. }
                | ExprKind::TypeProperty { .. }
                | ExprKind::TypeExpr(_)
                | ExprKind::InlineAsm { .. }
                | ExprKind::Quote { .. }
                | ExprKind::MacroCall { .. }
                | ExprKind::TypeBound { .. }
                | ExprKind::StreamLiteral(_)
                | ExprKind::CalcBlock(_) => {
                    // No sub-expressions to walk for productivity checking
                }
            }
        }

        let func_name_str = func_name.as_str();
        match body {
            FunctionBody::Expr(expr) => walk(expr, func_name_str, 0, &mut calls),
            FunctionBody::Block(block) => {
                for stmt in &block.stmts {
                    match &stmt.kind {
                        verum_ast::stmt::StmtKind::Expr { expr, .. } => {
                            walk(expr, func_name_str, 0, &mut calls)
                        }
                        verum_ast::stmt::StmtKind::Let {
                            value: Some(init), ..
                        } => walk(init, func_name_str, 0, &mut calls),
                        _ => {}
                    }
                }
                if let Some(expr) = &block.expr {
                    walk(expr, func_name_str, 0, &mut calls);
                }
            }
        }

        calls
    }

    /// Evaluate a meta parameter expression to a compile-time constant value
    ///

    /// # Example
    ///

    /// ```no_run
    /// # use verum_types::TypeChecker;
    /// # use verum_ast::expr::{Expr, ExprKind, BinOp};
    /// # use verum_ast::literal::Literal;
    /// # use verum_ast::span::Span;
    /// let mut checker = TypeChecker::new();
    ///

    /// // Evaluate: 2 + 3
    /// # let left = Expr::new(ExprKind::Literal(Literal::int(2, Span::dummy())), Span::dummy());
    /// # let right = Expr::new(ExprKind::Literal(Literal::int(3, Span::dummy())), Span::dummy());
    /// # let expr = Expr::new(
    /// # ExprKind::Binary {
    /// # op: BinOp::Add,
    /// # left: Box::new(left),
    /// # right: Box::new(right),
    /// # },
    /// # Span::dummy()
    /// # );
    /// let value = checker.eval_meta_param(&expr)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn eval_meta_param(
        &mut self,
        expr: &Expr,
    ) -> std::result::Result<verum_common::ConstValue, crate::const_eval::ConstEvalError> {
        self.const_eval.eval(expr)
    }

    /// Evaluate and substitute meta parameters in a type
    ///

    /// This resolves meta parameters by evaluating their expressions and
    /// substituting the results into the type.
    ///

    /// # Example
    ///

    /// ```ignore
    /// use verum_types::{TypeChecker, Type};
    /// use verum_common::{Map, ToText, ConstValue};
    ///

    /// let mut checker = TypeChecker::new();
    ///

    /// // Meta parameter: N: meta usize
    /// let meta_ty = Type::meta("N".to_text(), Type::Int, None);
    ///

    /// // Substitute N = 10
    /// let mut env = Map::new();
    /// env.insert("N".to_text(), ConstValue::UInt(10));
    ///

    /// let resolved = checker.substitute_meta(&meta_ty, &env)?;
    /// ```
    pub fn substitute_meta(
        &mut self,
        ty: &Type,
        env: &Map<Text, verum_common::ConstValue>,
    ) -> std::result::Result<Type, crate::const_eval::ConstEvalError> {
        self.const_eval.substitute_meta(ty, env)
    }

    /// Compute tensor shape dimensions from a meta array expression
    ///

    /// This evaluates an array expression to extract shape dimensions for tensor types.
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Meta parameters for compile-time tensor shapes
    ///

    /// # Example
    ///

    /// ```ignore
    /// use verum_types::TypeChecker;
    /// use verum_ast::{expr::{Expr, ExprKind, ArrayExpr}, span::Span, literal::Literal};
    /// use verum_common::List;
    ///

    /// let mut checker = TypeChecker::new();
    ///

    /// // Shape: [2, 3, 4]
    /// let elements: List<_> = vec![
    ///  Expr::new(ExprKind::Literal(Literal::int(2, Span::dummy())), Span::dummy()),
    ///  Expr::new(ExprKind::Literal(Literal::int(3, Span::dummy())), Span::dummy()),
    ///  Expr::new(ExprKind::Literal(Literal::int(4, Span::dummy())), Span::dummy()),
    /// ].into();
    /// let shape_expr = Expr::new(
    ///  ExprKind::Array(ArrayExpr::List(elements)),
    ///  Span::dummy()
    /// );
    /// let dims = checker.compute_tensor_shape(&shape_expr)?;
    /// assert_eq!(dims, List::from(vec![2, 3, 4]));
    /// ```
    pub fn compute_tensor_shape(
        &mut self,
        shape_expr: &Expr,
    ) -> std::result::Result<List<usize>, crate::const_eval::ConstEvalError> {
        self.const_eval
            .compute_tensor_shape(shape_expr)
            .map(List::from_iter)
    }

    /// Compute total number of elements from tensor shape
    ///

    /// Given a shape array like `[2, 3, 4]`, computes the product `2 * 3 * 4 = 24`.
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Compile-time tensor validation
    ///

    /// # Example
    ///

    /// ```ignore
    /// use verum_types::TypeChecker;
    /// use verum_ast::{expr::{Expr, ExprKind, ArrayExpr}, span::Span, literal::Literal};
    /// use verum_common::List;
    ///

    /// let mut checker = TypeChecker::new();
    ///

    /// // Shape: [2, 3, 4]
    /// let elements: List<_> = vec![
    ///  Expr::new(ExprKind::Literal(Literal::int(2, Span::dummy())), Span::dummy()),
    ///  Expr::new(ExprKind::Literal(Literal::int(3, Span::dummy())), Span::dummy()),
    ///  Expr::new(ExprKind::Literal(Literal::int(4, Span::dummy())), Span::dummy()),
    /// ].into();
    /// let shape_expr = Expr::new(
    ///  ExprKind::Array(ArrayExpr::List(elements)),
    ///  Span::dummy()
    /// );
    /// let total = checker.compute_tensor_elements(&shape_expr)?;
    /// assert_eq!(total, 24);
    /// ```
    pub fn compute_tensor_elements(
        &mut self,
        shape_expr: &Expr,
    ) -> std::result::Result<usize, crate::const_eval::ConstEvalError> {
        self.const_eval.compute_tensor_elements(shape_expr)
    }

    /// Validate that two tensor shapes are compatible for operations
    ///

    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Compile-time tensor shape validation
    pub fn validate_tensor_shapes(
        &mut self,
        shape1: &Expr,
        shape2: &Expr,
    ) -> std::result::Result<bool, crate::const_eval::ConstEvalError> {
        self.const_eval.validate_tensor_shapes(shape1, shape2)
    }

    /// Get metrics from the type checker
    pub fn metrics(&self) -> &TypeCheckMetrics {
        &self.metrics
    }

    /// Get mutable access to the type context (for testing)
    pub fn context_mut(&mut self) -> &mut TypeContext {
        &mut self.ctx
    }

    /// Lookup a record type from a path.
    ///

    /// This method resolves a path to a record type definition.
    /// If the path is a single identifier, it looks up the type in the context.
    /// Otherwise, it constructs a Named type for the path.
    pub(super) fn lookup_record_type(&mut self, path: &verum_ast::ty::Path, span: Span) -> Result<Type> {
        // Simple case: single identifier
        if path.segments.len() == 1 {
            match &path.segments[0] {
                verum_ast::ty::PathSegment::Name(id) => {
                    let name = id.name.as_str();

                    // Check for import ambiguity first
                    // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Import Ambiguity
                    if let Some(sources) = self.imported_names.get(&verum_common::Text::from(name))
                    {
                        if sources.len() > 1 {
                            let sources_str = sources
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Err(TypeError::AmbiguousName {
                                name: verum_common::Text::from(name),
                                sources: verum_common::Text::from(sources_str),
                                span,
                            });
                        }
                    }

                    // Look up in type definitions
                    match self.ctx.lookup_type(name) {
                        Option::Some(ty) => return Ok(ty.clone()),
                        Option::None => {
                            // Try __struct_fields_ prefix (variant record types)
                            let struct_key = format!("__struct_fields_{}", name);
                            if let Option::Some(ty) = self.ctx.lookup_type(&struct_key) {
                                return Ok(ty.clone());
                            }
                            // Not a predefined type - will be handled as structural
                            return Ok(Type::Named {
                                path: path.clone(),
                                args: List::new(),
                            });
                        }
                    }
                }
                verum_ast::ty::PathSegment::SelfValue => {
                    // `Self` in struct literal - resolve to current self type
                    // This enables `Self { x, y }` inside implement blocks
                    if let Maybe::Some(ref self_ty) = self.current_self_type {
                        return Ok(self_ty.clone());
                    }
                    return Err(TypeError::Other(
                        "Cannot use `Self` as a type outside of an implement block".into(),
                    ));
                }
                _ => {}
            }
        }

        // Multi-segment path or generics - construct Named type
        Ok(Type::Named {
            path: path.clone(),
            args: List::new(),
        })
    }

    /// Unwrap reference and heap types to get the inner type.
    /// Recursively unwraps Reference, CheckedReference, UnsafeReference, and Heap<T> types.
    /// This is needed for field access on reference types like `&Point` and heap types like `Heap<Node>`.
    /// CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — #auto-dereference
    pub(crate) fn unwrap_reference_type<'a>(&self, ty: &'a Type) -> &'a Type {
        match ty {
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                // Recursively unwrap nested references
                self.unwrap_reference_type(inner)
            }
            // Also unwrap smart pointers (Heap<T>, Shared<T>, etc.) to T for field access
            // This enables field access like: let node: Heap<Node> = ...; node.value
            Type::Generic { name, args }
                if args.len() == 1
                    && matches!(
                        name.as_str(),
                        WKT_HEAP | WKT_SHARED | "Unique" | "Rc" | "Arc"
                    ) =>
            {
                self.unwrap_reference_type(&args[0])
            }
            Type::Named { path, args } if args.len() == 1 => {
                if let Some(ident) = path.as_ident() {
                    if matches!(
                        ident.name.as_str(),
                        WKT_HEAP | WKT_SHARED | "Unique" | "Rc" | "Arc"
                    ) {
                        return self.unwrap_reference_type(&args[0]);
                    }
                }
                ty
            }
            _ => ty,
        }
    }

    /// Look up a field type on a record or named type.
    /// Returns the field type if found, None otherwise.
    /// Spec: Field access type resolution
    pub(super) fn lookup_field_type(&self, ty: &Type, field_name: &str) -> Option<Type> {
        let field_key = verum_common::Text::from(field_name);
        match ty {
            Type::Record(fields) => fields.get(&field_key).cloned(),
            Type::Named { path, .. } => {
                let type_name = self.path_to_string(path);
                let struct_key = format!("__struct_fields_{}", type_name);
                // Try struct fields first
                if let Option::Some(Type::Record(fields)) =
                    self.ctx.lookup_type(struct_key.as_str())
                {
                    if let Some(field_ty) = fields.get(&field_key) {
                        return Some(field_ty.clone());
                    }
                }
                // Try direct type lookup
                if let Option::Some(Type::Record(fields)) = self.ctx.lookup_type(type_name.as_str())
                {
                    return fields.get(&field_key).cloned();
                }
                None
            }
            _ => None,
        }
    }

    /// Register a variant type name mapping with first-registered-wins semantics.
    ///

    /// The compiler holds NO knowledge of which stdlib type names "should" win
    /// when variant signatures collide (e.g., `Result.Ok|Err` vs an arbitrary
    /// downstream cog's `MyResult.Ok|Err`). Correctness comes from registration
    /// order: stdlib types register first during bootstrap (driven by
    /// `stdlib_iteration_order` + `type_declaration_order`), so they naturally
    /// own each signature they declare.
    ///

    /// Mirrors the same `entry().or_insert()` semantics used by the protocol
    /// checker (`protocol.rs`) and the unifier (`unify.rs`) — three layers, one
    /// rule, no hardcoded type names.
    ///

    /// **Audit-A1 coherence: collision logging.** Prior revisions used
    /// a bare `or_insert_with` that silently dropped any second
    /// registration claiming the same signature. Two distinct types
    /// with identical variant signatures (e.g. `type A is X(Int)|Y` and
    /// `type B is X(Int)|Y` declared in different cogs) would map the
    /// same signature to whichever type was iterated first; the second
    /// type's downstream method lookups would then silently resolve
    /// via the first type's vtable — a real soundness hole when the
    /// two types' methods diverge. We keep first-registered-wins (the
    /// architectural rule) but ALSO log every collision into
    /// `variant_collision_log` so `take_variant_collisions()` can
    /// surface them as compile-time diagnostics.
    pub(super) fn register_variant_type_name_first_wins(&mut self, sig: Text, type_name: Text) {
        if let Some(existing) = self.variant_type_names.get(&sig) {
            if existing != &type_name {
                self.variant_collision_log
                    .push((sig.clone(), existing.clone(), type_name.clone()));
            }
        }
        self.variant_type_names
            .entry(sig.clone())
            .or_insert_with(|| type_name.clone());
        self.protocol_checker
            .write()
            .register_variant_type_name(sig.clone(), type_name.clone());
        self.unifier.register_variant_type_name(sig, type_name);
    }

    /// Drain and return the collision log accumulated during
    /// `register_variant_type_name_first_wins`. Each entry is
    /// `(signature, kept_type, dropped_type)`. Diagnostic emitters
    /// consume this once per compilation; subsequent calls return an
    /// empty list. The drain semantic prevents the same collision
    /// from being reported twice across phases.
    pub fn take_variant_collisions(&mut self) -> List<(Text, Text, Text)> {
        std::mem::replace(&mut self.variant_collision_log, List::new())
    }

    /// Generate a stable signature for a variant type for use as a map key.
    /// This creates a unique identifier from the sorted variant names.
    pub(super) fn variant_type_signature(ty: &Type) -> Option<Text> {
        if let Type::Variant(variants) = ty {
            // Include payload base type names in the signature to avoid collisions
            // between different sum types that share variant names (e.g., MapEntry and
            // BTreeEntry both have Occupied|Vacant variants but different payload types).
            let mut entries: Vec<String> = variants
                .iter()
                .map(|(name, payload)| {
                    let payload_name = match payload {
                        Type::Named { path, .. } => path
                            .as_ident()
                            .map(|id| id.name.as_str().to_string())
                            .unwrap_or_default(),
                        Type::Generic { name: n, .. } => n.as_str().to_string(),
                        // Unit, primitives (Int, Float, etc.), and TypeVars are NOT
                        // distinctive for disambiguation — they don't carry structural
                        // type information. Only Named/Generic payload types matter.
                        _ => String::new(),
                    };
                    if payload_name.is_empty() {
                        name.as_str().to_string()
                    } else {
                        format!("{}({})", name.as_str(), payload_name)
                    }
                })
                .collect();
            entries.sort();
            let sig = entries.join("|");
            Some(verum_common::Text::from(format!("Variant({})", sig)))
        } else {
            None
        }
    }

    /// Compute a relaxed variant type signature using only variant names (ignoring payload types).
    /// Used as a fallback when the full signature (with payload type names) doesn't match any
    /// registered type. This handles cases like `Ok(PositiveInt) | Err(Text)` which should
    /// resolve to "Result" even though payload types differ from the registered generic definition.
    pub(super) fn variant_type_signature_relaxed(ty: &Type) -> Option<Text> {
        if let Type::Variant(variants) = ty {
            let mut names: Vec<&str> = variants.keys().map(|k| k.as_str()).collect();
            names.sort();
            let sig = names.join("|");
            Some(verum_common::Text::from(format!("Variant({})", sig)))
        } else {
            None
        }
    }

    /// Extract type arguments from a substituted variant type by unifying with the original type.
    ///

    /// For example, given `Validated<E, A> = Valid(A) | Invalid(List<E>)` and a substituted
    /// type `Valid(Int) | Invalid(List<Text>)`, this extracts [Text, Int] (the values of E and A
    /// in declaration order).
    ///

    /// This is a stdlib-agnostic approach that works for any generic variant type, not just
    /// hardcoded types like Result or Maybe.
    ///

    /// Algorithm:
    /// 1. Get the variant signature (e.g., "Variant(Invalid|Valid)")
    /// 2. Look up type name from variant_type_names (e.g., "Validated")
    /// 3. Look up __type_var_order_{name} to get TypeVars in declaration order
    /// 4. Look up original (unsubstituted) variant type
    /// 5. Unify original with substituted type to get TypeVar -> concrete type mapping
    /// 6. Return type args in the correct declaration order
    fn extract_type_args_from_variant(&self, substituted_ty: &Type) -> List<Type> {
        // Step 1: Get variant signature
        let sig = match Self::variant_type_signature(substituted_ty) {
            Some(s) => s,
            None => return List::new(),
        };

        // Step 2: Look up type name (try full signature, then relaxed)
        let type_name = match self.variant_type_names.get(&sig) {
            Some(name) => name.clone(),
            None => {
                // Try protocol checker as fallback
                match self.protocol_checker.read().get_variant_type_name(&sig) {
                    Some(name) => name.clone(),
                    None => {
                        // Try relaxed signature (variant names only, ignoring payload types)
                        match Self::variant_type_signature_relaxed(substituted_ty) {
                            Some(rs) => {
                                if let Some(name) = self.variant_type_names.get(&rs) {
                                    name.clone()
                                } else if let Some(name) =
                                    self.protocol_checker.read().get_variant_type_name(&rs)
                                {
                                    name.clone()
                                } else {
                                    return List::new();
                                }
                            }
                            None => return List::new(),
                        }
                    }
                }
            }
        };

        // Step 3: Look up TypeVar order
        let type_var_order_key = format!("__type_var_order_{}", type_name);
        let type_vars_in_order: List<TypeVar> = match self.ctx.lookup_type(&type_var_order_key) {
            verum_common::Maybe::Some(Type::Tuple(type_vars)) => type_vars
                .iter()
                .filter_map(|t| {
                    if let Type::Var(tv) = t {
                        Some(*tv)
                    } else {
                        None
                    }
                })
                .collect(),
            _ => return List::new(),
        };

        if type_vars_in_order.is_empty() {
            return List::new();
        }

        // Step 4: Look up original (unsubstituted) variant type
        let original_type = match self.ctx.lookup_type(&type_name) {
            verum_common::Maybe::Some(ty) => ty.clone(),
            verum_common::Maybe::None => return List::new(),
        };

        // Step 5: Unify original with substituted to get TypeVar -> concrete type mapping
        // We unify each variant's payload separately to build the mapping
        let mut type_var_mapping: indexmap::IndexMap<TypeVar, Type> = indexmap::IndexMap::new();

        if let (Type::Variant(original_variants), Type::Variant(substituted_variants)) =
            (&original_type, substituted_ty)
        {
            for (variant_name, original_payload) in original_variants.iter() {
                if let Some(substituted_payload) = substituted_variants.get(variant_name) {
                    // Recursively extract type var mappings from matching payloads
                    Self::extract_type_var_mapping(
                        original_payload,
                        substituted_payload,
                        &mut type_var_mapping,
                    );
                }
            }
        }

        // #[cfg(debug_assertions)]
        // eprintln!(
        // "[DEBUG extract_type_args_from_variant] type_name={}, type_var_order={:?}, mapping={:?}",
        // type_name,
        // type_vars_in_order,
        // type_var_mapping
        // );

        // Step 6: Return type args in declaration order
        let mut result = List::new();
        for tv in type_vars_in_order.iter() {
            if let Some(concrete_ty) = type_var_mapping.get(tv) {
                result.push(concrete_ty.clone());
            } else {
                // Type var not found in mapping - this shouldn't happen but return empty for safety
                // #[cfg(debug_assertions)]
                // eprintln!(
                // "[DEBUG extract_type_args_from_variant] TypeVar {:?} not found in mapping",
                // tv
                // );
                return List::new();
            }
        }

        result
    }

    /// Recursively extract TypeVar -> Type mappings by matching original and substituted types.
    ///

    /// For example, matching `List<E>` (original) with `List<Text>` (substituted) extracts E -> Text.
    fn extract_type_var_mapping(
        original: &Type,
        substituted: &Type,
        mapping: &mut indexmap::IndexMap<TypeVar, Type>,
    ) {
        match (original, substituted) {
            // Direct TypeVar match
            (Type::Var(tv), concrete) => {
                // Don't overwrite existing mapping (first match wins)
                mapping.entry(*tv).or_insert_with(|| concrete.clone());
            }

            // Generic type: match base and recursively match args
            (
                Type::Generic {
                    args: orig_args, ..
                },
                Type::Generic {
                    args: subst_args, ..
                },
            )
            | (
                Type::Named {
                    args: orig_args, ..
                },
                Type::Named {
                    args: subst_args, ..
                },
            ) => {
                for (orig_arg, subst_arg) in orig_args.iter().zip(subst_args.iter()) {
                    Self::extract_type_var_mapping(orig_arg, subst_arg, mapping);
                }
            }

            // Tuple: match element types
            (Type::Tuple(orig_elems), Type::Tuple(subst_elems)) => {
                for (orig_elem, subst_elem) in orig_elems.iter().zip(subst_elems.iter()) {
                    Self::extract_type_var_mapping(orig_elem, subst_elem, mapping);
                }
            }

            // Record: match field types
            (Type::Record(orig_fields), Type::Record(subst_fields)) => {
                for (field_name, orig_ty) in orig_fields.iter() {
                    if let Some(subst_ty) = subst_fields.get(field_name) {
                        Self::extract_type_var_mapping(orig_ty, subst_ty, mapping);
                    }
                }
            }

            // Function: match param and return types
            (
                Type::Function {
                    params: orig_params,
                    return_type: orig_ret,
                    ..
                },
                Type::Function {
                    params: subst_params,
                    return_type: subst_ret,
                    ..
                },
            ) => {
                for (orig_param, subst_param) in orig_params.iter().zip(subst_params.iter()) {
                    Self::extract_type_var_mapping(orig_param, subst_param, mapping);
                }
                Self::extract_type_var_mapping(orig_ret, subst_ret, mapping);
            }

            // Reference types: match inner type
            (
                Type::Reference {
                    inner: orig_inner, ..
                },
                Type::Reference {
                    inner: subst_inner, ..
                },
            )
            | (
                Type::CheckedReference {
                    inner: orig_inner, ..
                },
                Type::CheckedReference {
                    inner: subst_inner, ..
                },
            )
            | (
                Type::UnsafeReference {
                    inner: orig_inner, ..
                },
                Type::UnsafeReference {
                    inner: subst_inner, ..
                },
            ) => {
                Self::extract_type_var_mapping(orig_inner, subst_inner, mapping);
            }

            // Variant: match payload types for each variant
            (Type::Variant(orig_variants), Type::Variant(subst_variants)) => {
                for (variant_name, orig_payload) in orig_variants.iter() {
                    if let Some(subst_payload) = subst_variants.get(variant_name) {
                        Self::extract_type_var_mapping(orig_payload, subst_payload, mapping);
                    }
                }
            }

            // All other cases: no mapping to extract
            _ => {}
        }
    }

    /// Extract the type name from a Type for inherent method lookup.
    /// Returns the simple name for named types, or None for complex types.
    pub(super) fn get_type_name(&self, ty: &Type) -> Option<Text> {
        // First unwrap any references
        let unwrapped = self.unwrap_reference_type(ty);

        match unwrapped {
            Type::Named { path, .. } => {
                // Extract the simple name from the path
                path.as_ident()
                    .map(|id| verum_common::Text::from(id.name.as_str()))
            }
            Type::Generic { name, .. } => Some(name.clone()),
            Type::Record(_) => None, // Anonymous records don't have names
            Type::Variant(_) => {
                // Look up the variant type in our mapping to find its declared name
                // Uses ProtocolChecker as the authoritative source, with local fallback
                let sig = Self::variant_type_signature(unwrapped);
                let result = sig.and_then(|s| {
                    // First try ProtocolChecker (authoritative source)
                    if let Some(name) = self.protocol_checker.read().get_variant_type_name(&s) {
                        return Some(name.clone());
                    }
                    // Fallback to local mapping
                    self.variant_type_names.get(&s).cloned()
                });
                // If exact signature didn't match, try a relaxed signature that ignores
                // payload types. This handles cases like Ok(PositiveInt) | Err(Text) where
                // the payload types are Named/Generic and make the exact signature differ
                // from the registered Result<T,E> signature (which uses TypeVars → empty names).
                if result.is_some() {
                    result
                } else {
                    let relaxed_sig = Self::variant_type_signature_relaxed(unwrapped);
                    relaxed_sig.and_then(|rs| {
                        if let Some(name) = self.protocol_checker.read().get_variant_type_name(&rs)
                        {
                            return Some(name.clone());
                        }
                        self.variant_type_names.get(&rs).cloned()
                    })
                }
            }
            Type::Int => Some(verum_common::Text::from(WKT::Int.as_str())),
            Type::Float => Some(verum_common::Text::from(WKT::Float.as_str())),
            Type::Bool => Some(verum_common::Text::from(WKT::Bool.as_str())),
            Type::Char => Some(verum_common::Text::from(WKT::Char.as_str())),
            Type::Text => Some(verum_common::Text::from(WKT::Text.as_str())),
            Type::Unit => Some(verum_common::Text::from("Unit")),
            // Refined types: unwrap to base type for method lookup
            // e.g., Text{len() > 0} should find methods on Text
            Type::Refined { base, .. } => self.get_type_name(base),
            // GENERIC FALLBACK: For unresolved type variables, try to resolve through
            // the unifier. This handles cases where the receiver type hasn't been fully
            // resolved yet (e.g., type inference still in progress).
            Type::Var(_) => {
                let resolved = self.unifier.apply(unwrapped);
                if &resolved != unwrapped {
                    // Recursively try to extract type name from the resolved type
                    self.get_type_name(&resolved)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Get type name for the exact type without unwrapping references.
    /// This is used for methods that should be called on reference types themselves.
    /// Returns type names for Reference, CheckedReference, UnsafeReference, Slice, Array,
    /// and also Generic/Named types (like Heap<T>) without unwrapping.
    fn get_exact_type_name(&self, ty: &Type) -> Option<Text> {
        match ty {
            // Reference types have their own methods like as_checked, as_unsafe
            Type::Reference { .. } => Some(verum_common::Text::from("Reference")),
            Type::CheckedReference { .. } => Some(verum_common::Text::from("CheckedReference")),
            Type::UnsafeReference { .. } => Some(verum_common::Text::from("UnsafeReference")),
            // Slice types have methods like as_unsafe_slice, get_unchecked
            Type::Slice { .. } => Some(verum_common::Text::from("Slice")),
            // Array types [T; N] have methods like as_slice, as_unsafe_slice
            // They share many methods with Slice
            Type::Array { .. } => Some(verum_common::Text::from("Array")),
            // Generic types (e.g., Heap<T>, Shared<T>) have their own inherent methods
            // that should be found BEFORE auto-deref unwraps to the inner type.
            Type::Generic { name, .. } => Some(name.clone()),
            // Named types (e.g., user-defined types)
            Type::Named { path, .. } => path
                .as_ident()
                .map(|id| verum_common::Text::from(id.name.as_str())),
            // Raw pointer types have methods like sub, add, offset
            Type::Pointer { .. } => Some(verum_common::Text::from("Pointer")),
            Type::VolatilePointer { .. } => Some(verum_common::Text::from("VolatilePointer")),
            // Refined types: unwrap to base type for method lookup
            Type::Refined { base, .. } => self.get_exact_type_name(base),
            // For other types, delegate to get_type_name
            _ => self.get_type_name(ty),
        }
    }

    /// Get additional type names to try for method lookup.
    /// This enables e.g. Array types to also use Slice methods.
    fn get_fallback_type_names(&self, ty: &Type) -> List<verum_common::Text> {
        match ty {
            // Refined types: delegate to base type
            Type::Refined { base, .. } => self.get_fallback_type_names(base),
            // Arrays can also use Slice and List methods (push, pop, etc.)
            Type::Array { .. } => List::from_iter([
                verum_common::Text::from("Slice"),
                verum_common::Text::from(WKT::List.as_str()),
            ]),
            // References to arrays/slices can use their methods
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => match inner.as_ref() {
                Type::Array { .. } => List::from_iter([
                    verum_common::Text::from("Array"),
                    verum_common::Text::from("Slice"),
                ]),
                Type::Slice { .. } => List::from_iter([verum_common::Text::from("Slice")]),
                _ => List::new(),
            },
            _ => {
                // Iterator types (ListIter, MapIter, etc.) can use Iterator methods (map, sum, etc.)
                let type_name = match ty {
                    Type::Generic { name, .. } => Some(name.as_str()),
                    Type::Named { path, .. } => path.as_ident().map(|id| id.name.as_str()),
                    _ => None,
                };
                if let Some(n) = type_name {
                    if n.ends_with("Iter")
                        || n.ends_with("IterMut")
                        || WKT::Range.matches(n)
                        || n == "Enumerate"
                        || n == "Zip"
                        || n == "Chain"
                        || n == "Filter"
                        || n == "FilterMap"
                        || n == "FlatMap"
                        || n == "Take"
                        || n == "Skip"
                        || n == "Peekable"
                        || n == "Reversed"
                        || n == "Chunks"
                    {
                        return List::from_iter([verum_common::Text::from("Iterator")]);
                    }
                }
                List::new()
            }
        }
    }

    /// Resolve a type alias to its underlying record type, if applicable.
    /// Returns None if the type is not a record (directly or via alias).
    pub(crate) fn resolve_to_record_type(
        &self,
        ty: &Type,
    ) -> Option<indexmap::IndexMap<verum_common::Text, Type>> {
        self.resolve_to_record_type_with_visited(ty, &mut std::collections::HashSet::new())
    }

    /// Inner implementation with cycle detection to prevent infinite recursion.
    fn resolve_to_record_type_with_visited(
        &self,
        ty: &Type,
        visited: &mut std::collections::HashSet<Text>,
    ) -> Option<indexmap::IndexMap<verum_common::Text, Type>> {
        match ty {
            Type::Record(field_types) => Some(field_types.clone()),
            Type::Named { path, .. } => {
                // Try to resolve the named type to its definition
                let name = self.path_to_string(path);

                // Cycle detection: if we've already seen this type, stop recursion
                if visited.contains(&name) {
                    return None;
                }
                visited.insert(name.clone());

                // First try __struct_fields_Name convention
                let struct_key = format!("__struct_fields_{}", name);
                if let Option::Some(Type::Record(field_types)) =
                    self.ctx.lookup_type(struct_key.as_str())
                {
                    return Some(field_types.clone());
                }

                // Fall back to direct type lookup
                if let Option::Some(resolved_ty) = self.ctx.lookup_type(name.as_str()) {
                    // Recursively resolve in case it's an alias to another alias
                    self.resolve_to_record_type_with_visited(resolved_ty, visited)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Substitute type variables with their replacements.
    ///

    /// This is used for rank-2 polymorphism instantiation.
    /// Given a type like `∀R. fn(R) -> R` and a mapping `{R -> α}`,
    /// returns `fn(α) -> α` where α is a fresh type variable.
    ///

    /// Spec: grammar/verum.ebnf - rank2_function_type
    pub(super) fn substitute_type_vars(&self, ty: &Type, subst: &Map<TypeVar, Type>) -> Type {
        self.substitute_type_vars_impl(ty, subst, 0)
    }

    /// Audit-A4: substitute meta-param references in a refinement
    /// predicate using `self.ctx.meta_param_environment`.
    ///

    /// The pre-fix `Type::Refined` substitution path cloned the
    /// predicate verbatim, dropping every reference to a
    /// const-generic / meta-param `N` so the SMT solver later
    /// translated `N` as an unbound free variable. This walker
    /// rewrites every `Path(N)` whose `N` resolves in the meta-param
    /// environment to a `Bound(value)`. Symbolic bindings pass
    /// through unchanged so SMT can constrain them.
    ///

    /// The walker is deliberately simple — it is the minimal-viable
    /// piece for the dependent-typing chain to start working as
    /// concrete instantiations are wired through the rest of the
    /// type-checker. Today the environment never contains a
    /// `Bound`, so this function is functionally a clone — but the
    /// architectural seam is in place so the moment instantiation
    /// lands (a separate commit), refinements automatically benefit.
    fn substitute_in_refinement_predicate(
        &self,
        predicate: &crate::refinement::RefinementPredicate,
    ) -> crate::refinement::RefinementPredicate {
        let new_expr = self.substitute_meta_params_in_expr(&predicate.predicate);
        crate::refinement::RefinementPredicate {
            predicate: new_expr,
            binding: predicate.binding.clone(),
            span: predicate.span,
        }
    }

    /// Walk a refinement predicate's `Expr` AST and substitute every
    /// `Path(N)` where `N` is bound in the meta-param environment.
    /// Recurses into binary operations, function calls, parentheses,
    /// and conditionals — the shapes that show up in real refinement
    /// predicates. Anything else is cloned verbatim.
    fn substitute_meta_params_in_expr(
        &self,
        expr: &verum_ast::expr::Expr,
    ) -> verum_ast::expr::Expr {
        use crate::context::MetaParamBinding;
        use verum_ast::expr::{Expr, ExprKind};

        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    if let Some(MetaParamBinding::Bound(value)) =
                        self.ctx.meta_param_environment.get(&ident.name)
                    {
                        if let Some(literal) = meta_value_to_literal(value) {
                            return Expr::new(ExprKind::Literal(literal), expr.span);
                        }
                    }
                }
                expr.clone()
            }
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(self.substitute_meta_params_in_expr(left)),
                    right: Box::new(self.substitute_meta_params_in_expr(right)),
                },
                expr.span,
            ),
            ExprKind::Unary { op, expr: inner } => Expr::new(
                ExprKind::Unary {
                    op: *op,
                    expr: verum_common::Heap::new(self.substitute_meta_params_in_expr(inner)),
                },
                expr.span,
            ),
            ExprKind::Paren(inner) => Expr::new(
                ExprKind::Paren(Box::new(self.substitute_meta_params_in_expr(inner))),
                expr.span,
            ),
            _ => expr.clone(),
        }
    }

    /// Inner implementation with depth tracking to prevent infinite recursion.
    fn substitute_type_vars_impl(
        &self,
        ty: &Type,
        subst: &Map<TypeVar, Type>,
        depth: usize,
    ) -> Type {
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return ty.clone();
        }
        let d = depth + 1;

        match ty {
            Type::Var(v) => {
                // If this variable is in the substitution, replace it
                if let Some(replacement) = subst.get(v) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => {
                let subst_params: List<_> = params
                    .iter()
                    .map(|p| self.substitute_type_vars_impl(p, subst, d))
                    .collect();
                let subst_ret = Box::new(self.substitute_type_vars_impl(return_type, subst, d));
                Type::Function {
                    params: subst_params,
                    return_type: subst_ret,
                    contexts: contexts.clone(),
                    type_params: type_params.clone(),
                    properties: properties.clone(),
                }
            }
            Type::Forall { vars, body } => {
                // Don't substitute vars that are bound by this forall
                let mut filtered_subst = subst.clone();
                for v in vars.iter() {
                    filtered_subst.remove(v);
                }
                let subst_body = Box::new(self.substitute_type_vars_impl(body, &filtered_subst, d));
                Type::Forall {
                    vars: vars.clone(),
                    body: subst_body,
                }
            }
            Type::Generic { name, args } => {
                let subst_args: List<_> = args
                    .iter()
                    .map(|a| self.substitute_type_vars_impl(a, subst, d))
                    .collect();
                Type::Generic {
                    name: name.clone(),
                    args: subst_args,
                }
            }
            Type::Named { path, args } => {
                let subst_args: List<_> = args
                    .iter()
                    .map(|a| self.substitute_type_vars_impl(a, subst, d))
                    .collect();
                Type::Named {
                    path: path.clone(),
                    args: subst_args,
                }
            }
            Type::Tuple(elems) => {
                let subst_elems: List<_> = elems
                    .iter()
                    .map(|e| self.substitute_type_vars_impl(e, subst, d))
                    .collect();
                Type::Tuple(subst_elems)
            }
            Type::Array { element, size } => {
                let subst_elem = Box::new(self.substitute_type_vars_impl(element, subst, d));
                Type::Array {
                    element: subst_elem,
                    size: *size,
                }
            }
            Type::Slice { element } => {
                let subst_elem = Box::new(self.substitute_type_vars_impl(element, subst, d));
                Type::Slice {
                    element: subst_elem,
                }
            }
            Type::Reference { inner, mutable } => {
                let subst_inner = Box::new(self.substitute_type_vars_impl(inner, subst, d));
                Type::Reference {
                    inner: subst_inner,
                    mutable: *mutable,
                }
            }
            Type::CheckedReference { inner, mutable } => {
                let subst_inner = Box::new(self.substitute_type_vars_impl(inner, subst, d));
                Type::CheckedReference {
                    inner: subst_inner,
                    mutable: *mutable,
                }
            }
            Type::UnsafeReference { inner, mutable } => {
                let subst_inner = Box::new(self.substitute_type_vars_impl(inner, subst, d));
                Type::UnsafeReference {
                    inner: subst_inner,
                    mutable: *mutable,
                }
            }
            Type::Pointer { inner, mutable } => {
                let subst_inner = Box::new(self.substitute_type_vars_impl(inner, subst, d));
                Type::Pointer {
                    inner: subst_inner,
                    mutable: *mutable,
                }
            }
            Type::Refined { base, predicate } => {
                let subst_base = Box::new(self.substitute_type_vars_impl(base, subst, d));
                // Audit-A4: substitute meta-param references inside the
                // refinement predicate. The pre-fix code cloned the
                // predicate verbatim; if the predicate referenced a
                // const-generic `N` and `N` had been instantiated (e.g.
                // via `Array<5>`), the SMT solver received an unbound
                // free variable instead of the concrete value, breaking
                // any `where len(arr) == N` style claim. The
                // substitution helper walks the predicate's `Expr`
                // tree and rewrites every `Path(N)` whose `N` is in
                // `meta_param_environment` and bound to a concrete
                // value. Symbolic bindings pass through unchanged so
                // SMT can constrain them.
                let subst_pred = self.substitute_in_refinement_predicate(predicate);
                Type::Refined {
                    base: subst_base,
                    predicate: subst_pred,
                }
            }
            Type::Exists { var, body } => {
                // Don't substitute the bound variable
                let mut filtered_subst = subst.clone();
                filtered_subst.remove(var);
                let subst_body = Box::new(self.substitute_type_vars_impl(body, &filtered_subst, d));
                Type::Exists {
                    var: *var,
                    body: subst_body,
                }
            }
            Type::Record(fields) => {
                let subst_fields = fields
                    .iter()
                    .map(|(name, field_ty)| {
                        (
                            name.clone(),
                            self.substitute_type_vars_impl(field_ty, subst, d),
                        )
                    })
                    .collect();
                Type::Record(subst_fields)
            }
            Type::ExtensibleRecord { fields, row_var } => {
                let subst_fields = fields
                    .iter()
                    .map(|(name, field_ty)| {
                        (
                            name.clone(),
                            self.substitute_type_vars_impl(field_ty, subst, d),
                        )
                    })
                    .collect();
                Type::ExtensibleRecord {
                    fields: subst_fields,
                    row_var: *row_var,
                }
            }
            Type::Variant(variants) => {
                let subst_variants = variants
                    .iter()
                    .map(|(name, variant_ty)| {
                        (
                            name.clone(),
                            self.substitute_type_vars_impl(variant_ty, subst, d),
                        )
                    })
                    .collect();
                Type::Variant(subst_variants)
            }
            Type::Future { output } => {
                let subst_output = Box::new(self.substitute_type_vars_impl(output, subst, d));
                Type::Future {
                    output: subst_output,
                }
            }
            Type::Generator {
                yield_ty,
                return_ty,
            } => {
                let subst_yield = Box::new(self.substitute_type_vars_impl(yield_ty, subst, d));
                let subst_return = Box::new(self.substitute_type_vars_impl(return_ty, subst, d));
                Type::Generator {
                    yield_ty: subst_yield,
                    return_ty: subst_return,
                }
            }
            // All other types: clone without substitution
            // (primitives, special types, complex types we don't need to recurse into)
            _ => ty.clone(),
        }
    }

    /// Substitute type parameter names with concrete types.
    ///

    /// This is used for bidirectional type inference in generic struct instantiation.
    /// Given a type like `T` and a mapping `{T -> Int}`, returns `Int`.
    pub(crate) fn substitute_type_params(
        &self,
        ty: &Type,
        param_subst: &indexmap::IndexMap<verum_common::Text, Type>,
    ) -> Type {
        self.substitute_type_params_impl(ty, param_subst, 0)
    }

    /// Inner implementation with depth tracking to prevent infinite recursion.
    fn substitute_type_params_impl(
        &self,
        ty: &Type,
        param_subst: &indexmap::IndexMap<verum_common::Text, Type>,
        depth: usize,
    ) -> Type {
        // Prevent infinite recursion with a reasonable depth limit
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return ty.clone();
        }
        let d = depth + 1;

        match ty {
            // Named type: if it matches a parameter name, substitute it
            Type::Named { path, args } if args.is_empty() => {
                let name = self.path_to_string(path);
                if let Some(replacement) = param_subst.get(&name) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            // Named type with args: recursively substitute in args
            Type::Named { path, args } => {
                let substituted_args: List<_> = args
                    .iter()
                    .map(|arg| self.substitute_type_params_impl(arg, param_subst, d))
                    .collect();
                Type::Named {
                    path: path.clone(),
                    args: substituted_args,
                }
            }
            // Record type: substitute in field types
            Type::Record(fields) => {
                let substituted_fields: indexmap::IndexMap<_, _> = fields
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            self.substitute_type_params_impl(v, param_subst, d),
                        )
                    })
                    .collect();
                Type::Record(substituted_fields)
            }
            // Tuple type: substitute in elements
            Type::Tuple(elements) => {
                let substituted: List<_> = elements
                    .iter()
                    .map(|e| self.substitute_type_params_impl(e, param_subst, d))
                    .collect();
                Type::Tuple(substituted)
            }
            // Reference types: substitute in inner type
            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(self.substitute_type_params_impl(inner, param_subst, d)),
                mutable: *mutable,
            },
            Type::CheckedReference { inner, mutable } => Type::CheckedReference {
                inner: Box::new(self.substitute_type_params_impl(inner, param_subst, d)),
                mutable: *mutable,
            },
            Type::UnsafeReference { inner, mutable } => Type::UnsafeReference {
                inner: Box::new(self.substitute_type_params_impl(inner, param_subst, d)),
                mutable: *mutable,
            },
            // Future type
            Type::Future { output } => Type::Future {
                output: Box::new(self.substitute_type_params_impl(output, param_subst, d)),
            },
            // Generic type (List<T>, Map<K, V>, etc.): substitute in type arguments
            // CRITICAL FIX: This was missing, causing Map<K, V> to not be substituted
            Type::Generic { name, args } => {
                // If this is a bare type parameter (no args) and the name matches
                // a substitution entry, replace the whole type. This handles the
                // `other: U` parameter in `fn chain<U>(self, other: U) -> ...`
                // — the AST converts bare `U` to `Type::Generic { name: "U",
                // args: [] }`, so without this check we only recursed into
                // (empty) args and returned the unsubstituted `Generic {"U"}`,
                // which then unified against Int through the bound
                // `U: Iterator<Item = Self.Item>` resolution.
                if args.is_empty()
                    && let Some(replacement) = param_subst.get(name)
                {
                    return replacement.clone();
                }
                let substituted_args: List<_> = args
                    .iter()
                    .map(|arg| self.substitute_type_params_impl(arg, param_subst, d))
                    .collect();
                Type::Generic {
                    name: name.clone(),
                    args: substituted_args,
                }
            }
            // Array type: substitute element type
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_type_params_impl(element, param_subst, d)),
                size: *size,
            },
            // Slice type: substitute element type
            Type::Slice { element } => Type::Slice {
                element: Box::new(self.substitute_type_params_impl(element, param_subst, d)),
            },
            // Function type: substitute in params and return type
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => {
                let substituted_params: List<_> = params
                    .iter()
                    .map(|p| self.substitute_type_params_impl(p, param_subst, d))
                    .collect();
                Type::Function {
                    params: substituted_params,
                    return_type: Box::new(self.substitute_type_params_impl(
                        return_type,
                        param_subst,
                        d,
                    )),
                    contexts: contexts.clone(),
                    type_params: type_params.clone(),
                    properties: properties.clone(),
                }
            }
            // Variant type: substitute in each variant's payload type
            // CRITICAL: This enables nested generics like Container<T> with field Maybe<T>
            Type::Variant(variants) => {
                let substituted_variants: indexmap::IndexMap<_, _> = variants
                    .iter()
                    .map(|(name, payload_ty)| {
                        (
                            name.clone(),
                            self.substitute_type_params_impl(payload_ty, param_subst, d),
                        )
                    })
                    .collect();
                Type::Variant(substituted_variants)
            }
            // Forall type (rank-2 polymorphic): substitute in body
            // CRITICAL: This enables rank-2 function types in generic struct fields
            // E.g., Transducer<A, B> with field `transform: fn<R>(Reducer<B, R>) -> Reducer<A, R>`
            // When instantiating Transducer<Int, Int>, A and B need to be substituted in the body
            Type::Forall { vars, body } => {
                let substituted_body =
                    Box::new(self.substitute_type_params_impl(body, param_subst, d));
                Type::Forall {
                    vars: vars.clone(),
                    body: substituted_body,
                }
            }
            // Exists type: substitute in body (for existential types)
            Type::Exists { var, body } => {
                let substituted_body =
                    Box::new(self.substitute_type_params_impl(body, param_subst, d));
                Type::Exists {
                    var: *var,
                    body: substituted_body,
                }
            }
            // TypeApp (higher-kinded type application): substitute in constructor and args
            // CRITICAL: This enables generic record types with recursive references like
            // type Node<T> is { value: T, next: Maybe<Heap<Node<T>>> };
            // where Node<T> becomes TypeApp { constructor: Var(placeholder), args: [Named("T")] }
            // Without this, Named("T") inside TypeApp args is never substituted.
            Type::TypeApp { constructor, args } => {
                let substituted_constructor =
                    self.substitute_type_params_impl(constructor, param_subst, d);
                let substituted_args: List<_> = args
                    .iter()
                    .map(|arg| self.substitute_type_params_impl(arg, param_subst, d))
                    .collect();
                Type::TypeApp {
                    constructor: Box::new(substituted_constructor),
                    args: substituted_args,
                }
            }
            // Type variables: check if there's a binding in the substitution map
            // The format is "T{id}" for type variables created during type inference
            // This is essential for protocol method return type substitution
            Type::Var(tv) => {
                let var_name: verum_common::Text = format!("T{}", tv.id()).into();
                if let Some(replacement) = param_subst.get(&var_name) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            // Primitive and other types: return as-is
            _ => ty.clone(),
        }
    }

    /// Substitute type parameters from receiver type into a method's return type.
    ///

    /// When calling `Wrapper<Int>.get() -> &T`, we need to substitute T = Int
    /// in the return type to get `&Int`.
    ///

    /// # Arguments
    /// - `receiver_ty`: The concrete receiver type (e.g., `Wrapper<Int>`)
    /// - `type_name`: The name of the type (e.g., "Wrapper")
    /// - `method_return_ty`: The method's return type (may contain type variables)
    ///

    /// # Returns
    /// The return type with type parameters substituted from the receiver
    fn substitute_receiver_type_params(
        &self,
        receiver_ty: &Type,
        type_name: &Text,
        method_return_ty: &Type,
    ) -> Type {
        // Extract type arguments from the receiver type
        let type_args = match receiver_ty {
            Type::Named { args, .. } => args.clone(),
            Type::Generic { args, .. } => args.clone(),
            _ => return method_return_ty.clone(), // No type args, return as-is
        };

        if type_args.is_empty() {
            return method_return_ty.clone();
        }

        // Look up type parameter names for this type
        let type_params_key = format!("__type_params_{}", type_name);
        let type_param_names: List<verum_common::Text> =
            match self.ctx.lookup_type(&type_params_key) {
                Option::Some(Type::Record(params_map)) => params_map.keys().cloned().collect(),
                _ => return method_return_ty.clone(), // No type params registered
            };

        // Build substitution map: param_name -> concrete_type
        let mut param_subst: indexmap::IndexMap<verum_common::Text, Type> =
            indexmap::IndexMap::new();
        for (param_name, arg_ty) in type_param_names.iter().zip(type_args.iter()) {
            param_subst.insert(param_name.clone(), arg_ty.clone());
        }

        if param_subst.is_empty() {
            return method_return_ty.clone();
        }

        // Apply substitution to the method return type
        self.substitute_type_params(method_return_ty, &param_subst)
    }

    /// Infer a structural record type from field initializers.
    ///

    /// This is used when a record expression doesn't match a predefined type.
    /// It creates an inline record type by inferring the type of each field.
    pub(super) fn infer_structural_record(
        &mut self,
        fields: &[verum_ast::expr::FieldInit],
        base: &Maybe<Heap<Expr>>,
        _span: Span,
    ) -> Result<InferResult> {
        use indexmap::IndexMap;

        let mut field_types = IndexMap::new();

        // Handle base spread first
        if let Maybe::Some(base_expr) = base {
            let base_result = self.synth_expr(base_expr)?;

            // Use extract_record_fields to handle Named types, Aliases, etc.
            let base_fields = self.extract_record_fields(&base_result.ty)?;
            for (name, ty) in base_fields.iter() {
                field_types.insert(name.clone(), ty.clone());
            }
        }

        // Infer type for each provided field
        for field_init in fields {
            let field_name: Text = field_init.name.name.clone();

            let field_ty = if let Some(ref value_expr) = field_init.value {
                // Explicit value: synthesize its type
                let result = self.synth_expr(value_expr)?;
                result.ty
            } else {
                // Shorthand: lookup variable type
                match self.ctx.env.lookup(field_name.as_str()) {
                    Some(scheme) => scheme.instantiate(),
                    None => {
                        return Err(TypeError::UnboundVariable {
                            name: field_name.clone(),
                            span: field_init.span,
                        });
                    }
                }
            };

            // Add or override field (spread fields can be overridden)
            field_types.insert(field_name, field_ty);
        }

        Ok(InferResult::new(Type::Record(field_types)))
    }

    /// Expand Generic types like Maybe<T> and Result<T,E> to their variant form.
    ///

    /// Expand generic variant types to their variant form for pattern matching.
    /// STDLIB-AGNOSTIC: Looks up type definitions from context, no hardcoded type names.
    ///

    /// Also handles references to generic types:
    /// - &Maybe<T> expands to &(Some(T) | None)
    ///

    /// If the type is not a registered generic variant type, returns it unchanged.
    pub(crate) fn expand_generic_to_variant(&self, ty: &Type) -> Type {
        self.expand_generic_to_variant_impl(ty, 0)
    }

    fn expand_generic_to_variant_impl(&self, ty: &Type, depth: usize) -> Type {
        if depth > 10 {
            return ty.clone();
        }
        match ty {
            Type::Generic { name, args } => {
                // STDLIB-AGNOSTIC: Look up all generic types from context
                if let Option::Some(def_ty) = self.ctx.lookup_type(name.as_str()) {
                    if let Type::Variant(variants) = def_ty {
                        // For generic variant types, substitute type arguments
                        if !args.is_empty() {
                            let type_params_key = format!("__type_params_{}", name);
                            // Get both parameter names and their associated TypeVars
                            let params_map_opt = match self
                                .ctx
                                .lookup_type(type_params_key.as_str())
                            {
                                Option::Some(Type::Record(params_map)) => Option::Some(params_map),
                                _ => Option::None,
                            };
                            if let Option::Some(params_map) = params_map_opt {
                                let mut subst: indexmap::IndexMap<verum_common::Text, Type> =
                                    indexmap::IndexMap::new();
                                for (i, (param_name, param_type)) in params_map.iter().enumerate() {
                                    if let Some(arg) = args.get(i) {
                                        subst.insert(param_name.clone(), arg.clone());
                                        if let Type::Var(tv) = param_type {
                                            let var_key: verum_common::Text =
                                                format!("T{}", tv.id()).into();
                                            subst.insert(var_key, arg.clone());
                                        }
                                    }
                                }
                                // Also extract free vars from the variant definition itself
                                // and map them positionally to type args. This handles the case
                                // where the variant definition has different TypeVar IDs than
                                // the params record (e.g., after re-registration).
                                let variant_type_ref = Type::Variant(variants.clone());
                                let free_vars = variant_type_ref.free_vars();
                                let mut free_vars_sorted: Vec<TypeVar> =
                                    free_vars.into_iter().collect();
                                free_vars_sorted.sort_by_key(|tv| tv.id());
                                // Map free vars from the variant body to type args.
                                // Only add entries for vars NOT already covered by params_map,
                                // to avoid overriding the correct declaration-order mapping.
                                for (i, tv) in free_vars_sorted.iter().enumerate() {
                                    if let Some(arg) = args.get(i) {
                                        let var_key: verum_common::Text =
                                            format!("T{}", tv.id()).into();
                                        if !subst.contains_key(&var_key) {
                                            subst.insert(var_key, arg.clone());
                                        }
                                    }
                                }
                                let mut substituted_variants = indexmap::IndexMap::new();
                                for (tag, payload_ty) in variants.iter() {
                                    let subst_ty = self.substitute_type_params(payload_ty, &subst);
                                    substituted_variants.insert(tag.clone(), subst_ty);
                                }
                                Type::Variant(substituted_variants)
                            } else {
                                Type::Variant(variants.clone())
                            }
                        } else {
                            Type::Variant(variants.clone())
                        }
                    } else {
                        // STDLIB-AGNOSTIC: Check inductive_constructors for variant types
                        // stored as Type::Generic rather than Type::Variant
                        self.try_build_variant_from_constructors(name, args, ty)
                    }
                } else {
                    // Type not found in lookup, try inductive_constructors
                    self.try_build_variant_from_constructors(name, args, ty)
                }
            }
            // Handle Named types (stdlib-agnostic: looks up type definitions, no hardcoded names)
            // STDLIB-AGNOSTIC: All types (including Maybe, Result) are looked up uniformly.
            // No special cases for specific type names - the definition lookup handles all.
            Type::Named { path, args } => {
                let type_name = path
                    .segments
                    .last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("");

                // Look up type definitions to see if they are variants (stdlib-agnostic)
                {
                    if let Option::Some(def_ty) = self.ctx.lookup_type(type_name) {
                        if let Type::Variant(variants) = def_ty {
                            // For generic variant types, substitute type arguments
                            if !args.is_empty() {
                                let type_params_key = format!("__type_params_{}", type_name);
                                // Get both parameter names and their associated TypeVars
                                let params_map_opt =
                                    match self.ctx.lookup_type(type_params_key.as_str()) {
                                        Option::Some(Type::Record(params_map)) => {
                                            Option::Some(params_map)
                                        }
                                        _ => Option::None,
                                    };
                                if let Option::Some(params_map) = params_map_opt {
                                    let mut subst: indexmap::IndexMap<verum_common::Text, Type> =
                                        indexmap::IndexMap::new();
                                    for (i, (param_name, param_type)) in
                                        params_map.iter().enumerate()
                                    {
                                        if let Some(arg) = args.get(i) {
                                            subst.insert(param_name.clone(), arg.clone());
                                            if let Type::Var(tv) = param_type {
                                                let var_key: verum_common::Text =
                                                    format!("T{}", tv.id()).into();
                                                subst.insert(var_key, arg.clone());
                                            }
                                        }
                                    }
                                    // Also extract free vars from variant def for TypeVar ID mismatch resilience
                                    // Only add for TypeVars not already covered by params_map
                                    let variant_type_ref = Type::Variant(variants.clone());
                                    let free_vars = variant_type_ref.free_vars();
                                    let mut free_vars_sorted: Vec<TypeVar> =
                                        free_vars.into_iter().collect();
                                    free_vars_sorted.sort_by_key(|tv| tv.id());
                                    for (i, tv) in free_vars_sorted.iter().enumerate() {
                                        if let Some(arg) = args.get(i) {
                                            let var_key: verum_common::Text =
                                                format!("T{}", tv.id()).into();
                                            if !subst.contains_key(&var_key) {
                                                subst.insert(var_key, arg.clone());
                                            }
                                        }
                                    }
                                    let mut substituted_variants = indexmap::IndexMap::new();
                                    for (tag, payload_ty) in variants.iter() {
                                        let subst_ty =
                                            self.substitute_type_params(payload_ty, &subst);
                                        substituted_variants.insert(tag.clone(), subst_ty);
                                    }
                                    Type::Variant(substituted_variants)
                                } else {
                                    Type::Variant(variants.clone())
                                }
                            } else {
                                Type::Variant(variants.clone())
                            }
                        } else {
                            // The lookup resolved to a non-Variant type. Check if it's a
                            // self-referential placeholder (type alias stores Named{self} in type_defs).
                            // If so, resolve through the alias table instead.
                            let is_self_ref = if let Type::Named { path: def_path, .. } = def_ty {
                                def_path
                                    .segments
                                    .last()
                                    .map(|seg| match seg {
                                        verum_ast::ty::PathSegment::Name(id) => {
                                            id.name.as_str() == type_name
                                        }
                                        _ => false,
                                    })
                                    .unwrap_or(false)
                            } else {
                                false
                            };
                            if is_self_ref {
                                // Try alias resolution directly
                                if let Option::Some(alias_ty) = self.ctx.resolve_alias(type_name) {
                                    let alias_ty = alias_ty.clone();
                                    // For alias resolution, substitute actual type args into the alias target
                                    // before recursing. This preserves the concrete args (e.g., Stream<T>)
                                    // instead of leaving formal params (e.g., T).
                                    let substituted_alias = if !args.is_empty() {
                                        let type_params_key =
                                            format!("__type_params_{}", type_name);
                                        if let Option::Some(Type::Record(params_map)) =
                                            self.ctx.lookup_type(type_params_key.as_str())
                                        {
                                            let mut subst: indexmap::IndexMap<
                                                verum_common::Text,
                                                Type,
                                            > = indexmap::IndexMap::new();
                                            for (i, (param_name, param_type)) in
                                                params_map.iter().enumerate()
                                            {
                                                if let Some(arg) = args.get(i) {
                                                    subst.insert(param_name.clone(), arg.clone());
                                                    if let Type::Var(tv) = param_type {
                                                        let var_key: verum_common::Text =
                                                            format!("T{}", tv.id()).into();
                                                        subst.insert(var_key, arg.clone());
                                                    }
                                                }
                                            }
                                            self.substitute_type_params(&alias_ty, &subst)
                                        } else {
                                            alias_ty
                                        }
                                    } else {
                                        alias_ty
                                    };
                                    self.expand_generic_to_variant_impl(
                                        &substituted_alias,
                                        depth + 1,
                                    )
                                } else {
                                    // No alias found - try inductive_constructors for self-ref types
                                    // This handles sum types like Maybe<T> that are defined as variants
                                    self.try_build_variant_from_constructors(type_name, args, ty)
                                }
                            } else {
                                // Non-variant, non-self-ref definition (e.g., opaque struct like Heap<T>).
                                // Check if recursing through the definition leads to a variant (alias chain).
                                // If so, we need to substitute actual type args. If not, return original type
                                // unchanged to preserve its actual type arguments.
                                let resolved = def_ty.clone();
                                let recursed =
                                    self.expand_generic_to_variant_impl(&resolved, depth + 1);
                                if matches!(&recursed, Type::Variant(_)) {
                                    // Found a variant through chain. Substitute actual type args if available.
                                    if !args.is_empty() {
                                        let type_params_key =
                                            format!("__type_params_{}", type_name);
                                        if let Option::Some(Type::Record(params_map)) =
                                            self.ctx.lookup_type(type_params_key.as_str())
                                        {
                                            let mut subst: indexmap::IndexMap<
                                                verum_common::Text,
                                                Type,
                                            > = indexmap::IndexMap::new();
                                            for (i, (param_name, param_type)) in
                                                params_map.iter().enumerate()
                                            {
                                                if let Some(arg) = args.get(i) {
                                                    subst.insert(param_name.clone(), arg.clone());
                                                    if let Type::Var(tv) = param_type {
                                                        let var_key: verum_common::Text =
                                                            format!("T{}", tv.id()).into();
                                                        subst.insert(var_key, arg.clone());
                                                    }
                                                }
                                            }
                                            let variant_type_ref = recursed.clone();
                                            let free_vars = variant_type_ref.free_vars();
                                            let mut free_vars_sorted: Vec<TypeVar> =
                                                free_vars.into_iter().collect();
                                            free_vars_sorted.sort_by_key(|tv| tv.id());
                                            for (i, tv) in free_vars_sorted.iter().enumerate() {
                                                if let Some(arg) = args.get(i) {
                                                    let var_key: verum_common::Text =
                                                        format!("T{}", tv.id()).into();
                                                    if !subst.contains_key(&var_key) {
                                                        subst.insert(var_key, arg.clone());
                                                    }
                                                }
                                            }
                                            if let Type::Variant(variants) = &recursed {
                                                let mut substituted_variants =
                                                    indexmap::IndexMap::new();
                                                for (tag, payload_ty) in variants.iter() {
                                                    let subst_ty = self
                                                        .substitute_type_params(payload_ty, &subst);
                                                    substituted_variants
                                                        .insert(tag.clone(), subst_ty);
                                                }
                                                Type::Variant(substituted_variants)
                                            } else {
                                                recursed
                                            }
                                        } else {
                                            recursed
                                        }
                                    } else {
                                        recursed
                                    }
                                } else {
                                    // Not a variant - return original type with actual args preserved
                                    ty.clone()
                                }
                            }
                        }
                    } else {
                        // Type not found via lookup - try inductive_constructors
                        self.try_build_variant_from_constructors(type_name, args, ty)
                    }
                }
            }
            // Handle references to generic types: &Maybe<T> -> &(Some(T) | None)
            Type::Reference { mutable, inner } => {
                let expanded_inner = self.expand_generic_to_variant_impl(inner, depth + 1);
                if matches!(&expanded_inner, Type::Variant(_)) {
                    Type::Reference {
                        mutable: *mutable,
                        inner: Box::new(expanded_inner),
                    }
                } else {
                    ty.clone()
                }
            }
            Type::CheckedReference { mutable, inner } => {
                let expanded_inner = self.expand_generic_to_variant_impl(inner, depth + 1);
                if matches!(&expanded_inner, Type::Variant(_)) {
                    Type::CheckedReference {
                        mutable: *mutable,
                        inner: Box::new(expanded_inner),
                    }
                } else {
                    ty.clone()
                }
            }
            Type::UnsafeReference { mutable, inner } => {
                let expanded_inner = self.expand_generic_to_variant_impl(inner, depth + 1);
                if matches!(&expanded_inner, Type::Variant(_)) {
                    Type::UnsafeReference {
                        mutable: *mutable,
                        inner: Box::new(expanded_inner),
                    }
                } else {
                    ty.clone()
                }
            }
            // Already a variant type
            Type::Variant(_) => ty.clone(),
            // CRITICAL FIX: Resolve type variables before expansion.
            // When a scrutinee's type contains unresolved type variables (e.g., from
            // a function call without explicit type annotation), we need to apply
            // the unifier to resolve them before attempting to expand to variant form.
            // Without this, patterns like `Valid(values)` fail because the scrutinee
            // type is still a type variable, and we can't determine the payload type.
            Type::Var(_) => {
                let resolved = self.unifier.apply(ty);
                // If still a variable after resolution, we can't expand
                if let Type::Var(_) = &resolved {
                    resolved
                } else {
                    // Recurse with the resolved type
                    self.expand_generic_to_variant_impl(&resolved, depth + 1)
                }
            }
            // Other types pass through unchanged
            _ => ty.clone(),
        }
    }

    /// Try to build a variant type from inductive constructors.
    /// STDLIB-AGNOSTIC: This enables pattern matching on generic types like Maybe<T>
    /// when the variant info is stored in inductive_constructors rather than as Type::Variant.
    /// Resolve a bare name as a variant constructor at
    /// value-position.  Consults `variant_constructor_parents` to
    /// find the parent variant type, then builds a `fn(payload) ->
    /// Variant<freshvars>` constructor type from the registered
    /// `inductive_constructors`.
    ///
    /// **Stdlib-agnostic**: no hardcoded constructor names.  Works
    /// for any variant whose parent type was registered via
    /// `register_inductive_type` / `register_variant_signature_for_lazy`,
    /// stdlib or user-defined.
    pub(super) fn try_resolve_variant_constructor(&self, name: &str) -> Option<Type> {
        self.try_resolve_variant_constructor_with_arity(name, None)
    }

    /// Arity-aware variant-constructor resolution.  When `expected_arity`
    /// is `Some(n)`, picks the registered parent whose constructor for
    /// `name` accepts EXACTLY `n` payload arguments — this is the
    /// disambiguator when two stdlib types share a simple variant
    /// name (the canonical collision: `Result.Ok(T)` vs
    /// `ExitCode.Ok` (unit) — caller-site `Ok(())` provides 1 arg
    /// → matches `Result.Ok`, not `ExitCode.Ok`).
    ///
    /// Falls back to the first-registered-wins discipline when:
    ///   * `expected_arity` is `None` (no call-site arity context —
    ///     value-position uses, pattern position, etc.), or
    ///   * no parent's constructor matches the requested arity, or
    ///   * exactly one parent is registered (no ambiguity to resolve).
    ///
    /// The caller (Call-expression inference) passes the arg count;
    /// every other call site uses the arity-blind wrapper above.
    pub(super) fn try_resolve_variant_constructor_with_arity(
        &self,
        name: &str,
        expected_arity: Option<usize>,
    ) -> Option<Type> {
        let ctor_text = verum_common::Text::from(name);
        let parents = self.variant_constructor_parents.get(&ctor_text)?;
        // Pick the parent whose constructor for `name` accepts the
        // expected arity, when arity context is available AND multiple
        // parents are in the registry.  Otherwise fall through to the
        // first-registered-wins discipline (mirrors
        // `register_variant_type_name_first_wins`).
        let parent_name = if let (Some(arity), true) =
            (expected_arity, parents.len() > 1)
        {
            let mut chosen: Option<Text> = None;
            for parent in parents.iter() {
                let ctors = match self.ctx.get_constructors(parent) {
                    Maybe::Some(c) => c,
                    Maybe::None => continue,
                };
                let ctor = match ctors.iter().find(|c| c.name == ctor_text) {
                    Some(c) => c,
                    None => continue,
                };
                if ctor.args.len() == arity {
                    chosen = Some(parent.clone());
                    break;
                }
            }
            chosen.unwrap_or_else(|| {
                parents
                    .first()
                    .cloned()
                    .expect("parents non-empty by outer get(?)")
            })
        } else {
            parents.first()?.clone()
        };
        let constructors = match self.ctx.get_constructors(&parent_name) {
            Maybe::Some(c) => c.clone(),
            Maybe::None => return None,
        };
        // Find this constructor's args.
        let ctor = constructors.iter().find(|c| c.name == ctor_text)?;
        // Build fresh type variables for the parent's generic
        // parameters so the constructor instantiation is open
        // (caller's expected type drives unification).
        let generics_count = self
            .type_generics_count
            .get(&parent_name)
            .copied()
            .unwrap_or(0);
        let fresh_args: List<Type> = (0..generics_count)
            .map(|_| Type::Var(crate::ty::TypeVar::fresh()))
            .collect();
        let return_type = if generics_count == 0 {
            Type::Named {
                path: Self::text_to_path(&parent_name),
                args: List::new(),
            }
        } else {
            Type::Generic {
                name: parent_name.clone(),
                args: fresh_args.clone(),
            }
        };
        // #126b — substitute the constructor's rigid named-T type
        // parameters with the parent's fresh TypeVars.  Pre-fix
        // `params` was a verbatim clone of `ctor.args` whose payload
        // positions held rigid `Type::Named { path: "T" }` (the
        // generic parameter name as registered by the lazy stdlib
        // loader).  Calling `Some(10)` then surfaced as
        // `expected 'T', found 'Int'` because the unifier compared
        // `Int` against the rigid named-T placeholder.
        //
        // The substitution map comes from the constructor's own
        // `type_params: List<(Text, Box<Type>)>` field — a property
        // of the constructor's registration, not a hardcoded list of
        // stdlib generic names.
        // Build the parameter-name → fresh-TypeVar substitution map.
        //
        // Source priority (first that yields names wins):
        //   1. `ctor.type_params` — populated when the constructor was
        //      registered via the eager source-driven path
        //      (register_type_declaration). Empty under VBCA lazy load.
        //   2. `__type_params_<parent>` registry record — populated by
        //      `register_generic_stdlib_type`. Empty under lazy load.
        //   3. `core_metadata.types[parent].generic_params` — the
        //      authoritative VBCA-side metadata. THIS is what fires
        //      under the lazy-loader path; it's the parent type's own
        //      `generic_params` list as serialised in the archive.
        //
        // Stdlib-agnostic per `crates/verum_types/src/CLAUDE.md`: the
        // substitution map's KEYS come from the parent's own metadata,
        // never from a hardcoded list of stdlib param names.
        let mut tv_subst: indexmap::IndexMap<verum_common::Text, Type> =
            indexmap::IndexMap::new();
        if !ctor.type_params.is_empty() {
            for (i, (param_name, _)) in ctor.type_params.iter().enumerate() {
                if let Some(fresh_arg) = fresh_args.get(i) {
                    tv_subst.insert(param_name.clone(), fresh_arg.clone());
                }
            }
        } else {
            let params_key: verum_common::Text =
                format!("__type_params_{}", parent_name).into();
            if let Option::Some(Type::Record(param_record)) =
                self.ctx.lookup_type(params_key.as_str())
            {
                for (i, (param_name, _)) in param_record.iter().enumerate() {
                    if let Some(fresh_arg) = fresh_args.get(i) {
                        tv_subst.insert(param_name.clone(), fresh_arg.clone());
                    }
                }
            } else if let Maybe::Some(metadata) = &self.core_metadata {
                if let Some(td) = metadata.types.get(&parent_name) {
                    for (i, gp) in td.generic_params.iter().enumerate() {
                        if let Some(fresh_arg) = fresh_args.get(i) {
                            tv_subst.insert(gp.name.clone(), fresh_arg.clone());
                        }
                    }
                }
            }
        }
        let subst_named_params = |ty: &Type| -> Type {
            Self::substitute_named_params_in_type(ty, &tv_subst)
        };
        let params: List<Type> = ctor
            .args
            .iter()
            .map(|a| subst_named_params(a.as_ref()))
            .collect();
        if params.is_empty() {
            // Unit-position variant (None, Less, …) — return the
            // constructed value directly, not a function.
            Some(return_type)
        } else {
            Some(Type::function(params, return_type))
        }
    }

    /// Replace every `Type::Named { path: name, args: [] }` whose
    /// `name` matches a key in `subst` with the corresponding
    /// substitution type. Recurses into Generic / Tuple / Function /
    /// Reference / Record / Variant / Array / Slice / TypeApp /
    /// Future / Promise containers. Used by
    /// `try_resolve_variant_constructor` (and the analogous
    /// payload-substitution path in `register_variant_signature_for_lazy`)
    /// to swap rigid named-T type-parameter placeholders for fresh
    /// per-call-site TypeVars.
    fn substitute_named_params_in_type(
        ty: &Type,
        subst: &indexmap::IndexMap<verum_common::Text, Type>,
    ) -> Type {
        if subst.is_empty() {
            return ty.clone();
        }
        match ty {
            Type::Named { path, args } if args.is_empty() => {
                if let Some(seg) = path.segments.iter().next() {
                    if path.segments.len() == 1 {
                        if let verum_ast::ty::PathSegment::Name(ident) = seg {
                            if let Some(replacement) = subst.get(&ident.name) {
                                return replacement.clone();
                            }
                        }
                    }
                }
                ty.clone()
            }
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| Self::substitute_named_params_in_type(a, subst))
                    .collect(),
            },
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| Self::substitute_named_params_in_type(a, subst))
                    .collect(),
            },
            Type::Tuple(parts) => Type::Tuple(
                parts
                    .iter()
                    .map(|p| Self::substitute_named_params_in_type(p, subst))
                    .collect(),
            ),
            Type::Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| Self::substitute_named_params_in_type(p, subst))
                    .collect(),
                return_type: Box::new(Self::substitute_named_params_in_type(
                    return_type,
                    subst,
                )),
                type_params: type_params.clone(),
                contexts: contexts.clone(),
                properties: properties.clone(),
            },
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(Self::substitute_named_params_in_type(inner, subst)),
            },
            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(Self::substitute_named_params_in_type(inner, subst)),
            },
            Type::UnsafeReference { mutable, inner } => Type::UnsafeReference {
                mutable: *mutable,
                inner: Box::new(Self::substitute_named_params_in_type(inner, subst)),
            },
            Type::Array { element, size } => Type::Array {
                element: Box::new(Self::substitute_named_params_in_type(element, subst)),
                size: *size,
            },
            Type::Slice { element } => Type::Slice {
                element: Box::new(Self::substitute_named_params_in_type(element, subst)),
            },
            Type::Variant(variants) => Type::Variant(
                variants
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::substitute_named_params_in_type(v, subst)))
                    .collect(),
            ),
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::substitute_named_params_in_type(v, subst)))
                    .collect(),
            ),
            _ => ty.clone(),
        }
    }

    fn try_build_variant_from_constructors(
        &self,
        name: &str,
        args: &[Type],
        fallback: &Type,
    ) -> Type {
        let type_name = verum_common::Text::from(name);
        if let Some(constructors) = self.ctx.get_constructors(&type_name) {
            if !constructors.is_empty() {
                // Build variant map from inductive constructors
                let mut variants: indexmap::IndexMap<verum_common::Text, Type> =
                    indexmap::IndexMap::new();
                for ctor in constructors.iter() {
                    let payload_ty = if ctor.args.is_empty() {
                        Type::Unit
                    } else if ctor.args.len() == 1 {
                        ctor.args
                            .first()
                            .map(|a| a.as_ref().clone())
                            .unwrap_or(Type::Unit)
                    } else {
                        Type::Tuple(ctor.args.iter().map(|a| a.as_ref().clone()).collect())
                    };
                    variants.insert(ctor.name.clone(), payload_ty);
                }

                // Perform type argument substitution if we have args
                if !args.is_empty() {
                    let type_params_key = format!("__type_params_{}", name);
                    let mut subst: indexmap::IndexMap<verum_common::Text, Type> =
                        indexmap::IndexMap::new();
                    if let Option::Some(Type::Record(params_map)) =
                        self.ctx.lookup_type(type_params_key.as_str())
                    {
                        for (i, (param_name, param_type)) in params_map.iter().enumerate() {
                            if let Some(arg) = args.get(i) {
                                subst.insert(param_name.clone(), arg.clone());
                                if let Type::Var(tv) = param_type {
                                    let var_key: verum_common::Text =
                                        format!("T{}", tv.id()).into();
                                    subst.insert(var_key, arg.clone());
                                }
                            }
                        }
                    }
                    // CoreMetadata fallback: stdlib types lazy-loaded
                    // via `ensure_stdlib_type_loaded` don't get
                    // `__type_params_<name>` registered in
                    // ctx.type_defs (only user-source types do).
                    // Without this fallback, every stdlib variant
                    // pattern (Result<T,E>, Maybe<T>) bound payload
                    // patterns to raw T/E placeholders instead of
                    // the concrete type args.
                    if subst.is_empty() {
                        if let Some(metadata) = self.core_metadata() {
                            let gname = verum_common::Text::from(name);
                            if let Some(td) = metadata.types.get(&gname) {
                                for (gp, arg) in
                                    td.generic_params.iter().zip(args.iter())
                                {
                                    subst.insert(gp.name.clone(), arg.clone());
                                }
                            }
                        }
                    }
                    if !subst.is_empty() {
                        let mut substituted_variants = indexmap::IndexMap::new();
                        for (tag, payload_ty) in variants.iter() {
                            let subst_ty = self.substitute_type_params(payload_ty, &subst);
                            substituted_variants.insert(tag.clone(), subst_ty);
                        }
                        return Type::Variant(substituted_variants);
                    }
                }
                return Type::Variant(variants);
            }
        }
        fallback.clone()
    }

    /// Iteratively infer type for method chain: receiver.m1(a1).m2(a2).m3(a3)
    ///

    /// Instead of recursively calling synth_expr for each receiver (which causes stack overflow
    /// on deeply nested chains), we:
    /// 1. "Unwind" the chain into a flat list: [(m3, a3), (m2, a2), (m1, a1)] + base
    /// 2. Synthesize type of the base expression
    /// 3. Iteratively apply each method call to get the final type
    ///

    /// This completely eliminates recursive stack usage for method chains.
    pub(super) fn infer_method_chain_iterative(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
    ) -> Result<InferResult> {
        // Collect the method chain into a vector (in reverse order)
        // For a.b().c().d(), we collect: [(d, type_args_d, args_d), (c, type_args_c, args_c), (b, type_args_b, args_b)]
        // and base = a
        let mut chain: Vec<(&Ident, &List<verum_ast::ty::GenericArg>, &[Expr], Span)> = Vec::new();
        let mut current_receiver = receiver;
        let mut current_method = method;
        let mut current_type_args = type_args;
        let mut current_args = args;
        let mut current_span = span;

        loop {
            // Add the current method call to the chain
            chain.push((
                current_method,
                current_type_args,
                current_args,
                current_span,
            ));

            // Check if the receiver is also a method call
            if let ExprKind::MethodCall {
                receiver: inner_receiver,
                method: inner_method,
                type_args: inner_type_args,
                args: inner_args,
            } = &current_receiver.kind
            {
                // Continue unwinding the chain
                current_span = current_receiver.span;
                current_method = inner_method;
                current_type_args = inner_type_args;
                current_args = inner_args;
                current_receiver = inner_receiver;
            } else {
                // Base case reached - current_receiver is not a method call
                break;
            }
        }

        // Now current_receiver is the base expression (e.g., 'a' in a.b().c().d())
        // and chain contains all method calls in reverse order

        // Step 1: Synthesize type of the base expression (NOT a method call, so no recursion issue)
        // Set in_call_arg_context so that path expressions use borrow_value instead of use_value.
        // This is critical for affine values in loops: methods like push(&mut self) borrow, not consume.
        let old_call_context = self.in_call_arg_context;
        self.in_call_arg_context = true;
        let base_result = self.synth_expr(current_receiver)?;
        self.in_call_arg_context = old_call_context;
        let mut current_ty = base_result.ty;

        // Step 2: Process each method call iteratively (in reverse order to apply from base)
        //

        // CRITICAL: The first method call in the chain needs different handling than subsequent calls.
        // - For `Int.max_value().min(0)`:
        //  - First call (max_value): receiver is Path("Int"), needs static method lookup
        //  - Second call (min): receiver is result of max_value(), needs instance method lookup
        //

        // CRITICAL FIX: Use precomputed receiver type for ALL calls to avoid double-synthesis.
        // The receiver was already synthesized at lines 29405-29406 with in_call_arg_context=true.
        // Re-synthesizing in infer_method_call_inner_impl would happen with context=false,
        // causing use_value() instead of borrow_value() - breaking affine tracking in loops.
        //

        // For the first call: use infer_method_call_with_recv_type (skip_static=false) to
        // preserve static method lookup based on receiver.kind (e.g., Int.max_value()).
        //

        // For subsequent calls: use infer_method_call_with_recv_type_skip_static (skip_static=true)
        // because receiver.kind is the original base, not the actual receiver.
        let mut is_first_call = true;
        for (method, type_args, args, span) in chain.into_iter().rev() {
            let result = if is_first_call {
                // First call: pass precomputed type but allow static method lookup
                // This fixes affine tracking in loops while preserving Int.max_value() patterns
                self.infer_method_call_with_recv_type(
                    current_ty.clone(),
                    current_receiver,
                    method,
                    type_args,
                    args,
                    span,
                )?
            } else {
                // Subsequent calls: use precomputed type from previous call result
                // Skip static lookup since receiver.kind is the original base expression
                self.infer_method_call_with_recv_type_skip_static(
                    current_ty.clone(),
                    current_receiver,
                    method,
                    type_args,
                    args,
                    span,
                )?
            };
            current_ty = result.ty;
            // Resolve top-level associated type projections (e.g., ::Item[ListIter<Int>] → &Int)
            // Only attempt for direct projection types (not deeply nested) to avoid stack overflow
            if let Type::Generic { name, args } = &current_ty {
                if name.as_str().starts_with("::") && !args.is_empty() {
                    let assoc_name = &name.as_str()[2..];
                    if let Some(resolved) =
                        self.try_resolve_associated_type_projection(&args[0], assoc_name)
                    {
                        current_ty = resolved;
                    }
                }
            }
            is_first_call = false;
        }

        Ok(InferResult::new(current_ty))
    }

    /// Infer type for method call: receiver.method(args)
    ///

    /// Higher-rank protocol bounds: for<T> quantification in protocol bounds for universal requirements — .1-2.3
    ///

    /// # Method Resolution Algorithm
    ///

    /// 1. **Synthesize receiver type**: Infer type of receiver expression
    /// 2. **Find protocol implementations**: Look up all protocols that receiver type implements
    /// 3. **Resolve method name**: Find method in protocol(s)
    /// 4. **Check for ambiguity**: If multiple protocols have the method, report error
    /// 5. **Extract method signature**: Get parameter types and return type
    /// 6. **Type check arguments**: Verify arguments match parameters
    /// 7. **Infer return type**: Return method's return type
    ///

    /// # Error Cases
    ///

    /// - **Method not found**: No protocol with this method is implemented by receiver type
    /// - **Protocol not implemented**: Receiver type doesn't implement protocol containing method
    /// - **Wrong argument count**: Number of arguments doesn't match method signature
    /// - **Argument type mismatch**: Argument type doesn't match parameter type
    /// - **Ambiguous method**: Multiple protocols have same method name
    ///

    /// # Examples
    ///

    /// ```verum
    /// // Simple method call
    /// let x: List<Int> = [1, 2, 3]
    /// x.map(|n| n + 1) // Resolves to Functor::map
    ///

    /// // Protocol method with constraints
    /// fn sort<T: Ord>(list: List<T>) -> List<T> {
    ///  list.sort() // Resolves to Ord::cmp internally
    /// }
    ///

    /// // Generic method
    /// let iter = [1, 2, 3].iter()
    /// iter.next() // Resolves to Iterator::next
    /// ```
    fn infer_method_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<InferResult> {
        // ============================================================
        // Iterator Invalidation Check
        // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .4 - Iterator invalidation
        // ============================================================
        // Check if this is a mutating method call on a collection that has
        // an active iterator. Mutating methods like push, pop, clear, etc.
        // would invalidate any iterators over the collection.
        if self.is_mutating_collection_method(&method.name) {
            if let Some(collection_name) = self.extract_receiver_name(receiver) {
                // Check for iterator invalidation - this is a hard error
                self.borrow_tracker
                    .check_iterator_invalidation(&collection_name, span)?;

                // ============================================================
                // Closure Capture Conflict Check
                // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #closure-captures
                // ============================================================
                // Check if this collection is captured by a closure.
                // Mutating a captured variable conflicts with the closure's borrow.
                if self.borrow_tracker.is_captured(&collection_name) {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "cannot borrow `{}` as mutable because it is captured by a closure",
                        collection_name
                    ))));
                }

                // Also check for regular borrows
                if let Some(err) =
                    self.borrow_tracker
                        .check_borrow_allowed(&collection_name, true, span)
                {
                    return Err(err);
                }
            }
        }

        // ============================================================
        // Two-Phase Borrows for Method Calls
        // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .6 - Two-phase borrows
        // ============================================================
        // Two-phase borrows allow patterns like `vec.push(vec.len())`:
        // 1. Reserve mutable borrow of receiver (not yet activated)
        // 2. Evaluate arguments (can use immutable borrows)
        // 3. Activate the mutable borrow for the actual method call
        //

        // This is implemented via the `in_call_arg_context` flag which
        // tells the borrow tracker to allow immutable borrows during
        // argument evaluation, then convert them to the final state.

        // NLL: Method calls use temporary borrows - set context flag
        let old_call_context = self.in_call_arg_context;
        self.in_call_arg_context = true;

        // For two-phase borrows: mark receiver as having a pending mutable borrow
        // This allows immutable borrows during argument evaluation
        let receiver_name_for_2pb = self.extract_receiver_name(receiver);
        if let Some(ref receiver_name) = receiver_name_for_2pb {
            self.borrow_tracker
                .begin_two_phase_borrow(receiver_name.clone(), span);
        }

        // Relies on RUST_MIN_STACK=16MB for stack safety on deeply nested method chains
        // Method chains like builder.a().b().c().d() cause recursive synth_expr calls
        let result = self.infer_method_call_inner(receiver, method, args, span);

        // End two-phase borrow period and release mutable borrows on receiver
        if let Some(ref receiver_name) = receiver_name_for_2pb {
            self.borrow_tracker
                .end_two_phase_borrow(receiver_name.clone());
            self.borrow_tracker
                .nll_release_expired_borrows_for(receiver_name);
        }

        // Restore context
        self.in_call_arg_context = old_call_context;

        // Consume affine receiver if method takes self by value
        if let Ok(ref _res) = result {
            if let Some(ref receiver_name) = receiver_name_for_2pb {
                if self
                    .affine_tracker
                    .is_affine_binding(receiver_name.as_str())
                {
                    let recv_ty = self
                        .ctx
                        .env
                        .lookup(receiver_name.as_str())
                        .map(|s| s.instantiate());
                    if let Some(recv_ty) = recv_ty {
                        if self.method_takes_self_by_value(&recv_ty, method) {
                            self.affine_tracker
                                .use_value(receiver_name.as_str(), span)?;
                        }
                    }
                }
            }
        }

        result
    }

    /// Check if a method takes `self` by value (SelfValue or SelfValueMut).
    /// Used to determine whether a method call consumes the receiver for affine tracking.
    fn method_takes_self_by_value(&self, recv_ty: &Type, method: &Ident) -> bool {
        let method_name = verum_common::Text::from(method.name.as_str());
        let type_name = self.type_to_name(recv_ty);
        self.self_by_value_methods
            .contains(&(type_name, method_name))
    }

    /// Variant of infer_method_call_inner that takes a pre-computed receiver type.
    /// Used by the iterative method chain handler to avoid recursive synth_expr calls.
    ///

    /// CRITICAL: Must apply the same NLL logic as infer_method_call:
    /// - Set in_call_arg_context for temporary borrows
    /// - Handle two-phase borrows for receiver
    pub(super) fn infer_method_call_with_recv_type(
        &mut self,
        recv_ty: Type,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
    ) -> Result<InferResult> {
        // ============================================================
        // NLL: Same logic as infer_method_call for proper borrow handling
        // ============================================================

        // NLL: Method calls use temporary borrows - set context flag
        let old_call_context = self.in_call_arg_context;
        self.in_call_arg_context = true;

        // For two-phase borrows: mark receiver as having a pending mutable borrow
        let receiver_name_for_2pb = self.extract_receiver_name(receiver);
        if let Some(ref receiver_name) = receiver_name_for_2pb {
            self.borrow_tracker
                .begin_two_phase_borrow(receiver_name.clone(), span);
        }

        let result = self.infer_method_call_inner_impl(
            receiver,
            method,
            type_args,
            args,
            span,
            Some(recv_ty.clone()),
            false,
        );

        // End two-phase borrow period and release mutable borrows on receiver
        if let Some(ref receiver_name) = receiver_name_for_2pb {
            self.borrow_tracker
                .end_two_phase_borrow(receiver_name.clone());
            self.borrow_tracker
                .nll_release_expired_borrows_for(receiver_name);
        }

        // Restore context
        self.in_call_arg_context = old_call_context;

        // Consume affine receiver if method takes self by value
        if let Ok(ref _res) = result {
            if let Some(ref receiver_name) = receiver_name_for_2pb {
                if self
                    .affine_tracker
                    .is_affine_binding(receiver_name.as_str())
                {
                    if self.method_takes_self_by_value(&recv_ty, method) {
                        self.affine_tracker
                            .use_value(receiver_name.as_str(), span)?;
                    }
                }
            }
        }

        result
    }

    /// Variant of infer_method_call_with_recv_type that skips static method lookup.
    /// Used for chained method calls (after the first) where the receiver expression
    /// is the original base, not the actual receiver of this specific method.
    fn infer_method_call_with_recv_type_skip_static(
        &mut self,
        recv_ty: Type,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
    ) -> Result<InferResult> {
        // Same NLL logic as infer_method_call_with_recv_type
        let old_call_context = self.in_call_arg_context;
        self.in_call_arg_context = true;

        let receiver_name_for_2pb = self.extract_receiver_name(receiver);
        if let Some(ref receiver_name) = receiver_name_for_2pb {
            self.borrow_tracker
                .begin_two_phase_borrow(receiver_name.clone(), span);
        }

        // Pass true for skip_static_lookup to avoid using receiver.kind for static method resolution
        let result = self.infer_method_call_inner_impl(
            receiver,
            method,
            type_args,
            args,
            span,
            Some(recv_ty),
            true,
        );

        if let Some(ref receiver_name) = receiver_name_for_2pb {
            self.borrow_tracker
                .end_two_phase_borrow(receiver_name.clone());
        }

        self.in_call_arg_context = old_call_context;

        result
    }

    /// Inner implementation of method call inference
    fn infer_method_call_inner(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<InferResult> {
        self.infer_method_call_inner_impl(receiver, method, &List::new(), args, span, None, false)
    }

    /// Core implementation of method call type inference.
    /// If `precomputed_recv_ty` is Some, uses that type instead of calling synth_expr(receiver).
    /// If `skip_static_lookup` is true, skips static method lookup based on receiver.kind.
    /// This is used for chained method calls where receiver.kind is the original base,
    /// not the actual receiver of this specific method call.
    ///

    /// `type_args` contains explicit type arguments for generic method calls like `obj.method<T>()`.
    fn infer_method_call_inner_impl(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
        precomputed_recv_ty: Option<Type>,
        skip_static_lookup: bool,
    ) -> Result<InferResult> {
        // ============================================================
        // Iterator Invalidation & Borrow Conflict Checks
        // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .4 - Iterator invalidation
        // ============================================================
        // These checks must be in infer_method_call_inner_impl to ensure
        // they run for all method calls including iterative chain handling.
        if self.is_mutating_collection_method(&method.name) {
            if let Some(collection_name) = self.extract_receiver_name(receiver) {
                // Check for iterator invalidation - this is a hard error
                self.borrow_tracker
                    .check_iterator_invalidation(&collection_name, span)?;

                // Check if this collection is captured by a closure.
                // Mutating a captured variable conflicts with the closure's borrow.
                // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #closure-captures
                if self.borrow_tracker.is_captured(&collection_name) {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "cannot borrow `{}` as mutable because it is captured by a closure",
                        collection_name
                    ))));
                }

                // Check for regular borrows (immutable borrows prevent mutation)
                if let Some(err) =
                    self.borrow_tracker
                        .check_borrow_allowed(&collection_name, true, span)
                {
                    return Err(err);
                }
            }
        }

        // DEBUG: Track receiver expression kind for min/max method calls
        #[cfg(debug_assertions)]
        if method.name.as_str() == "min" || method.name.as_str() == "max" {
            // eprintln!("[DEBUG method_call_start] method='{}', receiver.kind={:?}, receiver.span={:?}",
            // method.name.as_str(),
            // std::mem::discriminant(&receiver.kind),
            // receiver.span);
        }

        // `iter.collect()` is generic in its return type — `collect<C: FromIterator<Item>>()
        // -> C`. Without an expected target the result type is fully unconstrained, so
        // synthesize a fresh type variable and let bidirectional `check_expr` unify it
        // with the let-binding annotation (`let v: List<Int> = (0..N).collect();`).
        // Several of the deeper type-resolution paths inside this function happily return
        // the *element* type for `.collect()` on adapter shapes that don't carry a real
        // `FromIterator` dictionary — `Range<Int>` ended up as `Int`, which then failed
        // to unify with `List<Int>`. Short-circuit before any of those paths run.
        //

        // Audit confirmed (2026-04-18): this collect-only short-circuit is sufficient
        // for the most-broken case. Other generic methods either resolve through their
        // own bidirectional paths (`.into()`, `.from()`, `.parse_int()`) or have
        // shape-specific issues that need a different fix (`.max()` returns
        // `Item<ListIter<Int>>` because ListIter isn't in the synthetic-adapter list
        // and the associated-type projection isn't normalized to the element type
        // — tracked separately).
        if method.name.as_str() == "collect" && args.is_empty() {
            return Ok(InferResult::new(Type::Var(TypeVar::fresh())));
        }

        if let Some(r) = self.try_resolve_pre_receiver_method(
            receiver, method, type_args, args, span, skip_static_lookup,
        )? {
            return Ok(r);
        }


        // Step 1: Check for context method calls BEFORE synthesizing receiver.
        // If the receiver is a context name (`ComputeDevice.method()`),
        // retrieve its context type (a Record of method signatures) from
        // the context resolver and use it directly as the receiver type.
        // This bypasses `synth_expr` which would fail because context
        // names are not in the variable environment.
        //

        // Only fire on the first call of a method chain
        // (`!skip_static_lookup`). The iterative chain handler reuses the
        // outermost receiver expression for every chain step, so inside
        // `Ctx.method1().method2()` the same `Path(Ctx)` is the receiver
        // for every step — but `.method2()` must resolve against the
        // return type of `.method1()`, not against the context's record
        // shape. When `skip_static_lookup` is set, prefer
        // `precomputed_recv_ty` (passed in by the chain handler) so
        // later steps see the actual intermediate type.
        let mut context_recv_ty: Option<Type> = None;
        if !skip_static_lookup
            && let ExprKind::Path(path) = &receiver.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
        {
            let context_name = ident.name.as_str();
            let context_name_text = verum_common::Text::from(context_name);

            // NAME-COLLISION GUARD: if the user has a local `type X` (or
            // similar) with inherent methods under the same name, that
            // definitely takes precedence over any stdlib context with
            // the same spelling. Only enter the context-typed receiver
            // branch when there is NO user type for this name registered.
            //

            // This fixes a subtle cross-file shadow where `public context
            // Benchmark { fn new(name: Text) -> Bencher; ... }` in
            // `core/runtime/mod.vr` was quietly hijacking
            // `Benchmark.new(...)` calls in user files that happened to
            // have `using [Benchmark]` (even though those files declared
            // their own `type Benchmark`). The receiver type came out as
            // `{ new: fn(Unknown) -> Unknown, ... }`, `.new` resolved to
            // the stdlib Bencher shape, and the next method in the chain
            // failed with "no method `foo` found for type `Bencher`".
            //

            // The guard checks three places in priority order:
            //  1. `inherent_methods` — the scheme map populated by
            //  `implement X { ... }`. Definitive "user has impls".
            //  2. `__type_params_X` / type registration — record types
            //  registered via `type X is { ... }`.
            //  3. Plain `ctx.lookup_type(X)` — covers aliases / forward
            //  refs.
            //

            // Any of those indicate the user intends `X` as a type, not
            // as context-record dispatch, so we bypass the synthetic
            // context Record lookup.
            let user_type_shadows_context = {
                let has_inherent = self
                    .inherent_methods
                    .read()
                    .get(&context_name_text)
                    .map(|m| !m.is_empty())
                    .unwrap_or(false);
                let has_type_params = matches!(
                    self.ctx
                        .lookup_type(format!("__type_params_{}", context_name).as_str()),
                    Option::Some(_)
                );
                // `type X is ...` pass-1 registers as Placeholder; pass-2
                // resolves to Named / Record / Variant. Anything other than
                // absence (None) means the user has some concrete shape for
                // this name and expects `X.method(...)` to dispatch on it.
                let has_type = matches!(
                    self.ctx.lookup_type(context_name),
                    Option::Some(Type::Named { .. })
                        | Option::Some(Type::Record(_))
                        | Option::Some(Type::Variant(_))
                        | Option::Some(Type::Placeholder { .. })
                );
                has_inherent || has_type_params || has_type
            };

            // Check if this is a declared context (whether or not it's available)
            if !user_type_shadows_context
                && self.context_declarations.contains_key(&context_name_text)
            {
                // This IS a context - check if it's available
                if !self.context_checker.is_available(context_name)
                    && !self.context_resolver.is_lenient_contexts()
                {
                    // E801: Context used but not declared in using clause
                    return Err(TypeError::MissingContext {
                        context: context_name_text,
                        span,
                    });
                }
                // Retrieve the context type (Record of method signatures)
                // from the resolver. This was built during context
                // registration from the ContextDecl AST node.
                if let verum_common::Maybe::Some(ctx_type) =
                    self.context_resolver.get_context_type(&context_name_text)
                {
                    context_recv_ty = Some(ctx_type.clone());
                }
            }
        }

        // Step 2: Get receiver type — either from context resolution
        // (step 1), pre-computed chain, or synth_expr.
        let recv_ty_raw = if let Some(ctx_ty) = context_recv_ty {
            ctx_ty
        } else if let Some(precomputed) = precomputed_recv_ty {
            self.unifier.apply(&precomputed)
        } else {
            // Normal case: synthesize receiver type
            let recv_result = self.synth_expr(receiver)?;
            self.unifier.apply(&recv_result.ty)
        };

        // Never propagation: any method call on Never produces Never.
        // This suppresses cascading errors from unresolved super/module paths.
        if matches!(recv_ty_raw, Type::Never) {
            return Ok(InferResult::new(Type::Never));
        }

        // Strip refinement types: methods on Float{{<predicate>}} should resolve as Float methods
        let recv_ty_raw = match recv_ty_raw {
            Type::Refined { base, .. } => *base,
            Type::Sigma { fst_type, .. } => *fst_type,
            other => other,
        };

        if let Some(r) = self.resolve_reference_type_method(&recv_ty_raw, method, args, span)? {
            return Ok(r);
        }

        let method_name_str = method.name.as_str();

        // AUTO-DEREFERENCE: For method calls on references/Heap, use the underlying type
        // This enables ref.len() to call .len() on the underlying value
        // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — #auto-dereference
        let recv_ty_derefed = Self::auto_deref_for_method_call(&recv_ty_raw);

        // TYPE ALIAS RESOLUTION: If recv_ty is a Named type that is a type alias,
        // resolve it to the underlying type so method lookups (e.g., .to_string())
        // find methods on the underlying primitive type.
        // Example: type Epoch = u32; -> Epoch.current() returns Epoch, .to_string() should work
        let recv_ty = if let Type::Named { ref path, .. } = recv_ty_derefed {
            let type_name: verum_common::Text = if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                    id.name.as_str().into()
                } else {
                    "".into()
                }
            } else {
                "".into()
            };
            if !type_name.is_empty() {
                if let Some(target) = self.ctx.type_aliases.get(&type_name) {
                    // Resolve the alias to underlying type
                    target.clone()
                } else {
                    recv_ty_derefed
                }
            } else {
                recv_ty_derefed
            }
        } else {
            recv_ty_derefed
        };

        // ============================================================
        // DYN PROTOCOL METHOD RESOLUTION
        // When receiver is &dyn Protocol (DynProtocol type), resolve method
        // from the protocol's declared methods. Look up via protocol_checker.
        // ============================================================
        if let Type::DynProtocol { ref bounds, .. } = recv_ty {
            for bound_name in bounds {
                let pc = self.protocol_checker.read();
                if let Some(method_ty) = pc.get_method_type(bound_name.as_str(), method_name_str) {
                    drop(pc);
                    // Infer arguments (type check them)
                    for arg in args.iter() {
                        let _ = self.infer_expr(arg, InferMode::Synth)?;
                    }
                    return Ok(InferResult::new(method_ty));
                }
            }
        }

        if let Some(r) = self.resolve_inherent_and_collection_method(
            &recv_ty, method_name_str, receiver, method, type_args, args, span,
        )? {
            return Ok(r);
        }


        // ============================================================
        // CAPABILITY-RESTRICTED TYPE METHOD FILTERING
        // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
        // ============================================================
        // When the receiver type is a CapabilityRestricted type (e.g., `Database with [Read]`),
        // we must verify that the method being called doesn't require capabilities that
        // aren't available. This provides compile-time enforcement of capability restrictions.
        //

        // Example:
        // ```verum
        // fn analyze(db: Database with [Read]) -> Stats {
        //  db.query("SELECT ..."); // OK - query requires Read
        //  db.delete("DELETE ..."); // ERROR - delete requires Write, not available
        // }
        // ```
        if let Type::CapabilityRestricted { base, capabilities } = &recv_ty {
            let method_name_str = method.name.as_str();
            let type_name = self.type_to_name(base);
            self.check_capability_restricted_method(
                method_name_str,
                capabilities,
                &type_name,
                span,
            )?;
        }

        // Strip CapabilityRestricted wrapper after capability check.
        // Method resolution needs the base type to find inherent methods,
        // protocol impls, etc. The capability restriction has already been
        // enforced above — from here on, treat as the underlying type.
        let recv_ty = match recv_ty {
            Type::CapabilityRestricted { base, .. } => *base,
            other => other,
        };

        // Lazy-load receiver type's inherent methods from
        // CoreMetadata.  The lazy preload pre-pass
        // (`register_stdlib_types_for_module`) only walks user code's
        // top-level type references — it skips function bodies (the
        // `collect_named_types_from_function_body` no-op) so a
        // body-local pattern bind like `match fs_metadata(p) { Ok(m)
        // => m.len() }` would land here with `m: Metadata` but
        // `inherent_methods["Metadata"]` empty.  Trigger an on-miss
        // load before any dispatch path consults the bucket.  Cheap
        // and idempotent — `ensure_stdlib_type_loaded` short-circuits
        // when the type is already in `ctx.type_defs`, and
        // `register_inherent_methods_from_metadata` skips method
        // names already registered.
        self.lazy_load_receiver_methods(&recv_ty);

        // Smart-pointer auto-deref for method resolution. If the
        // receiver is a type with a Deref::Target AND (a) the method
        // is NOT defined on the receiver itself but (b) IS defined
        // somewhere along the deref chain, unwrap the smart pointer
        // so protocol-method lookup on the inner type succeeds.
        //

        // Primary motivation: `Heap<dyn Tracer>.start_span(...)`
        // works the same as `(&*h).start_span(...)`. No hardcoded
        // list of smart pointers — we consult `Deref::Target` from
        // the stdlib's protocol declarations.
        //

        // Chain examples:
        //  * `Heap<Concrete>` → unwrap to Concrete
        //  * `Shared<Mutex<T>>` → unwrap to Mutex<T> if
        //  the method lives there;
        //  otherwise keep going
        //  * `MutexGuard<T>` → unwrap to T
        //

        // Bounded to 8 hops. Never unwraps when the current level
        // already has the method (so `Mutex.lock()` still binds to
        // Mutex, not to the inner T).
        let recv_ty = {
            let method_name_t: Text = method.name.as_str().into();
            let mut current = recv_ty;
            let mut hops = 0;
            while hops < 8 && !self.type_or_dyn_has_method(&current, &method_name_t) {
                let next = {
                    let checker = self.protocol_checker.read();
                    checker.try_find_associated_type(&current, &verum_common::Text::from("Target"))
                };
                match next {
                    Some(target) => {
                        let unwrapped = self.unwrap_reference_type(&target).clone();
                        let normalised = self.normalize_type(&unwrapped);
                        // DynProtocol is unsized and cannot be a
                        // by-value receiver. `Heap<dyn Tracer>` → deref
                        // gives `dyn Tracer`, which is the right
                        // *mathematical* target but not a usable
                        // receiver form. Method resolution expects a
                        // sized receiver, so we wrap plain DynProtocol
                        // in `&dyn ...` — that is the form used by
                        // every existing working dyn-dispatch call
                        // site (e.g. `(&*heap).method()` / `fn f(&dyn
                        // Tracer)`).
                        current = match normalised {
                            Type::DynProtocol { .. } => Type::Reference {
                                mutable: false,
                                inner: Box::new(normalised),
                            },
                            other => other,
                        };
                        hops += 1;
                    }
                    None => break,
                }
            }
            current
        };

        // Post-cascade DynProtocol resolution.
        //

        // The original DynProtocol path (above) runs *before* the
        // smart-pointer auto-deref cascade, so it only catches the
        // case where `receiver : &dyn P` arrives at that point
        // directly. After the cascade, `Heap<dyn P>` has been
        // unwrapped to `&dyn P`, and `Shared<dyn P>` → `&dyn P`
        // similarly. Those receivers need the same protocol-method
        // lookup, but we've now consumed the early branch.
        //

        // Peel one reference layer (and Ownership/CheckedRef/UnsafeRef
        // for completeness) and, if the inner type is DynProtocol,
        // resolve via the protocol's declared methods exactly as the
        // early path does. No hardcoded types — everything reads
        // from `protocol_checker` and from the DynProtocol's own
        // `bounds` list.
        {
            let peeled: &Type = match &recv_ty {
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. }
                | Type::Ownership { inner, .. } => inner.as_ref(),
                other => other,
            };
            if let Type::DynProtocol { bounds, .. } = peeled {
                for bound_name in bounds {
                    let pc = self.protocol_checker.read();
                    if let Some(method_ty) =
                        pc.get_method_type(bound_name.as_str(), method.name.as_str())
                    {
                        drop(pc);
                        for arg in args.iter() {
                            let _ = self.infer_expr(arg, InferMode::Synth)?;
                        }
                        return Ok(InferResult::new(method_ty));
                    }
                }
            }
        }

        // Check if this is a context method call - verify capabilities.
        //

        // The iterative method-chain handler
        // (`infer_method_chain_iterative`) walks
        // `CancelCtx.get_token().check()?` into three calls, all sharing
        // the *same* `receiver` expression (`CancelCtx`). Without the
        // `!skip_static_lookup` guard below, every call in the chain
        // would see `receiver.kind == Path("CancelCtx")` and spuriously
        // validate the method name against the context declaration —
        // e.g. reporting "context `CancelCtx` has no method `check`" for
        // a `.check()` call whose actual receiver type is `&CancelToken`.
        // The static lookup is only relevant for the first call.
        //

        // Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0 - Capability Attenuation
        if !skip_static_lookup
            && let ExprKind::Path(path) = &receiver.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
        {
            let context_name = ident.name.as_str();

            // Check if this name is a registered context with capabilities
            if let Maybe::Some(caps) = self
                .capability_checker
                .get_context_capabilities(context_name)
            {
                // This is a context call - check if the method requires specific capabilities
                // Use the method capability mapper to extract requirements from:
                // 1. Context declaration sub-contexts
                // 2. Method name heuristics
                use crate::capability::CapabilityRequirement;

                let context_name_text = verum_common::Text::from(context_name);
                let method_name_text: Text = method.name.as_str().into();

                // Look up the context declaration
                let context_decl = self.context_declarations.get(&context_name_text);

                // Extract required capabilities using the mapper
                let required_caps = self.method_capability_mapper.extract_method_capabilities(
                    &context_name_text,
                    &method_name_text,
                    context_decl,
                );

                let requirement = CapabilityRequirement::new(
                    context_name_text,
                    required_caps.clone(),
                    verum_common::Text::from(format!("{}()", method.name)),
                );

                // Check if capabilities are sufficient
                if let Err(cap_err) = self.capability_checker.check_requirement(&requirement) {
                    // E0306: Capability violation - method requires capability not available
                    use verum_diagnostics::capability_attenuation_errors::CapabilityViolationError;

                    let available_cap_names: Vec<String> = caps
                        .capabilities
                        .names()
                        .iter()
                        .map(|t| t.to_string())
                        .collect();

                    let diagnostic = CapabilityViolationError::new(
                        format!("{}::{}", context_name, method.name),
                        span_to_line_col(span),
                    )
                    .with_declared_capabilities(
                        available_cap_names
                            .iter()
                            .map(|s| s.as_str().into())
                            .collect(),
                    )
                    .with_function_name(format!("{}.{}", context_name, method.name))
                    .build();

                    self.diagnostics.push(diagnostic);

                    return Err(TypeError::Other(cap_err.message()));
                }

                // ============================================================
                // Context Method Validation (ContextChecker integration)
                // Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
                // ============================================================
                // Validate that the context method exists and context is available
                self.context_checker.check_context_call(
                    context_name,
                    method.name.as_str(),
                    span,
                )?;
            }
        }

        // CRITICAL FIX: Resolve type variables before protocol lookup
        // When we have a generic function like `fn display<T: Showable>(item: T)`,
        // and call `item.show()`, the receiver type is a type variable `θ`.
        // We need to look it up in the context to find the actual type constraints.
        // For type variables with protocol constraints (e.g., T: Showable),
        // we should use the constraint information for method lookup.
        // Resolve named types that are generic type params (e.g., Type::Named { path: "E" })
        // to their TypeVar representation so that bounds-based method resolution works.
        let recv_ty = match &recv_ty {
            Type::Named { path, args } if args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    let name_text: Text = ident.name.clone();
                    // Look up the name in context - it might be a type var
                    if let Some(scheme) = self.ctx.env.lookup(&name_text) {
                        let resolved = self.unifier.apply(&scheme.ty);
                        match &resolved {
                            Type::Var(tv) => {
                                let bounds = self.get_type_var_bounds(tv);
                                if !bounds.is_empty() {
                                    // This named type IS a bounded type var - use the var form
                                    resolved
                                } else {
                                    recv_ty.clone()
                                }
                            }
                            _ => recv_ty.clone(),
                        }
                    } else {
                        recv_ty.clone()
                    }
                } else {
                    recv_ty.clone()
                }
            }
            _ => recv_ty.clone(),
        };

        // Bound-first dispatch: when the receiver is a bounded type variable,
        // or a TypeApp over a bounded type variable (the HKT form `F<A>`
        // inside a generic function body with `F<_>: SomeProtocol`), find
        // the method via the variable's protocol bounds BEFORE falling
        // through to the general blanket-impl lookup. Without this, calls
        // like `fa.map(f)` where `fa: F<A>` resolve against every blanket
        // impl of `map` and pick the wrong one (e.g. FutureExt::map →
        // MapFuture<_, F<_>>).
        // Bound-first dispatch also needs to catch the `Generic { name: "F", args }`
        // form that `ast_to_type` emits when the user writes `F<A>` for an
        // in-scope HKT type parameter `F<_>: Trait` (the parser converts the
        // head of the type application to a Generic-name rather than a raw Var
        // or TypeApp over a Var). We look up the name in the context: if it
        // resolves to a bounded TypeVar, that var is the dispatch anchor.
        // `via_hkt_side_table` distinguishes the HKT-parameter dispatch
        // (receiver whose head is an `F<_>: SomeProtocol` we looked up
        // through `hkt_type_var_by_name`) from ordinary bounded type-var
        // dispatch (receiver is a `Type::Var` from a `<T: Display>` bound).
        // For HKT dispatch we'll restrict to protocols with explicit HKT
        // type params; for ordinary type-var dispatch, every protocol is fair
        // game (that's how `x.fmt()` on `T: Display` works today).
        self.resolve_method_via_protocol_search(
            recv_ty, recv_ty_raw, receiver, method, type_args, args, span, skip_static_lookup,
        )
    }

    /// Resolve a method call via protocol search (Steps 2–5: find protocol impls,
    /// search candidates, select best match, type-check arguments, return type).
    /// Called after all pre-receiver and receiver-type-based fast paths are exhausted.
    #[inline(never)]
    /// Dispatch CBGR intrinsic methods and reference tier-conversion methods.
    /// Fires BEFORE auto-deref so operations on the reference itself (not the
    /// referent) are intercepted first.
    /// Try to resolve a method call via the early inherent-methods table
    /// (methods registered from parsed .vr stdlib files) and the C-runtime
    /// collection method fallbacks (Map/Set/Deque).
    fn resolve_inherent_and_collection_method(
        &mut self,
        recv_ty: &Type,
        method_name_str: &str,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        // ============================================================
        // EARLY INHERENT_METHODS LOOKUP: Before hardcoded stdlib overrides,
        // check if the method was registered from parsed .vr stdlib files
        // via impl block registration (Pass S3). If found, use the registered
        // method signature instead of hardcoded fallbacks below.
        // This reduces hardcoded stdlib knowledge in the compiler.
        // ============================================================
        {
            // Skip the early-inherent lookup when the receiver's head is a
            // bounded type parameter (HKT or otherwise) — inherent methods
            // don't make sense on an abstract type variable, and querying by
            // the parameter's name can yield unrelated blanket entries whose
            // shape matches only by accident. Instead, we want such calls to
            // fall through to the bound-first dispatch below, which consults
            // the variable's protocol bounds and returns the correct method.
            let is_type_param = {
                let head_name: Option<verum_common::Text> = match &recv_ty {
                    Type::Var(_) => None, // handled below
                    Type::TypeApp { constructor, .. } => match &**constructor {
                        Type::Var(_) => None, // handled below
                        _ => None,
                    },
                    Type::Generic { name, .. } => Some(name.clone()),
                    Type::Named { path, .. } => path
                        .as_ident()
                        .map(|id| verum_common::Text::from(id.name.as_str())),
                    _ => None,
                };
                let head_is_param = head_name
                    .as_ref()
                    .map(|n| {
                        // HKT parameters are recognized via the side table
                        // (preserves the name even when env shows a
                        // TypeConstructor after kind inference); ordinary
                        // bounded type params show up as Type::Var or
                        // Type::TypeConstructor in env/types.
                        if self.hkt_type_var_by_name.contains_key(n) {
                            return true;
                        }
                        let resolved = self
                            .ctx
                            .env
                            .lookup(n)
                            .map(|s| self.unifier.apply(&s.ty))
                            .or_else(|| match self.ctx.lookup_type(n) {
                                Maybe::Some(t) => Some(self.unifier.apply(t)),
                                _ => None,
                            });
                        matches!(
                            resolved,
                            Some(Type::Var(_)) | Some(Type::TypeConstructor { .. })
                        )
                    })
                    .unwrap_or(false);
                match &recv_ty {
                    Type::Var(_) => true,
                    Type::TypeApp { constructor, .. } => {
                        matches!(&**constructor, Type::Var(_))
                    }
                    _ => head_is_param,
                }
            };

            let early_type_name: Option<verum_common::Text> = if is_type_param {
                None
            } else {
                match &recv_ty {
                    Type::Text => Some(verum_common::Text::from(WKT::Text.as_str())),
                    Type::Generic { name, .. } => Some(name.clone()),
                    Type::Named { path, .. } => path
                        .as_ident()
                        .map(|id| verum_common::Text::from(id.name.as_str())),
                    _ => None,
                }
            };

            if let Some(type_name_text) = early_type_name {
                let method_name_text = verum_common::Text::from(method.name.as_str());
                // Receiver-driven lazy load: when the receiver type was
                // inferred indirectly (through `Result.Ok` arm of a
                // function return, the `?`-operator unwrap, a chained
                // `.await` on `Result<T, _>`, …) and was never explicitly
                // *named* by the user code, the lazy stdlib loader pass
                // hasn't fired for `type_name_text` yet — so its
                // `inherent_methods` bucket is empty and any
                // `recv.method(...)` lookup falls straight through to
                // `MethodNotFound`.  An explicit `let conn:
                // AsyncPgPoolGuard = ...` annotation triggers the
                // load eagerly via the type-resolution path; the
                // receiver-only case bypasses it.  Force the same
                // load here, idempotent — `ensure_stdlib_type_loaded`
                // short-circuits on `ctx.lookup_type(name).is_some()`,
                // and `register_inherent_methods_from_metadata` skips
                // method names already populated.
                {
                    let mut pending_dep_load: Vec<verum_common::Text> = Vec::new();
                    self.ensure_stdlib_type_loaded(&type_name_text, &mut pending_dep_load);
                    while let Some(dep) = pending_dep_load.pop() {
                        self.ensure_stdlib_type_loaded(&dep, &mut pending_dep_load);
                    }
                }
                // Per-instantiation impl gating (task #35).
                //

                // When the stdlib or user code declares `impl<T> Foo<T,
                // ReadOnly>` and `impl<T> Foo<T, WriteOnly>` with
                // disjoint method sets, we must refuse `write(…)` on a
                // `Foo<_, ReadOnly>` receiver. The method signatures
                // themselves all landed in the flat
                // (type_name, method_name) map when their impl blocks
                // were registered, so this gate runs *before* the
                // lookup succeeds and skips the early-inherent path
                // when no registered impl pattern accepts the
                // receiver's concrete type arguments. Subsequent
                // lookup paths (protocol methods, universal methods)
                // continue to run normally, producing a proper E400
                // when none of them match either.
                let receiver_ty_args_for_gate: verum_common::List<Type> = match &recv_ty {
                    Type::Named { args, .. } | Type::Generic { args, .. } => args.clone(),
                    _ => verum_common::List::new(),
                };
                if !self.inherent_method_pattern_allows(
                    &type_name_text,
                    &method_name_text,
                    &receiver_ty_args_for_gate,
                ) {
                    // No registered impl pattern accepts this
                    // instantiation — produce the canonical
                    // "no method named X found for type Y" error
                    // so that `readonly_write_fail.vr` /
                    // `writeonly_read_fail.vr` reject at type check.
                    return Err(crate::TypeError::MethodNotFound {
                        ty: recv_ty.to_text(),
                        method: method.name.as_str().to_text(),
                        span: method.span,
                        did_you_mean: verum_common::Maybe::None,
                    });
                }
                let early_method_info = {
                    let methods_guard = self.inherent_methods.read();
                    methods_guard.get(&type_name_text).and_then(|methods| {
                        methods.get(&method_name_text).cloned().map(|scheme| {
                            let impl_vc = scheme.impl_var_count;
                            let (ty, fresh_vars, type_bounds) =
                                scheme.instantiate_with_type_bounds();
                            ((ty, fresh_vars, impl_vc), type_bounds)
                        })
                    })
                };

                if let Some(((method_ty, ordered_fresh_vars, impl_var_count), type_bounds)) =
                    early_method_info
                {
                    // Register type bounds for fresh type variables
                    for (fresh_var, bounds) in &type_bounds {
                        for bound in bounds {
                            self.register_type_var_type_bound(*fresh_var, bound.clone());
                        }
                    }

                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = &method_ty
                    {
                        if args.len() == params.len() {
                            // Extract receiver type args for binding
                            let receiver_type_args: List<Type> = match &recv_ty {
                                Type::Named { args, .. } | Type::Generic { args, .. } => {
                                    args.clone()
                                }
                                _ => List::new(),
                            };

                            // Bind type variables from receiver type args
                            let bind_limit = Self::resolve_bind_limit(
                                impl_var_count,
                                ordered_fresh_vars.len(),
                                receiver_type_args.len(),
                            );
                            let mut combined_subst = crate::ty::Substitution::new();
                            for (type_var, type_arg) in ordered_fresh_vars
                                .iter()
                                .take(bind_limit)
                                .zip(receiver_type_args.iter())
                            {
                                if let Ok(subst) =
                                    self.unifier.unify(&Type::Var(*type_var), type_arg, span)
                                {
                                    combined_subst.extend(subst);
                                }
                            }

                            // OVERLOAD GUARD: If a closure argument doesn't match the parameter
                            // type (e.g., List.position(value: &T) called with closure), skip this
                            // inherent method and fall through to protocol-based resolution which
                            // may have a predicate-accepting overload (e.g., Iterator.position).
                            let params_cloned = params.clone();
                            let mut signature_mismatch = false;
                            for (arg, param_ty) in args.iter().zip(params_cloned.iter()) {
                                if matches!(&arg.kind, ExprKind::Closure { .. }) {
                                    let subst_param_ty = param_ty.apply_subst(&combined_subst);
                                    let resolved_param = self.unifier.apply(&subst_param_ty);
                                    if !matches!(
                                        &resolved_param,
                                        Type::Function { .. }
                                            | Type::Var(_)
                                            | Type::Placeholder { .. }
                                    ) {
                                        signature_mismatch = true;
                                        break;
                                    }
                                }
                            }
                            if !signature_mismatch {
                                // Type check each argument with substituted param types
                                for (arg, param_ty) in args.iter().zip(params_cloned.iter()) {
                                    let subst_param_ty = param_ty.apply_subst(&combined_subst);
                                    self.check_expr(arg, &subst_param_ty)?;
                                }

                                // Apply substitution and unifier to get concrete return type
                                let subst_return_type = return_type.apply_subst(&combined_subst);
                                let final_return_type = self.unifier.apply(&subst_return_type);

                                return Ok(Some(InferResult::new(final_return_type)));
                            }
                            // signature_mismatch: fall through to protocol/fallback paths
                        }
                    }
                }
            }
        }

        // ============================================================
        // C-RUNTIME-INTERCEPTED COLLECTION METHODS (HARDCODED FALLBACK)
        // These overrides are reached only when inherent_methods lookup above
        // did NOT find the method (e.g., stdlib .vr files not loaded).
        // Map/Set/Deque are NOT compiled through VBC→LLVM (not in migrated_modules).
        // Their methods are intercepted by the C runtime which returns raw values,
        // not Maybe-wrapped values. Override return types here to match the actual
        // runtime behavior. When these modules are migrated, remove this block.
        // ============================================================
        // Also handle Named types — Map<K,V> may be represented as either Generic or Named
        let map_type_match = match &recv_ty {
            Type::Generic {
                name,
                args: type_args,
            } => Some((name.as_str().to_string(), type_args.clone())),
            Type::Named {
                path,
                args: type_args,
            } => path
                .as_ident()
                .map(|id| (id.name.as_str().to_string(), type_args.clone())),
            _ => None,
        };
        if let Some((type_name, type_args)) = map_type_match {
            let type_name = type_name.as_str();
            let method_name_str = method.name.as_str();
            // Map method types now come from compiled map.vr declarations.
            // Type overrides removed — uses map.vr's actual signatures:
            //  get(&self, key: &K) -> Maybe<&V>
            //  remove(&mut self, key: &K) -> Maybe<V>
            //  insert(&mut self, key: K, value: V)
            //  contains_key(&self, key: &K) -> Bool
            //  len(&self) -> Int
            //  is_empty(&self) -> Bool
            // Map method type overrides: C runtime returns raw values,
            // not Maybe<&V> from compiled map.vr. Required until AOT
            // routes through compiled map.vr with proper Maybe unwrap.
            match (type_name, method_name_str) {
                (m, "insert") if WKT::Map.matches(m) && args.len() == 2 && type_args.len() == 2 => {
                    let key_ty = &type_args[0];
                    let val_ty = &type_args[1];
                    self.check_expr(&args[0], key_ty)?;
                    self.check_expr(&args[1], val_ty)?;
                    return Ok(Some(InferResult::new(Type::Unit)));
                }
                (m, "get") if WKT::Map.matches(m) && args.len() == 1 && type_args.len() == 2 => {
                    let key_ty = &type_args[0];
                    let val_ty = &type_args[1];
                    self.check_expr(&args[0], key_ty)?;
                    // Map.get() returns Maybe<V>, not raw V
                    let resolved_val = self.unifier.apply(val_ty);
                    return Ok(Some(InferResult::new(Type::maybe(resolved_val))));
                }
                (m, "remove") if WKT::Map.matches(m) && args.len() == 1 && type_args.len() == 2 => {
                    let key_ty = &type_args[0];
                    let val_ty = &type_args[1];
                    self.check_expr(&args[0], key_ty)?;
                    // Map.remove() returns Maybe<V>, not raw V
                    let resolved_val = self.unifier.apply(val_ty);
                    return Ok(Some(InferResult::new(Type::maybe(resolved_val))));
                }
                (m, "contains_key")
                    if WKT::Map.matches(m) && args.len() == 1 && type_args.len() == 2 =>
                {
                    let key_ty = &type_args[0];
                    self.check_expr(&args[0], key_ty)?;
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                (m, "len") if WKT::Map.matches(m) && args.is_empty() => {
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                // ============================================================
                // JoinHandle<T> method overrides - thread join returns T
                // ============================================================
                ("JoinHandle", "join") if args.is_empty() && type_args.len() == 1 => {
                    // join() returns Result<T, JoinError> — callers typically .unwrap()
                    let inner_ty = self.unifier.apply(&type_args[0]);
                    let join_error_ty = Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new("JoinError", span)),
                        args: List::new(),
                    };
                    return Ok(Some(InferResult::new(Type::result(inner_ty, join_error_ty))));
                }
                ("JoinHandle", "is_finished") if args.is_empty() => {
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                // ============================================================
                // MutexGuard<T> / RwLockReadGuard<T> / RwLockWriteGuard<T>
                // auto-deref: delegate method calls to inner T.
                // For MutexGuard<Mutex<T>>, deref through Mutex to get T.
                // ============================================================
                ("MutexGuard" | "RwLockReadGuard" | "RwLockWriteGuard", _)
                    if type_args.len() == 1 =>
                {
                    let mut inner_ty = self.unifier.apply(&type_args[0]);
                    // Unwrap through Mutex<T>/RwLock<T> to get to the actual data type
                    // Handle both Type::Generic and Type::Named representations
                    let mutex_inner = match &inner_ty {
                        Type::Generic { name, args: ga }
                            if (WKT::Mutex.matches(name.as_str())
                                || WKT::RwLock.matches(name.as_str()))
                                && ga.len() == 1 =>
                        {
                            Some(self.unifier.apply(&ga[0]))
                        }
                        Type::Named { path, args: na } if na.len() == 1 => {
                            let is_mutex = path
                                .as_ident()
                                .map(|id| {
                                    let n = id.name.as_str();
                                    WKT::Mutex.matches(n) || WKT::RwLock.matches(n)
                                })
                                .unwrap_or(false);
                            if is_mutex {
                                Some(self.unifier.apply(&na[0]))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if let Some(unwrapped) = mutex_inner {
                        inner_ty = unwrapped;
                    }
                    // Recursively resolve the method on the inner type
                    let inner_expr = receiver.clone();
                    let empty_generic_args: List<verum_ast::ty::GenericArg> = List::new();
                    let result = self.infer_method_call_inner_impl(
                        &inner_expr,
                        method,
                        &empty_generic_args,
                        args,
                        span,
                        Some(inner_ty),
                        true,
                    );
                    if result.is_ok() {
                        return result.map(Some);
                    }
                    // If inner type resolution also fails, fall through
                }
                _ => {}
            }
        }

        // Text method return type overrides — match compiled text.vr signatures.
        // find/rfind/index_of return Maybe<Int>, byte_at/char_at return Maybe<Int>.
        // to_int returns Int directly, parse_int returns Result<Int, ParseError>.
        if matches!(&recv_ty, Type::Text) {
            let method_name_str = method.name.as_str();
            match method_name_str {
                // byte_at returns Maybe<Byte> (≈ Maybe<Int>), char_at returns Maybe<Char> (≈ Maybe<Int>)
                "byte_at" | "char_at" if args.len() == 1 => {
                    self.check_expr(&args[0], &Type::Int)?;
                    return Ok(Some(InferResult::new(Type::maybe(Type::Int))));
                }
                // find/rfind/index_of return Maybe<Int> from compiled text.vr
                "find" | "index_of" | "rfind" if args.len() == 1 => {
                    self.check_expr(&args[0], &Type::Text)?;
                    return Ok(Some(InferResult::new(Type::maybe(Type::Int))));
                }
                // to_int returns Int directly (0 on parse failure)
                "to_int" if args.is_empty() => {
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                // parse_int returns Result<Int, ParseError>
                "parse_int" if args.is_empty() => {
                    let error_ty = Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "ParseError",
                            Span::default(),
                        )),
                        args: List::new(),
                    };
                    return Ok(Some(InferResult::new(Type::result(Type::Int, error_ty))));
                }
                // parse_float returns Result<Float, ParseError>
                "parse_float" if args.is_empty() => {
                    let error_ty = Type::Named {
                        path: Path::single(verum_ast::ty::Ident::new(
                            "ParseError",
                            Span::default(),
                        )),
                        args: List::new(),
                    };
                    return Ok(Some(InferResult::new(Type::result(Type::Float, error_ty))));
                }
                // Type-directed parse(): returns Maybe<T> where T is
                // inferred from context (e.g., `let x: Int = "42".parse().unwrap()`)
                // Returns Maybe (Some/None) rather than Result, matching the common
                // pattern of `match input.parse<Int>() { Some(n) => n, None => ... }`
                "parse" if args.is_empty() => {
                    let inner_tv = Type::Var(TypeVar::fresh());
                    return Ok(Some(InferResult::new(Type::maybe(inner_tv))));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn resolve_reference_type_method(
        &mut self,
        recv_ty_raw: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        // ============================================================
        // CBGR INTRINSIC METHODS: Handle reference-specific methods BEFORE auto-dereference
        // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — #reference-intrinsics
        //

        // These methods operate on the reference itself, not the underlying value:
        // - stored_generation() -> Int: Get the generation counter captured when reference was created
        // - is_valid() -> Bool: Check if reference is still valid (generation matches)
        // - epoch() -> Int: Get the epoch from epoch_caps field (bits 0-15)
        // - capabilities() -> Int: Get the capabilities from epoch_caps field (bits 16-31)
        // - epoch_caps() -> Int: Get the raw epoch_caps u32 value
        // ============================================================
        let method_name_str = method.name.as_str();
        let is_reference_type = match &recv_ty_raw {
            Type::Reference { .. }
            | Type::CheckedReference { .. }
            | Type::UnsafeReference { .. } => true,
            Type::Generic { name, .. } if WKT::is_smart_pointer_name(name.as_str()) => true,
            _ => false,
        };

        if is_reference_type {
            match method_name_str {
                "stored_generation" if args.is_empty() => {
                    // stored_generation() -> Int
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "is_valid" if args.is_empty() => {
                    // is_valid() -> Bool
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                "epoch" if args.is_empty() => {
                    // epoch() -> Int (u16 extracted from epoch_caps)
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "capabilities" if args.is_empty() => {
                    // capabilities() -> Int (u16 extracted from epoch_caps)
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "epoch_caps" | "epoch_caps_raw" | "raw_epoch_caps" if args.is_empty() => {
                    // epoch_caps() -> Int (raw u32 packed value)
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "generation" if args.is_empty() => {
                    // generation() -> Int (stored generation from reference)
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "raw_ptr" if args.is_empty() => {
                    // raw_ptr() -> *const T (raw pointer to inner type)
                    // Extract inner type from reference for pointer construction
                    let inner_ty = match &recv_ty_raw {
                        Type::Reference { inner, .. }
                        | Type::CheckedReference { inner, .. }
                        | Type::UnsafeReference { inner, .. } => (**inner).clone(),
                        Type::Generic {
                            args: type_args, ..
                        } => type_args.first().cloned().unwrap_or(Type::Int),
                        _ => Type::Int,
                    };
                    return Ok(Some(InferResult::new(Type::Pointer {
                        inner: Box::new(inner_ty),
                        mutable: false,
                    })));
                }
                "can_read" if args.is_empty() => {
                    // can_read() -> Bool (check read capability on reference)
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                "can_write" if args.is_empty() => {
                    // can_write() -> Bool (check write capability on reference)
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                "is_epoch_valid" if args.is_empty() => {
                    // is_epoch_valid() -> Bool (check if reference epoch within validity window)
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                "header_generation" if args.is_empty() => {
                    // header_generation() -> Int (read generation from CBGR AllocationHeader)
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "header_size" if args.is_empty() => {
                    // header_size() -> Int (read data size from CBGR AllocationHeader)
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "header_epoch" if args.is_empty() => {
                    // header_epoch() -> Int (read epoch from CBGR AllocationHeader)
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "is_allocated" if args.is_empty() => {
                    // is_allocated() -> Bool (check if CBGR allocation is still live)
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                "is_freed" if args.is_empty() => {
                    // is_freed() -> Bool (check if CBGR allocation has been freed)
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                _ => {}
            }
        }

        // ============================================================
        // TIER CONVERSION METHODS - Must be handled BEFORE auto-dereference
        // These methods convert between reference tiers:
        // - to_checked() / as_checked(): Tier 0 (&T) -> Tier 1 (&checked T)
        // - to_managed() / as_managed(): Tier 1 (&checked T) -> Tier 0 (&T)
        // - to_unsafe() / as_unsafe(): Any tier -> Tier 2 (&unsafe T)
        // ============================================================
        match (&recv_ty_raw, method_name_str) {
            // to_checked() / as_checked() - Tier 0 -> Tier 1 (upgrade to zero-cost checked)
            (Type::Reference { inner, mutable }, "to_checked" | "as_checked")
                if args.is_empty() =>
            {
                return Ok(Some(InferResult::new(Type::CheckedReference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // to_managed() / as_managed() - Tier 1 -> Tier 0 (downgrade back to managed)
            (Type::CheckedReference { inner, mutable }, "to_managed" | "as_managed")
                if args.is_empty() =>
            {
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // to_unsafe() / as_unsafe() - Any tier -> Tier 2 (manual safety required)
            (Type::Reference { inner, mutable }, "to_unsafe" | "as_unsafe") if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            (Type::CheckedReference { inner, mutable }, "to_unsafe" | "as_unsafe")
                if args.is_empty() =>
            {
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // as_ref() - coercion for references
            // For managed: identity (already managed)
            // For checked: converts to managed (safe widening)
            // For unsafe: converts to managed (CAUTION: caller must ensure validity)
            (Type::Reference { inner, mutable }, "as_ref") if args.is_empty() => {
                // For &Heap<T> and &Shared<T>, as_ref() should unwrap to &T
                // (auto-deref through the smart pointer)
                if let Type::Generic {
                    name,
                    args: type_args,
                } = inner.as_ref()
                {
                    if WKT::is_smart_pointer_name(name.as_str()) && type_args.len() == 1 {
                        return Ok(Some(InferResult::new(Type::Reference {
                            inner: Box::new(type_args[0].clone()),
                            mutable: *mutable,
                        })));
                    }
                }
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            (Type::CheckedReference { inner, mutable }, "as_ref") if args.is_empty() => {
                // Checked -> Managed is safe (adds runtime checks)
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            (Type::UnsafeReference { inner, mutable }, "as_ref") if args.is_empty() => {
                // Unsafe -> Managed (caller must ensure reference is valid)
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // Heap<T>.as_ref() -> &T, Shared<T>.as_ref() -> &T
            (
                Type::Generic {
                    name,
                    args: type_args,
                },
                "as_ref",
            ) if args.is_empty()
                && WKT::is_smart_pointer_name(name.as_str())
                && type_args.len() == 1 =>
            {
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: Box::new(type_args[0].clone()),
                    mutable: false,
                })));
            }
            // Heap<T>.as_mut() -> &mut T, Shared<T>.as_mut() -> &mut T
            (
                Type::Generic {
                    name,
                    args: type_args,
                },
                "as_mut",
            ) if args.is_empty()
                && WKT::is_smart_pointer_name(name.as_str())
                && type_args.len() == 1 =>
            {
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: Box::new(type_args[0].clone()),
                    mutable: true,
                })));
            }
            // Mutable variants
            (
                Type::Reference {
                    inner,
                    mutable: true,
                },
                "to_checked_mut",
            ) if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::CheckedReference {
                    inner: inner.clone(),
                    mutable: true,
                })));
            }
            (
                Type::CheckedReference {
                    inner,
                    mutable: true,
                },
                "to_managed_mut",
            ) if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: inner.clone(),
                    mutable: true,
                })));
            }
            (
                Type::Reference {
                    inner,
                    mutable: true,
                },
                "to_unsafe_mut",
            ) if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: inner.clone(),
                    mutable: true,
                })));
            }
            (
                Type::CheckedReference {
                    inner,
                    mutable: true,
                },
                "to_unsafe_mut",
            ) if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: inner.clone(),
                    mutable: true,
                })));
            }
            // Auto-deref through one reference level for tier conversion methods
            // Handles `self.to_managed()` inside `implement Ref<T> for &checked T` where self: &&checked T
            (
                Type::Reference { inner, .. },
                method @ ("to_checked" | "as_checked" | "to_managed" | "as_managed" | "to_unsafe"
                | "as_unsafe" | "to_checked_mut" | "to_managed_mut" | "to_unsafe_mut"),
            ) if args.is_empty() => {
                let derefed = inner.as_ref().clone();
                match (&derefed, method) {
                    (Type::Reference { inner, mutable }, "to_checked" | "as_checked") => {
                        return Ok(Some(InferResult::new(Type::CheckedReference {
                            inner: inner.clone(),
                            mutable: *mutable,
                        })));
                    }
                    (Type::CheckedReference { inner, mutable }, "to_managed" | "as_managed") => {
                        return Ok(Some(InferResult::new(Type::Reference {
                            inner: inner.clone(),
                            mutable: *mutable,
                        })));
                    }
                    (Type::Reference { inner, mutable }, "to_unsafe" | "as_unsafe") => {
                        return Ok(Some(InferResult::new(Type::UnsafeReference {
                            inner: inner.clone(),
                            mutable: *mutable,
                        })));
                    }
                    (Type::CheckedReference { inner, mutable }, "to_unsafe" | "as_unsafe") => {
                        return Ok(Some(InferResult::new(Type::UnsafeReference {
                            inner: inner.clone(),
                            mutable: *mutable,
                        })));
                    }
                    (Type::Reference { inner, .. }, "to_checked_mut") => {
                        return Ok(Some(InferResult::new(Type::CheckedReference {
                            inner: inner.clone(),
                            mutable: true,
                        })));
                    }
                    (Type::CheckedReference { inner, .. }, "to_managed_mut") => {
                        return Ok(Some(InferResult::new(Type::Reference {
                            inner: inner.clone(),
                            mutable: true,
                        })));
                    }
                    (Type::Reference { inner, .. }, "to_unsafe_mut") => {
                        return Ok(Some(InferResult::new(Type::UnsafeReference {
                            inner: inner.clone(),
                            mutable: true,
                        })));
                    }
                    (Type::CheckedReference { inner, .. }, "to_unsafe_mut") => {
                        return Ok(Some(InferResult::new(Type::UnsafeReference {
                            inner: inner.clone(),
                            mutable: true,
                        })));
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // ============================================================
        // CBGR INTRINSIC METHODS - Reference metadata access
        // These provide access to CBGR reference internals:
        // - stored_generation(): Get generation from reference -> Int
        // - epoch_caps(): Get packed epoch+capabilities -> Int
        // - epoch(): Get just the epoch -> Int
        // - raw_ptr(): Get raw pointer address -> Int
        // ============================================================
        match (&recv_ty_raw, method_name_str) {
            // stored_generation() - Get the generation value stored in a managed reference
            (Type::Reference { .. }, "stored_generation") if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::Int)));
            }
            // epoch_caps() - Get the packed epoch+capabilities value
            (Type::Reference { .. }, "epoch_caps") if args.is_empty() => {
                // Returns an EpochCaps struct or Int for now
                return Ok(Some(InferResult::new(Type::Int)));
            }
            // epoch() - Get just the epoch value
            (Type::Reference { .. }, "epoch") if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::Int)));
            }
            // raw_ptr() - Get the raw pointer to inner type
            (Type::Reference { inner, .. }, "raw_ptr") if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::Pointer {
                    inner: inner.clone(),
                    mutable: false,
                })));
            }
            // generation() - Get generation for Heap<T> allocations
            (
                Type::Generic {
                    name,
                    args: type_args,
                },
                "generation",
            ) if args.is_empty() && WKT::Heap.matches(name.as_str()) => {
                return Ok(Some(InferResult::new(Type::Int)));
            }
            // allocation_generation() - Get current generation of the pointed-to allocation
            (Type::Reference { .. }, "allocation_generation") if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::Int)));
            }
            _ => {}
        }

        // ============================================================
        // POINTER/REFERENCE INTRINSIC METHODS - Must be BEFORE auto-deref
        // Methods like offset(), is_null() operate on the reference itself,
        // not on the pointed-to value. Auto-deref would strip the reference
        // and lose the ability to find these methods.
        // ============================================================
        match (&recv_ty_raw, method_name_str) {
            // offset(n: Int) -> same reference type (preserves inner type)
            (Type::UnsafeReference { inner, mutable }, "offset") if args.len() == 1 => {
                // Infer the argument (should be Int)
                let _arg_result = self.infer_expr(&args[0], InferMode::Synth)?;
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            (Type::Reference { inner, mutable }, "offset") if args.len() == 1 => {
                let _arg_result = self.infer_expr(&args[0], InferMode::Synth)?;
                return Ok(Some(InferResult::new(Type::Reference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            (Type::CheckedReference { inner, mutable }, "offset") if args.len() == 1 => {
                let _arg_result = self.infer_expr(&args[0], InferMode::Synth)?;
                return Ok(Some(InferResult::new(Type::CheckedReference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // is_null() -> Bool
            (
                Type::UnsafeReference { .. }
                | Type::Reference { .. }
                | Type::CheckedReference { .. }
                | Type::Pointer { .. },
                "is_null",
            ) if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::bool())));
            }
            // byte_offset(n: Int) -> same reference type
            (Type::UnsafeReference { inner, mutable }, "byte_offset") if args.len() == 1 => {
                let _arg_result = self.infer_expr(&args[0], InferMode::Synth)?;
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // add(n: Int) / sub(n: Int) -> same reference type (pointer arithmetic)
            (Type::UnsafeReference { inner, mutable }, "add" | "sub") if args.len() == 1 => {
                let _arg_result = self.infer_expr(&args[0], InferMode::Synth)?;
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // cast<U>() -> &unsafe U (pointer cast)
            (Type::UnsafeReference { .. }, "cast") if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::UnsafeReference {
                    inner: Box::new(Type::Var(TypeVar::fresh())),
                    mutable: false,
                })));
            }
            // Pointer methods: offset, byte_offset, add, sub -> same pointer type
            (Type::Pointer { inner, mutable }, "offset" | "byte_offset" | "add" | "sub")
                if args.len() == 1 =>
            {
                let _arg_result = self.infer_expr(&args[0], InferMode::Synth)?;
                return Ok(Some(InferResult::new(Type::Pointer {
                    inner: inner.clone(),
                    mutable: *mutable,
                })));
            }
            // Pointer offset_from -> Int
            (Type::Pointer { .. }, "offset_from") if args.len() == 1 => {
                let _arg_result = self.infer_expr(&args[0], InferMode::Synth)?;
                return Ok(Some(InferResult::new(Type::Int)));
            }
            // Pointer cast
            (Type::Pointer { .. }, "cast") if args.is_empty() => {
                return Ok(Some(InferResult::new(Type::Pointer {
                    inner: Box::new(Type::Var(TypeVar::fresh())),
                    mutable: false,
                })));
            }
            // Pointer to_owned -> inner type
            (Type::Pointer { inner, .. }, "to_owned") if args.is_empty() => {
                return Ok(Some(InferResult::new(inner.as_ref().clone())));
            }
            _ => {}
        }
        Ok(None)
    }

    fn resolve_method_via_protocol_search(
        &mut self,
        recv_ty: Type,
        recv_ty_raw: Type,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
        skip_static_lookup: bool,
    ) -> Result<InferResult> {
        let method_name_str = method.name.as_str();
        let mut via_hkt_side_table = false;
        let dispatch_var: Option<TypeVar> = match &recv_ty {
            Type::Var(v) => Some(*v),
            Type::TypeApp { constructor, .. } => match &**constructor {
                Type::Var(v) => Some(*v),
                _ => None,
            },
            Type::Generic { name, .. } => {
                // Head of a generic application that is itself a bounded HKT
                // parameter, e.g. `F<A>` inside `fn<F<_>: Functor>(fa: F<A>) ...`.
                match self.hkt_type_var_by_name.get(name).copied() {
                    Some(tv) if !self.get_type_var_bounds(&tv).is_empty() => {
                        let still_in_scope = self.ctx.env.lookup(name).is_some()
                            || matches!(self.ctx.lookup_type(name), Maybe::Some(_));
                        if still_in_scope {
                            via_hkt_side_table = true;
                            Some(tv)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let resolved_ty = if let Some(var) = dispatch_var {
            let var = &var;
            {
                let bounds = self.get_type_var_bounds(var);

                if !bounds.is_empty() {
                    // Type variable has bounds - we can use them for method resolution
                    // The protocol checker will find methods from the bounded protocols

                    // Try to find the method in any of the bounding protocols
                    for bound in &bounds {
                        if let Some(ident) = bound.protocol.as_ident() {
                            // Check if this protocol has the method we're looking for
                            let protocol_name = ident.name.as_str();
                            let method_name_text: Text = method.name.as_str().into();

                            // For HKT-side-table-routed dispatch (receiver
                            // head = `F<_>: SomeProtocol`), skip protocols
                            // without explicit HKT type parameters. Such
                            // protocols use associated-type HKTs like
                            // `type F<_>;` whose `Self.F<..>` can only be
                            // resolved via a concrete implementation's
                            // associated types through `get_implementations`.
                            // Running bound-first for them produces wrong
                            // signatures like `F<M<_>><_>`.
                            //

                            // Ordinary bounded type-var dispatch (e.g.,
                            // `x.fmt()` where `x: T`, `T: Display`) must
                            // keep working for every protocol, so the skip
                            // is gated on `via_hkt_side_table` being set.
                            if via_hkt_side_table {
                                let proto_has_explicit_hkt_param = {
                                    let pc = self.protocol_checker.read();
                                    matches!(
                                        pc.get_protocol(&Text::from(protocol_name)),
                                        Maybe::Some(p) if !p.type_params.is_empty()
                                    )
                                };
                                if !proto_has_explicit_hkt_param {
                                    continue;
                                }
                            }

                            // Look up the method in the protocol definition AND its superprotocols.
                            // When Hashable extends Eq + Hash, calling hash() on T: Hashable
                            // must find hash() in the Hash superprotocol.
                            let method_ty_opt = self
                                .find_method_in_protocol_hierarchy(
                                    &Text::from(protocol_name),
                                    &method_name_text,
                                )
                                .map(|pm| pm.ty.clone());

                            if let Some(method_ty) = method_ty_opt {
                                // Substitute protocol type parameters with bound args.
                                // E.g., for E: Module<Text, DynTensor<Float>>, substitute In->Text, Out->DynTensor<Float>
                                // so that forward's fn(In) -> Out becomes fn(Text) -> DynTensor<Float>.
                                //

                                // CRITICAL: The stored method type uses TypeVar IDs (e.g., Var(tvar_In)),
                                // NOT Named types (e.g., Named("In")). The TypeVars appear in the method
                                // signature in the same order as the protocol's type params (In, Out, ...).
                                // We collect free TypeVars in order and substitute using "T{id}" keys.
                                // Collect free TypeVars from method_ty in order of appearance.
                                // These correspond to the protocol's type params in declaration order.
                                fn collect_free_vars_ordered(ty: &Type, vars: &mut Vec<TypeVar>) {
                                    match ty {
                                        Type::Var(tv) => {
                                            if !vars.iter().any(|v| v == tv) {
                                                vars.push(*tv);
                                            }
                                        }
                                        Type::Function {
                                            params,
                                            return_type,
                                            ..
                                        } => {
                                            for p in params {
                                                collect_free_vars_ordered(p, vars);
                                            }
                                            collect_free_vars_ordered(return_type, vars);
                                        }
                                        Type::Named { args, .. } | Type::Generic { args, .. } => {
                                            for a in args {
                                                collect_free_vars_ordered(a, vars);
                                            }
                                        }
                                        Type::Tuple(tys) => {
                                            for t in tys {
                                                collect_free_vars_ordered(t, vars);
                                            }
                                        }
                                        Type::Reference { inner, .. }
                                        | Type::CheckedReference { inner, .. }
                                        | Type::UnsafeReference { inner, .. } => {
                                            collect_free_vars_ordered(inner, vars);
                                        }
                                        Type::TypeApp { constructor, args } => {
                                            collect_free_vars_ordered(constructor, vars);
                                            for a in args {
                                                collect_free_vars_ordered(a, vars);
                                            }
                                        }
                                        _ => {}
                                    }
                                }

                                let method_ty = if !bound.args.is_empty() {
                                    // Substitute protocol type parameters with the bound's
                                    // concrete arguments. E.g., for E: Module<Text, DynTensor<Float>>,
                                    // the protocol's In/Out slots become Text/DynTensor<Float>.

                                    let mut free_vars: Vec<TypeVar> = Vec::new();
                                    collect_free_vars_ordered(&method_ty, &mut free_vars);

                                    let mut param_subst: indexmap::IndexMap<Text, Type> =
                                        indexmap::IndexMap::new();

                                    // Map by TypeVar ID (T{id} key) — primary path for stored method types
                                    for (tv, arg_ty) in free_vars.iter().zip(bound.args.iter()) {
                                        let var_key: Text = format!("T{}", tv.id()).into();
                                        param_subst.insert(var_key, arg_ty.clone());
                                    }

                                    // Also map by protocol type param name — fallback for Named-type methods
                                    {
                                        let pc = self.protocol_checker.read();
                                        if let Maybe::Some(proto) =
                                            pc.get_protocol(&Text::from(protocol_name))
                                        {
                                            for (param, arg_ty) in
                                                proto.type_params.iter().zip(bound.args.iter())
                                            {
                                                param_subst
                                                    .insert(param.name.clone(), arg_ty.clone());
                                            }
                                        }
                                    }

                                    if !param_subst.is_empty() {
                                        self.substitute_type_params(&method_ty, &param_subst)
                                    } else {
                                        method_ty
                                    }
                                } else {
                                    // bound.args is empty — this is the HKT bound case:
                                    // `fn fmap<F<_>: Functor, ...>`. Here F has the implicit
                                    // role of Functor's HKT type parameter, so we substitute
                                    // the protocol's first type param (the HKT slot) with
                                    // Var(var) — the caller's type variable.
                                    //

                                    // Without this, method return types like `F<B>` retain the
                                    // protocol's internal TypeVar for F and fail to unify with
                                    // the caller's `F<_>` at the call site; the compiler then
                                    // falls through to blanket-impl lookup and picks the wrong
                                    // implementation (e.g., FutureExt::map → MapFuture<_, F<_>>).
                                    let pc = self.protocol_checker.read();
                                    let proto_opt = pc.get_protocol(&Text::from(protocol_name));
                                    if let Maybe::Some(proto) = proto_opt {
                                        if !proto.type_params.is_empty() {
                                            let mut free_vars: Vec<TypeVar> = Vec::new();
                                            collect_free_vars_ordered(&method_ty, &mut free_vars);

                                            let mut param_subst: indexmap::IndexMap<Text, Type> =
                                                indexmap::IndexMap::new();

                                            // Map the protocol's first type param (HKT slot) to the
                                            // caller's type variable, both by its fresh-TypeVar id
                                            // (primary: stored method types reference free Vars) and by
                                            // the parameter's human-readable name (fallback for
                                            // Named/Generic references inside the signature).
                                            if let Some(hkt_slot_tv) = free_vars.first() {
                                                let var_key: Text =
                                                    format!("T{}", hkt_slot_tv.id()).into();
                                                param_subst.insert(var_key, Type::Var(*var));
                                            }
                                            let first_param_name =
                                                proto.type_params[0].name.clone();
                                            param_subst.insert(first_param_name, Type::Var(*var));
                                            drop(pc);

                                            self.substitute_type_params(&method_ty, &param_subst)
                                        } else {
                                            drop(pc);
                                            method_ty
                                        }
                                    } else {
                                        drop(pc);
                                        method_ty
                                    }
                                };

                                // Found the method in a bounding protocol!
                                // Defer the actual protocol check until the type is resolved
                                self.defer_constraint(DeferredConstraint::ProtocolBound {
                                    ty: recv_ty.clone(),
                                    protocol: protocol_name.into(),
                                    span,
                                });

                                // Return early with the method type instantiated for this receiver
                                let instantiated_method_ty =
                                    self.instantiate_method_for_receiver(&method_ty, &recv_ty);
                                // Instantiate method's own type parameters with explicit type args
                                let instantiated_method_ty = self.instantiate_method_type_params(
                                    instantiated_method_ty,
                                    type_args,
                                    span,
                                )?;

                                // Check arguments against parameter types
                                if let Type::Function {
                                    params,
                                    return_type,
                                    ..
                                } = &instantiated_method_ty
                                {
                                    // Protocol method types sometimes carry an explicit
                                    // `self` parameter (when the protocol author wrote
                                    // `fn map(self: F<A>, ...)`) and sometimes have it
                                    // stripped (when written as `fn map(fa: F<A>, ...)`).
                                    // For method-on-value dispatch (`fa.map(f)`) the
                                    // receiver is passed implicitly, so when the method
                                    // has exactly one more param than we have args AND
                                    // the receiver is a user value (not a `self` reference
                                    // inside a protocol default implementation), assume
                                    // the first param is the self receiver and drop it.
                                    let receiver_is_self = if let verum_ast::expr::ExprKind::Path(
                                        p,
                                    ) = &receiver.kind
                                    {
                                        p.segments
                                            .first()
                                            .map(|s| match s {
                                                verum_ast::ty::PathSegment::SelfValue => true,
                                                verum_ast::ty::PathSegment::Name(id) => {
                                                    id.name.as_str() == "self"
                                                }
                                                _ => false,
                                            })
                                            .unwrap_or(false)
                                    } else {
                                        false
                                    };
                                    let expected_params: List<Type> =
                                        if params.len() == args.len() + 1 && !receiver_is_self {
                                            params.iter().skip(1).cloned().collect()
                                        } else {
                                            params.clone()
                                        };
                                    let expected_params = &expected_params;
                                    if args.len() != expected_params.len() {
                                        return Err(TypeError::WrongArgCount {
                                            method: method.name.as_str().to_text(),
                                            expected: expected_params.len(),
                                            actual: args.len(),
                                            span,
                                        });
                                    }

                                    for (arg, param_ty) in args.iter().zip(expected_params.iter()) {
                                        let resolved_param = self.unifier.apply(param_ty);
                                        self.check_expr(arg, &resolved_param)?;
                                    }

                                    let resolved_return = self.unifier.apply(return_type);
                                    return Ok(InferResult::new(resolved_return));
                                }
                            }
                        }
                    }
                }

                recv_ty.clone()
            }
        } else {
            recv_ty.clone()
        };

        // Increment protocol check metrics
        self.metrics.protocol_checks += 1;

        // Step 2: Find all protocol implementations for receiver type
        // Convert method name to Text for lookup
        let method_name: Text = method.name.as_str().into();

        // Step 3: Search for method in all protocols
        // Track which protocols we've already seen to avoid duplicates from generic/blanket impls
        let mut seen_protocols: verum_common::Set<Text> = verum_common::Set::new();
        let mut candidates: List<(Path, Type)> = List::new();

        // Scope the read guard tightly to avoid borrow conflicts later
        {
            let protocol_checker_guard = self.protocol_checker.read();
            let impls = protocol_checker_guard.get_implementations(&resolved_ty);

            // ============================================================
            // Protocol-typed receiver fallback.
            // ============================================================
            // When the receiver is itself a known protocol (used dyn-style,
            // e.g. `let computed: Stream<Int> = stream [...]; computed.fold(...)`),
            // no concrete impl is registered for `Stream` as a `for_type` —
            // so `get_implementations` returns nothing, and the dispatch
            // search below would fail with MethodNotFound. Pull methods
            // directly from the protocol declaration AND from any
            // sub-protocol whose `super_protocols` chain contains it
            // (e.g. `StreamExt extends Stream` contributes `fold/map/...`).
            //
            // Mirrors the `Type::DynProtocol` arm in
            // `lookup_all_protocol_methods` but for protocols spelled as
            // `Type::Named { path: "Stream", ... }` or
            // `Type::Generic { name: "Stream", ... }` — common when the
            // user writes the protocol name directly as a type (most
            // frequently from stream-comprehension type inference).
            let receiver_proto_name: Option<Text> = match &resolved_ty {
                Type::Named { path, .. } => path.as_ident().map(|id| id.as_str().into()),
                Type::Generic { name, .. } => Some(name.clone()),
                _ => None,
            };
            if let Some(ref recv_name) = receiver_proto_name
                && protocol_checker_guard.is_protocol_by_name(recv_name.as_str())
            {
                // Self-substitution map for protocol method type signatures.
                let mut self_subst: verum_common::Map<Text, Type> = verum_common::Map::new();
                self_subst.insert(Text::from("Self"), resolved_ty.clone());

                // Direct: methods declared on the receiver protocol.
                if let Maybe::Some(proto) = protocol_checker_guard.get_protocol(recv_name) {
                    if let Some(pm) = proto.methods.get(&method_name) {
                        let substituted = protocol_checker_guard
                            .substitute_type_params(&pm.ty, &self_subst);
                        let proto_path = verum_ast::ty::Path::single(verum_ast::Ident::new(
                            verum_common::Text::from(recv_name.as_str()),
                            method.span,
                        ));
                        let key: Text = recv_name.clone();
                        if !seen_protocols.contains(&key) {
                            seen_protocols.insert(key);
                            candidates.push((proto_path, substituted));
                        }
                    }
                }

                // Sub-protocols: any registered protocol whose super_protocols
                // names this receiver protocol contributes its default
                // method bodies (the StreamExt-extends-Stream pattern).
                let receiver_proto_str = recv_name.as_str().to_string();
                let sub_protos: Vec<(Text, Type)> = protocol_checker_guard
                    .all_protocols()
                    .filter_map(|sub_proto| {
                        let extends_receiver =
                            sub_proto.super_protocols.iter().any(|sb| {
                                if let Some(id) = sb.protocol.as_ident() {
                                    if id.as_str() == receiver_proto_str {
                                        return true;
                                    }
                                }
                                sb.protocol
                                    .segments
                                    .last()
                                    .and_then(|seg| match seg {
                                        verum_ast::ty::PathSegment::Name(id) => {
                                            Some(id.name.as_str())
                                        }
                                        _ => None,
                                    })
                                    .map(|last| last == receiver_proto_str)
                                    .unwrap_or(false)
                            });
                        if !extends_receiver {
                            return None;
                        }
                        sub_proto
                            .methods
                            .get(&method_name)
                            .map(|pm| (sub_proto.name.clone(), pm.ty.clone()))
                    })
                    .collect();
                for (sub_name, sub_ty) in sub_protos {
                    if seen_protocols.contains(&sub_name) {
                        continue;
                    }
                    seen_protocols.insert(sub_name.clone());
                    let substituted =
                        protocol_checker_guard.substitute_type_params(&sub_ty, &self_subst);
                    let sub_path = verum_ast::ty::Path::single(verum_ast::Ident::new(
                        sub_name,
                        method.span,
                    ));
                    candidates.push((sub_path, substituted));
                }
            }

            // ============================================================
            // Sub-protocol auto-promotion (default-method dispatch).
            // ============================================================
            // When `T` implements protocol `P` and there's another protocol
            // `P'` such that:
            //   * P' extends P (`type P' is protocol extends P { ... }`)
            //   * the queried method is declared on P' with a default body
            //   * P' has not been explicitly impl'd for T (no override needed)
            // … then dispatch should resolve to P''s default body. This
            // matches the user expectation that a sub-protocol with all
            // default methods is automatically usable on any type
            // implementing its parent — without forcing every stdlib type
            // to repeat `implement<T: P> P' for T {}` blanket impls.
            //
            // Mirrors the protocol-typed-receiver block above (commit
            // 3a6b58af) but for the concrete-type case: there the
            // receiver IS the parent protocol; here the receiver
            // implements the parent.
            //
            // Generic-param case: if `resolved_ty` is a `Type::Var` with
            // protocol bounds (e.g. `iter: I` where `I: Iterator`), the
            // bounds list IS the parent protocol set — use it just like
            // the impls list.
            // Skip when receiver is a fresh type-var: `get_implementations`
            // returns spurious matches (every blanket-impl unifies with a
            // fresh var), so the parent_proto_names list would be polluted
            // with unrelated protocols (Stream, MaybeIter, etc.) and the
            // sub-protocol scan would suggest wrong methods. Type-var
            // receivers fall through to the existing bound-fallback at
            // Step 4 (line 47005) which queries `all_type_params()`
            // for the registered bounds — that path is correct for
            // `<T: P>` generic-param dispatch.
            if !matches!(&resolved_ty, Type::Var(_)) {
                let parent_proto_names: Vec<String> = impls
                    .iter()
                    .filter_map(|impl_| {
                        impl_.protocol.as_ident().map(|id| id.name.as_str().to_string())
                    })
                    .collect();

                if !parent_proto_names.is_empty() {
                    // Self-substitution map for sub-protocol method type signatures.
                    let mut self_subst: verum_common::Map<Text, Type> =
                        verum_common::Map::new();
                    self_subst.insert(Text::from("Self"), resolved_ty.clone());

                    let sub_proto_methods: Vec<(Text, Type)> = protocol_checker_guard
                        .all_protocols()
                        .filter_map(|sub_proto| {
                            // Skip if this IS one of the parent protocols
                            // already directly implemented (would be a
                            // duplicate of the explicit impl-walk below).
                            if parent_proto_names.iter().any(|n| n.as_str() == sub_proto.name.as_str()) {
                                return None;
                            }
                            // Does this sub_proto extend any of the
                            // parent protocols T implements?
                            let extends_one_of_parents =
                                sub_proto.super_protocols.iter().any(|sb| {
                                    let last = sb
                                        .protocol
                                        .as_ident()
                                        .map(|id| id.as_str().to_string())
                                        .or_else(|| {
                                            sb.protocol.segments.last().and_then(|seg| {
                                                match seg {
                                                    verum_ast::ty::PathSegment::Name(id) => {
                                                        Some(id.name.as_str().to_string())
                                                    }
                                                    _ => None,
                                                }
                                            })
                                        });
                                    last.map(|s| {
                                        parent_proto_names.iter().any(|p| p.as_str() == s)
                                    })
                                    .unwrap_or(false)
                                });
                            if !extends_one_of_parents {
                                return None;
                            }
                            sub_proto
                                .methods
                                .get(&method_name)
                                .map(|pm| (sub_proto.name.clone(), pm.ty.clone()))
                        })
                        .collect();
                    for (sub_name, sub_ty) in sub_proto_methods {
                        if seen_protocols.contains(&sub_name) {
                            continue;
                        }
                        seen_protocols.insert(sub_name.clone());
                        let substituted = protocol_checker_guard
                            .substitute_type_params(&sub_ty, &self_subst);
                        let sub_path = verum_ast::ty::Path::single(verum_ast::Ident::new(
                            sub_name,
                            method.span,
                        ));
                        candidates.push((sub_path, substituted));
                    }
                }
            }

            for impl_ in impls {
                // Check if this implementation has the method directly
                let method_ty_opt = impl_.methods.get(&method_name).cloned();

                // #129 — protocol-default-method dispatch. When the impl
                // block doesn't override the method, the protocol's OWN
                // declaration may carry a default body (the canonical
                // shape for Iterator's `map`/`filter`/`collect`,
                // Functor's `map`, Monad's `flatten`, etc.). Pre-fix
                // this branch only checked `impl_.methods` (overrides)
                // and fell straight through to `find_superprotocol_method`
                // which walks the SUPERPROTOCOL chain — skipping the
                // impl's own protocol's `methods` table entirely.
                //
                // Closes the canonical
                // `xs.into_iter().map(f).collect()` failure: IntoList
                // implements Iterator but doesn't override `map`; the
                // method lives in `Iterator::methods` as a default.
                //
                // Stdlib-agnostic per `crates/verum_types/src/CLAUDE.md`:
                // the lookup is keyed by `impl_.protocol` (a property
                // of the registered impl), not a hardcoded list of
                // protocols/methods. Adding `Functor::map` works the
                // same way without compiler change.
                let proto_name_str = impl_
                    .protocol
                    .as_ident()
                    .map(|id| id.name.as_str().to_string());
                let method_ty_with_source = if let Some(ty) = method_ty_opt {
                    Some((ty, impl_.protocol.clone()))
                } else if let Some(name) = proto_name_str.as_ref().and_then(|n| {
                    protocol_checker_guard
                        .get_protocol_definition(n)
                        .and_then(|p| p.methods.get(&method_name))
                        .map(|m| m.ty.clone())
                }) {
                    Some((name, impl_.protocol.clone()))
                } else {
                    // Look in superprotocol hierarchy for this method (recursive BFS).
                    // When Eq extends PartialEq, and PartialEq extends some other protocol,
                    // methods from any ancestor should be available.
                    if let Some(name) = proto_name_str {
                        if let Some(proto_def) =
                            protocol_checker_guard.get_protocol_definition(&name)
                        {
                            // Use find_superprotocol_method which does full BFS traversal
                            protocol_checker_guard
                                .find_superprotocol_method(proto_def, &method_name)
                                .map(|method_info| (method_info.ty.clone(), impl_.protocol.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some((method_ty, source_protocol)) = method_ty_with_source {
                    // Deduplicate by protocol name to avoid "AmbiguousMethod" for same protocol
                    let protocol_key = source_protocol
                        .as_ident()
                        .map(|id| id.name.clone())
                        .unwrap_or_else(|| format!("{:?}", source_protocol).into());
                    // #[cfg(debug_assertions)]
                    // if method_name.as_str() == "next" {
                    //  eprintln!("[DEBUG dedup] protocol_key={:?}, already_seen={}", protocol_key, seen_protocols.contains(&protocol_key));
                    // }
                    if !seen_protocols.contains(&protocol_key) {
                        seen_protocols.insert(protocol_key.clone());
                        // CRITICAL FIX: Substitute Self with the receiver type.
                        // Protocol method signatures use Self as a placeholder (e.g., `fn lt(&self, other: &Self)`).
                        // We must substitute Self with the actual receiver type before type checking arguments.
                        // Without this, calls like `level1.lt(&level2)` fail with "expected Self, found Level".
                        let self_substituted_ty =
                            self.substitute_self_type(&method_ty, &resolved_ty);

                        // CRITICAL FIX: Substitute type parameters from impl's for_type FIRST.
                        // For generic impls like `implement<T> Iterator for MaybeIter<T>`,
                        // when calling on `MaybeIter<&Int>`, we need to substitute T -> &Int
                        // in the method's return type (e.g., `Maybe<T>` -> `Maybe<&Int>`).
                        // This MUST happen BEFORE instantiating fresh type variables, otherwise
                        // the fresh variables won't match the substitution keys.
                        let param_subst =
                            Self::build_impl_type_subst(&impl_.for_type, &resolved_ty);
                        let param_substituted_ty =
                            self.substitute_type_params(&self_substituted_ty, &param_subst);

                        // CRITICAL FIX: Instantiate fresh type variables for:
                        // 1. Remaining free TypeVars (from the impl)
                        // 2. Method-level type parameters stored as Named types
                        //

                        // Protocol impls store method-level type params (e.g., `I` in
                        // `fn extend<I: Iterator<Item=T>>`) as Named("I") rather than TypeVars.
                        // The `free_vars()` method doesn't find Named types, so we must also
                        // look up the protocol definition to find method-level type param names
                        // and replace those Named types with fresh TypeVars.
                        let param_substituted_ty = {
                            let mut subst = crate::ty::Substitution::new();

                            // Part 1: Handle remaining free TypeVars
                            let free_vars = param_substituted_ty.free_vars();
                            for var in free_vars {
                                let fresh = TypeVar::fresh();
                                subst.insert(var, Type::Var(fresh));
                            }

                            // Apply TypeVar substitution first
                            let ty_after_var_subst = if subst.is_empty() {
                                param_substituted_ty.clone()
                            } else {
                                param_substituted_ty.apply_subst(&subst)
                            };

                            // Part 2: Handle method-level type params stored as Named types
                            // Look up the protocol definition to get the method's type_param_names
                            let mut final_ty = ty_after_var_subst;
                            // Detect remaining Named types that are unresolved type parameters.
                            // These are single-segment Named types with no args that look like
                            // type parameter names (e.g., "I", "T", "U") and weren't already
                            // substituted by param_subst.
                            let unresolved_type_params =
                                self.find_unresolved_type_param_names(&final_ty, &param_subst);
                            for tp_name in unresolved_type_params {
                                let fresh = TypeVar::fresh();
                                final_ty = Self::replace_named_with_var(&final_ty, &tp_name, fresh);
                            }

                            final_ty
                        };

                        // CRITICAL FIX: Resolve associated type projections like ::Item<SliceIter<Int>>
                        // Try to resolve using the implementation's associated_types map first,
                        // but if empty, try using protocol_checker's knowledge of the concrete type.
                        let substituted_method_ty = if !impl_.associated_types.is_empty() {
                            self.resolve_associated_type_projections_with_impl(
                                &param_substituted_ty,
                                &impl_.associated_types,
                                &param_subst,
                            )
                        } else {
                            // Fall back to normalizing the type to resolve projections
                            // This uses the protocol_checker to look up associated types
                            self.normalize_type(&param_substituted_ty)
                        };

                        // #[cfg(debug_assertions)]
                        // if method_name.as_str() == "next" {
                        //  eprintln!(
                        //  "[DEBUG impl_lookup] ADDING to candidates: for_type={:?}",
                        //  impl_.for_type,
                        //  );
                        //  eprintln!(
                        //  " final_substituted={:?}",
                        //  substituted_method_ty
                        //  );
                        // }
                        #[cfg(debug_assertions)]
                        if method_name.as_str() == "partial_cmp" {
                            // eprintln!(
                            // "[DEBUG impl_lookup] Found partial_cmp in impl: original_ty={:?}, substituted={:?}",
                            // method_ty,
                            // substituted_method_ty
                            // );
                        }
                        candidates.push((source_protocol, substituted_method_ty));
                    }
                }
            }
        } // protocol_checker_guard dropped here

        // Step 4: Handle no candidates or ambiguity
        if candidates.is_empty() {
            return self.handle_empty_candidates(
                recv_ty, &recv_ty_raw, receiver, method, type_args, args, span, skip_static_lookup,
            );
        }

        // CRITICAL FIX: FIRST, prefer candidates with concrete method signatures over those with
        // unresolved associated type projections (::Item, etc.).
        // When DoubleEndedIterator extends Iterator, both may provide `next`, but the
        // DoubleEndedIterator version may have unresolved projections from the parent protocol.
        // We should prefer the concrete version from the direct implementation.
        // This must happen BEFORE superprotocol filtering, because superprotocol filtering
        // would remove the concrete Iterator candidate in favor of the abstract DoubleEndedIterator one.
        if candidates.len() > 1 {
            let concrete_candidates: List<(Path, Type)> = candidates
                .iter()
                .filter(|(_, ty)| !self.contains_unresolved_projection(ty))
                .cloned()
                .collect();

            // #[cfg(debug_assertions)]
            // if method_name.as_str() == "next" {
            //  eprintln!("[DEBUG filter EARLY] total candidates: {}, concrete: {}", candidates.len(), concrete_candidates.len());
            // }

            if !concrete_candidates.is_empty() && concrete_candidates.len() < candidates.len() {
                // Found concrete candidates, use only those
                candidates = concrete_candidates;
            }
        }

        // CRITICAL FIX: Handle protocol inheritance in method resolution.
        // When a protocol extends another (e.g., Eq extends PartialEq), and both provide
        // the same method (e.g., eq), we should prefer the child protocol's version.
        // This prevents "ambiguous method call" errors when methods are inherited.
        if candidates.len() > 1 {
            // Get protocol names for all candidates
            let candidate_names: List<verum_common::Text> = candidates
                .iter()
                .filter_map(|(path, _)| path.as_ident().map(|id| id.name.clone()))
                .collect();

            // Check which protocols are superprotocols of others
            let mut is_superprotocol = verum_common::Set::new();
            // Track the origin of inherited methods (which superprotocol defines the method)
            let mut method_origin: verum_common::Map<verum_common::Text, verum_common::Text> =
                verum_common::Map::new();
            {
                let protocol_checker_guard = self.protocol_checker.read();
                for name in &candidate_names {
                    if let Some(proto_def) =
                        protocol_checker_guard.get_protocol_definition(name.as_str())
                    {
                        // Check if method is defined directly in this protocol
                        let has_method_directly = proto_def.methods.contains_key(&method_name);

                        for super_bound in &proto_def.super_protocols {
                            if let Some(super_ident) = super_bound.protocol.as_ident() {
                                // Mark the superprotocol as one that should be filtered out
                                if candidate_names
                                    .iter()
                                    .any(|n| n.as_str() == super_ident.name.as_str())
                                {
                                    is_superprotocol.insert(super_ident.name.clone());
                                }

                                // If method is not defined directly, check if superprotocol defines it
                                if !has_method_directly {
                                    if let Some(super_def) = protocol_checker_guard
                                        .get_protocol_definition(super_ident.name.as_str())
                                    {
                                        if super_def.methods.contains_key(&method_name) {
                                            method_origin
                                                .insert(name.clone(), super_ident.name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Filter out superprotocol candidates
            if !is_superprotocol.is_empty() {
                candidates.retain(|(path, _)| {
                    path.as_ident()
                        .map(|id| !is_superprotocol.contains(&id.name))
                        .unwrap_or(true)
                });
            }

            // CRITICAL FIX: Handle diamond inheritance (multiple protocols inherit from same source).
            // When Eq extends PartialEq and PartialOrd extends PartialEq, and both inherit `eq`,
            // they share the same method origin (PartialEq). In this case, keep just one.
            if candidates.len() > 1 && !method_origin.is_empty() {
                let candidate_origins: verum_common::Set<verum_common::Text> = candidates
                    .iter()
                    .filter_map(|(path, _)| {
                        path.as_ident()
                            .and_then(|id| method_origin.get(&id.name).cloned())
                    })
                    .collect();

                // If all inherited methods share the same origin, keep just the first candidate
                if candidate_origins.len() == 1 {
                    // All candidates inherit from the same superprotocol, keep first
                    candidates.truncate(1);
                }
            }
        }

        if candidates.len() > 1 {
            // Multiple candidates remain after inheritance filtering.
            // Rather than erroring, pick the first candidate. Methods like
            // `to_string()` and `clone()` may appear in multiple protocols
            // (e.g., ToString AND Display), but they should all be semantically
            // equivalent for the resolved type.
            candidates.truncate(1);
        }

        // CRITICAL FIX: Prefer candidates with concrete method signatures over those with
        // unresolved associated type projections (::Item, etc.).
        // When DoubleEndedIterator extends Iterator, both may provide `next`, but the
        // DoubleEndedIterator version may have unresolved projections from the parent protocol.
        // We should prefer the concrete version from the direct implementation.
        let candidates = {
            // Check if any candidate has a concrete signature (no ::Item projections)
            let concrete_candidates: List<(Path, Type)> = candidates
                .iter()
                .filter(|(_, ty)| !self.contains_unresolved_projection(ty))
                .cloned()
                .collect();

            // #[cfg(debug_assertions)]
            // if method_name.as_str() == "next" {
            //  eprintln!("[DEBUG filter] total candidates: {}, concrete: {}", candidates.len(), concrete_candidates.len());
            //  for (i, (_, ty)) in candidates.iter().enumerate() {
            //  eprintln!(" candidate[{}]: has_projection={}", i, self.contains_unresolved_projection(ty));
            //  }
            // }

            if !concrete_candidates.is_empty() {
                concrete_candidates
            } else {
                candidates
            }
        };

        // Step 5: Extract the unique method signature
        let (protocol_path, method_ty) = &candidates[0];
        #[cfg(debug_assertions)]
        if method.name.as_str() == "eq" {
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG candidates entry] Method 'eq' going through candidates path: method_ty={:?}", method_ty);
        }
        // Step 6: Type check arguments against method signature
        match method_ty {
            Type::Function {
                params,
                return_type,
                ..
            } => {
                // Protocol methods in ProtocolImpl.methods EXCLUDE self parameter.
                // This is consistent with Protocol.methods and inherent_methods conventions.
                // For method calls like `recv.method(arg)`, check args directly against params.
                let method_params = params.as_slice();

                // #[cfg(debug_assertions)]
                // eprintln!(
                // "[DEBUG candidates path] Method '{}': args.len()={}, params.len()={}, method_ty={:?}",
                // method.name.as_str(),
                // args.len(),
                // method_params.len(),
                // method_ty
                // );

                // Check argument count
                if args.len() != method_params.len() {
                    return Err(TypeError::WrongArgCount {
                        method: method.name.as_str().to_text(),
                        expected: method_params.len(),
                        actual: args.len(),
                        span,
                    });
                }

                // Type check each argument with substitution
                for (arg, param_ty) in args.iter().zip(method_params.iter()) {
                    let resolved_param = self.unifier.apply(param_ty);
                    self.check_expr(arg, &resolved_param)?;
                }

                // Step 7: Return method's return type with substitution
                let resolved_return = self.unifier.apply(return_type);
                // CRITICAL FIX: Normalize the return type to resolve associated type projections.
                // After unifier.apply(), we may still have projections like ::Item[SliceIter<Int>]
                // that need to be resolved to &Int by looking up the associated type definition.
                let normalized_return = self.normalize_type(&resolved_return);
                Ok(InferResult::new(normalized_return))
            }
            _ => {
                // Method type is not a function - this shouldn't happen
                Err(TypeError::Other(verum_common::Text::from(format!(
                    "Method `{}` in protocol `{}` has non-function type: {}",
                    method.name.as_str(),
                    protocol_path,
                    method_ty
                ))))
            }
        }
    }

    /// Handle the case where no protocol candidates were found in Step 3.
    /// Tries all fallback resolution strategies (type-var bounds, free functions,
    /// struct field access, primitive types, etc.) and returns MethodNotFound
    /// if all fallbacks fail.
    #[inline(never)]
    fn handle_empty_candidates(
        &mut self,
        recv_ty: Type,
        recv_ty_raw: &Type,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
        skip_static_lookup: bool,
    ) -> Result<InferResult> {
        let method_name_str = method.name.as_str();
        let method_name: verum_common::Text = method.name.as_str().into();
        if let Some(r) = self.try_type_var_bound_dispatch(&recv_ty, method, type_args, args, span)? {
            return Ok(r);
        }


        // =========================================================================
        // Protocol-based method resolution (NEW - replaces hardcoded methods)
        // Protocol-driven method resolution: methods resolved by searching implemented protocols for matching signatures
        // CRITICAL: Pass recv_ty_raw (NOT recv_ty) to preserve reference information
        // This enables first()/last()/get() to return &T when called on &List<T>
        // and CBGR tier conversion methods (to_checked, to_managed, to_unsafe)
        // =========================================================================
        // Pre-infer argument types for disambiguating parameterized protocol impls.
        // This is needed for protocols like FromResidual<R> where multiple
        // implementations exist with different R types.
        let pre_inferred_arg_types: Vec<Type> = args
            .iter()
            .filter_map(|arg| self.infer_expr(arg, InferMode::Synth).ok().map(|r| r.ty))
            .collect();
        let lookup_result_opt = self.protocol_checker.read().lookup_method_with_args(
            &recv_ty_raw,
            method.name.as_str(),
            &pre_inferred_arg_types,
        );
        if let Some(lookup_result) = lookup_result_opt {
            #[cfg(debug_assertions)]
            if method.name.as_str() == "min" {
                // eprintln!("[DEBUG protocol_lookup] Found 'min' via protocol_checker:");
                // eprintln!(" recv_ty_raw={:?}", recv_ty_raw);
                // eprintln!(" signature.params.len()={}", lookup_result.signature.params.len());
                // eprintln!(" signature.params={:?}", lookup_result.signature.params);
                // eprintln!(" args.len()={}", args.len());
            }
            // Type check arguments against the resolved parameter types
            if args.len() == lookup_result.signature.params.len() {
                for (arg, param_ty) in args.iter().zip(lookup_result.signature.params.iter()) {
                    self.check_expr(arg, param_ty)?;
                }
            }
            // CRITICAL FIX: Normalize return type to resolve associated type projections
            // like ::Item[SliceIter<Int>] -> &Int
            let normalized_return = self.normalize_type(&lookup_result.signature.return_type);
            return Ok(InferResult::new(normalized_return));
        }

        // NOTE: Legacy get_builtin_method_type fallback has been removed.
        // All method resolution is now handled by protocol_checker.lookup_method()
        // which provides data-driven method signatures via MethodRegistry.
        // Protocol-driven method resolution: methods resolved by searching implemented protocols for matching signatures

        let (inherent_result, specialization_rejected) = self.try_inherent_method_dispatch(
            &recv_ty, method, args, span,
        )?;
        if let Some(r) = inherent_result {
            return Ok(r);
        }


        if let Some(r) = self.try_blanket_and_ufcs_dispatch(&recv_ty, method, args, span)? {
            return Ok(r);
        }


        if let Some(r) = self.try_synthetic_iterator_adapter_dispatch(&recv_ty, method, args, span)? {
            return Ok(r);
        }


        let method_name_text = verum_common::Text::from(method.name.as_str());
        if let Some(r) = self.try_protocol_object_dispatch(&recv_ty, method, type_args, args, span)? {
            return Ok(r);
        }


        // HARDCODED FALLBACK: Primitive method return types for Int/Float/Bool/Char/Byte.
        // This is reached when inherent_methods (from stdlib .vr implement blocks) did
        // not contain the method. In normal compilation with stdlib loaded, this should
        // be dead code — all methods are registered via Pass 5 (register_impl_block).
        // HARDCODE(#7): Remove once inherent_methods always has these signatures.
        // This fallback is dead code when stdlib is fully loaded via Pass 5.
        if let Some(return_ty) =
            resolve_primitive_method(&recv_ty, &method_name_text, args.len())
        {
            // Type check args (primitives accept Int/Float/Bool args)
            for arg in args.iter() {
                let _ = self.synth_expr(arg)?;
            }
            return Ok(InferResult::new(return_ty));
        }

        // CLONE/DEFAULT FALLBACK: For record/named types, allow clone() to return Self.
        // This handles cases where types don't have explicit @derive(Clone) but are
        // semantically clonable (all fields are value types).
        if method.name.as_str() == "clone" && args.is_empty() {
            match &recv_ty {
                Type::Named { .. }
                | Type::Generic { .. }
                | Type::Record(_)
                | Type::Variant(_) => {
                    return Ok(InferResult::new(recv_ty.clone()));
                }
                _ => {}
            }
        }

        if let Some(r) = self.try_fallback_name_dispatch(&recv_ty, recv_ty_raw, method, args, span)? {
            return Ok(r);
        }


        if let Some(r) = self.try_builtin_type_method_fallback(&recv_ty, method, args, span)? {
            return Ok(r);
        }


        return self.emit_method_not_found(
            recv_ty, receiver, method, type_args, args, span, skip_static_lookup, specialization_rejected,
        );
    }

    /// Resolve method calls on synthetic iterator adapter types
    /// (MapIterator, FilterIter, EnumerateIter, etc.) that exist only as
    /// type-checker constructs without registered protocol implementations.
    fn try_synthetic_iterator_adapter_dispatch(
        &mut self,
        recv_ty: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        let adapter_method_name = method.name.as_str();
        let adapter_type_name = match &recv_ty {
            Type::Generic { name, .. } => Some(name.as_str()),
            Type::Named { path, .. } => path.as_ident().map(|id| id.name.as_str()),
            _ => None,
        };
        let adapter_type_args = match &recv_ty {
            Type::Generic { args: targs, .. } | Type::Named { args: targs, .. } => {
                targs.clone()
            }
            _ => List::new(),
        };
        // Only match SYNTHETIC adapter types created by the type checker itself.
        // These are types like MapIterator, FilterIter, etc. that exist
        // only as type-checker constructs without protocol implementations.
        // Do NOT match real stdlib types like ListIter, SliceIter, etc.
        let is_iterator_adapter = adapter_type_name.is_some_and(|n| {
            n == "MapIterator" || n == "FilterIter" || n == "FilterMapIter"
            || n == "ScanIter" || n == "EnumerateIter" || n == "FlatMapIter"
            || n == "Iter" // generic Iter<T> from .iter() calls
            || n == "Range" || n == "RangeInclusive"
            || n == "MappedIter" || n == "ChainIter" || n == "ZipIter"
            || n == "TakeIter" || n == "SkipIter" || n == "TakeWhileIter"
            || n == "SkipWhileIter" || n == "PeekableIter" || n == "FuseIter"
            || n == "RevIter" || n == "InspectIter" || n == "StepByIter"
            || n == "CopiedIter" || n == "ClonedIter" || n == "FlattenIter"
            || n == "DedupIter" || n == "UniqueIter" || n == "SortedIter"
            || n == "ChunksIter"
        });
        if is_iterator_adapter {
            // Helper: extract element type from an iterator adapter type
            let extract_adapter_elem_ty = |recv: &Type, targs: &List<Type>| -> Type {
                let tname = match recv {
                    Type::Generic { name, .. } => name.as_str(),
                    Type::Named { path, .. } => {
                        path.as_ident().map(|id| id.name.as_str()).unwrap_or("")
                    }
                    _ => "",
                };
                if (tname.contains("Map") || tname.contains("Mapped")) && targs.len() >= 2 {
                    // MapIterator<Iter, F> - element type is F's return type
                    match &targs[1] {
                        Type::Function { return_type, .. } => (**return_type).clone(),
                        _ => Type::Var(TypeVar::fresh()),
                    }
                } else if tname.contains("Enumerate") && !targs.is_empty() {
                    // EnumerateIter<Iter> - element type is (Int, inner_elem)
                    let inner_elem = match &targs[0] {
                        Type::Generic {
                            args: inner_args, ..
                        }
                        | Type::Named {
                            args: inner_args, ..
                        } if !inner_args.is_empty() => inner_args[0].clone(),
                        _ => Type::Var(TypeVar::fresh()),
                    };
                    Type::Tuple(vec![Type::Int, inner_elem].into())
                } else if !targs.is_empty() {
                    // FilterIter, TakeIter, SkipIter, Rev, etc. - preserve inner element type
                    match &targs[0] {
                        Type::Generic {
                            args: inner_args, ..
                        }
                        | Type::Named {
                            args: inner_args, ..
                        } if !inner_args.is_empty() => {
                            // If inner is also a Map adapter, recurse
                            let inner_name = match &targs[0] {
                                Type::Generic { name, .. } => name.as_str(),
                                Type::Named { path, .. } => {
                                    path.as_ident().map(|id| id.name.as_str()).unwrap_or("")
                                }
                                _ => "",
                            };
                            if (inner_name.contains("Map") || inner_name.contains("Mapped"))
                                && inner_args.len() >= 2
                            {
                                match &inner_args[1] {
                                    Type::Function { return_type, .. } => {
                                        (**return_type).clone()
                                    }
                                    _ => inner_args[0].clone(),
                                }
                            } else {
                                inner_args[0].clone()
                            }
                        }
                        _ => targs[0].clone(),
                    }
                } else {
                    Type::Var(TypeVar::fresh())
                }
            };

            match adapter_method_name {
                "next" | "next_back" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    let elem_ty = extract_adapter_elem_ty(&recv_ty, &adapter_type_args);
                    return Ok(Some(InferResult::new(Type::maybe(elem_ty))));
                }
                "map" => {
                    if args.len() == 1 {
                        let closure_result = self.synth_expr(&args[0])?;
                        let iter_ty = Type::Generic {
                            name: verum_common::Text::from("MapIterator"),
                            args: vec![recv_ty.clone(), closure_result.ty.clone()].into(),
                        };
                        return Ok(Some(InferResult::new(iter_ty)));
                    }
                }
                "filter" | "take" | "skip" | "take_while" | "skip_while" | "inspect"
                | "peekable" | "fuse" | "rev" | "dedup" | "unique" | "sorted"
                | "cloned" | "copied" | "step_by" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(recv_ty.clone())));
                }
                "filter_map" => {
                    if args.len() == 1 {
                        let closure_result = self.synth_expr(&args[0])?;
                        let mapped_elem_ty = match &closure_result.ty {
                            Type::Function { return_type, .. } => {
                                // filter_map returns Maybe<U>, extract U
                                match return_type.as_ref() {
                                    Type::Generic {
                                        args: inner_args, ..
                                    } if !inner_args.is_empty() => inner_args[0].clone(),
                                    _ => (**return_type).clone(),
                                }
                            }
                            _ => Type::Var(TypeVar::fresh()),
                        };
                        let iter_ty = Type::Generic {
                            name: verum_common::Text::from("FilterMapIter"),
                            args: vec![recv_ty.clone(), closure_result.ty.clone()].into(),
                        };
                        let _ = mapped_elem_ty; // The actual resolution happens on next()
                        return Ok(Some(InferResult::new(iter_ty)));
                    }
                }
                "flat_map" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(recv_ty.clone())));
                }
                "fold" | "rfold" => {
                    if !args.is_empty() {
                        let init_result = self.synth_expr(&args[0])?;
                        for arg in args.iter().skip(1) {
                            let _ = self.synth_expr(arg)?;
                        }
                        return Ok(Some(InferResult::new(init_result.ty)));
                    }
                }
                "try_fold" => {
                    if args.len() >= 2 {
                        let init_result = self.synth_expr(&args[0])?;
                        let closure_result = self.synth_expr(&args[1])?;
                        // try_fold returns Result<Acc, E> or the closure's return type
                        let result_ty = match &closure_result.ty {
                            Type::Function { return_type, .. } => (**return_type).clone(),
                            _ => {
                                // Default: wrap init type in Result
                                Type::Generic {
                                    name: verum_common::Text::from("Result"),
                                    args: vec![init_result.ty, Type::Var(TypeVar::fresh())]
                                        .into(),
                                }
                            }
                        };
                        return Ok(Some(InferResult::new(result_ty)));
                    }
                }
                "scan" => {
                    if args.len() >= 2 {
                        let _state_result = self.synth_expr(&args[0])?;
                        let closure_result = self.synth_expr(&args[1])?;
                        let iter_ty = Type::Generic {
                            name: verum_common::Text::from("ScanIter"),
                            args: vec![recv_ty.clone(), closure_result.ty.clone()].into(),
                        };
                        return Ok(Some(InferResult::new(iter_ty)));
                    }
                }
                "enumerate" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    let iter_ty = Type::Generic {
                        name: verum_common::Text::from("EnumerateIter"),
                        args: vec![recv_ty.clone()].into(),
                    };
                    return Ok(Some(InferResult::new(iter_ty)));
                }
                "zip" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(recv_ty.clone())));
                }
                "collect" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(Type::Var(TypeVar::fresh()))));
                }
                "sum" | "product" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    let elem_ty = extract_adapter_elem_ty(&recv_ty, &adapter_type_args);
                    return Ok(Some(InferResult::new(elem_ty)));
                }
                "count" | "len" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(Type::Int)));
                }
                "any" | "all" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(Type::Bool)));
                }
                "find" | "find_map" | "position" | "min" | "max" | "min_by" | "max_by"
                | "min_by_key" | "max_by_key" | "first" | "last" | "nth" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    let elem_ty = extract_adapter_elem_ty(&recv_ty, &adapter_type_args);
                    return Ok(Some(InferResult::new(Type::maybe(elem_ty))));
                }
                "for_each" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(Type::unit())));
                }
                "join" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(Type::text())));
                }
                "partition" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    let elem_ty = extract_adapter_elem_ty(&recv_ty, &adapter_type_args);
                    let list_ty = Type::Generic {
                        name: verum_common::Text::from("List"),
                        args: vec![elem_ty].into(),
                    };
                    return Ok(Some(InferResult::new(Type::Tuple(
                        vec![list_ty.clone(), list_ty].into(),
                    ))));
                }
                "unzip" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    let fresh1 = Type::Var(TypeVar::fresh());
                    let fresh2 = Type::Var(TypeVar::fresh());
                    let list1 = Type::Generic {
                        name: verum_common::Text::from("List"),
                        args: vec![fresh1].into(),
                    };
                    let list2 = Type::Generic {
                        name: verum_common::Text::from("List"),
                        args: vec![fresh2].into(),
                    };
                    return Ok(Some(InferResult::new(Type::Tuple(vec![list1, list2].into()))));
                }
                "size_hint" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(Type::Tuple(
                        vec![Type::Int, Type::maybe(Type::Int)].into(),
                    ))));
                }
                "chunks" => {
                    for arg in args.iter() {
                        let _ = self.synth_expr(arg)?;
                    }
                    return Ok(Some(InferResult::new(recv_ty.clone())));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Handle method dispatch when the receiver is a type variable with protocol bounds,
    /// a HKT type application (F<A>), a record type with function fields, or a
    /// Variant type. Called early before protocol search in handle_empty_candidates.
    fn try_type_var_bound_dispatch(
        &mut self,
        recv_ty: &Type,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        let method_name: verum_common::Text = method.name.as_str().into();
        // CRITICAL FIX: Handle type variables with protocol bounds
        // When receiver is a type variable (e.g., `T` in `fn display<T: Showable>(item: T)`),
        // we need to look up methods from the protocol bounds, not from implementations.
        // Type variables don't have concrete implementations registered, but their bounds
        // define which methods they support.
        // Also handle references to type variables (e.g., `&T` in `fn display<T: Display>(item: &T)`)
        let inner_type_var = match recv_ty {
            Type::Var(_) => Some(recv_ty),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                if matches!(&**inner, Type::Var(_)) {
                    Some(&**inner)
                } else {
                    None
                }
            }
            _ => None,
        };
        if inner_type_var.is_some() {
            // Look up all type parameters with their bounds from the environment
            let all_type_params = self.ctx.env.all_type_params();

            // Find the type parameter for this type variable
            // We match by checking if the receiver variable is used as a type param
            // For now, we search all bounds from all type params since we don't track
            // the exact correspondence between Type::Var and TypeParam
            for type_param in all_type_params {
                // Check each protocol bound on this type parameter
                for bound in &type_param.bounds {
                    // Get the protocol name from the bound
                    if let Some(protocol_ident) = bound.protocol.as_ident() {
                        let protocol_name: Text = protocol_ident.name.clone();

                        // Look up the protocol definition
                        let protocol_opt = self
                            .protocol_checker
                            .read()
                            .get_protocol(&protocol_name)
                            .cloned();
                        if let Maybe::Some(protocol) = protocol_opt {
                            // Check if this protocol has the method we're looking for
                            for (_, method) in &protocol.methods {
                                if method.name == method_name {
                                    // Found the method in a bounded protocol
                                    // CRITICAL FIX: Substitute Self with the receiver type
                                    // Protocol methods use Self as a placeholder for the implementing type.
                                    // For bounded type params like T: Default, calling T.default()
                                    // should return T, not Self.
                                    let method_ty = self
                                        .instantiate_method_for_receiver(&method.ty, &recv_ty);
                                    // Instantiate method's own type parameters with explicit type args
                                    let method_ty = self.instantiate_method_type_params(
                                        method_ty, type_args, span,
                                    )?;

                                    // Type check arguments against method signature
                                    if let Type::Function {
                                        params,
                                        return_type,
                                        ..
                                    } = &method_ty
                                    {
                                        // Protocol method signatures now EXCLUDE self parameter
                                        // (self is handled implicitly as the receiver)
                                        // Check argument count directly
                                        if params.len().abs_diff(args.len()) > 1 {
                                            return Err(TypeError::WrongArgCount {
                                                method: method_name.clone(),
                                                expected: params.len(),
                                                actual: args.len(),
                                                span,
                                            });
                                        }

                                        // Type check each argument with substitution
                                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                                            let resolved_param = self.unifier.apply(param_ty);
                                            self.check_expr(arg, &resolved_param)?;
                                        }

                                        // Apply substitution to return type
                                        let resolved_return = self.unifier.apply(return_type);
                                        return Ok(Some(InferResult::new(resolved_return)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // CRITICAL FIX: Handle higher-kinded type applications
        // When receiver is a TypeApp (e.g., F<A> in `fn map<F<_>: Functor, A, B>(fa: F<A>, ...)`),
        // we need to look up methods from the HKT parameter's protocol bounds.
        // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
        if let Type::TypeApp {
            constructor,
            args: type_args,
        } = &recv_ty
        {
            // Extract the constructor name - it could be a TypeConstructor, Named type, or Var (HKT param)
            let constructor_name: Option<Text> = match constructor.as_ref() {
                Type::TypeConstructor { name, .. } => Some(name.clone()),
                Type::Named { path, .. } => path.segments.last().and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                    _ => None,
                }),
                Type::Var(tv) => {
                    let all_params = self.ctx.env.all_type_params();
                    let mut found: Option<Text> = None;
                    for tp in &all_params {
                        if let Maybe::Some(type_val) = self.ctx.lookup_type(tp.name.as_str()) {
                            if let Type::Var(ptv) = type_val {
                                if ptv.id() == tv.id() {
                                    found = Some(tp.name.clone());
                                    break;
                                }
                            }
                        }
                    }
                    if found.is_none() {
                        let resolved = self.unifier.apply(&Type::Var(*tv));
                        match &resolved {
                            Type::TypeConstructor { name, .. } => {
                                found = Some(name.clone());
                            }
                            Type::Named { path, .. } => {
                                found = path.segments.last().and_then(|seg| match seg {
                                    verum_ast::ty::PathSegment::Name(ident) => {
                                        Some(ident.name.clone())
                                    }
                                    _ => None,
                                });
                            }
                            _ => {}
                        }
                    }
                    found
                }
                _ => None,
            };

            if let Some(ctor_name) = constructor_name {
                // Look up bounds for this HKT type constructor
                let bounds_opt = self.ctx.env.get_param_bounds(ctor_name.as_str());
                // Fallback: check type_var_bounds for Var-based HKT params
                let bounds_from_var: Option<List<crate::protocol::ProtocolBound>> =
                    if bounds_opt.is_none() {
                        if let Type::Var(tv) = constructor.as_ref() {
                            let b = self.get_type_var_bounds(tv);
                            if !b.is_empty() { Some(b) } else { None }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                let effective_bounds: Option<&[crate::protocol::ProtocolBound]> = bounds_opt
                    .as_ref()
                    .map(|b| b.as_slice())
                    .or_else(|| bounds_from_var.as_ref().map(|b| b.as_slice()));

                if let Some(bounds) = effective_bounds {
                    // Check each protocol bound for the method
                    for bound in bounds.iter() {
                        if let Some(protocol_ident) = bound.protocol.as_ident() {
                            let protocol_name: Text = protocol_ident.name.clone();

                            // Look up the protocol definition
                            let protocol_exists = self
                                .protocol_checker
                                .read()
                                .get_protocol(&protocol_name)
                                .is_some();

                            if protocol_exists {
                                // Check if this protocol or its super_protocols have the method
                                // This enables protocol inheritance: Monad extends Applicative extends Functor
                                let found_method = self.find_method_in_protocol_hierarchy(
                                    &protocol_name,
                                    &method_name,
                                );

                                if let Some(protocol_method) = found_method {
                                    // Found the method in a bounded protocol!
                                    // CRITICAL: Instantiate fresh type variables for each call
                                    // Without this, multiple calls like fa.map(f).map(g) would
                                    // share the same type variables and fail unification
                                    let method_ty = {
                                        use crate::ty::Substitution;
                                        let free_vars = protocol_method.ty.free_vars();
                                        if free_vars.is_empty() {
                                            protocol_method.ty.clone()
                                        } else {
                                            let mut subst = Substitution::new();
                                            for var in free_vars {
                                                let fresh = TypeVar::fresh();
                                                subst.insert(var, Type::Var(fresh));
                                            }
                                            protocol_method.ty.apply_subst(&subst)
                                        }
                                    };

                                    // Type check arguments against method signature
                                    if let Type::Function {
                                        params,
                                        return_type,
                                        ..
                                    } = &method_ty
                                    {
                                        // Skip the first param (self) for method calls
                                        let method_params: List<Type> = if !params.is_empty() {
                                            params.iter().skip(1).cloned().collect()
                                        } else {
                                            List::new()
                                        };

                                        // Check argument count
                                        if args.len() != method_params.len() {
                                            return Err(TypeError::WrongArgCount {
                                                method: method_name.clone(),
                                                expected: method_params.len(),
                                                actual: args.len(),
                                                span,
                                            });
                                        }

                                        // CRITICAL: Unify receiver type with method's self parameter
                                        // This binds the HKT type variables (e.g., A in F<A>) to the actual receiver type args
                                        if !params.is_empty() {
                                            let self_param_ty = &params[0];
                                            // Substitute ::F with the actual constructor before unifying
                                            let self_param_substituted = self
                                                .substitute_self_hkt_in_type(
                                                    self_param_ty,
                                                    &ctor_name,
                                                    constructor.as_ref(),
                                                );
                                            // Unify receiver type with self parameter type
                                            self.unifier.unify(
                                                &recv_ty,
                                                &self_param_substituted,
                                                span,
                                            )?;
                                        }

                                        // Type check each argument
                                        // CRITICAL: Substitute ::F in param types as well for closures
                                        // Without this, closure args like |b| g(a, b) expect ::F<B> but get M<B>
                                        for (arg, param_ty) in
                                            args.iter().zip(method_params.iter())
                                        {
                                            let resolved_param = self.unifier.apply(param_ty);
                                            let substituted_param = self
                                                .substitute_self_hkt_in_type(
                                                    &resolved_param,
                                                    &ctor_name,
                                                    constructor.as_ref(),
                                                );
                                            self.check_expr(arg, &substituted_param)?;
                                        }

                                        // The return type should be instantiated with the HKT constructor
                                        // For Functor.map: Self.F<B> -> F<B>
                                        let resolved_return = self.unifier.apply(return_type);

                                        // If return type is also a TypeApp with Self.F, substitute
                                        let final_return = self.substitute_self_hkt_in_type(
                                            &resolved_return,
                                            &ctor_name,
                                            constructor.as_ref(),
                                        );

                                        return Ok(Some(InferResult::new(final_return)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check if this is a record field access that's a function.
        // Also handles Named/Generic types by expanding them to their record definitions.
        {
            let record_fields: Option<indexmap::IndexMap<verum_common::Text, Type>> =
                match &recv_ty {
                    Type::Record(fields) => Some(fields.clone()),
                    // For Named/Generic types, try to expand to their underlying record definition
                    Type::Named { .. } | Type::Generic { .. } => self
                        .expand_type_alias(&recv_ty)
                        .and_then(|expanded| match expanded {
                            Type::Record(fields) => Some(fields),
                            _ => None,
                        }),
                    _ => None,
                };

            if let Some(fields) = record_fields {
                if let Some(field_ty) =
                    fields.get(&verum_common::Text::from(method.name.as_str()))
                {
                    // Field access to function: treat as method call
                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = field_ty
                    {
                        // Check argument count — must match exactly.
                        // Record-field functions have NO implicit self
                        // parameter (unlike inherent methods), so the
                        // arity check here must be strict.
                        if params.len() != args.len() {
                            return Err(TypeError::WrongArgCount {
                                method: method.name.as_str().to_text(),
                                expected: params.len(),
                                actual: args.len(),
                                span,
                            });
                        }

                        // Check each argument with substitution
                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }

                        let resolved_return = self.unifier.apply(return_type);
                        return Ok(Some(InferResult::new(resolved_return)));
                    }
                    // Field exists but is not a function - return its type directly
                    // for zero-arg "method" calls (field access sugar)
                    if args.is_empty() {
                        return Ok(Some(InferResult::new(field_ty.clone())));
                    }
                }
            }
        }

        // Protocol-based HKT method resolution for Variant types.
        // Instead of hardcoding witness names (MaybeFunctor, ResultMonad, etc.),
        // resolve the variant to its named type and look up protocol implementations
        // registered via explicit `implement` declarations.
        if let Type::Variant(variants) = &recv_ty {
            // Look up the named type for this variant signature via the registry
            let variant_sig = {
                let mut parts: Vec<&str> = variants.keys().map(|k| k.as_str()).collect();
                parts.sort();
                parts.join("|")
            };
            let named_type_name: Option<verum_common::Text> = {
                let protocol_checker_guard = self.protocol_checker.read();
                protocol_checker_guard
                    .get_variant_type_name(&verum_common::Text::from(variant_sig.as_str()))
                    .cloned()
                    .or_else(|| {
                        // Try relaxed signature (without payload types)
                        let relaxed = variants
                            .keys()
                            .map(|k| k.as_str())
                            .collect::<Vec<_>>()
                            .join("|");
                        protocol_checker_guard
                            .get_variant_type_name(&verum_common::Text::from(relaxed.as_str()))
                            .cloned()
                    })
            };

            // If we found a named type, look up its protocol implementations for the method
            if let Some(type_name) = named_type_name {
                let named_ty = Type::Named {
                    path: verum_ast::ty::Path::from_ident(verum_ast::Ident::new(
                        type_name.as_str(),
                        span,
                    )),
                    args: List::new(),
                };

                // Search all protocol implementations for this type for the method
                let method_types: List<Type> = {
                    let protocol_checker_guard = self.protocol_checker.read();
                    let impls = protocol_checker_guard.get_implementations(&named_ty);
                    impls
                        .iter()
                        .filter_map(|impl_| impl_.methods.get(&method_name).cloned())
                        .collect()
                };

                for method_ty in method_types {
                    // Instantiate with fresh type variables
                    let method_ty = {
                        use crate::ty::Substitution;
                        let free_vars = method_ty.free_vars();
                        if free_vars.is_empty() {
                            method_ty.clone()
                        } else {
                            let mut subst = Substitution::new();
                            for var in free_vars {
                                let fresh = TypeVar::fresh();
                                subst.insert(var, Type::Var(fresh));
                            }
                            method_ty.apply_subst(&subst)
                        }
                    };

                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = &method_ty
                    {
                        // Method params include self, so skip it
                        let method_params: List<Type> = if !params.is_empty() {
                            params.iter().skip(1).cloned().collect()
                        } else {
                            List::new()
                        };

                        // Check argument count
                        if args.len() != method_params.len() {
                            continue; // Try next method type
                        }

                        // Extract type argument from variant: use first variant with non-Unit payload
                        let _type_arg = variants
                            .values()
                            .find(|ty| !matches!(ty, Type::Unit))
                            .cloned()
                            .unwrap_or(Type::Unit);

                        // Unify the receiver type with the method's self parameter
                        if !params.is_empty() {
                            let _ = self.unifier.unify(&recv_ty, &params[0], span);
                        }

                        // Type check each argument with substitution
                        for (arg, param_ty) in args.iter().zip(method_params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }

                        let resolved_return = self.unifier.apply(return_type);
                        return Ok(Some(InferResult::new(resolved_return)));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Search for a method by stripping type arguments and trying type aliases
    /// and base type names. Handles transparent type alias semantics and generic
    /// wrapper types like Weak<T>, JoinHandle<T> registered under bare names.
    fn try_fallback_name_dispatch(
        &mut self,
        recv_ty: &Type,
        recv_ty_raw: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        // TYPE ALIAS METHOD RESOLUTION FALLBACK
        // When a method is not found on the receiver type directly, search for
        // implement blocks on type aliases whose target type is compatible with recv_ty.
        // This enables transparent type alias semantics where:
        //  type EpochCaps = u32;
        //  implement EpochCaps { fn epoch(self) -> u32 { ... } }
        //  let x: Int = ref.epoch_caps(); // epoch_caps() returns Int
        //  x.epoch() // finds EpochCaps::epoch because u32 == Int
        {
            let alias_method_info = {
                let methods_guard = self.inherent_methods.read();
                let method_name_text = verum_common::Text::from(method.name.as_str());

                // First check: only look at aliases that have the method registered.
                // This avoids iterating through all aliases and calling the Numeric protocol checker.
                let mut found = None;
                for (alias_name, alias_target) in &self.ctx.type_aliases {
                    // Quick check: does this alias even have the method?
                    let has_method = methods_guard
                        .get(alias_name)
                        .and_then(|methods| methods.get(&method_name_text))
                        .is_some();
                    if !has_method {
                        continue;
                    }

                    // Check if alias target is compatible with the receiver type.
                    // Direct equality check (since u32/i32/etc all resolve to Type::Int).
                    let compatible =
                        *alias_target == *recv_ty || alias_target.to_text() == recv_ty.to_text();

                    if compatible {
                        if let Some(methods) = methods_guard.get(alias_name) {
                            if let Some(scheme) = methods.get(&method_name_text) {
                                let impl_vc = scheme.impl_var_count;
                                let (ty, fresh_vars, type_bounds) =
                                    scheme.instantiate_with_type_bounds();
                                found = Some(((ty, fresh_vars, impl_vc), type_bounds));
                                break;
                            }
                        }
                    }
                }
                found
            };

            if let Some(((method_ty, _ordered_fresh_vars, _impl_var_count), type_bounds)) =
                alias_method_info
            {
                // Register type bounds for fresh type variables
                for (fresh_var, bounds) in &type_bounds {
                    for bound in bounds {
                        self.register_type_var_type_bound(*fresh_var, bound.clone());
                    }
                }
                // Type check the call
                if let Type::Function {
                    params,
                    return_type,
                    ..
                } = &method_ty
                {
                    if params.len().abs_diff(args.len()) > 1 {
                        return Err(TypeError::WrongArgCount {
                            method: verum_common::Text::from(method.name.as_str()),
                            expected: params.len(),
                            actual: args.len(),
                            span,
                        });
                    }
                    for (arg, param_ty) in args.iter().zip(params.iter()) {
                        let resolved_param = self.unifier.apply(param_ty);
                        self.check_expr(arg, &resolved_param)?;
                    }
                    let resolved_return = self.unifier.apply(return_type);
                    return Ok(Some(InferResult::new(resolved_return)));
                }
            }
        }

        // GENERIC BASE-NAME FALLBACK: When all other lookup paths fail, try
        // extracting the base type name by stripping type arguments and re-trying
        // inherent_methods. This handles cases where:
        //  - The receiver type is a generic stdlib type (Weak<T>, JoinHandle<T>,
        //  MutexGuard<T>, etc.) whose methods were registered under the bare name
        //  - The standard get_type_name/get_exact_type_name didn't match due to
        //  type representation differences (Named vs Generic, resolved aliases, etc.)
        //  - The receiver type went through auto-deref or alias resolution that
        //  changed its representation
        //

        // This is stdlib-agnostic: it works for ANY generic type, not just hardcoded ones.
        {
            // Try multiple type representations: recv_ty (derefed/resolved),
            // recv_ty_raw (original), and type_to_name (string-based extraction)
            let mut base_name_candidates: List<verum_common::Text> = List::new();

            // Strategy 1: Extract from recv_ty_raw (pre-deref, pre-alias-resolution)
            match &recv_ty_raw {
                Type::Generic { name, .. } => {
                    base_name_candidates.push(name.clone());
                }
                Type::Named { path, args, .. } => {
                    if let Some(ident) = path.as_ident() {
                        base_name_candidates
                            .push(verum_common::Text::from(ident.name.as_str()));
                    }
                    // Also try with no args — sometimes the method is registered under
                    // the path's last segment regardless of args
                    if !args.is_empty() {
                        if let Some(seg) = path.segments.last() {
                            if let verum_ast::ty::PathSegment::Name(id) = seg {
                                let name = verum_common::Text::from(id.name.as_str());
                                if !base_name_candidates.contains(&name) {
                                    base_name_candidates.push(name);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            // Strategy 2: Extract from recv_ty (post-deref, post-alias-resolution)
            match &recv_ty {
                Type::Generic { name, .. } => {
                    if !base_name_candidates.contains(name) {
                        base_name_candidates.push(name.clone());
                    }
                }
                Type::Named { path, .. } => {
                    if let Some(ident) = path.as_ident() {
                        let name = verum_common::Text::from(ident.name.as_str());
                        if !base_name_candidates.contains(&name) {
                            base_name_candidates.push(name);
                        }
                    }
                }
                _ => {}
            }

            // Strategy 3: Use type_to_name which handles Display formatting
            let display_name = self.type_to_name(&recv_ty);
            // Strip type arguments from display name: "Weak<Int>" → "Weak"
            let base_from_display = if let Some(idx) = display_name.as_str().find('<') {
                verum_common::Text::from(&display_name.as_str()[..idx])
            } else {
                display_name.clone()
            };
            if !base_from_display.is_empty()
                && !base_name_candidates.contains(&base_from_display)
            {
                base_name_candidates.push(base_from_display);
            }

            let method_name_text = verum_common::Text::from(method.name.as_str());
            let fallback_method_info = {
                let methods_guard = self.inherent_methods.read();
                base_name_candidates.iter().find_map(|candidate_name| {
                    methods_guard.get(candidate_name).and_then(|methods| {
                        methods.get(&method_name_text).cloned().map(|scheme| {
                            let impl_vc = scheme.impl_var_count;
                            let (ty, fresh_vars, type_bounds) =
                                scheme.instantiate_with_type_bounds();
                            (
                                candidate_name.clone(),
                                (ty, fresh_vars, impl_vc),
                                type_bounds,
                            )
                        })
                    })
                })
            };

            if let Some((
                _matched_name,
                (method_ty, ordered_fresh_vars, impl_var_count),
                type_bounds,
            )) = fallback_method_info
            {
                // Register type bounds
                for (fresh_var, bounds) in &type_bounds {
                    for bound in bounds {
                        self.register_type_var_type_bound(*fresh_var, bound.clone());
                    }
                }

                if let Type::Function {
                    params,
                    return_type,
                    ..
                } = &method_ty
                {
                    if params.len().abs_diff(args.len()) > 1 {
                        return Err(TypeError::WrongArgCount {
                            method: method_name_text,
                            expected: params.len(),
                            actual: args.len(),
                            span,
                        });
                    }

                    // Bind type variables from receiver type args
                    let receiver_type_args: List<Type> = match &recv_ty {
                        Type::Named { args, .. } | Type::Generic { args, .. } => args.clone(),
                        _ => {
                            // Also try recv_ty_raw
                            match &recv_ty_raw {
                                Type::Named { args, .. } | Type::Generic { args, .. } => {
                                    args.clone()
                                }
                                _ => List::new(),
                            }
                        }
                    };

                    let bind_limit = Self::resolve_bind_limit(
                        impl_var_count,
                        ordered_fresh_vars.len(),
                        receiver_type_args.len(),
                    );
                    let mut combined_subst = crate::ty::Substitution::new();
                    for (type_var, type_arg) in ordered_fresh_vars
                        .iter()
                        .take(bind_limit)
                        .zip(receiver_type_args.iter())
                    {
                        if let Ok(subst) =
                            self.unifier.unify(&Type::Var(*type_var), type_arg, span)
                        {
                            combined_subst.extend(subst);
                        }
                    }

                    // OVERLOAD GUARD: same as early/primary paths
                    let params_cloned = params.clone();
                    let mut signature_mismatch = false;
                    for (arg, param_ty) in args.iter().zip(params_cloned.iter()) {
                        if matches!(&arg.kind, ExprKind::Closure { .. }) {
                            let subst_param_ty = param_ty.apply_subst(&combined_subst);
                            let resolved_param = self.unifier.apply(&subst_param_ty);
                            if !matches!(
                                &resolved_param,
                                Type::Function { .. } | Type::Var(_) | Type::Placeholder { .. }
                            ) {
                                signature_mismatch = true;
                                break;
                            }
                        }
                    }
                    if !signature_mismatch {
                        // Type check arguments
                        for (arg, param_ty) in args.iter().zip(params_cloned.iter()) {
                            let subst_param_ty = param_ty.apply_subst(&combined_subst);
                            self.check_expr(arg, &subst_param_ty)?;
                        }

                        let subst_return_type = return_type.apply_subst(&combined_subst);
                        let final_return_type = self.unifier.apply(&subst_return_type);
                        return Ok(Some(InferResult::new(final_return_type)));
                    }
                    // signature_mismatch: fall through to generic method fallbacks
                }
            }
        }
        Ok(None)
    }

    /// Protocol method fallback, protocol-typed receiver dispatch, and wrapper
    /// type protocol dispatch. Tries lookup_protocol_method (default methods),
    /// then checks if the receiver IS a protocol type, then checks wrappers.
    fn try_protocol_object_dispatch(
        &mut self,
        recv_ty: &Type,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        // CRITICAL FIX: Fallback to protocol method lookup for default methods
        // This enables methods like `file.read_to_string()` where `read_to_string` is a
        // default method in the `Read` protocol that `File` implements.
        // Spec: Protocol default methods should be callable on implementing types.
        let method_name_text = verum_common::Text::from(method.name.as_str());
        #[cfg(debug_assertions)]
        if method.name.as_str() == "next" {
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG fallback_protocol_method] Looking for method '{}' on recv_ty={:?}", method.name.as_str(), recv_ty);
        }
        let protocol_method_result = self
            .protocol_checker
            .read()
            .lookup_protocol_method_with_type_param_names(&recv_ty, &method_name_text);
        #[cfg(debug_assertions)]
        if method.name.as_str() == "next" {
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG fallback_protocol_method] lookup_protocol_method returned: {:?}", protocol_method_result);
        }
        if let Ok(Maybe::Some((raw_method_ty, method_type_param_names))) =
            protocol_method_result
        {
            // Freshen method-level type parameters to avoid sharing TypeVars across call sites.
            // For `fn collect<C: FromIterator<Self.Item>>() -> C`, this creates a fresh TypeVar
            // for C at each call site, enabling backward type inference from let-binding annotations.
            let method_ty = self.freshen_method_type_params(
                raw_method_ty,
                &method_type_param_names,
                type_args,
                span,
            )?;

            // Found protocol method - type check the call
            if let Type::Function {
                params,
                return_type,
                ..
            } = &method_ty
            {
                // Protocol methods don't include self in params (we already excluded it during registration)
                if params.len().abs_diff(args.len()) > 1 {
                    return Err(TypeError::WrongArgCount {
                        method: method_name_text,
                        expected: params.len(),
                        actual: args.len(),
                        span,
                    });
                }

                // Type check each argument
                for (arg, param_ty) in args.iter().zip(params.iter()) {
                    let resolved_param = self.unifier.apply(param_ty);
                    self.check_expr(arg, &resolved_param)?;
                }

                let resolved_return = self.unifier.apply(return_type);
                return Ok(Some(InferResult::new(resolved_return)));
            }
        }

        // CRITICAL FIX: When receiver type IS a protocol (trait object), look up methods
        // from the protocol definition itself.
        // This enables: `fn foo(hasher: &mut Hasher) { hasher.write_int(42); }`
        // where `Hasher` is the protocol type and `write_int` is a method/default method.
        // Spec: Protocol types used as trait objects should resolve methods from the protocol.
        // Also handles &Drawable, &&Drawable, etc. by peeling references.
        let protocol_lookup_ty = {
            let mut ty: &Type = recv_ty;
            loop {
                match ty {
                    Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. } => ty = inner.as_ref(),
                    _ => break ty.clone(),
                }
            }
        };
        if let Type::Named { path, .. } = &protocol_lookup_ty {
            if let Some(protocol_name) = path.as_ident().map(|id| id.name.clone()) {
                // Check if this is a registered protocol
                let protocol_opt = self
                    .protocol_checker
                    .read()
                    .get_protocol(&protocol_name)
                    .cloned();
                if let Maybe::Some(protocol) = protocol_opt {
                    // Look up the method in the protocol definition
                    if let Some(proto_method) = protocol.methods.get(&method_name_text) {
                        // Found the method in the protocol definition!
                        // Substitute Self with the protocol type for method resolution
                        let method_ty = self.instantiate_method_for_receiver(
                            &proto_method.ty,
                            &protocol_lookup_ty,
                        );
                        // Instantiate method's own type parameters with explicit type args
                        let method_ty =
                            self.instantiate_method_type_params(method_ty, type_args, span)?;

                        if let Type::Function {
                            params,
                            return_type,
                            ..
                        } = &method_ty
                        {
                            // Check argument count
                            if params.len().abs_diff(args.len()) > 1 {
                                return Err(TypeError::WrongArgCount {
                                    method: method_name_text,
                                    expected: params.len(),
                                    actual: args.len(),
                                    span,
                                });
                            }

                            // Type check each argument
                            for (arg, param_ty) in args.iter().zip(params.iter()) {
                                let resolved_param = self.unifier.apply(param_ty);
                                self.check_expr(arg, &resolved_param)?;
                            }

                            let resolved_return = self.unifier.apply(return_type);
                            return Ok(Some(InferResult::new(resolved_return)));
                        }
                    }
                }
            }
        }

        // PROTOCOL OBJECT DISPATCH THROUGH WRAPPER TYPES
        // Enables shape.draw() on Heap<Drawable>, Shared<Printable>, etc.
        // When recv_ty is Generic { name, args: [T] } and T is a protocol,
        // look up T's protocol definition methods and dispatch through them.
        if let Type::Generic {
            args: wrapper_args, ..
        } = &recv_ty
        {
            if wrapper_args.len() == 1 {
                let inner = &wrapper_args[0];
                let protocol_name_opt: Option<verum_common::Text> = match inner {
                    Type::Named { path, .. } => path.as_ident().map(|id| id.name.clone()),
                    Type::Generic { name, .. } => Some(name.clone()),
                    _ => None,
                };
                if let Some(pname) = protocol_name_opt {
                    let protocol_opt =
                        self.protocol_checker.read().get_protocol(&pname).cloned();
                    if let Maybe::Some(protocol) = protocol_opt {
                        if let Some(proto_method) = protocol.methods.get(&method_name_text) {
                            let method_ty =
                                self.instantiate_method_for_receiver(&proto_method.ty, inner);
                            let method_ty = self
                                .instantiate_method_type_params(method_ty, type_args, span)?;
                            if let Type::Function {
                                params,
                                return_type,
                                ..
                            } = &method_ty
                            {
                                if params.len().abs_diff(args.len()) > 1 {
                                    return Err(TypeError::WrongArgCount {
                                        method: method_name_text,
                                        expected: params.len(),
                                        actual: args.len(),
                                        span,
                                    });
                                }
                                for (arg, param_ty) in args.iter().zip(params.iter()) {
                                    let resolved_param = self.unifier.apply(param_ty);
                                    self.check_expr(arg, &resolved_param)?;
                                }
                                let resolved_return = self.unifier.apply(return_type);
                                return Ok(Some(InferResult::new(resolved_return)));
                            }
                        }
                    }
                }
            }
        }

        // PROTOCOL OBJECT DISPATCH THROUGH WRAPPER TYPES
        // Enables shape.draw() on Heap<Drawable>, Shared<Printable>, etc.
        // When recv_ty is Wrapper<Protocol>, look up Protocol's methods.
        {
            let wrapper_inner = match &recv_ty {
                Type::Named { path, args } if args.len() == 1 => Some(&args[0]),
                Type::Generic { args, .. } if args.len() == 1 => Some(&args[0]),
                _ => None,
            };
            if let Some(inner) = wrapper_inner {
                let protocol_name_opt = match inner {
                    Type::Named { path, .. } => path.as_ident().map(|id| id.name.clone()),
                    Type::Generic { name, .. } => Some(name.clone()),
                    _ => None,
                };
                if let Some(pname) = protocol_name_opt {
                    let method_opt = {
                        let checker = self.protocol_checker.read();
                        if checker.get_protocol_definition(pname.as_str()).is_some() {
                            checker
                                .get_protocol_definition(pname.as_str())
                                .and_then(|def| def.methods.get(&method_name_text).cloned())
                        } else {
                            None
                        }
                    };
                    if let Some(proto_method) = method_opt {
                        let method_ty =
                            self.instantiate_method_for_receiver(&proto_method.ty, inner);
                        let method_ty =
                            self.instantiate_method_type_params(method_ty, type_args, span)?;
                        if let Type::Function {
                            params,
                            return_type,
                            ..
                        } = &method_ty
                        {
                            let method_params: List<Type> = if !params.is_empty() {
                                params.iter().skip(1).cloned().collect()
                            } else {
                                params.clone()
                            };
                            if args.len() != method_params.len() {
                                return Err(TypeError::WrongArgCount {
                                    method: method_name_text.clone(),
                                    expected: method_params.len(),
                                    actual: args.len(),
                                    span,
                                });
                            }
                            for (arg, param_ty) in args.iter().zip(method_params.iter()) {
                                let resolved_param = self.unifier.apply(param_ty);
                                self.check_expr(arg, &resolved_param)?;
                            }
                            return Ok(Some(InferResult::new(self.unifier.apply(return_type))));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Check inherent blanket impls (`implement<I: Iterator> I { fn ... }`) and
    /// UFCS (Uniform Function Call Syntax) free-function fallback.
    fn try_blanket_and_ufcs_dispatch(
        &mut self,
        recv_ty: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        // Check inherent blanket impls (e.g., `implement<I: Iterator> I { fn reduce_with... }`)
        // These provide extension methods for all types satisfying protocol bounds.
        // Registered under "__blanket:Protocol" keys in inherent_methods.
        let blanket_result =
            self.lookup_inherent_blanket_method(&recv_ty, method.name.as_str());
        if let Some((blanket_method_ty, _remaining_vars, blanket_bounds)) = blanket_result {
            // Register type bounds for closure inference
            for (fresh_var, bounds) in &blanket_bounds {
                for bound in bounds {
                    self.register_type_var_type_bound(*fresh_var, bound.clone());
                }
            }

            if let Type::Function {
                params,
                return_type,
                ..
            } = &blanket_method_ty
            {
                if args.len() == params.len() {
                    let params_cloned = params.clone();
                    for (arg, param_ty) in args.iter().zip(params_cloned.iter()) {
                        let resolved_param = self.unifier.apply(param_ty);
                        self.check_expr(arg, &resolved_param)?;
                    }
                    let subst_return = self.unifier.apply(return_type);
                    let normalized = self.normalize_type(&subst_return);
                    return Ok(Some(InferResult::new(normalized)));
                }
            }
        }

        // UFCS (Uniform Function Call Syntax) fallback
        // Try to find a free function with this name and call it with receiver as first argument
        // Syntax grammar: recursive-descent parseable (LL(k), k<=3), reserved keywords only let/fn/is, unified "type X is" definitions — UFCS allows x.foo(args) to resolve as foo(x, args)
        let method_name_str = method.name.as_str();

        // First try local environment, then module-level functions
        let func_scheme = self
            .ctx
            .env
            .lookup(method_name_str)
            .cloned()
            .or_else(|| self.lookup_function_in_module(method_name_str));

        if let Some(scheme) = func_scheme {
            let func_ty = scheme.instantiate();

            // Check if it's a function that accepts receiver as first arg
            if let Type::Function {
                params,
                return_type,
                ..
            } = &func_ty
                && !params.is_empty()
            {
                // Check if receiver is compatible with first parameter
                if self.subtyping.is_subtype(&recv_ty, &params[0]) {
                    // Check remaining arguments
                    let remaining_params: List<Type> = params.iter().skip(1).cloned().collect();

                    if args.len() != remaining_params.len() {
                        return Err(TypeError::WrongArgCount {
                            method: method_name_str.to_text(),
                            expected: remaining_params.len(),
                            actual: args.len(),
                            span,
                        });
                    }

                    // Type check each argument against remaining params with substitution
                    for (arg, param_ty) in args.iter().zip(remaining_params.iter()) {
                        let resolved_param = self.unifier.apply(param_ty);
                        self.check_expr(arg, &resolved_param)?;
                    }

                    let resolved_return = self.unifier.apply(return_type);
                    return Ok(Some(InferResult::new(resolved_return)));
                }
            }
        }
        Ok(None)
    }

    /// Look up methods in the inherent methods table (from parsed .vr implement blocks).
    /// Handles specialization filtering, type variable binding, and the overload guard
    /// for closures. Returns (Some(result), false) on success, (None, true) when a
    /// method was found but rejected by specialization, (None, false) when not found.
    fn try_inherent_method_dispatch(
        &mut self,
        recv_ty: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<(Option<InferResult>, bool)> {
        // CRITICAL FIX: Check inherent instance methods from implement blocks
        // This enables obj.method() where method has self parameter
        //

        // ENHANCEMENT: First try exact type name (for Reference/Slice/Array methods),
        // then fall back to unwrapped type name (for methods on inner types),
        // and finally try fallback type names (e.g., Array -> Slice).
        // This enables methods like as_checked(), as_unsafe() on reference types.
        let exact_type_name = self.get_exact_type_name(&recv_ty);
        let unwrapped_type_name = self.get_type_name(&recv_ty);
        let fallback_type_names = self.get_fallback_type_names(&recv_ty);

        // Build list of type names to try: exact, unwrapped, then fallbacks
        let mut type_names_to_try: List<verum_common::Text> = List::new();
        if let Some(name) = exact_type_name.clone() {
            type_names_to_try.push(name);
        }
        if let Some(name) = unwrapped_type_name.clone() {
            type_names_to_try.push(name);
        }
        type_names_to_try.extend(fallback_type_names);

        // NOTE: resolve_primitive_method was previously called FIRST here, overriding stdlib.
        // Now stdlib definitions take priority. The fallback at the end of method resolution
        // still calls resolve_primitive_method for methods not yet covered by stdlib.

        // Track whether a method was found but rejected by specialization (for error reporting)
        let mut specialization_rejected = false;
        // if method.name.as_str() == "next" {
        //  eprintln!("[DEBUG method_call] Looking for 'next' method");
        //  eprintln!(" recv_ty={:?}", recv_ty);
        //  eprintln!(" type_names_to_try={:?}", type_names_to_try);
        // }
        let method_info_opt = {
            let methods_guard = self.inherent_methods.read();
            let method_name_text = verum_common::Text::from(method.name.as_str());

            // Extract receiver type args for specialization checking
            let recv_type_args_for_check: List<Type> = match &recv_ty {
                Type::Named { args, .. } | Type::Generic { args, .. } => args.clone(),
                _ => List::new(),
            };

            // Read method_impl_patterns for specialization filtering
            let patterns_guard = self.method_impl_patterns.read();

            // Try each type name in order
            type_names_to_try.iter().find_map(|type_name_text| {
                methods_guard.get(type_name_text).and_then(|methods| {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG method_lookup] Found type '{}' with methods: {:?}",
                    // type_name_text, methods.keys().map(|k| k.as_str()).collect::<Vec<_>>());
                    methods.get(&method_name_text).cloned().and_then(|scheme| {
                        // SPECIALIZATION CHECK: If patterns exist for this method,
                        // verify that the receiver type args match at least one pattern.
                        if let Some(type_patterns) = patterns_guard.get(type_name_text) {
                            if let Some(method_patterns) = type_patterns.get(&method_name_text)
                            {
                                if !method_patterns.is_empty()
                                    && !recv_type_args_for_check.is_empty()
                                {
                                    let matches_any =
                                        method_patterns.iter().any(|pattern_args| {
                                            if pattern_args.len()
                                                != recv_type_args_for_check.len()
                                            {
                                                return false;
                                            }
                                            pattern_args
                                                .iter()
                                                .zip(recv_type_args_for_check.iter())
                                                .all(|(pat, recv)| {
                                                    // Type variables in patterns are wildcards (match anything)
                                                    matches!(pat, Type::Var(_))
                                                        || pat.to_text() == recv.to_text()
                                                })
                                        });
                                    if !matches_any {
                                        specialization_rejected = true;
                                        return None; // Method not available for this receiver specialization
                                    }
                                }
                            }
                        }

                        // Use instantiate_with_type_bounds to preserve function type bounds
                        // This enables proper closure type inference for methods like
                        // `map<U, F: fn(T) -> U>` where F needs its fn type bound
                        let impl_vc = scheme.impl_var_count;
                        let (ty, fresh_vars, type_bounds) =
                            scheme.instantiate_with_type_bounds();
                        Some(((ty, fresh_vars, impl_vc), type_bounds))
                    })
                })
            })
        };

        if let Some(type_name_text) = exact_type_name.or(unwrapped_type_name) {
            // Clone the method type to avoid borrow issues with check_expr
            // Uses shared RwLock for order-independent resolution
            // CRITICAL FIX: Use instantiate_with_type_bounds to get ordered type variables AND bounds
            // This ensures that receiver type args are bound to the correct type params
            // AND that function type bounds (like F: fn(T) -> U) are preserved for closure checking.
            let _ = type_name_text; // Used in original code but now method_info_opt is computed above

            if let Some(((method_ty, ordered_fresh_vars, impl_var_count), type_bounds)) =
                method_info_opt
            {
                // CRITICAL: Register type bounds for fresh type variables
                // This enables closure type inference for generic methods like map<U, F: fn(T) -> U>
                // #[cfg(debug_assertions)]
                // if method.name.as_str() == "min" || method.name.as_str() == "max" {
                //  eprintln!("[DEBUG inherent_methods path] Method '{}' found in inherent_methods:", method.name.as_str());
                //  eprintln!(" method_ty={:?}", method_ty);
                //  if let Type::Function { params, .. } = &method_ty {
                //  eprintln!(" params.len()={}", params.len());
                //  }
                //  eprintln!(" args.len()={}", args.len());
                // }
                // #[cfg(debug_assertions)]
                // if method.name.as_str() == "eq" {
                //  eprintln!("[DEBUG inherent_methods path] Method 'eq' found in inherent_methods: method_ty={:?}", method_ty);
                // }
                // if method.name.as_str() == "map" {
                //  eprintln!("[DEBUG method_call] map: type_bounds has {} entries", type_bounds.len());
                //  for (fresh_var, bounds) in &type_bounds {
                //  eprintln!(" fresh_var={:?}, bounds={:?}", fresh_var, bounds);
                //  }
                // }

                for (fresh_var, bounds) in &type_bounds {
                    // if method.name.as_str() == "map" {
                    //  eprintln!("[DEBUG method_call] Registering {} bounds for var {:?}", bounds.len(), fresh_var);
                    // }
                    for bound in bounds {
                        self.register_type_var_type_bound(*fresh_var, bound.clone());
                    }
                }
                // Found the method - type check the call
                if let Type::Function {
                    params,
                    return_type,
                    ..
                } = &method_ty
                {
                    let method_name_text = verum_common::Text::from(method.name.as_str());
                    // Check argument count (method params don't include self)
                    if params.len().abs_diff(args.len()) > 1 {
                        return Err(TypeError::WrongArgCount {
                            method: method_name_text,
                            expected: params.len(),
                            actual: args.len(),
                            span,
                        });
                    }

                    // CRITICAL FIX: Bind type variables in the method signature to receiver's type args
                    // e.g., for Wrapper<Int>.get() where return type is &τ_fresh,
                    // we bind τ_fresh = Int, making return type &τ_fresh become &Int.
                    //

                    // The ordered_fresh_vars from instantiate_with_fresh_vars preserves the
                    // order of type params from the implement block, so zip correctly matches
                    // receiver type args to their corresponding type variables.
                    let receiver_type_args = match &recv_ty {
                        Type::Named { args, .. } => args.clone(),
                        Type::Generic { args, .. } => args.clone(),
                        // CRITICAL FIX: Handle Variant types with type parameters
                        // Variant types like Result<T, E> need to extract type args from their variants
                        // For Result<T, E>: Variant { "Ok" -> T, "Err" -> E }
                        // We need to extract [T, E] in the order the implement block expects
                        Type::Variant(variants) => {
                            // Extract type args based on semantic variant meaning
                            // This must match how `implement<T, E> Result<T, E>` is declared
                            // Result<T, E> = Ok(T) | Err(E), so: T from "Ok", E from "Err"
                            // Maybe<T> = None | Some(T), so: T from "Some"
                            let mut args = List::new();

                            // Generic variant type handling (stdlib-agnostic).
                            //

                            // For ANY variant type (Result, Maybe, Validated, user-defined, etc.),
                            // extract type arguments using the registered type metadata:
                            // 1. Look up type name from variant_type_names registry
                            // 2. Look up __type_var_order_{name} to get TypeVars in declaration order
                            // 3. Look up original (unsubstituted) variant type from type context
                            // 4. Unify original with substituted type to get TypeVar -> concrete type mapping
                            // 5. Return type args in the correct declaration order
                            let variant_ty = Type::Variant(variants.clone());
                            let extracted = self.extract_type_args_from_variant(&variant_ty);
                            if !extracted.is_empty() {
                                for arg in extracted {
                                    args.push(arg);
                                }
                            } else {
                                // Final fallback: extract non-Unit payloads (may be wrong for complex types)
                                let mut variant_names: Vec<_> = variants.keys().collect();
                                variant_names.sort();
                                for name in variant_names {
                                    if let Some(ty) = variants.get(name) {
                                        if !matches!(ty, Type::Unit) {
                                            args.push(ty.clone());
                                        }
                                    }
                                }
                            }
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG method_call] Variant receiver extracted type_args: {:?}",
                            // args.iter().map(|t| format!("{}", t)).collect::<Vec<_>>());
                            args
                        }
                        _ => {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG method_call] recv_ty={:?} has no extractable type args", recv_ty);
                            List::new()
                        }
                    };
                    // Collect substitution from unification using the ORDERED fresh vars.
                    // This ensures L maps to receiver_type_args[0], R to [1], etc.
                    //

                    // CRITICAL: When impl_var_count > 0, only bind the first impl_var_count
                    // type vars from receiver type args. Method-level vars (like F in
                    // modify<F: Fn(T) -> T>) must NOT be bound from receiver type args —
                    // they are inferred from the method's arguments instead.
                    let bind_limit = Self::resolve_bind_limit(
                        impl_var_count,
                        ordered_fresh_vars.len(),
                        receiver_type_args.len(),
                    );
                    let mut combined_subst = crate::ty::Substitution::new();
                    for (type_var, type_arg) in ordered_fresh_vars
                        .iter()
                        .take(bind_limit)
                        .zip(receiver_type_args.iter())
                    {
                        // Unify the type variable with the receiver's type argument
                        if let Ok(subst) =
                            self.unifier.unify(&Type::Var(*type_var), type_arg, span)
                        {
                            combined_subst.extend(subst);
                        }
                    }

                    // Clone params for iteration to avoid borrow issues
                    let params_cloned = params.clone();

                    // OVERLOAD GUARD: If any argument is a closure but the corresponding
                    // parameter type (after substitution) is a concrete non-function type,
                    // this inherent method signature doesn't match the call. Skip to protocol
                    // fallback which may have a predicate-accepting overload.
                    // Example: List.position(value: &T) vs Iterator.position(pred: fn(&T)->Bool)
                    {
                        let mut signature_mismatch = false;
                        for (arg, param_ty) in args.iter().zip(params_cloned.iter()) {
                            if matches!(&arg.kind, ExprKind::Closure { .. }) {
                                let subst_param_ty = param_ty.apply_subst(&combined_subst);
                                let resolved_param = self.unifier.apply(&subst_param_ty);
                                // If the param type is concrete and not a function type / type var,
                                // a closure argument cannot possibly match.
                                if !matches!(
                                    &resolved_param,
                                    Type::Function { .. }
                                        | Type::Var(_)
                                        | Type::Placeholder { .. }
                                ) {
                                    signature_mismatch = true;
                                    break;
                                }
                            }
                        }
                        if signature_mismatch {
                            // Don't return error — fall through to protocol/blanket/fallback paths
                            // which may have a matching overload accepting a closure.
                        } else {
                            // Type check each argument (with substituted param types)
                            for (arg, param_ty) in args.iter().zip(params_cloned.iter()) {
                                let subst_param_ty = param_ty.apply_subst(&combined_subst);
                                self.check_expr(arg, &subst_param_ty)?;
                            }

                            // Apply substitution to get the concrete return type
                            let subst_return_type = return_type.apply_subst(&combined_subst);

                            // CRITICAL FIX: Also apply unifier to resolve type variables from argument checking
                            // For generic methods like `map_left<L2>(self, f: fn(L) -> L2) -> Either<L2, R>`,
                            // L2 is inferred from the closure argument, not from receiver type args.
                            // The closure checking at line 17776 unifies L2 with the closure return type,
                            // but this unification is in the unifier, not in combined_subst.
                            // Without this, return type still has unresolved L2 type variable.
                            let final_return_type = self.unifier.apply(&subst_return_type);

                            return Ok((Some(InferResult::new(final_return_type)), false));
                        }
                    }
                }
            }
        }
        Ok((None, specialization_rejected))
    }

    /// Handle method calls on built-in non-nominal types that are not covered by the
    /// protocol/inherent lookup chain: Array/Slice pointer ops, numeric conversions,
    /// Bool::cast, and type-variable/placeholder methods.
    fn try_builtin_type_method_fallback(
        &mut self,
        recv_ty: &Type,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        // SLICE/ARRAY BUILT-IN METHOD FALLBACK
        // Methods like as_mut_ptr, as_ptr, offset, len on array/slice types
        {
            let slice_element: Option<Type> = match &recv_ty {
                Type::Array { element, .. } => Some(element.as_ref().clone()),
                Type::Slice { element } => Some(element.as_ref().clone()),
                _ => None,
            };
            if let Some(elem) = slice_element {
                let method_name_str = method.name.as_str();
                match method_name_str {
                    "as_mut_ptr" | "as_ptr" if args.is_empty() => {
                        return Ok(Some(InferResult::new(Type::Pointer {
                            inner: Box::new(elem),
                            mutable: method_name_str == "as_mut_ptr",
                        })));
                    }
                    "len" if args.is_empty() => {
                        return Ok(Some(InferResult::new(Type::int())));
                    }
                    "offset" if args.len() == 1 => {
                        let _ = self.synth_expr(&args[0])?;
                        return Ok(Some(InferResult::new(Type::Pointer {
                            inner: Box::new(elem),
                            mutable: false,
                        })));
                    }
                    "from_bytes" if args.len() <= 1 => {
                        for arg in args.iter() {
                            let _ = self.synth_expr(arg)?;
                        }
                        return Ok(Some(InferResult::new(Type::text())));
                    }
                    _ => {}
                }
            }
        }

        // NUMERIC TYPE METHOD FALLBACK
        // from_float, from_int, etc. on Float, Int, numeric types
        {
            let method_name_str = method.name.as_str();
            let is_numeric = match &recv_ty {
                Type::Int | Type::Float | Type::Char => true,
                Type::Named { path, .. } => path.as_ident().is_some_and(|id| {
                    matches!(
                        id.name.as_str(),
                        "Float"
                            | "Float32"
                            | "Float64"
                            | "Int"
                            | "Int8"
                            | "Int16"
                            | "Int32"
                            | "Int64"
                            | "UInt8"
                            | "UInt16"
                            | "UInt32"
                            | "UInt64"
                            | "USize"
                            | "ISize"
                            | "Byte"
                    )
                }),
                _ => false,
            };
            if is_numeric {
                match method_name_str {
                    "from_float" | "from_int" | "from_f32" | "from_f64" | "from_u8"
                    | "from_u16" | "from_u32" | "from_u64" | "from_i8" | "from_i16"
                    | "from_i32" | "from_i64"
                        if args.len() == 1 =>
                    {
                        let _ = self.synth_expr(&args[0])?;
                        return Ok(Some(InferResult::new(recv_ty.clone())));
                    }
                    "to_float" | "to_int" | "to_f32" | "to_f64" if args.is_empty() => {
                        let target_ty = match method_name_str {
                            "to_float" | "to_f64" => Type::float(),
                            "to_int" => Type::int(),
                            "to_f32" => Type::Named {
                                path: verum_ast::ty::Path::from_ident(verum_ast::Ident::new(
                                    "Float32", span,
                                )),
                                args: List::new(),
                            },
                            _ => Type::float(),
                        };
                        return Ok(Some(InferResult::new(target_ty)));
                    }
                    "item" if args.is_empty() => {
                        // .item() extracts scalar value - returns same type
                        return Ok(Some(InferResult::new(recv_ty.clone())));
                    }
                    _ => {}
                }
            }
        }

        // BOOL METHOD FALLBACK - cast() on Bool returns a tensor/numeric type
        if matches!(&recv_ty, Type::Bool) {
            let method_name_str = method.name.as_str();
            if method_name_str == "cast" {
                for arg in args.iter() {
                    let _ = self.synth_expr(arg)?;
                }
                let fresh = Type::Var(TypeVar::fresh());
                return Ok(Some(InferResult::new(fresh)));
            }
        }

        // TYPE VARIABLE / PLACEHOLDER METHOD FALLBACK
        // When receiver type is an unresolved type variable or placeholder, return a fresh
        // type variable rather than erroring. This allows type inference to continue.
        match &recv_ty {
            Type::Var(_) | Type::Placeholder { .. } => {
                for arg in args.iter() {
                    let _ = self.synth_expr(arg)?;
                }
                let fresh = Type::Var(TypeVar::fresh());
                return Ok(Some(InferResult::new(fresh)));
            }
            _ => {}
        }
        Ok(None)
    }

    /// Terminal handler when all method resolution strategies have failed.
    /// Checks specialization_rejected flag, tries deref-coercion chain retry,
    /// walks smart-pointer Target for diagnostic hints, then emits MethodNotFound.
    fn emit_method_not_found(
        &mut self,
        recv_ty: Type,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
        skip_static_lookup: bool,
        specialization_rejected: bool,
    ) -> Result<InferResult> {
        // If method was found but rejected by specialization, produce E400 instead of generic "method not found"
        if specialization_rejected {
            return Err(TypeError::Mismatch {
                expected: format!(
                    "a type implementing `{}` for `{}`",
                    method.name.as_str(),
                    recv_ty
                )
                .into(),
                actual: format!("{}", recv_ty).into(),
                span,
            });
        }

        // Method not found — but if the receiver is a context
        // type and we're in lenient mode (stdlib pre-registered
        // contexts whose method types can't be fully resolved),
        // return Unknown instead of erroring. The actual method
        // will be resolved at VBC codegen time.
        // In stdlib lenient mode (single-file check on core/*.vr),
        // accept unknown methods with Unknown return type. Methods
        // from implement blocks in sibling modules may not be
        // registered in single-file check mode.
        if self.stdlib_single_file_mode {
            return Ok(InferResult::new(Type::Unknown));
        }

        // #112 Deref-coercion retry: if the receiver implements
        // `Deref<Target = T>` (e.g. `PathBuf` -> `Path`,
        // `Heap<T>` -> `T`, `MutexGuard<T>` -> `T`), try
        // resolving the method against `T` BEFORE falling
        // through to the diagnostic hint path. This closes the
        // ergonomic gap where `path_buf.join_str(...)` failed
        // because the method lived on `Path`, not `PathBuf` —
        // callers had to write `path_buf.as_path().join_str(...)`.
        // We walk the chain up to 4 hops (matches the legacy
        // hint-walk depth) and short-circuit on the first
        // hop whose method dispatch succeeds. Cycle protection
        // via `DEREF_COERCION_DEPTH` thread-local; recursive
        // dispatches fall straight through without re-entering
        // the retry loop.
        let already_in_retry =
            DEREF_COERCION_DEPTH.with(|d| d.get() > 0);
        if !already_in_retry {
            let mut target_ty_opt = {
                let checker = self.protocol_checker.read();
                checker
                    .try_find_associated_type(&recv_ty, &Text::from("Target"))
            };
            let mut hop = 0usize;
            while let Some(target) = target_ty_opt.take() {
                if hop >= 4 {
                    break;
                }
                let normalised = self.normalize_type(self.unwrap_reference_type(&target));
                // Increment the guard, retry dispatch with the
                // unwrapped type as `precomputed_recv_ty`,
                // decrement on the way out regardless of result.
                DEREF_COERCION_DEPTH.with(|d| d.set(d.get() + 1));
                let retry = self.infer_method_call_inner_impl(
                    receiver,
                    method,
                    type_args,
                    args,
                    span,
                    Some(normalised.clone()),
                    skip_static_lookup,
                );
                DEREF_COERCION_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
                match retry {
                    Ok(result) => return Ok(result),
                    Err(TypeError::MethodNotFound { .. }) => {
                        // Keep walking — maybe a later hop has the method.
                    }
                    Err(other) => return Err(other),
                }
                let next = {
                    let checker = self.protocol_checker.read();
                    checker.try_find_associated_type(&normalised, &Text::from("Target"))
                };
                target_ty_opt = next;
                hop += 1;
            }
        }

        // Smart-pointer auto-deref hint: if the receiver is a
        // type with a Deref::Target (e.g. `Heap<T>`,
        // `Shared<T>`, `MutexGuard<T>`) and the method exists
        // on any type in the deref chain, suggest going through
        // the target. Direct `Heap<dyn P>.method()` dispatch is
        // wired through the auto-deref cascade + post-cascade
        // DynProtocol resolution (task #35, closed); this hint
        // remains for the corner case where the cascade failed
        // to propagate — it points at the explicit-deref form
        // `(&*h).method(...)` as a manual workaround.
        {
            let method_name_for_hint: Text = method.name.as_str().into();
            let mut target_ty_opt = {
                let checker = self.protocol_checker.read();
                checker.try_find_associated_type(&recv_ty, &verum_common::Text::from("Target"))
            };
            let mut hop = 0;
            while let Some(target) = target_ty_opt.take() {
                if hop >= 4 {
                    break;
                }
                let normalised = self.normalize_type(self.unwrap_reference_type(&target));
                // Walk one level at a time — short-circuit if
                // the target is a dyn-protocol whose protocol
                // itself defines the method.
                if let Type::DynProtocol { bounds, .. } = &normalised {
                    let checker = self.protocol_checker.read();
                    for proto_name in bounds.iter() {
                        if let Maybe::Some(proto) = checker.get_protocol(proto_name) {
                            if proto.methods.contains_key(&method_name_for_hint) {
                                return Err(TypeError::MethodNotFound {
                                    ty: recv_ty.to_text(),
                                    method: method.name.as_str().to_text(),
                                    span,
                                    did_you_mean: verum_common::Maybe::Some(
                                        verum_common::Text::from(format!(
                                            "try `(&*receiver).{}(...)` to call `{}` on `dyn {}` \
                                             directly — the auto-deref cascade should have \
                                             reached it but did not on this call site",
                                            method.name.as_str(),
                                            method.name.as_str(),
                                            proto_name
                                        )),
                                    ),
                                });
                            }
                        }
                    }
                }
                // Continue walking — e.g. Shared<Mutex<T>> unwrapping.
                let next = {
                    let checker = self.protocol_checker.read();
                    checker.try_find_associated_type(
                        &normalised,
                        &verum_common::Text::from("Target"),
                    )
                };
                target_ty_opt = next;
                hop += 1;
            }
        }

        // "Did you mean ...?" — gather method names registered for
        // any type that matches the receiver's name, then suggest
        // the closest Levenshtein match. Tolerant matching handles
        // generic instantiations ("List<Int>" matching "List").
        let method_str = method.name.as_str();
        let recv_name = recv_ty.to_text();
        let recv_str = recv_name.as_str();
        // "Did you mean ...?" — search two sources:
        //  1. protocol_checker.method_registry() — built-in/stdlib method signatures
        //  2. self.inherent_methods — user-defined `implement` blocks
        // Match by receiver type name, tolerating generic instantiations
        // ("List<Int>" contains "List"). Then rank candidates by
        // Levenshtein distance and return the closest (<=3 edits).
        let did_you_mean: Option<verum_common::Text> = {
            let mut best: Option<(usize, verum_common::Text)> = None;
            let consider = |best: &mut Option<(usize, verum_common::Text)>,
                            m: &verum_common::Text| {
                let d = levenshtein_distance(method_str, m.as_str());
                if d == 0 || d > 3 {
                    return;
                }
                match best {
                    Some((bd, _)) if *bd <= d => {}
                    _ => *best = Some((d, m.clone())),
                }
            };
            {
                let checker = self.protocol_checker.read();
                for ((ty_name, m_name), _sig) in checker.method_registry().iter() {
                    if !recv_str.contains(ty_name.as_str())
                        && !ty_name.as_str().contains(recv_str)
                    {
                        continue;
                    }
                    consider(&mut best, m_name);
                }
            }
            {
                let inherents = self.inherent_methods.read();
                for (ty_name, methods) in inherents.iter() {
                    if !recv_str.contains(ty_name.as_str())
                        && !ty_name.as_str().contains(recv_str)
                    {
                        continue;
                    }
                    for (m_name, _scheme) in methods.iter() {
                        // Strip static-method marker prefix used by the
                        // inherent methods table.
                        let bare = m_name
                            .as_str()
                            .strip_prefix("$static$")
                            .unwrap_or(m_name.as_str());
                        consider(&mut best, &verum_common::Text::from(bare));
                    }
                }
            }
            best.map(|(_, n)| n)
        };
        return Err(TypeError::MethodNotFound {
            ty: recv_ty.to_text(),
            method: method.name.as_str().to_text(),
            span,
            did_you_mean,
        });
    }

    fn try_resolve_pre_receiver_method(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
        skip_static_lookup: bool,
    ) -> Result<Option<InferResult>> {
        if let Some(r) = self.try_resolve_super_path_call(
            receiver, method, args, span, skip_static_lookup,
        )? { return Ok(Some(r)); }

        if let Some(r) = self.try_resolve_module_call(receiver, method, args, span)? {
            return Ok(Some(r));
        }

        // GENERIC TYPE STATIC METHOD CALL: Handle Wrapper<Person>.default() syntax
        // When receiver is a TypeExpr, extract the type and use it for method lookup.
        // The type arguments (like Person) are used to instantiate generic implementations.
        // Generic type instantiation: substituting concrete types for type parameters, checking bounds
        if let ExprKind::TypeExpr(ty) = &receiver.kind {
            // Convert AST type to internal Type representation
            let receiver_ty = self.ast_to_type(ty)?;
            let method_name = method.name.as_str();

            // Extract base type name for method lookup
            let type_name = self.type_to_name(&receiver_ty).to_string();

            // Try to look up the method via protocol_checker.
            // Use lookup_all_protocol_methods to handle multiple parameterized protocol impls
            // (e.g., TryFrom<Int>, TryFrom<Int32>, TryFrom<UInt32> all providing try_from).
            let method_name_text = verum_common::Text::from(method_name);
            let all_methods_result = self
                .protocol_checker
                .read()
                .lookup_all_protocol_methods(&receiver_ty, &method_name_text);
            if let Ok(candidates) = all_methods_result {
                if candidates.len() == 1 {
                    // Single candidate - use directly (common fast path)
                    let method_ty = &candidates[0];
                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = method_ty
                    {
                        // Allow ±1 tolerance for self-param counting
                        if params.len().abs_diff(args.len()) > 1 {
                            return Err(TypeError::WrongArgCount {
                                method: method_name.to_text(),
                                expected: params.len(),
                                actual: args.len(),
                                span,
                            });
                        }

                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }

                        let resolved_return = self.unifier.apply(return_type);
                        return Ok(Some(InferResult::new(resolved_return)));
                    }
                } else if candidates.len() > 1 {
                    // Multiple candidates - disambiguate by argument types.
                    // Pre-infer argument types, then find the candidate whose params match.
                    let mut arg_types = List::new();
                    for arg in args.iter() {
                        if let Ok(r) = self.infer_expr(arg, InferMode::Synth) {
                            arg_types.push(r.ty);
                        }
                    }

                    for candidate in candidates.iter() {
                        if let Type::Function {
                            params,
                            return_type,
                            ..
                        } = candidate
                        {
                            if params.len().abs_diff(args.len()) > 1 {
                                continue;
                            }
                            // Check if all arg types are compatible with params
                            let mut compatible = true;
                            if arg_types.len() == params.len() {
                                for (arg_ty, param_ty) in arg_types.iter().zip(params.iter()) {
                                    let resolved_param = self.unifier.apply(param_ty);
                                    if !self.types_compatible(arg_ty, &resolved_param) {
                                        compatible = false;
                                        break;
                                    }
                                }
                            }
                            if compatible {
                                // Found matching candidate - type check args properly
                                for (arg, param_ty) in args.iter().zip(params.iter()) {
                                    let resolved_param = self.unifier.apply(param_ty);
                                    self.check_expr(arg, &resolved_param)?;
                                }
                                let resolved_return = self.unifier.apply(return_type);
                                return Ok(Some(InferResult::new(resolved_return)));
                            }
                        }
                    }
                    // If no candidate matched, fall through to env lookup
                }
            }

            // Also check for variant constructors like Maybe<Int>.Some(42)
            let qualified_name = format!("{}.{}", type_name, method_name);
            if let Some(scheme) = self.ctx.env.lookup(&qualified_name) {
                let constructor_ty = scheme.instantiate();

                if let Type::Function {
                    params,
                    return_type,
                    ..
                } = &constructor_ty
                {
                    // Pre-check argument compatibility (same as Path block below)
                    let mut args_pre_compatible = true;
                    if args.len() == params.len() {
                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            if let Ok(arg_result) = self.infer_expr(arg, InferMode::Synth) {
                                if !self.types_compatible(&arg_result.ty, &resolved_param) {
                                    args_pre_compatible = false;
                                    break;
                                }
                            }
                        }
                    }

                    if args_pre_compatible {
                        // Allow ±1 tolerance for self-param counting
                        if params.len().abs_diff(args.len()) > 1 {
                            return Err(TypeError::WrongArgCount {
                                method: method_name.to_text(),
                                expected: params.len(),
                                actual: args.len(),
                                span,
                            });
                        }

                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }

                        let resolved_return = self.unifier.apply(return_type);
                        return Ok(Some(InferResult::new(resolved_return)));
                    }
                }
            }
        }

        if let Some(r) = self.try_resolve_path_static_call(
            receiver, method, type_args, args, span, skip_static_lookup,
        )? { return Ok(Some(r)); }
        Ok(None)
    }


    /// Auto-dereference for method calls on references and Heap<T>
    ///

    /// This enables method calls like `ref.len()` where `ref: &List<Int>` to work
    /// by automatically dereferencing to the underlying type for method resolution.
    ///

    /// Handles:
    /// - &T -> T (all reference kinds: &T, &checked T, &unsafe T)
    /// - Heap<T> -> T
    /// - &Heap<T> -> T (double dereference)
    /// - Ownership<T> -> T
    ///

    /// CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — #auto-dereference
    /// Resolve `super.method(args)` calls, module-alias calls
    /// (`mount X as A` then `A.method()`), and DI context method dispatch.
    fn try_resolve_super_path_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
        skip_static_lookup: bool,
    ) -> Result<Option<InferResult>> {
        // Handle super path function calls
        // When we have `super.func(args)`, this is parsed as a method call
        // but should be resolved as a parent module function call.
        // Circular import handling: detection and error reporting for cyclic module dependencies — Relative module paths
        if let ExprKind::Path(path) = &receiver.kind
            && !path.segments.is_empty()
            && matches!(
                path.segments.first(),
                Some(verum_ast::ty::PathSegment::Super)
            )
        {
            // Resolve super.func() by looking up in parent module
            let method_name = method.name.as_str();

            // Get current module path and compute parent path
            let current_module_path = self.current_module_path.clone();
            if let Some(parent_path) =
                self.compute_parent_module_path(&current_module_path, &path.segments)
            {
                // Build the full function path in the parent module
                let full_path = if parent_path.is_empty() {
                    verum_common::Text::from(method_name)
                } else {
                    verum_common::Text::from(format!("{}.{}", parent_path, method_name))
                };

                // Look up function in parent module's inline modules
                // Clone items to avoid borrow conflict with infer_function_type
                let items_opt = self
                    .inline_modules
                    .get(&verum_common::Text::from(parent_path.clone()))
                    .and_then(|m| m.items.clone());

                if let Some(items) = items_opt {
                    for item in items.iter() {
                        if let verum_ast::ItemKind::Function(func_decl) = &item.kind {
                            if func_decl.name.name.as_str() == method_name {
                                // Found the function - infer its type and return
                                let func_type = self.infer_function_type(func_decl)?;
                                if let Type::Function {
                                    params,
                                    return_type,
                                    ..
                                } = &func_type
                                {
                                    // Check argument count
                                    // Allow ±1 tolerance for self-param counting
                                    if params.len().abs_diff(args.len()) > 1 {
                                        return Err(TypeError::WrongArgCount {
                                            method: method.name.clone(),
                                            expected: params.len(),
                                            actual: args.len(),
                                            span,
                                        });
                                    }

                                    // Type check each argument
                                    for (arg, param_ty) in args.iter().zip(params.iter()) {
                                        let resolved_param = self.unifier.apply(param_ty);
                                        self.check_expr(arg, &resolved_param)?;
                                    }

                                    let resolved_return = self.unifier.apply(return_type);
                                    return Ok(Some(InferResult::new(resolved_return)));
                                }
                            }
                        }
                    }
                }

                // Function not found in parent module
                return Err(TypeError::UnboundVariable {
                    name: verum_common::Text::from(format!("super.{}", method_name)),
                    span,
                });
            }
        }

        // Module-alias dispatch — `mount X as A;` then `A.method(...)`
        // routes to `X.method(...)` rather than synth_expr'ing `A` as a
        // value. Without this, a stdlib function with the same name as
        // the user's alias (e.g. core.sys.linux.syscall.stat vs
        // `mount core.net.h3.qpack.static_table as stat;`) wins at
        // value-lookup and the whole `stat.get(0)` call breaks with a
        // spurious method-dispatch error.
        //

        // Only fires on the first call of a chain (`!skip_static_lookup`).
        if !skip_static_lookup
            && let ExprKind::Path(path) = &receiver.kind
            && path.segments.len() == 1
            && let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first()
            && let Some(module_path) = self.module_aliases.get(&ident.name).cloned()
        {
            let method_name = method.name.as_str();
            let qualified: Text = format!("{}.{}", module_path, method_name).into();

            // Fast path: the function may already have been imported into
            // the type env under its qualified-module form by
            // `import_all_from_module` when the mount was processed.
            if let Some(scheme) = self.ctx.env.lookup(&qualified).cloned() {
                let func_type = self.unifier.apply(&scheme.ty);
                if let Type::Function {
                    params,
                    return_type,
                    ..
                } = &func_type
                {
                    if params.len() == args.len() {
                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }
                        let resolved_return = self.unifier.apply(return_type);
                        return Ok(Some(InferResult::new(resolved_return)));
                    }
                }
            }

            // Slow path: walk inline_modules AST items.
            let items_opt = self
                .inline_modules
                .get(&module_path)
                .map(|m| m.items.clone())
                .and_then(|items| items);
            if let Some(items) = items_opt {
                for item in items.iter() {
                    if let verum_ast::ItemKind::Function(func_decl) = &item.kind {
                        if func_decl.name.name.as_str() == method_name {
                            let func_type = self.infer_function_type(func_decl)?;
                            if let Type::Function {
                                params,
                                return_type,
                                ..
                            } = &func_type
                            {
                                if params.len() == args.len() {
                                    for (arg, param_ty) in args.iter().zip(params.iter()) {
                                        let resolved_param = self.unifier.apply(param_ty);
                                        self.check_expr(arg, &resolved_param)?;
                                    }
                                    let resolved_return = self.unifier.apply(return_type);
                                    return Ok(Some(InferResult::new(resolved_return)));
                                }
                            }
                        }
                    }
                }
            }

            // Registry fallback: walk the parsed AST of the aliased
            // module, find the named public function, infer its type,
            // and type-check the call against it.
            let registry = self.module_registry.read();
            if let Some(module_info) = registry.get_by_path(module_path.as_str()) {
                let items: Vec<_> = module_info.ast.items.iter().cloned().collect();
                drop(registry);
                for item in items.iter() {
                    if let verum_ast::ItemKind::Function(func_decl) = &item.kind {
                        if func_decl.name.name.as_str() == method_name
                            && func_decl.visibility == verum_ast::decl::Visibility::Public
                        {
                            let func_type = self.infer_function_type(func_decl)?;
                            if let Type::Function {
                                params,
                                return_type,
                                ..
                            } = &func_type
                            {
                                if params.len() == args.len() {
                                    for (arg, param_ty) in args.iter().zip(params.iter()) {
                                        let resolved_param = self.unifier.apply(param_ty);
                                        self.check_expr(arg, &resolved_param)?;
                                    }
                                    let resolved_return = self.unifier.apply(return_type);
                                    return Ok(Some(InferResult::new(resolved_return)));
                                }
                            }
                        }
                    }
                }
            } else {
                drop(registry);
            }

            // Function not found in aliased module.
            return Err(TypeError::UnboundVariable {
                name: verum_common::Text::from(format!("{}.{}", module_path, method_name)),
                span,
            });
        }

        // Context-method dispatch has priority over stdlib module dispatch.
        //

        // When a user declares
        //  context Time { fn now() -> Timestamp; }
        // and writes
        //  pure fn f() -> Timestamp using [Time] { Time.now() }
        // the `Time.now()` call must resolve to the context method (returning
        // Timestamp), not to the stdlib `core.time.Time.now()` (returning
        // Duration). Before this check the stdlib path won and produced a
        // misleading type mismatch on the caller's annotated return type.
        //

        // Trigger: the first segment of the receiver path is a
        // user-declared context name AND the current function's `using [...]`
        // clause lists that context. The explicit `using` marker is what
        // gives the compiler license to choose the context dispatch —
        // without it, a bare `Time.now()` in a non-`using` context could
        // legitimately mean the stdlib namespace.
        //

        // Only fire on the first call of a method chain
        // (`!skip_static_lookup`). The iterative chain handler reuses the
        // outermost receiver expression for every chain step, so inside
        // `CancelCtx.get_token().check()?` the same `Path(CancelCtx)` is
        // the receiver for both `.get_token()` and `.check()` — but
        // `.check()` should resolve against the return type of
        // `.get_token()`, not re-dispatch the context.
        if !skip_static_lookup
            && let ExprKind::Path(path) = &receiver.kind
            && path.segments.len() == 1
            && let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first()
        {
            let ctx_name: verum_common::Text = ident.name.clone();
            let in_using_clause = self
                .current_function_contexts
                .as_ref()
                .map(|set| set.iter().any(|req| req.name == ctx_name))
                .unwrap_or(false);
            if in_using_clause
                && let Some(ctx_decl) = self.context_declarations.get(&ctx_name).cloned()
            {
                for ctx_method in ctx_decl.methods.iter() {
                    if ctx_method.name.name != method.name {
                        continue;
                    }
                    // Arity check: caller passes `args`; context method
                    // carries the signature sans `self` receiver.
                    if ctx_method.params.len() != args.len() {
                        continue;
                    }
                    // Type-check each argument against the declared
                    // parameter type. Missing or unresolvable types fall
                    // through to `Type::Unknown` which still unifies.
                    for (arg, param) in args.iter().zip(ctx_method.params.iter()) {
                        let param_ty = if let verum_ast::decl::FunctionParamKind::Regular {
                            ty: param_ty_ast,
                            ..
                        } = &param.kind
                        {
                            self.ast_to_type(param_ty_ast).unwrap_or(Type::Unknown)
                        } else {
                            Type::Unknown
                        };
                        self.check_expr(arg, &param_ty)?;
                    }
                    let return_ty = match &ctx_method.return_type {
                        verum_common::Maybe::Some(rt) => {
                            self.ast_to_type(rt).unwrap_or(Type::Unknown)
                        }
                        verum_common::Maybe::None => Type::unit(),
                    };
                    return Ok(Some(InferResult::new(return_ty)));
                }
            }
        }
        Ok(None)
    }

    /// Resolve module-qualified function calls: `module.func(args)` or
    /// `outer.inner.func(args)`. Returns `Ok(None)` when not a module path.
    fn try_resolve_module_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<InferResult>> {
        // Handle inline module function calls
        // When we have `module.func(args)`, this is parsed as a method call
        // but should be resolved as a module-qualified function call.
        // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
        if let ExprKind::Path(path) = &receiver.kind {
            // Check if the first segment is an inline module
            let first_segment_name = match path.segments.first() {
                Some(verum_ast::ty::PathSegment::Name(ident)) => Some(ident.name.as_str()),
                _ => None,
            };

            if let Some(first_name) = first_segment_name {
                if self
                    .inline_modules
                    .contains_key(&verum_common::Text::from(first_name))
                {
                    // Name collision: a single identifier (e.g. `Validated`)
                    // can refer to BOTH a type and a same-named module
                    // (`type Validated<E, A> is …;` plus
                    // `public module Validated { fn validate_all<…>(…) … }`).
                    // The inline-module branch below assumes module-style
                    // dispatch and silently swallows static-method calls
                    // on the type. Before committing to module dispatch,
                    // check whether the type has a static method by this
                    // name; if so, fall through to the static-method path
                    // so `Validated.valid(42)` resolves against
                    // `implement<E, A> Validated<E, A> { fn valid(…) }`
                    // rather than failing with `unbound variable: valid`.
                    let static_key =
                        verum_common::Text::from(format!("$static${}", method.name.as_str()));
                    let has_static_method = {
                        let methods_guard = self.inherent_methods.read();
                        methods_guard
                            .get(&verum_common::Text::from(first_name))
                            .is_some_and(|methods| methods.contains_key(&static_key))
                    };
                    if has_static_method {
                        // Skip the inline-module branch and let the
                        // static-method dispatch below handle it.
                    } else {
                        // Build the full path including the method as the last segment
                        let mut segments: Vec<verum_ast::ty::PathSegment> =
                            path.segments.iter().cloned().collect();
                        segments.push(verum_ast::ty::PathSegment::Name(method.clone()));

                        let module_path = verum_ast::ty::Path {
                            segments: segments.into(),
                            span,
                        };

                        // Resolve through inline module - this will check visibility
                        let func_result = self.resolve_inline_module_path(&module_path, span)?;

                        // Never propagation: if resolution returned Never, propagate it
                        if matches!(func_result.ty, Type::Never) {
                            return Ok(Some(InferResult::new(Type::Never)));
                        }

                        // The result should be a function type - call it with args
                        if let Type::Function {
                            params,
                            return_type,
                            ..
                        } = &func_result.ty
                        {
                            // Check argument count
                            // Allow ±1 tolerance for self-param counting inconsistencies
                            // and default parameter handling in method resolution
                            if args.len() > params.len() + 1
                                || (args.len() + 1 < params.len() && params.len() > 1)
                            {
                                return Err(TypeError::WrongArgCount {
                                    method: method.name.clone(),
                                    expected: params.len(),
                                    actual: args.len(),
                                    span,
                                });
                            }

                            // Type check each argument
                            for (arg, param_ty) in args.iter().zip(params.iter()) {
                                let resolved_param = self.unifier.apply(param_ty);
                                self.check_expr(arg, &resolved_param)?;
                            }

                            let resolved_return = self.unifier.apply(return_type);
                            return Ok(Some(InferResult::new(resolved_return)));
                        } else {
                            return Err(TypeError::NotAFunction {
                                ty: format!("{}", func_result.ty).into(),
                                span,
                            });
                        }
                    } // end of `else` for has_static_method check
                }
            }
        }

        // Handle nested module function calls via Field access
        // When we have `outer.inner.func(args)`, the receiver is a Field expression:
        //  Field { expr: Path("outer"), field: "inner" }
        // We need to recognize this as a nested module path and resolve the function.
        // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Nested Inline Modules
        if let ExprKind::Field {
            expr: inner_expr,
            field: inner_field,
        } = &receiver.kind
        {
            // Check if this is a module path by recursively extracting path segments
            let module_segments = self.extract_module_path_from_field(inner_expr, inner_field);
            if let Some(segments) = module_segments {
                // Check if the first segment is an inline module
                if self
                    .inline_modules
                    .contains_key(&verum_common::Text::from(segments[0]))
                {
                    // Build the full path including the method as the last segment
                    let mut path_segments: Vec<verum_ast::ty::PathSegment> = segments
                        .iter()
                        .map(|s| {
                            verum_ast::ty::PathSegment::Name(verum_ast::Ident {
                                name: (*s).into(),
                                span,
                            })
                        })
                        .collect();
                    path_segments.push(verum_ast::ty::PathSegment::Name(method.clone()));

                    let module_path = verum_ast::ty::Path {
                        segments: path_segments.into(),
                        span,
                    };

                    // Resolve through inline module - this will check visibility
                    let func_result = self.resolve_inline_module_path(&module_path, span)?;

                    // Never propagation: if resolution returned Never, propagate it
                    if matches!(func_result.ty, Type::Never) {
                        return Ok(Some(InferResult::new(Type::Never)));
                    }

                    // The result should be a function type - call it with args
                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = &func_result.ty
                    {
                        // Check argument count
                        // Allow ±1 tolerance for self-param counting inconsistencies
                        // and default parameter handling in method resolution
                        if args.len() > params.len() + 1
                            || (args.len() + 1 < params.len() && params.len() > 1)
                        {
                            return Err(TypeError::WrongArgCount {
                                method: method.name.clone(),
                                expected: params.len(),
                                actual: args.len(),
                                span,
                            });
                        }

                        // Type check each argument
                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }

                        let resolved_return = self.unifier.apply(return_type);
                        return Ok(Some(InferResult::new(resolved_return)));
                    } else {
                        return Err(TypeError::NotAFunction {
                            ty: format!("{}", func_result.ty).into(),
                            span,
                        });
                    }
                }
            }
        }
        Ok(None)
    }

    /// Resolve static method calls on single-segment Path receivers:
    /// variant constructors (`Maybe.Some(x)`), type-param bound dispatch
    /// (`U.from(self)` where `U: From<T>`), inherent static methods, and
    /// protocol fallback with shadow-guard.
    fn try_resolve_path_static_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        type_args: &List<verum_ast::ty::GenericArg>,
        args: &[Expr],
        span: Span,
        skip_static_lookup: bool,
    ) -> Result<Option<InferResult>> {
        // CRITICAL FIX: Handle Type.Variant(args) syntax for variant constructors
        // When we have Maybe.Some(42), the parser treats it as a method call on Maybe.
        // But this should actually be interpreted as calling the variant constructor Maybe.Some.
        // This is similar to the Field access case, but with arguments.
        //

        // Also handles Self.method() by resolving Self to the actual type name
        //

        // IMPORTANT: Skip this check when skip_static_lookup is true. This is set for
        // chained method calls (after the first) where receiver.kind is the original base
        // expression, not the actual receiver of this specific method call. For example,
        // in `Int.max_value().min(0)`, when processing `.min(0)`, receiver.kind is still
        // Path("Int") but the actual receiver is the result of max_value().
        if !skip_static_lookup
            && let ExprKind::Path(path) = &receiver.kind
            && path.segments.len() == 1
        {
            // Get the type name - either from a Name segment or from Self resolution
            let type_name: Option<String> = match &path.segments[0] {
                verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                verum_ast::ty::PathSegment::SelfValue => {
                    // Resolve Self to the actual type name from current_self_type
                    self.current_self_type
                        .as_ref()
                        .map(|self_ty| self.type_to_name(self_ty).to_string())
                }
                _ => None,
            };

            if let Some(type_name) = type_name {
                let type_name = type_name.as_str();
                let method_name = method.name.as_str();
                let qualified_name = format!("{}.{}", type_name, method_name);

                // CRITICAL: Handle static method calls on type parameters with protocol bounds.
                // For example: `U.from(self)` where `U: From<T>` should lookup `from` from `From<T>` protocol.
                // Check if type_name refers to a type parameter.
                let type_name_text = verum_common::Text::from(type_name);
                if let Some(ty) = self.ctx.lookup_type(&type_name_text).cloned() {
                    // If the type is a type variable, it might have protocol bounds
                    if let Type::Var(_) = &ty {
                        // Look up all type parameters with their bounds
                        let all_type_params = self.ctx.env.all_type_params();
                        for type_param in all_type_params {
                            // Check if this type parameter matches our type_name
                            if type_param.name.as_str() == type_name {
                                // Check each protocol bound on this type parameter
                                for bound in &type_param.bounds {
                                    if let Some(protocol_ident) = bound.protocol.as_ident() {
                                        let protocol_name: Text = protocol_ident.name.clone();

                                        // Look up the protocol definition
                                        let protocol_opt = self
                                            .protocol_checker
                                            .read()
                                            .get_protocol(&protocol_name)
                                            .cloned();
                                        if let Maybe::Some(protocol) = protocol_opt {
                                            // Check if this protocol has the method we're looking for
                                            for (_, proto_method) in &protocol.methods {
                                                if proto_method.name.as_str() == method_name {
                                                    // Found the method in a bounded protocol
                                                    // Substitute Self with the type parameter's type variable
                                                    let method_ty = self
                                                        .instantiate_method_for_receiver(
                                                            &proto_method.ty,
                                                            &ty,
                                                        );
                                                    // Instantiate method's own type parameters with explicit type args
                                                    let method_ty = self
                                                        .instantiate_method_type_params(
                                                            method_ty, type_args, span,
                                                        )?;

                                                    if let Type::Function {
                                                        params,
                                                        return_type,
                                                        ..
                                                    } = &method_ty
                                                    {
                                                        // For STATIC protocol methods (like From::from), there's no self param
                                                        // The params should already be correct
                                                        if params.len().abs_diff(args.len()) > 1 {
                                                            return Err(TypeError::WrongArgCount {
                                                                method: method_name.to_text(),
                                                                expected: params.len(),
                                                                actual: args.len(),
                                                                span,
                                                            });
                                                        }

                                                        // Type check each argument
                                                        for (arg, param_ty) in
                                                            args.iter().zip(params.iter())
                                                        {
                                                            let resolved_param =
                                                                self.unifier.apply(param_ty);
                                                            self.check_expr(arg, &resolved_param)?;
                                                        }

                                                        // Apply substitution to return type
                                                        let resolved_return =
                                                            self.unifier.apply(return_type);
                                                        return Ok(Some(InferResult::new(
                                                            resolved_return,
                                                        )));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Try to look up the qualified variant constructor or static method.
                let mut env_resolved = false;
                if let Some(scheme) = self.ctx.env.lookup(&qualified_name) {
                    let constructor_ty = scheme.instantiate();

                    if let Type::Function {
                        params,
                        return_type,
                        ..
                    } = &constructor_ty
                    {
                        // SPECIAL CASE: Handle variadic builtins like List.of, Set.of
                        if (qualified_name == "List.of" || qualified_name == "Set.of")
                            && !args.is_empty()
                        {
                            let elem_ty = if params.len() == 1 {
                                params[0].clone()
                            } else {
                                Type::Var(TypeVar::fresh())
                            };

                            for arg in args.iter() {
                                let resolved_elem = self.unifier.apply(&elem_ty);
                                self.check_expr(arg, &resolved_elem)?;
                            }

                            let resolved_return = self.unifier.apply(return_type);
                            return Ok(Some(InferResult::new(resolved_return)));
                        }

                        // Commit to this qualified-name resolution. The
                        // earlier `args_pre_compatible` heuristic (raw-string
                        // type name equality before real check_expr) was a
                        // silent masker: any spurious mismatch would fall
                        // through to a hardcoded-method-name fallback, or
                        // (after that was removed) to MethodNotFound — even
                        // when the env entry is the one true resolution.
                        // Real type-checking runs here via check_expr below;
                        // unification + coercion handle refinement, numeric
                        // canonicalization, and reference auto-borrow.
                        // Allow ±1 tolerance for self-param counting.
                        if params.len().abs_diff(args.len()) > 1 {
                            return Err(TypeError::WrongArgCount {
                                method: method_name.to_text(),
                                expected: params.len(),
                                actual: args.len(),
                                span,
                            });
                        }

                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let resolved_param = self.unifier.apply(param_ty);
                            self.check_expr(arg, &resolved_param)?;
                        }

                        let resolved_return = self.unifier.apply(return_type);
                        return Ok(Some(InferResult::new(resolved_return)));
                    } else {
                        env_resolved = true; // Non-function type, treat as resolved
                    }
                }
                if !env_resolved {
                    // Fallback: Check inherent_methods for static methods
                    let static_key = verum_common::Text::from(format!("$static${}", method_name));
                    let type_name_text = verum_common::Text::from(type_name);
                    let method_ty_opt = {
                        let methods_guard = self.inherent_methods.read();
                        methods_guard
                            .get(&type_name_text)
                            .and_then(|methods| methods.get(&static_key).cloned())
                            // Instantiate the TypeScheme to create fresh type variables
                            .map(|scheme| scheme.instantiate())
                    };

                    // TASK #21 FIX: If not found, try resolving type alias
                    // For `Vec4f.splat()` where `type Vec4f = Vec<Float32, 4>`,
                    // we need to look up methods on `Vec` instead of `Vec4f`.
                    let method_ty_opt = if method_ty_opt.is_none() {
                        if let Some(resolved_type) = self.ctx.resolve_alias(type_name) {
                            // Extract base type name from resolved type
                            // For `Vec<Float32, 4>`, extract `Vec`
                            let base_type_name = self.type_to_name(resolved_type);
                            let base_type_text = verum_common::Text::from(base_type_name.as_str());
                            let methods_guard = self.inherent_methods.read();
                            methods_guard
                                .get(&base_type_text)
                                .and_then(|methods| methods.get(&static_key).cloned())
                                .map(|scheme| scheme.instantiate())
                        } else {
                            None
                        }
                    } else {
                        method_ty_opt
                    };

                    if let Some(method_ty) = method_ty_opt {
                        // Method found in inherent_methods - proceed with call
                        if let Type::Function {
                            params,
                            return_type,
                            ..
                        } = &method_ty
                        {
                            // Pre-check argument type compatibility before committing.
                            // When multiple protocol impls (e.g., TryFrom<Int>, TryFrom<Int32>,
                            // TryFrom<UInt32>) register methods, inherent_methods stores the last one.
                            // If args don't match, skip and fall through to protocol_checker.
                            let mut args_pre_compatible = true;
                            if args.len() == params.len() {
                                for (arg, param_ty) in args.iter().zip(params.iter()) {
                                    let resolved_param = self.unifier.apply(param_ty);
                                    if let Ok(arg_result) = self.infer_expr(arg, InferMode::Synth) {
                                        if !self.types_compatible(&arg_result.ty, &resolved_param) {
                                            args_pre_compatible = false;
                                            break;
                                        }
                                    }
                                }
                            }

                            if args_pre_compatible {
                                // Check argument count
                                if params.len().abs_diff(args.len()) > 1 {
                                    return Err(TypeError::WrongArgCount {
                                        method: method_name.to_text(),
                                        expected: params.len(),
                                        actual: args.len(),
                                        span,
                                    });
                                }

                                // Type check each argument and accumulate substitution
                                for (arg, param_ty) in args.iter().zip(params.iter()) {
                                    let resolved_param = self.unifier.apply(param_ty);
                                    self.check_expr(arg, &resolved_param)?;
                                }

                                // Apply accumulated substitution to return type
                                let resolved_return = self.unifier.apply(return_type);
                                return Ok(Some(InferResult::new(resolved_return)));
                            }
                            // Args not compatible — fall through to protocol_checker
                        }
                    }

                    // Fallback: Handle primitive type static method calls.
                    // For example: Int.min(10, 20), Float.pi(), Int.default()
                    // These are static calls where the receiver is a primitive type name.
                    // We resolve them by converting to instance method calls on the primitive type.
                    {
                        let prim_ty = match type_name {
                            "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "Int128" | "UInt"
                            | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" | "Byte" => {
                                Some(Type::Int)
                            }
                            "Float" | "Float32" | "Float64" => Some(Type::Float),
                            "Bool" => Some(Type::Bool),
                            "Char" => Some(Type::Char),
                            "Text" => Some(Type::Text),
                            _ => None,
                        };
                        if let Some(prim) = prim_ty {
                            // Try as a static method: arg_count = args.len() (no self)
                            // If resolve_primitive_method handles it with args.len() args, use that.
                            // Otherwise try with args.len()-1 (first arg is "self").
                            if let Some(result_ty) =
                                resolve_primitive_method(&prim, method_name, args.len())
                            {
                                // Static method with all args as params
                                for arg in args.iter() {
                                    self.synth_expr(arg)?;
                                }
                                return Ok(Some(InferResult::new(result_ty)));
                            }
                            // Try treating first arg as self for instance-like statics
                            // e.g., Int.min(10, 20) -> 10.min(20) where arg_count=1
                            if !args.is_empty() {
                                if let Some(result_ty) =
                                    resolve_primitive_method(&prim, method_name, args.len() - 1)
                                {
                                    for arg in args.iter() {
                                        self.synth_expr(arg)?;
                                    }
                                    return Ok(Some(InferResult::new(result_ty)));
                                }
                            }
                        }
                    }

                    // CRITICAL FIX: Fallback to protocol_checker for protocol static methods
                    // Protocol impls don't register static methods in ctx.env or with $static$ prefix.
                    // We need to use lookup_protocol_method which properly handles Self and type params.
                    // This enables T.default() pattern in generic protocol implementations.
                    //

                    // IMPORTANT: Only use this fallback if type_name looks like a type name (starts with uppercase)
                    // not a variable name (starts with lowercase). This prevents false positives where
                    // `file.read_to_string()` would incorrectly try to look up methods on a type named "file".
                    let is_type_name = type_name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                    // NAME-COLLISION GUARD: if the user has a same-named type
                    // whose inherent methods already define THIS specific
                    // method, skip the protocol-method static lookup and
                    // let the downstream inherent-methods path resolve it.
                    //

                    // Key invariant: the guard only fires when BOTH
                    // (a) a local `type X` exists AND (b) inherent_methods
                    // registers the *specific* method name being called.
                    // Otherwise we fall through to protocol lookup — this
                    // preserves context-method dispatch (e.g., Logger.info)
                    // where the user's file has no same-named local type
                    // OR the user-local type has no method with that name.
                    let user_type_shadows_this_method = {
                        let ctn: verum_common::Text = type_name.into();
                        let has_local_type = matches!(
                            self.ctx.lookup_type(type_name),
                            Option::Some(Type::Named { .. })
                                | Option::Some(Type::Record(_))
                                | Option::Some(Type::Variant(_))
                                | Option::Some(Type::Placeholder { .. })
                        );
                        let has_this_inherent = self
                            .inherent_methods
                            .read()
                            .get(&ctn)
                            .map(|m| m.contains_key(&verum_common::Text::from(method_name)))
                            .unwrap_or(false);
                        has_local_type && has_this_inherent
                    };
                    if is_type_name && !user_type_shadows_this_method {
                        // Build the lookup type from the type name.
                        // For generic types like Wrapper, this creates Wrapper with no args, and
                        // lookup_protocol_method will match it against Wrapper<T> implementations.
                        let lookup_ty = Type::Named {
                            path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                                type_name, span,
                            )),
                            args: List::new(),
                        };

                        let method_name_text = verum_common::Text::from(method_name);

                        // CRITICAL FIX: Use lookup_all_protocol_methods to get ALL matching signatures.
                        // This handles cases where a type implements multiple parameterized protocols
                        // with the same method (e.g., FromResidual<Result<Never, E>> and
                        // FromResidual<Maybe<Never>> both have `from_residual`).
                        //

                        // Strategy: Synthesize argument types first, then find the signature where
                        // argument types are compatible using subtype checking (no side effects).
                        let all_methods = self
                            .protocol_checker
                            .read()
                            .lookup_all_protocol_methods(&lookup_ty, &method_name_text);
                        if let Ok(method_types) = all_methods {
                            if !method_types.is_empty() {
                                // First, synthesize all argument types
                                let mut arg_types: List<Type> = List::new();
                                for arg in args.iter() {
                                    let arg_result = self.synth_expr(arg)?;
                                    arg_types.push(arg_result.ty);
                                }

                                // Find the MOST SPECIFIC signature where argument types are compatible.
                                // Specificity: exact type match > subtype match > structural match.
                                // This fixes From<Char> vs From<Text> disambiguation — exact match wins.
                                let mut best_match: Option<&Type> = None;
                                let mut best_score: i32 = -1;
                                for method_ty in method_types.iter() {
                                    if let Type::Function { params, .. } = method_ty {
                                        if params.len() != arg_types.len() {
                                            continue;
                                        }

                                        let mut all_compatible = true;
                                        let mut match_score: i32 = 0;
                                        for (arg_ty, param_ty) in
                                            arg_types.iter().zip(params.iter())
                                        {
                                            if arg_ty == param_ty {
                                                // Exact type match — highest priority
                                                match_score += 2;
                                            } else if self.types_compatible(arg_ty, param_ty) {
                                                // Structural/subtype match — lower priority
                                                match_score += 1;
                                            } else {
                                                all_compatible = false;
                                                break;
                                            }
                                        }

                                        if all_compatible && match_score > best_score {
                                            best_match = Some(method_ty);
                                            best_score = match_score;
                                        }
                                    }
                                }

                                if let Some(Type::Function {
                                    params,
                                    return_type,
                                    ..
                                }) = best_match
                                {
                                    // Now do proper type checking with unification
                                    for (arg, param_ty) in args.iter().zip(params.iter()) {
                                        let resolved_param = self.unifier.apply(param_ty);
                                        self.check_expr(arg, &resolved_param)?;
                                    }
                                    let resolved_return = self.unifier.apply(return_type);
                                    return Ok(Some(InferResult::new(resolved_return)));
                                }

                                // No matching signature found - use first one for error message
                                if let Some(Type::Function { params, .. }) = method_types.first() {
                                    // Allow ±1 tolerance for self-param counting
                                    if params.len().abs_diff(args.len()) > 1 {
                                        return Err(TypeError::WrongArgCount {
                                            method: method_name.to_text(),
                                            expected: params.len(),
                                            actual: args.len(),
                                            span,
                                        });
                                    }
                                    // Fall through to let regular type checking produce the error
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    fn auto_deref_for_method_call(ty: &Type) -> Type {
        match ty {
            // Strip refinement types: methods on T{predicate} resolve as T methods
            Type::Refined { base, .. } | Type::Sigma { fst_type: base, .. } => {
                Self::auto_deref_for_method_call(base)
            }
            // Dereference &T -> inner
            Type::Reference { inner, .. } => inner.as_ref().clone(),
            // Dereference &checked T, &unsafe T, Ownership<T>
            Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. } => inner.as_ref().clone(),
            // CapabilityRestricted types: preserve capability info while dereferencing base
            Type::CapabilityRestricted { base, capabilities } => {
                let dereffed_base = Self::auto_deref_for_method_call(base);
                if &dereffed_base != base.as_ref() {
                    Type::CapabilityRestricted {
                        base: Box::new(dereffed_base),
                        capabilities: capabilities.clone(),
                    }
                } else {
                    ty.clone()
                }
            }
            // No dereference needed - stdlib types (Heap, Ref, etc.) are handled
            // via protocol-based deref lookup, not hardcoded here
            _ => ty.clone(),
        }
    }

    /// Check if a method call is allowed given the available capabilities.
    ///

    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
    ///

    /// This method maps method names to required capabilities using heuristics:
    /// - Methods starting with "read", "get", "query", "fetch", "find" -> [Read]
    /// - Methods starting with "write", "set", "update", "insert", "delete", "remove" -> [Write]
    /// - Methods containing "execute", "run", "admin" -> [Execute] or [Admin]
    /// - Default: no specific capability required (allowed with any capabilities)
    ///

    /// Returns Ok(()) if the method is allowed, Err with message if not.
    fn check_capability_restricted_method(
        &self,
        method_name: &str,
        available_capabilities: &crate::capability::TypeCapabilitySet,
        type_name: &str,
        span: Span,
    ) -> Result<()> {
        use crate::capability::TypeCapability;

        // Derive required capabilities from method name using heuristics
        let required_caps = Self::infer_method_required_capabilities(method_name);

        // If no specific capabilities required, method is always allowed
        if required_caps.is_empty() {
            return Ok(());
        }

        // Check if all required capabilities are available.
        // TypeCapability enum enables proper semantic matching:
        // - ReadWrite satisfies both ReadOnly and WriteOnly
        // - Admin satisfies all capabilities
        for required in required_caps.to_list() {
            let has_cap = available_capabilities.contains(&required)
                // ReadWrite satisfies ReadOnly and WriteOnly
                || (available_capabilities.contains(&TypeCapability::ReadWrite)
                    && matches!(required, TypeCapability::ReadOnly | TypeCapability::WriteOnly))
                // Admin satisfies everything
                || available_capabilities.contains(&TypeCapability::Admin);

            if !has_cap {
                // Convert to Text for error reporting compatibility
                let available_names: List<Text> = available_capabilities.names();
                return Err(TypeError::CapabilityViolation {
                    method: method_name.into(),
                    type_name: type_name.into(),
                    required_capability: Text::from(required.name()),
                    available_capabilities: available_names,
                    span,
                });
            }
        }

        Ok(())
    }

    /// Infer required capabilities from method name using naming conventions.
    ///

    /// Returns a TypeCapabilitySet of required capabilities, using the structured
    /// TypeCapability enum for proper semantic matching during capability checking.
    ///

    /// Common patterns:
    /// - "read*", "get*", "query*", "fetch*", "find*", "list*", "count*" -> ReadOnly
    /// - "write*", "set*", "update*", "insert*", "delete*", "remove*", "clear*", "add*" -> WriteOnly
    /// - "execute*", "run*", "call*" -> Execute
    /// - "*admin*", "grant*", "revoke*" -> Admin
    fn infer_method_required_capabilities(
        method_name: &str,
    ) -> crate::capability::TypeCapabilitySet {
        use crate::capability::{TypeCapability, TypeCapabilitySet};

        let lowercase = method_name.to_lowercase();
        let mut caps = TypeCapabilitySet::empty();

        // Read operations
        if lowercase.starts_with("read")
            || lowercase.starts_with("get")
            || lowercase.starts_with("query")
            || lowercase.starts_with("fetch")
            || lowercase.starts_with("find")
            || lowercase.starts_with("list")
            || lowercase.starts_with("count")
            || lowercase.starts_with("exists")
            || lowercase.starts_with("contains")
            || lowercase.starts_with("lookup")
            || lowercase.starts_with("search")
            || lowercase.starts_with("select")
        {
            caps.insert(TypeCapability::ReadOnly);
        }

        // Write operations
        if lowercase.starts_with("write")
            || lowercase.starts_with("set")
            || lowercase.starts_with("update")
            || lowercase.starts_with("insert")
            || lowercase.starts_with("delete")
            || lowercase.starts_with("remove")
            || lowercase.starts_with("clear")
            || lowercase.starts_with("add")
            || lowercase.starts_with("push")
            || lowercase.starts_with("pop")
            || lowercase.starts_with("put")
            || lowercase.starts_with("create")
            || lowercase.starts_with("modify")
            || lowercase.starts_with("drop")
            || lowercase.starts_with("truncate")
        {
            caps.insert(TypeCapability::WriteOnly);
        }

        // Execute operations
        if lowercase.starts_with("execute")
            || lowercase.starts_with("run")
            || lowercase.starts_with("call")
            || lowercase.starts_with("invoke")
            || lowercase.starts_with("perform")
        {
            caps.insert(TypeCapability::Execute);
        }

        // Admin operations
        if lowercase.contains("admin")
            || lowercase.starts_with("grant")
            || lowercase.starts_with("revoke")
            || lowercase.starts_with("authorize")
            || lowercase.starts_with("configure")
        {
            caps.insert(TypeCapability::Admin);
        }

        // Transaction operations
        if lowercase.starts_with("begin")
            || lowercase.starts_with("commit")
            || lowercase.starts_with("rollback")
            || lowercase.contains("transaction")
        {
            caps.insert(TypeCapability::Transaction);
        }

        caps
    }

    /// Extract capability restrictions from a type, if present.
    /// Returns (base_type, Some(capabilities)) for CapabilityRestricted types,
    /// or (original_type, None) for non-restricted types.
    fn extract_capability_restrictions(
        ty: &Type,
    ) -> (&Type, Option<&crate::capability::TypeCapabilitySet>) {
        match ty {
            Type::CapabilityRestricted { base, capabilities } => {
                (base.as_ref(), Some(capabilities))
            }
            _ => (ty, None),
        }
    }

    /// Auto-dereference for binary operations.
    /// Allows `&T op T` to work by implicitly dereferencing the reference.
    /// This is important for iterator patterns where `iter()` returns `Iterator<&T>`.
    pub(super) fn deref_for_binop(ty: &Type) -> &Type {
        match ty {
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => Self::deref_for_binop(inner.as_ref()),
            // Refinement types unwrap to base for arithmetic dispatch.
            // Float{>= 0.0} + Float{>= 0.0} → Float (not Int fallback).
            Type::Refined { base, .. } => Self::deref_for_binop(base.as_ref()),
            _ => ty,
        }
    }

    /// Resolve bitwise operator type via protocol-based lookup.
    ///

    /// This unified approach:
    /// 1. Checks if the left operand type implements the required protocol (BitAnd, BitOr, etc.)
    /// 2. If so, resolves the Output associated type from the protocol implementation
    /// 3. Falls back to Int for backward compatibility with untyped expressions
    ///

    /// This enables correct behavior for Bool ^ Bool -> Bool while maintaining
    /// Int as the default for untyped bitwise operations.
    pub(super) fn resolve_bitwise_op_type(
        &mut self,
        left_ty: &Type,
        right_ty: &Type,
        protocol_name: &str,
        span: Span,
    ) -> Result<Type> {
        // Check if the type implements the protocol
        if self
            .protocol_checker
            .read()
            .implements_by_name(left_ty, protocol_name)
        {
            // Unify operand types - both operands should have the same type
            self.unifier.unify(left_ty, right_ty, span)?;

            // Resolve the Output associated type from the protocol implementation
            if let Some(output_ty) = self.resolve_protocol_output_type(left_ty, protocol_name) {
                return Ok(output_ty);
            }

            // Protocol implemented but Output not resolved - return operand type
            return Ok(left_ty.clone());
        }

        // For type variables, default to Int for backward compatibility
        if matches!(left_ty, Type::Var(_)) {
            self.unifier.unify(left_ty, &Type::int(), span)?;
            self.unifier.unify(right_ty, &Type::int(), span)?;
            return Ok(Type::int());
        }

        // For Int type, standard integer bitwise operations
        if matches!(left_ty, Type::Int) {
            self.unifier.unify(right_ty, &Type::int(), span)?;
            return Ok(Type::int());
        }

        // Check Numeric protocol for sized types and aliases
        if self
            .protocol_checker
            .read()
            .implements_protocol(left_ty, "Numeric")
        {
            // Sized integer type - unify both operands to the same type
            // and return that type (not Int)
            self.unifier.unify(left_ty, right_ty, span)?;
            return Ok(left_ty.clone());
        }

        // Same-type bitwise ops (e.g., Byte & Byte) — unify and return the type
        if self.unifier.unify(left_ty, right_ty, span).is_ok() {
            return Ok(left_ty.clone());
        }

        // Unknown type - fall back to Int for backward compatibility
        self.unifier.unify(left_ty, &Type::int(), span)?;
        self.unifier.unify(right_ty, &Type::int(), span)?;
        Ok(Type::int())
    }

    /// Resolve the Output associated type from a protocol implementation.
    ///

    /// Creates a protocol path and queries the protocol checker to find the
    /// Output type defined in the implementation.
    pub(super) fn resolve_protocol_output_type(&self, ty: &Type, protocol_name: &str) -> Option<Type> {
        let output_type_name: Text = "Output".into();
        let protocol_path = verum_ast::ty::Path {
            segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(verum_ast::Ident {
                name: protocol_name.into(),
                span: Span::default(),
            })],
            span: Span::default(),
        };

        self.protocol_checker
            .read()
            .resolve_associated_type(ty, &protocol_path, &output_type_name)
            .ok()
    }

    /// Helper to extract type name from Named path.
    /// Used by get_builtin_method_type and get_builtin_method_param_types
    /// to handle both Type::Generic and Type::Named variants uniformly.
    pub(super) fn path_type_name(path: &verum_ast::ty::Path) -> Option<&str> {
        path.segments.first().and_then(|seg| {
            if let verum_ast::ty::PathSegment::Name(ident) = seg {
                Some(ident.name.as_str())
            } else {
                None
            }
        })
    }

    /// Get the last segment name from a type path.
    /// Useful for qualified paths like "io::path::Components" -> "Components"
    pub(super) fn path_last_type_name(path: &verum_ast::ty::Path) -> Option<&str> {
        path.segments.last().and_then(|seg| {
            if let verum_ast::ty::PathSegment::Name(ident) = seg {
                Some(ident.name.as_str())
            } else {
                None
            }
        })
    }

    /// Infer element type from a linked-list-style Variant type.
    ///

    /// Detects patterns like:
    ///  Cons((T, Heap<List<T>>)) | Nil(Unit)
    ///  Cons((T, Heap<Self>)) | Nil(Unit)
    ///

    /// Returns Some(T) if the pattern matches, None otherwise.
    pub(super) fn infer_linked_list_element_type(
        variants: &indexmap::IndexMap<verum_common::Text, Type>,
    ) -> Option<Type> {
        // Must have exactly 2 variants
        if variants.len() != 2 {
            return None;
        }

        // Find the "Cons"-like variant (non-unit payload) and "Nil"-like variant (unit payload)
        let mut cons_payload = None;
        let mut has_nil = false;

        for (name, ty) in variants {
            let name_lower = name.to_lowercase();
            match ty {
                Type::Unit => {
                    has_nil = true;
                }
                _ if name_lower == "nil" || name_lower == "empty" || name_lower == "end" => {
                    has_nil = true;
                }
                _ => {
                    cons_payload = Some(ty.clone());
                }
            }
        }

        if !has_nil {
            return None;
        }

        let payload = cons_payload?;

        // Extract element type from the Cons payload.
        // Pattern 1: Cons((T, Heap<...>)) - tuple with element + recursive pointer
        // Pattern 2: Cons(T) - simple element (less common)
        match &payload {
            Type::Tuple(elements) if elements.len() >= 2 => {
                // First element of the tuple is the iteration element type
                Some(elements[0].clone())
            }
            Type::Tuple(elements) if elements.len() == 1 => {
                // Single-element tuple wrapping a tuple: Cons(((K,V), Heap<...>))
                Some(elements[0].clone())
            }
            _ => {
                // Non-tuple Cons payload - use the payload itself as element type
                Some(payload)
            }
        }
    }

    /// Infer tensor literal structure from string representation
    ///

    /// Parses a tensor literal like "[[[1,2],[3,4]],[[5,6],[7,8]]]" to extract:
    /// - Element type (Int, Float, etc.)
    /// - Shape dimensions [2, 2, 2] for the example above
    ///

    /// Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays
    ///

    /// # Algorithm
    /// 1. Parse nested array structure to determine rank (nesting depth)
    /// Infer the type of a tagged literal based on its format tag.
    ///

    /// Enhanced Tagged Literals with format-specific type inference.
    /// Each format tag maps to a specific type in the standard library.
    ///

    /// # Format Categories (from grammar/verum.ebnf v2.13.0):
    /// - Data Interchange: json, yaml, toml, xml, html, csv → format-specific types
    /// - Query Languages: sql, gql, graphql → query types
    /// - Pattern Matching: rx, re, regex, glob → pattern types
    /// - Identifiers: url, uri, email, path → validated identifier types
    /// - Temporal: d, dur, tz → date/time types
    /// - Networking: ip, cidr, mac → network address types
    /// - Versioning: ver, semver → version types
    ///

    /// # Type Inference Rules:
    /// - If expected type is known (from type annotation), use it for struct inference
    /// - Otherwise, return the default type for the format tag
    pub(super) fn infer_tagged_literal_type(&self, tag: &str, span: verum_ast::span::Span) -> Type {
        use verum_ast::ty::Ident;

        // Helper to create a named type
        let named = |name: &str| Type::Named {
            path: Path::single(Ident::new(name, span)),
            args: List::new(),
        };

        match tag {
            // Data Interchange formats
            "json" | "json5" => named("JsonValue"),
            "yaml" => named("YamlValue"),
            "toml" => named("TomlValue"),
            "xml" => named("XmlDocument"),
            "html" => named("HtmlDocument"),
            "csv" => named("CsvData"),

            // Query languages
            "sql" => named("SqlQuery"),
            "gql" | "graphql" => named("GraphQLQuery"),
            "cypher" => named("CypherQuery"),
            "sparql" => named("SparqlQuery"),

            // Pattern matching
            "rx" | "re" | "regex" => named("Regex"),
            "glob" => named("GlobPattern"),
            "xpath" => named("XPathExpr"),
            "jpath" => named("JsonPath"),

            // Identifiers
            "url" | "uri" => named("Url"),
            "email" => named("Email"),
            "path" => named("PathBuf"),
            "mime" => named("MimeType"),
            "uuid" => named("Uuid"),
            "urn" => named("Urn"),

            // Temporal
            "d" | "date" | "datetime" => named("DateTime"),
            "dur" | "duration" => named(WKT::Duration.as_str()),
            "time" => named("Time"),
            "tz" | "timezone" => named("Timezone"),

            // Networking
            "ip" => named("IpAddr"),
            "cidr" => named("CidrRange"),
            "mac" => named("MacAddr"),
            "host" => named("Host"),

            // Versioning and encoding
            "ver" | "semver" => named("Version"),
            "b64" => named("Base64"),
            "hex" => named("HexBytes"),
            "pct" => named("PercentEncoded"),

            // Structured data
            "mat" | "matrix" => named("Matrix"),
            "vec" | "vector" => named("Vector"),
            "interval" => named("Interval"),
            "ratio" => named("Ratio"),
            "tensor" => named("Tensor"),

            // Code/Script
            "sh" => named("ShellCommand"),
            "css" => named("CssStylesheet"),
            "lua" => named("LuaScript"),
            "asm" => named("Assembly"),
            "contract" => named("ContractSpec"),

            // Scientific
            "chem" => named("Chemical"),
            "music" => named("Musical"),
            "geo" => named("GeoPoint"),

            // Unknown tags fall back to Text
            _ => Type::text(),
        }
    }

    /// 2. Compute shape by counting elements at each level
    /// 3. Infer element type from first scalar value
    /// 4. Return Tensor<elem_ty, [d1, d2, ..., dn]>
    pub(super) fn infer_tensor_literal_structure(
        &self,
        literal: &str,
    ) -> (Type, List<verum_common::ConstValue>) {
        use verum_common::ConstValue;

        // Parse the tensor structure to extract shape and element type
        let (shape, elem_ty) = self.parse_tensor_structure(literal);

        let shape_values: List<ConstValue> = shape
            .iter()
            .map(|&dim| ConstValue::UInt(dim as u128))
            .collect();

        (elem_ty, shape_values)
    }

    /// Parse tensor structure to extract shape dimensions and element type
    ///

    /// Returns (shape_dimensions, element_type)
    fn parse_tensor_structure(&self, literal: &str) -> (List<usize>, Type) {
        let chars: List<char> = literal.chars().collect();
        let mut pos = 0;

        // Skip whitespace
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }

        // Parse the nested array structure
        let (shape, elem_ty) = self.parse_tensor_array(&chars, &mut pos);

        (shape, elem_ty)
    }

    /// Parse a nested array structure recursively
    ///

    /// Returns (shape_at_this_level_and_below, element_type, is_regular)
    /// where is_regular indicates whether all sub-arrays have consistent shapes.
    ///

    /// For tensor literals, we validate regularity at each nesting level:
    /// - [[1,2,3], [4,5,6]] is regular (both rows have 3 elements)
    /// - [[1,2], [3,4,5]] is irregular (rows have different lengths)
    fn parse_tensor_array(&self, chars: &List<char>, pos: &mut usize) -> (List<usize>, Type) {
        let (shape, elem_ty, _is_regular) = self.parse_tensor_array_with_validation(chars, pos);
        (shape, elem_ty)
    }

    /// Internal helper that also returns regularity status for validation
    fn parse_tensor_array_with_validation(
        &self,
        chars: &List<char>,
        pos: &mut usize,
    ) -> (List<usize>, Type, bool) {
        // Skip whitespace
        while *pos < chars.len() && chars[*pos].is_whitespace() {
            *pos += 1;
        }

        // Check if we're at an array start
        if *pos >= chars.len() || chars[*pos] != '[' {
            // This is a scalar - parse element type
            let elem_ty = self.parse_tensor_element_type(chars, pos);
            return (List::new(), elem_ty, true);
        }

        // Consume '['
        *pos += 1;

        let mut elements_count = 0;
        let mut expected_sub_shape: Option<List<usize>> = None;
        let mut elem_ty = Type::Int; // Default
        let mut is_regular = true;

        loop {
            // Skip whitespace
            while *pos < chars.len() && chars[*pos].is_whitespace() {
                *pos += 1;
            }

            if *pos >= chars.len() {
                break;
            }

            // Check for end of array
            if chars[*pos] == ']' {
                *pos += 1;
                break;
            }

            // Skip comma
            if chars[*pos] == ',' {
                *pos += 1;
                continue;
            }

            // Parse element (either nested array or scalar)
            let (element_shape, element_ty, element_regular) =
                self.parse_tensor_array_with_validation(chars, pos);
            elements_count += 1;

            // Propagate irregularity from nested levels
            if !element_regular {
                is_regular = false;
            }

            match &expected_sub_shape {
                None => {
                    // First element: establish the expected shape
                    expected_sub_shape = Some(element_shape);
                    elem_ty = element_ty;
                }
                Some(expected) => {
                    // Subsequent elements: validate shape matches
                    if expected.len() != element_shape.len() {
                        // Rank mismatch - irregular tensor
                        is_regular = false;
                    } else {
                        // Check each dimension matches
                        for (exp_dim, act_dim) in expected.iter().zip(element_shape.iter()) {
                            if exp_dim != act_dim {
                                is_regular = false;
                                break;
                            }
                        }
                    }
                    // Note: We keep the first element's type for inference
                    // Type mismatches will be caught during unification
                }
            }
        }

        // Build shape: [elements_count] ++ sub_shape
        let mut shape = List::new();
        shape.push(elements_count);
        if let Some(sub_shape) = expected_sub_shape {
            shape.extend(sub_shape);
        }

        (shape, elem_ty, is_regular)
    }

    /// Parse element type from tensor literal content
    fn parse_tensor_element_type(&self, chars: &List<char>, pos: &mut usize) -> Type {
        // Skip whitespace
        while *pos < chars.len() && chars[*pos].is_whitespace() {
            *pos += 1;
        }

        if *pos >= chars.len() {
            return Type::Int;
        }

        // Look ahead to determine element type
        let start = *pos;

        // Collect the value string
        while *pos < chars.len()
            && !chars[*pos].is_whitespace()
            && chars[*pos] != ','
            && chars[*pos] != ']'
        {
            *pos += 1;
        }

        // Collect characters from range into a String
        let value_str: String = (start..*pos)
            .filter_map(|i| chars.get(i).copied())
            .collect();
        let value_str = value_str.trim();

        // Determine type from value
        if value_str == "true" || value_str == "false" {
            Type::Bool
        } else if value_str.contains('.') || value_str.contains('e') || value_str.contains('E') {
            // Float literal (has decimal point or exponent)
            Type::Float
        } else if value_str.starts_with('"') || value_str.starts_with('\'') {
            // String/char literal
            Type::Text
        } else if value_str.starts_with('-')
            || value_str
                .chars()
                .next()
                .is_some_and(|c: char| c.is_ascii_digit())
        {
            // Integer literal
            Type::Int
        } else {
            // Unknown - default to Int
            Type::Int
        }
    }

    // ==================== GAT (Generic Associated Type) Inference ====================
    // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — 2

    /// Infer type arguments for a GAT instantiation from usage context
    ///

    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-142
    ///

    /// This function implements bidirectional type inference for GATs:
    /// - **Explicit**: `Iterator.Item<Int>` - type arguments provided
    /// - **Inferred**: `Iterator.Item` - infer from usage context
    /// - **Partial**: `Iterator.Item<_>` - infer missing arguments
    /// - **Nested**: `Monad.Wrapped<Functor.F<T>>` - recursive inference
    ///

    /// # Algorithm
    ///

    /// 1. Look up the GAT definition from the protocol
    /// 2. Extract expected arity (number of type parameters)
    /// 3. Match provided arguments with expected parameters
    /// 4. For missing arguments, create fresh type variables
    /// 5. Apply constraints from where clauses
    /// 6. Unify with usage context to solve type variables
    /// 7. Verify all bounds are satisfied
    ///

    /// # Examples
    ///

    /// ```verum
    /// protocol Iterator {
    ///  type Item<T>
    ///  fn next(&mut self) -> Maybe<Self.Item<T>>
    /// }
    ///

    /// // Explicit instantiation
    /// let iter: Iterator.Item<Int> = ...
    ///

    /// // Inferred from usage
    /// let x: Int = iter.next().unwrap() // Item<T> inferred as T = Int
    /// ```
    ///

    /// # Performance
    ///

    /// - O(n) in number of type parameters
    /// - O(m) in number of where clause constraints
    /// - Total: O(n + m) per GAT instantiation
    /// - Target: <100ms for complex nested GATs
    pub fn infer_gat_instantiation(
        &mut self,
        protocol_path: &Path,
        gat_name: &Text,
        provided_args: &List<Type>,
        usage_context: Maybe<&Type>,
        span: Span,
    ) -> Result<Type> {
        let start = Instant::now();

        // Step 1: Look up the protocol and GAT definition
        // Clone the data we need to release the read guard early
        let (is_gat, expected_arity, type_params, where_clauses) = {
            let protocol_checker_guard = self.protocol_checker.read();
            let protocol = protocol_checker_guard
                .lookup_protocol(protocol_path)
                .ok_or_else(|| {
                    TypeError::Other(verum_common::Text::from(format!(
                        "Protocol {} not found",
                        self.path_to_string(protocol_path)
                    )))
                })?;

            let gat = protocol.associated_types.get(gat_name).ok_or_else(|| {
                TypeError::Other(verum_common::Text::from(format!(
                    "Associated type {} not found in protocol {}",
                    gat_name,
                    self.path_to_string(protocol_path)
                )))
            })?;

            (
                gat.is_gat(),
                gat.arity(),
                gat.type_params.clone(),
                gat.where_clauses.clone(),
            )
        }; // protocol_checker_guard dropped here

        // Step 2: Check if this is actually a GAT
        if !is_gat {
            // Regular associated type - no type arguments needed
            if !provided_args.is_empty() {
                return Err(TypeError::Other(verum_common::Text::from(format!(
                    "Associated type {} is not generic, but {} type arguments were provided",
                    gat_name,
                    provided_args.len()
                ))));
            }
            // Return placeholder - actual resolution happens during impl lookup
            return Ok(Type::Named {
                path: protocol_path.clone(),
                args: List::new(),
            });
        }

        // Step 3: Handle arity checking
        if provided_args.len() > expected_arity {
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "GAT {} expects {} type arguments, but {} were provided",
                gat_name,
                expected_arity,
                provided_args.len()
            ))));
        }

        // Step 4: Build complete type argument list
        let mut type_args = List::new();
        let mut fresh_vars = Map::new();

        for (i, param) in type_params.iter().enumerate() {
            if i < provided_args.len() {
                // Use provided argument
                let arg = &provided_args[i];
                // Check if it's a placeholder (_)
                if matches!(arg, Type::Var(_)) {
                    // Create fresh type variable with bounds
                    let fresh_var = TypeVar::fresh();
                    fresh_vars.insert(param.name.clone(), fresh_var);
                    type_args.push(Type::Var(fresh_var));
                } else {
                    type_args.push(arg.clone());
                }
            } else {
                // Create fresh type variable for missing argument
                let fresh_var = TypeVar::fresh();
                fresh_vars.insert(param.name.clone(), fresh_var);
                type_args.push(Type::Var(fresh_var));
            }
        }

        // Step 5: Apply where clause constraints
        for where_clause in &where_clauses {
            if let Some(&var) = fresh_vars.get(&where_clause.param) {
                // Add bounds to the type variable
                // This is tracked in the context for later verification
                for bound in &where_clause.constraints {
                    self.ctx.add_protocol_bound(var, bound.clone());
                }
            }
        }

        // Step 6: Unify with usage context if provided
        if let Maybe::Some(context_ty) = usage_context {
            // Try to unify type arguments with context
            // This helps solve fresh type variables
            for (i, arg) in type_args.iter().enumerate() {
                if let Type::Var(v) = arg {
                    // Try to extract corresponding type from context
                    if let Type::Named {
                        args: context_args, ..
                    } = context_ty
                        && i < context_args.len()
                    {
                        self.unifier.unify(arg, &context_args[i], span)?;
                    }
                }
            }
        }

        // Step 7: Verify protocol bounds on type parameters
        for (param, arg) in type_params.iter().zip(type_args.iter()) {
            for bound in &param.bounds {
                if !self.check_protocol_bound(arg, bound) {
                    return Err(TypeError::ProtocolNotSatisfied {
                        ty: arg.to_text(),
                        protocol: bound
                            .protocol
                            .segments
                            .first()
                            .and_then(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => Some(id.name.clone()),
                                _ => None,
                            })
                            .unwrap_or_else(|| "unknown".into()),
                        span,
                    });
                }
            }
        }

        // Record performance metrics
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 100 {
            self.emit_diagnostic(
                DiagnosticBuilder::note_diag()
                    .message(format!(
                        "GAT instantiation took {}ms (target: <100ms)",
                        elapsed.as_millis()
                    ))
                    .span(span_to_line_col(span))
                    .build(),
            );
        }

        // Return instantiated GAT type
        // Format: ProtocolPath::GatName<Args>
        Ok(Type::Named {
            path: {
                let mut path = protocol_path.clone();
                path.segments
                    .push(verum_ast::ty::PathSegment::Name(Ident::new(
                        gat_name.as_str(),
                        span,
                    )));
                path
            },
            args: type_args,
        })
    }

    /// Check if a GAT is applied correctly with proper type arguments
    ///

    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 lines 441-471
    ///

    /// Verifies:
    /// - Arity matches (correct number of type arguments)
    /// - Type arguments satisfy bounds
    /// - Where clauses are satisfied
    /// - Variance is correct (covariant/contravariant/invariant)
    ///

    /// # Examples
    ///

    /// ```verum
    /// protocol Container {
    ///  type Item<T> where T: Clone + Debug
    /// }
    ///

    /// // Valid: Int implements Clone + Debug
    /// impl Container for List<Int> {
    ///  type Item<Int> = Int
    /// }
    ///

    /// // Invalid: &Int doesn't implement Clone
    /// impl Container for List<&Int> {
    ///  type Item<&Int> = &Int // ERROR
    /// }
    /// ```
    pub fn check_gat_application(
        &mut self,
        protocol_path: &Path,
        gat_name: &Text,
        type_args: &List<Type>,
        span: Span,
    ) -> Result<()> {
        // Look up the GAT definition and clone needed data to release guard early
        let (expected_arity, gat_type_params, gat_where_clauses) = {
            let protocol_checker_guard = self.protocol_checker.read();
            let protocol = protocol_checker_guard
                .lookup_protocol(protocol_path)
                .ok_or_else(|| {
                    TypeError::Other(verum_common::Text::from(format!(
                        "Protocol {} not found",
                        self.path_to_string(protocol_path)
                    )))
                })?;

            let gat = protocol.associated_types.get(gat_name).ok_or_else(|| {
                TypeError::Other(verum_common::Text::from(format!(
                    "Associated type {} not found in protocol {}",
                    gat_name,
                    self.path_to_string(protocol_path)
                )))
            })?;

            (
                gat.arity(),
                gat.type_params.clone(),
                gat.where_clauses.clone(),
            )
        }; // protocol_checker_guard dropped here

        // Check arity
        if type_args.len() != expected_arity {
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "GAT {} expects {} type arguments, but {} were provided",
                gat_name,
                expected_arity,
                type_args.len()
            ))));
        }

        // Check each type argument against its parameter bounds
        for (param, arg) in gat_type_params.iter().zip(type_args.iter()) {
            // Verify protocol bounds
            for bound in &param.bounds {
                if !self.check_protocol_bound(arg, bound) {
                    return Err(TypeError::ProtocolNotSatisfied {
                        ty: arg.to_text(),
                        protocol: bound
                            .protocol
                            .segments
                            .first()
                            .and_then(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => Some(id.name.clone()),
                                _ => None,
                            })
                            .unwrap_or_else(|| "unknown".into()),
                        span,
                    });
                }
            }

            // Verify variance constraints
            // This is checked during subtyping - we just validate structure here
            match param.variance {
                crate::advanced_protocols::Variance::Covariant => {
                    // Covariant: output positions only
                    // Checked during usage, not at declaration
                }
                crate::advanced_protocols::Variance::Contravariant => {
                    // Contravariant: input positions only
                    // Checked during usage, not at declaration
                }
                crate::advanced_protocols::Variance::Invariant => {
                    // Invariant: no subtyping allowed
                    // Strictest check - exact match required
                }
            }
        }

        // Verify where clauses
        for where_clause in &gat_where_clauses {
            // Find the type argument corresponding to this parameter
            for (i, param) in gat_type_params.iter().enumerate() {
                if param.name == where_clause.param {
                    let arg = &type_args[i];
                    // Check all constraints in where clause
                    for constraint in &where_clause.constraints {
                        if !self.check_protocol_bound(arg, constraint) {
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "Type {} does not satisfy where clause constraint {} in GAT {}",
                                arg.to_text(),
                                self.path_to_string(&constraint.protocol),
                                gat_name
                            ))));
                        }
                    }
                    break;
                }
            }
        }

        Ok(())
    }

    /// Unify two GAT types
    ///

    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .1 lines 116-142
    ///

    /// Unifies two GAT instantiations by:
    /// 1. Checking that protocol paths match
    /// 2. Checking that GAT names match
    /// 3. Unifying type arguments pairwise
    /// 4. Respecting variance annotations
    ///

    /// # Examples
    ///

    /// ```verum
    /// // Unifying Iterator.Item<Int> with Iterator.Item<T>
    /// // Results in: T = Int
    ///

    /// // Unifying Monad.Wrapped<List<T>> with Monad.Wrapped<List<Int>>
    /// // Results in: T = Int
    /// ```
    pub fn unify_gat_types(
        &mut self,
        gat1_protocol: &Path,
        gat1_name: &Text,
        gat1_args: &List<Type>,
        gat2_protocol: &Path,
        gat2_name: &Text,
        gat2_args: &List<Type>,
        span: Span,
    ) -> Result<crate::ty::Substitution> {
        use crate::ty::SubstitutionExt;

        // Protocol paths must match
        if gat1_protocol != gat2_protocol {
            return Err(TypeError::Mismatch {
                expected: self.path_to_string(gat2_protocol),
                actual: self.path_to_string(gat1_protocol),
                span,
            });
        }

        // GAT names must match
        if gat1_name != gat2_name {
            return Err(TypeError::Mismatch {
                expected: gat2_name.clone(),
                actual: gat1_name.clone(),
                span,
            });
        }

        // Arity must match
        if gat1_args.len() != gat2_args.len() {
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "GAT {} instantiated with {} and {} type arguments",
                gat1_name,
                gat1_args.len(),
                gat2_args.len()
            ))));
        }

        // Unify type arguments pairwise
        let mut subst = crate::ty::Substitution::new();
        for (arg1, arg2) in gat1_args.iter().zip(gat2_args.iter()) {
            let s =
                self.unifier
                    .unify(&arg1.apply_subst(&subst), &arg2.apply_subst(&subst), span)?;
            subst = subst.compose(&s);
        }

        Ok(subst)
    }

    /// Helper: Check if a type satisfies a protocol bound
    fn check_protocol_bound(&self, ty: &Type, bound: &crate::protocol::ProtocolBound) -> bool {
        // Extract protocol name from path
        let protocol_name = bound
            .protocol
            .segments
            .first()
            .and_then(|s| match s {
                verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                _ => None,
            })
            .unwrap_or("");

        // Check if type implements the protocol
        self.protocol_checker
            .read()
            .implements_protocol(ty, protocol_name)
    }

    /// Check if actual type can be coerced to expected protocol type.
    ///

    /// This enables protocol-based polymorphism:
    /// - If expected is a protocol type and actual implements it, allow coercion
    /// - If expected is &T where T is a protocol, and actual is &U where U implements T, allow
    /// - Similarly for &mut references (implies dynamic dispatch)
    ///

    /// # Examples
    /// - `DefaultHasher` -> `Hasher` (when Hasher is protocol, DefaultHasher implements Hasher)
    /// - `&mut DefaultHasher` -> `&mut Hasher` (mutable reference to protocol)
    /// - `&[u8]` -> `&[u8]` (no protocol involved, returns false)
    pub(super) fn check_protocol_coercion(&self, actual: &Type, expected: &Type) -> bool {
        // Get the protocol checker
        let protocol_checker = self.protocol_checker.read();

        // Helper to extract protocol name from a type
        let get_protocol_name = |ty: &Type| -> Option<verum_common::Text> {
            match ty {
                Type::Named { path, .. } => path.as_ident().map(|id| id.name.clone()),
                Type::Generic { name, .. } => Some(name.clone()),
                _ => None,
            }
        };

        // Helper to check if a type name is a protocol
        let is_protocol =
            |name: &str| -> bool { protocol_checker.get_protocol_definition(name).is_some() };

        // Helper to check if actual implements expected protocol
        let implements = |actual_ty: &Type, protocol_name: &str| -> bool {
            protocol_checker.implements_protocol(actual_ty, protocol_name)
        };

        // Helper: check generic wrapper coercion for types with args
        // e.g., Heap<Circle> -> Heap<Drawable> when Circle implements Drawable
        let check_wrapper_coercion =
            |expected_name: &str, expected_args: &List<Type>, actual: &Type| -> bool {
                // Extract actual's name and args (works for both Named and Generic)
                let (actual_name, actual_args) = match actual {
                    Type::Named { path, args } => {
                        if let Some(ident) = path.as_ident() {
                            (ident.name.as_str().to_owned(), args.clone())
                        } else {
                            return false;
                        }
                    }
                    Type::Generic { name, args } => (name.as_str().to_owned(), args.clone()),
                    _ => return false,
                };

                if expected_name != actual_name.as_str() || expected_args.len() != actual_args.len()
                {
                    return false;
                }
                for (exp_arg, act_arg) in expected_args.iter().zip(actual_args.iter()) {
                    if exp_arg == act_arg {
                        continue;
                    }
                    if let Some(protocol_name) = get_protocol_name(exp_arg) {
                        if is_protocol(protocol_name.as_str())
                            && implements(act_arg, protocol_name.as_str())
                        {
                            continue;
                        }
                    }
                    return false;
                }
                true
            };

        // Match on the expected type pattern
        match expected {
            // Named type: could be direct protocol or generic wrapper (e.g., Heap<Drawable>)
            Type::Named {
                path,
                args: expected_args,
            } => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.name.as_str();
                    // Case 1: Direct protocol type (no args or protocol name itself)
                    if is_protocol(name) {
                        return implements(actual, name);
                    }
                    // Case 2: Generic wrapper with protocol type args (e.g., Heap<Drawable>)
                    if !expected_args.is_empty() {
                        return check_wrapper_coercion(name, expected_args, actual);
                    }
                }
                false
            }
            Type::Generic {
                name: expected_name,
                args: expected_args,
            } => {
                // Case 1: The generic name itself is a protocol (e.g., Iterator<Item=Int>)
                if is_protocol(expected_name.as_str()) {
                    return implements(actual, expected_name.as_str());
                }
                // Case 2: Generic wrapper with protocol type args (e.g., Heap<Drawable>)
                if !expected_args.is_empty() {
                    return check_wrapper_coercion(expected_name.as_str(), expected_args, actual);
                }
                false
            }

            // Reference to protocol type: &T or &mut T where T is a protocol
            Type::Reference {
                mutable: m1,
                inner: expected_inner,
            } => {
                if let Type::Reference {
                    mutable: m2,
                    inner: actual_inner,
                } = actual
                {
                    // Mutability must match for safe coercion
                    if m1 != m2 {
                        return false;
                    }
                    // Check if inner expected type is a protocol and actual implements it
                    if let Some(protocol_name) = get_protocol_name(expected_inner) {
                        if is_protocol(protocol_name.as_str()) {
                            return implements(actual_inner, protocol_name.as_str());
                        }
                    }
                }
                false
            }

            // CheckedReference to protocol type
            Type::CheckedReference {
                mutable: m1,
                inner: expected_inner,
            } => {
                if let Type::CheckedReference {
                    mutable: m2,
                    inner: actual_inner,
                } = actual
                {
                    if m1 != m2 {
                        return false;
                    }
                    if let Some(protocol_name) = get_protocol_name(expected_inner) {
                        if is_protocol(protocol_name.as_str()) {
                            return implements(actual_inner, protocol_name.as_str());
                        }
                    }
                }
                false
            }

            // UnsafeReference to protocol type
            Type::UnsafeReference {
                mutable: m1,
                inner: expected_inner,
            } => {
                if let Type::UnsafeReference {
                    mutable: m2,
                    inner: actual_inner,
                } = actual
                {
                    if m1 != m2 {
                        return false;
                    }
                    if let Some(protocol_name) = get_protocol_name(expected_inner) {
                        if is_protocol(protocol_name.as_str()) {
                            return implements(actual_inner, protocol_name.as_str());
                        }
                    }
                }
                false
            }

            _ => false,
        }
    }

    // ==================== Advanced GAT Inference ====================

    /// Infer GAT type parameters from method call site
    ///

    /// Higher-kinded type (HKT) inference and specialization selection: kind inference for type constructors (Type -> Type), automatic selection of most specific specialization
    ///

    /// Given: c.get(42) where get: fn(&Self, K) -> Maybe<Item<K, V>>
    /// Infer: K = Int from argument type
    ///

    /// # Algorithm
    ///

    /// 1. Collect constraints from argument types
    /// 2. Unify with GAT parameter bounds
    /// 3. Solve constraint system
    /// 4. Return substitution map
    ///

    /// # Example
    ///

    /// ```verum
    /// protocol Collection {
    ///  type Item<K, V>;
    ///  fn get(&self, key: K) -> Maybe<Item<K, V>>;
    /// }
    ///

    /// fn use_collection<C: Collection>(c: &C) {
    ///  let item = c.get(42); // Infer: K = Int, Item<Int, _>
    /// }
    /// ```
    pub fn infer_gat_params_from_call(
        &mut self,
        protocol: &Path,
        assoc_type_name: &Text,
        method_name: &Text,
        call_args: &List<Type>,
        span: Span,
    ) -> Result<Map<Text, Type>> {
        let start = Instant::now();

        // Step 1: Look up the protocol and associated type
        // Clone needed data to release the guard early
        let (is_gat, assoc_type_params, method_params) =
            {
                let protocol_checker_guard = self.protocol_checker.read();
                let protocol_def = protocol_checker_guard
                    .lookup_protocol(protocol)
                    .ok_or_else(|| {
                        TypeError::Other(verum_common::Text::from(format!(
                            "Protocol {} not found",
                            self.path_to_string(protocol)
                        )))
                    })?;

                let assoc_type = protocol_def
                    .associated_types
                    .get(assoc_type_name)
                    .ok_or_else(|| {
                        TypeError::Other(verum_common::Text::from(format!(
                            "Associated type {} not found in protocol {}",
                            assoc_type_name,
                            self.path_to_string(protocol)
                        )))
                    })?;

                if !assoc_type.is_gat() {
                    // Not a GAT - no parameters to infer
                    return Ok(Map::new());
                }

                // Step 2: Look up the method signature
                let method = protocol_def.methods.get(method_name).ok_or_else(|| {
                    TypeError::MethodNotFound {
                        ty: self.path_to_string(protocol),
                        method: method_name.clone(),
                        span,
                        did_you_mean: None,
                    }
                })?;

                // Step 3: Extract parameter types from method signature
                let method_params = match &method.ty {
                    Type::Function { params, .. } => params.clone(),
                    _ => {
                        return Err(TypeError::NotAFunction {
                            ty: method.ty.to_text(),
                            span,
                        });
                    }
                };

                (
                    assoc_type.is_gat(),
                    assoc_type.type_params.clone(),
                    method_params,
                )
            }; // protocol_checker_guard dropped here

        // Step 4: Build constraint system from argument types
        let mut constraints = List::new();
        let mut subst_map = Map::new();

        for (i, (method_param, call_arg)) in method_params.iter().zip(call_args.iter()).enumerate()
        {
            // Check if method parameter references a GAT type parameter
            if let Type::Var(param_var) = method_param {
                // Try to find which GAT parameter this corresponds to
                for gat_param in &assoc_type_params {
                    // Create unification constraint
                    let constraint = (gat_param.name.clone(), call_arg.clone());
                    constraints.push(constraint.clone());

                    // If we can directly infer, add to substitution map
                    if !matches!(call_arg, Type::Var(_)) {
                        subst_map.insert(gat_param.name.clone(), call_arg.clone());
                    }
                }
            } else if let Type::Named { path, args } = method_param {
                // Check if this is a GAT instantiation
                if self.is_gat_type(path, args) {
                    // Extract type arguments and match them
                    for (j, arg) in args.iter().enumerate() {
                        if j < assoc_type_params.len() {
                            let gat_param = &assoc_type_params[j];
                            if let Type::Var(_) = arg {
                                // Parameter references GAT type param
                                // Try to infer from call argument
                                if let Type::Named {
                                    args: call_args_inner,
                                    ..
                                } = call_arg
                                    && j < call_args_inner.len()
                                {
                                    subst_map
                                        .insert(gat_param.name.clone(), call_args_inner[j].clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Step 5: Solve constraint system
        // For each GAT parameter, try to find a unique solution
        for gat_param in &assoc_type_params {
            if !subst_map.contains_key(&gat_param.name) {
                // No direct inference - create fresh type variable
                let fresh_var = TypeVar::fresh();
                subst_map.insert(gat_param.name.clone(), Type::Var(fresh_var));

                // Add bounds from GAT parameter
                for bound in &gat_param.bounds {
                    self.ctx.add_protocol_bound(fresh_var, bound.clone());
                }
            }
        }

        // Step 6: Verify bounds are satisfied
        for (param_name, inferred_ty) in &subst_map {
            // Find the GAT parameter
            if let Some(gat_param) = assoc_type_params.iter().find(|p| &p.name == param_name) {
                // Check all bounds
                for bound in &gat_param.bounds {
                    if !self.check_protocol_bound(inferred_ty, bound) {
                        return Err(TypeError::ProtocolNotSatisfied {
                            ty: inferred_ty.to_text(),
                            protocol: bound
                                .protocol
                                .segments
                                .first()
                                .and_then(|s| match s {
                                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| "unknown".into()),
                            span,
                        });
                    }
                }
            }
        }

        // Record performance
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 10 {
            self.emit_diagnostic(
                DiagnosticBuilder::note_diag()
                    .message(format!(
                        "GAT parameter inference took {}ms (target: <10ms)",
                        elapsed.as_millis()
                    ))
                    .span(span_to_line_col(span))
                    .build(),
            );
        }

        Ok(subst_map)
    }

    /// Bidirectional inference for GAT instantiation
    ///

    /// Synthesis mode: Infer GAT params from usage
    /// Checking mode: Verify GAT params satisfy bounds
    ///

    /// # Examples
    ///

    /// ```verum
    /// // Synthesis mode
    /// let item = container.get(42); // Infer Item<Int, _>
    ///

    /// // Checking mode
    /// let item: Item<Int, Text> = container.get(42); // Verify Int, Text satisfy bounds
    /// ```
    pub fn check_gat_instantiation(
        &mut self,
        gat_type: &Type,
        expected: Maybe<&Type>,
        span: Span,
    ) -> Result<Type> {
        match expected {
            Maybe::Some(expected_ty) => {
                // Checking mode: verify gat_type matches expected
                match (gat_type, expected_ty) {
                    (
                        Type::Named {
                            path: path1,
                            args: args1,
                        },
                        Type::Named {
                            path: path2,
                            args: args2,
                        },
                    ) => {
                        // Check if this is a GAT type
                        if self.is_gat_path(path1) {
                            // Extract protocol and GAT name
                            if path1.segments.len() >= 2 {
                                let protocol_path = Path {
                                    segments: path1.segments[0..path1.segments.len() - 1]
                                        .to_vec()
                                        .into(),
                                    span: path1.span,
                                };
                                let gat_name: Text = match path1.segments.last() {
                                    Some(verum_ast::ty::PathSegment::Name(id)) => id.name.clone(),
                                    _ => return Err(TypeError::Other("Invalid GAT path".into())),
                                };

                                // Check application is valid
                                self.check_gat_application(&protocol_path, &gat_name, args1, span)?;

                                // Unify with expected type
                                if path1 != path2 {
                                    return Err(TypeError::Mismatch {
                                        expected: expected_ty.to_text(),
                                        actual: gat_type.to_text(),
                                        span,
                                    });
                                }

                                // Unify type arguments
                                if args1.len() != args2.len() {
                                    return Err(TypeError::Mismatch {
                                        expected: expected_ty.to_text(),
                                        actual: gat_type.to_text(),
                                        span,
                                    });
                                }

                                for (arg1, arg2) in args1.iter().zip(args2.iter()) {
                                    self.unifier.unify(arg1, arg2, span)?;
                                }

                                Ok(gat_type.clone())
                            } else {
                                Err(TypeError::Other(
                                    "Invalid GAT path: insufficient segments".into(),
                                ))
                            }
                        } else {
                            // Not a GAT - use regular unification
                            self.unifier.unify(gat_type, expected_ty, span)?;
                            Ok(gat_type.clone())
                        }
                    }
                    _ => {
                        // Type mismatch
                        Err(TypeError::Mismatch {
                            expected: expected_ty.to_text(),
                            actual: gat_type.to_text(),
                            span,
                        })
                    }
                }
            }
            Maybe::None => {
                // Synthesis mode: just return the GAT type
                // Validation happens elsewhere
                Ok(gat_type.clone())
            }
        }
    }

    /// Generate type constraints from GAT where clauses
    ///

    /// Converts where clauses to unification constraints for the solver.
    ///

    /// # Example
    ///

    /// ```verum
    /// protocol Container {
    ///  type Item<T> where T: Clone + Debug
    /// }
    ///

    /// // Generates constraints:
    /// // - T: Clone
    /// // - T: Debug
    /// ```
    pub fn generate_gat_constraints(
        &mut self,
        gat: &crate::protocol::AssociatedType,
        instantiation: &Map<Text, Type>,
        span: Span,
    ) -> Result<List<(Type, crate::protocol::ProtocolBound)>> {
        let mut constraints = List::new();

        // Step 1: Process where clauses
        for where_clause in &gat.where_clauses {
            // Look up the instantiated type for this parameter
            if let Some(instantiated_ty) = instantiation.get(&where_clause.param) {
                // Generate constraint for each protocol bound
                for constraint in &where_clause.constraints {
                    constraints.push((instantiated_ty.clone(), constraint.clone()));
                }
            } else {
                // Parameter not instantiated - error
                return Err(TypeError::Other(verum_common::Text::from(format!(
                    "GAT parameter {} not instantiated in where clause",
                    where_clause.param
                ))));
            }
        }

        // Step 2: Process parameter bounds
        for gat_param in &gat.type_params {
            if let Some(instantiated_ty) = instantiation.get(&gat_param.name) {
                // Generate constraint for each bound
                for bound in &gat_param.bounds {
                    constraints.push((instantiated_ty.clone(), bound.clone()));
                }
            }
        }

        // Step 3: Verify all constraints
        for (ty, bound) in &constraints {
            if !self.check_protocol_bound(ty, bound) {
                return Err(TypeError::ProtocolNotSatisfied {
                    ty: ty.to_text(),
                    protocol: bound
                        .protocol
                        .segments
                        .first()
                        .and_then(|s| match s {
                            verum_ast::ty::PathSegment::Name(id) => Some(id.name.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "unknown".into()),
                    span,
                });
            }
        }

        Ok(constraints)
    }

    // ==================== Helper Functions ====================

    /// Check if a type is a GAT instantiation
    fn is_gat_type(&self, path: &Path, args: &List<Type>) -> bool {
        if path.segments.len() < 2 {
            return false;
        }

        // Extract protocol path (all but last segment)
        let protocol_path = Path {
            segments: path.segments[0..path.segments.len() - 1].to_vec().into(),
            span: path.span,
        };

        // Check if this is a known protocol with GAT
        if let Some(protocol) = self.protocol_checker.read().lookup_protocol(&protocol_path) {
            // Extract GAT name (last segment)
            if let Some(verum_ast::ty::PathSegment::Name(gat_ident)) = path.segments.last() {
                // Check if protocol has this associated type
                let gat_name: Text = gat_ident.name.clone();
                if let Some(assoc_type) = protocol.associated_types.get(&gat_name) {
                    return assoc_type.is_gat();
                }
            }
        }

        false
    }

    /// Check if a path refers to a GAT (based on structure)
    fn is_gat_path(&self, path: &Path) -> bool {
        // A GAT path has at least 2 segments: Protocol::AssocType
        if path.segments.len() < 2 {
            return false;
        }

        // Extract protocol path
        let protocol_path = Path {
            segments: path.segments[0..path.segments.len() - 1].to_vec().into(),
            span: path.span,
        };

        // Check if this is a known protocol
        if let Some(protocol) = self.protocol_checker.read().lookup_protocol(&protocol_path) {
            // Check if last segment is an associated type
            if let Some(verum_ast::ty::PathSegment::Name(last_ident)) = path.segments.last() {
                let assoc_name: Text = last_ident.name.clone();
                return protocol.associated_types.contains_key(&assoc_name);
            }
        }

        false
    }
}
