//! Builtin Meta Functions
//!
//! This module provides all compile-time intrinsic functions available
//! in meta expressions, organized by their context requirements.
//!
//! ## Unified Context Model
//!
//! All builtins are categorized into tiers based on what external state they access:
//!
//! ### Tier 0: Core Primitives (Always Available)
//!
//! Pure functions that operate only on their input values:
//!
//! - **Arithmetic**: `abs`, `min`, `max`, `int_to_text`, `text_to_int`
//! - **Collections**: `list_len`, `list_push`, `list_get`, `list_concat`, etc.
//! - **Text**: `text_concat`, `text_len`, `text_split`, `text_join`, etc.
//! - **Code Gen**: `quote`, `unquote`, `stringify`, `concat_idents`, `format_ident`
//!
//! ### Tier 1: Capability-Gated Functions (Require Context)
//!
//! Functions that access external state and require explicit context declaration:
//!
//! | Context | Module | Purpose |
//! |---------|--------|---------|
//! | `MetaTypes` | `reflection`, `type_props` | Type introspection and layout |
//! | `MetaRuntime` | `runtime` | Build/platform information |
//! | `CompileDiag` | `code_gen` | Compiler diagnostics |
//! | `BuildAssets` | `build_assets` | File system access |
//!
//! ## Usage
//!
//! ```verum
//! // Tier 0 - no context required
//! meta fn pure_example() -> Int {
//!     let x = abs(-5);       // Always available
//!     let s = text_len("hi"); // Always available
//!     x + s
//! }
//!
//! // Tier 1 - requires explicit context
//! meta fn reflect_example<T>() -> Text
//!     using [MetaTypes]
//! {
//!     type_name(T)  // Requires MetaTypes context
//! }
//!
//! meta fn diagnostic_example()
//!     using [CompileDiag]
//! {
//!     compile_warning("This is deprecated")  // Requires CompileDiag
//! }
//! ```
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

pub mod arithmetic;
pub mod build_assets;
pub mod code_gen;
pub mod code_search;
pub mod collections;
pub mod context_requirements;
pub mod debugging;
pub mod dep_graph;
pub mod meta_bench;
pub mod project_info;
pub mod reflection;
pub mod runtime;
pub mod schema;
pub mod source_map;
pub mod stage_info;
pub mod testing;
pub mod tier0;
pub mod tier1;
pub mod type_props;

use verum_common::Text;

use super::context::{ConstValue, MetaContext};
use super::MetaError;

// Re-export context requirement types
pub use context_requirements::{
    BuiltinInfo, BuiltinRegistry, EnabledContexts, MissingContextError, RequiredContext,
};

/// Built-in meta function type signature
///
/// Takes a mutable reference to MetaContext for type definition lookup
/// and variable binding, and a list of constant value arguments.
///
/// Returns a Result with either the computed ConstValue or a MetaError.
pub type BuiltinMetaFn = fn(&mut MetaContext, verum_common::List<ConstValue>) -> Result<ConstValue, MetaError>;

impl MetaContext {
    /// Get all built-in meta functions with their context requirements
    ///
    /// Returns a registry mapping function names to their implementations
    /// and required contexts.
    ///
    /// Compile-time intrinsics available in meta context. Organized in tiers:
    /// - Tier 0: Always available (pure computation: arithmetic, collections, debugging, testing)
    /// - Tier 1: Requires meta context (type introspection, AST manipulation, code generation)
    /// - Tier 2: Requires specific capabilities (file embedding via @embed, config access)
    ///   All intrinsics run in the meta sandbox: no I/O, no network, no system calls.
    pub fn builtins(&self) -> BuiltinRegistry {
        let mut map = BuiltinRegistry::new();

        // Tier 0: Always available (pure computation)
        arithmetic::register_builtins(&mut map);
        collections::register_builtins(&mut map);
        debugging::register_builtins(&mut map);
        testing::register_builtins(&mut map);

        // Tier 0/1 mixed: code_gen has both pure and diagnostic functions
        code_gen::register_builtins(&mut map);

        // Tier 1: Require MetaTypes context
        reflection::register_builtins(&mut map);
        type_props::register_builtins(&mut map);

        // Tier 1: Organized tier modules (includes improved reflection builtins)
        tier1::register_all(&mut map);

        // Tier 1: Require MetaRuntime context
        runtime::register_builtins(&mut map);

        // Tier 1: Require BuildAssets context
        build_assets::register_builtins(&mut map);

        // Tier 1: Require StageInfo context
        stage_info::register_builtins(&mut map);

        // Tier 1: Require SourceMap context
        source_map::register_builtins(&mut map);

        // Tier 1: Require ProjectInfo context
        project_info::register_builtins(&mut map);

        // Tier 1: Require MetaBench context
        meta_bench::register_builtins(&mut map);

        // Tier 1: Schema validation (MetaTypes)
        schema::register_builtins(&mut map);

        // Tier 1: Dependency graph analysis (MetaTypes)
        dep_graph::register_builtins(&mut map);

        // Tier 1: Code search (MetaTypes)
        code_search::register_builtins(&mut map);

        map
    }

