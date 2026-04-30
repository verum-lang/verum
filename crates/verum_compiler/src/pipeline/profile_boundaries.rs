//! Module-level language-profile boundary enforcement.
//!
//! Extracted from `pipeline.rs` (#106 Phase 7). Validates that
//! imports respect language-profile boundaries:
//!
//!   * **Application** profile can only import from Application modules.
//!   * **Systems** profile can import from Systems and Application modules.
//!   * **Research** profile can import from any module.
//!
//! Profile is read from a module's `@profile(...)` attribute (either
//! at module level or on a `module` declaration); the default for
//! modules with no annotation is `Application` (most restrictive,
//! safe assumption).
//!
//! `type_to_text` lives here too because it's the canonical
//! AST-type → human-readable text renderer used by both the
//! profile-boundary checker and the protocol-coherence cluster
//! (`pipeline/coherence.rs`); keeping it next to the most
//! consumer-heavy use site avoids circular dependencies between
//! the two submodules.

use anyhow::Result;

use verum_ast::Module;
use verum_common::Text;
use verum_diagnostics::{DiagnosticBuilder, Severity};
use verum_modules::{LanguageProfile, ProfileChecker};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Profile boundary enforcement (module-level profile checking).
    pub(super) fn check_profile_boundaries(&self, module: &Module) -> Result<()> {
        // Create a profile checker with the default compilation
        // profile. In a more complete implementation, this would
        // come from `verum.toml` or CLI flags.
        let _profile_checker = ProfileChecker::new(LanguageProfile::Application);

        // Extract the current module's profile from `@profile` attribute.
        let current_profile = self.extract_module_profile(module);

        // Get current module path for error reporting.
        let current_module_path = if let Some(item) = module.items.first() {
            if let Some(source_file) = self.session.get_source(item.span.file_id) {
                if let Some(ref file_path) = source_file.path {
                    file_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(Text::from)
                        .unwrap_or_else(|| Text::from("main"))
                } else {
                    Text::from("main")
                }
            } else {
                Text::from("main")
            }
        } else {
            Text::from("main")
        };

        // Check all imports for profile compatibility.
        for item in &module.items {
            if let verum_ast::ItemKind::Mount(import) = &item.kind {
                let target_path = self.import_to_module_path(import);
                let target_profile = self.get_module_profile_from_registry(&target_path);

                if !current_profile.can_access(target_profile) {
                    let mut builder = DiagnosticBuilder::new(Severity::Error)
                        .message(format!(
                            "Profile boundary violation: module '{}' with {} profile cannot import from module '{}' with {} profile",
                            current_module_path,
                            current_profile,
                            target_path,
                            target_profile
                        ));
                    builder = builder.span(self.session.convert_span(import.span));
                    self.session.emit_diagnostic(builder.build());
                }
            }
        }

        Ok(())
    }

    /// Extract the language profile from a module's `@profile` attribute.
    pub(super) fn extract_module_profile(&self, module: &Module) -> LanguageProfile {
        for attr in &module.attributes {
            if attr.name.as_str() == "profile" {
                if let verum_common::Maybe::Some(ref args) = attr.args
                    && let Some(first_arg) = args.first()
                {
                    if let Some(profile_name) = self.extract_profile_name(first_arg)
                        && let Some(profile) = LanguageProfile::from_str(&profile_name)
                    {
                        return profile;
                    }
                }
            }
        }

        for item in &module.items {
            for attr in &item.attributes {
                if attr.name.as_str() == "profile" {
                    if let verum_common::Maybe::Some(ref args) = attr.args
                        && let Some(first_arg) = args.first()
                    {
                        if let Some(profile_name) = self.extract_profile_name(first_arg)
                            && let Some(profile) = LanguageProfile::from_str(&profile_name)
                        {
                            return profile;
                        }
                    }
                }
            }
        }

        LanguageProfile::Application
    }

    /// Extract profile name from an expression (handles both string
    /// and identifier forms).
    pub(super) fn extract_profile_name(&self, expr: &verum_ast::expr::Expr) -> Option<String> {
        use verum_ast::expr::ExprKind;
        use verum_ast::LiteralKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Text(s) => Some(s.as_str().to_string()),
                _ => None,
            },
            ExprKind::Path(path) => {
                if path.segments.len() == 1
                    && let verum_ast::PathSegment::Name(ident) = &path.segments[0]
                {
                    return Some(ident.name.to_string());
                }
                None
            }
            _ => None,
        }
    }

    /// Convert an import to a module-path string.
    pub(super) fn import_to_module_path(&self, import: &verum_ast::decl::MountDecl) -> Text {
        use verum_ast::decl::MountTreeKind;
        use verum_ast::PathSegment;

        let path = match &import.tree.kind {
            MountTreeKind::Path(path) => path,
            MountTreeKind::Glob(path) => path,
            MountTreeKind::Nested { prefix, .. } => prefix,
            // #5 / P1.5 — file-relative mount. The session loader
            // has already resolved the file at this point; surface
            // the literal path as the module identifier so
            // downstream logging / diagnostics can refer to it.
            // Returning early avoids returning a synthesised empty
            // Path.
            MountTreeKind::File { path, .. } => return path.clone(),
        };

        let segments: Vec<&str> = path
            .segments
            .iter()
            .map(|seg| match seg {
                PathSegment::Name(ident) => ident.name.as_str(),
                PathSegment::SelfValue => "self",
                PathSegment::Super => "super",
                PathSegment::Cog => "cog",
                PathSegment::Relative => ".",
            })
            .collect();

        Text::from(segments.join("."))
    }

    /// Look up a module's profile in the registry; default to
    /// Application (safe / most restrictive) when not found.
    pub(super) fn get_module_profile_from_registry(&self, module_path: &Text) -> LanguageProfile {
        if let Some(module) = self.modules.get(module_path) {
            return self.extract_module_profile(module);
        }

        // Try dot-separated path-segment partial matches
        // (e.g., "core.sys.linux" → "core.sys").
        let path_str = module_path.as_str();
        for (key, module) in self.modules.iter() {
            if path_str.starts_with(key.as_str()) || key.as_str().starts_with(path_str) {
                return self.extract_module_profile(module);
            }
        }

        LanguageProfile::Application
    }

    /// Convert an AST type to its canonical text representation.
    ///
    /// Used by both the profile-boundary checker and the
    /// protocol-coherence cluster.
    pub(super) fn type_to_text(&self, ty: &verum_ast::Type) -> Text {
        use verum_ast::ty::{GenericArg, PathSegment, TypeKind};

        match &ty.kind {
            TypeKind::Path(path) => {
                if path.segments.len() == 1 {
                    match &path.segments[0] {
                        PathSegment::Name(ident) => ident.name.clone(),
                        PathSegment::SelfValue => Text::from("self"),
                        PathSegment::Super => Text::from("super"),
                        PathSegment::Cog => Text::from("cog"),
                        PathSegment::Relative => Text::from("."),
                    }
                } else {
                    let segments: Vec<&str> = path
                        .segments
                        .iter()
                        .map(|seg| match seg {
                            PathSegment::Name(ident) => ident.name.as_str(),
                            PathSegment::SelfValue => "self",
                            PathSegment::Super => "super",
                            PathSegment::Cog => "cog",
                            PathSegment::Relative => ".",
                        })
                        .collect();
                    Text::from(segments.join("."))
                }
            }
            TypeKind::Generic { base, args } => {
                let base_text = self.type_to_text(base);
                if args.is_empty() {
                    base_text
                } else {
                    let args_str: Vec<String> = args
                        .iter()
                        .map(|arg| match arg {
                            GenericArg::Type(t) => self.type_to_text(t).to_string(),
                            GenericArg::Const(e) => format!("{:?}", e),
                            GenericArg::Lifetime(_) => "'_".to_string(),
                            GenericArg::Binding(binding) => format!(
                                "{}={}",
                                binding.name.name,
                                self.type_to_text(&binding.ty)
                            ),
                        })
                        .collect();
                    Text::from(format!("{}<{}>", base_text, args_str.join(", ")))
                }
            }
            TypeKind::Tuple(types) => {
                let type_strs: Vec<String> =
                    types.iter().map(|t| self.type_to_text(t).to_string()).collect();
                Text::from(format!("({})", type_strs.join(", ")))
            }
            TypeKind::Reference { inner, mutable, .. } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("&mut {}", inner_text))
                } else {
                    Text::from(format!("&{}", inner_text))
                }
            }
            TypeKind::CheckedReference { inner, mutable } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("&checked mut {}", inner_text))
                } else {
                    Text::from(format!("&checked {}", inner_text))
                }
            }
            TypeKind::UnsafeReference { inner, mutable } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("&unsafe mut {}", inner_text))
                } else {
                    Text::from(format!("&unsafe {}", inner_text))
                }
            }
            TypeKind::Array { element, size } => {
                let elem_text = self.type_to_text(element);
                if let Some(size_expr) = size {
                    Text::from(format!("[{}; {:?}]", elem_text, size_expr))
                } else {
                    Text::from(format!("[{}]", elem_text))
                }
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                let params_str: Vec<String> = params
                    .iter()
                    .map(|p| self.type_to_text(p).to_string())
                    .collect();
                let ret_text = self.type_to_text(return_type);
                Text::from(format!("fn({}) -> {}", params_str.join(", "), ret_text))
            }
            TypeKind::Ownership { mutable, inner } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("Heap<mut {}>", inner_text))
                } else {
                    Text::from(format!("Heap<{}>", inner_text))
                }
            }
            TypeKind::Pointer { mutable, inner } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("*mut {}", inner_text))
                } else {
                    Text::from(format!("*const {}", inner_text))
                }
            }
            TypeKind::VolatilePointer { mutable, inner } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("*volatile mut {}", inner_text))
                } else {
                    Text::from(format!("*volatile {}", inner_text))
                }
            }
            TypeKind::Slice(inner) => {
                let inner_text = self.type_to_text(inner);
                Text::from(format!("[{}]", inner_text))
            }
            TypeKind::Qualified {
                self_ty,
                trait_ref,
                assoc_name,
            } => {
                let self_text = self.type_to_text(self_ty);
                Text::from(format!(
                    "<{} as {}>::{}",
                    self_text, trait_ref, assoc_name.name
                ))
            }
            TypeKind::Refined { base, predicate } => {
                let base_text = self.type_to_text(base);
                // The sigma surface form lives here too (binder carried
                // by the predicate); render it distinctly when bound.
                match &predicate.binding {
                    verum_common::Maybe::Some(binder) => {
                        Text::from(format!("{}: {} where ...", binder.name, base_text))
                    }
                    verum_common::Maybe::None => Text::from(format!("{}{{...}}", base_text)),
                }
            }
            TypeKind::Bounded { base, .. } => self.type_to_text(base),
            _ if ty.kind.primitive_name().is_some() => {
                Text::from(ty.kind.primitive_name().unwrap_or("?"))
            }
            _ => Text::from("?"),
        }
    }
}
