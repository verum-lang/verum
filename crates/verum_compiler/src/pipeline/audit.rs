//! Stdlib quality / audit reporting.
//!
//! Extracted from `pipeline.rs` (#106 Phase 18). Houses the
//! diagnostic introspection surface that quality-gate / audit
//! tooling consumes. Five methods:
//!
//!   * `count_stdlib_type_errors` — header-only registration
//!     errors (type names + function signatures + protocols +
//!     impl blocks). Cheap; suppressed during normal compilation
//!     but exposed here for `verum audit` and the
//!     `core_stdlib_validation_test` ratchet.
//!   * `count_stdlib_body_errors` — full-body type checking on
//!     all core/ modules; the comprehensive variant that
//!     exposes ~950 errors that were previously hidden by the
//!     skip-stdlib-bodies gate.
//!   * `get_stdlib_registry` / `get_stdlib_static_lib` /
//!     `get_stdlib_bitcode` — read-only accessors for the
//!     phase0-prepared artefacts (used by AOT linking + JIT
//!     dispatch).

use std::path::PathBuf;

use std::time::Instant;

use anyhow::Result;
use tracing::{debug, warn};

use verum_ast::Module;
use verum_common::Text;

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Count internal stdlib type registration errors.
    ///
    /// These are errors that occur when registering stdlib types, functions,
    /// protocols, and impl blocks into the type checker. They are suppressed
    /// during normal compilation but tracked for quality metrics.
    ///
    /// Returns (type_errors, func_errors, proto_errors, impl_errors, details)
    pub fn count_stdlib_type_errors(&mut self) -> (usize, usize, usize, usize, Vec<String>) {
        // Load stdlib modules if not yet loaded
        if self.modules.is_empty() {
            if let Err(e) = self.load_stdlib_modules() {
                warn!("Failed to load stdlib modules: {}", e);
                return (0, 0, 0, 0, vec![]);
            }
        }

        // Sort for deterministic iteration (self.modules is a HashMap):
        // shallower module keys come first so top-level stdlib functions beat
        // nested-module helpers when short names collide.
        let mut stdlib_entries: Vec<_> = self.modules.iter()
            .filter(|(k, _)| k.as_str().starts_with("core"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        stdlib_entries.sort_by(|(a, _), (b, _)| {
            let depth_a = a.as_str().matches('.').count();
            let depth_b = b.as_str().matches('.').count();
            depth_a.cmp(&depth_b).then_with(|| a.as_str().cmp(b.as_str()))
        });
        let stdlib_modules: Vec<_> = stdlib_entries.iter().map(|(_, v)| v.clone()).collect();

        if stdlib_modules.is_empty() {
            return (0, 0, 0, 0, vec![]);
        }

        let mut checker = verum_types::TypeChecker::with_minimal_context();
        checker.register_builtins();

        let mut type_errors = 0usize;
        let mut func_errors = 0usize;
        let mut proto_errors = 0usize;
        let mut impl_errors = 0usize;
        let mut details = Vec::new();

        // S0a: Register all stdlib type names
        for stdlib_mod in &stdlib_modules {
            checker.register_all_type_names(&stdlib_mod.items);
        }
        // S0b: Resolve stdlib type definitions
        let mut resolution_stack = verum_common::List::new();
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    if let Err(e) = checker.resolve_type_definition(type_decl, &mut resolution_stack) {
                        type_errors += 1;
                        details.push(format!("[TYPE] {}: {:?}", type_decl.name.name, e));
                    }
                }
            }
        }
        // S1: Register stdlib function signatures
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    if let Err(e) = checker.register_function_signature(func) {
                        func_errors += 1;
                        details.push(format!("[FUNC] {}: {:?}", func.name.name, e));
                    }
                }
            }
        }
        // S2: Register stdlib protocols
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                    if let Err(e) = checker.register_protocol(protocol_decl) {
                        proto_errors += 1;
                        details.push(format!("[PROTO] {}: {:?}", protocol_decl.name.name, e));
                    }
                }
            }
        }
        // S3: Register stdlib impl blocks
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                    if let Err(e) = checker.register_impl_block(impl_decl) {
                        impl_errors += 1;
                        let impl_name = match &impl_decl.kind {
                            verum_ast::decl::ImplKind::Inherent(ty) => format!("{:?}", ty),
                            verum_ast::decl::ImplKind::Protocol { protocol, for_type, .. } => {
                                format!("{:?} for {:?}", protocol, for_type)
                            }
                        };
                        details.push(format!("[IMPL] {}: {:?}", impl_name, e));
                    }
                }
            }
        }

        (type_errors, func_errors, proto_errors, impl_errors, details)
    }

    /// Count all stdlib errors including full body type checking.
    ///
    /// This runs the full type checker (including function bodies, impl blocks, etc.)
    /// on all core/ modules to identify all type errors. Returns a map of
    /// error category -> count, plus detailed error messages.
    ///
    /// Returns (total_errors, category_counts, details)
    pub fn count_stdlib_body_errors(&mut self) -> (usize, std::collections::HashMap<String, usize>, Vec<String>) {
        // Load stdlib modules if not yet loaded
        if self.modules.is_empty() {
            if let Err(e) = self.load_stdlib_modules() {
                warn!("Failed to load stdlib modules: {}", e);
                return (0, std::collections::HashMap::new(), vec![]);
            }
        }

        // Filter stdlib modules, excluding platform-specific modules for other OSes.
        // The @cfg(target_os = "X") on `module X;` declarations in mod.vr gates these
        // modules, but since file-based modules are loaded independently, we filter
        // by module path here.
        let host_os = {
            let raw = std::env::consts::OS;
            if raw == "darwin" { "macos" } else { raw }
        };
        let mut stdlib_modules: Vec<(Text, std::sync::Arc<Module>)> = self.modules.iter()
            .filter(|(k, _)| k.as_str().starts_with("core"))
            .filter(|(k, _)| {
                let mp = k.as_str();
                // Skip foreign platform modules
                if (mp.contains(".linux") && host_os != "linux") ||
                   (mp.contains(".windows") && host_os != "windows") ||
                   (mp.contains(".darwin") && host_os != "macos") ||
                   (mp.contains(".freebsd") && host_os != "freebsd") {
                    return false;
                }
                true
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Sort modules by path for deterministic type checking order.
        // Without this, HashMap iteration order varies between runs, causing
        // type variable bindings to differ and error counts to fluctuate.
        // Shallower (fewer-dot) module keys are prioritized so top-level stdlib
        // functions beat nested-module helpers when short names collide (e.g.
        // `core.base.memory::drop<T>` should win over
        // `core.base.iterator.Transducer::drop<A>`).
        stdlib_modules.sort_by(|(a, _), (b, _)| {
            let depth_a = a.as_str().matches('.').count();
            let depth_b = b.as_str().matches('.').count();
            depth_a.cmp(&depth_b).then_with(|| a.as_str().cmp(b.as_str()))
        });

        if stdlib_modules.is_empty() {
            return (0, std::collections::HashMap::new(), vec![]);
        }

        let mut checker = verum_types::TypeChecker::with_minimal_context();
        checker.register_builtins();

        // Enable lenient context resolution for stdlib body checking.
        // Context declarations (RandomSource, ComputeDevice, etc.) are defined
        // in core/ .vr files but may not be loaded into the context resolver.
        checker.set_lenient_contexts(true);

        // Configure type checker with module registry for cross-file resolution
        let registry = self.session.module_registry();
        checker.set_module_registry(registry.clone());

        // S0a: Register all stdlib type names
        for (_, stdlib_mod) in &stdlib_modules {
            checker.register_all_type_names(&stdlib_mod.items);
        }
        // S0b: Resolve stdlib type definitions
        let mut resolution_stack = verum_common::List::new();
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    let _ = checker.resolve_type_definition(type_decl, &mut resolution_stack);
                }
            }
        }
        // S1: Register stdlib function signatures
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    let _ = checker.register_function_signature(func);
                }
                // Register extern block (FFI) function signatures
                if let verum_ast::ItemKind::ExternBlock(extern_block) = &item.kind {
                    for func in &extern_block.functions {
                        let _ = checker.register_function_signature(func);
                    }
                }
            }
        }
        // S2: Register stdlib protocols
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                    let _ = checker.register_protocol(protocol_decl);
                }
            }
        }
        // S3: Register stdlib impl blocks
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                    let _ = checker.register_impl_block(impl_decl);
                }
            }
        }

        // S4: Process imports for each stdlib module
        // This resolves mount statements (e.g., mount super.constants.*)
        // so that imported names are available during body type checking.
        {
            let reg = registry.read();
            for (module_path, stdlib_mod) in &stdlib_modules {
                for item in &stdlib_mod.items {
                    if let verum_ast::ItemKind::Mount(import) = &item.kind {
                        let _ = checker.process_import(import, module_path.as_str(), &reg);
                    }
                }
            }
        }

        // S5: Register all const and static declarations as variables
        // This makes constants visible in function bodies.
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Const(const_decl) = &item.kind {
                    checker.pre_register_const(const_decl);
                }
                if let verum_ast::ItemKind::Static(static_decl) = &item.kind {
                    if let Ok(ty) = checker.ast_to_type(&static_decl.ty) {
                        checker.pre_register_static(&static_decl.name.name, ty);
                    }
                }
            }
        }

        // S6: Register platform intrinsics and module path stubs
        {
            use verum_types::{Type, TypeVar, TypeScheme};
            for &(name, pc) in &[("num_cpus",0),("getpagesize",0),("raw_read",3),("raw_write",3),("embedded_ctx_get",1),("embedded_ctx_set",2),("embedded_ctx_clear",1),("embedded_ctx_push_frame",0),("embedded_ctx_pop_frame",0),("__load_i64",1)] {
                let p: verum_common::List<Type> = (0..pc).map(|_| Type::Var(TypeVar::fresh())).collect();
                let r = Type::Var(TypeVar::fresh());
                let v: verum_common::List<TypeVar> = p.iter().filter_map(|t| if let Type::Var(v) = t { Some(*v) } else { None }).chain(std::iter::once(if let Type::Var(v) = &r { *v } else { unreachable!() })).collect();
                checker.ctx_env_insert(name, TypeScheme::poly(v, Type::function(p, r)));
            }
            for &s in &["core","darwin","linux","x86_64"] {
                let t = Type::Named { path: verum_ast::ty::Path::single(verum_ast::Ident::new(s, verum_ast::span::Span::default())), args: verum_common::List::new() };
                checker.ctx_env_insert(s, TypeScheme::mono(t));
            }
            for &n in &["GradientTape","GradientAccumulation","TIMER_STATUS","SysTlsError","ComputeDevice","RandomSource","SegmentError","CpuFeatures","SysStat","ChildSpecOpaque","Sockaddr","MemProt","MapFlags","VmAddress","ExitStatus","PathBuf","GlobalAllocator","GPUBuffer","FileDesc","Once","ProcessGroup","ExecutionEnv","ChildSpec","ContextSlots","ThreadControlBlock","Cotangent"] {
                let t = Type::Named { path: verum_ast::ty::Path::single(verum_ast::Ident::new(n, verum_ast::span::Span::default())), args: verum_common::List::new() };
                checker.ctx_define_type(n, t.clone());
                // Only register in env if not already present as a constructor function.
                // Types like FileDesc are newtypes with constructors registered at S0b.
                // Overwriting them here with TypeScheme::mono(Named) causes NotCallable errors.
                if checker.ctx_env_lookup(n).is_none() {
                    checker.ctx_env_insert(n, TypeScheme::mono(t));
                }
            }
        }

        // Now run full body type checking on functions only (not impl blocks, which
        // require additional setup).
        let mut total_errors = 0usize;
        let mut category_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut details = Vec::new();
        let mut checked_count = 0usize;
        let mut skipped_count = 0usize;

        // Create a CfgEvaluator for the current host platform.
        // This is used to skip items gated by @cfg predicates for other platforms.
        let cfg_eval = verum_ast::cfg::CfgEvaluator::new();

        for (module_path, stdlib_mod) in &stdlib_modules {
            let module_start = Instant::now();

            // Skip modules whose @cfg attributes don't match the current platform.
            if !cfg_eval.should_include(&stdlib_mod.attributes) {
                continue;
            }

            for item in &stdlib_mod.items {
                // Skip items gated by @cfg that don't match the current platform.
                if !cfg_eval.should_include(&item.attributes) {
                    continue;
                }
                // Critical fix mirroring `verum_vbc::should_compile_item`:
                // the parser puts `@cfg` on `Function.attributes` (inner
                // decl), not on `Item.attributes`.  The outer-only check
                // above silently bypasses every function-level @cfg gate.
                // Walk the inner FunctionDecl's attributes too.
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    if !cfg_eval.should_include(&func.attributes) {
                        continue;
                    }
                }

                // Only check functions (not impls - they need separate setup)
                let should_check = matches!(
                    &item.kind,
                    verum_ast::ItemKind::Function(_) |
                    verum_ast::ItemKind::Const(_) |
                    verum_ast::ItemKind::Static(_)
                );
                if !should_check {
                    continue;
                }

                // Skip if module already took too long (>10s per module = likely hanging)
                if module_start.elapsed().as_secs() > 10 {
                    skipped_count += 1;
                    continue;
                }

                checked_count += 1;
                let check_result = checker.check_item(item);

                if let Err(e) = check_result {
                    total_errors += 1;
                    let error_str = format!("{:?}", e);
                    // Categorize the error
                    let error_display = format!("{}", e);
                    let category = if error_str.contains("TypeNotFound") {
                        "TypeNotFound"
                    } else if error_str.contains("UnresolvedName") || error_str.contains("UndefinedVariable") {
                        "UnresolvedName"
                    } else if error_str.contains("TypeMismatch") || error_str.contains("Mismatch") || error_display.contains("Type mismatch") || error_display.contains("type mismatch") {
                        "TypeMismatch"
                    } else if error_str.contains("NotCallable") || error_display.contains("not a function type") {
                        "NotCallable"
                    } else if error_str.contains("FieldNotFound") || (error_display.contains("field") && error_display.contains("not found")) {
                        "FieldNotFound"
                    } else if error_str.contains("MethodNotFound") || (error_display.contains("method") && error_display.contains("not found")) {
                        "MethodNotFound"
                    } else if error_str.contains("ArityMismatch") || error_display.contains("wrong number of arguments") || error_display.contains("Function requires") || error_display.contains("Function accepts") {
                        "ArityMismatch"
                    } else if error_str.contains("UnresolvedPlaceholder") {
                        "UnresolvedPlaceholder"
                    } else if error_str.contains("NotImplemented") || error_str.contains("Unsupported") || error_display.contains("Unknown variant constructor") {
                        "NotImplemented"
                    } else if error_str.contains("unbound variable") || error_display.contains("unbound variable") {
                        "UnboundVariable"
                    } else if error_str.contains("super keyword") {
                        "SuperKeyword"
                    } else if error_str.contains("undefined context") || error_display.contains("undefined context") || error_display.contains("missing context") {
                        "UndefinedContext"
                    } else if error_display.contains("Pattern expects") || error_display.contains("Expected reference type for reference pattern") {
                        "PatternError"
                    } else if error_str.contains("infinite type") || error_display.contains("recursion") || error_display.contains("stack overflow") {
                        "InfiniteType"
                    } else if error_str.contains("invalid cast") || error_str.contains("InvalidCast") || error_display.contains("invalid cast") || error_display.contains("cannot cast") {
                        "InvalidCast"
                    } else if error_display.contains("Cannot iterate") {
                        "IterationError"
                    } else if error_display.contains("Cannot dereference") {
                        "DerefError"
                    } else if error_display.contains("Cannot access field") {
                        "FieldAccessError"
                    } else {
                        "Other"
                    };
                    *category_counts.entry(category.to_string()).or_insert(0) += 1;

                    // Extract item name for context
                    let item_name = match &item.kind {
                        verum_ast::ItemKind::Function(f) => format!("fn {}", f.name.name),
                        verum_ast::ItemKind::Impl(impl_decl) => {
                            match &impl_decl.kind {
                                verum_ast::decl::ImplKind::Inherent(ty) => format!("impl {:?}", ty),
                                verum_ast::decl::ImplKind::Protocol { protocol, for_type, .. } => {
                                    format!("impl {:?} for {:?}", protocol, for_type)
                                }
                            }
                        }
                        verum_ast::ItemKind::Const(c) => format!("const {}", c.name.name),
                        verum_ast::ItemKind::Static(s) => format!("static {}", s.name.name),
                        _ => format!("{:?}", std::mem::discriminant(&item.kind)),
                    };

                    if details.len() < 500 {
                        details.push(format!("[{}] {} in {}: {}", category, item_name, module_path.as_str(), e));
                    }
                }
            }
        }

        // Add summary stats to details
        details.push(format!("[SUMMARY] checked={}, skipped_timeout={}, errors={}", checked_count, skipped_count, total_errors));

        (total_errors, category_counts, details)
    }

    /// Get the stdlib registry (for interpreter/JIT)
    pub fn get_stdlib_registry(&self) -> Option<&crate::phases::phase0_stdlib::StdlibRegistry> {
        self.stdlib_artifacts.as_ref().map(|a| &a.registry)
    }

    /// Get the stdlib static library path (for AOT linking)
    pub fn get_stdlib_static_lib(&self) -> Option<&PathBuf> {
        self.stdlib_artifacts.as_ref().map(|a| &a.static_library)
    }

    /// Get the stdlib LLVM bitcode path (for LTO)
    pub fn get_stdlib_bitcode(&self) -> Option<&PathBuf> {
        self.stdlib_artifacts.as_ref().map(|a| &a.bitcode_library)
    }
}