    /// Get a builtin function by name, checking context requirements
    ///
    /// Returns an error if the function exists but the required context
    /// is not enabled. Returns the `BuiltinInfo` (cloned) if successful.
    ///
    /// Two gates apply, in order:
    ///   1. **Reflection sandbox** — `[meta] reflection = false`
    ///      sets `MetaContext.reflection_disabled = true`, which
    ///      hard-rejects reflection-tagged contexts
    ///      (`RequiredContext::is_reflection() == true` covers
    ///      `MetaTypes` and `CompileDiag`) regardless of any
    ///      function-level `using [...]` declaration. The user's
    ///      explicit capability request is OVERRIDDEN by the
    ///      language-level sandbox.
    ///   2. **Capability declaration** — the function's
    ///      `using [...]` clause must include the required
    ///      context. This is the standard Tier 1 capability gate.
    pub fn get_builtin(&self, name: &Text) -> Result<BuiltinInfo, MetaError> {
        let builtins = self.builtins();
        match builtins.get(name) {
            Some(info) => {
                // Gate 1: reflection sandbox — overrides any
                // user-supplied capability for reflection contexts.
                if self.reflection_disabled && info.required_context.is_reflection() {
                    return Err(MetaError::MissingContext {
                        function: name.clone(),
                        required: info.required_context,
                    });
                }
                // Gate 2: standard capability check.
                if !self.enabled_contexts.is_enabled(info.required_context) {
                    return Err(MetaError::MissingContext {
                        function: name.clone(),
                        required: info.required_context,
                    });
                }
                Ok(info.clone())
            }
            None => Err(MetaError::MetaFunctionNotFound(name.clone())),
        }
    }

    /// Check if a builtin can be called with current enabled contexts.
    ///
    /// Honors both gates that `get_builtin` applies: the reflection
    /// sandbox (overrides reflection-tagged contexts) and the
    /// capability declaration. Symmetrical with `get_builtin` so
    /// `can_call_builtin` ↔ `get_builtin().is_ok()`.
    pub fn can_call_builtin(&self, name: &Text) -> bool {
        let builtins = self.builtins();
        match builtins.get(name) {
            Some(info) => {
                if self.reflection_disabled && info.required_context.is_reflection() {
                    return false;
                }
                self.enabled_contexts.is_enabled(info.required_context)
            }
            None => false,
        }
    }

