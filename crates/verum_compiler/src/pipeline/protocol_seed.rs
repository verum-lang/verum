//! Protocol bodies for default methods and blanket implementations must have
//! the same declaration and source coordinates as the session's normal imports.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use verum_ast::Module;

use super::{BuildMode, CompilationPipeline};

const PROTOCOLS: &str = "core.base.protocols";

impl CompilationPipeline<'_> {
    pub(super) fn stdlib_source_root(&self) -> Option<PathBuf> {
        match &self.build_mode {
            BuildMode::StdlibBootstrap { config } => Some(config.stdlib_path.clone()),
            BuildMode::Normal => std::env::var_os("VERUM_STDLIB_PATH")
                .map(PathBuf::from)
                .filter(|path| path.exists()),
        }
    }

    pub(super) fn seed_typechecker_protocols(&self, checker: &mut verum_types::TypeChecker) {
        self.with_stdlib_protocols(|module| {
            checker.register_module_protocols(module, PROTOCOLS);
            checker.register_module_blanket_impls(module, PROTOCOLS);
        });
    }

    pub(super) fn seed_codegen_sources(
        &self,
        codegen: &mut verum_vbc::codegen::VbcCodegen,
        entry: &Module,
        siblings: &[Arc<Module>],
    ) -> anyhow::Result<()> {
        use verum_vbc::codegen::{CodegenError, SourceMountSession};
        let Some(root) = self.stdlib_source_root() else {
            self.with_stdlib_protocols(|module| codegen.collect_protocol_definitions(module));
            return Ok(());
        };
        let root = root.to_str().ok_or_else(|| anyhow::anyhow!("invalid stdlib source path"))?;
        let protocols = self.with_stdlib_protocols(Module::clone).filter(|module|
            entry.file_id != module.file_id && siblings.iter().all(|sibling| sibling.file_id != module.file_id));
        let mut roots = Vec::new();
        for module in protocols.iter().chain(std::iter::once(entry)).chain(siblings.iter().map(Arc::as_ref)) {
            let source = self.session.get_source(module.file_id)
                .ok_or_else(|| anyhow::anyhow!("selected declaration has no session source: {:?}", module.file_id))?;
            let path = source.path.as_ref().and_then(|path| path.to_str())
                .ok_or_else(|| anyhow::anyhow!("selected declaration requires a source path: {}", source.name))?;
            let mut selected = module.clone();
            verum_ast::prelude::inject_implicit_prelude_mount(&mut selected);
            roots.push((selected, path.to_owned()));
        }
        let files: Vec<_> = roots.iter().map(|(module, path)| (module, path.as_str())).collect();
        let open = |path: &std::path::Path| {
            let id = self.session.load_file(path).map_err(|error| CodegenError::internal(error.to_string()))?;
            self.session.get_source(id).ok_or_else(|| CodegenError::internal(
                format!("missing selected source for {}", path.display())))
        };
        codegen.resolve_unit_mounts(&files, root, Some(SourceMountSession {
            open: &open, allocate_id: &|| self.session.allocate_file_id(),
        }))?;
        if protocols.is_some() {
            // Root bodies are owned by the caller. Only the ambient protocol
            // root is contributed by this phase, including its runtime slots.
            codegen.collect_unit_declarations(&[files[0].0])?;
        }
        Ok(())
    }

    fn with_stdlib_protocols<R>(
        &self,
        seed: impl FnOnce(&Module) -> R,
    ) -> Option<R> {
        let registered = self.session.module_registry().read().get_by_path(PROTOCOLS);
        let from_source = self.stdlib_source_root().is_some();
        let source = if let Some(module) = registered.as_ref()
            && (from_source || !module.source.is_empty())
        {
            Some(&module.ast)
        } else if !from_source {
            embedded_protocols()
        } else {
            None
        };
        source.map(seed)
    }
}