    /// Get all builtins available with current enabled contexts.
    /// Honors both `reflection_disabled` and the capability gates.
    pub fn available_builtins(&self) -> Vec<Text> {
        let builtins = self.builtins();
        builtins
            .iter()
            .filter(|(_, info)| {
                if self.reflection_disabled && info.required_context.is_reflection() {
                    return false;
                }
                self.enabled_contexts.is_enabled(info.required_context)
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Get all Tier 0 (always available) builtins
    pub fn tier0_builtins(&self) -> Vec<Text> {
        let builtins = self.builtins();
        builtins
            .iter()
            .filter(|(_, info)| info.required_context == RequiredContext::None)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Get builtins requiring a specific context
    pub fn builtins_requiring_context(&self, context: RequiredContext) -> Vec<Text> {
        let builtins = self.builtins();
        builtins
            .iter()
            .filter(|(_, info)| info.required_context == context)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtins_registered() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        // Check that key builtins are registered
        assert!(builtins.contains_key(&Text::from("size_of")));
        assert!(builtins.contains_key(&Text::from("align_of")));
        assert!(builtins.contains_key(&Text::from("type_name")));
        assert!(builtins.contains_key(&Text::from("fields_of")));
        assert!(builtins.contains_key(&Text::from("variants_of")));
        assert!(builtins.contains_key(&Text::from("list_len")));
        assert!(builtins.contains_key(&Text::from("text_concat")));
        assert!(builtins.contains_key(&Text::from("abs")));
        assert!(builtins.contains_key(&Text::from("target_os")));
    }

    #[test]
    fn test_tier0_builtins_no_context_required() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        // Tier 0 functions should have RequiredContext::None
        let abs_info = builtins.get(&Text::from("abs")).unwrap();
        assert_eq!(abs_info.required_context, RequiredContext::None);

        let list_len_info = builtins.get(&Text::from("list_len")).unwrap();
        assert_eq!(list_len_info.required_context, RequiredContext::None);

        let text_concat_info = builtins.get(&Text::from("text_concat")).unwrap();
        assert_eq!(text_concat_info.required_context, RequiredContext::None);
    }

    #[test]
    fn test_tier1_builtins_require_context() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        // Reflection functions require MetaTypes
        let type_name_info = builtins.get(&Text::from("type_name")).unwrap();
        assert_eq!(type_name_info.required_context, RequiredContext::MetaTypes);

        let fields_of_info = builtins.get(&Text::from("fields_of")).unwrap();
        assert_eq!(fields_of_info.required_context, RequiredContext::MetaTypes);

        // Runtime functions require MetaRuntime
        let target_os_info = builtins.get(&Text::from("target_os")).unwrap();
        assert_eq!(target_os_info.required_context, RequiredContext::MetaRuntime);

        // Diagnostic functions require CompileDiag
        let compile_error_info = builtins.get(&Text::from("compile_error")).unwrap();
        assert_eq!(compile_error_info.required_context, RequiredContext::CompileDiag);
    }

    #[test]
    fn test_builtin_info_has_documentation() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        for (name, info) in builtins.iter() {
            assert!(!info.description.is_empty(), "Builtin '{}' has no description", name);
            assert!(!info.signature.is_empty(), "Builtin '{}' has no signature", name);
        }
    }

    #[test]
    fn test_tier0_always_available() {
        let ctx = MetaContext::new();

        // With no enabled contexts, Tier 0 should still be available
        assert!(ctx.can_call_builtin(&Text::from("abs")));
        assert!(ctx.can_call_builtin(&Text::from("list_len")));
        assert!(ctx.can_call_builtin(&Text::from("text_concat")));
    }

    #[test]
    fn test_tier1_not_available_without_context() {
        let ctx = MetaContext::new();

        // Tier 1 should NOT be available without context
        assert!(!ctx.can_call_builtin(&Text::from("type_name")));
        assert!(!ctx.can_call_builtin(&Text::from("target_os")));
        assert!(!ctx.can_call_builtin(&Text::from("compile_error")));
    }

    #[test]
    fn test_tier1_available_with_context() {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts.enable(RequiredContext::MetaTypes);

        // MetaTypes functions should now be available
        assert!(ctx.can_call_builtin(&Text::from("type_name")));
        assert!(ctx.can_call_builtin(&Text::from("fields_of")));

        // But MetaRuntime functions should still not be available
        assert!(!ctx.can_call_builtin(&Text::from("target_os")));
    }

    #[test]
    fn test_builtins_requiring_context() {
        let ctx = MetaContext::new();

        let meta_types_builtins = ctx.builtins_requiring_context(RequiredContext::MetaTypes);
        assert!(meta_types_builtins.contains(&Text::from("type_name")));
        assert!(meta_types_builtins.contains(&Text::from("fields_of")));

        let meta_runtime_builtins = ctx.builtins_requiring_context(RequiredContext::MetaRuntime);
        assert!(meta_runtime_builtins.contains(&Text::from("target_os")));
        assert!(meta_runtime_builtins.contains(&Text::from("target_arch")));
    }

    #[test]
    fn test_build_assets_builtins_registered() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        // BuildAssets functions should be registered
        assert!(builtins.contains_key(&Text::from("load_text")));
        assert!(builtins.contains_key(&Text::from("include_bytes")));
        assert!(builtins.contains_key(&Text::from("include_str")));
        assert!(builtins.contains_key(&Text::from("asset_exists")));
        assert!(builtins.contains_key(&Text::from("asset_list_dir")));
        assert!(builtins.contains_key(&Text::from("asset_metadata")));
    }

    #[test]
    fn test_build_assets_require_context() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        // BuildAssets functions require BuildAssets context
        let load_text_info = builtins.get(&Text::from("load_text")).unwrap();
        assert_eq!(load_text_info.required_context, RequiredContext::BuildAssets);

        let include_bytes_info = builtins.get(&Text::from("include_bytes")).unwrap();
        assert_eq!(include_bytes_info.required_context, RequiredContext::BuildAssets);
    }

    #[test]
    fn test_build_assets_not_available_without_context() {
        let ctx = MetaContext::new();

        // BuildAssets should NOT be available without context
        assert!(!ctx.can_call_builtin(&Text::from("load_text")));
        assert!(!ctx.can_call_builtin(&Text::from("include_bytes")));
    }

    #[test]
    fn test_build_assets_available_with_context() {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts.enable(RequiredContext::BuildAssets);

        // BuildAssets functions should now be available
        assert!(ctx.can_call_builtin(&Text::from("load_text")));
        assert!(ctx.can_call_builtin(&Text::from("include_bytes")));
        assert!(ctx.can_call_builtin(&Text::from("asset_exists")));
    }

    #[test]
    fn test_debugging_builtins_registered() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        // Debugging builtins should be registered as Tier 0
        assert!(builtins.contains_key(&Text::from("meta_trace_on")), "meta_trace_on not found");
        assert!(builtins.contains_key(&Text::from("meta_trace_off")), "meta_trace_off not found");
        assert!(builtins.contains_key(&Text::from("meta_trace_log")), "meta_trace_log not found");
        assert!(builtins.contains_key(&Text::from("meta_trace_dump")), "meta_trace_dump not found");
        assert!(builtins.contains_key(&Text::from("meta_trace_clear")), "meta_trace_clear not found");
        assert!(builtins.contains_key(&Text::from("meta_trace_is_enabled")), "meta_trace_is_enabled not found");
        assert!(builtins.contains_key(&Text::from("meta_trace_depth")), "meta_trace_depth not found");
    }

    #[test]
    fn test_debugging_builtins_are_tier0() {
        let ctx = MetaContext::new();
        let builtins = ctx.builtins();

        let trace_on_info = builtins.get(&Text::from("meta_trace_on")).expect("meta_trace_on not found");
        assert_eq!(trace_on_info.required_context, RequiredContext::None, "meta_trace_on should be Tier 0");

        // Should be callable without any context enabled
        assert!(ctx.can_call_builtin(&Text::from("meta_trace_on")));
        assert!(ctx.can_call_builtin(&Text::from("meta_trace_off")));
        assert!(ctx.can_call_builtin(&Text::from("meta_trace_log")));
    }

    #[test]
    fn test_debugging_builtins_via_get_builtin() {
        let ctx = MetaContext::new();

        // get_builtin should return Ok for tier 0 debugging builtins
        let result = ctx.get_builtin(&Text::from("meta_trace_on"));
        assert!(result.is_ok(), "get_builtin(meta_trace_on) failed: {:?}", result);

        let result = ctx.get_builtin(&Text::from("meta_trace_log"));
        assert!(result.is_ok(), "get_builtin(meta_trace_log) failed: {:?}", result);
    }

    /// Pin: `[meta] reflection = false` — reflection-tagged contexts
    /// are hard-rejected even when the function declared the
    /// capability via `using [...]`.
    #[test]
    fn reflection_disabled_rejects_meta_types_even_with_using_clause() {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts.enable(RequiredContext::MetaTypes);
        // Sanity: with reflection enabled the call goes through.
        assert!(ctx.can_call_builtin(&Text::from("type_name")));

        // Now seal the sandbox: reflection_disabled overrides the
        // user's `using [MetaTypes]` capability.
        ctx.reflection_disabled = true;
        assert!(
            !ctx.can_call_builtin(&Text::from("type_name")),
            "reflection_disabled must override user-supplied MetaTypes capability"
        );
        let err = ctx
            .get_builtin(&Text::from("type_name"))
            .expect_err("get_builtin must fail under reflection_disabled");
        // Error is MissingContext (the sandbox rejection reuses the
        // capability-missing error shape — the diagnostic builder
        // upstream surfaces the gate name).
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("MetaTypes") || msg.contains("MissingContext"),
            "diagnostic should mention the missing context (got: {})",
            msg
        );
    }

    /// Pin: `reflection_disabled` rejects `CompileDiag`.
    #[test]
    fn reflection_disabled_rejects_compile_diag() {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts.enable(RequiredContext::CompileDiag);
        assert!(ctx.can_call_builtin(&Text::from("compile_error")));
        ctx.reflection_disabled = true;
        assert!(!ctx.can_call_builtin(&Text::from("compile_error")));
    }

    /// Pin: `reflection_disabled` does NOT affect non-reflection
    /// Tier 1 contexts (`MetaRuntime`, `BuildAssets`, etc.) — only
    /// the reflection surface is sealed.
    #[test]
    fn reflection_disabled_preserves_non_reflection_capabilities() {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts.enable(RequiredContext::MetaRuntime);
        ctx.enabled_contexts.enable(RequiredContext::BuildAssets);
        ctx.reflection_disabled = true;
        // Non-reflection capabilities survive.
        assert!(
            ctx.can_call_builtin(&Text::from("target_os")),
            "MetaRuntime is NOT reflection — should survive sandbox"
        );
        assert!(
            ctx.can_call_builtin(&Text::from("load_text")),
            "BuildAssets is NOT reflection — should survive sandbox"
        );
    }

    /// Pin: `available_builtins` excludes reflection-tagged
    /// builtins under `reflection_disabled`, even when the
    /// capability was granted.
    #[test]
    fn available_builtins_excludes_reflection_under_seal() {
        let mut ctx = MetaContext::new();
        ctx.enabled_contexts.enable(RequiredContext::MetaTypes);
        ctx.enabled_contexts.enable(RequiredContext::MetaRuntime);
        ctx.reflection_disabled = true;
        let available = ctx.available_builtins();
        assert!(
            !available.contains(&Text::from("type_name")),
            "type_name (MetaTypes) must NOT be in available list under seal"
        );
        assert!(
            available.contains(&Text::from("target_os")),
            "target_os (MetaRuntime, non-reflection) must remain available"
        );
    }

    /// Pin: `RequiredContext::is_reflection()` covers the
    /// documented reflection set (`MetaTypes` + `CompileDiag`)
    /// and excludes everything else.
    #[test]
    fn is_reflection_covers_meta_types_and_compile_diag_only() {
        assert!(RequiredContext::MetaTypes.is_reflection());
        assert!(RequiredContext::CompileDiag.is_reflection());
        // Non-reflection contexts.
        assert!(!RequiredContext::None.is_reflection());
        assert!(!RequiredContext::MetaRuntime.is_reflection());
        assert!(!RequiredContext::BuildAssets.is_reflection());
        assert!(!RequiredContext::SourceMap.is_reflection());
        assert!(!RequiredContext::ProjectInfo.is_reflection());
        assert!(!RequiredContext::MetaBench.is_reflection());
        assert!(!RequiredContext::StageInfo.is_reflection());
    }
}