/// An archive projects an empty module for mount resolution. Its companion
/// source supplies protocol defaults which require actual method bodies.
pub(crate) fn embedded_protocols() -> Option<&'static Module> {
    static MODULE: OnceLock<Option<Module>> = OnceLock::new();
    MODULE
        .get_or_init(|| {
            let source =
                crate::embedded_stdlib::get_embedded_stdlib()?.get_file("base/protocols.vr")?;
            match verum_fast_parser::Parser::new(source).parse_module() {
                Ok(module) => Some(module),
                Err(error) => {
                    tracing::warn!("Unable to parse embedded core.base.protocols: {error:?}");
                    None
                }
            }
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompilerOptions, Session, pipeline::CoreConfig};
    use std::fs;
    use verum_ast::{ItemKind, TypeDeclBody, decl::ProtocolItemKind};
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

    #[test]
    fn a_source_protocol_seed_uses_the_selected_declaration_and_session_file() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for (index, (root, method)) in [
            (first.path(), "first_default"),
            (second.path(), "second_default"),
        ]
        .into_iter()
        .enumerate()
        {
            let core = root.join("core");
            let file = core.join("base/protocols.vr");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let source = format!(
                "module core.base.protocols;\nmount core.base.value.Selected;\npublic type Default is protocol {{ fn {method}(&self, value: Selected) -> Selected {{ value }} }};\n"
            );
            fs::write(&file, &source).unwrap();
            fs::write(
                core.join("base/value.vr"),
                "module core.base.value;\npublic type Selected is { public value: Int };\n",
            )
            .unwrap();
            let mut session = Session::new(CompilerOptions::default());
            for padding in 0..index + 1 {
                session
                    .load_source_string("", root.join(format!("input_{padding}.vr")))
                    .unwrap();
            }
            let mut pipeline = CompilationPipeline::new_core(&mut session, CoreConfig::new(core));
            pipeline.load_stdlib_modules_scoped(None).unwrap();
            let mut checker = verum_types::TypeChecker::with_minimal_context();
            checker.set_module_registry(pipeline.session.module_registry());
            checker.set_lazy_resolver(pipeline.lazy_resolver.clone());
            pipeline.seed_typechecker_protocols(&mut checker);
            assert!(
                checker.diagnostics().is_empty(),
                "{:?}",
                checker.diagnostics()
            );
            let mut codegen = VbcCodegen::with_config(CodegenConfig::new("probe"));
            pipeline
                .with_stdlib_protocols(|protocols| codegen.collect_protocol_definitions(protocols));
            assert!(
                codegen.export_protocols()["Default"]
                    .default_methods
                    .contains_key(method)
            );
            let mut seen = false;
            pipeline.with_stdlib_protocols(|module| {
                let selected = pipeline.session.get_source(module.file_id).unwrap();
                assert_eq!(selected.path.as_ref(), Some(&file));
                assert_eq!(selected.source.as_str(), source);
                let declaration = module
                    .items
                    .iter()
                    .find_map(|item| match &item.kind {
                        ItemKind::Type(ty) if ty.name.name.as_str() == "Default" => Some(ty),
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(declaration.span.file_id, module.file_id);
                let TypeDeclBody::Protocol(body) = &declaration.body else {
                    panic!("expected protocol")
                };
                let ProtocolItemKind::Function { decl, .. } = &body.items[0].kind else {
                    panic!("expected method")
                };
                assert_eq!(decl.name.name.as_str(), method);
                seen = true;
            });
            assert!(seen, "the selected protocol must be available for seeding");
        }
    }

    #[test]
    fn archive_companion_protocols_supply_their_default_method_bodies() {
        if crate::embedded_stdlib::get_embedded_stdlib().is_none() {
            return;
        }
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("probe"));
        codegen.collect_protocol_definitions(embedded_protocols().unwrap());
        let declared = codegen.export_protocols();
        assert!(declared["PartialEq"].default_methods.contains_key("ne"));
        assert!(declared["Hash"].default_methods.contains_key("hash_value"));
    }
}
